//! Tooltips for SDK apps: a pointer that rests on a control for the delay shows that control's tooltip.
//!
//! The state machine is `bifrost_ui::tooltip::Tooltips`. [`TooltipHost`] keeps the controls' rectangles and texts,
//! feeds the pointer to the state machine, and says which tooltip shows at a time. [`draw`] paints the tooltip with
//! the ToolTip material (`widgets::material`) and its text, placed above the control and kept on screen.
//!
//! Hover delivery: the view layer reports touch moves only, and the shell does not yet send hover positions to apps.
//! An app drives the host with whatever pointer positions it receives (`pointer_moved`, `pointer_left`). Showing a
//! tooltip for a real mouse rests on the shell passing hover to the app, which is not wired yet; the timing, the
//! placement and the drawing are tested headless here and checked on Midget.

use crate::kit::ui::{Weight, LABEL};
use crate::kit::Ui;
use crate::widgets::{blit, material, Material};
use bifrost_render::canvas::Canvas;
use bifrost_render::ui::MAC_CALLOUT_PT;
use bifrost_ui::geometry::Edge;
use bifrost_ui::split::place_popover;
pub use bifrost_ui::tooltip::{Timing, Tooltips};
use bifrost_ui::Rect;

/// Space inside the tooltip panel around its text, in points.
pub const TOOLTIP_PAD_PT: (f32, f32) = (8.0, 4.0);
/// Space between the control and the tooltip, in points.
pub const TOOLTIP_GAP_PT: f32 = 4.0;
/// Corner radius of the tooltip panel, in points.
pub const TOOLTIP_RADIUS_PT: f32 = 6.0;

fn contains(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
}

/// The controls that carry tooltips, and the pointer state that decides which one shows.
pub struct TooltipHost {
    tooltips: Tooltips,
    /// (owner id, control rectangle in points, text), in paint order: the last one under the pointer wins
    regions: Vec<(u32, Rect, String)>,
    under: Option<u32>,
}

impl Default for TooltipHost {
    fn default() -> Self {
        Self::new(Timing::default())
    }
}

impl TooltipHost {
    pub fn new(timing: Timing) -> Self {
        TooltipHost { tooltips: Tooltips::new(timing), regions: Vec::new(), under: None }
    }

    /// Replaces the controls with their tooltips. Call it when the layout changes.
    pub fn set_regions(&mut self, regions: Vec<(u32, Rect, String)>) {
        self.regions = regions;
    }

    /// The owner under the point, if any.
    pub fn owner_at(&self, x: f32, y: f32) -> Option<u32> {
        self.regions.iter().rev().find(|(_, r, _)| contains(r, x, y)).map(|(id, _, _)| *id)
    }

    /// The pointer moved to `(x, y)` at `now` seconds. Moving from one control to another hands the tooltip over.
    pub fn pointer_moved(&mut self, x: f32, y: f32, now: f32) {
        let owner = self.owner_at(x, y);
        if owner != self.under {
            if let Some(prev) = self.under {
                self.tooltips.leave(prev, now);
            }
            if let Some(id) = owner {
                self.tooltips.enter(id, now);
            }
            self.under = owner;
        }
    }

    /// The pointer left the view at `now` seconds.
    pub fn pointer_left(&mut self, now: f32) {
        if let Some(prev) = self.under.take() {
            self.tooltips.leave(prev, now);
        }
    }

    /// The control rectangle and text of the tooltip that shows at `now`, if one does.
    pub fn visible(&mut self, now: f32) -> Option<(Rect, &str)> {
        let id = self.tooltips.tick(now)?;
        self.regions.iter().find(|(owner, _, _)| *owner == id).map(|(_, r, t)| (*r, t.as_str()))
    }
}

/// The size of a tooltip panel for `text`, in points. The width is estimated from the character count, since the
/// text is drawn at a fixed size; it is generous enough for the Latin text the shell's controls use.
pub fn panel_size(text: &str) -> (f32, f32) {
    let chars = text.chars().count() as f32;
    (chars * MAC_CALLOUT_PT * 0.55 + 2.0 * TOOLTIP_PAD_PT.0, MAC_CALLOUT_PT * 1.4 + 2.0 * TOOLTIP_PAD_PT.1)
}

/// Draws the tooltip for `text` above `control` inside `bounds` (points), flipping below when there is no room. Returns
/// the panel rectangle.
pub fn draw(s: &mut dyn Canvas, ui: &Ui, control: Rect, text: &str, bounds: Rect) -> Rect {
    let (panel, _) = place_popover(control, panel_size(text), bounds, Edge::Top, TOOLTIP_GAP_PT);
    material(s, ui, panel, TOOLTIP_RADIUS_PT, Material::ToolTip);
    let img = ui.text(text, MAC_CALLOUT_PT, Weight::Regular, LABEL.now());
    blit(s, &img, ui.pt(panel.x + TOOLTIP_PAD_PT.0) as i32, ui.pt(panel.y + TOOLTIP_PAD_PT.1) as i32);
    panel
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control(x: f32) -> Rect {
        Rect { x, y: 100.0, w: 40.0, h: 20.0 }
    }

    fn host() -> TooltipHost {
        let mut h = TooltipHost::default();
        h.set_regions(vec![(7, control(0.0), "Dictate".to_string()), (8, control(60.0), "Settings".to_string())]);
        h
    }

    #[test]
    fn a_resting_pointer_shows_its_controls_tooltip_after_the_delay() {
        let mut h = host();
        h.pointer_moved(10.0, 110.0, 0.0);
        assert!(h.visible(0.5).is_none());
        let (rect, text) = h.visible(1.5).unwrap();
        assert_eq!(text, "Dictate");
        assert_eq!(rect, control(0.0));
    }

    #[test]
    fn moving_to_a_neighbour_shows_its_tooltip_at_once() {
        let mut h = host();
        h.pointer_moved(10.0, 110.0, 0.0);
        h.visible(1.5);
        h.pointer_moved(70.0, 110.0, 1.6);
        assert_eq!(h.visible(1.6).map(|(_, t)| t), Some("Settings"));
    }

    #[test]
    fn leaving_the_view_hides_the_tooltip() {
        let mut h = host();
        h.pointer_moved(10.0, 110.0, 0.0);
        h.visible(2.0);
        h.pointer_left(2.1);
        assert!(h.visible(2.1).is_none());
    }

    #[test]
    fn a_point_outside_every_control_has_no_owner() {
        let h = host();
        assert_eq!(h.owner_at(200.0, 200.0), None);
        assert_eq!(h.owner_at(60.0, 110.0), Some(8));
    }

    #[test]
    fn the_panel_is_wide_enough_for_its_text_and_grows_with_it() {
        let short = panel_size("Hi");
        let long = panel_size("Dictate into the focused field");
        assert!(long.0 > short.0);
        assert_eq!(short.1, long.1, "one line of text has one height");
    }

    #[test]
    fn the_tooltip_panel_stays_on_screen_and_below_a_control_at_the_top() {
        let bounds = Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };
        let (panel, _) = place_popover(control(0.0), panel_size("Dictate"), bounds, Edge::Top, TOOLTIP_GAP_PT);
        assert!(panel.y + panel.h <= control(0.0).y, "above the control when there is room");
        let top = Rect { x: 0.0, y: 0.0, w: 40.0, h: 20.0 };
        let (flipped, edge) = place_popover(top, panel_size("Dictate"), bounds, Edge::Top, TOOLTIP_GAP_PT);
        assert_eq!(edge, Edge::Bottom);
        assert!(flipped.y >= top.y + top.h, "below the control when there is no room above");
    }
}
