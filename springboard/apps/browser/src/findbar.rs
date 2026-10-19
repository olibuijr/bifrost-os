//! Find in page (Cmd+F; Browser's Find banner, `WebView.find(_:configuration:)`). The bar sits under the tab strip
//! and above the content, so `content_top` grows by `FIND_BAR_PT` and `place_web` shrinks the frame. It holds the
//! search field, the previous / next capsule (also Cmd+G, Cmd+Shift+G, and Return in the field), the match count and
//! Done (also Esc). The engine selects the current match with the page's selection; it reports the number of matches
//! but not which one is current, and the other matches are not highlighted (Servo 0.7.0 has no find API).
use bifrost_app::controls::{Node, Rect, Role, TextField};
use bifrost_app::kit::ui::{Weight, CARD_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_SUBHEAD_PT, ROW_PRESSED, SEPARATOR_C};
use bifrost_app::kit::{Surface, Ui};
use bifrost_app::proto::CHORD_SHIFT;
use bifrost_app::webview::FindDirection;
use bifrost_app::{widgets, KeyInput};

use crate::{Browser, Press};

/// Find bar height, points.
pub const FIND_BAR_PT: f32 = 36.0;
/// Side margin and the space between the controls.
const PAD_PT: f32 = 12.0;
const GAP_PT: f32 = 8.0;
/// Height of the controls inside the bar.
const CONTROL_PT: f32 = FIND_BAR_PT - 12.0;
/// Width of one step button (the previous / next capsule holds two) and of Done.
const STEP_PT: f32 = 28.0;
const DONE_PT: f32 = 64.0;
/// Room kept for the match count ("100 matches").
const COUNT_PT: f32 = 96.0;
const FIELD_MIN_PT: f32 = 120.0;
const FIELD_MAX_PT: f32 = 320.0;
/// evdev Enter: Shift+Return arrives as this key with `CHORD_SHIFT` when the shell reports it as a chord.
const KEY_ENTER: u32 = 28;

/// Control ids. They are toolbar-style ids below the tab ids (`Press::Bar`, accessibility activation).
pub const FIELD: u32 = 30;
pub const PREV: u32 = 31;
pub const NEXT: u32 = 32;
pub const DONE: u32 = 33;

#[derive(Default)]
pub struct FindBar {
    pub open: bool,
    pub field: Option<TextField>,
}

/// Where the controls sit in the bar (points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub field: Rect,
    pub prev: Rect,
    pub next: Rect,
    /// Where the match count is written, left aligned.
    pub count: Rect,
    pub done: Rect,
}

/// The controls of a bar `width` points wide whose top edge is `top`: the field, the previous / next capsule and the
/// count from the leading edge, Done at the trailing edge. The field takes what the rest leaves, within its limits.
pub fn layout(width: f32, top: f32) -> Layout {
    let y = top + (FIND_BAR_PT - CONTROL_PT) / 2.0;
    let reserved = 2.0 * PAD_PT + 3.0 * GAP_PT + 2.0 * STEP_PT + COUNT_PT + DONE_PT;
    let field = Rect { x: PAD_PT, y, w: (width - reserved).clamp(FIELD_MIN_PT, FIELD_MAX_PT), h: CONTROL_PT };
    let prev = Rect { x: field.x + field.w + GAP_PT, y, w: STEP_PT, h: CONTROL_PT };
    let next = Rect { x: prev.x + STEP_PT, y, w: STEP_PT, h: CONTROL_PT };
    let count = Rect { x: next.x + STEP_PT + GAP_PT, y, w: COUNT_PT, h: CONTROL_PT };
    let done = Rect { x: (width - PAD_PT - DONE_PT).max(0.0), y, w: DONE_PT, h: CONTROL_PT };
    Layout { field, prev, next, count, done }
}

/// The control under a point (points) in a bar of `width` whose top edge is `top`: a button's id, `FIELD` for the
/// rest of the bar (so a click on it never reaches the page), None outside the bar.
pub fn hit(width: f32, top: f32, px: f32, py: f32) -> Option<u32> {
    if py < top || py >= top + FIND_BAR_PT {
        return None;
    }
    let l = layout(width, top);
    Some([(PREV, l.prev), (NEXT, l.next), (DONE, l.done)].into_iter().find(|(_, r)| r.contains(px, py)).map_or(FIELD, |(id, _)| id))
}

/// The count line for the number of matches the engine reported (`None` until it answers).
pub fn count_label(count: Option<usize>, query_empty: bool) -> String {
    match count {
        Some(0) => "Not found".to_string(),
        Some(n) => format!("{n} match{}", if n == 1 { "" } else { "es" }),
        None if query_empty => String::new(),
        None => "Searching".to_string(),
    }
}

