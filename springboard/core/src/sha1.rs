//! SHA-1 (FIPS 180-4), incremental. Insecure: collisions have been produced, so this exists only to read or compare
//! checksums that an older service still publishes. Use `sha256` to authenticate anything.

/// Incremental SHA-1 state.
#[derive(Clone)]
pub struct Sha1 {
    h: [u32; 5],
    buf: [u8; 64],
    fill: usize,
    len: u64,
}

impl Default for Sha1 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha1 {
    pub fn new() -> Self {
        Sha1 { h: [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476, 0xc3d2_e1f0], buf: [0; 64], fill: 0, len: 0 }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.len += data.len() as u64;
        if self.fill > 0 {
            let n = (64 - self.fill).min(data.len());
            self.buf[self.fill..self.fill + n].copy_from_slice(&data[..n]);
            self.fill += n;
            data = &data[n..];
            if self.fill < 64 {
                return;
            }
            let block = self.buf;
            self.block(&block);
            self.fill = 0;
        }
        while data.len() >= 64 {
            let (b, rest) = data.split_at(64);
            self.block(b.try_into().unwrap());
            data = rest;
        }
        self.buf[..data.len()].copy_from_slice(data);
        self.fill = data.len();
    }

    /// Pads the message (a 0x80 byte, zeros to 56 mod 64, then the bit length as big-endian u64) and returns the
    /// 20-byte digest.
    pub fn finish(mut self) -> [u8; 20] {
        let bits = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.fill != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        let mut out = [0u8; 20];
        for (o, w) in out.chunks_mut(4).zip(self.h) {
            o.copy_from_slice(&w.to_be_bytes());
        }
        out
    }

    fn block(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 80];
        for (i, c) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for t in 16..80 {
            w[t] = (w[t - 3] ^ w[t - 8] ^ w[t - 14] ^ w[t - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = self.h;
        for (t, wt) in w.iter().enumerate() {
            let (f, k) = match t / 20 {
                0 => ((b & c) | (!b & d), 0x5a82_7999u32),
                1 => (b ^ c ^ d, 0x6ed9_eba1),
                2 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wt);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        for (s, v) in self.h.iter_mut().zip([a, b, c, d, e]) {
            *s = s.wrapping_add(v);
        }
    }
}

/// SHA-1 digest of `data`, 20 bytes.
pub fn digest(data: &[u8]) -> [u8; 20] {
    let mut s = Sha1::new();
    s.update(data);
    s.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(d: &[u8]) -> String {
        d.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn fips_180_vectors() {
        assert_eq!(hex(&digest(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(hex(&digest(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            hex(&digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
    }

    #[test]
    fn the_quick_brown_fox_vector() {
        assert_eq!(hex(&digest(b"The quick brown fox jumps over the lazy dog")), "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12");
    }

    #[test]
    fn chunked_updates_match_one_shot_at_every_length_around_the_block_size() {
        for len in 0..=130usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 11) as u8).collect();
            let mut s = Sha1::new();
            for c in data.chunks(9) {
                s.update(c);
            }
            assert_eq!(s.finish(), digest(&data), "length {len}");
        }
    }

    #[test]
    fn a_digest_is_twenty_bytes_and_depends_on_every_byte() {
        let mut data = vec![0u8; 64];
        let before = digest(&data);
        assert_eq!(before.len(), 20);
        data[63] = 1;
        assert_ne!(before, digest(&data));
    }

    #[test]
    fn a_message_of_a_million_a_characters_matches_the_fips_vector() {
        let data = vec![b'a'; 1_000_000];
        assert_eq!(hex(&digest(&data)), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
    }
}
