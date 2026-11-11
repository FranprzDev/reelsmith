//! Plates: the per-frame rasters fframes composites. fframes takes no per-frame raster input from a video file in this
//! harness, so the ffmpeg CLI cuts each shot into JPG frames at the output size (cover-fit, optional blur) and each
//! motion overlay into cropped RGBA PNG frames at its drawn width, one ffmpeg per plate and several plates at once.
//! Every shot / overlay is stamped with its parameters, so a rerun only re-cuts what changed. Overlay crops given as
//! `"auto"` / `"full"` are measured here and cached in `build/crops.json`.
use crate::config::{CropMode, Fit, Res};
use crate::encode::Backend;
use crate::error::MissingInput;
use crate::plan::{Overlay, Plan, file_fingerprint, key_of};
use crate::pool;
use crate::progress::{Events, Stage};
use anyhow::{Context, Result};
use datavid::media::{self, ffmpeg};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// skip work whose parameters did not change since the last run
fn fresh(stamp: &Path, key: &str) -> bool {
    std::fs::read_to_string(stamp).is_ok_and(|s| s == key)
}

/// Which plates to cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Only {
    All,
    Shots,
    Overlays,
}

/// One plate to cut: a shot (by index) or a motion overlay (by index).
#[derive(Debug, Clone, Copy)]
enum Cut {
    Shot(usize),
    Overlay(usize),
}

/// Cut the plates, `jobs` ffmpeg processes at a time (each shot and overlay is independent); the source clips are
/// decoded with the backend's hardware decoder when it has one.
pub fn run(plan: &mut Plan, r: Res, only: Only, jobs: usize, backend: &Backend, ev: &Events) -> Result<()> {
    measure_crops_with_jobs(plan, jobs)?;
    let dir = plan.frames_dir(r);
    std::fs::create_dir_all(&dir)?;
    let mut cuts = vec![];
    if only != Only::Overlays {
        cuts.extend((0..plan.shots.len()).map(Cut::Shot));
    }
    if only != Only::Shots {
        cuts.extend((0..plan.overlays.len()).map(Cut::Overlay));
    }
    let plan: &Plan = plan;
    let total = cuts.len();
    let done = AtomicUsize::new(0);
    pool::map(jobs, &cuts, |c| {
        let msg = match *c {
            Cut::Shot(i) => cut_shot(plan, r, i, backend)?,
            Cut::Overlay(i) => cut_overlay(plan, r, i)?,
        };
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(m) = msg {
            ev.log(&m);
            ev.progress(Stage::Plate, n, total);
        }
        Ok(())
    })?;
    Ok(())
}

/// Cut shot `i` into `bg_%05d.jpg` frames unless its stamp is current; returns the log line when it was cut.
fn cut_shot(plan: &Plan, r: Res, i: usize, backend: &Backend) -> Result<Option<String>> {
    let sh = &plan.shots[i];
    let dir = plan.frames_dir(r);
    let s = r.scale();
    let (ow, oh) = plan.out_size(r);
    let fps = plan.fps;
    let (a, b) = (plan.fi(sh.t0), plan.fi(sh.t1));
    let n = b - a;
    let key = key_of(&format!("{} {} {} {a} {n} {ow}x{oh} {} {:?} {:?}", sh.clip.display(), file_fingerprint(&sh.clip), sh.src_in, sh.blur, sh.focus, plan.look.fit));
    let stamp = dir.join(format!(".shot_{i:03}"));
    if n == 0 || (fresh(&stamp, &key) && dir.join(Plan::bg_name(b - 1)).exists()) {
        return Ok(None);
    }
    if !sh.clip.is_file() {
        return Err(MissingInput(format!("clip {}", sh.clip.display())).into());
    }
    let [fx, fy] = sh.focus;
    let mut vf = match plan.look.fit {
        Fit::Cover => format!("fps={fps},scale={ow}:{oh}:force_original_aspect_ratio=increase:flags=lanczos,crop={ow}:{oh}:(iw-{ow})*{fx}:(ih-{oh})*{fy}"),
        Fit::Stretch => format!("fps={fps},scale={ow}:{oh}:flags=lanczos"),
    };
    if sh.blur > 0.0 {
        vf.push_str(&format!(",gblur=sigma={:.1}", sh.blur * s));
    }
    let mut args = backend.decode_args();
    args.extend([
        "-ss".into(),
        format!("{:.3}", sh.src_in),
        "-i".into(),
        sh.clip.display().to_string(),
        "-frames:v".into(),
        n.to_string(),
        "-vf".into(),
        vf,
        "-q:v".into(),
        "2".into(),
        "-start_number".into(),
        a.to_string(),
        dir.join("bg_%05d.jpg").display().to_string(),
    ]);
    ffmpeg(&args, &sh.clip.display().to_string())?;
    for f in a..b {
        anyhow::ensure!(dir.join(Plan::bg_name(f)).exists(), "shot {i} ({}) runs out of source at frame {f}: lower `in` or shorten the shot", sh.clip.display());
    }
    std::fs::write(&stamp, &key)?;
    Ok(Some(format!("plate: shot {i:02} {} -> frames {a}..{b}", sh.clip.file_name().unwrap_or_default().to_string_lossy())))
}

