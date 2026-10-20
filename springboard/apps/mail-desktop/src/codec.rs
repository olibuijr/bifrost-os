//! Base64 and RFC 2047 encoded words, used by SMTP AUTH and by message headers.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { ALPHABET[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Decodes standard base64. Whitespace is skipped; padding ends the data. `None` on a foreign character.
pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for c in input.bytes() {
        if c == b'=' {
            break;
        }
        if c.is_ascii_whitespace() {
            continue;
        }
        let v: u32 = match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a') as u32 + 26,
            b'0'..=b'9' => (c - b'0') as u32 + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = ((acc << 6) | v) & 0xFF_FFFF;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1u32 << bits) - 1;
        }
    }
    Some(out)
}

/// Encodes a header value as an RFC 2047 word when it is not plain ASCII; plain text passes through.
pub fn encode_word(text: &str) -> String {
    if text.is_ascii() && !text.contains(['\r', '\n']) {
        text.to_string()
    } else {
        format!("=?UTF-8?B?{}?=", base64_encode(text.as_bytes()))
    }
}

/// Windows-1252 for the bytes 0x80 to 0x9F; the other bytes map to the code point of the same value (Latin-1).
const CP1252_HIGH: [char; 32] = [
    '\u{20ac}', '\u{81}', '\u{201a}', '\u{192}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{2c6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8d}', '\u{17d}', '\u{8f}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}', '\u{2dc}', '\u{2122}', '\u{161}', '\u{203a}', '\u{153}', '\u{9d}', '\u{17e}', '\u{178}',
];

pub(crate) fn windows_1252(b: u8) -> char {
    if (0x80..0xA0).contains(&b) {
        CP1252_HIGH[(b - 0x80) as usize]
    } else {
        b as char
    }
}

/// ISO-8859-15: Latin-1 with eight letters changed (the euro sign among them).
fn latin_9(b: u8) -> char {
    match b {
        0xA4 => '\u{20ac}',
        0xA6 => '\u{160}',
        0xA8 => '\u{161}',
        0xB4 => '\u{17d}',
        0xB8 => '\u{17e}',
        0xBC => '\u{152}',
        0xBD => '\u{153}',
        0xBE => '\u{178}',
        _ => b as char,
    }
}

/// Reads `bytes` in the charset named by a MIME label (any case; an RFC 2231 language suffix is ignored). UTF-8, US-ASCII,
/// ISO-8859-1, ISO-8859-15 and Windows-1252 are known. As on the web, ISO-8859-1 and US-ASCII read as Windows-1252, since
/// text labelled with them uses its letters; US-ASCII text that is valid UTF-8 reads as UTF-8. `None` for any other
/// charset.
pub fn decode_charset(bytes: &[u8], charset: &str) -> Option<String> {
    let label = charset.split('*').next().unwrap_or("").trim().trim_matches(['"', '\'']).to_ascii_lowercase();
    match label.as_str() {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" => Some(String::from_utf8_lossy(bytes).into_owned()),
        "us-ascii" | "ascii" | "ansi_x3.4-1968" if !bytes.is_ascii() && std::str::from_utf8(bytes).is_ok() => {
            Some(String::from_utf8_lossy(bytes).into_owned())
        }
        "us-ascii" | "ascii" | "ansi_x3.4-1968" | "iso-8859-1" | "iso8859-1" | "iso_8859-1" | "latin1" | "l1" | "windows-1252" | "windows1252" | "cp1252"
        | "x-cp1252" => Some(bytes.iter().map(|&b| windows_1252(b)).collect()),
        "iso-8859-15" | "iso8859-15" | "iso_8859-15" | "latin-9" | "latin9" | "l9" => Some(bytes.iter().map(|&b| latin_9(b)).collect()),
        _ => None,
    }
}

