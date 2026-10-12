//! LUKS2 unlock in the loader (task 10, docs/boot-architecture.md "Unlock is login"): the binary header and JSON
//! area, the keyslot's PBKDF2, the anti-forensic merge of the key material, and the check of the master key against
//! the digest. Disk reads go through a closure, so the logic is host-testable without a disk.
//!
//! Supported (the installer must create exactly this, `cryptsetup luksFormat --type luks2 --pbkdf pbkdf2
//! --pbkdf-force-iterations ... --hash sha256 --cipher aes-xts-plain64 --key-size 512`): keyslots of type luks2 with
//! a pbkdf2 sha256 KDF, an luks1 sha256 anti-forensic split with `raw` areas, the digest of the pbkdf2 type with
//! sha256, and one crypt segment with aes-xts-plain64 and 512-byte sectors. Anything else is `Unsupported` (argon2
//! keyslots are the open item in todo_left).

pub mod json;

use crate::crypto::{self, sha256::Sha256, xts};
use alloc::vec::Vec;
use json::Json;

pub const MAGIC: [u8; 6] = *b"LUKS\xba\xbe";
/// The binary header, the JSON area starts right after it.
pub const BINARY_LEN: usize = 4096;
/// Largest JSON area accepted (LUKS2 writes 16 KiB by default).
const MAX_JSON: u64 = 1 << 20;
const SECTOR: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Not a LUKS2 header, or a size field out of range.
    Header,
    /// The JSON area does not parse or lacks a field the unlock needs.
    Json,
    /// A KDF, hash, cipher or keyslot type this loader does not implement.
    Unsupported,
    /// The disk read failed.
    Io,
    /// No keyslot accepts this passphrase.
    WrongPassword,
}

/// The unlocked volume: the master key (volume key) and the byte offset of the data segment on the partition.
#[derive(Debug, PartialEq, Eq)]
pub struct Volume {
    pub key: Vec<u8>,
    pub offset: u64,
}

/// The length of the JSON area, from the binary header's own size field.
pub fn json_len(binary: &[u8]) -> Result<usize, Error> {
    if binary.len() < BINARY_LEN || binary[..6] != MAGIC {
        return Err(Error::Header);
    }
    if u16::from_be_bytes([binary[6], binary[7]]) != 2 {
        return Err(Error::Header);
    }
    let mut size = [0u8; 8];
    size.copy_from_slice(&binary[8..16]);
    let size = u64::from_be_bytes(size);
    // An empty JSON area is not a LUKS2 header either, so the size must be strictly above the binary part.
    if size <= BINARY_LEN as u64 || size > BINARY_LEN as u64 + MAX_JSON {
        return Err(Error::Header);
    }
    Ok((size - BINARY_LEN as u64) as usize)
}

/// Unlocks the volume with `password`. `json` is the JSON area (from `json_len`); `read(offset, len)` returns `len`
/// bytes of the partition at a byte offset (`None` on a read error).
pub fn unlock(json: &[u8], password: &[u8], read: &mut dyn FnMut(u64, usize) -> Option<Vec<u8>>) -> Result<Volume, Error> {
    let doc = json::parse(json).ok_or(Error::Json)?;
    let offset = segment_offset(&doc)?;
    let slots = doc.get("keyslots").and_then(Json::entries).ok_or(Error::Json)?;
    let mut unsupported = false;
    for (id, slot) in slots {
        match try_slot(&doc, id, slot, password, read) {
            Ok(Some(key)) => return Ok(Volume { key, offset }),
            Ok(None) => {}
            Err(Error::Unsupported) => unsupported = true,
            Err(e) => return Err(e),
        }
    }
    Err(if unsupported { Error::Unsupported } else { Error::WrongPassword })
}

/// The byte offset of the single crypt segment this loader supports.
fn segment_offset(doc: &Json) -> Result<u64, Error> {
    let segments = doc.get("segments").and_then(Json::entries).ok_or(Error::Json)?;
    let (_, seg) = segments
        .iter()
        .find(|(_, s)| s.get("type").and_then(Json::as_str) == Some("crypt"))
        .ok_or(Error::Json)?;
    if seg.get("encryption").and_then(Json::as_str) != Some("aes-xts-plain64")
        || seg.get("sector_size").and_then(Json::as_u64) != Some(SECTOR as u64)
    {
        return Err(Error::Unsupported);
    }
    seg.get("offset").and_then(Json::as_u64).ok_or(Error::Json)
}

