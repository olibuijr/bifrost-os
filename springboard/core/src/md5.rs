//! MD5 (RFC 1321), incremental. Insecure: two inputs can be made to collide cheaply, so this exists only to read or
//! compare checksums that an older service still publishes. Use `sha256` to authenticate anything.

/// Per-round left-rotation amounts (RFC 1321, section 3.4).
const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, //
    5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, //
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, //
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// Additive constants, K[i] = floor(2^32 * |sin(i + 1)|) (RFC 1321, section 3.4). They are computed from that
/// definition, not typed in, so the table follows the rule it is defined by.
fn constants() -> [u32; 64] {
    let mut k = [0u32; 64];
    for (i, slot) in k.iter_mut().enumerate() {
        let x = f64::from(i as u32 + 1);
        *slot = (x.sin().abs() * 4_294_967_296.0) as u32;
    }
    k
}

/// Incremental MD5 state.
#[derive(Clone)]
pub struct Md5 {
    h: [u32; 4],
    buf: [u8; 64],
    fill: usize,
    len: u64,
    k: [u32; 64],
}

impl Default for Md5 {
    fn default() -> Self {
        Self::new()
    }
}

impl Md5 {
    pub fn new() -> Self {
        Md5 { h: [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476], buf: [0; 64], fill: 0, len: 0, k: constants() }
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

    /// Pads the message (a 0x80 byte, zeros to 56 mod 64, then the bit length as little-endian u64) and returns the
    /// 16-byte digest in little-endian word order.
    pub fn finish(mut self) -> [u8; 16] {
        let bits = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.fill != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_le_bytes());
        let mut out = [0u8; 16];
        for (o, w) in out.chunks_mut(4).zip(self.h) {
            o.copy_from_slice(&w.to_le_bytes());
        }
        out
    }

    fn block(&mut self, block: &[u8; 64]) {
        let mut m = [0u32; 16];
        for (i, c) in block.chunks_exact(4).enumerate() {
            m[i] = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        }
        let [mut a, mut b, mut c, mut d] = self.h;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(self.k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        for (s, v) in self.h.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }
}

/// MD5 digest of `data`, 16 bytes.
pub fn digest(data: &[u8]) -> [u8; 16] {
    let mut s = Md5::new();
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
    fn first_and_last_constants_follow_the_sine_rule() {
        let k = constants();
        assert_eq!(k[0], 0xd76a_a478);
        assert_eq!(k[63], 0xeb86_d391);
    }

    #[test]
    fn rfc_1321_test_suite_vectors() {
        assert_eq!(hex(&digest(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&digest(b"a")), "0cc175b9c0f1b6a831c399e269772661");
        assert_eq!(hex(&digest(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex(&digest(b"The quick brown fox jumps over the lazy dog")), "9e107d9d372bb6826bd81d3542a419d6");
    }

    #[test]
    fn chunked_updates_match_one_shot_at_every_length_around_the_block_size() {
        for len in 0..=130usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 13) as u8).collect();
            let mut s = Md5::new();
            for c in data.chunks(7) {
                s.update(c);
            }
            assert_eq!(s.finish(), digest(&data), "length {len}");
        }
    }

    #[test]
    fn a_single_changed_byte_changes_the_digest() {
        let mut data = vec![b'x'; 100];
        let before = digest(&data);
        data[99] = b'y';
        assert_ne!(before, digest(&data));
    }

    #[test]
    fn a_message_of_exactly_56_bytes_needs_a_second_padding_block() {
        // 56 bytes leave no room for the 8-byte length in the first block, so the digest must still be computed
        // over both blocks; the one-shot and a split update agree.
        let data = vec![b'q'; 56];
        let mut s = Md5::new();
        s.update(&data[..30]);
        s.update(&data[30..]);
        assert_eq!(s.finish(), digest(&data));
        assert_ne!(digest(&data), digest(&data[..55]));
    }
}
