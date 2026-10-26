use anyhow::{Context, Result};
use fframes::{AudioMap, Color, CpuFrameRenderer, Duration, FFramesContext, Frame, Previewer, Svgr, Video};
use crate::config::{Aspect, Res};
use crate::encode::Backend;
use crate::frame::FrameBuilder;
use crate::plan::{Plan, key_of};
use crate::pool;
use crate::progress::{Events, Stage};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

const PART_EXT: &str = "mp4";
const PART_CODEC: &str = "h264-rgba-v1";
const CHUNK_ENV: &str = "PROMO_CHUNK";

#[derive(serde::Serialize, serde::Deserialize)]
struct ChunkRequest {
    start: usize,
    end: usize,
    resolution: Res,
}

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

fn stream_frames<'m, const W: usize, const H: usize>(fb: FrameBuilder, mp: &'m dyn fframes::MediaProvider<'m>) -> Result<ExitCode> {
    let video = PromoVideo::<W, H> { fb };
    let request: ChunkRequest = serde_json::from_str(&std::env::var(CHUNK_ENV).context("PROMO_CHUNK is not set")?)?;
    anyhow::ensure!(request.start < request.end && request.end <= video.fb.plan.frames(), "invalid chunk frame range");
    let options = fframes::RenderOptions { media: Some(mp), scale_resolution: request.resolution.scale(), ..Default::default() };
    let mut previewer = Previewer::new(&video, &options)?;
    let mut renderer = CpuFrameRenderer::default();
    let mut stdout = std::io::stdout().lock();
    for frame in request.start..request.end {
        stdout.write_all(&previewer.render(frame, &mut renderer)?.pixels)?;
    }
    stdout.flush()?;
    Ok(ExitCode::SUCCESS)
}

