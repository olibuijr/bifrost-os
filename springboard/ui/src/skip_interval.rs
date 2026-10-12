//! Interval skips: a skip forward or backward by a preferred time interval, clamped to the item.
//! Mirrors `MediaCommand.skipForward(preferredIntervals:_:)` and `skipBackward(preferredIntervals:_:)`. The
//! preferred intervals are read here as a list in preference order: the first interval is the one a skip uses.
//! The clamping rule is this module's own: a forward skip stops at a known end, a backward skip stops at zero,
//! and an item with no known end (live) is not clamped on the forward side. The official page does not state
//! these rules. Pure arithmetic: no seeking is done here.

/// Which way a skip moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

/// The cases `SkipCommand::new` rejects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipError {
    /// No preferred interval was supplied.
    NoIntervals,
    /// An interval is not a positive, finite number of seconds.
    InvalidInterval,
}

/// A skip command with its preferred intervals in seconds, in preference order.
#[derive(Clone, Debug, PartialEq)]
pub struct SkipCommand {
    direction: Direction,
    intervals: Vec<f64>,
}

impl SkipCommand {
    /// A skip in `direction` with the preferred intervals, in seconds, first choice first.
    pub fn new(direction: Direction, preferred: &[f64]) -> Result<Self, SkipError> {
        if preferred.is_empty() {
            return Err(SkipError::NoIntervals);
        }
        if preferred.iter().any(|s| !s.is_finite() || *s <= 0.0) {
            return Err(SkipError::InvalidInterval);
        }
        Ok(SkipCommand {
            direction,
            intervals: preferred.to_vec(),
        })
    }

    pub fn direction(&self) -> Direction {
        self.direction
    }

    /// The preferred intervals as given, first choice first.
    pub fn intervals(&self) -> &[f64] {
        &self.intervals
    }

    /// The interval a skip uses: the first preferred one.
    pub fn interval(&self) -> f64 {
        self.intervals.first().copied().unwrap_or(0.0)
    }

    /// The position after the skip. `position` and `duration` are in seconds. A NaN or negative position counts as
    /// zero. A duration of `None` means the item has no known end.
    pub fn target(&self, position: f64, duration: Option<f64>) -> f64 {
        let position = if position.is_finite() { position.max(0.0) } else { 0.0 };
        match self.direction {
            Direction::Forward => {
                let next = position + self.interval();
                match duration {
                    Some(end) if end.is_finite() && end >= 0.0 => next.min(end),
                    _ => next,
                }
            }
            Direction::Backward => (position - self.interval()).max(0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_empty_zero_negative_and_non_finite_intervals() {
        assert_eq!(SkipCommand::new(Direction::Forward, &[]), Err(SkipError::NoIntervals));
        assert_eq!(SkipCommand::new(Direction::Forward, &[15.0, 0.0]), Err(SkipError::InvalidInterval));
        assert_eq!(SkipCommand::new(Direction::Backward, &[-5.0]), Err(SkipError::InvalidInterval));
        assert_eq!(SkipCommand::new(Direction::Backward, &[f64::NAN]), Err(SkipError::InvalidInterval));
        assert!(SkipCommand::new(Direction::Forward, &[15.0, 30.0]).is_ok());
    }

    #[test]
    fn the_first_preferred_interval_is_the_one_used() {
        let cmd = SkipCommand::new(Direction::Forward, &[30.0, 15.0]).unwrap();
        assert_eq!(cmd.interval(), 30.0);
        assert_eq!(cmd.intervals(), &[30.0, 15.0]);
        assert_eq!(cmd.direction(), Direction::Forward);
    }

    #[test]
    fn a_forward_skip_moves_by_the_interval() {
        let cmd = SkipCommand::new(Direction::Forward, &[15.0]).unwrap();
        assert_eq!(cmd.target(100.0, Some(200.0)), 115.0);
    }

    #[test]
    fn a_forward_skip_stops_at_the_end_of_the_item() {
        let cmd = SkipCommand::new(Direction::Forward, &[15.0]).unwrap();
        assert_eq!(cmd.target(190.0, Some(200.0)), 200.0);
    }

    #[test]
    fn a_backward_skip_stops_at_zero() {
        let cmd = SkipCommand::new(Direction::Backward, &[15.0]).unwrap();
        assert_eq!(cmd.target(100.0, Some(200.0)), 85.0);
        assert_eq!(cmd.target(10.0, Some(200.0)), 0.0);
    }

    #[test]
    fn a_forward_skip_on_live_content_has_no_end_to_clamp_to() {
        let cmd = SkipCommand::new(Direction::Forward, &[30.0]).unwrap();
        assert_eq!(cmd.target(100.0, None), 130.0);
    }

    #[test]
    fn a_nan_position_counts_as_zero() {
        let back = SkipCommand::new(Direction::Backward, &[15.0]).unwrap();
        let fwd = SkipCommand::new(Direction::Forward, &[15.0]).unwrap();
        assert_eq!(back.target(f64::NAN, Some(60.0)), 0.0);
        assert_eq!(fwd.target(f64::NAN, Some(60.0)), 15.0);
    }
}
