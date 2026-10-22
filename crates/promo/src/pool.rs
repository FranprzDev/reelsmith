//! A bounded worker pool on scoped threads. Plates and render chunks are independent ffmpeg / fframes processes, so
//! running `jobs` of them at once keeps every core busy instead of waiting on one process at a time.
use anyhow::Result;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// The default number of parallel jobs: the cores this process may use (1 when the OS cannot tell).
pub fn default_jobs() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// Run `f` on every item with at most `jobs` running at once; the results come back in item order. After the first
/// error no new item starts and that error is returned.
pub fn map<T: Sync, R: Send>(jobs: usize, items: &[T], f: impl Fn(&T) -> Result<R> + Sync) -> Result<Vec<R>> {
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let out: Mutex<Vec<Option<Result<R>>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..jobs.clamp(1, items.len().max(1)) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= items.len() || failed.load(Ordering::Relaxed) {
                        break;
                    }
                    let r = f(&items[i]);
                    if r.is_err() {
                        failed.store(true, Ordering::Relaxed);
                    }
                    out.lock().unwrap_or_else(|e| e.into_inner())[i] = Some(r);
                }
            });
        }
    });
    out.into_inner().unwrap_or_else(|e| e.into_inner()).into_iter().flatten().collect()
}
