//! Plain text encodings for open and save: UTF-8, UTF-16 (either byte order), ISO Latin 1, Windows Latin 1 and Mac
//! Roman. `detect` reads a byte order mark and otherwise picks UTF-8 when the bytes are valid and Mac Roman or
//! Windows Latin 1 when not; `decode` and `encode` convert with an explicit choice, as the open panel's encoding
//! popup and Format > Text Encoding do. Pure std.
#![allow(dead_code)] // the open/save accessory and Text Encoding submenu call these next

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Latin1,
    Windows1252,
    MacRoman,
}

impl Encoding {
    pub const ALL: [Encoding; 6] =
        [Self::Utf8, Self::Utf16Le, Self::Utf16Be, Self::Latin1, Self::Windows1252, Self::MacRoman];

    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "Unicode (UTF-8)",
            Self::Utf16Le => "Unicode (UTF-16 little-endian)",
            Self::Utf16Be => "Unicode (UTF-16 big-endian)",
            Self::Latin1 => "Western (ISO Latin 1)",
            Self::Windows1252 => "Western (Windows Latin 1)",
            Self::MacRoman => "Western (Mac OS Roman)",
        }
    }

    /// The charset name used in HTML and mail headers.
    pub fn charset(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf16Le => "utf-16le",
            Self::Utf16Be => "utf-16be",
            Self::Latin1 => "iso-8859-1",
            Self::Windows1252 => "windows-1252",
            Self::MacRoman => "macintosh",
        }
    }

    pub fn from_charset(name: &str) -> Option<Encoding> {
        match name.trim().to_ascii_lowercase().as_str() {
            "utf-8" | "utf8" => Some(Self::Utf8),
            "utf-16le" | "utf-16" => Some(Self::Utf16Le),
            "utf-16be" => Some(Self::Utf16Be),
            "iso-8859-1" | "latin1" | "l1" => Some(Self::Latin1),
            "windows-1252" | "cp1252" => Some(Self::Windows1252),
            "macintosh" | "macroman" | "mac" | "x-mac-roman" => Some(Self::MacRoman),
            _ => None,
        }
    }
}

const MAC_HIGH: &str = "ÄÅÇÉÑÖÜáàâäãåçéèêëíìîïñóòôöõúùûü†°¢£§•¶ß®©™´¨≠ÆØ∞±≤≥¥µ∂∑∏π∫ªºΩæø¿¡¬√ƒ≈∆«»… ÀÃÕŒœ–—“”‘’÷◊ÿŸ⁄€‹›ﬁﬂ‡·‚„‰ÂÊÁËÈÍÎÏÌÓÔ\u{f8ff}ÒÚÛÙıˆ˜¯˘˙˚¸˝˛ˇ";

/// Windows Latin 1 for 0x80..=0x9F; `\0` marks the bytes it leaves undefined.
const WIN_HIGH: [char; 32] = [
    '€', '\0', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\0', 'Ž', '\0', '\0', '‘', '’', '“', '”', '•',
    '–', '—', '˜', '™', 'š', '›', 'œ', '\0', 'ž', 'Ÿ',
];

fn mac_char(b: u8) -> char {
    MAC_HIGH.chars().nth(usize::from(b - 0x80)).unwrap_or('\u{fffd}')
}

fn win_char(b: u8) -> char {
    match WIN_HIGH[usize::from(b - 0x80)] {
        '\0' => char::from(b),
        c => c,
    }
}

/// The encoding named by a byte order mark, and the length of the mark.
pub fn bom(bytes: &[u8]) -> Option<(Encoding, usize)> {
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        Some((Encoding::Utf8, 3))
    } else if bytes.starts_with(&[0xff, 0xfe]) {
        Some((Encoding::Utf16Le, 2))
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        Some((Encoding::Utf16Be, 2))
    } else {
        None
    }
}

