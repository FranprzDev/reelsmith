use crate::config::Res;
use crate::pipeline::BuildOutcome;
use crate::plan::Plan;
use crate::preview_gate;
use anyhow::Result;
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const LEDGER: &str = "renders.jsonl";

#[derive(Debug, Clone, Serialize)]
pub struct LedgerEntry {
    pub finished_at_unix: u64,
    pub name: String,
    pub config: PathBuf,
    pub resolution: String,
    pub output: PathBuf,
    pub ok: bool,
    pub exit_code: u8,
    pub seconds: f64,
    pub frames: usize,
    pub fingerprint: String,
}

pub fn record(p: &Plan, r: Res, o: &BuildOutcome) -> Result<LedgerEntry> {
    let entry = LedgerEntry {
        finished_at_unix: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        name: p.name.clone(),
        config: p.config.clone(),
        resolution: r.tag().to_string(),
        output: o.output.clone(),
        ok: o.ok,
        exit_code: o.code,
        seconds: p.seconds,
        frames: p.frames(),
        fingerprint: preview_gate::fingerprint(p)?,
    };
    let path = p.config.parent().unwrap_or(Path::new(".")).join(LEDGER);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    let mut line = serde_json::to_string(&entry)?;
    line.push('\n');
    f.write_all(line.as_bytes())?;
    Ok(entry)
}
