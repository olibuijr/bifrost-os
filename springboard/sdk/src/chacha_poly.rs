//! ChaCha20-Poly1305 for apps: a 256-bit key that seals a message into one box and opens it only when the tag matches.
//!
//! A sealed box is the 12-byte nonce, then the ciphertext, then the 16-byte tag. [`ChaChaPolyKey::seal`] draws a fresh
//! random nonce for every message, so two seals of the same text differ. The additional data is authenticated but
//! not stored: the caller passes the same bytes to [`ChaChaPolyKey::open`]. The cipher and the MAC live in
//! `bifrost_core::chacha_poly`; this module checks the key and box sizes and maps the model's errors.

use bifrost_core::chacha_poly as model;

use crate::entropy;

/// Key length in bytes (256 bits).
pub const KEY_BYTES: usize = model::KEY_LEN;
/// Nonce length in bytes.
pub const NONCE_BYTES: usize = model::NONCE_LEN;
/// Authentication tag length in bytes.
pub const TAG_BYTES: usize = model::TAG_LEN;

/// Why a key, seal or open call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The key is not [`KEY_BYTES`] bytes.
    InvalidKeyLength,
    /// The nonce is not [`NONCE_BYTES`] bytes.
    InvalidNonceLength,
    /// The sealed box is shorter than a nonce and a tag.
    InvalidBox,
    /// The random pool could not supply bytes.
    Unavailable,
    /// The tag does not match the box, the additional data or the key.
    Authentication,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Error::InvalidKeyLength => "key must be 32 bytes",
            Error::InvalidNonceLength => "nonce must be 12 bytes",
            Error::InvalidBox => "sealed box is shorter than a nonce and a tag",
            Error::Unavailable => "the random pool could not supply bytes",
            Error::Authentication => "authentication tag does not match",
        };
        f.write_str(text)
    }
}

/// A ChaCha20-Poly1305 key. `Debug` shows the size only, never the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct ChaChaPolyKey {
    bytes: [u8; KEY_BYTES],
}

impl std::fmt::Debug for ChaChaPolyKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ChaChaPolyKey({} bits)", KEY_BYTES * 8)
    }
}

impl ChaChaPolyKey {
    /// A new random key from the random pool.
    pub fn generate() -> Result<ChaChaPolyKey, Error> {
        let bytes = entropy::bytes(KEY_BYTES).map_err(|_| Error::Unavailable)?;
        ChaChaPolyKey::from_bytes(&bytes)
    }

    /// A key from existing bytes, for example one read back from the keychain.
    pub fn from_bytes(bytes: &[u8]) -> Result<ChaChaPolyKey, Error> {
        let bytes: [u8; KEY_BYTES] = bytes.try_into().map_err(|_| Error::InvalidKeyLength)?;
        Ok(ChaChaPolyKey { bytes })
    }

    /// The raw key bytes. The caller keeps them out of logs and files.
    pub fn bytes(&self) -> &[u8; KEY_BYTES] {
        &self.bytes
    }

    /// Encrypts `plain` under a fresh random nonce and returns the sealed box (nonce, ciphertext, tag).
    pub fn seal(&self, plain: &[u8], aad: &[u8]) -> Result<Vec<u8>, Error> {
        let nonce = entropy::bytes(NONCE_BYTES).map_err(|_| Error::Unavailable)?;
        self.seal_with_nonce(&nonce, plain, aad)
    }

