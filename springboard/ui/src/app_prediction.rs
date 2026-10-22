//! App prediction: which apps the user is likely to open next. The shell records each launch (an app key and a Unix
//! time) in a bounded [`LaunchLedger`]; [`LaunchLedger::predict`] ranks the apps for a moment from three signals that
//! all stay on the device:
//!
//! - frequency with recency: every launch counts, and a launch loses half its weight each [`HALF_LIFE_DAYS`] days;
//! - time of day and day type: a launch around the same hour, and on the same kind of day (weekday or weekend),
//!   counts more than one at another time (a mail app opened every morning is predicted in the morning);
//! - sequence: an app that usually follows the one just used within [`FOLLOW_WITHIN`] seconds gets a bonus.
//!
//! This is the pure model. The shell appends launches and keeps the ledger's text form beside the search preferences
//! ([`LaunchLedger::to_text`], [`LaunchLedger::from_text`]); nothing leaves the machine.

/// The most launches the ledger keeps; the oldest are dropped first.
pub const MAX_EVENTS: usize = 400;
/// A launch loses half its weight after this many days; one older than [`FORGET_DAYS`] is ignored.
pub const HALF_LIFE_DAYS: f64 = 7.0;
/// Launches older than this many days no longer count.
pub const FORGET_DAYS: f64 = 90.0;
/// An app opened within this many seconds of the previous one counts as following it.
pub const FOLLOW_WITHIN: u64 = 600;
/// The longest app key, in bytes.
pub const KEY_MAX: usize = 96;
/// The weight factor of a launch on the same kind of day (weekday or weekend) as the moment predicted.
const SAME_DAY_TYPE: f64 = 1.5;

const DAY: u64 = 86_400;

/// One recorded launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub key: String,
    pub at: u64,
}

/// The recorded launches, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchLedger {
    events: Vec<Launch>,
}

/// The hour of the day (0..24) of Unix time `at` in a zone `tz_offset` seconds east of UTC.
pub fn hour_of_day(at: u64, tz_offset: i64) -> u32 {
    let local = (at as i64 + tz_offset).rem_euclid(DAY as i64) as u64;
    (local / 3600) as u32
}

/// True when Unix time `at` falls on a Saturday or Sunday in a zone `tz_offset` seconds east of UTC.
pub fn is_weekend(at: u64, tz_offset: i64) -> bool {
    let days = (at as i64 + tz_offset).div_euclid(DAY as i64);
    // 1970-01-01 was a Thursday: (days + 4) % 7 is 0 for Sunday
    matches!((days + 4).rem_euclid(7), 0 | 6)
}

/// How near two hours of the day are on the 24 hour circle: 1 for the same hour, falling to 0 at six hours apart.
fn closeness(a: u32, b: u32) -> f64 {
    let d = a.abs_diff(b);
    let d = d.min(24 - d) as f64;
    (1.0 - d / 6.0).max(0.0)
}

impl LaunchLedger {
    /// A ledger with no launches.
    pub fn new() -> Self {
        Self::default()
    }

    /// The recorded launches, oldest first.
    pub fn events(&self) -> &[Launch] {
        &self.events
    }

    /// True when no launch is recorded.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Records a launch of `key` at Unix time `at`. A key that is empty, longer than [`KEY_MAX`] or holds a tab or a
    /// line break is refused (false). A launch earlier than the newest one is placed in order, and once the ledger
    /// holds [`MAX_EVENTS`] launches the oldest is dropped.
    pub fn record(&mut self, key: &str, at: u64) -> bool {
        if key.is_empty() || key.len() > KEY_MAX || key.contains(['\t', '\n', '\r']) {
            return false;
        }
        let place = self.events.partition_point(|e| e.at <= at);
        self.events.insert(place, Launch { key: key.to_string(), at });
        if self.events.len() > MAX_EVENTS {
            self.events.remove(0);
        }
        true
    }

    /// The launches of `key`.
    pub fn count(&self, key: &str) -> usize {
        self.events.iter().filter(|e| e.key == key).count()
    }

    /// Forgets every launch of `key` (an app that was uninstalled). Returns how many were dropped.
    pub fn forget(&mut self, key: &str) -> usize {
        let before = self.events.len();
        self.events.retain(|e| e.key != key);
        before - self.events.len()
    }

    /// Forgets every launch of an app that `keep` does not accept.
    pub fn retain_apps(&mut self, keep: &dyn Fn(&str) -> bool) {
        self.events.retain(|e| keep(&e.key));
    }

