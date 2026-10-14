//! Alarm schedules: the local time an alarm fires, the weekdays it repeats on, and the instants it fires at.
//!
//! The alarm service in the system layer keeps the alarms and rings them (see `docs/alarm-service-design.md`). A check
//! calls [`Alarm::occurs_in`] with the time of the previous check and the current time, so an alarm rings once per
//! occurrence. An occurrence older than [`GRACE_SECS`] is dropped, so a stalled check does not ring an hours-late alarm.
//! Times are Unix seconds and the local offset is seconds east of UTC.

/// How late, in seconds, an alarm may still ring after its time.
pub const GRACE_SECS: i64 = 10 * 60;
/// Repeat mask for every day of the week.
pub const EVERY_DAY: u8 = 0x7f;
/// Repeat mask for Monday to Friday.
pub const WEEKDAYS: u8 = 0b011_1110;

const SECS_PER_DAY: i64 = 86_400;
const MINUTES_PER_DAY: u16 = 24 * 60;

/// An alarm at a local minute of the day, repeating on the weekdays in its mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alarm {
    pub enabled: bool,
    /// Local minute of the day, 0 to 1439.
    pub minute: u16,
    /// Weekday mask, bit 0 is Sunday. 0 means the alarm fires once, at its next occurrence.
    pub repeat: u8,
}

impl Alarm {
    /// An enabled alarm from a validated minute and mask; None when the minute is outside the day or the mask has bits
    /// beyond the seven weekdays.
    pub fn new(minute: u16, repeat: u8) -> Option<Self> {
        (minute < MINUTES_PER_DAY && (repeat & !EVERY_DAY) == 0).then_some(Self { enabled: true, minute, repeat })
    }

    fn repeats_on(&self, day: i64) -> bool {
        self.repeat == 0 || self.repeat & (1u8 << weekday(day)) != 0
    }

    /// The Unix instant of this alarm on local day `day` (days since the Unix epoch).
    fn instant(&self, day: i64, offset: i32) -> i64 {
        day * SECS_PER_DAY + i64::from(self.minute) * 60 - i64::from(offset)
    }

    /// Whether the alarm has an occurrence after `after` and no later than `now`, and within [`GRACE_SECS`] of `now`.
    pub fn occurs_in(&self, after: i64, now: i64, offset: i32) -> bool {
        if !self.enabled {
            return false;
        }
        let lower = after.max(now - GRACE_SECS);
        let today = (now + i64::from(offset)).div_euclid(SECS_PER_DAY);
        [today - 1, today].into_iter().any(|day| {
            let at = self.instant(day, offset);
            self.repeats_on(day) && at > lower && at <= now
        })
    }

    /// The first instant after `now` at which the alarm fires, searching the next eight local days.
    /// None when the alarm is disabled.
    pub fn next_occurrence(&self, now: i64, offset: i32) -> Option<i64> {
        if !self.enabled {
            return None;
        }
        let today = (now + i64::from(offset)).div_euclid(SECS_PER_DAY);
        (today..=today + 7)
            .filter(|&day| self.repeats_on(day))
            .map(|day| self.instant(day, offset))
            .find(|&at| at > now)
    }
}

/// The weekday of a day number, 0 for Sunday. Day 0, 1970-01-01, was a Thursday.
pub fn weekday(day: i64) -> u32 {
    (day + 4).rem_euclid(7) as u32
}
