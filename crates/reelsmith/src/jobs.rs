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
use tokio::sync::Notify;

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
    /// woken when `outcome` is set; one per job so a completion wakes only its waiters
    notify: Arc<Notify>,
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
        jobs.insert(
            id.clone(),
            Job {
                config: config.clone(),
                resolution: opts.res,
                started: Instant::now(),
                finished: None,
                outcome: None,
                notify: Arc::new(Notify::new()),
            },
        );
        let inner = self.inner.clone();
        let job_id = id.clone();
        tokio::task::spawn_blocking(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pipeline::build(&config, &opts, &Events::new(false)).map_err(|e| format!("{e:#}"))
            }));
            let outcome = match outcome {
                Ok(r) => r,
                Err(_) => Err("render pipeline panicked".to_string()),
            };
            if let Ok(mut jobs) = inner.lock() {
                if let Some(j) = jobs.get_mut(&job_id) {
                    j.finished = Some(j.started.elapsed());
                    j.outcome = Some(outcome);
                    j.notify.notify_waiters();
                }
            }
        });
        Ok(id)
    }

    pub fn status(&self, id: &str) -> Result<JobStatus, String> {
        let jobs = self.inner.lock().map_err(|e| e.to_string())?;
        jobs.get(id).map(|j| j.status(id)).ok_or_else(|| format!("no render job {id} in this session"))
    }

    /// Wait until the job finishes or `wait` runs out.
    pub async fn wait(&self, id: &str, wait: Duration) -> Result<JobStatus, String> {
        let deadline = Instant::now() + wait;
        loop {
            let (status, notify) = {
                let jobs = self.inner.lock().map_err(|e| e.to_string())?;
                let job = jobs.get(id).ok_or_else(|| format!("no render job {id} in this session"))?;
                (job.status(id), job.notify.clone())
            };
            if status.state != JobState::Running || Instant::now() >= deadline {
                return Ok(status);
            }

            let notified = notify.notified();
            tokio::pin!(notified);
            // Register before rechecking: a completion between the status read above and this line calls
            // `notify_waiters`, which only reaches already-registered waiters.
            let _ = notified.as_mut().enable();
            let status = self.status(id)?;
            if status.state != JobState::Running {
                return Ok(status);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(status);
            }
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep(remaining) => {}
            }
        }
    }
}

#[cfg(test)]
impl Jobs {
    fn insert_running(&self, id: &str) {
        let mut jobs = self.inner.lock().expect("jobs lock");
        jobs.insert(
            id.to_string(),
            Job {
                config: PathBuf::from("test.toml"),
                resolution: Res::Preview,
                started: Instant::now(),
                finished: None,
                outcome: None,
                notify: Arc::new(Notify::new()),
            },
        );
    }

    fn insert_completed(&self, id: &str, outcome: Result<BuildOutcome, String>) {
        let mut jobs = self.inner.lock().expect("jobs lock");
        jobs.insert(
            id.to_string(),
            Job {
                config: PathBuf::from("test.toml"),
                resolution: Res::Preview,
                started: Instant::now(),
                finished: Some(Duration::from_millis(50)),
                outcome: Some(outcome),
                notify: Arc::new(Notify::new()),
            },
        );
    }

