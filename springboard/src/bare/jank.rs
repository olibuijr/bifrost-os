//! Always-on frame budget accounting: slow-frame log lines, per-screen/animation
//! counters and the perf.log summaries.
//!
//! Classification follows Android's jank stats (a frame that misses its vsync
//! deadline is janky; `dumpsys gfxinfo` and FrameMetrics also bucket frames
//! that miss by a whole extra vsync) and Chrome's frame timing (a frame that
//! takes more than one interval is slow, two or more intervals counts as
//! dropped). At 60 Hz: over 16.7 ms is slow, over 33 ms is very slow.
//!
//! Everything here is pure (times are passed in) so it is unit-tested; the
//! shell's frame loop only feeds it timings and writes what it returns.

use std::collections::HashMap;
use std::fmt::Write as _;

/// A frame longer than this missed its 60 Hz deadline.
pub const SLOW_MS: f64 = 16.7;
/// A frame longer than this missed at least two vsyncs.
pub const VERY_SLOW_MS: f64 = 33.0;
/// At most this many slow-frame lines per window.
pub const MAX_LINES: u32 = 20;
/// Rate-limit window.
pub const WINDOW_MS: f64 = 1000.0;
/// perf.log is rotated at this size (like crash.log).
pub const MAX_LOG_BYTES: u64 = 256 * 1024;
/// Counters are appended to perf.log this often, and at midnight.
pub const FLUSH_MS: f64 = 3_600_000.0;
/// Distinct screen/animation keys kept.
const MAX_KEYS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Ok,
    Slow,
    VerySlow,
}

pub fn classify(total_ms: f64) -> Class {
    if total_ms > VERY_SLOW_MS {
        Class::VerySlow
    } else if total_ms > SLOW_MS {
        Class::Slow
    } else {
        Class::Ok
    }
}

/// Timings of one presented frame, in ms (`total` runs from frame start to present).
#[derive(Clone, Copy, Debug, Default)]
pub struct Frame {
    pub total: f64,
    pub draw: f64,
    pub upload: f64,
    pub gpu: f64,
    pub present: f64,
    pub damage_pct: f64,
}

/// At most `MAX_LINES` lines per window; the rest are counted for a "+N more" line.
#[derive(Default)]
pub struct RateLimiter {
    start: f64,
    count: u32,
    suppressed: u32,
}

impl RateLimiter {
    pub fn allow(&mut self, now: f64) -> bool {
        if self.count == 0 || now - self.start >= WINDOW_MS {
            self.start = now;
            self.count = 0;
        }
        if self.count < MAX_LINES {
            self.count += 1;
            true
        } else {
            self.suppressed += 1;
            false
        }
    }

