//! Headless host tests for the PDF engine (`bifrost_render::cg::pdf`): reading documents, the page tree, text
//! extraction, the outline, cross-reference repair, and writing documents. Every test works on bytes in memory: no
//! display, no device, no network, and no clock. Each assertion checks a concrete value.
use bifrost_render::cg::pdf::doc::Doc;
use bifrost_render::cg::pdf::extract::{Extractor, Piece};
use bifrost_render::cg::pdf::obj::Obj;
use bifrost_render::cg::pdf::page::{self, pages, PageBox};
use bifrost_render::cg::pdf::writer::{self, text_literal, winansi, Page};

/// A PDF file with objects 1 to n (`objects[n - 1]`), a classic cross-reference table and a trailer. `extra` is added
/// to the trailer dictionary, and `startxref` is written as given, so a test can break it on purpose.
fn build(objects: &[&str], extra: &str, startxref: Option<usize>) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, body).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    let at = startxref.unwrap_or(xref);
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R{extra} >>\nstartxref\n{at}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

fn file(objects: &[&str]) -> Vec<u8> {
    build(objects, "", None)
}

/// Two pages that share a 200 by 100 media box from their parent. Page 3 sets a crop box; page 4 is rotated a quarter
/// turn and inherits the media box.
fn two_page_tree() -> Vec<&'static str> {
    vec![
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 100] >>",
        "<< /Type /Page /Parent 2 0 R /CropBox [10 10 110 60] >>",
        "<< /Type /Page /Parent 2 0 R /Rotate 90 >>",
    ]
}

// --- document opening and cross-reference repair -----------------------------------------------------------------------

#[test]
fn bytes_without_a_pdf_header_are_refused() {
    let err = Doc::open(b"plain text, not a document".to_vec()).err();
    assert_eq!(err.as_deref(), Some("not a PDF file"));
}

#[test]
fn a_document_with_a_correct_cross_reference_lists_its_objects() {
    let doc = Doc::open(file(&two_page_tree())).unwrap();
    assert_eq!(doc.object_numbers(), vec![1, 2, 3, 4]);
    assert!(matches!(doc.load(1), Obj::Dict(_)));
    assert!(!doc.is_encrypted());
    assert_eq!(pages(&doc).len(), 2);
}

#[test]
fn a_broken_startxref_is_repaired_by_scanning_the_objects() {
    let bad = build(&two_page_tree(), "", Some(3));
    let doc = Doc::open(bad).unwrap();
    assert_eq!(doc.object_numbers(), vec![1, 2, 3, 4]);
    let media: Vec<[f64; 4]> = pages(&doc).iter().map(|p| p.media_box()).collect();
    assert_eq!(media, vec![[0.0, 0.0, 200.0, 100.0], [0.0, 0.0, 200.0, 100.0]]);
}

// --- page tree, boxes and rotation ------------------------------------------------------------------------------------

#[test]
fn the_page_tree_walk_inherits_the_media_box_and_reads_the_rotation() {
    let doc = Doc::open(file(&two_page_tree())).unwrap();
    let all = pages(&doc);
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].index, 0);
    assert_eq!(all[1].index, 1);
    assert_eq!(all[0].size(PageBox::Media), (200.0, 100.0));
    assert_eq!(all[0].rotation(), 0);
    assert_eq!(all[1].rotation(), 90);
    assert_eq!(all[1].size(PageBox::Media), (100.0, 200.0));
    assert_eq!(page::page_count(&doc), 2);
}

#[test]
fn a_crop_box_is_reported_for_its_page_and_falls_back_to_the_media_box_elsewhere() {
    let doc = Doc::open(file(&two_page_tree())).unwrap();
    let all = pages(&doc);
    assert_eq!(all[0].rect(PageBox::Crop), [10.0, 10.0, 110.0, 60.0]);
    assert_eq!(all[0].size(PageBox::Crop), (100.0, 50.0));
    assert_eq!(all[1].rect(PageBox::Crop), [0.0, 0.0, 200.0, 100.0]);
    assert_eq!(all[1].rect(PageBox::Trim), [0.0, 0.0, 200.0, 100.0]);
}

// --- writing documents and reading them back ---------------------------------------------------------------------------

fn drawn(width: f64, height: f64, content: &str) -> Page {
    Page { width, height, content: content.as_bytes().to_vec(), images: Vec::new() }
}

#[test]
fn a_written_document_reads_back_with_its_page_sizes() {
    let bytes = writer::write(
        &[drawn(200.0, 100.0, "BT /F0 12 Tf 10 20 Td (Hello PDF) Tj ET"), drawn(595.0, 842.0, "0 0 1 rg 10 10 100 50 re f")],
        &["Helvetica"],
    );
    assert!(bytes.starts_with(b"%PDF-"));
    let doc = Doc::open(bytes).unwrap();
    let all = pages(&doc);
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].size(PageBox::Media), (200.0, 100.0));
    assert_eq!(all[1].size(PageBox::Media), (595.0, 842.0));
    assert_eq!(all[1].rotation(), 0);
}