/// Guesses the encoding of `bytes`: a byte order mark wins, then UTF-16 when most alternate bytes are zero, then
/// valid UTF-8, else Mac Roman when it holds bytes in 0x80..=0x9F that Windows Latin 1 would leave controls or odd, else
/// Windows Latin 1.
pub fn detect(bytes: &[u8]) -> Encoding {
    if let Some((e, _)) = bom(bytes) {
        return e;
    }
    if bytes.len() >= 4 && bytes.len() % 2 == 0 {
        let even_zero = bytes.iter().step_by(2).filter(|b| **b == 0).count();
        let odd_zero = bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
        let half = bytes.len() / 2;
        if odd_zero * 10 >= half * 8 {
            return Encoding::Utf16Le;
        }
        if even_zero * 10 >= half * 8 {
            return Encoding::Utf16Be;
        }
    }
    if std::str::from_utf8(bytes).is_ok() {
        return Encoding::Utf8;
    }
    if bytes.iter().any(|b| (0x80..=0x9f).contains(b) && WIN_HIGH[usize::from(b - 0x80)] == '\0') {
        return Encoding::MacRoman;
    }
    Encoding::Windows1252
}

/// Decodes `bytes` as `encoding`. A byte order mark that matches is skipped; invalid sequences become U+FFFD.
pub fn decode(bytes: &[u8], encoding: Encoding) -> String {
    let body = match bom(bytes) {
        Some((e, n)) if e == encoding => &bytes[n..],
        _ => bytes,
    };
    match encoding {
        Encoding::Utf8 => String::from_utf8_lossy(body).into_owned(),
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let units = body.chunks_exact(2).map(|p| {
                if encoding == Encoding::Utf16Le { u16::from_le_bytes([p[0], p[1]]) } else { u16::from_be_bytes([p[0], p[1]]) }
            });
            char::decode_utf16(units).map(|r| r.unwrap_or('\u{fffd}')).collect()
        }
        Encoding::Latin1 => body.iter().map(|b| char::from(*b)).collect(),
        Encoding::Windows1252 => body.iter().map(|b| if *b < 0x80 || *b >= 0xa0 { char::from(*b) } else { win_char(*b) }).collect(),
        Encoding::MacRoman => body.iter().map(|b| if *b < 0x80 { char::from(*b) } else { mac_char(*b) }).collect(),
    }
}

/// Encodes `text`. `Err` holds the characters the encoding cannot represent (each once, in order); the caller
/// offers to save lossy (`encode_lossy`) or to choose another encoding.
pub fn encode(text: &str, encoding: Encoding) -> Result<Vec<u8>, Vec<char>> {
    let mut lost: Vec<char> = vec![];
    let out = encode_with(text, encoding, &mut |c| {
        if !lost.contains(&c) {
            lost.push(c);
        }
        b'?'
    });
    if lost.is_empty() { Ok(out) } else { Err(lost) }
}

/// Encodes `text`, writing `?` for characters the encoding cannot represent.
pub fn encode_lossy(text: &str, encoding: Encoding) -> Vec<u8> {
    encode_with(text, encoding, &mut |_| b'?')
}

fn encode_with(text: &str, encoding: Encoding, missing: &mut dyn FnMut(char) -> u8) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        Encoding::Latin1 => text.chars().map(|c| u8::try_from(u32::from(c)).unwrap_or_else(|_| missing(c))).collect(),
        Encoding::Windows1252 => text
            .chars()
            .map(|c| match u8::try_from(u32::from(c)) {
                Ok(b) if !(0x80..0xa0).contains(&b) => b,
                _ => (0x80u8..0xa0).find(|b| WIN_HIGH[usize::from(b - 0x80)] == c).unwrap_or_else(|| missing(c)),
            })
            .collect(),
        Encoding::MacRoman => text
            .chars()
            .map(|c| {
                if u32::from(c) < 0x80 {
                    c as u8
                } else {
                    MAC_HIGH.chars().position(|m| m == c).map(|i| 0x80 + i as u8).unwrap_or_else(|| missing(c))
                }
            })
            .collect(),
    }
}

