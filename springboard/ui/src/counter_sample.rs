//! A fixed-size buffer of timestamped counter samples, as a pure model for measuring work between two points.
//!
//! The buffer holds `sample_count` slots. A sample is written into a slot by index with a value in nanoseconds, or
//! any other monotonically counted unit. Reading takes a range of slots and returns the values, or the intervals
//! between consecutive values. A read is refused as a whole when a slot in it was never written or when the values
//! go backwards, so a caller never gets a partial or negative duration. The model reads no clock and holds no device.

use std::ops::Range;

/// Why a call is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterError {
    /// A slot index is past the end of the buffer.
    OutOfRange { index: usize, count: usize },
    /// A range runs past the end of the buffer, or starts after it ends.
    BadRange { start: usize, end: usize, count: usize },
    /// A slot in the range has never been written.
    Unwritten { index: usize },
    /// A later sample holds a smaller value than the one before it.
    Backwards { index: usize },
}

/// A labelled buffer of `sample_count` slots. Slots start unwritten.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CounterSampleBuffer {
    label: String,
    samples: Vec<Option<u64>>,
}

impl CounterSampleBuffer {
    /// An empty buffer of `sample_count` slots with a label that names it.
    pub fn new(label: &str, sample_count: usize) -> Self {
        CounterSampleBuffer { label: label.to_string(), samples: vec![None; sample_count] }
    }

    /// The label given at creation.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The number of slots.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Writes `value` into slot `index`, replacing an earlier sample in that slot.
    pub fn record(&mut self, index: usize, value: u64) -> Result<(), CounterError> {
        let count = self.samples.len();
        let slot = self.samples.get_mut(index).ok_or(CounterError::OutOfRange { index, count })?;
        *slot = Some(value);
        Ok(())
    }

    /// The values in `range`, in slot order. Refused when the range is bad or holds an unwritten slot.
    pub fn resolve(&self, range: Range<usize>) -> Result<Vec<u64>, CounterError> {
        self.check_range(&range)?;
        let mut out = Vec::with_capacity(range.len());
        for index in range {
            match self.samples[index] {
                Some(value) => out.push(value),
                None => return Err(CounterError::Unwritten { index }),
            }
        }
        Ok(out)
    }

    /// The differences between consecutive values in `range`. A range of n values gives n - 1 intervals, and a range
    /// of fewer than two values gives none. Refused when a value is unwritten or goes backwards.
    pub fn intervals(&self, range: Range<usize>) -> Result<Vec<u64>, CounterError> {
        let start = range.start;
        let values = self.resolve(range)?;
        let mut out = Vec::with_capacity(values.len().saturating_sub(1));
        for (offset, pair) in values.windows(2).enumerate() {
            if pair[1] < pair[0] {
                return Err(CounterError::Backwards { index: start + offset + 1 });
            }
            out.push(pair[1] - pair[0]);
        }
        Ok(out)
    }

    fn check_range(&self, range: &Range<usize>) -> Result<(), CounterError> {
        let count = self.samples.len();
        if range.start > range.end || range.end > count {
            return Err(CounterError::BadRange { start: range.start, end: range.end, count });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(values: &[u64]) -> CounterSampleBuffer {
        let mut buffer = CounterSampleBuffer::new("frame", values.len());
        for (i, v) in values.iter().enumerate() {
            buffer.record(i, *v).unwrap();
        }
        buffer
    }

    #[test]
    fn new_buffer_has_its_label_and_unwritten_slots() {
        let buffer = CounterSampleBuffer::new("frame", 3);
        assert_eq!(buffer.label(), "frame");
        assert_eq!(buffer.sample_count(), 3);
        assert_eq!(buffer.resolve(0..1), Err(CounterError::Unwritten { index: 0 }));
    }

    #[test]
    fn record_past_the_end_is_refused() {
        let mut buffer = CounterSampleBuffer::new("frame", 2);
        assert_eq!(buffer.record(2, 10), Err(CounterError::OutOfRange { index: 2, count: 2 }));
    }

    #[test]
    fn resolve_returns_the_values_in_slot_order() {
        let buffer = filled(&[100, 250, 400]);
        assert_eq!(buffer.resolve(0..3).unwrap(), vec![100, 250, 400]);
        assert_eq!(buffer.resolve(1..2).unwrap(), vec![250]);
    }

    #[test]
    fn intervals_are_consecutive_differences() {
        let buffer = filled(&[100, 250, 400, 1000]);
        assert_eq!(buffer.intervals(0..4).unwrap(), vec![150, 150, 600]);
        assert_eq!(buffer.intervals(2..4).unwrap(), vec![600]);
    }

    #[test]
    fn a_range_of_fewer_than_two_values_has_no_intervals() {
        let buffer = filled(&[7, 9]);
        assert_eq!(buffer.intervals(0..1).unwrap(), Vec::<u64>::new());
        assert_eq!(buffer.intervals(1..1).unwrap(), Vec::<u64>::new());
    }

    #[test]
    fn backwards_samples_are_refused_with_the_later_index() {
        let buffer = filled(&[500, 300, 800]);
        assert_eq!(buffer.intervals(0..3), Err(CounterError::Backwards { index: 1 }));
        // A range that starts after the bad sample is still valid.
        assert_eq!(buffer.intervals(1..3).unwrap(), vec![500]);
    }

    #[test]
    fn a_read_with_an_unwritten_slot_or_a_bad_range_returns_nothing() {
        let mut buffer = CounterSampleBuffer::new("frame", 3);
        buffer.record(0, 5).unwrap();
        assert_eq!(buffer.resolve(0..3), Err(CounterError::Unwritten { index: 1 }));
        assert_eq!(buffer.resolve(2..1), Err(CounterError::BadRange { start: 2, end: 1, count: 3 }));
        assert_eq!(buffer.intervals(0..4), Err(CounterError::BadRange { start: 0, end: 4, count: 3 }));
    }
}
