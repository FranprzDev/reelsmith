//! One SVG per frame (rasterised by fframes): the shot plate with its push-in, a light vignette, the motion-graphic
//! overlay frames, the optional word-by-word caption with its pop-in (scale + blur → sharp) and the white cut flashes.
use crate::font::FontInfo;
use crate::plan::Plan;
use datavid::ease::{clamp, eout, lerp};
use datavid::svg::{blur_id, defs, esc, op};
use std::fmt::Write;

/// Builds frames for one plan. Captions are drawn only when the plan has them on and a font is loaded.
pub struct FrameBuilder {
    pub plan: Plan,
    pub font: Option<FontInfo>,
}

impl FrameBuilder {
    pub fn new(plan: Plan) -> anyhow::Result<Self> {
        let font = if plan.captions_on && !plan.captions.is_empty() { Some(FontInfo::load(&plan.font)?) } else { None };
        Ok(Self { plan, font })
    }

    /// caption size (design px) for `text`
    pub fn caption_size(&self, text: &str) -> Option<f64> {
        self.font.as_ref().map(|f| f.fit(text, self.plan.caption_style.size, self.plan.caption_max_w))
    }

    /// smallest caption size the video draws (`None` when no caption is drawn)
    pub fn min_caption_size(&self) -> Option<f64> {
        self.font.as_ref()?;
        self.plan.captions.iter().filter_map(|c| self.caption_size(&c.text)).reduce(f64::min)
    }

    /// The frame body (inside the root `<svg>`), in design pixels.
    pub fn svg(&self, f: usize) -> String {
        let p = &self.plan;
        let (w, h) = (p.width, p.height);
        let t = f as f64 / p.fps as f64;
        let mut s = String::with_capacity(8192);
        s.push_str(&defs(w, h));
        // shot plate, zoomed around the centre (the shot is picked by frame index, the way `plate` cut it)
        let sh = p.shot_of_frame(f);
        let k = clamp((t - sh.t0) / (sh.t1 - sh.t0));
        let z = lerp(sh.z0, sh.z1, k);
        let (zw, zh) = (w * z, h * z);
        let _ = write!(s, r#"<image href="{}" x="{:.2}" y="{:.2}" width="{zw:.2}" height="{zh:.2}" preserveAspectRatio="none"/>"#, Plan::bg_name(f), (w - zw) / 2.0, (h - zh) / 2.0);
        if p.look.vignette > 0.0 {
            let _ = write!(s, r#"<rect width="{w}" height="{h}" fill="url(#vig)" opacity="{}"/>"#, op(p.look.vignette));
        }
        for (i, o, kf) in p.overlays_at(f) {
            let [_, _, cw, ch] = o.crop_box.expect("overlay crop measured by `plate`");
            let oh = o.w * ch as f64 / cw as f64;
            let _ = write!(s, r#"<image href="{}" x="{:.1}" y="{:.1}" width="{:.1}" height="{oh:.1}" preserveAspectRatio="none"/>"#, Plan::ov_name(i, kf), o.x, o.y, o.w);
        }
        if let (Some(font), Some(c)) = (&self.font, p.caption_at(t)) {
            self.caption(&mut s, font, t - c.t0, &c.text);
        }
        let fd = p.look.flash_dur.max(1e-3);
        for &ft in &p.flashes {
            let u = t - ft;
            if (0.0..fd).contains(&u) {
                let _ = write!(s, r#"<rect width="{w}" height="{h}" fill="white" opacity="{}"/>"#, op(p.look.flash_opacity * (1.0 - u / fd)));
            }
        }
        s
    }

    /// the caption `u` seconds after its entrance: pop (scale 1 + pop_scale → 1, blur → sharp), outline + drop shadow
    fn caption(&self, s: &mut String, font: &FontInfo, u: f64, text: &str) {
        let st = &self.plan.caption_style;
        let sc = 1.0 + st.pop_scale * (1.0 - eout(u / st.pop_dur.max(1e-3)));
        let a = clamp(u / 0.05 + 0.35);
        let blur = st.pop_blur * (1.0 - clamp(u / (0.75 * st.pop_dur).max(1e-3)));
        let size = font.fit(text, st.size, self.plan.caption_max_w);
        let (cx, by) = (self.plan.width / 2.0, self.plan.caption_y);
        let cy = by - size * 0.35;
        let filt = if blur > 1.5 { format!(r#" filter="url(#b{})""#, blur_id(blur)) } else { String::new() };
        let tr = format!(r#"translate({cx:.1} {cy:.1}) scale({sc:.4}) translate({:.1} {:.1})"#, -cx, -cy);
        let txt = esc(text);
        let common = format!(r#"x="{cx:.1}" font-family="{}" font-size="{size:.1}" text-anchor="middle""#, esc(&font.family));
        let sw = size * st.outline_w;
        let _ = write!(
            s,
            r#"<g transform="{tr}" opacity="{}"{filt}><text {common} y="{:.1}" fill="black" fill-opacity="{sh}" stroke="black" stroke-opacity="{sh}" stroke-width="{sw:.1}" stroke-linejoin="round">{txt}</text><text {common} y="{by:.1}" fill="{}" stroke="{}" stroke-width="{sw:.1}" stroke-linejoin="round" paint-order="stroke">{txt}</text></g>"#,
            op(a),
            by + size * st.shadow_dy,
            esc(&st.fill),
            esc(&st.outline),
            sh = op(st.shadow),
        );
    }

    /// the full `<svg>` document fframes renders
    pub fn document(&self, f: usize) -> String {
        let (w, h) = (self.plan.width, self.plan.height);
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{w}" height="{h}" viewBox="0 0 {w} {h}">{}</svg>"#, self.svg(f))
    }
}
