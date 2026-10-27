//! The text a reader shows for a whole message: the plain part of a multipart message (or the HTML part, converted),
//! with base64 and quoted-printable transfer encodings undone and ISO-8859-1 read as Latin-1. Headers are read with
//! [`crate::message::parse_headers`], so folded lines and case are handled the same way as in the list.

use crate::codec::{base64_decode, decode_charset, parameter};
use crate::html;
use crate::message::parse_headers;

/// The readable text of a raw message. Empty when the message has no text part.
pub fn readable(raw: &str) -> String {
    match text_part(raw, 0, &mut { MAX_PARTS }) {
        Some(Part { html: true, text }) => html::to_text(&text),
        Some(Part { html: false, text }) => text,
        None => String::new(),
    }
}

/// How deep multipart entities are followed. A message nested deeper is hostile; the deeper parts are not read.
pub(crate) const MAX_DEPTH: usize = 12;

/// How many entities of one message are visited, so a wide and deep tree cannot multiply the work.
pub(crate) const MAX_PARTS: usize = 2000;

/// Takes one entity from `budget`; false when the budget is spent.
pub(crate) fn take_part(budget: &mut usize) -> bool {
    if *budget == 0 {
        return false;
    }
    *budget -= 1;
    true
}

/// How many remote images the HTML part of a raw message would load, counted on the decoded HTML alone (not on the
/// headers, boundaries and encoded attachments around it). Zero when the message has no HTML part.
pub fn remote_images(raw: &str) -> usize {
    match html_part(raw, 0, &mut { MAX_PARTS }) {
        Some(text) => html::sanitize(&text, false).remote_images_blocked,
        None => 0,
    }
}

/// The decoded text of the first text/html part of an entity, nested multipart entities included.
fn html_part(raw: &str, depth: usize, budget: &mut usize) -> Option<String> {
    if depth > MAX_DEPTH || !take_part(budget) {
        return None;
    }
    let (head, body) = split_head(raw);
    let headers = parse_headers(head);
    let kind = content_type(&headers);
    if let Some(boundary) = kind.strip_prefix("multipart/").and(boundary(&headers)) {
        return split_parts(body, &boundary).into_iter().find_map(|p| html_part(p, depth + 1, budget));
    }
    (kind == "text/html").then(|| decode_body(&headers, body, param(&headers, "charset").as_deref()))
}

struct Part {
    html: bool,
    text: String,
}

/// A file carried by a message: a part with a file name or an attachment disposition, with its transfer encoding
/// undone. `mime` is the media type, `application/octet-stream` when the part names none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// Every file of a raw message, in order, nested multipart parts included. Text parts with no file name are the body
/// (see [`readable`]), not attachments.
pub fn attachments(raw: &str) -> Vec<Attachment> {
    let mut out = Vec::new();
    collect_attachments(raw, &mut out, 0, &mut { MAX_PARTS });
    out
}

fn collect_attachments(raw: &str, out: &mut Vec<Attachment>, depth: usize, budget: &mut usize) {
    if depth > MAX_DEPTH || !take_part(budget) {
        return;
    }
    let (head, body) = split_head(raw);
    let headers = parse_headers(head);
    let kind = content_type(&headers);
    if kind.starts_with("multipart/") {
        if let Some(boundary) = boundary(&headers) {
            for part in split_parts(body, &boundary) {
                collect_attachments(part, out, depth + 1, budget);
            }
        }
        return;
    }
    if !is_file_part(&headers) {
        return;
    }
    let name = file_name(&headers).unwrap_or_else(|| format!("attachment-{}", out.len() + 1));
    let mime = if kind.is_empty() { "application/octet-stream".to_string() } else { kind };
    out.push(Attachment { name, mime, bytes: part_bytes(&headers, body) });
}

/// True when a (non-multipart) part is a file: it has an attachment disposition or a file name.
pub(crate) fn is_file_part(headers: &[(String, String)]) -> bool {
    let disposition = headers.iter().find(|(n, _)| n == "content-disposition").map(|(_, v)| v.to_ascii_lowercase()).unwrap_or_default();
    disposition.trim_start().starts_with("attachment") || file_name(headers).is_some()
}

/// The file name of a part: the `filename` of its Content-Disposition, else the `name` of its Content-Type. Encoded
/// words (RFC 2047) and the RFC 2231 forms (`filename*=utf-8''...`, continuations) are decoded.
pub(crate) fn file_name(headers: &[(String, String)]) -> Option<String> {
    ["content-disposition", "content-type"]
        .iter()
        .find_map(|header| {
            let value = headers.iter().find(|(n, _)| n == header).map(|(_, v)| v.as_str())?;
            parameter(value, "filename").or_else(|| parameter(value, "name")).filter(|name| !name.trim().is_empty())
        })
        .map(|name| name.trim().to_string())
}

