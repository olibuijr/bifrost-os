//! AES-128 and AES-256 block cipher (FIPS 197), byte-oriented. The S-box is computed at compile time from its
//! definition (inversion in GF(2^8) and the affine map), not typed in, and the tests check it against FIPS values.

/// The field multiplication in GF(2^8) with the AES polynomial x^8 + x^4 + x^3 + x + 1.
const fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    let mut i = 0;
    while i < 8 {
        if (b & 1) != 0 {
            p ^= a;
        }
        let high = a & 0x80;
        a <<= 1;
        if high != 0 {
            a ^= 0x1b;
        }
        b >>= 1;
        i += 1;
    }
    p
}

/// x^e in GF(2^8) by square and multiply.
const fn gf_pow(x: u8, mut e: u32) -> u8 {
    let mut r = 1u8;
    let mut b = x;
    while e > 0 {
        if (e & 1) == 1 {
            r = gf_mul(r, b);
        }
        b = gf_mul(b, b);
        e >>= 1;
    }
    r
}

const fn build_sbox() -> [u8; 256] {
    let mut t = [0u8; 256];
    let mut x = 0usize;
    while x < 256 {
        // The multiplicative inverse (0 maps to 0, and x^254 is x^-1 for x != 0), then the affine transform.
        let inv = gf_pow(x as u8, 254);
        t[x] = inv ^ inv.rotate_left(1) ^ inv.rotate_left(2) ^ inv.rotate_left(3) ^ inv.rotate_left(4) ^ 0x63;
        x += 1;
    }
    t
}

const fn invert(s: &[u8; 256]) -> [u8; 256] {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[s[i] as usize] = i as u8;
        i += 1;
    }
    t
}

const SBOX: [u8; 256] = build_sbox();
const INV_SBOX: [u8; 256] = invert(&SBOX);

/// An expanded AES key: 128-bit (10 rounds) or 256-bit (14 rounds).
#[derive(Clone)]
pub struct Aes {
    /// Round keys, 16 bytes each, rounds + 1 of them.
    rk: [u8; 240],
    rounds: usize,
}

impl Aes {
    /// `None` unless the key is 16 or 32 bytes.
    pub fn new(key: &[u8]) -> Option<Aes> {
        let (nk, rounds) = match key.len() {
            16 => (4usize, 10usize),
            32 => (8, 14),
            _ => return None,
        };
        let words = 4 * (rounds + 1);
        let mut w = [0u8; 240];
        w[..key.len()].copy_from_slice(key);
        let mut rcon = 1u8;
        for i in nk..words {
            let mut t = [w[4 * (i - 1)], w[4 * (i - 1) + 1], w[4 * (i - 1) + 2], w[4 * (i - 1) + 3]];
            if i % nk == 0 {
                t = [SBOX[t[1] as usize] ^ rcon, SBOX[t[2] as usize], SBOX[t[3] as usize], SBOX[t[0] as usize]];
                rcon = gf_mul(rcon, 2);
            } else if nk > 6 && i % nk == 4 {
                t = [SBOX[t[0] as usize], SBOX[t[1] as usize], SBOX[t[2] as usize], SBOX[t[3] as usize]];
            }
            for j in 0..4 {
                w[4 * i + j] = w[4 * (i - nk) + j] ^ t[j];
            }
        }
        Some(Aes { rk: w, rounds })
    }

    fn round_key(&self, round: usize) -> [u8; 16] {
        let mut k = [0u8; 16];
        k.copy_from_slice(&self.rk[16 * round..16 * round + 16]);
        k
    }

    pub fn encrypt_block(&self, block: &mut [u8; 16]) {
        add_key(block, &self.round_key(0));
        for round in 1..self.rounds {
            sub_bytes(block, &SBOX);
            shift_rows(block);
            mix_columns(block);
            add_key(block, &self.round_key(round));
        }
        sub_bytes(block, &SBOX);
        shift_rows(block);
        add_key(block, &self.round_key(self.rounds));
    }

    pub fn decrypt_block(&self, block: &mut [u8; 16]) {
        add_key(block, &self.round_key(self.rounds));
        for round in (1..self.rounds).rev() {
            inv_shift_rows(block);
            sub_bytes(block, &INV_SBOX);
            add_key(block, &self.round_key(round));
            inv_mix_columns(block);
        }
        inv_shift_rows(block);
        sub_bytes(block, &INV_SBOX);
        add_key(block, &self.round_key(0));
    }
}

// The state is column-major: byte index r + 4c is row r, column c (FIPS 197 section 3.4).