/// Cut overlay `i` into cropped `ov_<i>_%04d.png` frames unless its stamp is current; returns the log line when it was cut.
fn cut_overlay(plan: &Plan, r: Res, i: usize) -> Result<Option<String>> {
    let o = &plan.overlays[i];
    let dir = plan.frames_dir(r);
    let n = plan.fi(o.t1) - plan.fi(o.t0);
    let [cx, cy, cw, ch] = o.crop_box.context("overlay crop not measured")?;
    let pw = ((o.w * r.scale()) / 2.0).round() as u32 * 2;
    let key = key_of(&format!("{} {} {n} {cx} {cy} {cw} {ch} {pw}", o.asset.display(), file_fingerprint(&o.asset)));
    let stamp = dir.join(format!(".ov_{i:02}"));
    if n == 0 || (fresh(&stamp, &key) && dir.join(Plan::ov_name(i, n - 1)).exists()) {
        return Ok(None);
    }
    if !o.asset.is_file() {
        return Err(MissingInput(format!("motion asset {}", o.asset.display())).into());
    }
    let vf = format!("fps={},crop={cw}:{ch}:{cx}:{cy},scale={pw}:-2:flags=lanczos,format=rgba", plan.fps);
    let pat = dir.join(format!("ov_{i:02}_%04d.png"));
    let args: Vec<String> = ["-i".into(), o.asset.display().to_string(), "-frames:v".into(), n.to_string(), "-vf".into(), vf, "-start_number".into(), "0".into(), pat.display().to_string()].to_vec();
    ffmpeg(&args, &o.label())?;
    // an asset shorter than its slot holds its last frame
    let mut last = None;
    for k in 0..n {
        let p = dir.join(Plan::ov_name(i, k));
        if p.exists() {
            last = Some(p);
        } else if let Some(l) = &last {
            std::fs::copy(l, &p)?;
        }
    }
    anyhow::ensure!(last.is_some(), "overlay {} produced no frames", o.label());
    std::fs::write(&stamp, &key)?;
    Ok(Some(format!("plate: overlay {i:02} {} -> {n} frames at {pw} px", o.label())))
}

/// Resolve every `"auto"` / `"full"` crop (cached by asset path in `build/crops.json`).
pub fn measure_crops(plan: &mut Plan) -> Result<()> {
    measure_crops_with_jobs(plan, 1)
}

/// Resolve missing crops using up to `jobs` parallel measurements.
fn measure_crops_with_jobs(plan: &mut Plan, jobs: usize) -> Result<()> {
    let path = plan.crops_json();
    let mut cache: BTreeMap<String, [u32; 4]> = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let build = plan.build_dir.clone();

    // Existing explicit crops need no work; cached crops are assigned without probing.
    let pending_indices: Vec<usize> = plan
        .overlays
        .iter()
        .enumerate()
        .filter(|(_, o)| o.crop_box.is_none())
        .map(|(i, _)| i)
        .collect();

    for &i in &pending_indices {
        let k = plan.overlays[i].crop_key();
        if let Some(b) = cache.get(&k) {
            plan.overlays[i].crop_box = Some(*b);
        }
    }

    // Deduplicate the remaining (uncached) pending overlays by crop_key.
    let uncached: Vec<(usize, String)> = pending_indices
        .iter()
        .filter(|&&i| plan.overlays[i].crop_box.is_none())
        .map(|&i| (i, plan.overlays[i].crop_key()))
        .collect();
    let tasks = dedup_pending_crops(&uncached);

    if tasks.is_empty() {
        return Ok(());
    }

    // Build measurement inputs (cloned overlays) in first-occurrence order.
    let inputs: Vec<(usize, crate::plan::Overlay, String)> = tasks
        .iter()
        .map(|(key, members)| (members[0], plan.overlays[members[0]].clone(), key.clone()))
        .collect();

    let results = pool::map(jobs, &inputs, |(idx, o, _key)| measure(o, &build, *idx))?;

    // Assign and cache only after every measurement succeeded.
    for ((key, members), b) in tasks.iter().zip(results.iter()) {
        for &i in members {
            plan.overlays[i].crop_box = Some(*b);
        }
        cache.insert(key.clone(), *b);
    }

    std::fs::create_dir_all(&build)?;
    std::fs::write(&path, serde_json::to_string_pretty(&cache)?)?;
    Ok(())
}

