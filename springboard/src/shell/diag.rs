//! Analytics & Diagnostics data: crash.log entries, the bundle shared with the
//! developer, and the performance HUD's sensor readings. Pure parsing and
//! formatting where possible so it tests without a device.

use std::io;
use std::path::{Path, PathBuf};

/// One crash.log line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrashEntry {
    pub unix: i64,
    /// short description: the panic location and message, or the exit reason
    pub signature: String,
    /// the log file holding it (what a tap opens)
    pub file: PathBuf,
}

/// The pieces of `[<unix> hh:mm:ss] thread 'name': msg`.
pub fn parse_line(line: &str) -> Option<(i64, String)> {
    let rest = line.strip_prefix('[')?;
    let (stamp, rest) = rest.split_once(']')?;
    let unix = stamp.split_whitespace().next()?.parse().ok()?;
    let msg = rest.trim().split_once("': ").map_or(rest.trim(), |(_, m)| m);
    Some((unix, msg.to_string()))
}

/// "src/x.rs:12 boom" for a panic line, the message itself otherwise; at most 60 characters.
pub fn signature(msg: &str) -> String {
    let sig = match msg.strip_prefix("panic: ").and_then(|m| m.rsplit_once(" at ")) {
        Some((what, loc)) => {
            let loc = loc.rsplitn(2, ':').nth(1).unwrap_or(loc);
            format!("{loc} {what}")
        }
        None => msg.to_string(),
    };
    if sig.chars().count() > 60 {
        format!("{}…", sig.chars().take(59).collect::<String>())
    } else {
        sig
    }
}

/// Entries of one log file, oldest first; lines that do not parse are skipped.
pub fn parse_log(text: &str, file: &Path) -> Vec<CrashEntry> {
    text.lines().filter_map(parse_line).map(|(unix, msg)| CrashEntry { unix, signature: signature(&msg), file: file.to_path_buf() }).collect()
}

/// Every crash in `dir`, newest first (crash.log, then the rotated crash.log.old).
pub fn load(dir: &Path) -> Vec<CrashEntry> {
    let mut all = Vec::new();
    for name in ["crash.log.old", "crash.log"] {
        let path = dir.join(name);
        if let Ok(text) = std::fs::read_to_string(&path) {
            all.extend(parse_log(&text, &path));
        }
    }
    all.sort_by_key(|e| std::cmp::Reverse(e.unix));
    all
}

/// Removes every crash log; the count of files removed.
pub fn delete_all(dir: &Path) -> usize {
    ["crash.log", "crash.log.old"].iter().filter(|n| std::fs::remove_file(dir.join(n)).is_ok()).count()
}

/// "Oct 4, 14:05" for a Unix time in the zone `offset` seconds east of UTC.
pub fn date_text(unix: i64, offset: i32) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (_, mon, day, h, m) = crate::files::fs::civil(unix + offset as i64);
    format!("{} {day}, {h:02}:{m:02}", MONTHS[(mon as usize).saturating_sub(1).min(11)])
}

/// The text shared with the developer: build stamp, then each diagnostic file.
pub fn bundle(dir: &Path, build: &str, now: i64) -> String {
    let mut out = format!("springboard diagnostics\nbuild {build}\ntime {now}\n");
    for name in ["last_exit", "last_start", "heartbeat", "crash.log.old", "crash.log"] {
        let body = std::fs::read_to_string(dir.join(name)).unwrap_or_default();
        out.push_str(&format!("\n== {name} ==\n{}", body.trim_end()));
        out.push('\n');
    }
    out
}

/// The upload URL on the update server for a bundle called `name`: next to the update manifest.
pub fn upload_url(update_url: &str, name: &str) -> String {
    let from = update_url.find("://").map_or(0, |i| i + 3);
    let host_end = update_url[from..].find('/').map_or(update_url.len(), |i| from + i);
    format!("{}/diag/{name}", &update_url[..host_end])
}

/// POSTs `body` to `url`; Ok on a 2xx answer. HTTPS only.
pub fn post(url: &str, body: &[u8]) -> io::Result<()> {
    if !url.starts_with("https://") {
        return Err(io::Error::other("diagnostics upload requires https://"));
    }
    let code = super::store::net::post_text(url, body)?;
    if (200..300).contains(&code) {
        Ok(())
    } else {
        Err(io::Error::other(format!("server answered {code}")))
    }
}

// -- performance HUD -----------------------------------------------------------

/// What the HUD shows once a second.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sensors {
    /// hottest thermal zone, degrees C
    pub temp_c: Option<f32>,
    /// MemAvailable and MemTotal in MB
    pub mem_mb: Option<(u64, u64)>,
    /// battery current in mA, negative while discharging
    pub current_ma: Option<i32>,
}

/// The highest value among thermal zones given in millidegrees, as degrees.
pub fn max_temp(milli: &[i64]) -> Option<f32> {
    milli.iter().copied().filter(|m| (-40_000..150_000).contains(m)).max().map(|m| m as f32 / 1000.0)
}

/// (available, total) MB from /proc/meminfo text.
pub fn parse_mem(text: &str) -> Option<(u64, u64)> {
    let kb = |key: &str| -> Option<u64> { text.lines().find_map(|l| l.strip_prefix(key)?.split_whitespace().next()?.parse().ok()) };
    Some((kb("MemAvailable:")? / 1024, kb("MemTotal:")? / 1024))
}

/// Battery current_now (microamps) as milliamps.
pub fn current_ma(micro: i64) -> i32 {
    (micro / 1000) as i32
}