#[test]
fn text_drawn_on_a_written_page_is_extracted_in_reading_order() {
    let bytes = writer::write(
        &[drawn(200.0, 100.0, "BT /F0 12 Tf 10 20 Td (Hello PDF) Tj ET"), drawn(200.0, 100.0, "BT /F0 9 Tf 10 30 Td (Second) Tj ET")],
        &["Helvetica"],
    );
    let doc = Doc::open(bytes).unwrap();
    let mut extractor = Extractor::new(&doc);
    for p in &pages(&doc) {
        extractor.page(p);
    }
    let pages_seen = extractor.pieces.iter().filter(|p| matches!(p, Piece::Page)).count();
    assert_eq!(pages_seen, 2);
    let lines: Vec<(&str, f32)> = extractor
        .pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Line { text, size, .. } => Some((text.as_str(), *size)),
            _ => None,
        })
        .collect();
    assert_eq!(lines, vec![("Hello PDF", 12.0), ("Second", 9.0)]);
}

#[test]
fn an_empty_written_document_has_no_text_pieces() {
    let bytes = writer::write(&[drawn(100.0, 100.0, "0 0 0 rg 0 0 50 50 re f")], &["Helvetica"]);
    let doc = Doc::open(bytes).unwrap();
    let mut extractor = Extractor::new(&doc);
    extractor.page(&pages(&doc)[0]);
    let lines = extractor.pieces.iter().filter(|p| matches!(p, Piece::Line { .. })).count();
    assert_eq!(lines, 0);
}

// --- string and character encoding ----------------------------------------------------------------------------------

#[test]
fn text_literals_escape_parentheses_and_backslashes() {
    let mut out = Vec::new();
    text_literal("a(b)c\\d", &mut out);
    assert_eq!(out, b"(a\\(b\\)c\\\\d)".to_vec());
}

#[test]
fn text_literals_map_characters_outside_winansi_to_a_question_mark() {
    let mut out = Vec::new();
    text_literal("\u{e9}\u{20ac}\u{4e2d}", &mut out);
    assert_eq!(out, b"(\xe9\x80?)".to_vec());
}

#[test]
fn winansi_maps_ascii_latin1_and_punctuation_to_their_bytes() {
    assert_eq!(winansi('A'), b'A');
    assert_eq!(winansi('\u{e9}'), 0xe9);
    assert_eq!(winansi('\u{2014}'), 0x97);
    assert_eq!(winansi('\u{4e2d}'), b'?');
}

#[test]
fn text_strings_decode_utf16_utf8_and_latin1() {
    assert_eq!(page::text_string(&[0xfe, 0xff, 0x00, 0x48, 0x00, 0x69]), "Hi");
    assert_eq!(page::text_string(&[0xef, 0xbb, 0xbf, b'O', b'k']), "Ok");
    assert_eq!(page::text_string(b"Plain"), "Plain");
    assert_eq!(page::text_string(&[0xe9]), "\u{e9}");
}

// --- outline (bookmarks) ----------------------------------------------------------------------------------------------

#[test]
fn the_outline_is_flattened_in_reading_order_with_levels_and_page_targets() {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /Outlines 5 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 100] >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Type /Outlines /First 6 0 R /Last 8 0 R /Count 3 >>",
        "<< /Title (Intro) /Parent 5 0 R /Next 7 0 R /First 9 0 R /Dest [3 0 R /Fit] >>",
        "<< /Title (Body) /Parent 5 0 R /Prev 6 0 R /Next 8 0 R /Dest [4 0 R /Fit] >>",
        "<< /Title (Appendix) /Parent 5 0 R /Prev 7 0 R /Dest [3 0 R /Fit] >>",
        "<< /Title (Detail) /Parent 6 0 R /Dest [4 0 R /XYZ 0 50 null] >>",
    ];
    let doc = Doc::open(file(&objects)).unwrap();
    let got: Vec<(String, u8, u32)> = page::outline(&doc).iter().map(|o| (o.title.clone(), o.level, o.dest.page)).collect();
    assert_eq!(
        got,
        vec![
            ("Intro".to_string(), 0, 3),
            ("Detail".to_string(), 1, 4),
            ("Body".to_string(), 0, 4),
            ("Appendix".to_string(), 0, 3),
        ]
    );
}

#[test]
fn a_document_without_an_outline_has_no_bookmarks() {
    let doc = Doc::open(file(&two_page_tree())).unwrap();
    assert!(page::outline(&doc).is_empty());
}

// --- document information ---------------------------------------------------------------------------------------------

#[test]
fn the_document_information_dictionary_is_read_as_text() {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 100] >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Title (Quarterly) /Author (Olafur) /Producer (test) /CreationDate (D:20260101120000) >>",
    ];
    let doc = Doc::open(build(&objects, " /Info 4 0 R", None)).unwrap();
    let info = page::info(&doc);
    assert_eq!(info.title, "Quarterly");
    assert_eq!(info.author, "Olafur");
    assert_eq!(info.producer, "test");
    assert_eq!(info.creation_date, "D:20260101120000");
    assert_eq!(info.subject, "");
}
