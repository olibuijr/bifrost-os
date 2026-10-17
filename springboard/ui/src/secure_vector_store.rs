//! A secure vector store: embedding vectors kept by id, searched by cosine similarity, and stored on disk only as one
//! sealed blob. The vectors of private text (notes, mail, messages) reveal what the text is about, so the file is
//! encrypted and authenticated under a raw 16- or 32-byte key the caller keeps in the keychain (`sealed_box`), and it is
//! bound to a context string, so a store sealed for one app does not open as another's.
//!
//! Pure model: the caller makes the vectors (for example with the search embedding of the SDK), supplies the key and a
//! fresh random nonce for each seal, and writes the bytes. Nothing here reads a file, a clock or a random source.
//! Layout of the plain bytes before sealing: `BVS1`, dimension (u32), count (u32), then per item the id length (u16),
//! the id, and `dimension` little-endian `f32` values.

use crate::sealed_box::{self, NONCE_LEN};
use std::collections::BTreeMap;

const MAGIC: &[u8; 4] = b"BVS1";
/// The largest vector length a store accepts.
pub const MAX_DIMENSION: usize = 4096;
/// The most items one store holds.
pub const MAX_ITEMS: usize = 100_000;
/// The longest item id, in bytes.
pub const MAX_ID_LEN: usize = 256;

/// Why a store call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The dimension is zero or over `MAX_DIMENSION`, or a vector does not have the store's dimension.
    Dimension,
    /// The id is empty or longer than `MAX_ID_LEN`.
    BadId,
    /// A vector holds NaN or an infinity.
    NotFinite,
    /// The store holds `MAX_ITEMS` items and the id is new.
    Full,
    /// The key has the wrong length.
    Key,
    /// The bytes are not a store, or they were cut short.
    Malformed,
    /// Wrong key or context, or the bytes were changed.
    Authentication,
}

/// Vectors by id, all of one dimension.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorStore {
    dimension: usize,
    items: BTreeMap<String, Vec<f32>>,
}

fn take<'a>(bytes: &'a [u8], at: &mut usize, n: usize) -> Result<&'a [u8], Error> {
    let end = at.checked_add(n).ok_or(Error::Malformed)?;
    let slice = bytes.get(*at..end).ok_or(Error::Malformed)?;
    *at = end;
    Ok(slice)
}

fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

impl VectorStore {
    /// An empty store for vectors of `dimension` values.
    pub fn new(dimension: usize) -> Result<VectorStore, Error> {
        if dimension == 0 || dimension > MAX_DIMENSION {
            return Err(Error::Dimension);
        }
        Ok(VectorStore { dimension, items: BTreeMap::new() })
    }

    pub fn dimension(&self) -> usize {
        self.dimension
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.items.contains_key(id)
    }

    pub fn get(&self, id: &str) -> Option<&[f32]> {
        self.items.get(id).map(Vec::as_slice)
    }

    /// Stores `vector` under `id`, replacing an earlier vector with that id.
    pub fn insert(&mut self, id: &str, vector: &[f32]) -> Result<(), Error> {
        if id.is_empty() || id.len() > MAX_ID_LEN {
            return Err(Error::BadId);
        }
        if vector.len() != self.dimension {
            return Err(Error::Dimension);
        }
        if vector.iter().any(|x| !x.is_finite()) {
            return Err(Error::NotFinite);
        }
        if self.items.len() >= MAX_ITEMS && !self.items.contains_key(id) {
            return Err(Error::Full);
        }
        self.items.insert(id.to_string(), vector.to_vec());
        Ok(())
    }

    /// Removes an item; false when the id was not there.
    pub fn remove(&mut self, id: &str) -> bool {
        self.items.remove(id).is_some()
    }

