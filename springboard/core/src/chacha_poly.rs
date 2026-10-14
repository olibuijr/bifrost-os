//! ChaCha20-Poly1305 authenticated encryption (RFC 8439, section 2.8), pure std Rust.
//!
//! [`seal`] encrypts a message and appends a 16-byte Poly1305 tag computed over the additional data and the
//! ciphertext. [`open`] checks the tag in constant time and decrypts only when it matches. Keys are 32 bytes and
//! nonces are 12 bytes. The Poly1305 accumulator uses five 26-bit limbs, so no wide multiplication is needed. The
//! ChaCha20 rounds and the limb arithmetic take the same path for every input, so the only data-dependent branch
//! is the final tag comparison.

/// Key length in bytes (256 bits).
pub const KEY_LEN: usize = 32;
/// Nonce length in bytes (96 bits).
pub const NONCE_LEN: usize = 12;
/// Authentication tag length in bytes.
pub const TAG_LEN: usize = 16;

const MASK: u64 = 0x3ff_ffff;
/// The clamp applied to the Poly1305 `r` half of the one-time key (RFC 8439, section 2.5).
const CLAMP: u128 = 0x0fff_fffc_0fff_fffc_0fff_fffc_0fff_ffff;

/// Why a seal, open or MAC call was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The key is not [`KEY_LEN`] bytes.
    BadKeyLength,
    /// The nonce is not [`NONCE_LEN`] bytes.
    BadNonceLength,
    /// The tag does not match the ciphertext, the additional data or the key, or the sealed input is shorter than a tag.
    Authentication,
}

/// Encrypts `plain` and binds it to `aad`. The result is the ciphertext followed by the 16-byte tag.
pub fn seal(key: &[u8], nonce: &[u8], aad: &[u8], plain: &[u8]) -> Result<Vec<u8>, Error> {
    let key = key_array(key)?;
    let nonce = nonce_array(nonce)?;
    let mut out = plain.to_vec();
    chacha20_xor(&key, 1, &nonce, &mut out);
    let tag = aead_tag(&key, &nonce, aad, &out);
    out.extend_from_slice(&tag);
    Ok(out)
}

/// Checks the tag of `sealed` (ciphertext followed by the 16-byte tag) and returns the plaintext.
pub fn open(key: &[u8], nonce: &[u8], aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>, Error> {
    let key = key_array(key)?;
    let nonce = nonce_array(nonce)?;
    if sealed.len() < TAG_LEN {
        return Err(Error::Authentication);
    }
    let (cipher, tag) = sealed.split_at(sealed.len() - TAG_LEN);
    let want = aead_tag(&key, &nonce, aad, cipher);
    let diff = want.iter().zip(tag).fold(0u8, |acc, (a, b)| acc | (a ^ b));
    if diff != 0 {
        return Err(Error::Authentication);
    }
    let mut out = cipher.to_vec();
    chacha20_xor(&key, 1, &nonce, &mut out);
    Ok(out)
}

/// The Poly1305 one-time authenticator of `message` under a 32-byte key (RFC 8439, section 2.5).
pub fn poly1305(key: &[u8], message: &[u8]) -> Result<[u8; TAG_LEN], Error> {
    Ok(poly1305_mac(&key_array(key)?, message))
}

fn key_array(key: &[u8]) -> Result<[u8; KEY_LEN], Error> {
    key.try_into().map_err(|_| Error::BadKeyLength)
}

fn nonce_array(nonce: &[u8]) -> Result<[u8; NONCE_LEN], Error> {
    nonce.try_into().map_err(|_| Error::BadNonceLength)
}

fn quarter_round(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(7);
}

/// The 64-byte ChaCha20 keystream block for `counter` (RFC 8439, section 2.3).
fn chacha20_block(key: &[u8; KEY_LEN], counter: u32, nonce: &[u8; NONCE_LEN]) -> [u8; 64] {
    let mut init = [0u32; 16];
    init[..4].copy_from_slice(&[0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574]);
    for (i, word) in init[4..12].iter_mut().enumerate() {
        *word = u32::from_le_bytes([key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]]);
    }
    init[12] = counter;
    for (i, word) in init[13..16].iter_mut().enumerate() {
        *word = u32::from_le_bytes([nonce[4 * i], nonce[4 * i + 1], nonce[4 * i + 2], nonce[4 * i + 3]]);
    }
    let mut s = init;
    for _ in 0..10 {
        quarter_round(&mut s, 0, 4, 8, 12);
        quarter_round(&mut s, 1, 5, 9, 13);
        quarter_round(&mut s, 2, 6, 10, 14);
        quarter_round(&mut s, 3, 7, 11, 15);
        quarter_round(&mut s, 0, 5, 10, 15);
        quarter_round(&mut s, 1, 6, 11, 12);
        quarter_round(&mut s, 2, 7, 8, 13);
        quarter_round(&mut s, 3, 4, 9, 14);
    }
    let mut out = [0u8; 64];
    for (i, (w, base)) in s.iter().zip(init).enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&w.wrapping_add(base).to_le_bytes());
    }
    out
}

