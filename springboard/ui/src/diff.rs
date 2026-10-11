//! Snapshot diffing (`NativeDiffableDataSourceSnapshot`): the minimal edits that turn one list of identifiers into
//! another, so a list animates inserts, removals and moves instead of reloading. Linear time (hash maps).
use std::collections::HashMap;
use std::hash::Hash;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// Delete the item that was at `from` in the old list.
    Remove { from: usize },
    /// Insert the item that is at `to` in the new list.
    Insert { to: usize },
    /// The item kept its identity but its position relative to the other kept items changed.
    Move { from: usize, to: usize },
}

/// Changes from `old` to `new`. Identifiers must be unique inside each list. Kept items that stay in the
/// longest increasing run of old positions do not move; the others get a `Move`.
pub fn diff<T: Eq + Hash>(old: &[T], new: &[T]) -> Vec<Change> {
    let old_pos: HashMap<&T, usize> = old.iter().enumerate().map(|(i, t)| (t, i)).collect();
    let new_set: HashMap<&T, usize> = new.iter().enumerate().map(|(i, t)| (t, i)).collect();
    let mut out = Vec::new();
    for (i, t) in old.iter().enumerate() {
        if !new_set.contains_key(t) {
            out.push(Change::Remove { from: i });
        }
    }
    // Old positions of kept items in new order; the longest increasing subsequence stays put.
    let kept: Vec<(usize, usize)> = new.iter().enumerate().filter_map(|(j, t)| old_pos.get(t).map(|&i| (j, i))).collect();
    let stay = lis(&kept.iter().map(|k| k.1).collect::<Vec<_>>());
    for (k, &(j, i)) in kept.iter().enumerate() {
        if !stay[k] {
            out.push(Change::Move { from: i, to: j });
        }
    }
    for (j, t) in new.iter().enumerate() {
        if !old_pos.contains_key(t) {
            out.push(Change::Insert { to: j });
        }
    }
    out
}

/// Marks the members of one longest strictly increasing subsequence (patience sorting, O(n log n)).
fn lis(a: &[usize]) -> Vec<bool> {
    let mut tails: Vec<usize> = Vec::new(); // index into `a` of the smallest tail for each length
    let mut prev = vec![usize::MAX; a.len()];
    for i in 0..a.len() {
        let p = tails.partition_point(|&t| a[t] < a[i]);
        if p > 0 {
            prev[i] = tails[p - 1];
        }
        if p == tails.len() {
            tails.push(i);
        } else {
            tails[p] = i;
        }
    }
    let mut keep = vec![false; a.len()];
    let mut cur = tails.last().copied();
    while let Some(c) = cur {
        keep[c] = true;
        cur = if prev[c] == usize::MAX { None } else { Some(prev[c]) };
    }
    keep
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identical_lists_have_no_changes() {
        assert!(diff(&[1, 2, 3], &[1, 2, 3]).is_empty());
    }
    #[test]
    fn inserts_and_removes() {
        let d = diff(&["a", "b", "c"], &["a", "c", "d"]);
        assert_eq!(d, vec![Change::Remove { from: 1 }, Change::Insert { to: 2 }]);
    }
    #[test]
    fn moving_one_item_is_one_move() {
        let d = diff(&[1, 2, 3, 4], &[2, 3, 4, 1]);
        assert_eq!(d, vec![Change::Move { from: 0, to: 3 }]);
    }
    #[test]
    fn reversal_moves_all_but_one() {
        let d = diff(&[1, 2, 3, 4, 5], &[5, 4, 3, 2, 1]);
        assert_eq!(d.len(), 4);
    }
    #[test]
    fn empty_to_full_and_back() {
        assert_eq!(diff::<u8>(&[], &[1, 2]).len(), 2);
        assert_eq!(diff::<u8>(&[1, 2], &[]).len(), 2);
    }
}
