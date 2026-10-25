use crate::ledger::LedgerEntry;
use crate::progress::Events;
use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const QUEUE: &str = "renders.queue.jsonl";
const LOCK: &str = "renders.queue.lock";
const INSTALL: &str = "install.json";
const ACCEPTED: &str = "telemetry-accepted.json";
const MAX_FLUSH_PER_BUILD: usize = 25;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Install {
    install_id: String,
    secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Event {
    event_id: String,
    install_id: String,
    ts: u64,
    resolution: String,
    ok: bool,
    exit_code: u8,
    seconds: f64,
    project: String,
}

struct Cfg {
    endpoint: String,
    token: String,
}

fn cfg() -> Option<Cfg> {
    match std::env::var("REELSMITH_TELEMETRY").ok().map(|v| v.to_ascii_lowercase()) {
        Some(v) if v == "off" || v == "0" || v == "false" => return None,
        _ => {}
    }
    let endpoint = std::env::var("REELSMITH_TELEMETRY_ENDPOINT").ok()?;
    let token = std::env::var("REELSMITH_TELEMETRY_TOKEN").ok()?;
    if endpoint.is_empty() || token.is_empty() {
        return None;
    }
    Some(Cfg { endpoint, token })
}

fn state_dir() -> Result<PathBuf> {
    if let Some(d) = std::env::var_os("REELSMITH_TELEMETRY_HOME") {
        return Ok(PathBuf::from(d));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).context("no home directory for the telemetry state")?;
    Ok(PathBuf::from(home).join(".config").join("reelsmith"))
}

fn host(endpoint: &str) -> &str {
    endpoint.split('/').nth(2).unwrap_or(endpoint)
}

fn secure_scheme(endpoint: &str) -> bool {
    if endpoint.starts_with("https://") {
        return true;
    }
    endpoint
        .strip_prefix("http://")
        .and_then(|rest| rest.split(['/', ':']).next())
        .is_some_and(|h| h == "127.0.0.1" || h == "localhost" || h == "[::1]")
}

fn accepted(dir: &Path, endpoint: &str) -> bool {
    std::fs::read_to_string(dir.join(ACCEPTED))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("endpoint").and_then(|e| e.as_str()).map(str::to_string))
        .is_some_and(|e| e == endpoint)
}

pub fn describe() -> String {
    let mut s = String::from("telemetry: a small summary of each completed render sent to your own API.\n");
    let Some(c) = cfg() else {
        s.push_str("status: off (unset REELSMITH_TELEMETRY=off, then set REELSMITH_TELEMETRY_ENDPOINT and REELSMITH_TELEMETRY_TOKEN to configure it)\n");
        s.push_str("nothing is sent while it is not configured\n");
        return s;
    };
    s.push_str(&format!("destination: {}\n", c.endpoint));
    let ok = state_dir().map(|d| accepted(&d, &c.endpoint)).unwrap_or(false);
    s.push_str(if ok { "status: accepted on this machine, sent by default after each completed render\n" } else { "status: not accepted on this machine yet, nothing is sent\n" });
    s.push_str("each event carries: finish time, resolution (preview/draft/final), ok + exit code, length in seconds, a random install id and a pseudonymous project key derived per install\n");
    s.push_str("never sent: project names, file paths, briefs, texts, clips or videos\n");
    s.push_str("while offline, events queue locally and resend later without double counting\n");
    s.push_str("enable: promo telemetry accept   disable: set REELSMITH_TELEMETRY=off or unset the endpoint/token\n");
    s
}

