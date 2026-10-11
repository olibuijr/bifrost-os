//! Screen Time records: one small `key=value` text file per local day
//! (`screentime/<day>.txt`) holding foreground seconds per app and per hour,
//! pickups, and notifications received per app. The `Tracker` accumulates
//! them from the shell's tick; Settings reads the files back for its charts.

use super::super::{AppId, apps};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Days of history kept on disk.
pub const KEEP_DAYS: i64 = 28;
/// The longest gap one tick may credit to an app; a stalled loop must not
/// invent usage.
const MAX_STEP_SECS: i64 = 15;
/// How often unsaved usage is written out.
const FLUSH_SECS: i64 = 15;

/// The stable text key of an app (its lowercase launcher name).
pub fn key(app: AppId) -> String {
    apps().iter().find(|a| a.id == app).map(|a| a.name.to_lowercase()).unwrap_or_default()
}

/// The app a key names.
pub fn app_of_key(key: &str) -> Option<AppId> {
    apps().iter().find(|a| a.name.to_lowercase() == key).map(|a| a.id)
}

/// Local day number (days since 1970-01-01 in the wall-clock zone).
pub fn day_of(unix: i64, offset: i32) -> i64 {
    (unix + offset as i64).div_euclid(86_400)
}

/// Local minute of the day.
pub fn minute_of(unix: i64, offset: i32) -> u16 {
    ((unix + offset as i64).rem_euclid(86_400) / 60) as u16
}

/// Day of the week for a day number, 0 = Sunday (1970-01-01 was a Thursday).
pub fn weekday(day: i64) -> usize {
    crate::bare::timezone::weekday(day) as usize
}

/// What one day recorded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Day {
    /// foreground seconds per app key
    pub secs: BTreeMap<String, u32>,
    /// foreground seconds per local hour
    pub hours: [u32; 24],
    pub pickups: u32,
    /// notifications received per app key
    pub notifs: BTreeMap<String, u32>,
}

impl Day {
    /// Parses a day file, skipping lines it does not know.
    pub fn parse(text: &str) -> Day {
        let mut d = Day::default();
        for l in text.lines() {
            let Some((k, v)) = l.split_once('=') else { continue };
            let Ok(n) = v.trim().parse::<u32>() else { continue };
            let k = k.trim();
            if let Some(a) = k.strip_prefix("app.") {
                d.secs.insert(a.into(), n);
            } else if let Some(a) = k.strip_prefix("notif.") {
                d.notifs.insert(a.into(), n);
            } else if let Some(h) = k.strip_prefix("hour.").and_then(|h| h.parse::<usize>().ok()).filter(|&h| h < 24) {
                d.hours[h] = n;
            } else if k == "pickups" {
                d.pickups = n;
            }
        }
        d
    }

    pub fn to_text(&self) -> String {
        let mut out = format!("pickups={}\n", self.pickups);
        for (a, n) in &self.secs {
            out += &format!("app.{a}={n}\n");
        }
        for (h, n) in self.hours.iter().enumerate().filter(|(_, &n)| n > 0) {
            out += &format!("hour.{h}={n}\n");
        }
        for (a, n) in &self.notifs {
            out += &format!("notif.{a}={n}\n");
        }
        out
    }

    /// Total foreground seconds.
    pub fn total(&self) -> u32 {
        self.secs.values().sum()
    }

    pub fn app_secs(&self, app: AppId) -> u32 {
        self.secs.get(&key(app)).copied().unwrap_or(0)
    }

    pub fn notif_total(&self) -> u32 {
        self.notifs.values().sum()
    }

    /// Folds `other` into this day (for week totals).
    pub fn add(&mut self, other: &Day) {
        for (a, n) in &other.secs {
            *self.secs.entry(a.clone()).or_default() += n;
        }
        for (a, n) in &other.notifs {
            *self.notifs.entry(a.clone()).or_default() += n;
        }
        for (h, n) in other.hours.iter().enumerate() {
            self.hours[h] += n;
        }
        self.pickups += other.pickups;
    }

