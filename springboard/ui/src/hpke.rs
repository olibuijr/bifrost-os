//! Hybrid public key encryption (RFC 9180), base mode, with DHKEM(X25519, HKDF-SHA256), HKDF-SHA256 and AES-128-GCM.
//! A sender encapsulates to a recipient public key and seals any number of messages in order; the recipient
//! decapsulates with the private key and opens them in the same order. The caller supplies the sender's ephemeral
//! private key from the random pool, so the module itself is deterministic. Also an exporter for deriving further keys.

use crate::x25519::{PrivateKey, PublicKey};
use bifrost_core::{aes_gcm, hkdf};

const KEM_ID: u16 = 0x0020;
const KDF_ID: u16 = 0x0001;
const AEAD_ID: u16 = 0x0001;
const NK: usize = 16;
const NN: usize = 12;
const NH: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A key is not 32 bytes, or the agreement hit a low-order point.
    Key,
    /// The ciphertext did not authenticate.
    Open,
    /// More messages than the sequence number allows.
    SequenceOverflow,
    /// The exporter was asked for more than 255 hash blocks.
    Export,
}

fn suite_kem() -> Vec<u8> {
    let mut v = b"KEM".to_vec();
    v.extend_from_slice(&KEM_ID.to_be_bytes());
    v
}

fn suite_hpke() -> Vec<u8> {
    let mut v = b"HPKE".to_vec();
    v.extend_from_slice(&KEM_ID.to_be_bytes());
    v.extend_from_slice(&KDF_ID.to_be_bytes());
    v.extend_from_slice(&AEAD_ID.to_be_bytes());
    v
}

fn labeled_extract(suite: &[u8], salt: &[u8], label: &[u8], ikm: &[u8]) -> [u8; 32] {
    let mut labeled = b"HPKE-v1".to_vec();
    labeled.extend_from_slice(suite);
    labeled.extend_from_slice(label);
    labeled.extend_from_slice(ikm);
    hkdf::hkdf_extract(salt, &labeled)
}

fn labeled_expand(suite: &[u8], prk: &[u8], label: &[u8], info: &[u8], len: usize) -> Vec<u8> {
    let mut labeled = (len as u16).to_be_bytes().to_vec();
    labeled.extend_from_slice(b"HPKE-v1");
    labeled.extend_from_slice(suite);
    labeled.extend_from_slice(label);
    labeled.extend_from_slice(info);
    hkdf::hkdf_expand(prk, &labeled, len).unwrap_or_default()
}

fn extract_and_expand(dh: &[u8], kem_context: &[u8]) -> Vec<u8> {
    let suite = suite_kem();
    let prk = labeled_extract(&suite, b"", b"eae_prk", dh);
    labeled_expand(&suite, &prk, b"shared_secret", kem_context, NH)
}

/// A sealing or opening context: one key, a base nonce and a message counter.
#[derive(Clone, Debug)]
pub struct Context {
    key: Vec<u8>,
    base_nonce: Vec<u8>,
    exporter_secret: Vec<u8>,
    seq: u64,
}

impl Context {
    fn new(shared_secret: &[u8], info: &[u8]) -> Context {
        let suite = suite_hpke();
        let psk_id_hash = labeled_extract(&suite, b"", b"psk_id_hash", b"");
        let info_hash = labeled_extract(&suite, b"", b"info_hash", info);
        let mut ksc = vec![0u8]; // mode_base
        ksc.extend_from_slice(&psk_id_hash);
        ksc.extend_from_slice(&info_hash);
        let secret = labeled_extract(&suite, shared_secret, b"secret", b"");
        Context {
            key: labeled_expand(&suite, &secret, b"key", &ksc, NK),
            base_nonce: labeled_expand(&suite, &secret, b"base_nonce", &ksc, NN),
            exporter_secret: labeled_expand(&suite, &secret, b"exp", &ksc, NH),
            seq: 0,
        }
    }

    fn nonce(&self) -> Vec<u8> {
        let mut n = self.base_nonce.clone();
        let seq = self.seq.to_be_bytes();
        for (i, b) in seq.iter().enumerate() {
            n[NN - 8 + i] ^= b;
        }
        n
    }

    /// Seals the next message.
    pub fn seal(&mut self, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        let out = aes_gcm::seal(&self.key, &self.nonce(), aad, plaintext).map_err(|_| Error::Open)?;
        self.seq = self.seq.checked_add(1).ok_or(Error::SequenceOverflow)?;
        Ok(out)
    }

