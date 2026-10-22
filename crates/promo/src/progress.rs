//! Progress for people and for programs. Human logs always go to stderr; with `--json` every stage change, progress
//! tick and the final result is also printed to stdout as one JSON object per line (NDJSON), so an upload app can show
//! a progress bar and read the result without parsing logs:
//!
//! ```text
//! {"event":"stage","stage":"plate","status":"start"}
//! {"event":"progress","stage":"render","done":3,"total":12}
//! {"event":"stage","stage":"render","status":"done","seconds":31.2}
//! {"event":"result","ok":true,"exit":"ok","output":"/.../out/x.mp4","qa":{...}}
//! ```
use serde::Serialize;
use serde_json::{Value, json};
use std::io::Write;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Check,
    Plate,
    Audio,
    Render,
    Mux,
    Srt,
    Qa,
    Build,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Start,
    Done,
    Skipped,
    Failed,
}

#[derive(Debug, Clone)]
pub struct Events {
    pub json: bool,
    started: Instant,
}

impl Events {
    pub fn new(json: bool) -> Self {
        Self { json, started: Instant::now() }
    }
    fn emit(&self, v: Value) {
        if self.json {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{v}");
            let _ = out.flush();
        }
    }
    pub fn log(&self, msg: &str) {
        eprintln!("{msg}");
    }
    pub fn stage(&self, stage: Stage, status: Status) {
        let secs = (self.started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
        self.emit(json!({"event": "stage", "stage": stage, "status": status, "seconds": secs}));
    }
    pub fn progress(&self, stage: Stage, done: usize, total: usize) {
        self.emit(json!({"event": "progress", "stage": stage, "done": done, "total": total}));
    }
    /// the last line of a run (also written to `<out>/<name>.result.json` by `build`)
    pub fn result(&self, v: &Value) {
        self.emit(json!({"event": "result"}).as_object().cloned().map(|mut m| {
            if let Some(o) = v.as_object() {
                m.extend(o.clone());
            }
            Value::Object(m)
        }).unwrap_or(Value::Null));
    }
}
