//! QA on the delivered MP4, written to `build/qa/` as `QA.md` (people) and `qa.json` (tools) with `datavid::qa::Report`:
//! container (duration within one frame, size, fps, an audio stream), loudness and true peak of the encoded audio
//! (EBU R128, measured in Rust), black frames, the smallest caption size (only when captions are burned in), a contact
//! sheet and, with captions on, one frame per caption.
use crate::config::Res;
use crate::frame::FrameBuilder;
use crate::plan::Plan;
use anyhow::Result;
use datavid::media;
use datavid::qa::Report;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaSummary {
    pub ok: bool,
    pub duration_s: f64,
    pub width: u64,
    pub height: u64,
    pub integrated_lufs: f64,
    pub true_peak_dbtp: f64,
    pub lra: f64,
    pub black_frames: Vec<(f64, f64)>,
    pub min_caption_px: Option<f64>,
    pub failures: Vec<String>,
    pub report: String,
    pub contact_sheet: String,
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
    rep.check("audio stream", !a.is_null(), &format!("{} {} Hz", a["codec_name"], a["sample_rate"].as_str().unwrap_or("?")));
    rep.put("container", json!({"duration_s": dur, "video": v, "audio": a}));
    // ---- loudness of the encoded audio
    let l = media::loudness_of(video, &dir.join("decoded.tmp.wav"))?;
    rep.check("integrated loudness", (l.integrated_lufs - p.audio.lufs).abs() <= p.audio.lufs_tolerance, &format!("{:.2} LUFS (target {} ± {})", l.integrated_lufs, p.audio.lufs, p.audio.lufs_tolerance));
    rep.check("true peak", l.true_peak_dbtp <= p.audio.true_peak_max, &format!("{:.2} dBTP (max {})", l.true_peak_dbtp, p.audio.true_peak_max));
    rep.put("loudness_mp4", serde_json::to_value(&l)?);
    // ---- picture
    let blacks = media::black_frames(video, p.qa.black_min_dur, p.qa.black_pix_th)?;
    rep.check("black frames", blacks.is_empty(), &if blacks.is_empty() { "none".into() } else { format!("{blacks:?}") });
    let fb = FrameBuilder::new(p.clone())?;
    let min_cap = fb.min_caption_size();
    match min_cap {
        Some(m) => rep.check("min caption size", m >= p.qa.min_text_px, &format!("{m:.1} px at 1080p (min {})", p.qa.min_text_px)),
        None => rep.md.push_str("- captions are off (not burned in): no text-size check\n"),
    }
    let sheet = dir.join("contact.png");
    media::contact_sheet(video, &sheet, dur.max(frame), 6, 4, 180)?;
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
        min_caption_px: min_cap,
        failures: rep.failures.clone(),
        report: dir.join("QA.md").display().to_string(),
        contact_sheet: sheet.display().to_string(),
    };
    Ok(s)
}