    /// Encrypts `plain` under the given nonce. A nonce must never repeat under the same key; [`ChaChaPolyKey::seal`]
    /// is the safe choice for most callers.
    pub fn seal_with_nonce(&self, nonce: &[u8], plain: &[u8], aad: &[u8]) -> Result<Vec<u8>, Error> {
        if nonce.len() != NONCE_BYTES {
            return Err(Error::InvalidNonceLength);
        }
        let sealed = model::seal(&self.bytes, nonce, aad, plain).map_err(map_model_error)?;
        let mut out = Vec::with_capacity(NONCE_BYTES + sealed.len());
        out.extend_from_slice(nonce);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    /// Checks the tag of a sealed box and returns the plaintext. A box with a wrong key, a changed byte or different
    /// additional data fails with [`Error::Authentication`] and yields no plaintext.
    pub fn open(&self, sealed: &[u8], aad: &[u8]) -> Result<Vec<u8>, Error> {
        if sealed.len() < NONCE_BYTES + TAG_BYTES {
            return Err(Error::InvalidBox);
        }
        let (nonce, body) = sealed.split_at(NONCE_BYTES);
        model::open(&self.bytes, nonce, aad, body).map_err(map_model_error)
    }
}

fn map_model_error(error: model::Error) -> Error {
    match error {
        model::Error::BadKeyLength => Error::InvalidKeyLength,
        model::Error::BadNonceLength => Error::InvalidNonceLength,
        model::Error::Authentication => Error::Authentication,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_must_be_exactly_32_bytes() {
        assert!(ChaChaPolyKey::from_bytes(&[1u8; 32]).is_ok());
        assert_eq!(ChaChaPolyKey::from_bytes(&[1u8; 16]), Err(Error::InvalidKeyLength));
        assert_eq!(ChaChaPolyKey::from_bytes(&[1u8; 33]), Err(Error::InvalidKeyLength));
    }

    #[test]
    fn generated_keys_are_32_bytes_and_differ() {
        let a = ChaChaPolyKey::generate().unwrap();
        let b = ChaChaPolyKey::generate().unwrap();
        assert_eq!(a.bytes().len(), 32);
        assert_ne!(a, b);
    }

    #[test]
    fn debug_output_never_shows_the_key_bytes() {
        let key = ChaChaPolyKey::from_bytes(&[0xABu8; 32]).unwrap();
        assert_eq!(format!("{key:?}"), "ChaChaPolyKey(256 bits)");
    }

    #[test]
    fn a_sealed_box_opens_to_the_original_text_with_the_same_aad() {
        let key = ChaChaPolyKey::from_bytes(&[7u8; 32]).unwrap();
        let sealed = key.seal(b"stored secret", b"account-1").unwrap();
        assert_eq!(sealed.len(), NONCE_BYTES + "stored secret".len() + TAG_BYTES);
        assert_eq!(key.open(&sealed, b"account-1").unwrap(), b"stored secret".to_vec());
    }

    #[test]
    fn two_seals_of_the_same_text_differ_because_the_nonce_is_fresh() {
        let key = ChaChaPolyKey::from_bytes(&[7u8; 32]).unwrap();
        assert_ne!(key.seal(b"same", b"").unwrap(), key.seal(b"same", b"").unwrap());
    }

    #[test]
    fn a_changed_byte_or_different_additional_data_is_refused() {
        let key = ChaChaPolyKey::from_bytes(&[7u8; 32]).unwrap();
        let mut sealed = key.seal(b"payload", b"ctx").unwrap();
        assert_eq!(key.open(&sealed, b"other"), Err(Error::Authentication));
        let last = sealed.len() - 1;
        sealed[last] ^= 0x01;
        assert_eq!(key.open(&sealed, b"ctx"), Err(Error::Authentication));
    }

    #[test]
    fn another_key_cannot_open_the_box() {
        let key = ChaChaPolyKey::from_bytes(&[7u8; 32]).unwrap();
        let other = ChaChaPolyKey::from_bytes(&[8u8; 32]).unwrap();
        let sealed = key.seal(b"payload", b"").unwrap();
        assert_eq!(other.open(&sealed, b""), Err(Error::Authentication));
    }

    #[test]
    fn a_box_shorter_than_a_nonce_and_a_tag_is_invalid() {
        let key = ChaChaPolyKey::from_bytes(&[7u8; 32]).unwrap();
        assert_eq!(key.open(&[0u8; 27], b""), Err(Error::InvalidBox));
        assert_eq!(key.open(&[0u8; 28], b""), Err(Error::Authentication));
    }

    #[test]
    fn a_nonce_of_the_wrong_length_is_refused() {
        let key = ChaChaPolyKey::from_bytes(&[7u8; 32]).unwrap();
        assert_eq!(key.seal_with_nonce(&[0u8; 8], b"x", b""), Err(Error::InvalidNonceLength));
    }

    #[test]
    fn the_sealed_box_starts_with_the_nonce_used() {
        let key = ChaChaPolyKey::from_bytes(&[7u8; 32]).unwrap();
        let nonce = [9u8; 12];
        let sealed = key.seal_with_nonce(&nonce, b"abc", b"").unwrap();
        assert_eq!(&sealed[..12], &nonce);
        assert_eq!(key.open(&sealed, b"").unwrap(), b"abc".to_vec());
    }
}
