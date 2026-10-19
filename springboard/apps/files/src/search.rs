//! Search: the folder-tree query running on its own thread and streaming its hits.
use crate::*;

/// A search of the folder tree under the open folder, streaming its hits.
pub(crate) struct Search {
    pub(crate) rx: Receiver<(Vec<Entry>, bool)>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) query: String,
    /// The `tag:` term: only hits carrying this tag are shown.
    pub(crate) tag: Option<Tag>,
    /// The `date:` term: only hits changed in that span.
    pub(crate) date: Option<DateTerm>,
}

/// What a search field asks for: the name words, a kind, a tag, a date span and a start folder. `kind:images tag:red
/// date:week in:home holiday` searches "holiday" among the images tagged Red, changed this week, from the home folder.
/// A `kind:`, `tag:`, `date:` or `in:` with a value nobody knows stays one of the name words.
#[derive(Debug, PartialEq)]
pub(crate) struct Terms {
    pub(crate) words: String,
    pub(crate) kind: TypeFilter,
    pub(crate) tag: Option<Tag>,
    /// The `date:` term: only items changed in that span.
    pub(crate) date: Option<DateTerm>,
    /// The `in:` term: where the search starts.
    pub(crate) scope: Scope,
}

/// What a `date:` term names (`date:today`, `date:yesterday`, `date:week`, `date:month`, `date:year`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DateTerm {
    Today,
    Yesterday,
    Week,
    Month,
    Year,
}

impl DateTerm {
    fn named(value: &str) -> Option<DateTerm> {
        Some(match value {
            "today" => DateTerm::Today,
            "yesterday" => DateTerm::Yesterday,
            "week" | "7days" => DateTerm::Week,
            "month" | "30days" => DateTerm::Month,
            "year" => DateTerm::Year,
            _ => return None,
        })
    }
    /// True when an item modified at `mtime` (Unix seconds) falls in the span, counting whole days up to `now`.
    /// An item without a date matches nothing; one from the future counts as today.
    pub(crate) fn matches(self, mtime: i64, now: i64) -> bool {
        if mtime <= 0 {
            return false;
        }
        let age = now.div_euclid(86400) - mtime.div_euclid(86400);
        match self {
            DateTerm::Today => age <= 0,
            DateTerm::Yesterday => age == 1,
            DateTerm::Week => age <= 6,
            DateTerm::Month => age <= 29,
            DateTerm::Year => age <= 364,
        }
    }
}

/// Where a search starts: the open folder (the default), the home folder or the whole disk (`in:home`, `in:disk`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    Folder,
    Home,
    Disk,
}

impl Scope {
    fn named(value: &str) -> Option<Scope> {
        Some(match value {
            "folder" | "here" | "this" => Scope::Folder,
            "home" => Scope::Home,
            "disk" | "computer" | "everywhere" => Scope::Disk,
            _ => return None,
        })
    }
    /// The folder the search walks.
    pub(crate) fn root(self, open: &Path, home: &Path) -> PathBuf {
        match self {
            Scope::Folder => open.to_path_buf(),
            Scope::Home => home.to_path_buf(),
            Scope::Disk => PathBuf::from("/"),
        }
    }
}

pub(crate) fn parse_terms(query: &str) -> Terms {
    let mut words = vec![];
    let mut kind = TypeFilter::All;
    let mut tag = None;
    let mut date = None;
    let mut scope = Scope::Folder;
    for word in query.split_whitespace() {
        let lower = word.to_lowercase();
        if let Some(value) = lower.strip_prefix("date:") {
            if let Some(d) = DateTerm::named(value) {
                date = Some(d);
                continue;
            }
        } else if let Some(value) = lower.strip_prefix("in:") {
            if let Some(s) = Scope::named(value) {
                scope = s;
                continue;
            }
        } else if let Some(value) = lower.strip_prefix("kind:") {
            if let Some(k) = kind_named(value) {
                kind = k;
                continue;
            }
        } else if let Some(value) = lower.strip_prefix("tag:") {
            if let Some(t) = Tag::ALL.into_iter().find(|t| t.label().to_lowercase() == value) {
                tag = Some(t);
                continue;
            }
        }
        words.push(word);
    }
    Terms { words: words.join(" "), kind, tag, date, scope }
}

/// The type filter a `kind:` value names (lower case), or None.
fn kind_named(value: &str) -> Option<TypeFilter> {
    Some(match value {
        "folder" | "folders" => TypeFilter::Folders,
        "document" | "documents" | "doc" | "docs" => TypeFilter::Documents,
        "image" | "images" | "picture" | "pictures" => TypeFilter::Images,
        "video" | "videos" => TypeFilter::Video,
        "audio" | "music" | "song" | "songs" => TypeFilter::Audio,
        "archive" | "archives" => TypeFilter::Archives,
        _ => return None,
    })
}

