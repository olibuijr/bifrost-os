//! Signposts in the log: the intervals and events the SDK writes (`bifrost_app::signpost`, as the messages
//! "interval NAME id=N MICROS us" and "event NAME id=N"), read back into `bifrost_core::signpost` values so the
//! Signposts view can total their durations with `Summary` (count, total and longest per name). The Console also
//! times its own loads through the same signposter, so its work shows here too.

use crate::model::LogEntry;
use bifrost_app::signpost::{Id, Interval, Signposter, Summary};

/// The subsystem the Console logs its own signposts under (declared in manifest.toml).
pub const SUBSYSTEM: &str = "is.olibuijr.console.desktop";
/// The name of the Console's own timed load of a view.
pub const LOAD: &str = "view.load";

/// One signpost message, read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mark {
    /// A finished interval; `duration_ns()` is its length.
    Interval(Interval),
    /// A point of interest.
    Event { name: String, id: Id },
}

/// The signpost a message records, or None for an ordinary message.
pub fn parse(message: &str) -> Option<Mark> {
    let mut words = message.split_whitespace();
    match words.next()? {
        "interval" => {
            let name = words.next()?;
            let id = words.next()?.strip_prefix("id=")?.parse::<u64>().ok()?;
            let micros = words.next()?.parse::<u64>().ok()?;
            if words.next()? != "us" {
                return None;
            }
            Some(Mark::Interval(Interval { name: name.to_string(), id: Id(id), begun_ns: 0, ended_ns: micros.saturating_mul(1_000) }))
        }
        "event" => {
            let name = words.next()?;
            let id = words.next()?.strip_prefix("id=")?.parse::<u64>().ok()?;
            Some(Mark::Event { name: name.to_string(), id: Id(id) })
        }
        _ => None,
    }
}

/// The totals of every interval among `entries`, per name.
pub fn summarize<'a>(entries: impl Iterator<Item = &'a LogEntry>) -> Summary {
    let mut summary = Summary::default();
    for e in entries {
        if let Some(Mark::Interval(interval)) = parse(&e.message) {
            summary.record(&interval);
        }
    }
    summary
}

/// A duration for people: "850 ns", "42 us" or "3.2 ms".
pub fn duration_text(ns: u64) -> String {
    if ns >= 1_000_000 {
        format!("{:.1} ms", ns as f64 / 1_000_000.0)
    } else if ns >= 1_000 {
        format!("{} us", ns / 1_000)
    } else {
        format!("{ns} ns")
    }
}

/// The footer line for a summary: one part per name, in name order, such as "view.load 2x, 5.1 ms total, longest 3.0 ms".
pub fn summary_line(summary: &Summary) -> String {
    let parts: Vec<String> = summary
        .names()
        .filter_map(|name| summary.stats(name).map(|s| (name.to_string(), s)))
        .map(|(name, s)| format!("{name} {}x, {} total, longest {}", s.count, duration_text(s.total_ns), duration_text(s.max_ns)))
        .collect();
    parts.join(" · ")
}

/// Runs `work` as one interval named `LOAD` under the Console's own signposter, and returns its value with the
/// finished interval (None if the signposter were disabled). The interval also goes to the unified log.
pub fn timed<T>(work: impl FnOnce() -> T) -> (T, Option<Interval>) {
    let mut signposter = Signposter::new(SUBSYSTEM, "signpost");
    let id = signposter.make_id();
    let open = signposter.begin(LOAD, id);
    let out = work();
    let done = signposter.end(open, LOAD).ok().flatten();
    (out, done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Level;

    fn msg(text: &str) -> LogEntry {
        LogEntry { message: text.into(), level: Level::Info, ..LogEntry::default() }
    }

    #[test]
    fn an_interval_message_reads_back_with_its_duration() {
        let Some(Mark::Interval(i)) = parse("interval parse id=42 1500 us") else { panic!("an interval") };
        assert_eq!((i.name.as_str(), i.id, i.duration_ns()), ("parse", Id(42), 1_500_000));
    }

    #[test]
    fn an_event_message_reads_back_with_its_name_and_id() {
        assert_eq!(parse("event tap id=7"), Some(Mark::Event { name: "tap".into(), id: Id(7) }));
    }

    #[test]
    fn ordinary_and_malformed_messages_are_not_signposts() {
        for text in ["shell ready", "interval", "interval parse", "interval parse id=x 5 us", "interval parse id=1 5 ms", "event tap", "event tap 7", ""] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn durations_are_shown_in_the_unit_that_fits() {
        assert_eq!(duration_text(850), "850 ns");
        assert_eq!(duration_text(42_000), "42 us");
        assert_eq!(duration_text(3_200_000), "3.2 ms");
    }

    #[test]
    fn the_summary_totals_intervals_per_name_and_skips_events() {
        let entries = [msg("interval load id=1 10 us"), msg("interval load id=2 30 us"), msg("interval save id=3 5 us"), msg("event load id=4"), msg("hello")];
        let summary = summarize(entries.iter());
        assert_eq!(summary.stats("load").map(|s| (s.count, s.total_ns, s.max_ns)), Some((2, 40_000, 30_000)));
        assert_eq!(summary.stats("save").map(|s| s.count), Some(1));
        assert_eq!(summary.names().collect::<Vec<_>>(), vec!["load", "save"]);
        assert_eq!(summary_line(&summary), "load 2x, 40 us total, longest 30 us · save 1x, 5 us total, longest 5 us");
        assert_eq!(summary_line(&Summary::default()), "");
    }

    #[test]
    fn the_console_times_its_own_load_as_an_interval() {
        let (value, done) = timed(|| 6 * 7);
        assert_eq!(value, 42);
        let done = done.expect("an enabled signposter returns the interval");
        assert_eq!(done.name, LOAD);
        assert!(done.ended_ns >= done.begun_ns);
    }
}
