//! `Calendar` for the calendar systems Foundation names: `gregorian`, `buddhist`, `japanese`, `islamic-civil`
//! (the tabular Islamic calendar), `hebrew` and `iso8601`. Pure integer arithmetic over rata die day numbers
//! (day 1 = 0001-01-01 of the proleptic Gregorian calendar), after Calendrical Calculations. Not the EventKit
//! calendar (`calendar.rs`).
//!
//! Component numbering follows Foundation: `weekday` 1 = Sunday .. 7 = Saturday, `month` 1-based. In the
//! Hebrew calendar `month` is 1 = Tishri .. 13 = Elul with 6 (Adar I) present only in leap years.
//! The Japanese `year` counts within the era (Reiwa 1 = 2019); eras are numbered as Foundation does
//! (Meiji 232 .. Reiwa 236); dates before Meiji report era 0 and the Gregorian year.

use crate::time_zone::TimeZone;

/// Days between 0001-01-01 (rata die 1) and 1970-01-01, i.e. rata die of the Unix epoch day.
const UNIX_EPOCH_RD: i64 = 719_163;
const HEBREW_EPOCH: i64 = -1_373_427;
const ISLAMIC_EPOCH: i64 = 227_015;

/// Days since 1970-01-01 of a proleptic Gregorian date (month 1..=12).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The proleptic Gregorian date (year, month 1..=12, day) of a day count since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub fn is_gregorian_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn gregorian_month_days(y: i64, m: i64) -> i64 {
    match m {
        2 => 28 + i64::from(is_gregorian_leap(y)),
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

// ---------------------------------------------------------------- Hebrew (month numbers here: Nisan = 1 .. Elul = 6, Tishri = 7)

fn hebrew_leap(y: i64) -> bool {
    (7 * y + 1).rem_euclid(19) < 7
}

fn hebrew_months_in_year(y: i64) -> i64 {
    if hebrew_leap(y) { 13 } else { 12 }
}

fn hebrew_elapsed_days(y: i64) -> i64 {
    let months = (235 * y - 234).div_euclid(19);
    let parts = 12_084 + 13_753 * months;
    let days = 29 * months + parts.div_euclid(25_920);
    if (3 * (days + 1)).rem_euclid(7) < 3 { days + 1 } else { days }
}

fn hebrew_new_year_delay(y: i64) -> i64 {
    let (n0, n1, n2) = (hebrew_elapsed_days(y - 1), hebrew_elapsed_days(y), hebrew_elapsed_days(y + 1));
    if n2 - n1 == 356 {
        2
    } else if n1 - n0 == 382 {
        1
    } else {
        0
    }
}

fn hebrew_new_year(y: i64) -> i64 {
    HEBREW_EPOCH + hebrew_elapsed_days(y) + hebrew_new_year_delay(y)
}

fn hebrew_days_in_year(y: i64) -> i64 {
    hebrew_new_year(y + 1) - hebrew_new_year(y)
}

fn hebrew_last_day(y: i64, m: i64) -> i64 {
    let len = hebrew_days_in_year(y);
    match m {
        2 | 4 | 6 | 10 | 13 => 29,
        12 if !hebrew_leap(y) => 29,
        8 if !matches!(len, 355 | 385) => 29,
        9 if matches!(len, 353 | 383) => 29,
        _ => 30,
    }
}

fn hebrew_to_rd(y: i64, m: i64, d: i64) -> i64 {
    let mut rd = hebrew_new_year(y) + d - 1;
    if m < 7 {
        for mm in 7..=hebrew_months_in_year(y) {
            rd += hebrew_last_day(y, mm);
        }
        for mm in 1..m {
            rd += hebrew_last_day(y, mm);
        }
    } else {
        for mm in 7..m {
            rd += hebrew_last_day(y, mm);
        }
    }
    rd
}

fn hebrew_from_rd(rd: i64) -> (i64, i64, i64) {
    let approx = ((rd - HEBREW_EPOCH) as f64 * 98_496.0 / 35_975_351.0).floor() as i64;
    let mut y = approx - 1;
    while hebrew_new_year(y + 1) <= rd {
        y += 1;
    }
    let mut m = if rd < hebrew_to_rd(y, 1, 1) { 7 } else { 1 };
    while rd > hebrew_to_rd(y, m, hebrew_last_day(y, m)) {
        m += 1;
    }
    (y, m, rd - hebrew_to_rd(y, m, 1) + 1)
}

/// Chronological month position within the Hebrew year (1 = Tishri) of a Nisan-based month number.
fn hebrew_pos(y: i64, cc: i64) -> i64 {
    let before_nisan = hebrew_months_in_year(y) - 6;
    if cc >= 7 { cc - 6 } else { cc + before_nisan }
}

fn hebrew_cc(y: i64, pos: i64) -> i64 {
    let before_nisan = hebrew_months_in_year(y) - 6;
    if pos <= before_nisan { pos + 6 } else { pos - before_nisan }
}

/// Foundation's month number (Tishri = 1, Adar I = 6, Adar II / Adar = 7, Nisan = 8) of a position.
fn hebrew_month_number(y: i64, pos: i64) -> i64 {
    if hebrew_leap(y) || pos <= 5 { pos } else { pos + 1 }
}

fn hebrew_pos_from_number(y: i64, number: i64) -> Option<i64> {
    if hebrew_leap(y) || number <= 5 {
        Some(number)
    } else if number == 6 {
        None
    } else {
        Some(number - 1)
    }
}

// ---------------------------------------------------------------- Islamic (tabular, civil epoch)

fn islamic_leap(y: i64) -> bool {
    (14 + 11 * y).rem_euclid(30) < 11
}

fn islamic_month_days(y: i64, m: i64) -> i64 {
    if m % 2 == 1 || (m == 12 && islamic_leap(y)) { 30 } else { 29 }
}

fn islamic_to_rd(y: i64, m: i64, d: i64) -> i64 {
    d + 29 * (m - 1) + (6 * m - 1).div_euclid(11) + (y - 1) * 354 + (3 + 11 * y).div_euclid(30) + ISLAMIC_EPOCH - 1
}

fn islamic_from_rd(rd: i64) -> (i64, i64, i64) {
    let y = (30 * (rd - ISLAMIC_EPOCH) + 10_646).div_euclid(10_631);
    let prior = rd - islamic_to_rd(y, 1, 1);
    let m = (11 * prior + 330).div_euclid(325);
    (y, m, rd - islamic_to_rd(y, m, 1) + 1)
}

// ---------------------------------------------------------------- Japanese eras

/// (Foundation era number, first Gregorian year, month, day), newest first.
const JAPANESE_ERAS: [(i64, i64, i64, i64); 5] = [(236, 2019, 5, 1), (235, 1989, 1, 8), (234, 1926, 12, 25), (233, 1912, 7, 30), (232, 1868, 9, 8)];

fn japanese_era_of(y: i64, m: i64, d: i64) -> Option<(i64, i64)> {
    JAPANESE_ERAS.iter().find(|&&(_, ey, em, ed)| (y, m, d) >= (ey, em, ed)).map(|&(n, ey, ..)| (n, y - ey + 1))
}

// ---------------------------------------------------------------- Calendar

/// The calendar system, `Calendar.Identifier`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalendarIdentifier {
    Gregorian,
    Buddhist,
    Japanese,
    /// `islamicCivil`, the tabular calendar
    Islamic,
    Hebrew,
    Iso8601,
}

