//! Headless tests for the photo metadata and the photo library scan: GPS and EXIF reading from JPEG bytes, the info
//! sheet rows, and the picture names the library accepts. The JPEG bytes are built in the test, so the tests read no
//! camera file, draw nothing and need no display. Files for the scan are written under the cargo target directory.

use springboard::shell::photos::exif::{describe, format_date, format_exposure, format_gps, orientation, parse};
use springboard::shell::photos::library::scan_at;
use std::path::PathBuf;

/// One 12-byte IFD entry with its four value bytes given directly.
fn entry(out: &mut Vec<u8>, tag: u16, typ: u16, count: u32, value: [u8; 4]) {
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&typ.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&value);
}

fn offset_bytes(at: u32) -> [u8; 4] {
    at.to_le_bytes()
}

/// A little-endian TIFF block with a GPS IFD. Latitude and longitude are whole degrees, with minutes 30 and
/// seconds 0, so the stored values are `lat_deg + 0.5` and `lon_deg + 0.5`, with the references N and W.
fn gps_tiff(lat_deg: u32, lon_deg: u32, lat_ref: u8, lon_ref: u8) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(b"II");
    t.extend_from_slice(&42u16.to_le_bytes());
    t.extend_from_slice(&8u32.to_le_bytes());
    // IFD0 at 8: one entry, the GPS pointer, then no next IFD.
    t.extend_from_slice(&1u16.to_le_bytes());
    entry(&mut t, 0x8825, 4, 1, offset_bytes(26));
    t.extend_from_slice(&0u32.to_le_bytes());
    // GPS IFD at 26: four entries, rationals at 80 (latitude) and 104 (longitude).
    t.extend_from_slice(&4u16.to_le_bytes());
    entry(&mut t, 1, 2, 2, [lat_ref, 0, 0, 0]);
    entry(&mut t, 2, 5, 3, offset_bytes(80));
    entry(&mut t, 3, 2, 2, [lon_ref, 0, 0, 0]);
    entry(&mut t, 4, 5, 3, offset_bytes(104));
    t.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(t.len(), 80);
    for (n, d) in [(lat_deg, 1), (30, 1), (0, 1), (lon_deg, 1), (30, 1), (0, 1)] {
        t.extend_from_slice(&n.to_le_bytes());
        t.extend_from_slice(&d.to_le_bytes());
    }
    assert_eq!(t.len(), 128);
    t
}

/// A JPEG whose APP1 segment holds the TIFF block. The image data is not needed to read the EXIF data.
fn jpeg_with(tiff: &[u8]) -> Vec<u8> {
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend_from_slice(tiff);
    let len = (app1.len() + 2) as u16;
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
    jpeg.extend_from_slice(&len.to_be_bytes());
    jpeg.extend_from_slice(&app1);
    jpeg.extend_from_slice(&[0xFF, 0xD9]);
    jpeg
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("parity-sdk-tests-07-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn headless_gps_is_formatted_as_unsigned_degrees_with_hemisphere_letters() {
    assert_eq!(format_gps(64.1466, -21.9426), "64.1466 N, 21.9426 W");
    assert_eq!(format_gps(-33.5, 151.25), "33.5000 S, 151.2500 E");
    assert_eq!(format_gps(0.0, 0.0), "0.0000 N, 0.0000 E");
}

#[test]
fn headless_gps_tags_are_read_from_the_jpeg_and_signed_by_their_references() {
    let exif = parse(&jpeg_with(&gps_tiff(64, 21, b'N', b'W'))).expect("a JPEG with EXIF");
    assert_eq!(exif.gps, Some((64.5, -21.5)));
    assert!(exif.rows().contains(&("Location", "64.5000 N, 21.5000 W".to_string())));
    let south_east = parse(&jpeg_with(&gps_tiff(33, 151, b'S', b'E'))).expect("a JPEG with EXIF");
    assert_eq!(south_east.gps, Some((-33.5, 151.5)));
}

#[test]
fn headless_out_of_range_gps_is_dropped() {
    // 91 degrees of latitude is not a place on Earth.
    let exif = parse(&jpeg_with(&gps_tiff(91, 21, b'N', b'W'))).expect("a JPEG with EXIF");
    assert_eq!(exif.gps, None);
    assert!(exif.rows().iter().all(|(label, _)| *label != "Location"));
}

#[test]
fn headless_reader_rejects_non_jpeg_bytes_and_keeps_the_file_size_fallback() {
    assert_eq!(parse(b"\x89PNG\r\n\x1a\n"), None);
    assert_eq!(parse(&[]), None);
    assert_eq!(orientation(b"not a picture"), 1);
    let rows = describe("IMG_1.jpg", 2048, 0, 0, &jpeg_with(&gps_tiff(10, 20, b'N', b'E')));
    assert_eq!(rows[0], ("Name".to_string(), "IMG_1.jpg".to_string()));
    assert!(rows.iter().any(|(label, value)| label == "Location" && value == "10.5000 N, 20.5000 E"));
    assert!(rows.iter().any(|(label, _)| label == "Date"), "a file time stands in for a missing EXIF date");
    assert!(rows.iter().any(|(label, _)| label == "File Size"));
}

#[test]
fn headless_capture_date_and_exposure_are_formatted_for_the_info_sheet() {
    assert_eq!(format_date("2026:10:04 12:30:00"), "4 October 2026, 12:30");
    assert_eq!(format_date("not a date"), "not a date");
    assert_eq!(format_exposure(1, 250), "1/250 s");
    assert_eq!(format_exposure(2, 1), "2 s");
    assert_eq!(format_exposure(0, 0), "0 s");
}

#[test]
fn headless_library_accepts_picture_names_in_any_case_and_skips_hidden_entries() {
    let dir = scratch("media-types");
    for name in ["a.png", "b.jpg", "c.JPEG", "notes.txt", ".hidden.png"] {
        std::fs::write(dir.join(name), b"x").unwrap();
    }
    std::fs::create_dir_all(dir.join(".trash")).unwrap();
    std::fs::write(dir.join(".trash").join("gone.png"), b"x").unwrap();
    let mut names: Vec<String> = scan_at(std::slice::from_ref(&dir), 0).iter().map(|p| p.name()).collect();
    names.sort();
    assert_eq!(names, ["a.png", "b.jpg", "c.JPEG"]);
}
