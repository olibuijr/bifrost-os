//! Parity app-calendar: headless tests of the Quick Event notes, the travel time block in the Day and Week grids and
//! the date range of event search. No test draws a frame or opens a window.

use bifrost_calendar::calendar::ics::{IcsEvent, Local};
use bifrost_calendar::calendar::layout::travel_span;
use bifrost_calendar::calendar::quick::{parse, Quick};
use bifrost_calendar::calendar::search::{search, split_range, Range};
use bifrost_calendar::calendar::store::CalendarEvent;
use bifrost_calendar::calendar::Date;

/// Tuesday 13 October 2026.
fn today() -> Date {
    Date { year: 2026, month: 10, day: 13 }
}

fn day(day: u32) -> Date {
    Date { year: 2026, month: 10, day }
}

fn saved(id: u64, on: u32, title: &str) -> CalendarEvent {
    CalendarEvent { id, date: day(on), minute: 540, duration: 60, all_day: false, title: title.into(), location: String::new(), notes: String::new(), repeat: 0, alert: 0 }
}

#[test]
fn a_note_after_the_marker_becomes_the_notes_and_leaves_the_title() {
    let q: Quick = parse("Lunch with Ana tomorrow at 12:30 note: bring the slides", today());
    assert_eq!(q.title, "Lunch with Ana");
    assert_eq!(q.date, Some(day(14)));
    assert_eq!(q.minute, Some(12 * 60 + 30));
    assert_eq!(q.notes.as_deref(), Some("bring the slides"));
}

#[test]
fn the_notes_marker_is_read_in_any_case_and_as_a_plural() {
    assert_eq!(parse("Dentist NOTES: floss first", today()).notes.as_deref(), Some("floss first"));
    assert_eq!(parse("Dentist Note: call ahead", today()).notes.as_deref(), Some("call ahead"));
}

#[test]
fn a_marker_inside_a_word_is_not_a_note() {
    let q = parse("Read footnote: chapter two", today());
    assert_eq!(q.notes, None);
    assert_eq!(q.title, "Read footnote: chapter two");
}

#[test]
fn an_empty_note_is_no_note() {
    assert_eq!(parse("Dentist tomorrow notes:   ", today()).notes, None);
}

#[test]
fn the_notes_do_not_change_where_the_event_is() {
    let q = parse("Team lunch at Laugavegur 5 note: vegetarian options", today());
    assert_eq!(q.location.as_deref(), Some("Laugavegur 5"));
    assert_eq!(q.notes.as_deref(), Some("vegetarian options"));
}

#[test]
fn travel_time_extends_the_block_before_the_start() {
    // 09:00 start, 30 minutes of travel, one hour long: the block runs from 08:30 to 10:00
    assert_eq!(travel_span(540, 60, 30), (510, 600));
}

#[test]
fn travel_time_never_starts_the_block_before_midnight() {
    assert_eq!(travel_span(10, 60, 30), (0, 70));
}

#[test]
fn an_event_without_travel_keeps_its_own_span() {
    assert_eq!(travel_span(540, 60, 0), (540, 600));
}

#[test]
fn the_block_never_runs_past_midnight() {
    assert_eq!(travel_span(1400, 120, 0), (1400, 1440));
}

#[test]
fn a_from_and_a_to_word_set_the_range_and_leave_the_terms() {
    let (range, terms) = split_range("from:2026-10-05 to:2026-10-09 lunch");
    assert_eq!(range, Range { from: Some(day(5)), to: Some(day(9)) });
    assert_eq!(terms, "lunch");
}

#[test]
fn a_date_word_that_is_not_a_date_stays_a_term() {
    let (range, terms) = split_range("from:2026-13-40 sync");
    assert_eq!(range, Range::default());
    assert_eq!(terms, "from:2026-13-40 sync");
}

#[test]
fn the_range_is_inclusive_at_both_ends() {
    let range = Range { from: Some(day(5)), to: Some(day(9)) };
    assert!(range.contains(day(5)));
    assert!(range.contains(day(9)));
    assert!(!range.contains(day(4)));
    assert!(!range.contains(day(10)));
    assert!(Range::default().contains(day(1)), "an open range holds every day");
}

#[test]
fn a_search_limited_to_a_range_finds_only_the_events_in_it() {
    let store = vec![saved(1, 4, "Lunch"), saved(2, 6, "Lunch with Ann"), saved(3, 12, "Lunch")];
    let titles: Vec<_> = search("from:2026-10-05 to:2026-10-09 lunch", &store, &[]).into_iter().map(|h| h.title).collect();
    assert_eq!(titles, vec!["Lunch with Ann"]);
}

#[test]
fn a_date_range_alone_lists_every_event_in_it() {
    let store = vec![saved(1, 4, "Lunch"), saved(2, 6, "Standup"), saved(3, 12, "Review")];
    let titles: Vec<_> = search("from:2026-10-06", &store, &[]).into_iter().map(|h| h.title).collect();
    assert_eq!(titles, vec!["Standup", "Review"]);
}

#[test]
fn a_range_applies_to_imported_events_by_their_start_day() {
    let mut rich = IcsEvent::new("u1", "Offsite", Local::new(day(20), 540), Local::new(day(20), 600));
    rich.notes = "deck".into();
    let hits = search("to:2026-10-09 deck", &[], &[rich.clone()]);
    assert!(hits.is_empty(), "the event starts after the range ends");
    let hits = search("from:2026-10-19 deck", &[], &[rich]);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "Offsite");
}

#[test]
fn a_query_with_neither_words_nor_range_still_matches_nothing() {
    assert!(search("", &[saved(1, 1, "x")], &[]).is_empty());
    assert!(search("   ", &[saved(1, 1, "x")], &[]).is_empty());
}
