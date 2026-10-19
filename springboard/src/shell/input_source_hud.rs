//! The input source HUD: when the input source changes (Ctrl+Space, or a pick in the input menu), a small rounded panel
//! under the status bar names the source that is now active. It holds for 1.2 s after the switch, then fades out like
//! the volume HUD. The panel has a fixed width so its box is the same for every source: the damage box for a switch
//! covers the one a fade leaves behind.

use super::kb_layouts::Layout;
use super::ui::{Ui, Weight, WHITE};
use super::Shell;
use crate::gfx::{rgba, scale, Surface};
use crate::scene::STATUS_PT;

const W_PT: f32 = 210.0;
const H_PT: f32 = 34.0;
const RADIUS_PT: f32 = 6.0;
/// gap below the status bar
const GAP_PT: f32 = 8.0;
const LABEL_PT: f32 = 15.0;
/// fully visible for this long after a switch
pub const HOLD_MS: f64 = 1200.0;
/// then fades over this long
pub const FADE_MS: f64 = 250.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SourceHud {
    /// the source named, and when the switch happened
    shown: Option<(Layout, f64)>,
}

impl SourceHud {
    /// Shows (or restarts) the HUD for the source that is now active.
    pub fn show(&mut self, source: Layout, t: f64) {
        self.shown = Some((source, t));
    }

    /// The source the panel names while it shows.
    pub fn source(&self) -> Option<Layout> {
        self.shown.map(|(l, _)| l)
    }

    pub fn hide(&mut self) {
        self.shown = None;
    }

    /// 1 while holding, falling to 0 through the fade, 0 when hidden.
    pub fn alpha(&self, t: f64) -> f32 {
        match self.shown {
            Some((_, s)) => (1.0 - (t - s - HOLD_MS) / FADE_MS).clamp(0.0, 1.0) as f32,
            None => 0.0,
        }
    }

    pub fn visible(&self, t: f64) -> bool {
        self.alpha(t) > 0.0
    }

    /// When the fade starts, if it has not yet.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        self.shown.map(|(_, s)| s + HOLD_MS).filter(|&end| end > t)
    }

    /// Advances time: true while a frame is needed (fading, or the frame that removes it); hides itself once faded.
    pub fn tick(&mut self, t: f64) -> bool {
        let Some((_, s)) = self.shown else { return false };
        if !self.visible(t) {
            self.shown = None;
            return true;
        }
        t >= s + HOLD_MS
    }

    /// (x, y, w, h) in px for a screen `w` wide: centred under the status bar.
    pub fn rect(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
        let (pw, ph) = (ui.pt(W_PT), ui.pt(H_PT));
        ((w as f32 - pw) / 2.0, ui.pt(STATUS_PT + GAP_PT), pw, ph)
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, t: f64) {
        let alpha = (self.alpha(t) * 255.0) as u32;
        let Some(source) = self.source().filter(|_| alpha > 0) else { return };
        let a = |c: u32| scale(c, alpha);
        let (x, y, pw, ph) = Self::rect(ui, w);
        s.fill_rrect(x, y, pw, ph, ui.pt(RADIUS_PT), a(rgba(28, 28, 30, 235)));
        let label = ui.text(source.name(), LABEL_PT, Weight::Semibold, WHITE);
        s.blit(&label, (x + (pw - label.w as f32) / 2.0) as i32, (y + (ph - label.h as f32) / 2.0) as i32, 255, alpha);
    }
}

impl Shell {
    /// A keyboard layout was chosen: an input method that was the selected source stops, and the composition it held
    /// is kept in the Search field.
    pub(super) fn input_layout_selected(&mut self, layout: Layout) {
        let commits = self.input_providers.select_layout(layout);
        self.input_providers.apply_commits(&commits, &self.ui, self.quicksearch.field_mut());
    }

    /// A switch of the input source (Ctrl+Space or the input menu) names the new source in the HUD.
    pub(super) fn show_input_source(&mut self, source: Layout, t: f64) {
        self.input_hud.show(source, t);
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switch_names_the_new_source_and_holds_then_fades() {
        let mut h = SourceHud::default();
        assert!(!h.visible(0.0) && h.source().is_none());
        h.show(Layout::English, 1000.0);
        assert_eq!(h.source(), Some(Layout::English));
        assert_eq!(h.alpha(1000.0 + HOLD_MS - 1.0), 1.0, "holds for the hold time");
        assert!(h.alpha(1000.0 + HOLD_MS + FADE_MS / 2.0) < 1.0, "fades after it");
        assert!(!h.visible(1000.0 + HOLD_MS + FADE_MS + 1.0));
    }

    #[test]
    fn a_second_switch_restarts_the_hold_with_the_new_source() {
        let mut h = SourceHud::default();
        h.show(Layout::Icelandic, 0.0);
        h.show(Layout::Japanese, 1000.0);
        assert_eq!(h.source(), Some(Layout::Japanese));
        assert_eq!(h.next_wake(1100.0), Some(1000.0 + HOLD_MS));
        assert_eq!(h.alpha(1000.0 + HOLD_MS - 1.0), 1.0);
    }

    #[test]
    fn tick_asks_for_a_frame_while_fading_and_clears_once_faded() {
        let mut h = SourceHud::default();
        h.show(Layout::Icelandic, 0.0);
        assert!(!h.tick(HOLD_MS / 2.0), "holding needs no frame");
        assert!(h.tick(HOLD_MS + 1.0), "fading needs frames");
        assert!(h.tick(HOLD_MS + FADE_MS + 1.0), "the frame that removes it");
        assert!(h.source().is_none());
        assert!(!h.tick(HOLD_MS + FADE_MS + 2.0));
        assert_eq!(h.next_wake(0.0), None);
    }

    #[test]
    fn a_switch_on_the_shell_names_the_new_source() {
        let mut sh = crate::shell::tests::shell();
        sh.show_input_source(Layout::Japanese, 500.0);
        assert_eq!(sh.input_hud.source(), Some(Layout::Japanese));
        assert!(sh.input_hud.visible(500.0));
        assert!(sh.dirty);
    }

    #[test]
    fn hiding_removes_it_at_once() {
        let mut h = SourceHud::default();
        h.show(Layout::English, 0.0);
        h.hide();
        assert!(!h.visible(1.0));
    }
}