/// The bytes of a Q-encoded word's text (RFC 2047 section 4.2): `_` is a space and `=XX` is a byte. `None` for an `=`
/// that is not followed by two hexadecimal digits.
fn q_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'_' => out.push(b' '),
            b'=' => {
                let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
                if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    Some(out)
}

/// One encoded word read from the front of a text.
struct Word {
    /// How many bytes of the text the word takes, `=?` and `?=` included.
    len: usize,
    charset: String,
    bytes: Vec<u8>,
}

/// Reads the encoded word `=?charset?B?text?=` or `=?charset?Q?text?=` that starts `s`. `None` when `s` does not start
/// with a well-formed one.
fn parse_word(s: &str) -> Option<Word> {
    let rest = s.strip_prefix("=?")?;
    let (charset, rest) = rest.split_once('?')?;
    let (encoding, rest) = rest.split_once('?')?;
    let end = rest.find("?=")?;
    let text = &rest[..end];
    if charset.is_empty() || charset.contains(char::is_whitespace) || text.contains(['\r', '\n']) {
        return None;
    }
    let bytes = match encoding {
        "B" | "b" => base64_decode(text)?,
        "Q" | "q" => q_decode(text)?,
        _ => return None,
    };
    Some(Word { len: 2 + charset.len() + 1 + encoding.len() + 1 + end + 2, charset: charset.to_string(), bytes })
}

/// A piece of a header value: text as it stands, or encoded words in one charset that are read together (so a letter
/// split between two words comes out whole).
enum Piece {
    Text(String),
    Words { charset: String, bytes: Vec<u8>, raw: Vec<String> },
}

/// Decodes the RFC 2047 encoded words of a header value: B and Q encodings in any case, in UTF-8, US-ASCII, ISO-8859-1,
/// ISO-8859-15 and Windows-1252. Whitespace between two encoded words is dropped, and adjacent words of one charset are
/// decoded as one so a character split across them is whole. A word that is malformed, or in a charset Mail cannot read,
/// stays as written. The value is unfolded already (see `message::parse_headers`).
pub fn decode_words(value: &str) -> String {
    if !value.contains("=?") {
        return value.to_string();
    }
    let mut pieces: Vec<Piece> = Vec::new();
    let mut text = String::new();
    // the last piece is encoded words and only whitespace has been read since
    let mut after_word = false;
    let mut gap = String::new();
    let mut i = 0;
    while i < value.len() {
        let rest = &value[i..];
        if rest.starts_with("=?") {
            if let Some(word) = parse_word(rest) {
                let raw = rest[..word.len].to_string();
                i += word.len;
                if after_word {
                    // the whitespace between two encoded words is not part of the text
                    gap.clear();
                    if let Some(Piece::Words { charset, bytes, raw: raws }) = pieces.last_mut() {
                        if charset.eq_ignore_ascii_case(&word.charset) {
                            bytes.extend(word.bytes);
                            raws.push(raw);
                            continue;
                        }
                    }
                } else if !text.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                }
                pieces.push(Piece::Words { charset: word.charset, bytes: word.bytes, raw: vec![raw] });
                after_word = true;
                continue;
            }
        }
        let c = rest.chars().next().unwrap_or(' ');
        i += c.len_utf8();
        if after_word && c.is_whitespace() {
            gap.push(c);
            continue;
        }
        if after_word {
            // text follows the words: the whitespace read since belongs to it
            text.push_str(&std::mem::take(&mut gap));
            after_word = false;
        }
        text.push(c);
    }
    if after_word {
        text.push_str(&gap);
    }
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    let mut out = String::with_capacity(value.len());
    for piece in pieces {
        match piece {
            Piece::Text(t) => out.push_str(&t),
            Piece::Words { charset, bytes, raw } => match decode_charset(&bytes, &charset) {
                Some(decoded) => out.extend(decoded.chars().map(|c| if matches!(c, '\r' | '\n') { ' ' } else { c })),
                None => out.push_str(&raw.join(" ")),
            },
        }
    }
    out
}

