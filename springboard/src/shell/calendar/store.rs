//! Local civil-date events. The versioned file is replaced only after a
//! complete, synced write; title bytes are hex encoded to preserve Unicode.
use super::Date;
use crate::shell::sha256;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

pub const MAX_EVENTS: usize = 2048;
// 2048 events, each with three 160-character four-byte Unicode texts encoded
// as hex, occupy under 8 MiB. Keep the read bound above the largest valid save.
const MAX_FILE: u64 = 12 * 1024 * 1024;
const HEADER: &str = "springboard-calendar-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarEvent {
    pub id: u64,
    pub date: Date,
    pub minute: u16,
    pub duration: u16,
    pub all_day: bool,
    pub title: String,
    pub location: String,
    pub notes: String,
    /// 0 never, 1 daily, 2 weekly, 3 every 2 weeks, 4 monthly, 5 yearly.
    pub repeat: u8,
    /// 0 none, 1 at time, 2 5 min, 3 15 min, 4 30 min, 5 1 hour, 6 1 day before.
    pub alert: u8,
    /// The iCalendar UID of an event that came from a synced account, so its edits can be sent back to that
    /// account. Empty for an event made here.
    pub uid: String,
}

pub const REPEATS: [&str; 6] = [
    "Never",
    "Every Day",
    "Every Week",
    "Every 2 Weeks",
    "Every Month",
    "Every Year",
];
pub const ALERTS: [&str; 7] = [
    "None",
    "At time of event",
    "5 minutes before",
    "15 minutes before",
    "30 minutes before",
    "1 hour before",
    "1 day before",
];

impl CalendarEvent {
    pub fn valid(&self) -> bool {
        self.id != 0
            && self.date.valid()
            && self.minute < 1440
            && (15..=1440).contains(&self.duration)
            && !self.title.trim().is_empty()
            && self.title.chars().count() <= 160
            && !self.title.chars().any(char::is_control)
            && self.location.chars().count() <= 160
            && !self.location.chars().any(char::is_control)
            && self.notes.chars().count() <= 160
            && !self.notes.chars().any(char::is_control)
            && (self.repeat as usize) < REPEATS.len()
            && (self.alert as usize) < ALERTS.len()
            && self.uid.chars().count() <= 320
            && !self.uid.chars().any(char::is_control)
    }

    /// True when this event or one of its repeats falls on `d`.
    pub fn occurs_on(&self, d: Date) -> bool {
        if d < self.date {
            return false;
        }
        let gap = d.days() - self.date.days();
        match self.repeat {
            0 => d == self.date,
            1 => true,
            2 => gap % 7 == 0,
            3 => gap % 14 == 0,
            4 => d.day == self.date.day.min(d.month_len()),
            5 => d.month == self.date.month && d.day == self.date.day.min(d.month_len()),
            _ => false,
        }
    }
}

fn hex(text: &str) -> String {
    sha256::hex(text.as_bytes())
}

fn unhex(s: &str) -> Option<String> {
    if s.len() > 1280 {
        return None;
    }
    String::from_utf8(sha256::unhex(s)?).ok()
}

/// Invalid headers and oversized files are rejected; malformed records are
/// skipped independently so one damaged event does not hide the rest.
pub fn load(path: &Path) -> io::Result<Vec<CalendarEvent>> {
    let f = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e),
    };
    let mut text = String::new();
    f.take(MAX_FILE + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_FILE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "calendar file too large",
        ));
    }
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported calendar file",
        ));
    }
    let mut events = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for line in lines {
        let parts: Vec<_> = line.split('\t').collect();
        if parts.len() != 8 && parts.len() != 12 && parts.len() != 13 {
            continue;
        }
        let parse = || -> Option<CalendarEvent> {
            Some(CalendarEvent {
                id: parts[0].parse().ok()?,
                date: Date {
                    year: parts[1].parse().ok()?,
                    month: parts[2].parse().ok()?,
                    day: parts[3].parse().ok()?,
                },
                minute: parts[4].parse().ok()?,
                duration: parts[5].parse().ok()?,
                all_day: match parts[6] {
                    "0" => false,
                    "1" => true,
                    _ => return None,
                },
                title: unhex(parts[7])?,
                repeat: parts.get(8).map_or(Some(0), |p| p.parse().ok())?,
                alert: parts.get(9).map_or(Some(0), |p| p.parse().ok())?,
                location: parts.get(10).map_or(Some(String::new()), |p| unhex(p))?,
                notes: parts.get(11).map_or(Some(String::new()), |p| unhex(p))?,
                uid: parts.get(12).map_or(Some(String::new()), |p| unhex(p))?,
            })
        };
        if let Some(e) = parse().filter(CalendarEvent::valid) {
            if events.len() < MAX_EVENTS && ids.insert(e.id) {
                events.push(e);
            }
        }
    }
    sort(&mut events);
    Ok(events)
}

pub fn sort(events: &mut [CalendarEvent]) {
    events.sort_by_key(|e| (e.date, !e.all_day, e.minute, e.id));
}

pub fn next_id(events: &[CalendarEvent]) -> u64 {
    let mut id = events
        .iter()
        .map(|e| e.id)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .unwrap_or(1);
    while events.iter().any(|e| e.id == id) {
        id = id.checked_add(1).unwrap_or(1);
    }
    id
}

pub fn save(path: &Path, events: &[CalendarEvent]) -> io::Result<()> {
    if events.len() > MAX_EVENTS || events.iter().any(|e| !e.valid()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid calendar event",
        ));
    }
    let mut ids = std::collections::HashSet::new();
    if !events.iter().all(|e| ids.insert(e.id)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "duplicate event id",
        ));
    }
    let mut text = format!("{HEADER}\n");
    for e in events {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            e.id,
            e.date.year,
            e.date.month,
            e.date.day,
            e.minute,
            e.duration,
            e.all_day as u8,
            hex(&e.title),
            e.repeat,
            e.alert,
            hex(&e.location),
            hex(&e.notes),
            hex(&e.uid)
        ));
    }
    crate::shell::fsutil::write_atomic(path, text)
}

#[cfg(test)]
mod uid_tests {
    use super::*;

    fn event(id: u64, uid: &str) -> CalendarEvent {
        CalendarEvent {
            id,
            date: Date { year: 2026, month: 10, day: 17 },
            minute: 540,
            duration: 60,
            all_day: false,
            title: "Sync".into(),
            location: String::new(),
            notes: String::new(),
            repeat: 0,
            alert: 0,
            uid: uid.into(),
        }
    }

    #[test]
    fn a_synced_event_keeps_its_uid_and_a_local_one_has_none() {
        let path = std::env::temp_dir().join(format!("calendar-uid-{}.txt", std::process::id()));
        let events = vec![event(1, "abc-1@example.com"), event(2, "")];
        save(&path, &events).unwrap();
        let back = load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.iter().find(|e| e.id == 1).unwrap().uid, "abc-1@example.com");
        assert_eq!(back.iter().find(|e| e.id == 2).unwrap().uid, "");
    }

    #[test]
    fn a_file_written_before_uids_loads_with_none() {
        let path = std::env::temp_dir().join(format!("calendar-uid-old-{}.txt", std::process::id()));
        std::fs::write(&path, format!("{HEADER}\n1\t2026\t10\t17\t540\t60\t0\t{}\t0\t0\t\t\n", hex("Old"))).unwrap();
        let back = load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].uid, "");
    }
}
