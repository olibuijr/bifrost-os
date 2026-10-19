//! The time zone the grids show times in. An event keeps the wall time of its own zone (the Time Zone row of the event
//! form, `extras.rs`; the device's zone when it names none). The toolbar's Time Zone button and View > Time Zone choose
//! one zone of `zones::TABLE` to read every timed event in instead: the Day and Week grids, the month cells and the
//! agenda then show each event where it falls on that zone's clock, a night in Tokyo on the day before in New York.
//! All-day events keep their day. The saved events are never changed by the view: opening an event shows its own time,
//! and moving or resizing a block in the grid turns the new place back to the event's own clock before it is saved.
use super::zones::{self, Clock};
use super::*;
use std::borrow::Cow;

impl Calendar {
    /// The clock the grids show times on: the chosen zone, None to show each event's own time.
    pub(super) fn view_clock(&self) -> Option<Clock> {
        self.prefs.view_zone.and_then(zones::find).map(Clock::Zone)
    }

    /// The device's offset in minutes east of UTC.
    pub(super) fn device_minutes(&self) -> i32 {
        self.wall.offset / 60
    }

    /// The clock the times of saved event `id` are on: its zone, or the device's.
    pub(super) fn event_clock(&self, id: u64) -> Clock {
        let device = self.device_minutes();
        let tzid = self.extras.events.get(&id).and_then(|x| x.tzid.as_deref());
        Clock::of(tzid, device).unwrap_or(Clock::Device(device))
    }

    /// `e` as the grids show it: moved to the view zone's clock when a view zone is set. An event that does not move, an
    /// all-day event and a read-only one on the device's clock come back as they are.
    pub(super) fn in_view<'a>(&self, e: &'a CalendarEvent) -> Cow<'a, CalendarEvent> {
        let Some(to) = self.view_clock() else { return Cow::Borrowed(e) };
        if e.all_day {
            return Cow::Borrowed(e);
        }
        // a read-only event has no zone of its own: its time is the device's
        let from = if overlay::is_overlay(e.id) { Clock::Device(self.device_minutes()) } else { self.event_clock(e.id) };
        let (date, minute) = zones::convert_clock(from, e.date, e.minute as u32, to);
        if date == e.date && minute == e.minute as u32 {
            return Cow::Borrowed(e);
        }
        let mut shown = e.clone();
        shown.date = date;
        shown.minute = minute as u16;
        Cow::Owned(shown)
    }

    /// The place on the event's own clock of the grid place `(date, minute)` of saved event `id`: what a moved or
    /// resized block is saved as. With no view zone the place is the same.
    pub(super) fn to_own_clock(&self, id: u64, date: Date, minute: u32) -> (Date, u32) {
        match self.view_clock() {
            Some(view) => zones::convert_clock(view, date, minute, self.event_clock(id)),
            None => (date, minute),
        }
    }

    /// The grid place of the own-clock place `(date, minute)` of saved event `id`.
    pub(super) fn to_view_clock(&self, id: u64, date: Date, minute: u32) -> (Date, u32) {
        match self.view_clock() {
            Some(view) => zones::convert_clock(self.event_clock(id), date, minute, view),
            None => (date, minute),
        }
    }

    /// The toolbar button's words: "Time Zone" while each event shows its own time, else the chosen zone's name.
    pub fn view_zone_label(&self) -> &'static str {
        match self.prefs.view_zone {
            None => "Time Zone",
            Some(id) => zones::find(id).map_or("Time Zone", |z| z.title),
        }
    }

    /// Whether View > Time Zone choice `choice` is the current one: None is "Event Time Zones" (each event's own), `Some(i)`
    /// is zone `i` of `zones::TABLE`.
    pub fn view_zone_checked(&self, choice: Option<usize>) -> bool {
        match choice {
            None => self.prefs.view_zone.is_none(),
            Some(i) => zones::TABLE.get(i).is_some_and(|z| Some(z.id) == self.prefs.view_zone),
        }
    }

    /// Chooses the zone the grids show times in (View > Time Zone): None for each event's own time, `Some(i)` for zone `i`
    /// of `zones::TABLE`. False while a sheet or the event form is open, and for a zone that is not in the table.
    pub fn set_view_zone(&mut self, ui: &Ui, choice: Option<usize>) -> bool {
        if !self.settings_enabled() {
            return false;
        }
        let zone = match choice {
            None => None,
            Some(i) => match zones::TABLE.get(i) {
                Some(z) => Some(z.id),
                None => return false,
            },
        };
        self.prefs.view_zone = zone;
        self.save_prefs();
        self.fit_scroll(ui);
        self.dirty = true;
        true
    }

    /// The toolbar button: the next zone of the table, then back to each event's own time.
    pub(super) fn cycle_view_zone(&mut self, ui: &Ui) {
        let at = self.prefs.view_zone.and_then(|id| zones::TABLE.iter().position(|z| z.id == id));
        let next = match at {
            None if self.prefs.view_zone.is_none() => Some(0),
            None => None,
            Some(i) if i + 1 < zones::TABLE.len() => Some(i + 1),
            Some(_) => None,
        };
        self.set_view_zone(ui, next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(year: i64, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn the_cycle_visits_every_zone_and_comes_back_to_each_events_own_time() {
        let table: Vec<&str> = zones::TABLE.iter().map(|z| z.id).collect();
        let step = |current: Option<&'static str>| -> Option<&'static str> {
            let at = current.and_then(|id| zones::TABLE.iter().position(|z| z.id == id));
            match at {
                None if current.is_none() => zones::TABLE.first().map(|z| z.id),
                None => None,
                Some(i) => zones::TABLE.get(i + 1).map(|z| z.id),
            }
        };
        let mut seen = vec![];
        let mut at = step(None);
        while let Some(id) = at {
            seen.push(id);
            at = step(Some(id));
        }
        assert_eq!(seen, table);
    }

    #[test]
    fn a_night_in_tokyo_is_the_day_before_in_new_york() {
        let tokyo = Clock::Zone(zones::find("Asia/Tokyo").unwrap());
        let ny = Clock::Zone(zones::find("America/New_York").unwrap());
        // 01:00 on 20 January in Tokyo is 11:00 on the 19th in New York (winter)
        let (date, minute) = zones::convert_clock(tokyo, d(2026, 1, 20), 60, ny);
        assert_eq!((date, minute), (d(2026, 1, 19), 660));
        // and the grid place turns back to the event's own clock
        assert_eq!(zones::convert_clock(ny, date, minute, tokyo), (d(2026, 1, 20), 60));
    }
}
