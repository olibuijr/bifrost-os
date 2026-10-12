//! Runtime registry of media format readers (the registration of professional video workflow readers,
//! `MTRegisterProfessionalVideoWorkflowFormatReaders`). A reader is a named entry that claims file extensions and
//! header bytes; a reader marked professional is only found after `enable_professional_workflow(true)`, as the
//! registration call turns the professional readers on. Lookups return the first visible reader that claims the file,
//! in registration order. Pure model: it finds a reader for a file and never decodes anything.
//!
//! Gaps: readers are described, not loaded (no reader code runs), no out-of-process readers, no byte source or sidecar
//! handling, and the built-in demuxers are not entries in this registry.

/// A reader and what it claims. Extensions are stored lowercase without the leading dot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatReader {
    pub name: String,
    pub extensions: Vec<String>,
    /// Bytes that must start the file. Empty when the reader claims files by extension only.
    pub header: Vec<u8>,
    /// Whether the reader is one of the professional video workflow readers.
    pub professional: bool,
}

impl FormatReader {
    pub fn new(name: &str, extensions: &[&str], header: &[u8]) -> Self {
        Self {
            name: name.to_string(),
            extensions: extensions.iter().map(|e| normalize_extension(e)).collect(),
            header: header.to_vec(),
            professional: false,
        }
    }

    /// The same reader, marked as a professional video workflow reader.
    pub fn professional(mut self) -> Self {
        self.professional = true;
        self
    }
}

