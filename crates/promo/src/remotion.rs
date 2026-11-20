use crate::config::Res;
use crate::error::ConfigError;
use crate::font::FontInfo;
use crate::plan::Plan;
use crate::progress::Events;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

const PACKAGE_JSON: &[u8] = include_bytes!("../assets/remotion/package.json");
const TSCONFIG_JSON: &[u8] = include_bytes!("../assets/remotion/tsconfig.json");
const REMOTION_CONFIG_TS: &[u8] = include_bytes!("../assets/remotion/remotion.config.ts");
const INDEX_TS: &[u8] = include_bytes!("../assets/remotion/src/index.ts");
const ROOT_TSX: &[u8] = include_bytes!("../assets/remotion/src/Root.tsx");
const MAIN_TSX: &[u8] = include_bytes!("../assets/remotion/src/Main.tsx");

pub fn project_dir(p: &Plan, r: Res) -> PathBuf {
    p.build_dir.join(format!("remotion_{}", r.tag()))
}

fn link(src: &Path, dir: &Path, name: &str) -> Result<()> {
    let dst = dir.join(name);
    if dst.symlink_metadata().is_ok() {
        std::fs::remove_file(&dst)?;
    }
    link_file(src, &dst).with_context(|| format!("linking {} into the Remotion project", src.display()))?;
    Ok(())
}

#[cfg(unix)]
fn link_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

#[cfg(windows)]
fn link_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::copy(src, dst).map(|_| ())
}

pub fn write_project(p: &Plan, r: Res) -> Result<PathBuf> {
    if !p.overlays.is_empty() {
        return Err(ConfigError(format!(
            "the remotion engine does not support alpha motion overlays yet ({} in this config); use engine = \"fframes\"",
            p.overlays.len()
        ))
        .into());
    }
    let dir = project_dir(p, r);
    let src = dir.join("src");
    let assets = dir.join("public").join("assets");
    std::fs::create_dir_all(&src)?;
    std::fs::create_dir_all(&assets)?;
    std::fs::write(dir.join("package.json"), PACKAGE_JSON)?;
    std::fs::write(dir.join("tsconfig.json"), TSCONFIG_JSON)?;
    std::fs::write(dir.join("remotion.config.ts"), REMOTION_CONFIG_TS)?;
    std::fs::write(src.join("index.ts"), INDEX_TS)?;
    std::fs::write(src.join("Root.tsx"), ROOT_TSX)?;
    std::fs::write(src.join("Main.tsx"), MAIN_TSX)?;
    std::fs::copy(&p.font, assets.join("font.ttf")).with_context(|| format!("copying the caption font {}", p.font.display()))?;
    let mut shots = Vec::with_capacity(p.shots.len());
    for (i, s) in p.shots.iter().enumerate() {
        let ext = s.clip.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let name = format!("shot_{i:03}{ext}");
        link(&s.clip, &assets, &name)?;
        let from = p.fi(s.t0);
        shots.push(serde_json::json!({
            "src": format!("assets/{name}"),
            "from": from,
            "dur": (p.fi(s.t1) - from).max(1),
            "startFrom": p.fi(s.src_in),
            "z0": s.z0,
            "z1": s.z1,
        }));
    }
    let mut captions = Vec::new();
    if p.captions_on {
        let fi = FontInfo::load(&p.font)?;
        let k = r.scale();
        for c in &p.captions {
            let size = (fi.fit(&c.text, p.caption_style.size, p.caption_max_w) * k).round().max(8.0);
            let from = p.fi(c.t0);
            captions.push(serde_json::json!({
                "text": &c.text,
                "from": from,
                "dur": (p.fi(c.t1) - from).max(1),
                "size": size,
            }));
        }
    }
    let flashes: Vec<serde_json::Value> = p
        .flashes
        .iter()
        .map(|t| {
            serde_json::json!({
                "from": p.fi(*t),
                "dur": (p.look.flash_dur * p.fps as f64).round().max(1.0) as usize,
                "opacity": p.look.flash_opacity,
            })
        })
        .collect();
    let audio = if p.audio_wav().is_file() {
        link(&p.audio_wav(), &assets, "audio.wav")?;
        serde_json::Value::String("assets/audio.wav".into())
    } else {
        serde_json::Value::Null
    };
    let (w, h) = p.out_size(r);
    let cap_bottom = ((p.height - p.caption_y) * r.scale()).round();
    let props = serde_json::json!({
        "durationInFrames": p.frames().max(1),
        "fps": p.fps,
        "width": w,
        "height": h,
        "shots": shots,
        "captions": captions,
        "caption": {
            "bottom": cap_bottom,
            "fill": &p.caption_style.fill,
            "outline": &p.caption_style.outline,
            "uppercase": p.caption_style.uppercase,
        },
        "flashes": flashes,
        "audio": audio,
    });
    std::fs::write(dir.join("plan.json"), serde_json::to_string_pretty(&props)?)?;
    Ok(dir)
}

pub fn render(p: &Plan, r: Res, out: &Path, jobs: usize, ev: &Events) -> Result<()> {
    let dir = write_project(p, r)?;
    match Command::new("npx").arg("--version").output() {
        Ok(o) if o.status.success() => {}
        _ => return Err(ConfigError("the remotion engine needs Node.js 22+ with npx on PATH".into()).into()),
    }
    if !dir.join("node_modules").is_dir() {
        ev.log("remotion: installing the project dependencies (first render with this engine)");
        let status = Command::new("npm")
            .args(["install", "--no-audit", "--no-fund"])
            .current_dir(&dir)
            .status()
            .context("running npm install for the Remotion project")?;
        if !status.success() {
            return Err(ConfigError(format!("npm install exited with {status}")).into());
        }
    }
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d)?;
    }
    let status = Command::new("npx")
        .args(["remotion", "render", "src/index.ts", "Plan"])
        .arg(out)
        .arg(format!("--props={}", dir.join("plan.json").display()))
        .arg(format!("--concurrency={}", jobs.max(1)))
        .current_dir(&dir)
        .status()
        .context("running npx remotion render")?;
    if !status.success() {
        return Err(ConfigError(format!("remotion render exited with {status}")).into());
    }
    if !out.is_file() {
        return Err(ConfigError(format!("remotion render did not write {}", out.display())).into());
    }
    ev.log(&format!("mux: {}", out.display()));
    Ok(())
}
