//! Playback rate commands: the rates an item can play at, and the rate a request resolves to.
//! Mirrors `MediaCommand.changePlaybackRate(supported:_:)`, which creates a command that changes the rate and
//! takes the list of supported rates. The resolution rule (exact match, otherwise the nearest supported rate, a tie
//! going to the slower rate) is this module's own choice; the official page does not state it. Pure arithmetic:
//! no playback is started here.

/// Two rates closer than this are the same rate.
const TOLERANCE: f64 = 1e-9;

/// The cases `RateCommand::new` rejects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RateError {
    /// No rates were supplied.
    Empty,
    /// A rate is not a positive, finite number.
    Invalid,
}

/// A set of supported playback rates, kept in ascending order without duplicates.
#[derive(Clone, Debug, PartialEq)]
pub struct RateCommand {
    rates: Vec<f64>,
}

impl RateCommand {
    /// A command over the supported rates. Order and repeats in `supported` do not matter.
    pub fn new(supported: &[f64]) -> Result<Self, RateError> {
        if supported.is_empty() {
            return Err(RateError::Empty);
        }
        if supported.iter().any(|r| !r.is_finite() || *r <= 0.0) {
            return Err(RateError::Invalid);
        }
        let mut rates = supported.to_vec();
        rates.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        rates.dedup();
        Ok(RateCommand { rates })
    }

    /// The supported rates, slowest first.
    pub fn rates(&self) -> &[f64] {
        &self.rates
    }

    /// True when `rate` is one of the supported rates.
    pub fn accepts(&self, rate: f64) -> bool {
        self.rates.iter().any(|r| (r - rate).abs() < TOLERANCE)
    }

    /// The supported rate a request resolves to: the nearest one, with a tie going to the slower rate.
    /// Zero, negative and non-finite requests resolve to nothing.
    pub fn resolve(&self, requested: f64) -> Option<f64> {
        if !requested.is_finite() || requested <= 0.0 {
            return None;
        }
        let mut best: Option<f64> = None;
        for &rate in &self.rates {
            best = match best {
                None => Some(rate),
                Some(current) if (rate - requested).abs() < (current - requested).abs() => Some(rate),
                keep => keep,
            };
        }
        best
    }

    /// The next faster or slower supported rate from `current`. At either end the rate stays where it is.
    /// `current` is resolved first, so an unsupported rate steps from the rate it resolves to.
    pub fn step(&self, current: f64, faster: bool) -> Option<f64> {
        let here = self.resolve(current)?;
        let index = self.rates.iter().position(|r| *r == here)?;
        let target = if faster {
            self.rates.get(index + 1)
        } else {
            index.checked_sub(1).and_then(|i| self.rates.get(i))
        };
        Some(target.copied().unwrap_or(here))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_empty_zero_negative_and_non_finite_rates() {
        assert_eq!(RateCommand::new(&[]), Err(RateError::Empty));
        assert_eq!(RateCommand::new(&[1.0, 0.0]), Err(RateError::Invalid));
        assert_eq!(RateCommand::new(&[-2.0]), Err(RateError::Invalid));
        assert_eq!(RateCommand::new(&[f64::NAN]), Err(RateError::Invalid));
        assert!(RateCommand::new(&[1.0]).is_ok());
    }

    #[test]
    fn rates_are_sorted_and_deduplicated() {
        let cmd = RateCommand::new(&[2.0, 1.0, 1.5, 1.0]).unwrap();
        assert_eq!(cmd.rates(), &[1.0, 1.5, 2.0]);
    }

    #[test]
    fn accepts_only_the_supported_rates() {
        let cmd = RateCommand::new(&[0.5, 1.0, 2.0]).unwrap();
        assert!(cmd.accepts(1.0));
        assert!(cmd.accepts(0.5));
        assert!(!cmd.accepts(1.25));
        assert!(!cmd.accepts(3.0));
    }

    #[test]
    fn resolve_picks_the_nearest_supported_rate() {
        let cmd = RateCommand::new(&[0.5, 1.0, 2.0]).unwrap();
        assert_eq!(cmd.resolve(1.4), Some(1.0));
        assert_eq!(cmd.resolve(1.6), Some(2.0));
        assert_eq!(cmd.resolve(5.0), Some(2.0));
    }

    #[test]
    fn a_tie_resolves_to_the_slower_rate() {
        let cmd = RateCommand::new(&[0.5, 1.0, 2.0]).unwrap();
        // 0.75 is 0.25 from both 0.5 and 1.0; the slower rate wins.
        assert_eq!(cmd.resolve(0.75), Some(0.5));
    }

    #[test]
    fn non_positive_and_non_finite_requests_resolve_to_nothing() {
        let cmd = RateCommand::new(&[1.0, 2.0]).unwrap();
        assert_eq!(cmd.resolve(0.0), None);
        assert_eq!(cmd.resolve(-1.0), None);
        assert_eq!(cmd.resolve(f64::NAN), None);
    }

    #[test]
    fn step_moves_one_supported_rate_and_stops_at_the_ends() {
        let cmd = RateCommand::new(&[0.5, 1.0, 2.0]).unwrap();
        assert_eq!(cmd.step(1.0, true), Some(2.0));
        assert_eq!(cmd.step(1.0, false), Some(0.5));
        assert_eq!(cmd.step(2.0, true), Some(2.0));
        assert_eq!(cmd.step(0.5, false), Some(0.5));
        // 1.4 resolves to 1.0 first, so a faster step lands on 2.0.
        assert_eq!(cmd.step(1.4, true), Some(2.0));
    }
}
