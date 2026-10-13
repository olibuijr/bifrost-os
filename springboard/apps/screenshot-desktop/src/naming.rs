//! File names for captures: `Screenshot <date> at <time>.png`, where the date is
//! `YYYY-MM-DD` and the time is `HH.MM.SS` (24 hour clock), both in local time.

/// A local date and time for naming a capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Stamp {
    /// The date part, `YYYY-MM-DD`, zero padded.
    pub fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// The time part, `HH.MM.SS`, zero padded.
    pub fn time(&self) -> String {
        format!("{:02}.{:02}.{:02}", self.hour, self.minute, self.second)
    }
}

/// The file name for a still capture taken at `at`.
pub fn file_name(at: &Stamp) -> String {
    format!("Screenshot {} at {}.png", at.date(), at.time())
}

/// The file name for a screen recording taken at `at`.
pub fn recording_name(at: &Stamp) -> String {
    format!("Screen Recording {} at {}.mov", at.date(), at.time())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(y: i32, mo: u8, d: u8, h: u8, mi: u8, s: u8) -> Stamp {
        Stamp { year: y, month: mo, day: d, hour: h, minute: mi, second: s }
    }

    #[test]
    fn still_name_uses_date_and_time() {
        let at = stamp(2026, 10, 13, 14, 5, 9);
        assert_eq!(file_name(&at), "Screenshot 2026-10-13 at 14.05.09.png");
    }

    #[test]
    fn fields_are_zero_padded() {
        let at = stamp(2027, 1, 2, 0, 3, 4);
        assert_eq!(file_name(&at), "Screenshot 2027-01-02 at 00.03.04.png");
    }

    #[test]
    fn recording_name_uses_movie_extension() {
        let at = stamp(2026, 12, 31, 23, 59, 58);
        assert_eq!(recording_name(&at), "Screen Recording 2026-12-31 at 23.59.58.mov");
    }

    #[test]
    fn names_for_same_second_are_equal() {
        let a = stamp(2026, 10, 13, 9, 0, 0);
        assert_eq!(file_name(&a), file_name(&a));
    }
}
