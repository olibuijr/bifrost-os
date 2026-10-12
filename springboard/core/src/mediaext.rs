//! Registry of decoders that declare the four-character codec type they handle (std only).
//!
//! A codec type is four printable ASCII bytes such as `avc1` or `hvc1`. A decoder
//! that a package ships declares its codec type, the implementation id that the
//! extension host loads, and an optional display name. The declaration is a small
//! `key = value` text block, one key per line, with `#` comments:
//!
//! ```text
//! # decoder for H.264 in an ISO base media file
//! CodecType = avc1
//! ClassImplementationID = org.example.h264-decoder
//! DisplayName = Example H.264
//! ```
//!
//! This module parses those declarations and keeps the registry that maps a codec type
//! to one decoder. It loads no code, opens no file and starts no process: the caller
//! decides which implementation to run.

use std::fmt;

/// Longest implementation id accepted in a declaration.
const MAX_IMPLEMENTATION_ID: usize = 255;

/// Why a codec type, declaration or registration was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaExtError {
    /// The codec type is not exactly four printable ASCII bytes.
    InvalidCodecType,
    /// The implementation id is empty, too long, or holds a character outside `[A-Za-z0-9._-]`.
    InvalidImplementationId,
    /// A decoder for this codec type is already registered.
    DuplicateCodec(FourCc),
    /// The declaration has no value for a required key.
    MissingKey(&'static str),
    /// A line is not `key = value`, repeats a key, or uses an unknown key. Holds the line number, from 1.
    Malformed(usize),
}

impl fmt::Display for MediaExtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaExtError::InvalidCodecType => f.write_str("codec type must be four printable ASCII bytes"),
            MediaExtError::InvalidImplementationId => f.write_str("invalid decoder implementation id"),
            MediaExtError::DuplicateCodec(codec) => write!(f, "a decoder for {codec} is already registered"),
            MediaExtError::MissingKey(key) => write!(f, "declaration has no {key}"),
            MediaExtError::Malformed(line) => write!(f, "declaration line {line} is malformed"),
        }
    }
}

/// A four-character codec type, such as `avc1`. Ordered by its big-endian value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FourCc([u8; 4]);

impl FourCc {
    /// Parses exactly four printable ASCII bytes.
    pub fn parse(text: &str) -> Result<FourCc, MediaExtError> {
        let bytes = text.as_bytes();
        if bytes.len() != 4 || !bytes.iter().all(|b| (0x20..=0x7E).contains(b)) {
            return Err(MediaExtError::InvalidCodecType);
        }
        Ok(FourCc([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Builds a codec type from its big-endian value. Returns `None` when a byte is not printable ASCII.
    pub fn from_u32(value: u32) -> Option<FourCc> {
        let bytes = value.to_be_bytes();
        if bytes.iter().all(|b| (0x20..=0x7E).contains(b)) {
            Some(FourCc(bytes))
        } else {
            None
        }
    }

    /// The big-endian value, so `avc1` is `0x61766331`.
    pub fn as_u32(self) -> u32 {
        u32::from_be_bytes(self.0)
    }
}

impl fmt::Display for FourCc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Parsing guarantees printable ASCII, so the bytes are always valid UTF-8.
        f.write_str(std::str::from_utf8(&self.0).unwrap_or("????"))
    }
}

/// One decoder: the codec type it handles, the implementation id to load, and a name for UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecoderEntry {
    pub codec: FourCc,
    pub implementation_id: String,
    pub display_name: String,
}

/// Checks an implementation id: 1 to 255 characters from `[A-Za-z0-9._-]`.
fn valid_implementation_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_IMPLEMENTATION_ID
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
}

/// Parses a `key = value` declaration into one decoder entry.
///
/// `CodecType` and `ClassImplementationID` are required. `DisplayName` is optional and
/// defaults to an empty string. Blank lines and lines starting with `#` are skipped.
pub fn parse_declaration(text: &str) -> Result<DecoderEntry, MediaExtError> {
    let mut codec: Option<FourCc> = None;
    let mut implementation_id: Option<String> = None;
    let mut display_name: Option<String> = None;

    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(MediaExtError::Malformed(line_no))?;
        let (key, value) = (key.trim(), value.trim());
        match key {
            "CodecType" => {
                if codec.is_some() {
                    return Err(MediaExtError::Malformed(line_no));
                }
                codec = Some(FourCc::parse(value).map_err(|_| MediaExtError::Malformed(line_no))?);
            }
            "ClassImplementationID" => {
                if implementation_id.is_some() {
                    return Err(MediaExtError::Malformed(line_no));
                }
                if !valid_implementation_id(value) {
                    return Err(MediaExtError::InvalidImplementationId);
                }
                implementation_id = Some(value.to_string());
            }
            "DisplayName" => {
                if display_name.is_some() {
                    return Err(MediaExtError::Malformed(line_no));
                }
                display_name = Some(value.to_string());
            }
            _ => return Err(MediaExtError::Malformed(line_no)),
        }
    }

    Ok(DecoderEntry {
        codec: codec.ok_or(MediaExtError::MissingKey("CodecType"))?,
        implementation_id: implementation_id.ok_or(MediaExtError::MissingKey("ClassImplementationID"))?,
        display_name: display_name.unwrap_or_default(),
    })
}

/// One decoder per codec type, kept sorted by codec so lookups are a binary search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecoderRegistry {
    entries: Vec<DecoderEntry>,
}

