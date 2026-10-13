//! An IMAP connection (RFC 3501) for the commands the app sends: login, mailboxes, the newest message headers, one
//! message, and the Seen flag. One connection serves one job. Every literal (`{n}` followed by n bytes) is read here and
//! folded into its response line as a quoted string, so the parsers in [`crate::imap`] see one line per response.

use bifrost_app::controls::internet_accounts::{Security, Server};
use crate::flags::{Flag, Flags};
use crate::imap::{self, Mailbox, Reply, Status, Value};
use crate::line::Lines;
use crate::transport;
use springboard::deildu::http::{Stream, Transport};
use std::io::{self, Read, Write};

/// Newest messages a mailbox listing reads headers for.
pub const LIST_LIMIT: usize = 50;

/// One message as the listing needs it: its UID, flags and raw header block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub uid: u32,
    pub flags: Flags,
    pub header: String,
}

/// What one tagged command produced: the untagged lines, and the final status with its text.
struct Exchange {
    untagged: Vec<String>,
    status: Status,
    text: String,
}

pub struct Session<S: Read + Write> {
    lines: Lines<S>,
    tag: u32,
}

impl<S: Read + Write> Session<S> {
    /// Reads the server greeting and starts a session on `io`. The greeting must be `OK` or `PREAUTH`.
    pub fn open(io: S) -> Result<Self, String> {
        let mut lines = Lines::new(io);
        let greeting = lines.read_line().map_err(io_text)?;
        if !(greeting.starts_with("* OK") || greeting.starts_with("* PREAUTH")) {
            return Err(format!("the server did not greet: {greeting}"));
        }
        Ok(Session { lines, tag: 0 })
    }

    pub fn login(&mut self, user: &str, password: &str) -> Result<(), String> {
        self.run(&imap::login(user, password)).map(|_| ())
    }

    /// Every selectable mailbox, in the server's order.
    pub fn list(&mut self) -> Result<Vec<Mailbox>, String> {
        let untagged = self.run(&imap::list_all())?;
        Ok(untagged.iter().filter_map(|line| imap::parse_list(line)).filter(Mailbox::selectable).collect())
    }

    pub fn select(&mut self, mailbox: &str) -> Result<(), String> {
        self.run(&imap::select(mailbox)).map(|_| ())
    }

    /// The UIDs of every message in the selected mailbox, in the server's order (ascending for most servers).
    pub fn uids(&mut self) -> Result<Vec<u32>, String> {
        let untagged = self.run(&imap::uid_search_all())?;
        Ok(untagged.iter().find_map(|line| imap::parse_search(line)).unwrap_or_default())
    }

    /// Flags and header block of the given messages. Headers are read whole, so the list gets Subject, From and Date.
    pub fn headers(&mut self, uids: &[u32]) -> Result<Vec<Listed>, String> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let set: Vec<String> = uids.iter().map(u32::to_string).collect();
        let untagged = self.run(&format!("UID FETCH {} (UID FLAGS BODY.PEEK[HEADER])", set.join(",")))?;
        Ok(untagged.iter().filter_map(|line| listed(line)).collect())
    }

    /// The whole message (headers and body) as the server holds it. Does not set the Seen flag.
    pub fn message(&mut self, uid: u32) -> Result<String, String> {
        let untagged = self.run(&format!("UID FETCH {uid} (UID BODY.PEEK[])"))?;
        untagged.iter().find_map(|line| field(line, "BODY[]")).ok_or_else(|| "the message has no body".to_string())
    }

    pub fn mark_seen(&mut self, uid: u32) -> Result<(), String> {
        self.run(&imap::store_flags(uid, '+', Flags::empty().with(Flag::Seen, true))).map(|_| ())
    }

    pub fn logout(&mut self) {
        let _ = self.run(&imap::logout());
    }

    /// Sends one command and returns its untagged lines when it finished with OK; the server's text otherwise.
    fn run(&mut self, body: &str) -> Result<Vec<String>, String> {
        let exchange = self.command(body)?;
        match exchange.status {
            Status::Ok => Ok(exchange.untagged),
            _ => Err(exchange.text),
        }
    }

    fn command(&mut self, body: &str) -> Result<Exchange, String> {
        self.tag += 1;
        let tag = imap::tag(self.tag);
        self.lines.send(&imap::command(&tag, body)).map_err(io_text)?;
        let mut untagged = Vec::new();
        loop {
            let line = self.response_line()?;
            if let Some(rest) = line.strip_prefix("* ") {
                untagged.push(rest.to_string());
                continue;
            }
            match imap::parse_line(&line) {
                Reply::Tagged { tag: got, status, text } if got == tag => return Ok(Exchange { untagged, status, text }),
                Reply::Continuation(_) => return Err("the server asked for data the app did not send".into()),
                _ => {}
            }
        }
    }

    /// One response line with its literals folded in. A line ending in `{n}` is followed by n bytes and then the rest
    /// of the response on the next line; the bytes become a quoted string at the marker.
    fn response_line(&mut self) -> Result<String, String> {
        let mut line = self.lines.read_line().map_err(io_text)?;
        while let Some((at, n)) = literal_marker(&line) {
            let bytes = self.lines.read_bytes(n).map_err(io_text)?;
            line.truncate(at);
            line.push_str(&literal_text(&bytes));
            let rest = self.lines.read_line().map_err(io_text)?;
            line.push_str(&rest);
        }
        Ok(line)
    }
}

