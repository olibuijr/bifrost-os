//! Post-quantum key encapsulation for apps: ML-KEM-768 and ML-KEM-1024 (FIPS 203), the parameter sets CryptoKit
//! exposes as `MLKEM768` and `MLKEM1024`. The arithmetic lives in `bifrost_core::mlkem`; this module draws the random
//! seeds from the kernel pool ([`crate::entropy`]) and gives the bytes an object shape.
//!
//! An app generates a [`DecapsulationKey`], publishes its [`EncapsulationKey`] bytes, and the sender calls
//! [`EncapsulationKey::encapsulate`] to get a shared secret and a ciphertext. The receiver calls
//! [`DecapsulationKey::decapsulate`]. A tampered ciphertext does not fail: it yields a different secret (implicit
//! rejection), so the caller must check the secret with a later authenticated step, not with this call.
//!
//! Not here: X-Wing (ML-KEM with X25519), ML-DSA signatures, and the official FIPS 203 known-answer vectors, which are
//! not in the repo yet; see the `Post-quantum KEM` row in `docs/parity-matrix/data/apple-cryptokit.json`.

use crate::entropy;
use bifrost_core::mlkem::{self, Error as CoreError};

/// The parameter set: `MLKEM768` or `MLKEM1024`.
pub use bifrost_core::mlkem::Params as Variant;

/// The 32-byte shared secret that both sides of an encapsulation agree on.
pub type SharedSecret = [u8; 32];

/// Why a KEM call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KemError {
    /// An encapsulation or decapsulation key has the wrong length for its variant.
    IncorrectKeySize,
    /// The ciphertext has the wrong length for its variant.
    IncorrectCiphertextSize,
    /// An encapsulation key holds a coefficient that is not below the modulus q (FIPS 203 modulus check).
    InvalidEncapsulationKey,
    /// The decapsulation key does not match the hash of the encapsulation key it contains.
    InvalidDecapsulationKey,
    /// The random pool could not supply the seeds.
    Unavailable,
}

impl From<CoreError> for KemError {
    fn from(e: CoreError) -> KemError {
        match e {
            CoreError::EncapsulationKeyLength | CoreError::DecapsulationKeyLength => KemError::IncorrectKeySize,
            CoreError::CiphertextLength => KemError::IncorrectCiphertextSize,
            CoreError::EncapsulationKeyModulus => KemError::InvalidEncapsulationKey,
            CoreError::DecapsulationKeyHash => KemError::InvalidDecapsulationKey,
        }
    }
}

/// The result of [`EncapsulationKey::encapsulate`]: the secret for the sender and the ciphertext to send.
#[derive(Clone, PartialEq, Eq)]
pub struct Encapsulation {
    pub shared_secret: SharedSecret,
    pub ciphertext: Vec<u8>,
}

/// A public encapsulation key. Its bytes are not secret, so `Debug` shows them in full only through [`Self::bytes`].
#[derive(Clone, PartialEq, Eq)]
pub struct EncapsulationKey {
    variant: Variant,
    bytes: Vec<u8>,
}

impl EncapsulationKey {
    /// A key from bytes received from a peer. The length and the modulus check run here, once.
    pub fn from_bytes(variant: Variant, bytes: &[u8]) -> Result<EncapsulationKey, KemError> {
        mlkem::check_encapsulation_key(variant, bytes)?;
        Ok(EncapsulationKey { variant, bytes: bytes.to_vec() })
    }

    pub fn variant(&self) -> Variant {
        self.variant
    }

    /// The FIPS 203 encoding of the key: 1184 bytes for ML-KEM-768, 1568 for ML-KEM-1024.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Encapsulates to this key with fresh randomness from the kernel pool.
    pub fn encapsulate(&self) -> Result<Encapsulation, KemError> {
        let mut m = [0u8; 32];
        entropy::copy_bytes(&mut m).map_err(|_| KemError::Unavailable)?;
        let (shared_secret, ciphertext) = mlkem::encapsulate(self.variant, &self.bytes, &m)?;
        Ok(Encapsulation { shared_secret, ciphertext })
    }
}

/// A private decapsulation key. `Debug` shows the variant only, never the key bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct DecapsulationKey {
    variant: Variant,
    bytes: Vec<u8>,
}

impl std::fmt::Debug for DecapsulationKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DecapsulationKey({:?})", self.variant)
    }
}

