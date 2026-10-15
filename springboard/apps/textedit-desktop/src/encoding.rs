//! Plain text encodings. A plain text file is read by looking at its bytes (a byte order mark, valid UTF-8, or else
//! a single-byte Western text), and saved in the encoding the document carries, which Format > Text Encoding changes.
//! Pure: bytes in, text out, with no shell types.

use crate::rtf::cp1252;

/// The encodings Text Editor reads and writes for plain text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Encoding {
    #[default]
    Utf8,
    /// UTF-16 with a byte order mark, in either byte order
    Utf16Le,
    Utf16Be,
    /// ISO 8859-1
    Latin1,
    /// Windows code page 1252
    Windows1252,
    /// The classic Western single-byte set
    Roman,
    Ascii,
}

/// Every encoding, in the order the menus list them.
pub const ALL: [Encoding; 7] =
    [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1, Encoding::Windows1252, Encoding::Roman, Encoding::Ascii];

/// The upper half of the classic Western set (bytes 0x80 to 0xFF). The last byte has no Unicode character here.
const ROMAN_HIGH: [char; 128] = [
    'Ä', 'Å', 'Ç', 'É', 'Ñ', 'Ö', 'Ü', 'á', 'à', 'â', 'ä', 'ã', 'å', 'ç', 'é', 'è', //
    'ê', 'ë', 'í', 'ì', 'î', 'ï', 'ñ', 'ó', 'ò', 'ô', 'ö', 'õ', 'ú', 'ù', 'û', 'ü', //
    '†', '°', '¢', '£', '§', '•', '¶', 'ß', '®', '©', '™', '´', '¨', '≠', 'Æ', 'Ø', //
    '∞', '±', '≤', '≥', '¥', 'µ', '∂', '∑', '∏', 'π', '∫', 'ª', 'º', 'Ω', 'æ', 'ø', //
    '¿', '¡', '¬', '√', 'ƒ', '≈', '∆', '«', '»', '…', '\u{a0}', 'À', 'Ã', 'Õ', 'Œ', 'œ', //
    '–', '—', '“', '”', '‘', '’', '÷', '◊', 'ÿ', 'Ÿ', '⁄', '€', '‹', '›', 'ﬁ', 'ﬂ', //
    '‡', '·', '‚', '„', '‰', 'Â', 'Ê', 'Á', 'Ë', 'È', 'Í', 'Î', 'Ï', 'Ì', 'Ó', 'Ô', //
    '\u{fffd}', 'Ò', 'Ú', 'Û', 'Ù', 'ı', 'ˆ', '˜', '¯', '˘', '˙', '˚', '¸', '˝', '˛', 'ˇ',
];

impl Encoding {
    /// The name in menus.
    pub fn name(self) -> &'static str {
        match self {
            Encoding::Utf8 => "Unicode (UTF-8)",
            Encoding::Utf16Le => "Unicode (UTF-16 little-endian)",
            Encoding::Utf16Be => "Unicode (UTF-16 big-endian)",
            Encoding::Latin1 => "Western (ISO Latin 1)",
            Encoding::Windows1252 => "Western (Windows Latin 1)",
            Encoding::Roman => "Western (Classic Roman)",
            Encoding::Ascii => "ASCII",
        }
    }

    /// The stable identifier kept in the preferences.
    pub fn id(self) -> &'static str {
        match self {
            Encoding::Utf8 => "utf-8",
            Encoding::Utf16Le => "utf-16le",
            Encoding::Utf16Be => "utf-16be",
            Encoding::Latin1 => "latin1",
            Encoding::Windows1252 => "cp1252",
            Encoding::Roman => "roman",
            Encoding::Ascii => "ascii",
        }
    }

    pub fn from_id(id: &str) -> Option<Encoding> {
        ALL.iter().copied().find(|e| e.id() == id)
    }

    /// The position in `ALL`, which menu ids use.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn index(self) -> u8 {
        ALL.iter().position(|e| *e == self).unwrap_or(0) as u8
    }

    pub fn from_index(index: u8) -> Option<Encoding> {
        ALL.get(index as usize).copied()
    }

    /// The character a byte stands for in a single-byte encoding.
    fn byte_char(self, byte: u8) -> char {
        if byte < 0x80 {
            return byte as char;
        }
        match self {
            Encoding::Latin1 => byte as char,
            Encoding::Windows1252 => cp1252(byte),
            Encoding::Roman => ROMAN_HIGH[(byte - 0x80) as usize],
            // no high half
            _ => '\u{fffd}',
        }
    }

    /// The byte a character is in a single-byte encoding, if it has one.
    fn char_byte(self, ch: char) -> Option<u8> {
        if (ch as u32) < 0x80 {
            return Some(ch as u8);
        }
        if self == Encoding::Ascii || ch == '\u{fffd}' {
            return None;
        }
        (0x80..=0xffu8).find(|&b| self.byte_char(b) == ch)
    }
}

