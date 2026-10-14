//! Parity sdk-complete-10: headless tests of the proposed meeting time (`proposed_event`): days, clock times,
//! durations and refusals, read from message text. No clock, calendar or message store is touched.

use bifrost_ui::proposed_event::{find_proposal, Day, ProposedEvent, DEFAULT_DURATION_MINUTES};

fn span_text(text: &str, proposal: &ProposedEvent) -> String {
    text[proposal.span.clone()].to_string()
}

#[test]
fn a_weekday_with_at_and_a_clock_time_is_a_proposal() {
    let text = "Are you free Friday at 3pm?";
    let proposal = find_proposal(text).expect("a proposal");
    assert_eq!(proposal.day, Day::Weekday(4));
    assert_eq!(proposal.start_minute, 15 * 60);
    assert_eq!(proposal.duration_minutes, DEFAULT_DURATION_MINUTES);
    assert_eq!(span_text(text, &proposal), "Friday at 3pm");
}

#[test]
fn a_24_hour_clock_with_minutes_needs_no_am_or_pm() {
    let text = "tomorrow 14:30 works";
    let proposal = find_proposal(text).expect("a proposal");
    assert_eq!(proposal.day, Day::Tomorrow);
    assert_eq!(proposal.start_minute, 14 * 60 + 30);
    assert_eq!(span_text(text, &proposal), "tomorrow 14:30");
}

#[test]
fn twelve_am_is_midnight_and_twelve_pm_is_noon() {
    assert_eq!(find_proposal("12am").unwrap().start_minute, 0);
    assert_eq!(find_proposal("12pm").unwrap().start_minute, 12 * 60);
    assert_eq!(find_proposal("12:30 pm").unwrap().start_minute, 12 * 60 + 30);
}

#[test]
fn a_separate_am_or_pm_word_is_read_with_the_time() {
    let text = "Could we do 9:15 am on Monday?";
    let proposal = find_proposal(text).expect("a proposal");
    assert_eq!(proposal.start_minute, 9 * 60 + 15);
    assert_eq!(proposal.day, Day::Today, "the time comes first here, so the day is today");
    assert_eq!(span_text(text, &proposal), "9:15 am");
}

#[test]
fn a_duration_phrase_sets_the_length_of_the_event() {
    let text = "Tuesday at 10am for 45 minutes";
    let proposal = find_proposal(text).expect("a proposal");
    assert_eq!(proposal.day, Day::Weekday(1));
    assert_eq!(proposal.duration_minutes, 45);
    assert_eq!(span_text(text, &proposal), "Tuesday at 10am for 45 minutes");
    assert_eq!(find_proposal("Monday 1pm for an hour").unwrap().duration_minutes, 60);
    assert_eq!(find_proposal("Monday 1pm for 2 hours").unwrap().duration_minutes, 120);
}

#[test]
fn a_zero_or_oversized_duration_is_not_used() {
    let zero = "at 3pm for 0 minutes";
    let proposal = find_proposal(zero).expect("a proposal");
    assert_eq!(proposal.duration_minutes, DEFAULT_DURATION_MINUTES);
    assert_eq!(span_text(zero, &proposal), "at 3pm");
    let long = find_proposal("at 3pm for 30 hours").expect("a proposal");
    assert_eq!(long.duration_minutes, DEFAULT_DURATION_MINUTES);
}

#[test]
fn trailing_punctuation_is_not_part_of_the_span() {
    let text = "Friday at 3pm.";
    let proposal = find_proposal(text).expect("a proposal");
    assert_eq!(span_text(text, &proposal), "Friday at 3pm");
}

#[test]
fn text_without_a_clock_time_has_no_proposal() {
    assert_eq!(find_proposal("see you tomorrow"), None);
    assert_eq!(find_proposal("room 12 is free"), None, "a bare number is not a time");
    assert_eq!(find_proposal("meet at noon"), None, "named times are not read");
    assert_eq!(find_proposal(""), None);
}

#[test]
fn out_of_range_clock_times_are_refused() {
    assert_eq!(find_proposal("13pm"), None);
    assert_eq!(find_proposal("25:00"), None);
    assert_eq!(find_proposal("10:60"), None);
    assert_eq!(find_proposal("0pm"), None);
}

#[test]
fn days_count_forward_from_the_current_weekday() {
    let friday = find_proposal("Friday 9am").unwrap();
    assert_eq!(friday.days_from(4), 0, "a Friday proposal on a Friday is today");
    assert_eq!(friday.days_from(0), 4, "Monday to Friday is four days");
    let monday = find_proposal("Monday 9am").unwrap();
    assert_eq!(monday.days_from(6), 1, "Sunday to Monday is one day");
    assert_eq!(find_proposal("tomorrow 9am").unwrap().days_from(3), 1);
    assert_eq!(find_proposal("today 9am").unwrap().days_from(3), 0);
}

#[test]
fn the_first_proposal_in_the_text_wins() {
    let text = "Thursday 8am or Friday 9am";
    let proposal = find_proposal(text).expect("a proposal");
    assert_eq!(proposal.day, Day::Weekday(3));
    assert_eq!(proposal.start_minute, 8 * 60);
}
