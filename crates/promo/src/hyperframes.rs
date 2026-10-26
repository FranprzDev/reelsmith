use crate::config::Res;
use crate::error::ConfigError;
use crate::font::FontInfo;
use crate::plan::Plan;
use crate::progress::Events;
use anyhow::{Context, Result};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const GSAP: &[u8] = include_bytes!("../assets/gsap.min.js");

pub fn project_dir(p: &Plan, r: Res) -> PathBuf {
    p.build_dir.join(format!("hyperframes_{}", r.tag()))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn composition(p: &Plan, r: Res, fi: &FontInfo) -> Result<String> {
    let (w, h) = p.out_size(r);
    let k = r.scale();
    let px = |v: f64| (v * k).round() as i32;
    let cap_bottom = px(p.height - p.caption_y);
    let mut vids = String::new();
    let mut anims = String::new();
    for (i, s) in p.shots.iter().enumerate() {
        writeln!(vids, "  <video class=\"clip\" id=\"shot{i}\" data-start=\"{:.6}\" data-duration=\"{:.6}\" data-track-index=\"0\" src=\"__SHOT_{i}__#t={:.3}\" muted playsinline></video>", s.t0, s.t1 - s.t0, s.src_in)?;
        if (s.z0 - s.z1).abs() > 1e-3 {
            writeln!(anims, "  tl.fromTo('#shot{i}', {{ scale: {:.4} }}, {{ scale: {:.4}, duration: {:.4}, ease: 'none', immediateRender: false }}, {:.4});", s.z0, s.z1, s.t1 - s.t0, s.t0)?;
        }
    }
    let mut caps = String::new();
    if p.captions_on {
        for (i, c) in p.captions.iter().enumerate() {
            let size = (fi.fit(&c.text, p.caption_style.size, p.caption_max_w) * k).round().max(8.0);
            writeln!(caps, "  <div class=\"cap\" id=\"cap{i}\" style=\"font-size:{size:.0}px\" data-start=\"{:.6}\" data-duration=\"{:.6}\">{}</div>", c.t0, c.t1 - c.t0, esc(&c.text))?;
            writeln!(anims, "  tl.fromTo('#cap{i}', {{ opacity: 0, y: 14 }}, {{ opacity: 1, y: 0, duration: 0.18, ease: 'power2.out', immediateRender: false }}, {:.4});", c.t0)?;
            writeln!(anims, "  tl.to('#cap{i}', {{ opacity: 0, duration: 0.12, immediateRender: false }}, {:.4});", (c.t1 - 0.12).max(c.t0))?;
        }
    }
    let mut flashes = String::new();
    for (i, t) in p.flashes.iter().enumerate() {
        writeln!(flashes, "  <div class=\"flash\" id=\"flash{i}\"></div>")?;
        writeln!(anims, "  tl.fromTo('#flash{i}', {{ opacity: {:.2} }}, {{ opacity: 0, duration: {:.3}, ease: 'power1.out', immediateRender: false }}, {t:.4});", p.look.flash_opacity, p.look.flash_dur)?;
    }
    let audio = if p.audio_wav().is_file() {
        format!("  <audio id=\"mix\" data-start=\"0\" data-duration=\"{:.6}\" data-track-index=\"2\" data-volume=\"1\" src=\"assets/audio.wav\"></audio>", p.seconds)
    } else {
        String::new()
    };
    let case = if p.caption_style.uppercase { "uppercase" } else { "none" };
    let mut html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="UTF-8" />
<script src="assets/gsap.min.js"></script>
<style>
  @font-face {{ font-family: '__FAMILY__'; src: url('assets/font.ttf'); }}
  * {{ margin: 0; padding: 0; box-sizing: border-box; }}
  html, body {{ width: __W__px; height: __H__px; overflow: hidden; background: #000; }}
  #stage {{ position: relative; width: __W__px; height: __H__px; font-family: '__FAMILY__', sans-serif; }}
  .clip {{ position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }}
  .cap {{ position: absolute; top: auto; left: 0; right: 0; bottom: __CAPB__px; text-align: center; color: __FILL__; line-height: 1.05; text-shadow: 0 2px 0 __OUTLINE__, 0 0 18px rgba(0,0,0,.85); opacity: 0; padding: 0 12px; text-transform: __CASE__; }}
  .flash {{ position: absolute; inset: 0; background: #fff; opacity: 0; }}
</style>
</head>
<body>
<div id="stage" data-composition-id="__NAME__" data-start="0" data-duration="__DUR__" data-width="__W__" data-height="__H__" data-fps="__FPS__">
__VIDS____CAPS____FLASHES__
__AUDIO__
</div>
<script>
  const tl = gsap.timeline({{ paused: true }});
__ANIMS__
  window.__timelines = window.__timelines || {{}};
  window.__timelines.__NAME__ = tl;
</script>
</body>
</html>
"#);
    let rep = [
        ("__FAMILY__", fi.family.clone()),
        ("__W__", w.to_string()),
        ("__H__", h.to_string()),
        ("__CAPB__", cap_bottom.to_string()),
        ("__FILL__", esc(&p.caption_style.fill)),
        ("__OUTLINE__", esc(&p.caption_style.outline)),
        ("__CASE__", case.to_string()),
        ("__NAME__", p.name.clone()),
        ("__DUR__", format!("{:.6}", p.seconds)),
        ("__FPS__", p.fps.to_string()),
        ("__VIDS__", vids),
        ("__CAPS__", caps),
        ("__FLASHES__", flashes),
        ("__ANIMS__", anims),
        ("__AUDIO__", audio),
    ];
    for (k, v) in rep {
        html = html.replace(k, &v);
    }
    for (i, s) in p.shots.iter().enumerate() {
        let ext = s.clip.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        html = html.replace(&format!("__SHOT_{i}__"), &format!("assets/shot_{i:03}{ext}"));
    }
    Ok(html)
}

fn link(src: &Path, dir: &Path, name: &str) -> Result<()> {
    let dst = dir.join(name);
    if dst.symlink_metadata().is_ok() {
        std::fs::remove_file(&dst)?;
    }
    std::os::unix::fs::symlink(src, &dst).with_context(|| format!("linking {} into the HyperFrames project", src.display()))?;
    Ok(())
}

pub fn write_project(p: &Plan, r: Res) -> Result<PathBuf> {
    if !p.overlays.is_empty() {
        return Err(ConfigError(format!(
            "the hyperframes engine does not support alpha motion overlays yet ({} in this config); use engine = \"fframes\"",
            p.overlays.len()
        ))
        .into());
    }
    let dir = project_dir(p, r);
    let assets = dir.join("assets");
    std::fs::create_dir_all(&assets)?;
    std::fs::write(assets.join("gsap.min.js"), GSAP)?;
    std::fs::copy(&p.font, assets.join("font.ttf")).with_context(|| format!("copying the caption font {}", p.font.display()))?;
    for (i, s) in p.shots.iter().enumerate() {
        let ext = s.clip.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        link(&s.clip, &assets, &format!("shot_{i:03}{ext}"))?;
    }
    if p.audio_wav().is_file() {
        link(&p.audio_wav(), &assets, "audio.wav")?;
    }
    std::fs::write(dir.join("hyperframes.json"), "{\n  \"$schema\": \"https://hyperframes.heygen.com/schema/hyperframes.json\",\n  \"paths\": { \"assets\": \"assets\" }\n}\n")?;
    let fi = FontInfo::load(&p.font)?;
    std::fs::write(dir.join("index.html"), composition(p, r, &fi)?)?;
    Ok(dir)
}

pub fn render(p: &Plan, r: Res, out: &Path, jobs: usize, ev: &Events) -> Result<()> {
    let dir = write_project(p, r)?;
    match Command::new("npx").arg("--version").output() {
        Ok(o) if o.status.success() => {}
        _ => return Err(ConfigError("the hyperframes engine needs Node.js 22+ with npx on PATH".into()).into()),
    }
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d)?;
    }
    let workers = jobs.max(1).to_string();
    let status = Command::new("npx")
        .args(["-y", "hyperframes@latest", "render"])
        .arg(&dir)
        .arg("-o")
        .arg(out)
        .arg("-w")
        .arg(&workers)
        .status()
        .context("running npx hyperframes render")?;
    if !status.success() {
        return Err(ConfigError(format!("hyperframes render exited with {status}")).into());
    }
    if !out.is_file() {
        return Err(ConfigError(format!("hyperframes render did not write {}", out.display())).into());
    }
    ev.log(&format!("mux: {}", out.display()));
    Ok(())
}