impl Session<Box<dyn Stream>> {
    /// Upgrades a plain connection with STARTTLS (RFC 3501 section 6.2.1). The greeting has already been read.
    pub fn starttls(mut self, transport: &dyn Transport, host: &str) -> Result<Self, String> {
        self.run("STARTTLS")?;
        let Session { lines, tag } = self;
        let secure = transport.upgrade(host, lines.into_inner()).map_err(io_text)?;
        Ok(Session { lines: Lines::new(secure), tag })
    }
}

/// Connects to the IMAP server of `server`, upgrading with STARTTLS when the account says so.
pub fn connect(transport: &dyn Transport, server: &Server) -> Result<Session<Box<dyn Stream>>, String> {
    let stream = transport::open(transport, server).map_err(io_text)?;
    let session = Session::open(stream)?;
    match server.security {
        Security::StartTls => session.starttls(transport, &server.host),
        _ => Ok(session),
    }
}

/// `{n}` at the end of a response line: the byte count and where the marker starts.
fn literal_marker(line: &str) -> Option<(usize, usize)> {
    let inner = line.strip_suffix('}')?;
    let at = inner.rfind('{')?;
    let digits = &inner[at + 1..];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((at, digits.parse().ok()?))
}

/// A literal as a quoted IMAP string. UTF-8 stays as it is; any other bytes are read as ISO-8859-1, one char per byte,
/// so the message text keeps every byte and the body decoder can still find the bytes again.
pub fn literal_text(bytes: &[u8]) -> String {
    let text: String = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    };
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn listed(line: &str) -> Option<Listed> {
    let (_, pairs) = imap::fetch_pairs(line)?;
    let mut uid = None;
    let mut flags = Flags::empty();
    let mut header = String::new();
    for (key, value) in pairs {
        match key.to_ascii_uppercase().as_str() {
            "UID" => uid = value.text().and_then(|t| t.parse::<u32>().ok()),
            "FLAGS" => {
                if let Value::List(names) = &value {
                    flags = Flags::from_names(names.iter().filter_map(Value::text));
                }
            }
            "BODY[HEADER]" => header = value.text().unwrap_or_default().to_string(),
            _ => {}
        }
    }
    Some(Listed { uid: uid?, flags, header })
}

fn field(line: &str, name: &str) -> Option<String> {
    let (_, pairs) = imap::fetch_pairs(line)?;
    pairs.into_iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).and_then(|(_, value)| value.text().map(str::to_string))
}

fn io_text(e: io::Error) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::line::script::Script;

    fn session(script: &str) -> Session<Script> {
        Session::open(Script::new(&format!("* OK ready\r\n{script}"))).unwrap()
    }

    #[test]
    fn a_bad_greeting_is_refused() {
        assert!(Session::open(Script::new("* BYE go away\r\n")).is_err());
    }

    #[test]
    fn login_and_failures_carry_the_server_text() {
        let mut s = session("A001 OK LOGIN done\r\n");
        s.login("me", "pw").unwrap();
        assert!(s.lines.into_inner().sent().starts_with("A001 LOGIN \"me\" \"pw\"\r\n"));

        let mut s = session("A001 NO [AUTHENTICATIONFAILED] bad\r\n");
        assert_eq!(s.login("me", "x").unwrap_err(), "[AUTHENTICATIONFAILED] bad");
    }

    #[test]
    fn lists_only_selectable_mailboxes() {
        let mut s = session("* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n* LIST (\\Noselect) \"/\" \"Work\"\r\n* LIST (\\Sent) \"/\" \"Sent\"\r\nA001 OK done\r\n");
        let names: Vec<String> = s.list().unwrap().into_iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["INBOX", "Sent"]);
    }

    #[test]
    fn uids_come_from_the_search() {
        let mut s = session("* SEARCH 3 9 12\r\nA001 OK done\r\n");
        assert_eq!(s.uids().unwrap(), vec![3, 9, 12]);
    }

    #[test]
    fn headers_arrive_as_literals_with_flags() {
        let header = "Subject: Hi\r\nFrom: Ana <ana@example.com>\r\n\r\n";
        let script = format!("* 1 FETCH (UID 7 FLAGS (\\Seen) BODY[HEADER] {{{}}}\r\n{header})\r\nA001 OK done\r\n", header.len());
        let mut s = session(&script);
        let listed = s.headers(&[7]).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].uid, 7);
        assert!(listed[0].flags.contains(Flag::Seen));
        assert_eq!(listed[0].header, header);
    }

    #[test]
    fn a_message_body_keeps_its_bytes() {
        let raw = "Subject: x\r\n\r\n\"quoted\" \\ text\r\n";
        let script = format!("* 2 FETCH (UID 8 BODY[] {{{}}}\r\n{raw})\r\nA001 OK done\r\n", raw.len());
        let mut s = session(&script);
        assert_eq!(s.message(8).unwrap(), raw);
    }

    #[test]
    fn latin1_literals_map_one_char_per_byte() {
        assert_eq!(literal_text(b"\xe1"), "\"\u{e1}\"");
        assert_eq!(literal_text("á".as_bytes()), "\"á\"");
    }

    #[test]
    fn the_seen_flag_is_stored_silently() {
        let mut s = session("A001 OK done\r\n");
        s.mark_seen(5).unwrap();
        assert!(s.lines.into_inner().sent().contains("UID STORE 5 +FLAGS.SILENT (\\Seen)"));
    }

    #[test]
    fn literal_markers_are_found_at_the_end_only() {
        assert_eq!(literal_marker("* 1 FETCH (BODY[] {12}"), Some((18, 12)));
        assert_eq!(literal_marker("a {x}"), None);
        assert_eq!(literal_marker("ends with }"), None);
    }
}