    /// Apps with usage, most used first.
    pub fn ranked(&self) -> Vec<(AppId, u32)> {
        let mut v: Vec<(AppId, u32)> = self.secs.iter().filter(|(_, &n)| n > 0).filter_map(|(k, &n)| Some((app_of_key(k)?, n))).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| key(a.0).cmp(&key(b.0))));
        v
    }
}

fn day_path(dir: &Path, day: i64) -> PathBuf {
    dir.join(format!("{day}.txt"))
}

/// Loads one day (missing = empty).
pub fn load_day(dir: &Path, day: i64) -> Day {
    std::fs::read_to_string(day_path(dir, day)).map(|t| Day::parse(&t)).unwrap_or_default()
}

/// The `n` days ending at `today`, oldest first.
pub fn history(dir: &Path, today: i64, n: i64) -> Vec<(i64, Day)> {
    (today - n + 1..=today).map(|d| (d, load_day(dir, d))).collect()
}

fn save_day(dir: &Path, day: i64, d: &Day) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let path = day_path(dir, day);
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, d.to_text()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Deletes day files older than KEEP_DAYS before `today`.
fn prune(dir: &Path, today: i64) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let day = e.file_name().to_str().and_then(|n| n.strip_suffix(".txt")).and_then(|n| n.parse::<i64>().ok());
        if day.is_some_and(|d| d <= today - KEEP_DAYS) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Accumulates today's record from the shell's ticks.
#[derive(Debug)]
pub struct Tracker {
    dir: PathBuf,
    day: i64,
    today: Day,
    last_unix: i64,
    last_on: bool,
    dirty: bool,
    flushed_at: i64,
}