pub fn accept() -> Result<String> {
    let Some(c) = cfg() else {
        anyhow::bail!("set REELSMITH_TELEMETRY_ENDPOINT and REELSMITH_TELEMETRY_TOKEN first");
    };
    if !secure_scheme(&c.endpoint) {
        anyhow::bail!("the telemetry endpoint must be https (http is only allowed for a loopback address while testing locally)");
    }
    let dir = state_dir()?;
    std::fs::create_dir_all(&dir)?;
    if accepted(&dir, &c.endpoint) {
        return Ok(format!("telemetry: already accepted for {}", c.endpoint));
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let tmp = dir.join(format!("accepted.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(&serde_json::json!({"endpoint": c.endpoint, "accepted_at_unix": now}))?)?;
    std::fs::rename(&tmp, dir.join(ACCEPTED))?;
    Ok(format!("telemetry: accepted for {}; every completed render now sends a small summary (run `promo telemetry` to review it, REELSMITH_TELEMETRY=off to disable)", c.endpoint))
}

fn load_install(dir: &Path) -> Result<Install> {
    let path = dir.join(INSTALL);
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(i) = serde_json::from_str::<Install>(&text) {
            return Ok(i);
        }
    }
    let i = Install {
        install_id: uuid::Uuid::new_v4().simple().to_string(),
        secret: format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple()),
    };
    let tmp = dir.join(format!("install.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(&i)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(i)
}

fn project_key(secret: &str, fingerprint: &str) -> String {
    let mut h = Sha256::new();
    h.update(secret.as_bytes());
    h.update([0]);
    h.update(fingerprint.as_bytes());
    format!("{:x}", h.finalize())[..16].to_string()
}

fn with_lock<R>(dir: &Path, f: impl FnOnce() -> Result<R>) -> Result<R> {
    let lock = std::fs::OpenOptions::new().create(true).read(true).write(true).open(dir.join(LOCK))?;
    lock.lock_exclusive()?;
    let r = f();
    let _ = lock.unlock();
    r
}

fn enqueue(dir: &Path, line: &str) -> Result<()> {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(QUEUE))?;
    f.write_all(line.as_bytes())?;
    f.write_all(b"\n")?;
    Ok(())
}

fn flush_locked(dir: &Path, c: &Cfg, ev: &Events) -> Result<()> {
    let text = match std::fs::read_to_string(dir.join(QUEUE)) {
        Ok(t) => t,
        Err(_) => return Ok(()),
    };
    let lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
    if lines.is_empty() {
        return Ok(());
    }
    let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(5)).redirects(0).build();
    let mut pending: Vec<String> = Vec::new();
    let mut sent = 0usize;
    for (i, line) in lines.iter().enumerate() {
        if sent >= MAX_FLUSH_PER_BUILD {
            pending.extend_from_slice(&lines[i..]);
            break;
        }
        let Ok(event) = serde_json::from_str::<Event>(line.trim()) else {
            ev.log("warning: the telemetry queue holds an unreadable line, keeping it");
            pending.push(line.clone());
            continue;
        };
        let r = agent
            .post(&c.endpoint)
            .set("Authorization", &format!("Bearer {}", c.token))
            .set("Content-Type", "application/json")
            .send_string(&serde_json::to_string(&event)?);
        match r {
            Ok(_) => sent += 1,
            Err(e) => {
                ev.log(&format!("warning: telemetry could not reach the server ({e}); the render stays in the local queue"));
                pending.extend_from_slice(&lines[i..]);
                break;
            }
        }
    }
    let path = dir.join(QUEUE);
    if pending.is_empty() {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        return Ok(());
    }
    let tmp = dir.join(format!("queue.{}.tmp", std::process::id()));
    std::fs::write(&tmp, pending.join(""))?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn record(entry: &LedgerEntry, ev: &Events) {
    if let Err(e) = run(entry, ev) {
        ev.log(&format!("warning: telemetry could not be queued: {e:#}"));
    }
}

fn run(entry: &LedgerEntry, ev: &Events) -> Result<()> {
    let Some(c) = cfg() else { return Ok(()) };
    if !secure_scheme(&c.endpoint) {
        ev.log("warning: the telemetry endpoint is not https (http only works for a loopback address), nothing was sent");
        return Ok(());
    }
    let dir = state_dir()?;
    std::fs::create_dir_all(&dir)?;
    if !accepted(&dir, &c.endpoint) {
        ev.log(&format!(
            "telemetry: configured for {} but not accepted on this machine; run `promo telemetry` to review exactly what would be sent and `promo telemetry accept` to enable it",
            host(&c.endpoint)
        ));
        return Ok(());
    }
    let install = load_install(&dir)?;
    let event = Event {
        event_id: uuid::Uuid::new_v4().to_string(),
        install_id: install.install_id.clone(),
        ts: entry.finished_at_unix,
        resolution: entry.resolution.clone(),
        ok: entry.ok,
        exit_code: entry.exit_code,
        seconds: entry.seconds,
        project: project_key(&install.secret, &entry.fingerprint),
    };
    let line = serde_json::to_string(&event)?;
    with_lock(&dir, || {
        enqueue(&dir, &line)?;
        flush_locked(&dir, &c, ev)
    })
}
