//! RFC 5322 messages: the header parser that reads a message's headers for the list and threads, and the
//! renderer that turns a compose draft into the text SMTP sends.

use bifrost_app::controls::internet_accounts::valid_address;
use crate::body::Attachment;
use crate::category::{self, Category};
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
    /// The group the headers and the sender put the message in (View > Category).
    pub category: Category,
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
    /// This summary with its category worked out from the parsed headers.
    fn categorised(mut self, headers: &[(String, String)]) -> Summary {
        self.category = category::classify(headers, &self.from, &self.subject);
        self
    }

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
            category: Category::default(),
        }.categorised(&headers)
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
    /// Files sent with the message (File > Attach Files, Forward as Attachment).
    pub attachments: Vec<Attachment>,
}

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
        if self.attachments.is_empty() {
            out.push_str("Content-Type: text/plain; charset=UTF-8\r\n");
            out.push_str("Content-Transfer-Encoding: 8bit\r\n\r\n");
            push_text(&mut out, &self.body);
            return Ok(out);
        }
        let boundary = boundary_for(message_id);
        out.push_str(&format!("Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n"));
        out.push_str(&format!("--{boundary}\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n"));
        push_text(&mut out, &self.body);
        for file in &self.attachments {
            out.push_str(&attachment_part(&boundary, file));
        }
        out.push_str(&format!("--{boundary}--\r\n"));
        Ok(out)
    }

    /// Every address the message goes to (To, Cc and Bcc), for the SMTP RCPT commands.
    pub fn envelope_recipients(&self) -> Vec<&str> {
        self.to.iter().chain(&self.cc).chain(&self.bcc).map(String::as_str).collect()
    }
}

/// Body text with CRLF line ends.
fn push_text(out: &mut String, text: &str) {
    for line in text.split('\n') {
        out.push_str(line.strip_suffix('\r').unwrap_or(line));
        out.push_str("\r\n");
    }
}

/// A MIME boundary made from the message id, so the same message always gets the same one.
fn boundary_for(message_id: &str) -> String {
    let id: String = message_id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    format!("=_bifrost_{id}")
}

