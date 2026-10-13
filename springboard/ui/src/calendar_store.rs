//! The write side of a calendar store (`EKEventStore`): save and remove events and reminders, queue changes until
//! `commit()` or throw them away with `reset()`, and fetch what was committed. An `Access` value gates every call:
//! writing needs a write grant, reading needs full access. Removing one occurrence of a repeating event follows
//! `EKSpan`: `ThisEvent` skips that occurrence, `FutureEvents` ends the series just before it.
//!
//! Pure in-memory model with host unit tests. Times are seconds since 1970-01-01. It is not wired to the shell store
//! (`src/shell/calendar/store.rs`) yet. Saving replaces the whole stored item, whatever the span; attendees and
//! recurrence editing are not modelled.

use crate::calendar::Date;
use crate::calendar_access::{Access, EntityType};
use crate::calendar_error::ErrorCode;
use crate::calendar_store_changes::StoreChangeHub;

const DAY: i64 = 86_400;

/// `EKSpan`: how far a change to a repeating event reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    ThisEvent,
    FutureEvents,
}

/// `EKEvent`. An empty `calendar_item_identifier` means the event was never saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub calendar_item_identifier: String,
    pub title: String,
    pub start_date: i64,
    pub end_date: i64,
    pub is_all_day: bool,
    /// Repeat every this many days; `None` for a single event.
    pub repeat_days: Option<u32>,
}

impl Event {
    pub fn new(title: &str, start_date: i64, end_date: i64) -> Self {
        Event {
            calendar_item_identifier: String::new(),
            title: title.to_string(),
            start_date,
            end_date,
            is_all_day: false,
            repeat_days: None,
        }
    }
}

/// `EKReminder`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    pub calendar_item_identifier: String,
    pub title: String,
    pub due_date_components: Option<Date>,
    pub is_completed: bool,
}

impl Reminder {
    pub fn new(title: &str) -> Self {
        Reminder {
            calendar_item_identifier: String::new(),
            title: title.to_string(),
            due_date_components: None,
            is_completed: false,
        }
    }
}

/// What `predicateForReminders(in:)` and its incomplete/completed variants select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReminderPredicate {
    All,
    Incomplete,
    Completed,
}

#[derive(Clone, Debug)]
struct StoredEvent {
    event: Event,
    /// Occurrence start times removed with `Span::ThisEvent`.
    skipped: Vec<i64>,
    /// First start time that no longer occurs (set by `Span::FutureEvents`).
    ends_before: Option<i64>,
}

#[derive(Clone, Debug)]
enum Change {
    SaveEvent(Event),
    RemoveEvent(Event, Span),
    SaveReminder(Reminder),
    RemoveReminder(String),
}

/// The store: committed items plus the queue of changes made with `commit: false`.
#[derive(Debug, Default)]
pub struct EventStore {
    access: Access,
    events: Vec<StoredEvent>,
    reminders: Vec<Reminder>,
    pending: Vec<Change>,
    next_id: u64,
    changes: StoreChangeHub,
}

impl EventStore {
    pub fn new(access: Access) -> Self {
        EventStore { access, ..Default::default() }
    }

    /// The change counter; it rises once per `commit()` that applied something.
    pub fn change_generation(&self) -> u64 {
        self.changes.generation()
    }

    fn need_write(&self, e: EntityType) -> Result<(), ErrorCode> {
        if self.access.can_write(e) { Ok(()) } else { Err(ErrorCode::EventStoreNotAuthorized) }
    }

    fn need_read(&self, e: EntityType) -> Result<(), ErrorCode> {
        if self.access.can_read(e) { Ok(()) } else { Err(ErrorCode::EventStoreNotAuthorized) }
    }

