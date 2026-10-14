//! What the Inspector shows about the open file: its name, kind and size, the pages or pixels, a summary of the markup,
//! and for a JPEG the details its EXIF block holds (date, camera, exposure, location). The file facts are read once when
//! the file opens; the rows are put together from them and the live document each time the panel draws.

use crate::model::{Mark, MarkKind};
use std::io::Read;
use std::path::Path;

/// How much of a JPEG is read for its EXIF block, which sits before the image data.
const HEAD_BYTES: u64 = 256 * 1024;

/// What the file itself tells.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub name: String,
    pub kind: &'static str,
    pub bytes: u64,
    /// EXIF details of a JPEG, as label and value.
    pub exif: Vec<(&'static str, String)>,
}

/// Reads the facts of the file at `path`. A file that cannot be read has a name and kind and no size.
pub fn read(path: &Path) -> Facts {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let kind = kind_of(&ext);
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut exif = Vec::new();
    if kind == "JPEG image" {
        let mut head = Vec::new();
        if let Ok(file) = std::fs::File::open(path) {
            if file.take(HEAD_BYTES).read_to_end(&mut head).is_err() {
                head.clear();
            }
        }
        exif = exif_rows(&head);
    }
    Facts { name, kind, bytes, exif }
}

/// The kind of file for an extension (lower case).
pub fn kind_of(ext: &str) -> &'static str {
    match ext {
        "pdf" => "PDF document",
        "png" => "PNG image",
        "jpg" | "jpeg" => "JPEG image",
        _ => "Document",
    }
}

/// The EXIF details found at the start of a JPEG file, without the pixel size (the Inspector shows the picture's own).
pub fn exif_rows(head: &[u8]) -> Vec<(&'static str, String)> {
    springboard::shell::photos::exif::parse(head)
        .map(|e| e.rows())
        .unwrap_or_default()
        .into_iter()
        .filter(|(label, _)| *label != "Size")
        .collect()
}

/// The place the picture was taken, as the EXIF block gives it ("64.1466 N, 21.9426 W").
pub fn location(facts: &Facts) -> Option<&str> {
    facts.exif.iter().find(|(label, _)| *label == "Location").map(|(_, value)| value.as_str())
}

/// A file size in bytes, KB or MB.
pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} bytes")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{:.1} MB", b / (KB * KB))
    }
}

/// A count of the markup by kind: "2 highlights, 1 note", or "None".
pub fn annotation_summary(marks: &[Mark]) -> String {
    const KINDS: [MarkKind; 15] = [
        MarkKind::Highlight,
        MarkKind::Note,
        MarkKind::Underline,
        MarkKind::Strike,
        MarkKind::Rect,
        MarkKind::Rounded,
        MarkKind::Oval,
        MarkKind::Line,
        MarkKind::Arrow,
        MarkKind::Ink,
        MarkKind::Star,
        MarkKind::Polygon,
        MarkKind::Bubble,
        MarkKind::Text,
        MarkKind::Redact,
    ];
    let parts: Vec<String> = KINDS
        .iter()
        .filter_map(|&kind| {
            let n = marks.iter().filter(|m| m.kind == kind).count();
            let name = kind.label().to_lowercase();
            match n {
                0 => None,
                1 => Some(format!("1 {name}")),
                _ => Some(format!("{n} {name}s")),
            }
        })
        .collect();
    if parts.is_empty() {
        "None".into()
    } else {
        parts.join(", ")
    }
}

/// One row per mark, in document order: its kind as the label, and its page (and text, for a note or text box) as the
/// value, so the Inspector lists the markup itself and not only the counts.
pub fn annotation_rows(marks: &[Mark]) -> Vec<(String, String)> {
    marks
        .iter()
        .map(|m| {
            let value = if m.text.trim().is_empty() {
                format!("Page {}", m.page + 1)
            } else {
                format!("Page {}: {}", m.page + 1, m.text.trim())
            };
            (m.kind.label().to_string(), value)
        })
        .collect()
}

/// The live part of the Inspector's rows.
pub struct Shown<'a> {
    pub pages: usize,
    pub is_pdf: bool,
    /// The picture's size in pixels, for a picture document.
    pub size: Option<(usize, usize)>,
    pub marks: &'a [Mark],
    pub bookmarks: usize,
    /// The PDF's information rows and whether it is encrypted.
    pub info: &'a [(&'static str, String)],
    pub encrypted: bool,
}

/// A text string of a PDF: UTF-16 big endian when it starts with the byte order mark, otherwise one character per byte
/// (the PDFDocEncoding that Latin text uses).
pub fn pdf_text(bytes: &[u8]) -> String {
    match bytes.strip_prefix(&[0xFE, 0xFF]) {
        Some(rest) => {
            let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        }
        None => bytes.iter().map(|&b| b as char).collect(),
    }
}

