//! Building blocks for fframes SVG frames: a frame with a main layer and a glow layer (blurred and screened on top by
//! `finish`), text with optional glow and scale-around-centre, the "slam" entrance used for numbers and headlines, film
//! finish (vignette, grain, flash) and a few filters. Every text drawn goes through `Svg::text`, which records the smallest
//! size drawn so QA can check the min-size rule on real frames.
use crate::ease::{clamp, decay, eout, eout5};
use std::fmt::Write;
use std::sync::atomic::{AtomicU32, Ordering};

pub type Rgb = [u8; 3];

static MIN_FONT_X10: AtomicU32 = AtomicU32::new(u32::MAX);
/// smallest font size (px) drawn by any frame rendered in this process (`None` if no text was drawn)
pub fn min_font_drawn() -> Option<f64> {
    let v = MIN_FONT_X10.load(Ordering::Relaxed);
    (v != u32::MAX).then(|| v as f64 / 10.0)
}

pub fn rgb(c: Rgb) -> String {
    format!("rgb({},{},{})", c[0], c[1], c[2])
}
pub fn mixc(a: Rgb, b: Rgb, k: f64) -> Rgb {
    [0, 1, 2].map(|i| (a[i] as f64 + (b[i] as f64 - a[i] as f64) * k).round().clamp(0.0, 255.0) as u8)
}
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
/// opacity rounded to 3 decimals (keeps the SVG text short and stable)
pub fn op(x: f64) -> f64 {
    (x.clamp(0.0, 1.0) * 1000.0).round() / 1000.0
}
pub fn pts_d(pts: &[(f64, f64)], close: bool) -> String {
    let mut d = String::with_capacity(pts.len() * 16);
    for (i, (x, y)) in pts.iter().enumerate() {
        let _ = write!(d, "{}{:.1} {:.1}", if i == 0 { "M" } else { "L" }, x, y);
    }
    if close {
        d.push('Z');
    }
    d
}

/// A font role: family + weight as installed in the media folder, and its cap height / em (to scale around the middle).
#[derive(Debug, Clone, Copy)]
pub struct Font {
    pub family: &'static str,
    pub weight: u16,
    pub cap: f64,
}

/// One frame being built.
pub struct Svg {
    pub s: String,
    pub glow: String,
}

impl Default for Svg {
    fn default() -> Self {
        Self::new()
    }
}