    /// The number of lines held back, once their window is over.
    pub fn take_suppressed(&mut self, now: f64) -> Option<u32> {
        if self.suppressed > 0 && now - self.start >= WINDOW_MS {
            Some(std::mem::take(&mut self.suppressed))
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Counter {
    pub frames: u64,
    pub slow: u64,
    pub very_slow: u64,
    pub worst: f64,
}

impl Counter {
    fn add(&mut self, total: f64) {
        self.frames += 1;
        match classify(total) {
            Class::Ok => {}
            Class::Slow => self.slow += 1,
            Class::VerySlow => {
                self.slow += 1;
                self.very_slow += 1;
            }
        }
        self.worst = self.worst.max(total);
    }

    fn text(&self) -> String {
        format!("{}/{}/{}/{:.1}", self.frames, self.slow, self.very_slow, self.worst)
    }
}

#[derive(Default)]
pub struct Jank {
    limiter: RateLimiter,
    keys: HashMap<String, Counter>,
    total: Counter,
    last_flush: f64,
    last_day: Option<(i32, i32, i32)>,
}

impl Jank {
    /// Records a frame; returns the log lines to print (slow frame, preceded by a burst summary).
    pub fn frame(&mut self, screen: &str, anim: &str, path: &str, f: Frame, now: f64) -> Vec<String> {
        let key = format!("{screen}/{anim}");
        if self.keys.len() < MAX_KEYS || self.keys.contains_key(&key) {
            self.keys.entry(key).or_default().add(f.total);
        }
        self.total.add(f.total);
        let mut lines = Vec::new();
        if let Some(n) = self.limiter.take_suppressed(now) {
            lines.push(format!("springboard: +{n} more slow frames"));
        }
        if classify(f.total) != Class::Ok && self.limiter.allow(now) {
            lines.push(format!(
                "springboard: slow frame {:.1} ms screen={screen} anim={anim} draw={:.1} upload={:.1} gpu={:.1} present={:.1} damage={:.0}% path={path}",
                f.total, f.draw, f.upload, f.gpu, f.present, f.damage_pct
            ));
        }
        lines
    }

    /// A pending "+N more" line once a burst is over (call when idle).
    pub fn idle(&mut self, now: f64) -> Option<String> {
        self.limiter.take_suppressed(now).map(|n| format!("springboard: +{n} more slow frames"))
    }

    /// `key frames/slow/very_slow/worst` lines for the control socket.
    pub fn describe(&self) -> String {
        let mut out = format!("perf_total={}\n", self.total.text());
        let mut keys: Vec<_> = self.keys.iter().collect();
        keys.sort_by(|a, b| b.1.worst.total_cmp(&a.1.worst).then(a.0.cmp(b.0)));
        for (k, c) in keys {
            let _ = writeln!(out, "perf {k}={}", c.text());
        }
        out
    }

    /// The perf.log line for the counters since the last one, when it is due (hourly, or the date
    /// changed); the counters restart. `day` is the local (year, month, day), `hms` the time of day.
    pub fn summary(&mut self, day: (i32, i32, i32), hms: (i32, i32, i32), now: f64) -> Option<String> {
        let rolled = self.last_day.is_some_and(|d| d != day);
        let due = now - self.last_flush >= FLUSH_MS;
        let prev = self.last_day.unwrap_or(day);
        if self.last_day.is_none() {
            self.last_day = Some(day);
            self.last_flush = now;
            return None;
        }
        if !rolled && !due {
            return None;
        }
        self.last_day = Some(day);
        self.last_flush = now;
        let stamp_day = if rolled { prev } else { day };
        let line = if self.total.frames == 0 { None } else { Some(format_summary(stamp_day, hms, &self.total, &self.keys)) };
        self.total = Counter::default();
        self.keys.clear();
        line
    }
}

/// `YYYY-MM-DD HH:MM:SS total=F/S/V/W screen/anim=F/S/V/W ...`, worst keys first.
pub fn format_summary(day: (i32, i32, i32), hms: (i32, i32, i32), total: &Counter, keys: &HashMap<String, Counter>) -> String {
    let mut out = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02} total={}", day.0, day.1, day.2, hms.0, hms.1, hms.2, total.text());
    let mut ks: Vec<_> = keys.iter().collect();
    ks.sort_by(|a, b| b.1.worst.total_cmp(&a.1.worst).then(a.0.cmp(b.0)));
    for (k, c) in ks {
        let _ = write!(out, " {k}={}", c.text());
    }
    out.push('\n');
    out
}

pub fn should_rotate(len: u64) -> bool {
    len >= MAX_LOG_BYTES
}

/// Appends a summary to DIR/perf.log, rotating to perf.log.old at 256 KB.
pub fn append(dir: &str, line: &str) {
    use std::io::Write;
    let path = format!("{dir}/perf.log");
    if std::fs::metadata(&path).is_ok_and(|m| should_rotate(m.len())) {
        let _ = std::fs::rename(&path, format!("{path}.old"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_against_the_vsync_budget() {
        assert_eq!(classify(16.7), Class::Ok);
        assert_eq!(classify(16.8), Class::Slow);
        assert_eq!(classify(33.0), Class::Slow);
        assert_eq!(classify(33.1), Class::VerySlow);
    }

    #[test]
    fn limiter_caps_a_burst_and_reports_the_rest() {
        let mut l = RateLimiter::default();
        let allowed = (0..50).filter(|i| l.allow(100.0 + *i as f64)).count();
        assert_eq!(allowed, 20);
        assert_eq!(l.take_suppressed(500.0), None);
        assert_eq!(l.take_suppressed(1100.0), Some(30));
        assert_eq!(l.take_suppressed(1200.0), None);
        assert!(l.allow(1200.0));
    }

    #[test]
    fn limiter_window_reopens() {
        let mut l = RateLimiter::default();
        for _ in 0..20 {
            assert!(l.allow(0.0));
        }
        assert!(!l.allow(999.0));
        assert!(l.allow(1000.0));
    }

    #[test]
    fn slow_frames_log_one_line_and_count() {
        let mut j = Jank::default();
        let ok = Frame { total: 10.0, ..Frame::default() };
        let slow = Frame { total: 40.0, draw: 30.0, present: 10.0, damage_pct: 50.0, ..Frame::default() };
        assert!(j.frame("home", "none", "cpu", ok, 0.0).is_empty());
        let l = j.frame("home", "scroll", "gpu", slow, 20.0);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0], "springboard: slow frame 40.0 ms screen=home anim=scroll draw=30.0 upload=0.0 gpu=0.0 present=10.0 damage=50% path=gpu");
        let d = j.describe();
        assert!(d.contains("perf_total=2/1/1/40.0"));
        assert!(d.contains("perf home/scroll=1/1/1/40.0"));
        assert!(d.contains("perf home/none=1/0/0/10.0"));
    }

    #[test]
    fn burst_summary_comes_before_the_next_slow_line() {
        let mut j = Jank::default();
        let slow = Frame { total: 20.0, ..Frame::default() };
        for i in 0..30 {
            j.frame("home", "none", "cpu", slow, i as f64);
        }
        assert_eq!(j.idle(500.0), None);
        let l = j.frame("home", "none", "cpu", slow, 1500.0);
        assert_eq!(l[0], "springboard: +10 more slow frames");
        assert!(l[1].contains("slow frame"));
    }

    #[test]
    fn summary_on_rollover_and_hourly() {
        let mut j = Jank::default();
        let slow = Frame { total: 50.0, ..Frame::default() };
        assert_eq!(j.summary((2026, 10, 5), (23, 0, 0), 0.0), None);
        j.frame("home", "open", "gpu", slow, 1.0);
        assert_eq!(j.summary((2026, 10, 5), (23, 30, 0), 1000.0), None);
        let s = j.summary((2026, 10, 6), (0, 0, 1), 2000.0).unwrap();
        assert_eq!(s, "2026-10-05 00:00:01 total=1/1/1/50.0 home/open=1/1/1/50.0\n");
        assert_eq!(j.summary((2026, 10, 6), (0, 0, 2), 3000.0), None);
        j.frame("home", "open", "gpu", slow, 3001.0);
        assert!(j.summary((2026, 10, 6), (1, 0, 2), FLUSH_MS + 5000.0).unwrap().starts_with("2026-10-06 01:00:02 total=1/1/1/50.0"));
    }

    #[test]
    fn rotation_threshold() {
        assert!(!should_rotate(MAX_LOG_BYTES - 1));
        assert!(should_rotate(MAX_LOG_BYTES));
    }
}
