//! Render jobs. A render runs the whole `promo` pipeline on a blocking thread; the `render` tool waits for it up to a
//! limit and otherwise hands back a job id for `get_status`, so hosts with short tool timeouts still get the result.
//! One render at a time: fframes already keeps every core busy and two renders only compete for memory.
use promo::Res;
use promo::pipeline::{self, BuildOptions, BuildOutcome};
use promo::progress::Events;
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Succeeded,
    /// rendered, but QA found a problem (see `result.qa.failures`)
    QaFailed,
    Failed,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct JobStatus {
    pub job_id: String,
    pub state: JobState,
    pub config: PathBuf,
    pub resolution: Res,
    pub elapsed_s: f64,
    /// the build result (output MP4, SRT, plan, QA) once the job has finished
    pub result: Option<BuildOutcome>,
    pub error: Option<String>,
}

struct Job {
    config: PathBuf,
    resolution: Res,
    started: Instant,
    finished: Option<Duration>,
    outcome: Option<Result<BuildOutcome, String>>,
}

impl Job {
    fn status(&self, id: &str) -> JobStatus {
        let (state, result, error) = match &self.outcome {
            None => (JobState::Running, None, None),
            Some(Ok(r)) => (if r.ok { JobState::Succeeded } else { JobState::QaFailed }, Some(r.clone()), None),
            Some(Err(e)) => (JobState::Failed, None, Some(e.clone())),
        };
        JobStatus {
            job_id: id.to_string(),
            state,
            config: self.config.clone(),
            resolution: self.resolution,
            elapsed_s: (self.finished.unwrap_or_else(|| self.started.elapsed()).as_secs_f64() * 10.0).round() / 10.0,
            result,
            error,
        }
    }
}

#[derive(Clone, Default)]
pub struct Jobs {
    inner: Arc<Mutex<HashMap<String, Job>>>,
    next: Arc<Mutex<u64>>,
}

impl Jobs {
    /// Start a render; fails when another one is still running.
    pub fn start(&self, config: PathBuf, opts: BuildOptions) -> Result<String, String> {
        let mut jobs = self.inner.lock().map_err(|e| e.to_string())?;
        if let Some((id, _)) = jobs.iter().find(|(_, j)| j.outcome.is_none()) {
            return Err(format!("render job {id} is still running: wait for it with get_status"));
        }
        let id = {
            let mut n = self.next.lock().map_err(|e| e.to_string())?;
            *n += 1;
            format!("job-{}", *n)
        };
        jobs.insert(id.clone(), Job { config: config.clone(), resolution: opts.res, started: Instant::now(), finished: None, outcome: None });
        let inner = self.inner.clone();
        let job_id = id.clone();
        tokio::task::spawn_blocking(move || {
            let outcome = pipeline::build(&config, &opts, &Events::new(false)).map_err(|e| format!("{e:#}"));
            if let Ok(mut jobs) = inner.lock() {
                if let Some(j) = jobs.get_mut(&job_id) {
                    j.finished = Some(j.started.elapsed());
                    j.outcome = Some(outcome);
                }
            }
        });
        Ok(id)
    }

    pub fn status(&self, id: &str) -> Result<JobStatus, String> {
        let jobs = self.inner.lock().map_err(|e| e.to_string())?;
        jobs.get(id).map(|j| j.status(id)).ok_or_else(|| format!("no render job {id} in this session"))
    }

    /// Poll until the job finishes or `wait` runs out.
    pub async fn wait(&self, id: &str, wait: Duration) -> Result<JobStatus, String> {
        let deadline = Instant::now() + wait;
        loop {
            let s = self.status(id)?;
            if s.state != JobState::Running || Instant::now() >= deadline {
                return Ok(s);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}
