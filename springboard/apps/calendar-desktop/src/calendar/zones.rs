//! Time zones with daylight saving. An event names a zone by its IANA name; this table holds what Calendar needs to
//! turn the event's wall time into a moment (alerts) and into another zone (the converted time in the editor): the
//! standard offset and the rule for the summer shift. Only the rules in force today are kept (the European Union and
//! North American rules), so a date before those rules began is read with them too. Zones whose summer shift runs
//! across the new year (the southern hemisphere) are not listed.
use super::Date;

/// When a zone moves its clocks one hour forward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dst {
    /// The zone keeps its standard offset all year.
    Never,
    /// From the last Sunday of March to the last Sunday of October, both at 01:00 UTC.
    Eu,
    /// From the second Sunday of March to the first Sunday of November, both at 02:00 local time.
    Us,
}

/// One zone the editor offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zone {
    pub id: &'static str,
    /// The name the editor shows.
    pub title: &'static str,
    /// Minutes east of UTC outside daylight saving.
    pub standard: i32,
    pub dst: Dst,
}

/// The zones, in the order the editor's Time Zone row steps through them after "Device Time Zone".
pub const TABLE: [Zone; 11] = [
    Zone { id: "UTC", title: "UTC", standard: 0, dst: Dst::Never },
    Zone { id: "Atlantic/Reykjavik", title: "Iceland (GMT)", standard: 0, dst: Dst::Never },
    Zone { id: "Europe/London", title: "London", standard: 0, dst: Dst::Eu },
    Zone { id: "Europe/Paris", title: "Paris", standard: 60, dst: Dst::Eu },
    Zone { id: "Europe/Helsinki", title: "Helsinki", standard: 120, dst: Dst::Eu },
    Zone { id: "America/New_York", title: "New York", standard: -300, dst: Dst::Us },
    Zone { id: "America/Chicago", title: "Chicago", standard: -360, dst: Dst::Us },
    Zone { id: "America/Denver", title: "Denver", standard: -420, dst: Dst::Us },
    Zone { id: "America/Los_Angeles", title: "Los Angeles", standard: -480, dst: Dst::Us },
    Zone { id: "Asia/Kolkata", title: "India", standard: 330, dst: Dst::Never },
    Zone { id: "Asia/Tokyo", title: "Tokyo", standard: 540, dst: Dst::Never },
];

/// The zone named `id`, if it is in the table.
pub fn find(id: &str) -> Option<&'static Zone> {
    TABLE.iter().find(|z| z.id == id)
}

/// The last Sunday of `month`.
fn last_sunday(year: i64, month: u32) -> Date {
    let end = Date { year, month, day: 1 };
    let end = Date { year, month, day: end.month_len() };
    end.shift_days(-(end.weekday() as i32))
}

/// The `n`th Sunday (1 is the first) of `month`.
fn nth_sunday(year: i64, month: u32, n: u32) -> Date {
    let first = Date { year, month, day: 1 };
    let lead = (7 - first.weekday() as u32) % 7;
    Date { year, month, day: 1 + lead + 7 * (n - 1) }
}

/// The summer shift of `zone` in `year` as minutes since the epoch, UTC: it starts at the first and ends at the second
/// (the clock is back on standard time from then). None for a zone with no shift.
pub fn shift_span(zone: &Zone, year: i64) -> Option<(i64, i64)> {
    let std = zone.standard as i64;
    match zone.dst {
        Dst::Never => None,
        Dst::Eu => Some((last_sunday(year, 3).days() * 1440 + 60, last_sunday(year, 10).days() * 1440 + 60)),
        Dst::Us => Some((
            nth_sunday(year, 3, 2).days() * 1440 + 120 - std,
            nth_sunday(year, 11, 1).days() * 1440 + 120 - (std + 60),
        )),
    }
}

/// Minutes east of UTC that the wall time `minute` of `date` has in `zone`. A time the spring shift skips counts as
/// summer time; a time the autumn shift repeats counts as its first occurrence.
pub fn offset_at(zone: &Zone, date: Date, minute: u32) -> i32 {
    let Some((start, end)) = shift_span(zone, date.year) else { return zone.standard };
    let local = date.days() * 1440 + minute as i64;
    let std = zone.standard as i64;
    if local - std >= start && local - (std + 60) < end {
        zone.standard + 60
    } else {
        zone.standard
    }
}

