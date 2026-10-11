//! Touch handling for the home screen: tap-to-launch, horizontal paging with
//! rubber-banding at the ends and a velocity-aware snap animation.

const SLOP: f32 = 12.0;
const FLING: f32 = 0.4; // px per ms
const RUBBER: f32 = 0.35;
const SNAP_MS: f64 = 320.0;

#[derive(Clone, Copy, Debug)]
struct Touch {
    x0: f32,
    y0: f32,
    offset0: f32,
    dragging: bool,
    pressed: Option<usize>,
    last_x: f32,
    last_t: f64,
    vx: f32,
}

#[derive(Clone, Copy, Debug)]
struct Anim {
    from: f32,
    to: f32,
    start: f64,
}

#[derive(Debug)]
pub struct Pager {
    pub pages: usize,
    pub width: f32,
    offset: f32,
    touch: Option<Touch>,
    anim: Option<Anim>,
    /// a page "-1" (the Today View) sits left of the first page
    leading: bool,
}

impl Pager {
    pub fn new(pages: usize, width: f32) -> Self {
        Pager { pages: pages.max(1), width, offset: 0.0, touch: None, anim: None, leading: false }
    }

    /// Allows (or forbids) the leading page at offset `-width`; forbidding
    /// it while shown snaps back to the first page.
    pub fn set_leading(&mut self, on: bool) {
        self.leading = on;
        if !on && self.offset < 0.0 {
            self.offset = 0.0;
            self.anim = None;
        }
    }

    pub fn has_leading(&self) -> bool {
        self.leading
    }

    /// True while the leading page is the nearest one.
    pub fn leading(&self) -> bool {
        self.leading && (self.offset / self.width).round() < 0.0
    }

    fn min_offset(&self) -> f32 {
        if self.leading {
            -self.width
        } else {
            0.0
        }
    }

    fn min_page(&self) -> i32 {
        -(self.leading as i32)
    }

    /// Horizontal scroll position in px; page `i` sits at `i * width`.
    pub fn offset(&self) -> f32 {
        self.offset
    }

    pub fn page(&self) -> usize {
        ((self.offset / self.width).round().max(0.0) as usize).min(self.pages - 1)
    }

    /// The slot currently held down, for the pressed highlight.
    pub fn pressed(&self) -> Option<usize> {
        self.touch.and_then(|t| t.pressed)
    }

    pub fn animating(&self) -> bool {
        self.anim.is_some()
    }

    pub fn set_pages(&mut self, pages: usize) {
        self.pages = pages.max(1);
        let max = self.max_offset();
        if self.offset > max {
            self.offset = max;
        }
    }

    fn max_offset(&self) -> f32 {
        (self.pages - 1) as f32 * self.width
    }

    pub fn down(&mut self, x: f32, y: f32, t: f64, hit: Option<usize>) {
        self.anim = None;
        self.touch = Some(Touch { x0: x, y0: y, offset0: self.offset, dragging: false, pressed: hit, last_x: x, last_t: t, vx: 0.0 });
    }

    pub fn moved(&mut self, x: f32, y: f32, t: f64) {
        let (min, max) = (self.min_offset(), self.max_offset());
        let Some(tc) = self.touch.as_mut() else { return };
        if !tc.dragging && ((x - tc.x0).abs() > SLOP || (y - tc.y0).abs() > SLOP) {
            tc.dragging = true;
            tc.pressed = None;
        }
        if tc.dragging {
            let dt = (t - tc.last_t) as f32;
            if dt > 0.0 {
                tc.vx = 0.6 * ((x - tc.last_x) / dt) + 0.4 * tc.vx;
            }
            tc.last_x = x;
            tc.last_t = t;
            let raw = tc.offset0 - (x - tc.x0);
            self.offset = if raw < min {
                min + (raw - min) * RUBBER
            } else if raw > max {
                max + (raw - max) * RUBBER
            } else {
                raw
            };
        }
    }

    /// Ends the touch; returns the slot to launch for a tap.
    pub fn up(&mut self, x: f32, y: f32, t: f64, hit: Option<usize>) -> Option<usize> {
        self.moved(x, y, t);
        let tc = self.touch.take()?;
        if !tc.dragging {
            return tc.pressed.filter(|&p| hit == Some(p));
        }
        let start_page = (tc.offset0 / self.width).round() as i32;
        let mut target = (self.offset / self.width).round() as i32;
        if tc.vx < -FLING {
            target = start_page + 1;
        } else if tc.vx > FLING {
            target = start_page - 1;
        }
        let target = target.clamp(self.min_page(), self.pages as i32 - 1) as f32 * self.width;
        self.anim = Some(Anim { from: self.offset, to: target, start: t });
        None
    }

