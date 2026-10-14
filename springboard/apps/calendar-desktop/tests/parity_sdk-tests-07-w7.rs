//! Parity sdk-tests-07 (wave 7): headless tests for event alarms. Alarms are computed from calendar data in memory: which
//! alerts fall in a time window, the text of each alert, time zones, recurring events with exceptions, and the rule that
//! decides when the calendar is rebuilt. No notification is posted, no clock is read and no file is opened.

use std::cell::Cell;

use bifrost_calendar::calendar::alerts::{due, step, AlertState, ALERT_MINUTES, REPEAT_RULES};
use bifrost_calendar::calendar::ics::{IcsCalendar, IcsEvent, Local};
use bifrost_calendar::calendar::recur;
use bifrost_calendar::calendar::Date;

fn d(month: u32, day: u32) -> Date {
    Date { year: 2026, month, day }
}

fn unix(date: Date, hour: i64, minute: i64, offset: i32) -> i64 {
    date.days() * 86_400 + hour * 3600 + minute * 60 - offset as i64
}

/// A ten o'clock dentist appointment on the fifth of October, with the given alarms (minutes before the start).
fn dentist(alarms: Vec<u32>) -> IcsEvent {
    let mut e = IcsEvent::new("u", "Dentist", Local::new(d(10, 5), 600), Local::new(d(10, 5), 660));
    e.alarms = alarms;
    e
}

fn calendar_of(events: Vec<IcsEvent>) -> IcsCalendar {
    IcsCalendar { name: String::new(), events, timezones: vec![] }
}

#[test]
fn headless_the_alarm_choices_are_none_then_the_presets_from_the_same_minute_to_a_day() {
    assert_eq!(ALERT_MINUTES.len(), 7);
    assert_eq!(ALERT_MINUTES[0], None, "the first choice is no alarm");
    assert_eq!(ALERT_MINUTES[1], Some(0), "at the start time");
    assert_eq!(ALERT_MINUTES[3], Some(15));
    assert_eq!(ALERT_MINUTES[6], Some(1440), "one day before");
}

#[test]
fn headless_the_repeat_choices_start_with_no_rule_and_end_with_yearly() {
    assert_eq!(REPEAT_RULES.len(), 6);
    assert_eq!(REPEAT_RULES[0], "");
    assert_eq!(REPEAT_RULES[2], "FREQ=WEEKLY");
    assert_eq!(REPEAT_RULES[5], "FREQ=YEARLY");
}

#[test]
fn headless_a_single_alarm_fires_once_inside_its_window_with_the_event_details() {
    let cal = calendar_of(vec![dentist(vec![15])]);
    let fire = unix(d(10, 5), 9, 45, 0);
    let got = due(&cal, fire - 60, fire, 0);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].fire, fire);
    assert_eq!(got[0].start, unix(d(10, 5), 10, 0, 0));
    assert_eq!(got[0].uid, "u");
    assert_eq!(got[0].title, "Dentist");
    assert_eq!(got[0].body, "In 15 minutes");
    assert!(due(&cal, fire, fire + 60, 0).is_empty(), "the window is open at the start and closed at the end");
}

#[test]
fn headless_the_alert_text_names_the_lead_time_and_the_location() {
    let mut e = dentist(vec![0, 60, 120, 1440]);
    e.location = "Room 4".into();
    let cal = calendar_of(vec![e]);
    let start = unix(d(10, 5), 10, 0, 0);
    let bodies: Vec<String> = due(&cal, start - 2 * 86_400, start, 0).into_iter().map(|a| a.body).collect();
    assert_eq!(bodies, vec!["In 1 day - Room 4", "In 2 hours - Room 4", "In 1 hour - Room 4", "Now - Room 4"]);
}

#[test]
fn headless_an_all_day_alarm_reads_all_day_and_a_single_day_uses_the_singular() {
    let mut e = dentist(vec![1440]);
    e.all_day = true;
    let cal = calendar_of(vec![e]);
    let start = unix(d(10, 5), 0, 0, 0);
    let got = due(&cal, start - 2 * 86_400, start, 0);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].body, "All day");
    let one_day = calendar_of(vec![dentist(vec![1440])]);
    let got = due(&one_day, start - 2 * 86_400, unix(d(10, 5), 10, 0, 0), 0);
    assert_eq!(got[0].body, "In 1 day", "the singular is used for one day");
}

