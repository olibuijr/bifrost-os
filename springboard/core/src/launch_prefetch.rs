//! Launch-time prefetch: remember which apps are opened and when, predict the ones likely to be opened next, and
//! read their executables once so the page cache is warm when the user opens them.
//!
//! Everything stays on this machine. The history is a small text file in the state volume
//! ([`STATE_PATH`]); nothing is sent anywhere and nothing but the app id, its executable path and the launch time
//! is kept. Reading a file to warm the page cache needs no privilege and changes nothing on disk.
//!
//! The prediction is a recency-weighted count. Each launch counts `0.5^(age / 14 days)`, and a launch within one
//! hour of the same hour of the day (UTC, wrapping at midnight) counts three times as much, so an app opened every
//! morning ranks first in the morning. [`History::predict`] returns the best few app ids; [`History::warm`] reads
//! their executables within a byte budget.
//!
//! The shell records a launch in `bare::app_policy::command` ([`note_launch`]) and warms the predicted apps once at
//! start-up on a worker thread ([`warm_default`]).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Where the shell keeps the history (the `@state` volume).
pub const STATE_PATH: &str = "/var/lib/springboard/launch-history.txt";
/// Most launches kept; the oldest are dropped first.
pub const MAX_LAUNCHES: usize = 2000;
/// Most distinct apps remembered with an executable path.
pub const MAX_APPS: usize = 256;
/// Apps warmed at start-up.
pub const WARM_APPS: usize = 4;
/// Bytes read per start-up warm-up, over all apps.
pub const WARM_BUDGET: u64 = 256 * 1024 * 1024;

const DAY: u64 = 86_400;
/// Half-life of a launch's weight, in seconds.
const HALF_LIFE: f64 = 14.0 * DAY as f64;

/// The launches and executable paths of this machine.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    /// `(unix seconds, app id)`, oldest first.
    launches: Vec<(u64, String)>,
    exes: BTreeMap<String, PathBuf>,
}

fn clean(text: &str) -> bool {
    !text.is_empty() && !text.contains(['\t', '\n', '\r', '\0'])
}

/// Hour of the day (0 to 23, UTC) of a unix time.
fn hour_of(ts: u64) -> u64 {
    (ts % DAY) / 3600
}

/// Circular distance between two hours of the day.
fn hour_gap(a: u64, b: u64) -> u64 {
    let d = a.abs_diff(b);
    d.min(24 - d)
}

impl History {
    pub fn is_empty(&self) -> bool {
        self.launches.is_empty()
    }

    pub fn launch_count(&self) -> usize {
        self.launches.len()
    }

    /// The executable remembered for an app.
    pub fn exe(&self, app: &str) -> Option<&Path> {
        self.exes.get(app).map(PathBuf::as_path)
    }

    /// Notes that `app` was opened at `now` (unix seconds) from `exe`. An id or path with a tab or line break is
    /// ignored. The history keeps its newest [`MAX_LAUNCHES`] launches.
    pub fn record(&mut self, app: &str, exe: &Path, now: u64) {
        let Some(exe_text) = exe.to_str() else { return };
        if !clean(app) || !clean(exe_text) {
            return;
        }
        if self.exes.len() >= MAX_APPS && !self.exes.contains_key(app) {
            return;
        }
        self.exes.insert(app.to_owned(), exe.to_path_buf());
        self.launches.push((now, app.to_owned()));
        if self.launches.len() > MAX_LAUNCHES {
            let drop = self.launches.len() - MAX_LAUNCHES;
            self.launches.drain(..drop);
            let live: std::collections::BTreeSet<&str> = self.launches.iter().map(|(_, a)| a.as_str()).collect();
            self.exes.retain(|a, _| live.contains(a.as_str()));
        }
    }

