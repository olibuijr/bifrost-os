//! Security transforms for apps: SHA-256 digests, HMAC-SHA256 message authentication and a constant-time comparison.
//!
//! The hash is the shell's own SHA-256 (`bifrost_core::sha256`), so an app and the shell agree on every byte. The
//! functions are pure: no key store, no network and no hardware. Encryption and signatures with key objects are not
//! here; an app that needs to check a tag or a checksum it received uses [`hmac_sha256`] with its own secret, and
//! [`constant_time_eq`] to compare the result without leaking where the first mismatch is.

use bifrost_core::sha256::Sha256;

/// SHA-256 block size in bytes. HMAC pads or hashes its key to this length.
pub const BLOCK_SIZE: usize = 64;

/// SHA-256 digest of `data`, 32 bytes.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    bifrost_core::sha256::digest(data)
}

/// HMAC-SHA256 (RFC 2104) of `message` under `key`. A key longer than the block size is hashed first, and a shorter
/// key is padded with zero bytes.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        block[..32].copy_from_slice(&sha256(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36u8; BLOCK_SIZE];
    let mut outer_pad = [0x5cu8; BLOCK_SIZE];
    for (i, b) in block.iter().enumerate() {
        inner_pad[i] ^= b;
        outer_pad[i] ^= b;
    }
    let mut inner = Sha256::new();
    inner.update(&inner_pad);
    inner.update(message);
    let inner_digest = inner.finish();
    let mut outer = Sha256::new();
    outer.update(&outer_pad);
    outer.update(&inner_digest);
    outer.finish()
}

/// Compares two byte strings without stopping at the first difference. Strings of different length are unequal at
/// once; the length itself is not secret.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_of_abc_matches_the_fips_180_vector() {
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn hmac_matches_rfc_4231_case_2() {
        // Key "Jefe", message "what do ya want for nothing?"
        let tag = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(hex(&tag), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn a_key_longer_than_a_block_is_hashed_first() {
        // By the rule, hmac(K) with |K| > 64 equals hmac(SHA-256(K)) because the hashed key is 32 bytes and is padded.
        let long_key = vec![0xaau8; 131];
        let hashed = sha256(&long_key);
        assert_eq!(hmac_sha256(&long_key, b"msg"), hmac_sha256(&hashed, b"msg"));
    }

    #[test]
    fn an_empty_key_equals_a_key_of_zero_bytes_of_block_length() {
        // Both keys pad to 64 zero bytes, so they give the same block and the same tag.
        let zeros = [0u8; BLOCK_SIZE];
        assert_eq!(hmac_sha256(b"", b"data"), hmac_sha256(&zeros, b"data"));
    }

    #[test]
    fn a_different_key_gives_a_different_tag() {
        assert_ne!(hmac_sha256(b"key one", b"data"), hmac_sha256(b"key two", b"data"));
    }

    #[test]
    fn constant_time_eq_accepts_only_identical_strings() {
        assert!(constant_time_eq(b"tag", b"tag"));
        assert!(constant_time_eq(b"", b""));
        assert!(!constant_time_eq(b"tag", b"tab"));
        assert!(!constant_time_eq(b"tag", b"tags"));
        assert!(!constant_time_eq(b"", b"x"));
    }
}
