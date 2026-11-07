//! `morph_svg`: morph the shapes of one SVG into the shapes of another, as a short alpha motion graphic that the
//! promo template uses like any other motion asset (`overlay.asset`, `end_card.asset`).
//!
//! Both SVGs are parsed once with usvg (text to paths, styles resolved, shapes to paths, absolute coordinates).
//! Their filled paths are fitted into a shared canonical square and paired: equal path counts morph pairwise,
//! one-to-many uses flubber's `separate`, many-to-one its `combine`. The path interpolation itself is computed by
//! [flubber](https://github.com/veltman/flubber) (MIT, vendored in `crates/reelsmith/vendor/`, run inside the
//! binary on an embedded QuickJS runtime, so there is nothing extra to install); fill colors lerp alongside in
//! Rust. Every frame re-renders the interpolated paths with resvg and streams raw RGBA to `ffmpeg` on PATH as
//! QuickTime Animation (`qtrle`) or ProRes 4444, both with alpha.
use crate::animate::{self, AlphaCodec, Easing, Fps, Width};
use crate::clips;
use anyhow::{Context, Result, bail};
use datavid::ease;
use promo::error::{ConfigError, MissingInput};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;
use rquickjs::{Context as JsContext, Runtime};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const FLUBBER: &str = include_str!("../vendor/flubber.global.js");

