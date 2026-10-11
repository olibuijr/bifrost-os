//! Bookmarks, history and the open-tab session, persisted as small
//! tab-separated files under the app's data directory (written through a
//! temporary file and a rename, like the other apps' stores), plus the
//! address-bar suggestions drawn from them.

use std::io;
use std::path::{Path, PathBuf};

pub const MAX_HISTORY: usize = 500;

#[derive(Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Visit {
    pub url: String,
    pub title: String,
    /// unix seconds of the last visit
    pub time: i64,
    pub visits: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub title: String,
    pub url: String,
    pub bookmark: bool,
}

#[derive(Debug, Default)]
pub struct Library {
    dir: Option<PathBuf>,
    pub bookmarks: Vec<Bookmark>,
    /// newest first
    pub history: Vec<Visit>,
}

fn clean(s: &str) -> String {
    s.chars().map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c }).collect::<String>().trim().to_string()
}

fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

impl Library {
    /// A library that never touches the disk.
    pub fn memory() -> Library {
        Library::default()
    }

    /// Loads `bookmarks.tsv` and `history.tsv` from `dir` (missing files are empty).
    pub fn load(dir: impl Into<PathBuf>) -> Library {
        let dir = dir.into();
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap_or_default();
        let bookmarks = read("bookmarks.tsv")
            .lines()
            .filter_map(|l| {
                let (title, url) = l.split_once('\t')?;
                Some(Bookmark { title: title.to_string(), url: url.to_string() })
            })
            .collect();
        let history = read("history.tsv")
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.splitn(4, '\t').collect();
                (f.len() == 4).then(|| Visit { time: f[0].parse().unwrap_or(0), visits: f[1].parse().unwrap_or(1), url: f[2].to_string(), title: f[3].to_string() })
            })
            .collect();
        Library { dir: Some(dir), bookmarks, history }
    }

    fn save(&self) {
        let Some(dir) = &self.dir else { return };
        let b: String = self.bookmarks.iter().map(|b| format!("{}\t{}\n", b.title, b.url)).collect();
        let h: String = self.history.iter().map(|v| format!("{}\t{}\t{}\t{}\n", v.time, v.visits, v.url, v.title)).collect();
        for (name, text) in [("bookmarks.tsv", b), ("history.tsv", h)] {
            if let Err(e) = write_atomic(&dir.join(name), &text) {
                eprintln!("springboard: browser: {name}: {e}");
            }
        }
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.bookmarks.iter().any(|b| b.url == url)
    }

    /// Adds or removes the bookmark for `url`; true when it is bookmarked afterwards.
    pub fn toggle_bookmark(&mut self, title: &str, url: &str) -> bool {
        let now = if let Some(i) = self.bookmarks.iter().position(|b| b.url == url) {
            self.bookmarks.remove(i);
            false
        } else {
            let title = clean(title);
            self.bookmarks.insert(0, Bookmark { title: if title.is_empty() { url.to_string() } else { title }, url: clean(url) });
            true
        };
        self.save();
        now
    }

    pub fn remove_bookmark(&mut self, url: &str) {
        self.bookmarks.retain(|b| b.url != url);
        self.save();
    }

    /// Records a page view: a known URL moves to the front with its count
    /// raised, the oldest entries fall off past [`MAX_HISTORY`].
    pub fn visit(&mut self, url: &str, title: &str, time: i64) {
        let url = clean(url);
        if url.is_empty() || url.starts_with("about:") {
            return;
        }
        let (title, visits) = match self.history.iter().position(|v| v.url == url) {
            Some(i) => {
                let old = self.history.remove(i);
                (if clean(title).is_empty() { old.title } else { clean(title) }, old.visits + 1)
            }
            None => (clean(title), 1),
        };
        self.history.insert(0, Visit { url, title, time, visits });
        self.history.truncate(MAX_HISTORY);
        self.save();
    }

    /// Gives the latest visit of `url` its real title once the page has loaded.
    pub fn set_title(&mut self, url: &str, title: &str) {
        let title = clean(title);
        if title.is_empty() {
            return;
        }
        if let Some(v) = self.history.iter_mut().find(|v| v.url == url) {
            v.title = title.clone();
        }
        if let Some(b) = self.bookmarks.iter_mut().find(|b| b.url == url && (b.title == b.url || b.title.is_empty())) {
            b.title = title;
        }
        self.save();
    }

    pub fn remove_history(&mut self, url: &str) {
        self.history.retain(|v| v.url != url);
        self.save();
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
        self.save();
    }

    /// Up to `limit` bookmarks and history entries matching every word of
    /// `query` (empty: bookmarks, then recent pages). Host and prefix
    /// matches rank first, bookmarks and often-visited pages ahead of the rest.
    pub fn suggest(&self, query: &str, limit: usize) -> Vec<Suggestion> {
        let words: Vec<String> = query.split_whitespace().map(str::to_ascii_lowercase).collect();
        let mut hits: Vec<(f32, Suggestion)> = Vec::new();
        let candidates = self.bookmarks.iter().map(|b| (b.title.as_str(), b.url.as_str(), true, 0u32)).chain(self.history.iter().map(|v| (v.title.as_str(), v.url.as_str(), false, v.visits)));
        for (rank, (title, url, bookmark, visits)) in candidates.enumerate() {
            if hits.iter().any(|(_, s)| s.url == url) {
                continue;
            }
            let (t, u) = (title.to_ascii_lowercase(), super::nav::display_url(url).to_ascii_lowercase());
            let mut score = 0.0;
            let mut all_match = true;
            for w in &words {
                let part = if u.starts_with(w.as_str()) {
                    Some(0.0)
                } else if t.starts_with(w.as_str()) || t.contains(&format!(" {w}")) {
                    Some(1.0)
                } else if u.contains(w.as_str()) {
                    Some(2.0)
                } else if t.contains(w.as_str()) {
                    Some(3.0)
                } else {
                    None
                };
                match part {
                    Some(p) => score += p,
                    None => {
                        all_match = false;
                        break;
                    }
                }
            }
            if !all_match {
                continue;
            }
            score -= if bookmark { 0.5 } else { (visits.min(10) as f32) * 0.03 };
            score += rank as f32 * 0.0001;
            hits.push((score, Suggestion { title: title.to_string(), url: url.to_string(), bookmark }));
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        hits.into_iter().take(limit).map(|(_, s)| s).collect()
    }
}

