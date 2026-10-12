//! Peer identity: a peer with a display name and an id that stays the same while the app keeps its record.
//!
//! [`PeerId::new`] makes a new identity. Two ids made from the same display name are different peers, so an app that
//! wants a device to keep one identity over time stores the text from [`PeerId::to_record`] and restores it with
//! [`PeerId::from_record`]. The display name is read-only and holds at most [`MAX_NAME_BYTES`] bytes of UTF-8. A name
//! may not be empty and may not contain control characters, so a record can be split on its tab safely.
//!
//! This is the pure model. Nothing is written to disk here, the id is not exchanged with other devices, and the shell's
//! nearby-device list still identifies a device by its name and network address.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Longest display name, in bytes of UTF-8.
pub const MAX_NAME_BYTES: usize = 63;

static SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PeerId {
    name: String,
    id: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    EmptyName,
    NameTooLong,
    ControlCharacter,
    BadRecord,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Error::EmptyName => "display name is empty",
            Error::NameTooLong => "display name is longer than 63 bytes",
            Error::ControlCharacter => "display name contains a control character",
            Error::BadRecord => "peer record is malformed",
        };
        f.write_str(text)
    }
}

fn check_name(name: &str) -> Result<String, Error> {
    if name.is_empty() {
        return Err(Error::EmptyName);
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(Error::NameTooLong);
    }
    if name.chars().any(char::is_control) {
        return Err(Error::ControlCharacter);
    }
    Ok(name.to_string())
}

impl PeerId {
    /// A new identity for `display_name`. The id mixes the clock, the process id and a counter, so two calls never
    /// return the same id within one process.
    pub fn new(display_name: &str) -> Result<PeerId, Error> {
        let name = check_name(display_name)?;
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mixed = nanos.rotate_left(17) ^ (u64::from(std::process::id()) << 32) ^ sequence;
        Ok(PeerId { name, id: mixed.max(1) })
    }

    pub fn display_name(&self) -> &str {
        &self.name
    }

    /// The id as a number. Ids are never zero.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The identity as one line: the id in 16 hex digits, a tab, then the display name.
    pub fn to_record(&self) -> String {
        format!("{:016x}\t{}", self.id, self.name)
    }

    /// Restores an identity written by [`PeerId::to_record`]. Fails on a missing tab, a short or non-hex id, a zero id,
    /// or a name that would not pass [`PeerId::new`].
    pub fn from_record(record: &str) -> Result<PeerId, Error> {
        let (hex, name) = record.split_once('\t').ok_or(Error::BadRecord)?;
        if hex.len() != 16 {
            return Err(Error::BadRecord);
        }
        let id = u64::from_str_radix(hex, 16).map_err(|_| Error::BadRecord)?;
        if id == 0 {
            return Err(Error::BadRecord);
        }
        let name = check_name(name)?;
        Ok(PeerId { name, id })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_control_character_names_are_refused() {
        assert_eq!(PeerId::new("").unwrap_err(), Error::EmptyName);
        assert_eq!(PeerId::new("Anna\tB").unwrap_err(), Error::ControlCharacter);
        assert_eq!(PeerId::new("line\nbreak").unwrap_err(), Error::ControlCharacter);
    }

    #[test]
    fn name_limit_counts_utf8_bytes_not_characters() {
        // 31 characters of two bytes each is 62 bytes, and one ASCII letter brings it to 63.
        let ok = format!("{}a", "é".repeat(31));
        assert_eq!(ok.len(), 63);
        assert_eq!(PeerId::new(&ok).unwrap().display_name(), ok);
        // 32 characters of two bytes each is 64 bytes, one over the limit.
        assert_eq!(PeerId::new(&"é".repeat(32)).unwrap_err(), Error::NameTooLong);
    }

    #[test]
    fn ids_from_the_same_name_are_different_peers() {
        let a = PeerId::new("Anna").unwrap();
        let b = PeerId::new("Anna").unwrap();
        assert_eq!(a.display_name(), b.display_name());
        assert_ne!(a.id(), b.id());
        assert_ne!(a, b);
        assert_ne!(a.id(), 0);
    }

    #[test]
    fn record_round_trip_keeps_the_identity() {
        // 0x2a is 42, written as 16 hex digits with leading zeros.
        let restored = PeerId::from_record("000000000000002a\tAnna Ólafs").unwrap();
        assert_eq!(restored.id(), 42);
        assert_eq!(restored.display_name(), "Anna Ólafs");
        assert_eq!(restored.to_record(), "000000000000002a\tAnna Ólafs");
    }

    #[test]
    fn malformed_records_are_refused() {
        // No tab, a short id, a non-hex digit, a zero id and an empty name.
        assert_eq!(PeerId::from_record("000000000000002a Anna").unwrap_err(), Error::BadRecord);
        assert_eq!(PeerId::from_record("2a\tAnna").unwrap_err(), Error::BadRecord);
        assert_eq!(PeerId::from_record("00000000000000zz\tAnna").unwrap_err(), Error::BadRecord);
        assert_eq!(PeerId::from_record("0000000000000000\tAnna").unwrap_err(), Error::BadRecord);
        assert_eq!(PeerId::from_record("000000000000002a\t").unwrap_err(), Error::EmptyName);
    }
}
