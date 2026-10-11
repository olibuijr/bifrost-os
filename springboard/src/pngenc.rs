//! A minimal PNG writer for screenshots: 8-bit RGB, zlib framing around
//! stored (uncompressed) deflate blocks, so a frame is written in one pass
//! without a compressor. Every decoder reads it like any other PNG.

/// zlib stored blocks carry at most this many bytes each.
const BLOCK: usize = 65_535;

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
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
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
        assert!(bytes.len() > w * h * 3, "stored blocks never shrink the data");
        let (dw, dh, data) = decode(&bytes);
        assert_eq!((dw, dh), (1200, 1920));
        let i = 1000 * w + 777;
        assert_eq!(data[i * 3], (i % 251) as u8);
        assert_eq!(data[i * 3 + 1], (1000 % 256) as u8);
    }
}
