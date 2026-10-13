//! DEFLATE (RFC 1951) and zlib (RFC 1950) decoding, written out here so the
//! Books engine needs no compression crate. Canonical-Huffman decoding in
//! the style of zlib's `puff.c`.

/// Output cap, so a hostile zip bomb cannot eat the machine's memory.
pub const MAX_OUT: usize = 256 << 20;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Truncated,
    BadBlock,
    BadCode,
    BadDistance,
    TooBig,
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    n: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bits { data, pos: 0, buf: 0, n: 0 }
    }

    fn bit(&mut self) -> Result<u32, Error> {
        if self.n == 0 {
            let b = *self.data.get(self.pos).ok_or(Error::Truncated)?;
            self.pos += 1;
            self.buf = b as u32;
            self.n = 8;
        }
        let v = self.buf & 1;
        self.buf >>= 1;
        self.n -= 1;
        Ok(v)
    }

    /// `count` bits, least significant first.
    fn bits(&mut self, count: u32) -> Result<u32, Error> {
        let mut v = 0;
        for i in 0..count {
            v |= self.bit()? << i;
        }
        Ok(v)
    }

    fn align(&mut self) {
        self.n = 0;
        self.buf = 0;
    }
}

/// A canonical Huffman code: how many symbols have each length, and the
/// symbols ordered by code.
struct Huffman {
    count: [u16; 16],
    symbol: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Huffman {
        let mut count = [0u16; 16];
        for &l in lengths {
            count[l as usize] += 1;
        }
        let mut offs = [0u16; 16];
        for i in 1..15 {
            offs[i + 1] = offs[i] + count[i];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        Huffman { count, symbol }
    }

    fn decode(&self, b: &mut Bits) -> Result<u16, Error> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= b.bit()? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return self.symbol.get((index + (code - first)) as usize).copied().ok_or(Error::BadCode);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(Error::BadCode)
    }
}

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

fn codes(b: &mut Bits, out: &mut Vec<u8>, lit: &Huffman, dist: &Huffman, cap: usize) -> Result<(), Error> {
    loop {
        let sym = lit.decode(b)? as usize;
        match sym {
            0..=255 => out.push(sym as u8),
            256 => return Ok(()),
            257..=285 => {
                let i = sym - 257;
                let len = LEN_BASE[i] as usize + b.bits(LEN_EXTRA[i] as u32)? as usize;
                let ds = dist.decode(b)? as usize;
                if ds >= 30 {
                    return Err(Error::BadCode);
                }
                let d = DIST_BASE[ds] as usize + b.bits(DIST_EXTRA[ds] as u32)? as usize;
                if d > out.len() {
                    return Err(Error::BadDistance);
                }
                for _ in 0..len {
                    out.push(out[out.len() - d]);
                }
            }
            _ => return Err(Error::BadCode),
        }
        if out.len() > cap {
            return Err(Error::TooBig);
        }
    }
}

fn fixed() -> (Huffman, Huffman) {
    let mut l = [0u8; 288];
    for (i, v) in l.iter_mut().enumerate() {
        *v = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    (Huffman::new(&l), Huffman::new(&[5u8; 30]))
}

fn dynamic(b: &mut Bits) -> Result<(Huffman, Huffman), Error> {
    let nlen = b.bits(5)? as usize + 257;
    let ndist = b.bits(5)? as usize + 1;
    let ncode = b.bits(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err(Error::BadBlock);
    }
    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let mut cl = [0u8; 19];
    for &o in ORDER.iter().take(ncode) {
        cl[o] = b.bits(3)? as u8;
    }
    let clh = Huffman::new(&cl);
    let mut lengths = vec![0u8; nlen + ndist];
    let mut i = 0;
    while i < nlen + ndist {
        let sym = clh.decode(b)?;
        if sym < 16 {
            lengths[i] = sym as u8;
            i += 1;
            continue;
        }
        let (prev, rep) = match sym {
            16 => {
                if i == 0 {
                    return Err(Error::BadBlock);
                }
                (lengths[i - 1], 3 + b.bits(2)? as usize)
            }
            17 => (0, 3 + b.bits(3)? as usize),
            _ => (0, 11 + b.bits(7)? as usize),
        };
        if i + rep > nlen + ndist {
            return Err(Error::BadBlock);
        }
        for _ in 0..rep {
            lengths[i] = prev;
            i += 1;
        }
    }
    Ok((Huffman::new(&lengths[..nlen]), Huffman::new(&lengths[nlen..])))
}

/// Inflates a raw DEFLATE stream. A stream that ends early still returns
/// what was decoded when `partial` data is acceptable to the caller, so this
/// variant reports the error and the caller decides.
pub fn inflate(data: &[u8]) -> Result<Vec<u8>, Error> {
    inflate_limited(data, MAX_OUT)
}

/// Like `inflate` but fails with `TooBig` as soon as the output would pass
/// `cap` bytes, so a declared size can bound a hostile stream.
pub fn inflate_limited(data: &[u8], cap: usize) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    inflate_into(data, &mut out, cap).map(|_| out)
}

