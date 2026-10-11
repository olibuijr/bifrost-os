//! Sets of integers (`IndexSet`): sorted, disjoint, non-touching ranges, so a selection of a million rows costs one
//! entry per run. Membership and range queries find their run by binary search. A single integer is stored as
//! `i..i + 1`, so an integer must stay below `usize::MAX`.
use std::ops::Range;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexSet {
    // Invariant: every range is non-empty, the list is sorted, and no two ranges overlap or touch.
    ranges: Vec<Range<usize>>,
}

impl IndexSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// The set holding one integer (`init(integer:)`).
    pub fn from_index(index: usize) -> Self {
        let mut set = Self::new();
        set.insert(index);
        set
    }

    /// The set holding every integer in `range` (`init(integersIn:)`).
    pub fn from_range(range: Range<usize>) -> Self {
        let mut set = Self::new();
        set.insert_range(range);
        set
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// How many integers the set holds (`count`).
    pub fn count(&self) -> usize {
        self.ranges.iter().map(|r| r.end - r.start).sum()
    }

    pub fn first(&self) -> Option<usize> {
        self.ranges.first().map(|r| r.start)
    }

    pub fn last(&self) -> Option<usize> {
        self.ranges.last().map(|r| r.end - 1)
    }

    /// The runs of consecutive integers, ascending (`rangeView`).
    pub fn ranges(&self) -> &[Range<usize>] {
        &self.ranges
    }

    /// Every integer, ascending.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.ranges.iter().flat_map(|r| r.clone())
    }

    pub fn insert(&mut self, index: usize) {
        self.insert_range(index..index + 1);
    }

    /// Adds every integer in `range`, merging each run it overlaps or touches (`insert(integersIn:)`).
    pub fn insert_range(&mut self, range: Range<usize>) {
        if range.start >= range.end {
            return;
        }
        let lo = self.ranges.partition_point(|r| r.end < range.start);
        let hi = self.ranges.partition_point(|r| r.start <= range.end);
        let mut merged = range;
        if lo < hi {
            merged.start = merged.start.min(self.ranges[lo].start);
            merged.end = merged.end.max(self.ranges[hi - 1].end);
        }
        self.ranges.splice(lo..hi, std::iter::once(merged));
    }

    pub fn remove(&mut self, index: usize) {
        self.remove_range(index..index + 1);
    }

    /// Removes every integer in `range`, splitting a run that the range sits inside (`remove(integersIn:)`).
    pub fn remove_range(&mut self, range: Range<usize>) {
        if range.start >= range.end {
            return;
        }
        let lo = self.ranges.partition_point(|r| r.end <= range.start);
        let hi = self.ranges.partition_point(|r| r.start < range.end);
        let mut kept = Vec::with_capacity(2);
        for r in &self.ranges[lo..hi] {
            if r.start < range.start {
                kept.push(r.start..range.start);
            }
            if range.end < r.end {
                kept.push(range.end..r.end);
            }
        }
        self.ranges.splice(lo..hi, kept);
    }

    pub fn clear(&mut self) {
        self.ranges.clear();
    }

    pub fn contains(&self, index: usize) -> bool {
        let i = self.ranges.partition_point(|r| r.end <= index);
        self.ranges.get(i).is_some_and(|r| r.start <= index)
    }

    /// True when every integer in `range` is in the set. An empty range is always contained.
    pub fn contains_range(&self, range: Range<usize>) -> bool {
        if range.start >= range.end {
            return true;
        }
        let i = self.ranges.partition_point(|r| r.end <= range.start);
        self.ranges.get(i).is_some_and(|r| r.start <= range.start && range.end <= r.end)
    }

    /// True when at least one integer in `range` is in the set (`intersects(integersIn:)`).
    pub fn intersects_range(&self, range: Range<usize>) -> bool {
        if range.start >= range.end {
            return false;
        }
        let i = self.ranges.partition_point(|r| r.end <= range.start);
        self.ranges.get(i).is_some_and(|r| r.start < range.end)
    }

    /// How many integers of the set lie in `range` (`count(in:)`).
    pub fn count_in(&self, range: Range<usize>) -> usize {
        if range.start >= range.end {
            return 0;
        }
        let lo = self.ranges.partition_point(|r| r.end <= range.start);
        let hi = self.ranges.partition_point(|r| r.start < range.end);
        self.ranges[lo..hi].iter().map(|r| r.end.min(range.end) - r.start.max(range.start)).sum()
    }

    /// The largest member that is at most `index` (`integerLessThanOrEqualTo`).
    pub fn integer_less_than_or_equal_to(&self, index: usize) -> Option<usize> {
        let i = self.ranges.partition_point(|r| r.start <= index);
        let r = self.ranges.get(i.checked_sub(1)?)?;
        Some(index.min(r.end - 1))
    }

    /// The smallest member that is at least `index` (`integerGreaterThanOrEqualTo`).
    pub fn integer_greater_than_or_equal_to(&self, index: usize) -> Option<usize> {
        let i = self.ranges.partition_point(|r| r.end <= index);
        self.ranges.get(i).map(|r| index.max(r.start))
    }

    /// The smallest member that is greater than `index` (`integerGreaterThan`).
    pub fn integer_greater_than(&self, index: usize) -> Option<usize> {
        self.integer_greater_than_or_equal_to(index.checked_add(1)?)
    }

    /// The largest member that is less than `index` (`integerLessThan`).
    pub fn integer_less_than(&self, index: usize) -> Option<usize> {
        self.integer_less_than_or_equal_to(index.checked_sub(1)?)
    }

    /// Adds every member of `other` (`formUnion`).
    pub fn form_union(&mut self, other: &IndexSet) {
        for r in &other.ranges {
            self.insert_range(r.clone());
        }
    }

    /// Keeps only the members that are also in `other` (`formIntersection`).
    pub fn form_intersection(&mut self, other: &IndexSet) {
        *self = self.intersection(other);
    }

    /// Members in exactly one of the two sets (`formSymmetricDifference`).
    pub fn form_symmetric_difference(&mut self, other: &IndexSet) {
        *self = self.symmetric_difference(other);
    }

    pub fn union(&self, other: &IndexSet) -> IndexSet {
        let mut out = self.clone();
        out.form_union(other);
        out
    }

    pub fn intersection(&self, other: &IndexSet) -> IndexSet {
        let mut out = Vec::new();
        for a in &self.ranges {
            let first = other.ranges.partition_point(|b| b.end <= a.start);
            for b in &other.ranges[first..] {
                if b.start >= a.end {
                    break;
                }
                out.push(a.start.max(b.start)..a.end.min(b.end));
            }
        }
        IndexSet { ranges: out }
    }

    pub fn symmetric_difference(&self, other: &IndexSet) -> IndexSet {
        let mut out = self.union(other);
        for r in self.intersection(other).ranges {
            out.remove_range(r);
        }
        out
    }

    /// The members inside `within` that `keep` accepts (`filteredIndexSet(in:includeInteger:)`).
    pub fn filtered_in(&self, within: Range<usize>, mut keep: impl FnMut(usize) -> bool) -> IndexSet {
        let mut out = IndexSet::new();
        for i in self.intersection(&IndexSet::from_range(within)).iter() {
            if keep(i) {
                out.insert(i);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ranges: &[Range<usize>]) -> IndexSet {
        let mut s = IndexSet::new();
        for r in ranges {
            s.insert_range(r.clone());
        }
        s
    }

    #[test]
    fn insert_merges_overlapping_and_touching_runs() {
        let mut s = IndexSet::new();
        s.insert_range(0..3);
        s.insert_range(3..5);
        assert_eq!(s.ranges(), &[0..5], "touching runs merge");
        s.insert(10);
        assert_eq!(s.ranges(), &[0..5, 10..11]);
        s.insert_range(4..12);
        assert_eq!(s.ranges(), &[0..12], "an overlapping range joins both runs");
        assert_eq!(s.count(), 12);
        assert_eq!(s.iter().collect::<Vec<_>>(), (0..12).collect::<Vec<_>>());
    }

    #[test]
    fn remove_splits_a_run_and_keeps_both_sides() {
        let mut s = set(&[0..10]);
        s.remove_range(3..5);
        assert_eq!(s.ranges(), &[0..3, 5..10]);
        assert!(!s.contains(4));
        assert!(s.contains(5));
        s.remove(0);
        s.remove(9);
        assert_eq!(s.ranges(), &[1..3, 5..9]);
        assert_eq!(s.first(), Some(1));
        assert_eq!(s.last(), Some(8));
        s.clear();
        assert!(s.is_empty());
        assert_eq!(s.first(), None);
    }

    #[test]
    fn range_queries_respect_the_gaps() {
        let s = set(&[0..3, 5..10]);
        assert!(s.contains_range(5..10));
        assert!(!s.contains_range(2..6), "the gap at 3 and 4 is not in the set");
        assert!(s.contains_range(4..4), "an empty range is always contained");
        assert!(!s.intersects_range(3..5), "the gap alone does not intersect");
        assert!(s.intersects_range(4..6));
        assert_eq!(s.count_in(2..7), 3, "2, 5 and 6");
        assert_eq!(s.count(), 8);
    }

    #[test]
    fn neighbour_lookups_find_members_across_gaps() {
        let s = set(&[0..3, 5..10]);
        assert_eq!(s.integer_less_than_or_equal_to(4), Some(2));
        assert_eq!(s.integer_less_than_or_equal_to(7), Some(7));
        assert_eq!(s.integer_greater_than_or_equal_to(3), Some(5));
        assert_eq!(s.integer_greater_than(2), Some(5));
        assert_eq!(s.integer_greater_than(9), None);
        assert_eq!(s.integer_less_than(0), None);
        assert_eq!(s.integer_less_than(5), Some(2));
    }

    #[test]
    fn set_operations_match_their_definitions() {
        let a = set(&[0..4]);
        let b = set(&[2..6]);
        assert_eq!(a.union(&b).ranges(), &[0..6]);
        assert_eq!(a.intersection(&b).ranges(), &[2..4]);
        assert_eq!(a.symmetric_difference(&b).ranges(), &[0..2, 4..6]);
        let mut c = a.clone();
        c.form_symmetric_difference(&b);
        assert_eq!(c, a.symmetric_difference(&b));
        let mut d = a.clone();
        d.form_intersection(&b);
        assert_eq!(d.count(), 2);
    }

    #[test]
    fn filtered_in_keeps_only_accepted_members_of_the_window() {
        let s = set(&[0..12]);
        let evens = s.filtered_in(2..9, |i| i % 2 == 0);
        assert_eq!(evens.iter().collect::<Vec<_>>(), vec![2, 4, 6, 8]);
        assert_eq!(evens.count(), 4);
        assert_eq!(s.count(), 12, "the source set is unchanged");
    }
}