/// Minutes east of UTC that `zone` has at the moment `unix` (seconds).
pub fn offset_at_utc(zone: &Zone, unix: i64) -> i32 {
    let minutes = unix.div_euclid(60);
    let year = Date::from_days(minutes.div_euclid(1440)).year;
    match shift_span(zone, year) {
        Some((start, end)) if minutes >= start && minutes < end => zone.standard + 60,
        _ => zone.standard,
    }
}

/// Whether `zone` is on summer time at `unix`.
pub fn is_shifted(zone: &Zone, unix: i64) -> bool {
    offset_at_utc(zone, unix) != zone.standard
}

/// The moment (unix seconds) the wall time `minute` of `date` is in `zone`.
pub fn unix_of(zone: &Zone, date: Date, minute: u32) -> i64 {
    (date.days() * 1440 + minute as i64 - offset_at(zone, date, minute) as i64) * 60
}

/// The wall date and minute of the moment `unix` in `zone`.
pub fn local_in(zone: &Zone, unix: i64) -> (Date, u32) {
    let minutes = unix.div_euclid(60) + offset_at_utc(zone, unix) as i64;
    (Date::from_days(minutes.div_euclid(1440)), minutes.rem_euclid(1440) as u32)
}

/// Where a wall time in `from` (None: the device, whose offset is `device_offset` minutes) lands in the zone `to`.
/// None when `to` is not in the table.
pub fn convert(from: Option<&str>, date: Date, minute: u32, device_offset: i32, to: &str) -> Option<(Date, u32)> {
    let to = find(to)?;
    let unix = match from.map(|f| (f, find(f))) {
        None => (date.days() * 1440 + minute as i64 - device_offset as i64) * 60,
        Some((_, Some(zone))) => unix_of(zone, date, minute),
        Some((_, None)) => return None,
    };
    Some(local_in(to, unix))
}

/// "+1:00", "-5:30", "+0:00": a zone offset as the editor shows it.
pub fn offset_title(minutes: i32) -> String {
    let a = minutes.abs();
    format!("{}{}:{:02}", if minutes < 0 { '-' } else { '+' }, a / 60, a % 60)
}

