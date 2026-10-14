//! `DateInterval` parity: a span of absolute time from a start to an end, in Unix seconds. The span is half open:
//! it contains its start and not its end, so two intervals that only touch do not intersect. Calendar choices do not
//! change the span, so the type needs no calendar or time zone.

/// A span of time from `start` up to `end` (not included). An interval never runs backwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateInterval {
    start: i64,
    end: i64,
}

impl DateInterval {
    /// The interval from `start` to `end`. None when `end` is before `start`.
    pub fn new(start: i64, end: i64) -> Option<DateInterval> {
        (end >= start).then_some(DateInterval { start, end })
    }

    /// The interval that starts at `start` and lasts `duration` seconds (`DateInterval(start:duration:)`). None when the
    /// duration is negative or the end does not fit in an `i64`.
    pub fn with_duration(start: i64, duration: i64) -> Option<DateInterval> {
        start.checked_add(duration).and_then(|end| DateInterval::new(start, end))
    }

    pub fn start(&self) -> i64 {
        self.start
    }

    pub fn end(&self) -> i64 {
        self.end
    }

    /// The length in seconds.
    pub fn duration(&self) -> i64 {
        self.end - self.start
    }

    /// Whether `instant` falls inside the interval: at or after the start, and before the end.
    pub fn contains(&self, instant: i64) -> bool {
        self.start <= instant && instant < self.end
    }

    /// The span both intervals cover, or None when they share no time.
    pub fn intersection(&self, other: &DateInterval) -> Option<DateInterval> {
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        (start < end).then_some(DateInterval { start, end })
    }

    /// Whether the two intervals share any time.
    pub fn intersects(&self, other: &DateInterval) -> bool {
        self.intersection(other).is_some()
    }
}
