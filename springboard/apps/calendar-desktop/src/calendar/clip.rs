//! Cut, Copy and Paste of events (the Edit menu). An event travels on the pasteboard as .ics text, so it can be pasted
//! here, into another calendar app, or read in a text editor. Pasting puts the events on the shown day, keeping their
//! times, lengths, locations, notes, alert and (preset) repeat.
use super::alerts::{ALERT_MINUTES, REPEAT_RULES};
use super::ics::{self, IcsEvent};
use super::store::{self, CalendarEvent};
use super::{recur, Calendar, Date, UNDO_MS};
use bifrost_app::{Command as Edit, Commands};
use springboard::shell::keyboard::Out;
use springboard::shell::ui::Ui;

/// The longest text a store event keeps in each of its title, location and notes.
const FIELD_MAX: usize = 160;

/// Whether `text` holds at least one event, so Paste has something to do.
pub fn pasteable(text: &str) -> bool {
    text.contains("BEGIN:VEVENT")
}

/// One event as .ics text, with what the store holds beyond the .ics conversion (location, notes, alert, repeat).
pub fn copy_text(e: &CalendarEvent, stamp: &str) -> String {
    let mut ev = ics::from_calendar_event(e);
    ev.location = e.location.clone();
    ev.notes = e.notes.clone();
    ev.alarms = ALERT_MINUTES.get(e.alert as usize).copied().flatten().into_iter().collect();
    ev.rule = REPEAT_RULES.get(e.repeat as usize).and_then(|r| recur::parse_rrule(r));
    ics::write_single(&ev, &[], stamp)
}

/// The preset repeat (`store::REPEATS`) that matches the event's rule; Never when it has none or one the store cannot hold.
fn repeat_of(e: &IcsEvent) -> u8 {
    let Some(rule) = &e.rule else { return 0 };
    REPEAT_RULES
        .iter()
        .position(|r| !r.is_empty() && recur::parse_rrule(r).as_ref() == Some(rule))
        .unwrap_or(0) as u8
}

/// The preset alert (`store::ALERTS`) that matches the event's first alarm; None when it has none or an odd one.
fn alert_of(e: &IcsEvent) -> u8 {
    e.alarms
        .first()
        .and_then(|m| ALERT_MINUTES.iter().position(|a| *a == Some(*m)))
        .unwrap_or(0) as u8
}

/// Control characters (a line break in the notes) become spaces; the store keeps no control characters.
fn clean(text: &str) -> String {
    let flat: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    flat.trim().chars().take(FIELD_MAX).collect()
}

/// The events in `text` as store events on `target`: the earliest one lands on it, the others keep their distance from
/// it. Ids are new (after `existing`); events the store cannot hold are left out, and so is anything past its limit.
pub fn paste(text: &str, target: Date, existing: &[CalendarEvent]) -> Vec<CalendarEvent> {
    let cal = ics::parse(text);
    let Some(anchor) = cal.events.iter().map(|e| e.start.date).min() else { return vec![] };
    let delta = target.days() - anchor.days();
    let mut all = existing.to_vec();
    let mut added = vec![];
    for e in &cal.events {
        if all.len() >= store::MAX_EVENTS {
            break;
        }
        let Some(mut ev) = ics::to_calendar_event(e, store::next_id(&all)) else { continue };
        ev.date = Date::from_days(ev.date.days() + delta);
        ev.location = clean(&e.location);
        ev.notes = clean(&e.notes);
        ev.repeat = repeat_of(e);
        ev.alert = alert_of(e);
        if !ev.valid() {
            continue;
        }
        all.push(ev.clone());
        added.push(ev);
    }
    added
}

impl Calendar {
    /// The event Copy and Cut act on: the one open in the editor, else the one last opened or saved when it is on the
    /// shown day, else the first event of the shown day.
    fn copy_target(&self) -> Option<&CalendarEvent> {
        if let Some(draft) = self.editor.as_ref().filter(|d| d.existing) {
            return self.events.iter().find(|e| e.id == draft.event.id);
        }
        let on_day = |e: &&CalendarEvent| e.occurs_on(self.selected);
        self.picked
            .and_then(|id| self.events.iter().find(|e| e.id == id))
            .filter(on_day)
            .or_else(|| self.events.iter().find(on_day))
    }

    /// Which Edit menu commands apply now. In the event editor they belong to the focused text field; otherwise they
    /// act on events. `pasteboard` is the text on the pasteboard, if any.
    pub fn edit_commands(&self, pasteboard: Option<&str>) -> Commands {
        if self.editor.is_some() {
            let Some(i) = self.focused() else {
                return Commands::default().with(Edit::Copy, self.copy_target().is_some());
            };
            let (a, b) = self.fields[i].selection();
            let field = &self.fields[i];
            return Commands::default()
                .with(Edit::Cut, a < b)
                .with(Edit::Copy, a < b)
                .with(Edit::Paste, pasteboard.is_some())
                .with(Edit::SelectAll, !field.is_empty());
        }
        if self.search_focused {
            return Commands::default().with(Edit::Paste, pasteboard.is_some());
        }
        let have = self.goto.is_none() && self.copy_target().is_some();
        Commands::default()
            .with(Edit::Cut, have)
            .with(Edit::Copy, have)
            .with(Edit::Paste, self.goto.is_none() && pasteboard.is_some_and(pasteable))
    }

