//! Log positions for the Console window: the position of each message in the unified log's sequence, the walk
//! from a chosen message, and the subsystem filter that goes with it. The models are `bifrost_core::log_cursor`
//! (`Position`, `Enumerator`) and the entry kinds in `bifrost_core::log_entries`; this file converts the window's
//! `LogEntry` into them and back. Positions are only computed on demand (a selection, a menu action, a rebuild in
//! the walk mode), never per frame.

use crate::model::{Level, LogEntry};
use bifrost_core::log::{Level as CoreLevel, LogEntry as CoreEntry};
use bifrost_core::log_cursor::{Direction, Enumerator, Position};
use bifrost_core::log_entries::Entry;
use std::collections::BTreeSet;
use std::time::{Duration, UNIX_EPOCH};

/// The window's entry as the core model's plain log entry (`Entry::Log`), with its date as a system time.
pub fn core_entry(e: &LogEntry) -> Entry {
    Entry::Log(CoreEntry {
        date: UNIX_EPOCH + Duration::from_micros(e.date.max(0) as u64),
        process: e.process.clone(),
        pid: e.pid,
        subsystem: e.subsystem.clone(),
        category: e.category.clone(),
        level: core_level(e.level),
        message: e.message.clone(),
    })
}

fn core_level(level: Level) -> CoreLevel {
    match level {
        Level::Debug => CoreLevel::Debug,
        Level::Info => CoreLevel::Info,
        Level::Notice => CoreLevel::Notice,
        Level::Error => CoreLevel::Error,
        Level::Fault => CoreLevel::Fault,
    }
}

/// Every message with its position, oldest first, as `(position, sequence number)`. `entries` is given in buffer
/// order; the sort is the same stable sort `Enumerator::new` makes (an unknown date, 0, counts as the epoch), so the
/// two orders agree and each position lands on the right sequence number.
pub fn positions(entries: &[(u64, &LogEntry)]) -> Vec<(Position, u64)> {
    let mut order: Vec<(u64, &LogEntry)> = entries.to_vec();
    order.sort_by_key(|(_, e)| e.date.max(0));
    let core: Vec<Entry> = order.iter().map(|(_, e)| core_entry(e)).collect();
    let walk = Enumerator::new(core, Direction::Forward);
    walk.positions().into_iter().zip(order.iter().map(|(seq, _)| *seq)).collect()
}

/// The position of the message with sequence number `seq`, if it is in `positions`.
pub fn position_of(positions: &[(Position, u64)], seq: u64) -> Option<Position> {
    positions.iter().find(|(_, s)| *s == seq).map(|(p, _)| *p)
}

/// The sequence numbers of the messages after `from` in the walk (the newer ones, as the forward walk gives them).
/// `from` itself is not included, so a reader that saved its last position resumes after it.
pub fn after(positions: &[(Position, u64)], from: Position) -> BTreeSet<u64> {
    positions.iter().filter(|(p, _)| *p > from).map(|(_, seq)| *seq).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(date: i64, text: &str) -> LogEntry {
        LogEntry { date, process: "notes".into(), pid: 7, subsystem: "is.olibuijr.notes".into(), category: "sync".into(), level: Level::Notice, message: text.into(), ..LogEntry::default() }
    }

    #[test]
    fn a_window_entry_keeps_its_fields_and_its_micro_second_date() {
        let e = msg(1_500_000, "saved");
        let Entry::Log(core) = core_entry(&e) else { panic!("a plain message") };
        assert_eq!(core.date, UNIX_EPOCH + Duration::from_micros(1_500_000));
        assert_eq!((core.process.as_str(), core.pid, core.subsystem.as_str(), core.category.as_str()), ("notes", 7, "is.olibuijr.notes", "sync"));
        assert_eq!(core.level, CoreLevel::Notice);
        assert_eq!(core.message, "saved");
        let unknown = msg(0, "no date");
        let Entry::Log(core) = core_entry(&unknown) else { panic!() };
        assert_eq!(core.date, UNIX_EPOCH, "a negative or zero date becomes the epoch");
    }

    #[test]
    fn levels_map_one_to_one() {
        let levels = [Level::Debug, Level::Info, Level::Notice, Level::Error, Level::Fault];
        let core: Vec<CoreLevel> = levels.iter().map(|l| core_level(*l)).collect();
        assert_eq!(core, [CoreLevel::Debug, CoreLevel::Info, CoreLevel::Notice, CoreLevel::Error, CoreLevel::Fault]);
    }

    #[test]
    fn equal_times_get_ordinals_and_sequence_numbers_follow_the_date_order() {
        let a = msg(5, "a");
        let b = msg(5, "b");
        let c = msg(9, "c");
        // buffer order is c, a, b: the walk sorts by date and keeps a before b
        let ps = positions(&[(10, &c), (11, &a), (12, &b)]);
        let seqs: Vec<u64> = ps.iter().map(|(_, s)| *s).collect();
        assert_eq!(seqs, vec![11, 12, 10]);
        assert_eq!(ps[0].0.encode(), "5.1");
        assert_eq!(ps[1].0.encode(), "5.2");
        assert_eq!(ps[2].0.encode(), "9.1");
        assert_eq!(position_of(&ps, 12).map(|p| p.encode()), Some("5.2".to_string()));
        assert_eq!(position_of(&ps, 99), None);
    }

    #[test]
    fn the_messages_after_a_position_are_the_newer_ones() {
        let a = msg(10, "a");
        let b = msg(20, "b");
        let c = msg(30, "c");
        let ps = positions(&[(1, &a), (2, &b), (3, &c)]);
        let from_a = position_of(&ps, 1).unwrap();
        let newer: Vec<u64> = after(&ps, from_a).into_iter().collect();
        assert_eq!(newer, vec![2, 3], "the position itself is not included");
        let from_c = position_of(&ps, 3).unwrap();
        assert!(after(&ps, from_c).is_empty(), "nothing is newer than the last message");
        assert_eq!(after(&ps, Position::start()).len(), 3);
    }

    #[test]
    fn a_position_saved_as_text_still_selects_the_same_messages() {
        let a = msg(10, "a");
        let b = msg(20, "b");
        let ps = positions(&[(1, &a), (2, &b)]);
        let saved = position_of(&ps, 1).unwrap().encode();
        let back = Position::parse(&saved).expect("a saved position parses");
        assert_eq!(after(&ps, back).into_iter().collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn an_empty_buffer_has_no_positions() {
        assert!(positions(&[]).is_empty());
        assert!(after(&[], Position::start()).is_empty());
    }
}
