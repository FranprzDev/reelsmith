//! `animate_svg`: turn an SVG (a logo, an icon, a badge) into a short alpha motion graphic that the promo template uses
//! like any other motion asset (`overlay.asset`, `end_card.asset`).
//!
//! The SVG is parsed once with usvg and rasterised once with resvg at the largest size the animation reaches; every
//! frame then draws that raster with a transform and an opacity (tiny-skia, bicubic). `DrawOn` is the one preset that
//! changes the drawing itself: the SVG is normalised by usvg (text to paths, styles resolved, shapes to paths) and each
//! frame rewrites the dash of every stroked path before rendering it. Frames stream into the `ffmpeg` on PATH as raw RGBA
//! and come out as QuickTime Animation (`qtrle`, what the sample motion graphics use) or ProRes 4444, both with alpha.
use crate::clips;
use anyhow::{Context, Result, bail};
use datavid::ease;
use promo::error::{ConfigError, MissingInput};
use resvg::tiny_skia::{self, BlendMode, FilterQuality, IntSize, Pixmap, PixmapPaint, Transform};
use resvg::usvg;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// How the artwork comes in. The exit (when asked for) plays one of these backwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Animation {
    /// fades in while scaling from 85 % to 100 %
    FadeScale,
    /// fades in while rising into place
    SlideUp,
    /// grows from 20 % to full size (pair it with the `back` or `spring` easing for a bounce)
    Pop,
    /// strokes draw themselves (stroke-dashoffset), then the fills fade in; needs stroked paths in the SVG
    DrawOn,
    /// turns half a revolution while scaling and fading in
    Spin,
    /// fades in with a soft bloom, then a slow glow pulse while it holds
    Glow,
}

/// The timing curve of the entrance (the exit plays it backwards).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    /// fast start, gentle landing (cubic)
    #[default]
    EaseOut,
    /// gentle start and landing (cubic)
    EaseInOut,
    /// overshoots about 10 % and settles back (easeOutBack)
    Back,
    /// a damped spring: overshoots and wobbles once or twice before resting
    Spring,
}

/// Output frame rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub enum Fps {
    F30,
    F60,
}

impl TryFrom<u32> for Fps {
    type Error = String;
    fn try_from(v: u32) -> Result<Self, String> {
        match v {
            30 => Ok(Fps::F30),
            60 => Ok(Fps::F60),
            _ => Err(format!("fps must be 30 or 60, got {v}")),
        }
    }
}
impl From<Fps> for u32 {
    fn from(f: Fps) -> u32 {
        match f {
            Fps::F30 => 30,
            Fps::F60 => 60,
        }
    }
}
impl JsonSchema for Fps {
    fn schema_name() -> Cow<'static, str> {
        "Fps".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "integer", "enum": [30, 60], "description": "frames per second: 30 or 60" })
    }
}

/// Canvas width in pixels; the height follows the SVG's aspect plus room for the motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub enum Width {
    W540,
    W720,
    W1080,
    W1440,
}

impl Width {
    fn px(self) -> u32 {
        u32::from(self)
    }
}
impl TryFrom<u32> for Width {
    type Error = String;
    fn try_from(v: u32) -> Result<Self, String> {
        match v {
            540 => Ok(Width::W540),
            720 => Ok(Width::W720),
            1080 => Ok(Width::W1080),
            1440 => Ok(Width::W1440),
            _ => Err(format!("width must be 540, 720, 1080 or 1440, got {v}")),
        }
    }
}
impl From<Width> for u32 {
    fn from(w: Width) -> u32 {
        match w {
            Width::W540 => 540,
            Width::W720 => 720,
            Width::W1080 => 1080,
            Width::W1440 => 1440,
        }
    }
}
impl JsonSchema for Width {
    fn schema_name() -> Cow<'static, str> {
        "Width".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "integer", "enum": [540, 720, 1080, 1440], "description": "canvas width in pixels" })
    }
}

