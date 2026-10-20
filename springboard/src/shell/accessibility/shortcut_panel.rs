//! The Accessibility Shortcut panel (Option+Cmd+F5). The key opens a floating panel that lists the features chosen for the
//! shortcut (`shortcut::PREF`), each with a checkbox that shows whether the feature is on. A tap on a row switches that one
//! feature and saves the choice; Done, Escape, or the shortcut again hides the panel. While it is up it takes every touch,
//! as the character viewer does. Geometry is pure (`panel`, `row_rect`, `done_rect`, `hit`) and tested on the host; the
//! `Shell` methods at the bottom are the glue.

use super::shortcut::{self, Feature};
use super::AccessibilitySettings;
use crate::gfx::Surface;
use crate::shell::glyphs;
use crate::shell::ui::{Ui, Weight, CARD_BG, LABEL, LINK, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_TITLE3_PT, SECONDARY, SEPARATOR, WHITE};
use crate::shell::{Drawn, Effect, Event, Shell};

/// Panel width, the side padding, the title row (with Done), one feature row, the footer hint, in points.
const PANEL_PT: f32 = 320.0;
const PAD_PT: f32 = 16.0;
const TITLE_PT: f32 = 40.0;
const ROW_PT: f32 = 36.0;
const FOOT_PT: f32 = 36.0;
/// The checkbox square, its gap to the feature name, and its corner radius.
const BOX_PT: f32 = 18.0;
const BOX_GAP_PT: f32 = 12.0;
const BOX_RADIUS_PT: f32 = 4.0;
const DONE_W_PT: f32 = 56.0;
const RADIUS_PT: f32 = 13.0;
/// Top of the panel as a share of the screen height.
const TOP_SHARE: f32 = 0.22;
/// The hint under the rows when no feature is chosen, and the one when some are.
const HINT_NONE: &str = "Choose features in Settings > Accessibility";
const HINT_SOME: &str = "Choose a feature to switch it on or off";

/// x, y, w, h in px.
type Rect = (f32, f32, f32, f32);

/// What a touch on the panel lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Done,
    /// index into the panel's features
    Feature(usize),
    /// the panel itself, where no control is
    Panel,
}

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

/// The rows reserved for `n` features: at least one, so the empty hint has a row to sit in.
fn rows(n: usize) -> usize {
    n.max(1)
}

/// The panel rectangle for `n` features. `s` is one point in pixels; the panel is centred, its top at `TOP_SHARE` of the height.
pub fn panel(s: f32, w: f32, h: f32, n: usize) -> Rect {
    let pw = PANEL_PT * s;
    let ph = (TITLE_PT + rows(n) as f32 * ROW_PT + FOOT_PT) * s;
    ((w - pw) / 2.0, h * TOP_SHARE, pw, ph)
}

/// The rectangle of feature row `i`.
pub fn row_rect(s: f32, w: f32, h: f32, n: usize, i: usize) -> Rect {
    let (x, y, pw, _) = panel(s, w, h, n);
    (x, y + (TITLE_PT + i as f32 * ROW_PT) * s, pw, ROW_PT * s)
}

/// The Done button, at the right of the title row.
pub fn done_rect(s: f32, w: f32, h: f32, n: usize) -> Rect {
    let (x, y, pw, _) = panel(s, w, h, n);
    (x + pw - (PAD_PT + DONE_W_PT) * s, y, DONE_W_PT * s, TITLE_PT * s)
}

/// What a touch at (x, y) lands on for `n` features; None when it is off the panel.
pub fn hit(s: f32, w: f32, h: f32, n: usize, x: f32, y: f32) -> Option<Hit> {
    if !inside(panel(s, w, h, n), x, y) {
        return None;
    }
    if inside(done_rect(s, w, h, n), x, y) {
        return Some(Hit::Done);
    }
    if let Some(i) = (0..n).find(|&i| inside(row_rect(s, w, h, n, i), x, y)) {
        return Some(Hit::Feature(i));
    }
    Some(Hit::Panel)
}

/// The open panel: the features the shortcut switches, and the settings they act on (kept in step with the Settings copy).
#[derive(Clone, Debug, PartialEq)]
pub struct ShortcutPanel {
    a: AccessibilitySettings,
    features: Vec<Feature>,
}

impl ShortcutPanel {
    /// A panel for the features chosen in `a`, in panel order.
    pub fn new(a: AccessibilitySettings) -> Self {
        let features = Feature::ALL.into_iter().filter(|f| a.shortcut.contains(f)).collect();
        ShortcutPanel { a, features }
    }

    /// The features listed, in panel order.
    pub fn features(&self) -> &[Feature] {
        &self.features
    }

    /// The settings the panel edits.
    pub fn settings(&self) -> &AccessibilitySettings {
        &self.a
    }