/// The rows the Inspector lists, as label and value.
pub fn rows(facts: &Facts, shown: &Shown) -> Vec<(String, String)> {
    let mut out = vec![("Name".to_string(), facts.name.clone()), ("Kind".to_string(), facts.kind.to_string())];
    if facts.bytes > 0 {
        out.push(("File Size".into(), human_size(facts.bytes)));
    }
    if shown.is_pdf {
        out.push(("Pages".into(), shown.pages.to_string()));
    }
    if let (false, Some((w, h))) = (shown.is_pdf, shown.size) {
        out.push(("Dimensions".into(), format!("{w} x {h} pixels")));
    }
    out.push(("Annotations".into(), annotation_summary(shown.marks)));
    out.extend(annotation_rows(shown.marks));
    if shown.bookmarks > 0 {
        out.push(("Bookmarks".into(), shown.bookmarks.to_string()));
    }
    if shown.is_pdf {
        out.push(("Security".into(), if shown.encrypted { "Password protected".into() } else { "None".into() }));
        out.extend(shown.info.iter().map(|(label, value)| (label.to_string(), value.clone())));
    }
    out.extend(facts.exif.iter().map(|(label, value)| (label.to_string(), value.clone())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Norm, MARK_BLUE};

    fn mark(kind: MarkKind) -> Mark {
        Mark { kind, page: 0, rect: Norm { x: 0.0, y: 0.0, w: 0.1, h: 0.1 }, color: MARK_BLUE, text: String::new(), ink: Vec::new() }
    }

    #[test]
    fn kinds_follow_the_extension() {
        assert_eq!(kind_of("pdf"), "PDF document");
        assert_eq!(kind_of("jpeg"), "JPEG image");
        assert_eq!(kind_of("png"), "PNG image");
        assert_eq!(kind_of("txt"), "Document");
    }

    #[test]
    fn sizes_read_in_bytes_kb_and_mb() {
        assert_eq!(human_size(512), "512 bytes");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn the_markup_is_counted_by_kind() {
        assert_eq!(annotation_summary(&[]), "None");
        let marks = [mark(MarkKind::Highlight), mark(MarkKind::Note), mark(MarkKind::Highlight), mark(MarkKind::Strike)];
        assert_eq!(annotation_summary(&marks), "2 highlights, 1 note, 1 strike-through");
    }

    #[test]
    fn each_mark_is_listed_with_its_page_and_text() {
        let mut note = mark(MarkKind::Note);
        note.page = 2;
        note.text = "  check the total  ".into();
        let mut highlight = mark(MarkKind::Highlight);
        highlight.page = 0;
        let rows = annotation_rows(&[highlight, note]);
        assert_eq!(rows[0], ("Highlight".to_string(), "Page 1".to_string()));
        assert_eq!(rows[1], ("Note".to_string(), "Page 3: check the total".to_string()));
        assert!(annotation_rows(&[]).is_empty());
    }

    #[test]
    fn the_inspector_rows_hold_the_markup_list_after_the_count() {
        let marks = [mark(MarkKind::Underline)];
        let shown = Shown { pages: 1, is_pdf: true, size: None, marks: &marks, bookmarks: 0, info: &[], encrypted: false };
        let rows = super::rows(&Facts { name: "c.pdf".into(), kind: "PDF document", ..Facts::default() }, &shown);
        let at = rows.iter().position(|(l, _)| l == "Annotations").unwrap();
        assert_eq!(rows[at + 1], ("Underline".to_string(), "Page 1".to_string()));
    }

    #[test]
    fn rows_name_the_file_and_what_is_in_it() {
        let facts = Facts { name: "a.jpg".into(), kind: "JPEG image", bytes: 2048, exif: vec![("Camera", "Cam".into()), ("Location", "64.1466 N, 21.9426 W".into())] };
        let picture = Shown { pages: 1, is_pdf: false, size: Some((40, 30)), marks: &[], bookmarks: 0, info: &[], encrypted: false };
        let rows = rows(&facts, &picture);
        let get = |label: &str| rows.iter().find(|(l, _)| l == label).map(|(_, v)| v.as_str());
        assert_eq!(get("Name"), Some("a.jpg"));
        assert_eq!(get("File Size"), Some("2.0 KB"));
        assert_eq!(get("Dimensions"), Some("40 x 30 pixels"));
        assert_eq!(get("Pages"), None);
        assert_eq!(get("Annotations"), Some("None"));
        assert_eq!(get("Camera"), Some("Cam"));
        assert_eq!(location(&facts), Some("64.1466 N, 21.9426 W"));
        let info = [("Title", "Notes".to_string())];
        let pdf = Shown { pages: 9, is_pdf: true, size: None, marks: &[], bookmarks: 2, info: &info, encrypted: true };
        let rows = super::rows(&Facts { name: "b.pdf".into(), kind: "PDF document", ..Facts::default() }, &pdf);
        assert!(rows.contains(&("Pages".to_string(), "9".to_string())));
        assert!(rows.contains(&("Bookmarks".to_string(), "2".to_string())));
        assert!(rows.iter().all(|(l, _)| l != "File Size" && l != "Dimensions"));
        assert!(rows.contains(&("Security".to_string(), "Password protected".to_string())));
        assert!(rows.contains(&("Title".to_string(), "Notes".to_string())));
    }

    #[test]
    fn pdf_text_strings_decode_utf16_and_latin() {
        assert_eq!(pdf_text(b"Caf\xe9"), "Caf\u{e9}");
        assert_eq!(pdf_text(&[0xFE, 0xFF, 0x00, 0x41, 0x00, 0xF0]), "A\u{f0}");
    }

    #[test]
    fn a_file_without_exif_has_no_location() {
        assert!(exif_rows(b"\x89PNG\r\n\x1a\n").is_empty());
        assert_eq!(location(&Facts::default()), None);
    }

    #[test]
    fn reading_a_missing_file_still_names_it() {
        let facts = read(Path::new("/nonexistent/photo.JPG"));
        assert_eq!(facts.name, "photo.JPG");
        assert_eq!(facts.kind, "JPEG image");
        assert_eq!(facts.bytes, 0);
        assert!(facts.exif.is_empty());
    }
}
