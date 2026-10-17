//! Certificate revocation by filter cascade (std only): answer "is this certificate revoked?" locally, from a compact
//! filter set, with no request to a certificate authority or a third party.
//!
//! A person's device cannot ask an online responder about every site without telling it which sites they visit. A
//! cascade removes the question. A publisher builds it from the full lists of revoked and valid certificates (keyed by
//! the issuer's public key hash and the serial number) and ships it; the device holds it and answers offline.
//!
//! The cascade is a list of Bloom filters. Level 0 holds the revoked keys. Level 1 holds the valid keys that level 0
//! wrongly reports (its false positives), level 2 the revoked keys that level 1 wrongly reports, and so on, until a
//! level has no false positives. A lookup walks the levels until one does not contain the key: absent from an even
//! level means not revoked, absent from an odd level means revoked. If every level contains it, the last level decides.
//! For every key the builder saw the answer is exact; for a key it never saw the answer is the first level's guess,
//! so the caller combines it with normal chain validation and treats it as one signal.
//!
//! The hashing is SHA-256 with a per-level prefix, double hashing for the probe positions. The wire format is ours:
//! `BFCR`, version 1, level count, then per level the probe count, bit count and bits.

use crate::sha256;
use crate::x509::Certificate;
use std::fmt;

/// Probes per key in each level.
const HASHES: u8 = 4;
/// Bits per stored key in a level the builder makes.
const BITS_PER_KEY: usize = 12;
/// The fewest bits in a level.
const MIN_BITS: usize = 64;
/// Most levels a cascade may have.
pub const MAX_LEVELS: usize = 32;
/// Most bits a decoded level may claim (32 MiB of bits).
const MAX_LEVEL_BITS: u32 = 1 << 28;
const MAGIC: &[u8; 4] = b"BFCR";
const VERSION: u8 = 1;

/// Why a cascade was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CascadeError {
    /// The input is not a cascade (magic, version, probe count or level count wrong, or a size out of range).
    Malformed,
    /// The input ends inside a level.
    Truncated,
    /// Bytes follow the last level.
    TrailingData,
    /// The builder needed more than `MAX_LEVELS` levels.
    TooManyLevels,
    /// A key is on both lists.
    Contradiction,
}

impl fmt::Display for CascadeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CascadeError::Malformed => "not a revocation filter",
            CascadeError::Truncated => "revocation filter is cut off",
            CascadeError::TrailingData => "bytes follow the revocation filter",
            CascadeError::TooManyLevels => "revocation filter needs too many levels",
            CascadeError::Contradiction => "a certificate is listed as both revoked and valid",
        })
    }
}

impl std::error::Error for CascadeError {}

/// The lookup key of a certificate: the issuer's public key hash, then the serial number without leading zeros.
pub fn revocation_key(issuer_spki_sha256: &[u8; 32], serial: &[u8]) -> Vec<u8> {
    let skip = serial.iter().take_while(|b| **b == 0).count();
    let mut key = Vec::with_capacity(32 + serial.len() - skip);
    key.extend_from_slice(issuer_spki_sha256);
    key.extend_from_slice(&serial[skip..]);
    key
}

/// One Bloom filter of the cascade.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Level {
    /// Its position in the cascade, which salts the hash so levels do not share false positives.
    index: u8,
    hashes: u8,
    bit_count: u32,
    bits: Vec<u8>,
}

impl Level {
    fn empty(index: u8, bit_count: u32) -> Level {
        Level { index, hashes: HASHES, bit_count, bits: vec![0; (bit_count as usize).div_ceil(8)] }
    }

    fn probes(&self, key: &[u8]) -> impl Iterator<Item = u32> {
        let mut input = Vec::with_capacity(key.len() + 15);
        input.extend_from_slice(b"bifrost-crlite");
        input.push(self.index);
        input.extend_from_slice(key);
        let d = sha256::digest(&input);
        let h1 = u64::from_be_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]);
        // an odd step is never zero, so the probes move
        let h2 = u64::from_be_bytes([d[8], d[9], d[10], d[11], d[12], d[13], d[14], d[15]]) | 1;
        let m = u64::from(self.bit_count);
        (0..u64::from(self.hashes)).map(move |i| (h1.wrapping_add(i.wrapping_mul(h2)) % m) as u32)
    }

    fn insert(&mut self, key: &[u8]) {
        let positions: Vec<u32> = self.probes(key).collect();
        for p in positions {
            self.bits[(p / 8) as usize] |= 1 << (p % 8);
        }
    }

    fn contains(&self, key: &[u8]) -> bool {
        self.probes(key).all(|p| self.bits[(p / 8) as usize] & (1 << (p % 8)) != 0)
    }

    fn build(index: u8, keys: &[Vec<u8>]) -> Level {
        let bits = (keys.len() * BITS_PER_KEY).max(MIN_BITS);
        let mut level = Level::empty(index, u32::try_from(bits).unwrap_or(MAX_LEVEL_BITS).min(MAX_LEVEL_BITS));
        for k in keys {
            level.insert(k);
        }
        level
    }
}

