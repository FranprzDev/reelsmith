//! promo: a config-driven short-form promo / vertical ad template on the datavid harness.
//!
//! Pipeline (each stage reads only the resolved [`plan::Plan`]):
//! * [`config`]: what the author writes (`promo.toml` / JSON): beats ("texts in") or explicit shots, overlays,
//!   captions (off by default), end card, music, voice-over, loudness and QA targets; `extends` for shared styles.
//! * [`plan`]: the resolved timeline with absolute times and paths (`build/<name>/plan.json`).
//! * [`script`]: caption grouping, the timed script format, SRT in/out.
//! * [`plate`]: ffmpeg cuts shots into JPG frames and overlays into cropped RGBA PNG frames (cached, auto crops).
//! * [`audio`]: music bed + cut hits + cues + overlay sfx + voice-over, mastered to −14 LUFS.
//! * [`frame`]: one SVG per frame for fframes (push-in, vignette, overlays, caption pop, flashes).
//! * [`qa`]: loudness, true peak, black frames, caption size, contact sheet.
//! * [`render`]: the chunked, resumable fframes render (lossless FFV1 chunks) and the final mux.
//! * [`encode`]: the detected GPU / encoder backend (NVENC, AMF, QSV, VA-API, VideoToolbox or libx264 through the
//!   `ffmpeg` on PATH) with its paired hardware decoder and per-tier presets.
//! * [`pool`]: the bounded worker pool that runs plates and render chunks in parallel.
//! * [`pipeline`]: the whole build for one config, shared by the `promo` CLI and the Reelsmith MCP server.
//! * [`progress`] / [`error`]: NDJSON progress events and stable exit codes for callers.
pub mod audio;
pub mod config;
pub mod encode;
pub mod error;
pub mod font;
pub mod frame;
pub mod pipeline;
pub mod plan;
pub mod plate;
pub mod pool;
pub mod progress;
pub mod qa;
pub mod render;
pub mod script;

pub use config::{Aspect, Res};
pub use plan::{Overrides, Plan};