    pub fn cancel(&mut self, t: f64) {
        if self.touch.take().is_some() {
            let page = (self.offset / self.width).round().clamp(self.min_page() as f32, (self.pages - 1) as f32);
            self.anim = Some(Anim { from: self.offset, to: page * self.width, start: t });
        }
    }

    /// Shows `page` at once, without animation.
    pub fn jump_to(&mut self, page: usize) {
        self.touch = None;
        self.anim = None;
        self.offset = page.min(self.pages - 1) as f32 * self.width;
    }

    /// Adopts a new page width (the device turned), staying on the same page.
    pub fn resize(&mut self, width: f32) {
        let page = self.page();
        let leading = self.leading();
        self.width = width;
        self.offset = if leading { -width } else { page as f32 * width };
        self.touch = None;
        self.anim = None;
    }

    /// Animates to `page`.
    pub fn go(&mut self, page: usize, t: f64) {
        self.touch = None;
        self.anim = Some(Anim { from: self.offset, to: page.min(self.pages - 1) as f32 * self.width, start: t });
    }

    /// Jumps (animated) to page 0, used when Home is pressed on the home screen.
    pub fn home(&mut self, t: f64) {
        self.touch = None;
        self.anim = Some(Anim { from: self.offset, to: 0.0, start: t });
    }

