//! GPU and frame metrics for an app: per-frame GPU time and presentation intervals fed in by the presenter, summarised
//! as frame rate, hitch count, percentile GPU time and a daily-style payload (histogram buckets). Pure std.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Time the GPU spent on the frame, in milliseconds.
    pub gpu_ms: f32,
    /// Time since the previous frame was presented, in milliseconds.
    pub interval_ms: f32,
}

/// A rolling window of frames.
#[derive(Clone, Debug)]
pub struct Metrics {
    frames: Vec<Frame>,
    capacity: usize,
    /// The display's refresh interval in milliseconds; a frame later than 1.5 of these is a hitch.
    pub refresh_ms: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub frames: usize,
    pub fps: f32,
    pub hitches: usize,
    pub gpu_avg_ms: f32,
    pub gpu_p95_ms: f32,
    pub gpu_max_ms: f32,
}

impl Metrics {
    pub fn new(capacity: usize, refresh_hz: f32) -> Metrics {
        Metrics { frames: Vec::new(), capacity: capacity.max(1), refresh_ms: 1000.0 / refresh_hz.max(1.0) }
    }

    pub fn record(&mut self, frame: Frame) {
        if self.frames.len() == self.capacity {
            self.frames.remove(0);
        }
        self.frames.push(frame);
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }

    pub fn is_hitch(&self, f: &Frame) -> bool {
        f.interval_ms > self.refresh_ms * 1.5
    }

    pub fn summary(&self) -> Summary {
        let n = self.frames.len();
        if n == 0 {
            return Summary { frames: 0, fps: 0.0, hitches: 0, gpu_avg_ms: 0.0, gpu_p95_ms: 0.0, gpu_max_ms: 0.0 };
        }
        let total_interval: f32 = self.frames.iter().map(|f| f.interval_ms).sum();
        let mut gpu: Vec<f32> = self.frames.iter().map(|f| f.gpu_ms).collect();
        gpu.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p95_index = ((n as f32 * 0.95).ceil() as usize).clamp(1, n) - 1;
        Summary {
            frames: n,
            fps: if total_interval > 0.0 { n as f32 * 1000.0 / total_interval } else { 0.0 },
            hitches: self.frames.iter().filter(|f| self.is_hitch(f)).count(),
            gpu_avg_ms: gpu.iter().sum::<f32>() / n as f32,
            gpu_p95_ms: gpu[p95_index],
            gpu_max_ms: gpu[n - 1],
        }
    }

    /// Histogram of GPU time with the given upper bounds in milliseconds; the last bucket counts everything above.
    pub fn gpu_histogram(&self, bounds: &[f32]) -> Vec<usize> {
        let mut out = vec![0; bounds.len() + 1];
        for f in &self.frames {
            let i = bounds.iter().position(|b| f.gpu_ms <= *b).unwrap_or(bounds.len());
            out[i] += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(gpu: f32, interval: f32) -> Frame {
        Frame { gpu_ms: gpu, interval_ms: interval }
    }

    #[test]
    fn empty_summary() {
        assert_eq!(Metrics::new(10, 60.0).summary().frames, 0);
    }

    #[test]
    fn rate_hitches_and_percentiles() {
        let mut m = Metrics::new(100, 60.0);
        for i in 0..19 {
            m.record(frame(2.0 + i as f32 * 0.1, 16.67));
        }
        m.record(frame(20.0, 50.0)); // a slow frame, three refresh intervals
        let s = m.summary();
        assert_eq!(s.frames, 20);
        assert_eq!(s.hitches, 1);
        assert!(s.fps > 50.0 && s.fps < 60.0, "{}", s.fps);
        assert_eq!(s.gpu_max_ms, 20.0);
        assert!(s.gpu_p95_ms < 20.0 && s.gpu_p95_ms > 3.0, "{}", s.gpu_p95_ms);
        assert!(s.gpu_avg_ms > 2.0);
    }

    #[test]
    fn window_rolls_over() {
        let mut m = Metrics::new(3, 60.0);
        for i in 0..5 {
            m.record(frame(i as f32, 16.0));
        }
        assert_eq!(m.summary().frames, 3);
        assert_eq!(m.summary().gpu_max_ms, 4.0);
        m.clear();
        assert_eq!(m.summary().frames, 0);
    }

    #[test]
    fn histogram_buckets() {
        let mut m = Metrics::new(10, 60.0);
        for g in [1.0, 4.0, 4.0, 9.0, 40.0] {
            m.record(frame(g, 16.0));
        }
        assert_eq!(m.gpu_histogram(&[2.0, 5.0, 10.0]), vec![1, 2, 1, 1]);
    }
}
