//! Frame timing statistics for the bench and the perf HUD: per-frame compose CPU time, GPU time (timestamp queries),
//! present latency (submit to vblank), missed vblanks; rolling percentiles; written as JSON lines to
//! `$XDG_RUNTIME_DIR/bifrost-wm-frames.jsonl` when `BIFROST_WM_STATS=1`, read by `tools/bench.py --midget`.
//!
//! Owner: wave G1 agent `wm-stats`.

use std::time::Duration;

/// One composited frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FrameRecord {
    pub seq: u64,
    pub cpu: Duration,
    pub gpu: Duration,
    /// From submission to the vblank that showed it.
    pub latency: Duration,
    /// Vblanks skipped since the previous frame (0 is smooth).
    pub missed: u32,
    pub damaged_px: u64,
}

/// Rolling statistics over the last `capacity` frames.
#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub frames: Vec<FrameRecord>,
    pub capacity: usize,
}

impl Stats {
    pub fn new(capacity: usize) -> Stats {
        Stats { frames: Vec::new(), capacity }
    }

    pub fn push(&mut self, f: FrameRecord) {
        let _ = f;
        todo!("wm-stats")
    }

    /// The `p`-th percentile (0..100) of CPU time.
    pub fn cpu_percentile(&self, p: f32) -> Duration {
        let _ = p;
        todo!("wm-stats")
    }

    /// The `p`-th percentile of GPU time.
    pub fn gpu_percentile(&self, p: f32) -> Duration {
        let _ = p;
        todo!("wm-stats")
    }

    /// Fraction of frames with a missed vblank.
    pub fn dropped_ratio(&self) -> f32 {
        todo!("wm-stats")
    }

    /// One JSON line for the record (no serde: hand-written).
    pub fn json_line(f: &FrameRecord) -> String {
        let _ = f;
        todo!("wm-stats")
    }
}
