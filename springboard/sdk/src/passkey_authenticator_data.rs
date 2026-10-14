//! Authenticator data of a passkey ceremony: the byte string an authenticator returns with each registration and each
//! assertion (WebAuthn section 6.1). It holds the SHA-256 hash of the relying party identifier, the flags (user present,
//! user verified, backup eligible, backed up, attested credential data, extensions), the signature counter, and, for a
//! registration, the attested credential: the authenticator's AAGUID, the credential identifier and the credential public
//! key as a COSE_Key in CBOR.
//!
//! This module reads those bytes and checks their layout. It verifies no signature, creates no key and talks to no
//! authenticator. [`assertion_signed_bytes`] builds the bytes that an assertion signature covers, so a later signing step
//! has the exact input. The public key is kept as its CBOR bytes: the key's length is found by walking the CBOR items, and
//! the key is not decoded.

use crate::digest::sha256;
use std::fmt;

const FLAG_USER_PRESENT: u8 = 0x01;
const FLAG_USER_VERIFIED: u8 = 0x04;
const FLAG_BACKUP_ELIGIBLE: u8 = 0x08;
const FLAG_BACKED_UP: u8 = 0x10;
const FLAG_ATTESTED: u8 = 0x40;
const FLAG_EXTENSIONS: u8 = 0x80;

/// The fixed part of the data: the 32-byte hash, the flags byte and the 4-byte signature counter.
const FIXED_LEN: usize = 37;

/// How deeply CBOR items may nest in the credential public key or the extensions.
const MAX_CBOR_DEPTH: usize = 16;

/// Why authenticator data could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The data ends before a field it must hold.
    TooShort,
    /// The backed-up flag is set while the backup eligible flag is clear, which the specification forbids.
    BackupStateWithoutEligibility,
    /// The attested credential public key is not one complete CBOR data item.
    MalformedPublicKey,
    /// The extension data is not one complete CBOR data item.
    MalformedExtensions,
    /// Bytes are left over after the fields the flags announce.
    TrailingBytes,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Error::TooShort => "the authenticator data ends too early",
            Error::BackupStateWithoutEligibility => "the backed-up flag is set without the backup eligible flag",
            Error::MalformedPublicKey => "the credential public key is not a complete CBOR item",
            Error::MalformedExtensions => "the extension data is not a complete CBOR item",
            Error::TrailingBytes => "the authenticator data has bytes after its last field",
        };
        f.write_str(text)
    }
}

/// The credential created by a registration: who made it, its identifier and its public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttestedCredential {
    /// The authenticator model identifier, 16 bytes.
    pub aaguid: [u8; 16],
    pub credential_id: Vec<u8>,
    /// The credential public key as the CBOR bytes of a COSE_Key.
    pub public_key: Vec<u8>,
}

/// Authenticator data read from its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatorData {
    /// SHA-256 of the relying party identifier the credential is scoped to.
    pub rp_id_hash: [u8; 32],
    pub user_present: bool,
    pub user_verified: bool,
    pub backup_eligible: bool,
    pub backed_up: bool,
    /// The signature counter, big endian in the data.
    pub sign_count: u32,
    /// Present in a registration.
    pub attested: Option<AttestedCredential>,
    /// The raw CBOR map of extension outputs, present when the extension flag is set.
    pub extensions: Option<Vec<u8>>,
}

impl AuthenticatorData {
    /// Reads authenticator data. Every field the flags announce must be present, and nothing may follow the last one.
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let rp_id_hash = array::<32>(data, 0).ok_or(Error::TooShort)?;
        let flags = *data.get(32).ok_or(Error::TooShort)?;
        let sign_count = u32::from_be_bytes(array::<4>(data, 33).ok_or(Error::TooShort)?);
        let backup_eligible = flags & FLAG_BACKUP_ELIGIBLE != 0;
        let backed_up = flags & FLAG_BACKED_UP != 0;
        if backed_up && !backup_eligible {
            return Err(Error::BackupStateWithoutEligibility);
        }
        let mut at = FIXED_LEN;
        let attested = if flags & FLAG_ATTESTED != 0 {
            let aaguid = array::<16>(data, at).ok_or(Error::TooShort)?;
            at += 16;
            let id_len = usize::from(u16::from_be_bytes(array::<2>(data, at).ok_or(Error::TooShort)?));
            at += 2;
            let credential_id = data.get(at..at + id_len).ok_or(Error::TooShort)?.to_vec();
            at += id_len;
            let key_len = item_len(data.get(at..).ok_or(Error::TooShort)?, 0).ok_or(Error::MalformedPublicKey)?;
            let public_key = data.get(at..at + key_len).ok_or(Error::MalformedPublicKey)?.to_vec();
            at += key_len;
            Some(AttestedCredential { aaguid, credential_id, public_key })
        } else {
            None
        };
        let extensions = if flags & FLAG_EXTENSIONS != 0 {
            let len = item_len(data.get(at..).ok_or(Error::TooShort)?, 0).ok_or(Error::MalformedExtensions)?;
            let bytes = data.get(at..at + len).ok_or(Error::MalformedExtensions)?.to_vec();
            at += len;
            Some(bytes)
        } else {
            None
        };
        if at != data.len() {
            return Err(Error::TrailingBytes);
        }
        Ok(AuthenticatorData {
            rp_id_hash,
            user_present: flags & FLAG_USER_PRESENT != 0,
            user_verified: flags & FLAG_USER_VERIFIED != 0,
            backup_eligible,
            backed_up,
            sign_count,
            attested,
            extensions,
        })
    }

    /// Whether the hash in this data is the SHA-256 of `rp_id`, exactly as written (the hash is over the bytes).
    pub fn rp_id_matches(&self, rp_id: &str) -> bool {
        sha256(rp_id.as_bytes()) == self.rp_id_hash
    }
}

