//! Screen Time configuration for apps: whether the device is enforcing child
//! restrictions, and an observer that reports when that changes.
//!
//! The shell keeps its Screen Time settings in `screentime.ini`, a `key=value`
//! file (`#` starts a comment). Restrictions are enforced while the Downtime
//! window is on or any app has a daily limit above zero, which is the same
//! rule the shell applies in `Config::block`. The observer has no dispatch
//! queue: the app calls `poll` from its own run loop, and `poll` reports a new
//! configuration only when the file changed and the answer differs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The key that turns the Downtime window on (`1`) or off.
pub const DOWNTIME_ON: &str = "downtime.on";
/// The prefix of every per-app daily limit key, in minutes.
pub const LIMIT_PREFIX: &str = "limit.";

/// The configuration for this device (`configuration` on the observer).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Configuration {
    /// Whether the device is currently enforcing child restrictions.
    pub enforces_child_restrictions: bool,
}

impl Configuration {
    /// Reads the configuration from `screentime.ini` text. For a key that
    /// appears more than once the last line wins, as the shell's store does.
    /// Downtime counts as on when its value parses as `1`. A limit counts when
    /// it parses as a number above zero.
    pub fn from_settings_text(text: &str) -> Self {
        let mut values: BTreeMap<&str, &str> = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                values.insert(key.trim(), value.trim());
            }
        }
        let downtime = values.get(DOWNTIME_ON).and_then(|v| v.parse::<i64>().ok()) == Some(1);
        let limited = values
            .iter()
            .any(|(key, value)| key.starts_with(LIMIT_PREFIX) && value.parse::<i64>().is_ok_and(|m| m > 0));
        Configuration { enforces_child_restrictions: downtime || limited }
    }
}

/// Reads the configuration from the settings file at `path`. A missing or
/// unreadable file means no restrictions, as an empty settings store does.
fn read_configuration(path: &Path) -> Configuration {
    std::fs::read_to_string(path).map(|text| Configuration::from_settings_text(&text)).unwrap_or_default()
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Observes the device configuration (`STScreenTimeConfigurationObserver`).
/// `new` takes the settings file to watch in place of an update queue.
#[derive(Debug)]
pub struct ConfigurationObserver {
    source: PathBuf,
    observing: bool,
    stamp: Option<SystemTime>,
    current: Configuration,
}

impl ConfigurationObserver {
    /// An observer that reads the settings file at `source` once observing starts.
    pub fn new(source: impl Into<PathBuf>) -> Self {
        ConfigurationObserver { source: source.into(), observing: false, stamp: None, current: Configuration::default() }
    }

    /// The configuration last read. It is the default until `start_observing` runs.
    pub fn configuration(&self) -> Configuration {
        self.current
    }

    pub fn is_observing(&self) -> bool {
        self.observing
    }

    /// Starts observing and reads the configuration now. Returns the value read.
    pub fn start_observing(&mut self) -> Configuration {
        self.observing = true;
        self.stamp = modified(&self.source);
        self.current = read_configuration(&self.source);
        self.current
    }

    /// Stops observing. `poll` returns `None` until observing starts again.
    pub fn stop_observing(&mut self) {
        self.observing = false;
    }

    /// Re-reads the file if it changed since the last read. Returns the new
    /// configuration only when it differs from the one before, so the app is
    /// told about real changes and not about an edit that changed nothing.
    pub fn poll(&mut self) -> Option<Configuration> {
        if !self.observing {
            return None;
        }
        let stamp = modified(&self.source);
        if stamp == self.stamp {
            return None;
        }
        self.stamp = stamp;
        let next = read_configuration(&self.source);
        if next == self.current {
            return None;
        }
        self.current = next;
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_unrelated_settings_enforce_nothing() {
        assert!(!Configuration::from_settings_text("").enforces_child_restrictions);
        assert!(!Configuration::from_settings_text("limit.files=0\ndowntime.on=0\n").enforces_child_restrictions);
        assert!(!Configuration::from_settings_text("allow.clock=1\ndowntime.start=1320\n").enforces_child_restrictions);
    }

    #[test]
    fn downtime_on_enforces_and_off_does_not() {
        assert!(Configuration::from_settings_text("downtime.on=1\n").enforces_child_restrictions);
        assert!(!Configuration::from_settings_text("downtime.on=2\n").enforces_child_restrictions);
        assert!(Configuration::from_settings_text("downtime.on=01\n").enforces_child_restrictions);
    }

    #[test]
    fn a_positive_daily_limit_enforces_and_zero_or_negative_does_not() {
        assert!(Configuration::from_settings_text("limit.files=30\n").enforces_child_restrictions);
        assert!(!Configuration::from_settings_text("limit.files=0\n").enforces_child_restrictions);
        assert!(!Configuration::from_settings_text("limit.files=-5\n").enforces_child_restrictions);
        assert!(!Configuration::from_settings_text("limit.files=soon\n").enforces_child_restrictions);
    }

    #[test]
    fn the_last_line_for_a_key_wins() {
        assert!(!Configuration::from_settings_text("downtime.on=1\ndowntime.on=0\n").enforces_child_restrictions);
        assert!(Configuration::from_settings_text("downtime.on=0\ndowntime.on=1\n").enforces_child_restrictions);
        assert!(!Configuration::from_settings_text("limit.notes=30\nlimit.notes=0\n").enforces_child_restrictions);
    }

    #[test]
    fn comments_malformed_lines_and_spaces_are_handled() {
        assert!(!Configuration::from_settings_text("# downtime.on=1\nno separator here\n").enforces_child_restrictions);
        assert!(Configuration::from_settings_text("  downtime.on = 1  \n").enforces_child_restrictions);
        assert!(Configuration::from_settings_text("limit.files =  15\n").enforces_child_restrictions);
    }

    #[test]
    fn observer_reads_on_start_and_reports_only_real_changes() {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-screen-time-observer")).join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("screentime.ini");

        let mut obs = ConfigurationObserver::new(&path);
        assert_eq!(obs.configuration(), Configuration::default());
        assert!(obs.poll().is_none(), "no polls before observing starts");
        assert!(!obs.start_observing().enforces_child_restrictions, "a missing file enforces nothing");
        assert!(obs.is_observing());
        assert!(obs.poll().is_none(), "nothing changed yet");

        std::fs::write(&path, "downtime.on=1\n").unwrap();
        let changed = obs.poll().expect("the file now enforces restrictions");
        assert!(changed.enforces_child_restrictions);
        assert_eq!(obs.configuration(), changed);

        obs.stop_observing();
        assert!(!obs.is_observing());
        std::fs::write(&path, "downtime.on=0\n").unwrap();
        assert!(obs.poll().is_none(), "a stopped observer reports nothing");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
