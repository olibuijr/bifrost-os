//! The schedule for sending a playback command again after the audio service could not be reached.
//!
//! Tuning in to a station sends one command to the service. When that fails because the service is down, Music keeps
//! the wish and tries again on a growing wait (1 s, 2 s, 4 s, up to 30 s), so a service that comes back starts the
//! station without a click, and one that stays away is asked a few times a minute, never in a loop. Any choice the
//! person makes (another station, a song, pause, skip) ends the retries: Music never plays what was not asked for last.

/// The wait before the first retry, in milliseconds. Each retry doubles it.
pub const FIRST_MS: f64 = 1000.0;
/// The longest wait between two attempts, in milliseconds.
pub const MAX_MS: f64 = 30_000.0;

/// What to send again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wanted {
    /// tune in the station with this id
    Station(u64),
}

/// The pending retry, if any, and when it is due.
#[derive(Debug, Default)]
pub struct Retry {
    wanted: Option<Wanted>,
    /// attempts made so far for `wanted`
    attempts: u32,
    /// when the next attempt is due (the app clock, milliseconds)
    due: f64,
}

impl Retry {
    /// The wait after `attempts` attempts have been made: 1 s, 2 s, 4 s, 8 s, 16 s, then 30 s.
    pub fn delay_ms(attempts: u32) -> f64 {
        (FIRST_MS * 2f64.powi(attempts.min(16) as i32)).min(MAX_MS)
    }

    /// A command for `wanted` failed at `now`. It is tried again after the first wait. Called again for the wish that
    /// is already scheduled (the retry itself failed) it keeps its place on the schedule.
    pub fn arm(&mut self, wanted: Wanted, now: f64) {
        if self.wanted == Some(wanted) {
            return;
        }
        *self = Retry { wanted: Some(wanted), attempts: 0, due: now + Self::delay_ms(0) };
    }

    /// The person chose something else, or playback works: nothing is sent again.
    pub fn cancel(&mut self) {
        *self = Retry::default();
    }

    /// The wish being retried.
    pub fn pending(&self) -> Option<Wanted> {
        self.wanted
    }

    /// When the next attempt is due, for the app's wake-up time.
    pub fn next_wake(&self) -> Option<f64> {
        self.wanted.map(|_| self.due)
    }

    /// The command to send now, when its wait is over; the attempt after it is scheduled on the next, longer wait.
    pub fn take_due(&mut self, now: f64) -> Option<Wanted> {
        let wanted = self.wanted?;
        if now < self.due {
            return None;
        }
        self.attempts += 1;
        self.due = now + Self::delay_ms(self.attempts);
        Some(wanted)
    }

    /// How many retries have been sent for the pending wish.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_doubles_from_one_second_to_thirty() {
        let waits: Vec<f64> = (0..8).map(Retry::delay_ms).collect();
        assert_eq!(waits, [1000.0, 2000.0, 4000.0, 8000.0, 16_000.0, 30_000.0, 30_000.0, 30_000.0]);
        assert_eq!(Retry::delay_ms(u32::MAX), MAX_MS, "a huge attempt count cannot overflow the wait");
    }

    #[test]
    fn nothing_is_due_before_the_wait_and_each_attempt_waits_longer() {
        let mut retry = Retry::default();
        assert_eq!(retry.take_due(0.0), None, "nothing to retry");
        retry.arm(Wanted::Station(7), 10_000.0);
        assert_eq!(retry.next_wake(), Some(11_000.0));
        assert_eq!(retry.take_due(10_999.0), None, "the first wait is a second");
        assert_eq!(retry.take_due(11_000.0), Some(Wanted::Station(7)));
        assert_eq!(retry.next_wake(), Some(13_000.0), "then two seconds");
        assert_eq!(retry.take_due(12_999.0), None);
        assert_eq!(retry.take_due(13_000.0), Some(Wanted::Station(7)));
        assert_eq!(retry.next_wake(), Some(17_000.0), "then four");
        assert_eq!(retry.attempts(), 2);
    }

    #[test]
    fn a_late_poll_does_not_bunch_the_attempts_up() {
        let mut retry = Retry::default();
        retry.arm(Wanted::Station(1), 0.0);
        // the app was asleep for a minute: one attempt now, the next a full wait after it
        assert_eq!(retry.take_due(60_000.0), Some(Wanted::Station(1)));
        assert_eq!(retry.take_due(60_001.0), None);
        assert_eq!(retry.next_wake(), Some(62_000.0));
    }

    #[test]
    fn a_failed_retry_keeps_its_place_on_the_schedule() {
        let mut retry = Retry::default();
        retry.arm(Wanted::Station(3), 0.0);
        assert_eq!(retry.take_due(1000.0), Some(Wanted::Station(3)));
        // the retry failed and says so again: the schedule is not restarted
        retry.arm(Wanted::Station(3), 1000.0);
        assert_eq!(retry.next_wake(), Some(3000.0));
        assert_eq!(retry.attempts(), 1);
    }

    #[test]
    fn another_station_replaces_the_wish_and_starts_over() {
        let mut retry = Retry::default();
        retry.arm(Wanted::Station(3), 0.0);
        retry.take_due(1000.0);
        retry.take_due(3000.0);
        retry.arm(Wanted::Station(9), 5000.0);
        assert_eq!(retry.pending(), Some(Wanted::Station(9)));
        assert_eq!(retry.next_wake(), Some(6000.0));
        assert_eq!(retry.attempts(), 0);
    }

    #[test]
    fn pausing_or_choosing_something_else_ends_the_retries() {
        let mut retry = Retry::default();
        retry.arm(Wanted::Station(3), 0.0);
        retry.cancel();
        assert_eq!(retry.pending(), None);
        assert_eq!(retry.next_wake(), None);
        assert_eq!(retry.take_due(1_000_000.0), None, "nothing is ever sent again");
    }
}
