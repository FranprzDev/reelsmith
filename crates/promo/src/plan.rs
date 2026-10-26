//! The resolved timeline: every shot, overlay, caption, flash and cue with absolute times and absolute paths. A config
//! (beats or explicit) is resolved once into a [`Plan`]; every stage (plate, audio, render, qa) reads only the plan, and
//! the render chunks read it back from `build/plan.json`.
use crate::config::*;
use crate::error::{ConfigError, MissingInput};
use crate::script::{self, Caption};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub const CLIP_EXTS: &[&str] = &["mp4", "mov", "mkv", "webm", "m4v"];
pub const MOTION_EXTS: &[&str] = &["mov", "webm", "mp4", "mkv"];
/// time comparisons (s)
const EPS: f64 = 1e-6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shot {
    pub t0: f64,
    pub t1: f64,
    pub clip: PathBuf,
    pub src_in: f64,
    pub z0: f64,
    pub z1: f64,
    pub blur: f64,
    pub focus: [f64; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Overlay {
    pub asset: PathBuf,
    pub t0: f64,
    pub t1: f64,
    pub crop: Crop,
    /// the content box in asset pixels, once known (explicit, or measured by `plate`)
    pub crop_box: Option<[u32; 4]>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub sfx: f64,
}

impl Overlay {
    pub fn label(&self) -> String {
        self.asset.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }
    /// cache key of a measured crop
    pub fn crop_key(&self) -> String {
        format!("{}|{:?}", self.asset.display(), self.crop)
    }
}

/// Overrides from the command line (or an upload app) that win over the config.
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub engine: Option<Engine>,
    pub clips: Option<PathBuf>,
    pub motion: Option<PathBuf>,
    pub out_dir: Option<PathBuf>,
    pub build_dir: Option<PathBuf>,
    pub captions: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    #[serde(default)]
    pub engine: Engine,
    pub name: String,
    pub config: PathBuf,
    pub aspect: Aspect,
    pub fps: usize,
    /// design canvas (px)
    pub width: f64,
    pub height: f64,
    pub seconds: f64,
    pub clips_dir: PathBuf,
    pub motion_dir: PathBuf,
    pub out_dir: PathBuf,
    pub build_dir: PathBuf,
    pub look: LookCfg,
    pub shots: Vec<Shot>,
    pub overlays: Vec<Overlay>,
    pub flashes: Vec<f64>,
    pub cues: Vec<CueCfg>,
    pub captions: Vec<Caption>,
    pub captions_on: bool,
    pub caption_style: CaptionsCfg,
    pub font: PathBuf,
    pub caption_y: f64,
    pub caption_max_w: f64,
    pub music: MusicCfg,
    pub music_drop: f64,
    pub music_outro: f64,
    pub voiceover: Option<VoiceoverCfg>,
    pub audio: AudioCfg,
    pub qa: QaCfg,
}

/// The default caption font (Pixelify Sans Bold, OFL), embedded in the binary so a release build needs no repo checkout.
const DEFAULT_FONT: &[u8] = include_bytes!("../../../fonts/PixelifySans-Bold.ttf");

/// Path of the default caption font, written to the system temp folder on first use (fframes reads fonts from files).
/// A failed write leaves the file missing, which `missing_inputs` reports when captions are on.
pub fn default_font() -> PathBuf {
    let path = std::env::temp_dir().join("reelsmith").join("PixelifySans-Bold.ttf");
    if !path.is_file() && std::fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).is_ok() {
        if let Err(e) = std::fs::write(&path, DEFAULT_FONT) {
            eprintln!("warning: writing the default caption font to {}: {e}", path.display());
        }
    }
    path
}

fn find(dir: &Path, name: &str, exts: &[&str]) -> PathBuf {
    let p = Path::new(name);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    let direct = dir.join(name);
    if direct.is_file() {
        return direct;
    }
    for e in exts {
        let c = dir.join(format!("{name}.{e}"));
        if c.is_file() {
            return c;
        }
    }
    dir.join(format!("{name}.{}", exts[0]))
}

fn cerr(s: String) -> ConfigError {
    ConfigError(s)
}

impl Plan {
    /// Load a config file (TOML or JSON, `extends` resolved) and resolve it.
    pub fn from_file(path: &Path, ov: &Overrides) -> anyhow::Result<Self> {
        let path = std::fs::canonicalize(path).map_err(|e| MissingInput(format!("config {}: {e}", path.display())))?;
        let f = PromoFile::load(&path)?;
        Ok(Self::resolve(f, &path, ov)?)
    }

