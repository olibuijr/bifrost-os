//! Edit > Copy / Paste of one event: the pasteboard holds the event as an .ics file, so it pastes into this calendar,
//! another copy of the app, or any program that reads calendar text.
use super::ics;
use super::store::CalendarEvent;
use super::Date;

/// `e` as the text copied to the pasteboard (title, time, location and notes).
pub fn to_text(e: &CalendarEvent, stamp: &str) -> String {
    let mut ev = ics::from_calendar_event(e);
    ev.location = e.location.clone();
    ev.notes = e.notes.clone();
    ics::write_single(&ev, &[], stamp)
}

/// The first event in pasteboard `text`, moved to `date` (keeping its time of day and length) under id `id`. None when
/// the text holds no event.
pub fn from_text(text: &str, date: Date, id: u64) -> Option<CalendarEvent> {
    let cal = ics::parse(text);
    let src = cal.events.first()?;
    let mut e = ics::to_calendar_event(src, id)?;
    e.date = date;
    e.location = src.location.chars().filter(|c| !c.is_control()).take(160).collect();
    e.notes = src.notes.clone();
    e.valid().then_some(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(day: u32) -> Date {
        Date { year: 2026, month: 10, day }
    }
    fn event() -> CalendarEvent {
        CalendarEvent {
            id: 3,
            date: d(4),
            minute: 600,
            duration: 90,
            all_day: false,
            title: "Dentist".into(),
            location: "Main St 4".into(),
            notes: "Bring the card".into(),
            repeat: 0,
            alert: 0,
        }
    }

    #[test]
    fn an_event_round_trips_to_another_day() {
        let text = to_text(&event(), "20261004T000000Z");
        let got = from_text(&text, d(9), 8).expect("event");
        assert_eq!((got.id, got.date, got.minute, got.duration), (8, d(9), 600, 90));
        assert_eq!((got.title.as_str(), got.location.as_str(), got.notes.as_str()), ("Dentist", "Main St 4", "Bring the card"));
    }

    #[test]
    fn all_day_events_stay_all_day() {
        let mut e = event();
        e.all_day = true;
        e.minute = 0;
        e.duration = 1440;
        let got = from_text(&to_text(&e, "20261004T000000Z"), d(5), 9).expect("event");
        assert!(got.all_day);
        assert_eq!(got.date, d(5));
    }

    #[test]
    fn text_without_an_event_pastes_nothing() {
        assert!(from_text("just some words", d(1), 1).is_none());
        assert!(from_text("", d(1), 1).is_none());
    }
}
