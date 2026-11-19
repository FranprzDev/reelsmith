//! Projects and briefs from structured arguments. A project folder holds two configs in the `promo` format:
//!
//! * `promo.toml`, the style shared by every ad of the project: canvas, media folders, music, captions switch.
//! * `brief.toml`, one ad: its beats (text + clips + optional overlay) and end card. It `extends = "promo.toml"`, so
//!   the template resolves both into one timeline exactly as the `promo` CLI does.
use anyhow::{Context, Result, bail};
use promo::config::{Aspect, CueKind, Engine, MusicDirection, MusicKind, SoundfontStyle};
use promo::pipeline::{self, PlanSummary};
use promo::{Overrides, Plan, Res};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use toml::{Table, Value};

pub const STYLE_FILE: &str = "promo.toml";
pub const BRIEF_FILE: &str = "brief.toml";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateProjectArgs {
    /// folder for the project (created when missing); renders go to `<project_dir>/out`, caches to `<project_dir>/build`
    pub project_dir: PathBuf,
    /// folder with the recordings (shots)
    pub clips_folder: PathBuf,
    /// folder with alpha motion graphics (overlays, end card assets); defaults to `clips_folder`
    pub motion_folder: Option<PathBuf>,
    /// canvas shape: "9:16" (Reels/TikTok/Shorts), "16:9" (YouTube) or "1:1"
    #[serde(default)]
    pub aspect: Aspect,
    /// music bed: "procedural" (built-in, no samples), "file" (your own track in `music_file`), "soundfont" (local SF2 bank in `music_soundfont`) or "none"
    #[serde(default)]
    pub music: MusicKind,
    /// tempo of the procedural bed (default 128)
    pub bpm: Option<f64>,
    /// audio file for `music = "file"`
    pub music_file: Option<PathBuf>,
    /// optional music direction for procedural or sampled beds (mood): "calm", "uplifting" or "tense"
    #[serde(default, alias = "mood", alias = "music_mood", alias = "direction")]
    pub music_direction: Option<MusicDirection>,
    /// root note for directed music (MIDI note, 36..=72)
    #[serde(default, alias = "key_midi", alias = "root", alias = "root_midi")]
    pub music_key: Option<f64>,
    /// local SF2 bank for `music = "soundfont"`
    #[serde(default, alias = "soundfont_path")]
    pub music_soundfont: Option<PathBuf>,
    /// instrument palette for `music = "soundfont"`: "acoustic", "cinematic" or "funk"
    #[serde(default, alias = "soundfont_style", alias = "music_style")]
    pub music_soundfont_style: Option<SoundfontStyle>,
    /// burn the captions into the picture (default false: the texts only set the timing and go to an SRT sidecar)
    #[serde(default)]
    pub captions_burned_in: bool,
    /// replace an existing promo.toml
    #[serde(default)]
    pub overwrite: bool,
    /// render engine for this project: "fframes" (default) or "hyperframes" (HTML compositions, needs Node.js 22+)
    pub engine: Option<Engine>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectCreated {
    pub project_dir: PathBuf,
    pub style_file: PathBuf,
    pub clips_folder: PathBuf,
    pub motion_folder: PathBuf,
    pub out_dir: PathBuf,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClipArg {
    /// clip file stem as returned by list_clips (`name`)
    pub name: String,
    /// source in-point in seconds (default 0)
    pub in_s: Option<f64>,
    /// how long this clip stays on screen; default: the beat is shared evenly between its clips
    pub dur_s: Option<f64>,
    /// push-in as [start scale, end scale], e.g. [1.05, 1.15]; default: a slow push-in
    pub zoom: Option<[f64; 2]>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OverlayArg {
    /// motion asset file stem (an alpha video in the motion folder)
    pub asset: String,
    /// seconds after the beat starts (default 0.15)
    pub delay_s: Option<f64>,
    /// how long it stays; default: until the beat ends
    pub dur_s: Option<f64>,
    /// crop box [x, y, w, h] in source pixels; default: measured from the alpha channel
    pub crop: Option<[f64; 4]>,
    /// position and width on the 1080p design canvas; default: centred near the top at 80 % width
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub w: Option<f64>,
    /// gain of the asset's own sound effect (default 0.8)
    pub sfx_gain: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BeatArg {
    /// on-screen / SRT text; "|" splits caption groups ("WHAT IF | MINECRAFT | HAD | NEW | DIMENSIONS?")
    pub text: String,
    /// one or more clips; they share the beat
    pub clips: Vec<ClipArg>,
    /// beat length in seconds; default: as long as the text needs to be read
    pub dur_s: Option<f64>,
    /// white flash + whoosh/hit on the cut into this beat
    #[serde(default)]
    pub flash: bool,
    /// a sound cue on the cut into this beat
    pub cue: Option<CueKind>,
    /// relative music energy for this beat (0..=1)
    #[serde(default, alias = "energy", alias = "intensity", alias = "level")]
    pub music_energy: Option<f64>,
    pub overlay: Option<OverlayArg>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EndCardArg {
    /// the closing clip (it blurs behind the asset)
    pub clip: ClipArg,
    /// full-frame alpha card (call to action, server IP) from the motion folder
    pub asset: Option<String>,
    /// end card length in seconds (default 4.5)
    pub dur_s: Option<f64>,
    /// asset position and width on the 1080p design canvas; default: full frame
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub w: Option<f64>,
    /// closing text for the SRT / captions
    pub text: Option<String>,
    /// relative music energy for this card (0..=1)
    #[serde(default, alias = "energy", alias = "intensity", alias = "level")]
    pub music_energy: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteBriefArgs {
    /// a folder made by create_project (it holds promo.toml)
    pub project_dir: PathBuf,
    /// output name: files are `out/<name>_<preview|draft|final>_<w>x<h>.mp4`
    pub name: String,
    /// optional music direction for procedural or sampled beds (overrides the project style)
    #[serde(default, alias = "mood", alias = "music_mood", alias = "direction")]
    pub music_direction: Option<MusicDirection>,
    /// tempo for directed music (overrides the project style)
    #[serde(default)]
    pub music_bpm: Option<f64>,
    /// root note for directed music (MIDI note, 36..=72)
    #[serde(default, alias = "key_midi", alias = "root", alias = "root_midi")]
    pub music_key: Option<f64>,
    /// local SF2 bank for `music.kind = "soundfont"` (overrides the project style)
    #[serde(default, alias = "soundfont_path")]
    pub music_soundfont: Option<PathBuf>,
    /// instrument palette for `music.kind = "soundfont"` (overrides the project style)
    #[serde(default, alias = "soundfont_style", alias = "music_style")]
    pub music_soundfont_style: Option<SoundfontStyle>,
    /// one beat per idea, in order
    pub beats: Vec<BeatArg>,
    pub end_card: Option<EndCardArg>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BriefWritten {
    /// the config to pass to render
    pub config: PathBuf,
    pub style_file: PathBuf,
    /// the resolved timeline at preview size
    pub plan: PlanSummary,
    /// input files the brief names that do not exist (render fails until they do)
    pub missing: Vec<String>,
    /// one entry per clip / motion asset in `missing`, in the same order: the nearest existing
    /// file stems in the same folder (empty when nothing is close enough). Omitted when `missing`
    /// is empty or names no clip / motion asset. Fix the name in the brief and call write_brief again
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<MissingSuggestion>,
}

/// A missing clip / motion asset and the closest names that do exist in its folder.
#[derive(Debug, Serialize, JsonSchema)]
pub struct MissingSuggestion {
    /// the missing input, same entry as in `missing` ("clip ..." / "motion asset ...")
    pub input: String,
    /// existing file stems in that folder, nearest first (empty when nothing is close enough)
    pub closest: Vec<String>,
}

fn table(pairs: Vec<(&str, Option<Value>)>) -> Value {
    let mut t = Table::new();
    for (k, v) in pairs {
        if let Some(v) = v {
            t.insert(k.to_string(), v);
        }
    }
    Value::Table(t)
}

fn num(x: Option<f64>) -> Option<Value> {
    x.map(Value::Float)
}

fn path_str(p: &Path) -> Value {
    Value::String(p.display().to_string())
}

fn enum_str<T: Serialize>(x: &T) -> Result<Value> {
    Ok(Value::try_from(x).context("serializing an enum value")?)
}

fn absolute(p: &Path) -> Result<PathBuf> {
    Ok(if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir()?.join(p) })
}

pub fn create_project(a: &CreateProjectArgs) -> Result<ProjectCreated> {
    let dir = absolute(&a.project_dir)?;
    let clips = absolute(&a.clips_folder)?;
    let motion = absolute(a.motion_folder.as_deref().unwrap_or(&a.clips_folder))?;
    anyhow::ensure!(clips.is_dir(), "clips_folder {} is not a folder", clips.display());
    anyhow::ensure!(motion.is_dir(), "motion_folder {} is not a folder", motion.display());
    if a.music == MusicKind::File && a.music_file.is_none() {
        bail!("music = \"file\" needs music_file");
    }
    if a.music == MusicKind::Soundfont && a.music_soundfont.is_none() {
        bail!("music = \"soundfont\" needs music_soundfont");
    }
    if a.music == MusicKind::Soundfont && a.music_soundfont_style.is_none() {
        bail!("music = \"soundfont\" needs music_soundfont_style");
    }
    if a.music != MusicKind::Soundfont && (a.music_soundfont.is_some() || a.music_soundfont_style.is_some()) {
        bail!("music_soundfont / music_soundfont_style only apply when music = \"soundfont\"");
    }
    if let Some(bpm) = a.bpm {
        anyhow::ensure!(bpm.is_finite() && (1.0..=300.0).contains(&bpm), "bpm must be between 1 and 300");
    }
    if let Some(k) = a.music_key {
        anyhow::ensure!(k.is_finite() && k.fract() == 0.0 && (36.0..=72.0).contains(&k), "music_key must be a finite integer MIDI note 36..=72");
    }
    if a.music_key.is_some() && a.music_direction.is_none() {
        bail!("music_key requires music_direction");
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let style = dir.join(STYLE_FILE);
    if style.exists() && !a.overwrite {
        bail!("{} already exists (pass overwrite = true to replace it)", style.display());
    }
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("promo").to_string();
    let doc = table(vec![
        ("video", Some(table(vec![("name", Some(Value::String(name))), ("aspect", Some(enum_str(&a.aspect)?))]))),
        ("paths", Some(table(vec![("clips", Some(path_str(&clips))), ("motion", Some(path_str(&motion))), ("out", Some(Value::String("out".into())))]))),
        ("captions", Some(table(vec![("enabled", Some(Value::Boolean(a.captions_burned_in)))]))),
        ("render", a.engine.as_ref().map(|e| table(vec![("engine", Some(enum_str(e).unwrap_or(Value::String("fframes".into()))))]))),
        (
            "music",
            Some(table(vec![
                ("kind", Some(enum_str(&a.music)?)),
                ("direction", a.music_direction.as_ref().map(enum_str).transpose()?),
                ("key", num(a.music_key)),
                ("bpm", num(a.bpm)),
                ("file", a.music_file.as_deref().map(absolute).transpose()?.as_deref().map(path_str)),
                ("soundfont", a.music_soundfont.as_deref().map(absolute).transpose()?.as_deref().map(path_str)),
                ("soundfont_style", a.music_soundfont_style.as_ref().map(enum_str).transpose()?),
            ])),
        ),
    ]);
    let header = "# Project style (written by Reelsmith create_project). Every brief in this folder extends it.\n\n";
    std::fs::write(&style, format!("{header}{}", toml::to_string_pretty(&doc)?))?;
    Ok(ProjectCreated { project_dir: dir.clone(), style_file: style, clips_folder: clips, motion_folder: motion, out_dir: dir.join("out") })
}

fn clip_value(c: &ClipArg) -> Value {
    table(vec![
        ("clip", Some(Value::String(c.name.clone()))),
        ("in", num(c.in_s)),
        ("dur", num(c.dur_s)),
        ("zoom", c.zoom.map(|z| Value::Array(z.iter().map(|v| Value::Float(*v)).collect()))),
    ])
}

fn beat_value(b: &BeatArg) -> Result<Value> {
    anyhow::ensure!(!b.clips.is_empty(), "the beat \"{}\" has no clips", b.text);
    let overlay = b.overlay.as_ref().map(|o| {
        table(vec![
            ("asset", Some(Value::String(o.asset.clone()))),
            ("delay", num(o.delay_s)),
            ("dur", num(o.dur_s)),
            ("crop", Some(match o.crop {
                // the template reads the box as whole asset pixels
                Some(c) => Value::Array(c.iter().map(|v| Value::Integer(v.round() as i64)).collect()),
                None => Value::String("auto".into()),
            })),
            ("x", num(o.x)),
            ("y", num(o.y)),
            ("w", num(o.w)),
            ("sfx", num(o.sfx_gain)),
        ])
    });
    Ok(table(vec![
        ("text", Some(Value::String(b.text.clone()))),
        ("dur", num(b.dur_s)),
        ("flash", b.flash.then_some(Value::Boolean(true))),
        ("cue", b.cue.as_ref().map(enum_str).transpose()?),
        ("energy", num(b.music_energy)),
        ("clips", Some(Value::Array(b.clips.iter().map(clip_value).collect()))),
        ("overlay", overlay),
    ]))
}

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// a temporary file in the project folder; removing it on drop leaves the previous brief untouched when
/// writing, parsing or validating the new one fails
struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn create(dir: &Path) -> Result<(Self, std::fs::File)> {
        for _ in 0..100 {
            let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let path = dir.join(format!(".{BRIEF_FILE}.{}.{seq}.tmp", std::process::id()));
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((Self { path }, file)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e).with_context(|| format!("creating a temporary brief in {}", dir.display())),
            }
        }
        bail!("could not create a unique temporary brief in {}", dir.display())
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn write_brief(a: &WriteBriefArgs) -> Result<BriefWritten> {
    let dir = absolute(&a.project_dir)?;
    let style = dir.join(STYLE_FILE);
    anyhow::ensure!(style.is_file(), "{} has no {STYLE_FILE}: call create_project first", dir.display());
    anyhow::ensure!(!a.beats.is_empty(), "a brief needs at least one beat");
    if let Some(bpm) = a.music_bpm {
        anyhow::ensure!(bpm.is_finite() && (1.0..=300.0).contains(&bpm), "music_bpm must be between 1 and 300");
    }
    if let Some(k) = a.music_key {
        anyhow::ensure!(k.is_finite() && k.fract() == 0.0 && (36.0..=72.0).contains(&k), "music_key must be a finite integer MIDI note 36..=72");
    }
    for b in &a.beats {
        if let Some(e) = b.music_energy {
            anyhow::ensure!((0.0..=1.0).contains(&e), "music_energy must be 0..=1");
        }
    }
    if let Some(e) = &a.end_card {
        if let Some(en) = e.music_energy {
            anyhow::ensure!((0.0..=1.0).contains(&en), "music_energy must be 0..=1");
        }
    }
    anyhow::ensure!(
        !a.name.is_empty() && a.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "name must be letters, digits, '_' or '-' (it becomes the file name)"
    );
    let beats = a.beats.iter().map(beat_value).collect::<Result<Vec<_>>>()?;
    let end_card = a.end_card.as_ref().map(|e| {
        table(vec![
            ("clip", Some(clip_value(&e.clip))),
            ("asset", e.asset.clone().map(Value::String)),
            ("dur", num(e.dur_s)),
            ("x", num(e.x)),
            ("y", num(e.y)),
            ("w", num(e.w)),
            ("text", e.text.clone().map(Value::String)),
            ("energy", num(e.music_energy)),
        ])
    });
    let music = if a.music_direction.is_some() || a.music_bpm.is_some() || a.music_key.is_some() || a.music_soundfont.is_some() || a.music_soundfont_style.is_some() {
        Some(table(vec![
            ("direction", a.music_direction.as_ref().map(enum_str).transpose()?),
            ("bpm", num(a.music_bpm)),
            ("key", num(a.music_key)),
            ("soundfont", a.music_soundfont.as_deref().map(absolute).transpose()?.as_deref().map(path_str)),
            ("soundfont_style", a.music_soundfont_style.as_ref().map(enum_str).transpose()?),
        ]))
    } else {
        None
    };
    let doc = table(vec![
        ("extends", Some(Value::String(STYLE_FILE.into()))),
        ("video", Some(table(vec![("name", Some(Value::String(a.name.clone())))]))),
        ("beat", Some(Value::Array(beats))),
        ("end_card", end_card),
        ("music", music),
    ]);
    let header = "# Brief (written by Reelsmith write_brief): one [[beat]] per idea. Render it with the render tool.\n\n";
    let text = format!("{header}{}", toml::to_string_pretty(&doc)?);
    let config = dir.join(BRIEF_FILE);
    let (tmp, mut file) = TempFile::create(&dir)?;
    file.write_all(text.as_bytes()).with_context(|| format!("writing {}", tmp.path.display()))?;
    drop(file);
    let mut plan = Plan::from_file(&tmp.path, &Overrides::default())?;
    std::fs::rename(&tmp.path, &config).with_context(|| format!("replacing {}", config.display()))?;
    plan.config = plan.config.parent().unwrap_or(Path::new(".")).join(BRIEF_FILE);
    let missing = plan.missing_inputs();
    let suggestions = suggest_missing(&plan.clips_dir, &plan.motion_dir, &missing);
    Ok(BriefWritten { config, style_file: style, plan: pipeline::summary(&plan, Res::Preview), missing, suggestions })
}

/// stems of the media files with one of `exts` directly inside `dir`; an unreadable folder and a
/// folder without media both come back empty (a suggestion list is a nicety, not a diagnostic)
fn media_stems(dir: &Path, exts: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .filter_map(|e| {
                    let p = e.path();
                    // extension matching is case-sensitive on purpose: the resolver only tries
                    // lowercase `exts` via direct join, so accepting e.g. `.MP4` here would
                    // suggest a bare stem that cannot resolve.
                    let ext = p.extension().and_then(|x| x.to_str()).unwrap_or_default();
                    if exts.contains(&ext) {
                        // non-UTF-8 stems cannot be named in a brief either, so dropping them loses nothing
                        p.file_stem().and_then(|x| x.to_str()).map(str::to_string)
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.dedup();
    v
}

/// case-insensitive Levenshtein distance over chars (host typo correction: near names only)
fn lev(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.to_lowercase().chars().collect();
    let b: Vec<char> = b.to_lowercase().chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// the up to `n` candidates nearest to `needle`, nearest first; distant names are not suggestions
/// (ties prefer the shorter candidate, then alphabetical - a policy locked in by the tests). The
/// cutoff is tighter for very short needles, where a distance of 2 matches almost anything
fn closest_stems(needle: &str, candidates: &[&str], n: usize) -> Vec<String> {
    let len = needle.chars().count();
    let cutoff = if len < 3 { 1 } else { (len / 2).max(2) };
    let mut ranked: Vec<(usize, &&str)> = candidates
        .iter()
        .map(|c| (lev(needle, c), c))
        .filter(|(d, _)| *d <= cutoff)
        .collect();
    ranked.sort_by(|(da, ca), (db, cb)| da.cmp(db).then_with(|| ca.len().cmp(&cb.len())).then_with(|| ca.cmp(cb)));
    ranked.into_iter().take(n).map(|(_, c)| c.to_string()).collect()
}

fn stems_for<'a>(cache: &'a mut std::collections::HashMap<PathBuf, Vec<String>>, dir: &Path, exts: &[&str]) -> &'a [String] {
    cache.entry(dir.to_path_buf()).or_insert_with(|| media_stems(dir, exts))
}

/// "did you mean" entries for the missing clips / motion assets of a resolved plan, in the same
/// order they appear in `missing`; other missing inputs (font, voice-over, music) have no folder
/// of candidates to suggest from and are skipped
fn suggest_missing(clips_dir: &Path, motion_dir: &Path, missing: &[String]) -> Vec<MissingSuggestion> {
    let mut clip_stems: std::collections::HashMap<PathBuf, Vec<String>> = std::collections::HashMap::new();
    let mut motion_stems: std::collections::HashMap<PathBuf, Vec<String>> = std::collections::HashMap::new();
    let mut out = vec![];
    for m in missing {
        let (path, base, exts, cache) = if let Some(p) = m.strip_prefix("clip ") {
            (p, clips_dir, promo::plan::CLIP_EXTS, &mut clip_stems)
        } else if let Some(p) = m.strip_prefix("motion asset ") {
            (p, motion_dir, promo::plan::MOTION_EXTS, &mut motion_stems)
        } else {
            continue;
        };
        let p = Path::new(path);
        // a name may carry a subpath ("intro/foo"): search the folder the file would live in
        let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(base);
        let closest = match p.file_stem().and_then(|x| x.to_str()) {
            Some(needle) => {
                let stems = stems_for(cache, dir, exts);
                closest_stems(needle, &stems.iter().map(|s| s.as_str()).collect::<Vec<_>>(), 3)
            }
            None => vec![],
        };
        out.push(MissingSuggestion { input: m.clone(), closest });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lev_basics() {
        assert_eq!(lev("foo", "foo"), 0);
        assert_eq!(lev("Foo", "foo"), 0);
        assert_eq!(lev("foo", "fool"), 1);
        assert_eq!(lev("nebulaa", "nebula"), 1);
        assert_eq!(lev("abc", "xyz"), 3);
    }

    #[test]
    fn closest_ranks_and_limits() {
        let cands = ["nebula", "cells", "aurora", "grid", "nebular"];
        assert_eq!(closest_stems("nebulla", &cands, 3), vec!["nebula", "nebular"]);
        assert_eq!(closest_stems("nebula", &cands, 1), vec!["nebula"]);
        assert!(closest_stems("nebulla", &[], 3).is_empty());
    }

    #[test]
    fn distant_names_are_not_suggested() {
        assert!(closest_stems("nebula", &["zzz", "qqqqqqqq"], 3).is_empty());
    }

    #[test]
    fn short_needles_use_a_tight_cutoff() {
        // distance 1 still helps ("ac" -> "ab"), distance 2 on a 2-char name is noise ("ab" vs "cd")
        assert_eq!(closest_stems("ac", &["ab"], 3), vec!["ab"]);
        assert!(closest_stems("ab", &["cd"], 3).is_empty());
        assert!(closest_stems("a", &["xy"], 3).is_empty());
    }

    fn tmp_clips(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reelsmith-suggest-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["nebula.mp4", "cells.mp4", "badge.mov", "notes.txt"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        dir
    }

    #[test]
    fn suggests_for_clip_and_motion_and_skips_other_inputs() {
        let dir = tmp_clips("e2e");
        let missing = vec![
            format!("clip {}/nebulaa.mp4", dir.display()),
            format!("motion asset {}/badg.mov", dir.display()),
            "caption font /nowhere/font.ttf".to_string(),
        ];
        let got = suggest_missing(&dir, &dir, &missing);
        assert_eq!(got.len(), 2, "the font entry has no candidates folder and is skipped");
        assert_eq!(got[0].input, missing[0]);
        assert_eq!(got[0].closest, vec!["nebula"]);
        assert_eq!(got[1].input, missing[1]);
        assert_eq!(got[1].closest, vec!["badge"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn subpath_names_search_their_own_folder() {
        let dir = tmp_clips("sub");
        let sub = dir.join("intro");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("logo.mp4"), b"").unwrap();
        let missing = vec![format!("clip {}/intro/logoo.mp4", dir.display())];
        let got = suggest_missing(&dir, &dir, &missing);
        assert_eq!(got[0].closest, vec!["logo"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_missing_means_no_suggestions() {
        let dir = tmp_clips("empty");
        assert!(suggest_missing(&dir, &dir, &[]).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
