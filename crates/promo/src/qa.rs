//! QA on the delivered MP4, written to `build/qa/` as `QA.md` (people) and `qa.json` (tools) with `datavid::qa::Report`:
//! container (duration within one frame, size, fps, an audio stream), loudness and true peak of the encoded audio
//! (EBU R128, measured in Rust), black frames, the smallest caption size (only when captions are burned in), a contact
//! sheet and, with captions on, one frame per caption.
use crate::config::Res;
use crate::frame::FrameBuilder;
use crate::plan::{file_fingerprint, Plan};
use anyhow::Result;
use datavid::media;
use datavid::qa::Report;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct QaSummary {
    pub ok: bool,
    pub duration_s: f64,
    pub width: u64,
    pub height: u64,
    pub integrated_lufs: f64,
    pub true_peak_dbtp: f64,
    pub lra: f64,
    pub black_frames: Vec<(f64, f64)>,
    /// median per-frame YDIF of each shot's source window (`None` when the window is too short to measure)
    pub shot_motion: Vec<Option<f64>>,
    pub min_caption_px: Option<f64>,
    pub failures: Vec<String>,
    pub report: String,
    pub contact_sheet: String,
}

/// What a QA verdict on `video` depends on. Bump the tag when the checks themselves change.
fn qa_key(p: &Plan, r: Res, video: &Path) -> String {
    let (w, h) = p.out_size(r);
    crate::plan::key_of(&format!(
        "qa-v1 {} {} {} {} {:.6} {} {} {} {} {} {} {} {} {} {}",
        file_fingerprint(video), r.tag(), p.fps, p.seconds, p.frames(), w, h, p.captions_on,
        p.qa.black_min_dur, p.qa.black_pix_th, p.qa.min_motion, p.qa.min_text_px, p.audio.lufs, p.audio.lufs_tolerance, p.audio.true_peak_max
    ))
}

/// QA for the build pipeline: when the MP4 and every QA parameter are unchanged since the last pass, return the
/// cached summary instead of decoding the whole video again (a full 1080p decode is the expensive part). Explicit
/// `promo qa` / MCP qa calls use [`run`], which always measures fresh.
pub fn run_cached(p: &Plan, r: Res, video: &Path) -> Result<(QaSummary, bool)> {
    let dir = p.qa_dir();
    let key = qa_key(p, r, video);
    if let (Ok(saved), Ok(summary)) = (std::fs::read_to_string(dir.join(".summary_key")), std::fs::read_to_string(dir.join("summary.json"))) {
        if saved.trim() == key {
            if let Ok(s) = serde_json::from_str::<QaSummary>(&summary) {
                return Ok((s, true));
            }
        }
    }
    let s = run(p, r, video)?;
    std::fs::write(dir.join("summary.json"), serde_json::to_string_pretty(&s)?)?;
    std::fs::write(dir.join(".summary_key"), key)?;
    Ok((s, false))
}

