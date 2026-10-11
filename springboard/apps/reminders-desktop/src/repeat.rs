//! Repeat rules and next-occurrence logic (pure, host-testable).
use springboard::shell::clock::tz::{civil_from_days, days_from_civil};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Day,
    Week,
    Month,
    Year,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Repeat {
    pub unit: Unit,
    pub interval: u32,
}

pub const MAX_INTERVAL: u32 = 999;

impl Repeat {
    pub fn new(unit: Unit, interval: u32) -> Self {
        Self {
            unit,
            interval: interval.clamp(1, MAX_INTERVAL),
        }
    }
    pub fn label(self) -> String {
        let (one, many) = match self.unit {
            Unit::Day => ("Daily", "days"),
            Unit::Week => ("Weekly", "weeks"),
            Unit::Month => ("Monthly", "months"),
            Unit::Year => ("Yearly", "years"),
        };
        if self.interval == 1 {
            one.to_string()
        } else {
            format!("Every {} {}", self.interval, many)
        }
    }
    /// Compact persisted form: `d1`, `w2`, `m1`, `y3`.
    pub fn encode(self) -> String {
        let c = match self.unit {
            Unit::Day => 'd',
            Unit::Week => 'w',
            Unit::Month => 'm',
            Unit::Year => 'y',
        };
        format!("{c}{}", self.interval)
    }
    pub fn decode(text: &str) -> Option<Self> {
        let unit = match text.chars().next()? {
            'd' => Unit::Day,
            'w' => Unit::Week,
            'm' => Unit::Month,
            'y' => Unit::Year,
            _ => return None,
        };
        let interval: u32 = text.get(1..)?.parse().ok()?;
        (1..=MAX_INTERVAL)
            .contains(&interval)
            .then_some(Self { unit, interval })
    }
    /// Steps the Repeat to the next preset in the editor cycle.
    pub fn cycle(current: Option<Self>) -> Option<Self> {
        match current {
            None => Some(Self::new(Unit::Day, 1)),
            Some(r) if r.interval == 1 => match r.unit {
                Unit::Day => Some(Self::new(Unit::Week, 1)),
                Unit::Week => Some(Self::new(Unit::Month, 1)),
                Unit::Month => Some(Self::new(Unit::Year, 1)),
                Unit::Year => None,
            },
            Some(_) => None,
        }
    }
}

/// Adds `months` to a day keeping the day-of-month, clamped to the month end.
pub fn add_months(day: i64, months: i64) -> i64 {
    let (y, m, d) = civil_from_days(day);
    let total = y * 12 + m as i64 - 1 + months;
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) as u32 + 1);
    let last = days_from_civil(
        if nm == 12 { ny + 1 } else { ny },
        if nm == 12 { 1 } else { nm + 1 },
        1,
    ) - days_from_civil(ny, nm, 1);
    days_from_civil(ny, nm, d.min(last as u32))
}

/// The occurrence one step after `day`.
pub fn step(day: i64, r: Repeat) -> i64 {
    let n = r.interval as i64;
    match r.unit {
        Unit::Day => day + n,
        Unit::Week => day + 7 * n,
        Unit::Month => add_months(day, n),
        Unit::Year => add_months(day, 12 * n),
    }
}

/// The next due day after completing a reminder due on `due`: one step, then
/// further steps until the result is not in the past (no overdue pile-up).
pub fn next_occurrence(due: i64, r: Repeat, today: i64) -> i64 {
    let mut next = step(due, r);
    let mut guard = 0;
    while next < today && guard < 100_000 {
        next = step(next, r);
        guard += 1;
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d(y: i64, m: u32, dd: u32) -> i64 {
        days_from_civil(y, m, dd)
    }
    #[test]
    fn daily_weekly_steps() {
        assert_eq!(step(d(2026, 10, 4), Repeat::new(Unit::Day, 1)), d(2026, 10, 5));
        assert_eq!(step(d(2026, 12, 30), Repeat::new(Unit::Day, 3)), d(2027, 1, 2));
        assert_eq!(step(d(2026, 10, 4), Repeat::new(Unit::Week, 2)), d(2026, 10, 18));
    }
    #[test]
    fn monthly_clamps_to_month_end_and_crosses_years() {
        let m = Repeat::new(Unit::Month, 1);
        assert_eq!(step(d(2026, 1, 31), m), d(2026, 2, 28));
        assert_eq!(step(d(2028, 1, 31), m), d(2028, 2, 29));
        assert_eq!(step(d(2026, 12, 15), m), d(2027, 1, 15));
        assert_eq!(step(d(2026, 11, 30), Repeat::new(Unit::Month, 3)), d(2027, 2, 28));
    }
    #[test]
    fn yearly_handles_leap_day() {
        let y = Repeat::new(Unit::Year, 1);
        assert_eq!(step(d(2028, 2, 29), y), d(2029, 2, 28));
        assert_eq!(step(d(2026, 10, 4), Repeat::new(Unit::Year, 2)), d(2028, 10, 4));
    }
    #[test]
    fn next_occurrence_skips_past_dates() {
        let daily = Repeat::new(Unit::Day, 1);
        let today = d(2026, 10, 10);
        assert_eq!(next_occurrence(d(2026, 10, 10), daily, today), d(2026, 10, 11));
        assert_eq!(next_occurrence(d(2026, 10, 1), daily, today), today);
        let weekly = Repeat::new(Unit::Week, 1);
        assert_eq!(next_occurrence(d(2026, 9, 1), weekly, today), d(2026, 10, 13));
    }
    #[test]
    fn encode_decode_and_cycle() {
        for r in [Repeat::new(Unit::Day, 1), Repeat::new(Unit::Week, 2), Repeat::new(Unit::Month, 12), Repeat::new(Unit::Year, 999)] {
            assert_eq!(Repeat::decode(&r.encode()), Some(r));
        }
        assert_eq!(Repeat::decode("x1"), None);
        assert_eq!(Repeat::decode("d0"), None);
        assert_eq!(Repeat::decode("d1000"), None);
        assert_eq!(Repeat::decode(""), None);
        assert_eq!(Repeat::new(Unit::Week, 1).label(), "Weekly");
        assert_eq!(Repeat::new(Unit::Month, 3).label(), "Every 3 months");
        let mut c = None;
        let mut seen = vec![];
        for _ in 0..5 {
            c = Repeat::cycle(c);
            seen.push(c);
        }
        assert_eq!(seen[3], Some(Repeat::new(Unit::Year, 1)));
        assert_eq!(seen[4], None);
    }
}
