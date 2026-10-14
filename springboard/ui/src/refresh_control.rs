//! Pull to refresh for a scroll view (UIKit's `UIRefreshControl`).
//!
//! Pure model. The scroll view reports its offset and whether a finger or pointer still drags it. A pull past the
//! top (a negative offset) arms the control once it passes the threshold, and releasing the drag while armed starts
//! the refresh. The app ends it with [`RefreshControl::end_refreshing`]. A trackpad or wheel gesture reports the same
//! offsets with `dragging` set while its gesture phase is active.

/// The pull, in points, that arms the control.
pub const PULL_THRESHOLD: f32 = 72.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshState {
    Idle,
    /// Pulled past the top but not far enough to refresh on release.
    Pulling,
    /// Pulled past the threshold; releasing the drag starts the refresh.
    Armed,
    Refreshing,
}

#[derive(Debug)]
pub struct RefreshControl {
    threshold: f32,
    state: RefreshState,
    pull: f32,
    was_dragging: bool,
}

impl Default for RefreshControl {
    fn default() -> Self {
        Self::new(PULL_THRESHOLD)
    }
}

impl RefreshControl {
    pub fn new(threshold: f32) -> Self {
        RefreshControl { threshold: threshold.max(1.0), state: RefreshState::Idle, pull: 0.0, was_dragging: false }
    }

    pub fn state(&self) -> RefreshState {
        self.state
    }

    pub fn is_refreshing(&self) -> bool {
        self.state == RefreshState::Refreshing
    }

    /// How far the pull has got, from 0 to 1; the renderer draws the spinner with it.
    pub fn progress(&self) -> f32 {
        (self.pull / self.threshold).clamp(0.0, 1.0)
    }

    /// The space the content keeps above its first row while the refresh runs. The app adds it as a top inset.
    pub fn inset(&self) -> f32 {
        if self.is_refreshing() {
            self.threshold
        } else {
            0.0
        }
    }

    /// Feeds the scroll view's state. Returns true on the call that starts a refresh.
    pub fn observe(&mut self, scroll_offset: f32, dragging: bool) -> bool {
        let released = self.was_dragging && !dragging;
        self.was_dragging = dragging;
        if self.is_refreshing() {
            return false;
        }
        self.pull = (-scroll_offset).max(0.0);
        if dragging {
            self.state = if self.pull >= self.threshold {
                RefreshState::Armed
            } else if self.pull > 0.0 {
                RefreshState::Pulling
            } else {
                RefreshState::Idle
            };
            return false;
        }
        if released && self.state == RefreshState::Armed {
            self.state = RefreshState::Refreshing;
            return true;
        }
        self.state = RefreshState::Idle;
        false
    }

    /// Starts a refresh from the app, without a pull.
    pub fn begin_refreshing(&mut self) {
        self.state = RefreshState::Refreshing;
        self.pull = 0.0;
    }

    pub fn end_refreshing(&mut self) {
        if self.is_refreshing() {
            self.state = RefreshState::Idle;
        }
        self.pull = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drags down to `pull` points past the top, then releases.
    fn pull_and_release(c: &mut RefreshControl, pull: f32) -> bool {
        c.observe(0.0, true);
        c.observe(-pull, true);
        c.observe(-pull, false)
    }

    #[test]
    fn a_pull_past_the_threshold_refreshes_on_release() {
        let mut c = RefreshControl::default();
        assert!(pull_and_release(&mut c, PULL_THRESHOLD + 10.0));
        assert!(c.is_refreshing());
        assert_eq!(c.inset(), PULL_THRESHOLD);
    }

    #[test]
    fn a_short_pull_does_not_refresh() {
        let mut c = RefreshControl::default();
        assert!(!pull_and_release(&mut c, 30.0));
        assert_eq!(c.state(), RefreshState::Idle);
        assert_eq!(c.inset(), 0.0);
    }

    #[test]
    fn pulling_back_under_the_threshold_cancels_the_arm() {
        let mut c = RefreshControl::default();
        c.observe(0.0, true);
        c.observe(-100.0, true);
        assert_eq!(c.state(), RefreshState::Armed);
        c.observe(-20.0, true);
        assert_eq!(c.state(), RefreshState::Pulling);
        assert!(!c.observe(-20.0, false));
    }

    #[test]
    fn an_upward_scroll_is_not_a_pull() {
        let mut c = RefreshControl::default();
        c.observe(0.0, true);
        c.observe(50.0, true);
        assert_eq!(c.state(), RefreshState::Idle);
        assert_eq!(c.progress(), 0.0);
    }

    #[test]
    fn progress_grows_with_the_pull_and_stops_at_one() {
        let mut c = RefreshControl::new(100.0);
        c.observe(0.0, true);
        c.observe(-25.0, true);
        assert!((c.progress() - 0.25).abs() < 1e-6);
        c.observe(-400.0, true);
        assert_eq!(c.progress(), 1.0);
    }

    #[test]
    fn a_running_refresh_ignores_further_pulls_until_it_ends() {
        let mut c = RefreshControl::default();
        assert!(pull_and_release(&mut c, 120.0));
        assert!(!pull_and_release(&mut c, 120.0), "no second refresh while one runs");
        assert!(c.is_refreshing());
        c.end_refreshing();
        assert_eq!(c.state(), RefreshState::Idle);
        assert_eq!(c.inset(), 0.0);
        assert!(pull_and_release(&mut c, 120.0), "a new pull refreshes again");
    }

    #[test]
    fn the_app_can_start_a_refresh_itself() {
        let mut c = RefreshControl::default();
        c.begin_refreshing();
        assert!(c.is_refreshing());
        assert_eq!(c.progress(), 0.0);
        assert!(!c.observe(0.0, false), "the programmatic refresh does not restart");
        c.end_refreshing();
        assert!(!c.is_refreshing());
    }

    #[test]
    fn a_release_without_a_drag_never_refreshes() {
        let mut c = RefreshControl::default();
        assert!(!c.observe(-200.0, false));
        assert_eq!(c.state(), RefreshState::Idle);
    }
}
