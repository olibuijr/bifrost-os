//! Parity sdk-tests-16: headless tests for the preview helpers of the Files app: which preview a file gets, text
//! wrapping for the text preview, zoom and pan limits, the decode bounds, and picture sizes read from file headers.
//! Pure functions only: no file is opened, no window is drawn and no clock is read.

use springboard::files::fs::Kind;
use springboard::files::preview::{
    decode_allowed, image_size, is_pdf, preview_for, wrap_lines, Preview, Zoom, MAX_IMAGE_BYTES, MAX_PREVIEW_PIXELS, MAX_ZOOM,
    MIN_ZOOM,
};

#[test]
fn a_pdf_file_gets_the_pdf_preview_whatever_its_kind() {
    assert_eq!(preview_for("report.pdf", Kind::Text), Preview::Pdf);
    assert_eq!(preview_for("REPORT.Pdf", Kind::Other), Preview::Pdf);
}

#[test]
fn each_kind_of_file_gets_its_own_preview() {
    assert_eq!(preview_for("notes.txt", Kind::Text), Preview::Text);
    assert_eq!(preview_for("photo.jpg", Kind::Image), Preview::Image);
    assert_eq!(preview_for("song.mp3", Kind::Audio), Preview::Audio);
    assert_eq!(preview_for("clip.mp4", Kind::Video), Preview::Video);
    assert_eq!(preview_for("docs", Kind::Folder), Preview::FolderSummary);
}

#[test]
fn an_archive_or_an_unknown_file_gets_only_the_info_card() {
    assert_eq!(preview_for("bundle.zip", Kind::Archive), Preview::Info);
    assert_eq!(preview_for("data.bin", Kind::Other), Preview::Info);
}

#[test]
fn a_file_is_a_pdf_only_when_its_extension_is_pdf() {
    assert!(is_pdf("report.pdf"));
    assert!(is_pdf("REPORT.Pdf"));
    assert!(!is_pdf("report.pdf.txt"));
    assert!(!is_pdf("pdf"), "a bare word is not an extension");
    assert!(!is_pdf(".pdf"), "a dot-file has no stem");
}

#[test]
fn text_wraps_at_spaces_and_hard_breaks_long_words() {
    let wrapped = |s: &str, cols: usize| wrap_lines(s, cols).into_iter().map(|(line, _)| line).collect::<Vec<_>>();
    assert_eq!(wrapped("the quick brown fox", 10), ["the quick", "brown fox"]);
    assert_eq!(wrapped("one two three", 7), ["one two", "three"]);
    assert_eq!(wrapped("abcdefghijkl", 5), ["abcde", "fghij", "kl"]);
    assert_eq!(wrapped("", 5), [""]);
}

#[test]
fn text_wrapping_counts_characters_and_treats_zero_columns_as_one() {
    let wrapped = |s: &str, cols: usize| wrap_lines(s, cols).into_iter().map(|(line, _)| line).collect::<Vec<_>>();
    assert_eq!(wrapped("åäö", 2), ["åä", "ö"]);
    assert_eq!(wrapped("ab", 0), ["a", "b"]);
}

#[test]
fn wrapped_lines_remember_the_source_line_they_came_from() {
    let lines = wrap_lines("x\ny", 10);
    assert_eq!(lines, vec![("x".to_string(), 1), ("y".to_string(), 2)]);
    let long = wrap_lines("one two three four\nfive", 8);
    let numbers: Vec<usize> = long.iter().map(|(_, n)| *n).collect();
    assert_eq!(numbers.last(), Some(&2));
    assert!(numbers[..numbers.len() - 1].iter().all(|&n| n == 1));
}

#[test]
fn a_pinch_keeps_the_point_under_the_fingers_still_and_bounds_the_pan() {
    let (fit, view) = ((400.0, 300.0), (500.0, 500.0));
    let mut z = Zoom::default();
    z.pinch(2.0, (100.0, 0.0), fit, view);
    assert_eq!(z.scale, 2.0);
    assert_eq!(z.pan, (-100.0, 0.0), "the point at x = 100 stays under the focus");
    z.drag(-1000.0, 0.0, fit, view);
    assert_eq!(z.pan.0, -150.0, "the overhang on the x axis is (400 * 2 - 500) / 2");
}

#[test]
fn the_scale_stays_between_the_minimum_and_the_maximum() {
    let (fit, view) = ((400.0, 300.0), (500.0, 500.0));
    let mut z = Zoom::default();
    z.pinch(100.0, (0.0, 0.0), fit, view);
    assert_eq!(z.scale, MAX_ZOOM);
    z.pinch(0.01, (0.0, 0.0), fit, view);
    assert_eq!(z.scale, MIN_ZOOM);
    assert_eq!(z, Zoom::default(), "back at fit, centred");
}

#[test]
fn a_picture_larger_than_the_view_pans_only_as_far_as_its_overhang() {
    let (fit, view) = ((800.0, 600.0), (500.0, 500.0));
    let mut z = Zoom::default();
    z.drag(0.0, 999.0, fit, view);
    assert_eq!(z.pan, (0.0, 50.0), "the vertical overhang is (600 - 500) / 2");
    z.drag(999.0, 0.0, fit, view);
    assert_eq!(z.pan.0, 150.0, "the horizontal overhang is (800 - 500) / 2");
}

#[test]
fn a_double_tap_zooms_in_to_two_and_a_half_times_and_back_to_fit() {
    let (fit, view) = ((400.0, 400.0), (400.0, 400.0));
    let mut z = Zoom::default();
    z.toggle((0.0, 0.0), fit, view);
    assert_eq!(z.scale, 2.5);
    z.toggle((0.0, 0.0), fit, view);
    assert_eq!(z, Zoom::default());
}

#[test]
fn a_picture_decodes_only_within_the_pixel_and_byte_limits() {
    assert!(decode_allowed(4000, 4000, MAX_IMAGE_BYTES), "exactly at both limits is allowed");
    assert_eq!(MAX_PREVIEW_PIXELS, 16_000_000);
    assert!(!decode_allowed(4001, 4000, 1), "one pixel row over the pixel limit");
    assert!(!decode_allowed(100, 100, MAX_IMAGE_BYTES + 1), "one byte over the file limit");
    assert!(!decode_allowed(0, 10, 1), "an empty picture is not decoded");
}

#[test]
fn a_png_size_is_read_from_its_header() {
    let mut png = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
    png.extend_from_slice(&640u32.to_be_bytes());
    png.extend_from_slice(&480u32.to_be_bytes());
    assert_eq!(image_size(&png), Some((640, 480)));
}

#[test]
fn a_png_header_cut_short_has_no_size() {
    let short = [0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0, 0, 1, 0, 0];
    assert_eq!(image_size(&short), None);
}

#[test]
fn a_jpeg_size_skips_a_huffman_table_segment_and_reads_the_frame_header() {
    // SOI, DHT (length 4, skipped), SOF0 with 300 rows and 200 columns
    let jpg = [0xFF, 0xD8, 0xFF, 0xC4, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8, 1, 44, 0, 200, 3, 0, 0, 0];
    assert_eq!(image_size(&jpg), Some((200, 300)));
}

#[test]
fn an_unknown_format_has_no_size() {
    assert_eq!(image_size(b"nope"), None);
}
