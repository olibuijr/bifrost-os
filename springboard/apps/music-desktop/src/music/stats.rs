//! What the listener did with each song, kept apart from the files: a star rating, a favourite flag, how often it
//! played, when it last played, and the short history of what played lately. Songs are keyed by path, so a rescan of
//! the library does not lose them. Pure logic and one small text file, no UI.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The most songs the play history keeps.
pub const HISTORY_MAX: usize = 100;
/// The highest star rating.
pub const MAX_RATING: u8 = 5;
/// A song counts as played once it has been heard this long, or for half its length when it is shorter than twice this.
pub const COUNT_AFTER_MS: f64 = 30_000.0;
/// A song first seen this far in is taken as already counted (the service kept playing while the app was closed).
const JOINED_LATE_MS: f64 = 5_000.0;

/// What is kept about one song.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SongStats {
    /// 0 for none, then 1 to 5 stars
    pub rating: u8,
    pub favourite: bool,
    pub plays: u32,
    /// Unix seconds when the song last started, 0 when it never did
    pub last_played: u64,
}

/// The listening data of every song, and where it is saved.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    file: Option<PathBuf>,
    songs: HashMap<PathBuf, SongStats>,
    /// the songs that started lately, most recent first
    history: Vec<PathBuf>,
}

/// The star label of a rating: "None", "1 Star", "3 Stars".
pub fn rating_text(rating: u8) -> String {
    match rating {
        0 => "None".to_string(),
        1 => "1 Star".to_string(),
        n => format!("{n} Stars"),
    }
}

impl Stats {
    /// Reads the data saved in `file` (which may not exist yet) and saves there from now on.
    pub fn open(file: PathBuf) -> Self {
        let mut stats = Stats { file: Some(file.clone()), ..Stats::default() };
        if let Ok(text) = std::fs::read_to_string(&file) {
            stats.load(&text);
        }
        stats
    }

    fn load(&mut self, text: &str) {
        let mut history = Vec::new();
        for line in text.lines() {
            if let Some(path) = line.strip_prefix("play\t") {
                if !path.is_empty() {
                    history.push(PathBuf::from(path));
                }
                continue;
            }
            let mut fields = line.splitn(6, '\t');
            if fields.next() != Some("song") {
                continue;
            }
            let (Some(rating), Some(fav), Some(plays), Some(last), Some(path)) = (fields.next(), fields.next(), fields.next(), fields.next(), fields.next()) else { continue };
            if path.is_empty() {
                continue;
            }
            let stats = SongStats { rating: rating.parse().unwrap_or(0).min(MAX_RATING), favourite: fav == "1", plays: plays.parse().unwrap_or(0), last_played: last.parse().unwrap_or(0) };
            self.songs.insert(PathBuf::from(path), stats);
        }
        // the file lists the history oldest first
        history.reverse();
        history.truncate(HISTORY_MAX);
        self.history = history;
    }

    /// The text saved to disk: one `song` line per song with something kept, then the history oldest first.
    pub fn to_text(&self) -> String {
        let mut keep: Vec<(&PathBuf, &SongStats)> = self.songs.iter().filter(|(p, s)| **s != SongStats::default() && !has_line_break(p)).collect();
        keep.sort_by(|a, b| a.0.cmp(b.0));
        let mut out = String::new();
        for (path, s) in keep {
            out.push_str(&format!("song\t{}\t{}\t{}\t{}\t{}\n", s.rating, s.favourite as u8, s.plays, s.last_played, path.display()));
        }
        for path in self.history.iter().rev().filter(|p| !has_line_break(p)) {
            out.push_str(&format!("play\t{}\n", path.display()));
        }
        out
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = file.with_extension("tmp");
        if std::fs::write(&tmp, self.to_text()).is_ok() {
            let _ = std::fs::rename(&tmp, file);
        }
    }

    fn entry(&mut self, path: &Path) -> &mut SongStats {
        self.songs.entry(path.to_path_buf()).or_default()
    }

    /// Everything kept about the song at `path`.
    pub fn get(&self, path: &Path) -> SongStats {
        self.songs.get(path).copied().unwrap_or_default()
    }

    pub fn rating(&self, path: &Path) -> u8 {
        self.get(path).rating
    }

    pub fn is_favourite(&self, path: &Path) -> bool {
        self.get(path).favourite
    }

    pub fn plays(&self, path: &Path) -> u32 {
        self.get(path).plays
    }

    pub fn last_played(&self, path: &Path) -> u64 {
        self.get(path).last_played
    }

    /// Sets the stars (clamped to 5; 0 clears the rating).
    pub fn set_rating(&mut self, path: &Path, rating: u8) {
        self.entry(path).rating = rating.min(MAX_RATING);
        self.save();
    }

    /// The next rating in the cycle none, 1 star, ... 5 stars, none. Returns the new rating.
    pub fn cycle_rating(&mut self, path: &Path) -> u8 {
        let next = (self.rating(path) + 1) % (MAX_RATING + 1);
        self.set_rating(path, next);
        next
    }

