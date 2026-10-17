//! Post-quantum signatures for apps: ML-DSA-65 and ML-DSA-87 (FIPS 204). The arithmetic lives in
//! `bifrost_core::mldsa`; this module draws the random seeds from the kernel pool ([`crate::entropy`]) and gives the
//! bytes an object shape.
//!
//! An app generates a [`PrivateKey`], publishes its [`PublicKey`] bytes, and signs with [`PrivateKey::sign`] (hedged:
//! fresh randomness per signature) or [`PrivateKey::sign_deterministic`] (the same signature for the same message).
//! A verifier calls [`PublicKey::is_valid_signature`]. The context string is up to 255 bytes and separates uses of one
//! key; a signature made under one context does not verify under another.
//!
//! Not here: the official FIPS 204 known-answer vectors, which are not in the repo yet; see the `Post-quantum KEM and
//! signatures` row in `docs/parity-matrix/data/apple-cryptokit.json`.

use crate::entropy;
use bifrost_core::mldsa::{self, Error as CoreError};

/// The parameter set: ML-DSA-65 or ML-DSA-87.
pub use bifrost_core::mldsa::Params as Variant;

/// Why a signature call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureError {
    /// A public or private key has the wrong length for its variant.
    IncorrectKeySize,
    /// The context string is longer than 255 bytes.
    ContextTooLong,
    /// The random pool could not supply the signing randomness or the key seed.
    Unavailable,
}

impl From<CoreError> for SignatureError {
    fn from(e: CoreError) -> SignatureError {
        match e {
            CoreError::PublicKeyLength | CoreError::SecretKeyLength => SignatureError::IncorrectKeySize,
            CoreError::ContextTooLong => SignatureError::ContextTooLong,
        }
    }
}

/// A public verification key. Its bytes are not secret.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicKey {
    variant: Variant,
    bytes: Vec<u8>,
}

impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PublicKey({:?}, {} bytes)", self.variant, self.bytes.len())
    }
}

impl PublicKey {
    /// A key from bytes received from a peer. The length is checked here.
    pub fn from_bytes(variant: Variant, bytes: &[u8]) -> Result<PublicKey, SignatureError> {
        if bytes.len() != variant.public_key_len() {
            return Err(SignatureError::IncorrectKeySize);
        }
        Ok(PublicKey { variant, bytes: bytes.to_vec() })
    }

    pub fn variant(&self) -> Variant {
        self.variant
    }

    /// The FIPS 204 encoding of the key: 1952 bytes for ML-DSA-65, 2592 for ML-DSA-87.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Whether `signature` is a valid signature of `message` under `context` by this key. A malformed signature, a
    /// context longer than 255 bytes and a wrong message all give `false`; nothing here is an error.
    pub fn is_valid_signature(&self, signature: &[u8], message: &[u8], context: &[u8]) -> bool {
        mldsa::verify(self.variant, &self.bytes, message, context, signature).unwrap_or(false)
    }
}

/// A private signing key. `Debug` shows the variant only, never the key bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct PrivateKey {
    variant: Variant,
    bytes: Vec<u8>,
    public: Vec<u8>,
}

impl std::fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PrivateKey({:?})", self.variant)
    }
}

impl PrivateKey {
    /// A new key pair's private half: the 32-byte seed `xi` comes from the kernel pool.
    pub fn generate(variant: Variant) -> Result<PrivateKey, SignatureError> {
        let mut xi = [0u8; 32];
        entropy::copy_bytes(&mut xi).map_err(|_| SignatureError::Unavailable)?;
        Ok(Self::from_seed(variant, &xi))
    }

    /// The key that the FIPS 204 seed `xi` produces; the same seed always gives the same key.
    pub fn from_seed(variant: Variant, xi: &[u8; 32]) -> PrivateKey {
        let kp = mldsa::keygen(variant, xi);
        PrivateKey { variant, bytes: kp.secret_key, public: kp.public_key }
    }

    /// A key from its stored bytes, for example one read back from the keychain. The length is checked and the public
    /// half is recomputed from the secret bytes.
    pub fn from_bytes(variant: Variant, bytes: &[u8]) -> Result<PrivateKey, SignatureError> {
        let public = mldsa::public_key_from_secret_key(variant, bytes)?;
        Ok(PrivateKey { variant, bytes: bytes.to_vec(), public })
    }

    pub fn variant(&self) -> Variant {
        self.variant
    }