/// Side length of the canonical square both shape sets are fitted into before morphing.
const CANVAS: f64 = 1000.0;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MorphSvgArgs {
    /// absolute path of the SVG the morph starts from
    pub from_svg: PathBuf,
    /// absolute path of the SVG the morph lands on
    pub to_svg: PathBuf,
    /// morph length in seconds (0.2 to 10)
    pub duration_s: f64,
    /// 30 or 60 (default 60)
    #[serde(default = "animate::d_fps")]
    pub fps: Fps,
    /// canvas width: 540, 720, 1080 or 1440 px (default 1080); the canvas is square
    #[serde(default = "animate::d_width")]
    pub width: Width,
    /// timing curve of the morph: ease_out (default), ease_in_out, back or spring (morph progress is clamped to
    /// the shapes, so overshooting curves read as their in-range part)
    #[serde(default)]
    pub easing: Easing,
    /// seconds the final shape rests after the morph (default 1.0)
    pub hold_s: Option<f64>,
    /// seconds to morph back to the first shape at the end (default: none, the final shape stays)
    pub morph_back_s: Option<f64>,
    /// qtrle (default, what the promo overlays use) or prores4444
    #[serde(default)]
    pub codec: AlphaCodec,
    /// output path, a `.mov`; put it in the project's motion folder to use it as `overlay.asset` / `end_card.asset`
    pub output: PathBuf,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MorphedSvg {
    pub output: PathBuf,
    /// the asset name for a brief (`overlay.asset` / `end_card.asset`) when the file sits in the motion folder
    pub asset_name: String,
    /// a PNG (with alpha) of the mid-morph frame
    pub preview_png: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub frames: usize,
    pub duration_s: f64,
    pub codec: AlphaCodec,
    /// how many path pairs morph (one per flubber interpolator)
    pub pairs: usize,
}

fn cfg(msg: String) -> anyhow::Error {
    ConfigError(msg).into()
}

/// One drawable shape of a normalised SVG: its absolute `M L Q C Z` path data, fill color and opacity.
#[derive(Debug, Clone)]
struct Shape {
    d: String,
    fill: [f64; 3],
    opacity: f64,
}

/// The tokens of absolute path data: commands and their numbers, in order.
fn tokenize(d: &str) -> Result<Vec<&str>> {
    let toks: Vec<&str> = d.split_whitespace().collect();
    if toks.is_empty() {
        bail!("empty path data");
    }
    Ok(toks)
}

fn num(toks: &[&str], i: usize) -> Result<f64> {
    toks.get(i).context("truncated path data")?.parse::<f64>().context("bad number in path data")
}

/// Bounding box over the path's anchor and control points (control points can overshoot slightly; fine for layout).
fn bbox(d: &str) -> Result<(f64, f64, f64, f64)> {
    let toks = tokenize(d)?;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut push = |x: f64, y: f64| {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    };
    let mut i = 0;
    while i < toks.len() {
        let n = match toks[i] {
            "M" | "L" => 1,
            "Q" => 2,
            "C" => 3,
            "Z" => {
                i += 1;
                continue;
            }
            other => bail!("unexpected path command {other:?} in the normalised SVG"),
        };
        for j in 0..n {
            push(num(&toks, i + 1 + 2 * j)?, num(&toks, i + 2 + 2 * j)?);
        }
        i += 1 + 2 * n;
    }
    if !min_x.is_finite() {
        bail!("path data has no points");
    }
    Ok((min_x, min_y, max_x, max_y))
}

/// Rewrite the path data with every coordinate pair mapped by `f` (commands keep their shape).
fn remap(d: &str, f: impl Fn(f64, f64) -> (f64, f64)) -> Result<String> {
    let toks = tokenize(d)?;
    let mut out = String::with_capacity(d.len() + 8);
    let mut i = 0;
    while i < toks.len() {
        let n = match toks[i] {
            "M" | "L" => 1,
            "Q" => 2,
            "C" => 3,
            "Z" => {
                out.push_str(" Z");
                i += 1;
                continue;
            }
            other => bail!("unexpected path command {other:?} in the normalised SVG"),
        };
        out.push(' ');
        out.push_str(toks[i]);
        for j in 0..n {
            let (x, y) = f(num(&toks, i + 1 + 2 * j)?, num(&toks, i + 2 + 2 * j)?);
            out.push_str(&format!(" {x:.2} {y:.2}"));
        }
        i += 1 + 2 * n;
    }
    Ok(out.trim_start().to_string())
}

/// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa` (what usvg writes); missing fill is black, `none` is filtered earlier.
fn parse_fill(attrs: &[(String, String)]) -> Result<Option<([f64; 3], f64)>> {
    let fill = attrs.iter().find(|(k, _)| k == "fill").map(|(_, v)| v.as_str()).unwrap_or("#000000");
    if fill == "none" {
        return Ok(None);
    }
    let opacity = attrs
        .iter()
        .find(|(k, _)| k == "fill-opacity")
        .and_then(|(_, v)| v.parse::<f64>().ok())
        .unwrap_or(1.0);
    let hex = fill.strip_prefix('#').with_context(|| format!("unsupported fill {fill:?} in the normalised SVG (expected a hex color)"))?;
    let byte = |i: usize| -> Result<f64> {
        let lo = if hex.len() <= 4 { i / 2 } else { i };
        let s = if hex.len() <= 4 { &hex[lo..lo + 1] } else { &hex[lo..lo + 2] };
        let v = u8::from_str_radix(&s.repeat(if hex.len() <= 4 { 2 } else { 1 }), 16).with_context(|| format!("bad fill color {fill:?}"))?;
        Ok(v as f64)
    };
    if ![3, 4, 6, 8].contains(&hex.len()) {
        bail!("bad fill color {fill:?}");
    }
    let rgb = [byte(0)?, byte(2)?, byte(4)?];
    let a = if hex.len() == 4 || hex.len() == 8 { byte(6)? / 255.0 } else { 1.0 };
    Ok(Some((rgb, opacity * a)))
}

/// The filled paths of the normalised SVG (those after `</defs>`: clip paths, masks and patterns stay out).
fn shapes_of(tree: &usvg::Tree, which: &str) -> Result<Vec<Shape>> {
    let svg = tree.to_string(&usvg::WriteOptions::default());
    let body = svg.find("</defs>").map(|i| i + 7).unwrap_or(0);
    let mut shapes = vec![];
    let mut from = body;
    while let Some(off) = svg[from..].find("<path") {
        let start = from + off;
        let end = start + svg[start..].find('>').context("unterminated <path> tag")? + 1;
        let attrs = animate::parse_attrs(&svg[start..end])?;
        if let Some((fill, opacity)) = parse_fill(&attrs)? {
            let d = attrs.iter().find(|(k, _)| k == "d").map(|(_, v)| v.clone()).unwrap_or_default();
            if !d.is_empty() {
                shapes.push(Shape { d, fill, opacity });
            }
        }
        from = end;
    }
    if shapes.is_empty() {
        return Err(cfg(format!(
            "{which} has no filled paths to morph: morph works on shapes, so convert stroked artwork to outlines (stroke to path) first"
        )));
    }
    Ok(shapes)
}

/// Fit a set of shapes into the canonical square, preserving the set's own aspect, centred.
fn normalise(shapes: &[Shape]) -> Result<Vec<Shape>> {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for s in shapes {
        let (x0, y0, x1, y1) = bbox(&s.d)?;
        min_x = min_x.min(x0);
        min_y = min_y.min(y0);
        max_x = max_x.max(x1);
        max_y = max_y.max(y1);
    }
    let (w, h) = ((max_x - min_x).max(1e-6), (max_y - min_y).max(1e-6));
    let k = 0.9 * CANVAS / w.max(h);
    let (cx, cy) = ((min_x + max_x) / 2.0, (min_y + max_y) / 2.0);
    let map = |x: f64, y: f64| ((x - cx) * k + CANVAS / 2.0, (y - cy) * k + CANVAS / 2.0);
    shapes.iter().map(|s| Ok(Shape { d: remap(&s.d, map)?, ..s.clone() })).collect()
}

/// How the two shape sets pair up for the morph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pairing {
    /// same path count on both sides: path i morphs into path i
    Pairs,
    /// one shape into many: flubber `separate` (one interpolator per target)
    Separate,
    /// many shapes into one: flubber `combine` (one interpolator per source)
    Combine,
}

