//! Screen Time settings (`screentime.ini`, a `Prefs` file): per-app daily
//! limits, the Downtime schedule with its always-allowed apps, and the
//! temporary "ignore limit" grants. `Config::block` is the single policy
//! decision; Settings itself is never blocked.

use super::super::notify::schedule::Schedule;
use super::super::prefs::Prefs;
use super::super::AppId;
use super::usage::{day_of, key, minute_of};
use std::path::Path;

/// Daily limit choices in minutes, cycled by tapping an app in App Limits (0 = none).
pub const LIMIT_STEPS: [u32; 7] = [0, 15, 30, 60, 120, 180, 240];
/// How long "Ignore Limit for 15 min" lasts.
pub const IGNORE_SECS: i64 = 15 * 60;

/// Why an app is covered by the Time Limit screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Limit,
    Downtime,
}

/// How long an ignore grant lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ignore {
    Fifteen,
    Today,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Downtime {
    pub on: bool,
    /// minutes after midnight
    pub start: u16,
    pub end: u16,
}

impl Downtime {
    /// The same daily window as a notification `Schedule` (the one place that knows it may cross midnight).
    pub fn schedule(&self) -> Schedule {
        Schedule { enabled: self.on, start: self.start, end: self.end }
    }
}

/// The Screen Time preferences as stored on disk.
#[derive(Clone, Debug, Default)]
pub struct Config {
    prefs: Prefs,
}

fn num(prefs: &Prefs, k: &str) -> Option<i64> {
    prefs.get_parsed(k)
}

impl Config {
    pub fn load(path: &Path) -> Self {
        Config { prefs: Prefs::load(path) }
    }

    pub fn memory() -> Self {
        Config { prefs: Prefs::memory() }
    }

    /// The app's daily limit in minutes, if one is set.
    pub fn limit_mins(&self, app: AppId) -> Option<u32> {
        num(&self.prefs, &format!("limit.{}", key(app))).filter(|&m| m > 0).map(|m| m as u32)
    }

    pub fn set_limit(&mut self, app: AppId, mins: Option<u32>) {
        let _ = self.prefs.set(&format!("limit.{}", key(app)), &mins.unwrap_or(0).to_string());
    }

    /// The next limit step after the app's current one, wrapping to none.
    pub fn cycle_limit(&mut self, app: AppId) -> Option<u32> {
        let cur = self.limit_mins(app).unwrap_or(0);
        let at = LIMIT_STEPS.iter().position(|&m| m == cur).unwrap_or(0);
        let next = LIMIT_STEPS[(at + 1) % LIMIT_STEPS.len()];
        let next = Some(next).filter(|&m| m > 0);
        self.set_limit(app, next);
        next
    }

    pub fn downtime(&self) -> Downtime {
        let m = |k: &str, d: u16| num(&self.prefs, k).map_or(d, |v| (v.rem_euclid(1440)) as u16);
        Downtime { on: num(&self.prefs, "downtime.on") == Some(1), start: m("downtime.start", 22 * 60), end: m("downtime.end", 7 * 60) }
    }

    pub fn set_downtime(&mut self, d: Downtime) {
        let _ = self.prefs.set_many(&[
            ("downtime.on", if d.on { "1" } else { "0" }),
            ("downtime.start", &(d.start % 1440).to_string()),
            ("downtime.end", &(d.end % 1440).to_string()),
        ]);
    }

    /// Whether Downtime leaves `app` usable. Settings always is; Clock is by default.
    pub fn allowed(&self, app: AppId) -> bool {
        app == AppId::Settings || num(&self.prefs, &format!("allow.{}", key(app))).map_or(key(app) == "clock", |v| v == 1)
    }

    pub fn set_allowed(&mut self, app: AppId, on: bool) {
        let _ = self.prefs.set(&format!("allow.{}", key(app)), if on { "1" } else { "0" });
    }

    /// Grants a temporary pass for `app` from `unix` (local `offset`).
    pub fn grant(&mut self, app: AppId, how: Ignore, unix: i64, offset: i32) {
        let k = key(app);
        match how {
            Ignore::Fifteen => {
                let _ = self.prefs.set(&format!("ignore.until.{k}"), &(unix + IGNORE_SECS).to_string());
            }
            Ignore::Today => {
                let _ = self.prefs.set(&format!("ignore.day.{k}"), &day_of(unix, offset).to_string());
            }
        }
    }

    pub fn ignored(&self, app: AppId, unix: i64, offset: i32) -> bool {
        let k = key(app);
        num(&self.prefs, &format!("ignore.until.{k}")).is_some_and(|u| u > unix) || num(&self.prefs, &format!("ignore.day.{k}")) == Some(day_of(unix, offset))
    }

