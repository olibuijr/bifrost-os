//! A media picker: browse a library by artist, album, genre or playlist, search, multi-select items, and hand the
//! chosen items to a player as a queue; plus external player items an app publishes to the system (identifier,
//! title, a stream URL and artwork). Pure std over simple item records.

use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub seconds: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Artist,
    Album,
    Genre,
}

/// The distinct values of a grouping with their item counts, sorted without case.
pub fn groups(items: &[Item], by: Group) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = vec![];
    for it in items {
        let key = match by {
            Group::Artist => &it.artist,
            Group::Album => &it.album,
            Group::Genre => &it.genre,
        };
        if key.is_empty() {
            continue;
        }
        match out.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            Some((_, n)) => *n += 1,
            None => out.push((key.clone(), 1)),
        }
    }
    out.sort_by_key(|(k, _)| k.to_lowercase());
    out
}

/// The items whose title, artist or album contains every word of `query`.
pub fn search<'a>(items: &'a [Item], query: &str) -> Vec<&'a Item> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    items
        .iter()
        .filter(|i| {
            let hay = format!("{} {} {}", i.title, i.artist, i.album).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

/// The picker's selection: items in the order they were picked, optionally limited in size.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    ids: Vec<u64>,
    pub limit: Option<usize>,
}

impl Selection {
    pub fn with_limit(limit: usize) -> Selection {
        Selection { ids: vec![], limit: Some(limit) }
    }

    /// Picks or unpicks an item; false when the limit blocks a new pick.
    pub fn toggle(&mut self, id: u64) -> bool {
        if let Some(i) = self.ids.iter().position(|x| *x == id) {
            self.ids.remove(i);
            return true;
        }
        if self.limit.is_some_and(|l| self.ids.len() >= l) {
            return false;
        }
        self.ids.push(id);
        true
    }

    pub fn contains(&self, id: u64) -> bool {
        self.ids.contains(&id)
    }

    pub fn ids(&self) -> &[u64] {
        &self.ids
    }

    /// The chosen items as a queue, in pick order; ids missing from the library are skipped.
    pub fn queue<'a>(&self, items: &'a [Item]) -> Vec<&'a Item> {
        self.ids.iter().filter_map(|id| items.iter().find(|i| i.id == *id)).collect()
    }

    /// Total length of the chosen items in seconds.
    pub fn total_seconds(&self, items: &[Item]) -> u32 {
        self.queue(items).iter().map(|i| i.seconds).sum()
    }
}

/// An item an app publishes to the system so it appears in the picker and Now Playing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalItem {
    pub app_id: String,
    pub external_id: String,
    pub title: String,
    pub stream_url: String,
}

/// Registry of external items; an app's item with the same external id is replaced.
#[derive(Clone, Debug, Default)]
pub struct ExternalItems {
    items: Vec<ExternalItem>,
}

impl ExternalItems {
    pub fn publish(&mut self, item: ExternalItem) -> bool {
        if item.app_id.is_empty() || item.external_id.is_empty() || !(item.stream_url.starts_with("https://") || item.stream_url.starts_with("http://")) {
            return false;
        }
        self.items.retain(|i| !(i.app_id == item.app_id && i.external_id == item.external_id));
        self.items.push(item);
        true
    }

    pub fn withdraw_app(&mut self, app_id: &str) {
        self.items.retain(|i| i.app_id != app_id);
    }

    pub fn for_app(&self, app_id: &str) -> Vec<&ExternalItem> {
        self.items.iter().filter(|i| i.app_id == app_id).collect()
    }

    pub fn all(&self) -> &[ExternalItem] {
        &self.items
    }
}

/// Distinct apps that published items.
pub fn publishers(items: &ExternalItems) -> BTreeSet<&str> {
    items.all().iter().map(|i| i.app_id.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u64, title: &str, artist: &str, album: &str, genre: &str, secs: u32) -> Item {
        Item { id, title: title.into(), artist: artist.into(), album: album.into(), genre: genre.into(), seconds: secs }
    }

    fn library() -> Vec<Item> {
        vec![
            item(1, "Alpha", "Ann", "First", "Rock", 100),
            item(2, "Beta", "ann", "First", "Rock", 200),
            item(3, "Gamma", "Bob", "Second", "Jazz", 300),
            item(4, "Delta", "", "", "", 50),
        ]
    }

    #[test]
    fn groups_merge_case_and_skip_empty() {
        let g = groups(&library(), Group::Artist);
        assert_eq!(g, vec![("Ann".to_string(), 2), ("Bob".to_string(), 1)]);
        assert_eq!(groups(&library(), Group::Genre), vec![("Jazz".to_string(), 1), ("Rock".to_string(), 2)]);
    }

    #[test]
    fn search_needs_every_word() {
        let lib = library();
        assert_eq!(search(&lib, "ann first").len(), 2);
        assert_eq!(search(&lib, "beta ann")[0].id, 2);
        assert!(search(&lib, "zzz").is_empty());
        assert_eq!(search(&lib, "").len(), 4);
    }

    #[test]
    fn selection_keeps_pick_order_and_limit() {
        let lib = library();
        let mut s = Selection::with_limit(2);
        assert!(s.toggle(3));
        assert!(s.toggle(1));
        assert!(!s.toggle(2)); // limit
        assert!(s.toggle(3)); // unpick
        assert!(s.toggle(2));
        assert_eq!(s.ids(), [1, 2]);
        assert_eq!(s.queue(&lib).iter().map(|i| i.id).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(s.total_seconds(&lib), 300);
        assert!(s.contains(1) && !s.contains(3));
        s.toggle(1);
        s.limit = None;
        s.toggle(99);
        assert_eq!(s.queue(&lib).len(), 1);
    }

    #[test]
    fn external_items_are_replaced_and_withdrawn() {
        let mut e = ExternalItems::default();
        let it = |ext: &str, url: &str| ExternalItem { app_id: "radio".into(), external_id: ext.into(), title: "T".into(), stream_url: url.into() };
        assert!(e.publish(it("a", "https://x.is/a")));
        assert!(e.publish(it("a", "https://x.is/b")));
        assert!(!e.publish(it("c", "file:///etc/passwd")));
        assert!(!e.publish(it("", "https://x.is")));
        assert_eq!(e.all().len(), 1);
        assert_eq!(e.for_app("radio")[0].stream_url, "https://x.is/b");
        assert_eq!(publishers(&e).len(), 1);
        e.withdraw_app("radio");
        assert!(e.all().is_empty());
    }
}
