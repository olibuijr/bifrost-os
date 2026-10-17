//! Background suspension (RunningBoard, task 008): an app that nothing shows is stopped with `SIGSTOP` after the grace
//! period in `bifrost_core::occlusion`, and `SIGCONT` resumes it when it is focused or seen again. It runs only while
//! the Settings switch "Suspend Background Apps" (`suspend_background`) is on; turning the switch off resumes every
//! app it stopped.
//!
//! The per-app run state is `bifrost_core::occlusion::Occlusion`. This file keeps one model per external app slot and
//! feeds it the shell's facts: focus, whether a window is shown, and whether every shown window is covered by windows
//! of other apps. Process assertions an app takes through the `runningboard` service (`bifrost_core::process_assertion`) are
//! reported too: an app that holds one is never stopped while it lasts, and a lapsed one stops counting.

use bifrost_core::occlusion::{signal_for, Event, Occlusion, Run, SIGCONT};
use bifrost_core::process_assertion::{AssertionBook, Refused};
use std::collections::HashMap;

/// What the shell knows about one running external app at an update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    pub focused: bool,
    pub on_screen: bool,
    /// every shown window of the app is covered by windows of other apps (`occlusion::is_covered`)
    pub covered: bool,
}

/// One signal to send: the external app slot and the signal number.
pub type Signal = (u8, i32);

/// The suspension state of the external apps, by slot.
#[derive(Debug, Default)]
pub struct Suspender {
    apps: HashMap<u8, Occlusion>,
    book: AssertionBook,
}

impl Suspender {
    /// Feeds the facts of each running app at `now_ms` and returns the signals the transitions call for. An app that is
    /// not in `facts` (it quit) loses its state and gets no signal.
    pub fn update(&mut self, facts: &[(u8, Facts)], now_ms: u64) -> Vec<Signal> {
        let mut signals = Vec::new();
        self.book.expire(now_ms);
        for &(slot, f) in facts {
            let asserted = self.book.is_held(slot);
            let model = self.apps.entry(slot).or_insert_with(Occlusion::new);
            // the assertion goes first, so an app that took one while stopped is resumed before the other facts are read
            for event in [Event::Assertion(asserted), Event::Focus(f.focused), Event::OnScreen(f.on_screen), Event::Covered(f.covered)] {
                if let Some(sig) = signal_for(model.report(event, now_ms)) {
                    signals.push((slot, sig));
                }
            }
            // the grace period runs out with no change of fact, so check it on every update too
            if let Some(sig) = signal_for(model.tick(now_ms)) {
                signals.push((slot, sig));
            }
        }
        let gone: Vec<u8> = self.apps.keys().copied().filter(|slot| !facts.iter().any(|(s, _)| s == slot)).collect();
        for slot in gone {
            self.apps.remove(&slot);
            self.book.forget(slot);
        }
        signals
    }

    /// Takes a process assertion for the app in `slot` (see `AssertionBook::take`). Returns its id.
    pub fn take_assertion(&mut self, slot: u8, reason: &str, limit_ms: Option<u64>, now_ms: u64) -> Result<u32, Refused> {
        self.book.take(slot, reason, limit_ms, now_ms)
    }

    /// Drops the assertion `id` of the app in `slot`. Returns whether it was held.
    pub fn release_assertion(&mut self, slot: u8, id: u32) -> bool {
        self.book.release(slot, id)
    }

    /// How many assertions the app in `slot` holds at `now_ms`.
    pub fn assertions_held(&mut self, slot: u8, now_ms: u64) -> usize {
        self.book.expire(now_ms);
        self.book.of(slot).len()
    }

    /// Resumes every app this suspender stopped and forgets them: the switch was turned off.
    pub fn resume_all(&mut self) -> Vec<Signal> {
        let mut signals: Vec<Signal> = self
            .apps
            .iter()
            .filter(|(_, m)| m.run() == Run::Suspended)
            .map(|(slot, _)| (*slot, SIGCONT))
            .collect();
        signals.sort_unstable();
        self.apps.clear();
        signals
    }

