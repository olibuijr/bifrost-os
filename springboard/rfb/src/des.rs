//! The DES block cipher (FIPS 46-3) and the VNC authentication response built on it.
//!
//! DES is here for one reason: RFB's "VNC authentication" security type (2) answers a 16-byte challenge by encrypting
//! it with DES under a key made from the password. That scheme is weak (a password is cut to 8 characters, the key is
//! 56 bits, the challenge is the only protection against replay) and the viewer says so by offering it only when the
//! person typed a password; a connection that needs more should go through an SSH tunnel. Nothing else in the system
//! uses this module. It is a plain bit-level implementation, checked against published test vectors, fast enough for
//! the two blocks a login needs.

/// Initial permutation: output bit `i` takes input bit `IP[i]` (bits numbered 1 to 64 from the most significant).
const IP: [u8; 64] = [
    58, 50, 42, 34, 26, 18, 10, 2, 60, 52, 44, 36, 28, 20, 12, 4, 62, 54, 46, 38, 30, 22, 14, 6, 64, 56, 48, 40, 32, 24, 16, 8, 57, 49, 41, 33, 25, 17, 9, 1, 59, 51,
    43, 35, 27, 19, 11, 3, 61, 53, 45, 37, 29, 21, 13, 5, 63, 55, 47, 39, 31, 23, 15, 7,
];

/// Expansion of the 32-bit half to 48 bits.
const E: [u8; 48] = [
    32, 1, 2, 3, 4, 5, 4, 5, 6, 7, 8, 9, 8, 9, 10, 11, 12, 13, 12, 13, 14, 15, 16, 17, 16, 17, 18, 19, 20, 21, 20, 21, 22, 23, 24, 25, 24, 25, 26, 27, 28, 29, 28, 29,
    30, 31, 32, 1,
];

/// Permutation of the 32 bits that leave the S-boxes.
const P: [u8; 32] = [16, 7, 20, 21, 29, 12, 28, 17, 1, 15, 23, 26, 5, 18, 31, 10, 2, 8, 24, 14, 32, 27, 3, 9, 19, 13, 30, 6, 22, 11, 4, 25];

/// Permuted choice 1: the 64-bit key to 56 bits (the parity bits go).
const PC1: [u8; 56] = [
    57, 49, 41, 33, 25, 17, 9, 1, 58, 50, 42, 34, 26, 18, 10, 2, 59, 51, 43, 35, 27, 19, 11, 3, 60, 52, 44, 36, 63, 55, 47, 39, 31, 23, 15, 7, 62, 54, 46, 38, 30, 22,
    14, 6, 61, 53, 45, 37, 29, 21, 13, 5, 28, 20, 12, 4,
];

/// Permuted choice 2: the rotated 56 bits to a 48-bit round key.
const PC2: [u8; 48] = [
    14, 17, 11, 24, 1, 5, 3, 28, 15, 6, 21, 10, 23, 19, 12, 4, 26, 8, 16, 7, 27, 20, 13, 2, 41, 52, 31, 37, 47, 55, 30, 40, 51, 45, 33, 48, 44, 49, 39, 56, 34, 53,
    46, 42, 50, 36, 29, 32,
];

/// Left rotations of the two key halves, per round.
const SHIFTS: [u32; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];

