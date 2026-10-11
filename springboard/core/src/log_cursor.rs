//! Persistent positions in a sequence of log entries and an enumerator over them (`OSLogPosition`,
//! `OSLogEnumerator` with its reverse option, `OSLogStore.getEntries(at:)`).
//!
//! A `Position` is a point in time plus an ordinal that tells apart entries stamped with the same microsecond. It
//! encodes to a short string, so a reader can save where it stopped and resume after it on the next run. The
//! ordinals are assigned from the order of the sequence given to `Enumerator::new`; a position is stable as long as
//! entries are only appended after the saved point and never inserted before it.
//!
//! Enumeration is pure: it works on a `Vec<Entry>` already read by `log.rs` (the in-process ring or the journal).
use crate::log_entries::Entry;
use std::time::{SystemTime, UNIX_EPOCH};

/// A point in a sequence (`OSLogPosition`): microseconds since the epoch, then the ordinal among equal times.
/// The derived order is the sequence order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    micros: u128,
    ordinal: u32,
}

impl Position {
    /// Before every entry.
    pub fn start() -> Position {
        Position { micros: 0, ordinal: 0 }
    }

    /// After every entry.
    pub fn end() -> Position {
        Position { micros: u128::MAX, ordinal: u32::MAX }
    }

    /// The position of an entry dated `date` that is the `ordinal`-th (from 1) entry at that microsecond.
    pub fn new(date: SystemTime, ordinal: u32) -> Position {
        let micros = date.duration_since(UNIX_EPOCH).map(|d| d.as_micros()).unwrap_or(0);
        Position { micros, ordinal }
    }

    /// A short string that can be saved and read back: `<micros>.<ordinal>`.
    pub fn encode(&self) -> String {
        format!("{}.{}", self.micros, self.ordinal)
    }

    /// Reads a string made by `encode`. Anything else is `None`.
    pub fn parse(s: &str) -> Option<Position> {
        let (micros, ordinal) = s.trim().split_once('.')?;
        Some(Position { micros: micros.parse().ok()?, ordinal: ordinal.parse().ok()? })
    }
}

/// `OSLogEnumerator.Options`: which way to walk the sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Oldest first.
    Forward,
    /// Newest first (`reverse`).
    Reverse,
}

/// Walks a sequence of entries from a position, in either direction.
#[derive(Clone, Debug)]
pub struct Enumerator {
    items: Vec<(Position, Entry)>,
    direction: Direction,
}

impl Enumerator {
    /// Takes the entries in any order, sorts them oldest first (stable for equal times) and gives each its position.
    pub fn new(mut entries: Vec<Entry>, direction: Direction) -> Enumerator {
        entries.sort_by_key(|e| e.date());
        let mut items = Vec::with_capacity(entries.len());
        let mut last: Option<Position> = None;
        for e in entries {
            let date = e.date();
            let ordinal = match last {
                Some(p) if p.micros == Position::new(date, 0).micros => p.ordinal + 1,
                _ => 1,
            };
            let pos = Position::new(date, ordinal);
            last = Some(pos);
            items.push((pos, e));
        }
        Enumerator { items, direction }
    }