    /// The reason `app` must be covered now, given its seconds used today.
    /// Settings is exempt so the limits can always be changed or removed,
    /// and an unset clock (`unix` 0) never blocks anything.
    pub fn block(&self, app: AppId, used_secs: u32, unix: i64, offset: i32) -> Option<Reason> {
        if app == AppId::Settings || unix <= 0 || self.ignored(app, unix, offset) {
            return None;
        }
        if self.downtime().schedule().active(minute_of(unix, offset)) && !self.allowed(app) {
            return Some(Reason::Downtime);
        }
        match self.limit_mins(app) {
            Some(m) if used_secs >= m * 60 => Some(Reason::Limit),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOON: i64 = 1_700_000_000 - 1_700_000_000 % 86_400 + 12 * 3600;

    #[test]
    fn downtime_window_may_cross_midnight() {
        let d = Downtime { on: true, start: 22 * 60, end: 7 * 60 };
        assert!(d.schedule().active(23 * 60) && d.schedule().active(0) && d.schedule().active(6 * 60 + 59));
        assert!(!d.schedule().active(7 * 60) && !d.schedule().active(12 * 60));
        let day = Downtime { on: true, start: 9 * 60, end: 17 * 60 };
        assert!(day.schedule().active(9 * 60) && !day.schedule().active(17 * 60));
        assert!(!Downtime { on: false, ..d }.schedule().active(23 * 60));
        assert!(!Downtime { on: true, start: 5, end: 5 }.schedule().active(5));
    }

    #[test]
    fn limits_cycle_through_the_steps_and_back_to_none() {
        let mut c = Config::memory();
        assert_eq!(c.limit_mins(AppId::Files), None);
        assert_eq!(c.cycle_limit(AppId::Files), Some(15));
        assert_eq!(c.cycle_limit(AppId::Files), Some(30));
        for _ in 0..4 {
            c.cycle_limit(AppId::Files);
        }
        assert_eq!(c.limit_mins(AppId::Files), Some(240));
        assert_eq!(c.cycle_limit(AppId::Files), None);
    }

    #[test]
    fn a_reached_limit_blocks_until_ignored() {
        let mut c = Config::memory();
        c.set_limit(AppId::Files, Some(30));
        assert_eq!(c.block(AppId::Files, 1799, NOON, 0), None);
        assert_eq!(c.block(AppId::Files, 1800, NOON, 0), Some(Reason::Limit));
        assert_eq!(c.block(AppId::Photos, 99_999, NOON, 0), None, "no limit, no block");
        c.grant(AppId::Files, Ignore::Fifteen, NOON, 0);
        assert_eq!(c.block(AppId::Files, 1800, NOON + 899, 0), None);
        assert_eq!(c.block(AppId::Files, 1800, NOON + 901, 0), Some(Reason::Limit), "the pass expired");
        c.grant(AppId::Files, Ignore::Today, NOON, 0);
        assert_eq!(c.block(AppId::Files, 1800, NOON + 3600 * 5, 0), None);
        assert_eq!(c.block(AppId::Files, 1800, NOON + 86_400, 0), Some(Reason::Limit), "tomorrow it applies again");
    }

    #[test]
    fn downtime_blocks_apps_that_are_not_allowed() {
        let mut c = Config::memory();
        c.set_downtime(Downtime { on: true, start: 11 * 60, end: 13 * 60 });
        assert_eq!(c.block(AppId::Files, 0, NOON, 0), Some(Reason::Downtime));
        c.set_allowed(AppId::Files, true);
        assert_eq!(c.block(AppId::Files, 0, NOON, 0), None);
        c.set_allowed(AppId::Files, false);
        assert_eq!(c.block(AppId::Files, 0, NOON, 0), Some(Reason::Downtime));
        assert_eq!(c.block(AppId::Files, 0, NOON + 2 * 3600, 0), None, "outside the window");
    }

    #[test]
    fn settings_and_an_unset_clock_are_never_blocked() {
        let mut c = Config::memory();
        c.set_limit(AppId::Settings, Some(15));
        c.set_downtime(Downtime { on: true, start: 0, end: 1439 });
        c.set_allowed(AppId::Settings, false);
        assert_eq!(c.block(AppId::Settings, 99_999, NOON, 0), None);
        assert!(c.allowed(AppId::Settings));
        c.set_limit(AppId::Files, Some(15));
        assert_eq!(c.block(AppId::Files, 99_999, 0, 0), None);
    }

    #[test]
    fn settings_persist_to_the_file() {
        let d = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-screentime-cfg")).join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&d);
        let path = d.join("screentime.ini");
        let mut c = Config::load(&path);
        c.set_limit(AppId::Photos, Some(60));
        c.set_downtime(Downtime { on: true, start: 60, end: 120 });
        let back = Config::load(&path);
        assert_eq!(back.limit_mins(AppId::Photos), Some(60));
        assert_eq!(back.downtime(), Downtime { on: true, start: 60, end: 120 });
        let _ = std::fs::remove_dir_all(&d);
    }
}