impl Svg {
    pub fn new() -> Self {
        Self { s: String::with_capacity(400_000), glow: String::new() }
    }
    pub fn glow_line(&mut self, pts: &[(f64, f64)], col: Rgb, a: f64, w: f64) {
        if a <= 0.003 || pts.len() < 2 {
            return;
        }
        let _ = write!(self.glow, r#"<path d="{}" fill="none" stroke="{}" stroke-opacity="{}" stroke-width="{w}" stroke-linejoin="round"/>"#, pts_d(pts, false), rgb(col), op(a));
    }
    /// text anchored at its left baseline (or `anchor` = "end"/"middle"), scaled around its left-middle; optional glow copy
    #[allow(clippy::too_many_arguments)]
    pub fn text_a(&mut self, x: f64, y: f64, s: &str, f: Font, size: f64, col: Rgb, a: f64, scale: f64, glow: f64, anchor: &str) {
        if a <= 0.003 {
            return;
        }
        MIN_FONT_X10.fetch_min((size * 10.0).round() as u32, Ordering::Relaxed);
        let cy = y - size * f.cap / 2.0;
        let tr = if (scale - 1.0).abs() > 0.004 { format!(r#" transform="translate({x:.2} {cy:.2}) scale({scale:.4}) translate({:.2} {:.2})""#, -x, -cy) } else { String::new() };
        let anc = if anchor.is_empty() { String::new() } else { format!(r#" text-anchor="{anchor}""#) };
        let el = format!(r#"<text x="{x:.1}" y="{y:.1}" font-family="{}" font-weight="{}" font-size="{size}" fill="{}"{anc}{tr}"#, f.family, f.weight, rgb(col));
        let _ = write!(self.s, r#"{el} opacity="{}">{}</text>"#, op(a), esc(s));
        if glow > 0.0 {
            let _ = write!(self.glow, r#"{el} opacity="{}">{}</text>"#, op(glow * a), esc(s));
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn text(&mut self, x: f64, y: f64, s: &str, f: Font, size: f64, col: Rgb, a: f64, scale: f64, glow: f64) {
        self.text_a(x, y, s, f, size, col, a, scale, glow, "");
    }
    /// a text with a dark halo (labels drawn over imagery)
    #[allow(clippy::too_many_arguments)]
    pub fn text_halo(&mut self, x: f64, y: f64, s: &str, f: Font, size: f64, col: Rgb, halo: Rgb, r: f64, a: f64) {
        if a <= 0.003 {
            return;
        }
        MIN_FONT_X10.fetch_min((size * 10.0).round() as u32, Ordering::Relaxed);
        let _ = write!(
            self.s,
            r#"<text x="{x:.1}" y="{y:.1}" font-family="{}" font-weight="{}" font-size="{size}" fill="{}" stroke="{}" stroke-width="{}" stroke-linejoin="round" paint-order="stroke" opacity="{}">{}</text>"#,
            f.family,
            f.weight,
            rgb(col),
            rgb(halo),
            2.0 * r,
            op(a),
            esc(s)
        );
    }

    /// The "slam" entrance: `big` = scale from 1.45 with a glow flare (numbers, headlines); otherwise a soft rise and
    /// scale from 1.12. Fades out over 0.3 s from `t1`.
    #[allow(clippy::too_many_arguments)]
    pub fn slam(&mut self, x: f64, y: f64, s: &str, f: Font, size: f64, col: Rgb, t: f64, t0: f64, t1: Option<f64>, glow: f64, big: bool) {
        if t < t0 {
            return;
        }
        let u = t - t0;
        let (mut sc, mut a) = if big { (1.0 + 0.45 * (1.0 - eout5(u / 0.28)), clamp(u / 0.09)) } else { (1.0 + 0.12 * (1.0 - eout(u / 0.35)), clamp(u / 0.18)) };
        let flare = if big { 1.0 + 1.6 * decay(t, t0, 0.18) } else { 1.0 };
        if let Some(t1) = t1 {
            let o = clamp((t - t1) / 0.3);
            a *= 1.0 - o;
            sc *= 1.0 + 0.05 * o;
        }
        if a <= 0.0 {
            return;
        }
        let dy = if big { 0.0 } else { (1.0 - eout(u / 0.35)) * 18.0 };
        let g = if glow > 0.0 { glow * flare } else if big { 0.35 * (flare - 1.0) } else { 0.0 };
        self.text(x, y + dy, s, f, size, col, a, sc, g);
    }
    /// a tighter slam (scale from 1.08) for long headline lines
    #[allow(clippy::too_many_arguments)]
    pub fn slam_tight(&mut self, x: f64, y: f64, s: &str, f: Font, size: f64, col: Rgb, t: f64, t0: f64, t1: f64, glow: f64) {
        if t < t0 {
            return;
        }
        let u = t - t0;
        let mut sc = 1.0 + 0.08 * (1.0 - eout5(u / 0.28));
        let mut a = clamp(u / 0.09);
        let flare = 1.0 + 1.6 * decay(t, t0, 0.18);
        let o = clamp((t - t1) / 0.3);
        a *= 1.0 - o;
        sc *= 1.0 + 0.03 * o;
        if a > 0.0 {
            self.text(x, y, s, f, size, col, a, sc, glow * flare);
        }
    }

    /// glow layer (blurred, screened), white flash, vignette and animated grain (`grain_{0..5}.png` in the media folder)
    pub fn finish(&mut self, w: f64, h: f64, frame: usize, flash: f64, grain: bool) {
        if !self.glow.is_empty() {
            let _ = write!(self.s, r#"<g filter="url(#glow)" style="mix-blend-mode:screen">{}</g>"#, self.glow);
        }
        if flash > 0.002 {
            let _ = write!(self.s, r#"<rect width="{w}" height="{h}" fill="white" opacity="{}" style="mix-blend-mode:screen"/>"#, op(0.55 * flash * 0.45));
        }
        let _ = write!(self.s, r#"<rect width="{w}" height="{h}" fill="url(#vig)"/>"#);
        if grain {
            let _ = write!(self.s, r#"<image href="grain_{}.png" x="0" y="0" width="{w}" height="{h}" preserveAspectRatio="none" opacity="0.13" style="mix-blend-mode:hard-light"/>"#, frame % 6);
        }
    }
}

/// `content` with its own bloom (σ 10 + 36 px, the `#bloom` preset) screened on top at `bloom_opacity`, drawn in ONE pass.
///
/// Same picture as `<g style="isolation:isolate">{content}</g><g filter="url(#bloom)" opacity="k"
/// style="mix-blend-mode:screen">{content}</g>`, but the renderer rasterises `content` once instead of twice: the filter
/// blurs it, scales the bloom's alpha by `k` (what group opacity does) and screens it over the sharp copy (`feBlend`).
/// For layers made of dozens of screened rasters (the fire timelapse, the closing embers) this halves the image draws.
/// Results match the two-group form up to 8-bit rounding.
pub fn bloom_screen(content: &str, bloom_opacity: f64, w: f64, h: f64) -> String {
    let k = op(bloom_opacity);
    let id = format!("bs{}", (k * 1e4).round() as u32);
    format!(
        r#"<filter id="{id}" filterUnits="userSpaceOnUse" x="0" y="0" width="{w}" height="{h}" color-interpolation-filters="sRGB"><feGaussianBlur in="SourceGraphic" stdDeviation="10" result="a"/><feGaussianBlur in="SourceGraphic" stdDeviation="36" result="b"/><feComposite in="a" in2="b" operator="arithmetic" k1="0" k2="1" k3="1" k4="0" result="bl"/><feComponentTransfer in="bl" result="bk"><feFuncA type="linear" slope="{k}" intercept="0"/></feComponentTransfer><feBlend in="bk" in2="SourceGraphic" mode="screen"/></filter><g filter="url(#{id})">{content}</g>"#
    )
}

/// blur radii available as `filter="url(#bN)"`
pub const BLURS: [u32; 9] = [2, 3, 4, 6, 8, 12, 16, 24, 32];
pub fn blur_id(sigma: f64) -> u32 {
    *BLURS.iter().min_by(|a, b| ((**a as f64) - sigma).abs().partial_cmp(&((**b as f64) - sigma).abs()).unwrap()).unwrap()
}

/// Shared `<defs>`: glow (two Gaussians, σ 18 + 52 px), bloom (σ 10 + 36 px), ray glow, blur presets and the vignette.
pub fn defs(w: f64, h: f64) -> String {
    let mut d = String::from("<defs>");
    let _ = write!(d, r#"<filter id="glow" filterUnits="userSpaceOnUse" x="0" y="0" width="{w}" height="{h}" color-interpolation-filters="sRGB"><feGaussianBlur in="SourceGraphic" stdDeviation="18" result="a"/><feGaussianBlur in="SourceGraphic" stdDeviation="52" result="b"/><feComposite in="a" in2="b" operator="arithmetic" k1="0" k2="0.9" k3="0.8" k4="0"/></filter>"#);
    let _ = write!(d, r#"<filter id="bloom" filterUnits="userSpaceOnUse" x="0" y="0" width="{w}" height="{h}" color-interpolation-filters="sRGB"><feGaussianBlur in="SourceGraphic" stdDeviation="10" result="a"/><feGaussianBlur in="SourceGraphic" stdDeviation="36" result="b"/><feComposite in="a" in2="b" operator="arithmetic" k1="0" k2="1" k3="1" k4="0"/></filter>"#);
    let _ = write!(d, r#"<filter id="rayglow" filterUnits="userSpaceOnUse" x="0" y="0" width="{w}" height="{h}" color-interpolation-filters="sRGB"><feGaussianBlur in="SourceGraphic" stdDeviation="12"/><feComponentTransfer><feFuncA type="linear" slope="2.2"/></feComponentTransfer></filter>"#);
    for b in BLURS {
        let _ = write!(d, r#"<filter id="b{b}" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="{b}"/></filter>"#);
    }
    // vignette: x (1 - 0.55 * clip(r - 0.45)^1.5), r = elliptical distance from the centre (corners ~1.41)
    d.push_str(r#"<radialGradient id="vig" cx="0.5" cy="0.5" r="0.7072">"#);
    for i in 0..=10 {
        let r = i as f64 / 10.0 * 1.4143;
        let dk = 0.55 * (r - 0.45).clamp(0.0, 1.0).powf(1.5);
        let _ = write!(d, r#"<stop offset="{:.3}" stop-color="black" stop-opacity="{:.4}"/>"#, i as f64 / 10.0, dk);
    }
    d.push_str("</radialGradient></defs>");
    d
}

/// left-to-right darkening behind the text column: darkness(x) = clip(1 - (x/0.54)^4) * 0.94 * k
pub fn left_shade(s: &mut String, w: f64, h: f64, k: f64) {
    if k <= 0.003 {
        return;
    }
    let mut stops = String::new();
    for i in 0..=12 {
        let f = i as f64 / 12.0 * 0.6;
        let dk = (1.0 - (f / 0.54).powi(4)).clamp(0.0, 1.0) * 0.94 * k;
        let _ = write!(stops, r#"<stop offset="{:.3}" stop-color="black" stop-opacity="{}"/>"#, f, op(dk));
    }
    let _ = write!(s, r#"<linearGradient id="lsh" x1="0" y1="0" x2="1" y2="0">{stops}</linearGradient><rect width="{w}" height="{h}" fill="url(#lsh)"/>"#);
}
