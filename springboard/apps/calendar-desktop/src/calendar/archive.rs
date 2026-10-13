//! Calendar archive: one file holding the event store (`events.tsv`) and the rich sidecar (`calendar.ics`), used by
//! File > Export Calendar Archive and Import Calendar Archive to back up and restore the whole calendar.
//!
//! Layout: a header line, then two length-prefixed sections. The lengths are in bytes so the contents may hold any text.
//!
//! ```text
//! BIFROST-CALENDAR-ARCHIVE 1
//! events <len>
//! <len bytes>
//! ics <len>
//! <len bytes>
//! ```

/// The file extension of an archive.
pub const EXTENSION: &str = "bfcal";
const HEADER: &str = "BIFROST-CALENDAR-ARCHIVE 1";
/// The largest archive Import reads.
pub const MAX_BYTES: u64 = 24 * 1024 * 1024;

/// Packs the event store text and the rich sidecar text into one archive.
pub fn pack(events: &str, ics: &str) -> String {
    format!("{HEADER}\nevents {}\n{events}\nics {}\n{ics}\n", events.len(), ics.len())
}

/// Splits an archive back into (events text, ics text).
pub fn unpack(text: &str) -> Result<(String, String), String> {
    let rest = text.strip_prefix(HEADER).and_then(|r| r.strip_prefix('\n')).ok_or("This is not a calendar archive")?;
    let (events, rest) = section(rest, "events")?;
    let (ics, _) = section(rest, "ics")?;
    Ok((events.to_string(), ics.to_string()))
}

/// Reads one `name <len>\n<bytes>\n` section and returns its bytes and what follows it.
fn section<'a>(text: &'a str, name: &str) -> Result<(&'a str, &'a str), String> {
    let damaged = || "The calendar archive is damaged".to_string();
    let (line, body) = text.split_once('\n').ok_or_else(damaged)?;
    let len: usize = line.strip_prefix(name).and_then(|l| l.strip_prefix(' ')).and_then(|l| l.parse().ok()).ok_or_else(damaged)?;
    let content = body.get(..len).ok_or_else(damaged)?;
    let rest = body.get(len..).and_then(|r| r.strip_prefix('\n')).ok_or_else(damaged)?;
    Ok((content, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_both_sections() {
        let events = "springboard-calendar-v2\n1\t2026\t10\t4\n";
        let ics = "BEGIN:VCALENDAR\r\nSUMMARY:Thorrablot \u{00fe}\r\nEND:VCALENDAR\r\n";
        assert_eq!(unpack(&pack(events, ics)), Ok((events.to_string(), ics.to_string())));
    }

    #[test]
    fn empty_sections_round_trip() {
        assert_eq!(unpack(&pack("", "")), Ok((String::new(), String::new())));
    }

    #[test]
    fn sections_may_contain_header_like_lines() {
        let tricky = "ics 5\nevents 3\n";
        assert_eq!(unpack(&pack(tricky, tricky)), Ok((tricky.to_string(), tricky.to_string())));
    }

    #[test]
    fn rejects_foreign_and_truncated_files() {
        assert!(unpack("BEGIN:VCALENDAR\n").is_err());
        let packed = pack("abc", "def");
        assert!(unpack(&packed[..packed.len() - 6]).is_err());
        assert!(unpack(&packed.replace("events 3", "events 99")).is_err());
        assert!(unpack(&packed.replace("ics 3", "ics x")).is_err());
    }
}
