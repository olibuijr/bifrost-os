//! Playback-rate requests as a pure model. A playing item lists the rates it supports (the `supportedPlaybackRates`
//! of a playback-rate change command) and a player is asked to move to one of them. The model keeps the list sorted,
//! refuses any rate outside it, answers the nearest supported rate for a slider and steps up or down the list.
//! It starts no playback and reads no clock.

/// Why a rate request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateError {
    /// The value is not a finite positive number.
    Invalid,
    /// The value is valid but the item does not list it as supported.
    Unsupported,
}

/// Rates are kept in thousandths so that equal rates compare exactly.
const SCALE: f32 = 1000.0;

/// Upper bound in thousandths (1000x). Anything above it is treated as invalid.
const MAX_MILLI: f32 = 1_000_000.0;

fn to_milli(rate: f32) -> Option<u32> {
    if !rate.is_finite() || rate <= 0.0 {
        return None;
    }
    let m = (rate * SCALE).round();
    if m < 1.0 || m > MAX_MILLI {
        None
    } else {
        Some(m as u32)
    }
}

/// The rates an item supports and the rate it plays at now.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackRate {
    /// Supported rates in thousandths, ascending, without duplicates. Never empty.
    supported: Vec<u32>,
    /// The current rate in thousandths. Always one of `supported`.
    current: u32,
}

impl Default for PlaybackRate {
    fn default() -> Self {
        Self::new(&[1.0])
    }
}

impl PlaybackRate {
    /// Builds the model from the rates an item lists. Values that are not finite positive numbers are dropped and
    /// duplicates merged. With no usable rate the item plays at 1x only. The starting rate is the supported rate
    /// nearest to 1x.
    pub fn new(rates: &[f32]) -> Self {
        let mut supported: Vec<u32> = rates.iter().filter_map(|r| to_milli(*r)).collect();
        supported.sort_unstable();
        supported.dedup();
        if supported.is_empty() {
            supported.push(1000);
        }
        let mut model = PlaybackRate { supported, current: 1000 };
        model.current = model.nearest_milli(1000);
        model
    }

    /// The supported rates in ascending order.
    pub fn supported(&self) -> Vec<f32> {
        self.supported.iter().map(|m| *m as f32 / SCALE).collect()
    }

    /// The current rate. Zero is not a rate here: pausing is a separate command.
    pub fn rate(&self) -> f32 {
        self.current as f32 / SCALE
    }

    /// True when more than one rate is listed, so the rate can change.
    pub fn is_adjustable(&self) -> bool {
        self.supported.len() > 1
    }

    /// True when the value is one of the supported rates.
    pub fn supports(&self, rate: f32) -> bool {
        to_milli(rate).map_or(false, |m| self.supported.binary_search(&m).is_ok())
    }

    /// Asks for a rate. A supported rate becomes the current rate and is returned. Anything else leaves the current
    /// rate unchanged and says why.
    pub fn request(&mut self, rate: f32) -> Result<f32, RateError> {
        let m = to_milli(rate).ok_or(RateError::Invalid)?;
        if self.supported.binary_search(&m).is_err() {
            return Err(RateError::Unsupported);
        }
        self.current = m;
        Ok(self.rate())
    }

    /// The supported rate nearest to `rate`. A tie goes to the slower rate. None for an invalid value.
    pub fn nearest(&self, rate: f32) -> Option<f32> {
        let m = to_milli(rate)?;
        Some(self.nearest_milli(m) as f32 / SCALE)
    }

    fn nearest_milli(&self, m: u32) -> u32 {
        let mut best = self.supported[0];
        let mut best_gap = best.abs_diff(m);
        for &s in &self.supported[1..] {
            let gap = s.abs_diff(m);
            if gap < best_gap {
                best = s;
                best_gap = gap;
            }
        }
        best
    }

    /// Moves to the next supported rate faster (`faster = true`) or slower than the current one and returns the new
    /// rate. At either end the rate stays where it is.
    pub fn step(&mut self, faster: bool) -> f32 {
        let next = if faster {
            self.supported.iter().copied().find(|&s| s > self.current)
        } else {
            self.supported.iter().rev().copied().find(|&s| s < self.current)
        };
        if let Some(n) = next {
            self.current = n;
        }
        self.rate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_cleaned_sorted_and_starts_at_the_rate_nearest_one_times() {
        let r = PlaybackRate::new(&[2.0, 0.5, 1.0, 1.0, -1.0, 0.0, f32::NAN]);
        assert_eq!(r.supported(), vec![0.5, 1.0, 2.0]);
        assert_eq!(r.rate(), 1.0);
        assert!(r.is_adjustable());
    }

    #[test]
    fn an_item_with_no_usable_rate_plays_at_one_times_only() {
        let r = PlaybackRate::new(&[]);
        assert_eq!(r.supported(), vec![1.0]);
        assert_eq!(r.rate(), 1.0);
        assert!(!r.is_adjustable());
        assert_eq!(PlaybackRate::default(), r);
    }

    #[test]
    fn when_one_times_is_not_listed_the_start_is_the_nearest_listed_rate() {
        let r = PlaybackRate::new(&[1.5, 2.0]);
        assert_eq!(r.rate(), 1.5, "1.5 is 0.5 away from 1x, 2.0 is 1.0 away");
        assert_eq!(r.nearest(1.25), Some(1.5));
        assert_eq!(r.nearest(-1.0), None);
    }

    #[test]
    fn a_request_moves_only_to_a_listed_rate() {
        let mut r = PlaybackRate::new(&[0.5, 1.0, 2.0]);
        assert_eq!(r.request(2.0), Ok(2.0));
        assert_eq!(r.rate(), 2.0);
        assert_eq!(r.request(3.0), Err(RateError::Unsupported));
        assert_eq!(r.rate(), 2.0, "a refused request leaves the rate alone");
        assert_eq!(r.request(0.0), Err(RateError::Invalid));
        assert_eq!(r.request(f32::NAN), Err(RateError::Invalid));
        assert!(r.supports(0.5) && !r.supports(1.4));
    }

    #[test]
    fn a_tie_between_two_rates_goes_to_the_slower_one() {
        let r = PlaybackRate::new(&[1.0, 1.5, 2.0]);
        assert_eq!(r.nearest(1.25), Some(1.0), "1.25 is 250 thousandths from both 1.0 and 1.5");
        assert_eq!(r.nearest(1.3), Some(1.5), "1.3 is 200 from 1.5 and 300 from 1.0");
    }

    #[test]
    fn stepping_walks_the_list_and_stops_at_both_ends() {
        let mut r = PlaybackRate::new(&[0.5, 1.0, 1.5, 2.0]);
        assert_eq!(r.step(true), 1.5);
        assert_eq!(r.step(true), 2.0);
        assert_eq!(r.step(true), 2.0, "the fastest rate stays");
        assert_eq!(r.step(false), 1.5);
        assert_eq!(r.step(false), 1.0);
        assert_eq!(r.step(false), 0.5);
        assert_eq!(r.step(false), 0.5, "the slowest rate stays");
    }
}
