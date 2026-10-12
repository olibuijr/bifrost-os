//! Remote command models for the playback rate, rating and feedback commands of the media player framework. A command
//! holds what the app supports and what is currently chosen; a request is an incoming command event that the command
//! accepts or refuses. Pure model: the app owns playback and applies the accepted value itself.
//!
//! Semantics: a disabled command refuses every request with `CommandError::Disabled` and keeps its value. A playback
//! rate request must match one of the supported rates (within `RATE_EPS`); any other rate is refused with
//! `NotSupported`. A rating request must be finite and lie between the minimum and the maximum, inclusive; otherwise
//! it is refused with `OutOfRange`. A feedback request sets the on or off state directly.
//!
//! Gaps: no handler registration or event dispatch (apps call `request` from their own handlers), no language option
//! commands, no skip interval or position commands, and no shared command centre: each command is its own value.

/// Two playback rates closer than this are the same rate.
pub const RATE_EPS: f32 = 1e-4;

/// Why a command refused a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandError {
    /// The command is switched off and refuses every request.
    Disabled,
    /// The playback rate is not one of the supported rates.
    NotSupported,
    /// The rating is not finite or lies outside the command's range.
    OutOfRange,
}

/// Changes the playback rate of the playing item to one of its supported rates.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackRateCommand {
    enabled: bool,
    supported: Vec<f32>,
    rate: f32,
}

impl PlaybackRateCommand {
    /// A command over the given rates. Non-finite and non-positive rates are dropped, the rest are sorted ascending
    /// and duplicates are removed. The chosen rate starts at normal speed (1.0), enabled.
    pub fn new(rates: &[f32]) -> Self {
        let mut supported: Vec<f32> = rates.iter().copied().filter(|r| r.is_finite() && *r > 0.0).collect();
        supported.sort_by(|a, b| a.total_cmp(b));
        supported.dedup_by(|a, b| (*a - *b).abs() < RATE_EPS);
        Self { enabled: true, supported, rate: 1.0 }
    }

    /// The supported rates in ascending order.
    pub fn supported(&self) -> &[f32] {
        &self.supported
    }

    /// The rate chosen by the last accepted request.
    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// Whether the command accepts requests.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Switches the command on or off.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Accepts a request for `rate` if the command is enabled and the rate is supported. Returns the accepted rate.
    pub fn request(&mut self, rate: f32) -> Result<f32, CommandError> {
        if !self.enabled {
            return Err(CommandError::Disabled);
        }
        match self.supported.iter().copied().find(|s| (*s - rate).abs() < RATE_EPS) {
            Some(found) => {
                self.rate = found;
                Ok(found)
            }
            None => Err(CommandError::NotSupported),
        }
    }

    /// The supported rate closest to `target`. A tie goes to the lower rate. None when no rate is supported.
    pub fn nearest(&self, target: f32) -> Option<f32> {
        let mut best: Option<f32> = None;
        for &candidate in &self.supported {
            let better = match best {
                None => true,
                Some(current) => (candidate - target).abs() < (current - target).abs(),
            };
            if better {
                best = Some(candidate);
            }
        }
        best
    }
}

/// Sets a detailed rating for the playing item within a minimum and a maximum.
#[derive(Clone, Debug, PartialEq)]
pub struct RatingCommand {
    enabled: bool,
    minimum: f32,
    maximum: f32,
    rating: f32,
}

impl RatingCommand {
    /// A rating command for the range `minimum..=maximum`. None when either bound is not finite or the minimum is not
    /// below the maximum. The rating starts at 0 clamped into the range, enabled.
    pub fn new(minimum: f32, maximum: f32) -> Option<Self> {
        if !minimum.is_finite() || !maximum.is_finite() || minimum >= maximum {
            return None;
        }
        let rating = 0.0_f32.clamp(minimum, maximum);
        Some(Self { enabled: true, minimum, maximum, rating })
    }

    /// The lowest rating the command accepts.
    pub fn minimum(&self) -> f32 {
        self.minimum
    }

    /// The highest rating the command accepts.
    pub fn maximum(&self) -> f32 {
        self.maximum
    }

    /// The rating set by the last accepted request.
    pub fn rating(&self) -> f32 {
        self.rating
    }

    /// Switches the command on or off.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Accepts `value` when the command is enabled and the value is finite and inside the range. Returns the rating.
    pub fn request(&mut self, value: f32) -> Result<f32, CommandError> {
        if !self.enabled {
            return Err(CommandError::Disabled);
        }
        if !value.is_finite() || value < self.minimum || value > self.maximum {
            return Err(CommandError::OutOfRange);
        }
        self.rating = value;
        Ok(value)
    }
}

/// A like or dislike style toggle for the playing item, with the titles the system shows for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedbackCommand {
    enabled: bool,
    active: bool,
    title: String,
    short_title: String,
}