    pub fn resolve(f: PromoFile, config: &Path, ov: &Overrides) -> Result<Self, ConfigError> {
        let dir = config.parent().unwrap_or(Path::new(".")).to_path_buf();
        let abs = |s: &str| {
            let p = PathBuf::from(s);
            if p.is_absolute() { p } else { dir.join(p) }
        };
        if f.video.fps != FPS {
            return Err(cerr(format!("video.fps = {}: only {FPS} fps is supported for now", f.video.fps)));
        }
        if f.video.name.trim().is_empty() || f.video.name.contains(['/', '\\']) {
            return Err(cerr("video.name must be a plain file stem".into()));
        }
        let (cw, ch) = f.video.aspect.canvas();
        let (w, h) = (cw as f64, ch as f64);
        let clips_dir = ov.clips.clone().unwrap_or_else(|| abs(&f.paths.clips));
        let motion_dir = ov.motion.clone().unwrap_or_else(|| abs(&f.paths.motion));
        let out_dir = ov.out_dir.clone().unwrap_or_else(|| abs(&f.paths.out));
        let build_dir = ov.build_dir.clone().unwrap_or_else(|| f.paths.build.as_deref().map(abs).unwrap_or_else(|| dir.join("build").join(&f.video.name)));
        let snap = |t: f64| (t * FPS as f64).round() / FPS as f64;

        let mk_shot = |s: &ShotCfg, t0: f64, t1: f64, zoom: [f64; 2]| -> Result<Shot, ConfigError> {
            let clip = match &s.clip {
                Some(c) => find(&clips_dir, c, CLIP_EXTS),
                None => return Err(cerr(format!("the shot at {t0:.2} s names no `clip`"))),
            };
            let z = s.zoom.unwrap_or(zoom);
            Ok(Shot { t0, t1, clip, src_in: s.src_in, z0: z[0], z1: z[1], blur: s.blur, focus: s.focus })
        };
        let mk_overlay = |asset: &str, t0: f64, t1: f64, crop: &Option<Crop>, x: Option<f64>, y: Option<f64>, ow: Option<f64>, sfx: f64, full: bool| -> Overlay {
            let crop = crop.clone().unwrap_or(Crop::Mode(if full { CropMode::Full } else { CropMode::Auto }));
            let crop_box = match &crop {
                Crop::Box(b) => Some(*b),
                Crop::Mode(_) => None,
            };
            let ow = ow.unwrap_or(if full { w } else { 0.8 * w });
            Overlay { asset: find(&motion_dir, asset, MOTION_EXTS), t0, t1, crop, crop_box, x: x.unwrap_or((w - ow) / 2.0), y: y.unwrap_or(if full { 0.0 } else { 0.12 * h }), w: ow, sfx }
        };

        let mut shots = vec![];
        let mut overlays = vec![];
        let mut flashes = f.video.flashes.clone();
        let mut cues = f.cue.clone();
        let mut beat_caps: Vec<Caption> = vec![];
        let mut first_beat_end = None;
        let mut t = 0.0;
        if !f.shot.is_empty() && !f.beat.is_empty() {
            return Err(cerr("use either [[beat]] (texts in) or [[shot]] (explicit times), not both".into()));
        }
        for s in &f.shot {
            let t0 = s.t0.unwrap_or(t);
            let t1 = match (s.t1, s.dur) {
                (Some(t1), _) => t1,
                (None, Some(d)) => t0 + d,
                _ => return Err(cerr(format!("shot at {t0:.2} s needs `t1` or `dur`"))),
            };
            shots.push(mk_shot(s, t0, t1, [1.0, 1.10])?);
            t = t1;
        }
        for (i, b) in f.beat.iter().enumerate() {
            let gs = script::groups(&b.text);
            let lead = if i == 0 && !gs.is_empty() { f.captions.lead_in } else { 0.0 };
            let auto = gs.iter().map(|g| script::group_time(g)).sum::<f64>().max(1.2) + lead;
            let dur = snap(b.dur.unwrap_or(auto));
            let mut refs: Vec<ShotCfg> = b.clip.iter().chain(b.clips.iter()).map(ClipRef::to_shot).collect();
            if refs.is_empty() {
                return Err(cerr(format!("beat {} («{}») has no clip", i + 1, b.text)));
            }
            let fixed: f64 = refs.iter().filter_map(|r| r.dur).sum();
            let free = refs.iter().filter(|r| r.dur.is_none()).count();
            if fixed > dur + EPS || (free == 0 && (fixed - dur).abs() > 1e-3) {
                return Err(cerr(format!("beat {}: its clips last {fixed:.2} s but the beat lasts {dur:.2} s", i + 1)));
            }
            let share = if free > 0 { (dur - fixed) / free as f64 } else { 0.0 };
            let mut c = t;
            let n = refs.len();
            for (j, r) in refs.iter_mut().enumerate() {
                let d = r.dur.unwrap_or(share);
                let c1 = if j + 1 == n { t + dur } else { snap(c + d) };
                let zoom = if j % 2 == 0 { [1.0, 1.10] } else { [1.10, 1.0] };
                shots.push(mk_shot(r, c, c1, zoom)?);
                c = c1;
            }
            if let Some(o) = &b.overlay {
                let o = o.to_beat_overlay();
                let o0 = t + o.delay;
                let o1 = o.dur.map(|d| o0 + d).unwrap_or(t + dur).min(t + dur);
                overlays.push(mk_overlay(&o.asset, o0, o1, &o.crop, o.x, o.y, o.w, o.sfx, false));
            }
            if b.flash && t > EPS {
                flashes.push(t);
            }
            if let Some(k) = b.cue {
                cues.push(CueCfg { t, kind: k, gain: 0.22 });
            }
            beat_caps.extend(script::spread(&gs, t + lead, t + dur));
            t = snap(t + dur);
            if i == 0 {
                first_beat_end = Some(t);
            }
        }
        let mut end_card_t0 = None;
        if let Some(e) = &f.end_card {
            let base = e.clip.to_shot();
            let z = base.zoom.unwrap_or([1.0, 1.08]);
            let zm = z[0] + (z[1] - z[0]) * (e.sharp / e.dur).clamp(0.0, 1.0);
            let t1 = snap(t + e.dur);
            let ts = snap(t + e.sharp);
            shots.push(mk_shot(&ShotCfg { blur: 0.0, ..base.clone() }, t, ts, [z[0], zm])?);
            shots.push(mk_shot(&ShotCfg { blur: e.blur, src_in: base.src_in + e.sharp, ..base.clone() }, ts, t1, [zm, z[1]])?);
            if let Some(a) = &e.asset {
                overlays.push(mk_overlay(a, ts, t1, &e.crop, e.x, e.y, e.w, e.sfx, true));
            }
            if e.flash && t > EPS {
                flashes.push(t);
            }
            beat_caps.extend(script::spread(&script::groups(&e.text), t, t + e.dur.min(1.3)));
            end_card_t0 = Some(t);
            t = t1;
        }
        for o in &f.overlay {
            let t1 = match (o.t1, o.dur) {
                (Some(t1), _) => t1,
                (None, Some(d)) => o.t0 + d,
                _ => return Err(cerr(format!("overlay {} needs `t1` or `dur`", o.asset))),
            };
            overlays.push(mk_overlay(&o.asset, o.t0, t1, &o.crop, o.x, o.y, o.w, o.sfx, false));
        }
        if shots.is_empty() {
            return Err(cerr("no shots: add [[beat]] entries (or [[shot]])".into()));
        }
        let seconds = f.video.seconds.unwrap_or(t);
        flashes.sort_by(|a, b| a.partial_cmp(b).unwrap());
        flashes.dedup_by(|a, b| (*a - *b).abs() < EPS);

        // captions: voice-over script > captions script > explicit words > the beats' texts
        let cap_end = f.captions.end.unwrap_or(seconds);
        let read = |p: &str| std::fs::read_to_string(abs(p)).map_err(|e| ConfigError(format!("caption script {p}: {e}")));
        let mut captions = if let Some((s, off)) = f.voiceover.as_ref().and_then(|v| v.script.as_ref().map(|s| (s, v.offset))) {
            script::parse_script(&read(s)?, s.ends_with(".srt"), cap_end - off, off)?
        } else if let Some(s) = &f.captions.script {
            script::parse_script(&read(s)?, s.ends_with(".srt"), cap_end, 0.0)?
        } else if !f.captions.words.is_empty() {
            script::from_words(&f.captions.words, cap_end)
        } else {
            beat_caps
        };
        if f.captions.uppercase {
            captions.iter_mut().for_each(|c| c.text = c.text.to_uppercase());
        }
        let font = f.captions.font.as_deref().map(abs).unwrap_or_else(default_font);
        let music_drop = f.music.drop.or_else(|| flashes.iter().copied().find(|x| *x > EPS)).or(first_beat_end).unwrap_or(shots[0].t1);
        let music_outro = f.music.outro.or(end_card_t0).or_else(|| flashes.last().copied()).unwrap_or(seconds);
        let voiceover = f.voiceover.clone().map(|mut v| {
            v.file = abs(&v.file).display().to_string();
            v
        });
        let mut music = f.music.clone();
        music.file = music.file.as_deref().map(|p| abs(p).display().to_string());
        let mut caption_style = f.captions.clone();
        caption_style.words.clear();
        let plan = Plan {
            engine: ov.engine.unwrap_or(f.render.engine),
            name: f.video.name.clone(),
            config: config.to_path_buf(),
            aspect: f.video.aspect,
            fps: f.video.fps,
            width: w,
            height: h,
            seconds,
            clips_dir,
            motion_dir,
            out_dir,
            build_dir,
            look: f.look.clone(),
            shots,
            overlays,
            flashes,
            cues,
            captions,
            captions_on: ov.captions.unwrap_or(f.captions.enabled),
            caption_y: f.captions.y.unwrap_or(0.672 * h),
            caption_max_w: f.captions.max_w.unwrap_or(0.889 * w),
            caption_style,
            font,
            music,
            music_drop,
            music_outro,
            voiceover,
            audio: f.audio.clone(),
            qa: f.qa.clone(),
        };
        plan.validate()?;
        Ok(plan)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let mut t = 0.0;
        for s in &self.shots {
            let name = s.clip.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if (s.t0 - t).abs() > 1e-4 {
                return Err(cerr(format!("shot {name} starts at {:.3} s but the previous one ends at {t:.3} s (gap or overlap)", s.t0)));
            }
            if s.t1 <= s.t0 + EPS {
                return Err(cerr(format!("shot {name} at {:.2} s has no length", s.t0)));
            }
            if s.src_in < 0.0 || s.blur < 0.0 || s.z0 <= 0.0 || s.z1 <= 0.0 {
                return Err(cerr(format!("shot {name}: `in`, `blur` must be ≥ 0 and `zoom` > 0")));
            }
            t = s.t1;
        }
        if (t - self.seconds).abs() > 1e-4 {
            return Err(cerr(format!("the shots end at {t:.3} s but the video lasts {:.3} s", self.seconds)));
        }
        if self.seconds > 180.0 {
            return Err(cerr(format!("{:.1} s is longer than a short-form promo (max 180 s)", self.seconds)));
        }
        for o in &self.overlays {
            if o.t1 <= o.t0 + EPS || o.t0 < -EPS || o.t1 > self.seconds + 1e-4 {
                return Err(cerr(format!("overlay {} runs {:.2}–{:.2} s, outside the video (0–{:.2} s) or empty", o.label(), o.t0, o.t1, self.seconds)));
            }
            if let Some([_, _, cw, ch]) = o.crop_box {
                if cw == 0 || ch == 0 {
                    return Err(cerr(format!("overlay {}: empty crop", o.label())));
                }
            }
            if o.w <= 0.0 {
                return Err(cerr(format!("overlay {}: w must be > 0", o.label())));
            }
        }
        for &f in &self.flashes {
            if !(0.0..self.seconds).contains(&f) {
                return Err(cerr(format!("flash at {f:.2} s is outside the video")));
            }
        }
        for c in &self.cues {
            if !(0.0..self.seconds).contains(&c.t) {
                return Err(cerr(format!("cue at {:.2} s is outside the video", c.t)));
            }
        }
        for p in self.captions.windows(2) {
            if p[1].t0 < p[0].t0 {
                return Err(cerr(format!("captions must be in time order («{}» at {:.2} s)", p[1].text, p[1].t0)));
            }
        }
        if self.music.kind == MusicKind::File && self.music.file.is_none() {
            return Err(cerr("music.kind = \"file\" needs music.file".into()));
        }
        Ok(())
    }

