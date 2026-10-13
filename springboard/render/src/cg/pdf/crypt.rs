//! Cryptography for the standard security handler (empty user password):
//! MD5, SHA-2, RC4 and AES, and the key derivation of revisions 2 to 6.
//! Written out here so the PDF reader needs no crypto crate.

use super::obj::{Dict, Obj};

const MD5_K: [u32; 64] = [0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1, 0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391];
const MD5_S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
    15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];
const K256: [u32; 64] = [0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2];
const K512: [u64; 80] = [0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc, 0x3956c25bf348b538, 0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118, 0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2, 0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694, 0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65, 0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5, 0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4, 0xc6e00bf33da88fc2, 0xd5a79147930aa725, 0x06ca6351e003826f, 0x142929670a0e6e70, 0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df, 0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b, 0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30, 0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8, 0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8, 0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3, 0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec, 0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b, 0xca273eceea26619c, 0xd186b8c721c0c207, 0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178, 0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b, 0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c, 0x4cc5d4becb3e42b6, 0x597f299cfc657e2a, 0x5fcb6fab3ad6faec, 0x6c44198c4a475817];
const H256: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
const H512: [u64; 8] = [0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1, 0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179];
const H384: [u64; 8] = [0xcbbb9d5dc1059ed8, 0x629a292a367cd507, 0x9159015a3070dd17, 0x152fecd8f70e5939, 0x67332667ffc00b31, 0x8eb44a8768581511, 0xdb0c2e0d64f98fa7, 0x47b5481dbefa4fa4];
const SBOX: [u8; 256] = [0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76, 0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0, 0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15, 0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75, 0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84, 0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf, 0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8, 0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2, 0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73, 0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb, 0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79, 0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08, 0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a, 0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e, 0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf, 0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16];
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69,
    0x7A,
];

pub fn md5(data: &[u8]) -> [u8; 16] {
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f2 = f.wrapping_add(a).wrapping_add(MD5_K[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f2.rotate_left(MD5_S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, v) in [a0, b0, c0, d0].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_be_bytes());
    let mut h = H256;
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K256[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

fn sha512_with(data: &[u8], init: [u64; 8]) -> [u8; 64] {
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 128 != 112 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u128).wrapping_mul(8)).to_be_bytes());
    let mut h = init;
    for chunk in msg.chunks(128) {
        let mut w = [0u64; 80];
        for i in 0..16 {
            let mut b = [0u8; 8];
            b.copy_from_slice(&chunk[8 * i..8 * i + 8]);
            w[i] = u64::from_be_bytes(b);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..80 {
            let s1 = v[4].rotate_right(14) ^ v[4].rotate_right(18) ^ v[4].rotate_right(41);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K512[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(28) ^ v[0].rotate_right(34) ^ v[0].rotate_right(39);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = [0u8; 64];
    for (i, v) in h.iter().enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&v.to_be_bytes());
    }
    out
}

pub fn sha512(data: &[u8]) -> [u8; 64] {
    sha512_with(data, H512)
}

pub fn sha384(data: &[u8]) -> [u8; 48] {
    let full = sha512_with(data, H384);
    let mut out = [0u8; 48];
    out.copy_from_slice(&full[..48]);
    out
}

pub fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return data.to_vec();
    }
    let mut s: Vec<u8> = (0..=255).collect();
    let mut j = 0usize;
    for i in 0..256 {
        j = (j + s[i] as usize + key[i % key.len()] as usize) & 255;
        s.swap(i, j);
    }
    let (mut i, mut j) = (0usize, 0usize);
    data.iter()
        .map(|&b| {
            i = (i + 1) & 255;
            j = (j + s[i] as usize) & 255;
            s.swap(i, j);
            b ^ s[(s[i] as usize + s[j] as usize) & 255]
        })
        .collect()
}

fn xtime(a: u8) -> u8 {
    (a << 1) ^ if a & 0x80 != 0 { 0x1b } else { 0 }
}

fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    p
}

/// AES with a 128, 192 or 256 bit key; the state is column-major as in the standard.
pub struct Aes {
    rk: Vec<[u8; 16]>,
    inv: [u8; 256],
}

