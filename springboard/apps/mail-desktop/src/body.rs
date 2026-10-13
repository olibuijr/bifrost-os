//! The text a reader shows for a whole message: the plain part of a multipart message (or the HTML part, converted),
//! with base64 and quoted-printable transfer encodings undone and ISO-8859-1 read as Latin-1. Headers are read with
//! [`crate::message::parse_headers`], so folded lines and case are handled the same way as in the list.

use crate::codec::base64_decode;
use crate::html;
use crate::message::parse_headers;

/// The readable text of a raw message. Empty when the message has no text part.
pub fn readable(raw: &str) -> String {
    match text_part(raw) {
        Some(Part { html: true, text }) => html::to_text(&text),
        Some(Part { html: false, text }) => text,
        None => String::new(),
    }
}

struct Part {
    html: bool,
    text: String,
}

/// The best text part of one entity: a text/plain part, else a text/html part. Multipart entities are searched in
/// order, nested ones included.
fn text_part(raw: &str) -> Option<Part> {
    let (head, body) = split_head(raw);
    let headers = parse_headers(head);
    let kind = content_type(&headers);
    if let Some(boundary) = kind.strip_prefix("multipart/").and(boundary(&headers)) {
        let parts: Vec<Part> = split_parts(body, &boundary).into_iter().filter_map(text_part).collect();
        let plain = parts.iter().position(|p| !p.html);
        return match plain {
            Some(i) => parts.into_iter().nth(i),
            None => parts.into_iter().next(),
        };
    }
    let charset = param(&headers, "charset");
    let text = decode_body(&headers, body, charset.as_deref());
    match kind.as_str() {
        "text/html" => Some(Part { html: true, text }),
        "text/plain" | "" => Some(Part { html: false, text }),
        _ => None,
    }
}

/// The header block and the body. The split is the first empty line; a message with no empty line is all header.
fn split_head(raw: &str) -> (&str, &str) {
    let crlf = raw.find("\r\n\r\n").map(|i| (i, 4));
    let lf = raw.find("\n\n").map(|i| (i, 2));
    match [crlf, lf].into_iter().flatten().min_by_key(|(i, _)| *i) {
        Some((i, len)) => (&raw[..i], &raw[i + len..]),
        None => (raw, ""),
    }
}

/// The media type in lower case (`text/plain`), or empty when the message names none.
fn content_type(headers: &[(String, String)]) -> String {
    let value = headers.iter().find(|(n, _)| n == "content-type").map(|(_, v)| v.as_str()).unwrap_or("");
    value.split(';').next().unwrap_or("").trim().to_ascii_lowercase()
}

/// The `name=value` parameter of the Content-Type header, unquoted, in its own case.
fn param(headers: &[(String, String)], name: &str) -> Option<String> {
    let value = headers.iter().find(|(n, _)| n == "content-type").map(|(_, v)| v.as_str())?;
    value.split(';').skip(1).find_map(|part| {
        let (key, val) = part.split_once('=')?;
        (key.trim().eq_ignore_ascii_case(name)).then(|| val.trim().trim_matches('"').to_string())
    })
}

fn boundary(headers: &[(String, String)]) -> Option<String> {
    param(headers, "boundary").filter(|b| !b.is_empty())
}

/// The entities between the boundary lines. The preamble and epilogue are dropped.
fn split_parts<'a>(body: &'a str, boundary: &str) -> Vec<&'a str> {
    let delimiter = format!("--{boundary}");
    let mut parts = Vec::new();
    let mut rest = body;
    let Some(first) = rest.find(&delimiter) else { return parts };
    rest = &rest[first + delimiter.len()..];
    loop {
        // the boundary line ends here; a closing delimiter ends with "--"
        if rest.starts_with("--") {
            break;
        }
        let Some(eol) = rest.find('\n') else { break };
        rest = &rest[eol + 1..];
        match rest.find(&format!("\n{delimiter}")) {
            Some(end) => {
                parts.push(rest[..end].trim_end_matches('\r'));
                rest = &rest[end + 1 + delimiter.len()..];
            }
            None => break,
        }
    }
    parts
}

