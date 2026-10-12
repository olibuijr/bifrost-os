//! AES-128 and AES-256 in Galois/Counter Mode (GCM, NIST SP 800-38D), pure std Rust.
//!
//! [`seal`] encrypts a message and appends a 16-byte tag; [`open`] checks the tag and decrypts. Nonces are 12 bytes,
//! the length the mode is defined for. The block cipher follows FIPS-197. Its S-box is derived at first use from the
//! GF(2^8) inverse and the affine map, so no table is copied in by hand. Lookups index that table with secret bytes,
//! so timing can depend on key and data; this is for files and messages on a machine the owner controls, not a
//! hardened primitive. The tag comparison in [`open`] is constant time.

use std::fmt;
use std::sync::OnceLock;

/// Nonce length in bytes for GCM (96 bits).
pub const NONCE_LEN: usize = 12;
/// Authentication tag length in bytes (128 bits, the full tag).
pub const TAG_LEN: usize = 16;
/// AES block length in bytes.
pub const BLOCK_LEN: usize = 16;
/// Longest message GCM allows, in bytes (2^39 - 256 bits).
pub const MAX_MESSAGE_LEN: u64 = (1u64 << 36) - 32;

/// Why a seal or open was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The key is neither 16 bytes (AES-128) nor 32 bytes (AES-256).
    BadKeyLength,
    /// The nonce is not [`NONCE_LEN`] bytes.
    BadNonceLength,
    /// The message is longer than [`MAX_MESSAGE_LEN`].
    TooLong,
    /// The sealed box is shorter than a tag, or the tag does not match the data, the additional data or the key.
    Authentication,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Error::BadKeyLength => "key must be 16 or 32 bytes",
            Error::BadNonceLength => "nonce must be 12 bytes",
            Error::TooLong => "message is longer than GCM allows",
            Error::Authentication => "authentication tag does not match",
        };
        f.write_str(text)
    }
}

/// Multiplication in GF(2^8) modulo x^8 + x^4 + x^3 + x + 1 (0x11b), the field the AES S-box is built on.
fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        let carry = a & 0x80;
        a <<= 1;
        if carry != 0 {
            a ^= 0x1b;
        }
        b >>= 1;
    }
    p
}

/// Multiplicative inverse in GF(2^8) by a^254. The inverse of zero is defined as zero, as the S-box needs.
fn ginv(a: u8) -> u8 {
    if a == 0 {
        return 0;
    }
    let mut result = 1u8;
    let mut base = a;
    let mut e = 254u32;
    while e > 0 {
        if e & 1 != 0 {
            result = gmul(result, base);
        }
        base = gmul(base, base);
        e >>= 1;
    }
    result
}

/// The forward S-box (FIPS-197 section 5.1.1): inverse in GF(2^8), then the affine map with constant 0x63.
fn sbox() -> &'static [u8; 256] {
    static SBOX: OnceLock<[u8; 256]> = OnceLock::new();
    SBOX.get_or_init(|| {
        let mut t = [0u8; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let b = ginv(i as u8);
            *slot = b ^ b.rotate_left(1) ^ b.rotate_left(2) ^ b.rotate_left(3) ^ b.rotate_left(4) ^ 0x63;
        }
        t
    })
}

/// AES block cipher with an expanded key schedule. Only the forward (encrypt) direction is needed by GCM.
pub struct Aes {
    round_keys: Vec<[u8; 16]>,
}

