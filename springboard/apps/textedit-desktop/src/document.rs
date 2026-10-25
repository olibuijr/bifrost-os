//! The document Text Editor opens, saves and keeps: a `Text` plus whether it is rich text. Reading follows the bytes
//! (an RTF header, an HTML prologue, or else plain text) and writing follows the type the file is saved as, so a plain
//! text file is saved as plain text, an RTF file as RTF and an HTML file as HTML.

use crate::encoding::{self, Encoding};
use crate::properties::Properties;
use crate::text::{Cell, Text, OBJECT};
use crate::{docx, html, odt, rtf, rtfd, webarchive};
use bifrost_app::document::{Document, DocumentError};
use bifrost_app::document_package;
use std::sync::atomic::{AtomicBool, Ordering};

/// Settings > Open and Save: RTF and HTML files open as plain text showing their markup. The document reader is a
/// plain function of the bytes, so the setting is read from here.
static MARKUP_AS_CODE: AtomicBool = AtomicBool::new(false);

pub fn set_markup_as_code(on: bool) {
    MARKUP_AS_CODE.store(on, Ordering::Relaxed);
}

/// The file types Text Editor reads and writes (uniform type identifiers).
pub const RTF_TYPE: &str = "public.rtf";
pub const PLAIN_TYPE: &str = "public.plain-text";
pub const HTML_TYPE: &str = "public.html";
/// A folder package: rich text with its attachments (see `rtfd`). Read and written as a folder, not as bytes.
#[cfg_attr(not(test), allow(dead_code))]
pub const RTFD_TYPE: &str = "public.rtfd";

/// The cells without their attachment characters (U+FFFC), for the formats that have no picture to write them as.
fn without_pictures(cells: &[Cell]) -> Vec<Cell> {
    cells.iter().filter(|c| c.ch != OBJECT).copied().collect()
}

#[derive(Clone, Debug)]
pub struct TextDocument {
    pub text: Text,
    /// Whether the document is rich text. A plain text document has one style throughout and its formatting commands
    /// are off; Format > Make Plain Text and Make Rich Text switch it.
    pub rich: bool,
    /// Format > Prevent Editing: text commands and typing leave the document alone. A window setting, not kept in
    /// the file.
    pub read_only: bool,
    /// The encoding a plain text document is saved in; reading sets it to what the file turned out to be.
    pub encoding: Encoding,
    /// The title, author, subject, keywords and copyright (File > Show Properties).
    pub properties: Properties,
    /// An RTF or HTML file opened as plain text showing its markup: saved back as that text, not wrapped in the format.
    pub code: bool,
}

impl TextDocument {
    /// Switches to plain text, dropping the formatting (one undo entry), or back to rich text (nothing to restore).
    pub fn set_rich(&mut self, rich: bool) {
        if self.rich == rich {
            return;
        }
        if !rich {
            self.text.make_plain();
        }
        self.rich = rich;
    }

