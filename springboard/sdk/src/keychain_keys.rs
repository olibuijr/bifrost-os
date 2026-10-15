//! Typed key storage in the keychain: a secret key is stored together with its kind, so a key read back is never taken
//! for another kind of secret.
//!
//! A typed key is one keychain item. Its secret is a record: the magic `BFK1`, one kind byte, then the key bytes. The
//! key bytes are checked by [`SecretKey`] (16, 24 or 32 bytes), so a record with a wrong length is refused on read.
//! [`encode`] and [`decode`] are pure. [`add`] and [`update`] write a record through [`crate::keychain`], and
//! [`decode_reply`] reads the keychain's answer back into a key. Key references and key objects that live outside the
//! keychain are not modelled.

use crate::crypto_key::{Error as KeyError, SecretKey};
use crate::keychain::{self, Reply};
use crate::Ctx;

/// Magic bytes at the start of every typed key record.
const MAGIC: &[u8; 4] = b"BFK1";
/// Bytes before the key: the magic and the kind byte.
const HEADER: usize = MAGIC.len() + 1;

/// What a stored key is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    /// A symmetric key for a cipher with an AES-style length.
    Symmetric,
    /// A key for HMAC-SHA256 authentication codes.
    Authentication,
}

impl KeyKind {
    fn tag(self) -> u8 {
        match self {
            KeyKind::Symmetric => 1,
            KeyKind::Authentication => 2,
        }
    }

    fn from_tag(tag: u8) -> Option<KeyKind> {
        match tag {
            1 => Some(KeyKind::Symmetric),
            2 => Some(KeyKind::Authentication),
            _ => None,
        }
    }
}

/// A key read back from the keychain, with the kind it was stored under.
#[derive(Debug, PartialEq)]
pub struct StoredKey {
    pub kind: KeyKind,
    pub key: SecretKey,
}

/// Why a keychain secret is not a typed key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordError {
    /// The secret does not start with the record magic, so it was not written as a typed key.
    NotAKey,
    /// The kind byte names no kind this version knows.
    UnknownKind(u8),
    /// The key bytes are not a valid key length.
    InvalidKey(KeyError),
}

/// Why a typed key call failed: the keychain refused the call, or the secret it returned is not a typed key.
#[derive(Debug, PartialEq)]
pub enum StoreError {
    Keychain(keychain::Error),
    Record(RecordError),
}

/// The record that is stored as the secret: magic, kind byte, key bytes.
pub fn encode(kind: KeyKind, key: &SecretKey) -> Vec<u8> {
    let mut record = Vec::with_capacity(HEADER + key.bytes().len());
    record.extend_from_slice(MAGIC);
    record.push(kind.tag());
    record.extend_from_slice(key.bytes());
    record
}

/// Reads a record back into a key. The kind and the key length are both checked.
pub fn decode(record: &[u8]) -> Result<StoredKey, RecordError> {
    if !record.starts_with(MAGIC) || record.len() < HEADER {
        return Err(RecordError::NotAKey);
    }
    let kind = KeyKind::from_tag(record[MAGIC.len()]).ok_or(RecordError::UnknownKind(record[MAGIC.len()]))?;
    let key = SecretKey::from_bytes(&record[HEADER..]).map_err(RecordError::InvalidKey)?;
    Ok(StoredKey { kind, key })
}

/// Stores a new typed key under `(service, account)`. The reply is `DuplicateItem` when the item exists.
pub fn add(ctx: &Ctx, service: &str, account: &str, kind: KeyKind, key: &SecretKey) -> u32 {
    keychain::add(ctx, service, account, &encode(kind, key))
}

/// Replaces the typed key under `(service, account)`. The reply is `ItemNotFound` when there is no such item.
pub fn update(ctx: &Ctx, service: &str, account: &str, kind: KeyKind, key: &SecretKey) -> u32 {
    keychain::update(ctx, service, account, &encode(kind, key))
}

/// Reads a keychain reply that answers a typed key `get`: the key, or the keychain's error, or a record error.
pub fn decode_reply(reply: Reply) -> Result<StoredKey, StoreError> {
    let record = reply.result.map_err(StoreError::Keychain)?;
    decode(&record).map_err(StoreError::Record)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key16() -> SecretKey {
        SecretKey::from_bytes(&[7u8; 16]).unwrap()
    }

    #[test]
    fn a_record_starts_with_the_magic_and_the_kind_byte() {
        let record = encode(KeyKind::Symmetric, &key16());
        assert_eq!(&record[..4], b"BFK1");
        assert_eq!(record[4], 1);
        assert_eq!(record.len(), HEADER + 16);
    }

    #[test]
    fn a_key_round_trips_with_its_kind() {
        let stored = decode(&encode(KeyKind::Authentication, &key16())).unwrap();
        assert_eq!(stored.kind, KeyKind::Authentication);
        assert_eq!(stored.key, key16());
    }

    #[test]
    fn a_secret_without_the_magic_is_not_a_key() {
        assert_eq!(decode(b"plain password"), Err(RecordError::NotAKey));
        assert_eq!(decode(b"BFK"), Err(RecordError::NotAKey));
    }

    #[test]
    fn an_unknown_kind_byte_is_refused() {
        let mut record = encode(KeyKind::Symmetric, &key16());
        record[4] = 9;
        assert_eq!(decode(&record), Err(RecordError::UnknownKind(9)));
    }

    #[test]
    fn a_key_of_an_invalid_length_is_refused() {
        let mut record = encode(KeyKind::Symmetric, &key16());
        record.truncate(HEADER + 5);
        assert_eq!(decode(&record), Err(RecordError::InvalidKey(KeyError::InvalidSize)));
    }

    #[test]
    fn a_keychain_error_passes_through_decode_reply() {
        let reply = Reply { request: 3, result: Err(keychain::Error::ItemNotFound) };
        assert_eq!(decode_reply(reply), Err(StoreError::Keychain(keychain::Error::ItemNotFound)));
    }

    #[test]
    fn a_reply_with_a_typed_key_decodes_to_the_key() {
        let reply = Reply { request: 4, result: Ok(encode(KeyKind::Symmetric, &key16())) };
        let stored = decode_reply(reply).unwrap();
        assert_eq!(stored.kind, KeyKind::Symmetric);
        assert_eq!(stored.key, key16());
    }
}
