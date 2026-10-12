//! The books on the shelf as search index items: the title and author of each, so the desktop search panel finds a
//! book by its words and opens it here. The index follows the shelf: a book that is new, or whose title or author
//! changed, is sent once, and a book that left the shelf is removed. Nothing is sent when nothing changed. A book the
//! shell refuses (an identifier over the wire cap) is simply not indexed.

use bifrost_app::quicksearch::{CSSearchableIndex, CSSearchableItem, CSSearchableItemAttributeSet, UserActivity, ACTION_TYPE};
use bifrost_app::Ctx;
use std::collections::HashMap;
use std::path::PathBuf;

/// The search domain of every book; a search result is opened here only when it carries this domain.
pub const DOMAIN: &str = "books";

/// One book on the shelf, as the index sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub title: String,
    pub author: String,
    /// "PDF" or "EPUB", the kind of the book's file
    pub format: &'static str,
}

/// The items the index holds: the title and author last sent for each book, by path.
#[derive(Default)]
pub struct SearchIndex {
    sent: HashMap<String, (String, String)>,
}

impl SearchIndex {
    /// Brings the index in step with `shelf`: sends the books that are new or changed, and removes the ones that left.
    pub fn sync(&mut self, ctx: &Ctx, shelf: &[Entry]) {
        let mut changed = Vec::new();
        let mut current = HashMap::new();
        for e in shelf {
            let id = e.path.to_string_lossy().into_owned();
            let key = (e.title.clone(), e.author.clone());
            if self.sent.get(&id) != Some(&key) {
                changed.push(item(&id, e));
            }
            current.insert(id, key);
        }
        let gone: Vec<&str> = self.sent.keys().filter(|id| !current.contains_key(*id)).map(String::as_str).collect();
        let index = CSSearchableIndex::default_index();
        if !changed.is_empty() {
            index.index_searchable_items(ctx, &changed);
        }
        if !gone.is_empty() {
            index.delete_searchable_items(ctx, &gone);
        }
        self.sent = current;
    }
}

/// The index item of one book: its title, its author as a keyword, and its kind as the description.
fn item(id: &str, e: &Entry) -> CSSearchableItem {
    let mut set = CSSearchableItemAttributeSet::new(&e.title).content_description(&format!("{} book", e.format));
    if !e.author.trim().is_empty() {
        set = set.keywords(&[e.author.as_str()]);
    }
    CSSearchableItem::new(id, DOMAIN, set)
}

/// The book a search result opens: the item's identifier is the book's path, in this app's domain.
pub fn path_from_activity(activity: &UserActivity) -> Option<PathBuf> {
    let ours = activity.activity_type == ACTION_TYPE && activity.domain_identifier == DOMAIN;
    (ours && !activity.unique_identifier.is_empty()).then(|| PathBuf::from(&activity.unique_identifier))
}
