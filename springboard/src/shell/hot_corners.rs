//! Hot corners: Settings > Desktop & Dock > Hot Corners picks an action per screen corner, and the action runs when the
//! pointer rests in that corner.
//!
//! The pointer must stay inside the corner square for `DWELL_MS`. The action then runs once; it does not run again until
//! the pointer has left the corner. The corner square is `CORNER_PT` points on a side, so it scales with the screen
//! (`Ui::pt`). The decision (which corner, whether the dwell is up) is the pure [`HotCorners`] state, so it is tested
//! without a screen; the `Shell` methods below carry the action out.

use super::pages::{CORNER_ACTIONS, DeskPrefs};
use super::{Effect, Shell};

/// The corner square's side, in points.
pub const CORNER_PT: f32 = 4.0;
/// How long the pointer rests in a corner before its action runs, in milliseconds.
pub const DWELL_MS: f64 = 350.0;

/// What a corner can do; the index is the entry of `pages::CORNER_ACTIONS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CornerAction {
    None,
    Overview,
    AppsGrid,
    ShowDesktop,
    LockScreen,
    Search,
    StartScreenSaver,
}

impl CornerAction {
    /// The action for an index into `CORNER_ACTIONS`; an index past the list is `None`.
    pub fn from_index(i: usize) -> Self {
        match i {
            1 => CornerAction::Overview,
            2 => CornerAction::AppsGrid,
            3 => CornerAction::ShowDesktop,
            4 => CornerAction::LockScreen,
            5 => CornerAction::Search,
            6 => CornerAction::StartScreenSaver,
            _ => CornerAction::None,
        }
    }

    /// The label the Hot Corners page shows for this action.
    pub fn label(self) -> &'static str {
        let i = match self {
            CornerAction::None => 0,
            CornerAction::Overview => 1,
            CornerAction::AppsGrid => 2,
            CornerAction::ShowDesktop => 3,
            CornerAction::LockScreen => 4,
            CornerAction::Search => 5,
            CornerAction::StartScreenSaver => 6,
        };
        CORNER_ACTIONS[i]
    }
}

/// The corner (0 top left, 1 top right, 2 bottom left, 3 bottom right) a point at (`x`, `y`) lies in on a `w` by `h`
/// screen, with corner squares of `size` pixels; `None` elsewhere.
pub fn corner_at(x: f32, y: f32, w: f32, h: f32, size: f32) -> Option<usize> {
    if w <= 0.0 || h <= 0.0 || x < 0.0 || y < 0.0 || x > w || y > h {
        return None;
    }
    let left = x <= size;
    let right = x >= w - size;
    let top = y <= size;
    let bottom = y >= h - size;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(0),
        (_, true, true, _) => Some(1),
        (true, _, _, true) => Some(2),
        (_, true, _, true) => Some(3),
        _ => None,
    }
}

/// The dwell state: which corner the pointer rests in, since when, and whether its action already ran.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HotCorners {
    resting: Option<(usize, f64)>,
    fired: bool,
}

impl HotCorners {
    pub fn new() -> Self {
        HotCorners::default()
    }

    /// The pointer moved to `corner` (or out of every corner) at `t`. Moving to another corner restarts the dwell;
    /// staying in the same one keeps it.
    pub fn pointer(&mut self, corner: Option<usize>, t: f64) {
        match (corner, self.resting) {
            (Some(c), Some((old, _))) if c == old => {}
            (Some(c), _) => {
                self.resting = Some((c, t));
                self.fired = false;
            }
            (None, _) => {
                self.resting = None;
                self.fired = false;
            }
        }
    }

    /// Once the pointer has rested `DWELL_MS` in a corner, returns that corner, once. Call it every tick.
    pub fn due(&mut self, t: f64) -> Option<usize> {
        let (c, since) = self.resting?;
        if self.fired || t - since < DWELL_MS {
            return None;
        }
        self.fired = true;
        Some(c)
    }

    /// The corner the pointer is resting in, if any.
    pub fn resting_corner(&self) -> Option<usize> {
        self.resting.map(|(c, _)| c)
    }
}