    /// Input files that do not exist yet.
    pub fn missing_inputs(&self) -> Vec<String> {
        let mut v = vec![];
        for s in &self.shots {
            if !s.clip.is_file() {
                v.push(format!("clip {}", s.clip.display()));
            }
        }
        for o in &self.overlays {
            if !o.asset.is_file() {
                v.push(format!("motion asset {}", o.asset.display()));
            }
        }
        if self.captions_on && !self.font.is_file() {
            v.push(format!("caption font {}", self.font.display()));
        }
        if let Some(vo) = &self.voiceover {
            if !Path::new(&vo.file).is_file() {
                v.push(format!("voice-over {}", vo.file));
            }
        }
        if let (MusicKind::File, Some(m)) = (self.music.kind, &self.music.file) {
            if !Path::new(m).is_file() {
                v.push(format!("music {m}"));
            }
        }
        v.sort();
        v.dedup();
        v
    }

    pub fn ensure_inputs(&self) -> Result<(), MissingInput> {
        let m = self.missing_inputs();
        if m.is_empty() { Ok(()) } else { Err(MissingInput(m.join("; "))) }
    }

    pub fn fi(&self, t: f64) -> usize {
        (t * self.fps as f64).round() as usize
    }
    pub fn frames(&self) -> usize {
        self.fi(self.seconds)
    }
    /// output size (even pixels)
    pub fn out_size(&self, r: Res) -> (u32, u32) {
        let ev = |v: f64| ((v * r.scale() / 2.0).round() * 2.0) as u32;
        (ev(self.width), ev(self.height))
    }
    pub fn default_out(&self, r: Res) -> PathBuf {
        let (w, h) = self.out_size(r);
        let cap = if self.captions_on { "_captions" } else { "" };
        self.out_dir.join(format!("{}_{}_{w}x{h}{cap}.mp4", self.name, r.tag()))
    }
    pub fn frames_dir(&self, r: Res) -> PathBuf {
        self.build_dir.join(format!("frames_{}", r.tag()))
    }
    pub fn plan_json(&self) -> PathBuf {
        self.build_dir.join("plan.json")
    }
    pub fn crops_json(&self) -> PathBuf {
        self.build_dir.join("crops.json")
    }
    pub fn audio_wav(&self) -> PathBuf {
        self.build_dir.join("audio.wav")
    }
    pub fn qa_dir(&self) -> PathBuf {
        self.build_dir.join("qa")
    }
    pub fn shot_at(&self, t: f64) -> &Shot {
        self.shots.iter().find(|s| t >= s.t0 - EPS && t < s.t1 - EPS).unwrap_or_else(|| self.shots.last().unwrap())
    }
    /// shot covering frame `f` (by frame index, the way the plates are cut)
    pub fn shot_of_frame(&self, f: usize) -> &Shot {
        self.shots.iter().find(|s| f >= self.fi(s.t0) && f < self.fi(s.t1)).unwrap_or_else(|| self.shots.last().unwrap())
    }
    pub fn bg_name(f: usize) -> String {
        format!("bg_{f:05}.jpg")
    }
    pub fn ov_name(i: usize, k: usize) -> String {
        format!("ov_{i:02}_{k:04}.png")
    }
    /// overlays drawn at frame `f`: (index, overlay, its local frame)
    pub fn overlays_at(&self, f: usize) -> Vec<(usize, &Overlay, usize)> {
        self.overlays.iter().enumerate().filter_map(|(i, o)| (f >= self.fi(o.t0) && f < self.fi(o.t1)).then(|| (i, o, f - self.fi(o.t0)))).collect()
    }
    /// every raster frame `f` draws (the media subset of a render chunk)
    pub fn rasters_at(&self, f: usize) -> Vec<String> {
        let mut v = vec![Self::bg_name(f)];
        v.extend(self.overlays_at(f).into_iter().map(|(i, _, k)| Self::ov_name(i, k)));
        v
    }
    /// the caption shown at `t`, if any
    pub fn caption_at(&self, t: f64) -> Option<&Caption> {
        self.captions.iter().rev().find(|c| c.t0 <= t + 1e-9 && t < c.t1)
    }

    /// Fill the measured crops (`build/crops.json`, written by `plate`).
    pub fn load_crops(&mut self) -> anyhow::Result<()> {
        let m: BTreeMap<String, [u32; 4]> = match std::fs::read_to_string(self.crops_json()) {
            Ok(s) => serde_json::from_str(&s)?,
            Err(_) => BTreeMap::new(),
        };
        for o in self.overlays.iter_mut() {
            if o.crop_box.is_none() {
                o.crop_box = m.get(&o.crop_key()).copied();
            }
        }
        Ok(())
    }

    pub fn save(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.build_dir)?;
        std::fs::write(self.plan_json(), serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
    pub fn load_json(path: &Path) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }
}

/// A stable-enough 64-bit key for cache stamps (same binary, same input → same key).
pub fn key_of<T: Hash>(x: &T) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    x.hash(&mut h);
    format!("{:016x}", h.finish())
}
