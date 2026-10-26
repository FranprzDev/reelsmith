//! The whole pipeline for one config (check -> plate -> audio -> render -> mux -> srt -> qa), shared by every front end
//! (the `promo` CLI and the Reelsmith MCP server) so they produce the same file from the same config.
use crate::config::{Aspect, MusicKind, Res};
use crate::encode::{Backend, EncoderChoice};
use crate::error::Exit;
use crate::plan::{Overrides, Plan};
use crate::plate::{self, Only};
use crate::progress::{Events, Stage, Status};
use crate::{audio, qa, render, script};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How one build runs: the resolution, the per-job overrides and the render tuning.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub res: Res,
    pub overrides: Overrides,
    /// output MP4 (default: `<out>/<name>_<res>_<w>x<h>.mp4`)
    pub out: Option<PathBuf>,
    pub chunk_frames: usize,
    pub retries: usize,
    pub qa: bool,
    pub encoder: EncoderChoice,
    /// how many plates / render chunks run at once (see [`crate::pool::default_jobs`])
    pub jobs: usize,
}

/// What a resolved config will render.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlanSummary {
    pub name: String,
    pub aspect: Aspect,
    pub resolution: Res,
    pub width: u32,
    pub height: u32,
    pub seconds: f64,
    pub frames: usize,
    pub shots: usize,
    pub overlays: usize,
    pub captions: usize,
    pub captions_burned_in: bool,
    pub flashes: Vec<f64>,
    pub music: MusicKind,
    pub voiceover: bool,
    pub build_dir: PathBuf,
}

/// The result of one build (also written next to the MP4 as `<name>.result.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BuildOutcome {
    pub ok: bool,
    pub exit: Exit,
    pub code: u8,
    pub config: PathBuf,
    pub output: PathBuf,
    pub srt: Option<PathBuf>,
    /// the GPU / encoder backend this build used, with every probe and the reason it was chosen
    pub backend: Backend,
    pub plan: PlanSummary,
    pub qa: Option<qa::QaSummary>,
}

/// Load a config and its measured overlay crops.
pub fn load(cfg: &Path, ov: &Overrides) -> Result<Plan> {
    let mut p = Plan::from_file(cfg, ov)?;
    p.load_crops()?;
    Ok(p)
}

pub fn summary(p: &Plan, r: Res) -> PlanSummary {
    let (width, height) = p.out_size(r);
    PlanSummary {
        name: p.name.clone(),
        aspect: p.aspect,
        resolution: r,
        width,
        height,
        seconds: p.seconds,
        frames: p.frames(),
        shots: p.shots.len(),
        overlays: p.overlays.len(),
        captions: p.captions.len(),
        captions_burned_in: p.captions_on,
        flashes: p.flashes.clone(),
        music: p.music.kind,
        voiceover: p.voiceover.is_some(),
        build_dir: p.build_dir.clone(),
    }
}

pub fn log_qa(ev: &Events, s: &qa::QaSummary) {
    ev.log(&format!(
        "qa: {} — {:.3} s, {}x{}, {:.2} LUFS, {:.2} dBTP, black frames: {}, report {}",
        if s.ok { "PASS" } else { "FAIL" },
        s.duration_s,
        s.width,
        s.height,
        s.integrated_lufs,
        s.true_peak_dbtp,
        s.black_frames.len(),
        s.report
    ));
    for f in &s.failures {
        ev.log(&format!("  ✗ {f}"));
    }
}

/// Write the captions as `<out>/<name>.srt` (none when the config has no captions).
pub fn write_srt(p: &Plan) -> Result<Option<PathBuf>> {
    if p.captions.is_empty() {
        return Ok(None);
    }
    std::fs::create_dir_all(&p.out_dir)?;
    let out = p.out_dir.join(format!("{}.srt", p.name));
    std::fs::write(&out, script::to_srt(&p.captions))?;
    Ok(Some(out))
}

/// The whole pipeline for one config.
pub fn build(cfg: &Path, o: &BuildOptions, ev: &Events) -> Result<BuildOutcome> {
    let r = o.res;
    ev.stage(Stage::Check, Status::Start);
    let mut p = load(cfg, &o.overrides)?;
    p.ensure_inputs()?;
    crate::preview_gate::check(&p, r)?;
    let backend = o.encoder.resolve()?;
    ev.log(&backend.summary());
    ev.stage(Stage::Check, Status::Done);

    let out = o.out.clone().unwrap_or_else(|| p.default_out(r));
    match p.engine {
        crate::config::Engine::Fframes => {
            ev.stage(Stage::Plate, Status::Start);
            plate::run(&mut p, r, Only::All, o.jobs, &backend, ev)?;
            p.save()?;
            ev.stage(Stage::Plate, Status::Done);

            ev.stage(Stage::Audio, Status::Start);
            audio::run(&p, ev)?;
            ev.stage(Stage::Audio, Status::Done);

            ev.stage(Stage::Render, Status::Start);
            let fb = crate::frame::FrameBuilder::new(p.clone())?;
            render::render_all(&fb, r, o.chunk_frames, o.retries, None, o.jobs, &backend, ev)?;
            ev.stage(Stage::Render, Status::Done);

            ev.stage(Stage::Mux, Status::Start);
            render::mux(&p, r, o.chunk_frames, &backend, &out)?;
            ev.log(&format!("mux: {}", out.display()));
            ev.stage(Stage::Mux, Status::Done);
        }
        crate::config::Engine::Hyperframes => {
            ev.stage(Stage::Plate, Status::Skipped);
            ev.stage(Stage::Audio, Status::Start);
            audio::run(&p, ev)?;
            ev.stage(Stage::Audio, Status::Done);

            ev.stage(Stage::Render, Status::Start);
            crate::hyperframes::render(&p, r, &out, o.jobs, ev)?;
            ev.stage(Stage::Render, Status::Done);
            ev.stage(Stage::Mux, Status::Skipped);
        }
    }

    ev.stage(Stage::Srt, Status::Start);
    let srt = write_srt(&p)?;
    ev.stage(Stage::Srt, Status::Done);

    let qa_v = if o.qa {
        ev.stage(Stage::Qa, Status::Start);
        let s = qa::run(&p, r, &out)?;
        log_qa(ev, &s);
        ev.stage(Stage::Qa, if s.ok { Status::Done } else { Status::Failed });
        Some(s)
    } else {
        ev.stage(Stage::Qa, Status::Skipped);
        None
    };
    let ok = qa_v.as_ref().is_none_or(|s| s.ok);
    let exit = if ok { Exit::Ok } else { Exit::QaFailed };
    let res = BuildOutcome { ok, exit, code: exit as u8, config: cfg.to_path_buf(), output: out.clone(), srt, backend, plan: summary(&p, r), qa: qa_v };
    std::fs::write(out.with_extension("result.json"), serde_json::to_string_pretty(&res)?)?;
    match crate::ledger::record(&p, r, &res) {
        Ok(entry) => crate::telemetry::record(&entry, ev),
        Err(e) => ev.log(&format!("warning: the render ledger could not be written: {e:#}")),
    }
    if r == Res::Preview && res.ok {
        if let Err(e) = crate::preview_gate::record(&p, &res) {
            ev.log(&format!("warning: the preview proof could not be written: {e:#}"));
        }
    }
    Ok(res)
}