/// `encode`, with a byte order mark in front for the UTF-16 encodings (UTF-8 gets none).
pub fn encode_with_bom(text: &str, encoding: Encoding) -> Result<Vec<u8>, Vec<char>> {
    let mut out = match encoding {
        Encoding::Utf16Le => vec![0xff, 0xfe],
        Encoding::Utf16Be => vec![0xfe, 0xff],
        _ => vec![],
    };
    out.extend(encode(text, encoding)?);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_table_has_128_entries_and_known_points() {
        assert_eq!(MAC_HIGH.chars().count(), 128);
        assert_eq!(mac_char(0x80), 'Ä');
        assert_eq!(mac_char(0x8e), 'é');
        assert_eq!(mac_char(0xa5), '•');
        assert_eq!(mac_char(0xd2), '“');
        assert_eq!(mac_char(0xdb), '€');
        assert_eq!(mac_char(0xff), 'ˇ');
    }

    #[test]
    fn detection_order() {
        assert_eq!(detect(b"\xef\xbb\xbfhi"), Encoding::Utf8);
        assert_eq!(detect(b"\xff\xfeh\0i\0"), Encoding::Utf16Le);
        assert_eq!(detect("þetta er í lagi".as_bytes()), Encoding::Utf8);
        assert_eq!(detect(b"h\0i\0 \0y\0o\0u\0"), Encoding::Utf16Le);
        assert_eq!(detect(b"\0h\0i\0 \0y\0o\0u"), Encoding::Utf16Be);
        assert_eq!(detect(b"caf\xe9"), Encoding::Windows1252);
        assert_eq!(detect(b"a\x81b"), Encoding::MacRoman);
    }

    #[test]
    fn round_trips_in_each_encoding() {
        let text = "Þórður á tónleikum – “café” ö";
        for e in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            assert_eq!(decode(&encode(text, e).unwrap(), e), text, "{e:?}");
        }
        let latin = "Þórður á tónleikum café ö";
        assert_eq!(decode(&encode(latin, Encoding::Latin1).unwrap(), Encoding::Latin1), latin);
        let win = "Þórður – “café” €";
        assert_eq!(decode(&encode(win, Encoding::Windows1252).unwrap(), Encoding::Windows1252), win);
        let mac = "Þ is not here but é — “q” ✓";
        assert_eq!(encode(mac, Encoding::MacRoman), Err(vec!['Þ', '✓']));
        let mac = "café — “q” ö";
        assert_eq!(decode(&encode(mac, Encoding::MacRoman).unwrap(), Encoding::MacRoman), mac);
    }

    #[test]
    fn unrepresentable_characters_are_reported_or_replaced() {
        assert_eq!(encode("a€b", Encoding::Latin1), Err(vec!['€']));
        assert_eq!(encode_lossy("a€b€", Encoding::Latin1), b"a?b?");
        assert_eq!(encode_lossy("a€", Encoding::Windows1252), vec![b'a', 0x80]);
    }

    #[test]
    fn bom_is_skipped_on_decode_and_written_on_request() {
        assert_eq!(decode(b"\xef\xbb\xbfhi", Encoding::Utf8), "hi");
        assert_eq!(decode(b"\xff\xfeh\0i\0", Encoding::Utf16Le), "hi");
        assert_eq!(encode_with_bom("hi", Encoding::Utf16Be).unwrap(), vec![0xfe, 0xff, 0, b'h', 0, b'i']);
        assert_eq!(encode_with_bom("hi", Encoding::Utf8).unwrap(), b"hi");
    }

    #[test]
    fn charset_names() {
        for e in Encoding::ALL {
            assert_eq!(Encoding::from_charset(e.charset()), Some(e));
        }
        assert_eq!(Encoding::from_charset("KOI8"), None);
    }
}
