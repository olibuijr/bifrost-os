//! DNS-SD TXT record data: keys and values packed into length-prefixed strings (std only).
//!
//! A TXT record is a list of strings. Each string is one length byte followed by up to 255
//! bytes of `key`, `key=` or `key=value`. A key is printable ASCII (0x20 to 0x7E) without
//! `=`, and a key can exist in four states: absent, present with no value (`key` alone),
//! present with an empty value (`key=`), or present with a value (`key=value`).
//! This module builds, edits and parses that record data. It opens no socket and performs
//! no multicast query; the service layer hands it the bytes it received.

use std::fmt;

/// Longest string the length byte can describe.
const MAX_STRING: usize = 255;
/// Longest record: the length of a TXT record is a 16-bit value.
const MAX_RECORD: usize = u16::MAX as usize;

/// Why an entry was refused or a record could not be parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxtError {
    /// The key is empty or holds a byte outside printable ASCII, or the `=` separator.
    InvalidKey,
    /// The key and value together do not fit in one 255-byte string.
    TooLong,
    /// Adding the entry would push the record past its 16-bit length.
    StorageFull,
    /// The data ends in the middle of a string.
    Truncated,
}

impl fmt::Display for TxtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TxtError::InvalidKey => "invalid TXT key",
            TxtError::TooLong => "TXT string longer than 255 bytes",
            TxtError::StorageFull => "TXT record would exceed 65535 bytes",
            TxtError::Truncated => "TXT data ends inside a string",
        })
    }
}

impl std::error::Error for TxtError {}

/// The state of one key in a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence<'a> {
    /// The key is not in the record.
    Absent,
    /// The key appears alone, with no `=`.
    NoValue,
    /// The key appears as `key=` with an empty value.
    Empty,
    /// The key appears as `key=value`.
    Value(&'a [u8]),
}

/// One stored key. `None` means no `=`; `Some(empty)` means `key=`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    key: String,
    value: Option<Vec<u8>>,
}

impl Entry {
    /// Length of this entry as one string, without the length byte.
    fn string_len(&self) -> usize {
        match &self.value {
            None => self.key.len(),
            Some(v) => self.key.len() + 1 + v.len(),
        }
    }
}

/// An ordered TXT record. Keys are unique and keep their first position when replaced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TxtRecord {
    entries: Vec<Entry>,
}

/// Checks that a key is non-empty printable ASCII without `=`.
fn valid_key(key: &str) -> Result<(), TxtError> {
    if key.is_empty() || !key.bytes().all(|b| (0x20..=0x7E).contains(&b) && b != b'=') {
        return Err(TxtError::InvalidKey);
    }
    Ok(())
}

impl TxtRecord {
    /// An empty record.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of keys in the record.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the record holds no keys.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Length of the record data as it would be encoded.
    pub fn encoded_len(&self) -> usize {
        self.entries.iter().map(|e| 1 + e.string_len()).sum()
    }

    /// Sets `key` to `value`. `None` stores the key alone, `Some(&[])` stores `key=`.
    /// An existing key is replaced in place, so its position in the record is kept.
    pub fn set(&mut self, key: &str, value: Option<&[u8]>) -> Result<(), TxtError> {
        valid_key(key)?;
        let entry = Entry { key: key.to_string(), value: value.map(|v| v.to_vec()) };
        if entry.string_len() > MAX_STRING {
            return Err(TxtError::TooLong);
        }
        let existing = self.entries.iter().position(|e| e.key == key);
        let old_len = existing.map_or(0, |i| 1 + self.entries[i].string_len());
        let new_total = self.encoded_len() - old_len + 1 + entry.string_len();
        if new_total > MAX_RECORD {
            return Err(TxtError::StorageFull);
        }
        match existing {
            Some(i) => self.entries[i] = entry,
            None => self.entries.push(entry),
        }
        Ok(())
    }

