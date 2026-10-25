use crate::clips;
use anyhow::{Context, Result, bail};
use promo::error::{ConfigError, MissingInput};
use resvg::tiny_skia::Pixmap;
use resvg::usvg;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    Ascii,
    Dither,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StylizeClipArgs {
    #[schemars(description = "absolute path of the video to stylize")]
    pub input: PathBuf,
    #[schemars(description = "ascii: each cell becomes a glyph chosen by brightness; dither: ordered Bayer dithering")]
    pub style: Style,
    #[schemars(description = "ascii: glyph cell size in output pixels (4 to 64, default 10); dither: pixelation block (1 to 32, default 4)")]
    pub cell_px: Option<u32>,
    #[schemars(description = "keep the source colors (default true) or go monochrome (white on black)")]
    pub color: Option<bool>,
    #[schemars(description = "scale the output to this width in px (default: the source width)")]
    pub width: Option<u32>,
    #[schemars(description = "process at most this many seconds (default: the whole clip)")]
    pub duration_s: Option<f64>,
    #[schemars(description = "output path, an .mp4; put it in the project's clips folder to use it as a beat clip")]
    pub output: PathBuf,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StylizedClip {
    pub output: PathBuf,
    pub asset_name: String,
    pub preview_png: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub frames: usize,
    pub duration_s: f64,
    pub style: Style,
}

fn cfg(msg: String) -> anyhow::Error {
    ConfigError(msg).into()
}

struct Probe {
    w: u32,
    h: u32,
    fps: f64,
    fps_rate: String,
    audio: bool,
}

fn probe(input: &PathBuf) -> Result<Probe> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,r_frame_rate", "-of", "csv=p=0"])
        .arg(input)
        .output()
        .context("probing the input (is ffprobe on PATH?)")?;
    let line = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or_default().to_string();
    let parts: Vec<&str> = line.split(',').collect();
    if parts.len() < 3 {
        bail!("ffprobe could not read a video stream from {}", input.display());
    }
    let (w, h) = (parts[0].parse::<u32>()?, parts[1].parse::<u32>()?);
    let fps_rate = parts[2].to_string();
    let fps = match parts[2].split_once('/') {
        Some((n, d)) => n.parse::<f64>().unwrap_or(30.0) / d.parse::<f64>().unwrap_or(1.0),
        None => parts[2].parse::<f64>().unwrap_or(30.0),
    };
    let a = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a:0", "-show_entries", "stream=index", "-of", "csv=p=0"])
        .arg(input)
        .output()
        .context("probing the audio")?;
    Ok(Probe { w, h, fps, fps_rate, audio: !a.stdout.is_empty() })
}

fn even(x: f64) -> u32 {
    ((x / 2.0).round() as u32).max(1) * 2
}

const RAMP: &[char] = &[' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];

fn glyph_masks(cell: u32) -> Result<Vec<Vec<u8>>> {
    let mut opt = usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let mut masks = Vec::new();
    for ch in RAMP {
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{cell}\" height=\"{cell}\"><text x=\"0\" y=\"{y}\" font-family=\"Courier New, Menlo, Liberation Mono, DejaVu Sans Mono, Noto Sans Mono, monospace\" font-size=\"{cell}\" fill=\"white\">{ch}</text></svg>",
            y = (cell as f64 * 0.82).round() as u32
        );
        let tree = usvg::Tree::from_str(&svg, &opt).map_err(|e| cfg(format!("rasterizing the glyph atlas (needs an installed monospace font): {e}")))?;
        let mut pm = Pixmap::new(cell, cell).context("allocating the glyph atlas")?;
        resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pm.as_mut());
        let mask: Vec<u8> = pm.data().chunks_exact(4).map(|px| px[3]).collect();
        masks.push(mask);
    }
    Ok(masks)
}

const BAYER8: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

