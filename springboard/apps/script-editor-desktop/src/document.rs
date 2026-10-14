//! The document Script Editor opens and saves: a script's text, where the caret is and how far the editor is scrolled.
//! A script is plain text. Bytes are read as UTF-8 and written back as UTF-8 with `\n` line endings.

use crate::buffer::Buffer;
use bifrost_app::document::{Document, DocumentError};

/// Uniform type identifier of a shell script (`.sh`).
pub const SCRIPT_TYPE: &str = "public.shell-script";
/// Uniform type identifier of plain text, read and written as well.
pub const PLAIN_TYPE: &str = "public.plain-text";
/// UTF-8 plain text, the type some shells and editors report for the same files.
pub const UTF8_TYPE: &str = "public.utf8-plain-text";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptDocument {
    pub buffer: Buffer,
    /// The first line shown in the editor.
    pub scroll: usize,
}

impl Document for ScriptDocument {
    fn read(bytes: &[u8], _uttype: &str) -> Result<Self, DocumentError> {
        let text = std::str::from_utf8(bytes).map_err(|_| DocumentError::new("The script is not UTF-8 text"))?;
        Ok(ScriptDocument { buffer: Buffer::from_text(text), scroll: 0 })
    }

    fn write(&self, uttype: &str) -> Result<Vec<u8>, DocumentError> {
        match uttype {
            SCRIPT_TYPE | PLAIN_TYPE | UTF8_TYPE => Ok(self.buffer.text().into_bytes()),
            other => Err(DocumentError::new(&format!("Script Editor cannot write {other}"))),
        }
    }

    fn untitled() -> Self {
        ScriptDocument { buffer: Buffer::new(), scroll: 0 }
    }

    fn readable_types() -> &'static [&'static str] {
        &[SCRIPT_TYPE, PLAIN_TYPE, UTF8_TYPE]
    }

    fn default_type() -> &'static str {
        SCRIPT_TYPE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_round_trips_byte_for_byte() {
        let bytes = b"#!/bin/sh\necho \"hello\"\n\nexit 0\n";
        let doc = ScriptDocument::read(bytes, SCRIPT_TYPE).unwrap();
        assert_eq!(doc.buffer.line_count(), 5);
        assert_eq!(doc.write(SCRIPT_TYPE).unwrap(), bytes.to_vec());
    }

    #[test]
    fn a_new_document_is_empty_and_saves_as_a_shell_script() {
        let doc = ScriptDocument::untitled();
        assert_eq!(doc.buffer.text(), "");
        assert_eq!(ScriptDocument::default_type(), SCRIPT_TYPE);
        assert!(ScriptDocument::readable_types().contains(&SCRIPT_TYPE));
    }

    #[test]
    fn bytes_that_are_not_utf8_are_refused() {
        assert!(ScriptDocument::read(&[0xff, 0xfe, 0x00], SCRIPT_TYPE).is_err());
    }

    #[test]
    fn writing_an_unknown_type_is_refused() {
        let doc = ScriptDocument::untitled();
        assert!(doc.write("public.jpeg").is_err());
    }
}
