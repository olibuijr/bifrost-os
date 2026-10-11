//! Assets attached to records: file bytes stored once per content digest, referenced by a small `Asset` handle.
//!
//! Mirrors the reference asset type (`CKAsset`): an external file that belongs to a record. The record holds an
//! `Asset` (digest, file name, size); the bytes live in an `AssetStore`. Two records that attach the same bytes share
//! one stored copy. `retain` drops the bytes that no record refers to any more.
//!
//! The digest is FNV-1a over the bytes, which is not a security hash. Two different byte strings with the same digest
//! are reported as `AssetError::Collision` rather than merged. The size limit is our own choice: the reference
//! documentation this module mirrors does not state one.
//!
//! Gaps: no file-system handle (the caller reads and writes the bytes), no copy of an asset between containers, no
//! streamed upload, and no metadata beyond the file name.

use std::collections::BTreeMap;
use std::fmt;

/// The largest asset `AssetStore::new` accepts, in bytes.
pub const MAX_ASSET_BYTES: u64 = 250 * 1024 * 1024;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The content digest of `bytes`: 16 lowercase hex digits of FNV-1a 64.
pub fn digest(bytes: &[u8]) -> String {
    let mut hash = FNV_OFFSET;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

/// A handle to a stored file (`CKAsset`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    digest: String,
    file_name: String,
    size: u64,
}

impl Asset {
    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    pub fn size(&self) -> u64 {
        self.size
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetError {
    /// The file name is empty, hides the file (leading dot), or contains a path separator.
    InvalidName(String),
    /// The bytes are larger than the store's limit.
    TooLarge { size: u64, limit: u64 },
    /// Different bytes already hold this digest.
    Collision(String),
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetError::InvalidName(name) => write!(f, "invalid asset file name {name:?}"),
            AssetError::TooLarge { size, limit } => write!(f, "asset of {size} bytes is over the {limit} byte limit"),
            AssetError::Collision(digest) => write!(f, "two different files have the digest {digest}"),
        }
    }
}

impl std::error::Error for AssetError {}

/// Stored bytes, keyed by digest.
#[derive(Debug)]
pub struct AssetStore {
    limit: u64,
    items: BTreeMap<String, Vec<u8>>,
}

impl Default for AssetStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetStore {
    pub fn new() -> Self {
        Self::with_limit(MAX_ASSET_BYTES)
    }

    pub fn with_limit(limit: u64) -> Self {
        AssetStore { limit, items: BTreeMap::new() }
    }

    /// Stores `bytes` under `file_name`. Storing the same bytes again adds no copy.
    pub fn put(&mut self, file_name: &str, bytes: &[u8]) -> Result<Asset, AssetError> {
        if file_name.is_empty() || file_name.contains('/') || file_name.starts_with('.') {
            return Err(AssetError::InvalidName(file_name.to_string()));
        }
        let size = bytes.len() as u64;
        if size > self.limit {
            return Err(AssetError::TooLarge { size, limit: self.limit });
        }
        let key = digest(bytes);
        let same = self.items.get(&key).map(|stored| stored.as_slice() == bytes);
        match same {
            Some(true) => {}
            Some(false) => return Err(AssetError::Collision(key)),
            None => {
                self.items.insert(key.clone(), bytes.to_vec());
            }
        }
        Ok(Asset { digest: key, file_name: file_name.to_string(), size })
    }

    /// The bytes of `asset`, or `None` when the store does not hold them.
    pub fn get(&self, asset: &Asset) -> Option<&[u8]> {
        self.items.get(asset.digest()).map(|bytes| bytes.as_slice())
    }

    pub fn contains(&self, asset: &Asset) -> bool {
        self.items.contains_key(asset.digest())
    }

    /// Drops every stored file that no referenced asset names, and returns how many were dropped.
    pub fn retain(&mut self, referenced: &[&Asset]) -> usize {
        let keep: Vec<&str> = referenced.iter().map(|a| a.digest()).collect();
        let before = self.items.len();
        self.items.retain(|key, _| keep.contains(&key.as_str()));
        before - self.items.len()
    }

    /// Number of distinct stored files.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Total size of the stored bytes.
    pub fn total_bytes(&self) -> u64 {
        self.items.values().map(|b| b.len() as u64).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_fnv1a_and_stable() {
        assert_eq!(digest(b""), "cbf29ce484222325");
        assert_eq!(digest(b"a"), digest(b"a"));
        assert_ne!(digest(b"a"), digest(b"b"));
    }

    #[test]
    fn put_then_get_returns_the_bytes_with_name_and_size() {
        let mut store = AssetStore::new();
        let asset = store.put("cover.png", b"pixels").unwrap();
        assert_eq!(asset.file_name(), "cover.png");
        assert_eq!(asset.size(), 6);
        assert_eq!(store.get(&asset), Some(&b"pixels"[..]));
        assert!(store.contains(&asset));
    }

    #[test]
    fn the_same_bytes_share_one_stored_copy() {
        let mut store = AssetStore::new();
        let first = store.put("a.txt", b"same").unwrap();
        let second = store.put("b.txt", b"same").unwrap();
        assert_eq!(first.digest(), second.digest());
        assert_eq!(store.len(), 1);
        assert_eq!(store.total_bytes(), 4);
    }

    #[test]
    fn a_file_over_the_limit_is_refused() {
        let mut store = AssetStore::with_limit(3);
        let err = store.put("big.bin", b"four").unwrap_err();
        assert_eq!(err, AssetError::TooLarge { size: 4, limit: 3 });
        assert!(store.is_empty());
        assert!(store.put("small.bin", b"abc").is_ok());
    }

    #[test]
    fn names_that_hide_or_escape_the_file_are_refused() {
        let mut store = AssetStore::new();
        assert!(matches!(store.put("", b"x"), Err(AssetError::InvalidName(_))));
        assert!(matches!(store.put("dir/file", b"x"), Err(AssetError::InvalidName(_))));
        assert!(matches!(store.put(".hidden", b"x"), Err(AssetError::InvalidName(_))));
    }

    #[test]
    fn retain_keeps_only_referenced_files_and_counts_the_rest() {
        let mut store = AssetStore::new();
        let keep = store.put("keep", b"kept bytes").unwrap();
        let drop_me = store.put("drop", b"unused").unwrap();
        let removed = store.retain(&[&keep]);
        assert_eq!(removed, 1);
        assert!(store.contains(&keep));
        assert!(!store.contains(&drop_me));
        assert_eq!(store.get(&drop_me), None);
    }
}
