//! Calendar files: File > Import (an .ics or .vcs file, or a calendar archive) and File > Export Archive.
//!
//! An archive is one text file holding the event store (`events.tsv`) and the .ics sidecar with the repeat rules,
//! alerts and invitees (`calendar.ics`): the whole calendar in one file, for a backup or a move to another machine.
//! Importing adds what the calendar does not already hold and never removes anything.
use super::exchange::{load_rich, rich_path};
use super::ics::{self, IcsCalendar};
use super::store::{self, CalendarEvent};
use super::Calendar;
use std::io::Read;
use std::path::PathBuf;

/// The first line of an archive file.
pub const MAGIC: &str = "bifrost-calendar-archive-v1";
/// The extension an archive is saved with.
pub const EXTENSION: &str = "calarchive";
/// The largest file an import reads (the event store itself is limited to 12 MiB, the sidecar to 8 MiB).
pub const MAX_FILE: u64 = 24 * 1024 * 1024;

const DAMAGED: &str = "This calendar archive is damaged";

/// The two parts of an archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Archive {
    /// The text of the event store file (empty when the calendar had no events saved).
    pub events: String,
    /// The text of the .ics sidecar (empty when there is none).
    pub ics: String,
}

/// One text file holding both parts. Each part is a header line with its name and byte length, then its text and a
/// line break, so the text may hold anything.
pub fn pack(a: &Archive) -> String {
    format!("{MAGIC}\nevents.tsv {}\n{}\ncalendar.ics {}\n{}\n", a.events.len(), a.events, a.ics.len(), a.ics)
}

/// Whether `text` is an archive (its first line is the magic line).
pub fn is_archive(text: &str) -> bool {
    text.strip_prefix(MAGIC).is_some_and(|r| r.starts_with('\n'))
}

/// Reads the parts of an archive. Unknown parts are skipped, so a later version can add some.
pub fn unpack(text: &str) -> Result<Archive, String> {
    let mut rest = text
        .strip_prefix(MAGIC)
        .and_then(|r| r.strip_prefix('\n'))
        .ok_or_else(|| "This is not a calendar archive".to_string())?;
    let (mut events, mut ics) = (None, None);
    while !rest.is_empty() {
        let (head, tail) = rest.split_once('\n').ok_or(DAMAGED)?;
        let (name, len) = head.split_once(' ').ok_or(DAMAGED)?;
        let len: usize = len.parse().map_err(|_| DAMAGED)?;
        if len > tail.len() || !tail.is_char_boundary(len) {
            return Err(DAMAGED.into());
        }
        let (body, after) = tail.split_at(len);
        rest = after.strip_prefix('\n').ok_or(DAMAGED)?;
        match name {
            "events.tsv" => events = Some(body.to_string()),
            "calendar.ics" => ics = Some(body.to_string()),
            _ => {}
        }
    }
    Ok(Archive { events: events.ok_or(DAMAGED)?, ics: ics.unwrap_or_default() })
}

/// The events in `incoming` that `existing` does not hold (same day, time, kind and title), with ids that are free in
/// `existing`. Stops when the store is full.
pub fn merge_events(existing: &[CalendarEvent], incoming: &[CalendarEvent]) -> Vec<CalendarEvent> {
    let mut all = existing.to_vec();
    let mut added = vec![];
    for e in incoming {
        if all.len() >= store::MAX_EVENTS {
            break;
        }
        let held = all.iter().any(|x| x.date == e.date && x.minute == e.minute && x.all_day == e.all_day && x.title == e.title);
        if held {
            continue;
        }
        let ev = CalendarEvent { id: store::next_id(&all), ..e.clone() };
        all.push(ev.clone());
        added.push(ev);
    }
    added
}

/// Adds the events and time zones of `incoming` to `rich`; an event with a UID that `rich` already has replaces it.
pub fn merge_rich(rich: &mut IcsCalendar, incoming: IcsCalendar) {
    for e in incoming.events {
        rich.events.retain(|r| r.uid != e.uid);
        rich.events.push(e);
    }
    for z in incoming.timezones {
        if !rich.timezones.iter().any(|(n, _)| *n == z.0) {
            rich.timezones.push(z);
        }
    }
}

/// The file an `Input::Open` names: a `file://` URL (percent-escapes decoded, a host part ignored) or an absolute path.
pub fn path_from_open(url: &str) -> Option<PathBuf> {
    let path = match url.strip_prefix("file://") {
        Some(rest) => decode_percent(&rest[rest.find('/')?..])?,
        None if url.starts_with('/') => url.to_string(),
        None => return None,
    };
    Some(PathBuf::from(path))
}

