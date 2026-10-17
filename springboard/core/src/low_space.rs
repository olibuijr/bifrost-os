//! Low disk space: classifies a volume's available bytes into a warning level.
//!
//! The shell reads a volume's capacity with `volume::capacity` and asks this module how urgent a shortage is. A
//! volume is `Low` when its available bytes fall below both `LOW_BYTES` and `LOW_FRACTION` of its size, and
//! `Critical` when they fall below both `CRITICAL_BYTES` and `CRITICAL_FRACTION`. Taking the smaller of the two
//! limits lets a small volume be judged by its fraction and a large one by its bytes. The module does no I/O and reads
//! no clock; `notify_center::VOLUME_LOW_DISK_SPACE` is the name a shell posts when the level is not `Ok`.

use crate::volume::Capacity;

const GIB: u64 = 1024 * 1024 * 1024;

/// A volume with less available space than this is `Low`, unless its fraction limit is smaller.
pub const LOW_BYTES: u64 = 5 * GIB;
/// A volume with less available space than this is `Critical`, unless its fraction limit is smaller.
pub const CRITICAL_BYTES: u64 = GIB / 2;
/// The share of the volume that must stay available before it is `Low`.
pub const LOW_FRACTION: f64 = 0.10;
/// The share of the volume that must stay available before it is `Critical`.
pub const CRITICAL_FRACTION: f64 = 0.02;

/// How urgent a shortage of available space is. The variants are ordered from least to most urgent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Enough space is available.
    Ok,
    /// Space is running short; saves and downloads may fail soon.
    Low,
    /// Space is nearly gone; the system may stop writing.
    Critical,
}

/// The available bytes below which a volume of `total` bytes is short at the given byte and fraction limits.
fn limit(total: u64, bytes: u64, fraction: f64) -> u64 {
    let by_fraction = (total as f64 * fraction) as u64;
    bytes.min(by_fraction)
}

/// The warning level for a volume's capacity.
pub fn level(capacity: &Capacity) -> Level {
    let available = capacity.available;
    if available < limit(capacity.total, CRITICAL_BYTES, CRITICAL_FRACTION) {
        Level::Critical
    } else if available < limit(capacity.total, LOW_BYTES, LOW_FRACTION) {
        Level::Low
    } else {
        Level::Ok
    }
}

/// The notification name a shell posts for a level, or `None` when no warning is due.
pub fn notification_name(level: Level) -> Option<&'static str> {
    match level {
        Level::Ok => None,
        Level::Low | Level::Critical => Some(crate::notify_center::VOLUME_LOW_DISK_SPACE),
    }
}

/// The sentence shown with the warning for a volume, or `None` when no warning is due.
pub fn message(level: Level, volume: &str) -> Option<String> {
    match level {
        Level::Ok => None,
        Level::Low => Some(format!("{volume} is low on disk space. Free some space to keep saving and updating working.")),
        Level::Critical => Some(format!("{volume} is almost out of disk space. Free space now to avoid lost work.")),
    }
}

/// Once a shortage was announced, the same level is announced again after this many seconds.
pub const REMIND_EVERY_S: i64 = 6 * 3600;

/// Remembers the last level announced so a shortage is posted once per escalation, then again on a reminder interval.
#[derive(Clone, Copy, Debug, Default)]
pub struct Watch {
    last: Option<Level>,
    last_at: i64,
}

impl Watch {
    /// The level to announce now, or `None` when nothing new needs saying. Easing back to `Ok` clears the memory, so
    /// the next shortage is announced at once. A level above the last announced one is always announced; any other
    /// shortage is announced again once `REMIND_EVERY_S` has passed (or when the clock has gone backwards).
    pub fn observe(&mut self, level: Level, now: i64) -> Option<Level> {
        if level == Level::Ok {
            self.last = None;
            return None;
        }
        let escalated = self.last.map_or(true, |last| level > last);
        let due = now.saturating_sub(self.last_at) >= REMIND_EVERY_S || now < self.last_at;
        if escalated || due {
            self.last = Some(level);
            self.last_at = now;
            Some(level)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod watch_tests {
    use super::*;

    #[test]
    fn the_first_shortage_is_announced_and_repeats_wait_for_the_reminder_interval() {
        let mut w = Watch::default();
        assert_eq!(w.observe(Level::Low, 1000), Some(Level::Low));
        assert_eq!(w.observe(Level::Low, 1000 + REMIND_EVERY_S - 1), None);
        assert_eq!(w.observe(Level::Low, 1000 + REMIND_EVERY_S), Some(Level::Low));
    }

    #[test]
    fn escalating_is_announced_at_once_and_easing_is_not() {
        let mut w = Watch::default();
        assert_eq!(w.observe(Level::Low, 0), Some(Level::Low));
        assert_eq!(w.observe(Level::Critical, 10), Some(Level::Critical));
        assert_eq!(w.observe(Level::Critical, 20), None);
        assert_eq!(w.observe(Level::Low, 30), None);
    }

    #[test]
    fn easing_back_to_ok_rearms_the_next_shortage() {
        let mut w = Watch::default();
        assert_eq!(w.observe(Level::Critical, 0), Some(Level::Critical));
        assert_eq!(w.observe(Level::Ok, 5), None);
        assert_eq!(w.observe(Level::Low, 10), Some(Level::Low));
    }

    #[test]
    fn a_clock_that_goes_backwards_announces_again() {
        let mut w = Watch::default();
        assert_eq!(w.observe(Level::Low, 500), Some(Level::Low));
        assert_eq!(w.observe(Level::Low, 100), Some(Level::Low));
    }
}
