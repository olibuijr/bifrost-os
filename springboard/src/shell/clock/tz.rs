//! Civil time without a zone database: proleptic Gregorian conversions and
//! fixed offsets plus the three daylight-saving rules the World Clock
//! cities use (EU, US, Australia, New Zealand, Chile).

/// Which daylight-saving rule a city follows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dst {
    None,
    /// last Sunday of March 01:00 UTC to last Sunday of October 01:00 UTC
    Eu,
    /// second Sunday of March 02:00 local to first Sunday of November 02:00 local
    Us,
    /// first Sunday of October 02:00 local to first Sunday of April 03:00 local (southern summer)
    Au,
    /// last Sunday of September 02:00 local to first Sunday of April 03:00 local (southern summer)
    Nz,
    /// first Sunday on or after 2 September 04:00 UTC to first Sunday on or after 2 April 03:00 UTC (southern summer)
    Cl,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct City {
    pub name: &'static str,
    /// standard-time offset from UTC in minutes
    pub std_min: i32,
    pub dst: Dst,
}

/// The World Clock cities, in display order.
pub const CITIES: &[City] = &[
    City { name: "Reykjavik", std_min: 0, dst: Dst::None },
    City { name: "London", std_min: 0, dst: Dst::Eu },
    City { name: "Paris", std_min: 60, dst: Dst::Eu },
    City { name: "Moscow", std_min: 180, dst: Dst::None },
    City { name: "Dubai", std_min: 240, dst: Dst::None },
    City { name: "New York", std_min: -300, dst: Dst::Us },
    City { name: "San Francisco", std_min: -480, dst: Dst::Us },
    City { name: "Tokyo", std_min: 540, dst: Dst::None },
    City { name: "Sydney", std_min: 600, dst: Dst::Au },
    // the cities below are searchable and can be added; only the first
    // `DEFAULT_CITIES` show on a fresh install
    City { name: "Lisbon", std_min: 0, dst: Dst::Eu },
    City { name: "Dublin", std_min: 0, dst: Dst::Eu },
    City { name: "Berlin", std_min: 60, dst: Dst::Eu },
    City { name: "Madrid", std_min: 60, dst: Dst::Eu },
    City { name: "Rome", std_min: 60, dst: Dst::Eu },
    City { name: "Amsterdam", std_min: 60, dst: Dst::Eu },
    City { name: "Brussels", std_min: 60, dst: Dst::Eu },
    City { name: "Vienna", std_min: 60, dst: Dst::Eu },
    City { name: "Zurich", std_min: 60, dst: Dst::Eu },
    City { name: "Stockholm", std_min: 60, dst: Dst::Eu },
    City { name: "Oslo", std_min: 60, dst: Dst::Eu },
    City { name: "Copenhagen", std_min: 60, dst: Dst::Eu },
    City { name: "Warsaw", std_min: 60, dst: Dst::Eu },
    City { name: "Prague", std_min: 60, dst: Dst::Eu },
    City { name: "Helsinki", std_min: 120, dst: Dst::Eu },
    City { name: "Athens", std_min: 120, dst: Dst::Eu },
    City { name: "Johannesburg", std_min: 120, dst: Dst::None },
    City { name: "Istanbul", std_min: 180, dst: Dst::None },
    City { name: "Nairobi", std_min: 180, dst: Dst::None },
    City { name: "Mumbai", std_min: 330, dst: Dst::None },
    City { name: "Bangkok", std_min: 420, dst: Dst::None },
    City { name: "Singapore", std_min: 480, dst: Dst::None },
    City { name: "Hong Kong", std_min: 480, dst: Dst::None },
    City { name: "Beijing", std_min: 480, dst: Dst::None },
    City { name: "Perth", std_min: 480, dst: Dst::None },
    City { name: "Seoul", std_min: 540, dst: Dst::None },
    City { name: "Honolulu", std_min: -600, dst: Dst::None },
    City { name: "Anchorage", std_min: -540, dst: Dst::Us },
    City { name: "Los Angeles", std_min: -480, dst: Dst::Us },
    City { name: "Denver", std_min: -420, dst: Dst::Us },
    City { name: "Chicago", std_min: -360, dst: Dst::Us },
    City { name: "Mexico City", std_min: -360, dst: Dst::None },
    City { name: "Toronto", std_min: -300, dst: Dst::Us },
    City { name: "Buenos Aires", std_min: -180, dst: Dst::None },
    City { name: "Sao Paulo", std_min: -180, dst: Dst::None },
    City { name: "Cairo", std_min: 120, dst: Dst::None },
    City { name: "Lagos", std_min: 60, dst: Dst::None },
    City { name: "Casablanca", std_min: 0, dst: Dst::None },
    City { name: "Cape Town", std_min: 120, dst: Dst::None },
    City { name: "Tehran", std_min: 210, dst: Dst::None },
    City { name: "Karachi", std_min: 300, dst: Dst::None },
    City { name: "Delhi", std_min: 330, dst: Dst::None },
    City { name: "Dhaka", std_min: 360, dst: Dst::None },
    City { name: "Jakarta", std_min: 420, dst: Dst::None },
    City { name: "Manila", std_min: 480, dst: Dst::None },
    City { name: "Taipei", std_min: 480, dst: Dst::None },
    City { name: "Auckland", std_min: 720, dst: Dst::Nz },
    City { name: "Melbourne", std_min: 600, dst: Dst::Au },
    City { name: "Brisbane", std_min: 600, dst: Dst::None },
    City { name: "Vancouver", std_min: -480, dst: Dst::Us },
    City { name: "Miami", std_min: -300, dst: Dst::Us },
    City { name: "Bogota", std_min: -300, dst: Dst::None },
    City { name: "Lima", std_min: -300, dst: Dst::None },
    City { name: "Santiago", std_min: -240, dst: Dst::Cl },
    City { name: "Kyiv", std_min: 120, dst: Dst::Eu },
    City { name: "Budapest", std_min: 60, dst: Dst::Eu },
    City { name: "Riga", std_min: 120, dst: Dst::Eu },
];

