//! `addMultipartData(_:withName:type:filename:)` parity: a `multipart/form-data` body for an upload.
//!
//! [`MultipartBody`] collects named parts (text fields and file parts) and writes the body bytes together with the
//! `Content-Type` value that carries the boundary. Parts keep the order they were added in. Names and file names are
//! escaped (`"`, CR and LF become percent escapes), so a name cannot end its header early. The boundary is derived from the
//! content and changed until it does not occur inside any part, so a part body can never end the multipart early.

/// One part of a multipart body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub content_type: String,
    pub filename: Option<String>,
    pub data: Vec<u8>,
}

const DEFAULT_TYPE: &str = "application/octet-stream";
const TEXT_TYPE: &str = "text/plain; charset=utf-8";

/// A `multipart/form-data` body under construction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MultipartBody {
    parts: Vec<Part>,
}

impl MultipartBody {
    pub fn new() -> Self {
        Self::default()
    }

    /// `addMultipartData(_:withName:type:filename:)`: a file part. An empty `content_type` means `application/octet-stream`.
    pub fn add_data(&mut self, data: &[u8], name: &str, content_type: &str, filename: Option<&str>) {
        let content_type = content_type.trim();
        self.parts.push(Part {
            name: name.to_string(),
            content_type: if content_type.is_empty() { DEFAULT_TYPE.to_string() } else { content_type.to_string() },
            filename: filename.map(str::to_string),
            data: data.to_vec(),
        });
    }

    /// A plain text field: `text/plain; charset=utf-8` and no file name.
    pub fn add_field(&mut self, name: &str, value: &str) {
        self.parts.push(Part { name: name.to_string(), content_type: TEXT_TYPE.to_string(), filename: None, data: value.as_bytes().to_vec() });
    }

    /// Appends an already built part (used when a request merges its parameters with its uploads).
    pub fn add_part(&mut self, part: Part) {
        self.parts.push(part);
    }

    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// The boundary: `bifrost-` and a 64-bit hash of the parts, with `x` added until no part body contains it.
    pub fn boundary(&self) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for p in &self.parts {
            for b in p.name.bytes().chain(p.data.iter().copied()) {
                hash ^= b as u64;
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        }
        let mut boundary = format!("bifrost-{hash:016x}");
        while self.parts.iter().any(|p| contains(&p.data, boundary.as_bytes())) {
            boundary.push('x');
        }
        boundary
    }

    /// The `Content-Type` header value for [`to_bytes`](Self::to_bytes).
    pub fn content_type(&self) -> String {
        format!("multipart/form-data; boundary={}", self.boundary())
    }

    /// The body: each part as `--boundary`, its headers, a blank line, its data and CRLF, then the closing `--boundary--`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let boundary = self.boundary();
        let mut out = Vec::new();
        for p in &self.parts {
            out.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"", escape(&p.name)).as_bytes());
            if let Some(f) = &p.filename {
                out.extend_from_slice(format!("; filename=\"{}\"", escape(f)).as_bytes());
            }
            out.extend_from_slice(format!("\r\nContent-Type: {}\r\n\r\n", p.content_type).as_bytes());
            out.extend_from_slice(&p.data);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        out
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// Percent-escapes the characters that could end a quoted header value early.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("%22"),
            '\r' => out.push_str("%0D"),
            '\n' => out.push_str("%0A"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_and_file_parts_are_written_in_order() {
        let mut body = MultipartBody::new();
        body.add_field("caption", "hi");
        body.add_data(b"PNG", "image", "image/png", Some("a.png"));
        let b = body.boundary();
        let text = String::from_utf8(body.to_bytes()).unwrap();
        let expected = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"caption\"\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nhi\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"a.png\"\r\nContent-Type: image/png\r\n\r\nPNG\r\n\
             --{b}--\r\n"
        );
        assert_eq!(text, expected);
        assert_eq!(body.content_type(), format!("multipart/form-data; boundary={b}"));
    }

    #[test]
    fn empty_type_defaults_to_octet_stream() {
        let mut body = MultipartBody::new();
        body.add_data(&[1, 2], "blob", "  ", None);
        assert_eq!(body.parts()[0].content_type, "application/octet-stream");
        assert!(body.parts()[0].filename.is_none());
    }

    #[test]
    fn quotes_and_line_breaks_in_names_are_escaped() {
        let mut body = MultipartBody::new();
        body.add_data(b"x", "we\"ird\r\nname", "text/plain", Some("say \"hi\".txt"));
        let text = String::from_utf8(body.to_bytes()).unwrap();
        assert!(text.contains("name=\"we%22ird%0D%0Aname\"; filename=\"say %22hi%22.txt\""));
    }

    #[test]
    fn boundary_never_occurs_inside_a_part() {
        // A part that spells out the boundary prefix many times over, with a closing delimiter of its own.
        let mut body = MultipartBody::new();
        body.add_data(b"--bifrost--bifrost-0000--bifrost-", "tricky", "", None);
        let b = body.boundary();
        assert!(b.starts_with("bifrost-"));
        assert!(!body.parts()[0].data.windows(b.len()).any(|w| w == b.as_bytes()));
        // Only the delimiters the builder wrote contain the boundary: one per part plus the closing one.
        let text = body.to_bytes();
        assert_eq!(text.windows(b.len()).filter(|w| *w == b.as_bytes()).count(), 2);
    }

    #[test]
    fn same_parts_give_the_same_boundary() {
        let mut a = MultipartBody::new();
        a.add_field("k", "v");
        let mut b = MultipartBody::new();
        b.add_field("k", "v");
        assert_eq!(a.boundary(), b.boundary());
        assert_eq!(a.to_bytes(), b.to_bytes());
    }

    #[test]
    fn empty_body_is_only_the_closing_delimiter() {
        let body = MultipartBody::new();
        assert!(body.is_empty());
        let b = body.boundary();
        assert_eq!(String::from_utf8(body.to_bytes()).unwrap(), format!("--{b}--\r\n"));
    }
}
