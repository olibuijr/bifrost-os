//! Codec registrations from a property-list manifest (the declaration a decoder ships with). A manifest is a dictionary
//! with a `FourCC` string (the four-character codec type, such as `avc1`), a `Name`, and an optional `Professional`
//! boolean. `CodecRegistry` keeps the registrations and finds the decoder for a codec type; professional decoders are
//! only found once the professional workflow is enabled, as the format reader registry does. Pure model: it reads
//! manifests and answers lookups. No decoder code is loaded or run.

use bifrost_core::plist::{parse_any, Plist};

/// One decoder registration: the codec type it handles and what it is called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodecRegistration {
    pub fourcc: [u8; 4],
    pub name: String,
    pub professional: bool,
}

/// Why a manifest or a registration is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecManifestError {
    /// The text is not a property list.
    Unreadable,
    /// The property list is not a dictionary, or a key has the wrong type.
    BadValue(&'static str),
    /// A required key is absent.
    Missing(&'static str),
    /// The codec type is not exactly four printable ASCII characters.
    BadFourCC,
    /// The name is empty.
    EmptyName,
    /// Another registration already claims this codec type.
    Duplicate,
}

/// A four-character codec type from text such as `avc1`. Exactly four printable ASCII bytes, including spaces.
pub fn fourcc_from_str(text: &str) -> Option<[u8; 4]> {
    let bytes = text.as_bytes();
    if bytes.len() != 4 || !bytes.iter().all(|b| (0x20..=0x7e).contains(b)) {
        return None;
    }
    Some([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// The text of a codec type, for messages and listings.
pub fn fourcc_text(fourcc: [u8; 4]) -> String {
    fourcc.iter().map(|&b| b as char).collect()
}

fn value<'a>(entries: &'a [(String, Plist)], key: &str) -> Option<&'a Plist> {
    entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Reads a manifest from property-list text (XML or ASCII form).
pub fn parse_manifest(text: &str) -> Result<CodecRegistration, CodecManifestError> {
    let root = parse_any(text).map_err(|_| CodecManifestError::Unreadable)?;
    let Plist::Dict(entries) = root else {
        return Err(CodecManifestError::BadValue("root"));
    };
    let fourcc = match value(&entries, "FourCC") {
        Some(Plist::Str(s)) => fourcc_from_str(s).ok_or(CodecManifestError::BadFourCC)?,
        Some(_) => return Err(CodecManifestError::BadValue("FourCC")),
        None => return Err(CodecManifestError::Missing("FourCC")),
    };
    let name = match value(&entries, "Name") {
        Some(Plist::Str(s)) if s.trim().is_empty() => return Err(CodecManifestError::EmptyName),
        Some(Plist::Str(s)) => s.trim().to_string(),
        Some(_) => return Err(CodecManifestError::BadValue("Name")),
        None => return Err(CodecManifestError::Missing("Name")),
    };
    let professional = match value(&entries, "Professional") {
        None => false,
        Some(Plist::Bool(b)) => *b,
        Some(_) => return Err(CodecManifestError::BadValue("Professional")),
    };
    Ok(CodecRegistration { fourcc, name, professional })
}

/// The codec decoders known to the system, in registration order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodecRegistry {
    entries: Vec<CodecRegistration>,
    professional_enabled: bool,
}

impl CodecRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a registration. A codec type can be claimed once.
    pub fn register(&mut self, registration: CodecRegistration) -> Result<(), CodecManifestError> {
        if self.entries.iter().any(|e| e.fourcc == registration.fourcc) {
            return Err(CodecManifestError::Duplicate);
        }
        self.entries.push(registration);
        Ok(())
    }

    /// Reads a manifest and registers it.
    pub fn register_manifest(&mut self, text: &str) -> Result<(), CodecManifestError> {
        self.register(parse_manifest(text)?)
    }

    /// Removes the registration for `fourcc`. Returns whether one was removed.
    pub fn unregister(&mut self, fourcc: [u8; 4]) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.fourcc != fourcc);
        self.entries.len() != before
    }

    /// Turns the professional decoders on or off.
    pub fn enable_professional_workflow(&mut self, on: bool) {
        self.professional_enabled = on;
    }

    /// The decoder for `fourcc`, if one is registered and visible (professional ones only when enabled).
    pub fn find(&self, fourcc: [u8; 4]) -> Option<&CodecRegistration> {
        self.entries.iter().find(|e| e.fourcc == fourcc && (!e.professional || self.professional_enabled))
    }

    /// Names of the visible decoders, in registration order.
    pub fn names(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|e| !e.professional || self.professional_enabled)
            .map(|e| e.name.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AVC: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>FourCC</key><string>avc1</string>
<key>Name</key><string>H.264 decoder</string>
</dict></plist>"#;

    const PRORES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>FourCC</key><string>apch</string>
<key>Name</key><string>ProRes decoder</string>
<key>Professional</key><true/>
</dict></plist>"#;

    #[test]
    fn a_manifest_gives_the_codec_type_and_name() {
        let r = parse_manifest(AVC).unwrap();
        assert_eq!(r, CodecRegistration { fourcc: *b"avc1", name: "H.264 decoder".into(), professional: false });
        assert!(parse_manifest(PRORES).unwrap().professional);
    }

    #[test]
    fn a_manifest_without_a_codec_type_or_name_is_refused() {
        let no_code = AVC.replace("<key>FourCC</key><string>avc1</string>", "");
        assert_eq!(parse_manifest(&no_code), Err(CodecManifestError::Missing("FourCC")));
        let no_name = AVC.replace("<key>Name</key><string>H.264 decoder</string>", "");
        assert_eq!(parse_manifest(&no_name), Err(CodecManifestError::Missing("Name")));
        let blank = AVC.replace("H.264 decoder", "   ");
        assert_eq!(parse_manifest(&blank), Err(CodecManifestError::EmptyName));
        assert_eq!(parse_manifest("not a plist <<<"), Err(CodecManifestError::Unreadable));
    }

    #[test]
    fn codec_types_are_exactly_four_printable_ascii_bytes() {
        assert_eq!(fourcc_from_str("avc1"), Some(*b"avc1"));
        assert_eq!(fourcc_from_str("av1 "), Some(*b"av1 "));
        assert_eq!(fourcc_from_str("av"), None);
        assert_eq!(fourcc_from_str("avc11"), None);
        assert_eq!(fourcc_from_str("avé"), None);
        assert_eq!(fourcc_text(*b"hvc1"), "hvc1");
        let wrong = AVC.replace("avc1", "avc");
        assert_eq!(parse_manifest(&wrong), Err(CodecManifestError::BadFourCC));
    }

    #[test]
    fn professional_decoders_are_found_only_once_enabled() {
        let mut registry = CodecRegistry::new();
        registry.register_manifest(AVC).unwrap();
        registry.register_manifest(PRORES).unwrap();
        assert_eq!(registry.find(*b"avc1").map(|r| r.name.as_str()), Some("H.264 decoder"));
        assert!(registry.find(*b"apch").is_none());
        assert_eq!(registry.names(), vec!["H.264 decoder"]);
        registry.enable_professional_workflow(true);
        assert_eq!(registry.find(*b"apch").map(|r| r.name.as_str()), Some("ProRes decoder"));
        assert_eq!(registry.names(), vec!["H.264 decoder", "ProRes decoder"]);
    }

    #[test]
    fn a_codec_type_is_claimed_once_and_can_be_removed() {
        let mut registry = CodecRegistry::new();
        registry.register_manifest(AVC).unwrap();
        assert_eq!(registry.register_manifest(AVC), Err(CodecManifestError::Duplicate));
        assert!(registry.unregister(*b"avc1"));
        assert!(!registry.unregister(*b"avc1"));
        assert!(registry.find(*b"avc1").is_none());
    }
}
