//! Leading and trailing swipe actions for list rows (UIKit's `UISwipeActionsConfiguration` and `UIContextualAction`).
//!
//! Pure model: it follows the row offset while a drag moves, decides where the row settles on release and reports
//! which action was performed. A touch drag on the tablet and a pointer or trackpad drag on the desktop feed the same
//! calls, with the total horizontal travel since the drag began.
//!
//! Offsets are in points: a positive offset slides the row right and reveals the leading actions, a negative offset
//! slides it left and reveals the trailing actions. The first action of a side sits nearest the row edge. When that
//! action performs on a full swipe, the side is not resisted until the row is crossed, so the finger can reach the
//! threshold; otherwise the overshoot past the revealed buttons is rubber banded.

use crate::rubber_band;

/// Width of one action button in points.
pub const ACTION_WIDTH: f32 = 80.0;
/// The rubber band limit for overshoot past the revealed buttons when a full swipe is not available.
const RESIST: f32 = 120.0;
/// A full swipe performs the first action once the row has moved this fraction of its width.
pub const FULL_SWIPE_FRACTION: f32 = 0.5;
/// A released row stays open when it moved past this fraction of the revealed buttons.
const OPEN_FRACTION: f32 = 0.5;

/// The style of an action button (`UIContextualAction.Style`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionStyle {
    Normal,
    /// Marks a removing action; the renderer draws it in the destructive colour.
    Destructive,
}

/// One action button revealed by a swipe. `id` is what the app gets back when the action is performed.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextualAction {
    pub id: u32,
    pub title: String,
    pub style: ActionStyle,
    /// Overrides the style's background colour when set (packed like the renderer's colours).
    pub background: Option<u32>,
}

impl ContextualAction {
    pub fn new(id: u32, title: &str) -> Self {
        ContextualAction { id, title: title.into(), style: ActionStyle::Normal, background: None }
    }

    pub fn destructive(id: u32, title: &str) -> Self {
        ContextualAction { style: ActionStyle::Destructive, ..Self::new(id, title) }
    }
}

/// The actions of one edge of a row (`UISwipeActionsConfiguration`).
#[derive(Clone, Debug, PartialEq)]
pub struct SwipeConfig {
    pub actions: Vec<ContextualAction>,
    /// Whether a full swipe performs the first action without the user lifting to choose it.
    pub performs_first_full_swipe: bool,
}

impl SwipeConfig {
    pub fn new(actions: Vec<ContextualAction>) -> Self {
        SwipeConfig { actions, performs_first_full_swipe: true }
    }

    /// The width revealed when the row is open on this edge.
    pub fn width(&self) -> f32 {
        self.actions.len() as f32 * ACTION_WIDTH
    }

    /// The id of the action a full swipe performs, if this edge allows it.
    pub fn full_swipe_action(&self) -> Option<u32> {
        if self.performs_first_full_swipe {
            self.actions.first().map(|a| a.id)
        } else {
            None
        }
    }
}

/// The edge of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowEdge {
    Leading,
    Trailing,
}

/// What a release decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwipeOutcome {
    /// The row is closed.
    Closed,
    /// The row rests open with the actions of this edge revealed.
    Open(RowEdge),
    /// A full swipe performed the first action of this edge. The row is closed again; the app animates the row out
    /// and runs the action.
    Performed { edge: RowEdge, id: u32 },
}

/// The swipe state of one list row.
#[derive(Debug)]
pub struct RowSwipe {
    row_width: f32,
    leading: SwipeConfig,
    trailing: SwipeConfig,
    /// The offset the row rests at (0, the leading width or the negated trailing width).
    settled: f32,
    /// The current offset: the settled offset while idle, the finger's position while dragging.
    offset: f32,
    /// The settled offset at the start of the drag, while a drag is running.
    origin: Option<f32>,
}