    /// The most likely next apps at `now`, best first, at most `limit`. Apps with no known executable are skipped.
    pub fn predict(&self, now: u64, limit: usize) -> Vec<String> {
        let mut score: BTreeMap<&str, f64> = BTreeMap::new();
        for (ts, app) in &self.launches {
            if !self.exes.contains_key(app) {
                continue;
            }
            let age = now.saturating_sub(*ts) as f64;
            let mut weight = 0.5f64.powf(age / HALF_LIFE);
            if hour_gap(hour_of(*ts), hour_of(now)) <= 1 {
                weight *= 3.0;
            }
            *score.entry(app.as_str()).or_insert(0.0) += weight;
        }
        let mut ranked: Vec<(&str, f64)> = score.into_iter().collect();
        // best score first; equal scores in id order so the answer does not depend on map layout
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
        ranked.into_iter().take(limit).map(|(a, _)| a.to_owned()).collect()
    }

    /// Reads the executables of the apps predicted at `now`, at most `apps` of them and `budget` bytes in all, and
    /// returns the bytes read. A missing or unreadable executable is skipped.
    pub fn warm(&self, now: u64, apps: usize, budget: u64) -> u64 {
        let mut left = budget;
        let mut total = 0;
        for app in self.predict(now, apps) {
            if left == 0 {
                break;
            }
            if let Some(exe) = self.exe(&app) {
                let read = warm_file(exe, left).unwrap_or(0);
                left -= read;
                total += read;
            }
        }
        total
    }

    /// The history as text: `L<TAB>time<TAB>app` per launch and `E<TAB>app<TAB>path` per app.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (app, exe) in &self.exes {
            out.push_str(&format!("E\t{app}\t{}\n", exe.display()));
        }
        for (ts, app) in &self.launches {
            out.push_str(&format!("L\t{ts}\t{app}\n"));
        }
        out
    }

    /// Reads [`History::to_text`] output. A line that does not parse is skipped, and the limits apply.
    pub fn parse(text: &str) -> History {
        let mut h = History::default();
        for line in text.lines() {
            let mut f = line.split('\t');
            match (f.next(), f.next(), f.next(), f.next()) {
                (Some("E"), Some(app), Some(path), None) if clean(app) && clean(path) && h.exes.len() < MAX_APPS => {
                    h.exes.insert(app.to_owned(), PathBuf::from(path));
                }
                (Some("L"), Some(ts), Some(app), None) if clean(app) => {
                    if let Ok(ts) = ts.parse::<u64>() {
                        h.launches.push((ts, app.to_owned()));
                    }
                }
                _ => {}
            }
        }
        if h.launches.len() > MAX_LAUNCHES {
            let drop = h.launches.len() - MAX_LAUNCHES;
            h.launches.drain(..drop);
        }
        h.launches.retain(|(_, a)| h.exes.contains_key(a));
        h
    }

    pub fn load(path: &Path) -> History {
        fs::read_to_string(path).map(|t| History::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        crate::fsutil::write_atomic(path, self.to_text())
    }
}

/// Reads up to `budget` bytes of a file and drops them, which leaves the file in the page cache. Returns the bytes
/// read.
pub fn warm_file(path: &Path, budget: u64) -> io::Result<u64> {
    let mut file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Ok(0);
    }
    let mut buf = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    while total < budget {
        let want = buf.len().min((budget - total) as usize);
        let n = file.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        total += n as u64;
    }
    Ok(total)
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Records a launch in the state file. Best effort: a read-only or missing state volume is ignored.
pub fn note_launch(app: &str, exe: &Path) {
    note_launch_at(Path::new(STATE_PATH), app, exe, unix_now());
}

/// [`note_launch`] with the file and the time given.
pub fn note_launch_at(path: &Path, app: &str, exe: &Path, now: u64) {
    let mut h = History::load(path);
    h.record(app, exe, now);
    let _ = h.save(path);
}

