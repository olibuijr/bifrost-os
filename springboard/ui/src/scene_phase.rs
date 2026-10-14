//! Scene phase: whether an app's window is in the foreground, behind another window, or hidden.
//!
//! The shell tells an app two facts about its window: whether the window is visible on screen, and whether it has
//! keyboard focus. From those the phase follows: a focused, visible window is active; a visible window without focus
//! (another window, a sheet or the search panel is in front of it) is inactive; a window that is not visible is in the
//! background. [`SceneTracker`] reports each change once, so an app can save its state on the way to the background.
//! The model reads no shell state: the caller passes the two facts.

/// The phase of one window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenePhase {
    /// Visible and focused: the person is using the window.
    Active,
    /// Visible but not focused: the window is on screen behind something in front of it.
    Inactive,
    /// Not visible: the window is minimised, hidden or on another space.
    Background,
}

impl ScenePhase {
    /// The phase for a window that is `visible` on screen and `focused` for keyboard input.
    pub fn from_facts(visible: bool, focused: bool) -> Self {
        match (visible, focused) {
            (true, true) => ScenePhase::Active,
            (true, false) => ScenePhase::Inactive,
            (false, _) => ScenePhase::Background,
        }
    }
}

/// Follows the phase of one window and reports each change once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneTracker {
    phase: ScenePhase,
}

impl SceneTracker {
    /// A tracker for a window that starts in `phase`. The first update reports only a real change.
    pub fn new(phase: ScenePhase) -> Self {
        SceneTracker { phase }
    }

    pub fn phase(&self) -> ScenePhase {
        self.phase
    }

    /// Takes the window's new facts. Returns the new phase when it differs from the last one, and `None` otherwise.
    pub fn update(&mut self, visible: bool, focused: bool) -> Option<ScenePhase> {
        let next = ScenePhase::from_facts(visible, focused);
        if next == self.phase {
            return None;
        }
        self.phase = next;
        Some(next)
    }
}
