//! A minimal PNG writer for screenshots: 8-bit RGB inside a zlib stream. The
//! stream uses one fixed-Huffman deflate block with LZ77 back references
//! (`zlib_compressed`); `zlib_stored` keeps the uncompressed framing for data
//! that must be written without any coding work. Every decoder reads both.

/// zlib stored blocks carry at most this many bytes each.
const BLOCK: usize = 65_535;

/// Longest and shortest match deflate can code.
const MAX_MATCH: usize = 258;
const MIN_MATCH: usize = 3;
/// Deflate distances reach back this far.
const WINDOW: usize = 32_768;
/// Candidates examined per position by the match finder.
const MAX_CHAIN: usize = 32;
/// Size of the 3-byte hash table used to find match candidates.
const HASH_BITS: u32 = 15;
const NO_POS: usize = usize::MAX;

/// Base match length and extra bits of length symbols 257 to 285.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
/// Base distance and extra bits of distance codes 0 to 29.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// Encodes `w` x `h` premultiplied RGBA pixels (alpha ignored) as a PNG.
pub fn encode_rgb(w: usize, h: usize, px: &[u32]) -> Vec<u8> {
    assert!(px.len() >= w * h, "pixel buffer too small");
    let mut raw = Vec::with_capacity(h * (1 + 3 * w));
    for row in px[..w * h].chunks_exact(w.max(1)) {
        raw.push(0); // filter: none
        for &p in row {
            raw.extend([p as u8, (p >> 8) as u8, (p >> 16) as u8]);
        }
    }
    let mut out = Vec::with_capacity(raw.len() + raw.len() / BLOCK * 5 + 64);
    out.extend(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend((w as u32).to_be_bytes());
    ihdr.extend((h as u32).to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]); // 8 bits, RGB, deflate, no filter, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_compressed(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A zlib stream of `data`: one fixed-Huffman deflate block whose matches are found with a 3-byte hash chain.
pub fn zlib_compressed(data: &[u8]) -> Vec<u8> {
    let mut bits = BitWriter::new(data.len() / 2 + 16);
    bits.put(1, 1); // BFINAL
    bits.put(1, 2); // BTYPE 01: fixed Huffman codes
    let mut head = vec![NO_POS; 1 << HASH_BITS];
    let mut prev = vec![NO_POS; data.len()];
    let mut i = 0;
    while i < data.len() {
        let (mut best_len, mut best_dist) = (0, 0);
        if i + MIN_MATCH <= data.len() {
            let mut cand = head[hash3(data, i)];
            let mut depth = 0;
            while cand != NO_POS && depth < MAX_CHAIN && i - cand <= WINDOW {
                let len = match_len(data, cand, i);
                if len > best_len {
                    best_len = len;
                    best_dist = i - cand;
                    if len == MAX_MATCH {
                        break;
                    }
                }
                cand = prev[cand];
                depth += 1;
            }
        }
        let advance = if best_len >= MIN_MATCH {
            put_match(&mut bits, best_len, best_dist);
            best_len
        } else {
            put_literal(&mut bits, data[i]);
            1
        };
        for k in i..i + advance {
            insert_position(&mut head, &mut prev, data, k);
        }
        i += advance;
    }
    put_length_symbol(&mut bits, 256); // end of block
    let mut out = Vec::with_capacity(bits.out.len() + 6);
    out.extend([0x78, 0x01]); // deflate, 32K window, no preset dictionary
    out.extend(bits.finish());
    out.extend(adler32(data).to_be_bytes());
    out
}

fn hash3(data: &[u8], at: usize) -> usize {
    let h = ((data[at] as usize) << 10) ^ ((data[at + 1] as usize) << 5) ^ (data[at + 2] as usize);
    h & ((1 << HASH_BITS) - 1)
}

fn insert_position(head: &mut [usize], prev: &mut [usize], data: &[u8], at: usize) {
    if at + MIN_MATCH <= data.len() {
        let h = hash3(data, at);
        prev[at] = head[h];
        head[h] = at;
    }
}

/// Length of the common run starting at `earlier` and `at`, capped at the deflate maximum.
fn match_len(data: &[u8], earlier: usize, at: usize) -> usize {
    let max = (data.len() - at).min(MAX_MATCH);
    (0..max).take_while(|&l| data[earlier + l] == data[at + l]).count()
}

/// Writes a deflate length/distance pair with the fixed codes of RFC 1951, section 3.2.6.
fn put_match(bits: &mut BitWriter, len: usize, dist: usize) {
    let li = LENGTH_BASE.partition_point(|&b| b as usize <= len) - 1;
    put_length_symbol(bits, 257 + li);
    bits.put((len - LENGTH_BASE[li] as usize) as u32, LENGTH_EXTRA[li] as u32);
    let di = DIST_BASE.partition_point(|&b| b as usize <= dist) - 1;
    bits.put_code(di as u32, 5);
    bits.put((dist - DIST_BASE[di] as usize) as u32, DIST_EXTRA[di] as u32);
}

fn put_literal(bits: &mut BitWriter, lit: u8) {
    if lit < 144 {
        bits.put_code(0x30 + lit as u32, 8)
    } else {
        bits.put_code(0x190 + (lit as u32 - 144), 9)
    }
}

fn put_length_symbol(bits: &mut BitWriter, sym: usize) {
    if sym <= 279 {
        bits.put_code((sym - 256) as u32, 7)
    } else {
        bits.put_code(0xC0 + (sym - 280) as u32, 8)
    }
}

/// Packs bits least significant first, as deflate does.
struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    used: u32,
}

