//! Persistent history and bookmarks under `~/.local/share/bifrost-browser` (tab-separated, one entry per line,
//! written atomically via a temp file and rename).
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{collections::HashMap, fs, io};

/// History is written at most this often (and on exit via `Drop`).
pub const SAVE_INTERVAL: Duration = Duration::from_secs(5);

pub const MAX_HISTORY: usize = 5000;
/// The bundle identifier the history visits are recorded under (the web_history model keys visits by app).
const WEB_BUNDLE: &str = "is.olibuijr.browser.desktop";
/// Longest URL or title kept per history entry, bytes (a page cannot bloat the file with a huge `data:`-like URL).
pub const MAX_FIELD: usize = 2048;

fn clip(s: &str) -> String {
    if s.len() <= MAX_FIELD {
        return s.to_string();
    }
    let mut n = MAX_FIELD;
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    s[..n].to_string()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub url: String,
    pub title: String,
    pub visits: u32,
    /// Seconds since the Unix epoch of the last visit (or of creation for bookmarks).
    pub last: u64,
}

#[derive(Default)]
pub struct Store {
    pub history: Vec<Entry>,
    pub bookmarks: Vec<Entry>,
    dir: Option<PathBuf>,
    /// Private browsing records nothing (reserved for the private-window item).
    pub private: bool,
    /// url -> position in `history` (O(1) visit lookup); rebuilt whenever the order changes.
    index: HashMap<String, usize>,
    dirty: bool,
    last_save: Option<Instant>,
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n").replace('\r', "")
}

fn unesc(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn parse(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let url = unesc(f.next()?);
            if url.is_empty() {
                return None;
            }
            let title = unesc(f.next().unwrap_or(""));
            let visits = f.next().and_then(|v| v.parse().ok()).unwrap_or(1);
            let last = f.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            Some(Entry { url, title, visits, last })
        })
        .collect()
}

pub fn serialize(entries: &[Entry]) -> String {
    entries.iter().map(|e| format!("{}\t{}\t{}\t{}\n", esc(&e.url), esc(&e.title), e.visits, e.last)).collect()
}

fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(d) = path.parent() {
        fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Store {
    pub fn open(dir: PathBuf) -> Store {
        let read = |n: &str| fs::read_to_string(dir.join(n)).map(|t| parse(&t)).unwrap_or_default();
        let mut st = Store::default();
        st.history = read("history.tsv");
        st.bookmarks = read("bookmarks.tsv");
        st.dir = Some(dir);
        st.reindex();
        st
    }

    pub fn in_memory() -> Store {
        Store::default()
    }

    fn reindex(&mut self) {
        self.index = self.history.iter().enumerate().map(|(i, e)| (e.url.clone(), i)).collect();
    }

    /// Writes pending changes now (atomic). Called on exit and by `tick`.
    pub fn flush(&mut self) {
        if self.dirty {
            self.save();
        }
    }

    /// Debounce hook: writes pending history when `SAVE_INTERVAL` has passed since the last write.
    pub fn tick(&mut self) {
        if self.dirty && self.last_save.is_none_or(|t| t.elapsed() >= SAVE_INTERVAL) {
            self.save();
        }
    }

    fn save(&mut self) {
        self.dirty = false;
        self.last_save = Some(Instant::now());
        if let Some(d) = &self.dir {
            let _ = write_atomic(&d.join("history.tsv"), &serialize(&self.history));
            let _ = write_atomic(&d.join("bookmarks.tsv"), &serialize(&self.bookmarks));
        }
    }

    /// Record a finished load. Blank pages and non-web URLs are skipped.
    pub fn visit(&mut self, url: &str, title: &str, at: u64) {
        if self.private || !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("file://")) {
            return;
        }
        if url.len() > MAX_FIELD {
            return;
        }
        let title = clip(title);
        let title = title.as_str();
        match self.index.get(url).copied().and_then(|i| self.history.get_mut(i)) {
            Some(e) => {
                e.visits += 1;
                e.last = at;
                if !title.is_empty() {
                    e.title = title.to_string();
                }
            }
            None => {
                self.index.insert(url.to_string(), self.history.len());
                self.history.push(Entry { url: url.into(), title: title.into(), visits: 1, last: at });
                if self.history.len() > MAX_HISTORY {
                    self.history.sort_by(|a, b| b.last.cmp(&a.last));
                    self.history.truncate(MAX_HISTORY);
                    self.reindex();
                }
            }
        }
        if let Some(b) = self.bookmarks.iter_mut().find(|b| b.url == url) {
            if !title.is_empty() {
                b.title = title.to_string();
            }
        }
        self.dirty = true;
        self.tick();
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.bookmarks.iter().any(|b| b.url == url)
    }

    /// Add or remove the bookmark; returns whether it is bookmarked afterwards.
    pub fn toggle_bookmark(&mut self, url: &str, title: &str, at: u64) -> bool {
        if url.is_empty() || url == "about:blank" {
            return false;
        }
        let on = if let Some(i) = self.bookmarks.iter().position(|b| b.url == url) {
            self.bookmarks.remove(i);
            false
        } else {
            self.bookmarks.push(Entry { url: url.into(), title: title.into(), visits: 0, last: at });
            true
        };
        self.save();
        on
    }

    /// Bookmark at `url`, if any.
    pub fn bookmark(&self, url: &str) -> Option<&Entry> {
        self.bookmarks.iter().find(|b| b.url == url)
    }

    /// Adds `url` as a bookmark unless it is already one (or not bookmarkable); true when it is bookmarked afterwards.
    pub fn add_bookmark(&mut self, url: &str, title: &str, at: u64) -> bool {
        if url.is_empty() || url == "about:blank" {
            return false;
        }
        if !self.is_bookmarked(url) {
            self.bookmarks.push(Entry { url: clip(url), title: clip(title), visits: 0, last: at });
            self.save();
        }
        true
    }

    /// Renames the bookmark at `url`; false when there is none.
    pub fn rename_bookmark(&mut self, url: &str, title: &str) -> bool {
        let Some(b) = self.bookmarks.iter_mut().find(|b| b.url == url) else { return false };
        b.title = clip(title);
        self.save();
        true
    }

    /// Removes the bookmark at `url`; false when there is none.
    pub fn remove_bookmark(&mut self, url: &str) -> bool {
        let Some(i) = self.bookmarks.iter().position(|b| b.url == url) else { return false };
        self.bookmarks.remove(i);
        self.save();
        true
    }

    /// Removes one history entry; false when there is none.
    pub fn remove_history(&mut self, url: &str) -> bool {
        let Some(i) = self.history.iter().position(|h| h.url == url) else { return false };
        self.history.remove(i);
        self.reindex();
        self.save();
        true
    }

    /// Adds the links that are not bookmarks yet (blank and about:blank are skipped), with one save. Returns how many
    /// were added; a link already bookmarked keeps its name and position.
    pub fn import_bookmarks(&mut self, items: &[(String, String)], at: u64) -> usize {
        let mut added = 0;
        for (url, title) in items {
            if url.is_empty() || url == "about:blank" || self.is_bookmarked(url) {
                continue;
            }
            self.bookmarks.push(Entry { url: clip(url), title: clip(title), visits: 0, last: at });
            added += 1;
        }
        if added > 0 {
            self.save();
        }
        added
    }

    /// Removes the history entries whose last visit is in `from..to` (half-open, seconds since the epoch). Each entry
    /// stands for its last visit, so an entry is removed whole. Returns how many were removed.
    pub fn clear_history_during(&mut self, from: u64, to: u64) -> usize {
        let mut visits = bifrost_app::controls::web_history::WebHistory::new();
        for e in &self.history {
            visits.record(WEB_BUNDLE, &e.url, e.last);
        }
        visits.delete_during(from, to);
        let kept: std::collections::HashSet<String> = visits.fetch_all().into_iter().map(|v| v.url).collect();
        let before = self.history.len();
        // an entry without a URL has no visit to match, so it is kept
        self.history.retain(|e| e.url.is_empty() || kept.contains(&e.url));
        let removed = before - self.history.len();
        if removed > 0 {
            self.reindex();
            self.save();
        }
        removed
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
        self.index.clear();
        self.save();
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        self.flush();
    }
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("bifrost-browser")
}

