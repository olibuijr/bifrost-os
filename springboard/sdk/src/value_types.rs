//! Value types beyond the std collections: `IndexSet`, `OrderedSet` (insertion-ordered set), `CountedSet` (multiset)
//! and `Uuid` (128-bit identifier).
//!
//! * `IndexSet` is the shell's sorted-run set, re-exported unchanged.
//! * `OrderedSet` keeps each item once in insertion order. Inserting an item already present leaves it where it is.
//! * `CountedSet` counts how many times each item was added; an item leaves the set when its count reaches zero.
//! * `Uuid` is the 128-bit identifier, printed in upper case with hyphens. Random identifiers come from the system
//!   entropy source (`entropy`); the version and variant bits are set as RFC 4122 requires.
//!
//! `Decimal` lives in `decimal`. Gaps: no number or value wrappers, no key-value coding on these types, and no serialisation trait
//! (apps encode them as text with `Display` and `Uuid::parse`).
use crate::entropy;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::Hash;

pub use bifrost_ui::indexset::IndexSet;

/// A set that keeps its items in insertion order. Each item appears once.
#[derive(Clone, Debug)]
pub struct OrderedSet<T> {
    items: Vec<T>,
    members: HashSet<T>,
}

impl<T: Clone + Eq + Hash> OrderedSet<T> {
    pub fn new() -> Self {
        OrderedSet { items: Vec::new(), members: HashSet::new() }
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn contains(&self, item: &T) -> bool {
        self.members.contains(item)
    }
    /// Appends `item` unless it is already present. Returns true when it was added.
    pub fn insert(&mut self, item: T) -> bool {
        if self.members.contains(&item) {
            return false;
        }
        self.members.insert(item.clone());
        self.items.push(item);
        true
    }
    /// The position of `item`, counted from 0.
    pub fn index_of(&self, item: &T) -> Option<usize> {
        self.items.iter().position(|x| x == item)
    }
    /// The item at `index`.
    pub fn get(&self, index: usize) -> Option<&T> {
        self.items.get(index)
    }
    /// Removes `item`; the items after it keep their order. Returns true when it was present.
    pub fn remove(&mut self, item: &T) -> bool {
        if !self.members.remove(item) {
            return false;
        }
        if let Some(i) = self.index_of(item) {
            self.items.remove(i);
        }
        true
    }
    pub fn clear(&mut self) {
        self.items.clear();
        self.members.clear();
    }
    /// The items in order.
    pub fn as_slice(&self) -> &[T] {
        &self.items
    }
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }
}

impl<T: Clone + Eq + Hash> Default for OrderedSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// A multiset: each item with the number of times it was added.
#[derive(Clone, Debug)]
pub struct CountedSet<T> {
    counts: HashMap<T, usize>,
}

impl<T: Clone + Eq + Hash> CountedSet<T> {
    pub fn new() -> Self {
        CountedSet { counts: HashMap::new() }
    }
    /// Adds one occurrence of `item` and returns its new count.
    pub fn add(&mut self, item: T) -> usize {
        let c = self.counts.entry(item).or_insert(0);
        *c += 1;
        *c
    }
    /// How many times `item` was added and not removed.
    pub fn count_of(&self, item: &T) -> usize {
        self.counts.get(item).copied().unwrap_or(0)
    }
    /// Removes one occurrence of `item`. Returns false when it was not in the set.
    pub fn remove(&mut self, item: &T) -> bool {
        let Some(c) = self.counts.get_mut(item) else {
            return false;
        };
        if *c > 1 {
            *c -= 1;
            return true;
        }
        self.counts.remove(item);
        true
    }
    /// Removes every occurrence of `item` and returns how many there were.
    pub fn remove_all(&mut self, item: &T) -> usize {
        self.counts.remove(item).unwrap_or(0)
    }
    /// Number of distinct items.
    pub fn distinct_count(&self) -> usize {
        self.counts.len()
    }
    /// Number of occurrences across all items.
    pub fn total(&self) -> usize {
        self.counts.values().sum()
    }
    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }
    /// Each distinct item with its count, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&T, usize)> + '_ {
        self.counts.iter().map(|(k, v)| (k, *v))
    }
}

