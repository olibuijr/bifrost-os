//! Crash banner (Browser: "This webpage was reloaded because a problem occurred."). A crashed content process
//! reloads only its own tab and shows a banner over the top of the page with a close button. A tab that crashes
//! `MAX_CRASHES` times within `WINDOW_S` is not reloaded again: the banner says "A problem repeatedly occurred" and
//! offers a Reload button. The banner floats over the page (the frame is not resized), so it follows the tab.
use std::collections::HashMap;

use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::Rect;
use bifrost_app::glyphs;
use bifrost_app::kit::ui::{Weight, CARD_BG, LABEL, LINK, MAC_BODY_PT, SEPARATOR_C};
use bifrost_app::kit::{Surface, Ui};
use bifrost_app::webview::WebViewId;

use crate::Browser;

const MAX_CRASHES: usize = 2;
const WINDOW_S: f64 = 60.0;
const BANNER_H: f32 = 36.0;
const MARGIN: f32 = 8.0;
const BTN_W: f32 = 28.0;
const RELOAD_W: f32 = 64.0;

#[derive(Default)]
pub struct CrashBanner {
    times: HashMap<WebViewId, Vec<f64>>,
    /// Tabs showing the banner; the value is true for the "repeatedly" form.
    shown: HashMap<WebViewId, bool>,
}

impl CrashBanner {
    /// Records a crash at `now` (seconds); true when the tab may be reloaded automatically.
    pub fn on_crash(&mut self, v: WebViewId, now: f64) -> bool {
        let t = self.times.entry(v).or_default();
        t.retain(|&x| now - x < WINDOW_S);
        t.push(now);
        let repeated = t.len() >= MAX_CRASHES;
        self.shown.insert(v, repeated);
        !repeated
    }

    pub fn forget(&mut self, v: WebViewId) {
        self.times.remove(&v);
        self.shown.remove(&v);
    }

    pub fn dismiss(&mut self, v: WebViewId) {
        self.shown.remove(&v);
    }

    pub fn state(&self, v: WebViewId) -> Option<bool> {
        self.shown.get(&v).copied()
    }
}

/// Banner, close button and (repeated form) Reload button rectangles in points.
fn rects(wpt: f32, top: f32, repeated: bool) -> (Rect, Rect, Option<Rect>) {
    let w = (wpt - 2.0 * MARGIN).clamp(0.0, 520.0);
    let b = Rect { x: (wpt - w) / 2.0, y: top + MARGIN, w, h: BANNER_H };
    let close = Rect { x: b.x + b.w - BTN_W, y: b.y, w: BTN_W, h: BANNER_H };
    let reload = repeated.then(|| Rect { x: close.x - RELOAD_W, y: b.y, w: RELOAD_W, h: BANNER_H });
    (b, close, reload)
}

pub enum BannerHit {
    Close,
    Reload,
}

impl Browser {
    /// Which banner control, if any, is under the point (points) for the selected tab.
    pub(crate) fn banner_hit(&self, wpt: f32, px: f32, py: f32) -> Option<BannerHit> {
        let repeated = self.crash.state(self.view()?)?;
        let (_, close, reload) = rects(wpt, self.content_top(), repeated);
        if close.contains(px, py) {
            Some(BannerHit::Close)
        } else if reload.is_some_and(|r| r.contains(px, py)) {
            Some(BannerHit::Reload)
        } else {
            None
        }
    }

    pub(crate) fn banner_activate(&mut self, hit: BannerHit) {
        let Some(v) = self.view() else { return };
        self.crash.dismiss(v);
        if matches!(hit, BannerHit::Reload) {
            if let Some(p) = &mut self.pool {
                p.reload(v);
            }
        }
    }

    pub(crate) fn draw_banner(&self, s: &mut Surface, ui: &Ui, wpt: f32) {
        let Some(repeated) = self.view().and_then(|v| self.crash.state(v)) else { return };
        let (b, close, reload) = rects(wpt, self.content_top(), repeated);
        let (x, y, w, h) = (ui.pt(b.x), ui.pt(b.y), ui.pt(b.w), ui.pt(b.h));
        s.fill_rrect(x, y, w, h, ui.pt(m::SELECTION_RADIUS), CARD_BG.now());
        s.stroke_rrect(x, y, w, h, ui.pt(m::SELECTION_RADIUS), 1.0, SEPARATOR_C.now());
        let msg = if repeated { "A problem repeatedly occurred" } else { "This webpage was reloaded because a problem occurred." };
        let room = b.w - BTN_W - 12.0 - reload.map_or(0.0, |r| r.w);
        let img = ui.text(&ui.fit(msg, MAC_BODY_PT, ui.pt(room)), MAC_BODY_PT, Weight::Regular, LABEL.now());
        s.blit(&img, ui.pt(b.x + 12.0) as i32, (y + (h - img.h as f32) / 2.0) as i32, 255, 255);
        if let Some(r) = reload {
            let t = ui.text("Reload", MAC_BODY_PT, Weight::Regular, LINK);
            s.blit(&t, (ui.pt(r.x + r.w / 2.0) - t.w as f32 / 2.0) as i32, (y + (h - t.h as f32) / 2.0) as i32, 255, 255);
        }
        let g = Rect { x: close.x + 6.0, y: close.y + 10.0, w: 16.0, h: 16.0 };
        glyphs::glyph(s, ui, "xmark", g, LINK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_crash_within_a_minute_is_repeated() {
        let mut c = CrashBanner::default();
        assert!(c.on_crash(1, 0.0));
        assert_eq!(c.state(1), Some(false));
        assert!(!c.on_crash(1, 30.0));
        assert_eq!(c.state(1), Some(true));
    }

    #[test]
    fn old_crashes_expire_and_tabs_are_independent() {
        let mut c = CrashBanner::default();
        assert!(c.on_crash(1, 0.0));
        assert!(c.on_crash(1, 61.0));
        assert!(c.on_crash(2, 5.0));
        c.dismiss(1);
        assert_eq!(c.state(1), None);
    }
}
