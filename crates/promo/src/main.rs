//! `promo`: short-form promo / vertical ad videos from a config (see `docs/PROMO_VIDEOS.md`).
//!
//! ```text
//! promo build  CONFIG [flags]      check -> plate -> audio -> render -> mux -> srt -> qa
//! promo batch  CONFIG... | LIST.txt [flags]   build several configs, keep going past failures
//! promo check  CONFIG              resolve the config, list missing inputs (no rendering)
//! promo plan   CONFIG              print the resolved timeline as JSON
//! promo plate  CONFIG [shots|overlays]
//! promo audio  CONFIG
//! promo render CONFIG [--chunk K]
//! promo mux    CONFIG [--out FILE]
//! promo srt    CONFIG              write the captions as an SRT sidecar
//! promo qa     CONFIG [VIDEO]
//!
//! flags: --preview (360p) | --final (1080p) | default 720p draft
//!        --captions on|off  --clips DIR  --motion DIR  --out FILE  --out-dir DIR  --build-dir DIR
//!        --encoder auto|nvenc|videotoolbox|x264 (auto)
//!        --chunk-frames N (60)  --retries N (3)  --no-qa  --json (NDJSON progress + result on stdout)
//! exit:  0 ok, 1 failed, 2 bad config/usage, 3 QA failed, 4 missing input
//! ```
use anyhow::Result;
use promo::encode::EncoderChoice;
use promo::error::{ConfigError, Exit};
use promo::pipeline::{self, BuildOptions, BuildOutcome};
use promo::plate::Only;
use promo::progress::{Events, Stage};
use promo::{Overrides, Plan, Res, audio, plate, qa, render};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = include_str!("usage.txt");

#[derive(Debug, Default)]
struct Opts {
    cmd: String,
    pos: Vec<String>,
    res: Res,
    ov: Overrides,
    out: Option<PathBuf>,
    json: bool,
    chunk: Option<usize>,
    chunk_frames: usize,
    retries: usize,
    no_qa: bool,
    encoder: EncoderChoice,
}

fn parse(args: &[String]) -> Result<Opts, ConfigError> {
    let mut o = Opts { chunk_frames: 60, retries: 3, ..Default::default() };
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--preview" => o.res = Res::Preview,
            "--final" => o.res = Res::Final,
            "--draft" => o.res = Res::Draft,
            "--json" => o.json = true,
            "--no-qa" => o.no_qa = true,
            "--captions" => {
                o.ov.captions = Some(match val(&mut it, a)?.as_str() {
                    "on" | "true" | "1" => true,
                    "off" | "false" | "0" => false,
                    v => return Err(ConfigError(format!("--captions takes on|off, got {v}"))),
                })
            }
            "--encoder" => {
                o.encoder = match val(&mut it, a)?.as_str() {
                    "auto" => EncoderChoice::Auto,
                    "nvenc" => EncoderChoice::Nvenc,
                    "videotoolbox" => EncoderChoice::Videotoolbox,
                    "x264" => EncoderChoice::X264,
                    v => return Err(ConfigError(format!("--encoder takes auto|nvenc|videotoolbox|x264, got {v}"))),
                }
            }
            "--clips" => o.ov.clips = Some(abs(&val(&mut it, a)?)),
            "--motion" => o.ov.motion = Some(abs(&val(&mut it, a)?)),
            "--out-dir" => o.ov.out_dir = Some(abs(&val(&mut it, a)?)),
            "--build-dir" => o.ov.build_dir = Some(abs(&val(&mut it, a)?)),
            "--out" | "-o" => o.out = Some(abs(&val(&mut it, a)?)),
            "--chunk" => o.chunk = Some(num(&val(&mut it, a)?, a)?),
            "--chunk-frames" => o.chunk_frames = num(&val(&mut it, a)?, a)?.max(1),
            "--retries" => o.retries = num(&val(&mut it, a)?, a)?.max(1),
            f if f.starts_with("--") => return Err(ConfigError(format!("unknown flag {f}"))),
            p if o.cmd.is_empty() => o.cmd = p.to_string(),
            p => o.pos.push(p.to_string()),
        }
    }
    if std::env::var_os("PROMO_FINAL").is_some() {
        o.res = Res::Final;
    }
    Ok(o)
}

