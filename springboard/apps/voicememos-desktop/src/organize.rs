//! Organising recordings: favourites, search over titles and notes, Recently Deleted with recover and the 30-day
//! purge, folders, playback speed steps and a recording clock that can pause and resume. Pure std; the sidebar and
//! the transport call these, and `store.rs` does the file moves from the plans returned here.
#![allow(dead_code)] // the sidebar rows, the Recently Deleted view and the transport wire these in next

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Days a deleted recording stays in Recently Deleted.
pub const KEEP_DELETED_DAYS: i64 = 30;
/// The folder (inside the memo directory) deleted recordings move to.
pub const DELETED_DIR: &str = ".deleted";

/// The favourite recordings, by file name. Stored one name per line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Favourites {
    names: BTreeSet<String>,
}

impl Favourites {
    pub fn from_text(text: &str) -> Self {
        Favourites { names: text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect() }
    }

    pub fn to_text(&self) -> String {
        self.names.iter().map(|n| format!("{n}\n")).collect()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    /// Flips the flag; the new state.
    pub fn toggle(&mut self, name: &str) -> bool {
        if self.names.remove(name) {
            false
        } else {
            self.names.insert(name.to_string());
            true
        }
    }

    /// A recording was renamed: the flag follows it.
    pub fn rename(&mut self, from: &str, to: &str) {
        if self.names.remove(from) {
            self.names.insert(to.to_string());
        }
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }
}

/// The indices of the recordings whose title or notes contain every word of `query` (case-insensitive). An empty
/// query matches everything.
pub fn search(items: &[(&str, &str)], query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    items
        .iter()
        .enumerate()
        .filter(|(_, (title, notes))| {
            let hay = format!("{} {}", title.to_lowercase(), notes.to_lowercase());
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .map(|(i, _)| i)
        .collect()
}

/// Where a deleted recording goes: `<dir>/.deleted/<stamp>--<file name>`, so the deletion time survives in the name
/// and two recordings with one name do not collide.
pub fn deleted_path(dir: &Path, file_name: &str, now: i64) -> PathBuf {
    dir.join(DELETED_DIR).join(format!("{now}--{file_name}"))
}

/// Splits a path made by `deleted_path` back into the deletion time and the original file name.
pub fn parse_deleted(file_name: &str) -> Option<(i64, String)> {
    let (stamp, name) = file_name.split_once("--")?;
    let at = stamp.parse::<i64>().ok()?;
    (!name.is_empty()).then(|| (at, name.to_string()))
}

/// The original location of a recording in Recently Deleted (Recover).
pub fn recover_path(dir: &Path, deleted_file_name: &str) -> Option<PathBuf> {
    let (_, name) = parse_deleted(deleted_file_name)?;
    Some(dir.join(name))
}

/// The entries of Recently Deleted to remove for good: those deleted `KEEP_DELETED_DAYS` or more days before `now`.
/// `entries` are the file names in the `.deleted` folder; the result holds their indices.
pub fn due_for_purge(entries: &[&str], now: i64) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, n)| parse_deleted(n).is_some_and(|(at, _)| now - at >= KEEP_DELETED_DAYS * 86_400))
        .map(|(i, _)| i)
        .collect()
}

/// Days left before a deleted recording is purged (0 when due).
pub fn days_left(deleted_at: i64, now: i64) -> i64 {
    (KEEP_DELETED_DAYS - (now - deleted_at).max(0) / 86_400).max(0)
}

/// A folder name safe to create inside the memo directory: no separators, not hidden, trimmed, at most 40 chars.
pub fn folder_name(name: &str) -> Option<String> {
    let cleaned: String = name.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':')).collect();
    let cleaned: String = cleaned.trim().trim_start_matches('.').trim().chars().take(40).collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// The folder a recording sits in, relative to the memo directory (None for the top level).
pub fn folder_of(root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    let parent = rel.parent()?;
    let first = parent.components().next()?;
    let name = first.as_os_str().to_string_lossy().into_owned();
    (name != DELETED_DIR).then_some(name)
}

/// Where Move to Folder puts a recording. `None` as the folder means the top level.
pub fn move_target(root: &Path, folder: Option<&str>, file_name: &str) -> Option<PathBuf> {
    match folder {
        None => Some(root.join(file_name)),
        Some(f) => Some(root.join(folder_name(f)?).join(file_name)),
    }
}