impl Aes {
    pub fn new(key: &[u8]) -> Option<Aes> {
        let nk = match key.len() {
            16 => 4,
            24 => 6,
            32 => 8,
            _ => return None,
        };
        let nr = nk + 6;
        let mut w: Vec<[u8; 4]> = Vec::with_capacity(4 * (nr + 1));
        for i in 0..nk {
            w.push([key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]]);
        }
        let mut rcon = 1u8;
        for i in nk..4 * (nr + 1) {
            let mut t = w[i - 1];
            if i % nk == 0 {
                t = [SBOX[t[1] as usize] ^ rcon, SBOX[t[2] as usize], SBOX[t[3] as usize], SBOX[t[0] as usize]];
                rcon = xtime(rcon);
            } else if nk > 6 && i % nk == 4 {
                t = [SBOX[t[0] as usize], SBOX[t[1] as usize], SBOX[t[2] as usize], SBOX[t[3] as usize]];
            }
            let p = w[i - nk];
            w.push([p[0] ^ t[0], p[1] ^ t[1], p[2] ^ t[2], p[3] ^ t[3]]);
        }
        let rk = (0..=nr)
            .map(|r| {
                let mut k = [0u8; 16];
                for c in 0..4 {
                    k[4 * c..4 * c + 4].copy_from_slice(&w[4 * r + c]);
                }
                k
            })
            .collect();
        let mut inv = [0u8; 256];
        for (i, &s) in SBOX.iter().enumerate() {
            inv[s as usize] = i as u8;
        }
        Some(Aes { rk, inv })
    }

    fn add(s: &mut [u8; 16], k: &[u8; 16]) {
        for i in 0..16 {
            s[i] ^= k[i];
        }
    }

    pub fn encrypt_block(&self, block: &mut [u8; 16]) {
        let nr = self.rk.len() - 1;
        Self::add(block, &self.rk[0]);
        for r in 1..=nr {
            for b in block.iter_mut() {
                *b = SBOX[*b as usize];
            }
            let old = *block;
            for c in 0..4 {
                for row in 0..4 {
                    block[row + 4 * c] = old[row + 4 * ((c + row) % 4)];
                }
            }
            if r != nr {
                for c in 0..4 {
                    let col = [block[4 * c], block[4 * c + 1], block[4 * c + 2], block[4 * c + 3]];
                    for row in 0..4 {
                        block[4 * c + row] = xtime(col[row]) ^ (xtime(col[(row + 1) % 4]) ^ col[(row + 1) % 4]) ^ col[(row + 2) % 4] ^ col[(row + 3) % 4];
                    }
                }
            }
            Self::add(block, &self.rk[r]);
        }
    }

    pub fn decrypt_block(&self, block: &mut [u8; 16]) {
        let nr = self.rk.len() - 1;
        Self::add(block, &self.rk[nr]);
        for r in (0..nr).rev() {
            let old = *block;
            for c in 0..4 {
                for row in 0..4 {
                    block[row + 4 * ((c + row) % 4)] = old[row + 4 * c];
                }
            }
            for b in block.iter_mut() {
                *b = self.inv[*b as usize];
            }
            Self::add(block, &self.rk[r]);
            if r != 0 {
                for c in 0..4 {
                    let col = [block[4 * c], block[4 * c + 1], block[4 * c + 2], block[4 * c + 3]];
                    for row in 0..4 {
                        block[4 * c + row] = gmul(col[row], 14) ^ gmul(col[(row + 1) % 4], 11) ^ gmul(col[(row + 2) % 4], 13) ^ gmul(col[(row + 3) % 4], 9);
                    }
                }
            }
        }
    }
}

/// CBC decryption without padding removal; `iv` is the chaining start.
pub fn aes_cbc_decrypt_raw(key: &[u8], iv: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    let aes = Aes::new(key)?;
    let mut prev = [0u8; 16];
    prev.copy_from_slice(iv.get(..16)?);
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(16) {
        let mut b = [0u8; 16];
        b.copy_from_slice(chunk);
        let cipher = b;
        aes.decrypt_block(&mut b);
        for i in 0..16 {
            b[i] ^= prev[i];
        }
        out.extend_from_slice(&b);
        prev = cipher;
    }
    Some(out)
}

/// CBC encryption of whole blocks (no padding).
pub fn aes_cbc_encrypt_raw(key: &[u8], iv: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    let aes = Aes::new(key)?;
    let mut prev = [0u8; 16];
    prev.copy_from_slice(iv.get(..16)?);
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(16) {
        let mut b = [0u8; 16];
        for i in 0..16 {
            b[i] = chunk[i] ^ prev[i];
        }
        aes.encrypt_block(&mut b);
        out.extend_from_slice(&b);
        prev = b;
    }
    Some(out)
}