    pub fn direction(&self) -> Direction {
        self.direction
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The position of every entry, oldest first.
    pub fn positions(&self) -> Vec<Position> {
        self.items.iter().map(|(p, _)| *p).collect()
    }

    /// Every entry in the enumerator's direction.
    pub fn all(&self) -> Vec<&Entry> {
        let start = match self.direction {
            Direction::Forward => Position::start(),
            Direction::Reverse => Position::end(),
        };
        self.entries_after(start)
    }

    /// The entries that come after `pos` in the enumerator's direction. Forward gives the newer entries, oldest
    /// first; reverse gives the older ones, newest first. The point itself is not included, so a reader resumes
    /// after the last position it saved.
    pub fn entries_after(&self, pos: Position) -> Vec<&Entry> {
        match self.direction {
            Direction::Forward => self.items.iter().filter(|(p, _)| *p > pos).map(|(_, e)| e).collect(),
            Direction::Reverse => self.items.iter().rev().filter(|(p, _)| *p < pos).map(|(_, e)| e).collect(),
        }
    }

    /// The position of the last entry in the enumerator's direction: where a reader stopped after reading all.
    pub fn last_position(&self) -> Option<Position> {
        match self.direction {
            Direction::Forward => self.items.last().map(|(p, _)| *p),
            Direction::Reverse => self.items.first().map(|(p, _)| *p),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::{Level, LogEntry};
    use crate::log_entries::{Boundary, Payload, Argument};
    use std::time::Duration;

    fn at(micros: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_micros(micros)
    }

    fn msg(micros: u64, text: &str) -> Entry {
        Entry::Log(LogEntry {
            date: at(micros),
            process: "notes".into(),
            pid: 7,
            subsystem: "is.olibuijr.notes".into(),
            category: "sync".into(),
            level: Level::Notice,
            message: text.into(),
        })
    }

    fn text(e: &Entry) -> String {
        match e {
            Entry::Log(l) => l.message.clone(),
            Entry::Payload(p) => p.entry.message.clone(),
            _ => String::new(),
        }
    }

    #[test]
    fn positions_round_trip_through_their_string_form() {
        let p = Position::new(at(1_500_000), 3);
        assert_eq!(p.encode(), "1500000.3");
        assert_eq!(Position::parse(&p.encode()), Some(p));
        assert_eq!(Position::parse(" 1500000.3 "), Some(p));
        assert_eq!(Position::parse("1500000"), None);
        assert_eq!(Position::parse("x.1"), None);
        assert!(Position::start() < p && p < Position::end());
    }

    #[test]
    fn equal_times_are_told_apart_by_ordinal() {
        let e = Enumerator::new(vec![msg(5, "a"), msg(5, "b"), msg(9, "c")], Direction::Forward);
        let ps = e.positions();
        assert_eq!(ps.len(), 3);
        assert!(ps[0] < ps[1] && ps[1] < ps[2]);
        assert_eq!(ps[1].ordinal, 2);
        assert_eq!(ps[2].ordinal, 1);
    }

    #[test]
    fn forward_walk_gives_newer_entries_oldest_first() {
        let e = Enumerator::new(vec![msg(30, "c"), msg(10, "a"), msg(20, "b")], Direction::Forward);
        let got: Vec<String> = e.all().iter().map(|x| text(x)).collect();
        assert_eq!(got, ["a", "b", "c"]);
        let after_a = e.positions()[0];
        let got: Vec<String> = e.entries_after(after_a).iter().map(|x| text(x)).collect();
        assert_eq!(got, ["b", "c"]);
    }

    #[test]
    fn resuming_from_a_saved_position_returns_only_what_is_new() {
        let first = Enumerator::new(vec![msg(10, "a"), msg(20, "b")], Direction::Forward);
        let saved = first.last_position().unwrap().encode();
        assert!(first.entries_after(Position::parse(&saved).unwrap()).is_empty());

        let later = Enumerator::new(vec![msg(10, "a"), msg(20, "b"), msg(25, "c")], Direction::Forward);
        let got: Vec<String> = later.entries_after(Position::parse(&saved).unwrap()).iter().map(|x| text(x)).collect();
        assert_eq!(got, ["c"]);
    }

    #[test]
    fn reverse_walk_gives_newest_first_and_older_entries_after_a_point() {
        let e = Enumerator::new(vec![msg(10, "a"), msg(20, "b"), msg(30, "c")], Direction::Reverse);
        let got: Vec<String> = e.all().iter().map(|x| text(x)).collect();
        assert_eq!(got, ["c", "b", "a"]);
        let from_c = e.positions()[2];
        let got: Vec<String> = e.entries_after(from_c).iter().map(|x| text(x)).collect();
        assert_eq!(got, ["b", "a"]);
        assert_eq!(e.last_position(), Some(e.positions()[0]));
    }

    #[test]
    fn boundary_and_payload_entries_are_enumerated_with_plain_entries() {
        let payload = Payload::new(
            match msg(15, "") {
                Entry::Log(l) => l,
                _ => unreachable!(),
            },
            "saved %{public}s",
            vec![Argument::Text("doc".into())],
        )
        .unwrap();
        let b = Entry::Boundary(Boundary { date: at(12), process: "notes".into(), pid: 7 });
        let e = Enumerator::new(vec![msg(20, "tail"), Entry::Payload(payload), b], Direction::Forward);
        assert_eq!(e.len(), 3);
        let kinds: Vec<bool> = e.all().iter().map(|x| matches!(x, Entry::Payload(_))).collect();
        assert_eq!(kinds, [false, true, false]);
        assert_eq!(text(e.all()[1]), "saved doc");
    }

    #[test]
    fn empty_and_bounded_walks() {
        let e = Enumerator::new(Vec::new(), Direction::Forward);
        assert!(e.is_empty());
        assert!(e.all().is_empty());
        assert_eq!(e.last_position(), None);
        let one = Enumerator::new(vec![msg(1, "x")], Direction::Forward);
        assert_eq!(one.entries_after(Position::end()).len(), 0);
        assert_eq!(one.entries_after(Position::start()).len(), 1);
    }
}
