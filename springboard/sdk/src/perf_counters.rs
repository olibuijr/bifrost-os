//! Hardware and software performance counters through the Linux `perf` tool (the `perf` package): an app names events,
//! runs a program under `perf stat`, and gets the counts back. This is the SDK face of the counter library the reference
//! reads through its kernel bridge; the counters come from `perf_event_open`, which `perf` calls.
//!
//! `perf stat` writes its report to standard error in CSV form (`-x,`), so the program's own output is left alone. The
//! parser is pure and tested. Counters the machine does not offer come back as `None`, not as zero.

use std::fmt;
use std::process::Command;

/// One counter from a run: its value (None when the machine did not count it), its unit (empty for plain counts) and
/// the event name as `perf` prints it, such as `cycles` or `task-clock`.
#[derive(Clone, Debug, PartialEq)]
pub struct Counter {
    pub value: Option<f64>,
    pub unit: String,
    pub event: String,
}

/// Why a measurement failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PerfError {
    /// No event was named, or an event name holds a character `perf` does not take in a name.
    BadEvent(String),
    /// The program name is empty or holds a line break.
    BadProgram,
    /// `perf` could not be started (the `perf` package is missing).
    Spawn(String),
    /// `perf` ran but printed no counter lines; the text is its standard error.
    Failed(String),
}

impl fmt::Display for PerfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PerfError::BadEvent(name) => write!(f, "bad performance event name {name:?}"),
            PerfError::BadProgram => f.write_str("the program to measure is empty"),
            PerfError::Spawn(reason) => write!(f, "could not run perf: {reason}"),
            PerfError::Failed(text) => f.write_str(text),
        }
    }
}

impl std::error::Error for PerfError {}

/// An event name: letters, digits and `-_:.=/`, the characters `perf` event names use (`cpu/cycles/`, `r0c4`, `task-clock`).
fn valid_event(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_:.=/".contains(c))
}

/// The `perf stat` command: CSV output, the named events, then the program and its arguments after `--`.
pub fn stat_command(events: &[&str], program: &str, args: &[&str]) -> Result<Command, PerfError> {
    if events.is_empty() {
        return Err(PerfError::BadEvent(String::new()));
    }
    if let Some(bad) = events.iter().find(|e| !valid_event(e)) {
        return Err(PerfError::BadEvent((*bad).to_string()));
    }
    if program.is_empty() || program.contains(['\n', '\r', '\0']) {
        return Err(PerfError::BadProgram);
    }
    let mut cmd = Command::new("perf");
    let names = events.join(",");
    cmd.args(["stat", "-x,", "-e"]).arg(names);
    cmd.arg("--").arg(program).args(args);
    Ok(cmd)
}

/// Reads the CSV counter lines from `perf stat` standard error. A line is `value,unit,event,...`; a value that is not a
/// number (`<not counted>`, `<not supported>`) gives a counter with no value. Lines that are not counters are ignored.
pub fn parse_stat_csv(text: &str) -> Vec<Counter> {
    let mut out = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() < 3 {
            continue;
        }
        let event = fields[2].trim();
        if event.is_empty() {
            continue;
        }
        let raw = fields[0].trim();
        let value = if raw.starts_with('<') {
            None
        } else {
            match raw.parse::<f64>() {
                Ok(v) => Some(v),
                Err(_) => continue, // not a counter line
            }
        };
        out.push(Counter { value, unit: fields[1].trim().to_string(), event: event.to_string() });
    }
    out
}

/// Runs `program args` under `perf stat` for the events and returns the counters in the order `perf` reports them.
pub fn measure(events: &[&str], program: &str, args: &[&str]) -> Result<Vec<Counter>, PerfError> {
    let out = stat_command(events, program, args)?
        .output()
        .map_err(|e| PerfError::Spawn(e.to_string()))?;
    let text = String::from_utf8_lossy(&out.stderr);
    let counters = parse_stat_csv(&text);
    if counters.is_empty() {
        return Err(PerfError::Failed(text.trim().to_string()));
    }
    Ok(counters)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = "\
1523.12,msec,task-clock,1523120000,100.00,,
2048000,,cycles,1523120000,100.00,,
<not supported>,,r0c4,0,0.00,,
Performance counter stats for 'ls':
";

    #[test]
    fn counter_lines_parse_with_their_units_and_events() {
        let c = parse_stat_csv(REPORT);
        assert_eq!(c[0], Counter { value: Some(1523.12), unit: "msec".into(), event: "task-clock".into() });
        assert_eq!(c[1], Counter { value: Some(2048000.0), unit: String::new(), event: "cycles".into() });
    }

    #[test]
    fn a_counter_the_machine_does_not_offer_has_no_value_not_zero() {
        let c = parse_stat_csv(REPORT);
        let missing = c.iter().find(|c| c.event == "r0c4").unwrap();
        assert_eq!(missing.value, None);
    }

    #[test]
    fn prose_lines_are_skipped() {
        assert!(parse_stat_csv("Performance counter stats for 'ls':\n\n").is_empty());
        assert_eq!(parse_stat_csv("1000000,,instructions,1,100.00,,\n")[0].value, Some(1_000_000.0));
    }

    #[test]
    fn stat_command_names_the_events_and_puts_the_program_after_a_separator() {
        let cmd = stat_command(&["cycles", "task-clock"], "ls", &["-l"]).unwrap();
        assert_eq!(cmd.get_program(), "perf");
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, vec!["stat", "-x,", "-e", "cycles,task-clock", "--", "ls", "-l"]);
    }

    #[test]
    fn bad_events_and_programs_are_refused_before_anything_runs() {
        assert_eq!(stat_command(&[], "ls", &[]).err(), Some(PerfError::BadEvent(String::new())));
        assert_eq!(stat_command(&["cycles;rm"], "ls", &[]).err(), Some(PerfError::BadEvent("cycles;rm".into())));
        assert_eq!(stat_command(&["cycles"], "", &[]).err(), Some(PerfError::BadProgram));
        assert!(valid_event("cpu/cycles/"));
        assert!(valid_event("r0c4"));
    }
}
