//! Calendar data shared with the shell (lock widgets, Today): dates and the event store.
//! The Calendar app itself is the package in apps/calendar-desktop.
pub mod store;

use super::clock::tz;

pub const MONTHS: [&str; 12] = ["January","February","March","April","May","June","July","August","September","October","November","December"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i64,
    pub month: u32,
    pub day: u32,
}

impl Date {
    pub fn valid(self) -> bool {
        (1..=9999).contains(&self.year)
            && (1..=12).contains(&self.month)
            && self.day >= 1
            && self.day <= self.month_len()
    }

    pub fn month_len(self) -> u32 {
        if (1..=12).contains(&self.month) {
            tz::days_in_month(self.year, self.month)
        } else {
            0
        }
    }

    pub fn from_days(days: i64) -> Self {
        let (year, month, day) = tz::civil_from_days(days);
        Self { year, month, day }
    }

    pub fn days(self) -> i64 {
        tz::days_from_civil(self.year, self.month, self.day)
    }

    pub fn shift_days(self, by: i32) -> Self {
        Self::from_days((self.days() + by as i64).clamp(
            tz::days_from_civil(1, 1, 1),
            tz::days_from_civil(9999, 12, 31),
        ))
    }

    /// Month changes preserve the selected day, clamped to the new month.
    pub fn shift_months(self, by: i32) -> Self {
        let index = (self.year * 12 + self.month as i64 - 1 + by as i64).clamp(12, 9999 * 12 + 11);
        let mut next = Self {
            year: index / 12,
            month: (index % 12) as u32 + 1,
            day: self.day,
        };
        next.day = next.day.min(next.month_len());
        next
    }

    pub fn weekday(self) -> usize {
        (self.days() + 4).rem_euclid(7) as usize
    }

    pub fn label(self) -> String {
        format!(
            "{} {}, {}",
            MONTHS[(self.month - 1) as usize],
            self.day,
            self.year
        )
    }
    pub fn iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

