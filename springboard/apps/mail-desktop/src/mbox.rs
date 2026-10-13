//! Mailbox files (Mailbox > Import Mailbox and Export Mailbox): the mbox format in its `mboxrd` form. A file is a run of
//! messages, each introduced by a `From ` line; inside a message every line that begins with `From ` (after any number of
//! `>`) gets one more `>` so it cannot be taken for a separator, and reading takes that `>` off again. Lines end in LF in
//! the file; messages come back with CRLF line ends, as the server holds them.

use crate::message::{address_of, asctime, parse_headers};

/// The `From ` separator line of a message: the sender's address (`MAILER-DAEMON` when it has none) and a date.
fn separator(raw: &str, unix: i64) -> String {
    let headers = parse_headers(raw);
    let sender = headers.iter().find(|(n, _)| n == "from").map(|(_, v)| address_of(v).to_string()).unwrap_or_default();
    let sender = if sender.is_empty() || sender.contains(char::is_whitespace) { "MAILER-DAEMON".to_string() } else { sender };
    format!("From {sender} {}", asctime(unix))
}

/// True for a line that must be escaped: `From ` after zero or more `>`.
fn is_from_line(line: &str) -> bool {
    line.trim_start_matches('>').starts_with("From ")
}

/// The mbox file text for these messages (CRLF or LF source lines both accepted). `unix` dates the separator lines.
pub fn write(messages: &[String], unix: i64) -> String {
    let mut out = String::new();
    for raw in messages {
        out.push_str(&separator(raw, unix));
        out.push('\n');
        let text = raw.replace("\r\n", "\n");
        let text = text.trim_end_matches('\n');
        for line in text.split('\n') {
            if is_from_line(line) {
                out.push('>');
            }
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// The messages in an mbox file, each with CRLF line ends and without the separator line. Text before the first
/// separator is ignored. A separator is a `From ` line at the start of the file or after an empty line.
pub fn read(text: &str) -> Vec<String> {
    let mut messages: Vec<Vec<&str>> = Vec::new();
    let mut previous_blank = true;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if (previous_blank || messages.is_empty()) && line.starts_with("From ") {
            messages.push(Vec::new());
        } else if let Some(current) = messages.last_mut() {
            current.push(line);
        }
        previous_blank = line.is_empty();
    }
    messages
        .into_iter()
        .map(|mut lines| {
            while lines.last().is_some_and(|l| l.is_empty()) {
                lines.pop();
            }
            let mut raw = String::new();
            for line in lines {
                raw.push_str(line.strip_prefix('>').filter(|_| is_from_line(line)).unwrap_or(line));
                raw.push_str("\r\n");
            }
            raw
        })
        .filter(|raw| !raw.is_empty())
        .collect()
}

/// A file name for an exported mailbox: the last part of its name, made safe, ending in `.mbox`.
pub fn file_name(mailbox: &str) -> String {
    let last = mailbox.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(mailbox);
    let base = if last.eq_ignore_ascii_case("inbox") { "Inbox" } else { last };
    format!("{}.mbox", crate::attachments::safe_name(base))
}

/// The mailbox an imported file goes into: the file's name without its extension, made safe for a mailbox name.
/// A file with no usable name goes into `Imported`.
pub fn mailbox_name(file_name: &str) -> String {
    let stem = file_name.rsplit_once('.').map_or(file_name, |(stem, _)| stem);
    let name: String = crate::attachments::safe_name(stem).chars().filter(|c| !matches!(c, '"' | '*' | '%' | '\\')).take(60).collect();
    let name = name.trim();
    if name.is_empty() || name == "attachment" {
        "Imported".to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(from: &str, subject: &str, body: &str) -> String {
        format!("From: {from}\r\nSubject: {subject}\r\n\r\n{body}\r\n")
    }

    #[test]
    fn messages_round_trip_through_the_file() {
        let messages = vec![mail("Ana <ana@example.com>", "One", "first\r\nsecond"), mail("bo@example.com", "Two", "")];
        let file = write(&messages, 0);
        assert!(file.starts_with("From ana@example.com Thu Jan  1 00:00:00 1970\nFrom: Ana <ana@example.com>\n"), "{file}");
        assert!(file.contains("\nFrom bo@example.com Thu Jan  1 00:00:00 1970\n"));
        let back = read(&file);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0], messages[0]);
        assert_eq!(back[1], "From: bo@example.com\r\nSubject: Two\r\n");
    }

    #[test]
    fn a_from_line_in_the_text_is_escaped_and_restored() {
        let messages = vec![mail("a@example.com", "x", "hello\r\n\r\nFrom the desk\r\n>From quoted\r\n>>From twice")];
        let file = write(&messages, 0);
        assert!(file.contains("\n>From the desk\n>>From quoted\n>>>From twice\n"), "{file}");
        assert_eq!(read(&file), messages);
    }

    #[test]
    fn reading_ignores_text_before_the_first_separator_and_accepts_lf_files() {
        let file = "junk line\nFrom a@x Mon Jan  1 00:00:00 2024\nSubject: a\n\nbody\n\nFrom b@x Mon Jan  1 00:00:00 2024\nSubject: b\n\nbody2\n";
        let back = read(file);
        assert_eq!(back, vec!["Subject: a\r\n\r\nbody\r\n".to_string(), "Subject: b\r\n\r\nbody2\r\n".to_string()]);
        assert!(read("no separator at all\n").is_empty());
        assert!(read("").is_empty());
    }

    #[test]
    fn a_sender_that_is_not_an_address_becomes_the_daemon() {
        assert!(write(&["Subject: x\r\n\r\nbody\r\n".to_string()], 0).starts_with("From MAILER-DAEMON "));
        assert!(write(&["From: Ana Smith\r\n\r\nbody\r\n".to_string()], 0).starts_with("From MAILER-DAEMON "));
    }

    #[test]
    fn an_imported_file_goes_into_a_mailbox_named_after_it() {
        assert_eq!(mailbox_name("Archive.mbox"), "Archive");
        assert_eq!(mailbox_name("my.old.mail.mbox"), "my.old.mail");
        assert_eq!(mailbox_name("Work/2024.mbox"), "Work_2024");
        assert_eq!(mailbox_name(".mbox"), "Imported");
        assert_eq!(mailbox_name("  .mbox"), "Imported");
        assert_eq!(mailbox_name("a\"b*c.mbox"), "abc");
    }

    #[test]
    fn export_names_come_from_the_last_part_of_the_mailbox() {
        assert_eq!(file_name("INBOX"), "Inbox.mbox");
        assert_eq!(file_name("Work/Receipts"), "Receipts.mbox");
        assert_eq!(file_name("INBOX.Old Mail"), "INBOX.Old Mail.mbox");
        assert_eq!(file_name("../x"), "x.mbox");
    }
}
