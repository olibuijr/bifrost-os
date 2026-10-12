//! Timebases: a timeline under application control. A timebase maps a reference clock time to the time it shows,
//! through an anchor (a timeline time and the reference time it was set at) and a rate. It also keeps one-shot
//! timers on its timeline and records the time jumps made by setting the time. This is a pure model: the reference
//! clock time is passed in by the caller, and nothing here reads a clock or runs a timer.

use std::cmp::Ordering;

use crate::media_time::MediaTime;

/// Why a timebase call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimebaseError {
    /// The rate is NaN or infinite.
    NonFiniteRate,
    /// A time or reference time is not numeric, so the timebase cannot be anchored to it.
    InvalidTime,
}

/// A timeline with a rate. A new timebase is paused at zero, anchored to the reference time it was created at.
#[derive(Clone, Debug)]
pub struct Timebase {
    anchor_time: MediaTime,
    anchor_reference: MediaTime,
    rate: f64,
    timers: Vec<(u32, MediaTime)>,
    jumps: Vec<MediaTime>,
}

fn check_rate(rate: f64) -> Result<(), TimebaseError> {
    if rate.is_finite() {
        Ok(())
    } else {
        Err(TimebaseError::NonFiniteRate)
    }
}

impl Timebase {
    /// A paused timebase that shows zero at `reference`. A reference time that is not numeric is refused.
    pub fn new(reference: MediaTime) -> Result<Timebase, TimebaseError> {
        if !reference.is_numeric() {
            return Err(TimebaseError::InvalidTime);
        }
        Ok(Timebase {
            anchor_time: MediaTime::zero(),
            anchor_reference: reference,
            rate: 0.0,
            timers: Vec::new(),
            jumps: Vec::new(),
        })
    }

    /// The rate. Zero is paused, one is real time, and a negative rate runs backward.
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// The time shown at the reference time `reference`: the anchor time plus the elapsed reference time times the
    /// rate. A paused timebase shows its anchor time at every reference time. Returns `None` when `reference` or the
    /// anchor is not numeric.
    pub fn time_at(&self, reference: MediaTime) -> Option<MediaTime> {
        if !reference.is_numeric() || !self.anchor_time.is_numeric() || !self.anchor_reference.is_numeric() {
            return None;
        }
        if self.rate == 0.0 {
            return Some(self.anchor_time);
        }
        let elapsed = (reference - self.anchor_reference).seconds();
        let seconds = self.anchor_time.seconds() + elapsed * self.rate;
        let scale = self.anchor_time.timescale().max(reference.timescale());
        Some(MediaTime::from_seconds(seconds, scale))
    }

    /// Sets the time shown to `time` at the reference time `reference`, keeping the rate. The change is recorded as a
    /// time jump.
    pub fn set_time(&mut self, time: MediaTime, reference: MediaTime) -> Result<(), TimebaseError> {
        if !time.is_numeric() || !reference.is_numeric() {
            return Err(TimebaseError::InvalidTime);
        }
        self.anchor_time = time;
        self.anchor_reference = reference;
        self.jumps.push(time);
        Ok(())
    }

    /// Changes the rate at the reference time `reference` without changing the time shown at that instant. The new
    /// rate applies from `reference` on.
    pub fn set_rate(&mut self, rate: f64, reference: MediaTime) -> Result<(), TimebaseError> {
        check_rate(rate)?;
        let now = self.time_at(reference).ok_or(TimebaseError::InvalidTime)?;
        self.anchor_time = now;
        self.anchor_reference = reference;
        self.rate = rate;
        Ok(())
    }

    /// Sets the rate, and the time `anchor_time` shown at the reference time `reference`, in one step. This is
    /// not recorded as a time jump.
    pub fn set_rate_and_anchor_time(
        &mut self,
        rate: f64,
        anchor_time: MediaTime,
        reference: MediaTime,
    ) -> Result<(), TimebaseError> {
        check_rate(rate)?;
        if !anchor_time.is_numeric() || !reference.is_numeric() {
            return Err(TimebaseError::InvalidTime);
        }
        self.anchor_time = anchor_time;
        self.anchor_reference = reference;
        self.rate = rate;
        Ok(())
    }

    /// Adds a one-shot timer that is due once the timebase shows `fire_at` or later. A timer with the same `id` is
    /// replaced.
    pub fn add_timer(&mut self, id: u32, fire_at: MediaTime) -> Result<(), TimebaseError> {
        if !fire_at.is_numeric() {
            return Err(TimebaseError::InvalidTime);
        }
        self.timers.retain(|entry| entry.0 != id);
        self.timers.push((id, fire_at));
        Ok(())
    }

    /// Removes the timer `id`. Returns whether a timer was removed.
    pub fn remove_timer(&mut self, id: u32) -> bool {
        let before = self.timers.len();
        self.timers.retain(|entry| entry.0 != id);
        self.timers.len() != before
    }

    /// The number of timers that have not fired or been removed.
    pub fn timer_count(&self) -> usize {
        self.timers.len()
    }

