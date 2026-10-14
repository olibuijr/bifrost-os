//! The listener's state beside the subscriptions: which episodes are played, the Up Next queue, and the rules that
//! pick episodes across shows (New Episodes, auto-download, how many downloads to keep). Pure data and functions.

use crate::feed::{Episode, Show};
use springboard::shell::prefs::write_atomic;
use std::path::{Path, PathBuf};

/// Played episodes and the Up Next queue, by episode address.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Library {
    played: Vec<String>,
    queue: Vec<String>,
}

impl Library {
    pub fn is_played(&self, url: &str) -> bool {
        self.played.iter().any(|u| u == url)
    }

    /// Marks an episode played or unplayed. True when that changed something.
    pub fn set_played(&mut self, url: &str, played: bool) -> bool {
        let was = self.is_played(url);
        if played && !was {
            self.played.push(url.to_string());
        } else if !played && was {
            self.played.retain(|u| u != url);
        }
        was != played
    }

    pub fn queue(&self) -> &[String] {
        &self.queue
    }

    pub fn in_queue(&self, url: &str) -> bool {
        self.queue.iter().any(|u| u == url)
    }

    /// Plays Next: the episode goes to the front of Up Next.
    pub fn play_next(&mut self, url: &str) {
        self.queue.retain(|u| u != url);
        self.queue.insert(0, url.to_string());
    }

    /// Plays Last: the episode goes to the end of Up Next.
    pub fn play_last(&mut self, url: &str) {
        self.queue.retain(|u| u != url);
        self.queue.push(url.to_string());
    }

    pub fn dequeue(&mut self, url: &str) {
        self.queue.retain(|u| u != url);
    }

    /// Takes the episode that plays after the current one.
    pub fn take_next(&mut self) -> Option<String> {
        (!self.queue.is_empty()).then(|| self.queue.remove(0))
    }

    pub fn clear_queue(&mut self) {
        self.queue.clear();
    }

    /// Moves a queued episode one place toward the front (`up`) or the end. False at the edge or when not queued.
    pub fn move_in_queue(&mut self, url: &str, up: bool) -> bool {
        let Some(i) = self.queue.iter().position(|u| u == url) else { return false };
        let j = if up { i.checked_sub(1) } else { Some(i + 1).filter(|&j| j < self.queue.len()) };
        match j {
            Some(j) => {
                self.queue.swap(i, j);
                true
            }
            None => false,
        }
    }

    /// Moves a queued episode to position `to` of Up Next (clamped to the last place), for a drag. False when the
    /// episode is not queued or already there.
    pub fn move_to(&mut self, url: &str, to: usize) -> bool {
        let Some(from) = self.queue.iter().position(|u| u == url) else { return false };
        let to = to.min(self.queue.len() - 1);
        if from == to {
            return false;
        }
        let item = self.queue.remove(from);
        self.queue.insert(to, item);
        true
    }

    /// Forgets the played flags and queue entries of `urls` (an unfollowed show's episodes).
    pub fn forget(&mut self, urls: &[&str]) {
        self.played.retain(|u| !urls.contains(&u.as_str()));
        self.queue.retain(|u| !urls.contains(&u.as_str()));
    }

    fn file(dir: &Path, name: &str) -> PathBuf {
        dir.join(name)
    }

    fn read_lines(dir: &Path, name: &str) -> Vec<String> {
        std::fs::read_to_string(Self::file(dir, name)).unwrap_or_default().lines().filter(|l| !l.is_empty()).map(str::to_string).collect()
    }

    pub fn load(dir: &Path) -> Self {
        Library { played: Self::read_lines(dir, "played.tsv"), queue: Self::read_lines(dir, "queue.tsv") }
    }

    pub fn save(&self, dir: &Path) {
        let join = |v: &[String]| v.iter().map(|u| format!("{u}\n")).collect::<String>();
        let _ = write_atomic(&Self::file(dir, "played.tsv"), join(&self.played).as_bytes());
        let _ = write_atomic(&Self::file(dir, "queue.tsv"), join(&self.queue).as_bytes());
    }
}

/// Episode indices of `eps`, newest first (by release time; feed order breaks ties and covers feeds with no dates).
pub fn newest_first(eps: &[Episode]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..eps.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(eps[i].ts));
    order
}

/// The unplayed episodes of all shows, newest first, at most `limit`: (show index, episode index).
pub fn latest(shows: &[Show], library: &Library, limit: usize) -> Vec<(usize, usize)> {
    let mut all: Vec<(i64, usize, usize)> = Vec::new();
    for (si, show) in shows.iter().enumerate() {
        for (ei, e) in show.episodes.iter().enumerate() {
            if !library.is_played(&e.url) {
                all.push((e.ts, si, ei));
            }
        }
    }
    all.sort_by_key(|&(ts, si, ei)| (std::cmp::Reverse(ts), si, ei));
    all.into_iter().take(limit).map(|(_, si, ei)| (si, ei)).collect()
}

/// The episodes to download for a show with auto-download: the newest unplayed ones not yet downloaded, looking at
/// the newest `max(keep, 1)` unplayed episodes.
pub fn auto_downloads<'a>(eps: &'a [Episode], library: &Library, keep: u32, downloaded: impl Fn(&str) -> bool) -> Vec<&'a str> {
    let window = keep.max(1) as usize;
    newest_first(eps)
        .into_iter()
        .map(|i| &eps[i])
        .filter(|e| !library.is_played(&e.url))
        .take(window)
        .filter(|e| !downloaded(&e.url))
        .map(|e| e.url.as_str())
        .collect()
}

