//! Shared file list: the recent items and favourites an app keeps for its documents, for a recents menu and a
//! sidebar's Favourites section. A pure model with no I/O. Recents are kept most recent first and capped; a
//! favourite is never dropped by the cap. `entries` lists favourites first, then recents.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// A stable key, such as a file path or a document URL. Keys are unique within a list.
    pub key: String,
    pub title: String,
    pub favourite: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedList {
    /// Newest first. Favourites and recents share one ordering; `entries` splits them.
    items: Vec<Item>,
    /// Most recents kept. Favourites do not count against it.
    capacity: usize,
}

impl SharedList {
    /// A list that keeps `capacity` recents (at least one).
    pub fn new(capacity: usize) -> Self {
        SharedList { items: Vec::new(), capacity: capacity.max(1) }
    }

    /// Records a use of `key`: it moves to the front with the new title. An empty key is refused.
    pub fn add(&mut self, key: &str, title: &str) -> bool {
        if key.is_empty() {
            return false;
        }
        match self.items.iter().position(|i| i.key == key) {
            Some(pos) => {
                let mut item = self.items.remove(pos);
                item.title = title.to_string();
                self.items.insert(0, item);
            }
            None => self.items.insert(0, Item { key: key.into(), title: title.into(), favourite: false }),
        }
        self.trim();
        true
    }

    /// Drops the oldest recents until at most `capacity` remain. Favourites are kept.
    fn trim(&mut self) {
        let mut recents = self.items.iter().filter(|i| !i.favourite).count();
        while recents > self.capacity {
            match self.items.iter().rposition(|i| !i.favourite) {
                Some(idx) => {
                    self.items.remove(idx);
                    recents -= 1;
                }
                None => break,
            }
        }
    }

    /// Marks or unmarks a favourite. False when `key` is not in the list.
    pub fn set_favourite(&mut self, key: &str, on: bool) -> bool {
        match self.items.iter_mut().find(|i| i.key == key) {
            Some(item) => {
                item.favourite = on;
                self.trim();
                true
            }
            None => false,
        }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.items.iter().any(|i| i.key == key)
    }

    /// Removes one entry, favourite or not. False when `key` is not in the list.
    pub fn remove(&mut self, key: &str) -> bool {
        match self.items.iter().position(|i| i.key == key) {
            Some(pos) => {
                self.items.remove(pos);
                true
            }
            None => false,
        }
    }

    /// Clears the recents menu; favourites stay.
    pub fn clear_recents(&mut self) {
        self.items.retain(|i| i.favourite);
    }

    /// Every entry, newest first, with favourites in the place they were used. This is the order `from_recency_order`
    /// takes back, so a list survives a save and a load unchanged.
    pub fn in_recency_order(&self) -> &[Item] {
        &self.items
    }

    /// Rebuilds a list from entries in recency order, newest first. An empty key or a key already seen is dropped, and
    /// recents over `capacity` are trimmed as `add` would trim them.
    pub fn from_recency_order(capacity: usize, items: Vec<Item>) -> Self {
        let mut list = SharedList::new(capacity);
        for item in items {
            if !item.key.is_empty() && !list.contains(&item.key) {
                list.items.push(item);
            }
        }
        list.trim();
        list
    }

    /// Favourites in recency order, then recents in recency order.
    pub fn entries(&self) -> Vec<&Item> {
        self.items.iter().filter(|i| i.favourite).chain(self.items.iter().filter(|i| !i.favourite)).collect()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(l: &SharedList) -> Vec<&str> {
        l.entries().into_iter().map(|i| i.key.as_str()).collect()
    }

    #[test]
    fn newest_use_goes_first_and_reuse_does_not_duplicate() {
        let mut l = SharedList::new(5);
        assert!(l.add("a", "A"));
        assert!(l.add("b", "B"));
        assert!(l.add("c", "C"));
        assert_eq!(keys(&l), vec!["c", "b", "a"]);
        assert!(l.add("a", "A2"));
        assert_eq!(keys(&l), vec!["a", "c", "b"]);
        assert_eq!(l.len(), 3);
        assert_eq!(l.entries()[0].title, "A2");
    }

    #[test]
    fn recents_are_capped_and_the_oldest_is_dropped() {
        let mut l = SharedList::new(3);
        for k in ["a", "b", "c", "d"] {
            l.add(k, k);
        }
        // d, c, b, a were added; the cap of 3 drops a.
        assert_eq!(keys(&l), vec!["d", "c", "b"]);
        assert!(!l.contains("a"));
    }

    #[test]
    fn favourites_are_listed_first_and_survive_the_cap() {
        let mut l = SharedList::new(3);
        for k in ["a", "b", "c"] {
            l.add(k, k);
        }
        // Order is c, b, a. Marking a as a favourite keeps it past the cap.
        assert!(l.set_favourite("a", true));
        l.add("d", "d"); // recents d, c, b: three, so nothing is dropped
        assert_eq!(keys(&l), vec!["a", "d", "c", "b"]);
        l.add("e", "e"); // recents e, d, c, b: b is dropped, a stays
        assert_eq!(keys(&l), vec!["a", "e", "d", "c"]);
        l.add("f", "f"); // recents f, e, d, c: c is dropped
        assert_eq!(keys(&l), vec!["a", "f", "e", "d"]);
        assert!(l.contains("a"));
    }

    #[test]
    fn clearing_recents_keeps_favourites() {
        let mut l = SharedList::new(4);
        l.add("x", "X");
        l.add("y", "Y");
        assert!(l.set_favourite("x", true));
        l.clear_recents();
        assert_eq!(keys(&l), vec!["x"]);
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn unknown_keys_and_empty_keys_are_refused() {
        let mut l = SharedList::new(2);
        assert!(!l.add("", "nothing"));
        assert!(l.is_empty());
        assert!(!l.set_favourite("missing", true));
        assert!(!l.remove("missing"));
        l.add("p", "P");
        assert!(l.remove("p"));
        assert!(l.is_empty());
    }

    #[test]
    fn zero_capacity_is_treated_as_one_recent() {
        let mut l = SharedList::new(0);
        l.add("a", "A");
        l.add("b", "B");
        assert_eq!(keys(&l), vec!["b"]);
    }
}