fn val<'a>(it: &mut impl Iterator<Item = &'a String>, f: &str) -> Result<String, ConfigError> {
    it.next().cloned().ok_or_else(|| ConfigError(format!("{f} needs a value")))
}

fn num(v: &str, f: &str) -> Result<usize, ConfigError> {
    v.parse().map_err(|_| ConfigError(format!("{f} takes a number, got {v}")))
}

fn abs(p: &str) -> PathBuf {
    let p = PathBuf::from(p);
    if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
}

fn config_arg(o: &Opts) -> Result<PathBuf, ConfigError> {
    o.pos.first().map(|p| abs(p)).or_else(|| std::env::var_os("PROMO_CONFIG").map(PathBuf::from)).ok_or_else(|| ConfigError(format!("`promo {}` needs a CONFIG (a .toml or .json file)", o.cmd)))
}

fn load(cfg: &Path, o: &Opts) -> Result<Plan> {
    pipeline::load(cfg, &o.ov)
}

fn build_options(o: &Opts) -> BuildOptions {
    BuildOptions { res: o.res, overrides: o.ov.clone(), out: o.out.clone(), chunk_frames: o.chunk_frames, retries: o.retries, qa: !o.no_qa, encoder: o.encoder }
}

/// One build, with its result printed as the last NDJSON event.
fn build(cfg: &Path, o: &Opts, ev: &Events) -> Result<BuildOutcome> {
    let r = pipeline::build(cfg, &build_options(o), ev)?;
    ev.result(&serde_json::to_value(&r)?);
    Ok(r)
}

fn main() -> ExitCode {
    if std::env::var_os(render::PASSTHROUGH).is_some() {
        return match render::passthrough() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: {e:#}");
                ExitCode::FAILURE
            }
        };
    }
    let args: Vec<String> = std::env::args().collect();
    let o = match parse(&args) {
        Ok(o) => o,
        Err(e) => return fail(&Events::new(args.iter().any(|a| a == "--json")), &e.into()),
    };
    let ev = Events::new(o.json);
    match run(&o, &ev) {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => fail(&ev, &e),
    }
}

fn fail(ev: &Events, e: &anyhow::Error) -> ExitCode {
    let code = Exit::of(e);
    ev.log(&format!("error: {e:#}"));
    ev.result(&json!({"ok": false, "exit": code, "code": code as u8, "error": format!("{e:#}")}));
    ExitCode::from(code as u8)
}

