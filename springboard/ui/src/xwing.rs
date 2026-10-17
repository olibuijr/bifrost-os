//! X-Wing: a hybrid key encapsulation mechanism that combines ML-KEM-768 (FIPS 203) with X25519, following the IETF
//! draft `draft-connolly-cfrg-xwing-kem`. The secret stays safe while either component holds.
//!
//! The decapsulation key is a 32-byte seed `sk`. SHAKE256(`sk`, 96 bytes) splits into `d` and `z` (the ML-KEM-768
//! key generation seeds, 64 bytes) and the X25519 secret `sk_X` (32 bytes). The encapsulation key is
//! `pk_M || pk_X` (1184 + 32 = 1216 bytes). Encapsulation takes 64 random bytes: `m` for ML-KEM and the X25519
//! ephemeral scalar `ek_X`. The ciphertext is `ct_M || ct_X` (1088 + 32 = 1120 bytes). The shared secret is
//! SHA3-256(`ss_M || ss_X || ct_X || pk_X || label`), where the label is the six bytes `\./` followed by `/^\`.
//!
//! The functions take their seeds and randomness from the caller, so this module draws no randomness; the SDK fills
//! them from the kernel pool (`bifrost_app::kem::xwing`, in `springboard/sdk`).
//!
//! Status: the wiring is checked by round trips and by tampering tests, not by draft test vectors, which are not in
//! the repo. Interoperability with other X-Wing implementations is unverified until those vectors pass.

use crate::x25519;
use bifrost_core::mlkem::{self, Error as MlkemError, Params};
use bifrost_core::sha3::{sha3_256, Shake};

/// Length of a decapsulation key seed.
pub const SEED_LEN: usize = 32;
/// Length of an encapsulation key: ML-KEM-768 `pk_M` (1184 bytes) then X25519 `pk_X` (32 bytes).
pub const PUBLIC_KEY_LEN: usize = 1216;
/// Length of a ciphertext: ML-KEM-768 `ct_M` (1088 bytes) then X25519 `ct_X` (32 bytes).
pub const CIPHERTEXT_LEN: usize = 1120;
/// Length of the randomness for one encapsulation: 32 bytes for `m`, then 32 bytes for `ek_X`.
pub const ENCAPSULATION_SEED_LEN: usize = 64;
/// Length of the shared secret.
pub const SHARED_SECRET_LEN: usize = 32;

/// ML-KEM-768 encapsulation key length, the split point inside the X-Wing key and ciphertext.
const MLKEM_PK_LEN: usize = 1184;
/// ML-KEM-768 ciphertext length, the split point inside the X-Wing ciphertext.
const MLKEM_CT_LEN: usize = 1088;
/// The combiner label `\./` followed by `/^\`: the bytes 5c 2e 2f 2f 5e 5c.
pub const LABEL: [u8; 6] = [0x5c, 0x2e, 0x2f, 0x2f, 0x5e, 0x5c];

/// Why an X-Wing call was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// A key has the wrong length.
    KeyLength,
    /// The ciphertext has the wrong length.
    CiphertextLength,
    /// The ML-KEM-768 part failed its FIPS 203 check (modulus or length).
    Mlkem(MlkemError),
    /// The X25519 peer key has small order, so the agreement is all zero.
    LowOrderPoint,
}

/// SHAKE256 expansion of the seed into `(d, z, sk_X)`.
fn expand(seed: &[u8; SEED_LEN]) -> ([u8; 32], [u8; 32], [u8; 32]) {
    let mut out = [0u8; 96];
    let mut xof = Shake::new_256();
    xof.update(seed);
    xof.squeeze(&mut out);
    (arr32(&out[..32]), arr32(&out[32..64]), arr32(&out[64..]))
}

fn arr32(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(bytes);
    out
}

/// X25519 agreement that refuses an all-zero result (a small-order peer key).
fn agree(scalar: &[u8; 32], point: &[u8; 32]) -> Result<[u8; 32], Error> {
    let out = x25519::x25519(scalar, point);
    if out.iter().fold(0u8, |acc, b| acc | b) == 0 {
        return Err(Error::LowOrderPoint);
    }
    Ok(out)
}