/// Like `inflate` but keeps whatever was produced before an error, which is
/// how damaged PDF streams still yield most of their text.
pub fn inflate_lenient(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let _ = inflate_into(data, &mut out, MAX_OUT);
    out
}

fn inflate_into(data: &[u8], out: &mut Vec<u8>, cap: usize) -> Result<(), Error> {
    let mut b = Bits::new(data);
    loop {
        let last = b.bit()?;
        match b.bits(2)? {
            0 => {
                b.align();
                let p = b.pos;
                let hdr = data.get(p..p + 4).ok_or(Error::Truncated)?;
                let len = u16::from_le_bytes([hdr[0], hdr[1]]);
                let nlen = u16::from_le_bytes([hdr[2], hdr[3]]);
                if len != !nlen {
                    return Err(Error::BadBlock);
                }
                let body = data.get(p + 4..p + 4 + len as usize).ok_or(Error::Truncated)?;
                out.extend_from_slice(body);
                b.pos = p + 4 + len as usize;
            }
            1 => {
                let (l, d) = fixed();
                codes(&mut b, out, &l, &d, cap)?;
            }
            2 => {
                let (l, d) = dynamic(&mut b)?;
                codes(&mut b, out, &l, &d, cap)?;
            }
            _ => return Err(Error::BadBlock),
        }
        if out.len() > cap {
            return Err(Error::TooBig);
        }
        if last == 1 {
            return Ok(());
        }
    }
}

/// Decodes a zlib wrapper (2-byte header, Adler-32 trailer ignored).
pub fn zlib(data: &[u8]) -> Result<Vec<u8>, Error> {
    if data.len() < 2 {
        return Err(Error::Truncated);
    }
    inflate(&data[2..])
}

