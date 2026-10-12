//! Accessory hotspot joins for apps: the request an app makes to have a Wi-Fi accessory join a network, one join at a
//! time, with the same checks a hotspot configuration gets.
//!
//! This is a pure model with no device access. The transport that talks to the accessory calls `finish` with the
//! outcome, and the state tells the app whether a join is still running. A new join is refused while one is running,
//! so an app cannot send two sets of credentials to the same accessory at once.

use crate::hotspot_configuration::{HotspotConfiguration, HotspotError};
use std::fmt;

/// Where the accessory join stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinState {
    /// No join has been started, or the last one was taken off the list.
    Idle,
    /// The accessory is joining `ssid`. The transport has not reported an outcome yet.
    Joining { accessory: String, ssid: String },
    /// The accessory joined `ssid`.
    Joined { accessory: String, ssid: String },
    /// The last join ended with this error.
    Failed(HotspotError),
}

impl fmt::Display for JoinState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JoinState::Idle => write!(f, "Idle"),
            JoinState::Joining { accessory, ssid } => write!(f, "Joining {ssid} with {accessory}"),
            JoinState::Joined { accessory, ssid } => write!(f, "{accessory} joined {ssid}"),
            JoinState::Failed(e) => write!(f, "Failed: {e}"),
        }
    }
}

/// The join of one accessory to one network at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessoryHotspotJoin {
    state: JoinState,
}

impl Default for AccessoryHotspotJoin {
    fn default() -> Self {
        AccessoryHotspotJoin { state: JoinState::Idle }
    }
}

impl AccessoryHotspotJoin {
    /// A join with nothing running.
    pub fn new() -> Self {
        AccessoryHotspotJoin::default()
    }

    /// The current state, for the app to show.
    pub fn state(&self) -> &JoinState {
        &self.state
    }

    /// Starts a join of `accessory` to a protected network with `passphrase` (WPA/WPA2 personal). The name and
    /// passphrase are checked first, and nothing starts when they are not valid.
    pub fn join(&mut self, accessory: &str, ssid: &str, passphrase: &str) -> Result<(), HotspotError> {
        self.start(accessory, HotspotConfiguration::with_passphrase(ssid, passphrase, false))
    }

    /// Starts a join of `accessory` to an open network, with no passphrase.
    pub fn join_without_security(&mut self, accessory: &str, ssid: &str) -> Result<(), HotspotError> {
        self.start(accessory, HotspotConfiguration::open(ssid))
    }

    /// Records the outcome the transport reports for the running join. Returns false, and changes nothing, when no
    /// join is running.
    pub fn finish(&mut self, result: Result<(), HotspotError>) -> bool {
        let (accessory, ssid) = match &self.state {
            JoinState::Joining { accessory, ssid } => (accessory.clone(), ssid.clone()),
            _ => return false,
        };
        self.state = match result {
            Ok(()) => JoinState::Joined { accessory, ssid },
            Err(e) => JoinState::Failed(e),
        };
        true
    }

    fn start(&mut self, accessory: &str, config: HotspotConfiguration) -> Result<(), HotspotError> {
        if matches!(self.state, JoinState::Joining { .. }) {
            return Err(HotspotError::Pending);
        }
        config.validate()?;
        self.state = JoinState::Joining { accessory: accessory.to_string(), ssid: config.ssid().to_string() };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_join_starts_and_names_the_accessory_and_network() {
        let mut j = AccessoryHotspotJoin::new();
        assert_eq!(j.join("Thermostat", "Home", "12345678"), Ok(()));
        assert_eq!(
            j.state(),
            &JoinState::Joining { accessory: "Thermostat".to_string(), ssid: "Home".to_string() }
        );
        assert_eq!(j.state().to_string(), "Joining Home with Thermostat");
    }

    #[test]
    fn a_join_with_a_bad_passphrase_is_refused_and_stays_idle() {
        let mut j = AccessoryHotspotJoin::new();
        assert_eq!(j.join("Thermostat", "Home", "short"), Err(HotspotError::InvalidWpaPassphrase));
        assert_eq!(j.state(), &JoinState::Idle);
    }

    #[test]
    fn a_second_join_while_one_runs_is_pending_and_changes_nothing() {
        let mut j = AccessoryHotspotJoin::new();
        j.join("Thermostat", "Home", "12345678").unwrap();
        let before = j.state().clone();
        assert_eq!(j.join("Plug", "Office", "87654321"), Err(HotspotError::Pending));
        assert_eq!(j.join_without_security("Plug", "Guest"), Err(HotspotError::Pending));
        assert_eq!(j.state(), &before);
    }

    #[test]
    fn finish_records_success_and_is_refused_when_nothing_runs() {
        let mut j = AccessoryHotspotJoin::new();
        assert!(!j.finish(Ok(())), "no join is running, so there is nothing to finish");
        assert_eq!(j.state(), &JoinState::Idle);
        j.join_without_security("Plug", "Guest").unwrap();
        assert!(j.finish(Ok(())));
        assert_eq!(j.state(), &JoinState::Joined { accessory: "Plug".to_string(), ssid: "Guest".to_string() });
    }

    #[test]
    fn a_failed_join_frees_the_accessory_for_a_retry() {
        let mut j = AccessoryHotspotJoin::new();
        j.join("Thermostat", "Home", "12345678").unwrap();
        assert!(j.finish(Err(HotspotError::UserDenied)));
        assert_eq!(j.state(), &JoinState::Failed(HotspotError::UserDenied));
        assert_eq!(j.state().to_string(), "Failed: The user refused the network change");
        assert_eq!(j.join("Thermostat", "Home", "12345678"), Ok(()), "a finished join does not block the next one");
    }

    #[test]
    fn an_open_join_needs_only_a_valid_name() {
        let mut j = AccessoryHotspotJoin::new();
        assert_eq!(j.join_without_security("Plug", ""), Err(HotspotError::InvalidSsid));
        assert_eq!(j.state(), &JoinState::Idle);
        assert_eq!(j.join_without_security("Plug", "Guest"), Ok(()));
    }
}