impl<T: Clone + Eq + Hash> Default for CountedSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// A 128-bit identifier (`UUID`). Ordered and hashable, so it can key maps and sort.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uuid([u8; 16]);

impl Uuid {
    /// The all-zero identifier (`UUID()` in the nil form).
    pub const NIL: Uuid = Uuid([0; 16]);

    pub fn from_bytes(bytes: [u8; 16]) -> Uuid {
        Uuid(bytes)
    }
    pub fn bytes(self) -> [u8; 16] {
        self.0
    }
    /// A version 4 identifier built from 16 random bytes: the version and RFC 4122 variant bits are set.
    pub fn version4(mut random: [u8; 16]) -> Uuid {
        random[6] = (random[6] & 0x0f) | 0x40;
        random[8] = (random[8] & 0x3f) | 0x80;
        Uuid(random)
    }
    /// A new random (version 4) identifier from the system entropy source.
    pub fn new_random() -> Result<Uuid, entropy::Error> {
        let b = entropy::bytes(16)?;
        let mut a = [0u8; 16];
        a.copy_from_slice(&b);
        Ok(Uuid::version4(a))
    }
    /// `uuidString`: the 36-character upper-case hyphenated form.
    pub fn uuid_string(self) -> String {
        self.to_string()
    }
    /// The version number stored in the identifier.
    pub fn version(self) -> u8 {
        self.0[6] >> 4
    }
    /// Parses the 36-character hyphenated form in either case (`UUID(uuidString:)`). Returns `None` for any other text.
    pub fn parse(text: &str) -> Option<Uuid> {
        let b = text.as_bytes();
        if b.len() != 36 {
            return None;
        }
        let mut out = [0u8; 16];
        let mut j = 0;
        let mut i = 0;
        while i < 36 {
            if is_hyphen_position(i) {
                if b[i] != b'-' {
                    return None;
                }
                i += 1;
                continue;
            }
            let hi = hex_value(b[i])?;
            let lo = hex_value(b[i + 1])?;
            out[j] = (hi << 4) | lo;
            j += 1;
            i += 2;
        }
        Some(Uuid(out))
    }
}

impl std::str::FromStr for Uuid {
    type Err = ParseUuidError;
    fn from_str(text: &str) -> Result<Uuid, ParseUuidError> {
        Uuid::parse(text).ok_or(ParseUuidError)
    }
}

/// The text given to `Uuid::from_str` is not a 36-character hyphenated identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseUuidError;

impl fmt::Display for ParseUuidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid UUID string")
    }
}

impl std::error::Error for ParseUuidError {}

fn is_hyphen_position(i: usize) -> bool {
    matches!(i, 8 | 13 | 18 | 23)
}

