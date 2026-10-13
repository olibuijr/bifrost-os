//! Per-app frame metrics: GPU time per frame and the frame rate an app achieves, the figures behind GPUTimeMetric and
//! MetalFrameRateMetric (docs/parity-matrix, metrickit). A `Recorder` keeps the last N presented frames of one app
//! and summarises them.
//!
//! Callers supply every figure: the GPU time of each frame and the instant it was presented, both in nanoseconds
//! from any fixed origin. Nothing here reads a clock or a GPU counter. Presentation times that go backwards are
//! refused, and a frame interval longer than 1.5 refresh periods counts as a dropped frame.

use std::collections::VecDeque;

/// One presented frame: the GPU time spent on it and the instant it reached the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub gpu_ns: u64,
    pub presented_ns: u64,
}

/// Why a frame was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The presentation time is earlier than the last recorded frame.
    OutOfOrder,
}

/// Summary of the frames in the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    /// Frames in the window.
    pub frames: u64,
    /// Mean GPU time per frame, in nanoseconds. Zero when there are no frames.
    pub mean_gpu_ns: u64,
    /// Longest GPU time of one frame, in nanoseconds.
    pub max_gpu_ns: u64,
    /// Frames per second over the window. Zero with fewer than two frames.
    pub frame_rate: f64,
    /// Intervals between consecutive frames that were longer than 1.5 refresh periods.
    pub dropped: u64,
}

/// Keeps the last `capacity` frames of one app, and the refresh period they are measured against.
#[derive(Debug)]
pub struct Recorder {
    capacity: usize,
    refresh_ns: u64,
    frames: VecDeque<Frame>,
}

impl Recorder {
    /// A recorder that keeps `capacity` frames (at least one) against a display refresh period of `refresh_ns`.
    pub fn new(capacity: usize, refresh_ns: u64) -> Self {
        let capacity = capacity.max(1);
        Self { capacity, refresh_ns, frames: VecDeque::with_capacity(capacity) }
    }

    /// Records a presented frame. The oldest frame leaves the window when it is full.
    pub fn record(&mut self, gpu_ns: u64, presented_ns: u64) -> Result<(), Error> {
        if let Some(last) = self.frames.back() {
            if presented_ns < last.presented_ns {
                return Err(Error::OutOfOrder);
            }
        }
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(Frame { gpu_ns, presented_ns });
        Ok(())
    }

    /// The frames currently in the window, oldest first.
    pub fn frames(&self) -> impl Iterator<Item = &Frame> {
        self.frames.iter()
    }