    /// Switches feature `i` on, or off when it is on, and returns the settings to save. None for a row past the list.
    pub fn toggle(&mut self, i: usize) -> Option<AccessibilitySettings> {
        let f = *self.features.get(i)?;
        shortcut::toggle(&mut self.a, &[f]);
        Some(self.a.clone())
    }

    /// What a touch at (x, y) lands on in a `w` by `h` screen, with `s` one point in pixels.
    pub fn hit(&self, s: f32, w: f32, h: f32, x: f32, y: f32) -> Option<Hit> {
        hit(s, w, h, self.features.len(), x, y)
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        let sc = ui.pt(1.0);
        let (wf, hf) = (w as f32, h as f32);
        let n = self.features.len();
        let (x, y, pw, ph) = panel(sc, wf, hf, n);
        s.fill_rrect(x, y, pw, ph, ui.pt(RADIUS_PT), CARD_BG.now());
        s.stroke_rrect(x, y, pw, ph, ui.pt(RADIUS_PT), 1.0, SEPARATOR);

        let title = ui.text("Accessibility Shortcut", MAC_TITLE3_PT, Weight::Semibold, WHITE);
        s.blit_colored(&title, (x + PAD_PT * sc) as i32, (y + (TITLE_PT * sc - title.h as f32) / 2.0) as i32, LABEL.now(), 255);
        let (dx, dy, dw, dh) = done_rect(sc, wf, hf, n);
        let done = ui.text("Done", MAC_BODY_PT, Weight::Semibold, WHITE);
        s.blit_colored(&done, (dx + (dw - done.w as f32) / 2.0) as i32, (dy + (dh - done.h as f32) / 2.0) as i32, LINK, 255);

        if self.features.is_empty() {
            let (rx, ry, _, rh) = row_rect(sc, wf, hf, n, 0);
            let msg = ui.text("No features are chosen", MAC_BODY_PT, Weight::Regular, WHITE);
            s.blit_colored(&msg, (rx + PAD_PT * sc) as i32, (ry + (rh - msg.h as f32) / 2.0) as i32, SECONDARY, 255);
        }
        for (i, f) in self.features.iter().enumerate() {
            let (rx, ry, _, rh) = row_rect(sc, wf, hf, n, i);
            let bs = BOX_PT * sc;
            let (bx, by) = (rx + PAD_PT * sc, ry + (rh - bs) / 2.0);
            if f.is_on(&self.a) {
                s.fill_rrect(bx, by, bs, bs, BOX_RADIUS_PT * sc, LINK);
                let t = bs * 0.12;
                let (p1, p2, p3) = ((bx + bs * 0.24, by + bs * 0.52), (bx + bs * 0.42, by + bs * 0.70), (bx + bs * 0.76, by + bs * 0.30));
                glyphs::paint(s, (bx, by, bs, bs), WHITE, glyphs::either(glyphs::segment(p1, p2, t), glyphs::segment(p2, p3, t)));
            } else {
                s.stroke_rrect(bx, by, bs, bs, BOX_RADIUS_PT * sc, 1.5 * sc, SECONDARY);
            }
            let label = ui.text(f.name(), MAC_BODY_PT, Weight::Regular, WHITE);
            s.blit_colored(&label, (bx + bs + BOX_GAP_PT * sc) as i32, (ry + (rh - label.h as f32) / 2.0) as i32, LABEL.now(), 255);
        }

        let foot_y = y + (TITLE_PT + rows(n) as f32 * ROW_PT) * sc;
        let hint = if self.features.is_empty() { HINT_NONE } else { HINT_SOME };
        let foot = ui.text(hint, MAC_CALLOUT_PT, Weight::Regular, WHITE);
        s.blit_colored(&foot, (x + (pw - foot.w as f32) / 2.0) as i32, (foot_y + (FOOT_PT * sc - foot.h as f32) / 2.0) as i32, SECONDARY, 255);
    }
}

impl Shell {
    /// Requests a redraw of the whole frame, the panel's state having changed.
    pub(in crate::shell) fn a11y_panel_changed(&mut self) {
        self.drawn = Drawn::Nothing;
        self.dirty = true;
    }

    /// Escape hides the panel. True when it was up (the key is then used up).
    pub(in crate::shell) fn a11y_panel_escape(&mut self) -> bool {
        if self.a11y_panel.take().is_none() {
            return false;
        }
        self.a11y_panel_changed();
        true
    }