/// Tries one keyslot. `Ok(None)` means the passphrase does not open it.
fn try_slot(doc: &Json, id: &str, slot: &Json, password: &[u8], read: &mut dyn FnMut(u64, usize) -> Option<Vec<u8>>) -> Result<Option<Vec<u8>>, Error> {
    if slot.get("type").and_then(Json::as_str) != Some("luks2") {
        return Err(Error::Unsupported);
    }
    let kdf = slot.get("kdf").ok_or(Error::Json)?;
    if kdf.get("type").and_then(Json::as_str) != Some("pbkdf2") || kdf.get("hash").and_then(Json::as_str) != Some("sha256") {
        return Err(Error::Unsupported);
    }
    let key_size = slot.get("key_size").and_then(Json::as_u64).ok_or(Error::Json)? as usize;
    if key_size != 32 && key_size != 64 {
        return Err(Error::Unsupported);
    }
    let salt = base64(kdf.get("salt").and_then(Json::as_str).ok_or(Error::Json)?).ok_or(Error::Json)?;
    let iterations = kdf.get("iterations").and_then(Json::as_u64).ok_or(Error::Json)? as u32;
    let mut kek = alloc::vec![0u8; key_size];
    crypto::pbkdf2_sha256(password, &salt, iterations, &mut kek);

    let af = slot.get("af").ok_or(Error::Json)?;
    if af.get("type").and_then(Json::as_str) != Some("luks1") || af.get("hash").and_then(Json::as_str) != Some("sha256") {
        return Err(Error::Unsupported);
    }
    let stripes = af.get("stripes").and_then(Json::as_u64).ok_or(Error::Json)? as usize;
    if stripes == 0 {
        return Err(Error::Json);
    }
    let area = slot.get("area").ok_or(Error::Json)?;
    if area.get("type").and_then(Json::as_str) != Some("raw")
        || area.get("encryption").and_then(Json::as_str) != Some("aes-xts-plain64")
    {
        return Err(Error::Unsupported);
    }
    let area_offset = area.get("offset").and_then(Json::as_u64).ok_or(Error::Json)?;
    let need = key_size * stripes;
    let span = need.div_ceil(SECTOR) * SECTOR;
    let area_size = area.get("size").and_then(Json::as_u64).ok_or(Error::Json)? as usize;
    if area_size < span {
        return Err(Error::Json);
    }
    let mut material = read(area_offset, span).ok_or(Error::Io)?;
    if material.len() < span {
        return Err(Error::Io);
    }

    // The keyslot material is XTS-encrypted with the KEK, sector numbers counted from the start of the area.
    let (k1, k2) = xts::split_key(&kek).ok_or(Error::Unsupported)?;
    for (i, sector) in material.chunks_mut(SECTOR).enumerate() {
        xts::decrypt_sector(&k1, &k2, i as u64, sector).ok_or(Error::Json)?;
    }
    let master = af_merge(&material[..need], key_size, stripes);

    let Some((salt, iterations, expected)) = digest_for(doc, id)? else {
        return Err(Error::Json);
    };
    let mut check = [0u8; 32];
    crypto::pbkdf2_sha256(&master, &salt, iterations, &mut check);
    if expected.len() == check.len() && expected[..] == check[..] {
        Ok(Some(master))
    } else {
        Ok(None)
    }
}

/// The digest that covers keyslot `id`: (salt, iterations, expected digest bytes).
fn digest_for(doc: &Json, id: &str) -> Result<Option<(Vec<u8>, u32, Vec<u8>)>, Error> {
    let digests = doc.get("digests").and_then(Json::entries).ok_or(Error::Json)?;
    for (_, d) in digests {
        let covers = d
            .get("keyslots")
            .and_then(Json::as_arr)
            .map(|ids| ids.iter().any(|v| v.as_str() == Some(id)))
            .unwrap_or(false);
        if !covers {
            continue;
        }
        if d.get("type").and_then(Json::as_str) != Some("pbkdf2") || d.get("hash").and_then(Json::as_str) != Some("sha256") {
            return Err(Error::Unsupported);
        }
        let salt = base64(d.get("salt").and_then(Json::as_str).ok_or(Error::Json)?).ok_or(Error::Json)?;
        let iterations = d.get("iterations").and_then(Json::as_u64).ok_or(Error::Json)? as u32;
        let expected = base64(d.get("digest").and_then(Json::as_str).ok_or(Error::Json)?).ok_or(Error::Json)?;
        return Ok(Some((salt, iterations, expected)));
    }
    Ok(None)
}

/// LUKS1 anti-forensic merge: `stripes` key-sized stripes, each but the last hashed back into the running block.
pub fn af_merge(material: &[u8], key_size: usize, stripes: usize) -> Vec<u8> {
    let mut d = alloc::vec![0u8; key_size];
    for s in 0..stripes - 1 {
        for (j, byte) in d.iter_mut().enumerate() {
            *byte ^= material[s * key_size + j];
        }
        d = diffuse(&d);
    }
    let last = (stripes - 1) * key_size;
    for (j, byte) in d.iter_mut().enumerate() {
        *byte ^= material[last + j];
    }
    d
}

/// The LUKS1 diffusion: each digest-sized block `i` of the data becomes SHA-256(i as big-endian u32 || block), cut to
/// the block's length (the last block may be short). The output has the input's length.
fn diffuse(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for (index, block) in data.chunks(32).enumerate() {
        let mut h = Sha256::new();
        h.update(&(index as u32).to_be_bytes());
        h.update(block);
        let digest = h.finish();
        out.extend_from_slice(&digest[..block.len()]);
    }
    out
}