    /// Opens the next message. A failed open does not advance the counter.
    pub fn open(&mut self, aad: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, Error> {
        let out = aes_gcm::open(&self.key, &self.nonce(), aad, ciphertext).map_err(|_| Error::Open)?;
        self.seq = self.seq.checked_add(1).ok_or(Error::SequenceOverflow)?;
        Ok(out)
    }

    /// Derives `len` bytes bound to this context and to `exporter_context` (RFC 9180 section 5.3).
    pub fn export(&self, exporter_context: &[u8], len: usize) -> Result<Vec<u8>, Error> {
        if len > 255 * NH {
            return Err(Error::Export);
        }
        Ok(labeled_expand(&suite_hpke(), &self.exporter_secret, b"sec", exporter_context, len))
    }

    /// How many messages this context has sealed or opened.
    pub fn sequence(&self) -> u64 {
        self.seq
    }
}

/// Sender setup: returns the encapsulated key to send along and the sealing context.
pub fn setup_sender(recipient_public: &[u8], info: &[u8], ephemeral_private: &[u8]) -> Result<(Vec<u8>, Context), Error> {
    let pk_r = PublicKey::from_raw(recipient_public).map_err(|_| Error::Key)?;
    let sk_e = PrivateKey::from_raw(ephemeral_private).map_err(|_| Error::Key)?;
    let pk_e = sk_e.public_key().raw();
    let dh = sk_e.shared_secret(&pk_r).map_err(|_| Error::Key)?;
    let mut kem_context = pk_e.to_vec();
    kem_context.extend_from_slice(&pk_r.raw());
    let shared = extract_and_expand(dh.bytes(), &kem_context);
    Ok((pk_e.to_vec(), Context::new(&shared, info)))
}

/// Recipient setup from the sender's encapsulated key.
pub fn setup_recipient(enc: &[u8], recipient_private: &[u8], info: &[u8]) -> Result<Context, Error> {
    let pk_e = PublicKey::from_raw(enc).map_err(|_| Error::Key)?;
    let sk_r = PrivateKey::from_raw(recipient_private).map_err(|_| Error::Key)?;
    let dh = sk_r.shared_secret(&pk_e).map_err(|_| Error::Key)?;
    let mut kem_context = pk_e.raw().to_vec();
    kem_context.extend_from_slice(&sk_r.public_key().raw());
    let shared = extract_and_expand(dh.bytes(), &kem_context);
    Ok(Context::new(&shared, info))
}

/// One-shot seal: `enc || ciphertext` as `(enc, ciphertext)`.
pub fn seal_base(recipient_public: &[u8], info: &[u8], aad: &[u8], plaintext: &[u8], ephemeral_private: &[u8]) -> Result<(Vec<u8>, Vec<u8>), Error> {
    let (enc, mut ctx) = setup_sender(recipient_public, info, ephemeral_private)?;
    let ct = ctx.seal(aad, plaintext)?;
    Ok((enc, ct))
}

/// One-shot open of what `seal_base` returned.
pub fn open_base(enc: &[u8], recipient_private: &[u8], info: &[u8], aad: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, Error> {
    setup_recipient(enc, recipient_private, info)?.open(aad, ciphertext)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SK_R: [u8; 32] = [0x11; 32];
    const SK_E: [u8; 32] = [0x22; 32];

    fn pk_r() -> [u8; 32] {
        PrivateKey::from_raw(&SK_R).unwrap().public_key().raw()
    }

    #[test]
    fn one_shot_round_trip() {
        let (enc, ct) = seal_base(&pk_r(), b"info", b"aad", b"hello hpke", &SK_E).unwrap();
        assert_eq!(enc.len(), 32);
        assert_ne!(ct, b"hello hpke");
        assert_eq!(open_base(&enc, &SK_R, b"info", b"aad", &ct).unwrap(), b"hello hpke");
    }

    #[test]
    fn wrong_key_info_aad_or_tamper_fails() {
        let (enc, ct) = seal_base(&pk_r(), b"info", b"aad", b"msg", &SK_E).unwrap();
        assert_eq!(open_base(&enc, &[0x33; 32], b"info", b"aad", &ct), Err(Error::Open));
        assert_eq!(open_base(&enc, &SK_R, b"other", b"aad", &ct), Err(Error::Open));
        assert_eq!(open_base(&enc, &SK_R, b"info", b"other", &ct), Err(Error::Open));
        let mut bad = ct.clone();
        bad[0] ^= 1;
        assert_eq!(open_base(&enc, &SK_R, b"info", b"aad", &bad), Err(Error::Open));
        assert_eq!(setup_sender(&[1; 5], b"", &SK_E).err(), Some(Error::Key));
    }

    #[test]
    fn messages_open_in_order() {
        let (enc, mut tx) = setup_sender(&pk_r(), b"", &SK_E).unwrap();
        let a = tx.seal(b"", b"first").unwrap();
        let b = tx.seal(b"", b"second").unwrap();
        assert_eq!(tx.sequence(), 2);
        let mut rx = setup_recipient(&enc, &SK_R, b"").unwrap();
        assert_eq!(rx.open(b"", &b), Err(Error::Open)); // out of order
        assert_eq!(rx.sequence(), 0);
        assert_eq!(rx.open(b"", &a).unwrap(), b"first");
        assert_eq!(rx.open(b"", &b).unwrap(), b"second");
        assert_ne!(a, b);
    }

    #[test]
    fn both_sides_export_the_same_secret() {
        let (enc, tx) = setup_sender(&pk_r(), b"i", &SK_E).unwrap();
        let rx = setup_recipient(&enc, &SK_R, b"i").unwrap();
        let a = tx.export(b"ctx", 40).unwrap();
        assert_eq!(a.len(), 40);
        assert_eq!(a, rx.export(b"ctx", 40).unwrap());
        assert_ne!(a, rx.export(b"other", 40).unwrap());
        assert_eq!(tx.export(b"x", 255 * 32 + 1), Err(Error::Export));
    }
}