/// A revocation filter cascade.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cascade {
    levels: Vec<Level>,
}

impl Cascade {
    /// Builds the cascade that tells every key in `revoked` from every key in `valid`.
    pub fn build(revoked: &[Vec<u8>], valid: &[Vec<u8>]) -> Result<Cascade, CascadeError> {
        let valid_set: std::collections::HashSet<&Vec<u8>> = valid.iter().collect();
        if revoked.iter().any(|r| valid_set.contains(r)) {
            return Err(CascadeError::Contradiction);
        }
        let mut include: Vec<Vec<u8>> = revoked.to_vec();
        let mut exclude: Vec<Vec<u8>> = valid.to_vec();
        let mut levels: Vec<Level> = Vec::new();
        loop {
            if levels.len() >= MAX_LEVELS {
                return Err(CascadeError::TooManyLevels);
            }
            let level = Level::build(levels.len() as u8, &include);
            let false_positives: Vec<Vec<u8>> = exclude.iter().filter(|k| level.contains(k)).cloned().collect();
            levels.push(level);
            if false_positives.is_empty() {
                break;
            }
            exclude = include;
            include = false_positives;
        }
        Ok(Cascade { levels })
    }

    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// True when `key` (from `revocation_key`) is revoked.
    pub fn is_revoked(&self, key: &[u8]) -> bool {
        for (i, level) in self.levels.iter().enumerate() {
            if !level.contains(key) {
                return i % 2 == 1;
            }
        }
        (self.levels.len() - 1) % 2 == 0
    }

    /// True when `cert`, issued by the key whose SHA-256 is `issuer_spki_sha256`, is revoked.
    pub fn is_certificate_revoked(&self, cert: &Certificate, issuer_spki_sha256: &[u8; 32]) -> bool {
        self.is_revoked(&revocation_key(issuer_spki_sha256, &cert.serial))
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.push(self.levels.len() as u8);
        for l in &self.levels {
            out.push(l.hashes);
            out.extend_from_slice(&l.bit_count.to_be_bytes());
            out.extend_from_slice(&l.bits);
        }
        out
    }

