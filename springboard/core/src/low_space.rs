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