/// The alpha codec of the output `.mov`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlphaCodec {
    /// QuickTime Animation (`qtrle`, 8-bit ARGB, lossless): what the promo motion graphics use
    #[default]
    Qtrle,
    /// Apple ProRes 4444 (`prores_ks`, 10-bit + alpha): smaller, for editors such as Premiere or Resolve
    Prores4444,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnimateSvgArgs {
    /// absolute path of the SVG to animate
    pub svg_path: PathBuf,
    /// entrance preset: fade_scale, slide_up, pop, draw_on, spin or glow
    pub animation: Animation,
    /// entrance length in seconds (0.2 to 10)
    pub duration_s: f64,
    /// 30 or 60 (default 60)
    #[serde(default = "d_fps")]
    pub fps: Fps,
    /// canvas width: 540, 720, 1080 or 1440 px (default 1080)
    #[serde(default = "d_width")]
    pub width: Width,
    /// timing curve: ease_out (default), ease_in_out, back (overshoot) or spring
    #[serde(default)]
    pub easing: Easing,
    /// seconds the artwork rests after the entrance (default 1.0; glow keeps pulsing)
    pub hold_s: Option<f64>,
    /// exit preset, played as that entrance backwards (default: none, the last frame stays)
    pub exit: Option<Animation>,
    /// exit length in seconds (default: duration_s)
    pub exit_s: Option<f64>,
    /// qtrle (default, what the promo overlays use) or prores4444
    #[serde(default)]
    pub codec: AlphaCodec,
    /// output path, a `.mov`; put it in the project's motion folder to use it as `overlay.asset` / `end_card.asset`
    pub output: PathBuf,
}

fn d_fps() -> Fps {
    Fps::F60
}
fn d_width() -> Width {
    Width::W1080
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AnimatedSvg {
    pub output: PathBuf,
    /// the asset name for a brief (`overlay.asset` / `end_card.asset`) when the file sits in the motion folder
    pub asset_name: String,
    /// a PNG (with alpha) of the frame where the entrance has landed
    pub preview_png: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub frames: usize,
    pub duration_s: f64,
    pub codec: AlphaCodec,
}

fn cfg(msg: String) -> anyhow::Error {
    ConfigError(msg).into()
}

impl Easing {
    fn at(self, x: f64) -> f64 {
        let x = ease::clamp(x);
        match self {
            Easing::EaseOut => ease::eout(x),
            Easing::EaseInOut => ease::einout(x),
            Easing::Back => {
                let (c1, c3) = (1.70158, 2.70158);
                1.0 + c3 * (x - 1.0).powi(3) + c1 * (x - 1.0).powi(2)
            }
            Easing::Spring => {
                if x >= 1.0 {
                    1.0
                } else {
                    1.0 - (-6.0 * x).exp() * (3.0 * std::f64::consts::PI * x).cos()
                }
            }
        }
    }
}

/// What one frame draws: the artwork under a transform, its opacity and the glow strength.
struct Pose {
    scale: f64,
    rotate_deg: f64,
    dy: f64,
    opacity: f64,
    glow: f64,
    /// DrawOn progress (raw time 0..1 through the entrance, the easing is applied per path)
    draw: Option<f64>,
}

/// The pose of `anim` at eased progress `k` (0 = not in yet, 1 = landed; may overshoot) and raw progress `u`.
fn pose(anim: Animation, k: f64, u: f64, slide: f64) -> Pose {
    let o = ease::clamp(k);
    let base = Pose { scale: 1.0, rotate_deg: 0.0, dy: 0.0, opacity: o, glow: 0.0, draw: None };
    match anim {
        Animation::FadeScale => Pose { scale: ease::lerp(0.85, 1.0, k), ..base },
        Animation::SlideUp => Pose { dy: slide * (1.0 - k), ..base },
        Animation::Pop => Pose { scale: ease::lerp(0.2, 1.0, k).max(0.01), opacity: ease::clamp(k * 3.0), ..base },
        Animation::Spin => Pose { scale: ease::lerp(0.6, 1.0, k).max(0.01), rotate_deg: -180.0 * (1.0 - k), ..base },
        Animation::Glow => Pose { scale: ease::lerp(0.96, 1.0, k), glow: (std::f64::consts::PI * ease::clamp(u)).sin() * 0.9 + 0.35 * o, ..base },
        Animation::DrawOn => Pose { opacity: 1.0, draw: Some(u), ..base },
    }
}

/// Canvas and artwork boxes, in output pixels.
struct Layout {
    cw: u32,
    ch: u32,
    aw: f64,
    ah: f64,
    /// how far `slide_up` travels
    slide: f64,
}

fn even(x: f64) -> u32 {
    ((x / 2.0).round() as u32).max(1) * 2
}

fn layout(width: Width, size: usvg::Size, needs_rotation_room: bool) -> Layout {
    let cw = width.px() as f64;
    let aspect = size.height() as f64 / size.width() as f64;
    let pad = 0.12 * cw;
    if needs_rotation_room {
        // the artwork's diagonal must fit the square canvas at every angle
        let aw = 0.88 * cw / (1.0 + aspect * aspect).sqrt();
        return Layout { cw: even(cw), ch: even(cw), aw, ah: aw * aspect, slide: 0.0 };
    }
    let mut aw = cw - 2.0 * pad;
    // very tall artwork: keep it under 1.5x the width
    if aw * aspect > 1.5 * cw {
        aw = 1.5 * cw / aspect;
    }
    let ah = aw * aspect;
    Layout { cw: even(cw), ch: even(ah + 2.0 * pad), aw, ah, slide: (0.25 * ah).min(0.9 * pad) }
}

fn usvg_options(svg: &Path) -> usvg::Options<'static> {
    let mut opt = usvg::Options { resources_dir: svg.parent().map(Path::to_path_buf), ..Default::default() };
    opt.fontdb_mut().load_system_fonts();
    opt
}

