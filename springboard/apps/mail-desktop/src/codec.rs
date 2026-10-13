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

/// Decodes UTF-8 base64 encoded words (`=?UTF-8?B?...?=`) in a header value. Other tokens pass through unchanged.
pub fn decode_words(value: &str) -> String {
    value
        .split(' ')
        .map(|token| {
            let inner = token.strip_prefix("=?UTF-8?B?").or_else(|| token.strip_prefix("=?utf-8?b?")).and_then(|s| s.strip_suffix("?="));
            match inner.and_then(base64_decode).and_then(|bytes| String::from_utf8(bytes).ok()) {
                Some(text) => text,
                None => token.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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
}