impl CalendarIdentifier {
    /// The identifier's raw string (`gregorian`, `buddhist`, `japanese`, `islamic-civil`, `hebrew`, `iso8601`).
    pub fn name(self) -> &'static str {
        match self {
            CalendarIdentifier::Gregorian => "gregorian",
            CalendarIdentifier::Buddhist => "buddhist",
            CalendarIdentifier::Japanese => "japanese",
            CalendarIdentifier::Islamic => "islamic-civil",
            CalendarIdentifier::Hebrew => "hebrew",
            CalendarIdentifier::Iso8601 => "iso8601",
        }
    }

    /// Parses a raw identifier string; `islamic` is accepted as the tabular calendar too.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "gregorian" => CalendarIdentifier::Gregorian,
            "buddhist" => CalendarIdentifier::Buddhist,
            "japanese" => CalendarIdentifier::Japanese,
            "islamic-civil" | "islamic" => CalendarIdentifier::Islamic,
            "hebrew" => CalendarIdentifier::Hebrew,
            "iso8601" => CalendarIdentifier::Iso8601,
            _ => return None,
        })
    }
}

/// `Calendar.Component`, the units `range(of:in:for:)` understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
}

/// `DateComponents`: every field optional.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DateComponents {
    pub era: Option<i64>,
    pub year: Option<i64>,
    pub month: Option<i64>,
    pub day: Option<i64>,
    pub hour: Option<i64>,
    pub minute: Option<i64>,
    pub second: Option<i64>,
    /// 1 = Sunday .. 7 = Saturday; read-only output of `date_components`
    pub weekday: Option<i64>,
}