impl Browser {
    pub fn find_height(&self) -> f32 {
        if self.find.open { FIND_BAR_PT } else { 0.0 }
    }

    pub fn show_find(&mut self) {
        self.find.open = true;
        self.find.field.get_or_insert_with(TextField::new);
    }

    pub fn hide_find(&mut self) {
        self.find.open = false;
        if let (Some(p), Some(v)) = (&mut self.pool, self.tabs.get(self.current).map(|t| t.view)) {
            p.find_clear(v);
        }
    }

    pub fn find_step(&mut self, dir: FindDirection) {
        let q = self.find.field.as_ref().map(|f| f.to_string()).unwrap_or_default();
        if let (Some(p), Some(v)) = (&mut self.pool, self.tabs.get(self.current).map(|t| t.view)) {
            if q.is_empty() {
                p.find_clear(v);
            } else {
                p.find(v, &q, dir);
            }
        }
    }

    /// Typing in the find field; returns true when the key was used. Return is next (Shift+Return previous), Esc closes.
    pub fn find_key(&mut self, k: &KeyInput) -> bool {
        let Some(f) = self.find.field.as_mut() else { return false };
        match k {
            KeyInput::Text(s) => {
                f.push_str(s);
                self.find_step(FindDirection::New);
            }
            KeyInput::Code(8) => {
                f.pop();
                self.find_step(FindDirection::New);
            }
            KeyInput::Code(13) => self.find_step(FindDirection::Forward),
            _ if k.as_chord() == Some((KEY_ENTER, CHORD_SHIFT)) => self.find_step(FindDirection::Backward),
            KeyInput::Code(1) | KeyInput::Code(27) => self.hide_find(),
            KeyInput::Code(37) => f.move_caret(false),
            KeyInput::Code(39) => f.move_caret(true),
            _ => return false,
        }
        true
    }

    /// The match count the engine last reported for the selected tab.
    fn find_matches(&self) -> Option<usize> {
        self.view().and_then(|v| self.pool.as_ref()?.state(v)).and_then(|st| st.find_matches)
    }

    /// The bar's controls for accessibility: the field, previous, next and Done.
    pub fn find_controls(&self, ui: &Ui, w: u32) -> Vec<Node> {
        let l = layout(w as f32 / ui.pt(1.0), self.content_top() - FIND_BAR_PT);
        let query = self.find.field.as_ref().map(|f| f.to_string()).unwrap_or_default();
        let can_step = self.find_matches().is_some_and(|n| n > 0);
        let button = |id: u32, label: &str, bounds: Rect, enabled: bool| Node { id: id as u64, role: Role::Button, label: label.into(), value: String::new(), bounds, enabled };
        vec![
            Node { id: FIELD as u64, role: Role::TextField, label: "Find in page".into(), value: query, bounds: l.field, enabled: true },
            button(PREV, "Previous Match", l.prev, can_step),
            button(NEXT, "Next Match", l.next, can_step),
            button(DONE, "Done", l.done, true),
        ]
    }

