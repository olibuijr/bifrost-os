//! Code signing validity events (task theme-28). A monitor tracks whether each running process still matches the code
//! identity it was launched with, and reports the first time it stops matching. Callers supply the identities, read
//! from the package manifest or from their own reader; this module reads no process table, no signature and no file,
//! and runs no timer. BifrOSt does not sign code yet, so this is the contract for the events, not a live feed.

use std::collections::BTreeMap;

use crate::codereq::Identity;

/// Why a process no longer matches its launch identity. These categories are BifrOSt's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The current code carries no signature.
    Unsigned,
    /// The code directory hash differs from the one recorded at launch, so the code on disk changed.
    CodeChanged,
    /// The team or signing identifier differs from the one recorded at launch.
    IdentityChanged,
}

/// The validity of one process at one observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validity {
    Valid,
    Invalid(Reason),
}

/// Compares the current identity of a program with the identity recorded at launch. A field is compared only when the
/// launch record has it, so a missing hash or team never invalidates. Unsigned code is invalid even with no record.
pub fn classify(current: &Identity, launch: Option<&Identity>) -> Validity {
    if !current.signed {
        return Validity::Invalid(Reason::Unsigned);
    }
    let Some(launch) = launch else {
        return Validity::Valid;
    };
    if let (Some(recorded), Some(now)) = (&launch.code_directory_hash, &current.code_directory_hash) {
        if recorded != now {
            return Validity::Invalid(Reason::CodeChanged);
        }
    }
    let team_changed = launch.team_id.is_some() && launch.team_id != current.team_id;
    let signing_changed = launch.signing_id.is_some() && launch.signing_id != current.signing_id;
    if team_changed || signing_changed {
        return Validity::Invalid(Reason::IdentityChanged);
    }
    Validity::Valid
}

/// A process whose code signing status became invalid. One event is emitted per process and invalidation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub pid: u32,
    pub reason: Reason,
    /// Counts events in the order the monitor emitted them, starting at 1.
    pub seq: u64,
}

/// Tracks the last known validity of each process.
#[derive(Debug, Default)]
pub struct Monitor {
    status: BTreeMap<u32, Validity>,
    seq: u64,
}

impl Monitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one observation and returns an event only when the process first becomes invalid. Invalidation is
    /// sticky: once a process is invalid, a later valid observation does not clear it, and repeats do not emit again,
    /// until `exit` forgets the process.
    pub fn observe(&mut self, pid: u32, validity: Validity) -> Option<Event> {
        if let Some(Validity::Invalid(_)) = self.status.get(&pid) {
            return None;
        }
        match validity {
            Validity::Valid => {
                self.status.insert(pid, Validity::Valid);
                None
            }
            Validity::Invalid(reason) => {
                self.status.insert(pid, Validity::Invalid(reason));
                self.seq += 1;
                Some(Event { pid, reason, seq: self.seq })
            }
        }
    }

    /// Forgets a process that exited, so a new process with the same id starts clean.
    pub fn exit(&mut self, pid: u32) {
        self.status.remove(&pid);
    }

    /// The last recorded validity of a process, or None when it was never observed or has exited.
    pub fn status(&self, pid: u32) -> Option<Validity> {
        self.status.get(&pid).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(hash: &str, team: &str) -> Identity {
        Identity {
            signed: true,
            team_id: Some(team.to_string()),
            signing_id: Some("app.example".to_string()),
            code_directory_hash: Some(hash.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn unsigned_code_is_invalid_even_without_a_launch_record() {
        assert_eq!(classify(&Identity::default(), None), Validity::Invalid(Reason::Unsigned));
    }

    #[test]
    fn matching_identity_is_valid() {
        assert_eq!(classify(&app("aa", "T1"), Some(&app("aa", "T1"))), Validity::Valid);
    }

    #[test]
    fn changed_code_hash_and_changed_team_have_their_own_reasons() {
        assert_eq!(
            classify(&app("bb", "T1"), Some(&app("aa", "T1"))),
            Validity::Invalid(Reason::CodeChanged)
        );
        assert_eq!(
            classify(&app("aa", "T2"), Some(&app("aa", "T1"))),
            Validity::Invalid(Reason::IdentityChanged)
        );
    }

    #[test]
    fn a_launch_record_without_a_hash_does_not_invalidate() {
        let mut launch = app("aa", "T1");
        launch.code_directory_hash = None;
        assert_eq!(classify(&app("bb", "T1"), Some(&launch)), Validity::Valid);
    }

    #[test]
    fn monitor_emits_once_and_stays_invalid_until_exit() {
        let mut m = Monitor::new();
        assert_eq!(m.observe(7, Validity::Valid), None);
        assert_eq!(
            m.observe(7, Validity::Invalid(Reason::CodeChanged)),
            Some(Event { pid: 7, reason: Reason::CodeChanged, seq: 1 })
        );
        assert_eq!(m.observe(7, Validity::Valid), None);
        assert_eq!(m.status(7), Some(Validity::Invalid(Reason::CodeChanged)));
        assert_eq!(m.observe(7, Validity::Invalid(Reason::Unsigned)), None);
    }

    #[test]
    fn a_reused_process_id_can_be_invalidated_again_after_exit() {
        let mut m = Monitor::new();
        assert_eq!(m.observe(9, Validity::Invalid(Reason::Unsigned)).map(|e| e.seq), Some(1));
        m.exit(9);
        assert_eq!(m.status(9), None);
        assert_eq!(m.observe(9, Validity::Invalid(Reason::Unsigned)).map(|e| e.seq), Some(2));
    }
}