fn ascii_frame(src: &[u8], w: usize, h: usize, cell: usize, masks: &[Vec<u8>], color: bool, out: &mut Vec<u8>) {
    out.clear();
    out.resize(w * h * 4, 0);
    for px in out.chunks_exact_mut(4) {
        px[3] = 255;
    }
    for cy in 0..h / cell {
        for cx in 0..w / cell {
            let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
            for y in cy * cell..(cy + 1) * cell {
                for x in cx * cell..(cx + 1) * cell {
                    let i = (y * w + x) * 4;
                    r += src[i] as u64;
                    g += src[i + 1] as u64;
                    b += src[i + 2] as u64;
                }
            }
            let n = (cell * cell) as u64;
            let (r, g, b) = ((r / n) as u8, (g / n) as u8, (b / n) as u8);
            let (fr, fg, fb) = if color { (r, g, b) } else { (255, 255, 255) };
            for y in 0..cell {
                for x in 0..cell {
                    let i = ((cy * cell + y) * w + cx * cell + x) * 4;
                    out[i] = fr / 5;
                    out[i + 1] = fg / 5;
                    out[i + 2] = fb / 5;
                }
            }
            let lum = ((0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64) / 255.0).powf(0.7);
            let gi = (lum * (masks.len() - 1) as f64).round() as usize;
            if gi == 0 {
                continue;
            }
            let mask = &masks[gi];
            for y in 0..cell {
                for x in 0..cell {
                    let a = mask[y * cell + x] as u32;
                    if a == 0 {
                        continue;
                    }
                    let i = ((cy * cell + y) * w + cx * cell + x) * 4;
                    let inv = 255 - a;
                    out[i] = ((fr as u32 * a + out[i] as u32 * inv) / 255) as u8;
                    out[i + 1] = ((fg as u32 * a + out[i + 1] as u32 * inv) / 255) as u8;
                    out[i + 2] = ((fb as u32 * a + out[i + 2] as u32 * inv) / 255) as u8;
                    out[i + 3] = 255;
                }
            }
        }
    }
}

fn dither_frame(src: &[u8], w: usize, h: usize, block: usize, color: bool, out: &mut Vec<u8>) {
    out.clear();
    out.resize(w * h * 4, 0);
    for by in 0..(h + block - 1) / block {
        for bx in 0..(w + block - 1) / block {
            let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
            let mut n = 0u64;
            for y in by * block..((by + 1) * block).min(h) {
                for x in bx * block..((bx + 1) * block).min(w) {
                    let i = (y * w + x) * 4;
                    r += src[i] as u64;
                    g += src[i + 1] as u64;
                    b += src[i + 2] as u64;
                    n += 1;
                }
            }
            let (r, g, b) = ((r / n) as f64, (g / n) as f64, (b / n) as f64);
            for y in by * block..((by + 1) * block).min(h) {
                for x in bx * block..((bx + 1) * block).min(w) {
                    let t = (BAYER8[y % 8][x % 8] as f64 + 0.5) / 64.0 * 255.0;
                    let i = (y * w + x) * 4;
                    if color {
                        out[i] = if r > t { 255 } else { 0 };
                        out[i + 1] = if g > t { 255 } else { 0 };
                        out[i + 2] = if b > t { 255 } else { 0 };
                    } else {
                        let v = if 0.2126 * r + 0.7152 * g + 0.0722 * b > t { 255 } else { 0 };
                        out[i] = v;
                        out[i + 1] = v;
                        out[i + 2] = v;
                    }
                    out[i + 3] = 255;
                }
            }
        }
    }
}

fn spawn(args: &[String]) -> Result<Child> {
    Command::new("ffmpeg").args(args).stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().context("running ffmpeg (is it on PATH?)")
}