    /// The `limit` items nearest to `query` by cosine similarity, best first (ties by id). A zero vector, on either
    /// side, scores 0.
    pub fn nearest(&self, query: &[f32], limit: usize) -> Result<Vec<(String, f32)>, Error> {
        if query.len() != self.dimension {
            return Err(Error::Dimension);
        }
        if query.iter().any(|x| !x.is_finite()) {
            return Err(Error::NotFinite);
        }
        let nq = norm(query);
        let mut scored: Vec<(String, f32)> = self
            .items
            .iter()
            .map(|(id, v)| {
                let nv = norm(v);
                let score = if nq == 0.0 || nv == 0.0 { 0.0 } else { query.iter().zip(v).map(|(a, b)| a * b).sum::<f32>() / (nq * nv) };
                (id.clone(), score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.0.cmp(&b.0)));
        scored.truncate(limit);
        Ok(scored)
    }

    fn to_plain(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&(self.dimension as u32).to_be_bytes());
        out.extend_from_slice(&(self.items.len() as u32).to_be_bytes());
        for (id, v) in &self.items {
            out.extend_from_slice(&(id.len() as u16).to_be_bytes());
            out.extend_from_slice(id.as_bytes());
            for x in v {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        out
    }

    fn from_plain(bytes: &[u8]) -> Result<VectorStore, Error> {
        let mut at = 0;
        if take(bytes, &mut at, 4)? != &MAGIC[..] {
            return Err(Error::Malformed);
        }
        let dimension = u32::from_be_bytes(take(bytes, &mut at, 4)?.try_into().map_err(|_| Error::Malformed)?) as usize;
        let count = u32::from_be_bytes(take(bytes, &mut at, 4)?.try_into().map_err(|_| Error::Malformed)?) as usize;
        let mut store = VectorStore::new(dimension).map_err(|_| Error::Malformed)?;
        if count > MAX_ITEMS {
            return Err(Error::Malformed);
        }
        for _ in 0..count {
            let id_len = u16::from_be_bytes(take(bytes, &mut at, 2)?.try_into().map_err(|_| Error::Malformed)?) as usize;
            let id = std::str::from_utf8(take(bytes, &mut at, id_len)?).map_err(|_| Error::Malformed)?.to_string();
            let raw = take(bytes, &mut at, dimension * 4)?;
            let vector: Vec<f32> = raw.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            store.insert(&id, &vector).map_err(|_| Error::Malformed)?;
        }
        if at != bytes.len() || store.len() != count {
            return Err(Error::Malformed);
        }
        Ok(store)
    }

    /// Seals the whole store under `key` (16 or 32 bytes), bound to `context`. `nonce` must be fresh random bytes for
    /// every seal under one key.
    pub fn seal(&self, key: &[u8], nonce: &[u8; NONCE_LEN], context: &[u8]) -> Result<Vec<u8>, Error> {
        if key.len() != 16 && key.len() != 32 {
            return Err(Error::Key);
        }
        let mut plain = self.to_plain();
        let sealed = sealed_box::seal_with_key(&plain, key, nonce, context).map_err(|_| Error::Key);
        plain.iter_mut().for_each(|b| *b = 0);
        sealed
    }

    /// Opens a sealed store. `Authentication` for a wrong key or context or a changed byte; `Malformed` when the
    /// authenticated bytes are not a store.
    pub fn open(sealed: &[u8], key: &[u8], context: &[u8]) -> Result<VectorStore, Error> {
        if key.len() != 16 && key.len() != 32 {
            return Err(Error::Key);
        }
        let mut plain = sealed_box::open_with_key(sealed, key, context).map_err(|e| match e {
            sealed_box::Error::Authentication => Error::Authentication,
            _ => Error::Malformed,
        })?;
        let store = VectorStore::from_plain(&plain);
        plain.iter_mut().for_each(|b| *b = 0);
        store
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];
    const NONCE: [u8; NONCE_LEN] = [3; NONCE_LEN];

    fn store() -> VectorStore {
        let mut s = VectorStore::new(3).unwrap();
        s.insert("north", &[0.0, 1.0, 0.0]).unwrap();
        s.insert("east", &[1.0, 0.0, 0.0]).unwrap();
        s.insert("north-east", &[1.0, 1.0, 0.0]).unwrap();
        s.insert("zero", &[0.0, 0.0, 0.0]).unwrap();
        s
    }

    #[test]
    fn nearest_ranks_by_cosine_and_ties_by_id() {
        let hits = store().nearest(&[0.0, 2.0, 0.0], 3).unwrap();
        let ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["north", "north-east", "east"]);
        assert!((hits[0].1 - 1.0).abs() < 1e-6);
        assert!((hits[1].1 - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        // the zero vector and the orthogonal "east" both score 0, so the id decides and "zero" comes last
        let all = store().nearest(&[0.0, 1.0, 0.0], 10).unwrap();
        assert_eq!(all.last().unwrap().0, "zero");
        assert_eq!(store().nearest(&[0.0, 0.0, 0.0], 10).unwrap()[0].1, 0.0);
    }

    #[test]
    fn insert_replaces_and_refuses_bad_input() {
        let mut s = store();
        s.insert("east", &[0.0, 0.0, 1.0]).unwrap();
        assert_eq!(s.len(), 4);
        assert_eq!(s.get("east"), Some(&[0.0, 0.0, 1.0][..]));
        assert_eq!(s.insert("x", &[1.0, 2.0]), Err(Error::Dimension));
        assert_eq!(s.insert("", &[1.0, 2.0, 3.0]), Err(Error::BadId));
        assert_eq!(s.insert("x", &[1.0, f32::NAN, 3.0]), Err(Error::NotFinite));
        assert_eq!(s.nearest(&[1.0], 1), Err(Error::Dimension));
        assert!(s.remove("east"));
        assert!(!s.remove("east"));
        assert_eq!(VectorStore::new(0), Err(Error::Dimension));
        assert_eq!(VectorStore::new(MAX_DIMENSION + 1), Err(Error::Dimension));
    }

    #[test]
    fn a_sealed_store_round_trips_and_hides_its_ids() {
        let s = store();
        let sealed = s.seal(&KEY, &NONCE, b"notes").unwrap();
        assert!(!sealed.windows(5).any(|w| w == b"north"), "ids are not visible in the sealed bytes");
        assert_eq!(VectorStore::open(&sealed, &KEY, b"notes").unwrap(), s);
    }

    #[test]
    fn wrong_key_context_or_changed_bytes_do_not_open() {
        let sealed = store().seal(&KEY, &NONCE, b"notes").unwrap();
        assert_eq!(VectorStore::open(&sealed, &[8; 32], b"notes"), Err(Error::Authentication));
        assert_eq!(VectorStore::open(&sealed, &KEY, b"mail"), Err(Error::Authentication));
        let mut bad = sealed.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 1;
        assert_eq!(VectorStore::open(&bad, &KEY, b"notes"), Err(Error::Authentication));
        assert_eq!(VectorStore::open(&sealed[..8], &KEY, b"notes"), Err(Error::Malformed));
        assert_eq!(VectorStore::open(&sealed, &[1; 5], b"notes"), Err(Error::Key));
        assert_eq!(store().seal(&[1; 5], &NONCE, b""), Err(Error::Key));
    }

    #[test]
    fn authenticated_bytes_that_are_not_a_store_are_malformed() {
        let junk = sealed_box::seal_with_key(b"not a vector store", &KEY, &NONCE, b"c").unwrap();
        assert_eq!(VectorStore::open(&junk, &KEY, b"c"), Err(Error::Malformed));
    }

    #[test]
    fn an_empty_store_round_trips() {
        let s = VectorStore::new(8).unwrap();
        let sealed = s.seal(&KEY[..16], &NONCE, b"").unwrap();
        let back = VectorStore::open(&sealed, &KEY[..16], b"").unwrap();
        assert!(back.is_empty());
        assert_eq!(back.dimension(), 8);
    }
}
