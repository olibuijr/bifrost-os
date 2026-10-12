//! Sound classification results. A result covers one time range of an analysed stream or file and lists the labels the
//! classifier rated for that range, highest confidence first (`SoundClassificationResult`, `SoundClassification`,
//! `TimeRange`). These are pure data types: no classifier runs here. Times are seconds from the start of the analysis.

/// A span of time in seconds: `duration` seconds from `start`. Non-finite values become zero and negative values are
/// raised to zero, so a range always starts at or after zero and never runs backwards.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeRange {
    start: f64,
    duration: f64,
}

impl TimeRange {
    pub fn new(start: f64, duration: f64) -> Self {
        TimeRange {
            start: non_negative(start),
            duration: non_negative(duration),
        }
    }

    pub fn start(&self) -> f64 {
        self.start
    }

    pub fn duration(&self) -> f64 {
        self.duration
    }

    /// `start + duration`. The end is not included in the range.
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }

    /// True when `time` is at or after `start` and before `end`.
    pub fn contains(&self, time: f64) -> bool {
        time >= self.start && time < self.end()
    }
}

fn non_negative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn unit_interval(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// One label and its confidence. The confidence is kept in 0 to 1; non-finite values become zero.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundClassification {
    identifier: String,
    confidence: f64,
}

impl SoundClassification {
    pub fn new(identifier: &str, confidence: f64) -> Self {
        SoundClassification {
            identifier: identifier.to_string(),
            confidence: unit_interval(confidence),
        }
    }

    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    pub fn confidence(&self) -> f64 {
        self.confidence
    }
}

/// The classifications for one time range. Entries are sorted by confidence, highest first; equal confidences sort
/// by identifier. Each identifier appears once, with its highest confidence.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundClassificationResult {
    time_range: TimeRange,
    classifications: Vec<SoundClassification>,
}

impl SoundClassificationResult {
    pub fn new(time_range: TimeRange, classifications: Vec<SoundClassification>) -> Self {
        let mut sorted = classifications;
        sorted.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.identifier.cmp(&b.identifier))
        });
        let mut kept: Vec<SoundClassification> = Vec::with_capacity(sorted.len());
        for item in sorted {
            if !kept.iter().any(|k| k.identifier == item.identifier) {
                kept.push(item);
            }
        }
        SoundClassificationResult {
            time_range,
            classifications: kept,
        }
    }

    pub fn time_range(&self) -> TimeRange {
        self.time_range
    }

    /// Every classification, highest confidence first.
    pub fn classifications(&self) -> &[SoundClassification] {
        &self.classifications
    }

    /// The highest-ranking classification, if there is any.
    pub fn highest(&self) -> Option<&SoundClassification> {
        self.classifications.first()
    }

    /// The classification for `identifier`, if the classifier reported it.
    pub fn classification_for(&self, identifier: &str) -> Option<&SoundClassification> {
        self.classifications
            .iter()
            .find(|c| c.identifier == identifier)
    }

    /// Classifications whose confidence is at least `threshold`, highest first.
    pub fn above(&self, threshold: f64) -> Vec<&SoundClassification> {
        self.classifications
            .iter()
            .filter(|c| c.confidence >= threshold)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(result: &SoundClassificationResult) -> Vec<&str> {
        result.classifications().iter().map(|c| c.identifier()).collect()
    }

    #[test]
    fn sorts_highest_confidence_first() {
        let r = SoundClassificationResult::new(
            TimeRange::new(0.0, 1.0),
            vec![
                SoundClassification::new("dog", 0.2),
                SoundClassification::new("bell", 0.9),
                SoundClassification::new("cat", 0.5),
            ],
        );
        assert_eq!(labels(&r), vec!["bell", "cat", "dog"]);
        assert_eq!(r.highest().unwrap().identifier(), "bell");
    }

    #[test]
    fn equal_confidences_sort_by_identifier() {
        let r = SoundClassificationResult::new(
            TimeRange::new(0.0, 1.0),
            vec![
                SoundClassification::new("b", 0.5),
                SoundClassification::new("a", 0.5),
            ],
        );
        assert_eq!(labels(&r), vec!["a", "b"]);
    }

    #[test]
    fn duplicate_identifier_keeps_highest_confidence() {
        let r = SoundClassificationResult::new(
            TimeRange::new(0.0, 1.0),
            vec![
                SoundClassification::new("dog", 0.3),
                SoundClassification::new("dog", 0.7),
                SoundClassification::new("cat", 0.5),
            ],
        );
        assert_eq!(r.classifications().len(), 2);
        assert_eq!(r.classification_for("dog").unwrap().confidence(), 0.7);
        assert_eq!(labels(&r), vec!["dog", "cat"]);
    }

    #[test]
    fn confidence_is_clamped_to_unit_interval() {
        assert_eq!(SoundClassification::new("x", 1.4).confidence(), 1.0);
        assert_eq!(SoundClassification::new("y", -0.2).confidence(), 0.0);
        assert_eq!(SoundClassification::new("z", f64::NAN).confidence(), 0.0);
    }

    #[test]
    fn time_range_sanitises_bad_values() {
        let r = TimeRange::new(-1.0, f64::NAN);
        assert_eq!(r.start(), 0.0);
        assert_eq!(r.duration(), 0.0);
        let t = TimeRange::new(2.0, 3.0);
        assert_eq!(t.end(), 5.0);
        assert!(t.contains(2.0));
        assert!(!t.contains(5.0));
    }

    #[test]
    fn lookup_and_threshold() {
        let r = SoundClassificationResult::new(
            TimeRange::new(0.0, 1.0),
            vec![
                SoundClassification::new("bell", 0.8),
                SoundClassification::new("dog", 0.4),
            ],
        );
        assert!(r.classification_for("cat").is_none());
        assert_eq!(r.above(0.5).len(), 1);
        assert_eq!(r.above(0.5)[0].identifier(), "bell");
        assert_eq!(r.above(0.0).len(), 2);
    }

    #[test]
    fn empty_result_has_no_highest() {
        let r = SoundClassificationResult::new(TimeRange::new(4.0, 1.0), Vec::new());
        assert!(r.highest().is_none());
        assert_eq!(r.time_range().end(), 5.0);
    }
}
