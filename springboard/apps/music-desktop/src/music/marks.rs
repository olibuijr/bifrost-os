//! What the listener has marked or done with each song, kept by path so a rescan does not lose it: favourites,
//! star ratings (1 to 5) and the log of songs played. Plain text in one file, one record per line
//! (`f<TAB>path`, `r<TAB>stars<TAB>path`, `p<TAB>unix-seconds<TAB>path`). `Shelf` is the source list's smart lists built
//! from them and from the library's date added. Pure logic, no UI.

use super::library::Song;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The most played songs remembered.
const HISTORY_MAX: usize = 200;
/// The most songs a Recently Added list shows.
const RECENT_ADDED_MAX: usize = 100;
/// Ratings at or above this are in Top Rated.
const TOP_STARS: u8 = 4;

#[derive(Clone, Debug, Default)]
pub struct Marks {
    file: Option<PathBuf>,
    favourites: HashSet<PathBuf>,
    stars: HashMap<PathBuf, u8>,
    /// played songs, oldest first, each song once (at its latest play)
    played: Vec<(PathBuf, u64)>,
}

impl Marks {
    /// Reads `file` (which may not exist yet) and keeps later changes in it.
    pub fn open(file: PathBuf) -> Self {
        let mut m = Marks { file: Some(file.clone()), ..Marks::default() };
        if let Ok(text) = std::fs::read_to_string(&file) {
            m.parse(&text);
        }
        m
    }

    fn parse(&mut self, text: &str) {
        for line in text.lines() {
            let mut parts = line.splitn(3, '\t');
            match (parts.next(), parts.next(), parts.next()) {
                (Some("f"), Some(path), None) => {
                    self.favourites.insert(PathBuf::from(path));
                }
                (Some("r"), Some(n), Some(path)) => {
                    if let Some(n) = n.parse::<u8>().ok().filter(|n| (1..=5).contains(n)) {
                        self.stars.insert(PathBuf::from(path), n);
                    }
                }
                (Some("p"), Some(at), Some(path)) => {
                    if let Ok(at) = at.parse::<u64>() {
                        self.push_play(PathBuf::from(path), at);
                    }
                }
                _ => {}
            }
        }
    }

    fn text(&self) -> String {
        let mut favs: Vec<_> = self.favourites.iter().collect();
        favs.sort();
        let mut out = String::new();
        for p in favs {
            out.push_str(&format!("f\t{}\n", p.display()));
        }
        let mut stars: Vec<_> = self.stars.iter().collect();
        stars.sort();
        for (p, n) in stars {
            out.push_str(&format!("r\t{n}\t{}\n", p.display()));
        }
        for (p, at) in &self.played {
            out.push_str(&format!("p\t{at}\t{}\n", p.display()));
        }
        out
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = file.with_extension("tmp");
        if std::fs::write(&tmp, self.text()).is_ok() {
            let _ = std::fs::rename(&tmp, file);
        }
    }

    pub fn is_favourite(&self, song: &Path) -> bool {
        self.favourites.contains(song)
    }

    /// Flips the favourite flag; the new state.
    pub fn toggle_favourite(&mut self, song: &Path) -> bool {
        let on = !self.favourites.remove(song);
        if on {
            self.favourites.insert(song.to_path_buf());
        }
        self.save();
        on
    }

    /// 0 for unrated, else 1 to 5.
    pub fn rating(&self, song: &Path) -> u8 {
        self.stars.get(song).copied().unwrap_or(0)
    }

    /// Sets the rating (0 clears it, above 5 counts as 5).
    pub fn set_rating(&mut self, song: &Path, stars: u8) {
        match stars.min(5) {
            0 => {
                self.stars.remove(song);
            }
            n => {
                self.stars.insert(song.to_path_buf(), n);
            }
        }
        self.save();
    }

    fn push_play(&mut self, song: PathBuf, at: u64) {
        self.played.retain(|(p, _)| *p != song);
        self.played.push((song, at));
        if self.played.len() > HISTORY_MAX {
            self.played.remove(0);
        }
    }

    /// Logs a play at `at` (Unix seconds).
    pub fn record_play(&mut self, song: &Path, at: u64) {
        self.push_play(song.to_path_buf(), at);
        self.save();
    }

    /// Played songs, latest first.
    pub fn recent(&self) -> impl Iterator<Item = &Path> {
        self.played.iter().rev().map(|(p, _)| p.as_path())
    }
}

/// The smart lists in the source list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shelf {
    /// every song (the Songs tab)
    All,
    Favourites,
    RecentlyAdded,
    RecentlyPlayed,
    TopRated,
}

impl Shelf {
    /// The shelves the source list offers, in order.
    pub const LISTED: [Shelf; 4] = [Shelf::RecentlyAdded, Shelf::RecentlyPlayed, Shelf::Favourites, Shelf::TopRated];