    /// Advances the animation; returns true while it is still running.
    pub fn tick(&mut self, t: f64) -> bool {
        let Some(a) = self.anim else { return false };
        let p = ((t - a.start) / SNAP_MS).clamp(0.0, 1.0) as f32;
        let ease = 1.0 - (1.0 - p).powi(3);
        self.offset = a.from + (a.to - a.from) * ease;
        if p >= 1.0 {
            self.offset = a.to;
            self.anim = None;
        }
        self.anim.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(p: &mut Pager, t: f64) {
        p.tick(t + 1000.0);
    }

    #[test]
    fn tap_launches_the_pressed_slot() {
        let mut p = Pager::new(2, 1000.0);
        p.down(100.0, 100.0, 0.0, Some(3));
        assert_eq!(p.pressed(), Some(3));
        assert_eq!(p.up(103.0, 102.0, 80.0, Some(3)), Some(3));
        assert_eq!(p.pressed(), None);
    }

    #[test]
    fn tap_released_elsewhere_does_nothing() {
        let mut p = Pager::new(2, 1000.0);
        p.down(100.0, 100.0, 0.0, Some(3));
        assert_eq!(p.up(105.0, 105.0, 80.0, None), None);
    }

    #[test]
    fn drag_cancels_press_and_scrolls() {
        let mut p = Pager::new(2, 1000.0);
        p.down(800.0, 100.0, 0.0, Some(1));
        p.moved(700.0, 100.0, 100.0);
        assert_eq!(p.pressed(), None);
        assert_eq!(p.offset(), 100.0);
        assert_eq!(p.up(700.0, 100.0, 400.0, Some(1)), None);
    }

    #[test]
    fn slow_drag_past_half_snaps_forward() {
        let mut p = Pager::new(3, 1000.0);
        p.down(900.0, 0.0, 0.0, None);
        for i in 1..=60 {
            p.moved(900.0 - i as f32 * 10.0, 0.0, i as f64 * 50.0);
        }
        p.up(300.0, 0.0, 3100.0, None);
        settle(&mut p, 3100.0);
        assert_eq!(p.offset(), 1000.0);
        assert_eq!(p.page(), 1);
    }

    #[test]
    fn short_fling_turns_the_page() {
        let mut p = Pager::new(3, 1000.0);
        p.down(600.0, 0.0, 0.0, None);
        p.moved(560.0, 0.0, 16.0);
        p.moved(500.0, 0.0, 32.0);
        p.up(450.0, 0.0, 48.0, None);
        assert!(p.animating());
        settle(&mut p, 48.0);
        assert_eq!(p.page(), 1);
    }

    #[test]
    fn edges_rubber_band_and_clamp() {
        let mut p = Pager::new(1, 1000.0);
        p.down(100.0, 0.0, 0.0, None);
        p.moved(300.0, 0.0, 100.0);
        assert!(p.offset() < 0.0 && p.offset() > -200.0);
        p.up(300.0, 0.0, 200.0, None);
        settle(&mut p, 200.0);
        assert_eq!(p.offset(), 0.0);
    }

    #[test]
    fn animation_eases_to_target() {
        let mut p = Pager::new(2, 1000.0);
        p.down(500.0, 0.0, 0.0, None);
        p.moved(-100.0, 0.0, 1000.0);
        p.up(-100.0, 0.0, 2000.0, None);
        assert!(p.tick(2100.0));
        let mid = p.offset();
        assert!(mid > 600.0 && mid < 1000.0);
        assert!(!p.tick(2400.0));
        assert_eq!(p.offset(), 1000.0);
    }

    #[test]
    fn shrinking_pages_clamps_offset() {
        let mut p = Pager::new(3, 1000.0);
        p.down(900.0, 0.0, 0.0, None);
        p.moved(100.0, 0.0, 1000.0);
        p.moved(-700.0, 0.0, 2000.0);
        p.up(-700.0, 0.0, 3000.0, None);
        settle(&mut p, 3000.0);
        assert_eq!(p.page(), 2);
        p.set_pages(1);
        assert_eq!(p.offset(), 0.0);
    }

    #[test]
    fn leading_page_opens_with_a_swipe_right_only_when_allowed() {
        let mut p = Pager::new(2, 1000.0);
        p.down(100.0, 0.0, 0.0, None);
        p.moved(700.0, 0.0, 1000.0);
        assert!(p.offset() > -300.0, "without a leading page the edge rubber-bands");
        p.up(700.0, 0.0, 2000.0, None);
        settle(&mut p, 2000.0);
        assert_eq!(p.offset(), 0.0);
        assert!(!p.leading());

        p.set_leading(true);
        assert!(p.has_leading());
        p.down(100.0, 0.0, 3000.0, None);
        p.moved(700.0, 0.0, 4000.0);
        assert_eq!(p.offset(), -600.0, "the leading page follows the finger");
        assert!(p.leading());
        p.up(700.0, 0.0, 5000.0, None);
        settle(&mut p, 5000.0);
        assert_eq!(p.offset(), -1000.0);
        assert!(p.leading());
        assert_eq!(p.page(), 0, "page() never goes below the first page");
        // past the leading page the rubber band resists again
        p.down(100.0, 0.0, 6000.0, None);
        p.moved(500.0, 0.0, 7000.0);
        assert!(p.offset() < -1000.0 && p.offset() > -1200.0);
        p.cancel(7000.0);
        settle(&mut p, 7000.0);
        assert_eq!(p.offset(), -1000.0);
    }

    #[test]
    fn fling_left_from_the_leading_page_returns_and_home_resets() {
        let mut p = Pager::new(1, 1000.0);
        p.set_leading(true);
        p.down(100.0, 0.0, 0.0, None);
        p.moved(900.0, 0.0, 1000.0);
        p.up(900.0, 0.0, 2000.0, None);
        settle(&mut p, 2000.0);
        assert!(p.leading());
        p.down(600.0, 0.0, 3000.0, None);
        p.moved(560.0, 0.0, 3016.0);
        p.moved(500.0, 0.0, 3032.0);
        p.up(450.0, 0.0, 3048.0, None);
        settle(&mut p, 3048.0);
        assert_eq!(p.offset(), 0.0);
        assert!(!p.leading());
        p.down(100.0, 0.0, 4000.0, None);
        p.moved(900.0, 0.0, 5000.0);
        p.up(900.0, 0.0, 6000.0, None);
        settle(&mut p, 6000.0);
        assert!(p.leading());
        p.home(7000.0);
        settle(&mut p, 7000.0);
        assert_eq!(p.offset(), 0.0);
        p.down(100.0, 0.0, 8000.0, None);
        p.moved(900.0, 0.0, 9000.0);
        p.up(900.0, 0.0, 9100.0, None);
        settle(&mut p, 9100.0);
        p.set_leading(false);
        assert_eq!(p.offset(), 0.0, "forbidding the page while on it snaps back");
    }

    #[test]
    fn resize_keeps_the_page_and_drops_motion() {
        let mut p = Pager::new(3, 1000.0);
        p.down(900.0, 0.0, 0.0, None);
        p.moved(100.0, 0.0, 1000.0);
        p.up(100.0, 0.0, 2000.0, None);
        settle(&mut p, 2000.0);
        assert_eq!(p.page(), 1);
        p.down(500.0, 0.0, 3000.0, Some(1));
        p.resize(1600.0);
        assert_eq!((p.page(), p.offset()), (1, 1600.0));
        assert!(!p.animating() && p.pressed().is_none());
    }

    #[test]
    fn home_returns_to_first_page() {
        let mut p = Pager::new(2, 1000.0);
        p.down(900.0, 0.0, 0.0, None);
        p.moved(100.0, 0.0, 1000.0);
        p.up(100.0, 0.0, 2000.0, None);
        settle(&mut p, 2000.0);
        p.home(5000.0);
        settle(&mut p, 5000.0);
        assert_eq!(p.offset(), 0.0);
    }
}