fn run(o: &Opts, ev: &Events) -> Result<Exit> {
    match o.cmd.as_str() {
        "" | "help" | "-h" | "--help" => {
            print!("{USAGE}");
            Ok(Exit::Ok)
        }
        "build" => {
            Ok(build(&config_arg(o)?, o, ev)?.exit)
        }
        "batch" => batch(o, ev),
        "check" => {
            let p = load(&config_arg(o)?, o)?;
            let missing = p.missing_inputs();
            let s = pipeline::summary(&p, o.res);
            ev.log(&format!(
                "{}: {} {:.2} s, {} shots, {} overlays, {} captions (burned in: {}), {} flashes, music {:?}",
                p.name,
                p.out_size(o.res).0.to_string() + "x" + &p.out_size(o.res).1.to_string(),
                p.seconds,
                p.shots.len(),
                p.overlays.len(),
                p.captions.len(),
                p.captions_on,
                p.flashes.len(),
                p.music.kind
            ));
            for m in &missing {
                ev.log(&format!("  missing: {m}"));
            }
            ev.result(&json!({"ok": missing.is_empty(), "plan": s, "missing": missing}));
            Ok(if missing.is_empty() { Exit::Ok } else { Exit::MissingInput })
        }
        "plan" => {
            let p = load(&config_arg(o)?, o)?;
            println!("{}", serde_json::to_string_pretty(&p)?);
            Ok(Exit::Ok)
        }
        "plate" => {
            let mut p = load(&config_arg(o)?, o)?;
            let only = match o.pos.get(1).map(String::as_str) {
                None => Only::All,
                Some("shots") => Only::Shots,
                Some("overlays") => Only::Overlays,
                Some(x) => return Err(ConfigError(format!("plate takes shots|overlays, got {x}")).into()),
            };
            plate::run(&mut p, o.res, only, ev)?;
            p.save()?;
            Ok(Exit::Ok)
        }
        "audio" => {
            let mut p = load(&config_arg(o)?, o)?;
            plate::measure_crops(&mut p)?;
            audio::run(&p, ev)?;
            Ok(Exit::Ok)
        }
        "render" => {
            let mut p = load(&config_arg(o)?, o)?;
            plate::measure_crops(&mut p)?;
            p.save()?;
            let fb = promo::frame::FrameBuilder::new(p)?;
            render::render_all(&fb, o.res, o.chunk_frames, o.retries, o.chunk, ev)?;
            Ok(Exit::Ok)
        }
        "mux" => {
            let p = load(&config_arg(o)?, o)?;
            let out = o.out.clone().unwrap_or_else(|| p.default_out(o.res));
            render::mux(&p, o.res, o.chunk_frames, o.encoder.resolve()?, &out)?;
            ev.log(&format!("mux: {}", out.display()));
            Ok(Exit::Ok)
        }
        "srt" => {
            let p = load(&config_arg(o)?, o)?;
            let out = pipeline::write_srt(&p)?;
            ev.log(&format!("srt: {}", out.as_ref().map(|p| p.display().to_string()).unwrap_or("no captions".into())));
            Ok(Exit::Ok)
        }
        "qa" => {
            let p = load(&config_arg(o)?, o)?;
            let v = o.pos.get(1).map(|s| abs(s)).or(o.out.clone()).unwrap_or_else(|| p.default_out(o.res));
            let s = qa::run(&p, o.res, &v)?;
            pipeline::log_qa(ev, &s);
            ev.result(&json!({"ok": s.ok, "qa": s}));
            Ok(if s.ok { Exit::Ok } else { Exit::QaFailed })
        }
        c => Err(ConfigError(format!("unknown command `{c}` (promo help)")).into()),
    }
}

fn batch(o: &Opts, ev: &Events) -> Result<Exit> {
    let mut cfgs = vec![];
    for a in &o.pos {
        let p = abs(a);
        if p.extension().is_some_and(|x| x == "txt") {
            let dir = p.parent().unwrap_or(Path::new(".")).to_path_buf();
            let txt = std::fs::read_to_string(&p).map_err(|e| ConfigError(format!("batch list {}: {e}", p.display())))?;
            cfgs.extend(txt.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(|l| {
                let q = PathBuf::from(l);
                if q.is_absolute() { q } else { dir.join(q) }
            }));
        } else {
            cfgs.push(p);
        }
    }
    if cfgs.is_empty() {
        return Err(ConfigError("`promo batch` needs CONFIG files or a .txt list of them".into()).into());
    }
    if o.out.is_some() {
        return Err(ConfigError("--out names one file; use --out-dir with batch".into()).into());
    }
    let mut worst = Exit::Ok;
    let mut results = vec![];
    for (i, c) in cfgs.iter().enumerate() {
        ev.log(&format!("batch: {}/{} {}", i + 1, cfgs.len(), c.display()));
        let (code, v) = match build(c, o, ev) {
            Ok(r) => (r.exit, serde_json::to_value(&r)?),
            Err(e) => {
                let code = Exit::of(&e);
                ev.log(&format!("batch: {} failed: {e:#}", c.display()));
                (code, json!({"ok": false, "exit": code, "code": code as u8, "config": c, "error": format!("{e:#}")}))
            }
        };
        if worst == Exit::Ok && code != Exit::Ok {
            worst = code;
        }
        ev.progress(Stage::Build, i + 1, cfgs.len());
        results.push(v);
    }
    let ok = results.iter().filter(|r| r["ok"] == true).count();
    ev.log(&format!("batch: {ok}/{} ok", results.len()));
    ev.result(&json!({"ok": worst == Exit::Ok, "exit": worst, "code": worst as u8, "batch": results}));
    Ok(worst)
}