/// The X-Wing combiner: SHA3-256 over the two component secrets, the X25519 ciphertext and public key, and the label.
fn combine(ss_m: &[u8; 32], ss_x: &[u8; 32], ct_x: &[u8; 32], pk_x: &[u8; 32]) -> [u8; 32] {
    let mut input = Vec::with_capacity(4 * 32 + LABEL.len());
    input.extend_from_slice(ss_m);
    input.extend_from_slice(ss_x);
    input.extend_from_slice(ct_x);
    input.extend_from_slice(pk_x);
    input.extend_from_slice(&LABEL);
    sha3_256(&input)
}

/// A public encapsulation key: `pk_M || pk_X`. Public, so `Debug` may show it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EncapsulationKey {
    bytes: Vec<u8>,
}

impl EncapsulationKey {
    /// A key from bytes received from a peer. The length and the ML-KEM modulus check run here.
    pub fn from_bytes(bytes: &[u8]) -> Result<EncapsulationKey, Error> {
        if bytes.len() != PUBLIC_KEY_LEN {
            return Err(Error::KeyLength);
        }
        mlkem::check_encapsulation_key(Params::MlKem768, &bytes[..MLKEM_PK_LEN]).map_err(Error::Mlkem)?;
        Ok(EncapsulationKey { bytes: bytes.to_vec() })
    }

    /// The 1216-byte encoding: the ML-KEM-768 key, then the X25519 key.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Encapsulates with the 64 bytes of randomness `eseed`: `eseed[..32]` is `m`, `eseed[32..]` is `ek_X`. Returns
    /// the shared secret and the 1120-byte ciphertext. The same randomness always gives the same output.
    pub fn encapsulate(&self, eseed: &[u8; ENCAPSULATION_SEED_LEN]) -> Result<([u8; 32], Vec<u8>), Error> {
        let (m, ek_x) = eseed.split_at(32);
        let ek_x = arr32(ek_x);
        let pk_x = arr32(&self.bytes[MLKEM_PK_LEN..]);

        let (ss_m, ct_m) = mlkem::encapsulate(Params::MlKem768, &self.bytes[..MLKEM_PK_LEN], &arr32(m))
            .map_err(Error::Mlkem)?;
        let ct_x = x25519::x25519(&ek_x, &x25519::base_point());
        let ss_x = agree(&ek_x, &pk_x)?;
        let ss = combine(&ss_m, &ss_x, &ct_x, &pk_x);
        Ok((ss, [ct_m.as_slice(), ct_x.as_slice()].concat()))
    }
}

/// A private decapsulation key: the 32-byte seed, with the expanded ML-KEM and X25519 keys kept beside it.
#[derive(Clone, PartialEq, Eq)]
pub struct DecapsulationKey {
    seed: [u8; SEED_LEN],
    /// The FIPS 203 ML-KEM-768 decapsulation key, 2400 bytes.
    mlkem: Vec<u8>,
    /// The X25519 secret scalar `sk_X`.
    x25519: [u8; 32],
    /// The public key `pk_M || pk_X`.
    public: Vec<u8>,
}

impl std::fmt::Debug for DecapsulationKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DecapsulationKey(X-Wing)")
    }
}

impl DecapsulationKey {
    /// The key that the seed produces. The same seed always gives the same key.
    pub fn from_seed(seed: &[u8; SEED_LEN]) -> DecapsulationKey {
        let (d, z, sk_x) = expand(seed);
        let kp = mlkem::keygen(Params::MlKem768, &d, &z);
        let pk_x = x25519::x25519(&sk_x, &x25519::base_point());
        let mut public = kp.encapsulation_key;
        public.extend_from_slice(&pk_x);
        DecapsulationKey { seed: *seed, mlkem: kp.decapsulation_key, x25519: sk_x, public }
    }

    /// The 32-byte seed. Store it only in the keychain or in app memory.
    pub fn seed(&self) -> &[u8; SEED_LEN] {
        &self.seed
    }

    /// The public half, to publish to peers.
    pub fn encapsulation_key(&self) -> EncapsulationKey {
        EncapsulationKey { bytes: self.public.clone() }
    }

