//! The Do Not Disturb schedule: a daily window in local minutes of the day
//! that may wrap midnight (22:00 to 07:00). Kept in the notification store.

/// Minutes in a day.
const DAY_MIN: u16 = 1440;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub enabled: bool,
    /// local minute of the day the window opens
    pub start: u16,
    /// local minute of the day it closes
    pub end: u16,
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule { enabled: false, start: 22 * 60, end: 7 * 60 }
    }
}

impl Schedule {
    /// Whether the window covers `minute` (0..1440). An empty window
    /// (start == end) never covers anything.
    pub fn active(&self, minute: u16) -> bool {
        if !self.enabled || self.start == self.end {
            return false;
        }
        let m = minute % DAY_MIN;
        if self.start < self.end {
            m >= self.start && m < self.end
        } else {
            m >= self.start || m < self.end
        }
    }

    /// The local minute of the day for Unix `unix` at `offset` seconds from UTC.
    pub fn minute_of_day(unix: i64, offset: i32) -> u16 {
        ((unix + offset as i64).rem_euclid(86_400) / 60) as u16
    }
}

/// "22:30" for minute 1350.
pub fn format_minutes(m: u16) -> String {
    format!("{:02}:{:02}", (m % DAY_MIN) / 60, m % 60)
}

/// Parses "HH:MM" (24 h) into a minute of the day.
pub fn parse_minutes(text: &str) -> Option<u16> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn night() -> Schedule {
        Schedule { enabled: true, start: 22 * 60, end: 7 * 60 }
    }

    #[test]
    fn a_window_across_midnight_covers_both_sides() {
        let s = night();
        assert!(s.active(23 * 60));
        assert!(s.active(0));
        assert!(s.active(6 * 60 + 59));
        assert!(!s.active(7 * 60), "the end is exclusive");
        assert!(!s.active(12 * 60));
        assert!(s.active(22 * 60), "the start is inclusive");
    }

    #[test]
    fn a_daytime_window_and_the_degenerate_cases() {
        let day = Schedule { enabled: true, start: 9 * 60, end: 17 * 60 };
        assert!(day.active(9 * 60) && day.active(16 * 60 + 59));
        assert!(!day.active(17 * 60) && !day.active(8 * 60));
        assert!(!Schedule { enabled: false, ..night() }.active(23 * 60), "off is off");
        assert!(!Schedule { enabled: true, start: 600, end: 600 }.active(600), "an empty window covers nothing");
    }

    #[test]
    fn minute_of_day_follows_the_local_offset() {
        assert_eq!(Schedule::minute_of_day(0, 0), 0);
        assert_eq!(Schedule::minute_of_day(3600 * 23, 3600), 0, "23:00 UTC is midnight at +1");
        assert_eq!(Schedule::minute_of_day(60 * 90, -3600 * 2), 23 * 60 + 30, "negative offsets wrap backwards");
    }

    #[test]
    fn times_round_trip_through_text() {
        assert_eq!(format_minutes(22 * 60 + 5), "22:05");
        assert_eq!(parse_minutes("22:05"), Some(22 * 60 + 5));
        assert_eq!(parse_minutes(" 07:00 "), Some(420));
        assert_eq!(parse_minutes("24:00"), None);
        assert_eq!(parse_minutes("7"), None);
        assert_eq!(parse_minutes("12:60"), None);
    }
}