    /// The FIPS 204 encoding of the private key. Store it only in the keychain or in app memory.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The public half, to publish to verifiers.
    pub fn public_key(&self) -> PublicKey {
        PublicKey { variant: self.variant, bytes: self.public.clone() }
    }

    /// A hedged signature: the signing randomness comes fresh from the kernel pool, so two signatures of one message
    /// differ. Both verify.
    pub fn sign(&self, message: &[u8], context: &[u8]) -> Result<Vec<u8>, SignatureError> {
        let mut rnd = [0u8; 32];
        entropy::copy_bytes(&mut rnd).map_err(|_| SignatureError::Unavailable)?;
        Ok(mldsa::sign(self.variant, &self.bytes, message, context, &rnd)?)
    }

    /// A deterministic signature (FIPS 204 with zero signing randomness): the same key, message and context always
    /// give the same bytes.
    pub fn sign_deterministic(&self, message: &[u8], context: &[u8]) -> Result<Vec<u8>, SignatureError> {
        Ok(mldsa::sign(self.variant, &self.bytes, message, context, &[0u8; 32])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: [Variant; 2] = [Variant::MlDsa65, Variant::MlDsa87];

    #[test]
    fn generated_key_sizes_match_fips_204() {
        for v in BOTH {
            let sk = PrivateKey::generate(v).unwrap();
            assert_eq!(sk.bytes().len(), v.secret_key_len());
            let pk = sk.public_key();
            assert_eq!(pk.bytes().len(), v.public_key_len());
            assert_eq!(PublicKey::from_bytes(v, pk.bytes()).unwrap(), pk);
        }
    }

    #[test]
    fn random_sign_and_verify_round_trip() {
        for v in BOTH {
            let sk = PrivateKey::generate(v).unwrap();
            let pk = sk.public_key();
            let sig = sk.sign(b"invoice 42", b"billing").unwrap();
            assert_eq!(sig.len(), v.signature_len());
            assert!(pk.is_valid_signature(&sig, b"invoice 42", b"billing"));
            assert!(!pk.is_valid_signature(&sig, b"invoice 43", b"billing"));
            assert!(!pk.is_valid_signature(&sig, b"invoice 42", b"shipping"));
        }
    }

    #[test]
    fn deterministic_signatures_are_reproducible() {
        for v in BOTH {
            let sk = PrivateKey::from_seed(v, &[0x5au8; 32]);
            let a = sk.sign_deterministic(b"msg", b"ctx").unwrap();
            let b = PrivateKey::from_seed(v, &[0x5au8; 32]).sign_deterministic(b"msg", b"ctx").unwrap();
            assert_eq!(a, b);
            assert!(sk.public_key().is_valid_signature(&a, b"msg", b"ctx"));
        }
    }

    #[test]
    fn tampered_signature_and_long_context_are_refused() {
        let sk = PrivateKey::from_seed(Variant::MlDsa65, &[0x11u8; 32]);
        let pk = sk.public_key();
        let mut sig = sk.sign_deterministic(b"msg", b"ctx").unwrap();
        sig[10] ^= 0x01;
        assert!(!pk.is_valid_signature(&sig, b"msg", b"ctx"));
        let long = [0u8; 256];
        assert_eq!(sk.sign(b"msg", &long), Err(SignatureError::ContextTooLong));
        assert!(!pk.is_valid_signature(&[], b"msg", &long));
    }

    #[test]
    fn wrong_lengths_are_refused() {
        assert_eq!(PublicKey::from_bytes(Variant::MlDsa87, &[0u8; 10]), Err(SignatureError::IncorrectKeySize));
        assert_eq!(PrivateKey::from_bytes(Variant::MlDsa65, &[0u8; 10]).map(|_| ()), Err(SignatureError::IncorrectKeySize));
    }

    #[test]
    fn stored_private_key_recomputes_its_public_half() {
        let sk = PrivateKey::from_seed(Variant::MlDsa87, &[0x33u8; 32]);
        let back = PrivateKey::from_bytes(Variant::MlDsa87, sk.bytes()).unwrap();
        assert_eq!(back.public_key(), sk.public_key());
    }

    #[test]
    fn debug_hides_key_bytes() {
        let sk = PrivateKey::from_seed(Variant::MlDsa65, &[0x44u8; 32]);
        assert_eq!(format!("{:?}", sk), "PrivateKey(MlDsa65)");
    }
}