/// The eight S-boxes, each 4 rows of 16.
const SBOX: [[u8; 64]; 8] = [
    [
        14, 4, 13, 1, 2, 15, 11, 8, 3, 10, 6, 12, 5, 9, 0, 7, 0, 15, 7, 4, 14, 2, 13, 1, 10, 6, 12, 11, 9, 5, 3, 8, 4, 1, 14, 8, 13, 6, 2, 11, 15, 12, 9, 7, 3, 10, 5,
        0, 15, 12, 8, 2, 4, 9, 1, 7, 5, 11, 3, 14, 10, 0, 6, 13,
    ],
    [
        15, 1, 8, 14, 6, 11, 3, 4, 9, 7, 2, 13, 12, 0, 5, 10, 3, 13, 4, 7, 15, 2, 8, 14, 12, 0, 1, 10, 6, 9, 11, 5, 0, 14, 7, 11, 10, 4, 13, 1, 5, 8, 12, 6, 9, 3, 2,
        15, 13, 8, 10, 1, 3, 15, 4, 2, 11, 6, 7, 12, 0, 5, 14, 9,
    ],
    [
        10, 0, 9, 14, 6, 3, 15, 5, 1, 13, 12, 7, 11, 4, 2, 8, 13, 7, 0, 9, 3, 4, 6, 10, 2, 8, 5, 14, 12, 11, 15, 1, 13, 6, 4, 9, 8, 15, 3, 0, 11, 1, 2, 12, 5, 10, 14,
        7, 1, 10, 13, 0, 6, 9, 8, 7, 4, 15, 14, 3, 11, 5, 2, 12,
    ],
    [
        7, 13, 14, 3, 0, 6, 9, 10, 1, 2, 8, 5, 11, 12, 4, 15, 13, 8, 11, 5, 6, 15, 0, 3, 4, 7, 2, 12, 1, 10, 14, 9, 10, 6, 9, 0, 12, 11, 7, 13, 15, 1, 3, 14, 5, 2, 8,
        4, 3, 15, 0, 6, 10, 1, 13, 8, 9, 4, 5, 11, 12, 7, 2, 14,
    ],
    [
        2, 12, 4, 1, 7, 10, 11, 6, 8, 5, 3, 15, 13, 0, 14, 9, 14, 11, 2, 12, 4, 7, 13, 1, 5, 0, 15, 10, 3, 9, 8, 6, 4, 2, 1, 11, 10, 13, 7, 8, 15, 9, 12, 5, 6, 3, 0,
        14, 11, 8, 12, 7, 1, 14, 2, 13, 6, 15, 0, 9, 10, 4, 5, 3,
    ],
    [
        12, 1, 10, 15, 9, 2, 6, 8, 0, 13, 3, 4, 14, 7, 5, 11, 10, 15, 4, 2, 7, 12, 9, 5, 6, 1, 13, 14, 0, 11, 3, 8, 9, 14, 15, 5, 2, 8, 12, 3, 7, 0, 4, 10, 1, 13, 11,
        6, 4, 3, 2, 12, 9, 5, 15, 10, 11, 14, 1, 7, 6, 0, 8, 13,
    ],
    [
        4, 11, 2, 14, 15, 0, 8, 13, 3, 12, 9, 7, 5, 10, 6, 1, 13, 0, 11, 7, 4, 9, 1, 10, 14, 3, 5, 12, 2, 15, 8, 6, 1, 4, 11, 13, 12, 3, 7, 14, 10, 15, 6, 8, 0, 5, 9,
        2, 6, 11, 13, 8, 1, 4, 10, 7, 9, 5, 0, 15, 14, 2, 3, 12,
    ],
    [
        13, 2, 8, 4, 6, 15, 11, 1, 10, 9, 3, 14, 5, 0, 12, 7, 1, 15, 13, 8, 10, 3, 7, 4, 12, 5, 6, 11, 0, 14, 9, 2, 7, 11, 4, 1, 9, 12, 14, 2, 0, 6, 10, 13, 15, 3, 5,
        8, 2, 1, 14, 7, 4, 10, 8, 13, 15, 12, 9, 0, 3, 5, 6, 11,
    ],
];

/// Takes the bits of `input` (`in_bits` wide, numbered from 1 at the most significant) in the order `table` lists.
fn permute(input: u64, table: &[u8], in_bits: u32) -> u64 {
    let mut out = 0u64;
    for &pos in table {
        out = (out << 1) | ((input >> (in_bits - pos as u32)) & 1);
    }
    out
}

/// The final permutation, the inverse of [`IP`].
fn final_permutation() -> [u8; 64] {
    let mut fp = [0u8; 64];
    for (i, &p) in IP.iter().enumerate() {
        fp[p as usize - 1] = i as u8 + 1;
    }
    fp
}

fn rotl28(x: u64, n: u32) -> u64 {
    ((x << n) | (x >> (28 - n))) & 0x0FFF_FFFF
}

/// The sixteen 48-bit round keys of `key`.
fn round_keys(key: u64) -> [u64; 16] {
    let cd = permute(key, &PC1, 64);
    let (mut c, mut d) = ((cd >> 28) & 0x0FFF_FFFF, cd & 0x0FFF_FFFF);
    let mut keys = [0u64; 16];
    for (i, shift) in SHIFTS.iter().enumerate() {
        c = rotl28(c, *shift);
        d = rotl28(d, *shift);
        keys[i] = permute((c << 28) | d, &PC2, 56);
    }
    keys
}

/// The round function: expand the half, mix in the round key, substitute, permute.
fn feistel(r: u32, k: u64) -> u32 {
    let x = permute(r as u64, &E, 32) ^ k;
    let mut out = 0u32;
    for (i, sbox) in SBOX.iter().enumerate() {
        let six = ((x >> (42 - 6 * i as u32)) & 0x3F) as usize;
        let row = ((six >> 4) & 2) | (six & 1);
        let col = (six >> 1) & 0xF;
        out = (out << 4) | sbox[row * 16 + col] as u32;
    }
    permute(out as u64, &P, 32) as u32
}

