use crate::config::{MusicKind, Res};
use crate::error::ConfigError;
use crate::pipeline::BuildOutcome;
use crate::plan::Plan;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn marker_path(p: &Plan) -> PathBuf {
    let stem = p.config.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "config".into());
    p.config.parent().unwrap_or(Path::new(".")).join(format!("{stem}.preview.json"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewProof {
    pub fingerprint: String,
    pub finished_at_unix: u64,
    pub output: PathBuf,
}

fn hash_file(h: &mut Sha256, path: &Path) -> Result<()> {
    let mut f = std::fs::File::open(path)?;
    let mut buf = [0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(())
}

pub fn fingerprint(p: &Plan) -> Result<String> {
    let mut h = Sha256::new();
    let mut v = serde_json::to_value(p)?;
    if let Some(m) = v.as_object_mut() {
        m.remove("config");
        m.remove("out_dir");
        m.remove("build_dir");
        m.remove("qa");
        if let Some(overlays) = m.get_mut("overlays").and_then(|o| o.as_array_mut()) {
            for o in overlays.iter_mut() {
                if let Some(om) = o.as_object_mut() {
                    om.remove("crop_box");
                }
            }
        }
    }
    h.update(serde_json::to_vec(&v)?);
    let mut inputs: Vec<PathBuf> = Vec::new();
    for s in &p.shots {
        inputs.push(s.clip.clone());
    }
    for o in &p.overlays {
        inputs.push(o.asset.clone());
    }
    if p.captions_on {
        inputs.push(p.font.clone());
    }
    if let Some(vo) = &p.voiceover {
        inputs.push(PathBuf::from(&vo.file));
    }
    if let (MusicKind::File, Some(m)) = (p.music.kind, &p.music.file) {
        inputs.push(PathBuf::from(m));
    }
    inputs.sort();
    inputs.dedup();
    for f in inputs {
        h.update(f.to_string_lossy().as_bytes());
        h.update([0]);
        hash_file(&mut h, &f)?;
    }
    Ok(format!("{:x}", h.finalize()))
}

fn read(path: &Path) -> Option<PreviewProof> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

pub fn check(p: &Plan, r: Res) -> Result<()> {
    if r == Res::Preview {
        return Ok(());
    }
    let fp = fingerprint(p)?;
    match read(&marker_path(p)) {
        Some(proof) if proof.fingerprint == fp => Ok(()),
        Some(_) => Err(ConfigError(format!(
            "{} changed since its last preview (brief, captions or input files): render a fresh 360p preview before {}",
            p.name,
            r.tag()
        ))
        .into()),
        None => Err(ConfigError(format!(
            "no completed 360p preview for {}: render a 360p preview before {}",
            p.name,
            r.tag()
        ))
        .into()),
    }
}

pub fn record(p: &Plan, o: &BuildOutcome) -> Result<()> {
    let proof = PreviewProof {
        fingerprint: fingerprint(p)?,
        finished_at_unix: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        output: o.output.clone(),
    };
    let marker = marker_path(p);
    if let Some(dir) = marker.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = marker.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(&proof)?)?;
    std::fs::rename(&tmp, &marker)?;
    Ok(())
}