impl Pairing {
    fn choose(n_from: usize, n_to: usize) -> Result<Pairing> {
        if n_from == n_to {
            Ok(Pairing::Pairs)
        } else if n_from == 1 {
            Ok(Pairing::Separate)
        } else if n_to == 1 {
            Ok(Pairing::Combine)
        } else {
            Err(cfg(format!(
                "the two SVGs have different path counts ({n_from} vs {n_to}): morph pairs paths one to one, \
                 or splits one shape into many (or merges many into one) - simplify the artwork so the counts match, \
                 or so one side is a single path"
            )))
        }
    }

    fn mode(self) -> &'static str {
        match self {
            Pairing::Pairs => "pairs",
            Pairing::Separate => "separate",
            Pairing::Combine => "combine",
        }
    }
}

/// The (from, to) fill+opacity of every interpolator, following the pairing.
fn color_pairs(p: Pairing, from: &[Shape], to: &[Shape]) -> Vec<(Shape, Shape)> {
    let bare = |s: &Shape| Shape { d: String::new(), fill: s.fill, opacity: s.opacity };
    match p {
        Pairing::Pairs => from.iter().zip(to.iter()).map(|(a, b)| (bare(a), bare(b))).collect(),
        Pairing::Separate => to.iter().map(|b| (bare(&from[0]), bare(b))).collect(),
        Pairing::Combine => from.iter().map(|a| (bare(a), bare(&to[0]))).collect(),
    }
}

const INIT_JS: &str = r#"(function(){
  try {
    var opt = { maxSegmentLength: 2 };
    var from = JSON.parse(globalThis.__from_json);
    var to = JSON.parse(globalThis.__to_json);
    var mode = globalThis.__mode;
    var fs;
    if (mode === "pairs") { fs = from.map(function(f, i){ return flubber.interpolate(f, to[i], opt); }); }
    else if (mode === "separate") { fs = flubber.separate(from[0], to, opt); }
    else { fs = flubber.combine(from, to[0], opt); }
    globalThis.__interps = fs;
    return JSON.stringify({ ok: fs.length });
  } catch (e) {
    return JSON.stringify({ err: String((e && e.message) || e) });
  }
})()"#;