/// The VTIMEZONE sub-blocks of `zone` as .ics lines (no BEGIN/END VTIMEZONE and no TZID): one STANDARD block for a zone
/// with a fixed offset, and a STANDARD and a DAYLIGHT block with yearly rules for one with a summer shift.
pub fn vtimezone_body(zone: &Zone) -> Vec<String> {
    let off = |m: i32| {
        let a = m.abs();
        format!("{}{:02}{:02}", if m < 0 { '-' } else { '+' }, a / 60, a % 60)
    };
    let (std, sum) = (zone.standard, zone.standard + 60);
    let block = |kind: &str, from: i32, to: i32, start: &str, rule: &str| {
        vec![
            format!("BEGIN:{kind}"),
            format!("DTSTART:{start}"),
            format!("RRULE:{rule}"),
            format!("TZOFFSETFROM:{}", off(from)),
            format!("TZOFFSETTO:{}", off(to)),
            format!("END:{kind}"),
        ]
    };
    match zone.dst {
        Dst::Never => vec![
            "BEGIN:STANDARD".into(),
            "DTSTART:19700101T000000".into(),
            format!("TZOFFSETFROM:{}", off(std)),
            format!("TZOFFSETTO:{}", off(std)),
            "END:STANDARD".into(),
        ],
        // the EU shift is at 01:00 UTC: local 01:00 + standard offset going forward, 01:00 + summer offset going back
        Dst::Eu => {
            let wall = |m: i32| format!("19700329T{:02}{:02}00", (60 + m).div_euclid(60) % 24, (60 + m).rem_euclid(60));
            let back = |m: i32| format!("19701025T{:02}{:02}00", (60 + m).div_euclid(60) % 24, (60 + m).rem_euclid(60));
            let mut out = block("DAYLIGHT", std, sum, &wall(std), "FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU");
            out.extend(block("STANDARD", sum, std, &back(sum), "FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU"));
            out
        }
        Dst::Us => {
            let mut out = block("DAYLIGHT", std, sum, "19700308T020000", "FREQ=YEARLY;BYMONTH=3;BYDAY=2SU");
            out.extend(block("STANDARD", sum, std, "19701101T020000", "FREQ=YEARLY;BYMONTH=11;BYDAY=1SU"));
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(year: i64, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }
    fn zone(id: &str) -> &'static Zone {
        find(id).expect("zone in the table")
    }

    #[test]
    fn the_shift_sundays_are_right() {
        // 2026: March 29 and October 25 are the last Sundays; March 8 and November 1 the second and first
        assert_eq!(last_sunday(2026, 3), d(2026, 3, 29));
        assert_eq!(last_sunday(2026, 10), d(2026, 10, 25));
        assert_eq!(nth_sunday(2026, 3, 2), d(2026, 3, 8));
        assert_eq!(nth_sunday(2026, 11, 1), d(2026, 11, 1));
        // a month that starts on a Sunday
        assert_eq!(nth_sunday(2026, 2, 1), d(2026, 2, 1));
        assert_eq!(last_sunday(2024, 3), d(2024, 3, 31));
    }

    #[test]
    fn europe_is_an_hour_ahead_in_summer_only() {
        let paris = zone("Europe/Paris");
        assert_eq!(offset_at(paris, d(2026, 1, 15), 600), 60);
        assert_eq!(offset_at(paris, d(2026, 7, 15), 600), 120);
        // the shift day itself: the clocks jump at 02:00 local (01:00 UTC)
        assert_eq!(offset_at(paris, d(2026, 3, 29), 119), 60);
        assert_eq!(offset_at(paris, d(2026, 3, 29), 120), 120);
        assert_eq!(offset_at(paris, d(2026, 3, 29), 150), 120, "the skipped half hour counts as summer time");
        assert_eq!(offset_at(paris, d(2026, 10, 25), 179), 120, "the repeated hour counts as its first pass");
        assert_eq!(offset_at(paris, d(2026, 10, 25), 180), 60);
        assert_eq!(offset_at(zone("Europe/London"), d(2026, 6, 1), 0), 60);
    }

    #[test]
    fn north_america_shifts_on_other_days_than_europe() {
        let ny = zone("America/New_York");
        assert_eq!(offset_at(ny, d(2026, 1, 15), 600), -300);
        assert_eq!(offset_at(ny, d(2026, 7, 15), 600), -240);
        // between the two rules' dates (March 8 to March 29, 2026) only North America is on summer time
        assert_eq!(offset_at(ny, d(2026, 3, 15), 600), -240);
        assert_eq!(offset_at(zone("Europe/Paris"), d(2026, 3, 15), 600), 60);
        assert_eq!(offset_at(ny, d(2026, 3, 8), 119), -300);
        assert_eq!(offset_at(ny, d(2026, 3, 8), 180), -240);
        assert_eq!(offset_at(ny, d(2026, 11, 1), 59), -240);
        assert_eq!(offset_at(ny, d(2026, 11, 1), 120), -300);
    }

    #[test]
    fn zones_without_a_shift_keep_their_offset() {
        for id in ["UTC", "Atlantic/Reykjavik", "Asia/Tokyo", "Asia/Kolkata"] {
            let z = zone(id);
            assert_eq!(offset_at(z, d(2026, 1, 1), 0), z.standard);
            assert_eq!(offset_at(z, d(2026, 7, 1), 720), z.standard);
        }
        assert_eq!(zone("Asia/Kolkata").standard, 330);
    }

    #[test]
    fn a_wall_time_and_its_moment_agree_both_ways() {
        let ny = zone("America/New_York");
        for (date, minute) in [(d(2026, 1, 10), 0), (d(2026, 7, 4), 1439), (d(2026, 10, 31), 720), (d(2026, 12, 31), 1380)] {
            let unix = unix_of(ny, date, minute);
            assert_eq!(local_in(ny, unix), (date, minute));
        }
        // 2026-07-04 12:00 in New York is 16:00 UTC
        assert_eq!(unix_of(ny, d(2026, 7, 4), 720), (d(2026, 7, 4).days() * 1440 + 960) * 60);
        assert!(is_shifted(ny, unix_of(ny, d(2026, 7, 4), 720)));
        assert!(!is_shifted(ny, unix_of(ny, d(2026, 12, 4), 720)));
    }

    #[test]
    fn convert_moves_a_time_between_zones_across_midnight() {
        // 23:30 in Tokyo is 14:30 UTC, 15:30 in Paris (winter) and 09:30 in New York (winter)
        let day = d(2026, 1, 20);
        assert_eq!(convert(Some("Asia/Tokyo"), day, 1410, 0, "UTC"), Some((day, 870)));
        assert_eq!(convert(Some("Asia/Tokyo"), day, 1410, 0, "Europe/Paris"), Some((day, 930)));
        assert_eq!(convert(Some("Asia/Tokyo"), day, 1410, 0, "America/New_York"), Some((day, 570)));
        // 01:00 in Tokyo is the day before in New York
        assert_eq!(convert(Some("Asia/Tokyo"), day, 60, 0, "America/New_York"), Some((d(2026, 1, 19), 660)));
        // the device (UTC+2 now) as the source
        assert_eq!(convert(None, day, 60, 120, "UTC"), Some((d(2026, 1, 19), 1380)));
        assert_eq!(convert(Some("Mars/Olympus"), day, 0, 0, "UTC"), None);
        assert_eq!(convert(None, day, 0, 0, "Mars/Olympus"), None);
    }

    #[test]
    fn the_same_wall_time_is_a_different_moment_in_the_two_halves_of_the_year() {
        let paris = zone("Europe/Paris");
        let winter = unix_of(paris, d(2026, 2, 1), 600);
        let summer = unix_of(paris, d(2026, 8, 1), 600);
        assert_eq!(winter, (d(2026, 2, 1).days() * 1440 + 540) * 60);
        assert_eq!(summer, (d(2026, 8, 1).days() * 1440 + 480) * 60);
    }

    #[test]
    fn offsets_read_the_way_the_editor_shows_them() {
        assert_eq!(offset_title(0), "+0:00");
        assert_eq!(offset_title(330), "+5:30");
        assert_eq!(offset_title(-300), "-5:00");
    }

    #[test]
    fn a_fixed_zone_writes_one_block_and_a_shifting_zone_writes_two() {
        let fixed = vtimezone_body(zone("Asia/Tokyo"));
        assert_eq!(fixed.iter().filter(|l| l.starts_with("BEGIN:")).count(), 1);
        assert!(fixed.contains(&"TZOFFSETTO:+0900".to_string()));
        let eu = vtimezone_body(zone("Europe/Paris"));
        assert!(eu.contains(&"BEGIN:DAYLIGHT".to_string()) && eu.contains(&"BEGIN:STANDARD".to_string()));
        assert!(eu.contains(&"RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU".to_string()));
        // Paris moves at 02:00 local standard time going forward and 03:00 summer time going back
        assert!(eu.contains(&"DTSTART:19700329T020000".to_string()));
        assert!(eu.contains(&"DTSTART:19701025T030000".to_string()));
        let us = vtimezone_body(zone("America/Chicago"));
        assert!(us.contains(&"TZOFFSETFROM:-0600".to_string()) && us.contains(&"TZOFFSETTO:-0500".to_string()));
        assert!(us.contains(&"RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU".to_string()));
        // London: 01:00 UTC is 01:00 local going forward and 02:00 summer time going back
        let uk = vtimezone_body(zone("Europe/London"));
        assert!(uk.contains(&"DTSTART:19700329T010000".to_string()));
        assert!(uk.contains(&"DTSTART:19701025T020000".to_string()));
    }

    #[test]
    fn every_zone_in_the_table_has_a_unique_id() {
        for (i, z) in TABLE.iter().enumerate() {
            assert_eq!(TABLE.iter().position(|o| o.id == z.id), Some(i));
            assert!(find(z.id).is_some());
        }
        assert!(find("Zone/X").is_none());
    }
}
