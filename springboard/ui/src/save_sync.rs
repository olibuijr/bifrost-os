//! Synced save directories: the state and conflict model behind an app's save container.
//!
//! Mirrors the reference `GameSaveSyncedDirectory` types: `State` (closed, conflicted, error, local, offline, ready,
//! syncing), `Version`, and `resolveConflicts(with:)`. Pure data with no I/O: the SDK's `save_sync` module builds the
//! states from the files it holds, and this module decides what an app may do in each state and how a conflict
//! between versions is settled. Payloads the official documentation leaves open (what `Error` carries, which version
//! `Local` names) are our choice; see `docs/parity/gamesave.md`.

use std::fmt;

/// One version of a directory (`GameSaveSyncedDirectory.Version`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    /// `localizedNameOfSavingComputer`: the device that wrote this version.
    pub saving_device: String,
    /// `modifiedDate`: seconds since the Unix epoch.
    pub modified: u64,
    /// `isLocal`: the version exists only on this device.
    pub is_local: bool,
    /// Fingerprint of the directory contents; equal values mean equal data.
    pub fingerprint: u64,
}

/// `GameSaveSyncedDirectory.State`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// `closed`: the directory was closed and no longer syncs.
    Closed,
    /// `conflicted(versions:)`: the cloud holds a version that differs from this device; the app must choose one.
    Conflicted { versions: Vec<Version> },
    /// `error(_:)`: the directory cannot be used; the message says why.
    Error(String),
    /// `local(_:)`: kept only on this device, not synced.
    Local(Version),
    /// `offline(_:)`: available on this device, not fully synced because the device is offline.
    Offline(Version),
    /// `ready(_:)`: fully synced and ready to use.
    Ready(Version),
    /// `syncing`: a sync is running and the data is not ready yet.
    Syncing,
}

impl State {
    /// True when the app may read and write files now (`local`, `offline` and `ready`).
    pub fn is_usable(&self) -> bool {
        matches!(self, State::Local(_) | State::Offline(_) | State::Ready(_))
    }

    /// The version the directory holds, when the state names one.
    pub fn version(&self) -> Option<&Version> {
        match self {
            State::Local(v) | State::Offline(v) | State::Ready(v) => Some(v),
            _ => None,
        }
    }
}

/// Why `resolve_conflicts` refused a choice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConflictError {
    /// The state has no conflict to resolve.
    NotConflicted,
    /// The chosen version is not one of the conflicting versions.
    UnknownVersion,
}

impl fmt::Display for ConflictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConflictError::NotConflicted => f.write_str("the directory has no conflict"),
            ConflictError::UnknownVersion => f.write_str("the chosen version is not in the conflict"),
        }
    }
}

/// `resolveConflicts(with:)`: keeps `with` as the directory's version and returns the state after the choice, which is
/// `Ready` with that version. Fails when `state` is not a conflict or `with` is not one of its versions.
pub fn resolve_conflicts(state: &State, with: &Version) -> Result<State, ConflictError> {
    match state {
        State::Conflicted { versions } if versions.contains(with) => Ok(State::Ready(with.clone())),
        State::Conflicted { .. } => Err(ConflictError::UnknownVersion),
        _ => Err(ConflictError::NotConflicted),
    }
}

/// The version to suggest first in a conflict: the latest modification, with ties broken by the larger fingerprint so
/// the choice is the same on every device.
pub fn newest(versions: &[Version]) -> Option<&Version> {
    versions.iter().max_by_key(|v| (v.modified, v.fingerprint))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(device: &str, modified: u64, fingerprint: u64) -> Version {
        Version { saving_device: device.to_string(), modified, is_local: false, fingerprint }
    }

    #[test]
    fn only_local_offline_and_ready_are_usable() {
        let v = version("Desk", 10, 1);
        assert!(State::Local(v.clone()).is_usable());
        assert!(State::Offline(v.clone()).is_usable());
        assert!(State::Ready(v.clone()).is_usable());
        assert!(!State::Syncing.is_usable());
        assert!(!State::Closed.is_usable());
        assert!(!State::Error("disk full".into()).is_usable());
        assert!(!State::Conflicted { versions: vec![v] }.is_usable());
    }

    #[test]
    fn version_is_read_back_from_the_states_that_hold_one() {
        let v = version("Desk", 10, 1);
        assert_eq!(State::Ready(v.clone()).version(), Some(&v));
        assert_eq!(State::Syncing.version(), None);
        assert_eq!(State::Conflicted { versions: vec![v] }.version(), None);
    }

    #[test]
    fn resolving_a_conflict_keeps_the_chosen_version_as_ready() {
        let cloud = version("Laptop", 20, 7);
        let local = version("Desk", 15, 3);
        let state = State::Conflicted { versions: vec![cloud.clone(), local.clone()] };
        assert_eq!(resolve_conflicts(&state, &local), Ok(State::Ready(local)));
        assert_eq!(resolve_conflicts(&state, &cloud), Ok(State::Ready(cloud)));
    }

    #[test]
    fn resolving_rejects_an_unknown_version_and_a_state_without_conflict() {
        let a = version("Desk", 15, 3);
        let stranger = version("Phone", 99, 9);
        let state = State::Conflicted { versions: vec![a.clone()] };
        assert_eq!(resolve_conflicts(&state, &stranger), Err(ConflictError::UnknownVersion));
        assert_eq!(resolve_conflicts(&State::Ready(a.clone()), &a), Err(ConflictError::NotConflicted));
        assert_eq!(resolve_conflicts(&State::Syncing, &a), Err(ConflictError::NotConflicted));
    }

    #[test]
    fn newest_prefers_the_latest_modification_then_the_larger_fingerprint() {
        let old = version("Desk", 10, 50);
        let new_low = version("Laptop", 30, 1);
        let new_high = version("Phone", 30, 2);
        let all = [old, new_low, new_high.clone()];
        assert_eq!(newest(&all), Some(&new_high));
        assert_eq!(newest(&[]), None);
    }

    #[test]
    fn conflict_errors_have_readable_messages() {
        assert_eq!(ConflictError::NotConflicted.to_string(), "the directory has no conflict");
        assert!(ConflictError::UnknownVersion.to_string().contains("not in the conflict"));
    }
}
