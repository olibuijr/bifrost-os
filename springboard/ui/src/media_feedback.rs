//! Feedback and rating for the playing item as a pure model. The feedback commands (like, dislike and bookmark, each
//! with an on or off state and a negative flag on its event) and the rating command (a minimum and maximum on the
//! scale, and the rating the item has now) are answered here as state. Delivering events and drawing controls stay
//! with the caller.

/// A feedback command an item can receive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedbackKind {
    Like,
    Dislike,
    Bookmark,
}

impl FeedbackKind {
    /// Every feedback command, in the order the state reports them.
    pub const ALL: [FeedbackKind; 3] = [FeedbackKind::Like, FeedbackKind::Dislike, FeedbackKind::Bookmark];

    /// A dislike asks the target to do something negative (the negative flag on the feedback event).
    pub fn is_negative(self) -> bool {
        self == FeedbackKind::Dislike
    }

    fn index(self) -> usize {
        match self {
            FeedbackKind::Like => 0,
            FeedbackKind::Dislike => 1,
            FeedbackKind::Bookmark => 2,
        }
    }
}

/// The on or off state of each feedback command for one item. The commands are independent: each has its own
/// active flag, so liking an item does not clear a dislike of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeedbackState {
    active: [bool; 3],
}

impl FeedbackState {
    /// Every command starts off.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_active(&self, kind: FeedbackKind) -> bool {
        self.active[kind.index()]
    }

    /// Turns a command on or off.
    pub fn set(&mut self, kind: FeedbackKind, active: bool) {
        self.active[kind.index()] = active;
    }

    /// Flips a command and returns its new state.
    pub fn toggle(&mut self, kind: FeedbackKind) -> bool {
        let next = !self.is_active(kind);
        self.set(kind, next);
        next
    }

    /// The commands that are on, in `FeedbackKind::ALL` order.
    pub fn active(&self) -> Vec<FeedbackKind> {
        FeedbackKind::ALL.iter().copied().filter(|k| self.is_active(*k)).collect()
    }
}

/// Why a rating was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RatingError {
    /// The value is not a finite number.
    Invalid,
    /// The value is outside the scale.
    OutOfRange,
}

/// A rating command: the scale an item may be rated on and the rating it has now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RatingCommand {
    minimum: f32,
    maximum: f32,
    rating: Option<f32>,
}

impl RatingCommand {
    /// A scale needs finite bounds with the minimum below the maximum. None otherwise.
    pub fn new(minimum: f32, maximum: f32) -> Option<Self> {
        if minimum.is_finite() && maximum.is_finite() && minimum < maximum {
            Some(RatingCommand { minimum, maximum, rating: None })
        } else {
            None
        }
    }

    pub fn minimum(&self) -> f32 {
        self.minimum
    }

    pub fn maximum(&self) -> f32 {
        self.maximum
    }

    /// The rating the item has now, if one was given.
    pub fn rating(&self) -> Option<f32> {
        self.rating
    }

    /// Records a rating. It must be finite and inside the scale, both ends included. A refused rating leaves the
    /// current one in place.
    pub fn rate(&mut self, value: f32) -> Result<f32, RatingError> {
        if !value.is_finite() {
            return Err(RatingError::Invalid);
        }
        if value < self.minimum || value > self.maximum {
            return Err(RatingError::OutOfRange);
        }
        self.rating = Some(value);
        Ok(value)
    }

    /// Removes the item's rating.
    pub fn clear(&mut self) {
        self.rating = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_item_has_no_feedback_turned_on() {
        let s = FeedbackState::new();
        assert!(s.active().is_empty());
        assert!(!s.is_active(FeedbackKind::Like));
    }

    #[test]
    fn feedback_commands_are_independent() {
        let mut s = FeedbackState::new();
        s.set(FeedbackKind::Like, true);
        assert!(s.toggle(FeedbackKind::Dislike), "dislike turns on");
        assert_eq!(s.active(), vec![FeedbackKind::Like, FeedbackKind::Dislike], "liking is not cleared");
    }

    #[test]
    fn toggling_twice_turns_a_command_back_off() {
        let mut s = FeedbackState::new();
        assert!(s.toggle(FeedbackKind::Bookmark));
        assert!(!s.toggle(FeedbackKind::Bookmark));
        assert!(s.active().is_empty());
    }

    #[test]
    fn only_a_dislike_is_a_negative_action() {
        assert!(FeedbackKind::Dislike.is_negative());
        assert!(!FeedbackKind::Like.is_negative());
        assert!(!FeedbackKind::Bookmark.is_negative());
    }

    #[test]
    fn a_rating_must_be_finite_and_inside_the_scale_with_both_ends_included() {
        let mut r = RatingCommand::new(0.0, 5.0).expect("a valid scale");
        assert_eq!(r.rate(3.5), Ok(3.5));
        assert_eq!(r.rating(), Some(3.5));
        assert_eq!(r.rate(5.5), Err(RatingError::OutOfRange));
        assert_eq!(r.rating(), Some(3.5), "a refused rating leaves the old one");
        assert_eq!(r.rate(5.0), Ok(5.0), "the top of the scale is allowed");
        assert_eq!(r.rate(f32::NAN), Err(RatingError::Invalid));
        r.clear();
        assert_eq!(r.rating(), None);
    }

    #[test]
    fn a_scale_needs_finite_bounds_with_minimum_below_maximum() {
        assert!(RatingCommand::new(2.0, 2.0).is_none(), "an empty scale is refused");
        assert!(RatingCommand::new(f32::NAN, 1.0).is_none());
        let r = RatingCommand::new(-1.0, 1.0).expect("a scale below zero is allowed");
        assert_eq!((r.minimum(), r.maximum()), (-1.0, 1.0));
    }
}
