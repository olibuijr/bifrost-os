//! Read-only calendars shown beside the saved events: the bundled holidays (slot 0) and the subscribed calendars
//! (slots 1 and up, in the order they were added). Their events are worked out for the years around the shown one, get
//! ids that cannot collide with a saved event's, and are never written to the event store.
use super::exchange::occurrences;
use super::extras;
use super::holidays;
use super::ics;
use super::recur;
use super::store::CalendarEvent;
use super::{chrome, Calendar, Date};
use bifrost_app::kit::colors;

/// The calendar list's rows: the saved events, the holidays, and the subscriptions from `LAYER_SUBS` up. The calendars
/// the user made take `LAYER_CAL` plus their id (`extras::MAX_CALENDARS` is the highest).
pub const LAYER_LOCAL: u32 = 1;
pub const LAYER_HOLIDAYS: u32 = 2;
pub const LAYER_SUBS: u32 = 10;
pub const LAYER_CAL: u32 = 40;

/// Ids from here up belong to read-only events. A calendar's events take a block of `SLOT` ids.
pub const BASE_ID: u64 = 1 << 40;
const SLOT: u64 = 1 << 20;
/// Most events one subscribed calendar contributes to the years shown.
pub const MAX_PER_CALENDAR: usize = 5000;

/// Whether `id` names a read-only event.
pub fn is_overlay(id: u64) -> bool {
    id >= BASE_ID
}

/// The id of event `n` of the calendar in `slot`.
pub fn id_for(slot: usize, n: usize) -> u64 {
    BASE_ID + slot as u64 * SLOT + n as u64
}

/// The calendar (0 holidays, 1 and up subscriptions) an overlay id belongs to.
pub fn slot_of(id: u64) -> usize {
    (id.saturating_sub(BASE_ID) / SLOT) as usize
}

/// The colour of the calendar in `slot`: its dot in the calendar list and its events.
pub fn colour(slot: usize) -> u32 {
    [
        colors::SYSTEM_GREEN.now(),
        colors::SYSTEM_ORANGE.now(),
        colors::SYSTEM_PURPLE.now(),
        colors::SYSTEM_PINK.now(),
        colors::SYSTEM_TEAL.now(),
        colors::SYSTEM_INDIGO.now(),
    ][slot % 6]
}

/// The holidays of the years `first..=last` as all-day events.
pub fn holiday_events(first: i64, last: i64) -> Vec<CalendarEvent> {
    let mut out = vec![];
    for year in first..=last {
        for (date, name) in holidays::iceland(year) {
            out.push(CalendarEvent {
                id: id_for(0, out.len()),
                date,
                minute: 0,
                duration: 1440,
                all_day: true,
                title: name.to_string(),
                location: String::new(),
                notes: String::new(),
                repeat: 0,
                alert: 0,
            });
        }
    }
    out
}

/// The events of an .ics text that fall on `from..=to`, repeats expanded, in calendar `slot`.
pub fn ics_events(text: &str, slot: usize, from: Date, to: Date) -> Vec<CalendarEvent> {
    let cal = ics::parse(text);
    let mut out: Vec<CalendarEvent> = vec![];
    for e in &cal.events {
        let Some(base) = ics::to_calendar_event(e, 1) else { continue };
        let location: String = e.location.chars().filter(|c| !c.is_control()).take(160).collect();
        for date in occurrences(e, from, to) {
            if date < from || date > to {
                continue;
            }
            if out.len() >= MAX_PER_CALENDAR {
                return out;
            }
            out.push(CalendarEvent { id: id_for(slot, out.len()), date, location: location.trim().to_string(), ..base.clone() });
        }
    }
    out
}

