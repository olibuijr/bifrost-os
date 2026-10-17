//! CryptoKit for apps: the digests, key derivation, authenticated encryption, key agreement and signatures an app needs,
//! built on the shell's own pure Rust primitives (`bifrost_core` for the hashes, HKDF and the ciphers, `bifrost_ui` for
//! Curve25519 and Ed25519). Nothing here uses the network or hardware, and every random key or nonce comes from the
//! shell's random pool.
//!
//! What is here, grouped as the framework groups it:
//! - digests: [`sha256`], [`sha384`], [`sha512`]. HMAC-SHA256 is in [`digest`](super::digest).
//! - key derivation: [`hkdf_sha256`] (RFC 5869).
//! - authenticated encryption: [`AesGcmKey`] (AES-128 or AES-256 in GCM mode). ChaCha20-Poly1305 is in
//!   [`chacha_poly`](super::chacha_poly).
//! - key agreement: [`KeyAgreementKey`] (X25519).
//! - signatures: [`SigningKey`] and [`verify_signature`] (Ed25519).
//!
//! Sealed boxes from [`AesGcmKey::seal`] are the 12-byte nonce, then the ciphertext, then the 16-byte tag, the same
//! layout as [`ChaChaPolyKey`](super::chacha_poly::ChaChaPolyKey).

use bifrost_core::{aes_gcm, hkdf, sha256 as core_sha256, sha512 as core_sha512};
use bifrost_ui::{ed25519, x25519};

use crate::entropy;

/// Why a CryptoKit call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// A key is not the length its type takes.
    InvalidKeyLength,
    /// A sealed box is shorter than a nonce and a tag.
    InvalidBox,
    /// The data does not match its tag, the additional data or the key.
    Authentication,
    /// The random pool could not supply bytes.
    Unavailable,
    /// The message is longer than AES-GCM allows.
    TooLong,
    /// The requested HKDF output is longer than 255 hash blocks.
    OutputTooLong,
    /// The peer's public key has small order, so the agreed secret would be all zero.
    LowOrderPoint,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Error::InvalidKeyLength => "key has the wrong length",
            Error::InvalidBox => "sealed box is shorter than a nonce and a tag",
            Error::Authentication => "authentication tag does not match",
            Error::Unavailable => "the random pool could not supply bytes",
            Error::TooLong => "message is longer than AES-GCM allows",
            Error::OutputTooLong => "HKDF output is longer than 255 hash blocks",
            Error::LowOrderPoint => "peer key has small order",
        };
        f.write_str(text)
    }
}

impl std::error::Error for Error {}

/// SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    core_sha256::digest(data)
}

/// SHA-384 of `data`.
pub fn sha384(data: &[u8]) -> [u8; 48] {
    core_sha512::digest384(data)
}

/// SHA-512 of `data`.
pub fn sha512(data: &[u8]) -> [u8; 64] {
    core_sha512::digest512(data)
}

/// HKDF with SHA-256 (RFC 5869): `len` bytes of key material from input keying material, a salt and a context label.
/// An empty salt is the same as a salt of zeros, as the RFC defines it.
pub fn hkdf_sha256(ikm: &[u8], salt: &[u8], info: &[u8], len: usize) -> Result<Vec<u8>, Error> {
    hkdf::hkdf_sha256(ikm, salt, info, len).ok_or(Error::OutputTooLong)
}

/// An AES key for GCM: 16 bytes for AES-128 or 32 bytes for AES-256. `Debug` shows the size only.
#[derive(Clone, PartialEq, Eq)]
pub struct AesGcmKey {
    bytes: Vec<u8>,
}

impl std::fmt::Debug for AesGcmKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AesGcmKey({} bits)", self.bytes.len() * 8)
    }
}

impl AesGcmKey {
    /// A new random key of `len` bytes (16 or 32).
    pub fn generate(len: usize) -> Result<AesGcmKey, Error> {
        AesGcmKey::from_bytes(&entropy::bytes(len).map_err(|_| Error::Unavailable)?)
    }

    /// A key from existing bytes, which must be 16 or 32 long.
    pub fn from_bytes(bytes: &[u8]) -> Result<AesGcmKey, Error> {
        match bytes.len() {
            16 | 32 => Ok(AesGcmKey { bytes: bytes.to_vec() }),
            _ => Err(Error::InvalidKeyLength),
        }
    }