impl DecapsulationKey {
    /// A new key pair's private half: the seeds `d` and `z` come from the kernel pool.
    pub fn generate(variant: Variant) -> Result<DecapsulationKey, KemError> {
        let mut seeds = [0u8; 64];
        entropy::copy_bytes(&mut seeds).map_err(|_| KemError::Unavailable)?;
        let mut d = [0u8; 32];
        let mut z = [0u8; 32];
        d.copy_from_slice(&seeds[..32]);
        z.copy_from_slice(&seeds[32..]);
        Ok(Self::from_seeds(variant, &d, &z))
    }

    /// The key that the FIPS 203 seeds `d` and `z` produce; the same seeds always give the same key.
    pub fn from_seeds(variant: Variant, d: &[u8; 32], z: &[u8; 32]) -> DecapsulationKey {
        let kp = mlkem::keygen(variant, d, z);
        DecapsulationKey { variant, bytes: kp.decapsulation_key }
    }

    /// A key from its stored bytes, for example one read back from the keychain. Only the length is checked here; a
    /// damaged hash is found by [`Self::decapsulate`].
    pub fn from_bytes(variant: Variant, bytes: &[u8]) -> Result<DecapsulationKey, KemError> {
        if bytes.len() != variant.decapsulation_key_len() {
            return Err(KemError::IncorrectKeySize);
        }
        Ok(DecapsulationKey { variant, bytes: bytes.to_vec() })
    }

    pub fn variant(&self) -> Variant {
        self.variant
    }

    /// The FIPS 203 encoding of the private key. Store it only in the keychain or in app memory.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The public half, to publish to peers.
    pub fn encapsulation_key(&self) -> EncapsulationKey {
        let split = self.variant.encapsulation_key_len();
        let start = self.bytes.len() - 64 - split;
        EncapsulationKey { variant: self.variant, bytes: self.bytes[start..start + split].to_vec() }
    }

    /// Recovers the shared secret from a ciphertext. A ciphertext that was altered gives a different secret, not an
    /// error; see the module notes.
    pub fn decapsulate(&self, ciphertext: &[u8]) -> Result<SharedSecret, KemError> {
        Ok(mlkem::decapsulate(self.variant, &self.bytes, ciphertext)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: [Variant; 2] = [Variant::MlKem768, Variant::MlKem1024];

    #[test]
    fn generated_key_sizes_match_fips_203() {
        for v in BOTH {
            let dk = DecapsulationKey::generate(v).unwrap();
            assert_eq!(dk.bytes().len(), v.decapsulation_key_len());
            let ek = dk.encapsulation_key();
            assert_eq!(ek.bytes().len(), v.encapsulation_key_len());
            assert_eq!(EncapsulationKey::from_bytes(v, ek.bytes()).unwrap(), ek);
        }
    }

    #[test]
    fn random_round_trip_agrees_on_the_secret() {
        for v in BOTH {
            let dk = DecapsulationKey::generate(v).unwrap();
            let ek = dk.encapsulation_key();
            let sent = ek.encapsulate().unwrap();
            assert_eq!(sent.ciphertext.len(), v.ciphertext_len());
            assert_eq!(dk.decapsulate(&sent.ciphertext).unwrap(), sent.shared_secret);
        }
    }

    #[test]
    fn seeded_keys_are_reproducible_and_round_trip() {
        let d = [0x21u8; 32];
        let z = [0x42u8; 32];
        let a = DecapsulationKey::from_seeds(Variant::MlKem768, &d, &z);
        let b = DecapsulationKey::from_seeds(Variant::MlKem768, &d, &z);
        assert_eq!(a, b);
        let ek = a.encapsulation_key();
        let mut m_ct = mlkem::encapsulate(Variant::MlKem768, ek.bytes(), &[0x55u8; 32]).unwrap();
        assert_eq!(a.decapsulate(&m_ct.1).unwrap(), m_ct.0);
        m_ct.1[3] ^= 0x10;
        assert_ne!(a.decapsulate(&m_ct.1).unwrap(), m_ct.0);
    }

    #[test]
    fn wrong_lengths_are_refused() {
        assert_eq!(DecapsulationKey::from_bytes(Variant::MlKem768, &[0u8; 10]), Err(KemError::IncorrectKeySize));
        assert_eq!(EncapsulationKey::from_bytes(Variant::MlKem1024, &[0u8; 10]), Err(KemError::IncorrectKeySize));
        let dk = DecapsulationKey::generate(Variant::MlKem1024).unwrap();
        assert_eq!(dk.decapsulate(&[0u8; 10]), Err(KemError::IncorrectCiphertextSize));
    }

    #[test]
    fn debug_hides_key_bytes() {
        let dk = DecapsulationKey::generate(Variant::MlKem768).unwrap();
        assert_eq!(format!("{:?}", dk), "DecapsulationKey(MlKem768)");
    }
}