    /// Summary of the window. Empty windows give zeros.
    pub fn summary(&self) -> Summary {
        let frames = self.frames.len() as u64;
        if frames == 0 {
            return Summary { frames: 0, mean_gpu_ns: 0, max_gpu_ns: 0, frame_rate: 0.0, dropped: 0 };
        }
        let total: u128 = self.frames.iter().map(|f| u128::from(f.gpu_ns)).sum();
        let max_gpu_ns = self.frames.iter().map(|f| f.gpu_ns).max().unwrap_or(0);
        let mean_gpu_ns = (total / u128::from(frames)) as u64;
        let limit = self.refresh_ns.saturating_mul(3) / 2;
        let mut dropped = 0;
        let mut previous: Option<u64> = None;
        for frame in &self.frames {
            if let Some(prev) = previous {
                if self.refresh_ns > 0 && frame.presented_ns - prev > limit {
                    dropped += 1;
                }
            }
            previous = Some(frame.presented_ns);
        }
        let frame_rate = match (self.frames.front(), self.frames.back()) {
            (Some(first), Some(last)) if frames > 1 && last.presented_ns > first.presented_ns => {
                let span_s = (last.presented_ns - first.presented_ns) as f64 / 1e9;
                (frames - 1) as f64 / span_s
            }
            _ => 0.0,
        };
        Summary { frames, mean_gpu_ns, max_gpu_ns, frame_rate, dropped }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIXTY_HZ: u64 = 16_666_667;

    #[test]
    fn empty_window_summarises_to_zero() {
        let r = Recorder::new(8, SIXTY_HZ);
        assert_eq!(r.summary(), Summary { frames: 0, mean_gpu_ns: 0, max_gpu_ns: 0, frame_rate: 0.0, dropped: 0 });
    }

    #[test]
    fn mean_and_max_gpu_time() {
        let mut r = Recorder::new(8, SIXTY_HZ);
        r.record(2_000_000, 0).unwrap();
        r.record(4_000_000, SIXTY_HZ).unwrap();
        r.record(6_000_000, 2 * SIXTY_HZ).unwrap();
        let s = r.summary();
        assert_eq!(s.frames, 3);
        assert_eq!(s.mean_gpu_ns, 4_000_000);
        assert_eq!(s.max_gpu_ns, 6_000_000);
        assert_eq!(s.dropped, 0);
    }

    #[test]
    fn frame_rate_over_steady_frames_is_refresh_rate() {
        let mut r = Recorder::new(61, SIXTY_HZ);
        for i in 0..61u64 {
            r.record(1_000_000, i * SIXTY_HZ).unwrap();
        }
        let fps = r.summary().frame_rate;
        assert!((fps - 60.0).abs() < 0.01, "fps was {fps}");
    }

    #[test]
    fn long_interval_counts_as_dropped_frame() {
        let mut r = Recorder::new(8, SIXTY_HZ);
        r.record(0, 0).unwrap();
        r.record(0, SIXTY_HZ).unwrap();
        // Two refresh periods later: one frame was missed.
        r.record(0, 3 * SIXTY_HZ).unwrap();
        assert_eq!(r.summary().dropped, 1);
    }

    #[test]
    fn interval_at_one_and_a_half_periods_is_not_dropped() {
        let mut r = Recorder::new(8, 10);
        r.record(0, 0).unwrap();
        r.record(0, 15).unwrap();
        assert_eq!(r.summary().dropped, 0);
        r.record(0, 31).unwrap();
        assert_eq!(r.summary().dropped, 1);
    }

    #[test]
    fn out_of_order_presentation_is_refused() {
        let mut r = Recorder::new(8, SIXTY_HZ);
        r.record(0, 100).unwrap();
        assert_eq!(r.record(0, 99), Err(Error::OutOfOrder));
        assert_eq!(r.frames().count(), 1);
    }

    #[test]
    fn window_drops_oldest_frames() {
        let mut r = Recorder::new(2, SIXTY_HZ);
        r.record(9_000_000, 0).unwrap();
        r.record(1_000_000, SIXTY_HZ).unwrap();
        r.record(3_000_000, 2 * SIXTY_HZ).unwrap();
        let s = r.summary();
        assert_eq!(s.frames, 2);
        assert_eq!(s.max_gpu_ns, 3_000_000);
        assert_eq!(s.mean_gpu_ns, 2_000_000);
    }

    #[test]
    fn single_frame_has_no_rate() {
        let mut r = Recorder::new(4, SIXTY_HZ);
        r.record(1, 5).unwrap();
        assert_eq!(r.summary().frame_rate, 0.0);
    }

    #[test]
    fn equal_presentation_times_are_accepted_and_have_no_rate() {
        let mut r = Recorder::new(4, SIXTY_HZ);
        r.record(1, 50).unwrap();
        r.record(1, 50).unwrap();
        assert_eq!(r.summary().frames, 2);
        assert_eq!(r.summary().frame_rate, 0.0);
    }

    #[test]
    fn zero_refresh_period_counts_no_drops() {
        let mut r = Recorder::new(4, 0);
        r.record(0, 0).unwrap();
        r.record(0, 1_000_000_000).unwrap();
        assert_eq!(r.summary().dropped, 0);
    }

    #[test]
    fn dropped_frames_leave_with_the_window() {
        let mut r = Recorder::new(2, SIXTY_HZ);
        r.record(0, 0).unwrap();
        r.record(0, 4 * SIXTY_HZ).unwrap();
        assert_eq!(r.summary().dropped, 1);
        r.record(0, 5 * SIXTY_HZ).unwrap();
        assert_eq!(r.summary().dropped, 0);
    }

    #[test]
    fn zero_capacity_keeps_one_frame() {
        let mut r = Recorder::new(0, SIXTY_HZ);
        r.record(1, 0).unwrap();
        r.record(2, 1).unwrap();
        assert_eq!(r.summary().frames, 1);
        assert_eq!(r.summary().max_gpu_ns, 2);
    }
}
