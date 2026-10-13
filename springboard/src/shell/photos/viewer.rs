//! The full-screen viewer's touch state: horizontal paging between photos
//! with the home screen's paging physics, double-tap zoom to 2x with drag
//! to pan, and a single tap (held back for the double-tap window) that
//! toggles the chrome.

use super::zoom;
use crate::pager::Pager;

/// Two taps closer than this in time and space zoom.
pub const DOUBLE_TAP_MS: f64 = 300.0;
const TAP_SLOP: f32 = 24.0;
/// The magnification a double tap toggles.
pub const ZOOM: f32 = zoom::DOUBLE_TAP_ZOOM;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    None,
    /// a single tap settled: show or hide the bars
    ToggleChrome,
    /// a double tap changed the zoom
    Zoomed,
}

#[derive(Clone, Copy, Debug)]
struct Touch {
    x0: f32,
    y0: f32,
    last_x: f32,
    last_y: f32,
    moved: bool,
}

#[derive(Debug)]
pub struct Viewer {
    pub index: usize,
    pub count: usize,
    w: f32,
    h: f32,
    pager: Pager,
    zoom: f32,
    pan: (f32, f32),
    /// the pan a drag asked for before clamping
    raw_pan: (f32, f32),
    /// the rubber-band stretch past the pan bounds, drawn but not kept
    over: (f32, f32),
    /// the current photo's size at zoom 1, for the pan bounds
    fit: (f32, f32),
    touch: Option<Touch>,
    last_tap: Option<(f32, f32, f64)>,
    /// a single tap waiting out the double-tap window
    pending_tap: Option<f64>,
    /// the clock of the last `tick`, so the snap-back runs on elapsed time
    last_tick: Option<f64>,
    pub chrome: bool,
}

impl Viewer {
    /// Opens on photo `index` of `count` in a `w` x `h` px viewport.
    pub fn new(index: usize, count: usize, w: f32, h: f32) -> Self {
        let mut pager = Pager::new(count.max(1), w);
        pager.jump_to(index);
        Viewer { index: index.min(count.saturating_sub(1)), count, w, h, pager, zoom: 1.0, pan: (0.0, 0.0), raw_pan: (0.0, 0.0), over: (0.0, 0.0), fit: (w, h), touch: None, last_tap: None, pending_tap: None, last_tick: None, chrome: true }
    }

