//! RFC 5322 messages: the header parser that reads a message's headers for the list and threads, and the
//! renderer that turns a compose draft into the text SMTP sends.

use crate::account::valid_address;
use crate::codec::{decode_words, encode_word};

/// The headers of one message, as the list and the threads need them. Names are matched without case.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub message_id: Option<String>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    pub subject: String,
    pub from: String,
    pub to: String,
    pub date: String,
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
            date: get("date").unwrap_or_default().to_string(),
        }
    }
}

/// A message being written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
    /// The Message-ID being answered, when this is a reply.
    pub in_reply_to: Option<String>,
    /// The References chain of the message being answered, oldest first.
    pub references: Vec<String>,
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
        for addr in self.to.iter().chain(self.cc.iter()) {
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
        out.push_str(&format!("Subject: {}\r\n", encode_subject(&self.subject)));
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
        out.push_str("Content-Type: text/plain; charset=UTF-8\r\n");
        out.push_str("Content-Transfer-Encoding: 8bit\r\n\r\n");
        for line in self.body.split('\n') {
            out.push_str(line.strip_suffix('\r').unwrap_or(line));
            out.push_str("\r\n");
        }
        Ok(out)
    }

    /// Every address the message goes to, for the SMTP RCPT commands.
    pub fn envelope_recipients(&self) -> Vec<&str> {
        self.to.iter().chain(self.cc.iter()).map(String::as_str).collect()
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
    let mut references = original.references.clone();
    if references.is_empty() {
        references = original.in_reply_to.clone();
    }
    Draft {
        from: from.to_string(),
        to: vec![address_of(&original.from).to_string()],
        subject,
        in_reply_to: original.message_id.clone(),
        references,
        ..Draft::default()
    }
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
}