/// XORs `data` with the keystream that starts at block `counter`.
fn chacha20_xor(key: &[u8; KEY_LEN], counter: u32, nonce: &[u8; NONCE_LEN], data: &mut [u8]) {
    for (i, chunk) in data.chunks_mut(64).enumerate() {
        let block = chacha20_block(key, counter.wrapping_add(i as u32), nonce);
        for (byte, k) in chunk.iter_mut().zip(block) {
            *byte ^= k;
        }
    }
}

/// The Poly1305 input for the AEAD construction: padded additional data, padded ciphertext, then both lengths.
fn aead_mac_data(aad: &[u8], cipher: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(aad.len() + cipher.len() + 48);
    data.extend_from_slice(aad);
    data.resize(data.len() + (16 - aad.len() % 16) % 16, 0);
    data.extend_from_slice(cipher);
    data.resize(data.len() + (16 - cipher.len() % 16) % 16, 0);
    data.extend_from_slice(&(aad.len() as u64).to_le_bytes());
    data.extend_from_slice(&(cipher.len() as u64).to_le_bytes());
    data
}

fn aead_tag(key: &[u8; KEY_LEN], nonce: &[u8; NONCE_LEN], aad: &[u8], cipher: &[u8]) -> [u8; TAG_LEN] {
    let block = chacha20_block(key, 0, nonce);
    let mut one_time_key = [0u8; KEY_LEN];
    one_time_key.copy_from_slice(&block[..KEY_LEN]);
    poly1305_mac(&one_time_key, &aead_mac_data(aad, cipher))
}

fn carry_all(h: &mut [u64; 5]) {
    for i in 0..4 {
        let c = h[i] >> 26;
        h[i] &= MASK;
        h[i + 1] += c;
    }
    let c = h[4] >> 26;
    h[4] &= MASK;
    h[0] += c * 5;
    let c = h[0] >> 26;
    h[0] &= MASK;
    h[1] += c;
}

fn poly1305_mac(key: &[u8; KEY_LEN], message: &[u8]) -> [u8; TAG_LEN] {
    let r = u128::from_le_bytes(key[..16].try_into().unwrap()) & CLAMP;
    let s = u128::from_le_bytes(key[16..].try_into().unwrap());
    let mask = MASK as u128;
    let mut acc = Accumulator {
        r: [
            (r & mask) as u64,
            ((r >> 26) & mask) as u64,
            ((r >> 52) & mask) as u64,
            ((r >> 78) & mask) as u64,
            ((r >> 104) & mask) as u64,
        ],
        h: [0; 5],
    };
    for chunk in message.chunks(16) {
        let mut block = [0u8; 16];
        block[..chunk.len()].copy_from_slice(chunk);
        let hibit = if chunk.len() == 16 {
            1 << 24
        } else {
            block[chunk.len()] = 1;
            0
        };
        acc.absorb(&block, hibit);
    }
    acc.finish().wrapping_add(s).to_le_bytes()
}

