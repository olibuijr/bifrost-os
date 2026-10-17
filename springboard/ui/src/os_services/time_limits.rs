//! Daily app limits and the Downtime window, as a policy an app can ask about.
//!
//! The system's usage tracker supplies the seconds each app has been in use today, and the caller supplies the clock.
//! [`Policy::block`] answers whether an app is covered now and why. The rule is the one the shell applies: Settings is
//! always exempt so limits can be changed, an unset clock never blocks, a temporary ignore grant wins over both
//! reasons, and during Downtime only the always-allowed apps run.

use std::collections::{BTreeMap, BTreeSet};

/// Daily limit choices in minutes. 0 means no limit.
pub const LIMIT_STEPS: [u32; 7] = [0, 15, 30, 60, 120, 180, 240];
/// How long a fifteen-minute ignore grant lasts, in seconds.
pub const IGNORE_SECS: i64 = 15 * 60;
/// The app that is never covered by a limit or by Downtime.
pub const EXEMPT_APP: &str = "settings";
/// The app that Downtime allows before any choice is stored.
pub const DEFAULT_ALLOWED_APP: &str = "clock";

const MINUTES_PER_DAY: u16 = 24 * 60;
const SECS_PER_DAY: i64 = 86_400;

/// Why an app is covered now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The app's daily limit is used up.
    Limit,
    /// The Downtime window is open and the app is not always allowed.
    Downtime,
}

/// How long an ignore grant lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ignore {
    /// Fifteen minutes from the grant.
    Fifteen,
    /// Until local midnight of the grant's day.
    Today,
}

/// A limit that is not one of [`LIMIT_STEPS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnsupportedLimit(pub u32);

/// The daily window in which only the always-allowed apps run. It may cross midnight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Downtime {
    pub on: bool,
    /// Local minute of the day the window opens, 0 to 1439.
    pub start: u16,
    /// Local minute of the day it closes, 0 to 1439.
    pub end: u16,
}

impl Downtime {
    /// A window from validated minutes; None when either minute is outside the day.
    pub fn new(on: bool, start: u16, end: u16) -> Option<Self> {
        (start < MINUTES_PER_DAY && end < MINUTES_PER_DAY).then_some(Self { on, start, end })
    }

    /// Whether the window covers local `minute`. An empty window (start equal to end) covers nothing.
    pub fn active(&self, minute: u16) -> bool {
        if !self.on || self.start == self.end {
            return false;
        }
        let m = minute % MINUTES_PER_DAY;
        if self.start < self.end {
            m >= self.start && m < self.end
        } else {
            m >= self.start || m < self.end
        }
    }
}

impl Default for Downtime {
    fn default() -> Self {
        Self { on: false, start: 22 * 60, end: 7 * 60 }
    }
}

/// The limit that follows `current` minutes in [`LIMIT_STEPS`], wrapping from the last step to none.
pub fn next_limit_step(current: u32) -> u32 {
    let at = LIMIT_STEPS.iter().position(|&m| m == current).unwrap_or(0);
    LIMIT_STEPS[(at + 1) % LIMIT_STEPS.len()]
}

/// Limits, Downtime and grants for every app, keyed by the app's lowercase name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    downtime: Downtime,
    limits: BTreeMap<String, u32>,
    always_allowed: BTreeSet<String>,
    ignored_until: BTreeMap<String, i64>,
    ignored_day: BTreeMap<String, i64>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            downtime: Downtime::default(),
            limits: BTreeMap::new(),
            always_allowed: BTreeSet::from([DEFAULT_ALLOWED_APP.to_string()]),
            ignored_until: BTreeMap::new(),
            ignored_day: BTreeMap::new(),
        }
    }
}

impl Policy {
    /// The Downtime window.
    pub fn downtime(&self) -> Downtime {
        self.downtime
    }

    /// Replaces the Downtime window.
    pub fn set_downtime(&mut self, downtime: Downtime) {
        self.downtime = downtime;
    }

    /// The app's daily limit in minutes, if one is set.
    pub fn limit(&self, app: &str) -> Option<u32> {
        self.limits.get(app).copied()
    }

    /// Sets the app's daily limit. 0 removes it. Any other value must be one of [`LIMIT_STEPS`].
    pub fn set_limit(&mut self, app: &str, minutes: u32) -> Result<(), UnsupportedLimit> {
        if !LIMIT_STEPS.contains(&minutes) {
            return Err(UnsupportedLimit(minutes));
        }
        if minutes == 0 {
            self.limits.remove(app);
        } else {
            self.limits.insert(app.to_string(), minutes);
        }
        Ok(())
    }

    /// Whether Downtime leaves the app usable. Settings always does.
    pub fn allowed(&self, app: &str) -> bool {
        app == EXEMPT_APP || self.always_allowed.contains(app)
    }

    /// Adds or removes an app from the always-allowed list.
    pub fn set_allowed(&mut self, app: &str, on: bool) {
        if on {
            self.always_allowed.insert(app.to_string());
        } else {
            self.always_allowed.remove(app);
        }
    }

    /// Lets the app run past its limit for the grant's duration, counted from `unix` at `offset` seconds from UTC.
    pub fn grant_ignore(&mut self, app: &str, how: Ignore, unix: i64, offset: i32) {
        match how {
            Ignore::Fifteen => {
                self.ignored_until.insert(app.to_string(), unix + IGNORE_SECS);
            }
            Ignore::Today => {
                self.ignored_day.insert(app.to_string(), day_of(unix, offset));
            }
        }
    }

    /// Puts back a stored grant: the Unix time the fifteen-minute pass ends (None for none) and the local day
    /// number of a until-midnight pass (None for none). Used when the grants are read from the settings file.
    pub fn restore_ignore(&mut self, app: &str, until: Option<i64>, day: Option<i64>) {
        match until {
            Some(t) => self.ignored_until.insert(app.to_string(), t),
            None => self.ignored_until.remove(app),
        };
        match day {
            Some(d) => self.ignored_day.insert(app.to_string(), d),
            None => self.ignored_day.remove(app),
        };
    }

    /// Whether a grant covers the app at `unix`.
    pub fn ignored(&self, app: &str, unix: i64, offset: i32) -> bool {
        self.ignored_until.get(app).is_some_and(|&until| until > unix) || self.ignored_day.get(app) == Some(&day_of(unix, offset))
    }

    /// The reason the app must be covered now, given the seconds it has been used today. None means it may run.
    /// `unix` is the Unix time and `offset` the local offset from UTC in seconds.
    pub fn block(&self, app: &str, used_secs: u32, unix: i64, offset: i32) -> Option<Reason> {
        if app == EXEMPT_APP || unix <= 0 || self.ignored(app, unix, offset) {
            return None;
        }
        if self.downtime.active(minute_of(unix, offset)) && !self.allowed(app) {
            return Some(Reason::Downtime);
        }
        match self.limit(app) {
            Some(minutes) if used_secs >= minutes * 60 => Some(Reason::Limit),
            _ => None,
        }
    }
}

fn day_of(unix: i64, offset: i32) -> i64 {
    (unix + offset as i64).div_euclid(SECS_PER_DAY)
}

fn minute_of(unix: i64, offset: i32) -> u16 {
    ((unix + offset as i64).rem_euclid(SECS_PER_DAY) / 60) as u16
}
