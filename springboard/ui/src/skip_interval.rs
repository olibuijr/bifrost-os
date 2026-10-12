//! Skip forward and backward by an interval, as a pure model of the now-playing skip command. An app offers a list of
//! preferred skip intervals in seconds. A skip moves the playback position by one of them and stays inside the
//! item: it never goes before the start, and it stops at the end when the item has a known duration. A live
//! stream has no duration, so a forward skip has no upper bound.
//!
//! The first usable interval of the list is the default that a control without a choice uses. Intervals that are
//! not finite, not positive, or repeated are dropped; the order of the rest is kept.

/// Which way a skip moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

/// The usable skip intervals of `preferred`, in the given order, without repeats.
pub fn intervals(preferred: &[f64]) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    for &seconds in preferred {
        if seconds.is_finite() && seconds > 0.0 && !out.contains(&seconds) {
            out.push(seconds);
        }
    }
    out
}

/// The interval a control uses when the person has not chosen one: the first usable one.
pub fn default_interval(preferred: &[f64]) -> Option<f64> {
    intervals(preferred).first().copied()
}

/// The playback position after a skip of `seconds` from `position`. A position that is not finite counts as 0. The
/// result is at least 0, and at most `duration` when the item has one (a duration that is not finite or is negative
/// is treated as unknown). A non-usable `seconds` leaves the position where it was, clamped into range.
pub fn target(position: f64, duration: Option<f64>, direction: Direction, seconds: f64) -> f64 {
    let start = if position.is_finite() { position.max(0.0) } else { 0.0 };
    let step = if seconds.is_finite() && seconds > 0.0 { seconds } else { 0.0 };
    let moved = match direction {
        Direction::Forward => start + step,
        Direction::Backward => start - step,
    };
    let upper = duration.filter(|d| d.is_finite() && *d >= 0.0);
    let bounded = match upper {
        Some(end) => moved.min(end),
        None => moved,
    };
    bounded.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usable_intervals_keep_their_order_without_repeats() {
        let preferred = [30.0, 0.0, -5.0, 15.0, 30.0, f64::NAN, 10.0, f64::INFINITY];
        assert_eq!(intervals(&preferred), vec![30.0, 15.0, 10.0]);
    }

    #[test]
    fn the_default_is_the_first_usable_interval() {
        assert_eq!(default_interval(&[0.0, 45.0, 15.0]), Some(45.0));
        assert_eq!(default_interval(&[]), None);
        assert_eq!(default_interval(&[-1.0]), None);
    }

    #[test]
    fn a_forward_skip_moves_by_the_interval_inside_the_item() {
        assert_eq!(target(150.0, Some(300.0), Direction::Forward, 30.0), 180.0);
    }

    #[test]
    fn a_forward_skip_stops_at_the_end_of_the_item() {
        assert_eq!(target(290.0, Some(300.0), Direction::Forward, 30.0), 300.0);
    }

    #[test]
    fn a_backward_skip_stops_at_the_start() {
        assert_eq!(target(10.0, Some(300.0), Direction::Backward, 30.0), 0.0);
        assert_eq!(target(100.0, Some(300.0), Direction::Backward, 30.0), 70.0);
    }

    #[test]
    fn a_live_stream_has_no_upper_bound_for_a_forward_skip() {
        assert_eq!(target(100.0, None, Direction::Forward, 30.0), 130.0);
    }

    #[test]
    fn a_position_that_is_not_a_number_counts_as_the_start() {
        assert_eq!(target(f64::NAN, Some(60.0), Direction::Forward, 15.0), 15.0);
        assert_eq!(target(f64::NAN, None, Direction::Backward, 15.0), 0.0);
    }
}