/// The body as text: transfer encoding undone, then the charset applied to the bytes.
fn decode_body(headers: &[(String, String)], body: &str, charset: Option<&str>) -> String {
    let encoding = headers.iter().find(|(n, _)| n == "content-transfer-encoding").map(|(_, v)| v.trim().to_ascii_lowercase());
    let bytes: Vec<u8> = match encoding.as_deref() {
        Some("base64") => match base64_decode(body) {
            Some(bytes) => bytes,
            None => return body.to_string(),
        },
        Some("quoted-printable") => quoted_printable(body),
        _ => return body.to_string(),
    };
    apply_charset(&bytes, charset)
}

/// Reads bytes in the declared charset: ISO-8859-1 and Windows-1252 map one byte to one char, anything else is UTF-8.
pub fn apply_charset(bytes: &[u8], charset: Option<&str>) -> String {
    match charset.map(str::to_ascii_lowercase).as_deref() {
        Some("iso-8859-1") | Some("latin1") | Some("windows-1252") | Some("iso-8859-15") => bytes.iter().map(|&b| b as char).collect(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Quoted-printable (RFC 2045 section 6.7): `=XX` is a byte, `=` at a line end is a soft break, the rest is kept.
pub fn quoted_printable(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'=' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let rest = &bytes[i + 1..];
        if rest.starts_with(b"\r\n") {
            i += 3;
        } else if rest.first() == Some(&b'\n') {
            i += 2;
        } else if rest.len() >= 2 && rest[0].is_ascii_hexdigit() && rest[1].is_ascii_hexdigit() {
            let hex = std::str::from_utf8(&rest[..2]).unwrap_or("00");
            out.push(u8::from_str_radix(hex, 16).unwrap_or(0));
            i += 3;
        } else {
            out.push(b'=');
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_message_is_its_body() {
        assert_eq!(readable("Subject: x\r\n\r\nHello there\r\n"), "Hello there\r\n");
        assert_eq!(readable("Subject: x\n\nlf body\n"), "lf body\n");
        assert_eq!(readable("Subject: x\r\n"), "");
    }

    #[test]
    fn base64_bodies_are_decoded_as_utf8() {
        let raw = "Content-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\nw6FyYcOw\r\n";
        assert_eq!(readable(raw), "árað");
    }

    #[test]
    fn iso_8859_1_bytes_read_as_latin1() {
        let raw = "Content-Type: text/plain; charset=\"iso-8859-1\"\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nSu=F0ur =E1\r\n";
        assert_eq!(readable(raw), "Suður á\r\n");
    }

    #[test]
    fn quoted_printable_soft_breaks_and_escapes() {
        assert_eq!(quoted_printable("long=\r\nline =C3=A1"), b"longline \xc3\xa1".to_vec());
        assert_eq!(quoted_printable("a=3Db"), b"a=b".to_vec());
        assert_eq!(quoted_printable("100%=x"), b"100%=x".to_vec());
    }

    #[test]
    fn multipart_prefers_the_plain_part() {
        let raw = concat!(
            "Content-Type: multipart/alternative; boundary=\"XX\"\r\n\r\n",
            "preamble\r\n",
            "--XX\r\nContent-Type: text/html\r\n\r\n<p>Hi <b>Ana</b></p>\r\n",
            "--XX\r\nContent-Type: text/plain\r\n\r\nHi Ana\r\n",
            "--XX--\r\n",
        );
        assert_eq!(readable(raw), "Hi Ana");
    }

    #[test]
    fn an_html_only_message_is_converted_to_text() {
        let raw = "Content-Type: text/html; charset=utf-8\r\n\r\n<html><script>x()</script><p>Salut</p></html>";
        assert!(readable(raw).contains("Salut"));
        assert!(!readable(raw).contains("x()"));
    }

    #[test]
    fn nested_multipart_is_searched() {
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=outer\r\n\r\n",
            "--outer\r\nContent-Type: multipart/alternative; boundary=inner\r\n\r\n",
            "--inner\r\nContent-Type: text/plain\r\n\r\nNested text\r\n--inner--\r\n",
            "--outer\r\nContent-Type: application/pdf\r\n\r\nbinary\r\n--outer--\r\n",
        );
        assert_eq!(readable(raw), "Nested text");
    }
}