fn add_key(s: &mut [u8; 16], k: &[u8; 16]) {
    for (a, b) in s.iter_mut().zip(k.iter()) {
        *a ^= b;
    }
}

fn sub_bytes(s: &mut [u8; 16], table: &[u8; 256]) {
    for b in s.iter_mut() {
        *b = table[*b as usize];
    }
}

fn shift_rows(s: &mut [u8; 16]) {
    let old = *s;
    for r in 0..4 {
        for c in 0..4 {
            s[r + 4 * c] = old[r + 4 * ((c + r) % 4)];
        }
    }
}

fn inv_shift_rows(s: &mut [u8; 16]) {
    let old = *s;
    for r in 0..4 {
        for c in 0..4 {
            s[r + 4 * c] = old[r + 4 * ((c + 4 - r) % 4)];
        }
    }
}

fn mix_columns(s: &mut [u8; 16]) {
    for c in 0..4 {
        let a = [s[4 * c], s[4 * c + 1], s[4 * c + 2], s[4 * c + 3]];
        s[4 * c] = gf_mul(a[0], 2) ^ gf_mul(a[1], 3) ^ a[2] ^ a[3];
        s[4 * c + 1] = a[0] ^ gf_mul(a[1], 2) ^ gf_mul(a[2], 3) ^ a[3];
        s[4 * c + 2] = a[0] ^ a[1] ^ gf_mul(a[2], 2) ^ gf_mul(a[3], 3);
        s[4 * c + 3] = gf_mul(a[0], 3) ^ a[1] ^ a[2] ^ gf_mul(a[3], 2);
    }
}

fn inv_mix_columns(s: &mut [u8; 16]) {
    for c in 0..4 {
        let a = [s[4 * c], s[4 * c + 1], s[4 * c + 2], s[4 * c + 3]];
        s[4 * c] = gf_mul(a[0], 14) ^ gf_mul(a[1], 11) ^ gf_mul(a[2], 13) ^ gf_mul(a[3], 9);
        s[4 * c + 1] = gf_mul(a[0], 9) ^ gf_mul(a[1], 14) ^ gf_mul(a[2], 11) ^ gf_mul(a[3], 13);
        s[4 * c + 2] = gf_mul(a[0], 13) ^ gf_mul(a[1], 9) ^ gf_mul(a[2], 14) ^ gf_mul(a[3], 11);
        s[4 * c + 3] = gf_mul(a[0], 11) ^ gf_mul(a[1], 13) ^ gf_mul(a[2], 9) ^ gf_mul(a[3], 14);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> alloc::string::String {
        bytes.iter().map(|b| alloc::format!("{:02x}", b)).collect()
    }

    fn block(hex_text: &str) -> [u8; 16] {
        let mut out = [0u8; 16];
        for i in 0..16 {
            out[i] = u8::from_str_radix(&hex_text[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }

    #[test]
    fn sbox_matches_fips_197_values() {
        assert_eq!(SBOX[0x00], 0x63);
        assert_eq!(SBOX[0x01], 0x7c);
        assert_eq!(SBOX[0x53], 0xed);
        assert_eq!(SBOX[0xff], 0x16);
    }

    #[test]
    fn inverse_sbox_undoes_sbox() {
        for x in 0..256usize {
            assert_eq!(INV_SBOX[SBOX[x] as usize] as usize, x);
        }
    }

    #[test]
    fn aes128_fips_197_c1() {
        let key = block("000102030405060708090a0b0c0d0e0f");
        let aes = Aes::new(&key).unwrap();
        let mut b = block("00112233445566778899aabbccddeeff");
        aes.encrypt_block(&mut b);
        assert_eq!(hex(&b), "69c4e0d86a7b0430d8cdb78070b4c55a");
        aes.decrypt_block(&mut b);
        assert_eq!(hex(&b), "00112233445566778899aabbccddeeff");
    }

    #[test]
    fn aes256_fips_197_c3() {
        let mut key = [0u8; 32];
        for (i, k) in key.iter_mut().enumerate() {
            *k = i as u8;
        }
        let aes = Aes::new(&key).unwrap();
        let mut b = block("00112233445566778899aabbccddeeff");
        aes.encrypt_block(&mut b);
        assert_eq!(hex(&b), "8ea2b7ca516745bfeafc49904b496089");
        aes.decrypt_block(&mut b);
        assert_eq!(hex(&b), "00112233445566778899aabbccddeeff");
    }

    #[test]
    fn rejects_other_key_lengths() {
        assert!(Aes::new(&[0u8; 24]).is_none());
        assert!(Aes::new(&[0u8; 15]).is_none());
    }
}