    /// Returns the state of `key`. Lookup is case-sensitive, as keys are compared as written.
    pub fn get(&self, key: &str) -> Presence<'_> {
        match self.entries.iter().find(|e| e.key == key) {
            None => Presence::Absent,
            Some(Entry { value: None, .. }) => Presence::NoValue,
            Some(Entry { value: Some(v), .. }) if v.is_empty() => Presence::Empty,
            Some(Entry { value: Some(v), .. }) => Presence::Value(v),
        }
    }

    /// True when `key` is present with or without a value.
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key) != Presence::Absent
    }

    /// Removes `key`. Returns true when it was present.
    pub fn remove(&mut self, key: &str) -> bool {
        match self.entries.iter().position(|e| e.key == key) {
            Some(i) => {
                self.entries.remove(i);
                true
            }
            None => false,
        }
    }

    /// The key and state at `index`, in record order.
    pub fn item_at(&self, index: usize) -> Option<(&str, Presence<'_>)> {
        let e = self.entries.get(index)?;
        let state = match &e.value {
            None => Presence::NoValue,
            Some(v) if v.is_empty() => Presence::Empty,
            Some(v) => Presence::Value(v),
        };
        Some((e.key.as_str(), state))
    }

    /// Encodes the record as length-prefixed strings in record order.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.encoded_len());
        for e in &self.entries {
            out.push(e.string_len() as u8);
            out.extend_from_slice(e.key.as_bytes());
            if let Some(v) = &e.value {
                out.push(b'=');
                out.extend_from_slice(v);
            }
        }
        out
    }

    /// Parses length-prefixed record data. An empty string is skipped. When a key repeats,
    /// the first occurrence wins, and a later one is ignored.
    pub fn from_bytes(data: &[u8]) -> Result<Self, TxtError> {
        let mut record = TxtRecord::new();
        let mut pos = 0;
        while pos < data.len() {
            let len = data[pos] as usize;
            pos += 1;
            let end = pos + len;
            if end > data.len() {
                return Err(TxtError::Truncated);
            }
            let string = &data[pos..end];
            pos = end;
            if string.is_empty() {
                continue;
            }
            let (key_bytes, value) = match string.iter().position(|&b| b == b'=') {
                Some(eq) => (&string[..eq], Some(&string[eq + 1..])),
                None => (string, None),
            };
            let key = std::str::from_utf8(key_bytes).map_err(|_| TxtError::InvalidKey)?;
            valid_key(key)?;
            if record.contains_key(key) {
                continue;
            }
            record.entries.push(Entry { key: key.to_string(), value: value.map(|v| v.to_vec()) });
        }
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_key_states_are_told_apart() {
        let mut r = TxtRecord::new();
        r.set("flag", None).unwrap();
        r.set("empty", Some(b"")).unwrap();
        r.set("name", Some(b"bifrost")).unwrap();
        assert_eq!(r.get("missing"), Presence::Absent);
        assert_eq!(r.get("flag"), Presence::NoValue);
        assert_eq!(r.get("empty"), Presence::Empty);
        assert_eq!(r.get("name"), Presence::Value(b"bifrost"));
        assert!(r.contains_key("flag"));
        assert!(!r.contains_key("missing"));
    }

    #[test]
    fn invalid_keys_are_refused() {
        let mut r = TxtRecord::new();
        assert_eq!(r.set("", None), Err(TxtError::InvalidKey));
        assert_eq!(r.set("a=b", None), Err(TxtError::InvalidKey));
        assert_eq!(r.set("tab\there", None), Err(TxtError::InvalidKey));
        assert_eq!(r.set("caf\u{e9}", None), Err(TxtError::InvalidKey));
        assert!(r.is_empty());
    }

    #[test]
    fn setting_an_existing_key_replaces_it_in_place() {
        let mut r = TxtRecord::new();
        r.set("a", Some(b"1")).unwrap();
        r.set("b", Some(b"2")).unwrap();
        r.set("a", Some(b"third")).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r.item_at(0), Some(("a", Presence::Value(b"third"))));
        assert_eq!(r.item_at(1), Some(("b", Presence::Value(b"2"))));
    }

    #[test]
    fn length_limits_are_enforced() {
        let mut r = TxtRecord::new();
        // key "k" plus "=" plus 253 value bytes is exactly 255 bytes.
        r.set("k", Some(&[b'x'; 253])).unwrap();
        assert_eq!(r.set("k2", Some(&[b'x'; 253])), Err(TxtError::TooLong));
        // Fill the record close to 65535 bytes, then one more large string must not fit.
        let mut big = TxtRecord::new();
        let mut i = 0;
        while big.encoded_len() + 256 <= MAX_RECORD {
            big.set(&format!("k{i}"), Some(&[b'v'; 250])).unwrap();
            i += 1;
        }
        assert_eq!(big.set("last", Some(&[b'v'; 250])), Err(TxtError::StorageFull));
        assert!(big.encoded_len() <= MAX_RECORD);
    }

    #[test]
    fn encoding_matches_the_length_prefixed_layout() {
        let mut r = TxtRecord::new();
        r.set("a", Some(b"1")).unwrap();
        r.set("flag", None).unwrap();
        r.set("e", Some(b"")).unwrap();
        assert_eq!(r.to_bytes(), b"\x03a=1\x04flag\x02e=".to_vec());
        assert_eq!(r.encoded_len(), 4 + 5 + 3);
    }

    #[test]
    fn round_trip_keeps_order_and_states() {
        let mut r = TxtRecord::new();
        r.set("txtvers", Some(b"1")).unwrap();
        r.set("note", None).unwrap();
        r.set("blank", Some(b"")).unwrap();
        r.set("path", Some(b"/a=b")).unwrap();
        let parsed = TxtRecord::from_bytes(&r.to_bytes()).unwrap();
        assert_eq!(parsed, r);
        assert_eq!(parsed.get("path"), Presence::Value(b"/a=b"));
    }

    #[test]
    fn parsing_rejects_truncation_and_keeps_the_first_duplicate() {
        assert_eq!(TxtRecord::from_bytes(b"\x05ab"), Err(TxtError::Truncated));
        let parsed = TxtRecord::from_bytes(b"\x03k=1\x03k=2\x00\x02z=").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed.get("k"), Presence::Value(b"1"));
        assert_eq!(parsed.get("z"), Presence::Empty);
    }

    #[test]
    fn remove_and_item_at_follow_record_order() {
        let mut r = TxtRecord::new();
        r.set("x", None).unwrap();
        r.set("y", Some(b"2")).unwrap();
        assert!(r.remove("x"));
        assert!(!r.remove("x"));
        assert_eq!(r.item_at(0), Some(("y", Presence::Value(b"2"))));
        assert_eq!(r.item_at(1), None);
    }
}
