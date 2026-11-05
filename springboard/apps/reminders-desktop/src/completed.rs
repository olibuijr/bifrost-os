//! The completed header of a list (Golden Gate Reminders): under the large title, "N Completed • Clear" on the leading
//! edge and Show or Hide on the trailing edge, over a hairline. Clear asks first in an alert sheet (the kit's
//! `widgets::sheet`, `NativeAlert`), then removes the list's completed reminders with Undo; Show and Hide flip the list's
//! Show Completed option. The strip is laid out once (`strip_parts`) for drawing and hit testing.
use super::*;
use bifrost_app::controls::sheet::{ButtonRole, Hit, Sheet, SheetButton};
use bifrost_app::controls::Rect;
use bifrost_app::kit::ui::{Weight, LABEL_2, LINK, MAC_BODY_PT, SEPARATOR_C, SPACE_XS};
use bifrost_app::kit::Surface;
use bifrost_app::layout::UNIFIED_BAR_PT;

/// The strip's top under the window's top inset, and its height (points). The list's rows move down by its height.
const STRIP_TOP: f32 = 96.0;
pub(super) const STRIP_H: f32 = 24.0;
/// The alert's buttons.
pub(super) const ASK_CLEAR: u32 = 1;
pub(super) const ASK_CANCEL: u32 = 2;
/// Extra touch room around the strip's links (points).
const SLOP: f32 = 6.0;
/// " • " between the count and Clear.
const SEP: &str = " \u{2022} ";

/// "2 Completed".
pub(super) fn count_label(n: usize) -> String {
    format!("{n} Completed")
}

/// The question the Clear alert asks for `n` completed reminders.
pub(super) fn clear_question(n: usize) -> String {
    let noun = if n == 1 { "reminder" } else { "reminders" };
    format!("Clear {n} completed {noun}?")
}

/// The strip's pieces in column pixels: the count label's left, Clear's span and Show/Hide's span (left, width).
pub(super) struct StripParts {
    pub y: f32,
    pub h: f32,
    pub label_x: f32,
    pub clear: (f32, f32),
    pub toggle: (f32, f32),
}

impl Reminders {
    /// The completed top-level reminders of the open list: the strip shows while there are some, on a list (not a smart
    /// list or a search) and not on the Recently Deleted page.
    pub(super) fn completed_in_list(&self) -> usize {
        let View::List(list) = self.view else { return 0 };
        if self.searching || self.deleted.is_some() {
            return 0;
        }
        self.data.items.iter().filter(|r| r.list == list && r.completed && r.parent.is_none()).count()
    }

    /// The strip's height in points, 0 while it is not shown.
    pub(super) fn strip_h(&self) -> f32 {
        if self.completed_in_list() > 0 { STRIP_H } else { 0.0 }
    }

