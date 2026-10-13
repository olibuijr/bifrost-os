//! Key objects for apps: a secret key that is generated from the random pool, kept in memory, and used for
//! HMAC-SHA256 authentication codes with a constant-time check.
//!
//! The key is an object rather than a byte slice so that its length is checked once, its bytes never reach `Debug`
//! output, and the tag check cannot be done with a plain `==`. Sizes are whole bytes in the three AES-style lengths
//! (16, 24 and 32 bytes, that is 128, 192 and 256 bits). The key lives in app memory only; storing it is the job of
//! [`crate::keychain`]. Encryption and public-key signatures are not here, because the SDK has no cipher or curve
//! implementation yet.

use crate::digest::{constant_time_eq, hmac_sha256};
use crate::entropy;

/// Length of an HMAC-SHA256 tag in bytes.
pub const TAG_BYTES: usize = 32;

/// Why a key call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The key length is not 16, 24 or 32 bytes.
    InvalidSize,
    /// The random pool could not supply key bytes.
    Unavailable,
}

/// A secret key. `Debug` shows the length only.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretKey {
    bytes: Vec<u8>,
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretKey({} bytes)", self.bytes.len())
    }
}

fn valid_size(len: usize) -> bool {
    matches!(len, 16 | 24 | 32)
}

impl SecretKey {
    /// A new random key of `bits` bits (128, 192 or 256).
    pub fn generate(bits: u32) -> Result<SecretKey, Error> {
        let len = match bits {
            128 => 16,
            192 => 24,
            256 => 32,
            _ => return Err(Error::InvalidSize),
        };
        let bytes = entropy::bytes(len).map_err(|_| Error::Unavailable)?;
        Ok(SecretKey { bytes })
    }

    /// A key from existing bytes, for example one read back from the keychain.
    pub fn from_bytes(bytes: &[u8]) -> Result<SecretKey, Error> {
        if !valid_size(bytes.len()) {
            return Err(Error::InvalidSize);
        }
        Ok(SecretKey { bytes: bytes.to_vec() })
    }

    /// The key length in bits.
    pub fn bit_count(&self) -> u32 {
        (self.bytes.len() * 8) as u32
    }

    /// The raw key bytes. The caller is responsible for keeping them out of logs and files.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// HMAC-SHA256 authentication code of `data` under this key.
    pub fn authentication_code(&self, data: &[u8]) -> [u8; TAG_BYTES] {
        hmac_sha256(&self.bytes, data)
    }

    /// True when `tag` is the authentication code of `data` under this key. The comparison takes the same time
    /// whatever the first differing byte is.
    pub fn is_valid_code(&self, data: &[u8], tag: &[u8]) -> bool {
        constant_time_eq(&self.authentication_code(data), tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::hmac_sha256;

    #[test]
    fn generated_keys_have_the_requested_length_and_differ() {
        let a = SecretKey::generate(256).unwrap();
        let b = SecretKey::generate(256).unwrap();
        assert_eq!(a.bit_count(), 256);
        assert_eq!(a.bytes().len(), 32);
        assert_ne!(a, b);
        assert_eq!(SecretKey::generate(128).unwrap().bytes().len(), 16);
        assert_eq!(SecretKey::generate(192).unwrap().bytes().len(), 24);
    }

    #[test]
    fn other_key_sizes_are_refused() {
        assert_eq!(SecretKey::generate(100), Err(Error::InvalidSize));
        assert_eq!(SecretKey::from_bytes(&[0u8; 15]), Err(Error::InvalidSize));
        assert_eq!(SecretKey::from_bytes(&[0u8; 33]), Err(Error::InvalidSize));
        assert!(SecretKey::from_bytes(&[7u8; 24]).is_ok());
    }

    #[test]
    fn debug_output_never_shows_the_key_bytes() {
        let key = SecretKey::from_bytes(&[0xABu8; 16]).unwrap();
        let shown = format!("{key:?}");
        assert_eq!(shown, "SecretKey(16 bytes)");
        assert!(!shown.contains("ab") && !shown.contains("AB") && !shown.contains("171"));
    }

    #[test]
    fn an_authentication_code_checks_only_with_the_same_key_and_data() {
        let key = SecretKey::from_bytes(b"0123456789abcdef").unwrap();
        let other = SecretKey::from_bytes(b"fedcba9876543210").unwrap();
        let tag = key.authentication_code(b"message");
        assert!(key.is_valid_code(b"message", &tag));
        assert!(!key.is_valid_code(b"messagE", &tag));
        assert!(!other.is_valid_code(b"message", &tag));
        assert!(!key.is_valid_code(b"message", &tag[..31]));
    }

    #[test]
    fn the_code_matches_the_rfc_4231_test_case_two() {
        // RFC 4231 test case 2: key "Jefe" (4 bytes is too short for a key object, so the raw HMAC is checked).
        let expected = [
            0x5b, 0xdc, 0xc1, 0x46, 0xbf, 0x60, 0x75, 0x4e, 0x6a, 0x04, 0x24, 0x26, 0x08, 0x95, 0x75, 0xc7, 0x5a, 0x00,
            0x3f, 0x08, 0x9d, 0x27, 0x39, 0x83, 0x9d, 0xec, 0x58, 0xb9, 0x64, 0xec, 0x38, 0x43,
        ];
        assert_eq!(hmac_sha256(b"Jefe", b"what do ya want for nothing?"), expected);
    }
}