    pub fn draw_find(&self, s: &mut Surface, ui: &Ui, wpt: f32) {
        if !self.find.open {
            return;
        }
        let top = self.content_top() - FIND_BAR_PT;
        let (y, h) = (ui.pt(top) as usize, ui.pt(FIND_BAR_PT).ceil() as usize);
        s.fill_rect(0, y, s.w, h, CARD_BG.now());
        s.fill_rect(0, y + h - 1, s.w, 1, SEPARATOR_C.now());
        let l = layout(wpt, top);
        if let Some(f) = &self.find.field {
            widgets::search_field(s, ui, f, l.field, "Find in page", true);
        }
        let pressed = match self.press {
            Some((Press::Bar(id), _, _)) => Some(id),
            _ => None,
        };
        let count = self.find_matches();
        let can_step = count.is_some_and(|n| n > 0);
        // previous and next: one glass capsule with a hairline between the two glyphs
        let group = Rect { x: l.prev.x, y: l.prev.y, w: l.prev.w + l.next.w, h: l.prev.h };
        widgets::capsule(s, ui, group);
        widgets::outline_if_borders(s, ui, group, group.h / 2.0);
        s.fill_rect(ui.pt(l.next.x) as usize, ui.pt(group.y + group.h * 0.25) as usize, ui.pt(1.0).max(1.0) as usize, ui.pt(group.h * 0.5) as usize, SEPARATOR_C.now());
        for (id, r, name) in [(PREV, l.prev, "chevron.up"), (NEXT, l.next, "chevron.down")] {
            if pressed == Some(id) && can_step {
                s.fill_rrect(ui.pt(r.x + 2.0), ui.pt(r.y + 2.0), ui.pt(r.w - 4.0), ui.pt(r.h - 4.0), ui.pt((r.h - 4.0) / 2.0), ROW_PRESSED.now());
            }
            bifrost_app::glyphs::glyph(s, ui, name, Self::glyph_box(r, 0.2), if can_step { LABEL.now() } else { LABEL_2.now() });
        }
        let q_empty = self.find.field.as_ref().is_none_or(|f| f.to_string().is_empty());
        let label = count_label(count, q_empty);
        if !label.is_empty() {
            let img = ui.text(&label, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now());
            s.blit(&img, ui.pt(l.count.x) as i32, ui.pt(l.count.y + l.count.h / 2.0) as i32 - img.h as i32 / 2, 255, 255);
        }
        // Done closes the bar
        widgets::capsule(s, ui, l.done);
        widgets::outline_if_borders(s, ui, l.done, l.done.h / 2.0);
        if pressed == Some(DONE) {
            s.fill_rrect(ui.pt(l.done.x + 3.0), ui.pt(l.done.y + 3.0), ui.pt(l.done.w - 6.0), ui.pt(l.done.h - 6.0), ui.pt((l.done.h - 6.0) / 2.0), ROW_PRESSED.now());
        }
        let img = ui.text("Done", MAC_BODY_PT, Weight::Regular, LABEL.now());
        s.blit(&img, ui.pt(l.done.x + l.done.w / 2.0) as i32 - img.w as i32 / 2, ui.pt(l.done.y + l.done.h / 2.0) as i32 - img.h as i32 / 2, 255, 255);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_controls_fit_a_normal_window_in_order() {
        let l = layout(800.0, 100.0);
        assert!(l.field.x >= PAD_PT && l.field.right() < l.prev.x, "the field comes first");
        assert_eq!(l.prev.right(), l.next.x, "previous and next touch: one capsule");
        assert!(l.next.right() < l.count.x && l.count.right() <= l.done.x, "then the count, then Done");
        assert_eq!(l.done.right(), 800.0 - PAD_PT, "Done sits at the trailing margin");
        for r in [l.field, l.prev, l.next, l.count, l.done] {
            assert_eq!((r.y, r.h), (100.0 + 6.0, CONTROL_PT), "the controls are centred in the bar");
        }
    }

    #[test]
    fn the_field_is_limited_on_wide_and_narrow_windows() {
        assert_eq!(layout(3000.0, 0.0).field.w, FIELD_MAX_PT);
        assert_eq!(layout(200.0, 0.0).field.w, FIELD_MIN_PT);
        let l = layout(0.0, 0.0);
        assert_eq!(l.done.x, 0.0, "a window with no width does not push Done off to a negative position");
    }

    #[test]
    fn a_click_in_the_bar_is_a_control_or_the_bar_never_the_page() {
        let (w, top) = (800.0, 100.0);
        let l = layout(w, top);
        let mid = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let (x, y) = mid(l.prev);
        assert_eq!(hit(w, top, x, y), Some(PREV));
        let (x, y) = mid(l.next);
        assert_eq!(hit(w, top, x, y), Some(NEXT));
        let (x, y) = mid(l.done);
        assert_eq!(hit(w, top, x, y), Some(DONE));
        assert_eq!(hit(w, top, l.count.x + 1.0, top + 1.0), Some(FIELD), "empty bar space is the bar's");
        assert_eq!(hit(w, top, 10.0, top - 1.0), None, "above the bar");
        assert_eq!(hit(w, top, 10.0, top + FIND_BAR_PT), None, "below the bar");
    }

    #[test]
    fn the_count_label_says_what_the_engine_found() {
        assert_eq!(count_label(Some(0), false), "Not found");
        assert_eq!(count_label(Some(1), false), "1 match");
        assert_eq!(count_label(Some(6), false), "6 matches");
        assert_eq!(count_label(None, true), "");
        assert_eq!(count_label(None, false), "Searching");
    }

    #[test]
    fn control_ids_are_distinct_and_below_the_tab_ids() {
        let ids = [FIELD, PREV, NEXT, DONE];
        for (i, a) in ids.iter().enumerate() {
            assert!(*a < 100, "{a} must stay below the tab ids");
            assert!(ids[i + 1..].iter().all(|b| b != a), "{a} is used twice");
        }
    }
}
