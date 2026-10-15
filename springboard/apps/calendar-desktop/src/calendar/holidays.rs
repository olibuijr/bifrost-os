//! The bundled holiday calendar: Iceland's public holidays and observances, worked out from the year alone (Easter by
//! the Gregorian computus, the rest by fixed dates or by weekday rules). Shown as a read-only calendar beside the
//! saved events.
use super::Date;

/// Easter Sunday of `year` (Meeus, Jones and Butcher's algorithm for the Gregorian calendar).
pub fn easter(year: i64) -> Date {
    let (a, b, c) = (year % 19, year / 100, year % 100);
    let (d, e) = (b / 4, b % 4);
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let (i, k) = (c / 4, c % 4);
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    Date { year, month: month as u32, day: day as u32 }
}

/// The first `weekday` (0 = Sunday) on or after `from`.
fn on_or_after(from: Date, weekday: usize) -> Date {
    from.shift_days(((weekday + 7 - from.weekday()) % 7) as i32)
}

/// Iceland's holidays in `year` in date order: the public holidays and the days that close the shops early or are
/// kept as observances (First Day of Summer, Christmas Eve and New Year's Eve).
pub fn iceland(year: i64) -> Vec<(Date, &'static str)> {
    let on = |month: u32, day: u32| Date { year, month, day };
    let easter = easter(year);
    let after = |days: i32| easter.shift_days(days);
    let mut days = vec![
        (on(1, 1), "New Year's Day"),
        (after(-3), "Maundy Thursday"),
        (after(-2), "Good Friday"),
        (easter, "Easter Sunday"),
        (after(1), "Easter Monday"),
        (on_or_after(on(4, 19), 4), "First Day of Summer"),
        (on(5, 1), "Labour Day"),
        (after(39), "Ascension Day"),
        (after(49), "Whit Sunday"),
        (after(50), "Whit Monday"),
        (on(6, 17), "National Day"),
        (on_or_after(on(8, 1), 1), "Commerce Day"),
        (on(12, 24), "Christmas Eve"),
        (on(12, 25), "Christmas Day"),
        (on(12, 26), "Boxing Day"),
        (on(12, 31), "New Year's Eve"),
    ];
    days.sort_by_key(|(d, _)| *d);
    days
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(year: i64, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn easter_matches_the_published_dates() {
        for (year, month, day) in [(2000, 4, 23), (2019, 4, 21), (2024, 3, 31), (2025, 4, 20), (2026, 4, 5), (2027, 3, 28), (2038, 4, 25), (1943, 4, 25)] {
            assert_eq!(easter(year), d(year, month, day), "Easter {year}");
        }
    }

    #[test]
    fn the_holidays_of_2026() {
        let got = iceland(2026);
        let expected = [
            (d(2026, 1, 1), "New Year's Day"),
            (d(2026, 4, 2), "Maundy Thursday"),
            (d(2026, 4, 3), "Good Friday"),
            (d(2026, 4, 5), "Easter Sunday"),
            (d(2026, 4, 6), "Easter Monday"),
            (d(2026, 4, 23), "First Day of Summer"),
            (d(2026, 5, 1), "Labour Day"),
            (d(2026, 5, 14), "Ascension Day"),
            (d(2026, 5, 24), "Whit Sunday"),
            (d(2026, 5, 25), "Whit Monday"),
            (d(2026, 6, 17), "National Day"),
            (d(2026, 8, 3), "Commerce Day"),
            (d(2026, 12, 24), "Christmas Eve"),
            (d(2026, 12, 25), "Christmas Day"),
            (d(2026, 12, 26), "Boxing Day"),
            (d(2026, 12, 31), "New Year's Eve"),
        ];
        assert_eq!(got, expected);
    }

    #[test]
    fn the_weekday_rules_land_on_the_right_weekday_every_year() {
        for year in 1990..2060 {
            let all = iceland(year);
            let find = |name: &str| all.iter().find(|(_, n)| *n == name).unwrap().0;
            let summer = find("First Day of Summer");
            assert_eq!(summer.weekday(), 4, "a Thursday, {year}");
            assert!((19..=25).contains(&summer.day) && summer.month == 4, "{summer:?}");
            let commerce = find("Commerce Day");
            assert_eq!(commerce.weekday(), 1, "a Monday, {year}");
            assert!(commerce.month == 8 && commerce.day <= 7);
            assert_eq!(find("Easter Sunday").weekday(), 0);
            assert_eq!(find("Good Friday").weekday(), 5);
            assert_eq!(find("Ascension Day").weekday(), 4);
            assert!(all.windows(2).all(|w| w[0].0 <= w[1].0), "in date order, {year}");
        }
    }
}
