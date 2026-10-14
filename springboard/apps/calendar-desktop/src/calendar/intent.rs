//! The calendar's intents and control methods (`create-event`, `show-date`): parameter parsing and the event store write,
//! kept apart from the window so they run with the app in the background (no `Ctx`, only the data directory).
use super::store::{self, CalendarEvent};
use super::Date;
use std::path::Path;

/// `YYYY-MM-DD`, or ISO 8601 date and time (`2026-10-10T14:30:00`), of which only the date is read.
pub fn parse_date(text: &str) -> Option<Date> {
    let mut parts = text.trim().split('T').next()?.split('-');
    let date = Date { year: parts.next()?.parse().ok()?, month: parts.next()?.parse().ok()?, day: parts.next()?.parse().ok()? };
    (parts.next().is_none() && date.valid()).then_some(date)
}

/// `HH:MM` (24 hour) as the minute of the day.
pub fn parse_time(text: &str) -> Option<u16> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// Adds an event to the store file at `events` and returns it. Without a `minute` the event is an all-day one.
/// The title must not be empty, and the store must have room.
pub fn create_event(events: &Path, title: &str, date: Date, minute: Option<u16>, duration: u16) -> Result<CalendarEvent, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("The event needs a title".into());
    }
    let mut all = store::load(events).map_err(|e| e.to_string())?;
    if all.len() >= store::MAX_EVENTS {
        return Err("Calendar is full".into());
    }
    let event = CalendarEvent {
        id: store::next_id(&all),
        date,
        minute: minute.unwrap_or(0),
        duration: if minute.is_some() { duration.clamp(15, 1440) } else { 60 },
        all_day: minute.is_none(),
        title: title.to_string(),
        location: String::new(),
        notes: String::new(),
        repeat: 0,
        alert: 0,
    };
    all.push(event.clone());
    store::sort(&mut all);
    store::save(events, &all).map_err(|e| e.to_string())?;
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("springboard-calendar-intent-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("events.tsv")
    }

    #[test]
    fn parses_dates_and_times() {
        assert_eq!(parse_date("2026-10-04"), Some(Date { year: 2026, month: 10, day: 4 }));
        assert_eq!(parse_date("2026-10-04T14:30:00"), Some(Date { year: 2026, month: 10, day: 4 }));
        assert_eq!(parse_date("2026-02-30"), None);
        assert_eq!(parse_date("2026-10"), None);
        assert_eq!(parse_date("2026-10-04-1"), None);
        assert_eq!(parse_time("09:30"), Some(570));
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("9"), None);
    }

    #[test]
    fn creates_a_timed_and_an_all_day_event() {
        let path = temp("create");
        let date = Date { year: 2026, month: 10, day: 4 };
        let timed = create_event(&path, "  Dentist ", date, Some(600), 45).unwrap();
        assert_eq!((timed.title.as_str(), timed.minute, timed.duration, timed.all_day), ("Dentist", 600, 45, false));
        let all_day = create_event(&path, "Holiday", date, None, 45).unwrap();
        assert!(all_day.all_day);
        assert_ne!(timed.id, all_day.id);
        assert_eq!(store::load(&path).unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn refuses_an_empty_title() {
        let path = temp("empty");
        let date = Date { year: 2026, month: 10, day: 4 };
        assert!(create_event(&path, "   ", date, Some(0), 60).is_err());
        assert!(store::load(&path).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
