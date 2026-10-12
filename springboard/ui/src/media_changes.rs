//! Change records for a media library: which items were inserted, removed or changed since the last snapshot, and when
//! the library last changed. An item is identified by its identifier. A changed item keeps its identifier and gets a
//! new revision number. Notifications are delivered only while they are switched on, and the model still records the
//! snapshot and the modification time while they are off, so switching them on later does not replay old changes.
//! Pure: the app fetches the stamps, passes them in with the current time and keeps the returned change for itself.
//! Gaps: no library query (the stamps come from the caller), no authorization, and no item-level property diff.

use std::collections::HashMap;

/// One item in a fetched listing: its identifier and a revision that changes when its content changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub id: String,
    pub revision: u64,
}

/// What changed between two snapshots. Every list is sorted ascending by identifier.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LibraryChange {
    /// Identifiers that are in the new snapshot only.
    pub inserted: Vec<String>,
    /// Identifiers that were in the old snapshot only.
    pub removed: Vec<String>,
    /// Identifiers in both snapshots whose revision moved.
    pub changed: Vec<String>,
}

impl LibraryChange {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.inserted.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// The change tracker for one library: the notification switch, the last snapshot and the last modification time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaLibrary {
    generating: bool,
    loaded: bool,
    snapshot: HashMap<String, u64>,
    last_modified: Option<u64>,
}

impl MediaLibrary {
    /// A tracker with notifications off and no snapshot yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a listing has been observed since the last unload.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Drops the snapshot and the modification time, as an unload does. The notification switch is kept. The next
    /// observed listing counts every item as inserted.
    pub fn unload(&mut self) {
        self.loaded = false;
        self.snapshot.clear();
        self.last_modified = None;
    }

    /// Switches change notifications on.
    pub fn begin_generating(&mut self) {
        self.generating = true;
    }

    /// Switches change notifications off. The tracker keeps its snapshot and modification time.
    pub fn end_generating(&mut self) {
        self.generating = false;
    }

    /// Whether change notifications are on.
    pub fn is_generating(&self) -> bool {
        self.generating
    }

    /// The time of the last change seen, or None when no change has been seen.
    pub fn last_modified(&self) -> Option<u64> {
        self.last_modified
    }

    /// Compares `stamps` with the last snapshot and keeps them as the new snapshot. When anything changed, the
    /// modification time becomes `now`. The change is returned only while notifications are on. If the same identifier
    /// appears twice, the later stamp wins.
    pub fn observe(&mut self, stamps: &[Stamp], now: u64) -> Option<LibraryChange> {
        let mut next: HashMap<String, u64> = HashMap::new();
        for stamp in stamps {
            next.insert(stamp.id.clone(), stamp.revision);
        }
        let mut change = LibraryChange::default();
        for (id, revision) in &next {
            match self.snapshot.get(id) {
                None => change.inserted.push(id.clone()),
                Some(old) if old != revision => change.changed.push(id.clone()),
                Some(_) => {}
            }
        }
        for id in self.snapshot.keys() {
            if !next.contains_key(id) {
                change.removed.push(id.clone());
            }
        }
        change.inserted.sort();
        change.removed.sort();
        change.changed.sort();
        self.snapshot = next;
        self.loaded = true;
        if change.is_empty() {
            return None;
        }
        self.last_modified = Some(now);
        if self.generating {
            Some(change)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(id: &str, revision: u64) -> Stamp {
        Stamp { id: id.to_string(), revision }
    }

    #[test]
    fn first_snapshot_with_notifications_off_records_items_without_a_report() {
        let mut library = MediaLibrary::new();
        assert_eq!(library.last_modified(), None);
        assert_eq!(library.observe(&[stamp("a", 1), stamp("b", 1)], 100), None);
        assert_eq!(library.last_modified(), Some(100));
    }

    #[test]
    fn insert_remove_and_revision_change_are_reported_sorted() {
        let mut library = MediaLibrary::new();
        library.observe(&[stamp("b", 1), stamp("c", 1), stamp("d", 1)], 10);
        library.begin_generating();
        // "a" is new, "b" moved to revision 2, "c" is gone, "d" is unchanged.
        let change = library
            .observe(&[stamp("a", 1), stamp("b", 2), stamp("d", 1)], 20)
            .expect("notifications are on");
        assert_eq!(change.inserted, vec!["a".to_string()]);
        assert_eq!(change.changed, vec!["b".to_string()]);
        assert_eq!(change.removed, vec!["c".to_string()]);
        assert_eq!(library.last_modified(), Some(20));
    }

    #[test]
    fn unchanged_listing_reports_nothing_and_keeps_the_modification_time() {
        let mut library = MediaLibrary::new();
        library.begin_generating();
        library.observe(&[stamp("a", 1)], 5);
        assert_eq!(library.observe(&[stamp("a", 1)], 99), None);
        assert_eq!(library.last_modified(), Some(5));
    }

    #[test]
    fn changes_while_notifications_are_off_update_the_time_but_are_not_reported() {
        let mut library = MediaLibrary::new();
        library.observe(&[stamp("a", 1)], 1);
        library.end_generating();
        assert_eq!(library.observe(&[stamp("a", 1), stamp("z", 1)], 50), None);
        assert_eq!(library.last_modified(), Some(50));
        assert!(!library.is_generating());
    }

    #[test]
    fn switching_notifications_on_later_does_not_replay_old_changes() {
        let mut library = MediaLibrary::new();
        library.observe(&[stamp("a", 1)], 1);
        // The change happens while notifications are off.
        library.observe(&[stamp("a", 1), stamp("b", 1)], 2);
        library.begin_generating();
        assert_eq!(library.observe(&[stamp("a", 1), stamp("b", 1)], 3), None);
        assert_eq!(library.last_modified(), Some(2));
    }

    #[test]
    fn unload_drops_the_snapshot_and_the_next_listing_reports_every_item_as_inserted() {
        let mut library = MediaLibrary::new();
        assert!(!library.is_loaded());
        library.begin_generating();
        library.observe(&[stamp("a", 1), stamp("b", 1)], 7);
        assert!(library.is_loaded());
        library.unload();
        assert!(!library.is_loaded());
        assert_eq!(library.last_modified(), None);
        let change = library.observe(&[stamp("a", 1), stamp("b", 1)], 8).expect("notifications are on");
        assert_eq!(change.inserted, vec!["a".to_string(), "b".to_string()]);
        assert!(change.removed.is_empty() && change.changed.is_empty());
        assert_eq!(library.last_modified(), Some(8));
    }

    #[test]
    fn duplicate_identifier_takes_the_later_stamp() {
        let mut library = MediaLibrary::new();
        library.observe(&[stamp("a", 1)], 1);
        library.begin_generating();
        // The later stamp for "a" carries revision 1, so nothing changed.
        assert_eq!(library.observe(&[stamp("a", 9), stamp("a", 1)], 2), None);
    }
}