impl Tracker {
    /// A tracker writing into `dir`; nothing is read until the first tick.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Tracker { dir: dir.into(), day: i64::MIN, today: Day::default(), last_unix: 0, last_on: false, dirty: false, flushed_at: 0 }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn today(&self) -> &Day {
        &self.today
    }

    fn roll(&mut self, day: i64) {
        if self.day != i64::MIN {
            self.flush();
        }
        self.day = day;
        self.today = load_day(&self.dir, day);
        prune(&self.dir, day);
    }

    /// Credits the time since the last tick to `fg` (the app on screen, if
    /// any) while the screen is on, and counts a pickup when the screen
    /// comes on. Returns true when today's record changed.
    pub fn tick(&mut self, unix: i64, offset: i32, fg: Option<AppId>, screen_on: bool) -> bool {
        if unix <= 0 {
            return false;
        }
        let day = day_of(unix, offset);
        if day != self.day {
            self.roll(day);
            // the gap across midnight is not credited to either day
            self.last_unix = 0;
        }
        let mut changed = false;
        if self.last_unix > 0 {
            let dt = (unix - self.last_unix).clamp(0, MAX_STEP_SECS) as u32;
            // the screen must have been on for the whole interval (a wake-up tick credits nothing)
            if let (Some(app), true, true, true) = (fg, screen_on, self.last_on, dt > 0) {
                *self.today.secs.entry(key(app)).or_default() += dt;
                self.today.hours[(minute_of(unix, offset) / 60) as usize] += dt;
                changed = true;
            }
            if screen_on && !self.last_on {
                self.today.pickups += 1;
                changed = true;
            }
        }
        self.last_unix = unix;
        self.last_on = screen_on;
        self.dirty |= changed;
        if self.dirty && unix - self.flushed_at >= FLUSH_SECS {
            self.flush();
            self.flushed_at = unix;
        }
        changed
    }

    /// Counts a notification received for `app`.
    pub fn note_notification(&mut self, app: AppId) {
        *self.today.notifs.entry(key(app)).or_default() += 1;
        self.dirty = true;
    }

    /// Writes today's record if it changed.
    pub fn flush(&mut self) {
        if self.dirty && self.day != i64::MIN {
            save_day(&self.dir, self.day, &self.today);
            self.dirty = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-screentime")).join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn day_text_round_trips() {
        let mut d = Day::default();
        d.secs.insert("files".into(), 90);
        d.notifs.insert("photos".into(), 2);
        d.hours[13] = 90;
        d.pickups = 4;
        assert_eq!(Day::parse(&d.to_text()), d);
        assert_eq!(Day::parse("junk\napp.x=abc\nhour.99=5\npickups=2"), Day { pickups: 2, ..Default::default() });
    }

    #[test]
    fn keys_name_apps_both_ways() {
        assert_eq!(key(AppId::Files), "files");
        assert_eq!(app_of_key("files"), Some(AppId::Files));
        assert_eq!(app_of_key("nope"), None);
    }

    #[test]
    fn days_and_weekdays() {
        assert_eq!(day_of(0, 0), 0);
        assert_eq!(day_of(86_399, 1), 1);
        assert_eq!(day_of(0, -1), -1);
        assert_eq!(minute_of(3_600 * 5 + 120, 0), 302);
        assert_eq!(weekday(0), 4, "1970-01-01 was a Thursday");
        assert_eq!(weekday(3), 0);
    }

    #[test]
    fn ranked_sorts_by_time_and_skips_empty() {
        let mut d = Day::default();
        d.secs.insert("files".into(), 10);
        d.secs.insert("photos".into(), 50);
        d.secs.insert("camera".into(), 0);
        assert_eq!(d.ranked(), vec![(AppId::Photos, 50), (AppId::Files, 10)]);
    }

    #[test]
    fn tracker_credits_the_foreground_app_and_counts_pickups() {
        let d = dir("track");
        let mut t = Tracker::new(&d);
        let base = 1_700_000_000;
        assert!(!t.tick(base, 0, Some(AppId::Files), false));
        assert!(t.tick(base + 1, 0, Some(AppId::Files), true), "screen came on: a pickup");
        assert_eq!(t.today().pickups, 1);
        for s in 2..=11 {
            t.tick(base + s, 0, Some(AppId::Files), true);
        }
        assert_eq!(t.today().app_secs(AppId::Files), 10);
        t.tick(base + 12, 0, None, true);
        t.tick(base + 13, 0, Some(AppId::Files), false);
        assert_eq!(t.today().app_secs(AppId::Files), 10, "home screen and a dark screen add nothing");
        t.tick(base + 14, 0, Some(AppId::Files), true);
        t.tick(base + 1000, 0, Some(AppId::Files), true);
        assert_eq!(t.today().app_secs(AppId::Files), 10 + MAX_STEP_SECS as u32, "a stall is capped");
        t.note_notification(AppId::Photos);
        t.flush();
        let back = load_day(&d, day_of(base, 0));
        assert_eq!(back.notif_total(), 1);
        assert_eq!(back.hours.iter().sum::<u32>(), back.total());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_new_day_starts_fresh_and_old_files_are_pruned() {
        let d = dir("roll");
        let mut t = Tracker::new(&d);
        let base = 1_700_000_000;
        t.tick(base, 0, Some(AppId::Files), true);
        t.tick(base + 5, 0, Some(AppId::Files), true);
        let first_day = day_of(base, 0);
        t.tick(base + 86_400, 0, Some(AppId::Photos), true);
        assert_eq!(load_day(&d, first_day).app_secs(AppId::Files), 5, "the old day was saved on rollover");
        assert_eq!(t.today().total(), 0);
        t.tick(base + 86_400 * (KEEP_DAYS + 2), 0, None, true);
        assert!(!day_path(&d, first_day).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn history_lists_days_oldest_first() {
        let d = dir("hist");
        let h = history(&d, 10, 3);
        assert_eq!(h.iter().map(|(d, _)| *d).collect::<Vec<_>>(), vec![8, 9, 10]);
    }
}
