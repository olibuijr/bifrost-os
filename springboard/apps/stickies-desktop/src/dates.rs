//! Dates for the note info lines in the Window menu: Unix seconds as "2026-10-13 14:05" in the system time zone. The
//! time of day is written by `time_zone::time_text`, so it follows Settings > Date & Time > 24-Hour Time ("14:05", or
//! "2:05 PM" with it off) as the menu bar does. The lines are made again each time the menus are asked for, which the
//! runtime does when the clock style or the zone changes, so nothing here is cached.

use bifrost_app::coding::Date;
use bifrost_app::time_zone::{self, TimeZone};

/// Days since 1970-01-01 to (year, month, day), in the proleptic Gregorian calendar.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month_i = if mp < 10 { mp + 3 } else { mp - 9 };
    let month = month_i as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// "2026-10-13 14:05" in the system zone and the person's clock style; an empty string for a time of 0, which a note
/// from before times were kept has.
pub fn format(secs: u64) -> String {
    if secs == 0 {
        return String::new();
    }
    let offset = TimeZone::current().seconds_from_gmt(Date(secs as f64));
    format_at(secs, offset, time_zone::time_text)
}

/// `format` in a zone `offset` seconds east of GMT, the time of day (hour, minute) written by `time_text`.
fn format_at(secs: u64, offset: i32, time_text: impl Fn(i32, i32) -> String) -> String {
    let local = secs as i64 + i64::from(offset);
    let (y, mo, d) = civil_from_days(local.div_euclid(86_400));
    let rem = local.rem_euclid(86_400);
    format!("{y:04}-{mo:02}-{d:02} {}", time_text((rem / 3600) as i32, (rem % 3600 / 60) as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(hour: i32, minute: i32) -> String {
        format!("{hour:02}:{minute:02}")
    }

    #[test]
    fn formats_known_instants() {
        assert_eq!(format_at(1, 0, plain), "1970-01-01 00:00");
        assert_eq!(format_at(951_782_400 + 3600 * 13 + 60 * 7, 0, plain), "2000-02-29 13:07");
        assert_eq!(format_at(1_791_900_300, 0, plain), "2026-10-13 14:05");
    }

    #[test]
    fn the_date_and_the_time_are_those_of_the_zone() {
        assert_eq!(format_at(1_791_900_300, 3600, plain), "2026-10-13 15:05");
        assert_eq!(format_at(1_791_900_300, -15 * 3600, plain), "2026-10-12 23:05", "west of GMT it is still the day before");
        assert_eq!(format_at(1, -3600, plain), "1969-12-31 23:00", "also before 1970");
    }

    /// The time of day is the kit's (`time_zone::time_text`): 24-hour where no shell has said otherwise, as in a test,
    /// and whatever style it writes is what the line carries.
    #[test]
    fn the_time_of_day_is_written_by_the_clock_style() {
        assert_eq!(format_at(1_791_900_300, 0, time_zone::time_text), format!("2026-10-13 {}", time_zone::time_text(14, 5)));
        let twelve = |hour: i32, minute: i32| format!("{}:{minute:02} {}", (hour + 11) % 12 + 1, if hour < 12 { "AM" } else { "PM" });
        assert_eq!(format_at(1_791_900_300, 0, twelve), "2026-10-13 2:05 PM");
    }

    #[test]
    fn a_zero_time_is_blank() {
        assert_eq!(format(0), "");
    }
}