impl DateComponents {
    pub fn ymd(year: i64, month: i64, day: i64) -> Self {
        DateComponents { year: Some(year), month: Some(month), day: Some(day), ..Default::default() }
    }
    pub fn hms(mut self, hour: i64, minute: i64, second: i64) -> Self {
        self.hour = Some(hour);
        self.minute = Some(minute);
        self.second = Some(second);
        self
    }
}

/// A calendar system in a time zone.
#[derive(Clone, Debug)]
pub struct Calendar {
    pub identifier: CalendarIdentifier,
    pub time_zone: TimeZone,
    /// 1 = Sunday .. 7 = Saturday
    pub first_weekday: i64,
    pub minimum_days_in_first_week: i64,
}

impl Calendar {
    pub fn new(identifier: CalendarIdentifier, time_zone: TimeZone) -> Self {
        let (first_weekday, minimum_days_in_first_week) = if identifier == CalendarIdentifier::Iso8601 { (2, 4) } else { (1, 1) };
        Calendar { identifier, time_zone, first_weekday, minimum_days_in_first_week }
    }

    /// The time zone's offset from GMT in seconds at a Unix time (`secondsFromGMT(for:)`).
    fn offset_at(&self, unix: i64) -> i64 {
        i64::from(self.time_zone.seconds_from_gmt(crate::coding::Date(unix as f64)))
    }

    /// (year, month, day) in the calendar's own arithmetic: the Gregorian year for the Gregorian family,
    /// a chronological month position for the Hebrew calendar.
    fn ymd_from_rd(&self, rd: i64) -> (i64, i64, i64) {
        match self.identifier {
            CalendarIdentifier::Islamic => islamic_from_rd(rd),
            CalendarIdentifier::Hebrew => {
                let (y, cc, d) = hebrew_from_rd(rd);
                (y, hebrew_pos(y, cc), d)
            }
            _ => civil_from_days(rd - UNIX_EPOCH_RD),
        }
    }

    fn rd_from_ymd(&self, y: i64, m: i64, d: i64) -> i64 {
        match self.identifier {
            CalendarIdentifier::Islamic => islamic_to_rd(y, m, d),
            CalendarIdentifier::Hebrew => hebrew_to_rd(y, hebrew_cc(y, m), d),
            _ => days_from_civil(y, m, d) + UNIX_EPOCH_RD,
        }
    }

    fn months_in_year(&self, y: i64) -> i64 {
        if self.identifier == CalendarIdentifier::Hebrew { hebrew_months_in_year(y) } else { 12 }
    }