/// Decodes `bytes` in a known encoding. Bytes that mean nothing become the replacement character.
pub fn decode_as(bytes: &[u8], encoding: Encoding) -> String {
    match encoding {
        Encoding::Utf8 => String::from_utf8_lossy(bytes.strip_prefix(&[0xef, 0xbb, 0xbf][..]).unwrap_or(bytes)).into_owned(),
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let little = encoding == Encoding::Utf16Le;
            let bom: &[u8] = if little { &[0xff, 0xfe] } else { &[0xfe, 0xff] };
            let body = bytes.strip_prefix(bom).unwrap_or(bytes);
            let units = body.chunks_exact(2).map(|p| if little { u16::from_le_bytes([p[0], p[1]]) } else { u16::from_be_bytes([p[0], p[1]]) });
            char::decode_utf16(units).map(|r| r.unwrap_or('\u{fffd}')).collect()
        }
        single => bytes.iter().map(|&b| single.byte_char(b)).collect(),
    }
}

/// Finds the encoding of a plain text file and decodes it: a byte order mark decides, then a UTF-16 zero-byte pattern,
/// then valid UTF-8; anything else is Western single-byte text.
pub fn detect(bytes: &[u8]) -> (String, Encoding) {
    let encoding = if bytes.starts_with(&[0xff, 0xfe]) {
        Encoding::Utf16Le
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        Encoding::Utf16Be
    } else if let Some(utf16) = guess_utf16(bytes) {
        utf16
    } else if std::str::from_utf8(bytes).is_ok() {
        Encoding::Utf8
    } else {
        Encoding::Windows1252
    };
    (decode_as(bytes, encoding), encoding)
}

/// UTF-16 text without a byte order mark: mostly Latin text has a zero in every high byte.
fn guess_utf16(bytes: &[u8]) -> Option<Encoding> {
    if bytes.len() < 4 || bytes.len() % 2 != 0 {
        return None;
    }
    let pairs = bytes.len() / 2;
    let zero_high = bytes.chunks_exact(2).filter(|p| p[1] == 0 && p[0] != 0).count();
    let zero_low = bytes.chunks_exact(2).filter(|p| p[0] == 0 && p[1] != 0).count();
    if zero_high * 10 >= pairs * 9 {
        Some(Encoding::Utf16Le)
    } else if zero_low * 10 >= pairs * 9 {
        Some(Encoding::Utf16Be)
    } else {
        None
    }
}

/// Encodes text. Fails with the first character the encoding cannot hold, so saving never silently loses text.
pub fn encode(text: &str, encoding: Encoding) -> Result<Vec<u8>, char> {
    match encoding {
        Encoding::Utf8 => Ok(text.as_bytes().to_vec()),
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let little = encoding == Encoding::Utf16Le;
            let mut out: Vec<u8> = if little { vec![0xff, 0xfe] } else { vec![0xfe, 0xff] };
            for unit in text.encode_utf16() {
                out.extend_from_slice(&if little { unit.to_le_bytes() } else { unit.to_be_bytes() });
            }
            Ok(out)
        }
        single => text.chars().map(|ch| single.char_byte(ch).ok_or(ch)).collect(),
    }
}