impl FeedbackCommand {
    /// A feedback command that starts off and enabled, with its long and short titles.
    pub fn new(title: &str, short_title: &str) -> Self {
        Self { enabled: true, active: false, title: title.to_string(), short_title: short_title.to_string() }
    }

    /// The long title that describes the command.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The short title that describes the command.
    pub fn short_title(&self) -> &str {
        &self.short_title
    }

    /// Whether the feedback action is on.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Switches the command on or off.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Sets the on or off state. Returns the new state, or `Disabled` when the command is switched off.
    pub fn request(&mut self, active: bool) -> Result<bool, CommandError> {
        if !self.enabled {
            return Err(CommandError::Disabled);
        }
        self.active = active;
        Ok(active)
    }

    /// Flips the on or off state. Returns the new state, or `Disabled` when the command is switched off.
    pub fn toggle(&mut self) -> Result<bool, CommandError> {
        let next = !self.active;
        self.request(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_are_sorted_deduplicated_and_positive_only() {
        // 2.0, 0.5, 1.0, 1.5, a repeat of 1.0, a negative, zero and NaN: four rates remain.
        let cmd = PlaybackRateCommand::new(&[2.0, 0.5, 1.0, 1.5, 1.0, -1.0, 0.0, f32::NAN]);
        assert_eq!(cmd.supported(), &[0.5, 1.0, 1.5, 2.0]);
        assert_eq!(cmd.rate(), 1.0);
    }

    #[test]
    fn supported_rate_is_accepted_and_unsupported_rate_is_refused() {
        let mut cmd = PlaybackRateCommand::new(&[0.5, 1.0, 2.0]);
        assert_eq!(cmd.request(2.0), Ok(2.0));
        assert_eq!(cmd.rate(), 2.0);
        // 3.0 is not supported, so the chosen rate stays at 2.0.
        assert_eq!(cmd.request(3.0), Err(CommandError::NotSupported));
        assert_eq!(cmd.rate(), 2.0);
    }

    #[test]
    fn disabled_rate_command_refuses_and_keeps_its_value() {
        let mut cmd = PlaybackRateCommand::new(&[1.0, 2.0]);
        cmd.set_enabled(false);
        assert_eq!(cmd.request(2.0), Err(CommandError::Disabled));
        assert_eq!(cmd.rate(), 1.0);
        cmd.set_enabled(true);
        assert_eq!(cmd.request(2.0), Ok(2.0));
    }

    #[test]
    fn nearest_rate_breaks_ties_towards_the_lower_rate() {
        let cmd = PlaybackRateCommand::new(&[1.0, 1.5, 2.0]);
        // 1.2 is 0.2 from 1.0 and 0.3 from 1.5.
        assert_eq!(cmd.nearest(1.2), Some(1.0));
        // 1.25 is 0.25 from both 1.0 and 1.5: the lower one wins.
        assert_eq!(cmd.nearest(1.25), Some(1.0));
        assert_eq!(cmd.nearest(9.0), Some(2.0));
        assert_eq!(PlaybackRateCommand::new(&[]).nearest(1.0), None);
    }

    #[test]
    fn rating_range_is_inclusive_and_bounds_are_checked() {
        assert!(RatingCommand::new(2.0, 2.0).is_none());
        assert!(RatingCommand::new(f32::NAN, 5.0).is_none());
        let mut cmd = RatingCommand::new(0.0, 5.0).expect("valid range");
        assert_eq!(cmd.request(5.0), Ok(5.0));
        assert_eq!(cmd.request(3.5), Ok(3.5));
        assert_eq!(cmd.request(5.5), Err(CommandError::OutOfRange));
        assert_eq!(cmd.request(f32::NAN), Err(CommandError::OutOfRange));
        assert_eq!(cmd.rating(), 3.5);
    }

    #[test]
    fn rating_starts_at_zero_clamped_into_the_range() {
        assert_eq!(RatingCommand::new(-1.0, 1.0).map(|c| c.rating()), Some(0.0));
        assert_eq!(RatingCommand::new(1.0, 5.0).map(|c| c.rating()), Some(1.0));
        assert_eq!(RatingCommand::new(-5.0, -2.0).map(|c| c.rating()), Some(-2.0));
    }

    #[test]
    fn feedback_toggles_between_on_and_off() {
        let mut like = FeedbackCommand::new("Like", "Like");
        assert!(!like.is_active());
        assert_eq!(like.toggle(), Ok(true));
        assert_eq!(like.toggle(), Ok(false));
        assert_eq!(like.request(true), Ok(true));
        assert!(like.is_active());
    }

    #[test]
    fn disabled_feedback_keeps_its_state_and_titles_are_kept() {
        let mut fb = FeedbackCommand::new("Add to favourites", "Favourite");
        assert_eq!(fb.title(), "Add to favourites");
        assert_eq!(fb.short_title(), "Favourite");
        fb.request(true).expect("enabled");
        fb.set_enabled(false);
        assert_eq!(fb.toggle(), Err(CommandError::Disabled));
        assert!(fb.is_active());
    }
}