/// Deduplicate pending (overlay_index, crop_key) pairs by crop_key, preserving first occurrence.
fn dedup_pending_crops(items: &[(usize, String)]) -> Vec<(String, Vec<usize>)> {
    let mut order: Vec<String> = Vec::new();
    let mut members: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, key) in items {
        if !members.contains_key(key) {
            order.push(key.clone());
        }
        members.entry(key.clone()).or_default().push(*idx);
    }
    order
        .into_iter()
        .map(|k| (k.clone(), members.remove(&k).unwrap()))
        .collect()
}

/// Parse `(width, height, duration)` from a single ffprobe JSON value.
fn parse_asset_info(v: &serde_json::Value) -> Result<(u32, u32, Option<f64>)> {
    let stream = v["streams"]
        .as_array()
        .and_then(|s| s.iter().find(|x| x["codec_type"] == "video"))
        .context("asset has no video stream")?;
    let w = stream["width"].as_u64().unwrap_or(0) as u32;
    let h = stream["height"].as_u64().unwrap_or(0) as u32;
    let dur = v["format"]["duration"].as_str().and_then(|s| s.parse().ok());
    Ok((w, h, dur))
}

fn asset_info(o: &Overlay) -> Result<(u32, u32, Option<f64>)> {
    parse_asset_info(&media::probe(&o.asset)?)
}

/// the union of the non-transparent pixels over 8 frames spread across the asset (padded 4 px, even size)
fn measure(o: &Overlay, build: &Path, scan_idx: usize) -> Result<[u32; 4]> {
    let (w, h, dur_opt) = asset_info(o)?;
    if matches!(o.crop, crate::config::Crop::Mode(CropMode::Full)) {
        return Ok([0, 0, w, h]);
    }
    let dur = dur_opt.unwrap_or(1.0).max(0.1);
    let tmp = build.join(format!("cropscan_{}", scan_idx));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let result = (|| {
        let args: Vec<String> = ["-i".into(), o.asset.display().to_string(), "-vf".into(), format!("fps={:.4},format=rgba", 8.0 / dur), "-frames:v".into(), "8".into(), tmp.join("f_%02d.png").display().to_string()].to_vec();
        ffmpeg(&args, &format!("measuring the crop of {}", o.label()))?;
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for e in std::fs::read_dir(&tmp)? {
            let p = e?.path();
            let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&p)?));
            let mut rd = dec.read_info()?;
            let mut buf = vec![0; rd.output_buffer_size()];
            let info = rd.next_frame(&mut buf)?;
            if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
                return Ok([0, 0, w, h]);
            }
            for y in 0..info.height {
                let row = &buf[y as usize * info.line_size..][..info.width as usize * 4];
                for x in 0..info.width {
                    if row[x as usize * 4 + 3] > 16 {
                        x0 = x0.min(x);
                        x1 = x1.max(x);
                        y0 = y0.min(y);
                        y1 = y1.max(y);
                    }
                }
            }
        }
        if x0 == u32::MAX {
            return Ok([0, 0, w, h]);
        }
        let pad = 4;
        let (x0, y0) = (x0.saturating_sub(pad), y0.saturating_sub(pad));
        let (x1, y1) = ((x1 + pad + 1).min(w), (y1 + pad + 1).min(h));
        let (cw, ch) = ((x1 - x0) & !1, (y1 - y0) & !1);
        Ok([x0, y0, cw.max(2), ch.max(2)])
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_video_and_duration() {
        let v = serde_json::json!({
            "streams": [{"codec_type": "audio"}, {"codec_type": "video", "width": 1920, "height": 1080}],
            "format": {"duration": "12.5"}
        });
        assert_eq!(parse_asset_info(&v).unwrap(), (1920, 1080, Some(12.5)));
    }

    #[test]
    fn missing_video_is_error() {
        let v = serde_json::json!({
            "streams": [{"codec_type": "audio"}],
            "format": {"duration": "1.0"}
        });
        assert!(parse_asset_info(&v).is_err());
    }
}