/// How many of `CITIES` a fresh install shows.
pub const DEFAULT_CITIES: usize = 9;

/// Whether a local hour is daytime (sun symbol) rather than night.
pub fn is_daytime(hour: u32) -> bool {
    (6..18).contains(&hour)
}

const DAY: i64 = 86_400;

pub use crate::bare::timezone::{civil_from_days, days_from_civil, days_in_month};
use crate::bare::timezone::weekday;

/// Broken-down local time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Civil {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
    /// 0 = Sunday
    pub wday: u32,
}

/// The current Unix time in seconds (0 if the clock is before 1970).
pub fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Breaks a Unix time shifted by `offset` seconds into civil fields.
pub fn civil(unix: i64, offset: i32) -> Civil {
    let local = unix + offset as i64;
    let days = local.div_euclid(DAY);
    let secs = local.rem_euclid(DAY) as u32;
    let (year, month, day) = civil_from_days(days);
    Civil { year, month, day, hour: secs / 3600, min: secs / 60 % 60, sec: secs % 60, wday: weekday(days) as u32 }
}

/// Day of month of the `n`th Sunday (1-based) of a month.
fn nth_sunday(year: i64, month: u32, n: u32) -> u32 {
    let first = days_from_civil(year, month, 1);
    let wday = weekday(first) as u32;
    1 + (7 - wday) % 7 + 7 * (n - 1)
}

/// Day of month of the first Sunday on or after `from`.
fn sunday_from(year: i64, month: u32, from: u32) -> u32 {
    let wday = (days_from_civil(year, month, from) + 4).rem_euclid(7) as u32;
    from + (7 - wday) % 7
}

fn last_sunday(year: i64, month: u32) -> u32 {
    let len = days_in_month(year, month);
    let last_wday = weekday(days_from_civil(year, month, len)) as u32;
    len - last_wday
}

/// Unix time of `hour`:00 on a civil day, with the day `offset` seconds ahead of UTC.
fn at(year: i64, month: u32, day: u32, hour: i64, offset: i64) -> i64 {
    days_from_civil(year, month, day) * DAY + hour * 3600 - offset
}

impl City {
    /// Whether daylight saving is in force at `unix`.
    pub fn dst_at(&self, unix: i64) -> bool {
        let year = civil(unix, self.std_min * 60).year;
        let std = self.std_min as i64 * 60;
        match self.dst {
            Dst::None => false,
            Dst::Eu => {
                let (start, end) = (at(year, 3, last_sunday(year, 3), 1, 0), at(year, 10, last_sunday(year, 10), 1, 0));
                unix >= start && unix < end
            }
            Dst::Us => {
                let start = at(year, 3, nth_sunday(year, 3, 2), 2, std);
                let end = at(year, 11, nth_sunday(year, 11, 1), 2, std + 3600);
                unix >= start && unix < end
            }
            Dst::Au => {
                let end = at(year, 4, nth_sunday(year, 4, 1), 3, std + 3600);
                let start = at(year, 10, nth_sunday(year, 10, 1), 2, std);
                unix < end || unix >= start
            }
            Dst::Nz => {
                let end = at(year, 4, nth_sunday(year, 4, 1), 3, std + 3600);
                let start = at(year, 9, last_sunday(year, 9), 2, std);
                unix < end || unix >= start
            }
            Dst::Cl => {
                let end = at(year, 4, sunday_from(year, 4, 2), 3, 0);
                let start = at(year, 9, sunday_from(year, 9, 2), 4, 0);
                unix < end || unix >= start
            }
        }
    }