/// Warms the predicted apps from the state file, once. Meant for a worker thread at start-up.
pub fn warm_default() -> u64 {
    History::load(Path::new(STATE_PATH)).warm(unix_now(), WARM_APPS, WARM_BUDGET)
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u64 = 3600;
    // day 100 at 09:00 UTC
    const NOON: u64 = 100 * DAY + 9 * H;

    fn exe(name: &str) -> PathBuf {
        PathBuf::from(format!("/usr/lib/apps/{name}/{name}"))
    }

    #[test]
    fn hours_wrap_at_midnight() {
        assert_eq!(hour_gap(23, 0), 1);
        assert_eq!(hour_gap(1, 23), 2);
        assert_eq!(hour_gap(9, 9), 0);
        assert_eq!(hour_of(NOON), 9);
    }

    #[test]
    fn a_frequent_app_outranks_a_rare_one() {
        let mut h = History::default();
        for d in 1..=5 {
            h.record("mail", &exe("mail"), NOON - d * DAY);
        }
        h.record("notes", &exe("notes"), NOON - DAY);
        assert_eq!(h.predict(NOON, 2), vec!["mail", "notes"]);
        assert_eq!(h.predict(NOON, 1), vec!["mail"]);
    }

    #[test]
    fn the_same_hour_of_day_counts_more() {
        let mut h = History::default();
        // three evening launches and three morning launches, equally old; asked in the morning
        for d in 1..=3 {
            h.record("evening", &exe("evening"), NOON - d * DAY + 9 * H);
            h.record("morning", &exe("morning"), NOON - d * DAY);
        }
        assert_eq!(h.predict(NOON, 1), vec!["morning"]);
        assert_eq!(h.predict(NOON + 9 * H, 1), vec!["evening"]);
    }

    #[test]
    fn old_launches_fade() {
        let mut h = History::default();
        for _ in 0..4 {
            h.record("old", &exe("old"), NOON - 120 * DAY);
        }
        h.record("new", &exe("new"), NOON - 3 * DAY);
        assert_eq!(h.predict(NOON, 1), vec!["new"]);
    }

    #[test]
    fn ids_and_paths_with_separators_are_ignored() {
        let mut h = History::default();
        h.record("a\tb", &exe("x"), 1);
        h.record("", &exe("x"), 1);
        h.record("ok", Path::new("/bin/a\nb"), 1);
        assert!(h.is_empty());
        h.record("ok", &exe("ok"), 1);
        assert_eq!(h.launch_count(), 1);
    }

    #[test]
    fn the_history_is_bounded() {
        let mut h = History::default();
        for i in 0..(MAX_LAUNCHES as u64 + 50) {
            h.record("a", &exe("a"), i);
        }
        assert_eq!(h.launch_count(), MAX_LAUNCHES);
        let mut many = History::default();
        for i in 0..(MAX_APPS + 10) {
            many.record(&format!("app{i}"), &exe("x"), i as u64);
        }
        assert_eq!(many.exes.len(), MAX_APPS);
    }

    #[test]
    fn the_text_round_trips() {
        let mut h = History::default();
        h.record("mail", &exe("mail"), 10);
        h.record("notes", &exe("notes"), 20);
        h.record("mail", &exe("mail"), 30);
        let back = History::parse(&h.to_text());
        assert_eq!(back, h);
        let junk = History::parse("garbage\nL\tnotanumber\tmail\nL\t5\tghost\nE\tmail\t/bin/m\nL\t7\tmail\n");
        assert_eq!(junk.launch_count(), 1, "unknown apps and bad times are dropped");
        assert_eq!(junk.exe("mail"), Some(Path::new("/bin/m")));
    }

    #[test]
    fn warming_reads_within_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big");
        fs::write(&big, vec![7u8; 1_000_000]).unwrap();
        assert_eq!(warm_file(&big, u64::MAX).unwrap(), 1_000_000);
        assert_eq!(warm_file(&big, 4096).unwrap(), 4096);
        assert_eq!(warm_file(dir.path(), 4096).unwrap(), 0, "a directory is not read");
        assert!(warm_file(&dir.path().join("missing"), 10).is_err());

        let mut h = History::default();
        h.record("big", &big, NOON - H);
        h.record("gone", &dir.path().join("gone"), NOON - H);
        assert_eq!(h.warm(NOON, 4, 600_000), 600_000);
        assert_eq!(h.warm(NOON, 4, u64::MAX), 1_000_000);
    }

    #[test]
    fn a_launch_is_saved_and_reloaded() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state/launch-history.txt");
        note_launch_at(&state, "mail", &exe("mail"), 1000);
        note_launch_at(&state, "mail", &exe("mail"), 2000);
        let h = History::load(&state);
        assert_eq!(h.launch_count(), 2);
        assert_eq!(h.exe("mail"), Some(exe("mail").as_path()));
        assert!(History::load(&dir.path().join("none")).is_empty());
    }
}
