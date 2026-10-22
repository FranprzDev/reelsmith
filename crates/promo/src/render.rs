//! The fframes render: a `Video` per canvas shape, a chunked and resumable full render (each chunk its own process with
//! only its own rasters, retried when killed) and the final join + audio mux.
//!
//! A chunk is skipped when its part exists and its key (the SVG of every frame in it plus the plate stamps it draws)
//! is unchanged, so after an edit only the touched chunks render again.
use anyhow::{Context, Result};
use fframes::{AudioMap, Color, Duration, FFramesContext, Frame, Svgr, Video};
use crate::config::{Aspect, Res};
use crate::frame::FrameBuilder;
use crate::plan::{Plan, key_of};
use crate::progress::{Events, Stage};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// set on the child process that hands a frame range to the fframes CLI
pub const PASSTHROUGH: &str = "PROMO_PASSTHROUGH";
/// the child reads the resolved plan from this file
pub const PLAN_ENV: &str = "PROMO_PLAN";
/// the child's media folder (the chunk's rasters)
pub const MEDIA_ENV: &str = "PROMO_MEDIA";
/// write every frame's SVG into this folder (debugging)
pub const DUMP_ENV: &str = "PROMO_DUMP_SVG";

struct PromoVideo<const W: usize, const H: usize> {
    fb: FrameBuilder,
}

impl<const W: usize, const H: usize> Video for PromoVideo<W, H> {
    const FPS: usize = crate::config::FPS;
    const WIDTH: usize = W;
    const HEIGHT: usize = H;
    const BACKGROUND_COLOR: Color = Color::BLACK;

    fn duration(&self) -> Duration<'_> {
        Duration::Frames(self.fb.plan.frames())
    }

    fn audio(&self) -> AudioMap<'_> {
        // the mastered mix (build/audio.wav) is muxed once after the chunks are joined
        AudioMap::none()
    }

    fn render_frame<'a>(&'a self, frame: Frame, _ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let doc = self.fb.document(frame.global_index);
        if let Some(d) = std::env::var_os(DUMP_ENV) {
            let _ = std::fs::write(Path::new(&d).join(format!("f{:05}.svg", frame.global_index)), &doc);
        }
        Svgr::from(doc)
    }
}

fn run_cli<'m, const W: usize, const H: usize>(fb: FrameBuilder, mp: &'m dyn fframes::MediaProvider<'m>) -> ExitCode {
    let video = PromoVideo::<W, H> { fb };
    fframes::cli::new(
        &video,
        fframes::RenderOptions {
            media: Some(mp),
            video_encoder_options: fframes::EncoderOptions { preferred_encoder: Some("libx264"), codec_params: Some(&[("crf", "18"), ("preset", "veryfast")]), ..Default::default() },
            ..Default::default()
        },
    )
    .run()
}

/// The fframes CLI on the plan in `PROMO_PLAN`, with the media folder in `PROMO_MEDIA`.
pub fn passthrough() -> Result<ExitCode> {
    let plan = Plan::load_json(Path::new(&std::env::var(PLAN_ENV).context("PROMO_PLAN is not set")?))?;
    let mdir = PathBuf::from(std::env::var(MEDIA_ENV).context("PROMO_MEDIA is not set")?);
    let media = fframes::MediaDirectory::read_folder(&mdir).with_context(|| format!("media folder {}", mdir.display()))?;
    let mp = media.process_media_source().map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let fb = FrameBuilder::new(plan)?;
    Ok(match fb.plan.aspect {
        Aspect::Vertical => run_cli::<1080, 1920>(fb, &mp),
        Aspect::Horizontal => run_cli::<1920, 1080>(fb, &mp),
        Aspect::Square => run_cli::<1080, 1080>(fb, &mp),
    })
}

pub fn chunks_dir(p: &Plan, r: Res) -> PathBuf {
    p.build_dir.join(format!("chunks_{}", r.tag()))
}

/// frame ranges of `chunk_frames` each (the last one shorter)
pub fn ranges(p: &Plan, chunk_frames: usize) -> Vec<(usize, usize)> {
    let n = p.frames();
    let k = n.div_ceil(chunk_frames.max(1));
    (0..k).map(|i| (n * i / k, n * (i + 1) / k)).collect()
}

fn stamp(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}

/// what a chunk's pixels depend on
fn chunk_key(fb: &FrameBuilder, r: Res, a: usize, b: usize) -> String {
    let p = &fb.plan;
    let fd = p.frames_dir(r);
    let mut s = format!("{} {}x{} ", r.tag(), p.out_size(r).0, p.out_size(r).1);
    for (i, sh) in p.shots.iter().enumerate() {
        if p.fi(sh.t0) < b && p.fi(sh.t1) > a {
            s.push_str(&stamp(&fd, &format!(".shot_{i:03}")));
        }
    }
    for (i, o) in p.overlays.iter().enumerate() {
        if p.fi(o.t0) < b && p.fi(o.t1) > a {
            s.push_str(&stamp(&fd, &format!(".ov_{i:02}")));
        }
    }
    if fb.font.is_some() {
        s.push_str(&p.font.display().to_string());
    }
    for f in a..b {
        s.push_str(&fb.svg(f));
    }
    key_of(&s)
}