pub fn run(p: &Plan, r: Res, video: &Path) -> Result<QaSummary> {
    let dir = p.qa_dir();
    std::fs::create_dir_all(&dir)?;
    let mut rep = Report::new(&p.name);
    rep.section("Video file", &format!("`{}`", video.display()));
    // ---- container
    let pj = media::probe(video)?;
    let dur: f64 = pj["format"]["duration"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let streams = pj["streams"].as_array().cloned().unwrap_or_default();
    let v = streams.iter().find(|s| s["codec_type"] == "video").cloned().unwrap_or(Value::Null);
    let a = streams.iter().find(|s| s["codec_type"] == "audio").cloned().unwrap_or(Value::Null);
    let frame = 1.0 / p.fps as f64;
    rep.check("duration", (dur - p.seconds).abs() <= frame + 1e-3, &format!("{dur:.3} s (plan {:.3} s)", p.seconds));
    let (ew, eh) = p.out_size(r);
    let (vw, vh) = (v["width"].as_u64().unwrap_or(0), v["height"].as_u64().unwrap_or(0));
    rep.check("size", vw == ew as u64 && vh == eh as u64, &format!("{vw}×{vh} (expected {ew}×{eh}, {})", r.tag()));
    rep.check("fps", v["r_frame_rate"] == format!("{}/1", p.fps), &format!("{}", v["r_frame_rate"]));
    if let Some(n) = v["nb_frames"].as_str().and_then(|s| s.parse::<usize>().ok()) {
        rep.check("video frames", n == p.frames(), &format!("{n} (plan {})", p.frames()));
    }
    rep.check("audio stream", !a.is_null(), &format!("{} {} Hz", a["codec_name"], a["sample_rate"].as_str().unwrap_or("?")));
    rep.put("container", json!({"duration_s": dur, "video": v, "audio": a}));
    // ---- loudness of the encoded audio
    let l = media::loudness_of(video, &dir.join("decoded.tmp.wav"))?;
    rep.check("integrated loudness", (l.integrated_lufs - p.audio.lufs).abs() <= p.audio.lufs_tolerance, &format!("{:.2} LUFS (target {} ± {})", l.integrated_lufs, p.audio.lufs, p.audio.lufs_tolerance));
    rep.check("true peak", l.true_peak_dbtp <= p.audio.true_peak_max, &format!("{:.2} dBTP (max {})", l.true_peak_dbtp, p.audio.true_peak_max));
    rep.put("loudness_mp4", serde_json::to_value(&l)?);
    // ---- picture (black frames + contact sheet share one decode of the video)
    let sheet = dir.join("contact.png");
    let blacks = media::black_frames_and_contact_sheet(video, p.qa.black_min_dur, p.qa.black_pix_th, &sheet, dur.max(frame), 6, 4, 180)?;
    rep.check("black frames", blacks.is_empty(), &if blacks.is_empty() { "none".into() } else { format!("{blacks:?}") });
    // ---- motion: a shot whose source picture never changes is a photo, not a promo shot. Measured on the
    // shot's source window (median YDIF over 15 fps samples at 160 px), so the renderer's default push-in
    // can't mask it and cut-flash frames can't hide it: a frozen frame reads ~0.0005, real clips ~1+.
    let mut shot_motion: Vec<Option<f64>> = Vec::new();
    let mut stills: Vec<String> = Vec::new();
    // a shot's source motion does not depend on the render resolution, so the medians are cached across passes
    // (preview / draft / final) and reruns, keyed by the clip's fingerprint and the measured window
    let cache_path = dir.join("motion_cache.json");
    let mut cache: std::collections::BTreeMap<String, f64> = std::fs::read_to_string(&cache_path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let mut cache_changed = false;
    for (i, s) in p.shots.iter().enumerate() {
        let mkey = format!("{}|{}|{}|{}", s.clip.display(), file_fingerprint(&s.clip), s.src_in, s.t1 - s.t0);
        let median = match cache.get(&mkey) {
            Some(&m) => Some(m),
            None => {
                let mut vs: Vec<f64> = media::frame_motion(&s.clip, s.src_in, Some(s.t1 - s.t0))?.into_iter().map(|(_, v)| v).collect();
                if vs.len() < 2 {
                    None
                } else {
                    vs.sort_by(f64::total_cmp);
                    let m = vs[vs.len() / 2];
                    cache.insert(mkey, m);
                    cache_changed = true;
                    Some(m)
                }
            }
        };
        let Some(median) = median else {
            shot_motion.push(None);
            continue;
        };
        shot_motion.push(Some(median));
        if p.qa.min_motion > 0.0 && median < p.qa.min_motion {
            stills.push(format!("shot {i} ({:.1}-{:.1} s): median YDIF {median:.3}", s.t0, s.t1));
        }
    }
    if cache_changed {
        std::fs::write(&cache_path, serde_json::to_string_pretty(&cache)?)?;
    }
    rep.check(
        "still shots",
        stills.is_empty(),
        &if stills.is_empty() { format!("every shot's source moves (median YDIF >= {})", p.qa.min_motion) } else { stills.join("; ") },
    );
    rep.put("motion", json!({"min_motion": p.qa.min_motion, "shot_source_median_ydif": shot_motion}));
    let fb = FrameBuilder::new(p.clone())?;
    let min_cap = fb.min_caption_size();
    match min_cap {
        Some(m) => rep.check("min caption size", m >= p.qa.min_text_px, &format!("{m:.1} px at 1080p (min {})", p.qa.min_text_px)),
        None => rep.md.push_str("- captions are off (not burned in): no text-size check\n"),
    }
    if min_cap.is_some() {
        for (i, c) in p.captions.iter().enumerate() {
            media::frame_at(video, (c.t0 + 0.3).min(c.t1 - frame).max(c.t0), &dir.join(format!("cap_{i:02}.jpg")), 240)?;
        }
    }
    rep.save(&dir)?;
    let s = QaSummary {
        ok: rep.failures.is_empty(),
        duration_s: dur,
        width: vw,
        height: vh,
        integrated_lufs: l.integrated_lufs,
        true_peak_dbtp: l.true_peak_dbtp,
        lra: l.lra,
        black_frames: blacks,
        shot_motion,
        min_caption_px: min_cap,
        failures: rep.failures.clone(),
        report: dir.join("QA.md").display().to_string(),
        contact_sheet: sheet.display().to_string(),
    };
    Ok(s)
}
