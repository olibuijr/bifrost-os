//! RFC 5322 messages: the header parser that reads a message's headers for the list and threads, and the
//! renderer that turns a compose draft into the text SMTP sends.

use bifrost_app::controls::internet_accounts::valid_address;
use crate::body::Attachment;
use crate::codec::{base64_encode, decode_words, encode_word};

/// The headers of one message, as the list and the threads need them. Names are matched without case.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub message_id: Option<String>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    pub subject: String,
    pub from: String,
    pub to: String,
    pub cc: String,
    pub reply_to: String,
    pub date: String,
    /// The `List-Unsubscribe` header, empty when absent. Mailing lists and bulk senders set it.
    pub list_unsubscribe: String,
    /// The `List-Id` header, empty when absent.
    pub list_id: String,
    /// The `Precedence` header in lower case (`bulk`, `list`, `junk`), empty when absent.
    pub precedence: String,
    /// True when the message carries an `Auto-Submitted` header other than `no` (an automatic message).
    pub automatic: bool,
    /// The media type of the message itself in lower case (`multipart/mixed`), empty when it names none.
    pub content_type: String,
    /// The priority the sender set (`X-Priority`, or `Importance` when that is absent). Normal when neither says so.
    pub priority: Priority,
}

/// Unfolds and splits a header block into `(lowercase name, value)` pairs. Stops at the first empty line.
pub fn parse_headers(raw: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in raw.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            break;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some((_, value)) = out.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            out.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    out
}

/// The `<id>` tokens in a References or In-Reply-To value, with the angle brackets kept.
pub fn parse_ids(value: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find('<') {
        let Some(len) = rest[start..].find('>') else { break };
        ids.push(rest[start..start + len + 1].to_string());
        rest = &rest[start + len + 1..];
    }
    ids
}

impl Summary {
    pub fn from_headers(raw: &str) -> Summary {
        let headers = parse_headers(raw);
        let get = |name: &str| headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
        let message_id = get("message-id").and_then(|v| parse_ids(v).into_iter().next());
        Summary {
            message_id,
            in_reply_to: get("in-reply-to").map(parse_ids).unwrap_or_default(),
            references: get("references").map(parse_ids).unwrap_or_default(),
            subject: get("subject").map(decode_words).unwrap_or_default(),
            from: get("from").map(decode_words).unwrap_or_default(),
            to: get("to").map(decode_words).unwrap_or_default(),
            cc: get("cc").map(decode_words).unwrap_or_default(),
            reply_to: get("reply-to").map(decode_words).unwrap_or_default(),
            date: get("date").unwrap_or_default().to_string(),
            list_unsubscribe: get("list-unsubscribe").unwrap_or_default().to_string(),
            list_id: get("list-id").unwrap_or_default().to_string(),
            precedence: get("precedence").unwrap_or_default().to_ascii_lowercase(),
            automatic: get("auto-submitted").is_some_and(|v| !v.trim().eq_ignore_ascii_case("no")),
            content_type: get("content-type").and_then(|v| v.split(';').next()).unwrap_or_default().trim().to_ascii_lowercase(),
            priority: priority_of(get("x-priority"), get("importance")),
        }
    }

    /// True when the headers suggest the message carries files: a `multipart/mixed` (or `application/*`) top-level type.
    /// A guess made from the headers alone; the files themselves are known once the message is loaded.
    pub fn may_have_files(&self) -> bool {
        self.content_type == "multipart/mixed" || self.content_type.starts_with("application/")
    }
}

/// The priority a message's headers give: `X-Priority` 1 or 2 is high and 4 or 5 is low, and `Importance` is used when
/// `X-Priority` is absent or unclear. Anything else is normal.
pub fn priority_of(x_priority: Option<&str>, importance: Option<&str>) -> Priority {
    match x_priority.and_then(|v| v.trim().chars().next()) {
        Some('1' | '2') => return Priority::High,
        Some('3') => return Priority::Normal,
        Some('4' | '5') => return Priority::Low,
        _ => {}
    }
    match importance.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("high") => Priority::High,
        Some("low") => Priority::Low,
        _ => Priority::Normal,
    }
}

/// How urgent the sender marks a message (Message > Set Priority). It is sent as `X-Priority` and `Importance`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Priority {
    High,
    #[default]
    Normal,
    Low,
}

impl Priority {
    pub const ALL: [Priority; 3] = [Priority::High, Priority::Normal, Priority::Low];