    /// Reads a file. The bytes decide: a package, an RTF header or an HTML prologue means rich text, anything else is
    /// plain text in whatever encoding it turns out to be. With `markup_as_code` an RTF or HTML file opens as plain text
    /// showing its markup.
    pub fn read_bytes(bytes: &[u8], markup_as_code: bool) -> Result<Self, DocumentError> {
        let mut text = Text::new();
        let mut encoding = Encoding::Utf8;
        let mut properties = Properties::default();
        let mut code = false;
        let as_code = markup_as_code && (rtf::is_rtf(bytes) || html::is_html(bytes));
        let rich = if as_code {
            let (decoded, found) = encoding::detect(bytes);
            encoding = found;
            code = true;
            text.load(rtf::plain_cells(&decoded));
            false
        } else if webarchive::is_webarchive(bytes) {
            let Some((cells, tab, props)) = webarchive::read(bytes) else { return Err(DocumentError::new("The web archive could not be read")) };
            text.load(cells);
            text.set_tab_width(tab);
            properties = props;
            true
        } else if docx::is_docx(bytes) || odt::is_odt(bytes) {
            let cells = if docx::is_docx(bytes) { docx::read(bytes) } else { odt::read(bytes) };
            let Some(cells) = cells else { return Err(DocumentError::new("The document could not be read")) };
            text.load(cells);
            properties = if docx::is_docx(bytes) { docx::read_properties(bytes) } else { odt::read_properties(bytes) };
            true
        } else if crate::office::is_office(bytes) {
            // a workbook or a deck opens as its plain text (sheets as tab-separated rows, slides as paragraphs)
            let Some(plain) = crate::office::text(bytes) else { return Err(DocumentError::new("The document could not be read")) };
            text.load(rtf::plain_cells(&plain));
            false
        } else if rtf::is_rtf(bytes) {
            let (cells, tab, pictures) = rtf::read_doc_pictures(bytes);
            text.load_with_attachments(cells, rtfd::attachments_without_data(pictures));
            text.set_tab_width(tab);
            properties = rtf::read_info(bytes);
            true
        } else if html::is_html(bytes) {
            let (cells, tab) = html::read(bytes);
            text.load(cells);
            text.set_tab_width(tab);
            properties = html::read_properties(bytes);
            true
        } else {
            let (decoded, found) = encoding::detect(bytes);
            encoding = found;
            text.load(rtf::plain_cells(&decoded));
            false
        };
        if !rich {
            // a plain document has one font, the plain text font Make Plain Text switches to
            text.use_plain_font();
        }
        Ok(TextDocument { text, rich, read_only: false, encoding, properties, code })
    }

    /// Reads a folder package (RTFD): its files by name, as `document_package::read_package_dir` gives them. The text
    /// is rich text; its attachments are the files the package holds.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn read_package_files(files: &[(String, Vec<u8>)]) -> Result<Self, DocumentError> {
        let Some(package) = rtfd::read(files) else { return Err(DocumentError::new("The document package has no readable text")) };
        let mut text = Text::new();
        text.load_with_attachments(package.cells, package.attachments);
        text.set_tab_width(package.tab);
        Ok(TextDocument { text, rich: true, read_only: false, encoding: Encoding::Utf8, properties: package.properties, code: false })
    }

    /// The files of the package this document is saved as: `TXT.rtf`, then each attachment the text holds. Save them
    /// with `document_package::write_package_dir`, which replaces the package atomically.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn files_of_package(&self) -> Vec<(String, Vec<u8>)> {
        rtfd::write(self.text.cells(), self.text.tab_width(), &self.properties, self.text.attachments())
    }

    /// Saves this document as the package at `dir`, atomically.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn save_package(&self, dir: &std::path::Path) -> std::io::Result<()> {
        let files = self.files_of_package();
        let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        document_package::write_package_dir(dir, &refs)
    }

    /// Opens the package at `dir`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn open_package(dir: &std::path::Path) -> Result<Self, DocumentError> {
        let files = document_package::read_package_dir(dir).map_err(|e| DocumentError::new(&e.to_string()))?;
        Self::read_package_files(&files)
    }

    /// The text in the document's encoding. A character the encoding cannot hold stops the save, so nothing is lost
    /// silently.
    fn write_plain(&self) -> Result<Vec<u8>, DocumentError> {
        let text = rtf::plain_text(self.text.cells());
        encoding::encode(&text, self.encoding).map_err(|ch| {
            DocumentError::new(&format!("\"{ch}\" cannot be saved in {}. Choose another encoding in Format > Text Encoding.", self.encoding.name()))
        })
    }
}

impl Document for TextDocument {
    fn read(bytes: &[u8], _uttype: &str) -> Result<Self, DocumentError> {
        Self::read_bytes(bytes, MARKUP_AS_CODE.load(Ordering::Relaxed))
    }

