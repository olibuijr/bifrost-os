//! HMAC-SHA256 (RFC 2104) and HKDF-SHA256 (RFC 5869) on top of [`crate::sha256`].
//! HKDF derives symmetric key material from a shared secret; the keychain's per-app subkeys can use it.

use crate::sha256::{digest, Sha256};

/// SHA-256 block size in bytes; HMAC keys are padded or hashed to this length.
const BLOCK: usize = 64;
/// SHA-256 output size in bytes.
pub const HASH_LEN: usize = 32;

/// HMAC-SHA256 of `msg` under `key`. Keys longer than one block are hashed first.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; HASH_LEN] {
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..HASH_LEN].copy_from_slice(&digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(&k.map(|b| b ^ 0x36));
    inner.update(msg);
    let inner = inner.finish();
    let mut outer = Sha256::new();
    outer.update(&k.map(|b| b ^ 0x5c));
    outer.update(&inner);
    outer.finish()
}

/// HKDF-Extract: a pseudorandom key from input key material and a salt.
/// An empty salt means HashLen zero bytes; HMAC zero-pads its key, so an empty key gives the same result.
pub fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> [u8; HASH_LEN] {
    hmac_sha256(salt, ikm)
}

/// HKDF-Expand: `len` bytes of output keying material from a pseudorandom key and context `info`.
/// `None` when `len` is more than 255 hash blocks (8160 bytes), the RFC 5869 limit.
pub fn hkdf_expand(prk: &[u8], info: &[u8], len: usize) -> Option<Vec<u8>> {
    if len > 255 * HASH_LEN {
        return None;
    }
    let mut out = Vec::with_capacity(len);
    let mut t: Vec<u8> = Vec::new();
    let mut counter: u8 = 1;
    while out.len() < len {
        let mut msg = t.clone();
        msg.extend_from_slice(info);
        msg.push(counter);
        t = hmac_sha256(prk, &msg).to_vec();
        out.extend_from_slice(&t);
        counter = counter.wrapping_add(1);
    }
    out.truncate(len);
    Some(out)
}

/// HKDF-SHA256 in one call: extract with `salt`, then expand `len` bytes with `info`.
pub fn hkdf_sha256(ikm: &[u8], salt: &[u8], info: &[u8], len: usize) -> Option<Vec<u8>> {
    let prk = hkdf_extract(salt, ikm);
    hkdf_expand(&prk, info, len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::hex;

    #[test]
    fn hmac_matches_rfc4231_case_2() {
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn hmac_hashes_a_key_longer_than_one_block() {
        // RFC 4231 case 6: a 131-byte key is hashed to 32 bytes before padding
        let key = [0xaau8; 131];
        assert_eq!(
            hex(&hmac_sha256(&key, b"Test Using Larger Than Block-Size Key - Hash Key First")),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn hkdf_rfc5869_case_1() {
        let ikm = [0x0bu8; 22];
        let salt: Vec<u8> = (0u8..13).collect();
        let info: Vec<u8> = (0xf0u8..0xfa).collect();
        let prk = hkdf_extract(&salt, &ikm);
        assert_eq!(hex(&prk), "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5");
        let okm = hkdf_expand(&prk, &info, 42).unwrap();
        assert_eq!(hex(&okm), "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865");
        assert_eq!(hkdf_sha256(&ikm, &salt, &info, 42).unwrap(), okm);
    }

    #[test]
    fn empty_salt_equals_a_zero_salt() {
        let ikm = b"input key material";
        assert_eq!(hkdf_extract(&[], ikm), hkdf_extract(&[0u8; HASH_LEN], ikm));
    }

    #[test]
    fn output_length_is_exact_and_limit_is_enforced() {
        let prk = hkdf_extract(b"salt", b"ikm");
        assert_eq!(hkdf_expand(&prk, b"", 0).unwrap().len(), 0);
        assert_eq!(hkdf_expand(&prk, b"", 1).unwrap().len(), 1);
        assert_eq!(hkdf_expand(&prk, b"", 33).unwrap().len(), 33);
        assert!(hkdf_expand(&prk, b"", 255 * HASH_LEN).is_some());
        assert!(hkdf_expand(&prk, b"", 255 * HASH_LEN + 1).is_none());
    }

    #[test]
    fn different_info_gives_different_keys() {
        let a = hkdf_sha256(b"secret", b"", b"app-a", 32).unwrap();
        let b = hkdf_sha256(b"secret", b"", b"app-b", 32).unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
    }
}