    pub fn label(self) -> &'static str {
        match self {
            Priority::High => "High",
            Priority::Normal => "Normal",
            Priority::Low => "Low",
        }
    }

    /// The header lines for this priority, each ending in CRLF. Normal sends none.
    fn headers(self) -> &'static str {
        match self {
            Priority::High => "X-Priority: 1 (Highest)\r\nImportance: high\r\n",
            Priority::Normal => "",
            Priority::Low => "X-Priority: 5 (Lowest)\r\nImportance: low\r\n",
        }
    }
}

/// A message being written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    /// Blind copies: they get the message but are named in no header.
    pub bcc: Vec<String>,
    pub reply_to: Vec<String>,
    pub priority: Priority,
    pub subject: String,
    pub body: String,
    /// The Message-ID being answered, when this is a reply.
    pub in_reply_to: Option<String>,
    /// The References chain of the message being answered, oldest first.
    pub references: Vec<String>,
    /// Files sent with the message (Message > Attach File). With any, the message is `multipart/mixed`.
    pub attachments: Vec<Attachment>,
    /// The message as HTML, when it has formatting (the Format menu). It is sent next to the plain text as an alternative.
    pub html: Option<String>,
}

/// Most files one message carries.
pub const MAX_ATTACHMENTS: usize = 8;
/// Most bytes of files one message carries (the sum of the files before base64). Servers commonly refuse more.
pub const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Civil date (year, month, day) from days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe as i64 + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// RFC 2822 date in UTC, for example `Tue, 13 Oct 2026 09:30:00 +0000`.
pub fn rfc2822_date(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let weekday = DAYS[(days + 4).rem_euclid(7) as usize];
    format!(
        "{weekday}, {day:02} {} {year} {:02}:{:02}:{:02} +0000",
        MONTHS[(month - 1) as usize],
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// The time of day in UTC as `09:30:05`, for the activity list.
pub fn time_of_day(unix: i64) -> String {
    let secs = unix.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
}

/// A Message-ID for a new message: `<unix.seq@domain>`.
pub fn new_message_id(domain: &str, unix: i64, seq: u64) -> String {
    format!("<{unix}.{seq}@{domain}>")
}

impl Draft {
    /// Checks the addresses and that there is at least one recipient, then renders the message with CRLF line
    /// endings. `message_id` and `unix` come from the caller so rendering stays deterministic.
    pub fn render(&self, message_id: &str, unix: i64) -> Result<String, String> {
        if !valid_address(&self.from) {
            return Err("the sender address is not valid".into());
        }
        if self.to.is_empty() {
            return Err("add at least one recipient".into());
        }
        for addr in self.to.iter().chain(&self.cc).chain(&self.bcc).chain(&self.reply_to) {
            if !valid_address(addr) {
                return Err(format!("{addr} is not a valid address"));
            }
        }
        let mut out = String::new();
        out.push_str(&format!("Date: {}\r\n", rfc2822_date(unix)));
        out.push_str(&format!("From: {}\r\n", self.from));
        out.push_str(&format!("To: {}\r\n", self.to.join(", ")));
        if !self.cc.is_empty() {
            out.push_str(&format!("Cc: {}\r\n", self.cc.join(", ")));
        }
        if !self.reply_to.is_empty() {
            out.push_str(&format!("Reply-To: {}\r\n", self.reply_to.join(", ")));
        }
        out.push_str(&format!("Subject: {}\r\n", encode_subject(&self.subject)));
        out.push_str(self.priority.headers());
        out.push_str(&format!("Message-ID: {message_id}\r\n"));
        if let Some(parent) = &self.in_reply_to {
            out.push_str(&format!("In-Reply-To: {parent}\r\n"));
            let mut refs = self.references.clone();
            if !refs.contains(parent) {
                refs.push(parent.clone());
            }
            out.push_str(&format!("References: {}\r\n", refs.join(" ")));
        }
        out.push_str("MIME-Version: 1.0\r\n");
        let text = crlf_lines(&self.body);
        let html = self.html.as_deref().map(crlf_lines);
        // the readable part: plain text, or plain text and HTML as alternatives of each other
        let alternative = html.as_ref().map(|html| {
            let boundary = alternative_boundary_for(message_id, &format!("{text}{html}"));
            (format!("Content-Type: multipart/alternative; boundary=\"{boundary}\"\r\n\r\n"), alternative_body(&boundary, &text, html))
        });
        if self.attachments.is_empty() {
            match &alternative {
                Some((head, body)) => {
                    out.push_str(head);
                    out.push_str(body);
                }
                None => {
                    out.push_str("Content-Type: text/plain; charset=UTF-8\r\n");
                    out.push_str("Content-Transfer-Encoding: 8bit\r\n\r\n");
                    out.push_str(&text);
                }
            }
        } else {
            let boundary = boundary_for(message_id, &format!("{text}{}", html.as_deref().unwrap_or("")));
            out.push_str(&format!("Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n"));
            out.push_str("This is a multi-part message in MIME format.\r\n");
            out.push_str(&format!("--{boundary}\r\n"));
            match &alternative {
                Some((head, body)) => {
                    out.push_str(head);
                    out.push_str(body);
                }
                None => {
                    out.push_str("Content-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n");
                    out.push_str(&text);
                }
            }
            for file in &self.attachments {
                out.push_str(&format!("--{boundary}\r\n"));
                out.push_str(&file_part(file));
            }
            out.push_str(&format!("--{boundary}--\r\n"));
        }
        Ok(out)
    }

    /// Every address the message goes to (To, Cc and Bcc), for the SMTP RCPT commands.
    pub fn envelope_recipients(&self) -> Vec<&str> {
        self.to.iter().chain(&self.cc).chain(&self.bcc).map(String::as_str).collect()
    }
}

/// The body with every line ended by CRLF, the last line included.
fn crlf_lines(body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 2);
    for line in body.split('\n') {
        out.push_str(line.strip_suffix('\r').unwrap_or(line));
        out.push_str("\r\n");
    }
    out
}

/// The parts of a `multipart/alternative` body: the plain text, then the HTML. Both end in CRLF.
fn alternative_body(boundary: &str, text: &str, html: &str) -> String {
    format!(
        "--{boundary}\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{text}--{boundary}\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{html}--{boundary}--\r\n"
    )
}

/// The boundary of the `multipart/alternative` part. It does not start like [`boundary_for`]'s, because a boundary that
/// begins another boundary would end the outer part where the inner one begins.
fn alternative_boundary_for(message_id: &str, text: &str) -> String {
    let id: String = message_id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let mut boundary = format!("=_alt_{id}");
    while text.contains(&boundary) {
        boundary.push('x');
    }
    boundary
}

/// A multipart boundary built from the Message-ID, lengthened until the text does not contain it.
fn boundary_for(message_id: &str, text: &str) -> String {
    let id: String = message_id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let mut boundary = format!("=_bifrost_{id}");
    while text.contains(&boundary) {
        boundary.push('x');
    }
    boundary
}

/// A header parameter value (a file name) as a quoted string: quotes, backslashes and controls become plain characters,
/// and a name that is not ASCII is one encoded word.
fn param_value(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c == '"' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    format!("\"{}\"", encode_word(&clean))
}

/// One attached file as a MIME part (headers, blank line, body, ending in CRLF). A forwarded message
/// (`message/rfc822`) is carried as it is, in 8bit, because that type may not use base64; every other file is base64
/// in lines of 76 characters.
fn file_part(file: &Attachment) -> String {
    let mime = if file.mime.is_empty() || file.mime.contains(['\r', '\n', ';', ' ']) { "application/octet-stream" } else { file.mime.as_str() };
    let name = param_value(&file.name);
    let mut out = format!("Content-Type: {mime}; name={name}\r\nContent-Disposition: attachment; filename={name}\r\n");
    if mime.eq_ignore_ascii_case("message/rfc822") {
        if let Ok(text) = std::str::from_utf8(&file.bytes) {
            out.push_str("Content-Transfer-Encoding: 8bit\r\n\r\n");
            out.push_str(&crlf_lines(text.trim_end_matches(['\r', '\n'])));
            return out;
        }
    }
    out.push_str("Content-Transfer-Encoding: base64\r\n\r\n");
    let encoded = base64_encode(&file.bytes);
    for chunk in encoded.as_bytes().chunks(76) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or(""));
        out.push_str("\r\n");
    }
    out
}

