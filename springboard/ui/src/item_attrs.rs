//! Per-item attributes that a file provider can set for an item and the Files app can read back: opaque tag data and
//! the last-used time, both keyed by path. Pure model: the store keeps values in memory only, and the host decides
//! whether to persist them. Times are whole seconds since the Unix epoch, so the model needs no clock.
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Attrs {
    tag_data: Option<Vec<u8>>,
    last_used: Option<u64>,
}

/// The attributes of many items, keyed by path. An item with no attributes holds no entry.
#[derive(Clone, Debug, Default)]
pub struct ItemStore {
    items: BTreeMap<PathBuf, Attrs>,
}

impl ItemStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the tag data of an item. `None` clears it. The item keeps no entry once it has neither tag data nor a
    /// last-used time.
    pub fn set_tag_data(&mut self, path: &Path, data: Option<Vec<u8>>) {
        self.update(path, |a| a.tag_data = data);
    }

    pub fn tag_data(&self, path: &Path) -> Option<&[u8]> {
        self.items.get(path).and_then(|a| a.tag_data.as_deref())
    }

    /// Sets the last-used time of an item in seconds since the Unix epoch. `None` clears it.
    pub fn set_last_used(&mut self, path: &Path, when: Option<u64>) {
        self.update(path, |a| a.last_used = when);
    }

    pub fn last_used(&self, path: &Path) -> Option<u64> {
        self.items.get(path).and_then(|a| a.last_used)
    }

    /// Up to `limit` items that have a last-used time, most recent first. Items with the same time sort by path.
    pub fn most_recent(&self, limit: usize) -> Vec<&Path> {
        let mut used: Vec<(u64, &Path)> = self
            .items
            .iter()
            .filter_map(|(p, a)| a.last_used.map(|t| (t, p.as_path())))
            .collect();
        used.sort_by_key(|&(t, p)| (Reverse(t), p));
        used.into_iter().take(limit).map(|(_, p)| p).collect()
    }

    /// Drops every attribute of an item, for example after it was deleted or moved away.
    pub fn forget(&mut self, path: &Path) {
        self.items.remove(path);
    }

    fn update(&mut self, path: &Path, change: impl FnOnce(&mut Attrs)) {
        let entry = self.items.entry(path.to_path_buf()).or_default();
        change(entry);
        if *entry == Attrs::default() {
            self.items.remove(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_data_is_set_read_and_cleared() {
        let mut s = ItemStore::new();
        let p = Path::new("/home/u/a.txt");
        s.set_tag_data(p, Some(vec![1, 2, 3]));
        assert_eq!(s.tag_data(p), Some(&[1u8, 2, 3][..]));
        s.set_tag_data(p, None);
        assert_eq!(s.tag_data(p), None);
        assert!(s.items.is_empty());
    }

    #[test]
    fn last_used_is_set_and_cleared() {
        let mut s = ItemStore::new();
        let p = Path::new("/home/u/a.txt");
        s.set_last_used(p, Some(1_700_000_000));
        assert_eq!(s.last_used(p), Some(1_700_000_000));
        s.set_last_used(p, None);
        assert_eq!(s.last_used(p), None);
        assert!(s.items.is_empty());
    }

    #[test]
    fn tag_data_and_last_used_are_independent() {
        let mut s = ItemStore::new();
        let p = Path::new("/home/u/a.txt");
        s.set_tag_data(p, Some(vec![9]));
        s.set_last_used(p, Some(5));
        s.set_tag_data(p, None);
        assert_eq!(s.last_used(p), Some(5));
        assert_eq!(s.tag_data(p), None);
        assert_eq!(s.items.len(), 1);
    }

    #[test]
    fn most_recent_orders_by_time_then_path() {
        let mut s = ItemStore::new();
        s.set_last_used(Path::new("/d/a"), Some(10));
        s.set_last_used(Path::new("/d/b"), Some(30));
        s.set_last_used(Path::new("/d/c"), Some(20));
        s.set_last_used(Path::new("/d/z"), Some(20));
        let all: Vec<&Path> = s.most_recent(10);
        assert_eq!(all, vec![Path::new("/d/b"), Path::new("/d/c"), Path::new("/d/z"), Path::new("/d/a")]);
        assert_eq!(s.most_recent(2), vec![Path::new("/d/b"), Path::new("/d/c")]);
        assert!(s.most_recent(0).is_empty());
    }

    #[test]
    fn most_recent_skips_items_without_a_last_used_time() {
        let mut s = ItemStore::new();
        s.set_tag_data(Path::new("/d/tagged"), Some(vec![1]));
        s.set_last_used(Path::new("/d/used"), Some(1));
        assert_eq!(s.most_recent(5), vec![Path::new("/d/used")]);
    }

    #[test]
    fn forget_drops_every_attribute() {
        let mut s = ItemStore::new();
        let p = Path::new("/home/u/moved.txt");
        s.set_tag_data(p, Some(vec![7]));
        s.set_last_used(p, Some(42));
        s.forget(p);
        assert_eq!(s.tag_data(p), None);
        assert_eq!(s.last_used(p), None);
        assert!(s.items.is_empty());
    }
}