    /// The label of the trailing link: Hide while completed reminders are shown in the list, Show while they are not.
    fn toggle_label(&self) -> &'static str {
        if self.opts.show_completed { "Hide" } else { "Show" }
    }

    pub(super) fn strip_parts(&self, ui: &Ui) -> Option<StripParts> {
        let n = self.completed_in_list();
        if n == 0 {
            return None;
        }
        let label_x = ui.pt(SPACE_XL + SPACE_XS);
        let clear_x = label_x + ui.measure(&count_label(n), MAC_BODY_PT, Weight::Regular) + ui.measure(SEP, MAC_BODY_PT, Weight::Regular);
        let clear_w = ui.measure("Clear", MAC_BODY_PT, Weight::Regular);
        let toggle_w = ui.measure(self.toggle_label(), MAC_BODY_PT, Weight::Regular);
        let right = self.w as f32 - ui.pt(SPACE_XL + SPACE_XS);
        Some(StripParts { y: ui.pt(self.top_pt + STRIP_TOP), h: ui.pt(STRIP_H), label_x, clear: (clear_x, clear_w), toggle: (right - toggle_w, toggle_w) })
    }

    /// The strip's link under content pixel `(x, y)`.
    pub(super) fn strip_target(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let p = self.strip_parts(ui)?;
        if y < p.y || y >= p.y + p.h {
            return None;
        }
        let slop = ui.pt(SLOP);
        let on = |(left, w): (f32, f32)| x >= left - slop && x < left + w + slop;
        if on(p.clear) {
            return Some(Target::ClearAsk);
        }
        if on(p.toggle) {
            return Some(Target::OptCompleted);
        }
        None
    }

    /// Draws the strip (`draw_items` calls it under the large title).
    pub(super) fn draw_strip(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(p) = self.strip_parts(ui) else { return };
        let n = self.completed_in_list();
        let label = format!("{}{SEP}", count_label(n));
        let ty = p.y + (p.h - ui.pt(MAC_BODY_PT * 1.25)) / 2.0;
        self.text(s, ui, &label, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), p.label_x, ty);
        self.text(s, ui, "Clear", MAC_BODY_PT, Weight::Regular, LINK, p.clear.0, ty);
        let toggle = self.toggle_label();
        self.text(s, ui, toggle, MAC_BODY_PT, Weight::Regular, LINK, p.toggle.0, ty);
        let hair = ui.pt(0.5).max(1.0);
        s.fill_rrect(ui.pt(SPACE_XL), p.y + p.h - hair, self.w as f32 - ui.pt(SPACE_HUGE), hair, 0.0, SEPARATOR_C.now());
    }

    /// The Clear alert while it is up (or Recently Deleted's purge alert, `purge.rs`, which shares its buttons).
    pub(super) fn clear_sheet(&self) -> Option<Sheet> {
        if self.purge_ask.is_some() {
            return self.purge_sheet();
        }
        if !self.clear_ask {
            return None;
        }
        let n = self.completed_in_list();
        let buttons = vec![SheetButton::new(ASK_CLEAR, "Clear", ButtonRole::Default), SheetButton::new(ASK_CANCEL, "Cancel", ButtonRole::Cancel)];
        Some(Sheet::alert(&clear_question(n), "They are removed from this list. Edit > Undo puts them back.", buttons).with_focus(self.ask_focus))
    }

    /// A key press while the Clear or purge alert is up, through the kit's sheet keys (`Sheet::handle_key`): Return
    /// presses the default Clear (the purge alert's Delete is destructive, so never Return), Escape and Cmd+. cancel,
    /// Cmd+D presses a plain Delete, Tab moves the focus ring (drawn on the button) and Space presses the focused one.
    /// The alert takes every key. True when a frame is needed.
    pub(super) fn ask_key(&mut self, ui: &Ui, key: &bifrost_app::KeyInput, t: f64) -> bool {
        use bifrost_app::document::{sheet_key, AlertKey};
        let Some(mut sheet) = self.clear_sheet() else { return false };
        match sheet_key(&mut sheet, key) {
            AlertKey::Answer(ASK_CLEAR) if self.purge_ask.is_some() => self.activate(ui, Target::PurgeConfirm, t),
            AlertKey::Answer(ASK_CLEAR) => self.activate(ui, Target::ClearConfirm, t),
            AlertKey::Answer(_) => self.activate(ui, Target::ClearCancel, t),
            AlertKey::Focus => self.ask_focus = sheet.focus,
            AlertKey::Ignored => return false,
        }
        self.dirty = true;
        true
    }

    /// The window in points (the sheet's parent) and its title bar's height.
    pub(super) fn sheet_parent(&self, ui: &Ui) -> (Rect, f32) {
        let k = ui.pt(1.0);
        (Rect { x: 0.0, y: 0.0, w: self.full_w as f32 / k, h: self.h as f32 / k }, self.top_pt + UNIFIED_BAR_PT)
    }

    /// What a press at window pixel `(x, y)` does while the alert is up: its buttons answer, the rest is blocked.
    pub(super) fn ask_target(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let sheet = self.clear_sheet()?;
        let (parent, bar) = self.sheet_parent(ui);
        let k = ui.pt(1.0);
        match sheet.hit(parent, bar, 1.0, x / k, y / k) {
            Hit::Button(ASK_CLEAR) if self.purge_ask.is_some() => Some(Target::PurgeConfirm),
            Hit::Button(ASK_CLEAR) => Some(Target::ClearConfirm),
            Hit::Button(_) => Some(Target::ClearCancel),
            _ => None,
        }
    }

    /// Draws the alert over the whole window (`draw` calls it last).
    pub(super) fn draw_clear_ask(&self, s: &mut Surface, ui: &Ui) {
        let Some(sheet) = self.clear_sheet() else { return };
        let (parent, bar) = self.sheet_parent(ui);
        bifrost_app::widgets::sheet(s, ui, parent, bar, &sheet, 1.0, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strip_and_the_alert_read_the_count() {
        assert_eq!(count_label(2), "2 Completed");
        assert_eq!(clear_question(1), "Clear 1 completed reminder?");
        assert_eq!(clear_question(3), "Clear 3 completed reminders?");
    }
}