    /// Takes the timers that are due at the reference time `reference`, that is those whose fire time is at or
    /// before the time shown then. They are removed, so each fires once. The ids come back in fire-time order, and
    /// by id for equal fire times.
    pub fn due_timers(&mut self, reference: MediaTime) -> Vec<u32> {
        let now = match self.time_at(reference) {
            Some(t) => t,
            None => return Vec::new(),
        };
        let mut due: Vec<(MediaTime, u32)> = Vec::new();
        self.timers.retain(|entry| {
            let (id, fire) = *entry;
            let is_due = matches!(fire.compare(&now), Some(Ordering::Less) | Some(Ordering::Equal));
            if is_due {
                due.push((fire, id));
            }
            !is_due
        });
        due.sort_by(|a, b| a.0.compare(&b.0).unwrap_or(Ordering::Equal).then(a.1.cmp(&b.1)));
        due.into_iter().map(|(_, id)| id).collect()
    }

    /// Takes the time jumps recorded since the last call, oldest first.
    pub fn take_time_jumps(&mut self) -> Vec<MediaTime> {
        std::mem::take(&mut self.jumps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_time::MediaTime;

    fn secs(s: i64) -> MediaTime {
        MediaTime::new(s, 1)
    }

    #[test]
    fn a_new_timebase_is_paused_and_shows_its_anchor_at_every_reference_time() {
        let tb = Timebase::new(MediaTime::zero()).unwrap();
        assert_eq!(tb.rate(), 0.0);
        assert_eq!(tb.time_at(secs(5)).unwrap().seconds(), 0.0);
        assert_eq!(Timebase::new(MediaTime::invalid()).unwrap_err(), TimebaseError::InvalidTime);
    }

    #[test]
    fn a_rate_of_two_advances_the_time_twice_as_fast_as_the_reference() {
        let mut tb = Timebase::new(MediaTime::zero()).unwrap();
        tb.set_rate(2.0, secs(0)).unwrap();
        assert_eq!(tb.time_at(secs(1)).unwrap().seconds(), 2.0);
        assert_eq!(tb.time_at(secs(3)).unwrap().seconds(), 6.0);
    }

    #[test]
    fn changing_the_rate_keeps_the_time_already_reached() {
        let mut tb = Timebase::new(MediaTime::zero()).unwrap();
        tb.set_rate(1.0, secs(0)).unwrap();
        // At reference 4 s the timeline shows 4 s. From then on it runs at half speed.
        tb.set_rate(0.5, secs(4)).unwrap();
        assert_eq!(tb.time_at(secs(4)).unwrap().seconds(), 4.0);
        assert_eq!(tb.time_at(secs(8)).unwrap().seconds(), 6.0);
    }

    #[test]
    fn setting_the_time_records_one_jump_and_keeps_the_rate() {
        let mut tb = Timebase::new(MediaTime::zero()).unwrap();
        tb.set_rate(1.0, secs(0)).unwrap();
        tb.set_time(secs(10), secs(2)).unwrap();
        assert_eq!(tb.rate(), 1.0);
        assert_eq!(tb.time_at(secs(2)).unwrap().seconds(), 10.0);
        assert_eq!(tb.time_at(secs(3)).unwrap().seconds(), 11.0);
        let jumps = tb.take_time_jumps();
        assert_eq!(jumps.len(), 1);
        assert_eq!(jumps[0].seconds(), 10.0);
        assert!(tb.take_time_jumps().is_empty());
    }

    #[test]
    fn timers_fire_once_in_fire_time_order_when_the_timeline_reaches_them() {
        let mut tb = Timebase::new(MediaTime::zero()).unwrap();
        tb.set_rate(1.0, secs(0)).unwrap();
        tb.add_timer(7, secs(3)).unwrap();
        tb.add_timer(9, secs(1)).unwrap();
        assert_eq!(tb.timer_count(), 2);
        assert_eq!(tb.due_timers(secs(2)), vec![9]);
        assert!(tb.due_timers(secs(2)).is_empty());
        assert_eq!(tb.due_timers(secs(3)), vec![7]);
        assert_eq!(tb.timer_count(), 0);
    }

    #[test]
    fn a_timer_can_be_removed_before_it_fires() {
        let mut tb = Timebase::new(MediaTime::zero()).unwrap();
        tb.set_rate(1.0, secs(0)).unwrap();
        tb.add_timer(4, secs(1)).unwrap();
        assert!(tb.remove_timer(4));
        assert!(!tb.remove_timer(4));
        assert!(tb.due_timers(secs(5)).is_empty());
    }

    #[test]
    fn non_finite_rates_and_non_numeric_anchors_are_refused() {
        let mut tb = Timebase::new(MediaTime::zero()).unwrap();
        assert_eq!(tb.set_rate(f64::NAN, secs(0)), Err(TimebaseError::NonFiniteRate));
        assert_eq!(tb.set_rate(f64::INFINITY, secs(0)), Err(TimebaseError::NonFiniteRate));
        assert_eq!(tb.set_rate_and_anchor_time(1.0, MediaTime::indefinite(), secs(0)), Err(TimebaseError::InvalidTime));
        assert_eq!(tb.add_timer(1, MediaTime::invalid()), Err(TimebaseError::InvalidTime));
        assert_eq!(tb.rate(), 0.0);
    }
}