impl Calendar {
    /// The events the grids show: the saved ones (unless the Calendar row of the calendar list is off), then the
    /// read-only ones of the calendars that are on.
    pub(super) fn shown_events(&self) -> impl Iterator<Item = &CalendarEvent> + '_ {
        self.events.iter().filter(move |e| self.local_shown(e.id)).chain(self.overlay.iter())
    }

    /// Whether the saved event `id` is on a calendar that is shown. The built-in calendar follows the Calendar row; the
    /// calendars the user made follow their own rows.
    pub(super) fn local_shown(&self, id: u64) -> bool {
        let cal = self.extras.calendar_of(id);
        if cal == extras::DEFAULT_CALENDAR {
            self.prefs.show_local
        } else {
            self.extras.calendars.iter().find(|c| c.id == cal).map_or(true, |c| c.shown)
        }
    }

    /// The colour of a saved calendar: the accent for the built-in one, else the calendar's palette colour.
    pub(super) fn calendar_colour(&self, cal: u32) -> u32 {
        if cal == extras::DEFAULT_CALENDAR {
            return chrome::cal_colour();
        }
        self.extras.calendars.iter().find(|c| c.id == cal).map_or_else(chrome::cal_colour, |c| colour(c.colour as usize))
    }

    /// Whether event `e` falls on `d`. A custom repeat rule is worked out here; the presets are `CalendarEvent::occurs_on`.
    pub(super) fn occurs(&self, e: &CalendarEvent, d: Date) -> bool {
        match self.extras.events.get(&e.id).and_then(|x| x.rrule.as_deref()).and_then(recur::parse_rrule) {
            Some(rule) => !recur::expand(&rule, e.date, &[], d, d).is_empty(),
            None => e.occurs_on(d),
        }
    }

    /// Works the read-only events out again when the shown year or the set of calendars changed. They cover the shown
    /// year and the ones either side, so stepping across New Year needs no wait.
    pub(super) fn sync_overlay(&mut self) {
        let key = (self.selected.year, self.overlay_version);
        if self.overlay_for == Some(key) {
            return;
        }
        let (first, last) = ((self.selected.year - 1).max(1), (self.selected.year + 1).min(9999));
        let (from, to) = (Date { year: first, month: 1, day: 1 }, Date { year: last, month: 12, day: 31 });
        let mut events = vec![];
        if self.prefs.show_holidays {
            events.extend(holiday_events(first, last));
        }
        for (i, s) in self.subs.iter().enumerate().filter(|(_, s)| s.shown) {
            events.extend(ics_events(&s.text, i + 1, from, to));
        }
        self.overlay = events;
        self.overlay_for = Some(key);
    }

    /// The colour of event `id`: the calendar's own for a read-only event, else the accent.
    pub(super) fn event_colour(&self, id: u64) -> u32 {
        if is_overlay(id) {
            colour(slot_of(id))
        } else {
            self.calendar_colour(self.extras.calendar_of(id))
        }
    }

    /// A tap on a read-only event says what it is and which calendar it is from; it cannot be edited.
    pub(super) fn explain_overlay(&mut self, id: u64) {
        let Some(title) = self.overlay.iter().find(|e| e.id == id).map(|e| e.title.clone()) else { return };
        let slot = slot_of(id);
        let calendar = if slot == 0 { "Holidays".to_string() } else { self.subs.get(slot - 1).map_or_else(String::new, |s| s.name.clone()) };
        self.notice = Some(format!("{title} ({calendar})"));
    }

    /// A click on a row of the calendar list shows or hides that calendar.
    pub(super) fn toggle_layer(&mut self, id: u32) {
        match id {
            LAYER_LOCAL => {
                self.prefs.show_local = !self.prefs.show_local;
                self.save_prefs();
            }
            LAYER_HOLIDAYS => {
                self.prefs.show_holidays = !self.prefs.show_holidays;
                self.save_prefs();
            }
            n if (LAYER_CAL..=LAYER_CAL + extras::MAX_CALENDARS).contains(&n) => {
                let Some(c) = self.extras.calendars.iter_mut().find(|c| c.id == n - LAYER_CAL) else { return };
                c.shown = !c.shown;
                self.save_extras();
            }
            n if (LAYER_SUBS..LAYER_CAL).contains(&n) => {
                let Some(s) = self.subs.get_mut((n - LAYER_SUBS) as usize) else { return };
                s.shown = !s.shown;
                self.save_subscriptions();
            }
            _ => return,
        }
        self.overlay_version += 1;
        self.sync_overlay();
    }
}