/// The bytes a percent-encoded text stands for (`%E2%82%AC`); a `%` without two hexadecimal digits stays.
fn percent_bytes(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%').then(|| bytes.get(i + 1..i + 3)).flatten().and_then(|h| std::str::from_utf8(h).ok()).filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()));
        match hex {
            Some(h) => {
                out.push(u8::from_str_radix(h, 16).unwrap_or(b'%'));
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    out
}

/// The `key=value` parameters of a structured header value (`attachment; filename="a b.txt"`): the keys in lower case,
/// the values unquoted. A quoted value may hold `;` and backslash escapes. The text before the first `;` is skipped.
fn parameters(header: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = header.chars().peekable();
    // the media type or disposition type
    for c in chars.by_ref() {
        if c == ';' {
            break;
        }
    }
    loop {
        let mut key = String::new();
        let mut found_eq = false;
        while let Some(&c) = chars.peek() {
            chars.next();
            if c == '=' {
                found_eq = true;
                break;
            }
            if c == ';' {
                break;
            }
            key.push(c);
        }
        if !found_eq {
            if chars.peek().is_none() {
                break;
            }
            continue;
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut value = String::new();
        let quoted = chars.peek() == Some(&'"');
        if quoted {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => value.extend(chars.next()),
                    '"' => break,
                    c => value.push(c),
                }
            }
            // anything between the closing quote and the next `;` is dropped
            for c in chars.by_ref() {
                if c == ';' {
                    break;
                }
            }
        } else {
            for c in chars.by_ref() {
                if c == ';' {
                    break;
                }
                value.push(c);
            }
        }
        out.push((key.trim().to_ascii_lowercase(), if quoted { value } else { value.trim().to_string() }));
        if chars.peek().is_none() {
            break;
        }
    }
    out
}

/// The parameter `name` of a structured header value, decoded: RFC 2231 forms (`filename*=utf-8''%E2%82%AC.txt` and the
/// continuations `filename*0*=`, `filename*1=`) win over the plain form, whose RFC 2047 encoded words are decoded too.
/// `name` is in lower case.
pub fn parameter(header: &str, name: &str) -> Option<String> {
    let mut plain = None;
    // (index, extended, text)
    let mut parts: Vec<(usize, bool, String)> = Vec::new();
    for (key, value) in parameters(header) {
        let Some(rest) = key.strip_prefix(name) else { continue };
        match rest {
            "" => plain = Some(value),
            "*" => parts.push((0, true, value)),
            _ => {
                let Some(rest) = rest.strip_prefix('*') else { continue };
                let (number, extended) = match rest.strip_suffix('*') {
                    Some(number) => (number, true),
                    None => (rest, false),
                };
                if let Ok(index) = number.parse::<usize>() {
                    parts.push((index, extended, value));
                }
            }
        }
    }
    if parts.is_empty() {
        return plain.map(|v| decode_words(&v));
    }
    parts.sort_by_key(|(index, _, _)| *index);
    let mut charset = String::from("utf-8");
    let mut bytes = Vec::new();
    for (position, (_, extended, text)) in parts.iter().enumerate() {
        let mut text = text.as_str();
        if *extended {
            // the first extended part starts `charset'language'`
            if position == 0 {
                if let Some((label, rest)) = text.split_once('\'') {
                    if let Some((_, rest)) = rest.split_once('\'') {
                        if !label.is_empty() {
                            charset = label.to_string();
                        }
                        text = rest;
                    }
                }
            }
            bytes.extend(percent_bytes(text));
        } else {
            bytes.extend(text.bytes());
        }
    }
    Some(decode_charset(&bytes, &charset).unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_round_trips() {
        for text in ["", "a", "ab", "abc", "Hæ ð"] {
            assert_eq!(base64_decode(&base64_encode(text.as_bytes())).unwrap(), text.as_bytes());
        }
        assert_eq!(base64_decode("Zm9v\r\n").unwrap(), b"foo");
        assert!(base64_decode("Zm9v!").is_none());
    }

    #[test]
    fn encoded_words_round_trip() {
        assert_eq!(encode_word("Hello"), "Hello");
        let enc = encode_word("Fundur á morgun");
        assert!(enc.starts_with("=?UTF-8?B?"));
        assert_eq!(decode_words(&enc), "Fundur á morgun");
        assert_eq!(decode_words("plain words"), "plain words");
    }

    #[test]
    fn q_words_decode_in_any_case_with_underscores_as_spaces() {
        assert_eq!(decode_words("=?utf-8?q?Ap=C3=B3tekarinn?="), "Apótekarinn");
        assert_eq!(decode_words("=?UTF-8?Q?Fundur_=C3=A1_morgun?="), "Fundur á morgun");
        assert_eq!(decode_words("=?Utf-8?Q?=e2=82=ac_5?="), "€ 5");
    }

    #[test]
    fn b_words_decode_in_any_case() {
        assert_eq!(decode_words("=?utf-8?B?RnVuZHVyIMOhIG1vcmd1bg==?="), "Fundur á morgun");
        assert_eq!(decode_words("=?UTF-8?b?w4FzcsO6bg==?="), "Ásrún");
        assert_eq!(decode_words("=?UTF-8?B?w4FzcsO6bg==?= <ana@example.com>"), "Ásrún <ana@example.com>");
    }

    #[test]
    fn other_charsets_decode() {
        assert_eq!(decode_words("=?ISO-8859-1?Q?Su=F0ur_=E1?="), "Suður á");
        assert_eq!(decode_words("=?iso-8859-1?B?U3XwdXI=?="), "Suður");
        assert_eq!(decode_words("=?ISO-8859-15?Q?=A41=A4?="), "€1€");
        assert_eq!(decode_words("=?windows-1252?Q?=93quoted=94_=80?="), "\u{201c}quoted\u{201d} \u{20ac}");
        assert_eq!(decode_words("=?us-ascii?Q?plain_text?="), "plain text");
        assert_eq!(decode_words("=?utf-8*is?Q?=C3=A1?="), "á", "a language suffix is ignored");
    }

    #[test]
    fn adjacent_words_join_without_the_whitespace_between_them() {
        assert_eq!(decode_words("=?utf-8?q?Hello?= =?utf-8?q?World?="), "HelloWorld");
        assert_eq!(decode_words("=?utf-8?q?Hello_?=   \t=?utf-8?q?World?="), "Hello World");
        assert_eq!(decode_words("=?utf-8?q?a?==?utf-8?q?b?="), "ab");
        // text between words keeps its whitespace
        assert_eq!(decode_words("=?utf-8?q?a?= and =?utf-8?q?b?="), "a and b");
        assert_eq!(decode_words("Re: =?utf-8?q?a?= =?utf-8?q?b?= end"), "Re: ab end");
    }

    #[test]
    fn a_letter_split_between_two_words_comes_out_whole() {
        // the two bytes of "á" in different words
        assert_eq!(decode_words("=?utf-8?q?=C3?= =?utf-8?q?=A1?="), "á");
        assert_eq!(decode_words("=?utf-8?B?w6E=?= =?ISO-8859-1?Q?=E1?="), "áá", "words in another charset are read apart");
    }

    #[test]
    fn a_word_inside_quotes_or_a_phrase_decodes_in_place() {
        assert_eq!(decode_words("\"=?utf-8?q?Ap=C3=B3tekarinn?=\" <apotek@example.com>"), "\"Apótekarinn\" <apotek@example.com>");
        assert_eq!(decode_words("x=?utf-8?q?y?=z"), "xyz");
    }

    #[test]
    fn malformed_words_stay_as_written() {
        for bad in [
            "=?utf-8?B?!!!?=",
            "=?utf-8?Q?bad=Zq?=",
            "=?utf-8?X?abc?=",
            "=?utf-8?B?abc",
            "=?utf-8?B",
            "=??Q?abc?=",
            "=?",
            "?= =?",
        ] {
            assert_eq!(decode_words(bad), bad);
        }
        // a charset Mail cannot read keeps its words
        assert_eq!(decode_words("=?koi8-r?Q?=D0?= =?koi8-r?Q?=D1?="), "=?koi8-r?Q?=D0?= =?koi8-r?Q?=D1?=");
        // the good words around a bad one still decode
        assert_eq!(decode_words("=?utf-8?q?ok?= =?utf-8?B?!!?= =?utf-8?q?ok?="), "ok =?utf-8?B?!!?= ok");
    }

    #[test]
    fn a_decoded_line_break_becomes_a_space() {
        assert_eq!(decode_words("=?utf-8?q?a=0D=0Ab?="), "a  b");
    }

    #[test]
    fn charsets_are_read_by_label() {
        assert_eq!(decode_charset(&[0x53, 0xF0], "ISO-8859-1").as_deref(), Some("Sð"));
        assert_eq!(decode_charset(&[0x80], "latin1").as_deref(), Some("\u{20ac}"), "as on the web, Latin-1 is Windows-1252");
        assert_eq!(decode_charset(&[0xA4], "iso-8859-15").as_deref(), Some("€"));
        assert_eq!(decode_charset("ð".as_bytes(), "\"utf-8\"").as_deref(), Some("ð"));
        assert_eq!(decode_charset("á".as_bytes(), "us-ascii").as_deref(), Some("á"), "UTF-8 under an ASCII label reads as UTF-8");
        assert_eq!(decode_charset(&[0xE1], "us-ascii").as_deref(), Some("á"));
        assert_eq!(decode_charset(b"x", "shift_jis"), None);
    }

    #[test]
    fn parameters_are_split_outside_quotes() {
        let header = "attachment; filename=\"a; b.txt\"; size=12 ; note=\"say \\\"hi\\\"\"";
        assert_eq!(parameter(header, "filename").as_deref(), Some("a; b.txt"));
        assert_eq!(parameter(header, "size").as_deref(), Some("12"));
        assert_eq!(parameter(header, "note").as_deref(), Some("say \"hi\""));
        assert_eq!(parameter(header, "missing"), None);
        assert_eq!(parameter("attachment", "filename"), None);
    }

    #[test]
    fn file_names_may_be_encoded_words() {
        assert_eq!(parameter("attachment; filename=\"=?UTF-8?B?c2tqYWzDsC5wZGY=?=\"", "filename").as_deref(), Some("skjalð.pdf"));
        assert_eq!(parameter("attachment; filename==?ISO-8859-1?Q?sk=E1l.txt?=", "filename").as_deref(), Some("skál.txt"));
    }

    #[test]
    fn rfc_2231_file_names_decode() {
        assert_eq!(parameter("attachment; filename*=utf-8''%E2%82%AC%20rates.txt", "filename").as_deref(), Some("€ rates.txt"));
        assert_eq!(parameter("attachment; filename*=iso-8859-1'is'Su%F0ur.txt", "filename").as_deref(), Some("Suður.txt"));
        // the extended form wins over the plain one
        assert_eq!(parameter("attachment; filename=\"fallback.txt\"; filename*=utf-8''%C3%A1.txt", "filename").as_deref(), Some("á.txt"));
        // continuations, in any order, some extended and some not
        let split = "attachment; filename*1*=%C3%A1rsskýrsla.pdf; filename*0*=utf-8''Fyrir_";
        assert_eq!(parameter(split, "filename").as_deref(), Some("Fyrir_ársskýrsla.pdf"));
        let plain = "attachment; filename*0=\"long name \"; filename*1=\"in parts.txt\"";
        assert_eq!(parameter(plain, "filename").as_deref(), Some("long name in parts.txt"));
    }
}
