//! Background suspension (RunningBoard, task 008): an app that nothing shows is stopped with `SIGSTOP` after the grace
//! period in `bifrost_core::occlusion`, and `SIGCONT` resumes it when it is focused or seen again. It runs only while
//! the Settings switch "Suspend Background Apps" (`suspend_background`) is on; turning the switch off resumes every
//! app it stopped.
//!
//! The per-app run state is `bifrost_core::occlusion::Occlusion`. This file keeps one model per external app slot and
//! feeds it the shell's facts. Covering is not known yet, so an app with any shown window counts as on screen.

use bifrost_core::occlusion::{signal_for, Event, Occlusion, Run, SIGCONT};
use std::collections::HashMap;

/// What the shell knows about one running external app at an update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    pub focused: bool,
    pub on_screen: bool,
}

/// One signal to send: the external app slot and the signal number.
pub type Signal = (u8, i32);

/// The suspension state of the external apps, by slot.
#[derive(Debug, Default)]
pub struct Suspender {
    apps: HashMap<u8, Occlusion>,
}

impl Suspender {
    /// Feeds the facts of each running app at `now_ms` and returns the signals the transitions call for. An app that is
    /// not in `facts` (it quit) loses its state and gets no signal.
    pub fn update(&mut self, facts: &[(u8, Facts)], now_ms: u64) -> Vec<Signal> {
        let mut signals = Vec::new();
        for &(slot, f) in facts {
            let model = self.apps.entry(slot).or_insert_with(Occlusion::new);
            for event in [Event::Focus(f.focused), Event::OnScreen(f.on_screen)] {
                if let Some(sig) = signal_for(model.report(event, now_ms)) {
                    signals.push((slot, sig));
                }
            }
            // the grace period runs out with no change of fact, so check it on every update too
            if let Some(sig) = signal_for(model.tick(now_ms)) {
                signals.push((slot, sig));
            }
        }
        self.apps.retain(|slot, _| facts.iter().any(|(s, _)| s == slot));
        signals
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

    const HIDDEN: Facts = Facts { focused: false, on_screen: false };
    const SHOWN: Facts = Facts { focused: false, on_screen: true };

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
    fn focus_resumes_a_stopped_app() {
        let mut s = Suspender::default();
        s.update(&[(3, HIDDEN)], 0);
        s.update(&[(3, HIDDEN)], GRACE_MS);
        let focused = Facts { focused: true, on_screen: false };
        assert_eq!(s.update(&[(3, focused)], GRACE_MS + 5), vec![(3, SIGCONT)]);
        assert!(!s.is_suspended(3));
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