/// Every `<text>` must have been shaped with an installed font; usvg drops text it cannot shape.
fn check_text(g: &usvg::Group, source: &str) -> Result<()> {
    fn shaped(g: &usvg::Group) -> (usize, usize) {
        let (mut total, mut ok) = (0, 0);
        for n in g.children() {
            match n {
                usvg::Node::Text(t) => {
                    total += 1;
                    ok += t.flattened().has_children() as usize;
                }
                usvg::Node::Group(c) => {
                    let (a, b) = shaped(c);
                    total += a;
                    ok += b;
                }
                _ => {}
            }
        }
        (total, ok)
    }
    let (total, ok) = shaped(g);
    if (source.contains("<text") && total == 0) || ok < total {
        bail!(cfg("the SVG has text that no installed font could shape: convert the text to paths (outline it) or install its font".into()));
    }
    Ok(())
}

fn render_tree(tree: &usvg::Tree, w: u32, h: u32) -> Result<Pixmap> {
    let mut pm = Pixmap::new(w, h).context("allocating the raster")?;
    let s = tree.size();
    resvg::render(tree, Transform::from_scale(w as f32 / s.width(), h as f32 / s.height()), &mut pm.as_mut());
    Ok(pm)
}

/// Separable box blur on premultiplied RGBA, three passes (close to a Gaussian of sigma ~ r).
fn blur(pm: &Pixmap, r: usize) -> Result<Pixmap> {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let mut a: Vec<f32> = pm.data().iter().map(|v| *v as f32).collect();
    let mut b = vec![0f32; a.len()];
    let pass = |src: &[f32], dst: &mut [f32], len: usize, lines: usize, stride: usize, step: usize| {
        let norm = 1.0 / (2 * r + 1) as f32;
        for l in 0..lines {
            let base = l * stride;
            for c in 0..4 {
                let px = |i: isize| -> f32 { if i < 0 || i >= len as isize { 0.0 } else { src[base + i as usize * step + c] } };
                let mut acc: f32 = (-(r as isize)..=r as isize).map(px).sum();
                for i in 0..len {
                    dst[base + i * step + c] = acc * norm;
                    acc += px(i as isize + r as isize + 1) - px(i as isize - r as isize);
                }
            }
        }
    };
    for _ in 0..3 {
        pass(&a, &mut b, w, h, w * 4, 4);
        pass(&b, &mut a, h, w, 4, w * 4);
    }
    let data: Vec<u8> = a.iter().map(|v| v.round().clamp(0.0, 255.0) as u8).collect();
    let out = Pixmap::from_vec(data, IntSize::from_wh(w as u32, h as u32).context("blur size")?).context("blur raster")?;
    Ok(out)
}

/// A `<path .../>` of the normalised SVG that DrawOn animates.
struct DashPath {
    /// byte range of the tag in the normalised SVG
    start: usize,
    end: usize,
    attrs: Vec<(String, String)>,
    /// total length when the path is stroked
    length: Option<f64>,
}

fn parse_attrs(tag: &str) -> Result<Vec<(String, String)>> {
    let mut out = vec![];
    let mut rest = tag.trim_start_matches("<path").trim_end_matches("/>").trim_end_matches('>');
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let eq = rest.find("=\"").with_context(|| format!("unexpected attribute syntax in {tag}"))?;
        let name = rest[..eq].trim().to_string();
        let after = &rest[eq + 2..];
        let close = after.find('"').with_context(|| format!("unterminated attribute in {tag}"))?;
        out.push((name, after[..close].to_string()));
        rest = &after[close + 1..];
    }
    Ok(out)
}