    /// Runs an Edit menu command. `pasteboard` is read by the app when the command arrives. True when it did something.
    pub fn edit_command(&mut self, ui: &Ui, cmd: Edit, pasteboard: Option<&str>, t: f64) -> bool {
        self.dirty = true;
        // the search field, when it has the cursor, takes Paste; its other commands are not offered
        if self.search_focused && self.editor.is_none() {
            return match (cmd, pasteboard) {
                (Edit::Paste, Some(text)) => {
                    self.search_paste(ui, text);
                    true
                }
                _ => false,
            };
        }
        if self.editor.is_some() {
            if let Some(i) = self.focused() {
                let name = match cmd {
                    Edit::Cut => "Cut",
                    Edit::Copy => "Copy",
                    Edit::Paste => "Paste",
                    Edit::SelectAll => "SelectAll",
                    Edit::Undo => "Undo",
                    Edit::Redo => "Redo",
                };
                self.fields[i].edit(ui, &Out::Named(name), t);
                return true;
            }
            return cmd == Edit::Copy && self.copy_event();
        }
        match cmd {
            Edit::Copy => self.copy_event(),
            Edit::Cut => self.cut_event(t),
            Edit::Paste => pasteboard.is_some_and(|text| self.paste_events(ui, text)),
            _ => false,
        }
    }

    /// Copies the target event as .ics text for the app to put on the pasteboard (`take_copied`).
    pub fn copy_event(&mut self) -> bool {
        self.notice = None;
        let Some(e) = self.copy_target() else { return false };
        let text = copy_text(e, &ics::stamp(self.wall.unix));
        self.notice = Some(format!("Copied {}", e.title));
        self.copied = Some(text);
        true
    }

    /// Copies the target event, then deletes it with the usual Undo bar.
    pub fn cut_event(&mut self, t: f64) -> bool {
        if self.storage_blocked {
            self.error = Some("Saved events could not be read. Cut is unavailable.".into());
            return false;
        }
        let Some(id) = self.copy_target().map(|e| e.id) else { return false };
        if !self.copy_event() {
            return false;
        }
        let Some(removed) = self.events.iter().find(|e| e.id == id).cloned() else { return false };
        let mut events = self.events.clone();
        events.retain(|e| e.id != id);
        match store::save(&self.path, &events) {
            Ok(()) => {
                self.events = events;
                self.changes.post();
                self.notice = Some(format!("Cut {}", removed.title));
                self.undo = Some((removed, t + UNDO_MS));
                self.picked = None;
                true
            }
            Err(e) => {
                eprintln!("springboard: calendar: {e}");
                self.copied = None;
                self.notice = None;
                self.error = Some("Could not save. The event is still here.".into());
                false
            }
        }
    }

