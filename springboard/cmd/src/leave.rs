//! `leave`: waits until a time and then rings the bell and prints a reminder. The reference tool writes the reminder to
//! the user's terminals through utmp; here it goes to the terminal the command runs in, so run it in the background
//! (`leave 1730 &`) or in a terminal that stays open.
//! Supported: `hhmm` (a clock time today, local time) and `+hhmm` or `+mm` (a delay from now: the last two digits are
//! minutes, the rest hours, so `+5` is five minutes and `+130` is one hour and thirty minutes).
//! Refused: a clock time that has already passed, and a delay of zero.
//! Gap: the reference tool's interactive prompt (no argument) and its warning messages before the time are not
//! reproduced, and `hhmm` with seconds (`hhmmss`) is not accepted.

use std::process::Command;
use std::time::Duration;

pub const USAGE: &str = "usage: leave [+]hhmm";
/// What is printed when the time comes: the terminal bell, then the reminder.
pub const REMINDER: &str = "\x07Time to leave.";

/// A parsed time: a clock time today, or a delay from now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spec {
    /// `hhmm`: a local clock time.
    At { hour: u32, minute: u32 },
    /// `+...`: minutes from now (the total of the hours and minutes given).
    After { minutes: u32 },
}

/// Reads the one time argument.
pub fn parse(arg: &str) -> Result<Spec, String> {
    if let Some(rest) = arg.strip_prefix('+') {
        if rest.is_empty() || rest.len() > 4 || !rest.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("bad delay {arg}: give +hhmm with at most four digits"));
        }
        let padded = format!("{rest:0>4}");
        let hours: u32 = padded[..2].parse().unwrap_or(0);
        let minutes: u32 = padded[2..].parse().unwrap_or(0);
        let total = hours * 60 + minutes;
        if total == 0 {
            return Err("the delay is zero: nothing to wait for".into());
        }
        return Ok(Spec::After { minutes: total });
    }
    if arg.len() != 4 || !arg.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("bad time {arg}: give hhmm, for example 1730"));
    }
    let hour: u32 = arg[..2].parse().unwrap_or(99);
    let minute: u32 = arg[2..].parse().unwrap_or(99);
    if hour > 23 || minute > 59 {
        return Err(format!("bad time {arg}: the hour is 00 to 23 and the minute 00 to 59"));
    }
    Ok(Spec::At { hour, minute })
}

/// How long to wait for `spec`. `now_of_day` is the local time as seconds since midnight; it is only read for a clock
/// time, so a delay never needs the clock.
pub fn delay(spec: Spec, now_of_day: impl FnOnce() -> Result<u32, String>) -> Result<Duration, String> {
    match spec {
        Spec::After { minutes } => Ok(Duration::from_secs(u64::from(minutes) * 60)),
        Spec::At { hour, minute } => {
            let target = hour * 3600 + minute * 60;
            let now = now_of_day()?;
            if target <= now {
                return Err(format!("{hour:02}{minute:02} has already passed"));
            }
            Ok(Duration::from_secs(u64::from(target - now)))
        }
    }
}

/// Reads `HH:MM:SS` (the output of `date +%H:%M:%S`) as seconds since midnight.
pub fn parse_clock(text: &str) -> Result<u32, String> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.len() != 3 {
        return Err(format!("unexpected clock output {}", text.trim()));
    }
    let num = |p: &str| p.parse::<u32>().map_err(|_| format!("unexpected clock output {}", text.trim()));
    let (h, m, s) = (num(parts[0])?, num(parts[1])?, num(parts[2])?);
    if h > 23 || m > 59 || s > 60 {
        return Err(format!("unexpected clock output {}", text.trim()));
    }
    Ok(h * 3600 + m * 60 + s)
}

/// The local time of day, read from `date`, which knows the time zone.
pub fn now_of_day() -> Result<u32, String> {
    let out = Command::new("date").arg("+%H:%M:%S").output().map_err(|e| format!("date could not run: {e}"))?;
    if !out.status.success() {
        return Err("date could not read the clock".into());
    }
    parse_clock(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_time_parses_hours_and_minutes() {
        assert_eq!(parse("1730"), Ok(Spec::At { hour: 17, minute: 30 }));
        assert_eq!(parse("0000"), Ok(Spec::At { hour: 0, minute: 0 }));
        assert_eq!(parse("2359"), Ok(Spec::At { hour: 23, minute: 59 }));
    }

    #[test]
    fn clock_time_out_of_range_or_malformed_is_refused() {
        assert!(parse("2400").is_err());
        assert!(parse("1260").is_err());
        assert!(parse("930").is_err());
        assert!(parse("17:30").is_err());
        assert!(parse("abcd").is_err());
    }

    #[test]
    fn delay_reads_minutes_from_the_last_two_digits() {
        assert_eq!(parse("+5"), Ok(Spec::After { minutes: 5 }));
        assert_eq!(parse("+130"), Ok(Spec::After { minutes: 90 }));
        assert_eq!(parse("+0130"), Ok(Spec::After { minutes: 90 }));
        assert_eq!(parse("+1200"), Ok(Spec::After { minutes: 720 }));
    }

    #[test]
    fn zero_or_bad_delay_is_refused() {
        assert!(parse("+").is_err());
        assert!(parse("+0").is_err());
        assert!(parse("+00").is_err());
        assert!(parse("+12345").is_err());
        assert!(parse("+1x").is_err());
    }

    #[test]
    fn a_delay_waits_without_reading_the_clock() {
        let never = || -> Result<u32, String> { panic!("the clock must not be read for a delay") };
        assert_eq!(delay(Spec::After { minutes: 5 }, never), Ok(Duration::from_secs(300)));
    }

    #[test]
    fn a_clock_time_waits_until_it_comes() {
        // 09:00:00 now, leave at 10:30: one hour and a half
        let now = || Ok(9 * 3600);
        assert_eq!(delay(Spec::At { hour: 10, minute: 30 }, now), Ok(Duration::from_secs(5400)));
    }

    #[test]
    fn a_clock_time_already_passed_is_refused() {
        let now = || Ok(10 * 3600 + 31 * 60);
        assert!(delay(Spec::At { hour: 10, minute: 30 }, now).is_err());
        // the same minute is not a future time either
        let same = || Ok(10 * 3600 + 30 * 60);
        assert!(delay(Spec::At { hour: 10, minute: 30 }, same).is_err());
    }

    #[test]
    fn date_output_becomes_seconds_of_day() {
        assert_eq!(parse_clock("09:05:07\n"), Ok(9 * 3600 + 5 * 60 + 7));
        assert_eq!(parse_clock("23:59:59"), Ok(86399));
        assert!(parse_clock("09:05").is_err());
        assert!(parse_clock("25:00:00").is_err());
        assert!(parse_clock("").is_err());
    }
}
