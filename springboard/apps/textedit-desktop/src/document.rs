//! The document Text Editor opens, saves and keeps: a `Text` plus whether it is rich text. Reading and writing follow
//! the file's type, so a plain text file is saved as plain text and an RTF file as RTF.

use crate::rtf;
use crate::text::Text;
use bifrost_app::document::{Document, DocumentError};

/// The file types Text Editor reads and writes (uniform type identifiers).
pub const RTF_TYPE: &str = "public.rtf";
pub const PLAIN_TYPE: &str = "public.plain-text";

#[derive(Clone, Debug)]
pub struct TextDocument {
    pub text: Text,
    /// Whether the file was rich text (RTF). Saving follows the document type the framework passes, so this only
    /// records the source format (for the Format menu's Make Plain/Rich Text item, not built yet).
    #[cfg_attr(not(test), allow(dead_code))]
    pub rich: bool,
}

impl Document for TextDocument {
    fn read(bytes: &[u8], _uttype: &str) -> Result<Self, DocumentError> {
        // the bytes decide: an RTF header means rich text, anything else is plain text
        let rich = rtf::is_rtf(bytes);
        let cells = if rich { rtf::read(bytes) } else { rtf::read_plain(bytes) };
        let mut text = Text::new();
        text.load(cells);
        Ok(TextDocument { text, rich })
    }

    fn write(&self, uttype: &str) -> Result<Vec<u8>, DocumentError> {
        match uttype {
            RTF_TYPE => Ok(rtf::write(self.text.cells())),
            PLAIN_TYPE => Ok(rtf::write_plain(self.text.cells())),
            other => Err(DocumentError::new(&format!("Text Editor cannot write {other}"))),
        }
    }

    fn untitled() -> Self {
        TextDocument { text: Text::new(), rich: true }
    }

    fn readable_types() -> &'static [&'static str] {
        &[RTF_TYPE, PLAIN_TYPE]
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
    fn an_unknown_write_type_is_refused() {
        let doc = TextDocument::untitled();
        assert!(doc.write("public.image").is_err());
        assert_eq!(TextDocument::default_type(), RTF_TYPE);
        assert!(TextDocument::readable_types().contains(&PLAIN_TYPE));
    }
}
