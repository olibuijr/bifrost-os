//! `b64encode` and `b64decode`: base64 encoding and decoding of a file or standard input, written to standard output
//! or to an output file. Options: `-r` writes or reads raw base64 with no line wrapping. Without `-r`, encoded output
//! wraps at 76 columns. Gap: the reference tools' `begin-base64` header and their `-w` width option are not produced
//! or read, and decoding does not check a header.

pub const ENCODE_USAGE: &str = "usage: b64encode [-r] [input [output]]";
pub const DECODE_USAGE: &str = "usage: b64decode [-r] [input [output]]";

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const WRAP: usize = 76;

/// Encodes bytes as standard base64. With `wrap`, lines are broken at 76 columns and end with a newline.
pub fn encode(data: &[u8], wrap: bool) -> String {
    let mut raw = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let chars = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        for (i, c) in chars.iter().enumerate() {
            // Positions past the input's end are padding.
            if i <= chunk.len() {
                raw.push(ALPHABET[*c as usize] as char);
            } else {
                raw.push('=');
            }
        }
    }
    if !wrap {
        return raw;
    }
    let mut out = String::with_capacity(raw.len() + raw.len() / WRAP + 1);
    for line in raw.as_bytes().chunks(WRAP) {
        out.push_str(std::str::from_utf8(line).unwrap_or(""));
        out.push('\n');
    }
    out
}

/// Decodes base64 text. White space and line breaks are ignored. Padding is optional on the last group.
pub fn decode(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut seen_pad = false;
    for c in text.bytes() {
        if c.is_ascii_whitespace() {
            continue;
        }
        if c == b'=' {
            seen_pad = true;
            continue;
        }
        if seen_pad {
            return Err("data after base64 padding".into());
        }
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            other => return Err(format!("invalid base64 character '{}'", other as char)),
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    if bits >= 6 {
        return Err("truncated base64 group".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_rfc4648_vectors() {
        assert_eq!(encode(b"", false), "");
        assert_eq!(encode(b"f", false), "Zg==");
        assert_eq!(encode(b"fo", false), "Zm8=");
        assert_eq!(encode(b"foo", false), "Zm9v");
        assert_eq!(encode(b"foobar", false), "Zm9vYmFy");
    }

    #[test]
    fn decodes_the_rfc4648_vectors_with_or_without_padding() {
        assert_eq!(decode("Zg==").unwrap(), b"f");
        assert_eq!(decode("Zm8=").unwrap(), b"fo");
        assert_eq!(decode("Zm9vYmFy").unwrap(), b"foobar");
        assert_eq!(decode("Zm8").unwrap(), b"fo");
    }

    #[test]
    fn decode_ignores_line_breaks_and_rejects_bad_input() {
        assert_eq!(decode("Zm9v\nYmFy\n").unwrap(), b"foobar");
        assert!(decode("Zm9!").unwrap_err().contains("invalid"));
        assert!(decode("Zg==Zg").unwrap_err().contains("padding"));
        assert!(decode("Z").unwrap_err().contains("truncated"));
    }

    #[test]
    fn wrapped_output_breaks_at_76_columns_and_round_trips() {
        let data: Vec<u8> = (0u8..=255).cycle().take(300).collect();
        let text = encode(&data, true);
        assert!(text.lines().all(|l| l.len() <= WRAP));
        assert!(text.ends_with('\n'));
        assert_eq!(decode(&text).unwrap(), data);
    }

    #[test]
    fn binary_data_round_trips() {
        let data = [0u8, 255, 16, 128, 1];
        assert_eq!(decode(&encode(&data, false)).unwrap(), data);
    }
}