/// The tabs open when the app was closed: URLs and the active index.
pub fn load_session(dir: &Path) -> (Vec<String>, usize) {
    let text = std::fs::read_to_string(dir.join("session.tsv")).unwrap_or_default();
    let mut lines = text.lines();
    let active = lines.next().and_then(|l| l.parse().ok()).unwrap_or(0);
    let urls: Vec<String> = lines.filter(|l| !l.is_empty()).map(str::to_string).collect();
    let active = if active < urls.len() { active } else { 0 };
    (urls, active)
}

pub fn save_session(dir: &Path, urls: &[String], active: usize) {
    let mut text = format!("{active}\n");
    for u in urls {
        text.push_str(&clean(u));
        text.push('\n');
    }
    if let Err(e) = write_atomic(&dir.join("session.tsv"), &text) {
        eprintln!("springboard: browser: session: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-browser-store")).join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn bookmarks_toggle_and_persist() {
        let dir = scratch("bm");
        let mut lib = Library::load(&dir);
        assert!(lib.toggle_bookmark("Example\tSite", "https://example.com/"));
        assert!(lib.is_bookmarked("https://example.com/"));
        let again = Library::load(&dir);
        assert_eq!(again.bookmarks, vec![Bookmark { title: "Example Site".into(), url: "https://example.com/".into() }]);
        assert!(!lib.toggle_bookmark("x", "https://example.com/"));
        assert!(Library::load(&dir).bookmarks.is_empty());
    }

    #[test]
    fn visits_dedupe_count_and_cap() {
        let dir = scratch("hist");
        let mut lib = Library::load(&dir);
        lib.visit("https://a.test/", "A", 10);
        lib.visit("https://b.test/", "B", 20);
        lib.visit("https://a.test/", "", 30);
        assert_eq!(lib.history.iter().map(|v| v.url.as_str()).collect::<Vec<_>>(), ["https://a.test/", "https://b.test/"]);
        assert_eq!((lib.history[0].visits, lib.history[0].time, lib.history[0].title.as_str()), (2, 30, "A"));
        lib.visit("about:home", "Home", 40);
        assert_eq!(lib.history.len(), 2, "internal pages are not history");
        assert_eq!(Library::load(&dir).history, lib.history);
        for i in 0..MAX_HISTORY + 20 {
            lib.visit(&format!("https://h{i}.test/"), "h", i as i64);
        }
        assert_eq!(lib.history.len(), MAX_HISTORY);
        assert!(lib.history[0].url.contains(&format!("h{}", MAX_HISTORY + 19)));
        lib.clear_history();
        assert!(Library::load(&dir).history.is_empty());
    }

    #[test]
    fn titles_fill_in_after_the_page_loads() {
        let mut lib = Library::memory();
        lib.visit("https://a.test/", "", 1);
        lib.toggle_bookmark("", "https://a.test/");
        lib.set_title("https://a.test/", "A Site");
        assert_eq!(lib.history[0].title, "A Site");
        assert_eq!(lib.bookmarks[0].title, "A Site");
    }

    #[test]
    fn suggestions_rank_hosts_then_titles_and_prefer_bookmarks() {
        let mut lib = Library::memory();
        lib.visit("https://news.example.org/story", "Big Story", 1);
        lib.visit("https://example.com/", "Example Domain", 2);
        lib.visit("https://blog.test/post", "Why example code matters", 3);
        lib.toggle_bookmark("Rust docs", "https://doc.rust-lang.org/");
        let urls = |q: &str| lib.suggest(q, 5).into_iter().map(|s| s.url).collect::<Vec<_>>();
        assert_eq!(urls("example")[0], "https://example.com/");
        assert_eq!(urls("example").len(), 3);
        assert_eq!(urls("rust")[0], "https://doc.rust-lang.org/");
        assert_eq!(urls("big story"), ["https://news.example.org/story"]);
        assert!(urls("zzz").is_empty());
        assert_eq!(urls("")[0], "https://doc.rust-lang.org/", "bookmarks lead the empty query");
        assert_eq!(lib.suggest("", 2).len(), 2);
        assert!(lib.suggest("rust", 1)[0].bookmark);
    }

    #[test]
    fn the_session_round_trips() {
        let dir = scratch("session");
        save_session(&dir, &["https://a.test/".into(), "about:home".into(), "https://c.test/".into()], 2);
        assert_eq!(load_session(&dir), (vec!["https://a.test/".to_string(), "about:home".into(), "https://c.test/".into()], 2));
        assert_eq!(load_session(&scratch("none")), (vec![], 0));
        save_session(&dir, &["x".into()], 5);
        assert_eq!(load_session(&dir).1, 0, "a stale index falls back to the first tab");
    }
}