    fn days_in_month(&self, y: i64, m: i64) -> i64 {
        match self.identifier {
            CalendarIdentifier::Islamic => islamic_month_days(y, m),
            CalendarIdentifier::Hebrew => hebrew_last_day(y, hebrew_cc(y, m)),
            _ => gregorian_month_days(y, m),
        }
    }

    fn days_in_year(&self, y: i64) -> i64 {
        match self.identifier {
            CalendarIdentifier::Islamic => 354 + i64::from(islamic_leap(y)),
            CalendarIdentifier::Hebrew => hebrew_days_in_year(y),
            _ => 365 + i64::from(is_gregorian_leap(y)),
        }
    }

    /// The calendar's fields from an instant (`dateComponents([...], from:)`, every field).
    pub fn date_components(&self, unix: i64) -> DateComponents {
        let local = unix + self.offset_at(unix);
        let days = local.div_euclid(86_400);
        let sod = local.rem_euclid(86_400);
        let (y, m, d) = self.ymd_from_rd(days + UNIX_EPOCH_RD);
        let (era, year, month) = match self.identifier {
            CalendarIdentifier::Buddhist => (1, y + 543, m),
            CalendarIdentifier::Japanese => match japanese_era_of(y, m, d) {
                Some((era, year)) => (era, year, m),
                None => (0, y, m),
            },
            CalendarIdentifier::Hebrew => (1, y, hebrew_month_number(y, m)),
            CalendarIdentifier::Islamic => (1, y, m),
            _ => {
                if y >= 1 { (1, y, m) } else { (0, 1 - y, m) }
            }
        };
        DateComponents {
            era: Some(era),
            year: Some(year),
            month: Some(month),
            day: Some(d),
            hour: Some(sod / 3600),
            minute: Some(sod / 60 % 60),
            second: Some(sod % 60),
            weekday: Some((days + 4).rem_euclid(7) + 1),
        }
    }

    /// The internal (year, month) of components; None for an invalid month or an unknown era.
    fn internal_ym(&self, c: &DateComponents) -> Option<(i64, i64)> {
        let year = c.year?;
        let month = c.month.unwrap_or(1);
        match self.identifier {
            CalendarIdentifier::Buddhist => Some((year - 543, month)),
            CalendarIdentifier::Japanese => {
                let era = c.era.unwrap_or(236);
                let start = JAPANESE_ERAS.iter().find(|e| e.0 == era)?.1;
                Some((start + year - 1, month))
            }
            CalendarIdentifier::Hebrew => Some((year, hebrew_pos_from_number(year, month)?)),
            CalendarIdentifier::Islamic => Some((year, month)),
            _ => Some((if c.era == Some(0) { 1 - year } else { year }, month)),
        }
    }

    /// An instant from local wall-clock fields (`date(from:)`). Missing day is 1, missing time fields 0.
    /// None when the month or day is out of range. A wall time that does not exist (spring forward) or
    /// occurs twice (fall back) resolves to the earlier instant of the two readings.
    pub fn date(&self, c: &DateComponents) -> Option<i64> {
        let (y, m) = self.internal_ym(c)?;
        let d = c.day.unwrap_or(1);
        if m < 1 || m > self.months_in_year(y) || d < 1 || d > self.days_in_month(y, m) {
            return None;
        }
        let rd = self.rd_from_ymd(y, m, d);
        let sod = c.hour.unwrap_or(0) * 3600 + c.minute.unwrap_or(0) * 60 + c.second.unwrap_or(0);
        Some(self.from_local(rd, sod))
    }

    fn from_local(&self, rd: i64, sod: i64) -> i64 {
        let local = (rd - UNIX_EPOCH_RD) * 86_400 + sod;
        let o0 = self.offset_at(local);
        let u1 = local - o0;
        let o1 = self.offset_at(u1);
        if o1 == o0 {
            return u1;
        }
        let u2 = local - o1;
        if self.offset_at(u2) == o1 { u2.min(u1) } else { u1 }
    }

