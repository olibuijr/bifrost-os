//! Install coordination: one queue of app installs over the store installer
//! (`install::install_download`). It serves the install daemon, the install
//! authorisation agent, the install coordinator and the progress reporter: a
//! request is checked against the session owner, installs run one at a time,
//! and every phase change or byte count is kept as an event for the caller to
//! drain (the App Store Updates tab and the Today widget read those events).
//!
//! The coordinator does no I/O. The shell starts the worker thread, feeds
//! `progress` from the download counter and calls `finish` with the result.

/// Where an install is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Queued,
    Downloading,
    Verifying,
    Installed,
    Failed,
    Cancelled,
}

impl Phase {
    /// True once no more events will come for the job.
    pub fn is_final(self) -> bool {
        matches!(self, Phase::Installed | Phase::Failed | Phase::Cancelled)
    }
}

/// One thing that happened to a job, in the order it happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Phase { id: String, phase: Phase },
    Progress { id: String, done: u64, total: u64 },
}

/// Why a request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The requester is neither the session owner nor root.
    Unauthorised,
    /// An install of the same app is already queued or running.
    AlreadyQueued,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub id: String,
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
    /// The error message when the job failed.
    pub error: Option<String>,
}

impl Job {
    /// Share of the package downloaded, 0.0 to 1.0. An unknown size reads as 0.
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        (self.done as f32 / self.total as f32).clamp(0.0, 1.0)
    }
}

/// Queue of installs for one session. Finished jobs are dropped from the queue.
#[derive(Debug)]
pub struct Coordinator {
    owner_uid: u32,
    jobs: Vec<Job>,
    events: Vec<Event>,
}

impl Coordinator {
    pub fn new(owner_uid: u32) -> Self {
        Coordinator { owner_uid, jobs: Vec::new(), events: Vec::new() }
    }

    /// Queues an install for `id` on behalf of `uid`. Only the session owner
    /// and root may ask, and one app has at most one live install.
    pub fn request(&mut self, id: &str, uid: u32, total: u64) -> Result<(), Refused> {
        if uid != self.owner_uid && uid != 0 {
            return Err(Refused::Unauthorised);
        }
        if self.jobs.iter().any(|j| j.id == id) {
            return Err(Refused::AlreadyQueued);
        }
        self.jobs.push(Job { id: id.to_string(), phase: Phase::Queued, done: 0, total, error: None });
        self.events.push(Event::Phase { id: id.to_string(), phase: Phase::Queued });
        Ok(())
    }

    /// Starts the oldest queued job if nothing is downloading. Returns its id,
    /// which the caller passes to the store installer.
    pub fn start_next(&mut self) -> Option<String> {
        if self.jobs.iter().any(|j| j.phase == Phase::Downloading) {
            return None;
        }
        let job = self.jobs.iter_mut().find(|j| j.phase == Phase::Queued)?;
        job.phase = Phase::Downloading;
        let id = job.id.clone();
        self.events.push(Event::Phase { id: id.clone(), phase: Phase::Downloading });
        Some(id)
    }

    /// Records the bytes downloaded so far. Ignored for jobs that are not downloading.
    pub fn progress(&mut self, id: &str, done: u64) {
        let Some(job) = self.jobs.iter_mut().find(|j| j.id == id && j.phase == Phase::Downloading) else {
            return;
        };
        let done = if job.total > 0 { done.min(job.total) } else { done };
        if done == job.done {
            return;
        }
        job.done = done;
        let total = job.total;
        self.events.push(Event::Progress { id: id.to_string(), done, total });
    }

    /// The download is complete and the package is being hashed and unpacked.
    pub fn verify(&mut self, id: &str) {
        self.set_phase(id, Phase::Verifying, Some(Phase::Downloading));
    }

    /// Ends the job: `Ok` is `Installed`, `Err` is `Failed` with the message kept.
    pub fn finish(&mut self, id: &str, result: Result<(), String>) {
        let Some(pos) = self.jobs.iter().position(|j| j.id == id) else {
            return;
        };
        let mut job = self.jobs.remove(pos);
        if job.phase.is_final() {
            return;
        }
        job.phase = match result {
            Ok(()) => {
                job.done = job.total;
                Phase::Installed
            }
            Err(message) => {
                job.error = Some(message);
                Phase::Failed
            }
        };
        self.events.push(Event::Phase { id: id.to_string(), phase: job.phase });
    }

