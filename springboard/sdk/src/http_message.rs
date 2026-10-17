//! Raw HTTP/1.1 messages for apps: `CFHTTPMessage` parity for a request or a response built or read by hand.
//!
//! [`HTTPMessage`] holds a start line, ordered header fields (a name may repeat, as `CFHTTPMessageCopyHeaderFieldValue`
//! returns the first and `CFHTTPMessageCopyAllHeaderFields` keeps all), and a body. [`HTTPMessage::serialize`] writes the
//! bytes that go on the wire, adding `Content-Length` for a body that has none. [`HTTPMessage::parse`] reads a complete
//! message: the body is cut to `Content-Length` when that header is present, and otherwise runs to the end of the input.
//!
//! Pure std, no socket. Chunked transfer coding is not decoded; a `Transfer-Encoding: chunked` body comes back as the
//! raw chunks, so an app that receives one must decode it itself. Lines end in CRLF, and a bare LF is accepted on read.

use std::fmt;

/// The HTTP version written when a message does not name one.
pub const DEFAULT_VERSION: &str = "HTTP/1.1";

/// Why a byte sequence is not a complete HTTP message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HTTPMessageError {
    /// The blank line that ends the header block is missing.
    MissingHeaderEnd,
    /// The start line is neither `METHOD target VERSION` nor `VERSION status reason`.
    BadStartLine,
    /// A header line has no colon or an empty or spaced name.
    BadHeader,
    /// `Content-Length` is not a decimal number, or several values disagree.
    BadContentLength,
    /// The body is shorter than `Content-Length`.
    Truncated,
}

impl fmt::Display for HTTPMessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            HTTPMessageError::MissingHeaderEnd => "the header block is not terminated",
            HTTPMessageError::BadStartLine => "the start line is malformed",
            HTTPMessageError::BadHeader => "a header line is malformed",
            HTTPMessageError::BadContentLength => "Content-Length is not a valid length",
            HTTPMessageError::Truncated => "the body is shorter than Content-Length",
        };
        f.write_str(text)
    }
}

/// The first line of a message: a request line or a status line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartLine {
    Request { method: String, target: String },
    Response { status: u16, reason: String },
}

