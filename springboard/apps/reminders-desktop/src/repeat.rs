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
    /// The day of the month (29 to 31) a monthly or yearly series falls on, kept once a short month has moved a due
    /// day back to its last day, so the next longer month returns to it. Set by `next_occurrence`.
    pub anchor: Option<u8>,
}

pub const MAX_INTERVAL: u32 = 999;

impl Repeat {
    pub fn new(unit: Unit, interval: u32) -> Self {
        Self {
            unit,
            interval: interval.clamp(1, MAX_INTERVAL),
            anchor: None,
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
    /// Compact persisted form: `d1`, `w2`, `m1`, `y3`; a kept day of the month follows as `m1@31`.
    pub fn encode(self) -> String {
        let c = match self.unit {
            Unit::Day => 'd',
            Unit::Week => 'w',
            Unit::Month => 'm',
            Unit::Year => 'y',
        };
        match self.anchor {
            Some(day) => format!("{c}{}@{day}", self.interval),
            None => format!("{c}{}", self.interval),
        }
    }
    pub fn decode(text: &str) -> Option<Self> {
        let unit = match text.chars().next()? {
            'd' => Unit::Day,
            'w' => Unit::Week,
            'm' => Unit::Month,
            'y' => Unit::Year,
            _ => return None,
        };
        let rest = text.get(1..)?;
        let (count, anchor) = match rest.split_once('@') {
            Some((count, day)) => (count, Some(day.parse::<u8>().ok().filter(|d| (29..=31).contains(d))?)),
            None => (rest, None),
        };
        let interval: u32 = count.parse().ok()?;
        (1..=MAX_INTERVAL)
            .contains(&interval)
            .then_some(Self { unit, interval, anchor })
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

/// Days in month `m` (1 to 12) of year `y`.
fn month_len(y: i64, m: u32) -> u32 {
    let next = days_from_civil(if m == 12 { y + 1 } else { y }, if m == 12 { 1 } else { m + 1 }, 1);
    (next - days_from_civil(y, m, 1)) as u32
}

/// Day `d` of the month `months` after month `m` of year `y`, clamped to that month's end.
fn month_day(y: i64, m: u32, months: i64, d: u32) -> i64 {
    let total = y * 12 + m as i64 - 1 + months;
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) as u32 + 1);
    days_from_civil(ny, nm, d.min(month_len(ny, nm)))
}

/// Adds `months` to a day keeping the day-of-month, clamped to the month end.
pub fn add_months(day: i64, months: i64) -> i64 {
    let (y, m, d) = civil_from_days(day);
    month_day(y, m, months, d)
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
///
/// Also returns the rule to keep. A monthly or yearly series stays on its day of the month: every occurrence is
/// counted from `due`'s month with that day, not from the previous (possibly shortened) occurrence, and a day past
/// the 28th is kept in the rule so that 31 January, 28 February goes on to 31 March. A kept day is used only while
/// `due` is still that day clamped to its month; a due date moved elsewhere starts a new series.
pub fn next_occurrence(due: i64, r: Repeat, today: i64) -> (i64, Repeat) {
    let months = match r.unit {
        Unit::Day | Unit::Week => {
            let mut next = step(due, r);
            let mut guard = 0;
            while next < today && guard < 100_000 {
                next = step(next, r);
                guard += 1;
            }
            return (next, Repeat { anchor: None, ..r });
        }
        Unit::Month => r.interval as i64,
        Unit::Year => 12 * r.interval as i64,
    };
    let (y, m, d) = civil_from_days(due);
    let day = match r.anchor {
        Some(a) if u32::from(a) > d && d == month_len(y, m) => u32::from(a),
        _ => d,
    };
    let mut k = 1;
    let mut next = month_day(y, m, months, day);
    while next < today && k < 100_000 {
        k += 1;
        next = month_day(y, m, k * months, day);
    }
    let anchor = (day > 28).then_some(day as u8);
    (next, Repeat { anchor, ..r })
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
        assert_eq!(next_occurrence(d(2026, 10, 10), daily, today), (d(2026, 10, 11), daily));
        assert_eq!(next_occurrence(d(2026, 10, 1), daily, today), (today, daily));
        let weekly = Repeat::new(Unit::Week, 1);
        assert_eq!(next_occurrence(d(2026, 9, 1), weekly, today), (d(2026, 10, 13), weekly));
    }
    /// Completes a reminder `times` times, each on its due day, and lists the due days it moves through.
    fn series(mut due: i64, mut rule: Repeat, times: usize) -> Vec<i64> {
        (0..times)
            .map(|_| {
                (due, rule) = next_occurrence(due, rule, due);
                due
            })
            .collect()
    }
    #[test]
    fn a_monthly_repeat_returns_to_its_day_after_a_short_month() {
        let m = Repeat::new(Unit::Month, 1);
        assert_eq!(
            series(d(2026, 1, 31), m, 4),
            [d(2026, 2, 28), d(2026, 3, 31), d(2026, 4, 30), d(2026, 5, 31)]
        );
        assert_eq!(series(d(2026, 1, 30), m, 2), [d(2026, 2, 28), d(2026, 3, 30)]);
        // an overdue reminder catches up from its own day, not through the shortened months between
        assert_eq!(next_occurrence(d(2026, 1, 31), m, d(2026, 3, 5)).0, d(2026, 3, 31));
        // a day every month has needs nothing kept, and the stored rule is unchanged
        assert_eq!(next_occurrence(d(2026, 1, 15), m, d(2026, 1, 15)), (d(2026, 2, 15), m));
    }
    #[test]
    fn a_yearly_repeat_on_the_leap_day_returns_to_it() {
        let y = Repeat::new(Unit::Year, 1);
        assert_eq!(
            series(d(2028, 2, 29), y, 4),
            [d(2029, 2, 28), d(2030, 2, 28), d(2031, 2, 28), d(2032, 2, 29)]
        );
    }
    #[test]
    fn a_due_day_moved_by_hand_starts_a_new_series() {
        let (due, kept) = next_occurrence(d(2026, 1, 31), Repeat::new(Unit::Month, 1), d(2026, 1, 31));
        assert_eq!((due, kept.anchor), (d(2026, 2, 28), Some(31)));
        // the person moves it to the 10th: the kept 31st no longer applies and is dropped
        let (next, rule) = next_occurrence(d(2026, 2, 10), kept, d(2026, 2, 10));
        assert_eq!((next, rule.anchor), (d(2026, 3, 10), None));
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
        let kept = Repeat { anchor: Some(31), ..Repeat::new(Unit::Month, 1) };
        assert_eq!(kept.encode(), "m1@31");
        assert_eq!(Repeat::decode("m1@31"), Some(kept));
        assert_eq!(Repeat::decode("m1@28"), None);
        assert_eq!(Repeat::decode("m1@"), None);
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