    /// Flips the favourite flag; returns whether the song is a favourite now.
    pub fn toggle_favourite(&mut self, path: &Path) -> bool {
        let now = !self.is_favourite(path);
        self.entry(path).favourite = now;
        self.save();
        now
    }

    /// The song started playing at `now` (Unix seconds): it heads the history, and a song that is already at the head
    /// stays there once.
    pub fn record_start(&mut self, path: &Path, now: u64) {
        if self.history.first().map(PathBuf::as_path) != Some(path) {
            self.history.insert(0, path.to_path_buf());
            self.history.truncate(HISTORY_MAX);
        }
        self.entry(path).last_played = now;
        self.save();
    }

    /// The song was heard long enough to count as a play.
    pub fn record_play(&mut self, path: &Path, now: u64) {
        let e = self.entry(path);
        e.plays = e.plays.saturating_add(1);
        e.last_played = now;
        self.save();
    }

    /// The songs that started lately, most recent first.
    pub fn history(&self) -> &[PathBuf] {
        &self.history
    }

    /// How many songs carry a rating, a favourite flag or a play (the songs `to_text` keeps).
    pub fn len(&self) -> usize {
        self.songs.values().filter(|s| **s != SongStats::default()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn has_line_break(path: &Path) -> bool {
    path.to_string_lossy().contains(['\n', '\r'])
}

/// A change in what plays, found by `PlayTracker::observe`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlayEvent {
    /// a song began playing
    Started(PathBuf),
    /// the song was heard long enough to count (once per play)
    Played(PathBuf),
}

/// Decides, from the transport's position updates, when a song starts and when it has been played.
#[derive(Debug, Default)]
pub struct PlayTracker {
    current: Option<PathBuf>,
    counted: bool,
    last_ms: f64,
}

impl PlayTracker {
    /// Feeds one update while the transport plays. `track` is the file loaded (None for a station or nothing, which
    /// forgets the song), `position_ms` where it is and `duration_ms` its length. Returns what happened since the last
    /// update.
    pub fn observe(&mut self, track: Option<&Path>, position_ms: f64, duration_ms: Option<f64>) -> Vec<PlayEvent> {
        let mut events = Vec::new();
        let Some(path) = track else {
            *self = PlayTracker::default();
            return events;
        };
        let same = self.current.as_deref() == Some(path);
        // a counted song that jumps back to its start plays again (repeat one)
        let restarted = same && self.counted && position_ms < 2_000.0 && position_ms + JOINED_LATE_MS < self.last_ms;
        if !same || restarted {
            self.current = Some(path.to_path_buf());
            if !same && position_ms > JOINED_LATE_MS {
                // joined in the middle of a song: it was counted before
                self.counted = true;
                self.last_ms = position_ms;
                return events;
            }
            self.counted = false;
            events.push(PlayEvent::Started(path.to_path_buf()));
        }
        self.last_ms = position_ms;
        let threshold = duration_ms.filter(|d| *d > 0.0).map_or(COUNT_AFTER_MS, |d| (d / 2.0).min(COUNT_AFTER_MS));
        if !self.counted && position_ms >= threshold {
            self.counted = true;
            events.push(PlayEvent::Played(path.to_path_buf()));
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-stats").join(name);
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn rating_cycles_through_the_stars_and_back_to_none() {
        let mut s = Stats::default();
        let song = p("/m/a.mp3");
        let seen: Vec<u8> = (0..7).map(|_| s.cycle_rating(&song)).collect();
        assert_eq!(seen, [1, 2, 3, 4, 5, 0, 1]);
        s.set_rating(&song, 9);
        assert_eq!(s.rating(&song), MAX_RATING, "clamped");
        assert_eq!(rating_text(0), "None");
        assert_eq!(rating_text(1), "1 Star");
        assert_eq!(rating_text(4), "4 Stars");
    }

    #[test]
    fn favourites_toggle() {
        let mut s = Stats::default();
        let song = p("/m/a.mp3");
        assert!(!s.is_favourite(&song));
        assert!(s.toggle_favourite(&song));
        assert!(s.is_favourite(&song));
        assert!(!s.toggle_favourite(&song));
    }

    #[test]
    fn history_is_recent_first_without_repeats_at_the_head_and_bounded() {
        let mut s = Stats::default();
        s.record_start(Path::new("/m/a.mp3"), 10);
        s.record_start(Path::new("/m/b.mp3"), 20);
        s.record_start(Path::new("/m/b.mp3"), 30);
        s.record_start(Path::new("/m/a.mp3"), 40);
        assert_eq!(s.history(), [p("/m/a.mp3"), p("/m/b.mp3"), p("/m/a.mp3")]);
        assert_eq!(s.last_played(Path::new("/m/a.mp3")), 40);
        for i in 0..(HISTORY_MAX + 20) {
            s.record_start(Path::new(&format!("/m/{i}.mp3")), 50 + i as u64);
        }
        assert_eq!(s.history().len(), HISTORY_MAX);
        assert_eq!(s.history()[0], p(&format!("/m/{}.mp3", HISTORY_MAX + 19)));
    }

    #[test]
    fn plays_count_and_stamp_the_time() {
        let mut s = Stats::default();
        let song = Path::new("/m/a.mp3");
        s.record_play(song, 100);
        s.record_play(song, 200);
        assert_eq!((s.plays(song), s.last_played(song)), (2, 200));
        assert_eq!(s.plays(Path::new("/m/none.mp3")), 0);
    }

    #[test]
    fn everything_survives_a_reopen() {
        let d = dir("reopen");
        let file = d.join("songs.txt");
        let mut s = Stats::open(file.clone());
        assert!(s.is_empty());
        let a = Path::new("/m/a b.mp3");
        s.set_rating(a, 4);
        s.toggle_favourite(a);
        s.record_start(a, 500);
        s.record_play(a, 510);
        s.record_start(Path::new("/m/with\ttab.mp3"), 600);
        let back = Stats::open(file);
        assert_eq!(back.get(a), SongStats { rating: 4, favourite: true, plays: 1, last_played: 510 });
        assert_eq!(back.history(), [p("/m/with\ttab.mp3"), p("/m/a b.mp3")]);
        assert_eq!(back.last_played(Path::new("/m/with\ttab.mp3")), 600);
        assert_eq!(back.len(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_damaged_file_loses_only_its_bad_lines() {
        let mut s = Stats::default();
        s.load("song\t3\t1\t7\t99\t/m/a.mp3\nsong\tnot\tenough\nplay\t\ngarbage\nsong\t9\t0\tx\ty\t/m/b.mp3\n");
        assert_eq!(s.get(Path::new("/m/a.mp3")), SongStats { rating: 3, favourite: true, plays: 7, last_played: 99 });
        assert_eq!(s.get(Path::new("/m/b.mp3")), SongStats { rating: MAX_RATING, favourite: false, plays: 0, last_played: 0 }, "a bad number reads as 0, a big rating is clamped");
        assert!(s.history().is_empty());
    }

    #[test]
    fn a_song_counts_after_thirty_seconds_once() {
        let mut t = PlayTracker::default();
        let a = Path::new("/m/a.mp3");
        assert_eq!(t.observe(Some(a), 0.0, Some(200_000.0)), [PlayEvent::Started(p("/m/a.mp3"))]);
        assert!(t.observe(Some(a), 10_000.0, Some(200_000.0)).is_empty());
        assert_eq!(t.observe(Some(a), 31_000.0, Some(200_000.0)), [PlayEvent::Played(p("/m/a.mp3"))]);
        assert!(t.observe(Some(a), 60_000.0, Some(200_000.0)).is_empty(), "counted once");
    }

    #[test]
    fn a_short_song_counts_at_half_its_length() {
        let mut t = PlayTracker::default();
        let a = Path::new("/m/a.mp3");
        t.observe(Some(a), 0.0, Some(20_000.0));
        assert!(t.observe(Some(a), 9_000.0, Some(20_000.0)).is_empty());
        assert_eq!(t.observe(Some(a), 10_500.0, Some(20_000.0)), [PlayEvent::Played(p("/m/a.mp3"))]);
    }

    #[test]
    fn repeat_one_counts_each_pass_and_a_new_song_starts_over() {
        let mut t = PlayTracker::default();
        let (a, b) = (Path::new("/m/a.mp3"), Path::new("/m/b.mp3"));
        t.observe(Some(a), 0.0, Some(60_000.0));
        t.observe(Some(a), 31_000.0, Some(60_000.0));
        t.observe(Some(a), 59_000.0, Some(60_000.0));
        assert_eq!(t.observe(Some(a), 500.0, Some(60_000.0)), [PlayEvent::Started(p("/m/a.mp3"))], "back at the start: a new play");
        assert_eq!(t.observe(Some(a), 31_000.0, Some(60_000.0)), [PlayEvent::Played(p("/m/a.mp3"))]);
        assert_eq!(t.observe(Some(b), 0.0, None), [PlayEvent::Started(p("/m/b.mp3"))]);
        assert_eq!(t.observe(Some(b), 30_500.0, None), [PlayEvent::Played(p("/m/b.mp3"))], "no length: thirty seconds");
    }

    #[test]
    fn a_song_found_in_the_middle_is_not_counted_again() {
        let mut t = PlayTracker::default();
        let a = Path::new("/m/a.mp3");
        assert!(t.observe(Some(a), 90_000.0, Some(200_000.0)).is_empty());
        assert!(t.observe(Some(a), 91_000.0, Some(200_000.0)).is_empty());
    }

    #[test]
    fn a_station_forgets_the_song() {
        let mut t = PlayTracker::default();
        let a = Path::new("/m/a.mp3");
        t.observe(Some(a), 0.0, None);
        assert!(t.observe(None, 0.0, None).is_empty());
        assert_eq!(t.observe(Some(a), 0.0, None), [PlayEvent::Started(p("/m/a.mp3"))]);
    }
}
