//! Feed refresh status for the Podcasts list. Feeds load on worker threads; the app loop applies their results.
//! Each feed that starts or finishes sends one event on a subject, and the app pulls those events through an
//! async publisher sequence (`bifrost_app::async_publisher`) on the loop's tick. The counts become the status
//! line ("Updating feeds, 1 of 3") while a batch of feeds is in flight, and they reset when the next batch starts.

use bifrost_app::async_publisher::{AsyncPublisher, Step};
use bifrost_app::observable::PassthroughSubject;

/// One step of a feed refresh: a feed started loading, or it finished (and loaded or failed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Started,
    Finished { ok: bool },
}

/// The counts for the current batch of feed loads.
pub struct Status {
    subject: PassthroughSubject<Event>,
    sequence: AsyncPublisher<Event, ()>,
    total: usize,
    done: usize,
    failed: usize,
}

impl Status {
    /// No feed has started.
    pub fn new() -> Self {
        let subject = PassthroughSubject::new();
        let sequence = AsyncPublisher::from_subject(&subject);
        Status { subject, sequence, total: 0, done: 0, failed: 0 }
    }

    /// A feed started loading.
    pub fn started(&self) {
        self.subject.send(Event::Started);
    }

    /// A feed finished, loaded (`ok`) or failed.
    pub fn finished(&self, ok: bool) {
        self.subject.send(Event::Finished { ok });
    }

    /// Applies every event sent since the last call. Returns true when there were any.
    pub fn pump(&mut self) -> bool {
        let mut any = false;
        while let Step::Value(event) = self.sequence.next() {
            any = true;
            match event {
                Event::Started => {
                    // a new batch starts once the last one is done
                    if self.done >= self.total {
                        self.total = 0;
                        self.done = 0;
                        self.failed = 0;
                    }
                    self.total += 1;
                }
                Event::Finished { ok } => {
                    self.done += 1;
                    if !ok {
                        self.failed += 1;
                    }
                }
            }
        }
        any
    }

    /// Feeds finished and feeds in the batch, while the batch is still running.
    pub fn progress(&self) -> Option<(usize, usize)> {
        if self.total == 0 || self.done >= self.total {
            None
        } else {
            Some((self.done, self.total))
        }
    }

    /// Feeds in the current batch that failed to load.
    pub fn failed(&self) -> usize {
        self.failed
    }

    /// The status line for the list, or None when no batch is running. Failed feeds are named once there are any.
    pub fn text(&self) -> Option<String> {
        self.progress().map(|(done, total)| match self.failed() {
            0 => format!("Updating feeds, {done} of {total}"),
            n => format!("Updating feeds, {done} of {total}, {n} failed"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_feeds_means_no_status() {
        let mut s = Status::new();
        assert!(!s.pump());
        assert_eq!(s.progress(), None);
        assert_eq!(s.text(), None);
    }

    #[test]
    fn a_batch_counts_started_and_finished_feeds() {
        let mut s = Status::new();
        s.started();
        s.started();
        assert!(s.pump());
        assert_eq!(s.progress(), Some((0, 2)));
        s.finished(true);
        s.pump();
        assert_eq!(s.progress(), Some((1, 2)));
        assert_eq!(s.text().as_deref(), Some("Updating feeds, 1 of 2"));
    }

    #[test]
    fn the_status_line_names_failed_feeds_while_the_batch_runs() {
        let mut s = Status::new();
        s.started();
        s.started();
        s.started();
        s.finished(false);
        s.pump();
        assert_eq!(s.text().as_deref(), Some("Updating feeds, 1 of 3, 1 failed"));
    }

    #[test]
    fn the_batch_ends_when_every_feed_has_finished_and_counts_failures() {
        let mut s = Status::new();
        s.started();
        s.started();
        s.finished(true);
        s.finished(false);
        s.pump();
        assert_eq!(s.progress(), None);
        assert_eq!(s.text(), None);
        assert_eq!(s.failed(), 1);
    }

    #[test]
    fn the_next_batch_resets_the_counts() {
        let mut s = Status::new();
        s.started();
        s.started();
        s.finished(false);
        s.finished(true);
        s.pump();
        s.started();
        s.pump();
        assert_eq!(s.progress(), Some((0, 1)));
        assert_eq!(s.failed(), 0);
    }

    #[test]
    fn events_wait_for_pump() {
        let mut s = Status::new();
        s.started();
        assert_eq!(s.progress(), None);
        s.pump();
        assert_eq!(s.progress(), Some((0, 1)));
        assert!(!s.pump());
    }
}
