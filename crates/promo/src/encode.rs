use crate::config::Res;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// What the caller asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum EncoderChoice {
    /// the first hardware encoder that opens on this machine (see the module table), otherwise libx264
    #[default]
    Auto,
    Nvenc,
    Amf,
    Qsv,
    Vaapi,
    Videotoolbox,
    X264,
}

/// The encoder a build actually used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Encoder {
    /// `h264_nvenc` (NVIDIA)
    Nvenc,
    /// `h264_amf` (AMD, Windows)
    Amf,
    /// `h264_qsv` (Intel Quick Sync, Windows)
    Qsv,
    /// `h264_vaapi` (AMD / Intel, Linux)
    Vaapi,
    /// `h264_videotoolbox` (Apple)
    Videotoolbox,
    /// `libx264` (CPU)
    X264,
}

/// The GPU vendor behind the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Gpu {
    Nvidia,
    Amd,
    Intel,
    Apple,
    /// a VA-API render node whose PCI vendor is neither AMD nor Intel
    Unknown,
    /// no GPU encoder opened: everything runs on the CPU
    None,
}

/// Hardware decoders used to cut the source clips (`ffmpeg -hwaccel <name>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum HwDecode {
    Cuda,
    D3d11va,
    Vaapi,
    Videotoolbox,
}

impl HwDecode {
    pub fn ffmpeg_name(self) -> &'static str {
        match self {
            HwDecode::Cuda => "cuda",
            HwDecode::D3d11va => "d3d11va",
            HwDecode::Vaapi => "vaapi",
            HwDecode::Videotoolbox => "videotoolbox",
        }
    }
}

/// One encoder or decoder test-open.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Probe {
    /// the ffmpeg name that was tried (`h264_nvenc`, `cuda`, ...)
    pub name: String,
    pub opened: bool,
    /// ffmpeg's first error line when it did not open
    pub error: Option<String>,
}

/// The backend a build uses, with every probe and the reason it was chosen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Backend {
    pub encoder: Encoder,
    pub gpu: Gpu,
    /// the hardware decoder used for the source clips (`None`: decoded on the CPU)
    pub hw_decode: Option<HwDecode>,
    /// the VA-API render node (Linux only)
    pub device: Option<PathBuf>,
    pub reason: String,
    pub probes: Vec<Probe>,
}

