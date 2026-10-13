//! The document Text Editor opens, saves and keeps: a `Text` plus whether it is rich text. Reading follows the bytes
//! (an RTF header, an HTML prologue, or else plain text) and writing follows the type the file is saved as, so a plain
//! text file is saved as plain text, an RTF file as RTF and an HTML file as HTML.

use crate::text::Text;
use crate::{docx, html, odt, rtf};
use bifrost_app::document::{Document, DocumentError};

/// The file types Text Editor reads and writes (uniform type identifiers).
pub const RTF_TYPE: &str = "public.rtf";
pub const PLAIN_TYPE: &str = "public.plain-text";
pub const HTML_TYPE: &str = "public.html";

#[derive(Clone, Debug)]
pub struct TextDocument {
    pub text: Text,
    /// Whether the document is rich text. A plain text document has one style throughout and its formatting commands
    /// are off; Format > Make Plain Text and Make Rich Text switch it.
    pub rich: bool,
    /// Format > Prevent Editing: text commands and typing leave the document alone. A window setting, not kept in
    /// the file.
    pub read_only: bool,
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
}

impl Document for TextDocument {
    fn read(bytes: &[u8], _uttype: &str) -> Result<Self, DocumentError> {
        // the bytes decide: an RTF header or an HTML prologue means rich text, anything else is plain text
        let mut text = Text::new();
        let rich = if docx::is_docx(bytes) || odt::is_odt(bytes) {
            let cells = if docx::is_docx(bytes) { docx::read(bytes) } else { odt::read(bytes) };
            let Some(cells) = cells else { return Err(DocumentError::new("The document could not be read")) };
            text.load(cells);
            true
        } else if rtf::is_rtf(bytes) {
            let (cells, tab) = rtf::read_doc(bytes);
            text.load(cells);
            text.set_tab_width(tab);
            true
        } else if html::is_html(bytes) {
            let (cells, tab) = html::read(bytes);
            text.load(cells);
            text.set_tab_width(tab);
            true
        } else {
            text.load(rtf::read_plain(bytes));
            false
        };
        Ok(TextDocument { text, rich, read_only: false })
    }

    fn write(&self, uttype: &str) -> Result<Vec<u8>, DocumentError> {
        match uttype {
            RTF_TYPE => Ok(rtf::write_doc(self.text.cells(), self.text.tab_width())),
            HTML_TYPE => Ok(html::write(self.text.cells(), self.text.tab_width())),
            docx::TYPE => Ok(docx::write(self.text.cells())),
            odt::TYPE => Ok(odt::write(self.text.cells())),
            PLAIN_TYPE | "public.utf8-plain-text" => Ok(rtf::write_plain(self.text.cells())),
            other => Err(DocumentError::new(&format!("Text Editor cannot write {other}"))),
        }
    }

    fn untitled() -> Self {
        TextDocument { text: Text::new(), rich: true, read_only: false }
    }

    fn readable_types() -> &'static [&'static str] {
        &[RTF_TYPE, PLAIN_TYPE, HTML_TYPE, docx::TYPE, odt::TYPE]
    }

    fn default_type() -> &'static str {
        RTF_TYPE
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
    fn an_unknown_write_type_is_refused() {
        let doc = TextDocument::untitled();
        assert!(doc.write("public.image").is_err());
        assert_eq!(TextDocument::default_type(), RTF_TYPE);
        assert!(TextDocument::readable_types().contains(&PLAIN_TYPE));
        assert!(TextDocument::readable_types().contains(&HTML_TYPE));
    }
}