fn decode_percent(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The text of a chosen file, at most `MAX_FILE` bytes (anything longer is not a calendar).
pub fn read_text(file: impl Read) -> Result<String, String> {
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE {
        return Err("This file is too large to be a calendar".into());
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// What the calendar says after an import of `events` new events.
pub fn imported_message(events: usize) -> String {
    match events {
        0 => "Nothing new to import".to_string(),
        1 => "Imported 1 event".to_string(),
        n => format!("Imported {n} events"),
    }
}

impl Calendar {
    /// Shows `message` as an error (a file that could not be read or written).
    pub fn show_error(&mut self, message: &str) {
        self.notice = None;
        self.error = Some(message.to_string());
        self.dirty = true;
    }

    /// Shows `message` as a notice (a file that was written).
    pub fn show_notice(&mut self, message: &str) {
        self.error = None;
        self.notice = Some(message.to_string());
        self.dirty = true;
    }

    /// The whole calendar as an archive file (File > Export Archive).
    pub fn archive_text(&self) -> String {
        let events = std::fs::read_to_string(&self.path).unwrap_or_default();
        let ics = std::fs::read_to_string(rich_path(&self.path)).unwrap_or_default();
        pack(&Archive { events, ics })
    }

    /// The name the save panel suggests for an archive.
    pub fn archive_name(&self) -> String {
        format!("Calendar Archive {}.{EXTENSION}", self.today.iso())
    }

    /// Imports the text of a chosen file: an archive, or else an .ics / .vcs calendar. The result is shown as a notice
    /// or an error. True when something was read.
    pub fn import_file(&mut self, text: &str) -> bool {
        self.notice = None;
        self.error = None;
        let result = if is_archive(text) { self.import_archive(text) } else { self.import_ics(text) };
        self.dirty = true;
        match result {
            Ok(n) => {
                self.notice = Some(imported_message(n));
                true
            }
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }

    /// Adds the events and the sidecar of an archive. Returns how many events were new.
    pub fn import_archive(&mut self, text: &str) -> Result<usize, String> {
        if self.storage_blocked {
            return Err("Saved events could not be read. Import is unavailable.".into());
        }
        let archive = unpack(text)?;
        let incoming = self.read_archive_events(&archive.events)?;
        let added = merge_events(&self.events, &incoming);
        if !added.is_empty() {
            let mut events = self.events.clone();
            events.extend(added.iter().cloned());
            store::sort(&mut events);
            store::save(&self.path, &events).map_err(|e| e.to_string())?;
            self.events = events;
            self.changes.post();
        }
        let incoming_rich = ics::parse(&archive.ics);
        if !incoming_rich.events.is_empty() {
            let rp = rich_path(&self.path);
            let mut rich = load_rich(&rp);
            merge_rich(&mut rich, incoming_rich);
            springboard::shell::fsutil::write_atomic(&rp, &ics::write(&rich, &ics::stamp(self.wall.unix))).map_err(|e| e.to_string())?;
            self.changes.post();
        }
        Ok(added.len())
    }

    /// The events in the archive's store text. The store reads files only, so the text goes through a scratch file
    /// beside the events file, which is removed again.
    fn read_archive_events(&self, text: &str) -> Result<Vec<CalendarEvent>, String> {
        if text.trim().is_empty() {
            return Ok(vec![]);
        }
        let scratch = self.path.with_file_name("events.import.tmp");
        let loaded = springboard::shell::fsutil::write_atomic(&scratch, text).and_then(|_| store::load(&scratch));
        let _ = std::fs::remove_file(&scratch);
        loaded.map_err(|_| DAMAGED.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::super::ics::{IcsEvent, Local};
    use super::super::Date;
    use super::*;

    fn d(month: u32, day: u32) -> Date {
        Date { year: 2026, month, day }
    }
    fn ev(id: u64, date: Date, title: &str) -> CalendarEvent {
        CalendarEvent { id, date, minute: 540, duration: 60, all_day: false, title: title.into(), location: String::new(), notes: String::new(), repeat: 0, alert: 0, uid: String::new() }
    }

    #[test]
    fn an_archive_packs_and_unpacks_both_parts() {
        let a = Archive { events: "springboard-calendar-v1\n1\t2026\n".into(), ics: "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n".into() };
        let text = pack(&a);
        assert!(is_archive(&text));
        assert_eq!(unpack(&text), Ok(a));
    }

    #[test]
    fn parts_may_hold_anything_including_unicode_and_empty_text() {
        for (events, ics) in [("", ""), ("\u{00fe}orri\n\n", "x\ny z\n1 2\n"), ("calendar.ics 5\n", "events.tsv 1\n")] {
            let a = Archive { events: events.into(), ics: ics.into() };
            assert_eq!(unpack(&pack(&a)), Ok(a));
        }
    }

    #[test]
    fn other_text_is_not_an_archive() {
        assert!(!is_archive("BEGIN:VCALENDAR\n"));
        assert!(!is_archive(&format!("{MAGIC}x\n")));
        assert_eq!(unpack("BEGIN:VCALENDAR\n"), Err("This is not a calendar archive".to_string()));
    }

    #[test]
    fn a_damaged_archive_is_refused() {
        let good = pack(&Archive { events: "abc".into(), ics: "def".into() });
        // a length that runs past the end, a missing separator, a cut file, an unreadable length and a split character
        for bad in [
            format!("{MAGIC}\nevents.tsv 99\nabc\n"),
            format!("{MAGIC}\nevents.tsv 3\nabcX\n"),
            good[..good.len() - 6].to_string(),
            format!("{MAGIC}\nevents.tsv x\nabc\n"),
            format!("{MAGIC}\nevents.tsv 1\n\u{00fe}\n"),
            format!("{MAGIC}\nevents.tsv\n"),
        ] {
            assert_eq!(unpack(&bad), Err(DAMAGED.to_string()), "{bad:?}");
        }
        // an archive without its events part is damaged too
        assert_eq!(unpack(&format!("{MAGIC}\ncalendar.ics 0\n\n")), Err(DAMAGED.to_string()));
    }

    #[test]
    fn unknown_parts_are_skipped_and_the_sidecar_is_optional() {
        let text = format!("{MAGIC}\nfuture.dat 2\nzz\nevents.tsv 1\nq\n");
        assert_eq!(unpack(&text), Ok(Archive { events: "q".into(), ics: String::new() }));
    }

    #[test]
    fn merging_adds_only_events_the_calendar_lacks_with_free_ids() {
        let existing = vec![ev(1, d(10, 5), "Gym"), ev(2, d(10, 6), "Lunch")];
        let incoming = vec![ev(1, d(10, 5), "Gym"), ev(1, d(10, 7), "Dentist"), ev(2, d(10, 8), "Dentist")];
        let added = merge_events(&existing, &incoming);
        let got: Vec<(u64, Date)> = added.iter().map(|e| (e.id, e.date)).collect();
        assert_eq!(got, vec![(3, d(10, 7)), (4, d(10, 8))]);
        // importing the same events again adds nothing
        let mut all = existing.clone();
        all.extend(added);
        assert!(merge_events(&all, &incoming).is_empty());
    }

    #[test]
    fn merging_stops_at_the_store_limit() {
        let existing: Vec<CalendarEvent> = (1..=store::MAX_EVENTS as u64 - 1).map(|i| ev(i, d(1, 1), &format!("e{i}"))).collect();
        let incoming = vec![ev(1, d(2, 1), "a"), ev(2, d(2, 2), "b")];
        assert_eq!(merge_events(&existing, &incoming).len(), 1);
    }

    #[test]
    fn merged_sidecars_replace_by_uid_and_keep_one_copy_of_each_zone() {
        let mk = |uid: &str, title: &str| IcsEvent::new(uid, title, Local::new(d(10, 5), 540), Local::new(d(10, 5), 600));
        let mut rich = IcsCalendar { name: String::new(), events: vec![mk("a", "Old"), mk("b", "Keep")], timezones: vec![("Atlantic/Reykjavik".into(), 0)] };
        let incoming = IcsCalendar { name: String::new(), events: vec![mk("a", "New"), mk("c", "Added")], timezones: vec![("Atlantic/Reykjavik".into(), 0), ("Europe/Oslo".into(), 60)] };
        merge_rich(&mut rich, incoming);
        let mut titles: Vec<&str> = rich.events.iter().map(|e| e.summary.as_str()).collect();
        titles.sort_unstable();
        assert_eq!(titles, vec!["Added", "Keep", "New"]);
        assert_eq!(rich.timezones.len(), 2);
    }

    #[test]
    fn open_urls_become_paths() {
        assert_eq!(path_from_open("/home/ana/Calendar.ics"), Some(PathBuf::from("/home/ana/Calendar.ics")));
        assert_eq!(path_from_open("file:///home/ana/My%20Trip.ics"), Some(PathBuf::from("/home/ana/My Trip.ics")));
        assert_eq!(path_from_open("file://localhost/tmp/a.ics"), Some(PathBuf::from("/tmp/a.ics")));
        assert_eq!(path_from_open("file:///tmp/%C3%BE.ics"), Some(PathBuf::from("/tmp/\u{00fe}.ics")));
        assert_eq!(path_from_open("https://example.is/a.ics"), None);
        assert_eq!(path_from_open("relative/a.ics"), None);
        assert_eq!(path_from_open("file:///tmp/%zz.ics"), None);
        assert_eq!(path_from_open("file://nohost"), None);
    }

    #[test]
    fn a_file_is_read_up_to_the_limit() {
        assert_eq!(read_text(&b"BEGIN:VCALENDAR"[..]), Ok("BEGIN:VCALENDAR".to_string()));
        assert_eq!(read_text(&[0xff, b'a'][..]), Ok("\u{fffd}a".to_string()), "bad bytes do not stop the read");
        let big = std::io::repeat(b'x').take(MAX_FILE + 1);
        assert!(read_text(big).is_err());
        let exact = std::io::repeat(b'x').take(MAX_FILE);
        assert_eq!(read_text(exact).map(|t| t.len() as u64), Ok(MAX_FILE));
    }

    #[test]
    fn messages_count_the_events() {
        assert_eq!(imported_message(0), "Nothing new to import");
        assert_eq!(imported_message(1), "Imported 1 event");
        assert_eq!(imported_message(12), "Imported 12 events");
    }
}