impl Aes {
    /// Expands a 16-byte (AES-128) or 32-byte (AES-256) key.
    pub fn new(key: &[u8]) -> Result<Aes, Error> {
        let nk = match key.len() {
            16 => 4usize,
            32 => 8usize,
            _ => return Err(Error::BadKeyLength),
        };
        let nr = nk + 6;
        let total = 4 * (nr + 1);
        let sbox = sbox();
        let mut w: Vec<[u8; 4]> = Vec::with_capacity(total);
        for i in 0..nk {
            w.push([key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]]);
        }
        let mut rcon = 1u8;
        for i in nk..total {
            let mut t = w[i - 1];
            if i % nk == 0 {
                // RotWord, SubWord, then XOR the round constant into the first byte.
                t = [sbox[t[1] as usize] ^ rcon, sbox[t[2] as usize], sbox[t[3] as usize], sbox[t[0] as usize]];
                rcon = gmul(rcon, 2);
            } else if nk > 6 && i % nk == 4 {
                t = [sbox[t[0] as usize], sbox[t[1] as usize], sbox[t[2] as usize], sbox[t[3] as usize]];
            }
            let prev = w[i - nk];
            w.push([prev[0] ^ t[0], prev[1] ^ t[1], prev[2] ^ t[2], prev[3] ^ t[3]]);
        }
        let round_keys: Vec<[u8; 16]> = w
            .chunks(4)
            .map(|c| {
                let mut k = [0u8; 16];
                for (j, word) in c.iter().enumerate() {
                    k[4 * j..4 * j + 4].copy_from_slice(word);
                }
                k
            })
            .collect();
        Ok(Aes { round_keys })
    }

    /// Encrypts one 16-byte block (FIPS-197 section 5.1). The state is column-major, as the standard lays it out.
    pub fn encrypt_block(&self, input: &[u8; 16]) -> [u8; 16] {
        let sbox = sbox();
        let nr = self.round_keys.len() - 1;
        let mut s = *input;
        add_round_key(&mut s, &self.round_keys[0]);
        for round in 1..=nr {
            for b in s.iter_mut() {
                *b = sbox[*b as usize];
            }
            shift_rows(&mut s);
            if round != nr {
                mix_columns(&mut s);
            }
            add_round_key(&mut s, &self.round_keys[round]);
        }
        s
    }
}

fn add_round_key(s: &mut [u8; 16], k: &[u8; 16]) {
    for (b, kb) in s.iter_mut().zip(k.iter()) {
        *b ^= kb;
    }
}

/// Row r of the state moves left by r places.
fn shift_rows(s: &mut [u8; 16]) {
    let old = *s;
    for c in 0..4 {
        for r in 0..4 {
            s[r + 4 * c] = old[r + 4 * ((c + r) % 4)];
        }
    }
}

fn mix_columns(s: &mut [u8; 16]) {
    for c in 0..4 {
        let a = [s[4 * c], s[4 * c + 1], s[4 * c + 2], s[4 * c + 3]];
        s[4 * c] = gmul(a[0], 2) ^ gmul(a[1], 3) ^ a[2] ^ a[3];
        s[4 * c + 1] = a[0] ^ gmul(a[1], 2) ^ gmul(a[2], 3) ^ a[3];
        s[4 * c + 2] = a[0] ^ a[1] ^ gmul(a[2], 2) ^ gmul(a[3], 3);
        s[4 * c + 3] = gmul(a[0], 3) ^ a[1] ^ a[2] ^ gmul(a[3], 2);
    }
}

/// Multiplication in the GCM field GF(2^128) (SP 800-38D algorithm 1). Bit 0 of a block is its most significant bit.
fn gf_mul(x: u128, y: u128) -> u128 {
    const R: u128 = 0xe1u128 << 120;
    let mut z = 0u128;
    let mut v = y;
    for i in 0..128u32 {
        if (x >> (127 - i)) & 1 == 1 {
            z ^= v;
        }
        let lsb = v & 1;
        v >>= 1;
        if lsb == 1 {
            v ^= R;
        }
    }
    z
}

/// One block read big-endian, with a short final chunk padded by zero bytes on the right.
fn block_u128(chunk: &[u8]) -> u128 {
    let mut b = [0u8; 16];
    b[..chunk.len()].copy_from_slice(chunk);
    u128::from_be_bytes(b)
}

/// GHASH over the padded additional data, then the padded ciphertext, then the two bit lengths.
fn ghash(h: u128, aad: &[u8], ct: &[u8]) -> u128 {
    let mut y = 0u128;
    for chunk in aad.chunks(16).chain(ct.chunks(16)) {
        y = gf_mul(y ^ block_u128(chunk), h);
    }
    let lens = (((aad.len() as u64) * 8) as u128) << 64 | ((ct.len() as u64) * 8) as u128;
    gf_mul(y ^ lens, h)
}

/// Increments the last 32 bits of a counter block, big-endian, modulo 2^32.
fn inc32(block: &mut [u8; 16]) {
    let n = u32::from_be_bytes([block[12], block[13], block[14], block[15]]).wrapping_add(1);
    block[12..].copy_from_slice(&n.to_be_bytes());
}

