//! Frame timing statistics for the bench and the perf HUD: per-frame compose CPU time, GPU time (timestamp queries),
//! present latency (submit to vblank), missed vblanks; rolling percentiles; written as JSON lines to
//! `$XDG_RUNTIME_DIR/bifrost-wm-frames.jsonl` when `BIFROST_WM_STATS=1`, read by `tools/bench.py --midget`.
//!
//! Percentiles use the nearest-rank method: the `p`-th percentile of `n` sorted samples is the value at rank
//! `ceil(p/100 * n)` (1-based), with rank clamped to `1..=n`. Empty sets give zero.
//!
//! Owner: wave G1 agent `wm-stats`.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Name of the JSON lines file inside `$XDG_RUNTIME_DIR`.
pub const STATS_FILE: &str = "bifrost-wm-frames.jsonl";

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

    /// Adds a frame, dropping the oldest ones beyond `capacity` (a zero capacity keeps one frame).
    pub fn push(&mut self, f: FrameRecord) {
        self.frames.push(f);
        let cap = self.capacity.max(1);
        if self.frames.len() > cap {
            let excess = self.frames.len() - cap;
            self.frames.drain(..excess);
        }
    }

    /// The `p`-th percentile (0..100) of CPU time.
    pub fn cpu_percentile(&self, p: f32) -> Duration {
        let cpu: Vec<Duration> = self.frames.iter().map(|f| f.cpu).collect();
        percentile(&cpu, p)
    }

    /// The `p`-th percentile of GPU time.
    pub fn gpu_percentile(&self, p: f32) -> Duration {
        let gpu: Vec<Duration> = self.frames.iter().map(|f| f.gpu).collect();
        percentile(&gpu, p)
    }

    /// Fraction of frames with a missed vblank.
    pub fn dropped_ratio(&self) -> f32 {
        if self.frames.is_empty() {
            return 0.0;
        }
        let dropped = self.frames.iter().filter(|f| f.missed > 0).count();
        dropped as f32 / self.frames.len() as f32
    }

    /// One JSON line for the record (no serde: hand-written). Times are integer microseconds.
    pub fn json_line(f: &FrameRecord) -> String {
        format!(
            "{{\"seq\":{},\"cpu_us\":{},\"gpu_us\":{},\"latency_us\":{},\"missed\":{},\"damaged_px\":{}}}",
            f.seq,
            f.cpu.as_micros(),
            f.gpu.as_micros(),
            f.latency.as_micros(),
            f.missed,
            f.damaged_px
        )
    }
}

/// Nearest-rank percentile of `samples` (`p` in 0..100; NaN counts as 0, out of range is clamped).
pub(crate) fn percentile(samples: &[Duration], p: f32) -> Duration {
    if samples.is_empty() {
        return Duration::ZERO;
    }
    let mut sorted = samples.to_vec();
    sorted.sort();
    let p = f64::from(if p.is_nan() { 0.0 } else { p.clamp(0.0, 100.0) });
    let n = sorted.len();
    // The epsilon keeps exact products such as 0.95 * 20 = 19 from rounding up to 20.
    let rank = ((p / 100.0 * n as f64) - 1e-9).ceil().max(1.0) as usize;
    sorted[rank.min(n) - 1]
}

/// True when the `BIFROST_WM_STATS` value turns frame statistics on (exactly `1`).
pub fn enabled_from(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Whether frame statistics are written, from the environment.
pub fn enabled() -> bool {
    enabled_from(std::env::var("BIFROST_WM_STATS").ok().as_deref())
}

/// The stats file under the runtime directory; `None` when it is unset or empty.
pub fn path_in(runtime_dir: Option<&str>) -> Option<PathBuf> {
    runtime_dir.filter(|d| !d.is_empty()).map(|d| Path::new(d).join(STATS_FILE))
}

/// Appends one JSON line for `f` to `path`, creating the file if needed.
pub fn append_to(path: &Path, f: &FrameRecord) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let mut line = Stats::json_line(f);
    line.push('\n');
    file.write_all(line.as_bytes())
}