/// The bare address in a `Name <addr>` value, or the value itself when it has no angle brackets.
pub fn address_of(value: &str) -> &str {
    match (value.rfind('<'), value.rfind('>')) {
        (Some(open), Some(close)) if open < close => value[open + 1..close].trim(),
        _ => value.trim(),
    }
}

/// The draft a reply starts from: the subject gets `Re: ` unless it has it, the recipient is the original sender,
/// and the References chain carries the original's References and Message-ID.
pub fn reply_draft(original: &Summary, from: &str) -> Draft {
    let subject = if original.subject.to_ascii_lowercase().starts_with("re:") {
        original.subject.clone()
    } else {
        format!("Re: {}", original.subject)
    };
    let references = reply_references(original);
    let target = if original.reply_to.trim().is_empty() { &original.from } else { &original.reply_to };
    Draft {
        from: from.to_string(),
        to: vec![address_of(target).to_string()],
        subject,
        in_reply_to: original.message_id.clone(),
        references,
        ..Draft::default()
    }
}

/// Splits an address list on commas and semicolons that are outside quotes and angle brackets, so a display name such
/// as `"Smith, Ana" <ana@example.com>` stays whole. Empty parts are dropped.
pub fn split_addresses(list: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let (mut quoted, mut angle) = (false, false);
    for c in list.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            '<' if !quoted => {
                angle = true;
                current.push(c);
            }
            '>' if !quoted => {
                angle = false;
                current.push(c);
            }
            ',' | ';' if !quoted && !angle => {
                let part = current.trim();
                if !part.is_empty() {
                    out.push(part.to_string());
                }
                current.clear();
            }
            _ => current.push(c),
        }
    }
    let part = current.trim();
    if !part.is_empty() {
        out.push(part.to_string());
    }
    out
}

