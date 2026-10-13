//! Log files and archives the user opened (File > Open) or captured (File > Capture Archive): reading them into
//! entries. A file may be gzip compressed. The formats are the journal export (what `journalctl -o export` writes and
//! the archive capture saves), JSON lines (`journalctl -o json`) and plain text, one message per line. No window code
//! here: bytes in, entries out.

use crate::model::{self, Level, LogEntry, MAX_ENTRIES};
use bifrost_app::compression::{decode_buffer_limited, Algorithm};
use bifrost_app::proto::GrantId;

/// The most bytes read from a chosen file, and the most a compressed one may grow to.
pub const MAX_INPUT: usize = 64 << 20;
const MAX_OUTPUT: usize = 128 << 20;
/// The longest single field an export record may hold. A longer one marks the file as damaged.
const MAX_FIELD: usize = 16 << 20;

/// A log the window shows from the sidebar's Opened Files section.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenedLog {
    pub name: String,
    pub entries: Vec<LogEntry>,
    /// The document grant for the file, which Reveal in Files and Move to Trash use. None when the app holds none.
    pub grant: Option<GrantId>,
    /// Whether the grant allows writing, which Move to Trash needs.
    pub writable: bool,
}

/// The newest `MAX_ENTRIES` of `entries` (they are oldest first).
fn newest(mut entries: Vec<LogEntry>) -> Vec<LogEntry> {
    if entries.len() > MAX_ENTRIES {
        entries.drain(..entries.len() - MAX_ENTRIES);
    }
    entries
}

/// Unpacks a gzip file (RFC 1952) with a single member. The header's optional parts are skipped, the deflate data is
/// decoded and the stored size is checked against what came out.
pub fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    const HCRC: u8 = 2;
    const EXTRA: u8 = 4;
    const NAME: u8 = 8;
    const COMMENT: u8 = 16;
    if bytes.len() < 18 || bytes[..2] != [0x1f, 0x8b] || bytes[2] != 8 {
        return Err("This is not a gzip file.".into());
    }
    let flags = bytes[3];
    let mut pos = 10;
    let truncated = || "The compressed file ends early.".to_string();
    if flags & EXTRA != 0 {
        let len = bytes.get(pos..pos + 2).ok_or_else(truncated)?;
        pos += 2 + u16::from_le_bytes([len[0], len[1]]) as usize;
    }
    for flag in [NAME, COMMENT] {
        if flags & flag != 0 {
            let end = bytes.get(pos..).and_then(|rest| rest.iter().position(|b| *b == 0)).ok_or_else(truncated)?;
            pos += end + 1;
        }
    }
    if flags & HCRC != 0 {
        pos += 2;
    }
    if pos + 8 > bytes.len() {
        return Err(truncated());
    }
    let body = &bytes[pos..bytes.len() - 8];
    let out = decode_buffer_limited(Algorithm::Deflate, body, MAX_OUTPUT).map_err(|e| format!("Could not decompress the file: {e}."))?;
    let stored = u32::from_le_bytes([bytes[bytes.len() - 4], bytes[bytes.len() - 3], bytes[bytes.len() - 2], bytes[bytes.len() - 1]]);
    if stored != out.len() as u32 {
        return Err("The compressed file is damaged or has more than one part.".into());
    }
    Ok(out)
}

/// Reads journal export records as they arrive, so a capture streams through without holding the whole boot. Feed it
/// chunks of any size; `finish` gives the newest `MAX_ENTRIES` entries.
///
/// A record is lines of `FIELD=value`; a value with a newline or binary data is `FIELD`, a newline, eight bytes of
/// little-endian length, the data and a newline. A blank line ends the record.
#[derive(Default)]
pub struct ExportParser {
    buf: Vec<u8>,
    fields: Vec<(String, Vec<u8>)>,
    entries: std::collections::VecDeque<LogEntry>,
    damaged: bool,
}

