//! Encrypted pictures and files, as the Matrix attachment scheme has them: the bytes are sealed with AES-256 in counter mode
//! under a fresh random key, the ciphertext is uploaded as it is, and the message carries a `file` object (the URL, the key
//! as a JWK, the 16-byte counter block `iv`, and the SHA-256 of the ciphertext). Only the message is encrypted for the
//! room; the server holds ciphertext it cannot read.
//!
//! The block cipher is `bifrost_core::aes_gcm::Aes` (FIPS-197); [`ctr_xor`] is the counter mode on top of its block
//! function (NIST SP 800-38A section 6.5). The hash is `bifrost_core::sha256`. The base64 here is the unpadded form the
//! scheme uses (the key is base64url, the other fields standard base64).

use crate::crypto;
use crate::protocol;
use bifrost_core::aes_gcm::Aes;
use bifrost_core::json::Json;
use bifrost_core::sha256;

const STANDARD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL_SAFE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// AES counter mode: XORs `data` with the keystream of `key` (16 or 32 bytes) from the counter block `iv`. The counter is
/// incremented as a big-endian 128-bit number after each block, so the same call decrypts what it encrypted.
pub fn ctr_xor(key: &[u8], iv: &[u8; 16], data: &mut [u8]) -> Result<(), String> {
    let aes = Aes::new(key).map_err(|e| format!("{e:?}"))?;
    let mut counter = u128::from_be_bytes(*iv);
    for chunk in data.chunks_mut(16) {
        let stream = aes.encrypt_block(&counter.to_be_bytes());
        for (byte, k) in chunk.iter_mut().zip(stream.iter()) {
            *byte ^= k;
        }
        counter = counter.wrapping_add(1);
    }
    Ok(())
}