/// Reads text that was decoded in the wrong encoding again as another one: the characters are turned back into the
/// bytes of `from` and decoded as `to`. `None` when the text cannot be turned back into bytes.
pub fn reinterpret(text: &str, from: Encoding, to: Encoding) -> Option<String> {
    let bytes = encode(text, from).ok()?;
    Some(decode_as(&bytes, to))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_utf8_stays_utf8_and_a_byte_order_mark_is_dropped() {
        assert_eq!(detect("héllo".as_bytes()), ("héllo".to_string(), Encoding::Utf8));
        assert_eq!(detect(&[0xef, 0xbb, 0xbf, b'a']), ("a".to_string(), Encoding::Utf8));
        assert_eq!(detect(b"").1, Encoding::Utf8);
    }

    #[test]
    fn utf16_is_found_by_its_mark_or_by_its_zero_bytes() {
        let le = encode("Halló", Encoding::Utf16Le).unwrap();
        assert_eq!(detect(&le), ("Halló".to_string(), Encoding::Utf16Le));
        let be = encode("Halló", Encoding::Utf16Be).unwrap();
        assert_eq!(detect(&be), ("Halló".to_string(), Encoding::Utf16Be));
        // no mark: ASCII text with a zero in every other byte
        assert_eq!(detect(b"h\0i\0 \0y\0o\0"), ("hi yo".to_string(), Encoding::Utf16Le));
        assert_eq!(detect(b"\0h\0i\0 \0y\0o"), ("hi yo".to_string(), Encoding::Utf16Be));
    }

    #[test]
    fn invalid_utf8_is_read_as_western_single_byte_text() {
        // 0xe9 alone is not UTF-8; in the Western sets it is e acute
        assert_eq!(detect(&[b'c', b'a', b'f', 0xe9]), ("café".to_string(), Encoding::Windows1252));
        // the Windows set has a euro sign at 0x80 where ISO Latin 1 has a control
        assert_eq!(decode_as(&[0x80], Encoding::Windows1252), "\u{20ac}");
        assert_eq!(decode_as(&[0x80], Encoding::Latin1), "\u{80}");
    }

    #[test]
    fn the_roman_set_maps_its_high_half() {
        assert_eq!(decode_as(&[0x8a, 0x8e, 0xa5, 0xc9], Encoding::Roman), "äé•…");
        assert_eq!(encode("äé•…", Encoding::Roman).unwrap(), vec![0x8a, 0x8e, 0xa5, 0xc9]);
    }

    #[test]
    fn every_encoding_round_trips_the_text_it_can_hold() {
        for e in ALL {
            assert_eq!(decode_as(&encode("plain text", e).unwrap(), e), "plain text", "{}", e.name());
        }
        for e in [Encoding::Latin1, Encoding::Windows1252, Encoding::Roman] {
            assert_eq!(decode_as(&encode("æ ö é", e).unwrap(), e), "æ ö é", "{}", e.name());
        }
        assert_eq!(decode_as(&encode("😀 ok", Encoding::Utf16Be).unwrap(), Encoding::Utf16Be), "😀 ok");
    }

    #[test]
    fn saving_refuses_a_character_the_encoding_cannot_hold() {
        assert_eq!(encode("naïve", Encoding::Ascii), Err('ï'));
        assert_eq!(encode("€", Encoding::Latin1), Err('€'));
        assert!(encode("€", Encoding::Windows1252).is_ok());
        assert_eq!(encode("\u{fffd}", Encoding::Roman), Err('\u{fffd}'), "the unmapped byte is not a character to save");
    }

    #[test]
    fn reinterpreting_repairs_text_read_in_the_wrong_set() {
        // a classic-set file opened as Windows text shows the wrong letters; read again as the right set it is fixed
        let wrong = decode_as(&[0x8e], Encoding::Windows1252);
        assert_eq!(reinterpret(&wrong, Encoding::Windows1252, Encoding::Roman).as_deref(), Some("é"));
        assert_eq!(reinterpret("→", Encoding::Windows1252, Encoding::Roman), None, "text that is not bytes of the set");
    }

    #[test]
    fn ids_and_indexes_name_each_encoding_once() {
        for (i, e) in ALL.iter().enumerate() {
            assert_eq!(Encoding::from_id(e.id()), Some(*e));
            assert_eq!(Encoding::from_index(e.index()), Some(*e));
            assert_eq!(e.index() as usize, i);
        }
        assert_eq!(Encoding::from_id("klingon"), None);
    }
}
