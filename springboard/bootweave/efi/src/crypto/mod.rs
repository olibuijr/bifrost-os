//! Pure primitives for the pre-boot unlock (task 10): SHA-256, HMAC-SHA-256, PBKDF2-HMAC-SHA-256, AES-128/256 and
//! AES-XTS. No dependencies, no unsafe, no allocation in the block functions. Each file's tests use the official
//! vectors (FIPS 180-4, RFC 4231, FIPS 197, IEEE 1619, and the PBKDF2 vectors of RFC 7914 and its test set).

pub mod aes;
pub mod sha256;
pub mod xts;

use sha256::Sha256;

/// HMAC-SHA-256 with the inner and outer key states precomputed, so each PBKDF2 round costs two compressions.
pub struct Hmac {
    inner: Sha256,
    outer: Sha256,
}

impl Hmac {
    pub fn new(key: &[u8]) -> Hmac {
        let mut block = [0u8; 64];
        if key.len() > 64 {
            block[..32].copy_from_slice(&Sha256::digest(key));
        } else {
            block[..key.len()].copy_from_slice(key);
        }
        let mut inner_pad = [0x36u8; 64];
        let mut outer_pad = [0x5cu8; 64];
        for i in 0..64 {
            inner_pad[i] ^= block[i];
            outer_pad[i] ^= block[i];
        }
        let mut inner = Sha256::new();
        inner.update(&inner_pad);
        let mut outer = Sha256::new();
        outer.update(&outer_pad);
        Hmac { inner, outer }
    }

    /// MAC over the concatenation of `parts`.
    pub fn mac(&self, parts: &[&[u8]]) -> [u8; 32] {
        let mut inner = self.inner.clone();
        for part in parts {
            inner.update(part);
        }
        let digest = inner.finish();
        let mut outer = self.outer.clone();
        outer.update(&digest);
        outer.finish()
    }
}

/// PBKDF2-HMAC-SHA-256: fills `out` with the derived key (any length).
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32, out: &mut [u8]) {
    let mac = Hmac::new(password);
    for (block, chunk) in out.chunks_mut(32).enumerate() {
        let index = (block as u32 + 1).to_be_bytes();
        let mut u = mac.mac(&[salt, &index]);
        let mut t = u;
        for _ in 1..iterations.max(1) {
            u = mac.mac(&[&u]);
            for (a, b) in t.iter_mut().zip(u.iter()) {
                *a ^= b;
            }
        }
        chunk.copy_from_slice(&t[..chunk.len()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> alloc::string::String {
        bytes.iter().map(|b| alloc::format!("{:02x}", b)).collect()
    }

    #[test]
    fn hmac_rfc4231_case1() {
        let mac = Hmac::new(&[0x0b; 20]);
        assert_eq!(hex(&mac.mac(&[b"Hi There"])), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
    }

    #[test]
    fn hmac_rfc4231_case2() {
        let mac = Hmac::new(b"Jefe");
        assert_eq!(hex(&mac.mac(&[b"what do ya want for nothing?"])), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn hmac_splits_parts_like_one_message() {
        let mac = Hmac::new(b"key");
        assert_eq!(mac.mac(&[b"ab", b"cd"]), mac.mac(&[b"abcd"]));
    }

    #[test]
    fn pbkdf2_one_iteration() {
        let mut out = [0u8; 32];
        pbkdf2_sha256(b"password", b"salt", 1, &mut out);
        assert_eq!(hex(&out), "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b");
    }

    #[test]
    fn pbkdf2_two_iterations() {
        let mut out = [0u8; 32];
        pbkdf2_sha256(b"password", b"salt", 2, &mut out);
        assert_eq!(hex(&out), "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43");
    }

    #[test]
    fn pbkdf2_fills_partial_and_multi_block_outputs() {
        // 40 bytes is two PBKDF2 blocks; the first 32 bytes must equal the one-block output.
        let mut long = [0u8; 40];
        let mut short = [0u8; 32];
        pbkdf2_sha256(b"password", b"salt", 1, &mut long);
        pbkdf2_sha256(b"password", b"salt", 1, &mut short);
        assert_eq!(&long[..32], &short[..]);
        assert_ne!(&long[32..], &[0u8; 8][..]);
    }
}
