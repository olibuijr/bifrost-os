//! Background work the window waits for: a process sample or a diagnostics run. The work runs on its own thread and
//! reports a fraction done; the window polls for the outcome on its sampling tick, so the table stays live.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use bifrost_app::dispatch::{DispatchQueue, Qos};

/// What a job does, for the label and the menu state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Sample,
    Diagnostics,
}

/// How a job ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Text to show in the report view (and to save).
    Report { title: String, text: String },
    /// A finished archive in the app's storage, to be saved where the user chooses.
    Bundle(PathBuf),
    Failed(String),
    Cancelled,
}

/// Progress shared between the job and the window, and the stop request that goes the other way.
#[derive(Default)]
pub struct Progress {
    done: AtomicU32,
    total: AtomicU32,
    cancel: AtomicBool,
}

impl Progress {
    pub fn set(&self, done: u32, total: u32) {
        self.done.store(done, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
    }

    /// Asks the job to stop.
    pub fn request_stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// True once the user asked the job to stop; the job returns `Outcome::Cancelled` soon after.
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn counts(&self) -> (u32, u32) {
        (self.done.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed))
    }

    /// Done over total, 0 to 1 (0 while the total is not known).
    pub fn fraction(&self) -> f32 {
        let (done, total) = self.counts();
        if total == 0 {
            0.0
        } else {
            (done as f32 / total as f32).clamp(0.0, 1.0)
        }
    }
}

/// A job running on its own thread.
pub struct Job {
    pub kind: JobKind,
    /// What the progress card says, such as "Sampling firefox".
    pub label: String,
    progress: Arc<Progress>,
    rx: Receiver<Outcome>,
}

impl Job {
    /// Runs `work` on a new thread. The closure gets the progress to report to and to check for a stop request.
    pub fn start<F>(kind: JobKind, label: &str, work: F) -> Job
    where
        F: FnOnce(&Progress) -> Outcome + Send + 'static,
    {
        let progress = Arc::new(Progress::default());
        let (tx, rx) = channel();
        let shared = progress.clone();
        DispatchQueue::global(Qos::Utility).async_(move || {
            let outcome = work(&shared);
            // the window may have been closed meanwhile; nobody is waiting then
            DispatchQueue::main().async_(move || {
                let _ = tx.send(outcome);
            });
        });
        Job { kind, label: label.to_string(), progress, rx }
    }

    /// The outcome once the job has ended, else None. A job whose thread died without an answer fails.
    pub fn poll(&self) -> Option<Outcome> {
        match self.rx.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Outcome::Failed("The background task stopped unexpectedly".into())),
        }
    }

    /// Asks the job to stop.
    pub fn stop(&self) {
        self.progress.request_stop();
    }

    pub fn fraction(&self) -> f32 {
        self.progress.fraction()
    }

    pub fn counts(&self) -> (u32, u32) {
        self.progress.counts()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_for(job: &Job) -> Outcome {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(outcome) = job.poll() {
                return outcome;
            }
            assert!(Instant::now() < deadline, "the job did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_job_reports_its_outcome_once() {
        let job = Job::start(JobKind::Sample, "Sampling", |p| {
            p.set(3, 4);
            Outcome::Report { title: "T".into(), text: "body".into() }
        });
        assert_eq!(wait_for(&job), Outcome::Report { title: "T".into(), text: "body".into() });
        assert_eq!(job.counts(), (3, 4));
        assert!((job.fraction() - 0.75).abs() < 1e-6);
        assert_eq!(job.kind, JobKind::Sample);
        assert_eq!(job.label, "Sampling");
    }

    #[test]
    fn stopping_reaches_the_job() {
        let job = Job::start(JobKind::Diagnostics, "Gathering", |p| {
            while !p.cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
            Outcome::Cancelled
        });
        assert_eq!(job.poll(), None);
        job.stop();
        assert_eq!(wait_for(&job), Outcome::Cancelled);
    }

    #[test]
    fn a_job_that_panics_fails_instead_of_hanging() {
        let job = Job::start(JobKind::Sample, "Boom", |_| panic!("test panic"));
        match wait_for(&job) {
            Outcome::Failed(msg) => assert!(!msg.is_empty()),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn progress_without_a_total_is_zero() {
        let p = Progress::default();
        assert_eq!(p.fraction(), 0.0);
        p.set(9, 3);
        assert_eq!(p.fraction(), 1.0);
    }
}