/// The threading headers a reply carries: References of the original (or its In-Reply-To) and its Message-ID.
fn reply_references(original: &Summary) -> Vec<String> {
    if original.references.is_empty() {
        original.in_reply_to.clone()
    } else {
        original.references.clone()
    }
}

/// Reply All (Message > Reply All): the sender (or the Reply-To address) and everyone else in To go to To, everyone in
/// Cc stays in Cc. `own` (the user's address) is left out, and no address appears twice.
pub fn reply_all_draft(original: &Summary, own: &str) -> Draft {
    let mut draft = reply_draft(original, own);
    let mut seen: Vec<String> = vec![own.to_ascii_lowercase()];
    let mut take = |list: Vec<String>| -> Vec<String> {
        let mut kept = Vec::new();
        for entry in list {
            let addr = address_of(&entry).to_string();
            let key = addr.to_ascii_lowercase();
            if !addr.is_empty() && !seen.contains(&key) {
                seen.push(key);
                kept.push(addr);
            }
        }
        kept
    };
    let first = if original.reply_to.trim().is_empty() { split_addresses(&original.from) } else { split_addresses(&original.reply_to) };
    let mut to = take(first);
    to.extend(take(split_addresses(&original.to)));
    draft.to = to;
    draft.cc = take(split_addresses(&original.cc));
    draft
}

/// Forward (Message > Forward): the original's headers and text quoted under a `Fwd:` subject, with no recipient. It
/// starts a new conversation, so it carries no threading headers.
pub fn forward_draft(original: &Summary, from: &str, text: &str) -> Draft {
    let subject = if original.subject.to_ascii_lowercase().starts_with("fwd:") {
        original.subject.clone()
    } else {
        format!("Fwd: {}", original.subject)
    };
    let mut body = String::from("\n\n---------- Forwarded message ----------\n");
    body.push_str(&format!("From: {}\nDate: {}\nSubject: {}\nTo: {}\n", original.from, original.date, original.subject, original.to));
    if !original.cc.is_empty() {
        body.push_str(&format!("Cc: {}\n", original.cc));
    }
    body.push('\n');
    body.push_str(text);
    Draft { from: from.to_string(), subject, body, ..Draft::default() }
}

/// Forward as Attachment (Message > Forward as Attachment): the whole original goes along as a `message/rfc822` file under
/// a `Fwd:` subject; the body is empty for the sender to fill in. Like a plain forward it starts a new conversation.
pub fn forward_attachment_draft(original: &Summary, from: &str, raw: &str) -> Draft {
    let mut draft = forward_draft(original, from, "");
    draft.body.clear();
    draft.attachments.push(Attachment {
        name: crate::export::eml_name(&original.subject),
        mime: "message/rfc822".to_string(),
        bytes: crate::export::crlf(raw).into_bytes(),
    });
    draft
}

/// The draft a saved message reopens as (Message > Edit Draft): its addresses, subject, text and files, with the
/// threading headers, so a reply made from it still answers the message it answered. The text is the plain part, or
/// the HTML part as text; the HTML version is not kept.
pub fn draft_of(raw: &str) -> Draft {
    let s = Summary::from_headers(raw);
    let headers = parse_headers(raw);
    let get = |name: &str| headers.iter().find(|(n, _)| n == name).map(|(_, v)| decode_words(v)).unwrap_or_default();
    Draft {
        from: s.from.clone(),
        to: split_addresses(&s.to),
        cc: split_addresses(&s.cc),
        bcc: split_addresses(&get("bcc")),
        reply_to: split_addresses(&s.reply_to),
        priority: s.priority,
        subject: s.subject.clone(),
        body: crate::body::readable(raw),
        in_reply_to: s.in_reply_to.first().cloned(),
        references: s.references.clone(),
        attachments: crate::body::attachments(raw),
        html: None,
    }
}

