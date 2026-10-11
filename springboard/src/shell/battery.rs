//! Battery history: the shell binary samples the level every five minutes
//! into `battery.log` (`unix percent charging` per line); Settings › Battery
//! bins the last 24 hours into the bar graph bifrost draws.

use super::icons::GREEN;
use super::ui::{Ui, Weight, SECONDARY, SEPARATOR};
use crate::gfx::{pack, Surface};
use std::path::Path;

/// How often the binary samples the battery.
pub const SAMPLE_MS: f64 = 300_000.0;
/// The graph's span and resolution: 24 hours in 15-minute bins.
pub const SPAN_SECS: i64 = 24 * 3600;
pub const BINS: usize = 96;
/// Lines kept in the log before it is rewritten (two days of samples).
const KEEP: usize = 2 * 24 * 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub unix: i64,
    pub percent: u8,
    pub charging: bool,
}

/// Parses `battery.log` lines, skipping malformed ones.
pub fn parse(text: &str) -> Vec<Sample> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some(Sample { unix: f.next()?.parse().ok()?, percent: f.next()?.parse::<u8>().ok()?.min(100), charging: f.next()? == "1" })
        })
        .collect()
}

/// One log line for `s`.
pub fn line(s: &Sample) -> String {
    format!("{} {} {}\n", s.unix, s.percent, if s.charging { 1 } else { 0 })
}

/// Loads the log at `path` (missing = empty).
pub fn load(path: &Path) -> Vec<Sample> {
    std::fs::read_to_string(path).map(|t| parse(&t)).unwrap_or_default()
}

/// Appends `s` to the log and to `history`, rewriting the file with only
/// the newest KEEP samples once it grows past that.
pub fn record(path: &Path, history: &mut Vec<Sample>, s: Sample) -> std::io::Result<()> {
    history.push(s);
    if history.len() > KEEP {
        history.drain(..history.len() - KEEP);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        return std::fs::write(path, history.iter().map(line).collect::<String>());
    }
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(line(&s).as_bytes())
}

/// One 15-minute bin: the latest level seen in it, and whether it charged.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Bin {
    pub percent: Option<u8>,
    pub charging: bool,
}

/// The last 24 hours before `now` in BINS bins, oldest first. A bin with no
/// sample carries the previous bin's level so the graph has no holes.
pub fn bins(samples: &[Sample], now: i64) -> Vec<Bin> {
    let start = now - SPAN_SECS;
    let width = SPAN_SECS / BINS as i64;
    let mut out = vec![Bin::default(); BINS];
    for s in samples.iter().filter(|s| s.unix > start && s.unix <= now) {
        let i = (((s.unix - start - 1) / width) as usize).min(BINS - 1);
        out[i].percent = Some(s.percent);
        out[i].charging |= s.charging;
    }
    // carry levels forward from the last sample before the window too
    let mut last = samples.iter().filter(|s| s.unix <= start).max_by_key(|s| s.unix).map(|s| s.percent);
    for b in &mut out {
        match b.percent {
            Some(p) => last = Some(p),
            None => b.percent = last,
        }
    }
    out
}

/// The rendered graph: bins plus the hour the window ends at, for labels.
#[derive(Clone, Debug, PartialEq)]
pub struct Graph {
    pub bins: Vec<Bin>,
    /// local hour (0..24) and minute at the right edge
    pub end_hour: i32,
    pub end_min: i32,
}

const CHARGE_BG: u32 = pack(24, 70, 40, 255);
const AXIS: u32 = pack(72, 72, 74, 255);

