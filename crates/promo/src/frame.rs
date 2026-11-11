//! One SVG per frame (rasterised by fframes): the shot plate with its push-in, a light vignette, the motion-graphic
//! overlay frames, the optional word-by-word caption with its pop-in (scale + blur → sharp) and the white cut flashes.
use crate::font::FontInfo;
use crate::plan::Plan;
use datavid::ease::{clamp, eout, lerp};
use datavid::svg::{blur_id, defs, esc, op};
use std::fmt::Write;
use std::sync::Mutex;

struct CachedDefs {
    w: u64,
    h: u64,
    body: String,
}

/// Builds frames for one plan. Captions are drawn only when the plan has them on and a font is loaded.
pub struct FrameBuilder {
    pub plan: Plan,
    pub font: Option<FontInfo>,
    cached_defs: Mutex<Option<CachedDefs>>,
}

impl FrameBuilder {
    pub fn new(plan: Plan) -> anyhow::Result<Self> {
        let font = if plan.captions_on && !plan.captions.is_empty() { Some(FontInfo::load(&plan.font)?) } else { None };
        Ok(Self { plan, font, cached_defs: Mutex::new(None) })
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

    fn write_defs(&self, s: &mut String, w: f64, h: f64) {
        let (wb, hb) = (w.to_bits(), h.to_bits());
        let mut cache = self.cached_defs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = cache.as_ref() {
            if c.w == wb && c.h == hb {
                s.push_str(&c.body);
                return;
            }
        }
        let body = defs(w, h);
        s.push_str(&body);
        *cache = Some(CachedDefs { w: wb, h: hb, body });
    }

    fn write_svg(&self, s: &mut String, f: usize) {
        let p = &self.plan;
        let (w, h) = (p.width, p.height);
        let t = f as f64 / p.fps as f64;
        self.write_defs(s, w, h);
        // shot plate, zoomed around the centre (the shot is picked by frame index, the way `plate` cut it)
        let sh = p.shot_of_frame(f);
        let k = clamp((t - sh.t0) / (sh.t1 - sh.t0));
        let z = lerp(sh.z0, sh.z1, k);
        let (zw, zh) = (w * z, h * z);
        let _ = write!(s, r#"<image href="{}" x="{:.2}" y="{:.2}" width="{zw:.2}" height="{zh:.2}" preserveAspectRatio="none"/>"#, Plan::bg_name(f), (w - zw) / 2.0, (h - zh) / 2.0);
        if p.look.vignette > 0.0 {
            let _ = write!(s, r#"<rect width="{w}" height="{h}" fill="url(#vig)" opacity="{}"/>"#, op(p.look.vignette));
        }
        for (i, o) in p.overlays.iter().enumerate() {
            let first = p.fi(o.t0);
            if f < first || f >= p.fi(o.t1) {
                continue;
            }
            let kf = f - first;
            let [_, _, cw, ch] = o.crop_box.expect("overlay crop measured by `plate`");
            let oh = o.w * ch as f64 / cw as f64;
            let _ = write!(s, r#"<image href="{}" x="{:.1}" y="{:.1}" width="{:.1}" height="{oh:.1}" preserveAspectRatio="none"/>"#, Plan::ov_name(i, kf), o.x, o.y, o.w);
        }
        if let (Some(font), Some(c)) = (&self.font, p.caption_at(t)) {
            self.caption(s, font, t - c.t0, &c.text);
        }
        let fd = p.look.flash_dur.max(1e-3);
        for &ft in &p.flashes {
            let u = t - ft;
            if (0.0..fd).contains(&u) {
                let _ = write!(s, r#"<rect width="{w}" height="{h}" fill="white" opacity="{}"/>"#, op(p.look.flash_opacity * (1.0 - u / fd)));
            }
        }
    }

    /// The frame body (inside the root `<svg>`), in design pixels.
    pub fn svg(&self, f: usize) -> String {
        let mut s = String::with_capacity(8192);
        self.write_svg(&mut s, f);
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
        let mut s = String::with_capacity(8192 + 128);
        let _ = write!(s, r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{w}" height="{h}" viewBox="0 0 {w} {h}">"#);
        self.write_svg(&mut s, f);
        s.push_str("</svg>");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PromoFile;
    use crate::plan::Overrides;

    fn test_plan(aspect: &str) -> Plan {
        let dir = std::env::temp_dir().join(format!("reelsmith-frame-test-{}-{}", std::process::id(), aspect.replace(':', "_")));
        std::fs::create_dir_all(&dir).ok();
        let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fonts/PixelifySans-Bold.ttf");
        let font = font.to_string_lossy().replace('\\', "/");
        let cfg = format!(
            r#"
[video]
name = "frame-parity"
aspect = "{aspect}"
flashes = [0.0, 0.5, 1.0]

[[shot]]
clip = "clip-a"
dur = 0.6

[[shot]]
clip = "clip-b"
dur = 0.6

[[overlay]]
asset = "overlay"
t0 = 0.1
t1 = 1.1
crop = [0, 0, 100, 50]
x = 10.0
y = 20.0
w = 540.0

[look]
vignette = 0.42
flash_opacity = 0.63
flash_dur = 0.25

[captions]
enabled = true
font = "{font}"
words = [[0.0, "HELLO"], [0.45, "WORLD WIDE"], [1.0, "TAIL"]]
end = 1.2
"#
        );
        let f = PromoFile::from_toml_str(&cfg, &dir).unwrap();
        Plan::resolve(f, &dir.join("frame.toml"), &Overrides::default()).unwrap()
    }

    fn reference_svg(fb: &FrameBuilder, f: usize) -> String {
        let p = &fb.plan;
        let (w, h) = (p.width, p.height);
        let t = f as f64 / p.fps as f64;
        let mut s = String::with_capacity(8192);
        s.push_str(&defs(w, h));
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
        if let (Some(font), Some(c)) = (&fb.font, p.caption_at(t)) {
            fb.caption(&mut s, font, t - c.t0, &c.text);
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

    fn reference_document(fb: &FrameBuilder, f: usize) -> String {
        let (w, h) = (fb.plan.width, fb.plan.height);
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{w}" height="{h}" viewBox="0 0 {w} {h}">{}</svg>"#, reference_svg(fb, f))
    }

    fn push_around(frames: &mut Vec<usize>, center: usize) {
        for d in -1isize..=1 {
            let f = center as isize + d;
            if f >= 0 {
                frames.push(f as usize);
            }
        }
    }

    fn boundary_frames(fb: &FrameBuilder) -> Vec<usize> {
        let p = &fb.plan;
        let n = p.frames();
        let mut frames = vec![0, 1, n.saturating_sub(2), n.saturating_sub(1)];
        for s in &p.shots {
            push_around(&mut frames, p.fi(s.t0));
            push_around(&mut frames, p.fi(s.t1));
        }
        for o in &p.overlays {
            push_around(&mut frames, p.fi(o.t0));
            push_around(&mut frames, p.fi(o.t1));
        }
        for c in &p.captions {
            push_around(&mut frames, p.fi(c.t0));
            push_around(&mut frames, p.fi(c.t1));
        }
        for &t in &p.flashes {
            push_around(&mut frames, p.fi(t));
        }
        frames.retain(|&f| f < n);
        frames.sort_unstable();
        frames.dedup();
        frames
    }

    fn assert_matches_reference(fb: &FrameBuilder, frames: &[usize]) {
        for &f in frames {
            assert_eq!(fb.svg(f), reference_svg(fb, f), "svg frame {f}");
            assert_eq!(fb.document(f), reference_document(fb, f), "document frame {f}");
        }
    }

    #[test]
    fn svg_bytes_match_reference_across_aspects_and_boundaries() {
        for aspect in ["9:16", "16:9", "1:1"] {
            let fb = FrameBuilder::new(test_plan(aspect)).unwrap();
            assert_matches_reference(&fb, &boundary_frames(&fb));
        }
    }

    #[test]
    fn svg_bytes_match_reference_after_plan_mutations() {
        let mut fb = FrameBuilder::new(test_plan("9:16")).unwrap();
        assert_matches_reference(&fb, &boundary_frames(&fb));

        fb.plan.width = 720.5;
        fb.plan.height = 1280.25;
        fb.plan.look.vignette = 0.0;
        fb.plan.look.flash_opacity = 0.1234;
        fb.plan.look.flash_dur = 0.71;
        fb.plan.caption_style.pop_scale = 0.9;
        fb.plan.caption_max_w *= 0.5;
        assert_matches_reference(&fb, &boundary_frames(&fb));

        fb.plan.width = 1080.0;
        fb.plan.height = 1920.0;
        fb.plan.look.vignette = 0.85;
        assert_matches_reference(&fb, &boundary_frames(&fb));
    }

    #[test]
    #[ignore = "benchmark: cargo test -p promo --lib frame::bench_full_svg_10k -- --ignored --nocapture"]
    fn bench_full_svg_10k_reference_vs_new() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        use std::hint::black_box;
        use std::time::Instant;

        let fb = FrameBuilder::new(test_plan("9:16")).unwrap();
        let n = fb.plan.frames();
        let hash = |document: fn(&FrameBuilder, usize) -> String, label: &str| {
            let start = Instant::now();
            let mut h = DefaultHasher::new();
            for i in 0..10_000 {
                let s = document(&fb, i % n);
                h.write(s.as_bytes());
                let _ = black_box(&s);
            }
            eprintln!("{label}: {:?}", start.elapsed());
            h.finish()
        };
        let reference = hash(reference_document, "reference");
        let new = hash(FrameBuilder::document, "new");
        assert_eq!(reference, new);
    }
}
