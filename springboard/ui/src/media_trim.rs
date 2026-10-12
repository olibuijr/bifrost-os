//! Trimming session for a media player view as a pure model. It mirrors the player view's trim mode
//! (`canBeginTrimming`, `beginTrimming(completionHandler:)` and the two trim results: OK and Cancel). The model
//! holds the in-point and out-point in seconds, keeps them inside the media and keeps a minimum clip length.
//! Drawing the trim bar and writing the trimmed file are left to the caller.

/// The action the user took to finish trimming (`okButton` and `cancelButton`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrimResult {
    /// The user confirmed the trim; the range is kept.
    Ok,
    /// The user cancelled; nothing is trimmed.
    Cancel,
}

/// A trim session over media of `duration` seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct TrimSession {
    duration: f64,
    min_length: f64,
    trimming: bool,
    start: f64,
    end: f64,
}

impl TrimSession {
    /// A session for media of `duration` seconds. Negative or NaN durations count as zero; a negative or NaN
    /// minimum length counts as zero.
    pub fn new(duration: f64, min_length: f64) -> Self {
        let duration = if duration.is_finite() && duration > 0.0 { duration } else { 0.0 };
        let min_length = if min_length.is_finite() && min_length > 0.0 { min_length } else { 0.0 };
        TrimSession { duration, min_length, trimming: false, start: 0.0, end: duration }
    }

    /// True when the media is long enough to trim and the session is not already trimming.
    pub fn can_begin_trimming(&self) -> bool {
        !self.trimming && self.duration > 0.0 && self.duration >= self.min_length
    }

    /// Enters trimming mode with the whole media selected. False when it cannot begin.
    pub fn begin_trimming(&mut self) -> bool {
        if !self.can_begin_trimming() {
            return false;
        }
        self.trimming = true;
        self.start = 0.0;
        self.end = self.duration;
        true
    }

    pub fn is_trimming(&self) -> bool {
        self.trimming
    }

    /// Moves the in-point. It is clamped to `[0, end - min_length]` and returns the value that was set.
    /// Outside trimming mode the call does nothing and returns the current in-point.
    pub fn set_start(&mut self, seconds: f64) -> f64 {
        if !self.trimming {
            return self.start;
        }
        let upper = (self.end - self.min_length).max(0.0);
        self.start = clamp_to(seconds, 0.0, upper);
        self.start
    }

    /// Moves the out-point. It is clamped to `[start + min_length, duration]` and returns the value that was set.
    /// Outside trimming mode the call does nothing and returns the current out-point.
    pub fn set_end(&mut self, seconds: f64) -> f64 {
        if !self.trimming {
            return self.end;
        }
        let lower = (self.start + self.min_length).min(self.duration);
        self.end = clamp_to(seconds, lower, self.duration);
        self.end
    }

    /// The selected in-point and out-point in seconds.
    pub fn selection(&self) -> (f64, f64) {
        (self.start, self.end)
    }

    /// Length of the selected range in seconds.
    pub fn trimmed_duration(&self) -> f64 {
        self.end - self.start
    }

    /// Ends trimming. `Ok` returns the selected range; `Cancel` and a session that was not trimming return `None`.
    /// The session leaves trimming mode either way.
    pub fn finish(&mut self, result: TrimResult) -> Option<(f64, f64)> {
        if !self.trimming {
            return None;
        }
        self.trimming = false;
        match result {
            TrimResult::Ok => Some((self.start, self.end)),
            TrimResult::Cancel => None,
        }
    }
}

fn clamp_to(v: f64, lo: f64, hi: f64) -> f64 {
    if !v.is_finite() {
        return lo;
    }
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_enough_media_can_begin_and_selects_everything() {
        let mut s = TrimSession::new(120.0, 2.0);
        assert!(s.can_begin_trimming());
        assert!(s.begin_trimming());
        assert!(s.is_trimming());
        assert_eq!(s.selection(), (0.0, 120.0));
        assert_eq!(s.trimmed_duration(), 120.0);
        // Already trimming: cannot begin again.
        assert!(!s.can_begin_trimming());
        assert!(!s.begin_trimming());
    }

    #[test]
    fn media_shorter_than_the_minimum_cannot_be_trimmed() {
        let mut s = TrimSession::new(1.5, 2.0);
        assert!(!s.can_begin_trimming());
        assert!(!s.begin_trimming());
        assert!(!s.is_trimming());
        // Zero-length media never begins either.
        assert!(!TrimSession::new(0.0, 0.0).can_begin_trimming());
    }

    #[test]
    fn in_point_is_clamped_to_leave_the_minimum_length() {
        let mut s = TrimSession::new(100.0, 5.0);
        s.begin_trimming();
        // The out-point is at 100; the in-point may go up to 95.
        assert_eq!(s.set_start(98.0), 95.0);
        assert_eq!(s.set_start(-3.0), 0.0);
        assert_eq!(s.set_start(40.0), 40.0);
        assert_eq!(s.selection(), (40.0, 100.0));
    }

    #[test]
    fn out_point_is_clamped_to_leave_the_minimum_length() {
        let mut s = TrimSession::new(100.0, 5.0);
        s.begin_trimming();
        s.set_start(20.0);
        // The out-point may not go below 25 nor above the duration.
        assert_eq!(s.set_end(10.0), 25.0);
        assert_eq!(s.set_end(250.0), 100.0);
        assert_eq!(s.set_end(60.0), 60.0);
        assert_eq!(s.trimmed_duration(), 40.0);
    }

    #[test]
    fn ok_returns_the_range_and_cancel_returns_nothing() {
        let mut s = TrimSession::new(60.0, 1.0);
        s.begin_trimming();
        s.set_start(10.0);
        s.set_end(30.0);
        assert_eq!(s.finish(TrimResult::Ok), Some((10.0, 30.0)));
        assert!(!s.is_trimming());
        // A new session that is cancelled gives no range.
        assert!(s.begin_trimming());
        s.set_start(5.0);
        assert_eq!(s.finish(TrimResult::Cancel), None);
        assert!(!s.is_trimming());
        // Finishing when not trimming gives nothing.
        assert_eq!(s.finish(TrimResult::Ok), None);
    }

    #[test]
    fn edits_outside_trimming_mode_are_ignored() {
        let mut s = TrimSession::new(50.0, 1.0);
        assert_eq!(s.set_start(12.0), 0.0);
        assert_eq!(s.set_end(12.0), 50.0);
        assert_eq!(s.selection(), (0.0, 50.0));
    }

    #[test]
    fn bad_numbers_are_sanitised() {
        let s = TrimSession::new(f64::NAN, -4.0);
        assert_eq!(s.selection(), (0.0, 0.0));
        assert!(!s.can_begin_trimming());
        let mut t = TrimSession::new(30.0, 0.0);
        t.begin_trimming();
        assert_eq!(t.set_start(f64::NAN), 0.0);
        // A non-finite out-point falls back to the lower bound of the out-point, which is the in-point here (0).
        assert_eq!(t.set_end(f64::INFINITY), 0.0);
        assert_eq!(t.set_end(30.0), 30.0);
    }
}
