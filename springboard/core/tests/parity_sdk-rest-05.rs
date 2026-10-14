//! Parity sdk-rest-05: headless tests of the ChaCha20-Poly1305 model against the RFC 8439 published vectors.
//! No test touches a device, a clock or the random pool.

use bifrost_core::chacha_poly::{open, poly1305, seal, Error, NONCE_LEN, TAG_LEN};

const PLAIN: &[u8] = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";

fn vector_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = 0x80 + i as u8;
    }
    key
}

const VECTOR_NONCE: [u8; NONCE_LEN] = [0x07, 0x00, 0x00, 0x00, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47];
const VECTOR_AAD: [u8; 12] = [0x50, 0x51, 0x52, 0x53, 0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7];
const VECTOR_TAG: [u8; TAG_LEN] = [0x1a, 0xe1, 0x0b, 0x59, 0x4f, 0x09, 0xe2, 0x6a, 0x7e, 0x90, 0x2e, 0xcb, 0xd0, 0x60, 0x06, 0x91];

#[test]
fn the_rfc_8439_aead_vector_seals_to_the_published_tag() {
    let sealed = seal(&vector_key(), &VECTOR_NONCE, &VECTOR_AAD, PLAIN).unwrap();
    assert_eq!(sealed.len(), PLAIN.len() + TAG_LEN);
    assert_eq!(&sealed[..8], &[0xd3, 0x1a, 0x8d, 0x34, 0x64, 0x8e, 0x60, 0xdb]);
    assert_eq!(&sealed[PLAIN.len()..], &VECTOR_TAG);
}

#[test]
fn the_rfc_8439_aead_vector_opens_to_the_plaintext() {
    let sealed = seal(&vector_key(), &VECTOR_NONCE, &VECTOR_AAD, PLAIN).unwrap();
    assert_eq!(open(&vector_key(), &VECTOR_NONCE, &VECTOR_AAD, &sealed).unwrap(), PLAIN);
}

#[test]
fn the_rfc_8439_poly1305_vector_gives_the_published_tag() {
    let key = [
        0x85, 0xd6, 0xbe, 0x78, 0x57, 0x55, 0x6d, 0x33, 0x7f, 0x44, 0x52, 0xfe, 0x42, 0xd5, 0x06, 0xa8, 0x01, 0x03, 0x80,
        0x8a, 0xfb, 0x0d, 0xb2, 0xfd, 0x4a, 0xbf, 0xf6, 0xaf, 0x41, 0x49, 0xf5, 0x1b,
    ];
    let tag = poly1305(&key, b"Cryptographic Forum Research Group").unwrap();
    assert_eq!(tag, [0xa8, 0x06, 0x1d, 0xc1, 0x30, 0x51, 0x36, 0xc6, 0xc2, 0x2b, 0x8b, 0xaf, 0x0c, 0x01, 0x27, 0xa9]);
}

#[test]
fn a_poly1305_tag_of_an_empty_message_is_the_second_half_of_the_key() {
    let key = vector_key();
    assert_eq!(poly1305(&key, b"").unwrap().to_vec(), key[16..].to_vec());
}

#[test]
fn a_flipped_ciphertext_bit_is_refused_without_plaintext() {
    let mut sealed = seal(&vector_key(), &VECTOR_NONCE, &VECTOR_AAD, PLAIN).unwrap();
    sealed[10] ^= 0x40;
    assert_eq!(open(&vector_key(), &VECTOR_NONCE, &VECTOR_AAD, &sealed), Err(Error::Authentication));
}

#[test]
fn different_additional_data_is_refused() {
    let sealed = seal(&vector_key(), &VECTOR_NONCE, &VECTOR_AAD, PLAIN).unwrap();
    assert_eq!(open(&vector_key(), &VECTOR_NONCE, b"other", &sealed), Err(Error::Authentication));
}

#[test]
fn a_key_of_the_wrong_length_is_refused() {
    assert_eq!(seal(&[0u8; 16], &VECTOR_NONCE, b"", b"x"), Err(Error::BadKeyLength));
    assert_eq!(open(&[0u8; 33], &VECTOR_NONCE, b"", &[0u8; 16]), Err(Error::BadKeyLength));
}

#[test]
fn a_nonce_of_the_wrong_length_is_refused() {
    assert_eq!(seal(&vector_key(), &[0u8; 8], b"", b"x"), Err(Error::BadNonceLength));
}

#[test]
fn a_sealed_input_shorter_than_a_tag_is_refused() {
    assert_eq!(open(&vector_key(), &VECTOR_NONCE, b"", &[0u8; 15]), Err(Error::Authentication));
}

#[test]
fn an_empty_message_seals_to_just_a_tag_and_opens_back_to_nothing() {
    let sealed = seal(&vector_key(), &VECTOR_NONCE, b"", b"").unwrap();
    assert_eq!(sealed.len(), TAG_LEN);
    assert_eq!(open(&vector_key(), &VECTOR_NONCE, b"", &sealed).unwrap(), Vec::<u8>::new());
}
