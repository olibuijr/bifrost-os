//! Throughput and time remaining for a unit of work, from samples the owner of the work records. The caller passes
//! the elapsed time with each sample, so the model holds no clock and the tests are deterministic. The rate is the
//! average between the first and the latest sample, which is what an estimate of the finishing time needs.
//! The progress tree (`progress.rs`) does not compute these values: the owner of the work records them here.

#[derive(Clone, Copy, Debug, PartialEq)]
struct Sample {
    elapsed_secs: f64,
    completed: i64,
}

/// The samples of one unit of work. Samples must arrive in increasing time and never decrease in completed units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProgressRate {
    first: Option<Sample>,
    last: Option<Sample>,
}

impl ProgressRate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the completed units at `elapsed_secs` since the work started. Returns false, and keeps the samples
    /// as they were, when the time is not finite, does not move forward, or the completed units go backwards.
    pub fn record(&mut self, elapsed_secs: f64, completed: i64) -> bool {
        if !elapsed_secs.is_finite() || completed < 0 {
            return false;
        }
        let sample = Sample { elapsed_secs, completed };
        match self.last {
            None => {
                self.first = Some(sample);
                self.last = Some(sample);
                true
            }
            Some(last) => {
                if elapsed_secs <= last.elapsed_secs || completed < last.completed {
                    return false;
                }
                self.last = Some(sample);
                true
            }
        }
    }

    /// Units completed per second, from the first sample to the latest. None until two samples exist.
    pub fn throughput(&self) -> Option<f64> {
        let (first, last) = (self.first?, self.last?);
        if last.elapsed_secs <= first.elapsed_secs {
            return None;
        }
        Some((last.completed - first.completed) as f64 / (last.elapsed_secs - first.elapsed_secs))
    }

    /// Seconds until `total` units are complete at the average rate. Zero once the work is complete. None when the
    /// rate is not known yet or is zero, because a stalled job has no finishing time to estimate.
    pub fn estimated_remaining_secs(&self, total: i64) -> Option<f64> {
        let last = self.last?;
        if last.completed >= total {
            return Some(0.0);
        }
        let rate = self.throughput()?;
        if rate <= 0.0 {
            return None;
        }
        Some((total - last.completed) as f64 / rate)
    }
}