/// Length of the absolute `M L Q C Z` path data usvg writes (curves flattened into 32 segments each).
fn path_length(d: &str) -> Result<f64> {
    let toks: Vec<&str> = d.split_whitespace().collect();
    let num = |i: usize| -> Result<f64> { toks.get(i).context("truncated path data")?.parse::<f64>().context("bad number in path data") };
    let (mut i, mut len) = (0, 0.0);
    let (mut cur, mut start) = ((0.0, 0.0), (0.0, 0.0));
    let dist = |a: (f64, f64), b: (f64, f64)| ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    while i < toks.len() {
        match toks[i] {
            "M" => {
                cur = (num(i + 1)?, num(i + 2)?);
                start = cur;
                i += 3;
            }
            "L" => {
                let p = (num(i + 1)?, num(i + 2)?);
                len += dist(cur, p);
                cur = p;
                i += 3;
            }
            "Q" | "C" => {
                let n = if toks[i] == "Q" { 2 } else { 3 };
                let mut pts = vec![cur];
                for j in 0..n {
                    pts.push((num(i + 1 + 2 * j)?, num(i + 2 + 2 * j)?));
                }
                let mut prev = cur;
                for s in 1..=32 {
                    let t = s as f64 / 32.0;
                    // de Casteljau
                    let mut q = pts.clone();
                    while q.len() > 1 {
                        q = q.windows(2).map(|w| (ease::lerp(w[0].0, w[1].0, t), ease::lerp(w[0].1, w[1].1, t))).collect();
                    }
                    len += dist(prev, q[0]);
                    prev = q[0];
                }
                cur = *pts.last().unwrap_or(&cur);
                i += 1 + 2 * n;
            }
            "Z" => {
                len += dist(cur, start);
                cur = start;
                i += 1;
            }
            other => bail!("unexpected path command {other:?} in the normalised SVG"),
        }
    }
    Ok(len)
}

/// The normalised SVG and its drawable paths (those after `</defs>`: clip paths, masks and patterns stay untouched).
struct DrawOn {
    svg: String,
    paths: Vec<DashPath>,
    stroked: usize,
}

impl DrawOn {
    fn new(tree: &usvg::Tree) -> Result<DrawOn> {
        let svg = tree.to_string(&usvg::WriteOptions::default());
        let body = svg.find("</defs>").map(|i| i + 7).unwrap_or(0);
        let mut paths = vec![];
        let mut from = body;
        while let Some(off) = svg[from..].find("<path") {
            let start = from + off;
            let end = start + svg[start..].find('>').context("unterminated <path> tag")? + 1;
            let attrs = parse_attrs(&svg[start..end])?;
            let stroked = attrs.iter().any(|(k, v)| k == "stroke" && v != "none");
            let length = if stroked {
                let d = attrs.iter().find(|(k, _)| k == "d").map(|(_, v)| v.as_str()).unwrap_or("");
                Some(path_length(d)?).filter(|l| *l > 0.0)
            } else {
                None
            };
            paths.push(DashPath { start, end, attrs, length });
            from = end;
        }
        let stroked = paths.iter().filter(|p| p.length.is_some()).count();
        if stroked == 0 {
            bail!(cfg("draw_on needs stroked paths (stroke=\"...\"), and this SVG has none: pick another animation".into()));
        }
        Ok(DrawOn { svg, paths, stroked })
    }

    /// The SVG at raw entrance progress `u`: stroked path `i` draws over its own staggered window, fills fade in at the end.
    fn frame_svg(&self, u: f64, easing: Easing) -> String {
        let n = self.stroked.max(1);
        let fill = ease::eout(ease::clamp((u - 0.6) / 0.4));
        let mut out = String::with_capacity(self.svg.len() + self.paths.len() * 64);
        let mut last = 0;
        let mut si = 0;
        for p in &self.paths {
            out.push_str(&self.svg[last..p.start]);
            out.push_str("<path");
            let mut fill_opacity = None;
            for (k, v) in &p.attrs {
                match k.as_str() {
                    "stroke-dasharray" | "stroke-dashoffset" => {}
                    "fill-opacity" => fill_opacity = v.parse::<f64>().ok(),
                    _ => out.push_str(&format!(" {k}=\"{v}\"")),
                }
            }
            if let Some(l) = p.length {
                let s0 = if n > 1 { 0.25 * si as f64 / (n - 1) as f64 } else { 0.0 };
                let k = ease::clamp(easing.at((u - s0) / 0.6));
                // a hair past the length so round caps do not show a dot before the stroke starts
                out.push_str(&format!(" stroke-dasharray=\"{l:.3} {:.3}\" stroke-dashoffset=\"{:.3}\"", l + 1.0, (l + 1.0) * (1.0 - k)));
                si += 1;
            }
            out.push_str(&format!(" fill-opacity=\"{:.4}\"/>", fill_opacity.unwrap_or(1.0) * fill));
            last = p.end;
        }
        out.push_str(&self.svg[last..]);
        out
    }
}