#[cfg(test)]
mod bookmark_tests {
    use super::*;

    #[test]
    fn add_rename_remove() {
        let mut st = Store::in_memory();
        assert!(st.add_bookmark("https://a.test/", "A", 1));
        assert!(st.add_bookmark("https://a.test/", "again", 2));
        assert_eq!(st.bookmarks.len(), 1);
        assert!(st.rename_bookmark("https://a.test/", "Renamed"));
        assert_eq!(st.bookmark("https://a.test/").unwrap().title, "Renamed");
        assert!(st.remove_bookmark("https://a.test/"));
        assert!(!st.remove_bookmark("https://a.test/"));
        assert!(!st.add_bookmark("about:blank", "", 1));
    }

    #[test]
    fn remove_history_reindexes() {
        let mut st = Store::in_memory();
        st.visit("https://a.test/", "A", 1);
        st.visit("https://b.test/", "B", 2);
        assert!(st.remove_history("https://a.test/"));
        st.visit("https://b.test/", "B", 3);
        assert_eq!(st.history.len(), 1);
        assert_eq!(st.history[0].visits, 2);
    }
}

#[cfg(test)]
mod import_tests {
    use super::*;

    #[test]
    fn import_adds_new_links_and_keeps_existing_ones() {
        let mut s = Store::in_memory();
        s.add_bookmark("https://a.test/", "A", 1);
        let items = vec![
            ("https://a.test/".to_string(), "A renamed".to_string()),
            ("https://b.test/".to_string(), "B".to_string()),
            (String::new(), "no address".to_string()),
            ("about:blank".to_string(), "blank".to_string()),
        ];
        assert_eq!(s.import_bookmarks(&items, 5), 1);
        assert_eq!(s.bookmarks.len(), 2);
        assert_eq!(s.bookmark("https://a.test/").map(|b| b.title.as_str()), Some("A"));
        assert_eq!(s.bookmark("https://b.test/").map(|b| b.last), Some(5));
    }

    #[test]
    fn clear_during_removes_only_the_range() {
        let mut s = Store::in_memory();
        s.visit("https://a.test/", "A", 100);
        s.visit("https://b.test/", "B", 200);
        s.visit("https://c.test/", "C", 300);
        // 200 is inside 200..300 (half-open), 300 is not
        assert_eq!(s.clear_history_during(200, 300), 1);
        let urls: Vec<&str> = s.history.iter().map(|e| e.url.as_str()).collect();
        assert_eq!(urls, vec!["https://a.test/", "https://c.test/"]);
        assert_eq!(s.clear_history_during(0, 1), 0);
        assert_eq!(s.history.len(), 2);
    }
}