/// Playback speeds the transport offers.
pub const RATES: [f32; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

/// The speed after `current` (`faster` or slower), staying at the ends.
pub fn step_rate(current: f32, faster: bool) -> f32 {
    let i = RATES.iter().position(|r| (r - current).abs() < 0.01).unwrap_or(2);
    let j = if faster { (i + 1).min(RATES.len() - 1) } else { i.saturating_sub(1) };
    RATES[j]
}

/// Label for a speed: `1x`, `1.5x`.
pub fn rate_label(rate: f32) -> String {
    if (rate - rate.round()).abs() < 0.001 {
        format!("{}x", rate.round() as i32)
    } else {
        format!("{rate}x")
    }
}

/// The recording clock with pause and resume: elapsed time counts only while running.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RecordClock {
    /// time already counted before the current running stretch, in milliseconds
    base_ms: u64,
    /// when the current running stretch began (milliseconds on any monotonic clock); None while paused or stopped
    since: Option<u64>,
}

impl RecordClock {
    pub fn start(now_ms: u64) -> Self {
        RecordClock { base_ms: 0, since: Some(now_ms) }
    }

    pub fn pause(&mut self, now_ms: u64) {
        if let Some(s) = self.since.take() {
            self.base_ms += now_ms.saturating_sub(s);
        }
    }

    pub fn resume(&mut self, now_ms: u64) {
        if self.since.is_none() {
            self.since = Some(now_ms);
        }
    }

    pub fn is_paused(&self) -> bool {
        self.since.is_none()
    }

    pub fn elapsed_ms(&self, now_ms: u64) -> u64 {
        self.base_ms + self.since.map_or(0, |s| now_ms.saturating_sub(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favourites_toggle_rename_and_text() {
        let mut f = Favourites::default();
        assert!(f.toggle("a.wav"));
        assert!(f.toggle("b.wav"));
        assert!(!f.toggle("a.wav"));
        f.rename("b.wav", "c.wav");
        assert!(f.contains("c.wav") && !f.contains("b.wav"));
        assert_eq!(Favourites::from_text(&f.to_text()), f);
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn search_needs_every_word() {
        let items = [("Meeting notes", "budget for Q4"), ("Idea", "song about budget"), ("Shopping", "")];
        assert_eq!(search(&items, "budget"), vec![0, 1]);
        assert_eq!(search(&items, "BUDGET meeting"), vec![0]);
        assert_eq!(search(&items, ""), vec![0, 1, 2]);
        assert!(search(&items, "zzz").is_empty());
    }

    #[test]
    fn deleted_names_round_trip_and_purge() {
        let dir = Path::new("/m");
        let p = deleted_path(dir, "Idea.wav", 1000);
        assert_eq!(p, PathBuf::from("/m/.deleted/1000--Idea.wav"));
        assert_eq!(parse_deleted("1000--Idea.wav"), Some((1000, "Idea.wav".to_string())));
        assert_eq!(parse_deleted("junk"), None);
        assert_eq!(recover_path(dir, "1000--Idea.wav"), Some(PathBuf::from("/m/Idea.wav")));
        let now = 1000 + 30 * 86_400;
        let recent = format!("{}--b.wav", now - 86_400);
        assert_eq!(due_for_purge(&["1000--a.wav", recent.as_str(), "x"], now), vec![0]);
        assert_eq!(days_left(now - 86_400, now), 29);
        assert_eq!(days_left(0, now), 0);
    }

    #[test]
    fn folders() {
        let root = Path::new("/m");
        assert_eq!(folder_name("  Work/stuff "), Some("Workstuff".to_string()));
        assert_eq!(folder_name("..."), None);
        assert_eq!(folder_of(root, Path::new("/m/Work/a.wav")), Some("Work".to_string()));
        assert_eq!(folder_of(root, Path::new("/m/a.wav")), None);
        assert_eq!(folder_of(root, Path::new("/m/.deleted/1--a.wav")), None);
        assert_eq!(move_target(root, Some("Work"), "a.wav"), Some(PathBuf::from("/m/Work/a.wav")));
        assert_eq!(move_target(root, None, "a.wav"), Some(PathBuf::from("/m/a.wav")));
        assert_eq!(move_target(root, Some("//"), "a.wav"), None);
    }

    #[test]
    fn rates_step_and_label() {
        assert_eq!(step_rate(1.0, true), 1.25);
        assert_eq!(step_rate(1.0, false), 0.75);
        assert_eq!(step_rate(2.0, true), 2.0);
        assert_eq!(step_rate(0.5, false), 0.5);
        assert_eq!(rate_label(1.0), "1x");
        assert_eq!(rate_label(1.5), "1.5x");
    }

    #[test]
    fn clock_skips_paused_time() {
        let mut c = RecordClock::start(1000);
        assert_eq!(c.elapsed_ms(1500), 500);
        c.pause(2000);
        assert!(c.is_paused());
        assert_eq!(c.elapsed_ms(9000), 1000);
        c.resume(10_000);
        assert_eq!(c.elapsed_ms(10_250), 1250);
    }
}