    /// `date(byAdding:to:)`: years and months move in the calendar's own months (the day clamps to the
    /// month's length), days and weeks keep the wall-clock time across a DST change, hours, minutes and
    /// seconds are elapsed time.
    pub fn date_by_adding(&self, unix: i64, add: &DateComponents) -> i64 {
        let local = unix + self.offset_at(unix);
        let days = local.div_euclid(86_400);
        let sod = local.rem_euclid(86_400);
        let (mut y, mut m, d) = self.ymd_from_rd(days + UNIX_EPOCH_RD);
        y += add.year.unwrap_or(0);
        let mut n = add.month.unwrap_or(0);
        if self.identifier == CalendarIdentifier::Hebrew {
            m = m.min(self.months_in_year(y));
        }
        while n > 0 {
            if m == self.months_in_year(y) {
                y += 1;
                m = 1;
            } else {
                m += 1;
            }
            n -= 1;
        }
        while n < 0 {
            if m == 1 {
                y -= 1;
                m = self.months_in_year(y);
            } else {
                m -= 1;
            }
            n += 1;
        }
        let d = d.min(self.days_in_month(y, m));
        let rd = self.rd_from_ymd(y, m, d) + add.day.unwrap_or(0);
        let instant = self.from_local(rd, sod);
        instant + add.hour.unwrap_or(0) * 3600 + add.minute.unwrap_or(0) * 60 + add.second.unwrap_or(0)
    }

    /// `range(of: smaller, in: larger, for: date)` as (location, length). Supported pairs: day in month,
    /// day in year, month in year, hour in day, minute in hour, second in minute.
    pub fn range(&self, of: Component, within: Component, unix: i64) -> Option<(i64, i64)> {
        let local = unix + self.offset_at(unix);
        let (y, m, _) = self.ymd_from_rd(local.div_euclid(86_400) + UNIX_EPOCH_RD);
        match (of, within) {
            (Component::Day, Component::Month) => Some((1, self.days_in_month(y, m))),
            (Component::Day, Component::Year) => Some((1, self.days_in_year(y))),
            (Component::Month, Component::Year) => Some((1, self.months_in_year(y))),
            (Component::Hour, Component::Day) => Some((0, 24)),
            (Component::Minute, Component::Hour) => Some((0, 60)),
            (Component::Second, Component::Minute) => Some((0, 60)),
            _ => None,
        }
    }

