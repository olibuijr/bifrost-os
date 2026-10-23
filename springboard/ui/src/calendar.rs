//! Gregorian calendar arithmetic (`Calendar`, `DateComponents`) on days since 1970-01-01, with no time zone
//! and no allocation. Algorithms from Howard Hinnant's civil-date paper (proleptic Gregorian, exact).

use crate::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Date {
    pub year: i32,
    pub month: u32, // 1..=12
    pub day: u32,   // 1..=31
}

pub fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => if is_leap(year) { 29 } else { 28 },
    }
}

/// Days since 1970-01-01 (negative before).
pub fn to_days(d: Date) -> i64 {
    let y = d.year as i64 - (d.month <= 2) as i64;
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = d.month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d.day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn from_days(days: i64) -> Date {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (yoe + era * 400 + (month <= 2) as i64) as i32;
    Date { year, month, day }
}

/// 0 = Sunday .. 6 = Saturday.
pub fn weekday(d: Date) -> u32 {
    (to_days(d) + 4).rem_euclid(7) as u32
}

/// `Calendar.date(byAdding: .month)`: the day clamps to the end of a shorter month (Jan 31 + 1 = Feb 28/29).
pub fn add_months(d: Date, months: i32) -> Date {
    let total = d.year * 12 + (d.month as i32 - 1) + months;
    let (year, month) = (total.div_euclid(12), total.rem_euclid(12) as u32 + 1);
    Date { year, month, day: d.day.min(days_in_month(year, month)) }
}

/// A month grid like Calendar.app: rows of 7 optional days, weeks start on `first_weekday` (0 = Sunday, 1 = Monday).
pub fn month_grid(year: i32, month: u32, first_weekday: u32) -> Vec<[Option<u32>; 7]> {
    let lead = (weekday(Date { year, month, day: 1 }) + 7 - first_weekday % 7) % 7;
    let mut rows = Vec::new();
    let mut row = [None; 7];
    let mut col = lead as usize;
    for day in 1..=days_in_month(year, month) {
        row[col] = Some(day);
        col += 1;
        if col == 7 {
            rows.push(row);
            row = [None; 7];
            col = 0;
        }
    }
    if col > 0 {
        rows.push(row);
    }
    rows
}

/// Where the parts of a mini month calendar go (`NativeDatePicker`, `.clockAndCalendar` graphical style, as in the
/// Calendar and Reminders sidebars): a title row with previous/next buttons, a row of weekday initials, then six
/// rows of seven day cells (always six, so the height does not jump between months).
#[derive(Clone, Debug, PartialEq)]
pub struct MiniMonth {
    pub title: Rect,
    pub prev: Rect,
    pub next: Rect,
    /// The seven weekday initial cells, in display order.
    pub weekdays: [Rect; 7],
    /// The weekday (0 = Sunday) shown in each column.
    pub weekday_of_column: [u32; 7],
    /// `(day, cell)` for every day of the month.
    pub days: Vec<(u32, Rect)>,
    /// The previous month's last days before the 1st and the next month's first days after the last, filling the six
    /// rows (`NSDatePicker` shows them greyed, and a click on one goes to that day): `(date, cell)` in display order.
    pub outside: Vec<(Date, Rect)>,
}

impl MiniMonth {
    /// Lays `year`/`month` out in `bounds` with weeks starting on `first_weekday` (0 = Sunday, 1 = Monday).
    pub fn new(bounds: Rect, year: i32, month: u32, first_weekday: u32) -> Self {
        let cell_w = bounds.w / 7.0;
        let row_h = bounds.h / 8.0;
        let title = Rect { x: bounds.x, y: bounds.y, w: bounds.w - 2.0 * cell_w, h: row_h };
        let prev = Rect { x: bounds.right() - 2.0 * cell_w, y: bounds.y, w: cell_w, h: row_h };
        let next = Rect { x: bounds.right() - cell_w, y: bounds.y, w: cell_w, h: row_h };
        let cell = |row: usize, col: usize| Rect { x: bounds.x + col as f32 * cell_w, y: bounds.y + row as f32 * row_h, w: cell_w, h: row_h };
        let weekdays = std::array::from_fn(|c| cell(1, c));
        let weekday_of_column = std::array::from_fn(|c| (first_weekday + c as u32) % 7);
        let mut days = Vec::new();
        for (r, week) in month_grid(year, month, first_weekday).iter().enumerate() {
            for (c, d) in week.iter().enumerate() {
                if let Some(d) = d {
                    days.push((*d, cell(2 + r, c)));
                }
            }
        }
        // the cells before the 1st and after the last day, through the sixth row
        let first = Date { year, month, day: 1 };
        let lead = ((weekday(first) + 7 - first_weekday % 7) % 7) as usize;
        let len = days_in_month(year, month) as usize;
        let before = add_months(first, -1);
        let before_len = days_in_month(before.year, before.month) as usize;
        let after = add_months(first, 1);
        let mut outside = Vec::new();
        for i in (0..lead).chain(lead + len..42) {
            let date = if i < lead {
                Date { day: (before_len - lead + 1 + i) as u32, ..before }
            } else {
                Date { day: (i - lead - len + 1) as u32, ..after }
            };
            outside.push((date, cell(2 + i / 7, i % 7)));
        }
        MiniMonth { title, prev, next, weekdays, weekday_of_column, days, outside }
    }
    /// The day under `(x, y)`.
    pub fn hit_day(&self, x: f32, y: f32) -> Option<u32> {
        self.days.iter().find(|(_, r)| r.contains(x, y)).map(|(d, _)| *d)
    }
    /// The date under `(x, y)`: a day of the month, or a greyed day of the month before or after it.
    pub fn hit_date(&self, x: f32, y: f32, year: i32, month: u32) -> Option<Date> {
        if let Some(day) = self.hit_day(x, y) {
            return Some(Date { year, month, day });
        }
        self.outside.iter().find(|(_, r)| r.contains(x, y)).map(|(d, _)| *d)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_mini_month_has_a_title_row_weekdays_and_day_cells() {
        let b = crate::Rect { x: 0.0, y: 0.0, w: 210.0, h: 240.0 };
        let m = MiniMonth::new(b, 2026, 10, 1);
        assert_eq!(m.days.len(), 31);
        assert_eq!(m.weekday_of_column[0], 1, "weeks start on Monday");
        assert_eq!(m.weekdays[0].y, 30.0);
        // 1 October 2026 is a Thursday: column 3 when weeks start on Monday
        assert_eq!(m.days[0].1, crate::Rect { x: 90.0, y: 60.0, w: 30.0, h: 30.0 });
        assert_eq!(m.hit_day(95.0, 65.0), Some(1));
        assert_eq!(m.hit_day(5.0, 65.0), None, "before the first");
        assert!(m.next.right() <= 210.0 && m.prev.right() <= m.next.x);
    }
    #[test]
    fn the_days_of_the_months_around_fill_the_six_rows() {
        let b = crate::Rect { x: 0.0, y: 0.0, w: 210.0, h: 240.0 };
        // September 2026 with weeks from Sunday (the reference): 30 and 31 August lead, 1 to 10 October trail
        let m = MiniMonth::new(b, 2026, 9, 0);
        assert_eq!(m.days.len() + m.outside.len(), 42, "six full rows");
        assert_eq!(m.outside[0], (Date { year: 2026, month: 8, day: 30 }, crate::Rect { x: 0.0, y: 60.0, w: 30.0, h: 30.0 }));
        assert_eq!(m.outside[1].0, Date { year: 2026, month: 8, day: 31 });
        assert_eq!(m.outside[2].0, Date { year: 2026, month: 10, day: 1 });
        assert_eq!(m.outside.last().unwrap().0, Date { year: 2026, month: 10, day: 10 });
        assert_eq!(m.outside.last().unwrap().1, crate::Rect { x: 180.0, y: 210.0, w: 30.0, h: 30.0 }, "the last cell");
        assert_eq!(m.hit_date(5.0, 65.0, 2026, 9), Some(Date { year: 2026, month: 8, day: 30 }));
        assert_eq!(m.hit_date(65.0, 65.0, 2026, 9), Some(Date { year: 2026, month: 9, day: 1 }));
        assert_eq!(m.hit_date(185.0, 215.0, 2026, 9), Some(Date { year: 2026, month: 10, day: 10 }));
        // across a year: January leads with December of the year before
        let j = MiniMonth::new(b, 2027, 1, 1);
        assert!(j.outside.first().is_some_and(|(d, _)| d.year == 2026 && d.month == 12));
        assert!(j.outside.last().is_some_and(|(d, _)| d.year == 2027 && d.month == 2));
        // a month that starts on the first column has no leading days
        let feb = MiniMonth::new(b, 2026, 2, 0);
        assert_eq!(feb.outside[0].0, Date { year: 2026, month: 3, day: 1 });
        assert_eq!(feb.outside.len(), 14);
    }
    use super::*;
    fn d(year: i32, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }
    #[test]
    fn epoch_and_round_trip() {
        assert_eq!(to_days(d(1970, 1, 1)), 0);
        assert_eq!(to_days(d(2000, 3, 1)), 11_017);
        for n in (-800_000..800_000).step_by(997) {
            assert_eq!(to_days(from_days(n)), n);
        }
    }
    #[test]
    fn leap_years() {
        assert!(is_leap(2000) && is_leap(2024) && !is_leap(1900) && !is_leap(2023));
        assert_eq!(days_in_month(2024, 2), 29);
    }
    #[test]
    fn weekdays() {
        assert_eq!(weekday(d(1970, 1, 1)), 4); // Thursday
        assert_eq!(weekday(d(2024, 2, 29)), 4);
        assert_eq!(weekday(d(2026, 10, 9)), 5); // Friday
    }
    #[test]
    fn adding_months_clamps() {
        assert_eq!(add_months(d(2024, 1, 31), 1), d(2024, 2, 29));
        assert_eq!(add_months(d(2023, 12, 15), 2), d(2024, 2, 15));
        assert_eq!(add_months(d(2024, 3, 31), -1), d(2024, 2, 29));
        assert_eq!(add_months(d(2024, 1, 1), -13), d(2022, 12, 1));
    }
    #[test]
    fn grid_covers_every_day_once() {
        let g = month_grid(2026, 10, 1); // Oct 2026 starts on a Thursday; Monday-first
        assert_eq!(g[0][3], Some(1));
        let days: Vec<u32> = g.iter().flatten().flatten().copied().collect();
        assert_eq!(days, (1..=31).collect::<Vec<_>>());
        assert_eq!(g.len(), 5);
    }
}
