//! Media helpers around the `ffmpeg` / `ffprobe` CLIs: run ffmpeg with a readable error, probe a file, measure the
//! loudness of an encoded file (decoded by ffmpeg, measured in Rust with EBU R128), detect black frames, cut a contact
//! sheet and a single frame. Shared by the stories and the promo template so QA means the same thing everywhere.
use crate::audio::master::{Loudness, measure};
use crate::audio::read_wav;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;
use std::process::Command;

/// Run `ffmpeg -y -v error <args>`; on failure the error names `what` and carries ffmpeg's stderr.
pub fn ffmpeg<S: AsRef<std::ffi::OsStr>>(args: &[S], what: &str) -> Result<()> {
    let out = Command::new("ffmpeg").args(["-y", "-v", "error"]).args(args).output().context("running ffmpeg (is it on PATH?)")?;
    if !out.status.success() {
        bail!("ffmpeg failed for {what}:\n{}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

/// `ffprobe -of json` with the format duration and every stream's basic fields.
pub fn probe(file: &Path) -> Result<Value> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration:stream=codec_type,codec_name,width,height,r_frame_rate,nb_frames,sample_rate,channels,pix_fmt", "-of", "json"])
        .arg(file)
        .output()
        .context("running ffprobe")?;
    if !out.status.success() {
        bail!("ffprobe failed for {}: {}", file.display(), String::from_utf8_lossy(&out.stderr));
    }
    serde_json::from_slice(&out.stdout).context("parsing ffprobe output")
}

/// Duration in seconds (container), `None` when ffprobe reports none.
pub fn duration(file: &Path) -> Result<Option<f64>> {
    Ok(probe(file)?["format"]["duration"].as_str().and_then(|s| s.parse().ok()))
}

/// Whether the file has at least one audio stream.
pub fn has_audio(file: &Path) -> Result<bool> {
    Ok(probe(file)?["streams"].as_array().is_some_and(|s| s.iter().any(|x| x["codec_type"] == "audio")))
}

/// Decode any file's audio to 48 kHz stereo (optionally from `start` for `dur` seconds) and return the samples.
pub fn decode_audio(file: &Path, tmp_wav: &Path, start: f64, dur: Option<f64>) -> Result<[Vec<f64>; 2]> {
    let mut args: Vec<String> = vec![];
    if start > 0.0 {
        args.extend(["-ss".into(), format!("{start:.3}")]);
    }
    args.extend(["-i".into(), file.display().to_string()]);
    if let Some(d) = dur {
        args.extend(["-t".into(), format!("{d:.3}")]);
    }
    args.extend(["-vn", "-ac", "2", "-ar", "48000", "-c:a", "pcm_s16le"].map(String::from));
    args.push(tmp_wav.display().to_string());
    ffmpeg(&args, &format!("decoding the audio of {}", file.display()))?;
    let (sr, x) = read_wav(tmp_wav)?;
    let _ = std::fs::remove_file(tmp_wav);
    anyhow::ensure!(sr == 48000, "unexpected sample rate {sr} after decoding {}", file.display());
    Ok(x)
}

/// EBU R128 loudness of an encoded file's own audio (what the viewer hears, AAC overshoot included).
pub fn loudness_of(file: &Path, tmp_wav: &Path) -> Result<Loudness> {
    let x = decode_audio(file, tmp_wav, 0.0, None)?;
    measure(&x, 48000)
}

/// Black stretches `(start, end)` in seconds (`blackdetect`, `min_dur` seconds, pixel threshold `pix_th`).
pub fn black_frames(file: &Path, min_dur: f64, pix_th: f64) -> Result<Vec<(f64, f64)>> {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(file)
        .args(["-vf", &format!("blackdetect=d={min_dur}:pix_th={pix_th}"), "-an", "-f", "null", "-"])
        .output()
        .context("running ffmpeg blackdetect")?;
    let log = String::from_utf8_lossy(&out.stderr);
    let num = |l: &str, key: &str| l.split(key).nth(1).and_then(|r| r.split_whitespace().next()).and_then(|v| v.parse::<f64>().ok());
    Ok(log.lines().filter(|l| l.contains("black_start")).filter_map(|l| Some((num(l, "black_start:")?, num(l, "black_end:")?))).collect())
}

/// A `cols`×`rows` contact sheet spread evenly over `seconds`, each cell `cell_w` px wide.
pub fn contact_sheet(file: &Path, out: &Path, seconds: f64, cols: usize, rows: usize, cell_w: u32) -> Result<()> {
    let cells = (cols * rows) as f64;
    let vf = format!("fps={:.5},scale={cell_w}:-2,tile={cols}x{rows}", cells / seconds.max(0.1));
    let args: Vec<String> = ["-i".into(), file.display().to_string(), "-vf".into(), vf, "-frames:v".into(), "1".into(), "-update".into(), "1".into(), out.display().to_string()].to_vec();
    ffmpeg(&args, "the contact sheet")
}

/// One frame at `t` seconds, scaled to `width` px (keeps the aspect).
pub fn frame_at(file: &Path, t: f64, out: &Path, width: u32) -> Result<()> {
    let args: Vec<String> =
        ["-ss".into(), format!("{t:.3}"), "-i".into(), file.display().to_string(), "-frames:v".into(), "1".into(), "-update".into(), "1".into(), "-vf".into(), format!("scale={width}:-2"), out.display().to_string()].to_vec();
    ffmpeg(&args, &format!("the frame at {t:.2} s"))
}
