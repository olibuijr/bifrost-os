//! Memory pressure for apps (RunningBoard's memory-pressure events): reads the kernel's pressure stall information
//! for memory (`/proc/pressure/memory`) and turns it into a level apps are told about once, when it changes.
//! The file has two lines, `some` (at least one task waited for memory) and `full` (every non-idle task waited), each
//! with `avg10`, `avg60`, `avg300` percentages and a total. Pure parsing and level logic; `read` is the only I/O.

/// The kernel's memory pressure file.
pub const PRESSURE_FILE: &str = "/proc/pressure/memory";

/// The 10-second averages of the two stall figures, in percent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Psi {
    pub some_avg10: f32,
    pub full_avg10: f32,
}

/// How much memory pressure there is. Apps are told the level when it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Normal,
    /// Some task is waiting for memory a noticeable part of the time.
    Warning,
    /// Every running task is waiting for memory; apps should free what they can now.
    Critical,
}

/// Starting points for the thresholds, in percent of the last ten seconds.
pub const WARNING_SOME: f32 = 10.0;
pub const CRITICAL_SOME: f32 = 40.0;
pub const CRITICAL_FULL: f32 = 5.0;

/// The `avg10` value of one `some`/`full` line, or `None` for a line that is not one.
fn avg10_of(line: &str, prefix: &str) -> Option<f32> {
    let rest = line.strip_prefix(prefix)?.trim_start();
    let field = rest.split_whitespace().find_map(|f| f.strip_prefix("avg10="))?;
    field.parse::<f32>().ok().filter(|v| v.is_finite() && *v >= 0.0)
}

/// Parses the text of `/proc/pressure/memory`. Both lines must be present and valid, else `None`.
pub fn parse(text: &str) -> Option<Psi> {
    let mut some = None;
    let mut full = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("some") {
            some = avg10_of(line, "some");
        } else if line.starts_with("full") {
            full = avg10_of(line, "full");
        }
    }
    Some(Psi { some_avg10: some?, full_avg10: full? })
}

/// The level for one reading. Critical when the whole system stalls or the stall is heavy; warning when some task does.
pub fn level(psi: Psi) -> Level {
    if psi.some_avg10 >= CRITICAL_SOME || psi.full_avg10 >= CRITICAL_FULL {
        Level::Critical
    } else if psi.some_avg10 >= WARNING_SOME {
        Level::Warning
    } else {
        Level::Normal
    }
}

/// Reports a level once per change: the first reading and every reading that differs from the last reported one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reporter {
    last: Option<Level>,
}

impl Reporter {
    pub fn new() -> Reporter {
        Reporter { last: None }
    }

    /// The level to tell apps about for this reading, or `None` when it is the same as the last one told.
    pub fn observe(&mut self, psi: Psi) -> Option<Level> {
        let now = level(psi);
        if self.last == Some(now) {
            return None;
        }
        self.last = Some(now);
        Some(now)
    }
}

/// Reads the live pressure, or `None` when the kernel has no pressure file (no PSI support) or it cannot be parsed.
pub fn read() -> Option<Psi> {
    parse(&std::fs::read_to_string(PRESSURE_FILE).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";

    fn psi(some: f32, full: f32) -> Psi {
        Psi { some_avg10: some, full_avg10: full }
    }

    #[test]
    fn the_kernel_file_parses_to_both_averages() {
        let p = parse("some avg10=12.34 avg60=1.00 avg300=0.50 total=99\nfull avg10=0.75 avg60=0.10 avg300=0.01 total=4\n");
        assert_eq!(p, Some(psi(12.34, 0.75)));
        assert_eq!(parse(SAMPLE), Some(psi(0.0, 0.0)));
    }

    #[test]
    fn a_file_missing_a_line_or_with_junk_is_refused() {
        assert_eq!(parse("some avg10=1.00 avg60=0 avg300=0 total=1\n"), None, "no full line");
        assert_eq!(parse("some avg10=abc\nfull avg10=0.0\n"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("some avg10=-1.0\nfull avg10=0.0\n"), None, "negative stall is not a reading");
    }

    #[test]
    fn the_level_rises_with_the_stall() {
        assert_eq!(level(psi(0.0, 0.0)), Level::Normal);
        assert_eq!(level(psi(WARNING_SOME, 0.0)), Level::Warning);
        assert_eq!(level(psi(1.0, CRITICAL_FULL)), Level::Critical, "everything stalled is critical");
        assert_eq!(level(psi(CRITICAL_SOME, 0.0)), Level::Critical);
        assert!(Level::Critical > Level::Warning && Level::Warning > Level::Normal);
    }

    #[test]
    fn apps_hear_a_level_once_per_change() {
        let mut r = Reporter::new();
        assert_eq!(r.observe(psi(0.0, 0.0)), Some(Level::Normal), "the first reading is always told");
        assert_eq!(r.observe(psi(1.0, 0.0)), None, "still normal");
        assert_eq!(r.observe(psi(15.0, 0.0)), Some(Level::Warning));
        assert_eq!(r.observe(psi(16.0, 0.0)), None);
        assert_eq!(r.observe(psi(0.0, 0.0)), Some(Level::Normal), "back to normal is told too");
    }
}
