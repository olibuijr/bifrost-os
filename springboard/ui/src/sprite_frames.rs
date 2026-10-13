//! Frame sequences: a timed list of frame names, such as the cells of a walk cycle. Each name shows from its start time
//! until the next one starts, and the last one shows until the sequence's duration ends. The sequence is pure: the
//! caller passes the time and gets the name to show.
//!
//! Rules: a sequence has at least one frame. Start times are finite, not negative, and do not decrease. The duration
//! is finite and greater than the last start time. Looping wraps the time into the duration; without looping the
//! sequence holds its last frame after the duration ends.

/// One frame of a sequence: the frame name and the time, in seconds, when it starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub name: String,
    pub start: f64,
}

/// Why a sequence was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SequenceError {
    /// The sequence has no frames.
    Empty,
    /// A start time is NaN, infinite or negative.
    BadStart,
    /// The start times go backwards.
    OutOfOrder,
    /// The duration is not finite or does not lie after the last start.
    BadDuration,
}

/// A timed sequence of frame names.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameSequence {
    frames: Vec<Keyframe>,
    duration: f64,
}

impl FrameSequence {
    /// Builds a sequence from (name, start time) pairs and the total duration in seconds.
    pub fn new(frames: Vec<(String, f64)>, duration: f64) -> Result<Self, SequenceError> {
        if frames.is_empty() {
            return Err(SequenceError::Empty);
        }
        let mut previous = 0.0_f64;
        let mut keys = Vec::with_capacity(frames.len());
        for (i, (name, start)) in frames.into_iter().enumerate() {
            if !start.is_finite() || start < 0.0 {
                return Err(SequenceError::BadStart);
            }
            if i > 0 && start < previous {
                return Err(SequenceError::OutOfOrder);
            }
            previous = start;
            keys.push(Keyframe { name, start });
        }
        if !duration.is_finite() || duration <= previous {
            return Err(SequenceError::BadDuration);
        }
        Ok(Self {
            frames: keys,
            duration,
        })
    }

    /// The total duration in seconds.
    pub fn duration(&self) -> f64 {
        self.duration
    }

    /// The keyframes in time order.
    pub fn keyframes(&self) -> &[Keyframe] {
        &self.frames
    }

    /// The frame name shown at `time` seconds after the sequence started. With `looping` the time wraps into the
    /// duration; without it, times past the end show the last frame. A NaN time shows the first frame.
    pub fn frame_at(&self, time: f64, looping: bool) -> &str {
        let t = if !time.is_finite() {
            0.0
        } else if looping {
            time.rem_euclid(self.duration)
        } else {
            time.clamp(0.0, self.duration)
        };
        // The last keyframe whose start is at or before `t`. Starts are sorted, so this is a partition point.
        let index = self.frames.partition_point(|k| k.start <= t);
        let index = index.saturating_sub(1);
        &self.frames[index].name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk() -> FrameSequence {
        FrameSequence::new(
            vec![
                ("walk-0".to_string(), 0.0),
                ("walk-1".to_string(), 0.1),
                ("walk-2".to_string(), 0.2),
            ],
            0.4,
        )
        .unwrap()
    }

    #[test]
    fn a_frame_shows_from_its_start_until_the_next_one() {
        let s = walk();
        assert_eq!(s.frame_at(0.0, false), "walk-0");
        assert_eq!(s.frame_at(0.099, false), "walk-0");
        assert_eq!(s.frame_at(0.1, false), "walk-1");
        assert_eq!(s.frame_at(0.35, false), "walk-2");
    }

    #[test]
    fn looping_wraps_time_into_the_duration() {
        let s = walk();
        assert_eq!(s.frame_at(0.45, true), "walk-0");
        assert_eq!(s.frame_at(1.25, true), "walk-2");
    }

    #[test]
    fn without_looping_the_last_frame_holds() {
        let s = walk();
        assert_eq!(s.frame_at(9.0, false), "walk-2");
        assert_eq!(s.frame_at(-1.0, false), "walk-0");
    }

    #[test]
    fn empty_sequences_and_bad_times_are_refused() {
        assert_eq!(FrameSequence::new(vec![], 1.0), Err(SequenceError::Empty));
        assert_eq!(
            FrameSequence::new(vec![("a".into(), -0.1)], 1.0),
            Err(SequenceError::BadStart)
        );
        assert_eq!(
            FrameSequence::new(vec![("a".into(), 0.5), ("b".into(), 0.2)], 1.0),
            Err(SequenceError::OutOfOrder)
        );
        assert_eq!(
            FrameSequence::new(vec![("a".into(), 0.5)], 0.5),
            Err(SequenceError::BadDuration)
        );
        assert_eq!(
            FrameSequence::new(vec![("a".into(), 0.0)], f64::INFINITY),
            Err(SequenceError::BadDuration)
        );
    }

    #[test]
    fn a_nan_time_shows_the_first_frame() {
        assert_eq!(walk().frame_at(f64::NAN, true), "walk-0");
    }
}