    fn write(&self, uttype: &str) -> Result<Vec<u8>, DocumentError> {
        match uttype {
            // markup opened as code is saved as the text it shows
            RTF_TYPE | HTML_TYPE if self.code && !self.rich => self.write_plain(),
            RTF_TYPE => Ok(rtf::write_with_attachments(self.text.cells(), self.text.tab_width(), &self.properties, self.text.attachments())),
            // these formats carry no pictures from the text: an attachment is left out of them (RTF and RTFD keep it)
            HTML_TYPE => Ok(html::write_with(&without_pictures(self.text.cells()), self.text.tab_width(), &self.properties)),
            docx::TYPE => Ok(docx::write_with(&without_pictures(self.text.cells()), &self.properties)),
            odt::TYPE => Ok(odt::write_with(&without_pictures(self.text.cells()), &self.properties)),
            webarchive::TYPE => Ok(webarchive::write(&without_pictures(self.text.cells()), self.text.tab_width(), &self.properties)),
            PLAIN_TYPE => self.write_plain(),
            "public.utf8-plain-text" => Ok(rtf::write_plain(self.text.cells())),
            other => Err(DocumentError::new(&format!("Text Editor cannot write {other}"))),
        }
    }

    fn untitled() -> Self {
        TextDocument { text: Text::new(), rich: true, read_only: false, encoding: Encoding::Utf8, properties: Properties::default(), code: false }
    }