pub fn stylize(a: &StylizeClipArgs) -> Result<StylizedClip> {
    clips::ensure_ffmpeg()?;
    if !a.input.is_file() {
        return Err(MissingInput(format!("video {}", a.input.display())).into());
    }
    if a.output.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() != Some("mp4") {
        return Err(cfg(format!("output must be an .mp4 file, got {}", a.output.display())));
    }
    let p = probe(&a.input)?;
    let (w, h) = match a.width {
        Some(tw) => {
            if !(160..=1920).contains(&tw) {
                return Err(cfg(format!("width must be between 160 and 1920, got {tw}")));
            }
            (even(tw as f64), even(p.h as f64 * tw as f64 / p.w as f64))
        }
        None => (even(p.w as f64), even(p.h as f64)),
    };
    let cell = a.cell_px.unwrap_or(match a.style {
        Style::Ascii => 10,
        Style::Dither => 4,
    }) as usize;
    let (lo, hi) = match a.style {
        Style::Ascii => (4, 64),
        Style::Dither => (1, 32),
    };
    if !(lo..=hi).contains(&cell) {
        return Err(cfg(format!("cell_px must be between {lo} and {hi} for {:?}, got {cell}", a.style)));
    }
    let color = a.color.unwrap_or(true);
    let masks = if a.style == Style::Ascii { Some(glyph_masks(cell as u32)?) } else { None };
    if let Some(dir) = a.output.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let preview = a.output.with_extension("preview.png");
    let size = format!("{w}x{h}");
    let mut dec_args = vec!["-v".into(), "error".into()];
    if let Some(d) = a.duration_s {
        dec_args.extend(["-t".into(), d.to_string()]);
    }
    dec_args.extend(["-i".into(), a.input.to_string_lossy().into_owned(), "-vf".into(), format!("scale={w}:{h}"), "-f".into(), "rawvideo".into(), "-pix_fmt".into(), "rgba".into(), "-".into()]);
    let mut dec = Command::new("ffmpeg").args(&dec_args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().context("running ffmpeg (is it on PATH?)")?;
    let mut enc_args = vec![
        "-y".into(), "-v".into(), "error".into(),
        "-f".into(), "rawvideo".into(), "-pix_fmt".into(), "rgba".into(), "-s".into(), size.clone(), "-framerate".into(), p.fps_rate.clone(), "-i".into(), "-".into(),
        "-i".into(), a.input.to_string_lossy().into_owned(),
        "-map".into(), "0:v".into(),
    ];
    if p.audio {
        enc_args.extend(["-map".into(), "1:a?".into(), "-c:a".into(), "aac".into()]);
    }
    if let Some(d) = a.duration_s {
        enc_args.extend(["-t".into(), d.to_string()]);
    }
    enc_args.extend(["-c:v".into(), "libx264".into(), "-pix_fmt".into(), "yuv420p".into(), "-crf".into(), "20".into(), "-preset".into(), "fast".into(), "-movflags".into(), "+faststart".into(), a.output.to_string_lossy().into_owned()]);
    let mut enc = spawn(&enc_args)?;
    let mut din = dec.stdout.take().context("ffmpeg stdout")?;
    let mut eout = enc.stdin.take().context("ffmpeg stdin")?;
    let frame_bytes = w as usize * h as usize * 4;
    let mut raw = vec![0u8; frame_bytes];
    let mut out = Vec::with_capacity(frame_bytes);
    let mut frames = 0usize;
    let mut write_err = None;
    loop {
        match din.read_exact(&mut raw) {
            Ok(()) => {}
            Err(_) => break,
        }
        match a.style {
            Style::Ascii => ascii_frame(&raw, w as usize, h as usize, cell, masks.as_ref().unwrap(), color, &mut out),
            Style::Dither => dither_frame(&raw, w as usize, h as usize, cell, color, &mut out),
        }
        frames += 1;
        if frames == 1 {
            if let Ok(pm) = Pixmap::from_vec(out.clone(), resvg::tiny_skia::IntSize::from_wh(w, h).unwrap()).context("preview raster") {
                pm.save_png(&preview).with_context(|| format!("writing {}", preview.display()))?;
            }
        }
        if let Err(e) = eout.write_all(&out) {
            write_err = Some(e);
            break;
        }
    }
    drop(eout);
    let estatus = enc.wait_with_output().context("waiting for the encoder")?;
    let dstatus = dec.wait_with_output().context("waiting for the decoder")?;
    if frames == 0 {
        bail!("{} decoded no frames", a.input.display());
    }
    if !estatus.status.success() || write_err.is_some() {
        bail!("ffmpeg failed encoding {}:\n{}", a.output.display(), String::from_utf8_lossy(&estatus.stderr));
    }
    if !dstatus.status.success() {
        bail!("ffmpeg failed decoding {}:\n{}", a.input.display(), String::from_utf8_lossy(&dstatus.stderr));
    }
    let asset_name = a.output.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    Ok(StylizedClip { output: a.output.clone(), asset_name, preview_png: preview, width: w, height: h, fps: p.fps, frames, duration_s: frames as f64 / p.fps, style: a.style })
}