fn crypt(block: u64, keys: &[u64; 16], decrypt: bool) -> u64 {
    let ip = permute(block, &IP, 64);
    let (mut l, mut r) = ((ip >> 32) as u32, ip as u32);
    for i in 0..16 {
        let k = if decrypt { keys[15 - i] } else { keys[i] };
        let t = r;
        r = l ^ feistel(r, k);
        l = t;
    }
    // the halves are swapped before the final permutation
    permute(((r as u64) << 32) | l as u64, &final_permutation(), 64)
}

/// Encrypts one 8-byte block under an 8-byte key (the parity bits are ignored).
pub fn encrypt_block(key: &[u8; 8], block: &[u8; 8]) -> [u8; 8] {
    crypt(u64::from_be_bytes(*block), &round_keys(u64::from_be_bytes(*key)), false).to_be_bytes()
}

/// Decrypts one 8-byte block under an 8-byte key.
pub fn decrypt_block(key: &[u8; 8], block: &[u8; 8]) -> [u8; 8] {
    crypt(u64::from_be_bytes(*block), &round_keys(u64::from_be_bytes(*key)), true).to_be_bytes()
}

/// The DES key VNC makes from a password: its first 8 bytes (zero padded), each byte with its bits in reverse order.
pub fn vnc_key(password: &str) -> [u8; 8] {
    let mut key = [0u8; 8];
    for (k, b) in key.iter_mut().zip(password.bytes()) {
        *k = b.reverse_bits();
    }
    key
}

/// The 16-byte answer to a VNC authentication challenge: each 8-byte half encrypted under the password's key.
pub fn vnc_response(challenge: &[u8; 16], password: &str) -> [u8; 16] {
    let key = vnc_key(password);
    let mut out = [0u8; 16];
    for (i, half) in challenge.chunks_exact(8).enumerate() {
        let mut block = [0u8; 8];
        block.copy_from_slice(half);
        out[i * 8..i * 8 + 8].copy_from_slice(&encrypt_block(&key, &block));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> [u8; 8] {
        let mut out = [0u8; 8];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }

    #[test]
    fn the_final_permutation_undoes_the_initial_one() {
        let fp = final_permutation();
        let x = 0x0123_4567_89AB_CDEFu64;
        assert_eq!(permute(permute(x, &IP, 64), &fp, 64), x);
        // the published first row of the final permutation
        assert_eq!(&fp[..8], &[40, 8, 48, 16, 56, 24, 64, 32]);
    }

    #[test]
    fn published_test_vectors_encrypt_as_expected() {
        // the worked example found in most descriptions of DES
        assert_eq!(encrypt_block(&hex("133457799BBCDFF1"), &hex("0123456789ABCDEF")), hex("85E813540F0AB405"));
        // all-zero key and block
        assert_eq!(encrypt_block(&hex("0000000000000000"), &hex("0000000000000000")), hex("8CA64DE9C1B123A7"));
        // NIST variable-plaintext known answer
        assert_eq!(encrypt_block(&hex("0101010101010101"), &hex("8000000000000000")), hex("95F8A5E5DD31D900"));
    }

    #[test]
    fn decrypting_returns_the_plain_block() {
        let key = hex("133457799BBCDFF1");
        let plain = hex("0123456789ABCDEF");
        assert_eq!(decrypt_block(&key, &encrypt_block(&key, &plain)), plain);
        assert_eq!(decrypt_block(&key, &hex("85E813540F0AB405")), plain);
    }

    #[test]
    fn the_vnc_key_reverses_the_bits_of_each_password_byte() {
        assert_eq!(vnc_key("a"), [0x86, 0, 0, 0, 0, 0, 0, 0], "0x61 reversed is 0x86");
        assert_eq!(vnc_key("longerthaneight"), vnc_key("longerth"), "only the first eight bytes count");
        assert_eq!(vnc_key(""), [0u8; 8]);
    }

    #[test]
    fn the_vnc_response_encrypts_both_halves_under_the_password_key() {
        let challenge: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        let resp = vnc_response(&challenge, "secret");
        let key = vnc_key("secret");
        assert_eq!(&resp[..8], &encrypt_block(&key, &hex("0001020304050607")));
        assert_eq!(&resp[8..], &encrypt_block(&key, &hex("08090A0B0C0D0E0F")));
        // the other side, which knows the password, can undo it
        assert_eq!(&decrypt_block(&key, &hex(&resp[..8].iter().map(|b| format!("{b:02X}")).collect::<String>()))[..], &challenge[..8]);
        assert_ne!(vnc_response(&challenge, "Secret"), resp, "the password is case sensitive");
    }
}