/// A PDF AES stream or string: 16 byte IV, CBC, PKCS#7 padding.
pub fn aes_decrypt(key: &[u8], data: &[u8]) -> Vec<u8> {
    if data.len() < 32 {
        return Vec::new();
    }
    let body = &data[16..data.len() - (data.len() - 16) % 16];
    let Some(mut out) = aes_cbc_decrypt_raw(key, &data[..16], body) else { return Vec::new() };
    if let Some(&p) = out.last() {
        let p = p as usize;
        if (1..=16).contains(&p) && p <= out.len() {
            out.truncate(out.len() - p);
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    Identity,
    Rc4,
    AesV2,
    AesV3,
}

/// The decryptor of an encrypted document, opened with the empty user password.
pub struct Crypt {
    key: Vec<u8>,
    stm: Method,
    strm: Method,
    pub encrypt_metadata: bool,
}

fn hash_r6(password: &[u8], salt: &[u8], udata: &[u8]) -> Vec<u8> {
    let mut input = password.to_vec();
    input.extend_from_slice(salt);
    input.extend_from_slice(udata);
    let mut k: Vec<u8> = sha256(&input).to_vec();
    let mut i = 0usize;
    let mut e: Vec<u8> = vec![0];
    while i < 64 || (*e.last().unwrap_or(&0) as i64) > i as i64 - 32 {
        let mut unit = password.to_vec();
        unit.extend_from_slice(&k);
        unit.extend_from_slice(udata);
        let k1: Vec<u8> = unit.iter().cycle().take(unit.len() * 64).copied().collect();
        e = aes_cbc_encrypt_raw(&k[..16], &k[16..32], &k1).unwrap_or_default();
        let rem = e[..16.min(e.len())].iter().map(|&b| b as u32).sum::<u32>() % 3;
        k = match rem {
            0 => sha256(&e).to_vec(),
            1 => sha384(&e).to_vec(),
            _ => sha512(&e).to_vec(),
        };
        i += 1;
    }
    k.truncate(32);
    k
}

impl Crypt {
    /// Builds the decryptor from the (resolved) Encrypt dictionary and the first file ID.
    pub fn open(enc: &Dict, id0: &[u8]) -> Result<Crypt, String> {
        let int = |k: &str| enc.get(k).and_then(Obj::int);
        let bytes = |k: &str| match enc.get(k) {
            Some(Obj::Str(s)) => s.clone(),
            _ => Vec::new(),
        };
        if enc.get("Filter").and_then(Obj::name).is_some_and(|f| f != "Standard") {
            return Err("this PDF uses an unsupported security handler".into());
        }
        let v = int("V").unwrap_or(0);
        let r = int("R").unwrap_or(2);
        let p = int("P").unwrap_or(0) as i32;
        let encrypt_metadata = !matches!(enc.get("EncryptMetadata"), Some(Obj::Bool(false)));
        let (o, u) = (bytes("O"), bytes("U"));
        let method_of = |name: Option<&str>| -> Method {
            let Some(name) = name else { return Method::Identity };
            if name == "Identity" {
                return Method::Identity;
            }
            let cf = match enc.get("CF") {
                Some(Obj::Dict(cf)) => cf,
                _ => return Method::Rc4,
            };
            match cf.get(name) {
                Some(Obj::Dict(f)) => match f.get("CFM").and_then(Obj::name) {
                    Some("V2") => Method::Rc4,
                    Some("AESV2") => Method::AesV2,
                    Some("AESV3") => Method::AesV3,
                    _ => Method::Identity,
                },
                _ => Method::Identity,
            }
        };
        let (stm, strm) = if v >= 4 {
            (method_of(enc.get("StmF").and_then(Obj::name)), method_of(enc.get("StrF").and_then(Obj::name)))
        } else {
            (Method::Rc4, Method::Rc4)
        };
        if r >= 5 {
            // AES-256: the file key is wrapped by a key made from the (empty) password
            if u.len() < 48 {
                return Err("damaged encryption dictionary".into());
            }
            let ue = bytes("UE");
            let (vsalt, ksalt) = (&u[32..40], &u[40..48]);
            let hash = |salt: &[u8]| -> Vec<u8> {
                if r == 5 {
                    let mut s = salt.to_vec();
                    s.truncate(8);
                    sha256(&s).to_vec()
                } else {
                    hash_r6(b"", salt, b"")
                }
            };
            if hash(vsalt)[..] != u[..32] {
                return Err("this PDF is encrypted".into());
            }
            let ik = hash(ksalt);
            let key = aes_cbc_decrypt_raw(&ik, &[0u8; 16], ue.get(..32).unwrap_or(&[])).ok_or("damaged encryption dictionary")?;
            let aes3 = |m: Method| if m == Method::Identity { m } else { Method::AesV3 };
            return Ok(Crypt { key, stm: aes3(stm), strm: aes3(strm), encrypt_metadata });
        }
        let len_bits = if v == 1 { 40 } else { int("Length").unwrap_or(40) };
        let n = if r == 2 { 5 } else { ((len_bits / 8) as usize).clamp(5, 16) };
        let mut input = PAD.to_vec();
        input.extend_from_slice(&o);
        input.extend_from_slice(&(p as u32).to_le_bytes());
        input.extend_from_slice(id0);
        if r >= 4 && !encrypt_metadata {
            input.extend_from_slice(&[0xff; 4]);
        }
        let mut h = md5(&input);
        if r >= 3 {
            for _ in 0..50 {
                h = md5(&h[..n]);
            }
        }
        let key = h[..n].to_vec();
        // verify against /U so a non-empty password is reported rather than producing garbage
        let check = if r == 2 {
            rc4(&key, &PAD)
        } else {
            let mut c = PAD.to_vec();
            c.extend_from_slice(id0);
            let mut x = rc4(&key, &md5(&c));
            for i in 1..20u8 {
                let k2: Vec<u8> = key.iter().map(|b| b ^ i).collect();
                x = rc4(&k2, &x);
            }
            x
        };
        let cmp = if r == 2 { 32 } else { 16 };
        if u.len() >= cmp && check.len() >= cmp && u[..cmp] != check[..cmp] {
            return Err("this PDF is encrypted".into());
        }
        Ok(Crypt { key, stm, strm, encrypt_metadata })
    }

    fn object_key(&self, num: u32, gen: u16, aes: bool) -> Vec<u8> {
        let mut k = self.key.clone();
        k.extend_from_slice(&num.to_le_bytes()[..3]);
        k.extend_from_slice(&gen.to_le_bytes());
        if aes {
            k.extend_from_slice(b"sAlT");
        }
        let h = md5(&k);
        h[..(self.key.len() + 5).min(16)].to_vec()
    }

    fn apply(&self, m: Method, num: u32, gen: u16, data: &[u8]) -> Vec<u8> {
        match m {
            Method::Identity => data.to_vec(),
            Method::Rc4 => rc4(&self.object_key(num, gen, false), data),
            Method::AesV2 => aes_decrypt(&self.object_key(num, gen, true), data),
            Method::AesV3 => aes_decrypt(&self.key, data),
        }
    }

    pub fn string(&self, num: u32, gen: u16, data: &[u8]) -> Vec<u8> {
        self.apply(self.strm, num, gen, data)
    }

    pub fn stream(&self, num: u32, gen: u16, data: &[u8]) -> Vec<u8> {
        self.apply(self.stm, num, gen, data)
    }

    /// Decrypts every string inside `o` (streams are handled by the caller).
    pub fn object(&self, num: u32, gen: u16, o: &mut Obj) {
        match o {
            Obj::Str(s) => *s = self.string(num, gen, s),
            Obj::Array(a) => a.iter_mut().for_each(|x| self.object(num, gen, x)),
            Obj::Dict(d) => d.values_mut().for_each(|x| self.object(num, gen, x)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn digests_match_the_standard_vectors() {
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex(&sha384(b"abc")), "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7");
        assert!(hex(&sha512(b"abc")).starts_with("ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a"));
    }

    #[test]
    fn rc4_vector() {
        assert_eq!(hex(&rc4(b"Key", b"Plaintext")), "bbf316e8d940af0ad3");
    }

    #[test]
    fn aes_matches_the_fips_vectors() {
        let key: Vec<u8> = (0..16).collect();
        let mut block = [0u8; 16];
        for (i, b) in block.iter_mut().enumerate() {
            *b = (i as u8) * 0x11;
        }
        let aes = Aes::new(&key).unwrap();
        aes.encrypt_block(&mut block);
        assert_eq!(hex(&block), "69c4e0d86a7b0430d8cdb78070b4c55a");
        aes.decrypt_block(&mut block);
        assert_eq!(hex(&block), "00112233445566778899aabbccddeeff");
        let key: Vec<u8> = (0..32).collect();
        let mut block = [0u8; 16];
        for (i, b) in block.iter_mut().enumerate() {
            *b = (i as u8) * 0x11;
        }
        let aes = Aes::new(&key).unwrap();
        aes.encrypt_block(&mut block);
        assert_eq!(hex(&block), "8ea2b7ca516745bfeafc49904b496089");
    }

    #[test]
    fn cbc_round_trip_with_padding() {
        let key = [7u8; 16];
        let iv = [3u8; 16];
        let mut plain = b"hello pdf".to_vec();
        let pad = 16 - plain.len() % 16;
        plain.extend(std::iter::repeat(pad as u8).take(pad));
        let mut data = iv.to_vec();
        data.extend(aes_cbc_encrypt_raw(&key, &iv, &plain).unwrap());
        assert_eq!(aes_decrypt(&key, &data), b"hello pdf");
    }
}