const FRAME_JS: &str = r#"(function(){
  try {
    return JSON.stringify({ ok: globalThis.__interps.map(function(f){ return f(globalThis.__t); }) });
  } catch (e) {
    return JSON.stringify({ err: String((e && e.message) || e) });
  }
})()"#;

/// The morph engine: one QuickJS runtime holding the flubber interpolators.
struct Morph {
    // drop order matters: the context must go before its runtime
    ctx: JsContext,
    #[allow(dead_code)] // kept to own the runtime the context lives on
    rt: Runtime,
    pairs: usize,
}

impl Morph {
    fn new(from: &[Shape], to: &[Shape], pairing: Pairing) -> Result<Morph> {
        let rt = Runtime::new().context("starting the embedded morph engine")?;
        let ctx = JsContext::full(&rt).context("starting the embedded morph engine")?;
        let from_json = serde_json::to_string(&from.iter().map(|s| &s.d).collect::<Vec<_>>())?;
        let to_json = serde_json::to_string(&to.iter().map(|s| &s.d).collect::<Vec<_>>())?;
        let pairs = ctx.with(|ctx| -> Result<usize> {
            ctx.eval::<(), _>(FLUBBER).map_err(|e| anyhow::anyhow!("loading the embedded morph engine: {e}"))?;
            let g = ctx.globals();
            g.set("__from_json", from_json).map_err(|e| anyhow::anyhow!("passing the source shapes: {e}"))?;
            g.set("__to_json", to_json).map_err(|e| anyhow::anyhow!("passing the target shapes: {e}"))?;
            g.set("__mode", pairing.mode()).map_err(|e| anyhow::anyhow!("passing the pairing mode: {e}"))?;
            let out: String = ctx.eval(INIT_JS).map_err(|e| anyhow::anyhow!("preparing the morph: {e}"))?;
            let v: serde_json::Value = serde_json::from_str(&out).context("reading the morph engine's answer")?;
            if let Some(err) = v["err"].as_str() {
                bail!("the shapes cannot morph: {err}");
            }
            v["ok"].as_u64().map(|n| n as usize).context("reading the morph engine's answer")
        })?;
        Ok(Morph { ctx, rt, pairs })
    }

    /// The interpolated path data of every pair at eased progress `t` (clamped 0..1).
    fn at(&self, t: f64) -> Result<Vec<String>> {
        self.ctx.with(|ctx| -> Result<Vec<String>> {
            ctx.globals().set("__t", ease::clamp(t)).map_err(|e| anyhow::anyhow!("passing the morph time: {e}"))?;
            let out: String = ctx.eval(FRAME_JS).map_err(|e| anyhow::anyhow!("running the morph: {e}"))?;
            let v: serde_json::Value = serde_json::from_str(&out).context("reading the morph engine's answer")?;
            if let Some(err) = v["err"].as_str() {
                bail!("the morph failed at t={t:.3}: {err}");
            }
            serde_json::from_value(v["ok"].clone()).context("reading the morph engine's answer")
        })
    }
}

fn hex(c: [f64; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0].round() as u8, c[1].round() as u8, c[2].round() as u8)
}

/// The frame SVG at eased progress `k`: interpolated paths with lerped fills on the canonical square.
fn frame_svg(morph: &Morph, colors: &[(Shape, Shape)], k: f64) -> Result<String> {
    let ds = morph.at(k)?;
    let mut out = String::with_capacity(256 + ds.len() * 512);
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {CANVAS:.0} {CANVAS:.0}\" width=\"{CANVAS:.0}\" height=\"{CANVAS:.0}\">"
    ));
    for (d, (a, b)) in ds.iter().zip(colors.iter()) {
        let fill = hex([
            ease::lerp(a.fill[0], b.fill[0], k),
            ease::lerp(a.fill[1], b.fill[1], k),
            ease::lerp(a.fill[2], b.fill[2], k),
        ]);
        let op = ease::lerp(a.opacity, b.opacity, k);
        out.push_str(&format!("<path d=\"{d}\" fill=\"{fill}\" fill-opacity=\"{op:.4}\"/>"));
    }
    out.push_str("</svg>");
    Ok(out)
}

