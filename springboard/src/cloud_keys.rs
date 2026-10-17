//! The account key pair and the HPKE seal and open helpers that the account's end-to-end encrypted features share: Find
//! My (Locate) seals device reports and account keys with them, and Universal Clipboard seals copied items. The cloud
//! stores only the two base64 blobs of a [`Sealed`] value and cannot open either.
//!
//! HPKE is RFC 9180 base mode with the `bifrost_ui::hpke` suite. Each feature passes its own `aad` (its additional
//! authenticated data, one per feature, so a blob from one feature never opens as another) and an `info` that binds a
//! blob to what it describes (a device id, a pending device id, a share id). The caller supplies the sender's ephemeral
//! private key from its random pool, so this module is deterministic and testable.

use crate::keychain::Store;
use bifrost_core::plist::{base64_decode, base64_encode};
use bifrost_ui::hpke;
use bifrost_ui::x25519::PrivateKey;

/// The keychain service and account of the account private key (the Find My key, see `bifrost-locate`).
pub const KEY_SERVICE: &str = "is.olibuijr.locate.desktop";
pub const KEY_ACCOUNT: &str = "findmy-account-key";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sealed {
    /// base64 of the ciphertext (AES-128-GCM, with its tag).
    pub sealed: String,
    /// base64 of the HPKE encapsulated key.
    pub enc: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SealError {
    /// A key is not 32 bytes, or it is a low-order point.
    Key,
    /// The blob did not authenticate: wrong key, wrong `aad` or `info`, or a changed byte.
    Open,
    /// A blob is not valid base64, or a payload is not what the caller expected.
    Payload,
}

/// Seals `plaintext` to `recipient_public` (32 raw bytes) under `aad` and `info`, with the sender's `ephemeral` key.
pub fn seal(recipient_public: &[u8], aad: &[u8], info: &[u8], plaintext: &[u8], ephemeral: &[u8]) -> Result<Sealed, SealError> {
    let (enc, ct) = hpke::seal_base(recipient_public, info, aad, plaintext, ephemeral).map_err(|_| SealError::Key)?;
    Ok(Sealed { sealed: base64_encode(&ct), enc: base64_encode(&enc) })
}

/// Opens a blob with the recipient's private key (32 raw bytes) and the `aad` and `info` it was sealed under.
pub fn open(recipient_private: &[u8], aad: &[u8], info: &[u8], blob: &Sealed) -> Result<Vec<u8>, SealError> {
    let enc = base64_decode(&blob.enc).ok_or(SealError::Payload)?;
    let ct = base64_decode(&blob.sealed).ok_or(SealError::Payload)?;
    hpke::open_base(&enc, recipient_private, info, aad, &ct).map_err(|_| SealError::Open)
}

/// A blob as one string: the base64 encapsulated key, a dot, then the base64 ciphertext. Base64 has no dot, so
/// [`unpack`] splits the two halves back apart.
pub fn pack(blob: &Sealed) -> String {
    format!("{}.{}", blob.enc, blob.sealed)
}

/// Splits a packed blob back into its halves. `None` when there is no dot.
pub fn unpack(text: &str) -> Option<Sealed> {
    let (enc, sealed) = text.split_once('.')?;
    Some(Sealed { sealed: sealed.to_string(), enc: enc.to_string() })
}

/// The public key (32 raw bytes) of a 32-byte X25519 private key, or `None` when the key is not 32 bytes.
pub fn public_key(private: &[u8]) -> Option<[u8; 32]> {
    let raw: [u8; 32] = private.try_into().ok()?;
    Some(PrivateKey::from_raw(&raw).ok()?.public_key().raw())
}

/// Reads the account private key from `store` under `service` and `account`. `None` when the item is missing or is not
/// 32 bytes. The key leaves the keychain only to be used here; it is never sent in the clear.
pub fn load_account_key(store: &Store, service: &str, account: &str) -> Option<[u8; 32]> {
    let bytes = store.get(service, account).ok()?;
    bytes.as_slice().try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AAD: &[u8] = b"bifrost.test.v1";

    fn keys(seed: u8) -> ([u8; 32], [u8; 32]) {
        let private = [seed; 32];
        (private, public_key(&private).unwrap())
    }

    #[test]
    fn a_blob_round_trips_under_the_same_aad_and_info() {
        let (private, public) = keys(0x11);
        let blob = seal(&public, AAD, b"info-1", b"hello", &[0x22; 32]).unwrap();
        assert_eq!(open(&private, AAD, b"info-1", &blob).unwrap(), b"hello");
    }

    #[test]
    fn a_different_aad_or_info_does_not_open() {
        let (private, public) = keys(0x11);
        let blob = seal(&public, AAD, b"info-1", b"hello", &[0x22; 32]).unwrap();
        assert_eq!(open(&private, b"other.v1", b"info-1", &blob), Err(SealError::Open));
        assert_eq!(open(&private, AAD, b"info-2", &blob), Err(SealError::Open));
    }

    #[test]
    fn a_stranger_key_does_not_open_and_the_ciphertext_is_opaque() {
        let (_, public) = keys(0x11);
        let (stranger, _) = keys(0x33);
        let blob = seal(&public, AAD, b"i", b"secret text", &[0x22; 32]).unwrap();
        assert_eq!(open(&stranger, AAD, b"i", &blob), Err(SealError::Open));
        assert!(!blob.sealed.contains("secret"));
    }

    #[test]
    fn a_changed_ciphertext_byte_fails_to_open() {
        let (private, public) = keys(0x11);
        let mut blob = seal(&public, AAD, b"i", b"hello", &[0x22; 32]).unwrap();
        let mut ct = base64_decode(&blob.sealed).unwrap();
        ct[0] ^= 1;
        blob.sealed = base64_encode(&ct);
        assert_eq!(open(&private, AAD, b"i", &blob), Err(SealError::Open));
    }

    #[test]
    fn pack_and_unpack_are_inverse() {
        let blob = Sealed { sealed: "Y3Q=".into(), enc: "ZW5j".into() };
        let text = pack(&blob);
        assert_eq!(text, "ZW5j.Y3Q=");
        assert_eq!(unpack(&text), Some(blob));
        assert_eq!(unpack("nodot"), None);
    }

    #[test]
    fn a_non_base64_blob_is_a_payload_error() {
        let (private, _) = keys(0x11);
        let bad = Sealed { sealed: "!!".into(), enc: "!!".into() };
        assert_eq!(open(&private, AAD, b"i", &bad), Err(SealError::Payload));
    }

    #[test]
    fn a_public_key_needs_exactly_32_bytes() {
        assert!(public_key(&[1u8; 31]).is_none());
        assert!(public_key(&[1u8; 32]).is_some());
    }
}