/// Straight (non-premultiplied) RGBA bytes for ffmpeg's `rgba` input.
fn straight_rgba(pm: &Pixmap, out: &mut Vec<u8>) {
    out.clear();
    for px in pm.pixels() {
        let c = px.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
}

fn encoder_args(codec: AlphaCodec) -> Vec<&'static str> {
    match codec {
        AlphaCodec::Qtrle => vec!["-c:v", "qtrle", "-pix_fmt", "argb"],
        AlphaCodec::Prores4444 => vec!["-c:v", "prores_ks", "-profile:v", "4444", "-pix_fmt", "yuva444p10le", "-alpha_bits", "16", "-vendor", "apl0"],
    }
}

pub fn animate(a: &AnimateSvgArgs) -> Result<AnimatedSvg> {
    clips::ensure_ffmpeg()?;
    if !a.svg_path.is_file() {
        return Err(MissingInput(format!("SVG {}", a.svg_path.display())).into());
    }
    let dur = a.duration_s;
    let hold = a.hold_s.unwrap_or(1.0);
    let exit_s = if a.exit.is_some() { a.exit_s.unwrap_or(dur) } else { 0.0 };
    if !(0.2..=10.0).contains(&dur) {
        bail!(cfg(format!("duration_s must be between 0.2 and 10, got {dur}")));
    }
    if !(0.0..=30.0).contains(&hold) {
        bail!(cfg(format!("hold_s must be between 0 and 30, got {hold}")));
    }
    if a.exit.is_some() && !(0.2..=10.0).contains(&exit_s) {
        bail!(cfg(format!("exit_s must be between 0.2 and 10, got {exit_s}")));
    }
    if a.exit_s.is_some() && a.exit.is_none() {
        bail!(cfg("exit_s needs an exit animation".into()));
    }
    if a.output.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() != Some("mov") {
        bail!(cfg(format!("output must be a .mov file, got {}", a.output.display())));
    }

    let source = std::fs::read_to_string(&a.svg_path).with_context(|| format!("reading {}", a.svg_path.display()))?;
    let tree = usvg::Tree::from_str(&source, &usvg_options(&a.svg_path)).map_err(|e| cfg(format!("{} is not an SVG usvg can read: {e}", a.svg_path.display())))?;
    check_text(tree.root(), &source)?;

    let fps = u32::from(a.fps);
    let spins = a.animation == Animation::Spin || a.exit == Some(Animation::Spin);
    let lay = layout(a.width, tree.size(), spins);
    let (cw, ch) = (lay.cw, lay.ch);
    let n_in = (dur * fps as f64).round() as usize;
    let n_hold = (hold * fps as f64).round() as usize;
    let n_out = (exit_s * fps as f64).round() as usize;
    let frames = n_in + n_hold + n_out;

    // raster once, at the largest scale any pose reaches (Back / Spring overshoot ~10 %, glow pulse 2 %)
    const OVER: f64 = 1.25;
    let (rw, rh) = ((lay.aw * OVER).round().max(1.0) as u32, (lay.ah * OVER).round().max(1.0) as u32);
    let art = render_tree(&tree, rw, rh)?;
    if art.pixels().iter().all(|p| p.alpha() == 0) {
        bail!(cfg(format!("{} renders as a fully transparent image (nothing to animate)", a.svg_path.display())));
    }
    let uses_glow = a.animation == Animation::Glow || a.exit == Some(Animation::Glow);
    let glow = if uses_glow {
        let r = ((rw.max(rh) as f64) * 0.025).round().max(2.0) as usize;
        let pad = 3 * r;
        let mut padded = Pixmap::new(rw + 2 * pad as u32, rh + 2 * pad as u32).context("allocating the glow raster")?;
        padded.draw_pixmap(pad as i32, pad as i32, art.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        Some(blur(&padded, r)?)
    } else {
        None
    };
    let draw_on = if a.animation == Animation::DrawOn || a.exit == Some(Animation::DrawOn) { Some(DrawOn::new(&tree)?) } else { None };
    let flat_opt = usvg::Options::default();

    if let Some(dir) = a.output.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let preview = a.output.with_extension("preview.png");
    let mut child = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{cw}x{ch}"), "-framerate", &fps.to_string(), "-i", "-"])
        .args(encoder_args(a.codec))
        .arg(&a.output)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running ffmpeg (is it on PATH?)")?;
    let mut stdin = child.stdin.take().context("ffmpeg stdin")?;
    let mut canvas = Pixmap::new(cw, ch).context("allocating the canvas")?;
    let mut bytes = Vec::with_capacity((cw * ch * 4) as usize);
    let (cx, cy) = (cw as f64 / 2.0, ch as f64 / 2.0);
    let mut write_err = None;

    for f in 0..frames {
        let t = f as f64 / fps as f64;
        // which phase, its animation, raw progress u and eased progress k
        let (anim, u, k) = if f < n_in {
            let u = (f as f64 + 1.0) / n_in as f64;
            (a.animation, u, a.easing.at(u))
        } else if f < n_in + n_hold || a.exit.is_none() {
            (a.animation, 1.0, 1.0)
        } else {
            // the exit is its entrance backwards; its last frame is fully out
            let u = 1.0 - (f - n_in - n_hold + 1) as f64 / n_out as f64;
            (a.exit.unwrap_or(a.animation), u, a.easing.at(u))
        };
        let mut p = pose(anim, k, u, lay.slide);
        if anim == Animation::Glow && f >= n_in {
            // the hold pulse: one slow breath every 1.6 s, phase 0 where the entrance landed
            let ph = (t - dur) / 1.6 * std::f64::consts::TAU;
            let breath = 0.5 - 0.5 * ph.cos();
            p.glow = (0.35 + 0.3 * breath) * ease::clamp(k);
            p.scale *= 1.0 + 0.02 * breath;
        }
        canvas.fill(tiny_skia::Color::TRANSPARENT);
        match (p.draw, &draw_on) {
            (Some(u), Some(d)) => {
                let svg = d.frame_svg(u, a.easing);
                let t2 = usvg::Tree::from_str(&svg, &flat_opt).map_err(|e| anyhow::anyhow!("re-reading the dashed SVG: {e}"))?;
                let s = t2.size();
                let tf = Transform::from_scale(lay.aw as f32 / s.width(), lay.ah as f32 / s.height())
                    .post_translate((cx - lay.aw / 2.0) as f32, (cy - lay.ah / 2.0) as f32);
                resvg::render(&t2, tf, &mut canvas.as_mut());
            }
            _ => {
                let place = |w: f64, h: f64| {
                    // map the raster (w x h at OVER) to the art box, scaled and rotated about the canvas centre
                    let s = p.scale / OVER;
                    Transform::from_translate(-(w / 2.0) as f32, -(h / 2.0) as f32)
                        .post_scale(s as f32, s as f32)
                        .post_rotate(p.rotate_deg as f32)
                        .post_translate(cx as f32, (cy + p.dy) as f32)
                };
                if let (Some(g), true) = (&glow, p.glow > 0.001) {
                    let paint = PixmapPaint { opacity: (p.glow.min(1.0) * p.opacity) as f32, blend_mode: BlendMode::Plus, quality: FilterQuality::Bicubic };
                    canvas.draw_pixmap(0, 0, g.as_ref(), &paint, place(g.width() as f64, g.height() as f64), None);
                }
                let paint = PixmapPaint { opacity: p.opacity.clamp(0.0, 1.0) as f32, blend_mode: BlendMode::SourceOver, quality: FilterQuality::Bicubic };
                canvas.draw_pixmap(0, 0, art.as_ref(), &paint, place(rw as f64, rh as f64), None);
            }
        }
        if f + 1 == n_in.max(1) {
            canvas.save_png(&preview).with_context(|| format!("writing {}", preview.display()))?;
        }
        straight_rgba(&canvas, &mut bytes);
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
    Ok(AnimatedSvg { output: a.output.clone(), asset_name, preview_png: preview, width: cw, height: ch, fps, frames, duration_s: frames as f64 / fps as f64, codec: a.codec })
}