impl ExportParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        if self.damaged {
            return;
        }
        self.buf.extend_from_slice(chunk);
        let mut pos = 0;
        while let Some(next) = self.step(pos) {
            pos = next;
        }
        self.buf.drain(..pos);
    }

    /// Takes one line or one binary field starting at `pos`; the position after it, or None when more bytes are needed.
    fn step(&mut self, pos: usize) -> Option<usize> {
        if self.damaged {
            return None;
        }
        let rest = &self.buf[pos..];
        let nl = rest.iter().position(|b| *b == b'\n')?;
        let line = &rest[..nl];
        if line.is_empty() {
            self.flush();
            return Some(pos + nl + 1);
        }
        if let Some(eq) = line.iter().position(|b| *b == b'=') {
            let field = (String::from_utf8_lossy(&line[..eq]).into_owned(), line[eq + 1..].to_vec());
            self.fields.push(field);
            return Some(pos + nl + 1);
        }
        let key = String::from_utf8_lossy(line).into_owned();
        let head = nl + 1;
        let size = rest.get(head..head + 8)?;
        let len = u64::from_le_bytes([size[0], size[1], size[2], size[3], size[4], size[5], size[6], size[7]]) as usize;
        if len > MAX_FIELD {
            self.damaged = true;
            return None;
        }
        let data = rest.get(head + 8..head + 8 + len)?.to_vec();
        // the newline after the data may not have arrived yet
        rest.get(head + 8 + len)?;
        self.fields.push((key, data));
        Some(pos + head + 8 + len + 1)
    }

    fn flush(&mut self) {
        if self.fields.is_empty() {
            return;
        }
        let fields = std::mem::take(&mut self.fields);
        let lookup = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| String::from_utf8_lossy(v).into_owned());
        self.entries.push_back(model::entry_from_fields(&lookup));
        if self.entries.len() > MAX_ENTRIES {
            self.entries.pop_front();
        }
    }

    /// The entries read, oldest first. A record the data ended in the middle of is dropped.
    pub fn finish(mut self) -> Vec<LogEntry> {
        if !self.damaged {
            // a file may end without the blank line after its last record
            self.flush();
        }
        self.entries.into()
    }
}

/// Entries from journal export bytes.
pub fn parse_export(bytes: &[u8]) -> Vec<LogEntry> {
    let mut parser = ExportParser::new();
    parser.feed(bytes);
    parser.finish()
}

/// Entries from the bytes of a file named `name`: gzip is unpacked first, then the format is told from the content.
/// bzip2 and xz files are refused with a message that says so.
pub fn decode(name: &str, bytes: &[u8]) -> Result<Vec<LogEntry>, String> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let inner = gunzip(bytes)?;
        return decode_plain(name.strip_suffix(".gz").unwrap_or(name), &inner);
    }
    if bytes.starts_with(b"BZh") {
        return Err("bzip2 files cannot be read yet. Unpack the file first.".into());
    }
    if bytes.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0]) {
        return Err("xz files cannot be read yet. Unpack the file first.".into());
    }
    decode_plain(name, bytes)
}

