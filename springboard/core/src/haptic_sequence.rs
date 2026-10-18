//! Haptic sequences: an app authors a timed list of pulses (a tap, then a firmer click 120 ms later) and the shell plays
//! it. A [`Sequence`] is pure data: steps sorted by time, each a [`Pattern`] at an offset from the start. A [`Cursor`]
//! tracks how far a sequence has played, and hands out the steps that have come due, so the caller can feed each one to
//! [`crate::haptic_request::Requests`], which still applies the per-app throttle.

use crate::force_effect::{pulse_effect, Pattern};

/// The most steps one sequence may hold.
pub const MAX_STEPS: usize = 64;

/// One pulse of a sequence: when it fires and what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// Offset from the start of the sequence, in milliseconds.
    pub at_ms: u32,
    pub pattern: Pattern,
}

/// Why a step was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The step would start before the last one; steps are added in time order.
    OutOfOrder,
    /// The sequence already holds [`MAX_STEPS`] steps.
    TooLong,
}

/// A timed list of pulses, in time order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sequence {
    steps: Vec<Step>,
}

impl Sequence {
    pub fn new() -> Self {
        Sequence::default()
    }

    /// Appends a pulse at `at_ms`. It must not start before the last step; a step at the same time as the last one is
    /// allowed and plays in the order it was added.
    pub fn push(&mut self, at_ms: u32, pattern: Pattern) -> Result<(), Error> {
        if self.steps.len() >= MAX_STEPS {
            return Err(Error::TooLong);
        }
        if self.steps.last().is_some_and(|last| at_ms < last.at_ms) {
            return Err(Error::OutOfOrder);
        }
        self.steps.push(Step { at_ms, pattern });
        Ok(())
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// When the last pulse has finished, in milliseconds from the start; 0 for an empty sequence.
    pub fn duration_ms(&self) -> u64 {
        self.steps
            .iter()
            .map(|s| u64::from(s.at_ms) + u64::from(pulse_effect(s.pattern).duration_ms))
            .max()
            .unwrap_or(0)
    }
}

/// How far one playback of a sequence has got.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    next: usize,
}

impl Cursor {
    pub fn new() -> Self {
        Cursor::default()
    }

    /// The steps that are due at `elapsed_ms` and were not handed out before, in order. Each step is returned once;
    /// call again with a later time for the next ones.
    pub fn due<'a>(&mut self, sequence: &'a Sequence, elapsed_ms: u64) -> &'a [Step] {
        let steps = sequence.steps();
        let end = steps.partition_point(|s| u64::from(s.at_ms) <= elapsed_ms).max(self.next);
        let due = &steps[self.next..end];
        self.next = end;
        due
    }

    /// True once every step has been handed out.
    pub fn finished(&self, sequence: &Sequence) -> bool {
        self.next >= sequence.steps().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_must_come_in_time_order() {
        let mut s = Sequence::new();
        s.push(0, Pattern::Generic).unwrap();
        s.push(0, Pattern::Alignment).unwrap();
        assert_eq!(s.push(10, Pattern::LevelChange), Ok(()));
        assert_eq!(s.push(9, Pattern::Generic), Err(Error::OutOfOrder));
        assert_eq!(s.steps().len(), 3);
    }

    #[test]
    fn a_sequence_holds_at_most_max_steps() {
        let mut s = Sequence::new();
        for i in 0..MAX_STEPS as u32 {
            s.push(i, Pattern::Generic).unwrap();
        }
        assert_eq!(s.push(MAX_STEPS as u32, Pattern::Generic), Err(Error::TooLong));
    }

    #[test]
    fn duration_runs_to_the_end_of_the_last_pulse() {
        let mut s = Sequence::new();
        assert_eq!(s.duration_ms(), 0);
        s.push(0, Pattern::Generic).unwrap();
        s.push(100, Pattern::LevelChange).unwrap();
        let level = u64::from(pulse_effect(Pattern::LevelChange).duration_ms);
        assert_eq!(s.duration_ms(), 100 + level);
    }

    #[test]
    fn a_cursor_hands_out_each_due_step_once_in_order() {
        let mut s = Sequence::new();
        s.push(0, Pattern::Generic).unwrap();
        s.push(50, Pattern::Alignment).unwrap();
        s.push(50, Pattern::LevelChange).unwrap();
        s.push(200, Pattern::Generic).unwrap();
        let mut c = Cursor::new();
        assert_eq!(c.due(&s, 0).len(), 1);
        assert!(c.due(&s, 10).is_empty());
        let at_50 = c.due(&s, 50);
        assert_eq!(at_50.iter().map(|x| x.pattern).collect::<Vec<_>>(), vec![Pattern::Alignment, Pattern::LevelChange]);
        assert!(!c.finished(&s));
        assert_eq!(c.due(&s, 10_000).len(), 1);
        assert!(c.finished(&s));
        assert!(c.due(&s, 10_000).is_empty());
    }

    #[test]
    fn an_empty_sequence_is_finished_at_once() {
        assert!(Cursor::new().finished(&Sequence::new()));
    }
}
