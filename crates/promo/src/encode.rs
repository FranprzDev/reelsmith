//! The delivery encoder. fframes renders each chunk losslessly (FFV1 in Matroska); the mux joins the chunks and encodes
//! the delivery MP4 once with the `ffmpeg` on PATH, which is where the hardware encoders live (NVENC on NVIDIA GPUs under
//! Windows/Linux, VideoToolbox on macOS). `Auto` picks the first one that actually opens on this machine and falls back
//! to libx264.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::process::Command;

/// What the caller asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EncoderChoice {
    /// NVENC when an NVIDIA GPU answers, VideoToolbox on macOS, otherwise libx264
    #[default]
    Auto,
    Nvenc,
    Videotoolbox,
    X264,
}

/// The encoder a build actually used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoder {
    /// `h264_nvenc` (NVIDIA)
    Nvenc,
    /// `h264_videotoolbox` (Apple)
    Videotoolbox,
    /// `libx264` (CPU)
    X264,
}

impl Encoder {
    pub fn ffmpeg_name(self) -> &'static str {
        match self {
            Encoder::Nvenc => "h264_nvenc",
            Encoder::Videotoolbox => "h264_videotoolbox",
            Encoder::X264 => "libx264",
        }
    }

    /// ffmpeg arguments for a visually lossless H.264 delivery file of `w`×`h` at `fps`.
    pub fn args(self, w: u32, h: u32, fps: usize) -> Vec<String> {
        let mut a: Vec<String> = vec!["-c:v".into(), self.ffmpeg_name().into()];
        match self {
            Encoder::Nvenc => a.extend(["-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", "19", "-b:v", "0"].map(String::from)),
            Encoder::Videotoolbox => {
                // VideoToolbox has no constant-quality mode on every Mac: target ~0.15 bit per pixel
                let kbps = (w as f64 * h as f64 * fps as f64 * 0.15 / 1000.0).round() as u64;
                a.extend(["-b:v".into(), format!("{kbps}k"), "-maxrate".into(), format!("{}k", kbps * 2), "-bufsize".into(), format!("{}k", kbps * 2)]);
            }
            Encoder::X264 => a.extend(["-crf", "18", "-preset", "veryfast"].map(String::from)),
        }
        a.extend(["-pix_fmt", "yuv420p"].map(String::from));
        a
    }

    /// Whether the PATH ffmpeg can open this encoder here (a hardware encoder needs the device, not just the build).
    pub fn available(self) -> bool {
        Command::new("ffmpeg")
            .args(["-hide_banner", "-v", "error", "-f", "lavfi", "-i", "color=c=black:s=256x256:r=30", "-frames:v", "3", "-c:v", self.ffmpeg_name(), "-f", "null", "-"])
            .output()
            .is_ok_and(|o| o.status.success())
    }
}

impl EncoderChoice {
    /// The concrete encoder for this machine; an explicit choice that cannot open is an error, not a silent fallback.
    pub fn resolve(self) -> Result<Encoder> {
        let wanted = match self {
            EncoderChoice::Auto => {
                let hw = if cfg!(target_os = "macos") { Encoder::Videotoolbox } else { Encoder::Nvenc };
                if hw.available() {
                    return Ok(hw);
                }
                Encoder::X264
            }
            EncoderChoice::Nvenc => Encoder::Nvenc,
            EncoderChoice::Videotoolbox => Encoder::Videotoolbox,
            EncoderChoice::X264 => Encoder::X264,
        };
        if !wanted.available() {
            bail!("the ffmpeg on PATH cannot open the {} encoder on this machine (check `ffmpeg -encoders` and the GPU driver)", wanted.ffmpeg_name());
        }
        Ok(wanted)
    }
}