impl DecoderRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a decoder. A second decoder for the same codec type is refused and the first is kept.
    pub fn register(&mut self, entry: DecoderEntry) -> Result<(), MediaExtError> {
        match self.entries.binary_search_by(|e| e.codec.cmp(&entry.codec)) {
            Ok(_) => Err(MediaExtError::DuplicateCodec(entry.codec)),
            Err(position) => {
                self.entries.insert(position, entry);
                Ok(())
            }
        }
    }

    /// The decoder registered for a codec type, if any.
    pub fn lookup(&self, codec: FourCc) -> Option<&DecoderEntry> {
        self.entries
            .binary_search_by(|e| e.codec.cmp(&codec))
            .ok()
            .map(|position| &self.entries[position])
    }

    /// Removes the decoder for a codec type. Returns whether one was registered.
    pub fn unregister(&mut self, codec: FourCc) -> bool {
        match self.entries.binary_search_by(|e| e.codec.cmp(&codec)) {
            Ok(position) => {
                self.entries.remove(position);
                true
            }
            Err(_) => false,
        }
    }

    /// Every registered codec type, in ascending order.
    pub fn codecs(&self) -> Vec<FourCc> {
        self.entries.iter().map(|e| e.codec).collect()
    }

    /// Number of registered decoders.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no decoder is registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(codec: &str, id: &str) -> DecoderEntry {
        DecoderEntry {
            codec: FourCc::parse(codec).unwrap(),
            implementation_id: id.to_string(),
            display_name: String::new(),
        }
    }

    #[test]
    fn fourcc_value_is_big_endian_and_round_trips() {
        let avc1 = FourCc::parse("avc1").unwrap();
        assert_eq!(avc1.as_u32(), 0x6176_6331);
        assert_eq!(avc1.to_string(), "avc1");
        let hvc1 = FourCc::from_u32(0x6876_6331).unwrap();
        assert_eq!(hvc1.to_string(), "hvc1");
        assert_eq!(FourCc::from_u32(0x6176_0A31), None);
    }

    #[test]
    fn fourcc_rejects_wrong_length_and_non_printable_bytes() {
        assert_eq!(FourCc::parse("avc"), Err(MediaExtError::InvalidCodecType));
        assert_eq!(FourCc::parse("avc11"), Err(MediaExtError::InvalidCodecType));
        assert_eq!(FourCc::parse("av\u{7f}1"), Err(MediaExtError::InvalidCodecType));
        // "h" + "é" (two bytes) + "1" is four bytes, but holds a non-ASCII byte.
        assert_eq!(FourCc::parse("hé1"), Err(MediaExtError::InvalidCodecType));
    }

    #[test]
    fn registry_keeps_codecs_sorted_and_looks_them_up() {
        let mut reg = DecoderRegistry::new();
        assert!(reg.is_empty());
        reg.register(entry("hvc1", "org.example.hevc")).unwrap();
        reg.register(entry("avc1", "org.example.h264")).unwrap();
        assert_eq!(reg.len(), 2);
        let codecs: Vec<String> = reg.codecs().iter().map(|c| c.to_string()).collect();
        assert_eq!(codecs, vec!["avc1", "hvc1"]);
        let found = reg.lookup(FourCc::parse("hvc1").unwrap()).unwrap();
        assert_eq!(found.implementation_id, "org.example.hevc");
        assert!(reg.lookup(FourCc::parse("vp09").unwrap()).is_none());
    }

    #[test]
    fn registry_refuses_a_second_decoder_for_the_same_codec() {
        let mut reg = DecoderRegistry::new();
        reg.register(entry("avc1", "org.example.first")).unwrap();
        let err = reg.register(entry("avc1", "org.example.second")).unwrap_err();
        assert_eq!(err, MediaExtError::DuplicateCodec(FourCc::parse("avc1").unwrap()));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.lookup(FourCc::parse("avc1").unwrap()).unwrap().implementation_id, "org.example.first");
    }

    #[test]
    fn unregister_removes_once() {
        let mut reg = DecoderRegistry::new();
        reg.register(entry("avc1", "org.example.h264")).unwrap();
        let avc1 = FourCc::parse("avc1").unwrap();
        assert!(reg.unregister(avc1));
        assert!(!reg.unregister(avc1));
        assert!(reg.lookup(avc1).is_none());
        assert!(reg.is_empty());
    }

    #[test]
    fn declaration_parses_required_and_optional_keys() {
        let text = "# decoder\n\nCodecType = avc1\nClassImplementationID = org.example.h264-decoder\nDisplayName = Example H.264\n";
        let parsed = parse_declaration(text).unwrap();
        assert_eq!(parsed.codec.to_string(), "avc1");
        assert_eq!(parsed.implementation_id, "org.example.h264-decoder");
        assert_eq!(parsed.display_name, "Example H.264");

        let bare = parse_declaration("CodecType=hvc1\nClassImplementationID=org.example.hevc\n").unwrap();
        assert_eq!(bare.display_name, "");
    }

    #[test]
    fn declaration_reports_missing_and_bad_keys_by_line() {
        assert_eq!(
            parse_declaration("ClassImplementationID = org.example.x\n"),
            Err(MediaExtError::MissingKey("CodecType"))
        );
        assert_eq!(
            parse_declaration("CodecType = avc1\n\nColour = red\n"),
            Err(MediaExtError::Malformed(3))
        );
        assert_eq!(
            parse_declaration("CodecType = av\n"),
            Err(MediaExtError::Malformed(1))
        );
        assert_eq!(
            parse_declaration("CodecType = avc1\nClassImplementationID = bad id\n"),
            Err(MediaExtError::InvalidImplementationId)
        );
    }
}
