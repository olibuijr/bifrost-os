//! The Details popover (Golden Gate Reminders): the "i" on the selected row opens a reminder's details in a popover beside
//! the row, over the list, instead of a form that replaces the list. The popover is the kit's `widgets::popover`; inside it
//! the details form keeps its rows (`form_rows`) and scrolls when it is taller than the window. The list keeps its own
//! scroll underneath (`list_scroll`). A click outside the popover closes it and keeps the edits (Done), as a transient
//! `NativePopover` does; Escape cancels them, Return in the title saves.
use super::*;
use bifrost_app::controls::geometry::Edge;
use bifrost_app::controls::Rect;
use bifrost_app::kit::ui::LINK;
use bifrost_app::kit::Surface;
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets::POPOVER_ARROW_PT;

/// The popover's width and inner padding, and the "i" button's width at a row's trailing edge (points).
pub(super) const DETAILS_W: f32 = 360.0;
pub(super) const DETAILS_PAD: f32 = 12.0;
pub(super) const INFO_W: f32 = 32.0;
const INFO_GLYPH: f32 = 18.0;

/// Where the popover goes, in content-column points: the "i" it points at, its size, the room it is placed in, and
/// the panel `place_popover` gives it (the same panel `widgets::popover` draws).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DetailsGeometry {
    pub anchor: Rect,
    pub size: (f32, f32),
    pub bounds: Rect,
    pub panel: Rect,
}

impl Reminders {
    /// Whether a saved reminder's details are open: they show in the popover over the list.
    pub(super) fn details_popover(&self) -> bool {
        self.screen == Screen::Editor && self.editor.as_ref().is_some_and(|e| e.id.is_some())
    }

    /// Whether row of reminder `id` shows its "i": the selected reminder in the list, and the one whose details are open.
    pub(super) fn info_shown(&self, id: u64) -> bool {
        match self.screen {
            Screen::Items => self.deleted.is_none() && self.selected == Some(id),
            Screen::Editor => self.editor.as_ref().and_then(|e| e.id) == Some(id),
            Screen::ListEditor => false,
        }
    }

    /// The "i" of list row `row` (content-column points).
    pub(super) fn info_rect(&self, ui: &Ui, row: usize) -> Rect {
        let k = ui.pt(1.0);
        let (x, y, w, h) = self.row_rect(ui, row);
        Rect { x: (x + w) / k - INFO_W, y: y / k, w: INFO_W, h: h / k }
    }

    /// The list's top and the list's scroll offset (px): while the popover is open the scroller is the form's, and the
    /// list stays where it was.
    pub(super) fn list_top_px(&self, ui: &Ui) -> f32 {
        ui.pt(self.top_pt + ITEMS_TOP + self.strip_h())
    }
    pub(super) fn list_offset(&self) -> f32 {
        if self.details_popover() { self.list_scroll } else { self.scroll.offset() }
    }

    /// The popover's place while the details are open.
    pub(super) fn details_geometry(&self, ui: &Ui) -> Option<DetailsGeometry> {
        if !self.details_popover() {
            return None;
        }
        let k = ui.pt(1.0);
        let top = self.top_pt + UNIFIED_BAR_PT + SPACE_S;
        let bounds = Rect { x: 0.0, y: top, w: self.w as f32 / k, h: (self.h as f32 / k - top - SPACE_S).max(0.0) };
        let id = self.editor.as_ref().and_then(|e| e.id);
        let row = self.rows.iter().position(|r| matches!(r, Row::Item { idx, .. } if Some(self.data.items[*idx].id) == id));
        // the row's "i", kept inside the list when the row has scrolled away (or is not in the list any more)
        let fallback = Rect { x: bounds.w - SPACE_XL - INFO_W, y: self.list_top_px(ui) / k, w: INFO_W, h: ITEM_H };
        let mut anchor = row.map_or(fallback, |r| self.info_rect(ui, r));
        anchor.y = anchor.y.clamp(bounds.y, (bounds.y + bounds.h - anchor.h).max(bounds.y));
        let size = (DETAILS_W.min(bounds.w), (self.form_rows().1 + 2.0 * DETAILS_PAD).min(bounds.h));
        let (panel, _) = bifrost_app::controls::split::place_popover(anchor, size, bounds, Edge::Right, POPOVER_ARROW_PT.1);
        Some(DetailsGeometry { anchor, size, bounds, panel })
    }

    /// Draws the "i" of a row whose middle is at `cy` and trailing edge at `right` (content pixels).
    pub(super) fn draw_info(&self, s: &mut Surface, ui: &Ui, right: f32, cy: f32) {
        let k = ui.pt(1.0);
        let g = INFO_GLYPH;
        let bounds = Rect { x: right / k - INFO_W / 2.0 - g / 2.0, y: cy / k - g / 2.0, w: g, h: g };
        bifrost_app::glyphs::glyph(s, ui, "info.circle", bounds, LINK);
    }

    /// Draws the popover and the details form in it, over the list (`draw` calls it after the list).
    pub(super) fn draw_details(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let Some(g) = self.details_geometry(ui) else { return };
        bifrost_app::widgets::popover(s, ui, g.anchor, g.size, g.bounds, Edge::Right);
        self.draw_form(s, ui, t);
    }

    /// A click outside the popover: the details are saved as Done saves them, or dropped when the title was emptied.
    pub(super) fn close_details(&mut self, ui: &Ui, t: f64) {
        if !clean(self.title.text()).is_empty() {
            self.save_editor(ui, t);
            // saved and closed, or the save failed and the edits stay on screen with the error
            if self.editor.is_none() || self.error {
                return;
            }
        }
        self.discard_new_pictures();
        self.close_form(ui, t);
    }
}