    pub fn label(self) -> &'static str {
        match self {
            Shelf::All => "Songs",
            Shelf::Favourites => "Favourite Songs",
            Shelf::RecentlyAdded => "Recently Added",
            Shelf::RecentlyPlayed => "Recently Played",
            Shelf::TopRated => "Top Rated",
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Shelf::All => "list.bullet",
            Shelf::Favourites => "heart",
            Shelf::RecentlyAdded => "clock",
            Shelf::RecentlyPlayed => "clock.arrow.circlepath",
            Shelf::TopRated => "star",
        }
    }

    /// Narrows and orders `ids` (library song ids, already filtered by the search) for this shelf.
    pub fn apply(self, ids: &mut Vec<usize>, songs: &[Song], marks: &Marks) {
        match self {
            Shelf::All => {}
            Shelf::Favourites => ids.retain(|&i| marks.is_favourite(&songs[i].path)),
            Shelf::TopRated => {
                ids.retain(|&i| marks.rating(&songs[i].path) >= TOP_STARS);
                ids.sort_by_key(|&i| std::cmp::Reverse(marks.rating(&songs[i].path)));
            }
            Shelf::RecentlyAdded => {
                ids.sort_by_key(|&i| std::cmp::Reverse(songs[i].added));
                ids.truncate(RECENT_ADDED_MAX);
            }
            Shelf::RecentlyPlayed => {
                let shown: HashMap<&Path, usize> = ids.iter().map(|&i| (songs[i].path.as_path(), i)).collect();
                *ids = marks.recent().filter_map(|p| shown.get(p).copied()).collect();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: usize, name: &str, added: u64) -> Song {
        Song {
            id,
            path: PathBuf::from(format!("/m/{name}.mp3")),
            title: name.into(),
            artist: String::new(),
            album: String::new(),
            album_artist: String::new(),
            track: None,
            year: None,
            duration_ms: None,
            has_cover: false,
            added,
        }
    }

    fn songs() -> Vec<Song> {
        vec![song(0, "a", 10), song(1, "b", 30), song(2, "c", 20)]
    }

    #[test]
    fn favourites_toggle_and_ratings_clamp() {
        let mut m = Marks::default();
        let p = Path::new("/m/a.mp3");
        assert!(m.toggle_favourite(p));
        assert!(m.is_favourite(p));
        assert!(!m.toggle_favourite(p));
        m.set_rating(p, 9);
        assert_eq!(m.rating(p), 5);
        m.set_rating(p, 0);
        assert_eq!(m.rating(p), 0);
    }

    #[test]
    fn a_replayed_song_moves_to_the_front_once_and_the_log_is_capped() {
        let mut m = Marks::default();
        m.record_play(Path::new("/m/a.mp3"), 1);
        m.record_play(Path::new("/m/b.mp3"), 2);
        m.record_play(Path::new("/m/a.mp3"), 3);
        assert_eq!(m.recent().collect::<Vec<_>>(), vec![Path::new("/m/a.mp3"), Path::new("/m/b.mp3")]);
        for n in 0..(HISTORY_MAX + 20) {
            m.record_play(Path::new(&format!("/x/{n}.mp3")), n as u64);
        }
        assert_eq!(m.recent().count(), HISTORY_MAX);
    }

    #[test]
    fn marks_survive_a_reopen() {
        let dir = std::env::temp_dir().join(format!("music-marks-{}", std::process::id()));
        let file = dir.join("marks.txt");
        let mut m = Marks::open(file.clone());
        m.toggle_favourite(Path::new("/m/a.mp3"));
        m.set_rating(Path::new("/m/b.mp3"), 4);
        m.record_play(Path::new("/m/c.mp3"), 77);
        let again = Marks::open(file);
        assert!(again.is_favourite(Path::new("/m/a.mp3")));
        assert_eq!(again.rating(Path::new("/m/b.mp3")), 4);
        assert_eq!(again.recent().collect::<Vec<_>>(), vec![Path::new("/m/c.mp3")]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shelves_filter_and_order() {
        let s = songs();
        let mut m = Marks::default();
        m.toggle_favourite(&s[2].path);
        m.set_rating(&s[0].path, 5);
        m.set_rating(&s[1].path, 4);
        m.set_rating(&s[2].path, 2);
        m.record_play(&s[0].path, 1);
        m.record_play(&s[1].path, 2);

        let mut ids = vec![0, 1, 2];
        Shelf::Favourites.apply(&mut ids, &s, &m);
        assert_eq!(ids, vec![2]);

        let mut ids = vec![0, 1, 2];
        Shelf::TopRated.apply(&mut ids, &s, &m);
        assert_eq!(ids, vec![0, 1]);

        let mut ids = vec![0, 1, 2];
        Shelf::RecentlyAdded.apply(&mut ids, &s, &m);
        assert_eq!(ids, vec![1, 2, 0]);

        let mut ids = vec![0, 1, 2];
        Shelf::RecentlyPlayed.apply(&mut ids, &s, &m);
        assert_eq!(ids, vec![1, 0]);

        // the search narrows first: a song outside it stays out
        let mut ids = vec![0];
        Shelf::RecentlyPlayed.apply(&mut ids, &s, &m);
        assert_eq!(ids, vec![0]);
    }
}