    pub fn decode(data: &[u8]) -> Result<Cascade, CascadeError> {
        if data.len() < 6 {
            return Err(CascadeError::Truncated);
        }
        if &data[..4] != MAGIC || data[4] != VERSION {
            return Err(CascadeError::Malformed);
        }
        let count = usize::from(data[5]);
        if count == 0 || count > MAX_LEVELS {
            return Err(CascadeError::Malformed);
        }
        let mut pos = 6;
        let mut levels = Vec::with_capacity(count);
        for index in 0..count {
            let head = data.get(pos..pos + 5).ok_or(CascadeError::Truncated)?;
            let hashes = head[0];
            let bit_count = u32::from_be_bytes([head[1], head[2], head[3], head[4]]);
            if hashes == 0 || hashes > 16 || bit_count == 0 || bit_count > MAX_LEVEL_BITS {
                return Err(CascadeError::Malformed);
            }
            pos += 5;
            let len = (bit_count as usize).div_ceil(8);
            let bits = data.get(pos..pos + len).ok_or(CascadeError::Truncated)?.to_vec();
            pos += len;
            levels.push(Level { index: index as u8, hashes, bit_count, bits });
        }
        if pos != data.len() {
            return Err(CascadeError::TrailingData);
        }
        Ok(Cascade { levels })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oid::Oid;
    use crate::x509::{encode as encode_certificate, parse, Template};

    fn keys(issuer: u8, range: std::ops::Range<u32>) -> Vec<Vec<u8>> {
        range.map(|i| revocation_key(&[issuer; 32], &i.to_be_bytes())).collect()
    }

    #[test]
    fn every_listed_key_is_answered_exactly() {
        let revoked = keys(1, 0..200);
        let valid = keys(1, 1_000..3_000);
        let cascade = Cascade::build(&revoked, &valid).unwrap();
        assert!(revoked.iter().all(|k| cascade.is_revoked(k)), "a revoked key was missed");
        assert!(valid.iter().all(|k| !cascade.is_revoked(k)), "a valid key was flagged");
        assert!(cascade.level_count() >= 1);
    }

    #[test]
    fn an_empty_revoked_list_flags_nothing() {
        let valid = keys(2, 0..500);
        let cascade = Cascade::build(&[], &valid).unwrap();
        assert_eq!(cascade.level_count(), 1);
        assert!(valid.iter().all(|k| !cascade.is_revoked(k)));
        assert!(!cascade.is_revoked(&revocation_key(&[9; 32], &[1])));
    }

    #[test]
    fn a_dense_revoked_list_still_separates() {
        // more revoked than valid: level 0 is full of the valid keys' false positives
        let revoked = keys(3, 0..1_500);
        let valid = keys(3, 5_000..5_100);
        let cascade = Cascade::build(&revoked, &valid).unwrap();
        assert!(revoked.iter().all(|k| cascade.is_revoked(k)));
        assert!(valid.iter().all(|k| !cascade.is_revoked(k)));
    }

    #[test]
    fn a_key_on_both_lists_is_a_contradiction() {
        let a = keys(1, 0..3);
        let b = keys(1, 2..5);
        assert_eq!(Cascade::build(&a, &b).unwrap_err(), CascadeError::Contradiction);
    }

    #[test]
    fn the_cascade_survives_encoding_and_building_is_deterministic() {
        let revoked = keys(4, 0..120);
        let valid = keys(4, 500..1_500);
        let cascade = Cascade::build(&revoked, &valid).unwrap();
        let bytes = cascade.encode();
        assert_eq!(&bytes[..4], b"BFCR");
        let back = Cascade::decode(&bytes).unwrap();
        assert_eq!(back, cascade);
        assert!(revoked.iter().all(|k| back.is_revoked(k)));
        assert!(valid.iter().all(|k| !back.is_revoked(k)));
        assert_eq!(Cascade::build(&revoked, &valid).unwrap().encode(), bytes);
    }

    #[test]
    fn damaged_filters_are_refused() {
        let cascade = Cascade::build(&keys(1, 0..10), &keys(1, 100..200)).unwrap();
        let bytes = cascade.encode();
        assert_eq!(Cascade::decode(&bytes[..3]).unwrap_err(), CascadeError::Truncated);
        assert_eq!(Cascade::decode(&bytes[..bytes.len() - 1]).unwrap_err(), CascadeError::Truncated);
        let mut extra = bytes.clone();
        extra.push(0);
        assert_eq!(Cascade::decode(&extra).unwrap_err(), CascadeError::TrailingData);
        let mut magic = bytes.clone();
        magic[0] = b'X';
        assert_eq!(Cascade::decode(&magic).unwrap_err(), CascadeError::Malformed);
        let mut version = bytes.clone();
        version[4] = 2;
        assert_eq!(Cascade::decode(&version).unwrap_err(), CascadeError::Malformed);
        let mut none = bytes.clone();
        none[5] = 0;
        assert_eq!(Cascade::decode(&none).unwrap_err(), CascadeError::Malformed);
        let mut huge = bytes.clone();
        huge[7..11].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(Cascade::decode(&huge).unwrap_err(), CascadeError::Malformed);
        let mut zero_probes = bytes;
        zero_probes[6] = 0;
        assert_eq!(Cascade::decode(&zero_probes).unwrap_err(), CascadeError::Malformed);
    }

    #[test]
    fn serial_numbers_compare_without_leading_zeros() {
        let issuer = [7u8; 32];
        assert_eq!(revocation_key(&issuer, &[0, 0, 5, 6]), revocation_key(&issuer, &[5, 6]));
        assert_ne!(revocation_key(&issuer, &[5, 6]), revocation_key(&[8u8; 32], &[5, 6]));
        assert_eq!(revocation_key(&issuer, &[0]).len(), 32);
    }

    #[test]
    fn a_parsed_certificate_is_looked_up_by_issuer_and_serial() {
        let alg = Oid::parse("1.2.840.10045.4.3.2").unwrap();
        let spki = crate::der::sequence(&[crate::der::sequence(&[crate::der::oid(&Oid::parse("1.2.840.10045.2.1").unwrap()).unwrap()]), crate::der::bit_string(0, &[4u8; 65])]);
        let make = |serial: &[u8]| {
            let name = [("CN", "Issuer")];
            let t = Template { serial, signature_algorithm: &alg, issuer: &name, subject: &[("CN", "Leaf")], not_before: 100, not_after: 200, spki: &spki, dns_names: &[], authority: false };
            parse(&encode_certificate(&t, &[1]).unwrap()).unwrap()
        };
        let issuer_key = [5u8; 32];
        let bad = make(&[0x12, 0x34]);
        let good = make(&[0x12, 0x35]);
        let revoked = vec![revocation_key(&issuer_key, &bad.serial)];
        let valid = vec![revocation_key(&issuer_key, &good.serial)];
        let cascade = Cascade::build(&revoked, &valid).unwrap();
        assert!(cascade.is_certificate_revoked(&bad, &issuer_key));
        assert!(!cascade.is_certificate_revoked(&good, &issuer_key));
    }
}
