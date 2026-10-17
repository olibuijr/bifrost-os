//! Screen Time settings (`screentime.ini`, a `Prefs` file): per-app daily
//! limits, the Downtime schedule with its always-allowed apps, and the
//! temporary "ignore limit" grants. `Config::block` is the single policy
//! decision; Settings itself is never blocked.

use super::super::prefs::Prefs;
use super::super::AppId;
use super::usage::{day_of, key, minute_of};
use std::path::Path;

/// Daily limit choices in minutes, cycled by tapping an app in App Limits (0 = none).
pub const LIMIT_STEPS: [u32; 7] = [0, 15, 30, 60, 120, 180, 240];
/// How long "Ignore Limit for 15 min" lasts.
pub const IGNORE_SECS: i64 = 15 * 60;
/// How long "Ignore Limit for 1 more minute" lasts.
pub const ONE_MORE_MINUTE_SECS: i64 = 60;
/// How long before a daily limit runs out the warning notification is posted.
pub const WARN_LEAD_SECS: u32 = 5 * 60;

/// Why an app is covered by the Time Limit screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Limit,
    Downtime,
}

/// How long an ignore grant lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ignore {
    OneMinute,
    Fifteen,
    Today,
}

/// The Downtime window. One copy lives in the UI model (`os_services::time_limits`), which the shell uses as-is.
pub use bifrost_ui::os_services::time_limits::Downtime;

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
            Ignore::OneMinute => {
                let _ = self.prefs.set(&format!("ignore.until.{k}"), &(unix + ONE_MORE_MINUTE_SECS).to_string());
            }
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

    /// True when `app` has a daily limit that runs out within `WARN_LEAD_SECS`
    /// (and has not run out yet), so the shell can warn once. An ignored or
    /// unlimited app never warns.
    pub fn warn_due(&self, app: AppId, used_secs: u32, unix: i64, offset: i32) -> bool {
        if app == AppId::Settings || unix <= 0 || self.ignored(app, unix, offset) {
            return false;
        }
        match self.limit_mins(app) {
            Some(m) => {
                let limit = m * 60;
                used_secs < limit && used_secs + WARN_LEAD_SECS >= limit
            }
            None => false,
        }
    }

    /// The reason `app` must be covered now, given its seconds used today.
    /// Settings is exempt so the limits can always be changed or removed,
    /// and an unset clock (`unix` 0) never blocks anything.
    pub fn block(&self, app: AppId, used_secs: u32, unix: i64, offset: i32) -> Option<Reason> {
        if app == AppId::Settings || unix <= 0 || self.ignored(app, unix, offset) {
            return None;
        }
        if self.downtime().active(minute_of(unix, offset)) && !self.allowed(app) {
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
    const MIDNIGHT: i64 = NOON - 12 * 3600;

    /// The shield's decision and the UI model's decision (`os_services::time_limits::Policy`) must be one rule.
    /// Each case builds both from the same settings and compares the reason at several times and usages.
    #[test]
    fn the_shield_rule_agrees_with_the_model_rule() {
        use bifrost_ui::os_services::time_limits::{Downtime as ModelDowntime, Ignore as ModelIgnore, Policy, Reason as ModelReason};

        fn model_reason(r: Option<Reason>) -> Option<ModelReason> {
            r.map(|r| match r {
                Reason::Limit => ModelReason::Limit,
                Reason::Downtime => ModelReason::Downtime,
            })
        }

        let mut cfg = Config::memory();
        let mut model = Policy::default();
        cfg.set_limit(AppId::Terminal, Some(30));
        model.set_limit("terminal", 30).expect("30 minutes is a step");
        cfg.set_limit(AppId::Photos, Some(60));
        model.set_limit("photos", 60).expect("60 minutes is a step");
        cfg.set_downtime(Downtime { on: true, start: 22 * 60, end: 7 * 60 });
        model.set_downtime(ModelDowntime::new(true, 22 * 60, 7 * 60).expect("minutes are in the day"));
        cfg.set_allowed(AppId::Photos, true);
        model.set_allowed("photos", true);

        let apps = [(AppId::Terminal, "terminal"), (AppId::Photos, "photos"), (AppId::Files, "files"), (AppId::Settings, "settings")];
        let times = [MIDNIGHT + 3 * 3600, NOON, MIDNIGHT + 23 * 3600];
        let usages = [0u32, 1_800, 3_600, 7_200];
        for (app, name) in apps {
            for &unix in &times {
                for &used in &usages {
                    assert_eq!(model_reason(cfg.block(app, used, unix, 0)), model.block(name, used, unix, 0), "{name} at {unix} with {used}s");
                }
            }
        }

        cfg.grant(AppId::Terminal, Ignore::Fifteen, NOON, 0);
        model.grant_ignore("terminal", ModelIgnore::Fifteen, NOON, 0);
        for &unix in &[NOON, NOON + 60, NOON + 16 * 60] {
            assert_eq!(model_reason(cfg.block(AppId::Terminal, 7_200, unix, 0)), model.block("terminal", 7_200, unix, 0), "grant at {unix}");
        }
    }

    #[test]
    fn downtime_window_may_cross_midnight() {
        let d = Downtime::new(true, 22 * 60, 7 * 60).unwrap();
        assert!(d.active(23 * 60) && d.active(0) && d.active(6 * 60 + 59));
        assert!(!d.active(7 * 60) && !d.active(12 * 60));
        let day = Downtime::new(true, 9 * 60, 17 * 60).unwrap();
        assert!(day.active(9 * 60) && !day.active(17 * 60));
        assert!(!Downtime { on: false, ..d }.active(23 * 60));
        assert!(!Downtime::new(true, 5, 5).unwrap().active(5));
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
    fn one_more_minute_and_fifteen_minute_grants_expire_separately() {
        let mut c = Config::memory();
        c.set_limit(AppId::Files, Some(30));
        c.grant(AppId::Files, Ignore::OneMinute, NOON, 0);
        assert_eq!(c.block(AppId::Files, 1800, NOON + 59, 0), None);
        assert_eq!(c.block(AppId::Files, 1800, NOON + 61, 0), Some(Reason::Limit), "one minute is all it grants");
        c.grant(AppId::Files, Ignore::Fifteen, NOON, 0);
        assert_eq!(c.block(AppId::Files, 1800, NOON + 14 * 60, 0), None);
    }

    #[test]
    fn a_limit_warns_five_minutes_before_it_runs_out_once_it_is_close() {
        let mut c = Config::memory();
        c.set_limit(AppId::Files, Some(30));
        assert!(!c.warn_due(AppId::Files, 1499, NOON, 0), "more than five minutes left");
        assert!(c.warn_due(AppId::Files, 1500, NOON, 0), "exactly five minutes left");
        assert!(c.warn_due(AppId::Files, 1799, NOON, 0));
        assert!(!c.warn_due(AppId::Files, 1800, NOON, 0), "the limit itself is a block, not a warning");
        assert!(!c.warn_due(AppId::Photos, 1700, NOON, 0), "no limit, no warning");
        assert!(!c.warn_due(AppId::Settings, 1700, NOON, 0));
        c.grant(AppId::Files, Ignore::OneMinute, NOON, 0);
        assert!(!c.warn_due(AppId::Files, 1700, NOON, 0), "an ignored limit does not warn");
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