/// The downloaded episodes to delete so that only the newest `keep` stay; `keep` of 0 keeps all. `protected` (the
/// episode playing) is never listed.
pub fn prune<'a>(eps: &'a [Episode], keep: u32, downloaded: impl Fn(&str) -> bool, protected: Option<&str>) -> Vec<&'a str> {
    if keep == 0 {
        return Vec::new();
    }
    newest_first(eps)
        .into_iter()
        .map(|i| &eps[i])
        .filter(|e| downloaded(&e.url))
        .skip(keep as usize)
        .filter(|e| protected != Some(e.url.as_str()))
        .map(|e| e.url.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(url: &str, ts: i64) -> Episode {
        Episode { url: url.into(), ts, ..Episode::default() }
    }

    fn show(eps: Vec<Episode>) -> Show {
        Show { url: "s".into(), title: "S".into(), episodes: eps }
    }

    #[test]
    fn played_flags_toggle_and_report_change() {
        let mut lib = Library::default();
        assert!(lib.set_played("a", true));
        assert!(!lib.set_played("a", true));
        assert!(lib.is_played("a"));
        assert!(lib.set_played("a", false));
        assert!(!lib.set_played("a", false));
        assert!(!lib.is_played("a"));
    }

    #[test]
    fn the_queue_orders_next_and_last_without_duplicates() {
        let mut lib = Library::default();
        lib.play_last("a");
        lib.play_last("b");
        lib.play_next("c");
        lib.play_last("a");
        assert_eq!(lib.queue(), ["c", "b", "a"]);
        assert!(lib.in_queue("b"));
        assert_eq!(lib.take_next().as_deref(), Some("c"));
        lib.dequeue("a");
        assert_eq!(lib.queue(), ["b"]);
        lib.clear_queue();
        assert_eq!(lib.take_next(), None);
    }

    #[test]
    fn queued_episodes_move_one_place_at_a_time() {
        let mut lib = Library::default();
        for u in ["a", "b", "c"] {
            lib.play_last(u);
        }
        assert!(!lib.move_in_queue("a", true));
        assert!(lib.move_in_queue("c", true));
        assert_eq!(lib.queue(), ["a", "c", "b"]);
        assert!(lib.move_in_queue("a", false));
        assert!(!lib.move_in_queue("b", false));
        assert!(!lib.move_in_queue("zzz", true));
        assert_eq!(lib.queue(), ["c", "a", "b"]);
    }

    #[test]
    fn a_dragged_episode_lands_at_the_target_place() {
        let mut lib = Library::default();
        for u in ["a", "b", "c", "d"] {
            lib.play_last(u);
        }
        assert!(!lib.move_to("b", 1), "already there");
        assert!(lib.move_to("a", 2));
        assert_eq!(lib.queue(), ["b", "c", "a", "d"]);
        assert!(lib.move_to("d", 0));
        assert_eq!(lib.queue(), ["d", "b", "c", "a"]);
        assert!(lib.move_to("d", 99), "past the end clamps to the last place");
        assert_eq!(lib.queue(), ["b", "c", "a", "d"]);
        assert!(!lib.move_to("zzz", 0));
    }

    #[test]
    fn forgetting_episodes_clears_played_and_queue() {
        let mut lib = Library::default();
        lib.set_played("a", true);
        lib.play_last("a");
        lib.play_last("b");
        lib.forget(&["a"]);
        assert!(!lib.is_played("a"));
        assert_eq!(lib.queue(), ["b"]);
    }

    #[test]
    fn the_library_is_written_and_read() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("test-library-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut lib = Library::default();
        lib.set_played("p", true);
        lib.play_last("q");
        lib.save(&dir);
        assert_eq!(Library::load(&dir), lib);
        assert_eq!(Library::load(&dir.join("none")), Library::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_episodes_merge_shows_newest_first_and_skip_played() {
        let shows = vec![show(vec![ep("a1", 300), ep("a2", 100)]), show(vec![ep("b1", 200), ep("b2", 400)])];
        let mut lib = Library::default();
        let all = latest(&shows, &lib, 10);
        assert_eq!(all, [(1, 1), (0, 0), (1, 0), (0, 1)]);
        lib.set_played("b2", true);
        assert_eq!(latest(&shows, &lib, 2), [(0, 0), (1, 0)]);
    }

    #[test]
    fn undated_episodes_keep_feed_order() {
        let eps = vec![ep("x", 0), ep("y", 0), ep("z", 0)];
        assert_eq!(newest_first(&eps), [0, 1, 2]);
    }

    #[test]
    fn auto_download_takes_the_newest_unplayed_missing_ones() {
        let eps = vec![ep("old", 1), ep("mid", 2), ep("new", 3)];
        let mut lib = Library::default();
        let none = |_: &str| false;
        assert_eq!(auto_downloads(&eps, &lib, 0, none), ["new"]);
        assert_eq!(auto_downloads(&eps, &lib, 2, none), ["new", "mid"]);
        assert_eq!(auto_downloads(&eps, &lib, 2, |u| u == "new"), ["mid"]);
        lib.set_played("new", true);
        assert_eq!(auto_downloads(&eps, &lib, 0, none), ["mid"]);
    }

    #[test]
    fn pruning_keeps_the_newest_downloads_and_spares_the_playing_one() {
        let eps = vec![ep("a", 1), ep("b", 2), ep("c", 3), ep("d", 4)];
        let have = |u: &str| u != "d";
        assert!(prune(&eps, 0, have, None).is_empty());
        assert_eq!(prune(&eps, 1, have, None), ["b", "a"]);
        assert_eq!(prune(&eps, 1, have, Some("a")), ["b"]);
        assert!(prune(&eps, 5, have, None).is_empty());
    }
}