fn hex_value(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.0;
        let a = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        let bb = u16::from_be_bytes([b[4], b[5]]);
        let c = u16::from_be_bytes([b[6], b[7]]);
        let d = u16::from_be_bytes([b[8], b[9]]);
        let mut e = 0u64;
        for &x in &b[10..] {
            e = (e << 8) | u64::from(x);
        }
        write!(f, "{a:08X}-{bb:04X}-{c:04X}-{d:04X}-{e:012X}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_set_is_the_shared_run_set() {
        let mut s = IndexSet::new();
        s.insert_range(2..5);
        s.insert(9);
        assert_eq!(s.count(), 4);
        assert!(s.contains(4) && !s.contains(5));
    }

    #[test]
    fn ordered_set_keeps_insertion_order_and_ignores_duplicates() {
        let mut s = OrderedSet::new();
        assert!(s.insert("b"));
        assert!(s.insert("a"));
        assert!(!s.insert("b"));
        assert_eq!(s.as_slice(), &["b", "a"]);
        assert_eq!(s.index_of(&"a"), Some(1));
        assert_eq!(s.get(0), Some(&"b"));
    }

    #[test]
    fn ordered_set_remove_keeps_the_order_of_the_rest() {
        let mut s: OrderedSet<u32> = OrderedSet::default();
        for x in [3, 1, 4, 1, 5] {
            s.insert(x);
        }
        assert_eq!(s.len(), 4);
        assert!(s.remove(&1));
        assert!(!s.remove(&1));
        assert_eq!(s.iter().copied().collect::<Vec<_>>(), vec![3, 4, 5]);
        assert!(!s.contains(&1));
    }

    #[test]
    fn counted_set_counts_occurrences() {
        let mut s = CountedSet::new();
        assert_eq!(s.add("x"), 1);
        assert_eq!(s.add("x"), 2);
        s.add("y");
        assert_eq!(s.count_of(&"x"), 2);
        assert_eq!(s.distinct_count(), 2);
        assert_eq!(s.total(), 3);
        assert!(s.remove(&"x"));
        assert_eq!(s.count_of(&"x"), 1);
        assert!(s.remove(&"x"));
        assert_eq!(s.count_of(&"x"), 0);
        assert!(!s.remove(&"x"));
    }

    #[test]
    fn counted_set_remove_all_clears_every_occurrence() {
        let mut s = CountedSet::new();
        s.add(7u8);
        s.add(7u8);
        s.add(7u8);
        assert_eq!(s.remove_all(&7), 3);
        assert_eq!(s.remove_all(&7), 0);
        assert!(s.is_empty());
    }

    #[test]
    fn uuid_formats_upper_case_with_hyphens() {
        assert_eq!(Uuid::NIL.to_string(), "00000000-0000-0000-0000-000000000000");
        let u = Uuid::from_bytes([0x6b, 0xa7, 0xb8, 0x10, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8]);
        assert_eq!(u.to_string(), "6BA7B810-9DAD-11D1-80B4-00C04FD430C8");
    }

    #[test]
    fn uuid_parse_accepts_either_case_and_rejects_bad_text() {
        let u = Uuid::parse("6ba7b810-9dad-11d1-80b4-00c04fd430c8").expect("valid lower case");
        assert_eq!(u.bytes()[0], 0x6b);
        assert_eq!(Uuid::parse(&u.to_string()), Some(u));
        assert_eq!(Uuid::parse("6BA7B8109DAD11D180B400C04FD430C8"), None);
        assert_eq!(Uuid::parse("6BA7B810-9DAD-11D1-80B4-00C04FD430C"), None);
        assert_eq!(Uuid::parse("6BA7B810-9DAD-11D1-80B4-00C04FD430CG"), None);
    }

    #[test]
    fn uuid_string_and_from_str_round_trip() {
        let u = Uuid::version4([7; 16]);
        assert_eq!(u.uuid_string(), u.to_string());
        assert_eq!(u.uuid_string().parse::<Uuid>(), Ok(u));
        assert_eq!(u.uuid_string().to_lowercase().parse::<Uuid>(), Ok(u));
        assert_eq!("nope".parse::<Uuid>(), Err(ParseUuidError));
    }

    #[test]
    fn uuid_orders_by_bytes_and_hashes() {
        let a = Uuid::from_bytes([0; 16]);
        let mut hi = [0u8; 16];
        hi[0] = 1;
        let b = Uuid::from_bytes(hi);
        assert!(a < b);
        let set: HashSet<Uuid> = [a, b, a].into_iter().collect();
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn version4_sets_version_and_variant_bits() {
        let u = Uuid::version4([0xff; 16]);
        assert_eq!(u.version(), 4);
        assert_eq!(u.bytes()[8] & 0xc0, 0x80);
        if let Ok(random) = Uuid::new_random() {
            assert_eq!(random.version(), 4);
        }
    }
}