impl Files {
    pub(crate) fn stop_search(&mut self) {
        if let Some(search) = self.search.take() {
            search.cancel.store(true, Ordering::Relaxed);
        }
        self.hits = None;
    }
    /// The typed query changed: search the folder tree for it, or show the folder again.
    pub(crate) fn query_changed(&mut self) {
        self.stop_search();
        self.scroll = 0.;
        self.selection.clear();
        let q = self.query.trim().to_string();
        if q.is_empty() || self.in_recents() || self.in_bin() {
            self.refresh_rows();
            return;
        }
        let (tx, rx) = feed();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let terms = parse_terms(&q);
        let root = terms.scope.root(&self.path, &self.home);
        let (search_for, kind) = (terms.words.clone(), terms.kind);
        DispatchQueue::global(Qos::Utility).async_(move || {
            let mut file_manager = FileManager::new(&root, &search_for, kind);
            let mut out = vec![];
            while !file_manager.done() && !flag.load(Ordering::Relaxed) {
                out.extend(file_manager.step(16));
                let _ = tx.send((out.clone(), false));
            }
            let _ = tx.send((out, true));
        });
        self.search = Some(Search { rx, cancel, query: q, tag: terms.tag, date: terms.date });
        self.revision += 1;
    }
    /// Takes the newest hits from the search thread. Returns true when the screen changed.
    pub(crate) fn poll_search(&mut self) -> bool {
        let Some(search) = self.search.as_ref() else { return false };
        let mut latest = None;
        let mut done = false;
        while let Ok((hits, finished)) = search.rx.try_recv() {
            latest = Some(hits);
            done = finished;
            if finished {
                break;
            }
        }
        let Some(mut hits) = latest else { return false };
        let for_query = search.query.clone();
        if for_query != self.query.trim() {
            return false;
        }
        let (tag, date) = (search.tag, search.date);
        if let Some(tag) = tag {
            hits.retain(|h| self.tag_index.has(&h.path, tag));
        }
        if let Some(date) = date {
            let now = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
            hits.retain(|h| date.matches(h.mtime, now));
        }
        self.order(&mut hits);
        self.hits = Some(hits);
        self.refresh_rows();
        if done {
            self.search = None;
        }
        self.revision += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::files::{find::TypeFilter, tags::Tag};

    #[test]
    fn kind_and_tag_terms_are_taken_out_of_the_name_words() {
        assert_eq!(parse_terms("kind:images tag:red holiday"), Terms { words: "holiday".into(), kind: TypeFilter::Images, tag: Some(Tag::ALL[0]), date: None, scope: Scope::Folder });
        assert_eq!(parse_terms("Kind:Docs  report 2026").kind, TypeFilter::Documents, "keys and values ignore case");
        assert_eq!(parse_terms("kind:folders"), Terms { words: String::new(), kind: TypeFilter::Folders, tag: None, date: None, scope: Scope::Folder });
    }

    #[test]
    fn unknown_terms_stay_as_name_words() {
        let t = parse_terms("kind:spreadsheet tag:nope plain");
        assert_eq!(t.words, "kind:spreadsheet tag:nope plain");
        assert_eq!(t.kind, TypeFilter::All);
        assert_eq!(t.tag, None);
        let t = parse_terms("date:someday in:mars plain");
        assert_eq!(t.words, "date:someday in:mars plain");
        assert_eq!((t.date, t.scope), (None, Scope::Folder));
    }

    #[test]
    fn date_and_scope_terms_are_taken_out_of_the_name_words() {
        let t = parse_terms("Date:Week in:home report");
        assert_eq!((t.words.as_str(), t.date, t.scope), ("report", Some(DateTerm::Week), Scope::Home));
        assert_eq!(parse_terms("in:disk").scope, Scope::Disk);
        assert_eq!(parse_terms("date:yesterday").date, Some(DateTerm::Yesterday));
    }

    #[test]
    fn date_spans_count_whole_days() {
        let now = 100 * 86400 + 3600; // 01:00 on day 100
        let day = |d: i64| d * 86400 + 80000;
        assert!(DateTerm::Today.matches(day(100), now));
        assert!(DateTerm::Today.matches(day(101), now), "the future counts as today");
        assert!(!DateTerm::Today.matches(day(99), now));
        assert!(DateTerm::Yesterday.matches(day(99), now) && !DateTerm::Yesterday.matches(day(98), now));
        assert!(DateTerm::Week.matches(day(94), now) && !DateTerm::Week.matches(day(93), now));
        assert!(DateTerm::Month.matches(day(71), now) && !DateTerm::Month.matches(day(70), now));
        let year = 400 * 86400 + 3600;
        assert!(DateTerm::Year.matches(day(36), year) && !DateTerm::Year.matches(day(35), year));
        assert!(!DateTerm::Year.matches(0, now), "no date matches nothing");
    }

    #[test]
    fn scopes_pick_the_start_folder() {
        let (open, home) = (Path::new("/a/b"), Path::new("/h"));
        assert_eq!(Scope::Folder.root(open, home), PathBuf::from("/a/b"));
        assert_eq!(Scope::Home.root(open, home), PathBuf::from("/h"));
        assert_eq!(Scope::Disk.root(open, home), PathBuf::from("/"));
    }
}