    /// Offset from UTC in seconds at `unix`.
    pub fn offset_at(&self, unix: i64) -> i32 {
        self.std_min * 60 + if self.dst_at(unix) { 3600 } else { 0 }
    }

    pub fn local(&self, unix: i64) -> Civil {
        civil(unix, self.offset_at(unix))
    }
}

/// "Today"/"Tomorrow"/"Yesterday" and the hour difference of a city
/// relative to the device's own zone, e.g. ("Tomorrow", 9).
pub fn relative(city: &City, unix: i64, home_offset: i32) -> (&'static str, i32) {
    let (home, there) = (civil(unix, home_offset), city.local(unix));
    let day = (days_from_civil(there.year, there.month, there.day) - days_from_civil(home.year, home.month, home.day)).signum();
    let word = match day {
        1 => "Tomorrow",
        -1 => "Yesterday",
        _ => "Today",
    };
    (word, (city.offset_at(unix) - home_offset) / 3600)
}

/// "+9 HRS", "-1 HR", "+0 HRS" as the bifrost prints it.
pub fn hours_text(diff: i32) -> String {
    format!("{}{} {}", if diff < 0 { "-" } else { "+" }, diff.abs(), if diff.abs() == 1 { "HR" } else { "HRS" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auckland_follows_the_new_zealand_rule_not_the_australian_one() {
        let nz = city("Auckland");
        // zdump: NZDT ends Sat 4 Apr 2026 14:00 UTC, resumes Sat 26 Sep 2026 14:00 UTC
        let (end, start) = (days_from_civil(2026, 4, 4) * DAY + 14 * 3600, days_from_civil(2026, 9, 26) * DAY + 14 * 3600);
        assert!(nz.dst_at(end - 1) && !nz.dst_at(end));
        assert!(!nz.dst_at(start - 1) && nz.dst_at(start));
        // the Australian start (first Sunday of October) is already a week in
        assert_eq!(nz.offset_at(days_from_civil(2026, 10, 1) * DAY), 13 * 3600);
        assert_eq!(nz.offset_at(days_from_civil(2026, 7, 1) * DAY), 12 * 3600);
    }

    #[test]
    fn santiago_shifts_on_the_chilean_dates() {
        let cl = city("Santiago");
        // zdump: -03 until Sun 5 Apr 2026 03:00 UTC, from Sun 6 Sep 2026 04:00 UTC
        let (end, start) = (days_from_civil(2026, 4, 5) * DAY + 3 * 3600, days_from_civil(2026, 9, 6) * DAY + 4 * 3600);
        assert!(cl.dst_at(end - 1) && !cl.dst_at(end));
        assert!(!cl.dst_at(start - 1) && cl.dst_at(start));
        assert_eq!(cl.offset_at(days_from_civil(2026, 1, 15) * DAY), -3 * 3600);
        assert_eq!(cl.offset_at(days_from_civil(2026, 7, 15) * DAY), -4 * 3600);
    }

    #[test]
    fn tehran_keeps_a_fixed_offset_since_iran_dropped_daylight_saving() {
        assert_eq!(city("Tehran").dst, Dst::None);
    }

    #[test]
    fn extra_cities_are_unique_and_day_night_flips_at_six_and_eighteen() {
        for (i, c) in CITIES.iter().enumerate() {
            assert_eq!(CITIES.iter().position(|o| o.name == c.name), Some(i), "{} is listed twice", c.name);
        }
        assert!(CITIES.len() >= 50, "the built-in table has 50+ cities");
        assert!(CITIES.len() > DEFAULT_CITIES);
        assert!(!is_daytime(5) && is_daytime(6) && is_daytime(17) && !is_daytime(18));
    }

    fn utc(y: i64, m: u32, d: u32, h: i64, min: i64) -> i64 {
        days_from_civil(y, m, d) * DAY + h * 3600 + min * 60
    }

    fn city(name: &str) -> &'static City {
        CITIES.iter().find(|c| c.name == name).unwrap()
    }

    #[test]
    fn civil_round_trips_and_knows_weekdays() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        for days in [-1_000_000, -1, 0, 1, 19_000, 60_000, 2_000_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        let c = civil(utc(2026, 10, 3, 23, 32), 0);
        assert_eq!((c.year, c.month, c.day, c.hour, c.min, c.wday), (2026, 10, 3, 23, 32, 6), "Saturday");
        let c = civil(utc(2026, 10, 3, 23, 32), -7 * 3600);
        assert_eq!((c.day, c.hour), (3, 16));
        let c = civil(utc(2026, 10, 3, 23, 32), 3600);
        assert_eq!((c.day, c.hour, c.wday), (4, 0, 0), "crosses midnight into Sunday");
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29), "leap day");
    }

    #[test]
    fn sunday_lookups() {
        assert_eq!(nth_sunday(2026, 3, 2), 8);
        assert_eq!(last_sunday(2026, 3), 29);
        assert_eq!(last_sunday(2026, 10), 25);
        assert_eq!(nth_sunday(2026, 11, 1), 1);
        assert_eq!(nth_sunday(2026, 10, 1), 4);
        assert_eq!(nth_sunday(2026, 4, 1), 5);
        assert_eq!(last_sunday(2025, 12), 28);
    }

    #[test]
    fn eu_rule_switches_at_one_utc() {
        let london = city("London");
        assert!(!london.dst_at(utc(2026, 3, 29, 0, 59)));
        assert!(london.dst_at(utc(2026, 3, 29, 1, 0)));
        assert!(london.dst_at(utc(2026, 10, 25, 0, 59)));
        assert!(!london.dst_at(utc(2026, 10, 25, 1, 0)));
        assert_eq!(london.offset_at(utc(2026, 10, 3, 23, 32)), 3600, "BST on the capture date");
        assert_eq!(city("Paris").offset_at(utc(2026, 10, 3, 23, 32)), 7200);
        assert_eq!(city("Reykjavik").offset_at(utc(2026, 7, 1, 0, 0)), 0, "Iceland never shifts");
    }

    #[test]
    fn us_rule_switches_at_two_local() {
        let ny = city("New York");
        // 2026-03-08 02:00 EST = 07:00 UTC
        assert!(!ny.dst_at(utc(2026, 3, 8, 6, 59)));
        assert!(ny.dst_at(utc(2026, 3, 8, 7, 0)));
        // 2026-11-01 02:00 EDT = 06:00 UTC
        assert!(ny.dst_at(utc(2026, 11, 1, 5, 59)));
        assert!(!ny.dst_at(utc(2026, 11, 1, 6, 0)));
        assert_eq!(ny.offset_at(utc(2026, 10, 3, 23, 32)), -4 * 3600);
        assert_eq!(city("San Francisco").offset_at(utc(2026, 10, 3, 23, 32)), -7 * 3600);
        assert_eq!(city("San Francisco").offset_at(utc(2026, 12, 25, 0, 0)), -8 * 3600);
    }

    #[test]
    fn australian_summer_spans_the_new_year() {
        let sydney = city("Sydney");
        assert_eq!(sydney.offset_at(utc(2026, 1, 15, 0, 0)), 11 * 3600);
        assert_eq!(sydney.offset_at(utc(2026, 7, 1, 0, 0)), 10 * 3600);
        // 2026-10-04 02:00 AEST = 2026-10-03 16:00 UTC
        assert!(!sydney.dst_at(utc(2026, 10, 3, 15, 59)));
        assert!(sydney.dst_at(utc(2026, 10, 3, 16, 0)));
        // 2026-04-05 03:00 AEDT = 2026-04-04 16:00 UTC
        assert!(sydney.dst_at(utc(2026, 4, 4, 15, 59)));
        assert!(!sydney.dst_at(utc(2026, 4, 4, 16, 0)));
    }

    #[test]
    fn matches_the_bifrost_capture() {
        // Reykjavik 23:32 Sat 3 Oct 2026
        let now = utc(2026, 10, 3, 23, 32);
        let times: Vec<(String, &str, i32)> = CITIES
            .iter()
            .map(|c| {
                let l = c.local(now);
                let (word, diff) = relative(c, now, 0);
                (format!("{:02}:{:02}", l.hour, l.min), word, diff)
            })
            .collect();
        assert_eq!(times[0], ("23:32".into(), "Today", 0));
        assert_eq!(times[1], ("00:32".into(), "Tomorrow", 1));
        assert_eq!(times[2], ("01:32".into(), "Tomorrow", 2));
        assert_eq!(times[3], ("02:32".into(), "Tomorrow", 3));
        assert_eq!(times[5], ("19:32".into(), "Today", -4));
        assert_eq!(times[6], ("16:32".into(), "Today", -7));
        assert_eq!(times[7], ("08:32".into(), "Tomorrow", 9));
        assert_eq!(times[8], ("10:32".into(), "Tomorrow", 11), "Sydney already on summer time");
        assert_eq!(relative(city("San Francisco"), utc(2026, 10, 4, 5, 0), 0).0, "Yesterday");
        assert_eq!(hours_text(1), "+1 HR");
        assert_eq!(hours_text(-7), "-7 HRS");
        assert_eq!(hours_text(0), "+0 HRS");
    }
}