/// Unpadded base64 of `bytes` in the given alphabet.
fn encode(bytes: &[u8], alphabet: &[u8; 64]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..chunk.len() + 1 {
            out.push(alphabet[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

fn sextet(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some((c - b'A') as u32),
        b'a'..=b'z' => Some((c - b'a') as u32 + 26),
        b'0'..=b'9' => Some((c - b'0') as u32 + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

/// The bytes of base64 text in either alphabet, with or without the `=` padding. `None` for any other character.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    if text.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        acc = (acc << 6) | sextet(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// One encrypted attachment: the ciphertext and the key and counter block that open it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub cipher: Vec<u8>,
    pub key: [u8; 32],
    pub iv: [u8; 16],
}

/// Encrypts `plain` under a new random key. The counter block has random high 64 bits and zero low 64 bits.
pub fn encrypt(plain: &[u8]) -> Result<Attachment, String> {
    let key: [u8; 32] = crypto::random_bytes()?;
    let high: [u8; 8] = crypto::random_bytes()?;
    let mut iv = [0u8; 16];
    iv[..8].copy_from_slice(&high);
    let mut cipher = plain.to_vec();
    ctr_xor(&key, &iv, &mut cipher)?;
    Ok(Attachment { cipher, key, iv })
}

/// The SHA-256 of `cipher`, as the `hashes` entry of the file object (unpadded standard base64).
pub fn hash_of(cipher: &[u8]) -> String {
    encode(&sha256::digest(cipher), STANDARD)
}

/// The `file` object of a message that carries this attachment at `url` (an `mxc://` URI).
pub fn file_object(url: &str, attachment: &Attachment) -> Json {
    let jwk = protocol::obj(vec![
        ("kty", protocol::text("oct")),
        ("key_ops", Json::Arr(vec![protocol::text("encrypt"), protocol::text("decrypt")])),
        ("alg", protocol::text("A256CTR")),
        ("k", protocol::text(&encode(&attachment.key, URL_SAFE))),
        ("ext", Json::Bool(true)),
    ]);
    protocol::obj(vec![
        ("url", protocol::text(url)),
        ("key", jwk),
        ("iv", protocol::text(&encode(&attachment.iv, STANDARD))),
        ("hashes", protocol::obj(vec![("sha256", protocol::text(&hash_of(&attachment.cipher)))])),
        ("v", protocol::text("v2")),
    ])
}

/// What a received `file` object says about its attachment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileInfo {
    pub url: String,
    pub key: [u8; 32],
    pub iv: [u8; 16],
    /// The SHA-256 of the ciphertext, as the object gives it.
    pub sha256: String,
}

/// Reads a received `file` object. `None` when a field is missing, the key is not an A256CTR JWK, or a size is wrong.
pub fn parse_file_object(file: &Json) -> Option<FileInfo> {
    let url = file.str_field("url")?;
    let jwk = file.get("key")?;
    if jwk.str_field("kty")? != "oct" || jwk.str_field("alg")? != "A256CTR" {
        return None;
    }
    let key: [u8; 32] = decode(&jwk.str_field("k")?)?.try_into().ok()?;
    let iv: [u8; 16] = decode(&file.str_field("iv")?)?.try_into().ok()?;
    let sha256 = file.get("hashes")?.str_field("sha256")?;
    Some(FileInfo { url, key, iv, sha256 })
}

/// Decrypts the ciphertext downloaded for `info`. The hash is checked first, so a changed or substituted ciphertext
/// gives an error and no plaintext.
pub fn decrypt(info: &FileInfo, cipher: &[u8]) -> Result<Vec<u8>, String> {
    if decode(&info.sha256) != Some(sha256::digest(cipher).to_vec()) {
        return Err("the attachment failed its hash check".into());
    }
    let mut plain = cipher.to_vec();
    ctr_xor(&info.key, &info.iv, &mut plain)?;
    Ok(plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        sha256::unhex(s).expect("hex")
    }

    /// NIST SP 800-38A F.5.1 (CTR-AES128.Encrypt) and F.5.5 (CTR-AES256.Encrypt): four blocks each.
    const PLAIN: &str = "6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e5130c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710";
    const IV: &str = "f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff";

    #[test]
    fn ctr_matches_the_nist_aes128_vectors() {
        let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
        let mut data = hex(PLAIN);
        ctr_xor(&key, &hex(IV).try_into().unwrap(), &mut data).unwrap();
        assert_eq!(
            data,
            hex("874d6191b620e3261bef6864990db6ce9806f66b7970fdff8617187bb9fffdff5ae4df3edbd5d35e5b4f09020db03eab1e031dda2fbe03d1792170a0f3009cee")
        );
    }

    #[test]
    fn ctr_matches_the_nist_aes256_vectors() {
        let key = hex("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4");
        let mut data = hex(PLAIN);
        ctr_xor(&key, &hex(IV).try_into().unwrap(), &mut data).unwrap();
        assert_eq!(
            data,
            hex("601ec313775789a5b7a7f504bbf3d228f443e3ca4d62b59aca84e990cacaf5c52b0930daa23de94ce87017ba2d84988ddfc9c58db67aada613c2dd08457941a6")
        );
    }

    #[test]
    fn ctr_handles_a_partial_last_block_and_is_its_own_inverse() {
        let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
        let iv: [u8; 16] = hex(IV).try_into().unwrap();
        let mut short = hex(PLAIN)[..20].to_vec();
        ctr_xor(&key, &iv, &mut short).unwrap();
        assert_eq!(short[..], hex("874d6191b620e3261bef6864990db6ce9806f66b")[..]);
        ctr_xor(&key, &iv, &mut short).unwrap();
        assert_eq!(short, hex(PLAIN)[..20].to_vec());
    }

    #[test]
    fn base64_is_unpadded_in_both_alphabets_and_decodes_either() {
        assert_eq!(encode(b"foobar", STANDARD), "Zm9vYmFy");
        assert_eq!(encode(b"fo", STANDARD), "Zm8");
        assert_eq!(encode(&[0xFF, 0xEF], URL_SAFE), "_-8");
        assert_eq!(decode("Zm9vYmFy"), Some(b"foobar".to_vec()));
        assert_eq!(decode("Zg=="), Some(b"f".to_vec()), "padding is accepted");
        assert_eq!(decode("_-8"), Some(vec![0xFF, 0xEF]));
        assert_eq!(decode("Z"), None, "one character left over is not base64");
        assert_eq!(decode("Zm9!"), None);
    }

    #[test]
    fn an_attachment_round_trips_through_its_file_object() {
        let plain: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let att = encrypt(&plain).unwrap();
        assert_ne!(att.cipher, plain);
        assert_eq!(&att.iv[8..], &[0u8; 8], "the low 64 bits of the counter start at zero");
        let file = file_object("mxc://example.org/abc", &att);
        let info = parse_file_object(&file).expect("a file object");
        assert_eq!(info.url, "mxc://example.org/abc");
        assert_eq!(info.key, att.key);
        assert_eq!(info.iv, att.iv);
        assert_eq!(decrypt(&info, &att.cipher).unwrap(), plain);
    }

    #[test]
    fn a_changed_ciphertext_fails_the_hash_check() {
        let att = encrypt(b"a picture, say").unwrap();
        let info = parse_file_object(&file_object("mxc://x/y", &att)).unwrap();
        let mut bad = att.cipher.clone();
        bad[0] ^= 1;
        assert!(decrypt(&info, &bad).is_err());
    }

    #[test]
    fn a_file_object_that_is_not_an_a256ctr_key_is_refused() {
        let att = encrypt(b"x").unwrap();
        let good = file_object("mxc://x/y", &att);
        assert!(parse_file_object(&good).is_some());
        assert!(parse_file_object(&protocol::obj(vec![("url", protocol::text("mxc://x/y"))])).is_none(), "no key");
        let not_an_attachment = protocol::obj(vec![("url", protocol::text("mxc://x/y")), ("iv", protocol::text("AAAAAAAAAAAAAAAAAAAAAA"))]);
        assert!(parse_file_object(&not_an_attachment).is_none());
    }
}
