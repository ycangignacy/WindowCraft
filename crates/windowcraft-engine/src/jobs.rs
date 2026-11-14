// Made by Nordan
//! Background jobs with cooperative cancellation and bounded buffers.
//!
//! Progress flows through a bounded channel and logs through a ring buffer, so
//! a multi-hour ISO build cannot grow memory no matter how chatty it is.

use crate::error::{EngineError, Result};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

pub const EVENT_CAPACITY: usize = 64;
pub const LOG_CAPACITY: usize = 2_000;

#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() { Err(EngineError::Cancelled) } else { Ok(()) }
    }
}

/// Fixed-capacity log. The oldest line is evicted once it fills.
#[derive(Debug, Clone)]
pub struct LogRing {
    inner: Arc<Mutex<VecDeque<String>>>,
    capacity: usize,
}

impl LogRing {
    pub fn new(capacity: usize) -> Self {
        Self { inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity.min(256)))), capacity }
    }

    pub fn push(&self, line: impl Into<String>) {
        if let Ok(mut lines) = self.inner.lock() {
            if lines.len() == self.capacity {
                lines.pop_front();
            }
            lines.push_back(line.into());
        }
    }

    pub fn snapshot(&self) -> Vec<String> {
        self.inner.lock().map_or_else(|_| Vec::new(), |lines| lines.iter().cloned().collect())
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map_or(0, |lines| lines.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum JobEvent {
    Progress { fraction: f32, step: String },
    Finished { outcome: std::result::Result<String, JobFailure> },
}

/// Why a job ended without a summary. The flags come from the error variant,
/// so the app never has to read them out of translated text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JobFailure {
    pub message: String,
    pub needs_elevation: bool,
    pub cancelled: bool,
    pub panicked: bool,
}

static ERROR_TEXT: std::sync::OnceLock<fn(&crate::error::EngineError) -> String> = std::sync::OnceLock::new();

/// Sets how a failed job words its error, so the app can use the user's
/// language. Without it the English `Display` text is used. The first call wins.
pub fn set_error_text(text: fn(&crate::error::EngineError) -> String) {
    let _ = ERROR_TEXT.set(text);
}

impl JobFailure {
    pub fn from_error(error: &crate::error::EngineError) -> Self {
        use crate::error::EngineError;
        Self {
            message: ERROR_TEXT.get().map_or_else(|| error.to_string(), |text| text(error)),
            needs_elevation: matches!(error, EngineError::NeedsElevation(_)),
            cancelled: matches!(error, EngineError::Cancelled),
            panicked: false,
        }
    }

    pub fn panicked() -> Self {
        Self { message: "the job panicked".to_owned(), panicked: true, ..Self::default() }
    }
}

impl std::fmt::Display for JobFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Handed to job bodies for reporting and cancellation.
#[derive(Clone)]
pub struct JobContext {
    events: mpsc::Sender<JobEvent>,
    pub cancel: CancelToken,
    pub log: LogRing,
}

impl JobContext {
    /// Non-blocking: when the UI falls behind, intermediate progress is
    /// dropped instead of stalling the worker.
    pub fn progress(&self, fraction: f32, step: impl Into<String>) {
        let step = step.into();
        self.log.push(step.clone());
        // A full channel means the UI already has a newer update queued.
        let _ = self.events.try_send(JobEvent::Progress { fraction: fraction.clamp(0.0, 1.0), step });
    }

    pub fn log(&self, line: impl Into<String>) {
        self.log.push(line);
    }
}

pub struct JobHandle {
    pub events: mpsc::Receiver<JobEvent>,
    pub cancel: CancelToken,
    pub log: LogRing,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl JobHandle {
    /// Releases the handle, keeping only the event stream.
    pub fn into_events(self) -> mpsc::Receiver<JobEvent> {
        self.events
    }

    /// Waits for the worker thread to exit.
    pub fn join(mut self) {
        if let Some(worker) = self.worker.take() {
            // A panicking job already reported through its completion guard.
            let _ = worker.join();
        }
    }
}

/// Runs `body` on its own OS thread, so COM apartments and blocking Win32
/// calls never run on the async runtime.
pub fn spawn<F>(name: &str, body: F) -> JobHandle
where
    F: FnOnce(&JobContext) -> Result<String> + Send + 'static,
{
    let (sender, events) = mpsc::channel(EVENT_CAPACITY);
    let cancel = CancelToken::new();
    let log = LogRing::new(LOG_CAPACITY);
    let context = JobContext { events: sender.clone(), cancel: cancel.clone(), log: log.clone() };

    let worker = std::thread::Builder::new().name(format!("job-{name}")).spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(&context)));
        let outcome = match outcome {
            Ok(Ok(summary)) => Ok(summary),
            Ok(Err(error)) => Err(JobFailure::from_error(&error)),
            Err(_) => Err(JobFailure::panicked()),
        };
        // The final event must arrive even if the channel is momentarily full,
        // but a dropped receiver means nobody is listening any more.
        let _ = sender.blocking_send(JobEvent::Finished { outcome });
    });

    JobHandle { events, cancel, log, worker: worker.ok() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_ring_evicts_oldest() {
        let ring = LogRing::new(3);
        for i in 0..1_000 {
            ring.push(i.to_string());
        }
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.snapshot(), ["997", "998", "999"]);
    }

    #[test]
    fn job_reports_success() {
        let mut job = spawn("ok", |ctx| {
            ctx.progress(0.5, "halfway");
            Ok("done".into())
        });
        let mut last = None;
        while let Some(event) = job.events.blocking_recv() {
            last = Some(event);
        }
        assert_eq!(last, Some(JobEvent::Finished { outcome: Ok("done".into()) }));
    }

    #[test]
    fn job_panic_is_contained_and_reported() {
        let mut job = spawn("panic", |_| panic!("boom"));
        let mut last = None;
        while let Some(event) = job.events.blocking_recv() {
            last = Some(event);
        }
        assert_eq!(last, Some(JobEvent::Finished { outcome: Err(JobFailure::panicked()) }));
    }

    #[test]
    fn flooding_progress_never_blocks_a_worker() {
        let job = spawn("flood", |ctx| {
            for i in 0..100_000 {
                ctx.progress(0.5, i.to_string());
            }
            Ok(String::new())
        });
        // Nothing drains the channel, so a blocking send would hang here.
        // The worker's final send only completes once the receiver drops.
        let JobHandle { events, log, worker, .. } = job;
        std::thread::sleep(std::time::Duration::from_millis(50));
        drop(events);
        worker.unwrap().join().unwrap();
        assert!(log.len() <= LOG_CAPACITY);
    }

    #[test]
    fn rapid_cancellation_stops_every_job() {
        let mut jobs = Vec::new();
        for _ in 0..64 {
            jobs.push(spawn("cancel", |ctx| {
                loop {
                    ctx.cancel.check()?;
                    std::thread::yield_now();
                }
            }));
        }
        for job in &jobs {
            job.cancel.cancel();
        }
        for mut job in jobs {
            let mut last = None;
            while let Some(event) = job.events.blocking_recv() {
                last = Some(event);
            }
            assert!(matches!(last, Some(JobEvent::Finished { outcome: Err(_) })));
            job.join();
        }
    }

    #[test]
    fn dropping_the_handle_does_not_hang_the_worker() {
        let job = spawn("detached", |ctx| {
            for _ in 0..EVENT_CAPACITY * 3 {
                ctx.progress(0.1, "tick");
            }
            Ok(String::new())
        });
        let JobHandle { events, worker, .. } = job;
        drop(events);
        worker.unwrap().join().unwrap();
    }
}