fn decode_plain(name: &str, bytes: &[u8]) -> Result<Vec<LogEntry>, String> {
    if bytes.starts_with(b"__CURSOR=") {
        return Ok(parse_export(bytes));
    }
    let head = &bytes[..bytes.len().min(4096)];
    if head.contains(&0) {
        return Err("This file is not a text log.".into());
    }
    let text = String::from_utf8_lossy(bytes);
    if head.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{') {
        let entries: Vec<LogEntry> = text.lines().filter_map(model::parse_journal_line).collect();
        if !entries.is_empty() {
            return Ok(newest(entries));
        }
    }
    let entries = text.lines().map(|line| LogEntry { process: name.to_string(), level: Level::Notice, message: line.to_string(), ..LogEntry::default() }).collect();
    Ok(newest(entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A gzip file around `data` with the optional header parts `flags` asks for.
    fn gzip(data: &[u8], flags: u8) -> Vec<u8> {
        let mut out = vec![0x1f, 0x8b, 8, flags, 0, 0, 0, 0, 0, 3];
        if flags & 4 != 0 {
            out.extend([3, 0, b'a', b'b', b'c']);
        }
        if flags & 8 != 0 {
            out.extend(b"syslog\0");
        }
        if flags & 16 != 0 {
            out.extend(b"a comment\0");
        }
        if flags & 2 != 0 {
            out.extend([0, 0]);
        }
        out.extend(bifrost_app::compression::encode_buffer(Algorithm::Deflate, data).unwrap());
        out.extend([0, 0, 0, 0]);
        out.extend((data.len() as u32).to_le_bytes());
        out
    }

    fn record(message: &str, priority: u8, time: u64) -> Vec<u8> {
        format!("__CURSOR=s=1\n__REALTIME_TIMESTAMP={time}\nPRIORITY={priority}\n_PID=42\nSYSLOG_IDENTIFIER=sshd\nMESSAGE={message}\n\n").into_bytes()
    }

    #[test]
    fn gunzip_returns_the_original_bytes() {
        let data = b"Oct 10 sshd: accepted\nOct 10 sshd: closed\n".repeat(20);
        assert_eq!(gunzip(&gzip(&data, 0)).unwrap(), data);
    }

    #[test]
    fn gunzip_skips_every_optional_header_part() {
        let data = b"hello hello hello hello";
        for flags in [2, 4, 8, 16, 2 | 4 | 8 | 16] {
            assert_eq!(gunzip(&gzip(data, flags)).unwrap(), data, "flags {flags}");
        }
    }

    #[test]
    fn gunzip_refuses_what_is_not_a_whole_gzip_file() {
        assert!(gunzip(b"plain text, not gzip at all").unwrap_err().contains("not a gzip"));
        let good = gzip(b"hello hello hello hello", 0);
        assert!(gunzip(&good[..good.len() - 12]).is_err(), "cut short");
        let mut wrong = good.clone();
        let n = wrong.len();
        wrong[n - 4] ^= 1;
        assert!(gunzip(&wrong).unwrap_err().contains("damaged"), "the stored size is wrong");
    }

    #[test]
    fn an_export_record_becomes_an_entry() {
        let entries = parse_export(&record("accepted publickey", 6, 1_700_000_000_000_000));
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!((e.process.as_str(), e.pid, e.level), ("sshd", 42, Level::Info));
        assert_eq!(e.message, "accepted publickey");
        assert_eq!(e.date, 1_700_000_000_000_000);
    }

    #[test]
    fn a_binary_field_carries_a_message_with_a_newline() {
        let mut bytes = b"__CURSOR=s=1\n__REALTIME_TIMESTAMP=5\nMESSAGE\n".to_vec();
        let text = "line one\nline two";
        bytes.extend((text.len() as u64).to_le_bytes());
        bytes.extend(text.as_bytes());
        bytes.extend(b"\nPRIORITY=3\n\n");
        bytes.extend(record("next", 6, 6));
        let entries = parse_export(&bytes);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "line one\nline two");
        assert_eq!(entries[0].level, Level::Error);
        assert_eq!(entries[1].message, "next");
    }

    #[test]
    fn the_parser_gives_the_same_entries_for_any_chunking() {
        let mut bytes = Vec::new();
        for i in 0..5u64 {
            bytes.extend(record(&format!("message {i}"), 6, 10 + i));
        }
        bytes.extend(b"__REALTIME_TIMESTAMP=99\nMESSAGE\n");
        bytes.extend(11u64.to_le_bytes());
        bytes.extend(b"two\nlines.\n\n\n");
        let whole = parse_export(&bytes);
        assert_eq!(whole.len(), 6);
        for size in [1, 3, 7, 64] {
            let mut parser = ExportParser::new();
            for chunk in bytes.chunks(size) {
                parser.feed(chunk);
            }
            assert_eq!(parser.finish(), whole, "chunks of {size}");
        }
    }

    #[test]
    fn a_last_record_without_a_blank_line_is_kept_and_a_cut_binary_field_is_not() {
        let mut bytes = record("first", 6, 1);
        bytes.extend(b"__REALTIME_TIMESTAMP=2\nMESSAGE=second\n");
        assert_eq!(parse_export(&bytes).len(), 2);
        let mut cut = record("first", 6, 1);
        cut.extend(b"MESSAGE\n");
        cut.extend(50u64.to_le_bytes());
        cut.extend(b"only a few bytes");
        assert_eq!(parse_export(&cut).len(), 1, "the unfinished record is dropped");
    }

    #[test]
    fn an_absurd_field_length_marks_the_file_damaged() {
        let mut bytes = record("first", 6, 1);
        bytes.extend(b"MESSAGE\n");
        bytes.extend(u64::MAX.to_le_bytes());
        bytes.extend(record("after", 6, 2));
        let entries = parse_export(&bytes);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "first");
    }

    #[test]
    fn decode_tells_the_format_from_the_content() {
        let export = decode("boot.logarchive", &record("hello", 6, 1)).unwrap();
        assert_eq!(export[0].process, "sshd");
        let json = decode("journal.json", br#"{"MESSAGE":"hi","PRIORITY":"3","_COMM":"cron"}"#).unwrap();
        assert_eq!((json[0].process.as_str(), json[0].level, json[0].message.as_str()), ("cron", Level::Error, "hi"));
        let text = decode("auth.log", b"one\ntwo\n").unwrap();
        assert_eq!(text.len(), 2);
        assert_eq!((text[1].process.as_str(), text[1].message.as_str()), ("auth.log", "two"));
    }

    #[test]
    fn decode_unpacks_gzip_and_names_the_process_without_the_suffix() {
        let packed = gzip(b"first line\nsecond line\n", 0);
        let entries = decode("syslog.1.gz", &packed).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].process, "syslog.1");
        let inner = gzip(&record("inside", 6, 3), 0);
        assert_eq!(decode("boot.gz", &inner).unwrap()[0].message, "inside");
    }

    #[test]
    fn decode_refuses_binary_data_and_the_formats_it_cannot_unpack() {
        assert!(decode("blob", &[1, 2, 0, 3]).unwrap_err().contains("not a text log"));
        assert!(decode("a.bz2", b"BZh91AY").unwrap_err().contains("bzip2"));
        assert!(decode("a.xz", &[0xfd, b'7', b'z', b'X', b'Z', 0, 1]).unwrap_err().contains("xz"));
        assert!(decode("empty.log", b"").unwrap().is_empty());
    }

    #[test]
    fn a_long_file_keeps_the_newest_entries() {
        let text: String = (0..MAX_ENTRIES + 5).map(|i| format!("line {i}\n")).collect();
        let entries = decode("big.log", text.as_bytes()).unwrap();
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries[0].message, "line 5");
    }
}
