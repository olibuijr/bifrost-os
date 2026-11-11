//! Keychain Access logic that needs no window: the order of the items, the group titles and the time of the last
//! change. Tested alone.

use bifrost_app::keychain_groups::Row;

/// Items in order by service, then account.
pub fn sort(rows: &mut [Row]) {
    rows.sort_by(|a, b| a.service.cmp(&b.service).then_with(|| a.account.cmp(&b.account)));
}

/// The title of a group: its last dotted part with the first letter upper case (`is.bifrost.passwords` is
/// `Passwords`).
pub fn title(group: &str) -> String {
    let last = group.rsplit('.').next().unwrap_or(group);
    let mut chars = last.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => group.to_string(),
    }
}

/// When an item was last changed, at `now` (Unix seconds), as the Files app writes a date: "Today at 09:30",
/// "Yesterday at 18:02" or "14 Nov 2023 at 22:13". The time is told in the system zone and the person's clock style,
/// both as the shell tells the app (`bifrost_app::time_zone`) and read when called, so the list follows a change of
/// Settings > Date & Time: "Today at 9:30 AM" with 24-Hour Time off.
pub fn stamp(unix: u64, now: i64) -> String {
    // each instant at the zone's offset then: an item changed before a daylight saving change keeps its time
    let offset = |t: i64| bifrost_app::time_zone::TimeZone::auto_updating_current().seconds_from_gmt(bifrost_app::coding::Date(t as f64));
    let at = i64::try_from(unix).unwrap_or(i64::MAX / 2);
    stamp_in(at, offset(at), now, offset(now), bifrost_app::time_zone::uses_24_hour_clock())
}

/// `stamp` with the zone's offset from UTC in seconds at each of the two instants, and a given clock style. The text
/// is the shell's (`springboard::files::fs::date_text_in`), which reads both instants at one offset: the item's time
/// is moved by the difference of the two, so its day and time are the ones of its own offset.
pub fn stamp_in(unix: i64, offset: i32, now: i64, now_offset: i32, h24: bool) -> String {
    springboard::files::fs::date_text_in(unix + i64::from(offset) - i64::from(now_offset), now, now_offset, h24)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(service: &str, account: &str) -> Row {
        Row { service: service.into(), account: account.into(), modified: 0 }
    }

    #[test]
    fn items_sort_by_service_then_account() {
        let mut rows = vec![row("b", "2"), row("a", "z"), row("b", "1")];
        sort(&mut rows);
        let order: Vec<(&str, &str)> = rows.iter().map(|r| (r.service.as_str(), r.account.as_str())).collect();
        assert_eq!(order, vec![("a", "z"), ("b", "1"), ("b", "2")]);
    }

    #[test]
    fn group_titles_come_from_the_last_part() {
        assert_eq!(title("is.bifrost.passwords"), "Passwords");
        assert_eq!(title("is.bifrost.account"), "Account");
        assert_eq!(title("plain"), "Plain");
        assert_eq!(title(""), "");
    }

    #[test]
    fn stamps_are_dates_with_the_time_in_the_clock_style() {
        // 2023-11-14 22:13:20 UTC and the leap day of 2000, read long after
        let now = 1_791_849_600_i64;
        assert_eq!(stamp_in(1_700_000_000, 0, now, 0, true), "14 Nov 2023 at 22:13");
        assert_eq!(stamp_in(1_700_000_000, 0, now, 0, false), "14 Nov 2023 at 10:13 PM");
        assert_eq!(stamp_in(951_782_400, 0, now, 0, true), "29 Feb 2000 at 00:00");
        assert_eq!(stamp_in(0, 0, now, 0, true), "1 Jan 1970 at 00:00");
        // 2026-10-13 00:00:00 UTC, then a morning, noon, an afternoon and midnight of that day
        let at = |h: i64, m: i64| now + h * 3600 + m * 60;
        let late = at(23, 59);
        let today = |h24: bool| [at(8, 5), at(12, 0), at(15, 58), at(0, 0)].map(|t| stamp_in(t, 0, late, 0, h24));
        assert_eq!(today(true), ["Today at 08:05", "Today at 12:00", "Today at 15:58", "Today at 00:00"]);
        assert_eq!(today(false), ["Today at 8:05 AM", "Today at 12:00 PM", "Today at 3:58 PM", "Today at 12:00 AM"]);
        assert_eq!(stamp_in(at(15, 58) - 86_400, 0, late, 0, false), "Yesterday at 3:58 PM");
        // each instant at its own offset: changed at 23:30 UTC+2, read after the clocks went back to UTC+1
        assert_eq!(stamp_in(at(21, 30) - 86_400, 7200, late, 3600, true), "12 Oct 2026 at 23:30");
        assert_eq!(stamp_in(at(21, 30), 7200, late, 3600, true), "Yesterday at 23:30");
        // no shell speaks in a test: 24-hour time, in the zone of the test
        assert!(stamp(1_700_000_000, now).contains(" Nov 2023 at "));
    }
}