#[cfg(test)]
mod tests {
    use super::super::ics::{IcsCalendar, IcsEvent, Local};
    use super::super::recur;
    use super::*;

    fn d(month: u32, day: u32) -> Date {
        Date { year: 2026, month, day }
    }
    fn calendar_text(events: Vec<IcsEvent>) -> String {
        ics::write(&IcsCalendar { name: "Fixtures".into(), events, timezones: vec![] }, "20261004T000000Z")
    }

    #[test]
    fn overlay_ids_are_far_from_saved_ids_and_map_back_to_their_calendar() {
        assert!(!is_overlay(1) && !is_overlay(2048) && !is_overlay(BASE_ID - 1));
        assert!(is_overlay(BASE_ID));
        for (slot, n) in [(0, 0), (0, 4999), (1, 0), (3, 17)] {
            let id = id_for(slot, n);
            assert!(is_overlay(id));
            assert_eq!(slot_of(id), slot);
        }
        assert_ne!(id_for(0, 5), id_for(1, 5));
    }

    #[test]
    fn holidays_are_all_day_events_with_distinct_ids() {
        let events = holiday_events(2025, 2027);
        assert_eq!(events.len(), 3 * holidays::iceland(2026).len());
        assert!(events.iter().all(|e| e.all_day && e.duration == 1440 && e.minute == 0 && is_overlay(e.id) && slot_of(e.id) == 0));
        let mut ids: Vec<u64> = events.iter().map(|e| e.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), events.len());
        assert!(events.iter().any(|e| e.title == "Good Friday" && e.date == d(4, 3)));
    }

    #[test]
    fn a_calendar_text_gives_its_events_in_the_window_with_repeats_expanded() {
        let mut gym = IcsEvent::new("gym@x", "Gym", Local::new(d(10, 5), 600), Local::new(d(10, 5), 660));
        gym.rule = recur::parse_rrule("FREQ=WEEKLY;COUNT=4");
        gym.location = "Hall\n2".into();
        let once = IcsEvent::new("once@x", "Concert", Local::new(d(12, 1), 1200), Local::new(d(12, 1), 1320));
        let early = IcsEvent::new("early@x", "Old news", Local::new(d(1, 5), 600), Local::new(d(1, 5), 660));
        let text = calendar_text(vec![gym, once, early]);
        let got = ics_events(&text, 2, d(10, 1), d(12, 31));
        let titles: Vec<(&str, Date)> = got.iter().map(|e| (e.title.as_str(), e.date)).collect();
        assert_eq!(titles, vec![("Gym", d(10, 5)), ("Gym", d(10, 12)), ("Gym", d(10, 19)), ("Gym", d(10, 26)), ("Concert", d(12, 1))]);
        assert!(got.iter().all(|e| slot_of(e.id) == 2 && e.valid()));
        assert_eq!(got[0].location, "Hall2", "a line break is dropped");
        assert_eq!((got[0].minute, got[0].duration), (600, 60));
    }

    #[test]
    fn text_that_is_not_a_calendar_gives_nothing() {
        assert!(ics_events("<html>Not found</html>", 1, d(1, 1), d(12, 31)).is_empty());
        assert!(ics_events("", 1, d(1, 1), d(12, 31)).is_empty());
    }

    #[test]
    fn colours_cycle_through_the_palette() {
        assert_eq!(colour(0), colour(6));
        assert_ne!(colour(0), colour(1));
    }
}