    fn readable_types() -> &'static [&'static str] {
        &[RTF_TYPE, PLAIN_TYPE, HTML_TYPE, docx::TYPE, odt::TYPE, webarchive::TYPE]
    }

    fn default_type() -> &'static str {
        RTF_TYPE
    }

    fn read_package(files: &[(String, Vec<u8>)], uttype: &str) -> Result<Self, DocumentError> {
        if uttype != RTFD_TYPE {
            return Err(DocumentError::new(&format!("Text Editor cannot read {uttype} packages")));
        }
        Self::read_package_files(files)
    }

    fn package_files(&self, uttype: &str) -> Result<Vec<(String, Vec<u8>)>, DocumentError> {
        if uttype != RTFD_TYPE {
            return Err(DocumentError::new(&format!("Text Editor cannot write {uttype} packages")));
        }
        Ok(self.files_of_package())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_file_stays_plain_and_an_rtf_file_stays_rich() {
        let plain = TextDocument::read(b"hello\nworld", PLAIN_TYPE).unwrap();
        assert!(!plain.rich);
        assert_eq!(plain.text.text(), "hello\nworld");
        let out = plain.write(PLAIN_TYPE).unwrap();
        assert_eq!(out, b"hello\nworld".to_vec());
        assert!(plain.text.cells().iter().all(|c| c.style.font == crate::text::PLAIN_FONT), "a plain file opens in the plain text font");

        let rich = TextDocument::read(b"{\\rtf1\\ansi \\b bold\\b0  text}", RTF_TYPE).unwrap();
        assert!(rich.rich);
        assert_eq!(rich.text.text(), "bold text");
        assert!(rich.write(RTF_TYPE).unwrap().starts_with(b"{\\rtf1"));
    }

    #[test]
    fn html_opens_as_rich_text_and_saves_back_as_html() {
        let mut doc = TextDocument::read(b"<!DOCTYPE html><html><body><p>one <b>two</b></p></body></html>", HTML_TYPE).unwrap();
        assert!(doc.rich);
        assert_eq!(doc.text.text(), "one two");
        doc.text.set_tab_width(60.0);
        let out = doc.write(HTML_TYPE).unwrap();
        assert!(out.starts_with(b"<!DOCTYPE html>"));
        let back = TextDocument::read(&out, HTML_TYPE).unwrap();
        assert_eq!(back.text.cells(), doc.text.cells());
        assert_eq!(back.text.tab_width(), 60.0);
    }

    #[test]
    fn word_processing_packages_open_as_rich_text_and_save_back() {
        let mut doc = TextDocument::untitled();
        // a paragraph break carries paragraph properties only, so it is not bold
        doc.text.load("one\ntwo".chars().map(|ch| crate::text::Cell { ch, style: crate::text::Style { bold: ch != '\n', ..Default::default() } }).collect());
        for kind in [docx::TYPE, odt::TYPE] {
            let bytes = doc.write(kind).unwrap();
            let back = TextDocument::read(&bytes, kind).unwrap();
            assert!(back.rich, "{kind}");
            assert_eq!(back.text.cells(), doc.text.cells(), "{kind}");
        }
        assert!(TextDocument::readable_types().contains(&docx::TYPE));
        assert!(TextDocument::readable_types().contains(&odt::TYPE));
    }

    #[test]
    fn the_tab_stop_distance_is_kept_in_rtf() {
        let mut doc = TextDocument::untitled();
        doc.text.set_tab_width(72.0);
        let back = TextDocument::read(&doc.write(RTF_TYPE).unwrap(), RTF_TYPE).unwrap();
        assert_eq!(back.text.tab_width(), 72.0);
    }

    #[test]
    fn making_a_document_plain_drops_its_formatting() {
        let mut doc = TextDocument::read(b"{\\rtf1 \\b x}", RTF_TYPE).unwrap();
        assert!(doc.text.cells()[0].style.bold);
        doc.set_rich(false);
        assert!(!doc.rich);
        assert!(!doc.text.cells()[0].style.bold);
        doc.set_rich(true);
        assert!(doc.rich);
    }

    #[test]
    fn a_plain_file_keeps_the_encoding_it_was_read_in() {
        let utf16 = crate::encoding::encode("Halló\nheimur", Encoding::Utf16Be).unwrap();
        let doc = TextDocument::read(&utf16, PLAIN_TYPE).unwrap();
        assert_eq!(doc.encoding, Encoding::Utf16Be);
        assert_eq!(doc.text.text(), "Halló\nheimur");
        assert_eq!(doc.write(PLAIN_TYPE).unwrap(), utf16, "saved the way it was read");

        let western = TextDocument::read(&[b'c', b'a', b'f', 0xe9], PLAIN_TYPE).unwrap();
        assert_eq!(western.encoding, Encoding::Windows1252);
        assert_eq!(western.write(PLAIN_TYPE).unwrap(), vec![b'c', b'a', b'f', 0xe9]);
    }

    #[test]
    fn changing_the_encoding_changes_the_saved_bytes_and_unsupported_text_stops_the_save() {
        let mut doc = TextDocument::read("naïve €".as_bytes(), PLAIN_TYPE).unwrap();
        assert_eq!(doc.encoding, Encoding::Utf8);
        doc.encoding = Encoding::Windows1252;
        assert_eq!(doc.write(PLAIN_TYPE).unwrap(), vec![b'n', b'a', 0xef, b'v', b'e', b' ', 0x80]);
        doc.encoding = Encoding::Latin1;
        let err = doc.write(PLAIN_TYPE).unwrap_err();
        assert!(err.0.contains("cannot be saved in Western (ISO Latin 1)"), "{}", err.0);
        assert_eq!(doc.write("public.utf8-plain-text").unwrap(), "naïve €".as_bytes().to_vec(), "the UTF-8 type is always UTF-8");
    }

    #[test]
    fn the_properties_are_read_from_and_written_to_each_format() {
        let mut doc = TextDocument::untitled();
        doc.text.load("body".chars().map(|ch| crate::text::Cell { ch, style: Default::default() }).collect());
        doc.properties.title = "Plan".into();
        doc.properties.author = "Ada".into();
        doc.properties.subject = "Q3".into();
        doc.properties.keywords = "a, b".into();
        for kind in [docx::TYPE, odt::TYPE] {
            let back = TextDocument::read(&doc.write(kind).unwrap(), kind).unwrap();
            assert_eq!(back.properties, doc.properties, "{kind}");
        }
        doc.properties.copyright = "2026".into();
        let back = TextDocument::read(&doc.write(RTF_TYPE).unwrap(), RTF_TYPE).unwrap();
        assert_eq!(back.properties, doc.properties);
        assert_eq!(back.text.text(), "body");
        assert!(TextDocument::read(b"plain", PLAIN_TYPE).unwrap().properties.is_empty());
    }

    #[test]
    fn markup_opened_as_code_is_plain_text_and_saves_as_the_same_text() {
        let rtf_src = b"{\\rtf1 \\b bold\\b0 }".to_vec();
        let as_code = TextDocument::read_bytes(&rtf_src, true).unwrap();
        let html_src = b"<!DOCTYPE html><p>hi</p>".to_vec();
        let html_code = TextDocument::read_bytes(&html_src, true).unwrap();
        assert!(!as_code.rich && as_code.code);
        assert_eq!(as_code.text.text(), "{\\rtf1 \\b bold\\b0 }");
        assert_eq!(as_code.write(RTF_TYPE).unwrap(), rtf_src);
        assert_eq!(html_code.write(HTML_TYPE).unwrap(), html_src);
        // switched off, the same bytes open as formatted text
        let formatted = TextDocument::read_bytes(&rtf_src, false).unwrap();
        assert!(formatted.rich && !formatted.code);
        assert_eq!(formatted.text.text(), "bold");
    }

    #[test]
    fn an_rtfd_package_round_trips_its_text_and_attachment_through_a_folder() {
        let png = {
            let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
            v.extend_from_slice(&13u32.to_be_bytes());
            v.extend_from_slice(b"IHDR");
            v.extend_from_slice(&4u32.to_be_bytes());
            v.extend_from_slice(&2u32.to_be_bytes());
            v
        };
        let mut doc = TextDocument::untitled();
        doc.text.load("see ".chars().map(|ch| crate::text::Cell { ch, style: Default::default() }).collect());
        doc.text.select(4, 4);
        doc.text.insert_attachment(crate::text::Attachment { file: "my photo.png".into(), width: 40.0, height: 20.0, data: png.clone() }, crate::text::Kind::Pasting);
        let dir = std::env::temp_dir().join(format!("textedit-rtfd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pkg = dir.join("Note.rtfd");
        std::fs::create_dir_all(&dir).unwrap();
        doc.save_package(&pkg).unwrap();
        let back = TextDocument::open_package(&pkg).unwrap();
        assert!(back.rich);
        assert_eq!(back.text.text(), doc.text.text());
        assert_eq!(back.text.attachments()[0].data, png);
        assert_eq!(back.text.attachments()[0].file, "my_photo.png");
        assert_eq!(back.text.attachments()[0].width, 40.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_rtf_file_with_a_picture_keeps_it_as_an_attachment() {
        // the size in the file is in twips: 800 x 400 twips is 40 x 20 points
        let doc = TextDocument::read(b"{\\rtf1 a{\\NeXTGraphic p.png \\width800\\height400}b}", RTF_TYPE).unwrap();
        assert_eq!(doc.text.attachments().len(), 1);
        assert!(doc.text.has_attachments());
        let back = TextDocument::read(&doc.write(RTF_TYPE).unwrap(), RTF_TYPE).unwrap();
        assert_eq!(back.text.attachments()[0].file, "p.png");
        assert_eq!(back.text.attachments()[0].width, 40.0);
        // plain text drops the attachment character
        assert_eq!(doc.write(PLAIN_TYPE).unwrap(), b"ab".to_vec());
    }

    #[test]
    fn the_formats_without_pictures_leave_the_attachment_character_out() {
        let doc = TextDocument::read(b"{\\rtf1 a{\\NeXTGraphic p.png \\width40\\height20}b}", RTF_TYPE).unwrap();
        assert_eq!(doc.text.text().chars().count(), 3);
        for kind in [docx::TYPE, odt::TYPE, HTML_TYPE, webarchive::TYPE] {
            let bytes = doc.write(kind).unwrap();
            let back = TextDocument::read(&bytes, kind).unwrap();
            assert!(!back.text.text().contains('\u{fffc}'), "{kind}");
        }
    }

    #[test]
    fn an_unknown_write_type_is_refused() {
        let doc = TextDocument::untitled();
        assert!(doc.write("public.image").is_err());
        assert_eq!(TextDocument::default_type(), RTF_TYPE);
        assert!(TextDocument::readable_types().contains(&PLAIN_TYPE));
        assert!(TextDocument::readable_types().contains(&HTML_TYPE));
    }
}