/// A raw HTTP message (`CFHTTPMessageRef`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HTTPMessage {
    pub start: StartLine,
    pub version: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HTTPMessage {
    /// `CFHTTPMessageCreateRequest`: an empty request with a method and a request target.
    pub fn request(method: &str, target: &str) -> Self {
        HTTPMessage {
            start: StartLine::Request { method: method.to_string(), target: target.to_string() },
            version: DEFAULT_VERSION.to_string(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// `CFHTTPMessageCreateResponse`: an empty response with a status code and a reason phrase.
    pub fn response(status: u16, reason: &str) -> Self {
        HTTPMessage {
            start: StartLine::Response { status, reason: reason.to_string() },
            version: DEFAULT_VERSION.to_string(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// True for a request, false for a response.
    pub fn is_request(&self) -> bool {
        matches!(self.start, StartLine::Request { .. })
    }

    /// The request method, or `None` for a response.
    pub fn method(&self) -> Option<&str> {
        match &self.start {
            StartLine::Request { method, .. } => Some(method),
            StartLine::Response { .. } => None,
        }
    }

    /// The request target, or `None` for a response.
    pub fn target(&self) -> Option<&str> {
        match &self.start {
            StartLine::Request { target, .. } => Some(target),
            StartLine::Response { .. } => None,
        }
    }

    /// The status code, or `None` for a request.
    pub fn status_code(&self) -> Option<u16> {
        match &self.start {
            StartLine::Response { status, .. } => Some(*status),
            StartLine::Request { .. } => None,
        }
    }

    /// `CFHTTPMessageCopyHeaderFieldValue`: the first value of a header, matched without regard to case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// `CFHTTPMessageCopyAllHeaderFields` for one name: every value, in the order they were added.
    pub fn header_values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// All header fields in order.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// `CFHTTPMessageSetHeaderFieldValue`: replaces every value of the header with this one.
    pub fn set_header(&mut self, name: &str, value: &str) {
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.headers.push((name.to_string(), value.to_string()));
    }

    /// Appends a value without replacing the ones already there, as a repeated field.
    pub fn add_header(&mut self, name: &str, value: &str) {
        self.headers.push((name.to_string(), value.to_string()));
    }

    /// `CFHTTPMessageSetBody`.
    pub fn set_body(&mut self, body: &[u8]) {
        self.body = body.to_vec();
    }

    /// The body as it was set or read.
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// The bytes to send: the start line, the header fields, a blank line and the body. `Content-Length` is written for
    /// a non-empty body that has no such header.
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = String::new();
        match &self.start {
            StartLine::Request { method, target } => {
                out.push_str(&format!("{method} {target} {}\r\n", self.version));
            }
            StartLine::Response { status, reason } => {
                out.push_str(&format!("{} {status} {reason}\r\n", self.version));
            }
        }
        for (name, value) in &self.headers {
            out.push_str(&format!("{name}: {value}\r\n"));
        }
        if !self.body.is_empty() && self.header("Content-Length").is_none() {
            out.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
        }
        out.push_str("\r\n");
        let mut bytes = out.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }

    /// Reads one complete message from the start of `bytes`. Bytes after the body are ignored, so a caller reading a
    /// stream can pass the whole buffer and keep the rest for the next message.
    pub fn parse(bytes: &[u8]) -> Result<HTTPMessage, HTTPMessageError> {
        let (head_end, body_start) = find_head_end(bytes).ok_or(HTTPMessageError::MissingHeaderEnd)?;
        let head = std::str::from_utf8(&bytes[..head_end]).map_err(|_| HTTPMessageError::BadStartLine)?;
        let mut lines = head.lines();
        let first = lines.next().ok_or(HTTPMessageError::BadStartLine)?;
        let mut message = parse_start_line(first)?;

        for line in lines {
            let (name, value) = line.split_once(':').ok_or(HTTPMessageError::BadHeader)?;
            if name.is_empty() || name.contains(|c: char| c.is_ascii_whitespace()) {
                return Err(HTTPMessageError::BadHeader);
            }
            message.headers.push((name.to_string(), value.trim().to_string()));
        }

        let rest = &bytes[body_start..];
        message.body = match message.content_length()? {
            Some(length) => {
                if rest.len() < length {
                    return Err(HTTPMessageError::Truncated);
                }
                rest[..length].to_vec()
            }
            None => rest.to_vec(),
        };
        Ok(message)
    }

    /// The `Content-Length` value, `None` when absent. Repeated headers must all carry the same number.
    fn content_length(&self) -> Result<Option<usize>, HTTPMessageError> {
        let values = self.header_values("Content-Length");
        let mut found: Option<usize> = None;
        for value in values {
            let parsed = value.parse::<usize>().map_err(|_| HTTPMessageError::BadContentLength)?;
            match found {
                Some(previous) if previous != parsed => return Err(HTTPMessageError::BadContentLength),
                _ => found = Some(parsed),
            }
        }
        Ok(found)
    }
}

/// Splits `bytes` at the blank line that ends the header block. Returns the end of the head (without its line break)
/// and the index where the body starts.
fn find_head_end(bytes: &[u8]) -> Option<(usize, usize)> {
    let crlf = bytes.windows(4).position(|w| w == b"\r\n\r\n").map(|i| (i, i + 4));
    let lf = bytes.windows(2).position(|w| w == b"\n\n").map(|i| (i, i + 2));
    match (crlf, lf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn parse_start_line(line: &str) -> Result<HTTPMessage, HTTPMessageError> {
    if let Some(rest) = line.strip_prefix("HTTP/") {
        let mut parts = rest.splitn(3, ' ');
        let version_number = parts.next().ok_or(HTTPMessageError::BadStartLine)?;
        let status = parts
            .next()
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|s| (100..=999).contains(s))
            .ok_or(HTTPMessageError::BadStartLine)?;
        let reason = parts.next().unwrap_or("").to_string();
        if version_number.is_empty() {
            return Err(HTTPMessageError::BadStartLine);
        }
        let mut message = HTTPMessage::response(status, &reason);
        message.version = format!("HTTP/{version_number}");
        return Ok(message);
    }

    let mut parts = line.splitn(3, ' ');
    let method = parts.next().filter(|m| !m.is_empty()).ok_or(HTTPMessageError::BadStartLine)?;
    let target = parts.next().filter(|t| !t.is_empty()).ok_or(HTTPMessageError::BadStartLine)?;
    let version = parts.next().filter(|v| v.starts_with("HTTP/")).ok_or(HTTPMessageError::BadStartLine)?;
    let mut message = HTTPMessage::request(method, target);
    message.version = version.to_string();
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_serializes_with_its_start_line_and_headers() {
        let mut m = HTTPMessage::request("GET", "/index.html");
        m.set_header("Host", "example.com");
        m.set_header("Accept", "*/*");
        assert_eq!(
            m.serialize(),
            b"GET /index.html HTTP/1.1\r\nHost: example.com\r\nAccept: */*\r\n\r\n".to_vec()
        );
    }

    #[test]
    fn a_body_gets_a_content_length_header_when_none_is_set() {
        let mut m = HTTPMessage::request("POST", "/submit");
        m.set_body(b"hello");
        let bytes = m.serialize();
        assert!(String::from_utf8_lossy(&bytes).contains("Content-Length: 5\r\n\r\nhello"));
    }

    #[test]
    fn a_response_is_read_and_its_body_cut_to_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\n\r\nabcdEXTRA";
        let m = HTTPMessage::parse(raw).unwrap();
        assert!(!m.is_request());
        assert_eq!(m.status_code(), Some(200));
        assert_eq!(m.version, "HTTP/1.1");
        assert_eq!(m.body(), b"abcd");
    }

    #[test]
    fn header_lookup_ignores_case_and_set_replaces() {
        let mut m = HTTPMessage::response(200, "OK");
        m.add_header("Set-Cookie", "a=1");
        m.add_header("set-cookie", "b=2");
        assert_eq!(m.header("SET-COOKIE"), Some("a=1"));
        assert_eq!(m.header_values("Set-Cookie"), vec!["a=1", "b=2"]);
        m.set_header("Set-Cookie", "c=3");
        assert_eq!(m.header_values("set-cookie"), vec!["c=3"]);
    }

    #[test]
    fn a_request_round_trips_with_its_body() {
        let mut m = HTTPMessage::request("PUT", "/doc");
        m.set_header("Host", "h");
        m.set_body(b"payload bytes");
        let parsed = HTTPMessage::parse(&m.serialize()).unwrap();
        assert!(parsed.is_request());
        assert_eq!(parsed.method(), Some("PUT"));
        assert_eq!(parsed.target(), Some("/doc"));
        assert_eq!(parsed.header("content-length"), Some("13"));
        assert_eq!(parsed.body(), b"payload bytes");
    }

    #[test]
    fn a_bare_line_feed_is_accepted_on_read() {
        let m = HTTPMessage::parse(b"HTTP/1.0 404 Not Found\nServer: x\n\n").unwrap();
        assert_eq!(m.status_code(), Some(404));
        assert_eq!(m.header("Server"), Some("x"));
        assert!(m.body().is_empty());
    }

    #[test]
    fn a_short_body_is_truncated_and_a_missing_end_is_refused() {
        assert_eq!(
            HTTPMessage::parse(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc"),
            Err(HTTPMessageError::Truncated)
        );
        assert_eq!(
            HTTPMessage::parse(b"HTTP/1.1 200 OK\r\nServer: x\r\n"),
            Err(HTTPMessageError::MissingHeaderEnd)
        );
    }

    #[test]
    fn bad_content_length_and_bad_headers_are_refused() {
        assert_eq!(
            HTTPMessage::parse(b"HTTP/1.1 200 OK\r\nContent-Length: ten\r\n\r\n"),
            Err(HTTPMessageError::BadContentLength)
        );
        assert_eq!(
            HTTPMessage::parse(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\nx"),
            Err(HTTPMessageError::BadContentLength)
        );
        assert_eq!(
            HTTPMessage::parse(b"HTTP/1.1 200 OK\r\nNoColonHere\r\n\r\n"),
            Err(HTTPMessageError::BadHeader)
        );
    }

    #[test]
    fn a_malformed_start_line_is_refused() {
        assert_eq!(HTTPMessage::parse(b"HTTP/1.1 twohundred OK\r\n\r\n"), Err(HTTPMessageError::BadStartLine));
        assert_eq!(HTTPMessage::parse(b"GET /only-target\r\n\r\n"), Err(HTTPMessageError::BadStartLine));
    }
}
