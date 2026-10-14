//! Import and export of mailboxes in the mbox format (the `mboxrd` variant): every message starts with a
//! `From sender date` line, and body lines that begin with `From ` (after any number of `>`) get one more `>` on
//! write and lose one on read, so a message round-trips unchanged. Pure std; the window adds the open and save panels.

/// One message of an mbox file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The sender address from the separator line (`-` when unknown).
    pub sender: String,
    /// The date text from the separator line (asctime style, for example `Tue Oct 13 09:30:00 2026`).
    pub date: String,
    /// The message with `\n` line ends and no separator.
    pub raw: String,
}

/// True for a line that must be escaped: `From ` after zero or more `>`.
fn from_line(line: &str) -> bool {
    line.trim_start_matches('>').starts_with("From ")
}

/// Writes `entries` as one mbox text. Each message ends with a blank line; CRLF in a message becomes LF.
pub fn write(entries: &[Entry]) -> String {
    let mut out = String::new();
    for e in entries {
        let sender = if e.sender.trim().is_empty() { "-" } else { e.sender.trim() };
        let sender = sender.replace(char::is_whitespace, "");
        out.push_str(&format!("From {sender} {}\n", e.date.trim()));
        let raw = e.raw.replace("\r\n", "\n").replace('\r', "\n");
        let raw = raw.trim_end_matches('\n');
        for line in raw.split('\n') {
            if from_line(line) {
                out.push('>');
            }
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// Reads an mbox text. Text before the first `From ` line is ignored. A message's trailing blank line (the
/// separator) is removed, so `read(write(x)) == x` up to line ends and trailing blank lines.
pub fn read(text: &str) -> Vec<Entry> {
    let text = text.replace("\r\n", "\n");
    let mut out: Vec<Entry> = vec![];
    let mut current: Option<(String, String, Vec<String>)> = None;
    for line in text.split('\n') {
        if line.starts_with("From ") && !line.starts_with("From:") {
            if let Some(c) = current.take() {
                out.push(finish(c));
            }
            let rest = &line[5..];
            let (sender, date) = rest.split_once(' ').unwrap_or((rest, ""));
            current = Some((sender.to_string(), date.trim().to_string(), vec![]));
        } else if let Some((_, _, lines)) = current.as_mut() {
            let line = if line.starts_with('>') && from_line(line) { &line[1..] } else { line };
            lines.push(line.to_string());
        }
    }
    if let Some(c) = current.take() {
        out.push(finish(c));
    }
    out
}

fn finish((sender, date, mut lines): (String, String, Vec<String>)) -> Entry {
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    Entry { sender, date, raw: lines.join("\n") }
}

/// The asctime date for a separator line from a Unix time (UTC).
pub fn asctime(unix: i64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{} {} {:2} {:02}:{:02}:{:02} {}",
        DAYS[(days + 4).rem_euclid(7) as usize],
        MONTHS[month as usize - 1],
        day,
        secs / 3600,
        secs % 3600 / 60,
        secs % 60,
        year
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sender: &str, raw: &str) -> Entry {
        Entry { sender: sender.into(), date: "Tue Oct 13 09:30:00 2026".into(), raw: raw.into() }
    }

    #[test]
    fn round_trips_several_messages() {
        let a = entry("a@x.is", "Subject: one\n\nhello\nworld");
        let b = entry("b@y.is", "Subject: two\n\nbye");
        let text = write(&[a.clone(), b.clone()]);
        assert!(text.starts_with("From a@x.is Tue Oct 13 09:30:00 2026\n"));
        assert_eq!(read(&text), vec![a, b]);
    }

    #[test]
    fn from_lines_in_bodies_are_quoted_and_restored() {
        let m = entry("a@x.is", "Subject: s\n\nFrom here on\n>From quoted\nplain");
        let text = write(&[m.clone()]);
        assert!(text.contains("\n>From here on\n"));
        assert!(text.contains("\n>>From quoted\n"));
        assert_eq!(read(&text), vec![m]);
    }

    #[test]
    fn header_named_from_is_not_a_separator() {
        let m = entry("a@x.is", "From: A <a@x.is>\nSubject: s\n\nbody");
        assert_eq!(read(&write(&[m.clone()])), vec![m]);
    }

    #[test]
    fn crlf_and_missing_sender() {
        let text = write(&[entry("  ", "Subject: s\r\n\r\nbody\r\n\r\n")]);
        assert!(text.starts_with("From - "));
        assert_eq!(read(&text)[0].raw, "Subject: s\n\nbody");
        assert_eq!(read("junk before\nFrom x@y Mon Jan 1 00:00:00 2024\nA: b\n")[0].raw, "A: b");
        assert!(read("").is_empty());
    }

    #[test]
    fn asctime_formats_utc() {
        assert_eq!(asctime(0), "Thu Jan  1 00:00:00 1970");
        assert_eq!(asctime(1_792_065_600), "Thu Oct 15 12:00:00 2026");
    }
}