    /// The first instant of the local day that holds `unix` (`startOfDay(for:)`).
    pub fn start_of_day(&self, unix: i64) -> i64 {
        let local = unix + self.offset_at(unix);
        self.from_local(local.div_euclid(86_400) + UNIX_EPOCH_RD, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cal(id: CalendarIdentifier) -> Calendar {
        Calendar::new(id, TimeZone::utc())
    }

    fn unix(y: i64, m: i64, d: i64) -> i64 {
        days_from_civil(y, m, d) * 86_400
    }

    #[test]
    fn civil_day_conversion_round_trips_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        for z in [-1_000_000, -1, 0, 1, 20_000, 2_000_000] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
    }

    #[test]
    fn identifiers_round_trip_through_their_raw_strings() {
        for id in [CalendarIdentifier::Gregorian, CalendarIdentifier::Buddhist, CalendarIdentifier::Japanese, CalendarIdentifier::Islamic, CalendarIdentifier::Hebrew, CalendarIdentifier::Iso8601] {
            assert_eq!(CalendarIdentifier::from_name(id.name()), Some(id));
        }
        assert_eq!(CalendarIdentifier::from_name("klingon"), None);
    }

    #[test]
    fn adding_a_month_clamps_at_month_ends_and_leap_years() {
        let c = cal(CalendarIdentifier::Gregorian);
        let one_month = DateComponents { month: Some(1), ..Default::default() };
        assert_eq!(c.date_by_adding(unix(2026, 1, 31), &one_month), unix(2026, 2, 28));
        assert_eq!(c.date_by_adding(unix(2028, 1, 31), &one_month), unix(2028, 2, 29));
        let one_year = DateComponents { year: Some(1), ..Default::default() };
        assert_eq!(c.date_by_adding(unix(2028, 2, 29), &one_year), unix(2029, 2, 28));
        let back = DateComponents { month: Some(-2), ..Default::default() };
        assert_eq!(c.date_by_adding(unix(2026, 1, 15), &back), unix(2025, 11, 15));
    }

    #[test]
    fn components_report_weekday_and_time() {
        let c = cal(CalendarIdentifier::Gregorian);
        let comp = c.date_components(unix(2026, 10, 13) + 3 * 3600 + 4 * 60 + 5);
        assert_eq!((comp.year, comp.month, comp.day), (Some(2026), Some(10), Some(13)));
        assert_eq!((comp.hour, comp.minute, comp.second), (Some(3), Some(4), Some(5)));
        assert_eq!(comp.weekday, Some(3), "2026-10-13 is a Tuesday");
        assert_eq!(c.date(&DateComponents::ymd(2026, 10, 13).hms(3, 4, 5)), Some(unix(2026, 10, 13) + 11_045));
        assert_eq!(c.date(&DateComponents::ymd(2026, 2, 30)), None);
        assert_eq!(c.date(&DateComponents::ymd(2026, 13, 1)), None);
    }

    #[test]
    fn ranges_count_days_and_months() {
        let c = cal(CalendarIdentifier::Gregorian);
        assert_eq!(c.range(Component::Day, Component::Month, unix(2028, 2, 10)), Some((1, 29)));
        assert_eq!(c.range(Component::Day, Component::Month, unix(2026, 2, 10)), Some((1, 28)));
        assert_eq!(c.range(Component::Day, Component::Year, unix(2028, 6, 1)), Some((1, 366)));
        assert_eq!(c.range(Component::Month, Component::Year, unix(2026, 6, 1)), Some((1, 12)));
        assert_eq!(c.range(Component::Year, Component::Day, 0), None);
    }

    #[test]
    fn iso8601_starts_the_week_on_monday() {
        assert_eq!(cal(CalendarIdentifier::Iso8601).first_weekday, 2);
        assert_eq!(cal(CalendarIdentifier::Gregorian).first_weekday, 1);
    }

    #[test]
    fn buddhist_year_is_gregorian_plus_543() {
        let comp = cal(CalendarIdentifier::Buddhist).date_components(unix(2026, 10, 13));
        assert_eq!((comp.year, comp.month, comp.day), (Some(2569), Some(10), Some(13)));
        let c = cal(CalendarIdentifier::Buddhist);
        assert_eq!(c.date(&DateComponents::ymd(2569, 10, 13)), Some(unix(2026, 10, 13)));
    }

    #[test]
    fn japanese_eras_start_on_their_accession_days() {
        let c = cal(CalendarIdentifier::Japanese);
        let at = |y, m, d| {
            let comp = c.date_components(unix(y, m, d));
            (comp.era.unwrap(), comp.year.unwrap())
        };
        assert_eq!(at(2026, 10, 13), (236, 8));
        assert_eq!(at(2019, 5, 1), (236, 1));
        assert_eq!(at(2019, 4, 30), (235, 31));
        assert_eq!(at(1989, 1, 7), (234, 64));
        assert_eq!(at(1989, 1, 8), (235, 1));
        assert_eq!(at(1912, 7, 30), (233, 1));
        assert_eq!(at(1868, 9, 8), (232, 1));
        assert_eq!(at(1800, 1, 1), (0, 1800));
        let reiwa8 = DateComponents { era: Some(236), ..DateComponents::ymd(8, 10, 13) };
        assert_eq!(c.date(&reiwa8), Some(unix(2026, 10, 13)));
    }

    #[test]
    fn islamic_tabular_matches_known_days() {
        assert_eq!(islamic_to_rd(1, 1, 1), 227_015, "the civil epoch");
        assert_eq!(islamic_from_rd(227_015), (1, 1, 1));
        let c = cal(CalendarIdentifier::Islamic);
        let comp = c.date_components(unix(2025, 6, 27));
        assert_eq!((comp.year, comp.month, comp.day), (Some(1447), Some(1), Some(1)));
        for rd in [227_015, 500_000, 739_429, 760_000] {
            let (y, m, d) = islamic_from_rd(rd);
            assert_eq!(islamic_to_rd(y, m, d), rd);
        }
        assert_eq!(c.range(Component::Month, Component::Year, unix(2025, 7, 1)), Some((1, 12)));
        assert_eq!(c.range(Component::Day, Component::Year, unix(2025, 7, 1)), Some((1, 354 + i64::from(islamic_leap(1447)))));
    }

    #[test]
    fn hebrew_new_years_and_leap_years() {
        let c = cal(CalendarIdentifier::Hebrew);
        let comp = c.date_components(unix(2025, 9, 23));
        assert_eq!((comp.year, comp.month, comp.day), (Some(5786), Some(1), Some(1)), "Rosh Hashana 5786");
        let comp = c.date_components(unix(2026, 9, 12));
        assert_eq!((comp.year, comp.month, comp.day), (Some(5787), Some(1), Some(1)), "Rosh Hashana 5787");
        assert!(hebrew_leap(5784) && !hebrew_leap(5786));
        assert_eq!(c.range(Component::Month, Component::Year, unix(2024, 1, 1)), Some((1, 13)));
        assert_eq!(c.range(Component::Month, Component::Year, unix(2026, 1, 1)), Some((1, 12)));
        for rd in [1_000, 500_000, 738_000, 740_000] {
            let (y, m, d) = hebrew_from_rd(rd);
            assert_eq!(hebrew_to_rd(y, m, d), rd);
        }
        assert_eq!(c.date(&DateComponents::ymd(5786, 1, 1)), Some(unix(2025, 9, 23)));
        assert_eq!(c.date(&DateComponents::ymd(5786, 6, 1)), None, "Adar I exists only in leap years");
    }

    #[test]
    fn hebrew_month_arithmetic_walks_the_months_of_the_year() {
        let c = cal(CalendarIdentifier::Hebrew);
        let start = unix(2025, 9, 23);
        let after = c.date_by_adding(start, &DateComponents { month: Some(12), ..Default::default() });
        let comp = c.date_components(after);
        assert_eq!((comp.year, comp.month, comp.day), (Some(5787), Some(1), Some(1)), "a 12 month year (5786) ends after 12 months");
    }

    #[test]
    fn day_arithmetic_keeps_the_wall_clock_across_dst() {
        let ny = TimeZone::from_posix_rule("America/New_York", "EST5EDT,M3.2.0,M11.1.0").unwrap();
        let c = Calendar::new(CalendarIdentifier::Gregorian, ny);
        let noon = c.date(&DateComponents::ymd(2026, 3, 7).hms(12, 0, 0)).unwrap();
        let next = c.date_by_adding(noon, &DateComponents { day: Some(1), ..Default::default() });
        assert_eq!(next - noon, 23 * 3600, "the clock stays at noon, so the elapsed time is 23 hours");
        let elapsed = c.date_by_adding(noon, &DateComponents { hour: Some(24), ..Default::default() });
        assert_eq!(elapsed - noon, 24 * 3600);
        let comp = c.date_components(next);
        assert_eq!((comp.day, comp.hour), (Some(8), Some(12)));
        assert_eq!(c.start_of_day(next), c.date(&DateComponents::ymd(2026, 3, 8)).unwrap());
    }
}