impl Sensors {
    /// Reads sysfs and procfs.
    pub fn read() -> Self {
        let mut temps = Vec::new();
        if let Ok(rd) = std::fs::read_dir("/sys/class/thermal") {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().starts_with("thermal_zone") {
                    if let Some(v) = std::fs::read_to_string(e.path().join("temp")).ok().and_then(|s| s.trim().parse::<i64>().ok()) {
                        temps.push(v);
                    }
                }
            }
        }
        Sensors {
            temp_c: max_temp(&temps),
            mem_mb: std::fs::read_to_string("/proc/meminfo").ok().and_then(|t| parse_mem(&t)),
            current_ma: std::fs::read_to_string("/sys/class/power_supply/battery/current_now").ok().and_then(|s| s.trim().parse().ok()).map(current_ma),
        }
    }
}

/// Frame statistics over the last second.
#[derive(Clone, Debug, Default)]
pub struct PerfHud {
    pub enabled: bool,
    frames: u32,
    draw_sum: f64,
    window_start: f64,
    /// results of the last full window
    pub fps: f32,
    pub frame_ms: f32,
    pub sensors: Sensors,
}

impl PerfHud {
    /// Records one drawn frame that took `draw_ms`.
    pub fn frame(&mut self, draw_ms: f64) {
        self.frames += 1;
        self.draw_sum += draw_ms;
    }

    /// Rolls the window once a second (reading the sensors); true when the numbers changed.
    pub fn roll(&mut self, t: f64) -> bool {
        if !self.enabled {
            self.window_start = t;
            return false;
        }
        let span = t - self.window_start;
        if span < 1000.0 {
            return false;
        }
        self.fps = (self.frames as f64 * 1000.0 / span) as f32;
        self.frame_ms = if self.frames > 0 { (self.draw_sum / self.frames as f64) as f32 } else { 0.0 };
        self.frames = 0;
        self.draw_sum = 0.0;
        self.window_start = t;
        self.sensors = Sensors::read();
        true
    }

    /// When the next window ends, while enabled.
    pub fn next_wake(&self) -> Option<f64> {
        self.enabled.then_some(self.window_start + 1000.0)
    }

    pub fn lines(&self) -> Vec<String> {
        let s = &self.sensors;
        vec![
            format!("{:.1} ms  {:.0} fps", self.frame_ms, self.fps),
            s.temp_c.map_or("temp --".into(), |c| format!("temp {c:.1} C")),
            s.mem_mb.map_or("mem --".into(), |(free, total)| format!("mem {} / {total} MB", total.saturating_sub(free))),
            s.current_ma.map_or("batt --".into(), |ma| format!("batt {ma} mA")),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_panic_line() {
        let line = "[1790000000 14:05:09] thread 'main': panic: index out of bounds at src/shell/mod.rs:812:9";
        let (unix, msg) = parse_line(line).unwrap();
        assert_eq!(unix, 1_790_000_000);
        assert_eq!(signature(&msg), "src/shell/mod.rs:812 index out of bounds");
    }

    #[test]
    fn watchdog_lines_keep_their_text_and_long_ones_are_cut() {
        let (_, msg) = parse_line("[5 00:00:05] thread 'watchdog': ui loop stalled for 21s").unwrap();
        assert_eq!(signature(&msg), "ui loop stalled for 21s");
        assert_eq!(signature(&"x".repeat(100)).chars().count(), 60);
        assert!(parse_line("garbage").is_none());
    }

    #[test]
    fn entries_come_back_newest_first_across_both_files() {
        let dir = std::env::temp_dir().join("sb-diag-load");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("crash.log.old"), "[10 00:00:10] thread 'a': old\n").unwrap();
        std::fs::write(dir.join("crash.log"), "[20 00:00:20] thread 'a': mid\n[30 00:00:30] thread 'a': new\n").unwrap();
        let all = load(&dir);
        assert_eq!(all.iter().map(|e| e.signature.as_str()).collect::<Vec<_>>(), ["new", "mid", "old"]);
        assert_eq!(all[2].file, dir.join("crash.log.old"));
        assert_eq!(delete_all(&dir), 2);
        assert!(load(&dir).is_empty());
    }

    #[test]
    fn bundle_names_the_build_and_every_file() {
        let dir = std::env::temp_dir().join("sb-diag-bundle");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("crash.log"), "[1 00:00:01] thread 'a': x\n").unwrap();
        let b = bundle(&dir, "0.1.0 abc123", 99);
        assert!(b.contains("build 0.1.0 abc123") && b.contains("== crash.log ==\n[1 00:00:01]") && b.contains("== heartbeat =="));
    }

    #[test]
    fn upload_url_sits_next_to_the_manifest() {
        assert_eq!(upload_url("http://192.168.1.12:8190/springboard/latest.json", "d.txt"), "http://192.168.1.12:8190/diag/d.txt");
    }

    #[test]
    fn sensors_parse() {
        assert_eq!(max_temp(&[41_000, 52_500, -274_000]), Some(52.5));
        assert_eq!(max_temp(&[]), None);
        assert_eq!(parse_mem("MemTotal:  3800000 kB\nMemFree: 1 kB\nMemAvailable: 1900000 kB\n"), Some((1855, 3710)));
        assert_eq!(current_ma(-412_000), -412);
    }

    #[test]
    fn hud_rolls_a_window_per_second() {
        let mut h = PerfHud { enabled: true, ..PerfHud::default() };
        assert!(!h.roll(500.0));
        h.frame(4.0);
        h.frame(6.0);
        assert!(h.roll(1000.0));
        assert_eq!(h.frame_ms, 5.0);
        assert_eq!(h.fps, 2.0);
        assert_eq!(h.next_wake(), Some(2000.0));
        h.enabled = false;
        assert_eq!(h.next_wake(), None);
    }

    #[test]
    fn date_reads_like_the_bifrost() {
        assert_eq!(date_text(0, 0), "Jan 1, 00:00");
    }
}
