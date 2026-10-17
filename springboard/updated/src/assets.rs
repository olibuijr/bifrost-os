//! Background asset pack downloads for the update agent (the asset daemon). Each app has one managed download queue
//! (`bifrost_ui::managed_download`), which owns the queue rules: budget, metering, priority and three attempts. This
//! module keeps one queue per app, starts a new budget day, and decides when a background pass may run at all.

use bifrost_ui::managed_download::{Conditions, ManagedDownloads, RequestError};
use std::collections::BTreeMap;

/// Bytes an app may fetch in the background each day.
pub const DAILY_BUDGET: u64 = 512 * 1024 * 1024;

/// The asset daemon: the managed download queues of all apps, run while the machine is idle.
#[derive(Default)]
pub struct AssetDaemon {
    queues: BTreeMap<String, ManagedDownloads>,
    day: u64,
}

impl AssetDaemon {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues `pack` for `app`. Refused when that pack is already queued, downloading or done.
    pub fn request(&mut self, app: &str, pack: &str, bytes: u64, priority: u8) -> Result<(), RequestError> {
        self.queues.entry(app.to_string()).or_insert_with(|| ManagedDownloads::new(DAILY_BUDGET)).request(pack, bytes, priority)
    }

    /// The queue of `app`, if it has ever requested a pack.
    pub fn queue(&self, app: &str) -> Option<&ManagedDownloads> {
        self.queues.get(app)
    }

    /// Runs every app's queue under `conditions` until nothing more may start. `day` is the current day number (days
    /// since the Unix epoch): when it changes, every queue gets its budget back. `transfer` is called with the app and
    /// pack id and returns true when the pack arrived. Returns how many packs finished.
    pub fn pass<F: FnMut(&str, &str) -> bool>(&mut self, day: u64, conditions: Conditions, mut transfer: F) -> usize {
        if day != self.day {
            self.day = day;
            for q in self.queues.values_mut() {
                q.reset_day();
            }
        }
        let mut done = 0;
        for (app, q) in self.queues.iter_mut() {
            done += q.run_all(conditions, |pack| transfer(app.as_str(), pack));
        }
        done
    }
}

/// The conditions of a background pass from the update agent's state: the machine must be on an unmetered (Wi-Fi or
/// wired) connection, and a pass never runs in low-power mode. `None` means no pass may run.
pub fn background_conditions(wifi: bool, low_power: bool) -> Option<Conditions> {
    if low_power || !wifi {
        return None;
    }
    Some(Conditions { network_available: true, unmetered: true, app_running: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::managed_download::PackState;

    const WIRED: Conditions = Conditions { network_available: true, unmetered: true, app_running: false };
    const METERED: Conditions = Conditions { network_available: true, unmetered: false, app_running: false };

    #[test]
    fn a_wired_pass_fetches_the_queued_pack() {
        let mut d = AssetDaemon::new();
        d.request("levels", "maps-eu", 10, 1).unwrap();
        let done = d.pass(1, WIRED, |app, pack| {
            assert_eq!((app, pack), ("levels", "maps-eu"));
            true
        });
        assert_eq!(done, 1);
        assert_eq!(d.queue("levels").unwrap().state("maps-eu"), Some(PackState::Done));
    }

    #[test]
    fn a_metered_pass_fetches_nothing() {
        let mut d = AssetDaemon::new();
        d.request("levels", "maps-eu", 10, 1).unwrap();
        let mut called = false;
        assert_eq!(d.pass(1, METERED, |_, _| {
            called = true;
            true
        }), 0);
        assert!(!called);
        assert_eq!(d.queue("levels").unwrap().state("maps-eu"), Some(PackState::Queued));
    }

    #[test]
    fn a_duplicate_request_is_refused() {
        let mut d = AssetDaemon::new();
        d.request("levels", "maps-eu", 10, 1).unwrap();
        assert_eq!(d.request("levels", "maps-eu", 10, 1), Err(RequestError::Duplicate));
    }

    #[test]
    fn a_failing_pack_gets_three_attempts_then_fails() {
        let mut d = AssetDaemon::new();
        d.request("levels", "maps-eu", 10, 1).unwrap();
        let mut attempts = 0;
        assert_eq!(d.pass(1, WIRED, |_, _| {
            attempts += 1;
            false
        }), 0);
        assert_eq!(attempts, 3);
        assert_eq!(d.queue("levels").unwrap().state("maps-eu"), Some(PackState::Failed));
    }

    #[test]
    fn the_budget_returns_on_a_new_day() {
        let mut d = AssetDaemon::new();
        d.request("levels", "big", DAILY_BUDGET, 1).unwrap();
        d.request("levels", "next", 1, 1).unwrap();
        assert_eq!(d.pass(1, WIRED, |_, _| true), 1);
        assert_eq!(d.queue("levels").unwrap().state("next"), Some(PackState::Queued));
        assert_eq!(d.pass(2, WIRED, |_, _| true), 1);
        assert_eq!(d.queue("levels").unwrap().state("next"), Some(PackState::Done));
    }

    #[test]
    fn background_passes_stop_in_low_power_and_on_metered_links() {
        assert_eq!(background_conditions(true, false), Some(WIRED));
        assert_eq!(background_conditions(true, true), None);
        assert_eq!(background_conditions(false, false), None);
    }
}
