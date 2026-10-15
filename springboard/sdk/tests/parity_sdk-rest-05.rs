//! Parity sdk-rest-05: headless tests of the app-facing ChaCha20-Poly1305 key object (bifrost_app::chacha_poly).

use bifrost_app::chacha_poly::{ChaChaPolyKey, Error, NONCE_BYTES, TAG_BYTES};
use bifrost_app::encrypted_archive::{self, Error as ArchiveError, HEADER_BYTES, MAGIC, VERSION};

#[test]
fn an_app_can_seal_a_message_and_open_it_with_its_key() {
    let key = ChaChaPolyKey::generate().unwrap();
    let sealed = key.seal(b"draft text", b"notes/1").unwrap();
    assert_eq!(sealed.len(), NONCE_BYTES + b"draft text".len() + TAG_BYTES);
    assert_eq!(key.open(&sealed, b"notes/1").unwrap(), b"draft text".to_vec());
}

#[test]
fn a_key_read_back_from_its_bytes_opens_a_box_sealed_by_the_original() {
    let key = ChaChaPolyKey::from_bytes(&[0x42u8; 32]).unwrap();
    let sealed = key.seal(b"token", b"").unwrap();
    let again = ChaChaPolyKey::from_bytes(key.bytes()).unwrap();
    assert_eq!(again.open(&sealed, b"").unwrap(), b"token".to_vec());
}

#[test]
fn a_key_of_the_wrong_size_is_reported_as_an_invalid_key_length() {
    assert_eq!(ChaChaPolyKey::from_bytes(&[0u8; 24]), Err(Error::InvalidKeyLength));
}

#[test]
fn a_tampered_box_is_reported_as_an_authentication_failure() {
    let key = ChaChaPolyKey::from_bytes(&[0x42u8; 32]).unwrap();
    let mut sealed = key.seal(b"token", b"").unwrap();
    sealed[NONCE_BYTES] ^= 0x01;
    assert_eq!(key.open(&sealed, b""), Err(Error::Authentication));
}

#[test]
fn a_box_without_room_for_a_nonce_and_tag_is_reported_as_invalid() {
    let key = ChaChaPolyKey::from_bytes(&[0x42u8; 32]).unwrap();
    assert_eq!(key.open(&[0u8; 10], b""), Err(Error::InvalidBox));
}

#[test]
fn the_debug_form_of_a_key_shows_its_size_only() {
    let key = ChaChaPolyKey::from_bytes(&[0xFFu8; 32]).unwrap();
    assert_eq!(format!("{key:?}"), "ChaChaPolyKey(256 bits)");
}

#[test]
fn an_encrypted_archive_round_trips_the_callers_archive_bytes() {
    let key = ChaChaPolyKey::from_bytes(&[0x29; 32]).unwrap();
    let archive = b"compressed archive bytes";
    let sealed = encrypted_archive::seal(&key, archive).unwrap();
    assert_eq!(&sealed[..MAGIC.len()], MAGIC);
    assert_eq!(sealed[MAGIC.len()], VERSION);
    assert_eq!(sealed.len(), HEADER_BYTES + NONCE_BYTES + archive.len() + TAG_BYTES);
    assert_eq!(encrypted_archive::open(&key, &sealed).unwrap(), archive);
}

#[test]
fn an_encrypted_archive_rejects_a_wrong_key_and_tampered_ciphertext() {
    let key = ChaChaPolyKey::from_bytes(&[0x29; 32]).unwrap();
    let wrong_key = ChaChaPolyKey::from_bytes(&[0x30; 32]).unwrap();
    let mut sealed = encrypted_archive::seal(&key, b"private archive").unwrap();
    assert_eq!(encrypted_archive::open(&wrong_key, &sealed), Err(ArchiveError::WrongKeyOrCorrupt));
    sealed[HEADER_BYTES + NONCE_BYTES] ^= 1;
    assert_eq!(encrypted_archive::open(&key, &sealed), Err(ArchiveError::WrongKeyOrCorrupt));
}

#[test]
fn malformed_and_unknown_version_headers_are_refused() {
    let key = ChaChaPolyKey::from_bytes(&[0x29; 32]).unwrap();
    assert_eq!(encrypted_archive::open(&key, b"BFEA"), Err(ArchiveError::NotEncryptedArchive));
    let mut sealed = encrypted_archive::seal(&key, b"payload").unwrap();
    sealed[0] ^= 1;
    assert_eq!(encrypted_archive::open(&key, &sealed), Err(ArchiveError::NotEncryptedArchive));
    sealed[0] ^= 1;
    sealed[MAGIC.len()] = VERSION + 1;
    assert_eq!(encrypted_archive::open(&key, &sealed), Err(ArchiveError::UnsupportedVersion(VERSION + 1)));
}