/// The part's bytes with its transfer encoding undone (no charset: attachments are kept as bytes).
fn part_bytes(headers: &[(String, String)], body: &str) -> Vec<u8> {
    let encoding = headers.iter().find(|(n, _)| n == "content-transfer-encoding").map(|(_, v)| v.trim().to_ascii_lowercase());
    match encoding.as_deref() {
        Some("base64") => base64_decode(body).unwrap_or_default(),
        Some("quoted-printable") => quoted_printable(body),
        _ => body.as_bytes().to_vec(),
    }
}

/// The best text part of one entity: a text/plain part, else a text/html part. Multipart entities are searched in
/// order, nested ones included.
fn text_part(raw: &str, depth: usize, budget: &mut usize) -> Option<Part> {
    if depth > MAX_DEPTH || !take_part(budget) {
        return None;
    }
    let (head, body) = split_head(raw);
    let headers = parse_headers(head);
    let kind = content_type(&headers);
    if let Some(boundary) = kind.strip_prefix("multipart/").and(boundary(&headers)) {
        let parts: Vec<Part> =
            split_parts(body, &boundary).into_iter().filter_map(|p| text_part(p, depth + 1, budget)).collect();
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
pub(crate) fn split_head(raw: &str) -> (&str, &str) {
    let crlf = raw.find("\r\n\r\n").map(|i| (i, 4));
    let lf = raw.find("\n\n").map(|i| (i, 2));
    match [crlf, lf].into_iter().flatten().min_by_key(|(i, _)| *i) {
        Some((i, len)) => (&raw[..i], &raw[i + len..]),
        None => (raw, ""),
    }
}

/// The media type in lower case (`text/plain`), or empty when the message names none.
pub(crate) fn content_type(headers: &[(String, String)]) -> String {
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

pub(crate) fn boundary(headers: &[(String, String)]) -> Option<String> {
    param(headers, "boundary").filter(|b| !b.is_empty())
}

/// The entities between the boundary lines. The preamble and epilogue are dropped.
pub(crate) fn split_parts<'a>(body: &'a str, boundary: &str) -> Vec<&'a str> {
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

/// Reads bytes in the declared charset (see [`decode_charset`]); a charset Mail does not know reads as UTF-8.
pub fn apply_charset(bytes: &[u8], charset: Option<&str>) -> String {
    charset.and_then(|label| decode_charset(bytes, label)).unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
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
    fn remote_images_are_counted_in_the_decoded_html_part() {
        let html = "<p>hi</p><img src=\"https://t.example/a.png\"><img src=\"http://t.example/b.png\">";
        let encoded = crate::codec::base64_encode(html.as_bytes());
        let raw = format!(
            "Content-Type: multipart/alternative; boundary=B\r\n\r\n--B\r\nContent-Type: text/plain\r\n\r\nhi\r\n--B\r\nContent-Type: text/html\r\nContent-Transfer-Encoding: base64\r\n\r\n{encoded}\r\n--B--\r\n"
        );
        assert_eq!(remote_images(&raw), 2);
        assert_eq!(remote_images("Subject: x\r\n\r\nno html\r\n"), 0);
    }

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
    fn attachments_are_the_named_parts_with_their_bytes() {
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=\"B\"\r\n\r\n",
            "--B\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nSee the file\r\n",
            "--B\r\nContent-Type: application/pdf; name=\"skjal.pdf\"\r\n",
            "Content-Disposition: attachment; filename=\"skjal.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\n",
            "aGk=\r\n",
            "--B\r\nContent-Type: text/plain\r\nContent-Disposition: attachment\r\n\r\nnotes\r\n--B--\r\n",
        );
        let files = attachments(raw);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0], Attachment { name: "skjal.pdf".into(), mime: "application/pdf".into(), bytes: b"hi".to_vec() });
        assert_eq!(files[1].name, "attachment-2");
        assert_eq!(files[1].bytes, b"notes".to_vec());
    }

    #[test]
    fn attachment_names_are_decoded_in_every_form() {
        let part = |disposition: &str| {
            format!("Content-Type: multipart/mixed; boundary=\"B\"\r\n\r\n--B\r\nContent-Type: application/pdf\r\nContent-Disposition: {disposition}\r\n\r\ndata\r\n--B--\r\n")
        };
        let name = |disposition: &str| attachments(&part(disposition)).remove(0).name;
        assert_eq!(name("attachment; filename=\"=?UTF-8?B?c2tqYWzDsC5wZGY=?=\""), "skjalð.pdf");
        assert_eq!(name("attachment; filename=\"=?iso-8859-1?q?sk=E1l.txt?=\""), "skál.txt");
        assert_eq!(name("attachment; filename*=utf-8''%E2%82%AC.txt"), "€.txt");
        assert_eq!(name("attachment; filename*0*=utf-8''ars; filename*1*=%C3%A1.pdf"), "arsá.pdf");
        assert_eq!(name("attachment; filename=\"a; b.txt\""), "a; b.txt");
    }

    #[test]
    fn declared_charsets_are_applied_to_bodies() {
        assert_eq!(apply_charset(&[0x80, 0xE1], Some("Windows-1252")), "\u{20ac}á");
        assert_eq!(apply_charset(&[0xA4], Some("iso-8859-15")), "€");
        assert_eq!(apply_charset("á".as_bytes(), Some("utf-8")), "á");
        assert_eq!(apply_charset("á".as_bytes(), Some("x-unknown")), "á", "an unknown charset reads as UTF-8");
        assert_eq!(apply_charset("á".as_bytes(), None), "á");
    }

    #[test]
    fn a_plain_message_has_no_attachments() {
        assert!(attachments("Content-Type: text/plain\r\n\r\nhello\r\n").is_empty());
    }

    /// A large `data:` PNG tag (not a real image), as senders put at the top of a welcome mail.
    fn big_png_tag() -> String {
        format!("<img src=\"data:image/png;base64,iVBORw0KGgo{}\" width=\"600\" alt=\"Logo\">", "QUFB".repeat(20_000))
    }

    /// `text` as a sender writes quoted-printable: "=3D" for each "=", lines of at most 75 characters joined by "=" CRLF.
    fn qp_encode(text: &str) -> String {
        let mut out = String::new();
        let mut line = 0;
        for c in text.chars() {
            let piece = match c {
                '=' => "=3D".to_string(),
                '\u{e1}' => "=C3=A1".to_string(),
                c => c.to_string(),
            };
            if line + piece.len() > 75 {
                out.push_str("=\r\n");
                line = 0;
            }
            out.push_str(&piece);
            line += piece.len();
        }
        out
    }

    #[test]
    fn an_html_body_that_starts_with_a_large_inline_image_shows_its_text() {
        let raw = format!("Content-Type: text/html; charset=utf-8\r\n\r\n{}<p>Welcome aboard</p>\r\n", big_png_tag());
        let text = readable(&raw);
        assert_eq!(text, "Welcome aboard");
        assert!(!text.contains("<img") && !text.contains("base64") && !text.contains("iVBOR"));
        // the same body as the only alternative of a message
        let alt = format!("Content-Type: multipart/alternative; boundary=B\r\n\r\n--B\r\nContent-Type: text/html\r\n\r\n{}<p>Welcome aboard</p>\r\n--B--\r\n", big_png_tag());
        assert_eq!(readable(&alt), "Welcome aboard");
    }

    #[test]
    fn a_quoted_printable_image_tag_split_by_soft_breaks_is_one_tag() {
        let encoded = qp_encode(&format!("{}<p>Hello \u{e1} there</p>", big_png_tag()));
        assert!(encoded.contains("=\r\n"));
        let raw = format!("Content-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n{encoded}\r\n");
        assert_eq!(readable(&raw), "Hello \u{e1} there");
        assert_eq!(remote_images(&raw), 0);
        // a break inside the "src" of a small tag too
        let small = "Content-Type: text/html\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n<img sr=\r\nc=3D\"data:image/png;base64,iVBO=\r\nRw0=\">=\r\n<p>x</p>\r\n";
        assert_eq!(readable(small), "x");
    }

    #[test]
    fn a_cid_image_of_a_related_message_is_not_printed() {
        let image = "iVBORw0KGgo".repeat(2000);
        let raw = format!(
            "Content-Type: multipart/related; boundary=R\r\n\r\n--R\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<img src=\"cid:logo@mail\"><p>Hello</p>\r\n--R\r\nContent-Type: image/png\r\nContent-ID: <logo@mail>\r\nContent-Transfer-Encoding: base64\r\n\r\n{image}\r\n--R--\r\n"
        );
        assert_eq!(readable(&raw), "Hello");
        assert_eq!(remote_images(&raw), 0);
        // the image resolves from its MIME part once the sanitizer is given the part as a data URI
        let html = html_part(&raw, 0, &mut { MAX_PARTS }).unwrap();
        let cids = vec![("logo@mail".to_string(), format!("data:image/png;base64,{image}"))];
        let shown = html::sanitize_with_parts(&html, false, &cids);
        assert!(shown.html.starts_with("<img src=\"data:image/png;base64,iVBORw0KGgo"));
        assert!(shown.html.ends_with("<p>Hello</p>"));
    }

    #[test]
    fn a_remote_image_beside_a_large_inline_one_stays_blocked() {
        let raw = format!("Content-Type: text/html\r\n\r\n{}<img src=\"https://t.example/px.gif\"><p>Hi</p>", big_png_tag());
        assert_eq!(remote_images(&raw), 1);
        assert_eq!(readable(&raw), "Hi");
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