    /// Encrypts `plain` under a fresh random nonce and returns the sealed box (nonce, ciphertext, tag).
    pub fn seal(&self, plain: &[u8], aad: &[u8]) -> Result<Vec<u8>, Error> {
        let nonce = entropy::bytes(aes_gcm::NONCE_LEN).map_err(|_| Error::Unavailable)?;
        let sealed = aes_gcm::seal(&self.bytes, &nonce, aad, plain).map_err(map_error)?;
        let mut out = nonce;
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    /// Checks the tag of a sealed box and returns the plaintext. A wrong key, a changed byte or different additional
    /// data fails with [`Error::Authentication`] and yields no plaintext.
    pub fn open(&self, sealed: &[u8], aad: &[u8]) -> Result<Vec<u8>, Error> {
        if sealed.len() < aes_gcm::NONCE_LEN + aes_gcm::TAG_LEN {
            return Err(Error::InvalidBox);
        }
        let (nonce, body) = sealed.split_at(aes_gcm::NONCE_LEN);
        aes_gcm::open(&self.bytes, nonce, aad, body).map_err(map_error)
    }
}

fn map_error(error: aes_gcm::Error) -> Error {
    match error {
        aes_gcm::Error::BadKeyLength | aes_gcm::Error::BadNonceLength => Error::InvalidKeyLength,
        aes_gcm::Error::TooLong => Error::TooLong,
        aes_gcm::Error::Authentication => Error::Authentication,
    }
}

/// An X25519 private key for key agreement. `Debug` never shows the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct KeyAgreementKey {
    inner: x25519::PrivateKey,
}

impl std::fmt::Debug for KeyAgreementKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyAgreementKey(X25519)")
    }
}

impl KeyAgreementKey {
    /// A new random key from the random pool.
    pub fn generate() -> Result<KeyAgreementKey, Error> {
        let bytes = entropy::bytes(32).map_err(|_| Error::Unavailable)?;
        KeyAgreementKey::from_bytes(&bytes)
    }

    /// A key from 32 existing bytes, for example one read back from the keychain. The scalar is clamped when used.
    pub fn from_bytes(bytes: &[u8]) -> Result<KeyAgreementKey, Error> {
        let inner = x25519::PrivateKey::from_raw(bytes).map_err(|_| Error::InvalidKeyLength)?;
        Ok(KeyAgreementKey { inner })
    }

    /// The public key to send to the other side: 32 bytes.
    pub fn public_key(&self) -> [u8; 32] {
        self.inner.public_key().raw()
    }

    /// The shared secret with a peer's public key: 32 bytes that both sides compute to the same value. Hash it with
    /// [`hkdf_sha256`] before using it as a key.
    pub fn agree(&self, peer_public_key: &[u8]) -> Result<[u8; 32], Error> {
        let peer = x25519::PublicKey::from_raw(peer_public_key).map_err(|_| Error::InvalidKeyLength)?;
        let secret = self.inner.shared_secret(&peer).map_err(|_| Error::LowOrderPoint)?;
        Ok(*secret.bytes())
    }
}

/// An Ed25519 signing key. `Debug` never shows the bytes.
#[derive(Clone)]
pub struct SigningKey {
    inner: ed25519::PrivateKey,
}

impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SigningKey(Ed25519)")
    }
}

impl SigningKey {
    /// A new random key from the random pool.
    pub fn generate() -> Result<SigningKey, Error> {
        let inner = ed25519::PrivateKey::generate().map_err(|_| Error::Unavailable)?;
        Ok(SigningKey { inner })
    }

    /// A key from its 32-byte seed.
    pub fn from_bytes(seed: &[u8]) -> Result<SigningKey, Error> {
        let inner = ed25519::PrivateKey::from_raw_representation(seed).map_err(|_| Error::InvalidKeyLength)?;
        Ok(SigningKey { inner })
    }

    /// The 32-byte public key that verifies this key's signatures.
    pub fn public_key(&self) -> [u8; 32] {
        self.inner.public_key().raw_representation()
    }

    /// The 64-byte signature of `data`. Ed25519 is deterministic: the same key and data give the same signature.
    pub fn sign(&self, data: &[u8]) -> [u8; 64] {
        self.inner.signature_for(data)
    }
}