/// The fframes CLI on the plan in `PROMO_PLAN`, with the media folder in `PROMO_MEDIA`.
pub fn passthrough() -> Result<ExitCode> {
    let plan = Plan::load_json(Path::new(&std::env::var(PLAN_ENV).context("PROMO_PLAN is not set")?))?;
    let mdir = PathBuf::from(std::env::var(MEDIA_ENV).context("PROMO_MEDIA is not set")?);
    let media = fframes::MediaDirectory::read_folder(&mdir).with_context(|| format!("media folder {}", mdir.display()))?;
    let mp = media.process_media_source().map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let fb = FrameBuilder::new(plan)?;
    match fb.plan.aspect {
        Aspect::Vertical => stream_frames::<1080, 1920>(fb, &mp),
        Aspect::Horizontal => stream_frames::<1920, 1080>(fb, &mp),
        Aspect::Square => stream_frames::<1080, 1080>(fb, &mp),
    }
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
fn chunk_key(fb: &FrameBuilder, r: Res, a: usize, b: usize, backend: &Backend) -> String {
    let p = &fb.plan;
    let fd = p.frames_dir(r);
    let (w, h) = p.out_size(r);
    let mut s = format!("{PART_CODEC} {} {w}x{h} {} {:?} {:?}", r.tag(), p.fps, backend.encoder.args(r, w, h, p.fps), backend.device);
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
pub fn render_chunk(fb: &FrameBuilder, r: Res, k: usize, (a, b): (usize, usize), retries: usize, backend: &Backend) -> Result<(PathBuf, bool)> {
    let p = &fb.plan;
    let work = chunks_dir(p, r);
    std::fs::create_dir_all(&work)?;
    let part = work.join(format!("part_{k:03}.{PART_EXT}"));
    let key_file = work.join(format!("part_{k:03}.key"));
    let key = chunk_key(fb, r, a, b, backend);
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
    let tmp = work.join(format!("part_{k:03}.tmp.{PART_EXT}"));
    let log = work.join(format!("part_{k:03}.log"));
    let mut last = String::new();
    for attempt in 1..=retries.max(1) {
        let _ = std::fs::remove_file(&tmp);
        let mut producer = Command::new(std::env::current_exe()?)
            .env(PASSTHROUGH, "1")
            .env(PLAN_ENV, p.plan_json())
            .env(MEDIA_ENV, &media)
            .env(CHUNK_ENV, serde_json::to_string(&ChunkRequest { start: a, end: b, resolution: r })?)
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(&log)?)
            .spawn()?;
        let (w, h) = p.out_size(r);
        let mut args = vec!["-hide_banner".to_string(), "-v".into(), "error".into(), "-y".into()];
        args.extend(backend.encoder.input_args(backend.device.as_deref()));
        args.extend(["-f".into(), "rawvideo".into(), "-pixel_format".into(), "rgba".into(), "-video_size".into(), format!("{w}x{h}"), "-framerate".into(), p.fps.to_string(), "-i".into(), "pipe:0".into(), "-an".into(), "-frames:v".into(), (b - a).to_string()]);
        let tail = backend.encoder.filter_tail();
        if !tail.is_empty() {
            args.extend(["-vf".into(), tail.trim_start_matches(',').into()]);
        }
        args.extend(backend.encoder.args(r, w, h, p.fps));
        args.push(tmp.display().to_string());
        let encoder = Command::new("ffmpeg")
            .args(args)
            .stdin(producer.stdout.take().context("the chunk renderer has no pixel stream")?)
            .stdout(Stdio::null())
            .stderr(std::fs::OpenOptions::new().append(true).open(&log)?)
            .spawn();
        let st = match encoder {
            Ok(mut encoder) => match encoder.wait() {
                Ok(status) => status,
                Err(error) => {
                    let _ = encoder.kill();
                    let _ = encoder.wait();
                    let _ = producer.kill();
                    let _ = producer.wait();
                    return Err(error.into());
                }
            },
            Err(error) => {
                let _ = producer.kill();
                let _ = producer.wait();
                return Err(error.into());
            }
        };
        if !st.success() {
            let _ = producer.kill();
        }
        let rendered = producer.wait()?;
        if st.success() && rendered.success() && tmp.exists() {
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

/// Every chunk (or only `only`), `jobs` fframes processes at a time. Returns the parts in order.
pub fn render_all(fb: &FrameBuilder, r: Res, chunk_frames: usize, retries: usize, only: Option<usize>, jobs: usize, backend: &Backend, ev: &Events) -> Result<Vec<PathBuf>> {
    let rs = ranges(&fb.plan, chunk_frames);
    let todo: Vec<usize> = (0..rs.len()).filter(|k| only.is_none_or(|o| o == *k)).collect();
    let done = AtomicUsize::new(0);
    let parts = pool::map(jobs, &todo, |&k| {
        let range = rs[k];
        let (part, rendered) = render_chunk(fb, r, k, range, retries, backend)?;
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        ev.log(&format!("render: chunk {}/{} frames {}..{} {}", k + 1, rs.len(), range.0, range.1, if rendered { "rendered" } else { "current" }));
        ev.progress(Stage::Render, n, todo.len());
        Ok(part)
    })?;
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

pub fn mux(p: &Plan, r: Res, chunk_frames: usize, _backend: &Backend, out: &Path) -> Result<()> {
    let work = chunks_dir(p, r);
    let ranges = ranges(p, chunk_frames);
    let mut list = String::new();
    for (k, &(a, b)) in ranges.iter().enumerate() {
        let part = work.join(format!("part_{k:03}.{PART_EXT}"));
        anyhow::ensure!(part.exists(), "chunk {k} is not rendered yet (promo render)");
        let path = part.display().to_string().replace('\\', "\\\\").replace('\'', "'\\''");
        list.push_str(&format!("file '{path}'\nduration {:.12}\n", (b - a) as f64 / p.fps as f64));
    }
    std::fs::write(work.join("parts.txt"), list)?;
    anyhow::ensure!(p.audio_wav().exists(), "no mastered audio yet (promo audio)");
    let tmp = std::env::temp_dir().join(format!("promo_{}_{}.mp4", p.name, std::process::id()));
    let mut args: Vec<String> = ["-f", "concat", "-safe", "0", "-i"].map(String::from).to_vec();
    args.extend([
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
    ]);
    datavid::media::ffmpeg(&args, "the video stream-copy / audio mux")?;
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::copy(&tmp, out).with_context(|| format!("copying the MP4 to {}", out.display()))?;
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}