    /// Stops a job that is queued or downloading. Returns false if there was nothing to stop.
    pub fn cancel(&mut self, id: &str) -> bool {
        let Some(pos) = self.jobs.iter().position(|j| j.id == id) else {
            return false;
        };
        if matches!(self.jobs[pos].phase, Phase::Verifying) {
            return false;
        }
        let mut job = self.jobs.remove(pos);
        job.phase = Phase::Cancelled;
        self.events.push(Event::Phase { id: id.to_string(), phase: Phase::Cancelled });
        true
    }

    pub fn job(&self, id: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    /// Live jobs in the order they were requested.
    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    /// Takes every event since the last call, oldest first.
    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    fn set_phase(&mut self, id: &str, phase: Phase, only_from: Option<Phase>) {
        let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) else {
            return;
        };
        if only_from.is_some_and(|from| from != job.phase) {
            return;
        }
        job.phase = phase;
        self.events.push(Event::Phase { id: id.to_string(), phase });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_owner_or_root_may_request_an_install() {
        let mut c = Coordinator::new(1000);
        assert_eq!(c.request("is.olibuijr.notes", 1001, 10), Err(Refused::Unauthorised));
        assert!(c.request("is.olibuijr.notes", 1000, 10).is_ok());
        assert!(c.request("is.olibuijr.maps", 0, 10).is_ok());
    }

    #[test]
    fn one_live_install_per_app() {
        let mut c = Coordinator::new(1000);
        c.request("is.olibuijr.notes", 1000, 10).unwrap();
        assert_eq!(c.request("is.olibuijr.notes", 1000, 10), Err(Refused::AlreadyQueued));
    }

    #[test]
    fn installs_run_one_at_a_time_in_request_order() {
        let mut c = Coordinator::new(1000);
        c.request("a.one", 1000, 4).unwrap();
        c.request("a.two", 1000, 4).unwrap();
        assert_eq!(c.start_next().as_deref(), Some("a.one"));
        assert_eq!(c.start_next(), None, "a second download waits for the first");
        c.finish("a.one", Ok(()));
        assert_eq!(c.start_next().as_deref(), Some("a.two"));
    }

    #[test]
    fn progress_reports_bytes_and_ignores_repeats_and_overshoot() {
        let mut c = Coordinator::new(1000);
        c.request("a.one", 1000, 100).unwrap();
        c.start_next();
        c.progress("a.one", 40);
        c.progress("a.one", 40);
        c.progress("a.one", 500);
        let events = c.drain_events();
        let progress: Vec<&Event> = events.iter().filter(|e| matches!(e, Event::Progress { .. })).collect();
        assert_eq!(progress.len(), 2);
        assert_eq!(c.job("a.one").unwrap().done, 100);
        assert!((c.job("a.one").unwrap().fraction() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_job_moves_through_verify_to_installed_and_leaves_the_queue() {
        let mut c = Coordinator::new(1000);
        c.request("a.one", 1000, 8).unwrap();
        c.start_next();
        c.progress("a.one", 8);
        c.verify("a.one");
        assert_eq!(c.job("a.one").unwrap().phase, Phase::Verifying);
        c.finish("a.one", Ok(()));
        assert!(c.job("a.one").is_none());
        let phases: Vec<Phase> = c
            .drain_events()
            .into_iter()
            .filter_map(|e| match e {
                Event::Phase { phase, .. } => Some(phase),
                _ => None,
            })
            .collect();
        assert_eq!(phases, vec![Phase::Queued, Phase::Downloading, Phase::Verifying, Phase::Installed]);
    }

    #[test]
    fn a_failure_keeps_its_message_and_frees_the_slot() {
        let mut c = Coordinator::new(1000);
        c.request("a.one", 1000, 8).unwrap();
        c.start_next();
        c.finish("a.one", Err("sha256 mismatch".into()));
        assert!(c.jobs().is_empty());
        assert!(c.drain_events().contains(&Event::Phase { id: "a.one".into(), phase: Phase::Failed }));
    }

    #[test]
    fn cancel_stops_a_download_but_not_a_verify() {
        let mut c = Coordinator::new(1000);
        c.request("a.one", 1000, 8).unwrap();
        c.start_next();
        c.verify("a.one");
        assert!(!c.cancel("a.one"));
        c.request("a.two", 1000, 8).unwrap();
        assert!(c.cancel("a.two"));
        assert!(c.job("a.two").is_none());
        assert!(!c.cancel("a.two"));
    }

    #[test]
    fn an_unknown_size_reads_as_zero_fraction() {
        let job = Job { id: "a.one".into(), phase: Phase::Downloading, done: 5, total: 0, error: None };
        assert_eq!(job.fraction(), 0.0);
    }
}
