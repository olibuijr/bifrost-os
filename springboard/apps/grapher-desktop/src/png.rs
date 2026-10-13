//! A small PNG encoder for the export: 8-bit RGBA, one filter type (none) for every row, and a zlib stream made of
//! stored (uncompressed) deflate blocks. Needs no dependency; the files are larger than a compressed PNG, which is the
//! price of a short encoder.

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
/// The longest payload of one stored deflate block.
const MAX_STORED: usize = 65_535;
/// The modulus of the Adler-32 checksum.
const ADLER_MOD: u64 = 65_521;

/// Encodes `rgba` (`width * height * 4` bytes, rows top to bottom) as a PNG file. Width and height must be at least 1.
pub fn encode_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    assert!(width > 0 && height > 0, "a PNG has at least one pixel");
    let row_bytes = width as usize * 4;
    assert_eq!(rgba.len(), row_bytes * height as usize, "the pixel data does not match the size");

    let mut raw = Vec::with_capacity((row_bytes + 1) * height as usize);
    for row in rgba.chunks(row_bytes) {
        raw.push(0); // filter type: none
        raw.extend_from_slice(row);
    }

    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    // bit depth 8, colour type 6 (RGBA), compression 0 (deflate), filter method 0, no interlace
    header.extend_from_slice(&[8, 6, 0, 0, 0]);

    let mut out = Vec::with_capacity(raw.len() + 64);
    out.extend_from_slice(&SIGNATURE);
    write_chunk(&mut out, b"IHDR", &header);
    write_chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    write_chunk(&mut out, b"IEND", &[]);
    out
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[&kind[..], data]).to_be_bytes());
}

/// A zlib stream (RFC 1950) of `data` in stored deflate blocks (RFC 1951, block type 0).
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    // CMF 0x78 (deflate, 32 KiB window) and FLG 0x01 (no dictionary, check bits make the pair a multiple of 31)
    let mut out = vec![0x78, 0x01];
    if data.is_empty() {
        // one final stored block with no payload
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xff, 0xff]);
    }
    let mut blocks = data.chunks(MAX_STORED).peekable();
    while let Some(block) = blocks.next() {
        let last = blocks.peek().is_none();
        out.push(if last { 0x01 } else { 0x00 });
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// CRC-32 (the PNG and zip polynomial) of the concatenation of `parts`.
fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for part in parts {
        for &byte in *part {
            crc ^= byte as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u64, 0u64);
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += byte as u64;
            b += a;
        }
        a %= ADLER_MOD;
        b %= ADLER_MOD;
    }
    ((b << 16) | a) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_adler_known_values() {
        assert_eq!(crc32(&[&b"IEND"[..]]), 0xAE42_6082);
        assert_eq!(crc32(&[&b""[..], &b"IEND"[..]]), 0xAE42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(adler32(b""), 1);
    }

    #[test]
    fn the_stored_stream_holds_the_data() {
        let data: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
        let stream = zlib_stored(&data);
        assert_eq!(&stream[..2], &[0x78, 0x01]);
        // walk the stored blocks and collect the payload
        let mut pos = 2;
        let mut payload = Vec::new();
        loop {
            let header = stream[pos];
            let len = u16::from_le_bytes([stream[pos + 1], stream[pos + 2]]);
            let nlen = u16::from_le_bytes([stream[pos + 3], stream[pos + 4]]);
            assert_eq!(nlen, !len);
            payload.extend_from_slice(&stream[pos + 5..pos + 5 + len as usize]);
            pos += 5 + len as usize;
            if header & 1 == 1 {
                break;
            }
        }
        assert_eq!(payload, data);
        assert_eq!(&stream[pos..], &adler32(&data).to_be_bytes());
    }

    #[test]
    fn an_empty_payload_is_a_valid_stream() {
        let stream = zlib_stored(&[]);
        assert_eq!(&stream[..7], &[0x78, 0x01, 0x01, 0x00, 0x00, 0xff, 0xff]);
    }

    #[test]
    fn the_file_has_signature_header_and_end_chunk() {
        let pixels = [255u8, 0, 0, 255, 0, 255, 0, 255];
        let file = encode_rgba(2, 1, &pixels);
        assert_eq!(&file[..8], &SIGNATURE);
        // IHDR: length 13, then the type, then the width and height
        assert_eq!(&file[8..12], &13u32.to_be_bytes());
        assert_eq!(&file[12..16], b"IHDR");
        assert_eq!(&file[16..20], &2u32.to_be_bytes());
        assert_eq!(&file[20..24], &1u32.to_be_bytes());
        assert_eq!(&file[24..29], &[8, 6, 0, 0, 0]);
        assert!(file.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]));
    }

    #[test]
    fn rows_are_prefixed_with_a_filter_byte() {
        let pixels: Vec<u8> = (0..16u8).collect();
        let file = encode_rgba(1, 4, &pixels);
        // the IDAT payload starts after the two zlib bytes and the 5-byte block header
        let idat = file.windows(4).position(|w| w == b"IDAT").unwrap() + 4;
        let payload = &file[idat + 2 + 5..];
        assert_eq!(payload[0], 0);
        assert_eq!(&payload[1..5], &[0, 1, 2, 3]);
        assert_eq!(payload[5], 0);
    }
}