/// Standard base64 with padding; `None` for characters outside the alphabet.
pub fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1u32 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(json_area: u64) -> Vec<u8> {
        let mut h = alloc::vec![0u8; BINARY_LEN];
        h[..6].copy_from_slice(&MAGIC);
        h[6..8].copy_from_slice(&2u16.to_be_bytes());
        h[8..16].copy_from_slice(&(BINARY_LEN as u64 + json_area).to_be_bytes());
        h
    }

    #[test]
    fn json_length_comes_from_the_header_size() {
        // hdr_size 16384 = 4096 binary + 12288 JSON.
        assert_eq!(json_len(&header(12288)), Ok(12288));
    }

    #[test]
    fn rejects_bad_magic_version_and_sizes() {
        let mut bad_magic = header(4096);
        bad_magic[0] = b'X';
        assert_eq!(json_len(&bad_magic), Err(Error::Header));
        let mut v1 = header(4096);
        v1[7] = 1;
        assert_eq!(json_len(&v1), Err(Error::Header));
        assert_eq!(json_len(&header(0)), Err(Error::Header));
        assert_eq!(json_len(&header(MAX_JSON + 1)), Err(Error::Header));
        assert_eq!(json_len(&[0u8; 100]), Err(Error::Header));
    }

    #[test]
    fn base64_decodes_standard_text() {
        assert_eq!(base64("TWFu"), Some(b"Man".to_vec()));
        assert_eq!(base64("TWE="), Some(b"Ma".to_vec()));
        assert_eq!(base64("TQ=="), Some(b"M".to_vec()));
        assert_eq!(base64(""), Some(Vec::new()));
        assert_eq!(base64("T!=="), None);
    }

    #[test]
    fn af_merge_with_one_stripe_is_the_material() {
        let material = [9u8, 8, 7, 6];
        assert_eq!(af_merge(&material, 4, 1), material.to_vec());
    }

    #[test]
    fn af_merge_hashes_the_running_block_before_the_last_stripe() {
        // Two stripes of 32 bytes: d = diffuse(stripe0) XOR stripe1. Check the length and the XOR with the last stripe.
        let material = [0x11u8; 64];
        let merged = af_merge(&material, 32, 2);
        assert_eq!(merged.len(), 32);
        let expected_head = diffuse(&[0x11u8; 32]);
        for j in 0..32 {
            assert_eq!(merged[j], expected_head[j] ^ 0x11);
        }
    }

    #[test]
    fn diffuse_keeps_the_input_length() {
        assert_eq!(diffuse(&[0u8; 40]).len(), 40);
        assert_eq!(diffuse(&[0u8; 5]).len(), 5);
    }

    #[test]
    fn diffuse_hashes_each_digest_block_with_its_index() {
        // 40 bytes: block 0 is the first 32 bytes, block 1 the last 8. Block i is SHA-256(be32(i) || block), and the
        // short block keeps the first 8 bytes of its digest.
        let data = [0x11u8; 40];
        let out = diffuse(&data);
        let mut first = alloc::vec![0u8, 0, 0, 0];
        first.extend_from_slice(&data[..32]);
        assert_eq!(&out[..32], &Sha256::digest(&first)[..]);
        let mut second = alloc::vec![0u8, 0, 0, 1];
        second.extend_from_slice(&data[32..]);
        assert_eq!(&out[32..], &Sha256::digest(&second)[..8]);
    }

    #[test]
    fn segment_must_be_aes_xts_with_512_byte_sectors() {
        let ok = json::parse(br#"{"segments":{"0":{"type":"crypt","offset":"16777216","size":"dynamic","encryption":"aes-xts-plain64","sector_size":512}}}"#).unwrap();
        assert_eq!(segment_offset(&ok), Ok(16777216));
        let other = json::parse(br#"{"segments":{"0":{"type":"crypt","offset":"0","encryption":"aes-cbc-essiv:sha256","sector_size":512}}}"#).unwrap();
        assert_eq!(segment_offset(&other), Err(Error::Unsupported));
    }

    #[test]
    fn a_missing_disk_read_is_an_io_error_not_a_wrong_password() {
        let doc = br#"{"keyslots":{"0":{"type":"luks2","key_size":32,
            "kdf":{"type":"pbkdf2","hash":"sha256","iterations":1,"salt":"c2FsdA=="},
            "af":{"type":"luks1","hash":"sha256","stripes":1},
            "area":{"type":"raw","offset":"32768","size":"512","encryption":"aes-xts-plain64","key_size":32}}},
            "digests":{"0":{"type":"pbkdf2","keyslots":["0"],"hash":"sha256","iterations":1,
            "salt":"c2FsdA==","digest":"AAAA"}},
            "segments":{"0":{"type":"crypt","offset":"16777216","encryption":"aes-xts-plain64","sector_size":512}}}"#;
        let mut no_disk = |_: u64, _: usize| -> Option<Vec<u8>> { None };
        assert_eq!(unlock(doc, b"pass", &mut no_disk), Err(Error::Io));
    }

    #[test]
    fn unlock_rejects_a_document_without_keyslots() {
        let doc = br#"{"segments":{}}"#;
        let mut disk = |_: u64, _: usize| -> Option<Vec<u8>> { None };
        assert_eq!(unlock(doc, b"pass", &mut disk), Err(Error::Json));
    }
}
