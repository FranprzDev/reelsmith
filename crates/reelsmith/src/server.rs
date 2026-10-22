//! The MCP tools. Each one is a thin, typed wrapper over the `promo` template and the harness media helpers.
use crate::animate::{self, AnimateSvgArgs, AnimatedSvg};
use crate::brief::{self, BriefWritten, CreateProjectArgs, ProjectCreated, WriteBriefArgs};
use crate::clips::{self, ClipInfo, ClipList};
use crate::jobs::{JobStatus, Jobs};
use crate::sample;
use promo::encode::{Backend, EncoderChoice};
use promo::pipeline::{self, BuildOptions};
use promo::qa::QaSummary;
use promo::{Overrides, Res};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{Json, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

const INSTRUCTIONS: &str = "Reelsmith renders short-form promo videos (Reels/TikTok/Shorts ads) from local clips. \
Flow: list_clips on the clips folder -> create_project (style: aspect, music, captions) -> write_brief (one beat per \
idea: text + clip names + optional overlay, plus an end card) -> render with resolution \"preview\" (360p) to check the \
cut, \"draft\" (720p) for review, \"final\" (1080p) only after the user approves the draft. render returns the MP4 path \
and the QA report (duration, size, loudness -14 LUFS, true peak, black frames); if it is still running it returns a \
job_id: call get_status. Burned-in captions are off by default (an SRT is written). Never invent claims for on-screen \
text: use the user's own wording. To open or close an ad with a logo, animate_svg turns an SVG into an alpha .mov: \
write it into the project's motion folder and use its asset_name as a beat overlay (intro) or as end_card.asset.";

/// Seconds `render` waits before handing back a job id (hosts such as Codex time tool calls out at 60 s).
const DEFAULT_WAIT_S: u64 = 45;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FolderArgs {
    /// absolute path of a local folder (with video files, or where the sample library goes)
    pub folder: PathBuf,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PathArgs {
    /// absolute path of a video file
    pub path: PathBuf,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenderArgs {
    /// the config to render: a brief.toml from write_brief or any promo.toml / brief.toml
    pub config: PathBuf,
    /// "preview" (360p, quick check), "draft" (720p, for review) or "final" (1080p, after approval)
    pub resolution: Res,
    /// override the config's captions switch (burn captions into the picture or not)
    pub captions_burned_in: Option<bool>,
    /// output MP4 path (default: `<project>/out/<name>_<resolution>_<w>x<h>.mp4`)
    pub out: Option<PathBuf>,
    /// GPU / encoder backend: "auto" (the first hardware encoder that opens: NVENC, AMF, QSV, VA-API or VideoToolbox,
    /// else libx264), or force "nvenc", "amf", "qsv", "vaapi", "videotoolbox" or "x264" (an error if it cannot open)
    #[serde(default)]
    pub encoder: EncoderChoice,
    /// how long to wait for the render before returning a job id (default 45)
    pub wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SystemInfoArgs {
    /// the backend to test (default "auto": detect the best one)
    #[serde(default)]
    pub encoder: EncoderChoice,
}

/// What this machine renders with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SystemInfo {
    /// the GPU / encoder backend a render would use, every probe tried and why it was chosen
    pub backend: Backend,
    /// plates and render chunks run this many at a time
    pub jobs: usize,
    /// the first line of `ffmpeg -version`
    pub ffmpeg: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StatusArgs {
    pub job_id: String,
    /// wait up to this many seconds for the job to finish before answering (default 0)
    pub wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct QaArgs {
    pub config: PathBuf,
    pub resolution: Res,
    /// the MP4 to check (default: the config's default output for this resolution)
    pub video: Option<PathBuf>,
}

/// Starting points shipped in the binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TemplateKind {
    /// a commented beats brief to copy and fill in
    PromoBrief,
    /// a timed voice-over / caption script ("SECONDS TEXT" lines)
    VoiceoverScript,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Template {
    pub kind: TemplateKind,
    pub file_name: String,
    pub description: String,
    pub content: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TemplateList {
    pub templates: Vec<Template>,
}

fn templates() -> Vec<Template> {
    vec![
        Template {
            kind: TemplateKind::PromoBrief,
            file_name: "brief.toml".into(),
            description: "A beats brief: one [[beat]] per idea with its text, clips and optional overlay, an end card and music. write_brief produces the same shape from structured arguments.".into(),
            content: include_str!("../../../templates/promo/brief.toml").into(),
        },
        Template {
            kind: TemplateKind::VoiceoverScript,
            file_name: "vo.txt".into(),
            description: "A timed script for a voice-over: captions aligned to the voice (`[voiceover] script = \"vo.txt\"`).".into(),
            content: include_str!("../../../templates/promo/vo.txt").into(),
        },
    ]
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> Result<T, String> {
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r.map_err(|e| format!("{e:#}")),
        Err(e) => Err(format!("internal error: {e}")),
    }
}

#[derive(Clone)]
pub struct Reelsmith {
    tool_router: ToolRouter<Self>,
    jobs: Jobs,
}

#[tool_router(router = tool_router)]
impl Reelsmith {
    pub fn new() -> Self {
        Self { tool_router: Self::tool_router(), jobs: Jobs::default() }
    }

    #[tool(description = "List the video files in a local folder with their duration, resolution, fps and role (clip, or motion graphic when the file has an alpha channel). Use the returned `name` in briefs.")]
    async fn list_clips(&self, Parameters(a): Parameters<FolderArgs>) -> Result<Json<ClipList>, String> {
        Ok(Json(blocking(move || clips::list(&a.folder)).await?))
    }

    #[tool(description = "Probe one video file: duration, resolution, fps, codec, pixel format, audio, role.")]
    async fn probe_clip(&self, Parameters(a): Parameters<PathArgs>) -> Result<Json<ClipInfo>, String> {
        Ok(Json(blocking(move || clips::probe(&a.path)).await?))
    }

    #[tool(description = "Write a procedural sample library into a folder (4 clips drawn by FFmpeg: nebula, cells, aurora, grid; 2 alpha motion graphics with a short sound: badge, cta) and list it. Use it to try or demo the flow without footage.")]
    async fn make_sample_clips(&self, Parameters(a): Parameters<FolderArgs>) -> Result<Json<ClipList>, String> {
        Ok(Json(blocking(move || sample::make(&a.folder)).await?))
    }

    #[tool(description = "The starting templates shipped with Reelsmith (a commented beats brief and a timed voice-over script).")]
    async fn list_templates(&self) -> Json<TemplateList> {
        Json(TemplateList { templates: templates() })
    }

    #[tool(description = "Animate an SVG (logo, icon, badge) into a transparent motion graphic: an alpha .mov (qtrle or ProRes 4444) at 30 or 60 fps with an entrance preset (fade_scale, slide_up, pop, draw_on, spin, glow), an easing, an optional hold and exit. Returns the .mov, its frame count and duration and a PNG preview. Save it in the project's motion folder and use asset_name as a beat overlay (intro logo) or end_card.asset.")]
    async fn animate_svg(&self, Parameters(a): Parameters<AnimateSvgArgs>) -> Result<Json<AnimatedSvg>, String> {
        Ok(Json(blocking(move || animate::animate(&a)).await?))
    }

    #[tool(description = "Create a project folder with its promo.toml style: canvas aspect, clips and motion folders, music bed, captions switch (off by default).")]
    async fn create_project(&self, Parameters(a): Parameters<CreateProjectArgs>) -> Result<Json<ProjectCreated>, String> {
        Ok(Json(blocking(move || brief::create_project(&a)).await?))
    }

    #[tool(description = "Write the project's brief.toml from beats (text + clip names + optional overlay) and an optional end card, then resolve it: returns the config path to render, the timeline summary and any missing input files.")]
    async fn write_brief(&self, Parameters(a): Parameters<WriteBriefArgs>) -> Result<Json<BriefWritten>, String> {
        Ok(Json(blocking(move || brief::write_brief(&a)).await?))
    }

    #[tool(description = "Render a config: plates, procedural/external music mastered to -14 LUFS, chunked fframes render, mux, SRT and QA. resolution: preview (360p), draft (720p) or final (1080p, only after the draft is approved). Returns the output MP4 and the QA report when done, or a running job_id to poll with get_status.")]
    async fn render(&self, Parameters(a): Parameters<RenderArgs>) -> Result<Json<JobStatus>, String> {
        clips::ensure_ffmpeg().map_err(|e| format!("{e:#}"))?;
        let config = if a.config.is_absolute() { a.config.clone() } else { std::env::current_dir().map_err(|e| e.to_string())?.join(&a.config) };
        let opts = BuildOptions {
            res: a.resolution,
            overrides: Overrides { captions: a.captions_burned_in, ..Default::default() },
            out: a.out,
            chunk_frames: 60,
            retries: 3,
            qa: true,
            encoder: a.encoder,
            jobs: promo::pool::default_jobs(),
        };
        let id = self.jobs.start(config, opts)?;
        Ok(Json(self.jobs.wait(&id, Duration::from_secs(a.wait_seconds.unwrap_or(DEFAULT_WAIT_S))).await?))
    }

    #[tool(description = "Detect the GPU and encoder this machine renders with (NVIDIA NVENC, AMD AMF / VA-API, Intel QSV / VA-API, Apple VideoToolbox, else libx264 on the CPU) by test-opening each one with ffmpeg; returns the chosen backend, its hardware decoder, every probe with its error and the reason. render reports the same backend in its result.")]
    async fn system_info(&self, Parameters(a): Parameters<SystemInfoArgs>) -> Result<Json<SystemInfo>, String> {
        Ok(Json(
            blocking(move || {
                clips::ensure_ffmpeg()?;
                let v = std::process::Command::new("ffmpeg").arg("-version").output()?;
                let ffmpeg = String::from_utf8_lossy(&v.stdout).lines().next().unwrap_or_default().to_string();
                Ok(SystemInfo { backend: a.encoder.resolve()?, jobs: promo::pool::default_jobs(), ffmpeg })
            })
            .await?,
        ))
    }

    #[tool(description = "Status of a render job: running, succeeded, qa_failed or failed, with the result (output MP4, SRT, QA) once finished.")]
    async fn get_status(&self, Parameters(a): Parameters<StatusArgs>) -> Result<Json<JobStatus>, String> {
        Ok(Json(self.jobs.wait(&a.job_id, Duration::from_secs(a.wait_seconds.unwrap_or(0))).await?))
    }

    #[tool(description = "Run QA on a rendered MP4 of a config: duration, size, fps, audio stream, integrated loudness, true peak, black frames, caption size, contact sheet.")]
    async fn get_qa(&self, Parameters(a): Parameters<QaArgs>) -> Result<Json<QaSummary>, String> {
        let s = blocking(move || {
            clips::ensure_ffmpeg()?;
            let p = pipeline::load(&a.config, &Overrides::default())?;
            let video = a.video.unwrap_or_else(|| p.default_out(a.resolution));
            anyhow::ensure!(video.is_file(), "no rendered video at {} (render it first)", video.display());
            promo::qa::run(&p, a.resolution, &video)
        })
        .await?;
        Ok(Json(s))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Reelsmith {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("reelsmith", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}