    /// Recovers the shared secret from a 1120-byte ciphertext. An altered ciphertext gives a different secret, not
    /// an error (the ML-KEM part uses implicit rejection), so the caller must check the secret with a later step.
    pub fn decapsulate(&self, ciphertext: &[u8]) -> Result<[u8; 32], Error> {
        if ciphertext.len() != CIPHERTEXT_LEN {
            return Err(Error::CiphertextLength);
        }
        let (ct_m, ct_x) = ciphertext.split_at(MLKEM_CT_LEN);
        let ss_m = mlkem::decapsulate(Params::MlKem768, &self.mlkem, ct_m).map_err(Error::Mlkem)?;
        let ct_x = arr32(ct_x);
        let ss_x = agree(&self.x25519, &ct_x)?;
        let pk_x = arr32(&self.public[MLKEM_PK_LEN..]);
        Ok(combine(&ss_m, &ss_x, &ct_x, &pk_x))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [0x17; 32];
    const RANDOMNESS: [u8; 64] = [0x29; 64];

    #[test]
    fn sizes_match_the_draft() {
        assert_eq!(Params::MlKem768.encapsulation_key_len(), MLKEM_PK_LEN);
        assert_eq!(Params::MlKem768.ciphertext_len(), MLKEM_CT_LEN);
        assert_eq!(MLKEM_PK_LEN + 32, PUBLIC_KEY_LEN);
        assert_eq!(MLKEM_CT_LEN + 32, CIPHERTEXT_LEN);
        let ek = DecapsulationKey::from_seed(&SEED).encapsulation_key();
        let (ss, ct) = ek.encapsulate(&RANDOMNESS).unwrap();
        assert_eq!(ss.len(), SHARED_SECRET_LEN);
        assert_eq!(ct.len(), CIPHERTEXT_LEN);
    }

    #[test]
    fn deterministic_round_trip_from_fixed_seeds() {
        let a = DecapsulationKey::from_seed(&SEED);
        let b = DecapsulationKey::from_seed(&SEED);
        assert_eq!(a, b);
        assert_eq!(a.seed(), &SEED);
        let ek = a.encapsulation_key();
        let (ss, ct) = ek.encapsulate(&RANDOMNESS).unwrap();
        assert_eq!(ek.encapsulate(&RANDOMNESS).unwrap(), (ss, ct.clone()));
        assert_eq!(a.decapsulate(&ct).unwrap(), ss);
    }

    #[test]
    fn tampered_ciphertext_x_changes_the_secret() {
        let dk = DecapsulationKey::from_seed(&SEED);
        let (ss, mut ct) = dk.encapsulation_key().encapsulate(&RANDOMNESS).unwrap();
        ct[CIPHERTEXT_LEN - 1] ^= 0x01;
        assert_ne!(dk.decapsulate(&ct).unwrap(), ss);
    }

    #[test]
    fn tampered_ciphertext_m_changes_the_secret() {
        let dk = DecapsulationKey::from_seed(&SEED);
        let (ss, mut ct) = dk.encapsulation_key().encapsulate(&RANDOMNESS).unwrap();
        ct[5] ^= 0x10;
        assert_ne!(dk.decapsulate(&ct).unwrap(), ss);
    }

    #[test]
    fn combiner_label_bytes() {
        assert_eq!(&LABEL[..3], &b"\\./"[..]);
        assert_eq!(&LABEL[3..], &b"/^\\"[..]);
        assert_eq!(LABEL, [0x5c, 0x2e, 0x2f, 0x2f, 0x5e, 0x5c]);
    }

    #[test]
    fn combiner_hashes_the_parts_in_draft_order() {
        let (ss_m, ss_x, ct_x, pk_x) = ([1u8; 32], [2u8; 32], [3u8; 32], [4u8; 32]);
        let mut expected = Vec::new();
        for part in [ss_m, ss_x, ct_x, pk_x] {
            expected.extend_from_slice(&part);
        }
        expected.extend_from_slice(&LABEL);
        assert_eq!(combine(&ss_m, &ss_x, &ct_x, &pk_x), sha3_256(&expected));
    }

    #[test]
    fn wrong_lengths_are_refused() {
        assert_eq!(EncapsulationKey::from_bytes(&[0u8; 10]), Err(Error::KeyLength));
        let dk = DecapsulationKey::from_seed(&SEED);
        assert_eq!(dk.decapsulate(&[0u8; 10]), Err(Error::CiphertextLength));
    }

    #[test]
    fn debug_hides_the_seed() {
        let dk = DecapsulationKey::from_seed(&SEED);
        assert_eq!(format!("{:?}", dk), "DecapsulationKey(X-Wing)");
    }
}