    /// Tells the viewer how big the current photo draws at zoom 1.
    pub fn set_fit(&mut self, w: f32, h: f32) {
        self.fit = (w, h);
        self.clamp_pan();
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn pan(&self) -> (f32, f32) {
        self.pan
    }

    /// Horizontal scroll position: page `i` sits at `i * w`.
    pub fn offset(&self) -> f32 {
        self.pager.offset()
    }

    pub fn animating(&self) -> bool {
        self.pager.animating() || self.over != (0.0, 0.0)
    }

    /// The rubber-band stretch currently drawn past the pan bounds.
    pub fn overscroll(&self) -> (f32, f32) {
        self.over
    }

    /// Where photo `page` draws: the current one at its zoom and pan, the
    /// neighbours fitted and centred on their own page.
    pub fn frame(&self, page: usize, fit_w: f32, fit_h: f32) -> (f32, f32, f32, f32) {
        let page_x = page as f32 * self.w - self.offset();
        if page == self.index {
            let (w, h) = (fit_w * self.zoom, fit_h * self.zoom);
            (page_x + (self.w - w) / 2.0 + self.pan.0 + self.over.0, (self.h - h) / 2.0 + self.pan.1 + self.over.1, w, h)
        } else {
            (page_x + (self.w - fit_w) / 2.0, (self.h - fit_h) / 2.0, fit_w, fit_h)
        }
    }

    fn clamp_pan(&mut self) {
        self.pan = zoom::clamp_pan(self.pan, self.fit, self.zoom, (self.w, self.h));
        self.raw_pan = self.pan;
    }

    /// A pinch step: scales the zoom by `scale` about screen point `focal`.
    /// The shell's touch events carry one finger, so a front end with two
    /// (a gesture recogniser) calls this; `end_pinch` settles the result.
    pub fn pinch(&mut self, scale: f32, focal: (f32, f32)) {
        let (z, px, py) = zoom::pinch(self.zoom, self.pan, scale, focal, (self.w, self.h));
        self.zoom = z;
        // while pinching the pan is free; the settle clamps it
        self.pan = (px, py);
        self.raw_pan = self.pan;
    }

    /// Fingers lifted: the zoom comes to rest inside its limits and the
    /// pan inside the picture.
    pub fn end_pinch(&mut self) {
        self.zoom = zoom::settle_zoom(self.zoom);
        if self.zoom <= zoom::MIN_ZOOM {
            self.pan = (0.0, 0.0);
        }
        self.clamp_pan();
    }

    pub fn down(&mut self, x: f32, y: f32, t: f64) {
        self.touch = Some(Touch { x0: x, y0: y, last_x: x, last_y: y, moved: false });
        self.raw_pan = self.pan;
        if self.zoom == 1.0 {
            self.pager.down(x, y, t, None);
        }
    }

    pub fn moved(&mut self, x: f32, y: f32, t: f64) {
        let Some(tc) = self.touch.as_mut() else { return };
        if (x - tc.x0).abs() > TAP_SLOP || (y - tc.y0).abs() > TAP_SLOP {
            tc.moved = true;
        }
        if self.zoom > 1.0 {
            let (dx, dy) = (x - tc.last_x, y - tc.last_y);
            tc.last_x = x;
            tc.last_y = y;
            if tc.moved {
                self.raw_pan = (self.raw_pan.0 + dx, self.raw_pan.1 + dy);
                let kept = zoom::clamp_pan(self.raw_pan, self.fit, self.zoom, (self.w, self.h));
                self.pan = kept;
                // dragging past the edge stretches like a rubber band
                self.over = (zoom::rubber_band(self.raw_pan.0 - kept.0, self.w * zoom::BAND_FRACTION), zoom::rubber_band(self.raw_pan.1 - kept.1, self.h * zoom::BAND_FRACTION));
            }
        } else {
            self.pager.moved(x, y, t);
        }
    }

    pub fn up(&mut self, x: f32, y: f32, t: f64) -> Action {
        let Some(tc) = self.touch.take() else { return Action::None };
        self.raw_pan = self.pan;
        if self.zoom == 1.0 {
            self.pager.up(x, y, t, None);
        }
        if tc.moved {
            self.last_tap = None;
            return Action::None;
        }
        if let Some((lx, ly, lt)) = self.last_tap.take() {
            if t - lt <= DOUBLE_TAP_MS && (x - lx).abs() <= TAP_SLOP && (y - ly).abs() <= TAP_SLOP {
                self.pending_tap = None;
                self.toggle_zoom(x, y);
                return Action::Zoomed;
            }
        }
        self.last_tap = Some((x, y, t));
        self.pending_tap = Some(t);
        Action::None
    }

    /// Zooms to `ZOOM` keeping the tapped point still, or back to fit.
    fn toggle_zoom(&mut self, x: f32, y: f32) {
        let target = zoom::double_tap_target(self.zoom);
        self.pan = zoom::zoom_to(target, self.zoom, self.pan, (x, y), (self.w, self.h), self.fit);
        self.zoom = target;
        self.raw_pan = self.pan;
        self.over = (0.0, 0.0);
    }

    /// Advances paging and the tap timer; the action a settled tap resolves to.
    pub fn tick(&mut self, t: f64) -> Action {
        self.pager.tick(t);
        let dt = self.last_tick.map_or(0.0, |last| ((t - last) / 1000.0) as f32);
        self.last_tick = Some(t);
        if self.touch.is_none() && self.over != (0.0, 0.0) {
            self.over = (zoom::decay(self.over.0, dt), zoom::decay(self.over.1, dt));
        }
        let page = self.pager.page();
        if page != self.index && self.touch.is_none() {
            self.index = page;
            self.zoom = 1.0;
            self.pan = (0.0, 0.0);
            self.raw_pan = self.pan;
            self.over = (0.0, 0.0);
        }
        if self.pending_tap.is_some_and(|p| t - p >= DOUBLE_TAP_MS) {
            self.pending_tap = None;
            self.chrome = !self.chrome;
            return Action::ToggleChrome;
        }
        Action::None
    }

    /// When `tick` next has something to do without input.
    pub fn next_wake(&self) -> Option<f64> {
        self.pending_tap.map(|p| p + DOUBLE_TAP_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap(v: &mut Viewer, x: f32, y: f32, t: f64) -> Action {
        v.down(x, y, t);
        v.up(x, y, t + 30.0)
    }

    #[test]
    fn opens_on_the_requested_photo() {
        let v = Viewer::new(3, 10, 1200.0, 1920.0);
        assert_eq!(v.index, 3);
        assert_eq!(v.offset(), 3600.0);
        assert_eq!(v.frame(3, 1200.0, 900.0), (0.0, 510.0, 1200.0, 900.0));
        assert_eq!(v.frame(4, 600.0, 900.0).0, 1200.0 + 300.0, "the next photo waits one page to the right");
        assert_eq!(Viewer::new(9, 3, 1200.0, 1920.0).index, 2, "index is clamped");
    }

    #[test]
    fn single_tap_toggles_chrome_after_the_double_tap_window() {
        let mut v = Viewer::new(0, 2, 1200.0, 1920.0);
        assert_eq!(tap(&mut v, 600.0, 900.0, 1000.0), Action::None);
        assert_eq!(v.next_wake(), Some(1030.0 + DOUBLE_TAP_MS));
        assert_eq!(v.tick(1200.0), Action::None, "still waiting");
        assert_eq!(v.tick(1340.0), Action::ToggleChrome);
        assert!(!v.chrome);
        assert_eq!(v.tick(1400.0), Action::None);
        assert_eq!(v.next_wake(), None);
    }

    #[test]
    fn double_tap_zooms_around_the_finger_and_drags_pan() {
        let mut v = Viewer::new(0, 1, 1200.0, 1920.0);
        v.set_fit(1200.0, 900.0);
        tap(&mut v, 300.0, 960.0, 0.0);
        assert_eq!(tap(&mut v, 300.0, 960.0, 100.0), Action::Zoomed);
        assert_eq!(v.zoom(), ZOOM);
        assert_eq!(v.next_wake(), None, "the single tap was cancelled");
        assert_eq!(v.tick(1000.0), Action::None);
        assert!(v.chrome, "chrome untouched");
        // the tapped point (300 px left of centre) stays put: pan = +300
        assert_eq!(v.pan(), (300.0, 0.0));
        let (x, _, w, h) = v.frame(0, 1200.0, 900.0);
        assert_eq!((w, h), (2400.0, 1800.0));
        assert_eq!(x, -600.0 + 300.0);
        // drag pans, clamped to the picture's edge
        v.down(600.0, 900.0, 2000.0);
        v.moved(640.0, 900.0, 2010.0);
        v.moved(2000.0, 900.0, 2020.0);
        assert_eq!(v.up(2000.0, 900.0, 2030.0), Action::None);
        assert_eq!(v.pan(), (600.0, 0.0));
        assert_eq!(v.offset(), 0.0, "panning never pages");
        // double tap again zooms back out
        tap(&mut v, 100.0, 100.0, 3000.0);
        assert_eq!(tap(&mut v, 100.0, 100.0, 3100.0), Action::Zoomed);
        assert_eq!((v.zoom(), v.pan()), (1.0, (0.0, 0.0)));
    }

    #[test]
    fn swiping_pages_to_the_next_photo_and_resets_zoom() {
        let mut v = Viewer::new(0, 3, 1200.0, 1920.0);
        v.down(900.0, 900.0, 0.0);
        v.moved(860.0, 900.0, 16.0);
        v.moved(780.0, 900.0, 32.0);
        v.moved(700.0, 900.0, 48.0);
        v.up(650.0, 900.0, 56.0);
        assert!(v.animating());
        assert_eq!(v.tick(60.0), Action::None);
        assert!(v.offset() > 0.0 && v.index == 0, "mid-flight the index holds");
        v.tick(2000.0);
        assert_eq!((v.index, v.offset()), (1, 1200.0));
        assert_eq!(v.next_wake(), None, "a drag is not a tap");
        tap(&mut v, 600.0, 900.0, 3000.0);
        tap(&mut v, 600.0, 900.0, 3100.0);
        assert_eq!(v.zoom(), ZOOM);
        v.index = 2;
        v.zoom = 1.0;
        assert_eq!(v.frame(2, 1200.0, 900.0).0, 1200.0, "frames follow the pager offset");
    }

    #[test]
    fn dragging_past_the_edge_stretches_then_springs_back() {
        let mut v = Viewer::new(0, 1, 1200.0, 1920.0);
        v.set_fit(1200.0, 900.0);
        tap(&mut v, 600.0, 960.0, 0.0);
        tap(&mut v, 600.0, 960.0, 100.0);
        assert_eq!(v.zoom(), ZOOM);
        v.down(600.0, 900.0, 2000.0);
        v.moved(900.0, 900.0, 2010.0);
        v.moved(1100.0, 900.0, 2020.0);
        assert_eq!(v.pan(), (500.0, 0.0), "the kept pan stays inside");
        v.moved(1400.0, 900.0, 2030.0);
        assert_eq!(v.pan(), (600.0, 0.0));
        let (ox, oy) = v.overscroll();
        assert!(ox > 0.0 && ox < 300.0 && oy == 0.0, "stretch {ox}");
        let (x, ..) = v.frame(0, 1200.0, 900.0);
        assert_eq!(x, -600.0 + 600.0 + ox);
        v.up(1400.0, 900.0, 2040.0);
        assert!(v.animating(), "the band is still stretched");
        let mut t = 2100.0;
        while v.animating() {
            v.tick(t);
            t += 16.0;
            assert!(t < 5000.0, "the band settles");
        }
        assert_eq!(v.overscroll(), (0.0, 0.0));
        assert_eq!(v.pan(), (600.0, 0.0));
    }

    #[test]
    fn pinch_zooms_about_the_fingers_and_settles_within_limits() {
        let mut v = Viewer::new(0, 1, 1200.0, 1920.0);
        v.set_fit(1200.0, 900.0);
        v.pinch(2.0, (300.0, 960.0));
        assert_eq!(v.zoom(), 2.0);
        assert_eq!(v.pan(), (300.0, 0.0));
        v.pinch(4.0, (600.0, 960.0));
        assert!(v.zoom() > zoom::MAX_ZOOM, "stretches past the limit while pinching");
        v.end_pinch();
        assert_eq!(v.zoom(), zoom::MAX_ZOOM);
        assert!(v.pan().0.abs() <= 1200.0 * zoom::MAX_ZOOM / 2.0 - 600.0);
        v.pinch(0.1, (600.0, 960.0));
        assert!(v.zoom() < 1.0);
        v.end_pinch();
        assert_eq!((v.zoom(), v.pan()), (1.0, (0.0, 0.0)));
    }
}
