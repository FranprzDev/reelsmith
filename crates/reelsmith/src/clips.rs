//! The clip library: what is in a local folder and what each file is (duration, resolution, fps, alpha), read with the
//! harness's own ffprobe helper.
use anyhow::{Context, Result, bail};
use datavid::media;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What a media file can be used for in a promo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MediaRole {
    /// opaque footage: a shot (`clip` of a beat or the end card)
    Clip,
    /// video with an alpha channel: a motion-graphics overlay (`overlay.asset`, `end_card.asset`)
    Motion,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ClipInfo {
    /// file stem: the name a brief uses (`clip = "<name>"` or `"<name>@<in-point>"`)
    pub name: String,
    pub path: PathBuf,
    pub role: MediaRole,
    pub duration_s: Option<f64>,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub fps: Option<f64>,
    pub video_codec: Option<String>,
    pub pixel_format: Option<String>,
    pub has_audio: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ClipList {
    pub folder: PathBuf,
    pub clips: Vec<ClipInfo>,
    /// files with a video extension that ffprobe could not read
    pub unreadable: Vec<String>,
}

/// ffmpeg and ffprobe must be on PATH: the plates, the mux and QA run them.
pub fn ensure_ffmpeg() -> Result<()> {
    for tool in ["ffmpeg", "ffprobe"] {
        let found = Command::new(tool).arg("-version").output().is_ok_and(|o| o.status.success());
        if !found {
            bail!(
                "`{tool}` was not found on PATH. Install FFmpeg (Windows: `winget install Gyan.FFmpeg`, macOS: `brew install ffmpeg`, Linux: your package manager) and restart the MCP host."
            );
        }
    }
    Ok(())
}

fn is_video(p: &Path) -> bool {
    let ext = p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    promo::plan::CLIP_EXTS.contains(&ext.as_str()) || promo::plan::MOTION_EXTS.contains(&ext.as_str())
}

fn ratio(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/')?;
    let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
    (d > 0.0).then(|| n / d)
}

/// Probe one file without re-checking that ffmpeg/ffprobe are on PATH.
fn probe_unchecked(path: &Path) -> Result<ClipInfo> {
    anyhow::ensure!(path.is_file(), "no such file: {}", path.display());
    let j = media::probe(path)?;
    let streams = j["streams"].as_array().cloned().unwrap_or_default();
    let v = streams.iter().find(|s| s["codec_type"] == "video").cloned().unwrap_or(Value::Null);
    let pix = v["pix_fmt"].as_str().map(String::from);
    let alpha = pix.as_deref().is_some_and(|p| p.starts_with("yuva") || p.contains("rgba") || p.contains("argb") || p.contains("bgra") || p.starts_with("gbrap") || p.starts_with("ya"));
    Ok(ClipInfo {
        name: path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string(),
        path: path.to_path_buf(),
        role: if alpha { MediaRole::Motion } else { MediaRole::Clip },
        duration_s: j["format"]["duration"].as_str().and_then(|s| s.parse().ok()),
        width: v["width"].as_u64(),
        height: v["height"].as_u64(),
        fps: v["r_frame_rate"].as_str().and_then(ratio),
        video_codec: v["codec_name"].as_str().map(String::from),
        pixel_format: pix,
        has_audio: streams.iter().any(|s| s["codec_type"] == "audio"),
    })
}

/// Probe one file, checking ffmpeg/ffprobe are on PATH first.
pub fn probe(path: &Path) -> Result<ClipInfo> {
    ensure_ffmpeg()?;
    probe_unchecked(path)
}

fn list_with_probe<F>(folder: &Path, probe: F) -> Result<ClipList>
where
    F: Fn(&Path) -> Result<ClipInfo> + Sync,
{
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .with_context(|| format!("reading the folder {}", folder.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && is_video(p))
        .collect();
    files.sort();
    let jobs = promo::pool::default_jobs().min(4).max(1);
    let results = promo::pool::map(jobs, &files, |f| Ok(probe(f)))?;
    let mut clips = Vec::with_capacity(files.len());
    let mut unreadable = Vec::new();
    for (f, r) in files.iter().zip(results) {
        match r {
            Ok(c) => clips.push(c),
            Err(e) => unreadable.push(format!("{}: {e:#}", f.display())),
        }
    }
    Ok(ClipList { folder: folder.to_path_buf(), clips, unreadable })
}

/// Every video file directly inside `folder`, sorted by name.
pub fn list(folder: &Path) -> Result<ClipList> {
    ensure_ffmpeg()?;
    list_with_probe(folder, probe_unchecked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn ratio_parses_and_rejects() {
        assert_eq!(ratio("30/1"), Some(30.0));
        assert_eq!(ratio("24000/1001"), Some(24000.0 / 1001.0));
        assert_eq!(ratio("1/0"), None);
        assert_eq!(ratio("abc/def"), None);
        assert_eq!(ratio("nope"), None);
        assert_eq!(ratio("1.5/2"), Some(0.75));
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reelsmith-clips-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn list_with_probe_preserves_order_and_unreadable() {
        let dir = tmp_dir("order");
        for f in ["b.mp4", "a.mp4", "c.mov", "skip.txt"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let got = list_with_probe(&dir, |p| {
            let name = p.file_stem().unwrap().to_str().unwrap().to_string();
            if name == "b" {
                anyhow::bail!("boom");
            }
            Ok(ClipInfo {
                name,
                path: p.to_path_buf(),
                role: MediaRole::Clip,
                duration_s: None,
                width: None,
                height: None,
                fps: None,
                video_codec: None,
                pixel_format: None,
                has_audio: false,
            })
        })
        .unwrap();
        assert_eq!(got.clips.len(), 2);
        assert_eq!(got.clips[0].name, "a");
        assert_eq!(got.clips[1].name, "c");
        assert_eq!(got.unreadable.len(), 1);
        assert!(got.unreadable[0].contains("b.mp4"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_with_probe_empty_folder() {
        let dir = tmp_dir("empty");
        let got = list_with_probe(&dir, |_| unreachable!()).unwrap();
        assert!(got.clips.is_empty());
        assert!(got.unreadable.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn probe_unchecked_missing_file_errors() {
        let p = PathBuf::from("/definitely/not/a/file.mp4");
        assert!(probe_unchecked(&p).is_err());
    }
}