    /// A touch while the panel is up: a release on Done closes it, a release on a row switches that feature and saves.
    /// Every touch is used up by the panel.
    pub(in crate::shell) fn a11y_panel_event(&mut self, ev: Event) -> Vec<Effect> {
        let Event::Up(x, y) = ev else { return vec![] };
        let sc = self.ui.pt(1.0);
        let (w, h) = (self.w as f32, self.h as f32);
        let hit = self.a11y_panel.as_ref().and_then(|p| p.hit(sc, w, h, x, y));
        match hit {
            Some(Hit::Done) => self.a11y_panel = None,
            Some(Hit::Feature(i)) => {
                let saved = self.a11y_panel.as_mut().and_then(|p| p.toggle(i));
                if let Some(a) = saved {
                    self.save_accessibility(a);
                }
            }
            None | Some(Hit::Panel) => {}
        }
        self.a11y_panel_changed();
        vec![]
    }

    /// Saves the panel's change like the Settings page does: the pref, the open Settings copy, and the live hooks.
    fn save_accessibility(&mut self, a: AccessibilitySettings) {
        a.save(&mut self.prefs);
        if let Some(st) = &mut self.settings {
            st.set_accessibility(a.clone());
        }
        a.publish();
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(features: Vec<Feature>) -> ShortcutPanel {
        ShortcutPanel::new(AccessibilitySettings { shortcut: features, ..AccessibilitySettings::DEFAULT })
    }

    #[test]
    fn the_panel_lists_the_chosen_features_in_panel_order() {
        let p = with(vec![Feature::ReduceMotion, Feature::ScreenReader]);
        assert_eq!(p.features(), &[Feature::ScreenReader, Feature::ReduceMotion]);
        assert!(with(vec![]).features().is_empty());
    }

    #[test]
    fn a_row_tap_switches_only_its_feature_and_returns_the_settings_to_save() {
        let mut p = with(vec![Feature::ReduceMotion, Feature::IncreaseContrast]);
        let a = p.toggle(1).expect("row 1 exists");
        assert!(a.increase_contrast && !a.reduce_motion, "only the tapped feature changes");
        assert!(p.settings().increase_contrast, "the panel keeps the change for the next tap");
        let b = p.toggle(1).expect("row 1 exists");
        assert!(!b.increase_contrast);
        assert!(p.toggle(5).is_none(), "a row past the list changes nothing");
    }

    #[test]
    fn a_checked_row_reads_the_feature_state() {
        let p = with(vec![Feature::Zoom]);
        assert!(!p.features()[0].is_on(p.settings()));
        let mut q = p.clone();
        q.toggle(0);
        assert!(q.features()[0].is_on(q.settings()), "zoom reads on once tapped");
    }

    #[test]
    fn the_panel_is_centred_and_its_rows_stack_under_the_title() {
        let (x, y, pw, ph) = panel(1.0, 400.0, 1000.0, 2);
        assert_eq!((x, pw), (40.0, PANEL_PT));
        assert_eq!(y, 220.0);
        assert_eq!(ph, TITLE_PT + 2.0 * ROW_PT + FOOT_PT);
        assert_eq!(row_rect(1.0, 400.0, 1000.0, 2, 1), (40.0, 220.0 + TITLE_PT + ROW_PT, PANEL_PT, ROW_PT));
    }

    #[test]
    fn a_touch_lands_on_done_a_row_the_panel_or_nothing() {
        let s = 1.0;
        assert_eq!(hit(s, 400.0, 1000.0, 2, 300.0, 230.0), Some(Hit::Done));
        assert_eq!(hit(s, 400.0, 1000.0, 2, 60.0, 230.0), Some(Hit::Panel), "the title, left of Done");
        assert_eq!(hit(s, 400.0, 1000.0, 2, 60.0, 270.0), Some(Hit::Feature(0)));
        assert_eq!(hit(s, 400.0, 1000.0, 2, 60.0, 300.0), Some(Hit::Feature(1)));
        assert_eq!(hit(s, 400.0, 1000.0, 2, 60.0, 350.0), Some(Hit::Panel), "the footer hint");
        assert_eq!(hit(s, 400.0, 1000.0, 2, 10.0, 300.0), None, "off the panel");
    }

    #[test]
    fn an_empty_panel_has_a_row_for_its_hint_and_still_closes() {
        assert_eq!(hit(1.0, 400.0, 1000.0, 0, 60.0, 270.0), Some(Hit::Panel));
        assert_eq!(hit(1.0, 400.0, 1000.0, 0, 300.0, 230.0), Some(Hit::Done));
        assert_eq!(panel(1.0, 400.0, 1000.0, 0).3, TITLE_PT + ROW_PT + FOOT_PT);
    }

    #[test]
    fn geometry_scales_with_one_point() {
        let (x1, _, w1, _) = panel(1.0, 800.0, 1000.0, 1);
        let (x2, _, w2, _) = panel(2.0, 800.0, 1000.0, 1);
        assert_eq!(w2, 2.0 * w1);
        assert_eq!(x1, (800.0 - PANEL_PT) / 2.0);
        assert_eq!(x2, (800.0 - 2.0 * PANEL_PT) / 2.0);
    }
}