    /// The apps most likely to be opened at Unix time `now`, best first, at most `limit`. `tz_offset` is the zone's
    /// offset east of UTC in seconds; `previous` is the app just used (for the sequence signal); apps in `exclude`
    /// (for example the ones already open) are left out. An app never launched in the last [`FORGET_DAYS`] days is not
    /// predicted. Equal scores go to the more recently launched app, then to the lower key.
    pub fn predict(&self, now: u64, tz_offset: i64, previous: Option<&str>, exclude: &[&str], limit: usize) -> Vec<String> {
        let (now_hour, now_weekend) = (hour_of_day(now, tz_offset), is_weekend(now, tz_offset));
        // key -> (score, newest launch)
        let mut scores: std::collections::BTreeMap<&str, (f64, u64)> = std::collections::BTreeMap::new();
        for (i, e) in self.events.iter().enumerate() {
            if e.at > now || exclude.contains(&e.key.as_str()) {
                continue;
            }
            let age_days = (now - e.at) as f64 / DAY as f64;
            if age_days > FORGET_DAYS {
                continue;
            }
            let decay = 0.5f64.powf(age_days / HALF_LIFE_DAYS);
            let mut time = 1.0 + 1.5 * closeness(hour_of_day(e.at, tz_offset), now_hour);
            if is_weekend(e.at, tz_offset) == now_weekend {
                time *= SAME_DAY_TYPE;
            }
            let mut score = decay * time;
            if let (Some(prev), Some(before)) = (previous, i.checked_sub(1).map(|j| &self.events[j])) {
                if before.key == prev && e.key != prev && e.at.saturating_sub(before.at) <= FOLLOW_WITHIN {
                    score += decay;
                }
            }
            let slot = scores.entry(e.key.as_str()).or_insert((0.0, 0));
            slot.0 += score;
            slot.1 = slot.1.max(e.at);
        }
        let mut ranked: Vec<(&str, (f64, u64))> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1 .0.partial_cmp(&a.1 .0).unwrap_or(std::cmp::Ordering::Equal).then(b.1 .1.cmp(&a.1 .1)).then(a.0.cmp(b.0)));
        ranked.into_iter().take(limit).map(|(k, _)| k.to_string()).collect()
    }

    /// The ledger as text: one `unix-seconds<TAB>key` line per launch, oldest first.
    pub fn to_text(&self) -> String {
        self.events.iter().map(|e| format!("{}\t{}\n", e.at, e.key)).collect()
    }

    /// Reads [`LaunchLedger::to_text`] output. A malformed line is skipped, and only the newest [`MAX_EVENTS`] launches
    /// are kept.
    pub fn from_text(text: &str) -> Self {
        let mut ledger = Self::new();
        for line in text.lines() {
            let Some((at, key)) = line.split_once('\t') else { continue };
            let Ok(at) = at.trim().parse::<u64>() else { continue };
            ledger.record(key, at);
        }
        ledger
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2000-03-01 (a Wednesday) 00:00 UTC.
    const WED: u64 = 11_017 * DAY;
    const HOUR: u64 = 3600;

    fn ledger(events: &[(&str, u64)]) -> LaunchLedger {
        let mut l = LaunchLedger::new();
        for (k, at) in events {
            assert!(l.record(k, *at));
        }
        l
    }

    #[test]
    fn hours_and_weekends_follow_the_zone() {
        assert_eq!(hour_of_day(WED + 9 * HOUR, 0), 9);
        assert_eq!(hour_of_day(WED + 23 * HOUR, 2 * 3600), 1, "past midnight in a zone east of UTC");
        assert_eq!(hour_of_day(WED + HOUR, -2 * 3600), 23, "before midnight in a zone west of UTC");
        assert!(!is_weekend(WED, 0));
        assert!(is_weekend(WED + 3 * DAY, 0), "Saturday");
        assert!(is_weekend(WED + 4 * DAY, 0), "Sunday");
        assert!(!is_weekend(WED + 5 * DAY, 0), "Monday");
    }

    #[test]
    fn a_launch_is_refused_for_a_bad_key() {
        let mut l = LaunchLedger::new();
        assert!(!l.record("", 1));
        assert!(!l.record("a\tb", 1));
        assert!(!l.record("a\nb", 1));
        assert!(!l.record(&"k".repeat(KEY_MAX + 1), 1));
        assert!(l.is_empty());
    }

    #[test]
    fn the_ledger_is_kept_in_time_order_and_bounded() {
        let mut l = ledger(&[("b", 20), ("a", 10), ("c", 30)]);
        let keys: Vec<&str> = l.events().iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["a", "b", "c"]);
        for n in 0..(MAX_EVENTS as u64 + 5) {
            l.record("x", 100 + n);
        }
        assert_eq!(l.events().len(), MAX_EVENTS);
        assert_eq!(l.count("a"), 0, "the oldest launches are dropped first");
    }

    #[test]
    fn the_most_launched_recent_app_comes_first() {
        let now = WED + 12 * HOUR;
        let l = ledger(&[("mail", now - 3 * HOUR), ("mail", now - 2 * HOUR), ("notes", now - HOUR)]);
        assert_eq!(l.predict(now, 0, None, &[], 5), ["mail", "notes"]);
        assert_eq!(l.predict(now, 0, None, &[], 1), ["mail"]);
        assert!(l.predict(now, 0, None, &[], 0).is_empty());
    }

    #[test]
    fn old_launches_fade_and_very_old_ones_are_forgotten() {
        let now = WED + 12 * HOUR;
        let l = ledger(&[("old", now - 60 * DAY), ("old", now - 60 * DAY + 1), ("old", now - 60 * DAY + 2), ("new", now - DAY)]);
        assert_eq!(l.predict(now, 0, None, &[], 2), ["new", "old"], "three old launches lose to one new one");
        let gone = ledger(&[("ancient", now - 100 * DAY)]);
        assert!(gone.predict(now, 0, None, &[], 5).is_empty());
    }

    #[test]
    fn the_hour_of_day_shapes_the_prediction() {
        // yesterday, mail at 08:00 and a game at 22:00
        let yesterday = WED - DAY;
        let l = ledger(&[("mail", yesterday + 8 * HOUR), ("game", yesterday + 22 * HOUR)]);
        assert_eq!(l.predict(WED + 8 * HOUR + 1800, 0, None, &[], 2)[0], "mail");
        assert_eq!(l.predict(WED + 22 * HOUR + 1800, 0, None, &[], 2)[0], "game");
    }

    #[test]
    fn a_weekday_habit_beats_a_slightly_newer_weekend_launch_on_a_weekday() {
        // Monday 10:00; work was opened on Friday at 10:00 (3 days ago) and the camera on Sunday at 10:00 (1 day ago)
        let l = ledger(&[("work", WED + 2 * DAY + 10 * HOUR), ("camera", WED + 4 * DAY + 10 * HOUR)]);
        assert_eq!(l.predict(WED + 5 * DAY + 10 * HOUR, 0, None, &[], 2)[0], "work");
        // on the Sunday evening the weekend launch of the day before wins for the camera instead
        assert_eq!(l.predict(WED + 5 * DAY - 2 * HOUR, 0, None, &[], 2)[0], "camera");
    }

    #[test]
    fn the_app_that_usually_follows_the_last_one_gets_a_bonus() {
        let now = WED + 12 * HOUR;
        // "chat" is launched a little more often, but "maps" always follows "calendar"
        let l = ledger(&[
            ("calendar", now - 3 * HOUR),
            ("maps", now - 3 * HOUR + 60),
            ("chat", now - 2 * HOUR),
            ("chat", now - HOUR),
            ("calendar", now - 1800),
            ("maps", now - 1700),
        ]);
        assert_eq!(l.predict(now, 0, None, &[], 1), ["chat"]);
        assert_eq!(l.predict(now, 0, Some("calendar"), &["calendar"], 1), ["maps"]);
    }

    #[test]
    fn excluded_apps_and_future_launches_are_left_out() {
        let now = WED + 12 * HOUR;
        let l = ledger(&[("open", now - HOUR), ("later", now + HOUR), ("other", now - 2 * HOUR)]);
        assert_eq!(l.predict(now, 0, None, &["open"], 5), ["other"]);
    }

    #[test]
    fn equal_scores_go_to_the_newer_launch_then_the_key() {
        let l = ledger(&[("b", 1000), ("a", 1000)]);
        assert_eq!(l.predict(2000, 0, None, &[], 2), ["a", "b"]);
    }

    #[test]
    fn forgetting_an_app_drops_its_launches() {
        let mut l = ledger(&[("a", 1), ("b", 2), ("a", 3)]);
        assert_eq!(l.forget("a"), 2);
        assert_eq!(l.count("a"), 0);
        l.retain_apps(&|k| k != "b");
        assert!(l.is_empty());
    }

    #[test]
    fn the_text_form_round_trips_and_skips_bad_lines() {
        let l = ledger(&[("mail", 100), ("notes", 200)]);
        assert_eq!(LaunchLedger::from_text(&l.to_text()), l);
        let back = LaunchLedger::from_text("100\tmail\nnot a line\nx\tbad\n\n200\t\n300\tnotes\n");
        let keys: Vec<&str> = back.events().iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["mail", "notes"]);
    }
}
