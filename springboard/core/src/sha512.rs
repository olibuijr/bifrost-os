//! SHA-512 and SHA-384 (FIPS 180-4), incremental. The 64-bit members of the SHA-2 family,
//! next to the 32-bit [`crate::sha256`]. Used for digests that need a wider output.

const K: [u64; 80] = [
    0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc, 0x3956c25bf348b538, 0x59f111f1b605d019,
    0x923f82a4af194f9b, 0xab1c5ed5da6d8118, 0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694, 0xe49b69c19ef14ad2, 0xefbe4786384f25e3,
    0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65, 0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5,
    0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4, 0xc6e00bf33da88fc2, 0xd5a79147930aa725,
    0x06ca6351e003826f, 0x142929670a0e6e70, 0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df,
    0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b, 0xa2bfe8a14cf10364, 0xa81a664bbc423001,
    0xc24b8b70d0f89791, 0xc76c51a30654be30, 0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8, 0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb,
    0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3, 0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec,
    0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b, 0xca273eceea26619c, 0xd186b8c721c0c207,
    0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178, 0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b,
    0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c, 0x4cc5d4becb3e42b6, 0x597f299cfc657e2a,
    0x5fcb6fab3ad6faec, 0x6c44198c4a475817,
];

const IV_512: [u64; 8] = [
    0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1, 0x510e527fade682d1, 0x9b05688c2b3e6c1f,
    0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
];

const IV_384: [u64; 8] = [
    0xcbbb9d5dc1059ed8, 0x629a292a367cd507, 0x9159015a3070dd17, 0x152fecd8f70e5939, 0x67332667ffc00b31, 0x8eb44a8768581511,
    0xdb0c2e0d64f98fa7, 0x47b5481dbefa4fa4,
];

/// Incremental SHA-512 or SHA-384 hash. Create with [`Sha512::new`] or [`Sha512::new_384`].
#[derive(Clone)]
pub struct Sha512 {
    h: [u64; 8],
    buf: [u8; 128],
    fill: usize,
    len: u64,
    out: usize,
}

impl Default for Sha512 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha512 {
    /// SHA-512: a 64-byte digest.
    pub fn new() -> Self {
        Sha512 { h: IV_512, buf: [0; 128], fill: 0, len: 0, out: 64 }
    }

    /// SHA-384: the SHA-512 compression with its own initial values, truncated to 48 bytes.
    pub fn new_384() -> Self {
        Sha512 { h: IV_384, buf: [0; 128], fill: 0, len: 0, out: 48 }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.len += data.len() as u64;
        if self.fill > 0 {
            let n = (128 - self.fill).min(data.len());
            self.buf[self.fill..self.fill + n].copy_from_slice(&data[..n]);
            self.fill += n;
            data = &data[n..];
            if self.fill < 128 {
                return;
            }
            let block = self.buf;
            self.block(&block);
            self.fill = 0;
        }
        while data.len() >= 128 {
            let (b, rest) = data.split_at(128);
            self.block(b.try_into().unwrap());
            data = rest;
        }
        self.buf[..data.len()].copy_from_slice(data);
        self.fill = data.len();
    }

    /// Pads, finishes and returns the digest: 64 bytes for SHA-512, 48 for SHA-384.
    pub fn finish(mut self) -> Vec<u8> {
        // The length field is 128 bits wide, so the bit count is widened before padding.
        let bits = (self.len as u128) * 8;
        self.update(&[0x80]);
        while self.fill != 112 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        let mut out: Vec<u8> = self.h.iter().flat_map(|w| w.to_be_bytes()).collect();
        out.truncate(self.out);
        out
    }

    fn block(&mut self, b: &[u8; 128]) {
        let mut w = [0u64; 80];
        for (i, c) in b.chunks(8).enumerate() {
            let mut word = [0u8; 8];
            word.copy_from_slice(c);
            w[i] = u64::from_be_bytes(word);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b_, mut c, mut d, mut e, mut f, mut g, mut h] = self.h;
        for i in 0..80 {
            let big_s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ (!e & g);
            let t1 = h.wrapping_add(big_s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let big_s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b_) ^ (a & c) ^ (b_ & c);
            let t2 = big_s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b_;
            b_ = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in self.h.iter_mut().zip([a, b_, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }
}

/// SHA-512 of `data` in one call.
pub fn digest512(data: &[u8]) -> [u8; 64] {
    let mut s = Sha512::new();
    s.update(data);
    let mut out = [0u8; 64];
    out.copy_from_slice(&s.finish());
    out
}

/// SHA-384 of `data` in one call.
pub fn digest384(data: &[u8]) -> [u8; 48] {
    let mut s = Sha512::new_384();
    s.update(data);
    let mut out = [0u8; 48];
    out.copy_from_slice(&s.finish());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::hex;

    #[test]
    fn sha512_known_vectors() {
        assert_eq!(
            hex(&digest512(b"")),
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
        );
        assert_eq!(
            hex(&digest512(b"abc")),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        assert_eq!(
            hex(&digest512(b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu")),
            "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909"
        );
    }

    #[test]
    fn sha384_known_vectors() {
        assert_eq!(
            hex(&digest384(b"")),
            "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b"
        );
        assert_eq!(
            hex(&digest384(b"abc")),
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
        );
    }

    #[test]
    fn output_lengths_are_64_and_48_bytes() {
        assert_eq!(Sha512::new().finish().len(), 64);
        assert_eq!(Sha512::new_384().finish().len(), 48);
    }

    #[test]
    fn chunked_updates_match_one_shot() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 7) as u8).collect();
        let mut s = Sha512::new();
        for c in data.chunks(37) {
            s.update(c);
        }
        assert_eq!(s.finish(), digest512(&data).to_vec());
        let mut t = Sha512::new_384();
        for c in data.chunks(129) {
            t.update(c);
        }
        assert_eq!(t.finish(), digest384(&data).to_vec());
    }

    #[test]
    fn million_a_sha512() {
        let million = vec![b'a'; 1_000_000];
        assert_eq!(
            hex(&digest512(&million)),
            "e718483d0ce769644e2e42c7bc15b4638e1f98b13b2044285632a803afa973ebde0ff244877ea60a4cb0432ce577c31beb009c5c2c49aa2e4eadb217ad8cc09b"
        );
    }
}
