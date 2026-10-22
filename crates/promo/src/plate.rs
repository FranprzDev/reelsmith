//! Plates: the per-frame rasters fframes composites. fframes takes no per-frame raster input from a video file in this
//! harness, so the ffmpeg CLI cuts each shot into JPG frames at the output size (cover-fit, optional blur) and each
//! motion overlay into cropped RGBA PNG frames at its drawn width. Every shot / overlay is stamped with its parameters,
//! so a rerun only re-cuts what changed. Overlay crops given as `"auto"` / `"full"` are measured here and cached in
//! `build/crops.json`.
use crate::config::{CropMode, Fit, Res};
use crate::error::MissingInput;
use crate::plan::{Overlay, Plan, key_of};
use crate::progress::{Events, Stage};
use anyhow::{Context, Result};
use datavid::media::{self, ffmpeg};
use std::collections::BTreeMap;
use std::path::Path;

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

pub fn run(plan: &mut Plan, r: Res, only: Only, ev: &Events) -> Result<()> {
    measure_crops(plan)?;
    let dir = plan.frames_dir(r);
    std::fs::create_dir_all(&dir)?;
    let s = r.scale();
    let (ow, oh) = plan.out_size(r);
    let fps = plan.fps;
    let total = plan.shots.len() + plan.overlays.len();
    let mut done = 0;
    if only != Only::Overlays {
        for (i, sh) in plan.shots.iter().enumerate() {
            let (a, b) = (plan.fi(sh.t0), plan.fi(sh.t1));
            let n = b - a;
            let key = key_of(&format!("{} {} {a} {n} {ow}x{oh} {} {:?} {:?}", sh.clip.display(), sh.src_in, sh.blur, sh.focus, plan.look.fit));
            let stamp = dir.join(format!(".shot_{i:03}"));
            done += 1;
            if n == 0 || (fresh(&stamp, &key) && dir.join(Plan::bg_name(b - 1)).exists()) {
                continue;
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
            let args: Vec<String> = [
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
            ]
            .to_vec();
            ffmpeg(&args, &sh.clip.display().to_string())?;
            for f in a..b {
                anyhow::ensure!(dir.join(Plan::bg_name(f)).exists(), "shot {i} ({}) runs out of source at frame {f}: lower `in` or shorten the shot", sh.clip.display());
            }
            std::fs::write(&stamp, &key)?;
            ev.log(&format!("plate: shot {i:02} {} -> frames {a}..{b}", sh.clip.file_name().unwrap_or_default().to_string_lossy()));
            ev.progress(Stage::Plate, done, total);
        }
    } else {
        done += plan.shots.len();
    }
    if only != Only::Shots {
        for (i, o) in plan.overlays.iter().enumerate() {
            done += 1;
            let n = plan.fi(o.t1) - plan.fi(o.t0);
            let [cx, cy, cw, ch] = o.crop_box.context("overlay crop not measured")?;
            let pw = ((o.w * s) / 2.0).round() as u32 * 2;
            let key = key_of(&format!("{} {n} {cx} {cy} {cw} {ch} {pw}", o.asset.display()));
            let stamp = dir.join(format!(".ov_{i:02}"));
            if n == 0 || (fresh(&stamp, &key) && dir.join(Plan::ov_name(i, n - 1)).exists()) {
                continue;
            }
            if !o.asset.is_file() {
                return Err(MissingInput(format!("motion asset {}", o.asset.display())).into());
            }
            let vf = format!("fps={fps},crop={cw}:{ch}:{cx}:{cy},scale={pw}:-2:flags=lanczos,format=rgba");
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
            ev.log(&format!("plate: overlay {i:02} {} -> {n} frames at {pw} px", o.label()));
            ev.progress(Stage::Plate, done, total);
        }
    }
    Ok(())
}

/// Resolve every `"auto"` / `"full"` crop (cached by asset path in `build/crops.json`).
pub fn measure_crops(plan: &mut Plan) -> Result<()> {
    let path = plan.crops_json();
    let mut cache: BTreeMap<String, [u32; 4]> = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let mut changed = false;
    for o in plan.overlays.iter_mut() {
        if o.crop_box.is_some() {
            continue;
        }
        let k = o.crop_key();
        if let Some(b) = cache.get(&k) {
            o.crop_box = Some(*b);
            continue;
        }
        if !o.asset.is_file() {
            return Err(MissingInput(format!("motion asset {}", o.asset.display())).into());
        }
        let b = measure(o, &plan.build_dir)?;
        o.crop_box = Some(b);
        cache.insert(k, b);
        changed = true;
    }
    if changed {
        std::fs::create_dir_all(&plan.build_dir)?;
        std::fs::write(&path, serde_json::to_string_pretty(&cache)?)?;
    }
    Ok(())
}

fn asset_size(o: &Overlay) -> Result<(u32, u32)> {
    let p = media::probe(&o.asset)?;
    let v = p["streams"].as_array().and_then(|s| s.iter().find(|x| x["codec_type"] == "video").cloned()).context("asset has no video stream")?;
    Ok((v["width"].as_u64().unwrap_or(0) as u32, v["height"].as_u64().unwrap_or(0) as u32))
}

/// the union of the non-transparent pixels over 8 frames spread across the asset (padded 4 px, even size)
fn measure(o: &Overlay, build: &Path) -> Result<[u32; 4]> {
    let (w, h) = asset_size(o)?;
    if matches!(o.crop, crate::config::Crop::Mode(CropMode::Full)) {
        return Ok([0, 0, w, h]);
    }
    let dur = media::duration(&o.asset)?.unwrap_or(1.0).max(0.1);
    let tmp = build.join("cropscan");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
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
            let _ = std::fs::remove_dir_all(&tmp);
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
    let _ = std::fs::remove_dir_all(&tmp);
    if x0 == u32::MAX {
        return Ok([0, 0, w, h]);
    }
    let pad = 4;
    let (x0, y0) = (x0.saturating_sub(pad), y0.saturating_sub(pad));
    let (x1, y1) = ((x1 + pad + 1).min(w), (y1 + pad + 1).min(h));
    let (cw, ch) = ((x1 - x0) & !1, (y1 - y0) & !1);
    Ok([x0, y0, cw.max(2), ch.max(2)])
}