/// Checks an Ed25519 signature against a 32-byte public key. A signature of the wrong length or from another key is
/// `Ok(false)`; a public key of the wrong length is an error.
pub fn verify_signature(public_key: &[u8], signature: &[u8], data: &[u8]) -> Result<bool, Error> {
    let key = ed25519::PublicKey::from_raw_representation(public_key).map_err(|_| Error::InvalidKeyLength)?;
    Ok(key.is_valid_signature(signature, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn digests_match_the_published_test_vectors() {
        assert_eq!(
            hex(&sha384(b"abc")),
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
        );
        assert_eq!(
            hex(&sha512(b"abc")),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        assert_eq!(sha256(b"abc")[0], 0xba);
    }

    #[test]
    fn hkdf_matches_rfc_5869_test_case_1() {
        let ikm = [0x0bu8; 22];
        let salt: Vec<u8> = (0x00..=0x0c).collect();
        let info: Vec<u8> = (0xf0..=0xf9).collect();
        let okm = hkdf_sha256(&ikm, &salt, &info, 42).unwrap();
        assert_eq!(
            hex(&okm),
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
        );
    }

    #[test]
    fn hkdf_refuses_an_output_longer_than_255_blocks() {
        assert_eq!(hkdf_sha256(b"k", b"", b"", 255 * 32 + 1), Err(Error::OutputTooLong));
    }

    #[test]
    fn aes_gcm_keys_are_16_or_32_bytes() {
        assert!(AesGcmKey::from_bytes(&[1u8; 16]).is_ok());
        assert!(AesGcmKey::from_bytes(&[1u8; 32]).is_ok());
        assert_eq!(AesGcmKey::from_bytes(&[1u8; 24]), Err(Error::InvalidKeyLength));
        assert_eq!(AesGcmKey::generate(20), Err(Error::InvalidKeyLength));
    }

    #[test]
    fn aes_gcm_round_trips_and_refuses_tampering() {
        for len in [16usize, 32] {
            let key = AesGcmKey::generate(len).unwrap();
            let sealed = key.seal(b"stored secret", b"ctx").unwrap();
            assert_eq!(sealed.len(), aes_gcm::NONCE_LEN + "stored secret".len() + aes_gcm::TAG_LEN);
            assert_eq!(key.open(&sealed, b"ctx").unwrap(), b"stored secret".to_vec());
            assert_eq!(key.open(&sealed, b"other"), Err(Error::Authentication));
            let mut bad = sealed.clone();
            let last = bad.len() - 1;
            bad[last] ^= 1;
            assert_eq!(key.open(&bad, b"ctx"), Err(Error::Authentication));
        }
    }

    #[test]
    fn aes_gcm_boxes_are_fresh_and_short_boxes_are_invalid() {
        let key = AesGcmKey::from_bytes(&[3u8; 16]).unwrap();
        assert_ne!(key.seal(b"same", b"").unwrap(), key.seal(b"same", b"").unwrap());
        assert_eq!(key.open(&[0u8; 27], b""), Err(Error::InvalidBox));
    }

    #[test]
    fn key_agreement_matches_rfc_7748_and_is_symmetric() {
        let alice = KeyAgreementKey::from_bytes(
            &hex_to_bytes("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a"),
        )
        .unwrap();
        let bob = KeyAgreementKey::from_bytes(
            &hex_to_bytes("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb"),
        )
        .unwrap();
        assert_eq!(hex(&alice.public_key()), "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a");
        assert_eq!(hex(&bob.public_key()), "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f");
        let expected = "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742";
        assert_eq!(hex(&alice.agree(&bob.public_key()).unwrap()), expected);
        assert_eq!(hex(&bob.agree(&alice.public_key()).unwrap()), expected);
    }

    #[test]
    fn key_agreement_refuses_a_zero_public_key_and_bad_lengths() {
        let key = KeyAgreementKey::generate().unwrap();
        assert_eq!(key.agree(&[0u8; 32]), Err(Error::LowOrderPoint));
        assert_eq!(key.agree(&[1u8; 31]), Err(Error::InvalidKeyLength));
        assert_eq!(KeyAgreementKey::from_bytes(&[1u8; 16]), Err(Error::InvalidKeyLength));
    }

    #[test]
    fn signatures_verify_with_the_matching_public_key_only() {
        let key = SigningKey::generate().unwrap();
        let other = SigningKey::generate().unwrap();
        let sig = key.sign(b"release 1.2");
        assert_eq!(verify_signature(&key.public_key(), &sig, b"release 1.2"), Ok(true));
        assert_eq!(verify_signature(&key.public_key(), &sig, b"release 1.3"), Ok(false));
        assert_eq!(verify_signature(&other.public_key(), &sig, b"release 1.2"), Ok(false));
        assert_eq!(verify_signature(&key.public_key(), &sig[..63], b"release 1.2"), Ok(false));
        assert_eq!(verify_signature(&[0u8; 31], &sig, b""), Err(Error::InvalidKeyLength));
    }

    #[test]
    fn signing_is_deterministic_for_one_seed() {
        let a = SigningKey::from_bytes(&[5u8; 32]).unwrap();
        let b = SigningKey::from_bytes(&[5u8; 32]).unwrap();
        assert_eq!(a.sign(b"x"), b.sign(b"x"));
        assert_eq!(SigningKey::from_bytes(&[5u8; 31]).err(), Some(Error::InvalidKeyLength));
    }

    #[test]
    fn debug_output_never_shows_key_bytes() {
        assert_eq!(format!("{:?}", AesGcmKey::from_bytes(&[9u8; 32]).unwrap()), "AesGcmKey(256 bits)");
        assert_eq!(format!("{:?}", KeyAgreementKey::from_bytes(&[9u8; 32]).unwrap()), "KeyAgreementKey(X25519)");
    }

    fn hex_to_bytes(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }
}
