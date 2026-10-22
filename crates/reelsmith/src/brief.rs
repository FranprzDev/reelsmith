//! Projects and briefs from structured arguments. A project folder holds two configs in the `promo` format:
//!
//! * `promo.toml`, the style shared by every ad of the project: canvas, media folders, music, captions switch.
//! * `brief.toml`, one ad: its beats (text + clips + optional overlay) and end card. It `extends = "promo.toml"`, so
//!   the template resolves both into one timeline exactly as the `promo` CLI does.
use anyhow::{Context, Result, bail};
use promo::config::{Aspect, CueKind, MusicKind};
use promo::pipeline::{self, PlanSummary};
use promo::{Overrides, Plan, Res};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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
    /// music bed: "procedural" (built-in, no samples), "file" (your own track in `music_file`) or "none"
    #[serde(default)]
    pub music: MusicKind,
    /// tempo of the procedural bed (default 128)
    pub bpm: Option<f64>,
    /// audio file for `music = "file"`
    pub music_file: Option<PathBuf>,
    /// burn the captions into the picture (default false: the texts only set the timing and go to an SRT sidecar)
    #[serde(default)]
    pub captions_burned_in: bool,
    /// replace an existing promo.toml
    #[serde(default)]
    pub overwrite: bool,
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
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteBriefArgs {
    /// a folder made by create_project (it holds promo.toml)
    pub project_dir: PathBuf,
    /// output name: files are `out/<name>_<preview|draft|final>_<w>x<h>.mp4`
    pub name: String,
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
        (
            "music",
            Some(table(vec![
                ("kind", Some(enum_str(&a.music)?)),
                ("bpm", num(a.bpm)),
                ("file", a.music_file.as_deref().map(absolute).transpose()?.as_deref().map(path_str)),
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
                Some(c) => Value::Array(c.iter().map(|v| Value::Float(*v)).collect()),
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
        ("clips", Some(Value::Array(b.clips.iter().map(clip_value).collect()))),
        ("overlay", overlay),
    ]))
}

pub fn write_brief(a: &WriteBriefArgs) -> Result<BriefWritten> {
    let dir = absolute(&a.project_dir)?;
    let style = dir.join(STYLE_FILE);
    anyhow::ensure!(style.is_file(), "{} has no {STYLE_FILE}: call create_project first", dir.display());
    anyhow::ensure!(!a.beats.is_empty(), "a brief needs at least one beat");
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
        ])
    });
    let doc = table(vec![
        ("extends", Some(Value::String(STYLE_FILE.into()))),
        ("video", Some(table(vec![("name", Some(Value::String(a.name.clone())))]))),
        ("beat", Some(Value::Array(beats))),
        ("end_card", end_card),
    ]);
    let header = "# Brief (written by Reelsmith write_brief): one [[beat]] per idea. Render it with the render tool.\n\n";
    let config = dir.join(BRIEF_FILE);
    std::fs::write(&config, format!("{header}{}", toml::to_string_pretty(&doc)?))?;
    let plan = Plan::from_file(&config, &Overrides::default())?;
    Ok(BriefWritten { config, style_file: style, plan: pipeline::summary(&plan, Res::Preview), missing: plan.missing_inputs() })
}