impl Encoder {
    pub fn ffmpeg_name(self) -> &'static str {
        match self {
            Encoder::Nvenc => "h264_nvenc",
            Encoder::Amf => "h264_amf",
            Encoder::Qsv => "h264_qsv",
            Encoder::Vaapi => "h264_vaapi",
            Encoder::Videotoolbox => "h264_videotoolbox",
            Encoder::X264 => "libx264",
        }
    }

    /// the encoders `Auto` tries on this OS, best first
    fn candidates() -> &'static [Encoder] {
        if cfg!(windows) {
            &[Encoder::Nvenc, Encoder::Amf, Encoder::Qsv, Encoder::X264]
        } else if cfg!(target_os = "macos") {
            &[Encoder::Videotoolbox, Encoder::X264]
        } else {
            &[Encoder::Nvenc, Encoder::Vaapi, Encoder::X264]
        }
    }

    /// the hardware decoder that pairs with this encoder on this OS
    fn decoder(self) -> Option<HwDecode> {
        match self {
            Encoder::Nvenc => Some(HwDecode::Cuda),
            Encoder::Amf | Encoder::Qsv => Some(HwDecode::D3d11va),
            Encoder::Vaapi => Some(HwDecode::Vaapi),
            Encoder::Videotoolbox => Some(HwDecode::Videotoolbox),
            Encoder::X264 => None,
        }
    }

    /// ffmpeg arguments placed before the inputs (the VA-API device)
    pub fn input_args(self, device: Option<&Path>) -> Vec<String> {
        match (self, device) {
            (Encoder::Vaapi, Some(d)) => vec!["-vaapi_device".into(), d.display().to_string()],
            _ => vec![],
        }
    }

    /// the filters appended to the video chain (VA-API encodes from GPU surfaces)
    pub fn filter_tail(self) -> &'static str {
        match self {
            Encoder::Vaapi => ",format=nv12,hwupload",
            _ => "",
        }
    }

    /// ffmpeg arguments for a visually lossless H.264 delivery file of `w`×`h` at `fps`, tuned per quality tier:
    /// preview favours speed, draft is balanced, final favours quality.
    pub fn args(self, r: Res, w: u32, h: u32, fps: usize) -> Vec<String> {
        let mut a: Vec<String> = vec!["-c:v".into(), self.ffmpeg_name().into()];
        let tier = |preview: &'static str, draft: &'static str, fin: &'static str| match r {
            Res::Preview => preview,
            Res::Draft => draft,
            Res::Final => fin,
        };
        match self {
            Encoder::Nvenc => {
                a.extend(["-preset", tier("p4", "p5", "p6"), "-tune", "hq", "-rc", "vbr", "-cq", tier("19", "19", "18"), "-b:v", "0", "-bf", "3", "-spatial_aq", "1"].map(String::from));
                if r == Res::Final {
                    a.extend(["-temporal_aq", "1", "-rc-lookahead", "32"].map(String::from));
                }
            }
            Encoder::Amf => a.extend(["-usage", "transcoding", "-quality", tier("speed", "balanced", "quality"), "-rc", "cqp", "-qp_i", tier("20", "19", "18"), "-qp_p", tier("22", "21", "20")].map(String::from)),
            Encoder::Qsv => a.extend(["-preset", tier("veryfast", "medium", "slower"), "-global_quality", tier("21", "20", "19")].map(String::from)),
            Encoder::Vaapi => a.extend(["-rc_mode", "CQP", "-qp", tier("21", "20", "19")].map(String::from)),
            Encoder::Videotoolbox => {
                // VideoToolbox has no constant-quality mode on every Mac: target ~0.15 bit per pixel
                let kbps = (w as f64 * h as f64 * fps as f64 * 0.15 / 1000.0).round() as u64;
                a.extend(["-b:v".into(), format!("{kbps}k"), "-maxrate".into(), format!("{}k", kbps * 2), "-bufsize".into(), format!("{}k", kbps * 2)]);
            }
            Encoder::X264 => a.extend(["-crf", "18", "-preset", tier("veryfast", "fast", "medium")].map(String::from)),
        }
        match self {
            // VA-API frames are already nv12 surfaces; QSV takes nv12
            Encoder::Vaapi => {}
            Encoder::Qsv => a.extend(["-pix_fmt", "nv12"].map(String::from)),
            _ => a.extend(["-pix_fmt", "yuv420p"].map(String::from)),
        }
        a
    }

    /// Test-open this encoder with a one-frame null encode.
    fn probe(self, device: Option<&Path>) -> Probe {
        if self == Encoder::Vaapi && device.is_none() {
            return Probe { name: self.ffmpeg_name().into(), opened: false, error: Some("no /dev/dri render node".into()) };
        }
        let mut args: Vec<String> = ["-hide_banner", "-v", "error"].map(String::from).to_vec();
        args.extend(self.input_args(device));
        args.extend(["-f", "lavfi", "-i", "color=c=black:s=256x256:r=30", "-frames:v", "1"].map(String::from));
        let tail = self.filter_tail();
        if !tail.is_empty() {
            args.extend(["-vf".into(), tail.trim_start_matches(',').to_string()]);
        }
        args.extend(self.args(Res::Preview, 256, 256, 30));
        args.extend(["-f", "null", "-"].map(String::from));
        run_probe(self.ffmpeg_name(), &args)
    }

    fn gpu(self, device: Option<&Path>) -> Gpu {
        match self {
            Encoder::Nvenc => Gpu::Nvidia,
            Encoder::Amf => Gpu::Amd,
            Encoder::Qsv => Gpu::Intel,
            Encoder::Vaapi => device.map_or(Gpu::Unknown, pci_vendor),
            Encoder::Videotoolbox => Gpu::Apple,
            Encoder::X264 => Gpu::None,
        }
    }
}

impl EncoderChoice {
    fn encoder(self) -> Option<Encoder> {
        match self {
            EncoderChoice::Auto => None,
            EncoderChoice::Nvenc => Some(Encoder::Nvenc),
            EncoderChoice::Amf => Some(Encoder::Amf),
            EncoderChoice::Qsv => Some(Encoder::Qsv),
            EncoderChoice::Vaapi => Some(Encoder::Vaapi),
            EncoderChoice::Videotoolbox => Some(Encoder::Videotoolbox),
            EncoderChoice::X264 => Some(Encoder::X264),
        }
    }

    /// Detect the backend for this machine. `Auto` takes the first encoder that opens; an explicit choice that cannot
    /// open is an error carrying ffmpeg's message.
    pub fn resolve(self) -> Result<Backend> {
        let device = render_node();
        let mut probes = vec![];
        let (encoder, reason) = match self.encoder() {
            Some(e) => {
                let p = e.probe(device.as_deref());
                let opened = p.opened;
                let err = p.error.clone().unwrap_or_default();
                probes.push(p);
                if !opened {
                    bail!("the ffmpeg on PATH cannot open the {} encoder on this machine: {err} (check `ffmpeg -encoders` and the GPU driver)", e.ffmpeg_name());
                }
                (e, format!("{} was asked for and opened", e.ffmpeg_name()))
            }
            None => {
                let mut chosen = None;
                for &e in Encoder::candidates() {
                    let p = e.probe(device.as_deref());
                    let opened = p.opened;
                    probes.push(p);
                    if opened {
                        chosen = Some(e);
                        break;
                    }
                }
                let Some(e) = chosen else { bail!("no H.264 encoder opened, not even libx264 (check the ffmpeg on PATH)") };
                let failed: Vec<&str> = probes.iter().filter(|p| !p.opened).map(|p| p.name.as_str()).collect();
                let reason = if failed.is_empty() {
                    format!("auto: {} is the first choice on this OS and opened", e.ffmpeg_name())
                } else {
                    format!("auto: {} did not open, {} did", failed.join(", "), e.ffmpeg_name())
                };
                (e, reason)
            }
        };
        let mut hw_decode = None;
        let mut reason = reason;
        if let Some(d) = encoder.decoder() {
            let p = probe_decoder(d, device.as_deref());
            if p.opened {
                hw_decode = Some(d);
            } else {
                reason.push_str(&format!("; {} decode did not open, clips are decoded on the CPU", d.ffmpeg_name()));
            }
            probes.push(p);
        }
        let device = if encoder == Encoder::Vaapi { device } else { None };
        Ok(Backend { encoder, gpu: encoder.gpu(device.as_deref()), hw_decode, device, reason, probes })
    }
}