    /// Pastes the events in `text` onto the shown day and saves them. False (with a message) when there is nothing to
    /// paste or the store cannot take it.
    pub fn paste_events(&mut self, ui: &Ui, text: &str) -> bool {
        self.notice = None;
        self.error = None;
        if self.storage_blocked {
            self.error = Some("Saved events could not be read. Paste is unavailable.".into());
            return false;
        }
        let added = paste(text, self.selected, &self.events);
        let Some(last) = added.last() else {
            self.error = Some("There are no events to paste".into());
            return false;
        };
        let last = last.id;
        let mut events = self.events.clone();
        events.extend(added.iter().cloned());
        store::sort(&mut events);
        match store::save(&self.path, &events) {
            Ok(()) => {
                self.events = events;
                self.changes.post();
                self.picked = Some(last);
                self.notice = Some(match added.len() {
                    1 => "Pasted 1 event".to_string(),
                    n => format!("Pasted {n} events"),
                });
                self.fit_scroll(ui);
                true
            }
            Err(e) => {
                eprintln!("springboard: calendar: {e}");
                self.error = Some("Could not save the pasted events".into());
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ics::Local;
    use super::*;

    fn d(month: u32, day: u32) -> Date {
        Date { year: 2026, month, day }
    }
    fn ev(id: u64, date: Date, title: &str) -> CalendarEvent {
        CalendarEvent { id, date, minute: 600, duration: 45, all_day: false, title: title.into(), location: String::new(), notes: String::new(), repeat: 0, alert: 0 }
    }
    const STAMP: &str = "20261004T120000Z";

    #[test]
    fn a_copied_event_pastes_with_everything_the_store_holds() {
        let mut e = ev(7, d(10, 5), "Dentist");
        e.location = "Laugavegur 5".into();
        e.notes = "Bring the form, ask about x-rays".into();
        e.alert = 3;
        e.repeat = 4;
        let text = copy_text(&e, STAMP);
        assert!(pasteable(&text));
        let got = paste(&text, d(10, 20), &[ev(1, d(10, 1), "Other")]);
        assert_eq!(got.len(), 1);
        let p = &got[0];
        assert_eq!(p.id, 2, "a new id after the existing events");
        assert_eq!((p.date, p.minute, p.duration, p.all_day), (d(10, 20), 600, 45, false));
        assert_eq!(p.title, "Dentist");
        assert_eq!(p.location, "Laugavegur 5");
        assert_eq!(p.notes, "Bring the form, ask about x-rays");
        assert_eq!((p.repeat, p.alert), (4, 3));
        assert!(p.valid());
    }

    #[test]
    fn every_preset_repeat_and_alert_survives_the_round_trip() {
        for repeat in 0..store::REPEATS.len() as u8 {
            for alert in 0..store::ALERTS.len() as u8 {
                let mut e = ev(1, d(10, 5), "Round trip");
                e.repeat = repeat;
                e.alert = alert;
                let got = paste(&copy_text(&e, STAMP), d(10, 5), &[]);
                assert_eq!((got[0].repeat, got[0].alert), (repeat, alert), "repeat {repeat} alert {alert}");
            }
        }
    }

    #[test]
    fn an_all_day_event_stays_all_day() {
        let mut e = ev(1, d(10, 5), "Holiday");
        e.all_day = true;
        e.minute = 0;
        e.duration = 1440;
        let got = paste(&copy_text(&e, STAMP), d(11, 2), &[]);
        assert_eq!((got[0].date, got[0].all_day, got[0].duration), (d(11, 2), true, 1440));
    }

    #[test]
    fn several_events_keep_their_distance_from_the_earliest() {
        let cal = ics::IcsCalendar {
            name: String::new(),
            events: vec![
                IcsEvent::new("a", "Second", Local::new(d(10, 7), 540), Local::new(d(10, 7), 600)),
                IcsEvent::new("b", "First", Local::new(d(10, 5), 540), Local::new(d(10, 5), 600)),
            ],
            timezones: vec![],
        };
        let text = ics::write(&cal, STAMP);
        let got = paste(&text, d(12, 1), &[]);
        let days: Vec<(String, Date)> = got.iter().map(|e| (e.title.clone(), e.date)).collect();
        assert_eq!(days, vec![("Second".to_string(), d(12, 3)), ("First".to_string(), d(12, 1))]);
        assert_ne!(got[0].id, got[1].id);
    }

    #[test]
    fn a_rule_the_store_cannot_hold_pastes_as_a_single_event() {
        let mut e = IcsEvent::new("a", "Gym", Local::new(d(10, 5), 540), Local::new(d(10, 5), 600));
        e.rule = recur::parse_rrule("FREQ=WEEKLY;BYDAY=MO,WE");
        let text = ics::write(&ics::IcsCalendar { name: String::new(), events: vec![e], timezones: vec![] }, STAMP);
        let got = paste(&text, d(10, 12), &[]);
        assert_eq!((got.len(), got[0].repeat), (1, 0));
    }

    #[test]
    fn line_breaks_in_notes_become_spaces_and_long_text_is_cut() {
        assert_eq!(clean("line one\nline two\r\n"), "line one line two");
        assert_eq!(clean(&"x".repeat(500)).chars().count(), FIELD_MAX);
        let mut e = IcsEvent::new("a", "Notes", Local::new(d(10, 5), 540), Local::new(d(10, 5), 600));
        e.notes = "first\nsecond".into();
        let text = ics::write(&ics::IcsCalendar { name: String::new(), events: vec![e], timezones: vec![] }, STAMP);
        assert_eq!(paste(&text, d(10, 5), &[])[0].notes, "first second");
    }

    #[test]
    fn text_without_events_pastes_nothing() {
        assert!(!pasteable("buy milk"));
        assert!(paste("buy milk", d(10, 5), &[]).is_empty());
        assert!(paste("BEGIN:VCALENDAR\nEND:VCALENDAR\n", d(10, 5), &[]).is_empty());
    }

    #[test]
    fn pasting_stops_at_the_store_limit() {
        let existing: Vec<CalendarEvent> = (1..=store::MAX_EVENTS as u64).map(|i| ev(i, d(10, 1), "x")).collect();
        let text = copy_text(&ev(1, d(10, 5), "One more"), STAMP);
        assert!(paste(&text, d(10, 5), &existing).is_empty());
        assert_eq!(paste(&text, d(10, 5), &existing[..existing.len() - 1]).len(), 1);
    }
}
