//! The "Found in Mail" banner of the reading pane: the events, flights, contacts and addresses the open message
//! mentions, offered as things to add or open. The reading is `bifrost_ui::found_in` over the text the reader shows; this
//! module turns an accepted suggestion into what the apps take: the times of a calendar entry, the link that opens an
//! address in Maps. Everything runs on this machine over text Mail already has.

use bifrost_app::controls::proposed_event::ProposedEvent;

/// What a tap in the banner means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoundHit {
    Add,
    Dismiss,
    Next,
    Previous,
}

/// The label of the Add link for a kind of suggestion, or `None` when Mail has nothing to add it to (a flight is shown,
/// and can be turned down).
pub fn add_label(kind: bifrost_app::controls::found_in::SuggestionKind) -> Option<&'static str> {
    use bifrost_app::controls::found_in::SuggestionKind as K;
    match kind {
        K::Event => Some("Add to Calendar"),
        K::Contact => Some("Add to Contacts"),
        K::Address => Some("Show in Maps"),
        K::Flight => None,
    }
}

/// The start and end of the calendar entry for a proposed event, as the Calendar service writes them
/// (`YYYY-MM-DDTHH:MM`, local time). `now` is the Unix time and `offset` the local zone's seconds east of GMT. A
/// weekday means its next occurrence, today included. The entry lasts the proposal's length, at least 15 minutes and at
/// most a day; one that runs past midnight ends on the next day.
pub fn event_times(event: &ProposedEvent, now: i64, offset: i64) -> (String, String) {
    let today = (now + offset).div_euclid(86_400);
    let weekday = (today + 3).rem_euclid(7) as u8; // Monday = 0
    let day = today + i64::from(event.days_from(weekday));
    let start = u32::from(event.start_minute);
    let length = u32::from(event.duration_minutes).clamp(15, 1440);
    let end = start + length;
    let write = |day: i64, minute: u32| {
        let (year, month, date) = crate::message::civil_from_days(day);
        format!("{year:04}-{month:02}-{date:02}T{:02}:{:02}", minute / 60, minute % 60)
    };
    (write(day, start), write(day + i64::from(end / 1440), end % 1440))
}

/// The `maps:` link that searches for `place` in the Maps app, with the text percent-encoded. Nothing leaves the
/// machine: the Maps app opens it.
pub fn maps_link(place: &str) -> String {
    let mut out = String::from("maps://?q=");
    for byte in place.trim().bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::found_in::SuggestionKind;
    use bifrost_app::controls::proposed_event::Day;

    fn event(day: Day, start_minute: u16, duration_minutes: u16) -> ProposedEvent {
        ProposedEvent { day, start_minute, duration_minutes, span: 0..0 }
    }

    // 2026-10-17 is a Saturday; 12:00 UTC
    const NOON: i64 = 1_792_238_400;

    #[test]
    fn a_proposed_event_becomes_local_times_on_the_right_day() {
        assert_eq!(event_times(&event(Day::Today, 13 * 60, 60), NOON, 0), ("2026-10-17T13:00".into(), "2026-10-17T14:00".into()));
        assert_eq!(event_times(&event(Day::Tomorrow, 9 * 60 + 30, 30), NOON, 0).0, "2026-10-18T09:30");
        // Monday (0) after Saturday is two days on
        assert_eq!(event_times(&event(Day::Weekday(0), 8 * 60, 60), NOON, 0).0, "2026-10-19T08:00");
        // the local zone moves the day: 23:00 UTC in a zone one hour east is already tomorrow
        assert_eq!(event_times(&event(Day::Today, 10 * 60, 60), NOON + 11 * 3600, 3600).0, "2026-10-18T10:00");
    }

    #[test]
    fn a_short_event_lasts_a_quarter_hour_and_one_past_midnight_ends_the_next_day() {
        assert_eq!(event_times(&event(Day::Today, 10 * 60, 0), NOON, 0).1, "2026-10-17T10:15");
        assert_eq!(event_times(&event(Day::Today, 23 * 60 + 30, 90), NOON, 0), ("2026-10-17T23:30".into(), "2026-10-18T01:00".into()));
    }

    #[test]
    fn an_address_becomes_a_maps_search_link_and_a_flight_has_nothing_to_add_to() {
        assert_eq!(maps_link(" Laugavegur 1, Reykjavik "), "maps://?q=Laugavegur%201%2C%20Reykjavik");
        assert_eq!(maps_link("Hallgrímskirkja"), "maps://?q=Hallgr%C3%ADmskirkja");
        assert_eq!(add_label(SuggestionKind::Event), Some("Add to Calendar"));
        assert_eq!(add_label(SuggestionKind::Flight), None);
    }
}
