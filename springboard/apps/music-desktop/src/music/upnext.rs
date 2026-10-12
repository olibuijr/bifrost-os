//! The play queue as the user sees it: what has played, what plays now and
//! the Up Next list, with add next / add last, reorder, remove and shuffle.
//! Pure logic; it holds file paths so it survives a library rescan, and
//! saves to a small text file so the queue survives a restart. The audio
//! engine always gets the list in play order (shuffle off), so what this
//! shows is what will play.

use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpNext {
    items: Vec<PathBuf>,
    current: usize,
    shuffled: bool,
    /// list order before shuffling, restored when shuffle goes off
    original: Vec<PathBuf>,
}

/// Fisher-Yates with an xorshift generator, so a seed gives one order.
pub fn shuffle_slice<T>(items: &mut [T], seed: u64) {
    let mut x = (seed ^ 0x9E37_79B9_7F4A_7C15).wrapping_mul(0x2545_F491_4F6C_DD1D) | 1;
    for i in (1..items.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        items.swap(i, (x % (i as u64 + 1)) as usize);
    }
}

impl UpNext {
    /// A queue over `items`, playing `start` (clamped).
    pub fn new(items: Vec<PathBuf>, start: usize) -> Self {
        let current = start.min(items.len().saturating_sub(1));
        UpNext { items, current, shuffled: false, original: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Every item, in play order.
    pub fn items(&self) -> &[PathBuf] {
        &self.items
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn current_path(&self) -> Option<&PathBuf> {
        self.items.get(self.current)
    }

    pub fn shuffled(&self) -> bool {
        self.shuffled
    }

    /// What plays after the current song.
    pub fn upcoming(&self) -> &[PathBuf] {
        self.items.get(self.current + 1..).unwrap_or(&[])
    }

    /// The engine moved to item `index` of the list.
    pub fn set_current(&mut self, index: usize) {
        if index < self.items.len() {
            self.current = index;
        }
    }

    /// Plays `path` right after the current song.
    pub fn play_next(&mut self, path: PathBuf) {
        let at = if self.items.is_empty() { 0 } else { self.current + 1 };
        self.items.insert(at, path.clone());
        if self.shuffled {
            self.original.push(path);
        }
    }

    /// Plays `path` after everything already queued.
    pub fn play_last(&mut self, path: PathBuf) {
        self.items.push(path.clone());
        if self.shuffled {
            self.original.push(path);
        }
    }

    /// Takes the `i`-th upcoming song out of the queue.
    pub fn remove_upcoming(&mut self, i: usize) -> bool {
        let at = self.current + 1 + i;
        if self.items.is_empty() || at >= self.items.len() {
            return false;
        }
        let gone = self.items.remove(at);
        if let Some(p) = self.original.iter().position(|p| *p == gone) {
            self.original.remove(p);
        }
        true
    }

    /// Takes every upcoming song out of the queue; the current song and what has played stay. Returns how many
    /// songs were taken out (0 when nothing was upcoming).
    pub fn clear_upcoming(&mut self) -> usize {
        let from = (self.current + 1).min(self.items.len());
        let gone: Vec<PathBuf> = self.items.drain(from..).collect();
        for path in &gone {
            if let Some(p) = self.original.iter().position(|q| q == path) {
                self.original.remove(p);
            }
        }
        gone.len()
    }

    /// Moves the `from`-th upcoming song to the `to`-th place.
    pub fn move_upcoming(&mut self, from: usize, to: usize) -> bool {
        let n = self.upcoming().len();
        if from >= n || to >= n {
            return false;
        }
        let item = self.items.remove(self.current + 1 + from);
        self.items.insert(self.current + 1 + to, item);
        true
    }

    /// Shuffle on: the upcoming songs take a seeded random order (what has
    /// played and the current song stay). Off: the list order from before.
    pub fn set_shuffle(&mut self, on: bool, seed: u64) {
        if on && !self.shuffled {
            self.original = self.items.clone();
            let from = (self.current + 1).min(self.items.len());
            shuffle_slice(&mut self.items[from..], seed);
            self.shuffled = true;
        } else if !on && self.shuffled {
            let now = self.current_path().cloned();
            let mut list = std::mem::take(&mut self.original);
            if let Some(i) = now.and_then(|n| list.iter().position(|p| *p == n)) {
                self.current = i;
            } else {
                self.current = self.current.min(list.len().saturating_sub(1));
            }
            std::mem::swap(&mut self.items, &mut list);
            self.shuffled = false;
        }
    }

    /// Drops every item `keep` refuses (songs that left the library); the
    /// current song stays current when it is kept, else the next kept one.
    pub fn retain(&mut self, keep: impl Fn(&PathBuf) -> bool) {
        let before = self.items.len();
        let now = self.current;
        let mut kept_before = 0;
        let mut kept = Vec::with_capacity(before);
        for (i, p) in self.items.drain(..).enumerate() {
            if keep(&p) {
                if i < now {
                    kept_before += 1;
                }
                kept.push(p);
            }
        }
        self.items = kept;
        self.original.retain(|p| keep(p));
        self.current = kept_before.min(self.items.len().saturating_sub(1));
    }

    /// The text saved to disk, with where the song was in ms.
    pub fn to_text(&self, position_ms: f64) -> String {
        let mut out = format!("springboard-queue 1\ncurrent {}\nposition {}\nshuffle {}\n", self.current, position_ms.max(0.0) as u64, self.shuffled as u8);
        for p in &self.items {
            out.push_str(&format!("q {}\n", p.display()));
        }
        for p in &self.original {
            out.push_str(&format!("o {}\n", p.display()));
        }
        out
    }

    /// Reads what `to_text` wrote; None for anything else.
    pub fn from_text(text: &str) -> Option<(UpNext, f64)> {
        let mut lines = text.lines();
        if lines.next()? != "springboard-queue 1" {
            return None;
        }
        let mut q = UpNext::default();
        let mut position = 0.0;
        for line in lines {
            if let Some(v) = line.strip_prefix("current ") {
                q.current = v.parse().ok()?;
            } else if let Some(v) = line.strip_prefix("position ") {
                position = v.parse().ok()?;
            } else if let Some(v) = line.strip_prefix("shuffle ") {
                q.shuffled = v == "1";
            } else if let Some(v) = line.strip_prefix("q ") {
                q.items.push(PathBuf::from(v));
            } else if let Some(v) = line.strip_prefix("o ") {
                q.original.push(PathBuf::from(v));
            }
        }
        if q.items.is_empty() || q.current >= q.items.len() {
            return None;
        }
        Some((q, position))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(|n| PathBuf::from(format!("/m/{n}.mp3"))).collect()
    }

    fn names(items: &[PathBuf]) -> Vec<String> {
        items.iter().map(|p| p.file_stem().unwrap().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn upcoming_is_everything_after_the_current_song() {
        let q = UpNext::new(paths(&["a", "b", "c", "d"]), 1);
        assert_eq!(names(q.upcoming()), ["c", "d"]);
        assert_eq!(q.current_path(), Some(&PathBuf::from("/m/b.mp3")));
        assert!(UpNext::default().upcoming().is_empty());
        assert_eq!(UpNext::new(paths(&["a"]), 9).current(), 0, "start is clamped");
    }

    #[test]
    fn play_next_and_last_place_songs() {
        let mut q = UpNext::new(paths(&["a", "b", "c"]), 0);
        q.play_next("/m/x.mp3".into());
        q.play_last("/m/y.mp3".into());
        q.play_next("/m/z.mp3".into());
        assert_eq!(names(q.items()), ["a", "z", "x", "b", "c", "y"]);
        let mut empty = UpNext::default();
        empty.play_next("/m/x.mp3".into());
        assert_eq!((names(empty.items()), empty.current()), (vec!["x".to_string()], 0));
    }

    #[test]
    fn remove_and_reorder_touch_only_upcoming() {
        let mut q = UpNext::new(paths(&["a", "b", "c", "d", "e"]), 1);
        assert!(q.remove_upcoming(0));
        assert_eq!(names(q.items()), ["a", "b", "d", "e"]);
        assert!(!q.remove_upcoming(5));
        assert!(q.move_upcoming(1, 0));
        assert_eq!(names(q.items()), ["a", "b", "e", "d"]);
        assert!(q.move_upcoming(0, 1));
        assert_eq!(names(q.items()), ["a", "b", "d", "e"]);
        assert!(!q.move_upcoming(0, 2));
        assert_eq!(q.current(), 1);
    }

    #[test]
    fn shuffle_keeps_played_and_current_and_off_restores_the_list() {
        let list = paths(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        let mut q = UpNext::new(list.clone(), 2);
        q.set_shuffle(true, 7);
        assert!(q.shuffled());
        assert_eq!(&q.items()[..3], &list[..3]);
        let mut rest = q.upcoming().to_vec();
        rest.sort();
        assert_eq!(rest, list[3..].to_vec(), "same songs");
        assert_ne!(q.upcoming(), &list[3..], "reordered");
        let again = {
            let mut r = UpNext::new(list.clone(), 2);
            r.set_shuffle(true, 7);
            r
        };
        assert_eq!(q, again, "a seed gives one order");
        q.set_current(4);
        let now = q.current_path().cloned();
        q.set_shuffle(false, 0);
        assert_eq!(q.items(), &list[..]);
        assert_eq!(q.current_path(), now.as_ref());
        assert!(!q.shuffled());
    }

    #[test]
    fn songs_added_while_shuffled_survive_turning_shuffle_off() {
        let mut q = UpNext::new(paths(&["a", "b", "c"]), 0);
        q.set_shuffle(true, 3);
        q.play_last("/m/y.mp3".into());
        q.set_shuffle(false, 0);
        assert_eq!(names(q.items()), ["a", "b", "c", "y"]);
    }

    #[test]
    fn retain_keeps_the_current_song_current() {
        let mut q = UpNext::new(paths(&["a", "b", "c", "d"]), 2);
        q.retain(|p| !p.ends_with("a.mp3") && !p.ends_with("d.mp3"));
        assert_eq!((names(q.items()), q.current()), (vec!["b".to_string(), "c".to_string()], 1));
        q.retain(|_| false);
        assert!(q.is_empty());
    }

    #[test]
    fn saves_and_restores_through_text() {
        let mut q = UpNext::new(paths(&["a", "b", "c"]), 1);
        q.set_shuffle(true, 5);
        let (back, pos) = UpNext::from_text(&q.to_text(12_345.9)).unwrap();
        assert_eq!(back, q);
        assert_eq!(pos, 12_345.0);
        assert!(UpNext::from_text("junk").is_none());
        assert!(UpNext::from_text("springboard-queue 1\ncurrent 3\nq /m/a.mp3\n").is_none(), "current out of range");
        assert!(UpNext::from_text("springboard-queue 1\ncurrent 0\n").is_none(), "no songs");
    }

    #[test]
    fn clear_up_next_keeps_the_current_song_and_what_played() {
        let mut q = UpNext::new(paths(&["a", "b", "c", "d", "e"]), 1);
        assert_eq!(q.clear_upcoming(), 3);
        assert_eq!(names(q.items()), ["a", "b"]);
        assert_eq!((q.current(), q.current_path().cloned()), (1, Some(PathBuf::from("/m/b.mp3"))));
        assert!(q.upcoming().is_empty());
        assert_eq!(q.clear_upcoming(), 0, "nothing left to clear");
    }

    #[test]
    fn clear_up_next_with_nothing_upcoming_changes_nothing() {
        let mut empty = UpNext::default();
        assert_eq!(empty.clear_upcoming(), 0);
        let mut last = UpNext::new(paths(&["a", "b"]), 1);
        assert_eq!(last.clear_upcoming(), 0);
        assert_eq!(names(last.items()), ["a", "b"]);
    }

    #[test]
    fn clear_up_next_while_shuffled_restores_the_played_order_when_shuffle_goes_off() {
        let mut q = UpNext::new(paths(&["a", "b", "c", "d", "e"]), 1);
        q.set_shuffle(true, 7);
        assert_eq!(q.clear_upcoming(), 3);
        assert_eq!(names(q.items()), ["a", "b"]);
        q.set_shuffle(false, 0);
        assert_eq!(names(q.items()), ["a", "b"]);
        assert_eq!(q.current(), 1);
    }
}