impl BitWriter {
    fn new(capacity: usize) -> Self {
        BitWriter { out: Vec::with_capacity(capacity), acc: 0, used: 0 }
    }

    /// Appends the low `count` bits of `value`, the lowest bit first.
    fn put(&mut self, value: u32, count: u32) {
        self.acc |= (value as u64) << self.used;
        self.used += count;
        while self.used >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.used -= 8;
        }
    }

    /// Appends a Huffman code of `len` bits, whose first bit is the most significant.
    fn put_code(&mut self, code: u32, len: u32) {
        self.put(code.reverse_bits() >> (32 - len), len);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.used > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// A zlib stream whose deflate payload is stored blocks of `data`.
pub fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / BLOCK * 5 + 11);
    out.extend([0x78, 0x01]); // deflate, 32K window, no preset dictionary
    let blocks = data.chunks(BLOCK).count().max(1);
    let mut chunks = data.chunks(BLOCK);
    for i in 0..blocks {
        let block = chunks.next().unwrap_or(&[]);
        out.push((i + 1 == blocks) as u8); // BFINAL, BTYPE = 00
        let len = block.len() as u16;
        out.extend(len.to_le_bytes());
        out.extend((!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend(adler32(data).to_be_bytes());
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend_from_slice(data);
    out.extend(crc32(&out[start..]).to_be_bytes());
}

/// The CRC-32 (IEEE 802.3, as PNG uses it) of `data`.
pub use crate::files::zip::crc32;

/// The Adler-32 checksum of `data`, as zlib trailers carry it.
pub fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let (mut a, mut b) = (1u32, 0u32);
    // 5552 bytes fit in u32 before a reduction is needed
    for block in data.chunks(5552) {
        for &x in block {
            a += x as u32;
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    b << 16 | a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;

    #[test]
    fn checksums_match_the_known_vectors() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(adler32(b""), 1);
        let long = vec![0xABu8; 70_000];
        assert_eq!(adler32(&long), adler32_reference(&long));
    }

    fn adler32_reference(data: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for &x in data {
            a = (a + x as u32) % 65_521;
            b = (b + a) % 65_521;
        }
        b << 16 | a
    }

    #[test]
    fn stored_zlib_splits_into_final_and_non_final_blocks() {
        let data = vec![7u8; BLOCK + 10];
        let z = zlib_stored(&data);
        assert_eq!(&z[..2], &[0x78, 0x01]);
        assert_eq!(z[2], 0, "first block is not final");
        assert_eq!(u16::from_le_bytes([z[3], z[4]]), BLOCK as u16);
        assert_eq!(u16::from_le_bytes([z[5], z[6]]), !(BLOCK as u16));
        let second = 7 + BLOCK;
        assert_eq!(z[second], 1, "last block is final");
        assert_eq!(u16::from_le_bytes([z[second + 1], z[second + 2]]), 10);
        assert_eq!(z.len(), 2 + 5 + BLOCK + 5 + 10 + 4);
        assert_eq!(zlib_stored(&[]).len(), 2 + 5 + 4, "empty input still has one final block");
    }

    fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let d = png::Decoder::new(std::io::Cursor::new(png_bytes));
        let mut r = d.read_info().expect("valid png");
        let mut buf = vec![0u8; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgb);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        buf.truncate(info.buffer_size());
        (info.width, info.height, buf)
    }

    #[test]
    fn round_trips_through_the_png_crate() {
        let (w, h) = (37usize, 11usize);
        let px: Vec<u32> = (0..w * h).map(|i| pack((i * 7 % 256) as u8, (i * 3 % 256) as u8, (i * 11 % 256) as u8, 255)).collect();
        let bytes = encode_rgb(w, h, &px);
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        let (dw, dh, data) = decode(&bytes);
        assert_eq!((dw, dh), (w as u32, h as u32));
        for (i, &p) in px.iter().enumerate() {
            assert_eq!(&data[i * 3..i * 3 + 3], &[p as u8, (p >> 8) as u8, (p >> 16) as u8], "pixel {i}");
        }
    }

    #[test]
    fn a_screen_sized_frame_spans_many_blocks_and_still_decodes() {
        let (w, h) = (1200usize, 1920usize);
        let px: Vec<u32> = (0..w * h).map(|i| pack((i % 251) as u8, (i / w % 256) as u8, 9, 255)).collect();
        let bytes = encode_rgb(w, h, &px);
        let (dw, dh, data) = decode(&bytes);
        assert_eq!((dw, dh), (1200, 1920));
        let i = 1000 * w + 777;
        assert_eq!(data[i * 3], (i % 251) as u8);
        assert_eq!(data[i * 3 + 1], (1000 % 256) as u8);
    }
}
