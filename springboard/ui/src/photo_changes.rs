//! Change details for a photo library (the change record a library observer receives). Given the assets a fetch
//! returned before a change and after it, this lists which assets were inserted, removed, moved, or changed in content.
//! An asset is identified by its identifier; a content change keeps the identifier and gives the asset a new
//! modification stamp. The order comes from `diff::diff`, so the moves are the fewest that turn the old order into
//! the new one. Pure: the shell keeps the last listing, fetches the new one and asks for the details.

use crate::diff::{self, Change};
use std::collections::{HashMap, HashSet};

/// One asset in a fetched listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub id: String,
    /// The modification stamp (for example seconds since the epoch). A new value means the content changed.
    pub modified: u64,
}

/// What changed between two listings. Indexes are positions in the listing they refer to: `removed` in the old
/// listing, `inserted`, `moved` and `changed` in the new one. Every list is sorted ascending.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Details {
    pub removed: Vec<usize>,
    pub inserted: Vec<usize>,
    pub moved: Vec<usize>,
    pub changed: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// An identifier appears twice in one listing, so the two entries cannot be told apart.
    Duplicate(String),
}

impl Details {
    /// True when the two listings are the same, in order and in content.
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.inserted.is_empty() && self.moved.is_empty() && self.changed.is_empty()
    }
}

/// The changes from `old` to `new`. Fails when either listing names an identifier twice.
pub fn details(old: &[Asset], new: &[Asset]) -> Result<Details, Error> {
    check_unique(old)?;
    check_unique(new)?;
    let old_ids: Vec<&str> = old.iter().map(|a| a.id.as_str()).collect();
    let new_ids: Vec<&str> = new.iter().map(|a| a.id.as_str()).collect();
    let mut out = Details::default();
    for change in diff::diff(&old_ids, &new_ids) {
        match change {
            Change::Remove { from } => out.removed.push(from),
            Change::Insert { to } => out.inserted.push(to),
            Change::Move { to, .. } => out.moved.push(to),
        }
    }
    let old_stamps: HashMap<&str, u64> = old.iter().map(|a| (a.id.as_str(), a.modified)).collect();
    for (j, asset) in new.iter().enumerate() {
        if let Some(&stamp) = old_stamps.get(asset.id.as_str()) {
            if stamp != asset.modified {
                out.changed.push(j);
            }
        }
    }
    out.removed.sort_unstable();
    out.inserted.sort_unstable();
    out.moved.sort_unstable();
    out.changed.sort_unstable();
    Ok(out)
}

fn check_unique(listing: &[Asset]) -> Result<(), Error> {
    let mut seen = HashSet::new();
    for asset in listing {
        if !seen.insert(asset.id.as_str()) {
            return Err(Error::Duplicate(asset.id.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(id: &str, modified: u64) -> Asset {
        Asset { id: id.to_string(), modified }
    }

    #[test]
    fn identical_listings_have_no_changes() {
        let list = [a("x", 1), a("y", 2)];
        let d = details(&list, &list).unwrap();
        assert!(d.is_empty());
    }

    #[test]
    fn a_new_asset_at_the_end_is_inserted_at_its_index() {
        let d = details(&[a("a", 1), a("b", 1)], &[a("a", 1), a("b", 1), a("c", 1)]).unwrap();
        assert_eq!(d.inserted, vec![2]);
        assert!(d.removed.is_empty() && d.moved.is_empty() && d.changed.is_empty());
    }

    #[test]
    fn a_removed_asset_is_reported_at_its_old_index() {
        let d = details(&[a("a", 1), a("b", 1), a("c", 1)], &[a("a", 1), a("c", 1)]).unwrap();
        assert_eq!(d.removed, vec![1]);
        assert!(d.inserted.is_empty() && d.moved.is_empty());
    }

    #[test]
    fn a_new_stamp_marks_the_asset_changed_and_keeps_its_place() {
        let d = details(&[a("a", 1), a("b", 1)], &[a("a", 1), a("b", 2)]).unwrap();
        assert_eq!(d.changed, vec![1]);
        assert!(d.inserted.is_empty() && d.removed.is_empty() && d.moved.is_empty());
    }

    #[test]
    fn moving_the_last_asset_to_the_front_moves_only_that_asset() {
        // Old order a, b, c. New order c, a, b: a and b keep their relative order, so only c moves, to index 0.
        let d = details(&[a("a", 1), a("b", 1), a("c", 1)], &[a("c", 1), a("a", 1), a("b", 1)]).unwrap();
        assert_eq!(d.moved, vec![0]);
        assert!(d.inserted.is_empty() && d.removed.is_empty() && d.changed.is_empty());
    }

    #[test]
    fn insert_remove_and_change_in_one_fetch() {
        // a is gone, b changed its stamp and is now first, c is new at the end.
        let d = details(&[a("a", 1), a("b", 1)], &[a("b", 2), a("c", 1)]).unwrap();
        assert_eq!(d.removed, vec![0]);
        assert_eq!(d.inserted, vec![1]);
        assert_eq!(d.changed, vec![0]);
        assert!(d.moved.is_empty());
    }

    #[test]
    fn a_repeated_identifier_is_refused() {
        let err = details(&[a("a", 1)], &[a("a", 1), a("a", 2)]).unwrap_err();
        assert_eq!(err, Error::Duplicate("a".to_string()));
    }
}
