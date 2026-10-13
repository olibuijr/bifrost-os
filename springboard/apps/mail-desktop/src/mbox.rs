//! The mbox file format (the `mboxrd` variant): many messages in one text file, each started by a `From ` line. Lines in
//! a message that begin with `From ` (after any number of `>`) get one more `>` when written and lose one when read,
//! so a message comes back exactly as it went in.

/// The separator line written before every message.
const SEPARATOR: &str = "From MAILER-DAEMON Thu Jan  1 00:00:00 1970";

/// True for a body line that must be escaped: zero or more `>` and then `From `.
fn needs_escape(line: &str) -> bool {
    line.trim_start_matches('>').starts_with("From ")
}

/// The mbox text for `messages` (raw RFC 5322 text, any line ends). Every message is followed by one blank line.
pub fn write(messages: &[String]) -> String {
    let mut out = String::new();
    for message in messages {
        out.push_str(SEPARATOR);
        out.push('\n');
        let text = message.replace("\r\n", "\n").replace('\r', "\n");
        let text = text.strip_suffix('\n').unwrap_or(&text);
        for line in text.split('\n') {
            if needs_escape(line) {
                out.push('>');
            }
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// The messages in mbox text, with `\n` line ends and the escaping undone. Text before the first `From ` line is
/// ignored.
pub fn read(text: &str) -> Vec<String> {
    let normalised = text.replace("\r\n", "\n");
    let mut messages: Vec<Vec<&str>> = Vec::new();
    for line in normalised.split('\n') {
        if line.starts_with("From ") {
            messages.push(Vec::new());
        } else if let Some(current) = messages.last_mut() {
            current.push(line);
        }
    }
    messages
        .into_iter()
        .map(|mut lines| {
            // the blank line the writer adds after each message, and the empty piece after the final newline
            while lines.last() == Some(&"") {
                lines.pop();
                if lines.last().is_some_and(|l| !l.is_empty()) {
                    break;
                }
            }
            let mut out = String::new();
            for line in lines {
                out.push_str(if line.starts_with('>') && needs_escape(line) { &line[1..] } else { line });
                out.push('\n');
            }
            out
        })
        .filter(|m| !m.trim().is_empty())
        .collect()
}

/// The file name for an exported mailbox: its last name part made safe, ending in `.mbox`.
pub fn file_name(mailbox: &str) -> String {
    let last = mailbox.rsplit(['/', '.']).next().unwrap_or(mailbox);
    let base = if last.trim().is_empty() { "Mailbox" } else { last };
    let safe = crate::attachments::safe_name(if base.eq_ignore_ascii_case("INBOX") { "Inbox" } else { base });
    format!("{safe}.mbox")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_through_the_file() {
        let a = "Subject: one\n\nhello\n".to_string();
        let b = "Subject: two\n\nbye\n".to_string();
        let text = write(&[a.clone(), b.clone()]);
        assert_eq!(text.matches(SEPARATOR).count(), 2);
        assert_eq!(read(&text), vec![a, b]);
    }

    #[test]
    fn from_lines_in_a_body_are_escaped_and_restored() {
        let message = "Subject: x\n\nFrom here on\n>From quoted\nplain\n".to_string();
        let text = write(&[message.clone()]);
        assert!(text.contains("\n>From here on\n"));
        assert!(text.contains("\n>>From quoted\n"));
        assert_eq!(read(&text), vec![message]);
    }

    #[test]
    fn crlf_input_is_written_with_plain_line_ends() {
        let text = write(&["Subject: x\r\n\r\nbody\r\n".to_string()]);
        assert!(!text.contains('\r'));
        assert_eq!(read(&text), vec!["Subject: x\n\nbody\n".to_string()]);
    }

    #[test]
    fn text_before_the_first_separator_is_ignored() {
        assert_eq!(read("junk\nFrom a@b Mon\nSubject: y\n\nz\n\n"), vec!["Subject: y\n\nz\n".to_string()]);
        assert!(read("no separators here").is_empty());
    }

    #[test]
    fn exported_files_are_named_after_the_mailbox() {
        assert_eq!(file_name("INBOX"), "Inbox.mbox");
        assert_eq!(file_name("Work/Receipts"), "Receipts.mbox");
        assert_eq!(file_name(""), "Mailbox.mbox");
    }
}