/// Appends `f` to `$XDG_RUNTIME_DIR/bifrost-wm-frames.jsonl` when `BIFROST_WM_STATS=1`; otherwise does nothing.
pub fn append_frame(f: &FrameRecord) -> io::Result<()> {
    if !enabled() {
        return Ok(());
    }
    match path_in(std::env::var("XDG_RUNTIME_DIR").ok().as_deref()) {
        Some(path) => append_to(&path, f),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    fn frame(seq: u64, cpu_ms: u64, gpu_ms: u64, missed: u32) -> FrameRecord {
        FrameRecord {
            seq,
            cpu: ms(cpu_ms),
            gpu: ms(gpu_ms),
            latency: ms(16),
            missed,
            damaged_px: 0,
        }
    }

    #[test]
    fn push_drops_the_oldest_beyond_capacity() {
        let mut s = Stats::new(3);
        for i in 0..5 {
            s.push(frame(i, 1, 1, 0));
        }
        let seqs: Vec<u64> = s.frames.iter().map(|f| f.seq).collect();
        assert_eq!(seqs, vec![2, 3, 4]);
    }

    #[test]
    fn zero_capacity_keeps_one_frame() {
        let mut s = Stats::new(0);
        s.push(frame(1, 1, 1, 0));
        s.push(frame(2, 1, 1, 0));
        assert_eq!(s.frames.len(), 1);
        assert_eq!(s.frames[0].seq, 2);
    }

    #[test]
    fn cpu_percentiles_use_nearest_rank() {
        // Samples 1..=10 ms: rank = ceil(p/100 * 10).
        let mut s = Stats::new(100);
        for i in 1..=10 {
            s.push(frame(i, i, 0, 0));
        }
        assert_eq!(s.cpu_percentile(0.0), ms(1));
        assert_eq!(s.cpu_percentile(50.0), ms(5)); // rank 5
        assert_eq!(s.cpu_percentile(90.0), ms(9)); // rank 9
        assert_eq!(s.cpu_percentile(95.0), ms(10)); // rank ceil(9.5) = 10
        assert_eq!(s.cpu_percentile(100.0), ms(10));
    }

    #[test]
    fn gpu_percentile_reads_gpu_time() {
        let mut s = Stats::new(100);
        for (i, g) in [4u64, 1, 3, 2].iter().enumerate() {
            s.push(frame(i as u64, 9, *g, 0));
        }
        // Sorted 1,2,3,4: rank ceil(0.5 * 4) = 2 gives 2 ms; rank ceil(0.75 * 4) = 3 gives 3 ms.
        assert_eq!(s.gpu_percentile(50.0), ms(2));
        assert_eq!(s.gpu_percentile(75.0), ms(3));
        assert_eq!(s.gpu_percentile(100.0), ms(4));
    }

    #[test]
    fn empty_stats_are_zero() {
        let s = Stats::new(10);
        assert_eq!(s.cpu_percentile(95.0), Duration::ZERO);
        assert_eq!(s.gpu_percentile(50.0), Duration::ZERO);
        assert_eq!(s.dropped_ratio(), 0.0);
    }

    #[test]
    fn dropped_ratio_counts_frames_with_a_missed_vblank() {
        let mut s = Stats::new(10);
        for (i, missed) in [0u32, 0, 1, 0, 2].iter().enumerate() {
            s.push(frame(i as u64, 1, 1, *missed));
        }
        assert!((s.dropped_ratio() - 0.4).abs() < 1e-6);
    }

    #[test]
    fn percentile_clamps_out_of_range_and_nan() {
        let v = [ms(1), ms(2), ms(3)];
        assert_eq!(percentile(&v, 250.0), ms(3));
        assert_eq!(percentile(&v, -5.0), ms(1));
        assert_eq!(percentile(&v, f32::NAN), ms(1));
        assert_eq!(percentile(&[], 50.0), Duration::ZERO);
    }

    #[test]
    fn json_line_has_integer_microseconds() {
        let f = FrameRecord {
            seq: 7,
            cpu: Duration::from_micros(1500),
            gpu: Duration::from_micros(2000),
            latency: Duration::from_micros(16_000),
            missed: 1,
            damaged_px: 12_345,
        };
        assert_eq!(
            Stats::json_line(&f),
            r#"{"seq":7,"cpu_us":1500,"gpu_us":2000,"latency_us":16000,"missed":1,"damaged_px":12345}"#
        );
    }

    #[test]
    fn stats_are_enabled_only_by_the_value_one() {
        assert!(enabled_from(Some("1")));
        assert!(!enabled_from(Some("0")));
        assert!(!enabled_from(Some("true")));
        assert!(!enabled_from(None));
    }

    #[test]
    fn path_is_under_the_runtime_dir() {
        assert_eq!(
            path_in(Some("/run/user/1000")),
            Some(PathBuf::from("/run/user/1000/bifrost-wm-frames.jsonl"))
        );
        assert_eq!(path_in(Some("")), None);
        assert_eq!(path_in(None), None);
    }

    #[test]
    fn append_writes_one_line_per_frame() {
        let dir = std::env::temp_dir().join(format!("bifrost-wm-stats-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(STATS_FILE);
        let _ = std::fs::remove_file(&path);
        append_to(&path, &frame(1, 1, 2, 0)).unwrap();
        append_to(&path, &frame(2, 3, 4, 1)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("{\"seq\":1,"));
        assert!(lines[1].contains("\"missed\":1"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