impl Backend {
    /// ffmpeg arguments placed before a source clip's `-i` (hardware decode, frames copied back to system memory)
    pub fn decode_args(&self) -> Vec<String> {
        let Some(d) = self.hw_decode else { return vec![] };
        let mut a = vec!["-hwaccel".into(), d.ffmpeg_name().into()];
        if let (HwDecode::Vaapi, Some(dev)) = (d, &self.device) {
            a.extend(["-hwaccel_device".into(), dev.display().to_string()]);
        }
        a
    }

    /// one line for logs: what runs where and why
    pub fn summary(&self) -> String {
        let dec = self.hw_decode.map_or("cpu".to_string(), |d| d.ffmpeg_name().to_string());
        format!("backend: {} (gpu: {:?}), decode: {dec} — {}", self.encoder.ffmpeg_name(), self.gpu, self.reason)
    }
}

/// Test-open a hardware decoder on a two-frame H.264 stream made by libx264. Any warning counts as a failure: ffmpeg
/// quietly falls back to software decoding when a hwaccel cannot initialise.
fn probe_decoder(d: HwDecode, device: Option<&Path>) -> Probe {
    let sample = std::env::temp_dir().join(format!("promo_hwdec_probe_{}.h264", std::process::id()));
    let made = run_probe(
        "libx264",
        &["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=256x256:r=30", "-frames:v", "2", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-f", "h264", &sample.display().to_string()].map(String::from),
    );
    if !made.opened {
        return Probe { name: d.ffmpeg_name().into(), opened: false, error: Some(format!("could not make the probe stream: {}", made.error.unwrap_or_default())) };
    }
    let mut args: Vec<String> = ["-hide_banner", "-v", "warning", "-hwaccel", d.ffmpeg_name()].map(String::from).to_vec();
    if let (HwDecode::Vaapi, Some(dev)) = (d, device) {
        args.extend(["-hwaccel_device".into(), dev.display().to_string()]);
    }
    args.extend(["-i".into(), sample.display().to_string(), "-f".into(), "null".into(), "-".into()]);
    let out = Command::new("ffmpeg").args(&args).output();
    let _ = std::fs::remove_file(&sample);
    match out {
        Ok(o) if o.status.success() && o.stderr.iter().all(u8::is_ascii_whitespace) => Probe { name: d.ffmpeg_name().into(), opened: true, error: None },
        Ok(o) => Probe { name: d.ffmpeg_name().into(), opened: false, error: Some(first_error(&o.stderr)) },
        Err(e) => Probe { name: d.ffmpeg_name().into(), opened: false, error: Some(format!("running ffmpeg: {e}")) },
    }
}

fn run_probe(name: &str, args: &[String]) -> Probe {
    match Command::new("ffmpeg").args(args).output() {
        Ok(o) if o.status.success() => Probe { name: name.into(), opened: true, error: None },
        Ok(o) => Probe { name: name.into(), opened: false, error: Some(first_error(&o.stderr)) },
        Err(e) => Probe { name: name.into(), opened: false, error: Some(format!("running ffmpeg: {e}")) },
    }
}

/// ffmpeg's first error line names the cause ("Cannot load libcuda.so.1"); the later ones only say the encode failed
fn first_error(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr).lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("ffmpeg failed without a message").to_string()
}

/// The first DRM render node (`/dev/dri/renderD*`), Linux only.
fn render_node() -> Option<PathBuf> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let mut nodes: Vec<PathBuf> = std::fs::read_dir("/dev/dri").ok()?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("renderD"))).collect();
    nodes.sort();
    nodes.into_iter().next()
}

/// The PCI vendor of a render node (`/sys/class/drm/<node>/device/vendor`).
fn pci_vendor(node: &Path) -> Gpu {
    let Some(name) = node.file_name() else { return Gpu::Unknown };
    let vendor = std::fs::read_to_string(Path::new("/sys/class/drm").join(name).join("device/vendor")).unwrap_or_default();
    match vendor.trim() {
        "0x1002" => Gpu::Amd,
        "0x8086" => Gpu::Intel,
        "0x10de" => Gpu::Nvidia,
        _ => Gpu::Unknown,
    }
}
