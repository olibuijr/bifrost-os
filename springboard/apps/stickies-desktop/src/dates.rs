//! Dates for the note info lines in the Window menu: Unix seconds as "2026-10-13 14:05" in UTC.

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

/// "2026-10-13 14:05" (UTC); an empty string for a time of 0, which a note from before times were kept has.
pub fn format(secs: u64) -> String {
    if secs == 0 {
        return String::new();
    }
    let (y, mo, d) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!("{y:04}-{mo:02}-{d:02} {:02}:{:02}", rem / 3600, rem % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instants() {
        assert_eq!(format(1), "1970-01-01 00:00");
        assert_eq!(format(951_782_400 + 3600 * 13 + 60 * 7), "2000-02-29 13:07");
        assert_eq!(format(1_791_900_300), "2026-10-13 14:05");
    }

    #[test]
    fn a_zero_time_is_blank() {
        assert_eq!(format(0), "");
    }
}