/// Redirect (Message > Redirect): the original message exactly as it is, with `Resent-` headers in front that name the
/// person redirecting it (`from`) and the new recipients. The recipient sees the original sender and subject.
pub fn redirect_message(raw: &str, from: &str, to: &[String], message_id: &str, unix: i64) -> String {
    let mut out = format!("Resent-Date: {}\r\nResent-From: {from}\r\nResent-To: {}\r\nResent-Message-ID: {message_id}\r\n", rfc2822_date(unix), to.join(", "));
    out.push_str(&crate::export::crlf(raw));
    out
}

/// The date as the first line of an mbox message separator reads it: `Tue Oct 13 09:30:00 2026` (UTC).
pub fn asctime(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let weekday = DAYS[(days + 4).rem_euclid(7) as usize];
    format!("{weekday} {} {day:2} {:02}:{:02}:{:02} {year}", MONTHS[(month - 1) as usize], secs / 3600, secs % 3600 / 60, secs % 60)
}

/// Paste as Quotation (Edit menu): every line of `text` starts with `> ` (an empty line with `>`).
pub fn quote_text(text: &str) -> String {
    text.lines().map(|line| if line.is_empty() { ">".to_string() } else { format!("> {line}") }).collect::<Vec<_>>().join("\n")
}

/// The Subject value: leading plain-ASCII words stay readable ("Re: "), the rest from the first word that needs it is
/// one RFC 2047 encoded word.
fn encode_subject(subject: &str) -> String {
    let split = subject.split(' ').scan(0usize, |at, w| { let start = *at; *at += w.len() + 1; Some((start, w)) })
        .find(|(_, w)| !w.is_ascii()).map(|(start, _)| start);
    match split {
        Some(0) | None => encode_word(subject),
        Some(at) => format!("{}{}", &subject[..at], encode_word(&subject[at..])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = "Message-ID: <abc@mail.example>\r\nIn-Reply-To: <root@mail.example>\r\nReferences: <root@mail.example>\r\n <mid@x>\r\nSubject: =?UTF-8?B?RnVuZHVyIMOhIG1vcmd1bg==?=\r\nFrom: Ana <ana@example.com>\r\n\r\nbody <nope@x>";

    #[test]
    fn the_priority_comes_from_x_priority_then_importance() {
        assert_eq!(priority_of(Some("1 (Highest)"), None), Priority::High);
        assert_eq!(priority_of(Some("2"), Some("low")), Priority::High, "X-Priority wins over Importance");
        assert_eq!(priority_of(Some("5 (Lowest)"), None), Priority::Low);
        assert_eq!(priority_of(Some("3"), Some("high")), Priority::Normal, "3 is normal and says nothing more");
        assert_eq!(priority_of(None, Some(" High ")), Priority::High);
        assert_eq!(priority_of(None, Some("low")), Priority::Low);
        assert_eq!(priority_of(None, None), Priority::Normal);
        let high = Summary::from_headers("X-Priority: 1 (Highest)\r\nSubject: Now\r\n\r\nbody");
        assert_eq!(high.priority, Priority::High);
    }

    #[test]
    fn parses_headers_with_folding_and_encoded_subjects() {
        let s = Summary::from_headers(RAW);
        assert_eq!(s.message_id.as_deref(), Some("<abc@mail.example>"));
        assert_eq!(s.in_reply_to, vec!["<root@mail.example>"]);
        assert_eq!(s.references, vec!["<root@mail.example>", "<mid@x>"]);
        assert_eq!(s.subject, "Fundur á morgun");
        assert_eq!(s.from, "Ana <ana@example.com>");
        assert!(parse_headers(RAW).iter().all(|(n, _)| n != "nope"));
    }

    #[test]
    fn ids_are_found_between_brackets() {
        assert_eq!(parse_ids("<a@b> junk <c@d>"), vec!["<a@b>", "<c@d>"]);
        assert!(parse_ids("no ids here").is_empty());
        assert!(parse_ids("<unclosed").is_empty());
    }

    #[test]
    fn address_extraction() {
        assert_eq!(address_of("Ana <ana@example.com>"), "ana@example.com");
        assert_eq!(address_of(" bo@example.com "), "bo@example.com");
    }

    #[test]
    fn dates_match_the_calendar() {
        assert_eq!(rfc2822_date(0), "Thu, 01 Jan 1970 00:00:00 +0000");
        assert_eq!(rfc2822_date(1_000_000_000), "Sun, 09 Sep 2001 01:46:40 +0000");
        assert_eq!(rfc2822_date(951_782_400), "Tue, 29 Feb 2000 00:00:00 +0000");
    }

    #[test]
    fn renders_a_reply_with_threading_headers() {
        let original = Summary::from_headers(RAW);
        let mut draft = reply_draft(&original, "olibuijr@example.com");
        draft.body = "Já\n.dot line".into();
        let text = draft.render("<new@example.com>", 0).unwrap();
        assert!(text.contains("To: ana@example.com\r\n"));
        // the plain words stay readable; the encoded word starts at the first word that needs it
        assert!(text.contains("Subject: Re: Fundur =?UTF-8?B?"), "{text}");
        assert!(text.contains("In-Reply-To: <abc@mail.example>\r\n"));
        assert!(text.contains("References: <root@mail.example> <mid@x> <abc@mail.example>\r\n"));
        assert!(text.ends_with("\r\n\r\nJá\r\n.dot line\r\n"));
    }

    #[test]
    fn render_rejects_bad_drafts() {
        let mut draft = Draft { from: "me@example.com".into(), ..Draft::default() };
        assert!(draft.render("<x@y>", 0).is_err());
        draft.to = vec!["not an address".into()];
        assert!(draft.render("<x@y>", 0).unwrap_err().contains("not a valid address"));
        draft.to = vec!["you@example.com".into()];
        draft.from = "bad".into();
        assert!(draft.render("<x@y>", 0).is_err());
    }

    fn summary(extra: &str) -> Summary {
        Summary::from_headers(&format!("Message-ID: <m@x>\r\nSubject: Plan\r\nFrom: \"Smith, Ana\" <ana@example.com>\r\nTo: me@example.com, Bo <bo@example.com>\r\n{extra}\r\n"))
    }

    #[test]
    fn address_lists_split_outside_quotes_and_brackets() {
        assert_eq!(split_addresses("\"Smith, Ana\" <a@x>, b@x; c@x ,"), vec!["\"Smith, Ana\" <a@x>", "b@x", "c@x"]);
        assert!(split_addresses("  ").is_empty());
    }

    #[test]
    fn reply_all_keeps_everyone_once_and_leaves_the_user_out() {
        let original = summary("Cc: Cy <cy@example.com>, BO@example.com, me@example.com\r\n");
        let draft = reply_all_draft(&original, "Me@example.com");
        assert_eq!(draft.to, vec!["ana@example.com", "bo@example.com"]);
        assert_eq!(draft.cc, vec!["cy@example.com"]);
        assert_eq!(draft.in_reply_to.as_deref(), Some("<m@x>"));
        assert_eq!(draft.subject, "Re: Plan");
    }

    #[test]
    fn a_reply_goes_to_the_reply_to_address_when_there_is_one() {
        let original = summary("Reply-To: list@example.com\r\n");
        assert_eq!(reply_draft(&original, "me@example.com").to, vec!["list@example.com"]);
        assert_eq!(reply_all_draft(&original, "me@example.com").to, vec!["list@example.com", "bo@example.com"]);
    }

    #[test]
    fn a_forward_quotes_the_original_under_a_fwd_subject() {
        let draft = forward_draft(&summary(""), "me@example.com", "Hello");
        assert_eq!(draft.subject, "Fwd: Plan");
        assert!(draft.to.is_empty() && draft.in_reply_to.is_none());
        assert!(draft.body.contains("---------- Forwarded message ----------\nFrom: \"Smith, Ana\" <ana@example.com>"));
        assert!(draft.body.ends_with("\n\nHello"));
        let again = forward_draft(&Summary { subject: "Fwd: Plan".into(), ..Summary::default() }, "me@example.com", "");
        assert_eq!(again.subject, "Fwd: Plan");
    }

    #[test]
    fn bcc_is_sent_but_never_named_and_priority_and_reply_to_are_headers() {
        let draft = Draft {
            from: "me@example.com".into(),
            to: vec!["a@example.com".into()],
            cc: vec!["c@example.com".into()],
            bcc: vec!["secret@example.com".into()],
            reply_to: vec!["back@example.com".into()],
            priority: Priority::High,
            ..Draft::default()
        };
        let text = draft.render("<x@y>", 0).unwrap();
        assert!(!text.contains("secret@example.com"));
        assert!(text.contains("Cc: c@example.com\r\n") && text.contains("Reply-To: back@example.com\r\n"));
        assert!(text.contains("X-Priority: 1 (Highest)\r\nImportance: high\r\n"));
        assert_eq!(draft.envelope_recipients(), vec!["a@example.com", "c@example.com", "secret@example.com"]);
        let plain = Draft { priority: Priority::Normal, ..draft.clone() }.render("<x@y>", 0).unwrap();
        assert!(!plain.contains("X-Priority"));
        assert!(Draft { bcc: vec!["bad".into()], ..draft }.render("<x@y>", 0).is_err());
    }

    #[test]
    fn a_quotation_prefixes_every_line() {
        assert_eq!(quote_text("one\n\ntwo"), "> one\n>\n> two");
        assert_eq!(quote_text(""), "");
    }

    fn file(name: &str, mime: &str, bytes: &[u8]) -> Attachment {
        Attachment { name: name.into(), mime: mime.into(), bytes: bytes.to_vec() }
    }

    fn draft_with(files: Vec<Attachment>) -> Draft {
        Draft { from: "me@example.com".into(), to: vec!["a@example.com".into()], subject: "Skjal".into(), body: "See the file".into(), attachments: files, ..Draft::default() }
    }

    #[test]
    fn a_message_with_files_is_multipart_and_reads_back_through_the_body_parser() {
        let text = draft_with(vec![file("skjal.pdf", "application/pdf", b"hello world"), file("ísland.txt", "text/plain", "á\n".as_bytes())])
            .render("<id1@example.com>", 0)
            .unwrap();
        assert!(text.contains("Content-Type: multipart/mixed; boundary=\"=_bifrost_id1examplecom\"\r\n\r\n"), "{text}");
        assert!(text.ends_with("--=_bifrost_id1examplecom--\r\n"));
        assert_eq!(crate::body::readable(&text), "See the file");
        let files = crate::body::attachments(&text);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0], file("skjal.pdf", "application/pdf", b"hello world"));
        assert_eq!(files[1].name, "ísland.txt");
        assert_eq!(files[1].bytes, "á\n".as_bytes());
    }

    #[test]
    fn base64_lines_stay_within_76_characters() {
        let text = draft_with(vec![file("big.bin", "application/octet-stream", &[7u8; 500])]).render("<x@y>", 0).unwrap();
        assert!(!text.split("\r\n").any(|l| l.len() > 76 && l.chars().all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))), "{text}");
        assert_eq!(crate::body::attachments(&text)[0].bytes, vec![7u8; 500]);
    }

    #[test]
    fn a_file_name_cannot_break_out_of_its_header() {
        let text = draft_with(vec![file("a\"b\r\nBcc: x@y.is", "text/plain; evil=1", b"x")]).render("<x@y>", 0).unwrap();
        assert!(!text.contains("\r\nBcc:"), "{text}");
        assert!(text.contains("Content-Type: application/octet-stream; name=\"a_b__Bcc: x@y.is\""), "{text}");
    }

    #[test]
    fn a_boundary_that_the_text_contains_is_lengthened() {
        let mut draft = draft_with(vec![file("a.txt", "text/plain", b"x")]);
        draft.body = "=_bifrost_x1".into();
        let text = draft.render("<x1>", 0).unwrap();
        assert!(text.contains("boundary=\"=_bifrost_x1x\""), "{text}");
    }

    #[test]
    fn forward_as_attachment_carries_the_original_as_a_message_part() {
        let original = summary("");
        let raw = "Subject: Plan\nFrom: ana@example.com\n\nHello\n";
        let draft = forward_attachment_draft(&original, "me@example.com", raw);
        assert_eq!(draft.subject, "Fwd: Plan");
        assert!(draft.body.is_empty() && draft.to.is_empty());
        assert_eq!(draft.attachments.len(), 1);
        assert_eq!(draft.attachments[0].name, "Plan.eml");
        assert_eq!(draft.attachments[0].mime, "message/rfc822");
        let sent = Draft { to: vec!["bo@example.com".into()], ..draft }.render("<f@x>", 0).unwrap();
        assert!(sent.contains("Content-Type: message/rfc822; name=\"Plan.eml\"\r\n"));
        assert!(sent.contains("Content-Transfer-Encoding: 8bit\r\n\r\nSubject: Plan\r\nFrom: ana@example.com\r\n\r\nHello\r\n--"), "{sent}");
    }

    #[test]
    fn a_saved_draft_reopens_with_its_fields_and_threading() {
        let raw = "Message-ID: <d1@x>\r\nFrom: Ana <ana@example.com>\r\nTo: bo@example.com, cy@example.com\r\nCc: di@example.com\r\nBcc: ed@example.com\r\nSubject: Halló\r\nIn-Reply-To: <orig@x>\r\nReferences: <root@x> <orig@x>\r\nX-Priority: 1\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello there\r\n";
        let d = draft_of(raw);
        assert_eq!(d.to.len(), 2);
        assert_eq!(d.cc.len(), 1);
        assert_eq!(d.bcc.len(), 1);
        assert_eq!(d.subject, "Halló");
        assert!(d.body.contains("Hello there"));
        assert_eq!(d.in_reply_to.as_deref(), Some("<orig@x>"));
        assert_eq!(d.references, vec!["<root@x>".to_string(), "<orig@x>".to_string()]);
        assert_eq!(d.priority, Priority::High);
        assert!(d.attachments.is_empty());
    }

    #[test]
    fn redirect_puts_resent_headers_in_front_of_the_untouched_original() {
        let raw = "From: ana@example.com\nSubject: Plan\n\nHello\n";
        let out = redirect_message(raw, "me@example.com", &["bo@example.com".to_string(), "cy@example.com".to_string()], "<r@x>", 0);
        assert!(out.starts_with("Resent-Date: Thu, 01 Jan 1970 00:00:00 +0000\r\nResent-From: me@example.com\r\nResent-To: bo@example.com, cy@example.com\r\nResent-Message-ID: <r@x>\r\nFrom: ana@example.com\r\n"));
        assert!(out.ends_with("Subject: Plan\r\n\r\nHello\r\n"));
    }

    #[test]
    fn summaries_read_the_bulk_mail_headers() {
        let s = Summary::from_headers("List-Unsubscribe: <mailto:u@x>\r\nList-Id: <news.x>\r\nPrecedence: Bulk\r\nAuto-Submitted: auto-generated\r\nContent-Type: Multipart/Mixed; boundary=x\r\n\r\n");
        assert_eq!((s.list_unsubscribe.as_str(), s.list_id.as_str(), s.precedence.as_str()), ("<mailto:u@x>", "<news.x>", "bulk"));
        assert!(s.automatic && s.may_have_files());
        let plain = Summary::from_headers("Auto-Submitted: no\r\nContent-Type: text/plain\r\n\r\n");
        assert!(!plain.automatic && !plain.may_have_files());
    }

    #[test]
    fn the_time_of_day_is_read_in_utc() {
        assert_eq!(time_of_day(0), "00:00:00");
        assert_eq!(time_of_day(1_000_000_000), "01:46:40");
        assert_eq!(time_of_day(-1), "23:59:59");
    }

    #[test]
    fn asctime_matches_the_calendar() {
        assert_eq!(asctime(0), "Thu Jan  1 00:00:00 1970");
        assert_eq!(asctime(1_000_000_000), "Sun Sep  9 01:46:40 2001");
    }

    #[test]
    fn a_styled_message_goes_as_plain_text_and_html_alternatives() {
        let draft = Draft { html: Some("<p>Hi <b>Ana</b></p>".into()), ..draft_with(Vec::new()) };
        let text = draft.render("<h@x>", 0).unwrap();
        assert!(text.contains("Content-Type: multipart/alternative; boundary=\"=_alt_hx\"\r\n\r\n--=_alt_hx\r\nContent-Type: text/plain"), "{text}");
        assert!(text.contains("Content-Type: text/html; charset=UTF-8"));
        assert!(text.ends_with("<p>Hi <b>Ana</b></p>\r\n--=_alt_hx--\r\n"), "{text}");
        assert_eq!(crate::body::readable(&text), "See the file", "a reader that shows text gets the plain part");
        assert!(crate::body::attachments(&text).is_empty());
    }

    #[test]
    fn a_styled_message_with_files_nests_the_alternatives_in_the_mixed_message() {
        let draft = Draft { html: Some("<b>x</b>".into()), ..draft_with(vec![file("a.txt", "text/plain", b"hello")]) };
        let text = draft.render("<n@x>", 0).unwrap();
        assert!(text.contains("Content-Type: multipart/mixed; boundary=\"=_bifrost_nx\""), "{text}");
        assert!(text.contains("--=_bifrost_nx\r\nContent-Type: multipart/alternative; boundary=\"=_alt_nx\""), "{text}");
        assert!(text.ends_with("--=_bifrost_nx--\r\n"));
        assert_eq!(crate::body::readable(&text), "See the file");
        let files = crate::body::attachments(&text);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].bytes, b"hello".to_vec());
        let stripped = crate::mime::strip_attachments(&text);
        assert_eq!(stripped.removed, 1);
        assert!(stripped.raw.contains("<b>x</b>"), "Remove Attachments keeps the HTML part");
    }

    #[test]
    fn a_plain_message_has_no_alternative() {
        let text = draft_with(Vec::new()).render("<p@x>", 0).unwrap();
        assert!(text.contains("Content-Type: text/plain; charset=UTF-8\r\n") && !text.contains("multipart"));
    }
}