    fn new_id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}-{}", self.next_id)
    }

    /// `save(_:span:commit:)`. Assigns `calendar_item_identifier` on a first save.
    pub fn save_event(&mut self, event: &mut Event, _span: Span, commit: bool) -> Result<(), ErrorCode> {
        self.need_write(EntityType::Event)?;
        if event.end_date < event.start_date {
            return Err(ErrorCode::DatesInverted);
        }
        if event.calendar_item_identifier.is_empty() {
            event.calendar_item_identifier = self.new_id("event");
        }
        self.pending.push(Change::SaveEvent(event.clone()));
        self.finish(commit)
    }

    /// `remove(_:span:commit:)`.
    pub fn remove_event(&mut self, event: &Event, span: Span, commit: bool) -> Result<(), ErrorCode> {
        self.need_write(EntityType::Event)?;
        if !self.events.iter().any(|s| s.event.calendar_item_identifier == event.calendar_item_identifier) {
            return Err(ErrorCode::InternalFailure);
        }
        self.pending.push(Change::RemoveEvent(event.clone(), span));
        self.finish(commit)
    }

    /// `save(_:commit:)` for reminders. Assigns the identifier on a first save.
    pub fn save_reminder(&mut self, reminder: &mut Reminder, commit: bool) -> Result<(), ErrorCode> {
        self.need_write(EntityType::Reminder)?;
        if reminder.calendar_item_identifier.is_empty() {
            reminder.calendar_item_identifier = self.new_id("reminder");
        }
        self.pending.push(Change::SaveReminder(reminder.clone()));
        self.finish(commit)
    }

    /// `remove(_:commit:)` for reminders.
    pub fn remove_reminder(&mut self, reminder: &Reminder, commit: bool) -> Result<(), ErrorCode> {
        self.need_write(EntityType::Reminder)?;
        let id = &reminder.calendar_item_identifier;
        if !self.reminders.iter().any(|r| &r.calendar_item_identifier == id) {
            return Err(ErrorCode::InternalFailure);
        }
        self.pending.push(Change::RemoveReminder(id.clone()));
        self.finish(commit)
    }

    fn finish(&mut self, commit: bool) -> Result<(), ErrorCode> {
        if commit { self.commit() } else { Ok(()) }
    }

    /// `commit()`: applies the queued changes in order.
    pub fn commit(&mut self) -> Result<(), ErrorCode> {
        let queued = std::mem::take(&mut self.pending);
        if queued.is_empty() {
            return Ok(());
        }
        for change in queued {
            self.apply(change);
        }
        self.changes.post();
        Ok(())
    }

    /// `reset()`: drops the queued changes.
    pub fn reset(&mut self) {
        self.pending.clear();
    }

    fn apply(&mut self, change: Change) {
        match change {
            Change::SaveEvent(e) => {
                let id = e.calendar_item_identifier.clone();
                let stored = StoredEvent { event: e, skipped: Vec::new(), ends_before: None };
                match self.events.iter_mut().find(|s| s.event.calendar_item_identifier == id) {
                    Some(slot) => *slot = stored,
                    None => self.events.push(stored),
                }
            }
            Change::RemoveEvent(e, span) => {
                let id = &e.calendar_item_identifier;
                let Some(i) = self.events.iter().position(|s| &s.event.calendar_item_identifier == id) else {
                    return;
                };
                if self.events[i].event.repeat_days.is_none() {
                    self.events.remove(i);
                    return;
                }
                match span {
                    Span::ThisEvent => self.events[i].skipped.push(e.start_date),
                    Span::FutureEvents if e.start_date <= self.events[i].event.start_date => {
                        self.events.remove(i);
                    }
                    Span::FutureEvents => self.events[i].ends_before = Some(e.start_date),
                }
            }
            Change::SaveReminder(r) => {
                let id = r.calendar_item_identifier.clone();
                match self.reminders.iter_mut().find(|x| x.calendar_item_identifier == id) {
                    Some(slot) => *slot = r,
                    None => self.reminders.push(r),
                }
            }
            Change::RemoveReminder(id) => self.reminders.retain(|r| r.calendar_item_identifier != id),
        }
    }

    /// `fetchReminders(matching:)`, with the result returned directly.
    pub fn fetch_reminders(&self, predicate: ReminderPredicate) -> Result<Vec<Reminder>, ErrorCode> {
        self.need_read(EntityType::Reminder)?;
        Ok(self
            .reminders
            .iter()
            .filter(|r| match predicate {
                ReminderPredicate::All => true,
                ReminderPredicate::Incomplete => !r.is_completed,
                ReminderPredicate::Completed => r.is_completed,
            })
            .cloned()
            .collect())
    }

    /// `events(matching:)` for a date range: every occurrence starting in `[from, to)`, ordered by start.
    pub fn events_between(&self, from: i64, to: i64) -> Result<Vec<Event>, ErrorCode> {
        self.need_read(EntityType::Event)?;
        let mut out = Vec::new();
        for s in &self.events {
            let e = &s.event;
            let step = e.repeat_days.map(|d| i64::from(d.max(1)) * DAY);
            let length = e.end_date - e.start_date;
            let mut start = e.start_date;
            while start < to {
                if s.ends_before.is_some_and(|b| start >= b) {
                    break;
                }
                if start >= from && !s.skipped.contains(&start) {
                    out.push(Event { start_date: start, end_date: start + length, ..e.clone() });
                }
                match step {
                    Some(step) => start += step,
                    None => break,
                }
            }
        }
        out.sort_by_key(|e| e.start_date);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> EventStore {
        let mut a = Access::new();
        a.request_full_events(true);
        a.request_full_reminders(true);
        EventStore::new(a)
    }

    fn weekly() -> Event {
        let mut e = Event::new("Standup", 0, 3600);
        e.repeat_days = Some(7);
        e
    }

    #[test]
    fn saving_assigns_an_identifier_and_the_event_can_be_fetched() {
        let mut s = full();
        let mut e = Event::new("Lunch", 1000, 4600);
        s.save_event(&mut e, Span::ThisEvent, true).unwrap();
        assert!(!e.calendar_item_identifier.is_empty());
        assert_eq!(s.events_between(0, 10_000).unwrap(), vec![e]);
    }

    #[test]
    fn an_uncommitted_save_is_invisible_until_commit_and_reset_drops_it() {
        let mut s = full();
        let mut e = Event::new("Lunch", 1000, 4600);
        s.save_event(&mut e, Span::ThisEvent, false).unwrap();
        assert!(s.events_between(0, 10_000).unwrap().is_empty());
        s.reset();
        s.commit().unwrap();
        assert!(s.events_between(0, 10_000).unwrap().is_empty());
        s.save_event(&mut e, Span::ThisEvent, false).unwrap();
        s.commit().unwrap();
        assert_eq!(s.events_between(0, 10_000).unwrap().len(), 1);
    }

    #[test]
    fn saving_twice_replaces_the_event() {
        let mut s = full();
        let mut e = Event::new("Lunch", 1000, 4600);
        s.save_event(&mut e, Span::ThisEvent, true).unwrap();
        e.title = "Brunch".into();
        s.save_event(&mut e, Span::ThisEvent, true).unwrap();
        let got = s.events_between(0, 10_000).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].title, "Brunch");
    }

    #[test]
    fn an_event_that_ends_before_it_starts_is_refused() {
        let mut s = full();
        let mut e = Event::new("Bad", 5000, 1000);
        assert_eq!(s.save_event(&mut e, Span::ThisEvent, true), Err(ErrorCode::DatesInverted));
    }

    #[test]
    fn removing_a_single_event_deletes_it() {
        let mut s = full();
        let mut e = Event::new("Lunch", 1000, 4600);
        s.save_event(&mut e, Span::ThisEvent, true).unwrap();
        s.remove_event(&e, Span::ThisEvent, true).unwrap();
        assert!(s.events_between(0, 10_000).unwrap().is_empty());
        assert_eq!(s.remove_event(&e, Span::ThisEvent, true), Err(ErrorCode::InternalFailure));
    }

    #[test]
    fn this_event_skips_one_occurrence_of_a_series() {
        let mut s = full();
        let mut e = weekly();
        s.save_event(&mut e, Span::FutureEvents, true).unwrap();
        let second = Event { start_date: 7 * DAY, end_date: 7 * DAY + 3600, ..e.clone() };
        s.remove_event(&second, Span::ThisEvent, true).unwrap();
        let starts: Vec<i64> = s.events_between(0, 22 * DAY).unwrap().iter().map(|e| e.start_date).collect();
        assert_eq!(starts, vec![0, 14 * DAY, 21 * DAY]);
    }

    #[test]
    fn future_events_ends_the_series_before_the_occurrence() {
        let mut s = full();
        let mut e = weekly();
        s.save_event(&mut e, Span::FutureEvents, true).unwrap();
        let second = Event { start_date: 7 * DAY, end_date: 7 * DAY + 3600, ..e.clone() };
        s.remove_event(&second, Span::FutureEvents, true).unwrap();
        let starts: Vec<i64> = s.events_between(0, 60 * DAY).unwrap().iter().map(|e| e.start_date).collect();
        assert_eq!(starts, vec![0]);
    }

    #[test]
    fn future_events_on_the_first_occurrence_removes_the_whole_series() {
        let mut s = full();
        let mut e = weekly();
        s.save_event(&mut e, Span::FutureEvents, true).unwrap();
        s.remove_event(&e, Span::FutureEvents, true).unwrap();
        assert!(s.events_between(0, 60 * DAY).unwrap().is_empty());
    }

    #[test]
    fn reminders_save_complete_fetch_and_remove() {
        let mut s = full();
        let mut a = Reminder::new("Milk");
        a.due_date_components = Some(Date { year: 2026, month: 10, day: 14 });
        let mut b = Reminder::new("Tax");
        b.is_completed = true;
        s.save_reminder(&mut a, true).unwrap();
        s.save_reminder(&mut b, true).unwrap();
        assert_eq!(s.fetch_reminders(ReminderPredicate::All).unwrap().len(), 2);
        assert_eq!(s.fetch_reminders(ReminderPredicate::Incomplete).unwrap(), vec![a.clone()]);
        assert_eq!(s.fetch_reminders(ReminderPredicate::Completed).unwrap(), vec![b]);
        s.remove_reminder(&a, true).unwrap();
        assert_eq!(s.fetch_reminders(ReminderPredicate::All).unwrap().len(), 1);
    }

    #[test]
    fn writes_need_a_grant_and_reads_need_full_access() {
        let mut e = Event::new("Lunch", 0, 10);
        let mut s = EventStore::new(Access::new());
        assert_eq!(s.save_event(&mut e, Span::ThisEvent, true), Err(ErrorCode::EventStoreNotAuthorized));
        assert_eq!(s.fetch_reminders(ReminderPredicate::All), Err(ErrorCode::EventStoreNotAuthorized));

        let mut a = Access::new();
        a.request_write_only_events(true);
        let mut s = EventStore::new(a);
        s.save_event(&mut e, Span::ThisEvent, true).unwrap();
        assert_eq!(s.events_between(0, 100), Err(ErrorCode::EventStoreNotAuthorized));
    }

    #[test]
    fn each_commit_that_applies_changes_posts_one_store_change() {
        let mut s = full();
        let mut a = Event::new("A", 0, 1);
        let mut b = Event::new("B", 0, 1);
        s.save_event(&mut a, Span::ThisEvent, false).unwrap();
        s.save_event(&mut b, Span::ThisEvent, false).unwrap();
        s.commit().unwrap();
        s.commit().unwrap();
        assert_eq!(s.change_generation(), 1);
    }
}