    fn complete(&self, id: &str, outcome: Result<BuildOutcome, String>) {
        let mut jobs = self.inner.lock().expect("jobs lock");
        if let Some(j) = jobs.get_mut(id) {
            j.finished = Some(j.started.elapsed());
            j.outcome = Some(outcome);
            j.notify.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use promo::config::{Aspect, MusicKind};
    use promo::encode::{Backend, Encoder, Gpu};
    use promo::error::Exit;
    use promo::pipeline::PlanSummary;

    fn fake_outcome(ok: bool) -> BuildOutcome {
        BuildOutcome {
            ok,
            exit: if ok { Exit::Ok } else { Exit::QaFailed },
            code: if ok { 0 } else { 3 },
            config: PathBuf::from("test.toml"),
            output: PathBuf::from("test.mp4"),
            srt: None,
            backend: Backend {
                encoder: Encoder::X264,
                gpu: Gpu::None,
                hw_decode: None,
                device: None,
                reason: "test".to_string(),
                probes: Vec::new(),
            },
            plan: PlanSummary {
                name: "test".to_string(),
                aspect: Aspect::Vertical,
                resolution: Res::Preview,
                width: 360,
                height: 640,
                seconds: 0.05,
                frames: 2,
                shots: 1,
                overlays: 0,
                captions: 0,
                captions_burned_in: false,
                flashes: Vec::new(),
                music: MusicKind::None,
                voiceover: false,
                build_dir: PathBuf::from("build/test"),
            },
            qa: None,
        }
    }

    #[tokio::test]
    async fn completed_job_returns_immediately() {
        let jobs = Jobs::default();
        jobs.insert_completed("done", Ok(fake_outcome(true)));
        let started = Instant::now();
        let status = jobs.wait("done", Duration::from_secs(5)).await.unwrap();
        assert_eq!(status.state, JobState::Succeeded);
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "a completed job must not wait for a notification: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn completed_before_wait_is_seen() {
        let jobs = Jobs::default();
        jobs.insert_running("done");
        jobs.complete("done", Ok(fake_outcome(true)));
        let status = jobs.wait("done", Duration::from_secs(5)).await.unwrap();
        assert_eq!(status.state, JobState::Succeeded);
    }

    #[tokio::test]
    async fn qa_failed_job_is_complete() {
        let jobs = Jobs::default();
        jobs.insert_completed("qa", Ok(fake_outcome(false)));
        let status = jobs.wait("qa", Duration::from_secs(5)).await.unwrap();
        assert_eq!(status.state, JobState::QaFailed);
    }

    #[tokio::test]
    async fn failed_job_is_complete() {
        let jobs = Jobs::default();
        jobs.insert_completed("failed", Err("boom".to_string()));
        let status = jobs.wait("failed", Duration::from_secs(5)).await.unwrap();
        assert_eq!(status.state, JobState::Failed);
        assert_eq!(status.error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn unknown_job_errors() {
        let jobs = Jobs::default();
        let err = jobs.wait("missing", Duration::from_secs(1)).await.unwrap_err();
        assert!(err.contains("no render job missing"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn zero_wait_returns_current_status() {
        let jobs = Jobs::default();
        jobs.insert_running("job");
        let status = jobs.wait("job", Duration::ZERO).await.unwrap();
        assert_eq!(status.state, JobState::Running);

        jobs.complete("job", Ok(fake_outcome(true)));
        let status = jobs.wait("job", Duration::ZERO).await.unwrap();
        assert_eq!(status.state, JobState::Succeeded);
    }

    #[tokio::test]
    async fn wait_times_out_with_current_status() {
        let jobs = Jobs::default();
        jobs.insert_running("slow");
        let started = Instant::now();
        let status = jobs.wait("slow", Duration::from_millis(50)).await.unwrap();
        let elapsed = started.elapsed();
        assert_eq!(status.state, JobState::Running);
        assert!(elapsed >= Duration::from_millis(40), "wait returned before the deadline: {elapsed:?}");
        let now = jobs.status("slow").unwrap();
        assert_eq!(status.job_id, now.job_id);
        assert_eq!(status.state, now.state);
        assert_eq!(status.result.is_some(), now.result.is_some());
        assert_eq!(status.error, now.error);
    }

    #[tokio::test]
    async fn notification_wakes_waiter() {
        let jobs = Jobs::default();
        jobs.insert_running("job");
        let waiter = tokio::spawn({
            let jobs = jobs.clone();
            async move { jobs.wait("job", Duration::from_secs(5)).await.unwrap() }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let completed = Instant::now();
        jobs.complete("job", Ok(fake_outcome(true)));
        let status = waiter.await.unwrap();
        let latency = completed.elapsed();
        assert_eq!(status.state, JobState::Succeeded);
        assert!(latency < Duration::from_millis(400), "waiter did not wake on completion: {latency:?}");
    }

    #[tokio::test]
    async fn multiple_waiters_all_wake() {
        let jobs = Jobs::default();
        jobs.insert_running("job");
        let mut waiters = Vec::new();
        for _ in 0..8 {
            let jobs = jobs.clone();
            waiters.push(tokio::spawn(async move { jobs.wait("job", Duration::from_secs(5)).await.unwrap() }));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        jobs.complete("job", Ok(fake_outcome(true)));
        for waiter in waiters {
            assert_eq!(waiter.await.unwrap().state, JobState::Succeeded);
        }
    }

    #[tokio::test]
    #[ignore = "benchmark: compares wait latency, not render speed"]
    async fn benchmark_notify_completion_delay_vs_500ms_poll() {
        // Synthetic 50 ms job: measures how quickly `wait` observes completion, not how quickly anything renders.
        let notify_delay = {
            let jobs = Jobs::default();
            jobs.insert_running("notify");
            let waiter = tokio::spawn({
                let jobs = jobs.clone();
                async move { jobs.wait("notify", Duration::from_secs(5)).await.unwrap() }
            });
            tokio::time::sleep(Duration::from_millis(50)).await;
            let completed = Instant::now();
            jobs.complete("notify", Ok(fake_outcome(true)));
            assert_eq!(waiter.await.unwrap().state, JobState::Succeeded);
            completed.elapsed()
        };

        let poll_delay = {
            let jobs = Jobs::default();
            jobs.insert_running("poll");
            let waiter = tokio::spawn({
                let jobs = jobs.clone();
                async move { wait_poll(&jobs, "poll", Duration::from_secs(5)).await }
            });
            tokio::time::sleep(Duration::from_millis(50)).await;
            let completed = Instant::now();
            jobs.complete("poll", Ok(fake_outcome(true)));
            assert_eq!(waiter.await.unwrap().state, JobState::Succeeded);
            completed.elapsed()
        };

        eprintln!(
            "completion delay: notify {notify_delay:?}, 500 ms poll {poll_delay:?} (wait latency only, not render speed)"
        );
        assert!(notify_delay < poll_delay, "event-driven wait should observe completion sooner than a fixed 500 ms poll");
    }

    async fn wait_poll(jobs: &Jobs, id: &str, wait: Duration) -> JobStatus {
        let deadline = Instant::now() + wait;
        loop {
            let status = jobs.status(id).unwrap();
            if status.state != JobState::Running || Instant::now() >= deadline {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}