/// The Poly1305 accumulator `h` and clamped key `r`, in five 26-bit limbs.
struct Accumulator {
    r: [u64; 5],
    h: [u64; 5],
}

impl Accumulator {
    /// Adds one 16-byte block (with the 2^128 bit `hibit` for full blocks) and multiplies by `r` modulo 2^130 - 5.
    fn absorb(&mut self, block: &[u8; 16], hibit: u64) {
        let x = u128::from_le_bytes(*block);
        let mask = MASK as u128;
        self.h[0] += (x & mask) as u64;
        self.h[1] += ((x >> 26) & mask) as u64;
        self.h[2] += ((x >> 52) & mask) as u64;
        self.h[3] += ((x >> 78) & mask) as u64;
        self.h[4] += ((x >> 104) as u64) | hibit;

        let [r0, r1, r2, r3, r4] = self.r;
        let (s1, s2, s3, s4) = (r1 * 5, r2 * 5, r3 * 5, r4 * 5);
        let [h0, h1, h2, h3, h4] = self.h;
        let d = [
            h0 * r0 + h1 * s4 + h2 * s3 + h3 * s2 + h4 * s1,
            h0 * r1 + h1 * r0 + h2 * s4 + h3 * s3 + h4 * s2,
            h0 * r2 + h1 * r1 + h2 * r0 + h3 * s4 + h4 * s3,
            h0 * r3 + h1 * r2 + h2 * r1 + h3 * r0 + h4 * s4,
            h0 * r4 + h1 * r3 + h2 * r2 + h3 * r1 + h4 * r0,
        ];
        let mut out = [0u64; 5];
        let mut carry = 0u64;
        for (o, v) in out.iter_mut().zip(d) {
            let t = v + carry;
            *o = t & MASK;
            carry = t >> 26;
        }
        out[0] += carry * 5;
        let c = out[0] >> 26;
        out[0] &= MASK;
        out[1] += c;
        self.h = out;
    }

    /// The accumulator reduced below 2^130 - 5 and truncated to 128 bits, ready to add the `s` half of the key.
    fn finish(&self) -> u128 {
        let mut h = self.h;
        carry_all(&mut h);
        // g = h + 5. Its carry out of bit 130 is set exactly when h >= 2^130 - 5, the case where g is the result.
        let mut g = [0u64; 5];
        let mut c = 5u64;
        for (gi, hi) in g.iter_mut().zip(h) {
            let t = hi + c;
            *gi = t & MASK;
            c = t >> 26;
        }
        let take_g = c.wrapping_neg();
        for (hi, gi) in h.iter_mut().zip(g) {
            *hi = (*hi & !take_g) | (gi & take_g);
        }
        h.iter().enumerate().fold(0u128, |acc, (i, limb)| acc | ((*limb as u128) << (26 * i)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poly1305_matches_the_rfc_8439_section_2_5_2_vector() {
        let key = [
            0x85, 0xd6, 0xbe, 0x78, 0x57, 0x55, 0x6d, 0x33, 0x7f, 0x44, 0x52, 0xfe, 0x42, 0xd5, 0x06, 0xa8, 0x01, 0x03,
            0x80, 0x8a, 0xfb, 0x0d, 0xb2, 0xfd, 0x4a, 0xbf, 0xf6, 0xaf, 0x41, 0x49, 0xf5, 0x1b,
        ];
        let tag = poly1305(&key, b"Cryptographic Forum Research Group").unwrap();
        assert_eq!(
            tag,
            [0xa8, 0x06, 0x1d, 0xc1, 0x30, 0x51, 0x36, 0xc6, 0xc2, 0x2b, 0x8b, 0xaf, 0x0c, 0x01, 0x27, 0xa9]
        );
    }
}