/// The bytes an assertion signature covers: the raw authenticator data followed by the SHA-256 of the client data JSON.
pub fn assertion_signed_bytes(authenticator_data: &[u8], client_data_json: &[u8]) -> Vec<u8> {
    let mut out = authenticator_data.to_vec();
    out.extend_from_slice(&sha256(client_data_json));
    out
}

/// The `N` bytes at `at`, or `None` when the data is shorter.
fn array<const N: usize>(data: &[u8], at: usize) -> Option<[u8; N]> {
    <[u8; N]>::try_from(data.get(at..)?.get(..N)?).ok()
}

/// The length in bytes of the first CBOR data item in `data`, or `None` when the item is truncated, malformed, uses an
/// indefinite length or nests deeper than [`MAX_CBOR_DEPTH`].
fn item_len(data: &[u8], depth: usize) -> Option<usize> {
    if depth > MAX_CBOR_DEPTH {
        return None;
    }
    let initial = *data.first()?;
    let major = initial >> 5;
    let info = initial & 0x1f;
    let (argument, header) = match info {
        0..=23 => (u64::from(info), 1),
        24 => (u64::from(*data.get(1)?), 2),
        25 => (u64::from(u16::from_be_bytes(array::<2>(data, 1)?)), 3),
        26 => (u64::from(u32::from_be_bytes(array::<4>(data, 1)?)), 5),
        27 => (u64::from_be_bytes(array::<8>(data, 1)?), 9),
        _ => return None,
    };
    match major {
        0 | 1 | 7 => Some(header),
        2 | 3 => {
            let end = header.checked_add(usize::try_from(argument).ok()?)?;
            (end <= data.len()).then_some(end)
        }
        4 | 5 => {
            let items = if major == 4 { argument } else { argument.checked_mul(2)? };
            let mut total = header;
            for _ in 0..items {
                total += item_len(data.get(total..)?, depth + 1)?;
            }
            Some(total)
        }
        6 => header.checked_add(item_len(data.get(header..)?, depth + 1)?),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A COSE_Key-shaped map: {1: 2, 3: -7}.
    const COSE_KEY: [u8; 5] = [0xa2, 0x01, 0x02, 0x03, 0x26];

    fn data(flags: u8, counter: u32, tail: &[u8]) -> Vec<u8> {
        let mut out = vec![0x11; 32];
        out.push(flags);
        out.extend_from_slice(&counter.to_be_bytes());
        out.extend_from_slice(tail);
        out
    }

    fn attested_tail(key: &[u8]) -> Vec<u8> {
        let mut tail = vec![0xaa; 16];
        tail.extend_from_slice(&[0x00, 0x03, 1, 2, 3]);
        tail.extend_from_slice(key);
        tail
    }

    #[test]
    fn flags_and_counter_are_read_from_an_assertion_without_credential_data() {
        let parsed = AuthenticatorData::parse(&data(0x05, 7, &[])).unwrap();
        assert!(parsed.user_present && parsed.user_verified);
        assert!(!parsed.backup_eligible && !parsed.backed_up);
        assert_eq!(parsed.sign_count, 7);
        assert_eq!(parsed.rp_id_hash, [0x11; 32]);
        assert!(parsed.attested.is_none() && parsed.extensions.is_none());
    }

    #[test]
    fn a_registration_yields_the_aaguid_credential_id_and_key_bytes() {
        let parsed = AuthenticatorData::parse(&data(0x45, 0, &attested_tail(&COSE_KEY))).unwrap();
        let credential = parsed.attested.expect("the attested flag announces a credential");
        assert_eq!(credential.aaguid, [0xaa; 16]);
        assert_eq!(credential.credential_id, vec![1, 2, 3]);
        assert_eq!(credential.public_key, COSE_KEY.to_vec());
    }

    #[test]
    fn extension_data_follows_the_key_and_is_kept_as_its_cbor_bytes() {
        let extensions = [0xa1, 0x61, 0x61, 0xf5];
        let mut tail = attested_tail(&COSE_KEY);
        tail.extend_from_slice(&extensions);
        let parsed = AuthenticatorData::parse(&data(0xc5, 3, &tail)).unwrap();
        assert_eq!(parsed.extensions, Some(extensions.to_vec()));
        assert!(parsed.attested.is_some());
    }

    #[test]
    fn the_backup_flags_need_eligibility_first() {
        assert_eq!(
            AuthenticatorData::parse(&data(0x11, 0, &[])),
            Err(Error::BackupStateWithoutEligibility)
        );
        let parsed = AuthenticatorData::parse(&data(0x19, 0, &[])).unwrap();
        assert!(parsed.backup_eligible && parsed.backed_up);
    }

    #[test]
    fn data_shorter_than_its_fixed_part_is_refused() {
        assert_eq!(AuthenticatorData::parse(&[0x11; 36]), Err(Error::TooShort));
        assert_eq!(AuthenticatorData::parse(&data(0x45, 0, &[0xaa; 10])), Err(Error::TooShort));
    }

    #[test]
    fn bytes_after_the_last_announced_field_are_refused() {
        assert_eq!(AuthenticatorData::parse(&data(0x01, 0, &[0x00])), Err(Error::TrailingBytes));
        let mut tail = attested_tail(&COSE_KEY);
        tail.push(0x00);
        assert_eq!(AuthenticatorData::parse(&data(0x45, 0, &tail)), Err(Error::TrailingBytes));
    }

    #[test]
    fn a_public_key_that_is_not_one_cbor_item_is_refused() {
        assert_eq!(
            AuthenticatorData::parse(&data(0x45, 0, &attested_tail(&[0xff]))),
            Err(Error::MalformedPublicKey),
            "major type 7 with indefinite length is not allowed"
        );
        assert_eq!(
            AuthenticatorData::parse(&data(0x45, 0, &attested_tail(&[0xa2, 0x01]))),
            Err(Error::MalformedPublicKey),
            "a map that promises two pairs and holds one is truncated"
        );
    }

    #[test]
    fn a_malformed_extension_map_is_refused() {
        let mut tail = attested_tail(&COSE_KEY);
        tail.extend_from_slice(&[0x62, 0x61]);
        assert_eq!(AuthenticatorData::parse(&data(0xc5, 0, &tail)), Err(Error::MalformedExtensions));
    }

    #[test]
    fn nesting_is_limited_to_sixteen_levels() {
        let mut shallow_key = vec![0x81; 10];
        shallow_key.push(0x00);
        assert!(AuthenticatorData::parse(&data(0x45, 0, &attested_tail(&shallow_key))).is_ok());
        let mut deep_key = vec![0x81; 20];
        deep_key.push(0x00);
        assert_eq!(
            AuthenticatorData::parse(&data(0x45, 0, &attested_tail(&deep_key))),
            Err(Error::MalformedPublicKey)
        );
    }

    #[test]
    fn the_relying_party_hash_is_the_sha_256_of_the_identifier_text() {
        let mut bytes = data(0x01, 0, &[]);
        bytes[..32].copy_from_slice(&sha256(b"example.com"));
        let parsed = AuthenticatorData::parse(&bytes).unwrap();
        assert!(parsed.rp_id_matches("example.com"));
        assert!(!parsed.rp_id_matches("EXAMPLE.com"), "the hash is over the exact bytes");
        assert!(!parsed.rp_id_matches("sub.example.com"));
    }

    #[test]
    fn the_signed_bytes_are_the_authenticator_data_then_the_client_data_digest() {
        let auth = data(0x05, 1, &[]);
        let signed = assertion_signed_bytes(&auth, b"abc");
        assert_eq!(signed.len(), auth.len() + 32);
        assert_eq!(&signed[..auth.len()], auth.as_slice());
        // SHA-256("abc") starts ba 78 16 bf.
        assert_eq!(&signed[auth.len()..auth.len() + 4], &[0xba, 0x78, 0x16, 0xbf]);
    }

    #[test]
    fn the_error_messages_name_the_fault() {
        assert_eq!(Error::TooShort.to_string(), "the authenticator data ends too early");
        assert_eq!(Error::TrailingBytes.to_string(), "the authenticator data has bytes after its last field");
    }
}