/// Lenient zlib: a bad header falls back to raw DEFLATE, errors keep the
/// partial output.
pub fn zlib_lenient(data: &[u8]) -> Vec<u8> {
    let header_ok = data.len() >= 2 && data[0] & 0x0f == 8 && (((data[0] as u16) << 8) | data[1] as u16).is_multiple_of(31);
    if header_ok {
        inflate_lenient(&data[2..])
    } else {
        inflate_lenient(data)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A bit writer for hand-made fixed-Huffman streams.
    pub struct W {
        pub out: Vec<u8>,
        acc: u32,
        n: u32,
    }

    impl W {
        pub fn new() -> W {
            W { out: vec![], acc: 0, n: 0 }
        }
        pub fn bits(&mut self, v: u32, count: u32) {
            for i in 0..count {
                self.acc |= ((v >> i) & 1) << self.n;
                self.n += 1;
                if self.n == 8 {
                    self.out.push(self.acc as u8);
                    self.acc = 0;
                    self.n = 0;
                }
            }
        }
        /// A Huffman code goes in most significant bit first.
        pub fn code(&mut self, v: u32, count: u32) {
            for i in (0..count).rev() {
                self.bits((v >> i) & 1, 1);
            }
        }
        pub fn lit(&mut self, b: u8) {
            if b < 144 {
                self.code(0x30 + b as u32, 8);
            } else {
                self.code(0x190 + (b as u32 - 144), 9);
            }
        }
        pub fn finish(mut self) -> Vec<u8> {
            if self.n > 0 {
                self.out.push(self.acc as u8);
            }
            self.out
        }
    }

    /// Fixed-Huffman DEFLATE of `text` followed by a length-3 match at distance 1.
    pub fn deflate_fixed_with_match(text: &[u8]) -> Vec<u8> {
        let mut w = W::new();
        w.bits(1, 1);
        w.bits(1, 2);
        for &b in text {
            w.lit(b);
        }
        w.code(1, 7); // length symbol 257 = length 3
        w.code(0, 5); // distance symbol 0 = distance 1
        w.code(0, 7); // end of block
        w.finish()
    }

    #[test]
    fn limited_inflate_stops_at_the_cap() {
        let packed = deflate_fixed_with_match(b"xa");
        assert_eq!(inflate_limited(&packed, 5).unwrap(), b"xaaaa");
        assert_eq!(inflate_limited(&packed, 4), Err(Error::TooBig));
    }

    #[test]
    fn stored_block() {
        let data = [0x01, 0x05, 0x00, 0xfa, 0xff, b'h', b'e', b'l', b'l', b'o'];
        assert_eq!(inflate(&data).unwrap(), b"hello");
    }

    #[test]
    fn fixed_block_known_vectors() {
        assert_eq!(inflate(&[0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00]).unwrap(), b"hello");
        assert_eq!(zlib(&[0x78, 0x9c, 0x4b, 0x04, 0x00, 0x00, 0x62, 0x00, 0x62]).unwrap(), b"a");
    }

    #[test]
    fn back_reference_copies_overlapping_bytes() {
        assert_eq!(inflate(&deflate_fixed_with_match(b"xa")).unwrap(), b"xaaaa");
    }

    #[test]
    fn rejects_bad_distance_and_truncation() {
        let mut w = W::new();
        w.bits(1, 1);
        w.bits(1, 2);
        w.code(1, 7);
        w.code(0, 5);
        assert_eq!(inflate(&w.finish()), Err(Error::BadDistance));
        assert_eq!(inflate(&[0xcb, 0x48]), Err(Error::Truncated));
        assert_eq!(inflate_lenient(&[0xcb, 0x48]).len() <= 5, true);
    }

    #[test]
    fn dynamic_block_from_a_reference_encoder() {
        let z: [u8; 182] = [0x65, 0x51, 0xd1, 0x0e, 0x84, 0x20, 0x0c, 0xfb, 0x15, 0x7e, 0x6d, 0x44, 0xa2, 0xe6, 0xe0, 0x24, 0x39, 0x7c, 0xe1, 0xeb, 0x6f, 0xae, 0x93, 0x0d, 0x4d, 0x14, 0xb0, 0xeb, 0xda, 0x3a, 0x56, 0x2a, 0x85, 0xc2, 0x87, 0x6a, 0xa5, 0x10, 0x53, 0xa3, 0x90, 0xea, 0x6f, 0xcf, 0xc7, 0x17, 0x1f, 0x6d, 0x7b, 0xae, 0x99, 0x4a, 0x5c, 0x98, 0xc5, 0xc7, 0x25, 0xe5, 0x46, 0x9e, 0x47, 0xb9, 0x6e, 0x28, 0x5d, 0x2f, 0x34, 0x81, 0x95, 0x53, 0x29, 0xb7, 0x3a, 0x03, 0x68, 0x77, 0xce, 0xdd, 0x34, 0xfc, 0xaa, 0x8e, 0xfb, 0x31, 0x39, 0x28, 0x0a, 0x91, 0x0b, 0x60, 0x45, 0x54, 0x85, 0x08, 0xdc, 0x07, 0x77, 0x70, 0xb7, 0xf4, 0x93, 0xcc, 0x1c, 0xd1, 0xbc, 0xa4, 0x55, 0x99, 0x92, 0x74, 0x95, 0xa1, 0x29, 0xc2, 0xc6, 0xd3, 0xcc, 0xf8, 0xbb, 0xeb, 0xce, 0x8f, 0xf4, 0x3e, 0x45, 0x34, 0xb5, 0x76, 0xdd, 0x3b, 0x46, 0xe1, 0xd2, 0x3e, 0xa7, 0x88, 0x12, 0x9a, 0x2f, 0xd7, 0x36, 0x8d, 0x02, 0xa1, 0xfa, 0xe8, 0x66, 0x86, 0x25, 0x14, 0x38, 0xbe, 0xae, 0x51, 0x88, 0xee, 0x97, 0x86, 0x6d, 0x37, 0xea, 0x18, 0xec, 0x74, 0xc9, 0x76, 0x8f, 0x48, 0xe8, 0xd7, 0x57, 0x28, 0x27, 0x8e, 0xf4, 0x10, 0xc1, 0x59, 0x60, 0xab, 0xfd, 0x01];
        let out = inflate(&z).unwrap();
        assert_eq!(out.len(), 657);
        assert!(out.starts_with(b"gamma kappa beta eps"));
        assert_eq!(String::from_utf8(out).unwrap().split(' ').count(), 120);
    }
}
