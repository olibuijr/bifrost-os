//! SMTP (RFC 5321) on the wire: reply parsing, the commands this app sends, AUTH PLAIN (RFC 4616) and the DATA
//! body with dot-stuffing. Nothing here opens a socket.

use crate::codec::base64_encode;

/// One reply line: `250-SIZE 35882577` (more follows) or `250 OK` (last line).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyLine {
    pub code: u16,
    /// True when another line of the same reply follows (`250-`).
    pub more: bool,
    pub text: String,
}

/// Parses one reply line (CRLF removed). `None` when it does not start with a three-digit code.
pub fn parse_reply_line(line: &str) -> Option<ReplyLine> {
    let bytes = line.as_bytes();
    if bytes.len() < 3 || !bytes[..3].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let code: u16 = line[..3].parse().ok()?;
    let (more, text) = match bytes.get(3) {
        None => (false, ""),
        Some(b' ') => (false, &line[4..]),
        Some(b'-') => (true, &line[4..]),
        _ => return None,
    };
    Some(ReplyLine { code, more, text: text.to_string() })
}

/// Collects the lines of a multi-line reply up to the final one. Returns the code and all text lines.
pub fn collect_reply(lines: &[&str]) -> Option<(u16, Vec<String>)> {
    let mut code = None;
    let mut texts = Vec::new();
    for line in lines {
        let parsed = parse_reply_line(line)?;
        if let Some(c) = code {
            if c != parsed.code {
                return None;
            }
        }
        code = Some(parsed.code);
        texts.push(parsed.text);
        if !parsed.more {
            return Some((parsed.code, texts));
        }
    }
    None
}

/// Reply classes (RFC 5321 section 4.2.1): 2xx done, 3xx more input, 4xx and 5xx failures.
pub fn is_positive(code: u16) -> bool {
    (200..400).contains(&code)
}

pub fn ehlo(domain: &str) -> String {
    format!("EHLO {domain}\r\n")
}

/// `MAIL FROM:<addr>`; the address must already be checked with [`bifrost_app::controls::internet_accounts::valid_address`].
pub fn mail_from(addr: &str) -> String {
    format!("MAIL FROM:<{addr}>\r\n")
}

pub fn rcpt_to(addr: &str) -> String {
    format!("RCPT TO:<{addr}>\r\n")
}

pub fn data() -> String {
    "DATA\r\n".to_string()
}

pub fn quit() -> String {
    "QUIT\r\n".to_string()
}

/// `AUTH PLAIN` with the initial response in one line: base64 of NUL user NUL password.
pub fn auth_plain(user: &str, password: &str) -> String {
    let initial = format!("\0{user}\0{password}");
    format!("AUTH PLAIN {}\r\n", base64_encode(initial.as_bytes()))
}

/// Turns a message into the DATA payload: CRLF line endings, a leading dot doubled, and the closing `.` line.
pub fn data_payload(message: &str) -> String {
    let mut out = String::with_capacity(message.len() + 16);
    // a lone CR is a line end too: a server that takes <CR>.<CR> as the end of the data must not see one in the text
    let message = message.replace("\r\n", "\n").replace('\r', "\n");
    for line in message.split('\n') {
        if line.starts_with('.') {
            out.push('.');
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    // the split above leaves an empty last element after a trailing newline; drop the extra CRLF it made
    if message.ends_with('\n') {
        out.truncate(out.len() - 2);
    }
    out.push_str(".\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_lines_and_multiline_replies() {
        assert_eq!(parse_reply_line("250-SIZE 100"), Some(ReplyLine { code: 250, more: true, text: "SIZE 100".into() }));
        assert_eq!(parse_reply_line("250 OK"), Some(ReplyLine { code: 250, more: false, text: "OK".into() }));
        assert_eq!(parse_reply_line("250"), Some(ReplyLine { code: 250, more: false, text: String::new() }));
        assert_eq!(parse_reply_line("250 x"), Some(ReplyLine { code: 250, more: false, text: "x".into() }), "code, space, text is a reply");
        assert_eq!(parse_reply_line("250x"), None);
        let (code, texts) = collect_reply(&["250-mail.example", "250-SIZE 10", "250 AUTH PLAIN"]).unwrap();
        assert_eq!(code, 250);
        assert_eq!(texts, vec!["mail.example", "SIZE 10", "AUTH PLAIN"]);
        assert!(collect_reply(&["250-a", "354 b"]).is_none());
        assert!(collect_reply(&["250-a"]).is_none());
        assert!(is_positive(250) && is_positive(354) && !is_positive(535));
    }

    #[test]
    fn commands() {
        assert_eq!(ehlo("client.local"), "EHLO client.local\r\n");
        assert_eq!(mail_from("a@b.is"), "MAIL FROM:<a@b.is>\r\n");
        assert_eq!(rcpt_to("c@d.is"), "RCPT TO:<c@d.is>\r\n");
        assert_eq!(data(), "DATA\r\n");
        assert_eq!(quit(), "QUIT\r\n");
        // base64 of "\0user\0pw" is AHVzZXIAcHc=
        assert_eq!(auth_plain("user", "pw"), "AUTH PLAIN AHVzZXIAcHc=\r\n");
    }

    #[test]
    fn data_is_dot_stuffed_and_terminated() {
        assert_eq!(data_payload("Subject: x\r\n\r\n.hidden\nline\n"), "Subject: x\r\n\r\n..hidden\r\nline\r\n.\r\n");
        assert_eq!(data_payload("no newline"), "no newline\r\n.\r\n");
        assert_eq!(data_payload(".\n"), "..\r\n.\r\n");
        // a bare CR is a line end, so a dot after it is stuffed and no CR.CR survives
        assert_eq!(data_payload("x\r.\rMAIL\r\ny\n"), "x\r\n..\r\nMAIL\r\ny\r\n.\r\n");
    }
}