    /// Whether the app in `slot` is stopped by this suspender.
    pub fn is_suspended(&self, slot: u8) -> bool {
        self.apps.get(&slot).is_some_and(|m| m.run() == Run::Suspended)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::occlusion::{GRACE_MS, SIGSTOP};

    const HIDDEN: Facts = Facts { focused: false, on_screen: false, covered: false };
    const SHOWN: Facts = Facts { focused: false, on_screen: true, covered: false };
    const COVERED: Facts = Facts { focused: false, on_screen: true, covered: true };

    #[test]
    fn an_unseen_app_is_stopped_only_after_the_grace_period() {
        let mut s = Suspender::default();
        assert!(s.update(&[(2, HIDDEN)], 0).is_empty(), "the wait starts now");
        assert!(s.update(&[(2, HIDDEN)], GRACE_MS - 1).is_empty());
        assert_eq!(s.update(&[(2, HIDDEN)], GRACE_MS), vec![(2, SIGSTOP)]);
        assert!(s.is_suspended(2));
        assert!(s.update(&[(2, HIDDEN)], GRACE_MS + 10).is_empty(), "stopped once, not again");
    }

    #[test]
    fn a_shown_app_is_never_stopped() {
        let mut s = Suspender::default();
        assert!(s.update(&[(1, SHOWN)], 0).is_empty());
        assert!(s.update(&[(1, SHOWN)], GRACE_MS * 10).is_empty());
        assert!(!s.is_suspended(1));
    }

    #[test]
    fn a_covered_app_is_stopped_after_the_grace_period() {
        let mut s = Suspender::default();
        assert!(s.update(&[(7, COVERED)], 0).is_empty(), "the wait starts now");
        assert_eq!(s.update(&[(7, COVERED)], GRACE_MS), vec![(7, SIGSTOP)]);
        assert!(s.is_suspended(7));
    }

    #[test]
    fn focus_resumes_a_stopped_app() {
        let mut s = Suspender::default();
        s.update(&[(3, HIDDEN)], 0);
        s.update(&[(3, HIDDEN)], GRACE_MS);
        let focused = Facts { focused: true, on_screen: false, covered: false };
        assert_eq!(s.update(&[(3, focused)], GRACE_MS + 5), vec![(3, SIGCONT)]);
        assert!(!s.is_suspended(3));
    }

    #[test]
    fn a_held_assertion_keeps_an_unseen_app_running() {
        let mut s = Suspender::default();
        s.update(&[(2, HIDDEN)], 0);
        s.take_assertion(2, "Download", Some(10 * GRACE_MS), 0).unwrap();
        assert!(s.update(&[(2, HIDDEN)], 3 * GRACE_MS).is_empty(), "held: not stopped");
        assert!(!s.is_suspended(2));
        // the assertion lapses; the grace period starts then and the app is stopped after it
        assert!(s.update(&[(2, HIDDEN)], 10 * GRACE_MS).is_empty());
        assert_eq!(s.update(&[(2, HIDDEN)], 11 * GRACE_MS), vec![(2, SIGSTOP)]);
    }

    #[test]
    fn an_assertion_taken_by_a_stopped_app_resumes_it() {
        let mut s = Suspender::default();
        s.update(&[(2, HIDDEN)], 0);
        assert_eq!(s.update(&[(2, HIDDEN)], GRACE_MS), vec![(2, SIGSTOP)]);
        let id = s.take_assertion(2, "Export", None, GRACE_MS + 1).unwrap();
        assert_eq!(s.update(&[(2, HIDDEN)], GRACE_MS + 2), vec![(2, SIGCONT)]);
        assert_eq!(s.assertions_held(2, GRACE_MS + 3), 1);
        assert!(s.release_assertion(2, id));
        assert_eq!(s.assertions_held(2, GRACE_MS + 4), 0);
    }

    #[test]
    fn an_app_that_quit_loses_its_assertions() {
        let mut s = Suspender::default();
        s.update(&[(6, HIDDEN)], 0);
        s.take_assertion(6, "Download", None, 0).unwrap();
        s.update(&[], 1);
        assert_eq!(s.assertions_held(6, 2), 0);
    }

    #[test]
    fn turning_the_switch_off_resumes_every_stopped_app() {
        let mut s = Suspender::default();
        s.update(&[(4, HIDDEN), (5, SHOWN)], 0);
        s.update(&[(4, HIDDEN), (5, SHOWN)], GRACE_MS);
        assert_eq!(s.resume_all(), vec![(4, SIGCONT)]);
        assert!(s.resume_all().is_empty(), "nothing is left to resume");
    }

    #[test]
    fn an_app_that_quit_loses_its_state() {
        let mut s = Suspender::default();
        s.update(&[(6, HIDDEN)], 0);
        s.update(&[], GRACE_MS);
        assert!(!s.is_suspended(6));
        assert!(s.update(&[(6, SHOWN)], GRACE_MS + 1).is_empty(), "a new app in the slot starts fresh");
    }
}