impl RowSwipe {
    pub fn new(row_width: f32, leading: SwipeConfig, trailing: SwipeConfig) -> Self {
        RowSwipe { row_width: row_width.max(0.0), leading, trailing, settled: 0.0, offset: 0.0, origin: None }
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    /// The offset the row rests at once the animation to it ends.
    pub fn settled(&self) -> f32 {
        self.settled
    }

    pub fn is_dragging(&self) -> bool {
        self.origin.is_some()
    }

    pub fn begin_drag(&mut self) {
        self.origin = Some(self.settled);
    }

    /// Moves the row by the total horizontal travel of the drag so far.
    pub fn drag(&mut self, total_dx: f32) {
        let origin = self.origin.unwrap_or(self.settled);
        self.origin = Some(origin);
        self.offset = self.clamp(origin + total_dx);
    }

    /// Whether the row is past the full swipe threshold on an edge that performs its first action.
    pub fn full_swipe_armed(&self) -> bool {
        let x = self.offset.abs();
        x > 0.0 && x >= self.row_width * FULL_SWIPE_FRACTION && self.full_action_for(self.offset).is_some()
    }

    /// Ends the drag and settles the row. A full swipe performs the first action; otherwise the row opens when it moved
    /// past half of the revealed buttons and closes when it did not.
    pub fn release(&mut self) -> SwipeOutcome {
        self.origin = None;
        if let Some(id) = self.full_action_for(self.offset).filter(|_| self.full_swipe_armed()) {
            let edge = if self.offset > 0.0 { RowEdge::Leading } else { RowEdge::Trailing };
            self.close();
            return SwipeOutcome::Performed { edge, id };
        }
        if self.offset > 0.0 && settles_open(self.offset, self.leading.width()) {
            self.settle(self.leading.width());
            return SwipeOutcome::Open(RowEdge::Leading);
        }
        if self.offset < 0.0 && settles_open(-self.offset, self.trailing.width()) {
            self.settle(-self.trailing.width());
            return SwipeOutcome::Open(RowEdge::Trailing);
        }
        self.close();
        SwipeOutcome::Closed
    }

    /// Opens the row on an edge without a gesture, for example from a menu command.
    pub fn open(&mut self, edge: RowEdge) {
        self.origin = None;
        match edge {
            RowEdge::Leading => self.settle(self.leading.width()),
            RowEdge::Trailing => self.settle(-self.trailing.width()),
        }
    }

    /// Closes the row at once.
    pub fn close(&mut self) {
        self.origin = None;
        self.settle(0.0);
    }

    /// The action button under a point of the row, in row coordinates (x from the leading edge). Only the revealed
    /// buttons are hit.
    pub fn action_at(&self, x: f32) -> Option<(RowEdge, u32)> {
        if self.offset > 0.0 {
            let revealed = self.offset.min(self.leading.width());
            if x >= 0.0 && x < revealed {
                let slot = (x / ACTION_WIDTH) as usize;
                return self.leading.actions.get(slot).map(|a| (RowEdge::Leading, a.id));
            }
        }
        if self.offset < 0.0 {
            let revealed = (-self.offset).min(self.trailing.width());
            if x <= self.row_width && x > self.row_width - revealed {
                let slot = ((self.row_width - x) / ACTION_WIDTH) as usize;
                return self.trailing.actions.get(slot).map(|a| (RowEdge::Trailing, a.id));
            }
        }
        None
    }

    fn settle(&mut self, to: f32) {
        self.settled = to;
        self.offset = to;
    }

    fn full_action_for(&self, offset: f32) -> Option<u32> {
        if offset > 0.0 {
            self.leading.full_swipe_action()
        } else if offset < 0.0 {
            self.trailing.full_swipe_action()
        } else {
            None
        }
    }

    fn clamp(&self, raw: f32) -> f32 {
        let (max, min) = (self.leading.width(), -self.trailing.width());
        if raw > max {
            if self.leading.full_swipe_action().is_some() {
                return raw.min(self.row_width);
            }
            return max + rubber_band(raw - max, RESIST);
        }
        if raw < min {
            if self.trailing.full_swipe_action().is_some() {
                return raw.max(-self.row_width);
            }
            return min + rubber_band(raw - min, RESIST);
        }
        raw
    }
}

fn settles_open(moved: f32, revealed: f32) -> bool {
    revealed > 0.0 && moved > revealed * OPEN_FRACTION
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trailing_pair() -> SwipeConfig {
        SwipeConfig::new(vec![ContextualAction::destructive(1, "Delete"), ContextualAction::new(2, "Flag")])
    }

    fn leading_read() -> SwipeConfig {
        SwipeConfig::new(vec![ContextualAction::new(7, "Read")])
    }

    #[test]
    fn a_short_drag_snaps_back_closed() {
        let mut r = RowSwipe::new(400.0, SwipeConfig::new(vec![]), trailing_pair());
        r.begin_drag();
        r.drag(-40.0);
        assert!(r.is_dragging());
        assert_eq!(r.release(), SwipeOutcome::Closed);
        assert_eq!(r.offset(), 0.0);
        assert!(!r.is_dragging());
    }

    #[test]
    fn a_drag_past_half_the_buttons_rests_open() {
        let mut r = RowSwipe::new(400.0, SwipeConfig::new(vec![]), trailing_pair());
        r.begin_drag();
        r.drag(-100.0);
        assert_eq!(r.release(), SwipeOutcome::Open(RowEdge::Trailing));
        assert_eq!(r.settled(), -160.0);
        assert_eq!(r.offset(), -160.0);
    }

    #[test]
    fn a_full_swipe_performs_the_first_action() {
        let mut r = RowSwipe::new(400.0, leading_read(), SwipeConfig::new(vec![]));
        r.begin_drag();
        r.drag(220.0);
        assert!(r.full_swipe_armed());
        assert_eq!(r.release(), SwipeOutcome::Performed { edge: RowEdge::Leading, id: 7 });
        assert_eq!(r.offset(), 0.0);
    }

    #[test]
    fn a_full_swipe_on_the_trailing_edge_performs_its_first_action() {
        let mut r = RowSwipe::new(400.0, SwipeConfig::new(vec![]), trailing_pair());
        r.begin_drag();
        r.drag(-300.0);
        assert_eq!(r.release(), SwipeOutcome::Performed { edge: RowEdge::Trailing, id: 1 });
    }

    #[test]
    fn a_full_swipe_disabled_only_opens_the_row() {
        let mut leading = leading_read();
        leading.performs_first_full_swipe = false;
        let mut r = RowSwipe::new(400.0, leading, SwipeConfig::new(vec![]));
        r.begin_drag();
        r.drag(350.0);
        assert!(!r.full_swipe_armed());
        assert_eq!(r.release(), SwipeOutcome::Open(RowEdge::Leading));
        assert_eq!(r.offset(), 80.0);
    }

    #[test]
    fn overshoot_without_full_swipe_is_rubber_banded() {
        let mut trailing = trailing_pair();
        trailing.performs_first_full_swipe = false;
        let mut r = RowSwipe::new(1200.0, SwipeConfig::new(vec![]), trailing);
        r.begin_drag();
        r.drag(-1000.0);
        assert!(r.offset() < -160.0 && r.offset() > -160.0 - RESIST, "{}", r.offset());
    }

    #[test]
    fn a_full_swipe_side_is_not_resisted_up_to_the_row_width() {
        let mut r = RowSwipe::new(400.0, leading_read(), SwipeConfig::new(vec![]));
        r.begin_drag();
        r.drag(300.0);
        assert_eq!(r.offset(), 300.0);
        r.drag(900.0);
        assert_eq!(r.offset(), 400.0, "clamped to the row width");
    }

    #[test]
    fn the_drag_is_measured_from_where_the_row_rests() {
        let mut r = RowSwipe::new(400.0, SwipeConfig::new(vec![]), trailing_pair());
        r.begin_drag();
        r.drag(-190.0);
        assert_eq!(r.release(), SwipeOutcome::Open(RowEdge::Trailing));
        r.begin_drag();
        r.drag(50.0);
        assert_eq!(r.offset(), -110.0, "a drag to the right from the open row closes it part way");
        assert_eq!(r.release(), SwipeOutcome::Open(RowEdge::Trailing));
    }

    #[test]
    fn open_and_close_without_a_gesture() {
        let mut r = RowSwipe::new(400.0, leading_read(), trailing_pair());
        r.open(RowEdge::Trailing);
        assert_eq!(r.offset(), -160.0);
        r.close();
        assert_eq!(r.offset(), 0.0);
        r.open(RowEdge::Leading);
        assert_eq!(r.settled(), 80.0);
    }

    #[test]
    fn action_at_hits_only_revealed_buttons() {
        let mut r = RowSwipe::new(400.0, leading_read(), trailing_pair());
        r.open(RowEdge::Trailing);
        assert_eq!(r.action_at(399.0), Some((RowEdge::Trailing, 1)));
        assert_eq!(r.action_at(300.0), Some((RowEdge::Trailing, 2)));
        assert_eq!(r.action_at(100.0), None);
        r.open(RowEdge::Leading);
        assert_eq!(r.action_at(10.0), Some((RowEdge::Leading, 7)));
        assert_eq!(r.action_at(100.0), None, "past the leading buttons");
        r.close();
        assert_eq!(r.action_at(10.0), None);
    }

    #[test]
    fn a_row_without_actions_never_opens() {
        let mut r = RowSwipe::new(400.0, SwipeConfig::new(vec![]), SwipeConfig::new(vec![]));
        r.begin_drag();
        r.drag(120.0);
        assert!(r.offset() > 0.0 && r.offset() <= RESIST);
        assert_eq!(r.release(), SwipeOutcome::Closed);
        assert_eq!(r.offset(), 0.0);
    }

    #[test]
    fn full_swipe_action_is_the_first_one() {
        assert_eq!(trailing_pair().full_swipe_action(), Some(1));
        assert_eq!(SwipeConfig::new(vec![]).full_swipe_action(), None);
        assert_eq!(trailing_pair().width(), 160.0);
    }
}