/// Render chunk `k` (frames `a..b`) unless it is current; a killed render is retried `retries` times.
pub fn render_chunk(fb: &FrameBuilder, r: Res, k: usize, (a, b): (usize, usize), retries: usize) -> Result<(PathBuf, bool)> {
    let p = &fb.plan;
    let work = chunks_dir(p, r);
    std::fs::create_dir_all(&work)?;
    let part = work.join(format!("part_{k:03}.mp4"));
    let key_file = work.join(format!("part_{k:03}.key"));
    let key = chunk_key(fb, r, a, b);
    if part.exists() && std::fs::read_to_string(&key_file).is_ok_and(|s| s == key) {
        return Ok((part, false));
    }
    let media = work.join(format!("media_{k:03}"));
    let _ = std::fs::remove_dir_all(&media);
    std::fs::create_dir_all(&media)?;
    let src = p.frames_dir(r);
    let mut names: Vec<String> = (a..b).flat_map(|f| p.rasters_at(f)).collect();
    names.sort();
    names.dedup();
    for nm in &names {
        let from = src.join(nm);
        anyhow::ensure!(from.exists(), "frame raster {nm} is missing (run `promo plate`)");
        if std::fs::hard_link(&from, media.join(nm)).is_err() {
            std::fs::copy(&from, media.join(nm))?;
        }
    }
    if fb.font.is_some() {
        std::fs::copy(&p.font, media.join(p.font.file_name().unwrap()))?;
    }
    let tmp = work.join(format!("part_{k:03}.tmp.mp4"));
    let log = work.join(format!("part_{k:03}.log"));
    let mut last = String::new();
    for attempt in 1..=retries.max(1) {
        let _ = std::fs::remove_file(&tmp);
        let st = Command::new(std::env::current_exe()?)
            .args(["render", &format!("{a}f..{b}f"), "--scale", &r.scale().to_string(), "-o"])
            .arg(&tmp)
            .env(PASSTHROUGH, "1")
            .env(PLAN_ENV, p.plan_json())
            .env(MEDIA_ENV, &media)
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log)?)
            .status()?;
        if st.success() && tmp.exists() {
            std::fs::rename(&tmp, &part)?;
            std::fs::write(&key_file, &key)?;
            let _ = std::fs::remove_dir_all(&media);
            return Ok((part, true));
        }
        last = format!("chunk {k} (frames {a}..{b}) attempt {attempt}: {st} (log {})", log.display());
        eprintln!("render: {last}");
    }
    anyhow::bail!("render failed: {last}")
}

/// Every chunk in order (or only `only`). Returns the parts.
pub fn render_all(fb: &FrameBuilder, r: Res, chunk_frames: usize, retries: usize, only: Option<usize>, ev: &Events) -> Result<Vec<PathBuf>> {
    let rs = ranges(&fb.plan, chunk_frames);
    let mut parts = vec![];
    for (k, range) in rs.iter().enumerate() {
        if only.is_some_and(|o| o != k) {
            continue;
        }
        let (part, rendered) = render_chunk(fb, r, k, *range, retries)?;
        ev.log(&format!("render: chunk {}/{} frames {}..{} {}", k + 1, rs.len(), range.0, range.1, if rendered { "rendered" } else { "current" }));
        ev.progress(Stage::Render, k + 1, rs.len());
        parts.push(part);
    }
    // parts of an older, longer cut must not be joined
    if only.is_none() {
        for e in std::fs::read_dir(chunks_dir(&fb.plan, r))? {
            let p = e?.path();
            let stale = p.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix("part_")).and_then(|n| n.get(..3)).and_then(|n| n.parse::<usize>().ok()).is_some_and(|i| i >= rs.len());
            if stale {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    Ok(parts)
}

/// Join the chunks without re-encoding and mux the mastered audio. The MP4 is written to the system temp folder and
/// then copied (`+faststart` rewrites the file in place, which fails on some network/FUSE filesystems).
pub fn mux(p: &Plan, r: Res, chunk_frames: usize, out: &Path) -> Result<()> {
    let work = chunks_dir(p, r);
    let n = ranges(p, chunk_frames).len();
    let mut list = String::new();
    for k in 0..n {
        let part = work.join(format!("part_{k:03}.mp4"));
        anyhow::ensure!(part.exists(), "chunk {k} is not rendered yet (promo render)");
        list.push_str(&format!("file '{}'\n", part.display()));
    }
    std::fs::write(work.join("parts.txt"), list)?;
    anyhow::ensure!(p.audio_wav().exists(), "no mastered audio yet (promo audio)");
    let tmp = std::env::temp_dir().join(format!("promo_{}_{}.mp4", p.name, std::process::id()));
    let args: Vec<String> = vec![
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        work.join("parts.txt").display().to_string(),
        "-i".into(),
        p.audio_wav().display().to_string(),
        "-map".into(),
        "0:v".into(),
        "-map".into(),
        "1:a".into(),
        "-c:v".into(),
        "copy".into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "192k".into(),
        "-t".into(),
        format!("{:.4}", p.seconds),
        "-movflags".into(),
        "+faststart".into(),
        tmp.display().to_string(),
    ];
    datavid::media::ffmpeg(&args, "the concat / audio mux")?;
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::copy(&tmp, out).with_context(|| format!("copying the MP4 to {}", out.display()))?;
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}
