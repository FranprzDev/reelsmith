//! Error kinds a caller can act on, and the process exit code each maps to (stable: an upload app branches on them).
use std::fmt;

/// The config (or a CLI flag) is wrong: fix the input, do not retry.
#[derive(Debug)]
pub struct ConfigError(pub String);

/// A file the config names does not exist (clip, motion asset, font, voice-over, music): upload / fetch it.
#[derive(Debug)]
pub struct MissingInput(pub String);

/// The video was produced but QA found a problem (`qa.json` lists the failures).
#[derive(Debug)]
pub struct QaFailed(pub Vec<String>);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "config: {}", self.0)
    }
}
impl fmt::Display for MissingInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "missing input: {}", self.0)
    }
}
impl fmt::Display for QaFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QA failed: {}", self.0.join("; "))
    }
}
impl std::error::Error for ConfigError {}
impl std::error::Error for MissingInput {}
impl std::error::Error for QaFailed {}

/// Process exit codes of the `promo` binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exit {
    /// success
    Ok = 0,
    /// anything unexpected (ffmpeg crashed, disk full, a render chunk kept failing): may be retried
    Failed = 1,
    /// bad config or command line
    Usage = 2,
    /// rendered, but QA failed
    QaFailed = 3,
    /// an input file is missing
    MissingInput = 4,
}

impl Exit {
    pub fn of(e: &anyhow::Error) -> Exit {
        for c in e.chain() {
            if c.is::<ConfigError>() {
                return Exit::Usage;
            }
            if c.is::<MissingInput>() {
                return Exit::MissingInput;
            }
            if c.is::<QaFailed>() {
                return Exit::QaFailed;
            }
        }
        Exit::Failed
    }
}