#[test]
fn headless_a_floating_event_follows_the_device_offset_and_a_utc_event_does_not() {
    let cal = calendar_of(vec![dentist(vec![0])]);
    let fire = unix(d(10, 5), 10, 0, 3600);
    assert_eq!(due(&cal, fire - 1, fire, 3600).len(), 1);
    assert!(due(&cal, fire - 1, fire, 0).is_empty());

    let mut utc = dentist(vec![0]);
    utc.tzid = Some("UTC".into());
    let utc_fire = unix(d(10, 5), 10, 0, 0);
    assert_eq!(due(&calendar_of(vec![utc]), utc_fire - 1, utc_fire, 7200).len(), 1, "UTC ignores the device offset");
}

#[test]
fn headless_a_named_zone_uses_its_own_offset_from_the_calendar_zones() {
    let mut e = dentist(vec![0]);
    e.tzid = Some("Zone/X".into());
    let cal = IcsCalendar { name: String::new(), events: vec![e], timezones: vec![("Zone/X".into(), -300)] };
    let fire = unix(d(10, 5), 10, 0, -300 * 60);
    assert_eq!(due(&cal, fire - 1, fire, 0).len(), 1);
}

#[test]
fn headless_a_recurring_alarm_fires_for_each_occurrence_except_the_exceptions() {
    let mut e = dentist(vec![10]);
    e.rule = recur::parse_rrule("FREQ=DAILY;COUNT=4");
    e.exdates = vec![d(10, 6)];
    let cal = calendar_of(vec![e]);
    let got = due(&cal, unix(d(10, 4), 0, 0, 0), unix(d(10, 9), 0, 0, 0), 0);
    let fires: Vec<i64> = got.iter().map(|a| a.fire).collect();
    let want: Vec<i64> = [5, 7, 8].iter().map(|&day| unix(d(10, day), 9, 50, 0)).collect();
    assert_eq!(fires, want);
}

#[test]
fn headless_a_day_before_alarm_fires_across_midnight_in_time_order() {
    let cal = calendar_of(vec![dentist(vec![1440, 15])]);
    let got = due(&cal, unix(d(10, 3), 0, 0, 0), unix(d(10, 6), 0, 0, 0), 0);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].fire, unix(d(10, 4), 10, 0, 0), "the day-before alarm comes first");
    assert_eq!(got[1].fire, unix(d(10, 5), 9, 45, 0));
}

#[test]
fn headless_alerts_from_several_events_come_out_ordered_by_fire_time() {
    let mut standup = IcsEvent::new("s", "Standup", Local::new(d(10, 5), 570), Local::new(d(10, 5), 585));
    standup.alarms = vec![0];
    let cal = calendar_of(vec![dentist(vec![15]), standup]);
    let titles: Vec<String> = due(&cal, unix(d(10, 5), 9, 0, 0), unix(d(10, 5), 10, 0, 0), 0)
        .into_iter()
        .map(|a| a.title)
        .collect();
    assert_eq!(titles, vec!["Standup", "Dentist"]);
}

#[test]
fn headless_an_empty_or_reversed_window_has_no_alerts() {
    let cal = calendar_of(vec![dentist(vec![0])]);
    assert!(due(&cal, 100, 100, 0).is_empty());
    assert!(due(&cal, 200, 100, 0).is_empty());
}

#[test]
fn headless_the_calendar_is_rebuilt_only_when_an_alert_is_due_or_the_events_change() {
    let loads = Cell::new(0);
    let build = || {
        loads.set(loads.get() + 1);
        calendar_of(vec![dentist(vec![15])])
    };
    let start = unix(d(10, 5), 10, 0, 0);
    let fire = start - 900;
    let mut state = AlertState::default();

    assert!(step(&mut state, start - 3600, 0, 0, &build).is_empty(), "the first poll only schedules");
    assert_eq!(loads.get(), 1);

    let alerts = step(&mut state, fire, 0, 0, &build);
    assert_eq!(alerts.len(), 1, "the alert is posted when its time arrives");
    assert_eq!(alerts[0].fire, fire);
    assert_eq!(loads.get(), 2);

    assert!(step(&mut state, fire + 60, 0, 0, &build).is_empty());
    assert_eq!(loads.get(), 2, "nothing is due and the events did not change, so no rebuild");

    assert!(step(&mut state, fire + 120, 1, 0, &build).is_empty());
    assert_eq!(loads.get(), 3, "a change in the events rebuilds the calendar");
}
