//! datavid: the shared media layer of the harness, rendered with [fframes](https://fframes.studio).
//!
//! The pieces the promo template needs, all in Rust:
//! * [`svg`]: frame building blocks for fframes SVG scenes (text with glow, the "slam" text animation, film finish).
//! * [`ease`]: easing and interpolation helpers.
//! * [`audio`]: procedural synthesis primitives, Butterworth filters, FFT reverb, a true-peak limiter and EBU R128
//!   loudness mastering (default −14 LUFS).
//! * [`qa`]: a machine- and human-readable QA report.
//! * [`media`]: ffmpeg/ffprobe helpers (probe, loudness of an encoded file, black frames, contact sheet, frames).
//! * [`env`]: `${VAR:-default}` expansion for config values.
pub mod audio;
pub mod ease;
pub mod env;
pub mod media;
pub mod qa;
pub mod svg;