/// XORs `data` with the keystream: E(inc32(J0)), E(inc32(inc32(J0))), and so on.
fn ctr_xor(aes: &Aes, j0: &[u8; 16], data: &mut [u8]) {
    let mut counter = *j0;
    for chunk in data.chunks_mut(BLOCK_LEN) {
        inc32(&mut counter);
        let keystream = aes.encrypt_block(&counter);
        for (b, k) in chunk.iter_mut().zip(keystream.iter()) {
            *b ^= k;
        }
    }
}

/// Key schedule, hash subkey H = E(0^128) and the pre-counter block J0 = nonce || 0x00000001.
fn prepare(key: &[u8], nonce: &[u8]) -> Result<(Aes, u128, [u8; 16]), Error> {
    if nonce.len() != NONCE_LEN {
        return Err(Error::BadNonceLength);
    }
    let aes = Aes::new(key)?;
    let h = u128::from_be_bytes(aes.encrypt_block(&[0u8; 16]));
    let mut j0 = [0u8; 16];
    j0[..NONCE_LEN].copy_from_slice(nonce);
    j0[15] = 1;
    Ok((aes, h, j0))
}

/// The full 16-byte tag: GHASH(H, A, C) XOR E(J0).
fn tag_for(aes: &Aes, h: u128, j0: &[u8; 16], aad: &[u8], ct: &[u8]) -> [u8; 16] {
    let s = ghash(h, aad, ct).to_be_bytes();
    let mask = aes.encrypt_block(j0);
    let mut t = [0u8; 16];
    for i in 0..16 {
        t[i] = s[i] ^ mask[i];
    }
    t
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Encrypts `plaintext` and authenticates it together with `aad`. The result is the ciphertext followed by the
/// 16-byte tag, the same layout as a combined sealed box.
pub fn seal(key: &[u8], nonce: &[u8], aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, Error> {
    if plaintext.len() as u64 > MAX_MESSAGE_LEN {
        return Err(Error::TooLong);
    }
    let (aes, h, j0) = prepare(key, nonce)?;
    let mut out = plaintext.to_vec();
    ctr_xor(&aes, &j0, &mut out);
    let tag = tag_for(&aes, h, &j0, aad, &out);
    out.extend_from_slice(&tag);
    Ok(out)
}

/// Checks the tag of a ciphertext-and-tag box made by [`seal`] and returns the plaintext. Any mismatch in the key,
/// nonce, additional data, ciphertext or tag gives [`Error::Authentication`] and no plaintext.
pub fn open(key: &[u8], nonce: &[u8], aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>, Error> {
    let (aes, h, j0) = prepare(key, nonce)?;
    if sealed.len() < TAG_LEN {
        return Err(Error::Authentication);
    }
    let (ct, tag) = sealed.split_at(sealed.len() - TAG_LEN);
    let expected = tag_for(&aes, h, &j0, aad, ct);
    if !constant_time_eq(&expected, tag) {
        return Err(Error::Authentication);
    }
    let mut out = ct.to_vec();
    ctr_xor(&aes, &j0, &mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn arr16(s: &str) -> [u8; 16] {
        let v = hex(s);
        let mut a = [0u8; 16];
        a.copy_from_slice(&v);
        a
    }

    #[test]
    fn gf256_products_match_the_standard_examples() {
        // FIPS-197 section 4.2: {57} x {83} = {c1}, and {57} x {02} = {ae} (xtime).
        assert_eq!(gmul(0x57, 0x83), 0xc1);
        assert_eq!(gmul(0x57, 0x02), 0xae);
        assert_eq!(gmul(0x53, ginv(0x53)), 1);
    }

    #[test]
    fn sbox_entries_follow_the_affine_rule() {
        // FIPS-197 section 5.1.1: S(00) = 63, S(01) = 7c, S(53) = ed.
        assert_eq!(sbox()[0x00], 0x63);
        assert_eq!(sbox()[0x01], 0x7c);
        assert_eq!(sbox()[0x53], 0xed);
    }

    #[test]
    fn aes128_block_matches_fips197_appendix_c() {
        let key = hex("000102030405060708090a0b0c0d0e0f");
        let aes = Aes::new(&key).unwrap();
        let out = aes.encrypt_block(&arr16("00112233445566778899aabbccddeeff"));
        assert_eq!(out.to_vec(), hex("69c4e0d86a7b0430d8cdb78070b4c55a"));
    }

    #[test]
    fn aes256_block_matches_fips197_appendix_c() {
        let key: Vec<u8> = (0u8..32).collect();
        let aes = Aes::new(&key).unwrap();
        let out = aes.encrypt_block(&arr16("00112233445566778899aabbccddeeff"));
        assert_eq!(out.to_vec(), hex("8ea2b7ca516745bfeafc49904b496089"));
    }

    #[test]
    fn gcm_empty_message_gives_the_standard_tag() {
        // GCM test case 1: zero key, zero nonce, no data. The tag is 58e2fccefa7e3061367f1d57a4e7455a.
        let out = seal(&[0u8; 16], &[0u8; 12], &[], &[]).unwrap();
        assert_eq!(out, hex("58e2fccefa7e3061367f1d57a4e7455a"));
    }

    #[test]
    fn gcm_one_zero_block_matches_the_standard_vector() {
        // GCM test case 2: zero key, zero nonce, one zero block.
        let out = seal(&[0u8; 16], &[0u8; 12], &[], &[0u8; 16]).unwrap();
        assert_eq!(&out[..16], hex("0388dace60b6a392f328c2b971b2fe78").as_slice());
        assert_eq!(&out[16..], hex("ab6e47d42cec13bdf53a67b21257bddf").as_slice());
    }

    #[test]
    fn gcm_empty_message_tag_is_the_encrypted_counter_block() {
        // With no data, GHASH is zero, so the tag is E(nonce || 00000001) for any key. Worked out from the rule.
        let key: Vec<u8> = (1u8..=16).collect();
        let nonce: Vec<u8> = (100u8..112).collect();
        let mut j0 = [0u8; 16];
        j0[..12].copy_from_slice(&nonce);
        j0[15] = 1;
        let expected = Aes::new(&key).unwrap().encrypt_block(&j0);
        let out = seal(&key, &nonce, &[], &[]).unwrap();
        assert_eq!(out, expected.to_vec());
    }

    #[test]
    fn gcm_round_trip_and_tamper_checks() {
        let key: Vec<u8> = (0u8..32).collect();
        let nonce = [7u8; 12];
        let aad = b"header";
        let msg = b"secret note, longer than one block of sixteen bytes";
        let boxed = seal(&key, &nonce, aad, msg).unwrap();
        assert_eq!(boxed.len(), msg.len() + TAG_LEN);
        assert_eq!(open(&key, &nonce, aad, &boxed).unwrap(), msg.to_vec());
        // A changed byte of the data, a changed tag byte and different additional data all fail.
        let mut bad_data = boxed.clone();
        bad_data[0] ^= 1;
        assert_eq!(open(&key, &nonce, aad, &bad_data), Err(Error::Authentication));
        let mut bad_tag = boxed.clone();
        let last = bad_tag.len() - 1;
        bad_tag[last] ^= 0x80;
        assert_eq!(open(&key, &nonce, aad, &bad_tag), Err(Error::Authentication));
        assert_eq!(open(&key, &nonce, b"other", &boxed), Err(Error::Authentication));
        assert_eq!(open(&key, &nonce, aad, &boxed[..TAG_LEN - 1]), Err(Error::Authentication));
    }

    #[test]
    fn gcm_rejects_bad_key_and_nonce_lengths() {
        assert_eq!(Aes::new(&[0u8; 24]).err(), Some(Error::BadKeyLength));
        assert_eq!(seal(&[0u8; 15], &[0u8; 12], &[], b"x"), Err(Error::BadKeyLength));
        assert_eq!(seal(&[0u8; 16], &[0u8; 8], &[], b"x"), Err(Error::BadNonceLength));
    }

    #[test]
    fn gf128_one_is_the_identity() {
        // In GCM bit order the field element 1 is the leading bit alone, 0x80 in the first byte.
        let one = 1u128 << 127;
        let y = 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210u128;
        assert_eq!(gf_mul(one, y), y);
        assert_eq!(gf_mul(y, one), y);
    }
}
