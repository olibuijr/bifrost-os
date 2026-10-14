//! Parity sdk-complete-23: headless tests for the metadata reader of PNG files. A PNG carries its EXIF data in an
//! `eXIf` chunk and its size in the `IHDR` chunk. The tests build the bytes in memory and read them back.

use springboard::shell::photos::exif::{orientation, parse, parse_image, parse_png};

/// Little-endian TIFF data with a camera name (ASCII, tag 0x010F) and an orientation (SHORT, tag 0x0112).
fn tiff_le(make: &str, orientation: u16) -> Vec<u8> {
    let mut ascii = make.as_bytes().to_vec();
    ascii.push(0);
    let ifd_at: u32 = 8;
    let entries: u16 = 2;
    let strings_at = ifd_at + 2 + 12 * entries as u32 + 4;
    let mut out = b"II".to_vec();
    out.extend(42u16.to_le_bytes());
    out.extend(ifd_at.to_le_bytes());
    out.extend(entries.to_le_bytes());
    out.extend(0x010Fu16.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend((ascii.len() as u32).to_le_bytes());
    out.extend(strings_at.to_le_bytes());
    out.extend(0x0112u16.to_le_bytes());
    out.extend(3u16.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(orientation.to_le_bytes());
    out.extend([0u8, 0]);
    out.extend(0u32.to_le_bytes());
    out.extend(ascii);
    out
}

/// One PNG chunk: a big-endian length, the four-byte kind, the data and a CRC. The reader does not check the CRC.
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(kind);
    out.extend(data);
    out.extend([0u8; 4]);
}

/// A PNG with a header of the given size, an optional `eXIf` chunk and an end chunk.
fn png(width: u32, height: u32, exif: Option<&[u8]>) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8u8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    if let Some(tiff) = exif {
        chunk(&mut out, b"eXIf", tiff);
    }
    chunk(&mut out, b"IEND", &[]);
    out
}

#[test]
fn headless_a_png_exif_chunk_gives_the_camera_name_and_the_orientation() {
    let file = png(640, 480, Some(&tiff_le("Acme Lab", 6)));
    let exif = parse_png(&file).expect("a PNG signature is read");
    assert_eq!(exif.make.as_deref(), Some("Acme Lab"));
    assert_eq!(exif.orientation, Some(6));
}

#[test]
fn headless_the_size_comes_from_the_header_when_the_exif_has_no_dimensions() {
    let file = png(320, 200, None);
    let exif = parse_image(&file).expect("the signature selects the PNG reader");
    assert_eq!((exif.width, exif.height), (Some(320), Some(200)));
    assert_eq!(exif.make, None);
    assert_eq!(exif.orientation, None);
}

#[test]
fn headless_the_orientation_of_a_png_is_read_from_its_exif_and_defaults_to_one() {
    assert_eq!(orientation(&png(4, 4, Some(&tiff_le("X", 6)))), 6);
    assert_eq!(orientation(&png(4, 4, None)), 1);
}

#[test]
fn headless_a_png_cut_short_keeps_the_fields_read_before_the_cut() {
    let mut file = png(640, 480, Some(&tiff_le("Acme Lab", 3)));
    file.truncate(file.len() - 3);
    let exif = parse_png(&file).expect("the signature is intact");
    assert_eq!(exif.make.as_deref(), Some("Acme Lab"));
    assert_eq!((exif.width, exif.height), (Some(640), Some(480)));
}

#[test]
fn headless_bytes_without_a_known_signature_are_not_read_as_an_image() {
    assert!(parse_image(b"GIF89a------").is_none());
    assert!(parse_png(&[0x89, b'P']).is_none());
    assert_eq!(orientation(b"GIF89a------"), 1);
}

#[test]
fn headless_the_jpeg_reader_refuses_a_png_file() {
    assert!(parse(&png(4, 4, Some(&tiff_le("X", 1)))).is_none());
}