impl Graph {
    /// Draws into `(x, y, w, h)` px: the bar area with 100%/50%/0% labels on
    /// the right and hour labels every three hours underneath.
    pub fn draw(&self, s: &mut Surface, ui: &Ui, (x, y, w, h): (f32, f32, f32, f32)) {
        let title = ui.text("BATTERY LEVEL", 13.0, Weight::Regular, SECONDARY);
        s.blit(&title, x as i32, y as i32, 255, 255);
        let label_w = ui.pt(44.0);
        let (gx, gy) = (x, y + ui.pt(26.0));
        let gw = w - label_w;
        let gh = h - ui.pt(26.0 + 30.0);
        // charging stretches behind the bars
        let bw = gw / self.bins.len() as f32;
        for (i, b) in self.bins.iter().enumerate() {
            if b.charging {
                s.fill_rrect(gx + i as f32 * bw, gy, bw, gh, 0.0, CHARGE_BG);
            }
        }
        for (k, label) in [(0.0, "100%"), (0.5, "50%"), (1.0, "0%")] {
            let ly = gy + gh * k;
            s.fill_rrect(gx, ly - 0.5, gw, 1.0, 0.0, if k == 1.0 { SEPARATOR } else { AXIS });
            let img = ui.text(label, 12.0, Weight::Regular, SECONDARY);
            s.blit(&img, (gx + gw + ui.pt(8.0)) as i32, (ly - img.h as f32 / 2.0) as i32, 255, 255);
        }
        for (i, b) in self.bins.iter().enumerate() {
            if let Some(p) = b.percent {
                let bh = gh * p as f32 / 100.0;
                let bx = gx + i as f32 * bw + bw * 0.15;
                s.fill_rrect(bx, gy + gh - bh, bw * 0.7, bh, 0.0, GREEN);
            }
        }
        // hour ticks every three hours, labelled with the local hour
        let end_frac = self.end_min as f32 / 60.0;
        let hours_back = (self.end_hour as f32 + end_frac).floor() as i32;
        let px_per_hour = gw / 24.0;
        for k in 0..=24 {
            let hour = (hours_back - k).rem_euclid(24);
            if hour % 3 != 0 {
                continue;
            }
            let tx = gx + gw - (k as f32 + end_frac) * px_per_hour;
            if tx < gx - 1.0 {
                continue;
            }
            s.fill_rrect(tx, gy, 1.0, gh + ui.pt(4.0), 0.0, AXIS);
            let img = ui.text(&format!("{hour:02}"), 12.0, Weight::Regular, SECONDARY);
            s.blit(&img, (tx + ui.pt(4.0)) as i32, (gy + gh + ui.pt(8.0)) as i32, 255, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_lines() {
        let s = parse("100 80 0\n200 79 1\nbad\n300 x 0\n400 250 1\n500 -1 0\n");
        assert_eq!(s, vec![Sample { unix: 100, percent: 80, charging: false }, Sample { unix: 200, percent: 79, charging: true }, Sample { unix: 400, percent: 100, charging: true }]);
        assert_eq!(line(&s[1]), "200 79 1\n");
        assert_eq!(parse(&s.iter().map(line).collect::<String>()), s);
    }

    #[test]
    fn bins_take_the_latest_sample_and_carry_levels_forward() {
        let now = 1_000_000;
        let at = |secs_ago: i64, p: u8, c: bool| Sample { unix: now - secs_ago, percent: p, charging: c };
        let samples = vec![at(SPAN_SECS + 100, 90, false), at(SPAN_SECS - 10, 85, false), at(SPAN_SECS - 400, 84, false), at(3600, 50, true), at(10, 55, true)];
        let b = bins(&samples, now);
        assert_eq!(b.len(), BINS);
        assert_eq!(b[0], Bin { percent: Some(84), charging: false }, "the latest sample in the bin wins");
        assert_eq!(b[1].percent, Some(84), "carried forward");
        assert_eq!(b[BINS - 1], Bin { percent: Some(55), charging: true });
        assert_eq!(b[BINS - 5].percent, Some(50));
        assert!(b[BINS - 5].charging);
        assert!(!b[BINS - 6].charging);
        let empty = bins(&[], now);
        assert!(empty.iter().all(|b| b.percent.is_none()));
        let before_only = bins(&[at(SPAN_SECS * 2, 33, false)], now);
        assert!(before_only.iter().all(|b| b.percent == Some(33)), "an old level still fills the window");
        let future = bins(&[at(-100, 1, false)], now);
        assert!(future.iter().all(|b| b.percent.is_none()), "samples after now are ignored");
    }

    #[test]
    fn record_appends_and_trims_the_file() {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-battery")).join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("battery.log");
        let mut history = load(&path);
        assert!(history.is_empty());
        for i in 0..3 {
            record(&path, &mut history, Sample { unix: i, percent: 50, charging: false }).unwrap();
        }
        assert_eq!(load(&path), history);
        assert_eq!(history.len(), 3);
        for i in 3..(KEEP as i64 + 5) {
            record(&path, &mut history, Sample { unix: i, percent: 50, charging: false }).unwrap();
        }
        assert_eq!(history.len(), KEEP);
        assert_eq!(history[0].unix, 5);
        assert_eq!(load(&path).len(), KEEP, "the file was rewritten with the newest samples");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn graph_draws_bars_and_labels() {
        let ui = crate::shell::tests::ui();
        let mut b = vec![Bin { percent: Some(60), charging: false }; BINS];
        b[BINS - 1].charging = true;
        let g = Graph { bins: b, end_hour: 14, end_min: 30 };
        let mut buf = vec![0u32; 1000 * 400];
        g.draw(&mut Surface { w: 1000, h: 400, stride: 1000, px: &mut buf, y0: 0 }, &ui, (20.0, 10.0, 960.0, 380.0));
        assert!(buf.contains(&GREEN), "bars");
        assert!(buf.contains(&CHARGE_BG), "charging background");
        let green_rows: Vec<usize> = (0..400).filter(|&y| buf[y * 1000 + 100] == GREEN).collect();
        let green_px = buf.iter().filter(|&&p| p == GREEN).count();
        assert!(green_px > 1000);
        assert!(!green_rows.is_empty());
        let empty = Graph { bins: vec![Bin::default(); BINS], end_hour: 0, end_min: 0 };
        let mut buf = vec![0u32; 1000 * 400];
        empty.draw(&mut Surface { w: 1000, h: 400, stride: 1000, px: &mut buf, y0: 0 }, &ui, (20.0, 10.0, 960.0, 380.0));
        assert!(!buf.contains(&GREEN), "no bars without samples");
    }
}

/// Health rows (label, value) from sysfs power_supply text values; absent
/// nodes are skipped. `read` maps a node name to its trimmed contents.
pub fn health_rows(read: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    let num = |n: &str| read(n).and_then(|s| s.trim().parse::<i64>().ok());
    let mut rows = Vec::new();
    if let (Some(full), Some(design)) = (num("charge_full"), num("charge_full_design")) {
        if design > 0 {
            rows.push(("Maximum Capacity".to_string(), format!("{}%", (full * 100 / design).clamp(0, 100))));
        }
    }
    if let Some(c) = num("cycle_count").filter(|c| *c >= 0) {
        rows.push(("Cycle Count".to_string(), c.to_string()));
    }
    if let Some(h) = read("health").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        rows.push(("Condition".to_string(), h));
    }
    if let Some(t) = num("temp") {
        rows.push(("Temperature".to_string(), format!("{:.1} \u{b0}C", t as f32 / 10.0)));
    }
    if let Some(v) = num("voltage_now") {
        rows.push(("Voltage".to_string(), format!("{:.2} V", v as f32 / 1_000_000.0)));
    }
    rows
}

/// Reads the health rows from /sys/class/power_supply/battery.
pub fn health() -> Vec<(String, String)> {
    health_rows(&|n| std::fs::read_to_string(format!("/sys/class/power_supply/battery/{n}")).ok())
}