/// A file name that can stand in a quoted header parameter: quotes, backslashes and line breaks become `_`, and a name
/// with non-ASCII characters is sent as an encoded word.
fn header_file_name(name: &str) -> String {
    let plain: String = name.chars().map(|c| if c == '"' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    encode_word(&plain)
}

/// One attachment as a MIME part, closed by its own line end, behind a `--boundary` line. A forwarded message
/// (`message/rfc822`) is sent as it is; other files are base64 in lines of 76 characters.
fn attachment_part(boundary: &str, file: &Attachment) -> String {
    let name = header_file_name(&file.name);
    let mime = if file.mime.is_empty() || file.mime.contains(['\r', '\n']) { "application/octet-stream" } else { file.mime.as_str() };
    let mut out = format!("--{boundary}\r\nContent-Type: {mime}; name=\"{name}\"\r\n");
    if mime.eq_ignore_ascii_case("message/rfc822") {
        out.push_str(&format!("Content-Transfer-Encoding: 8bit\r\nContent-Disposition: attachment; filename=\"{name}\"\r\n\r\n"));
        push_text(&mut out, &String::from_utf8_lossy(&file.bytes));
        return out;
    }
    out.push_str(&format!("Content-Transfer-Encoding: base64\r\nContent-Disposition: attachment; filename=\"{name}\"\r\n\r\n"));
    let encoded = base64_encode(&file.bytes);
    for chunk in encoded.as_bytes().chunks(76) {
        out.push_str(&String::from_utf8_lossy(chunk));
        out.push_str("\r\n");
    }
    out
}

/// A contact's names and address from a `Name <addr>` value: the given name is the first word of the display name, the
/// family name the rest. Without a display name the part of the address before the `@` is the given name.
pub fn contact_parts(from: &str) -> (String, String, String) {
    let email = address_of(from).to_string();
    let name = from.split('<').next().unwrap_or("").trim().trim_matches('"').trim();
    let name = if name.contains('@') { "" } else { name };
    if name.is_empty() {
        let local = email.split('@').next().unwrap_or("").to_string();
        return (local, String::new(), email);
    }
    match name.split_once(' ') {
        Some((given, family)) => (given.to_string(), family.trim().to_string(), email),
        None => (name.to_string(), String::new(), email),
    }
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
/// a `Fwd:` subject, with an empty body.
pub fn forward_attachment_draft(original: &Summary, from: &str, raw: &str) -> Draft {
    let mut draft = forward_draft(original, from, "");
    draft.body = String::new();
    draft.attachments = vec![Attachment {
        name: crate::export::eml_name(&original.subject),
        mime: "message/rfc822".into(),
        bytes: crate::export::crlf(raw).into_bytes(),
    }];
    draft
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
    fn forward_as_attachment_carries_the_message_and_no_text() {
        let original = Summary { subject: "Plan".into(), ..Summary::default() };
        let draft = forward_attachment_draft(&original, "me@example.com", "Subject: Plan\n\nhi\n");
        assert_eq!(draft.subject, "Fwd: Plan");
        assert!(draft.body.is_empty());
        assert_eq!(draft.attachments.len(), 1);
        assert_eq!(draft.attachments[0].name, "Plan.eml");
        assert_eq!(draft.attachments[0].mime, "message/rfc822");
        assert_eq!(draft.attachments[0].bytes, b"Subject: Plan\r\n\r\nhi\r\n".to_vec());
    }

    #[test]
    fn a_quotation_prefixes_every_line() {
        assert_eq!(quote_text("one\n\ntwo"), "> one\n>\n> two");
        assert_eq!(quote_text(""), "");
    }

    #[test]
    fn attachments_make_a_multipart_message() {
        let draft = Draft {
            from: "me@example.com".into(),
            to: vec!["a@example.com".into()],
            body: "See attached".into(),
            attachments: vec![Attachment { name: "a\"b.txt".into(), mime: "text/plain".into(), bytes: b"hello".to_vec() }],
            ..Draft::default()
        };
        let text = draft.render("<1.2@x.is>", 0).unwrap();
        assert!(text.contains("Content-Type: multipart/mixed; boundary=\"=_bifrost_12xis\"\r\n\r\n--=_bifrost_12xis\r\n"));
        assert!(text.contains("See attached\r\n--=_bifrost_12xis\r\nContent-Type: text/plain; name=\"a_b.txt\"\r\n"));
        assert!(text.contains("Content-Transfer-Encoding: base64\r\nContent-Disposition: attachment; filename=\"a_b.txt\"\r\n\r\naGVsbG8=\r\n"));
        assert!(text.ends_with("--=_bifrost_12xis--\r\n"));
        let plain = Draft { attachments: vec![], ..draft }.render("<1.2@x.is>", 0).unwrap();
        assert!(plain.contains("Content-Type: text/plain; charset=UTF-8") && !plain.contains("multipart"));
    }

    #[test]
    fn long_files_are_wrapped_at_76_characters_and_forwarded_messages_stay_text() {
        let big = Attachment { name: "big.bin".into(), mime: String::new(), bytes: vec![7u8; 200] };
        let part = attachment_part("B", &big);
        assert!(part.contains("Content-Type: application/octet-stream; name=\"big.bin\""));
        assert!(part.lines().all(|l| l.len() <= 76 || l.starts_with("Content") || l.starts_with("--")));
        let forwarded = Attachment { name: "Plan.eml".into(), mime: "message/rfc822".into(), bytes: b"Subject: Plan\n\nhi\n".to_vec() };
        let part = attachment_part("B", &forwarded);
        assert!(part.contains("Content-Transfer-Encoding: 8bit") && part.ends_with("Subject: Plan\r\n\r\nhi\r\n\r\n"));
    }

    #[test]
    fn a_contact_is_made_from_a_sender() {
        assert_eq!(contact_parts("Ana María O <ana@example.com>"), ("Ana".into(), "María O".into(), "ana@example.com".into()));
        assert_eq!(contact_parts("\"Bo\" <bo@example.com>"), ("Bo".into(), String::new(), "bo@example.com".into()));
        assert_eq!(contact_parts("cy@example.com"), ("cy".into(), String::new(), "cy@example.com".into()));
    }
}
