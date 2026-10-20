//! An IMAP connection (RFC 3501) for the commands the app sends: login, mailboxes, the newest message headers, one
//! message, and the Seen flag. One connection serves one job. Every literal (`{n}` followed by n bytes) is read here and
//! folded into its response line as a quoted string, so the parsers in [`crate::imap`] see one line per response.

use bifrost_app::controls::internet_accounts::{Security, Server};
use crate::flags::{Flag, FlagColor, Flags, COLOR_KEYWORDS};
use crate::imap::{self, Mailbox, Reply, Status, Value};
use crate::line::{response_after, Lines};
use crate::transport;
use springboard::deildu::http::{Stream, Transport};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

/// How long one IDLE lasts before it is ended and started again. RFC 2177 lets a server drop a client that idles for
/// 30 minutes or more, so the client renews well before that.
pub const IDLE_REFRESH: Duration = Duration::from_secs(25 * 60);

/// How an IDLE ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Idle {
    /// The mailbox reported a new message.
    NewMail,
    /// The renewal time came; nothing happened. The caller starts IDLE again.
    Refresh,
    /// The caller asked to stop.
    Cancelled,
}

/// True for the errors a socket read timeout gives.
fn is_timeout(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock)
}

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

    /// The UID values of every message in the selected mailbox, in the server's order (ascending for most servers).
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

    /// Adds (`on`) or removes one flag on a message, silently.
    pub fn set_flag(&mut self, uid: u32, flag: Flag, on: bool) -> Result<(), String> {
        let op = if on { '+' } else { '-' };
        self.run(&imap::store_flags(uid, op, Flags::empty().with(flag, true))).map(|_| ())
    }

    /// Flags a message in `color`, or clears its flag and colour (`None`). The old colour bits are cleared first, so the
    /// colours never add up.
    pub fn set_color(&mut self, uid: u32, color: Option<FlagColor>) -> Result<(), String> {
        let everything = Flags::from_names(COLOR_KEYWORDS).with(Flag::Flagged, true);
        self.run(&imap::store_flags(uid, '-', everything))?;
        if let Some(color) = color {
            self.run(&imap::store_flags(uid, '+', Flags::empty().with_color(color)))?;
        }
        Ok(())
    }

    /// Moves one message to `mailbox`: `UID MOVE` (RFC 6851) when the server has it; otherwise the message is copied,
    /// marked `\Deleted`, and that one UID is expunged (UIDPLUS, RFC 4315), so other deleted messages stay.
    pub fn move_to(&mut self, uid: u32, mailbox: &str) -> Result<(), String> {
        let target = imap::quote(mailbox);
        if self.run(&format!("UID MOVE {uid} {target}")).is_ok() {
            return Ok(());
        }
        self.run(&format!("UID COPY {uid} {target}"))?;
        self.run(&imap::store_flags(uid, '+', Flags::empty().with(Flag::Deleted, true)))?;
        self.run(&format!("UID EXPUNGE {uid}")).map(|_| ())
    }

    /// Deletes one message of the selected mailbox: it is marked deleted and that one UID is expunged (UIDPLUS, RFC 4315),
    /// so other messages marked for deletion stay.
    pub fn remove(&mut self, uid: u32) -> Result<(), String> {
        self.run(&imap::store_flags(uid, '+', Flags::empty().with(Flag::Deleted, true)))?;
        self.run(&format!("UID EXPUNGE {uid}")).map(|_| ())
    }

    /// Adds or removes one keyword (`$Junk`, ...) on a message, silently.
    pub fn set_keyword(&mut self, uid: u32, keyword: &str, on: bool) -> Result<(), String> {
        let op = if on { '+' } else { '-' };
        self.run(&format!("UID STORE {uid} {op}FLAGS.SILENT ({keyword})")).map(|_| ())
    }

    /// Adds a message to `mailbox` with `flags` (APPEND, RFC 3501 section 6.3.11). The text is sent as a literal with
    /// CRLF line ends once the server has answered the command with a continuation.
    pub fn append(&mut self, mailbox: &str, flags: Flags, raw: &str) -> Result<(), String> {
        let data = crate::export::crlf(raw);
        self.tag += 1;
        let tag = imap::tag(self.tag);
        let command = format!("APPEND {} {} {{{}}}", imap::quote(mailbox), flags.to_imap_list(), data.len());
        self.lines.send(&imap::command(&tag, &command)).map_err(io_text)?;
        loop {
            let line = self.response_line()?;
            match imap::parse_line(&line) {
                Reply::Continuation(_) => break,
                Reply::Tagged { tag: got, text, .. } if got == tag => return Err(text),
                _ => {}
            }
        }
        self.lines.send(&format!("{data}\r\n")).map_err(io_text)?;
        loop {
            let line = self.response_line()?;
            if let Reply::Tagged { tag: got, status, text } = imap::parse_line(&line) {
                if got == tag {
                    return match status {
                        Status::Ok => Ok(()),
                        _ => Err(text),
                    };
                }
            }
        }
    }

    /// Copies one message to `mailbox` and leaves the original where it is (Message > Copy To).
    pub fn copy_to(&mut self, uid: u32, mailbox: &str) -> Result<(), String> {
        self.run(&format!("UID COPY {uid} {}", imap::quote(mailbox))).map(|_| ())
    }

    /// Erases every message of the selected mailbox: all are marked `\Deleted` and expunged (Mailbox > Erase Deleted
    /// Items, Erase Junk Mail).
    pub fn erase_all(&mut self) -> Result<(), String> {
        self.run("STORE 1:* +FLAGS.SILENT (\\Deleted)")?;
        self.run("EXPUNGE").map(|_| ())
    }

    pub fn create_mailbox(&mut self, name: &str) -> Result<(), String> {
        self.run(&format!("CREATE {}", imap::quote(name))).map(|_| ())
    }

    pub fn rename_mailbox(&mut self, from: &str, to: &str) -> Result<(), String> {
        self.run(&format!("RENAME {} {}", imap::quote(from), imap::quote(to))).map(|_| ())
    }

    pub fn delete_mailbox(&mut self, name: &str) -> Result<(), String> {
        self.run(&format!("DELETE {}", imap::quote(name))).map(|_| ())
    }

    /// Messages of the selected mailbox that match `criteria` (IMAP search keys, see [`crate::search::Query::imap_criteria`]),
    /// as UID values in the server's order.
    pub fn search(&mut self, criteria: &str) -> Result<Vec<u32>, String> {
        let untagged = self.run(&format!("UID SEARCH {criteria}"))?;
        Ok(untagged.iter().find_map(|line| imap::parse_search(line)).unwrap_or_default())
    }

    /// The capability names the server advertises (CAPABILITY), for example `IDLE`.
    pub fn capabilities(&mut self) -> Result<Vec<String>, String> {
        let untagged = self.run(&imap::capability())?;
        let names = untagged
            .iter()
            .find_map(|line| line.strip_prefix("CAPABILITY ").map(|rest| rest.split_whitespace().map(str::to_string).collect::<Vec<String>>()));
        Ok(names.unwrap_or_default())
    }

    /// Waits in IDLE (RFC 2177) until the selected mailbox reports a new message, then ends IDLE. Other untagged
    /// updates (flags, expunges, keep-alives) keep it waiting. A `BYE` or a refused IDLE is an error.
    pub fn idle(&mut self) -> Result<(), String> {
        self.idle_until(Duration::MAX, &|| false).map(|_| ())
    }

    /// Like [`Session::idle`], for a connection that idles for a long time. The socket's read timeout only means that
    /// nothing has happened yet, so it keeps waiting. IDLE ends when a message arrives, after `refresh` (so the caller
    /// can start it again before the server drops the connection), or when `cancelled` says so; `cancelled` is asked
    /// each time the read times out.
    pub fn idle_until(&mut self, refresh: Duration, cancelled: &dyn Fn() -> bool) -> Result<Idle, String> {
        self.tag += 1;
        let tag = imap::tag(self.tag);
        self.lines.send(&imap::command(&tag, &imap::idle())).map_err(io_text)?;
        loop {
            let line = self.response_line()?;
            match imap::parse_line(&line) {
                Reply::Continuation(_) => break,
                Reply::Tagged { tag: got, text, .. } if got == tag => return Err(text),
                _ => {}
            }
        }
        let started = Instant::now();
        let mut outcome = Idle::Refresh;
        loop {
            let first = match self.lines.read_line() {
                Ok(line) => line,
                Err(e) if is_timeout(&e) => {
                    if cancelled() {
                        outcome = Idle::Cancelled;
                        break;
                    }
                    if started.elapsed() >= refresh {
                        break;
                    }
                    continue;
                }
                Err(e) => return Err(io_text(e)),
            };
            let line = self.finish_line(first).map_err(io_text)?;
            if let Reply::Untagged(rest) = imap::parse_line(&line) {
                if rest.starts_with("BYE") {
                    return Err(rest);
                }
                if imap::is_new_message(&rest) {
                    outcome = Idle::NewMail;
                    break;
                }
            }
        }
        self.lines.send(imap::IDLE_DONE).map_err(io_text)?;
        let mut stalls = 0;
        loop {
            let first = match self.lines.read_line() {
                Ok(line) => line,
                // the server may take a moment to answer DONE
                Err(e) if is_timeout(&e) && stalls < 3 => {
                    stalls += 1;
                    continue;
                }
                Err(e) => return Err(io_text(e)),
            };
            let line = self.finish_line(first).map_err(io_text)?;
            match imap::parse_line(&line) {
                Reply::Tagged { tag: got, status, text } if got == tag => {
                    return match status {
                        Status::Ok => Ok(outcome),
                        _ => Err(text),
                    };
                }
                Reply::Untagged(rest) if imap::is_new_message(&rest) && outcome == Idle::Refresh => outcome = Idle::NewMail,
                _ => {}
            }
        }
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
        let first = self.lines.read_line().map_err(io_text)?;
        self.finish_line(first).map_err(io_text)
    }

    /// Folds the literals that follow `line`, the first line of a response, into it.
    fn finish_line(&mut self, mut line: String) -> io::Result<String> {
        let mut folded = 0;
        while let Some((at, n)) = literal_marker(&line) {
            folded = response_after(folded, n)?;
            let bytes = self.lines.read_bytes(n)?;
            line.truncate(at);
            line.push_str(&literal_text(&bytes));
            let rest = self.lines.read_line()?;
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
    use crate::line::MAX_LITERAL;

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
    fn copy_erase_and_mailbox_commands_are_sent_as_imap_writes_them() {
        let mut s = session("A001 OK done\r\nA002 OK done\r\nA003 OK done\r\nA004 OK done\r\nA005 OK done\r\nA006 OK done\r\nA007 OK done\r\n");
        s.copy_to(5, "Work").unwrap();
        s.erase_all().unwrap();
        s.create_mailbox("Receipts").unwrap();
        s.rename_mailbox("Receipts", "Bills").unwrap();
        s.delete_mailbox("Bills").unwrap();
        let sent = s.lines.into_inner().sent();
        assert_eq!(
            sent,
            "A001 UID COPY 5 \"Work\"\r\nA002 STORE 1:* +FLAGS.SILENT (\\Deleted)\r\nA003 EXPUNGE\r\nA004 CREATE \"Receipts\"\r\nA005 RENAME \"Receipts\" \"Bills\"\r\nA006 DELETE \"Bills\"\r\n"
        );
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
    fn search_returns_the_matching_uids() {
        let mut s = session("* SEARCH 2 5\r\nA001 OK done\r\n");
        assert_eq!(s.search("FROM \"ana\"").unwrap(), vec![2, 5]);
        assert!(s.lines.into_inner().sent().starts_with("A001 UID SEARCH FROM \"ana\"\r\n"));
    }

    #[test]
    fn capabilities_are_read_from_the_capability_line() {
        let mut s = session("* CAPABILITY IMAP4rev1 IDLE UIDPLUS\r\nA001 OK done\r\n");
        assert_eq!(s.capabilities().unwrap(), vec!["IMAP4rev1", "IDLE", "UIDPLUS"]);
    }

    #[test]
    fn idle_waits_for_a_new_message_then_sends_done() {
        let mut s = session("+ idling\r\n* 3 FETCH (FLAGS (\\Seen))\r\n* 4 EXISTS\r\nA001 OK IDLE terminated\r\n");
        s.idle().unwrap();
        let sent = s.lines.into_inner().sent();
        assert!(sent.starts_with("A001 IDLE\r\n"));
        assert!(sent.ends_with("DONE\r\n"));
    }

    /// A connection that stalls (a read timeout) wherever the script has a gap.
    struct Stalling {
        chunks: std::collections::VecDeque<Option<Vec<u8>>>,
        output: Vec<u8>,
    }

    impl Stalling {
        fn new(chunks: Vec<Option<&str>>) -> Self {
            Stalling { chunks: chunks.into_iter().map(|c| c.map(|t| t.as_bytes().to_vec())).collect(), output: Vec::new() }
        }
    }

    impl Read for Stalling {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.chunks.pop_front() {
                None => Ok(0),
                Some(None) => Err(io::Error::new(io::ErrorKind::TimedOut, "timed out")),
                Some(Some(bytes)) => {
                    buf[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
            }
        }
    }

    impl Write for Stalling {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn stalling(chunks: Vec<Option<&str>>) -> Session<Stalling> {
        let mut all = vec![Some("* OK ready\r\n")];
        all.extend(chunks);
        Session::open(Stalling::new(all)).unwrap()
    }

    #[test]
    fn a_read_timeout_does_not_end_idle() {
        let mut s = stalling(vec![Some("+ idling\r\n"), None, None, Some("* 4 EXISTS\r\n"), Some("A001 OK done\r\n")]);
        assert_eq!(s.idle_until(IDLE_REFRESH, &|| false).unwrap(), Idle::NewMail);
        assert!(String::from_utf8_lossy(&s.lines.into_inner().output).ends_with("DONE\r\n"));
    }

    #[test]
    fn idle_is_renewed_when_its_time_is_up() {
        let mut s = stalling(vec![Some("+ idling\r\n"), None, Some("A001 OK done\r\n")]);
        assert_eq!(s.idle_until(Duration::ZERO, &|| false).unwrap(), Idle::Refresh);
    }

    #[test]
    fn idle_stops_when_cancelled() {
        let mut s = stalling(vec![Some("+ idling\r\n"), None, Some("A001 OK done\r\n")]);
        assert_eq!(s.idle_until(IDLE_REFRESH, &|| true).unwrap(), Idle::Cancelled);
        assert!(String::from_utf8_lossy(&s.lines.into_inner().output).ends_with("DONE\r\n"));
    }

    #[test]
    fn idle_refused_or_ended_by_the_server_is_an_error() {
        let mut s = session("A001 BAD IDLE not allowed\r\n");
        assert_eq!(s.idle().unwrap_err(), "IDLE not allowed");
        let mut s = session("+ idling\r\n* BYE logout\r\n");
        assert_eq!(s.idle().unwrap_err(), "BYE logout");
    }

    #[test]
    fn a_flag_colour_clears_the_old_bits_then_sets_the_new_ones() {
        let mut s = session("A001 OK done\r\nA002 OK done\r\nA003 OK done\r\n");
        s.set_color(7, Some(FlagColor::Green)).unwrap();
        s.set_color(7, None).unwrap();
        let sent = s.lines.into_inner().sent();
        assert!(
            sent.starts_with("A001 UID STORE 7 -FLAGS.SILENT (\\Flagged $MailFlagBit0 $MailFlagBit1 $MailFlagBit2)\r\nA002 UID STORE 7 +FLAGS.SILENT (\\Flagged $MailFlagBit0 $MailFlagBit1)\r\nA003 UID STORE 7 -FLAGS.SILENT"),
            "{sent}"
        );
    }

    #[test]
    fn append_sends_the_message_as_a_literal_after_the_continuation() {
        let mut s = session("+ Ready for literal data\r\nA001 OK [APPENDUID 1 9] done\r\n");
        s.append("Archive", Flags::empty().with(Flag::Seen, true), "Subject: x\n\nbody\n").unwrap();
        let sent = s.lines.into_inner().sent();
        assert_eq!(sent, "A001 APPEND \"Archive\" (\\Seen) {20}\r\nSubject: x\r\n\r\nbody\r\n\r\n");
    }

    #[test]
    fn append_refused_before_the_data_is_an_error_and_sends_no_data() {
        let mut s = session("A001 NO [TRYCREATE] no such mailbox\r\n");
        assert_eq!(s.append("Nope", Flags::empty(), "Subject: x\r\n\r\n").unwrap_err(), "[TRYCREATE] no such mailbox");
        assert!(!s.lines.into_inner().sent().contains("Subject"));
        let mut s = session("+ go\r\nA001 NO quota exceeded\r\n");
        assert_eq!(s.append("Box", Flags::empty(), "x").unwrap_err(), "quota exceeded");
    }

    #[test]
    fn remove_marks_the_message_deleted_and_expunges_only_it() {
        let mut s = session("A001 OK done\r\nA002 OK done\r\n");
        s.remove(9).unwrap();
        assert_eq!(s.lines.into_inner().sent(), "A001 UID STORE 9 +FLAGS.SILENT (\\Deleted)\r\nA002 UID EXPUNGE 9\r\n");
    }

    #[test]
    fn keywords_are_stored_silently_with_their_sign() {
        let mut s = session("A001 OK done\r\nA002 OK done\r\n");
        s.set_keyword(4, "$Junk", true).unwrap();
        s.set_keyword(4, "$Junk", false).unwrap();
        assert_eq!(s.lines.into_inner().sent(), "A001 UID STORE 4 +FLAGS.SILENT ($Junk)\r\nA002 UID STORE 4 -FLAGS.SILENT ($Junk)\r\n");
    }

    #[test]
    fn the_seen_flag_is_stored_silently() {
        let mut s = session("A001 OK done\r\n");
        s.mark_seen(5).unwrap();
        assert!(s.lines.into_inner().sent().contains("UID STORE 5 +FLAGS.SILENT (\\Seen)"));
    }

    #[test]
    fn an_oversized_literal_marker_is_an_error_not_a_hang() {
        let mut s = session("* 1 FETCH (UID 1 BODY[] {999999999999}\r\n");
        let err = s.message(1).unwrap_err();
        assert_eq!(err, "server literal too large");
    }

    /// A connection whose input is assembled from pieces, so a test can stream literals of any size without holding them.
    struct Wire(Box<dyn Read>);

    impl Read for Wire {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Write for Wire {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn literals_that_add_up_past_the_response_cap_are_refused() {
        // three literals in one response: two of the largest size fill the cap, the third one (1 byte) goes past it
        let filler = |n: usize| io::repeat(b'x').take(n as u64);
        let text = |s: String| io::Cursor::new(s.into_bytes());
        let input = text(format!("* OK ready\r\n* 1 FETCH (A {{{MAX_LITERAL}}}\r\n"))
            .chain(filler(MAX_LITERAL))
            .chain(text(format!(" B {{{MAX_LITERAL}}}\r\n")))
            .chain(filler(MAX_LITERAL))
            .chain(text(" C {1}\r\n".to_string()))
            .chain(filler(1));
        let mut s = Session::open(Wire(Box::new(input))).unwrap();
        assert_eq!(s.message(1).unwrap_err(), "server response too large");
    }

    #[test]
    fn literal_markers_are_found_at_the_end_only() {
        assert_eq!(literal_marker("* 1 FETCH (BODY[] {12}"), Some((18, 12)));
        assert_eq!(literal_marker("a {x}"), None);
        assert_eq!(literal_marker("ends with }"), None);
    }
}