/// Lowercase, with a leading dot removed: ".MXF" and "mxf" are the same extension.
fn normalize_extension(ext: &str) -> String {
    ext.strip_prefix('.').unwrap_or(ext).to_ascii_lowercase()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryError {
    /// The reader has no name.
    EmptyName,
    /// A reader with this name is already registered.
    Duplicate(String),
    /// The reader claims no extension and no header, so no file could ever find it.
    NoMatchRule,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReaderRegistry {
    readers: Vec<FormatReader>,
    professional_enabled: bool,
}

impl ReaderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a reader. Names are unique and compared exactly.
    pub fn register(&mut self, reader: FormatReader) -> Result<(), RegistryError> {
        if reader.name.is_empty() {
            return Err(RegistryError::EmptyName);
        }
        if reader.extensions.is_empty() && reader.header.is_empty() {
            return Err(RegistryError::NoMatchRule);
        }
        if self.readers.iter().any(|r| r.name == reader.name) {
            return Err(RegistryError::Duplicate(reader.name));
        }
        self.readers.push(reader);
        Ok(())
    }

    /// Removes the reader with this name. False when there is none.
    pub fn unregister(&mut self, name: &str) -> bool {
        match self.readers.iter().position(|r| r.name == name) {
            Some(i) => {
                self.readers.remove(i);
                true
            }
            None => false,
        }
    }

    /// Turns the professional workflow readers on or off. Off by default.
    pub fn enable_professional_workflow(&mut self, on: bool) {
        self.professional_enabled = on;
    }

    pub fn professional_enabled(&self) -> bool {
        self.professional_enabled
    }

    /// Reader names in registration order, including the professional readers while they are hidden.
    pub fn names(&self) -> Vec<&str> {
        self.readers.iter().map(|r| r.name.as_str()).collect()
    }

    /// The readers a lookup can return now: all of them, or only the non-professional ones while the workflow is off.
    fn visible(&self) -> impl Iterator<Item = &FormatReader> + '_ {
        let allow_professional = self.professional_enabled;
        self.readers.iter().filter(move |r| allow_professional || !r.professional)
    }

    /// The first visible reader that claims this extension. The extension may be given with a dot and in any case.
    pub fn find_by_extension(&self, extension: &str) -> Option<&FormatReader> {
        let wanted = normalize_extension(extension);
        self.visible().find(|r| r.extensions.iter().any(|e| *e == wanted))
    }

    /// The first visible reader whose header bytes start the file's first bytes. A header longer than the data does not match.
    pub fn find_by_header(&self, head: &[u8]) -> Option<&FormatReader> {
        self.visible().find(|r| !r.header.is_empty() && head.starts_with(&r.header))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_found_in_any_case_with_or_without_the_dot() {
        let mut reg = ReaderRegistry::new();
        reg.register(FormatReader::new("mxf-reader", &[".MXF"], &[])).unwrap();
        assert_eq!(reg.find_by_extension("mxf").map(|r| r.name.as_str()), Some("mxf-reader"));
        assert_eq!(reg.find_by_extension(".Mxf").map(|r| r.name.as_str()), Some("mxf-reader"));
        assert!(reg.find_by_extension("mov").is_none());
    }

    #[test]
    fn registration_refuses_empty_names_duplicates_and_readers_that_match_nothing() {
        let mut reg = ReaderRegistry::new();
        assert_eq!(reg.register(FormatReader::new("", &["x"], &[])), Err(RegistryError::EmptyName));
        assert_eq!(reg.register(FormatReader::new("bare", &[], &[])), Err(RegistryError::NoMatchRule));
        reg.register(FormatReader::new("mxf-reader", &["mxf"], &[])).unwrap();
        assert_eq!(
            reg.register(FormatReader::new("mxf-reader", &["mxf2"], &[])),
            Err(RegistryError::Duplicate("mxf-reader".to_string()))
        );
        assert_eq!(reg.names(), vec!["mxf-reader"]);
    }

    #[test]
    fn professional_readers_are_hidden_until_the_workflow_is_enabled() {
        let mut reg = ReaderRegistry::new();
        reg.register(FormatReader::new("prores-pro", &["mov"], &[]).professional()).unwrap();
        assert!(reg.find_by_extension("mov").is_none());
        // hidden readers still hold their place in the names
        assert_eq!(reg.names(), vec!["prores-pro"]);
        reg.enable_professional_workflow(true);
        assert!(reg.professional_enabled());
        assert_eq!(reg.find_by_extension("mov").map(|r| r.name.as_str()), Some("prores-pro"));
        reg.enable_professional_workflow(false);
        assert!(reg.find_by_extension("mov").is_none());
    }

    #[test]
    fn the_first_registered_reader_wins_and_unregistering_reveals_the_next() {
        let mut reg = ReaderRegistry::new();
        reg.register(FormatReader::new("first", &["mov"], &[])).unwrap();
        reg.register(FormatReader::new("second", &["mov"], &[])).unwrap();
        assert_eq!(reg.find_by_extension("mov").map(|r| r.name.as_str()), Some("first"));
        assert!(reg.unregister("first"));
        assert!(!reg.unregister("first"));
        assert_eq!(reg.find_by_extension("mov").map(|r| r.name.as_str()), Some("second"));
    }

    #[test]
    fn headers_match_the_start_of_the_file() {
        let mut reg = ReaderRegistry::new();
        reg.register(FormatReader::new("wave", &[], b"RIFF")).unwrap();
        let head = b"RIFF\x24\x00\x00\x00WAVE";
        assert_eq!(reg.find_by_header(head).map(|r| r.name.as_str()), Some("wave"));
        // three bytes are not enough to show the four-byte header
        assert!(reg.find_by_header(b"RIF").is_none());
        // a header that is not at the start does not count
        assert!(reg.find_by_header(b"xxRIFF").is_none());
    }

    #[test]
    fn names_keep_registration_order_after_a_removal() {
        let mut reg = ReaderRegistry::new();
        reg.register(FormatReader::new("a", &["a"], &[])).unwrap();
        reg.register(FormatReader::new("b", &["b"], &[])).unwrap();
        reg.register(FormatReader::new("c", &["c"], &[])).unwrap();
        assert!(reg.unregister("b"));
        assert_eq!(reg.names(), vec!["a", "c"]);
    }
}
