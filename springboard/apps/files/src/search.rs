//! Search: the folder-tree query running on its own thread and streaming its hits.
use crate::*;

/// A search of the folder tree under the open folder, streaming its hits.
pub(crate) struct Search {
    pub(crate) rx: Receiver<(Vec<Entry>, bool)>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) query: String,
    /// The `tag:` term: only hits carrying this tag are shown.
    pub(crate) tag: Option<Tag>,
}

/// What a search field asks for: the name words, a kind and a tag. `kind:images tag:red holiday` searches "holiday"
/// among the images tagged Red. A `kind:` or `tag:` with a value nobody knows stays one of the name words.
#[derive(Debug, PartialEq)]
pub(crate) struct Terms {
    pub(crate) words: String,
    pub(crate) kind: TypeFilter,
    pub(crate) tag: Option<Tag>,
}

pub(crate) fn parse_terms(query: &str) -> Terms {
    let mut words = vec![];
    let mut kind = TypeFilter::All;
    let mut tag = None;
    for word in query.split_whitespace() {
        let lower = word.to_lowercase();
        if let Some(value) = lower.strip_prefix("kind:") {
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
    Terms { words: words.join(" "), kind, tag }
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
        let root = self.path.clone();
        let terms = parse_terms(&q);
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
        self.search = Some(Search { rx, cancel, query: q, tag: terms.tag });
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
        let tag = search.tag;
        if let Some(tag) = tag {
            hits.retain(|h| self.tag_index.has(&h.path, tag));
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
        assert_eq!(parse_terms("kind:images tag:red holiday"), Terms { words: "holiday".into(), kind: TypeFilter::Images, tag: Some(Tag::ALL[0]) });
        assert_eq!(parse_terms("Kind:Docs  report 2026").kind, TypeFilter::Documents, "keys and values ignore case");
        assert_eq!(parse_terms("kind:folders"), Terms { words: String::new(), kind: TypeFilter::Folders, tag: None });
    }

    #[test]
    fn unknown_terms_stay_as_name_words() {
        let t = parse_terms("kind:spreadsheet tag:nope plain");
        assert_eq!(t.words, "kind:spreadsheet tag:nope plain");
        assert_eq!(t.kind, TypeFilter::All);
        assert_eq!(t.tag, None);
    }
}