impl Shell {
    /// Pointer motion at (`x`, `y`): arms or clears the dwell of a hot corner.
    pub(super) fn hot_corner_pointer(&mut self, x: f32, y: f32, t: f64) {
        if !self.desk_active() || self.is_locked() {
            self.hot_corners.pointer(None, t);
            return;
        }
        let size = self.ui.pt(CORNER_PT).max(1.0);
        self.hot_corners.pointer(corner_at(x, y, self.w as f32, self.h as f32, size), t);
    }

    /// Runs the action of the corner whose dwell is up. Called once per tick.
    pub(super) fn hot_corner_tick(&mut self, t: f64) -> Vec<Effect> {
        let Some(corner) = self.hot_corners.due(t) else { return vec![] };
        if !self.screen_on || self.is_locked() {
            return vec![];
        }
        let action = CornerAction::from_index(DeskPrefs::load(&self.prefs).corners[corner.min(3)]);
        self.dirty = true;
        match action {
            CornerAction::None => vec![],
            CornerAction::Overview => {
                self.overview_toggle(t);
                vec![]
            }
            CornerAction::AppsGrid => {
                self.appgrid_toggle(t);
                vec![]
            }
            CornerAction::ShowDesktop => {
                self.desk_show_desktop();
                vec![]
            }
            CornerAction::LockScreen => self.sleep_screen(),
            CornerAction::Search => {
                self.open_quicksearch_hw(t);
                vec![]
            }
            CornerAction::StartScreenSaver => {
                self.saver_preview(t);
                vec![]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_corners_are_found_and_the_middle_is_not() {
        let (w, h, s) = (1000.0, 600.0, 4.0);
        assert_eq!(corner_at(0.0, 0.0, w, h, s), Some(0));
        assert_eq!(corner_at(999.0, 1.0, w, h, s), Some(1));
        assert_eq!(corner_at(2.0, 599.0, w, h, s), Some(2));
        assert_eq!(corner_at(1000.0, 600.0, w, h, s), Some(3));
        assert_eq!(corner_at(500.0, 300.0, w, h, s), None);
        // on an edge but outside the corner square
        assert_eq!(corner_at(500.0, 0.0, w, h, s), None);
        assert_eq!(corner_at(0.0, 300.0, w, h, s), None);
        // off screen, and a degenerate screen
        assert_eq!(corner_at(-1.0, 0.0, w, h, s), None);
        assert_eq!(corner_at(0.0, 0.0, 0.0, 0.0, s), None);
    }

    #[test]
    fn the_action_runs_after_the_dwell_and_only_once() {
        let mut c = HotCorners::new();
        c.pointer(Some(1), 1000.0);
        assert_eq!(c.due(1000.0 + DWELL_MS - 1.0), None);
        assert_eq!(c.due(1000.0 + DWELL_MS), Some(1));
        assert_eq!(c.due(1000.0 + DWELL_MS * 5.0), None, "it does not repeat while the pointer rests");
        // moving inside the same corner keeps the state
        c.pointer(Some(1), 3000.0);
        assert_eq!(c.due(9000.0), None);
        // leaving and coming back arms it again
        c.pointer(None, 9000.0);
        c.pointer(Some(1), 9100.0);
        assert_eq!(c.due(9100.0 + DWELL_MS), Some(1));
    }

    #[test]
    fn another_corner_restarts_the_dwell() {
        let mut c = HotCorners::new();
        c.pointer(Some(0), 0.0);
        c.pointer(Some(3), 300.0);
        assert_eq!(c.due(DWELL_MS), None, "the dwell counts from entering the new corner");
        assert_eq!(c.due(300.0 + DWELL_MS), Some(3));
        assert_eq!(c.resting_corner(), Some(3));
    }

    #[test]
    fn leaving_the_corner_cancels_a_pending_action() {
        let mut c = HotCorners::new();
        c.pointer(Some(2), 0.0);
        c.pointer(None, 100.0);
        assert_eq!(c.due(10_000.0), None);
        assert_eq!(c.resting_corner(), None);
    }

    #[test]
    fn indices_map_to_the_page_labels() {
        for i in 0..CORNER_ACTIONS.len() {
            assert_eq!(CornerAction::from_index(i).label(), CORNER_ACTIONS[i]);
        }
        assert_eq!(CornerAction::from_index(99), CornerAction::None);
        assert_eq!(CornerAction::from_index(6), CornerAction::StartScreenSaver);
    }
}