pub fn morph(a: &MorphSvgArgs) -> Result<MorphedSvg> {
    clips::ensure_ffmpeg()?;
    for (which, p) in [("from_svg", &a.from_svg), ("to_svg", &a.to_svg)] {
        if !p.is_file() {
            return Err(MissingInput(format!("{which} {}", p.display())).into());
        }
    }
    let dur = a.duration_s;
    let hold = a.hold_s.unwrap_or(1.0);
    let back = a.morph_back_s.unwrap_or(0.0);
    if !(0.2..=10.0).contains(&dur) {
        return Err(cfg(format!("duration_s must be between 0.2 and 10, got {dur}")));
    }
    if !(0.0..=30.0).contains(&hold) {
        return Err(cfg(format!("hold_s must be between 0 and 30, got {hold}")));
    }
    if a.morph_back_s.is_some() && !(0.2..=10.0).contains(&back) {
        return Err(cfg(format!("morph_back_s must be between 0.2 and 10, got {back}")));
    }
    if a.output.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() != Some("mov") {
        return Err(cfg(format!("output must be a .mov file, got {}", a.output.display())));
    }

    let mut trees = Vec::new();
    for (which, p) in [("from_svg", &a.from_svg), ("to_svg", &a.to_svg)] {
        let source = std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
        let tree = usvg::Tree::from_str(&source, &animate::usvg_options(p)).map_err(|e| cfg(format!("{} is not an SVG usvg can read: {e}", p.display())))?;
        animate::check_text(tree.root(), &source)?;
        let _ = which;
        trees.push(tree);
    }
    let from = normalise(&shapes_of(&trees[0], "from_svg")?)?;
    let to = normalise(&shapes_of(&trees[1], "to_svg")?)?;
    let pairing = Pairing::choose(from.len(), to.len())?;
    let colors = color_pairs(pairing, &from, &to);
    let engine = Morph::new(&from, &to, pairing)?;

    let fps = u32::from(a.fps);
    let lay = animate::layout(a.width, usvg::Size::from_wh(CANVAS as f32, CANVAS as f32).context("canonical size")?, false, 0.12);
    let (cw, ch) = (lay.cw, lay.ch);
    let n_in = (dur * fps as f64).round() as usize;
    let n_hold = (hold * fps as f64).round() as usize;
    let n_out = (back * fps as f64).round() as usize;
    let frames = n_in + n_hold + n_out;

    if let Some(dir) = a.output.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let preview = a.output.with_extension("preview.png");
    let (size, rate) = (format!("{cw}x{ch}"), fps.to_string());
    let mut child = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", size.as_str(), "-framerate", rate.as_str(), "-i", "-"])
        .args(animate::encoder_args(a.codec))
        .arg(&a.output)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running ffmpeg (is it on PATH?)")?;
    let mut stdin = child.stdin.take().context("ffmpeg stdin")?;
    let mut canvas = Pixmap::new(cw, ch).context("allocating the canvas")?;
    let mut bytes = Vec::with_capacity((cw * ch * 4) as usize);
    let flat_opt = usvg::Options::default();
    let (cx, cy) = (cw as f64 / 2.0, ch as f64 / 2.0);
    let mid = n_in / 2;
    let mut write_err = None;

    for f in 0..frames {
        // which phase, and eased morph progress k (1 = fully the second shape; the morph-back runs it to 0)
        let k = if f < n_in {
            a.easing.at((f as f64 + 1.0) / n_in as f64)
        } else if f < n_in + n_hold || n_out == 0 {
            1.0
        } else {
            a.easing.at(1.0 - (f - n_in - n_hold + 1) as f64 / n_out as f64)
        };
        let svg = frame_svg(&engine, &colors, k)?;
        let t2 = usvg::Tree::from_str(&svg, &flat_opt).map_err(|e| anyhow::anyhow!("re-reading the morphed SVG: {e}"))?;
        let s = t2.size();
        canvas.fill(resvg::tiny_skia::Color::TRANSPARENT);
        let tf = Transform::from_scale(lay.aw as f32 / s.width(), lay.ah as f32 / s.height())
            .post_translate((cx - lay.aw / 2.0) as f32, (cy - lay.ah / 2.0) as f32);
        resvg::render(&t2, tf, &mut canvas.as_mut());
        if f == mid {
            canvas.save_png(&preview).with_context(|| format!("writing {}", preview.display()))?;
        }
        animate::straight_rgba(&canvas, &mut bytes);
        if let Err(e) = stdin.write_all(&bytes) {
            write_err = Some(e);
            break;
        }
    }
    drop(stdin);
    let out = child.wait_with_output().context("waiting for ffmpeg")?;
    if !out.status.success() || write_err.is_some() {
        bail!("ffmpeg failed encoding {}:\n{}", a.output.display(), String::from_utf8_lossy(&out.stderr));
    }
    let asset_name = a.output.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    Ok(MorphedSvg {
        output: a.output.clone(),
        asset_name,
        preview_png: preview,
        width: cw,
        height: ch,
        fps,
        frames,
        duration_s: frames as f64 / fps as f64,
        codec: a.codec,
        pairs: engine.pairs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_covers_anchors_and_controls() {
        let (x0, y0, x1, y1) = bbox("M 0 0 C 10 20 30 40 50 60 Z").unwrap();
        assert_eq!((x0, y0, x1, y1), (0.0, 0.0, 50.0, 60.0));
    }

    #[test]
    fn remap_maps_every_pair() {
        let d = remap("M 0 0 L 10 10 Z", |x, y| (x + 1.0, y * 2.0)).unwrap();
        assert_eq!(d, "M 1.00 0.00 L 11.00 20.00 Z");
    }

    #[test]
    fn pairing_rules() {
        assert_eq!(Pairing::choose(3, 3).unwrap(), Pairing::Pairs);
        assert_eq!(Pairing::choose(1, 4).unwrap(), Pairing::Separate);
        assert_eq!(Pairing::choose(4, 1).unwrap(), Pairing::Combine);
        assert!(Pairing::choose(2, 5).is_err());
    }

    #[test]
    fn fill_parsing() {
        let attrs = vec![("fill".to_string(), "#ff0080".to_string()), ("fill-opacity".to_string(), "0.5".to_string())];
        let (rgb, op) = parse_fill(&attrs).unwrap().unwrap();
        assert_eq!(rgb, [255.0, 0.0, 128.0]);
        assert_eq!(op, 0.5);
        let none = vec![("fill".to_string(), "none".to_string())];
        assert!(parse_fill(&none).unwrap().is_none());
        let short = vec![("fill".to_string(), "#f08".to_string())];
        assert_eq!(parse_fill(&short).unwrap().unwrap().0, [255.0, 0.0, 136.0]);
    }

    #[test]
    fn flubber_interpolates_in_process() {
        let from = vec![Shape { d: "M 100 100 L 900 100 L 900 900 L 100 900 Z".into(), fill: [0.0, 0.0, 0.0], opacity: 1.0 }];
        let to = vec![Shape { d: "M 500 100 L 900 500 L 500 900 L 100 500 Z".into(), fill: [0.0, 0.0, 0.0], opacity: 1.0 }];
        let m = Morph::new(&from, &to, Pairing::Pairs).unwrap();
        assert_eq!(m.pairs, 1);
        let mid = &m.at(0.5).unwrap()[0];
        assert!(mid.starts_with('M') && mid.len() > 10);
        let ends = m.at(1.0).unwrap();
        assert_eq!(ends.len(), 1);
    }
}
