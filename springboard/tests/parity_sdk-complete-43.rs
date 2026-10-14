//! Parity sdk-complete-43: headless tests of the PNG writer's zlib stream. Deflate output is checked by decoding the
//! PNG with the png crate, and by the framing bytes of the zlib and deflate headers.

use springboard::gfx::pack;
use springboard::pngenc::{encode_rgb, zlib_compressed};

fn decode_rgb(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes)).read_info().expect("valid png");
    let mut buf = vec![0u8; reader.output_buffer_size().expect("buffer size")];
    let info = reader.next_frame(&mut buf).expect("decodes");
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

#[test]
fn a_compressed_zlib_stream_starts_with_a_default_header_and_a_fixed_huffman_block() {
    let z = zlib_compressed(b"abcabcabc");
    assert_eq!(&z[..2], &[0x78, 0x01], "deflate, 32K window, no dictionary");
    assert_eq!(z[2] & 0b111, 0b011, "BFINAL set, BTYPE 01 (fixed Huffman codes)");
}

#[test]
fn a_compressed_zlib_stream_ends_with_the_adler32_of_its_input() {
    let data = b"Wikipedia";
    let z = zlib_compressed(data);
    let tail = &z[z.len() - 4..];
    assert_eq!(tail, &[0x11, 0xE6, 0x03, 0x98]);
}

#[test]
fn an_empty_input_compresses_to_a_stream_with_the_empty_checksum() {
    let z = zlib_compressed(&[]);
    assert_eq!(&z[z.len() - 4..], &[0, 0, 0, 1], "adler32 of nothing is one");
    assert!(z.len() <= 2 + 1 + 1 + 4, "header, end-of-block code and checksum only");
}

#[test]
fn a_long_run_of_one_byte_is_coded_as_repeated_matches() {
    let data = vec![0u8; 10_000];
    let z = zlib_compressed(&data);
    assert!(z.len() < 200, "10000 equal bytes need about 40 matches, got {} bytes", z.len());
}

#[test]
fn incompressible_bytes_still_produce_a_stream_no_larger_than_nine_bits_per_byte() {
    let data: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    let z = zlib_compressed(&data);
    assert!(z.len() <= 2 + data.len() * 9 / 8 + 16, "literal codes are at most nine bits, got {}", z.len());
}

#[test]
fn a_solid_screen_frame_is_written_far_smaller_than_its_raw_pixels() {
    let (w, h) = (1200usize, 1920usize);
    let px = vec![pack(20, 40, 60, 255); w * h];
    let bytes = encode_rgb(w, h, &px);
    assert!(bytes.len() * 50 < w * h * 3, "a flat frame compresses, got {} bytes", bytes.len());
    let (dw, dh, data) = decode_rgb(&bytes);
    assert_eq!((dw, dh), (1200, 1920));
    assert_eq!(&data[data.len() - 3..], &[60, 40, 20]);
}

#[test]
fn a_compressed_frame_decodes_to_the_same_pixels() {
    let (w, h) = (37usize, 11usize);
    let px: Vec<u32> = (0..w * h).map(|i| pack((i * 7 % 256) as u8, (i * 3 % 256) as u8, (i * 11 % 256) as u8, 255)).collect();
    let bytes = encode_rgb(w, h, &px);
    let (dw, dh, data) = decode_rgb(&bytes);
    assert_eq!((dw, dh), (w as u32, h as u32));
    for (i, &p) in px.iter().enumerate() {
        assert_eq!(&data[i * 3..i * 3 + 3], &[p as u8, (p >> 8) as u8, (p >> 16) as u8], "pixel {i}");
    }
}

#[test]
fn a_frame_with_repeated_rows_decodes_through_back_references() {
    let (w, h) = (64usize, 40usize);
    let px: Vec<u32> = (0..w * h).map(|i| pack((i % w) as u8, 128, 3, 255)).collect();
    let bytes = encode_rgb(w, h, &px);
    let (_, _, data) = decode_rgb(&bytes);
    let row = 3 * w;
    assert_eq!(&data[row * 5..row * 6], &data[row * 17..row * 18], "row five equals row seventeen");
}
