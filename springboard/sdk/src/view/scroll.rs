//! `NativeScrollView`: one document view inside a clipping frame, panned by dragging, with a thin overlay scroller.
use super::{View, ViewBase, ViewEvent};
use crate::kit::ui::LABEL_2;
use crate::kit::{Event, Ui};
use bifrost_render::canvas::{Canvas, RectF};
use bifrost_ui::Rect;

/// Overlay scroller knob, in points (Golden Gate overlay style: thin, inset from the right edge).
const KNOB_W_PT: f32 = 3.0;
const KNOB_INSET_PT: f32 = 3.0;
const KNOB_MIN_PT: f32 = 24.0;
/// A touch must move this far before it pans the content (points).
const DRAG_SLOP_PT: f32 = 12.0;

/// A scroll view (`NativeScrollView` + `NativeClipView`) holding one document view. Vertical only: the document is as wide as
/// the scroll view and as tall as its `intrinsic` height (or the scroll view's, when it has none), so `Surface`'s row
/// clip is enough. Dragging pans the content one to one with the finger; a release runs the reference fling or spring
/// back (`ScrollPhysics`), driven by `View::tick`; the knob shows while the content moves. There is no wheel input yet
/// (the proto delivers only Down, Move, Up and Cancel).
pub struct ScrollView {
    base: ViewBase,
    kids: Vec<Box<dyn View>>,
    physics: ScrollPhysics,
    /// the motion a release started (a fling or a spring back) and the frame time (seconds) it started at
    motion: Option<(Motion, f64)>,
    /// where the touch went down; `None` when no touch is down
    down_y: Option<f32>,
    /// the finger's last applied position (the drag anchor until the content first moves)
    last_y: f32,
    /// the touch has passed the slop and pans the content
    dragging: bool,
    /// frame time of the last `tick` (seconds): `ViewEvent` carries no timestamp, so touches are timed with it
    now: f64,
    doc_h: f32,
    knob: bool,
}

impl ScrollView {
    pub fn new(document: Box<dyn View>) -> Self {
        ScrollView {
            base: ViewBase::default(),
            kids: vec![document],
            physics: ScrollPhysics::new(0.0, 0.0),
            motion: None,
            down_y: None,
            last_y: 0.0,
            dragging: false,
            now: 0.0,
            doc_h: 0.0,
            knob: false,
        }
    }
    /// `NativeClipView.bounds.origin.y`: how far the document is scrolled (may be negative or past the end mid rubber band).
    pub fn content_offset(&self) -> f32 {
        match self.motion {
            Some((motion, start)) => motion.at(self.now - start).0,
            None => self.physics.offset(),
        }
    }
    /// The largest offset: document height minus the visible height.
    pub fn max_offset(&self) -> f32 {
        self.physics.max_offset()
    }
    /// `NativeClipView.scroll(to:)`: jumps to `y`, clamped to `0..=max_offset()`, and stops any fling.
    pub fn scroll_to(&mut self, y: f32) {
        self.motion = None;
        self.physics.set_offset(y);
        self.base.dirty = true;
    }
    /// The touch is at `y`: past the slop it pans the content, one to one with the finger.
    fn finger(&mut self, y: f32) {
        let Some(y0) = self.down_y else { return };
        if !self.dragging {
            if (y - y0).abs() <= DRAG_SLOP_PT {
                return;
            }
            // the drag starts at the slop's edge, so the content does not jump by the slop
            self.dragging = true;
            self.last_y = y0 + DRAG_SLOP_PT.copysign(y - y0);
        }
        // a finger moving up (smaller y) scrolls towards the end
        self.physics.drag(self.now * 1000.0, self.last_y - y);
        self.last_y = y;
    }
    /// The touch lifted at `y`: a dragged touch starts its motion (a fling, or a spring back past an edge).
    fn lift(&mut self, y: f32) {
        self.finger(y);
        if self.dragging {
            self.motion = self.physics.release(self.now * 1000.0).map(|m| (m, self.now));
        }
        self.down_y = None;
        self.dragging = false;
    }
    /// `documentView`, downcast.
    pub fn document_view<T: View>(&self) -> Option<&T> {
        self.kids[0].as_any().downcast_ref::<T>()
    }
    pub fn document_view_mut<T: View>(&mut self) -> Option<&mut T> {
        self.kids[0].as_any_mut().downcast_mut::<T>()
    }
}

impl View for ScrollView {
    crate::view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.kids
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.kids
    }
    fn child_offset(&self) -> (f32, f32) {
        (0.0, self.content_offset())
    }
    fn clips_children(&self) -> bool {
        true
    }
    /// A dirty document row repaints the whole scroll view: partial repaints would need the offset and a clip.
    fn opaque_backdrop(&self) -> bool {
        true
    }
    fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.set_frame(bounds);
        let (_, ih) = self.kids[0].intrinsic(ui);
        let h = if ih > 0.0 { ih } else { bounds.h };
        self.kids[0].layout(ui, Rect { x: bounds.x, y: bounds.y, w: bounds.w, h });
        self.doc_h = self.kids[0].frame().h;
        self.physics.set_bounds(self.base.frame.h, self.doc_h);
        if !self.dragging && self.motion.is_none() {
            // an idle view keeps its offset inside the new range
            let o = self.physics.offset();
            self.physics.set_offset(o);
        }
        self.base.dirty = true;
    }
    fn handle(&mut self, ev: &ViewEvent) -> bool {
        match ev {
            ViewEvent::Pointer(Event::Down(_, y)) => {
                // a touch during a fling or a spring back catches the content where it is
                if let Some((motion, start)) = self.motion.take() {
                    let at = motion.at(self.now - start).0;
                    self.physics.set_offset(at);
                }
                self.down_y = Some(*y);
                self.last_y = *y;
                self.dragging = false;
            }
            ViewEvent::Pointer(Event::Move(_, y)) => self.finger(*y),
            ViewEvent::Pointer(Event::Up(_, y)) => self.lift(*y),
            ViewEvent::Cancel => {
                let y = self.last_y;
                self.lift(y);
            }
            _ => return false,
        }
        self.base.dirty = true;
        true
    }
    fn wants_tick(&self) -> bool {
        self.dragging || self.motion.is_some() || self.knob
    }
    fn tick(&mut self, t: f64) {
        self.now = t;
        if let Some((motion, start)) = self.motion {
            if motion.at(t - start).1 {
                // the content rests at the end the physics model already holds
                self.motion = None;
            }
        }
        let moving = self.dragging || self.motion.is_some();
        if moving || self.knob {
            self.base.dirty = true;
        }
        self.knob = moving;
    }
    fn draw_overlay(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let max = self.max_offset();
        if !self.knob || max <= 0.0 {
            return;
        }
        let f = self.base.frame;
        let knob_h = (f.h * f.h / self.doc_h).max(KNOB_MIN_PT).min(f.h);
        let t = (self.content_offset() / max).clamp(0.0, 1.0);
        let y = f.y + t * (f.h - knob_h);
        // opaque colour: overlays repaint every frame, so a translucent knob would build up
        s.fill_rrect(RectF::new(ui.pt(f.right() - KNOB_INSET_PT - KNOB_W_PT), ui.pt(y), ui.pt(KNOB_W_PT), ui.pt(knob_h)), ui.pt(KNOB_W_PT / 2.0), LABEL_2.now());
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Scroll physics: the reference platform's model behind `ScrollView`. The finger's position and the velocity of the last
// window come from `ScrollPhysics`; a release turns into a `Motion`, a fling that decelerates or a spring back from the
// rubber band, which `ScrollView` evaluates on each frame.
// ---------------------------------------------------------------------------------------------------------------------

/// The fling speed is the displacement over the last this many milliseconds.
pub const VELOCITY_WINDOW_MS: f64 = 100.0;
/// Rubber band: the share of the finger's travel the content follows at the edge.
pub const RUBBER_C: f32 = 0.55;
/// Speed factor per millisecond of a fling (the reference platform's normal deceleration rate).
pub const DECEL_PER_MS: f64 = 0.998;
/// Below this speed (points per second) a release stops without a fling.
pub const STOP_SPEED: f32 = 20.0;
/// Spring stiffness of a bounce back to an edge, per millisecond: critically damped, about 400 ms to settle.
pub const BOUNCE_W_PER_MS: f64 = 0.012;

/// Exponential decay rate per second: speed is `v0 * e^(-rate * t)`.
fn decay_rate() -> f64 {
    -DECEL_PER_MS.ln() * 1000.0
}

/// The rubber band's displayed overshoot for `x` points past an edge, in a view `dim` points long: it follows `x` at
/// first and levels off towards `dim`.
pub fn rubber(x: f32, dim: f32) -> f32 {
    if dim <= 0.0 || x <= 0.0 {
        return 0.0;
    }
    (1.0 - 1.0 / (x * RUBBER_C / dim + 1.0)) * dim
}

/// The offset shown for a finger position: inside `0..=max` it is the position, outside it is rubber-banded.
pub fn displayed_offset(finger: f32, max: f32, dim: f32) -> f32 {
    if finger < 0.0 {
        -rubber(-finger, dim)
    } else if finger > max {
        max + rubber(finger - max, dim)
    } else {
        finger
    }
}

/// Finger samples (time in ms, shown offset) within the last `VELOCITY_WINDOW_MS`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct VelocityTracker {
    samples: Vec<(f64, f32)>,
}

impl VelocityTracker {
    /// Records a sample and drops the ones older than the window before it.
    pub fn push(&mut self, t_ms: f64, offset: f32) {
        self.samples.push((t_ms, offset));
        let cutoff = t_ms - VELOCITY_WINDOW_MS;
        self.samples.retain(|(t, _)| *t >= cutoff);
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Points per second over the window ending at `now_ms`: the displacement from its first sample to its last.
    /// Zero with fewer than two samples in the window, or when the finger has been still for the whole window.
    pub fn velocity(&self, now_ms: f64) -> f32 {
        let window: Vec<&(f64, f32)> =
            self.samples.iter().filter(|(t, _)| *t >= now_ms - VELOCITY_WINDOW_MS && *t <= now_ms).collect();
        if window.len() < 2 {
            return 0.0;
        }
        let (t0, y0) = *window[0];
        let (t1, y1) = *window[window.len() - 1];
        let dt = t1 - t0;
        if dt <= 0.0 {
            return 0.0;
        }
        ((y1 - y0) as f64 / dt * 1000.0) as f32
    }
}

/// A fling: the offset starts at `from` with `velocity` (points per second) and decelerates; it is kept inside
/// `min..=max` (the edge it reaches stops it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fling {
    pub from: f32,
    pub velocity: f32,
    pub min: f32,
    pub max: f32,
}

impl Fling {
    /// Seconds until the speed falls below `STOP_SPEED`.
    pub fn duration(&self) -> f64 {
        (self.velocity.abs() as f64 / STOP_SPEED as f64).ln() / decay_rate()
    }

    /// The offset `t` seconds after release (`t` is clamped to 0..=duration, and the result to the range).
    pub fn offset_at(&self, t: f64) -> f32 {
        let k = decay_rate();
        let t = t.clamp(0.0, self.duration());
        let travelled = self.velocity as f64 / k * (1.0 - (-k * t).exp());
        ((self.from as f64 + travelled) as f32).clamp(self.min, self.max)
    }
}

/// What a release starts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Motion {
    /// The content glides on after a fling.
    Fling(Fling),
    /// The content springs from the rubber band back to the edge `to`; `velocity` is the release speed.
    Bounce { from: f32, to: f32, velocity: f32 },
}

impl Motion {
    /// The offset `dt` seconds after the release, and whether the motion has finished by then (the content then rests
    /// at the end the physics model already holds).
    pub fn at(&self, dt: f64) -> (f32, bool) {
        let dt = dt.max(0.0);
        match *self {
            Motion::Fling(fling) => (fling.offset_at(dt), dt >= fling.duration()),
            Motion::Bounce { from, to, velocity } => bounce_at(from, to, velocity, dt),
        }
    }
}

/// A critically damped spring from `from` to `to`, starting with `velocity` (points per second), `dt` seconds on.
/// Settled (within half a point and 0.01 points per millisecond) it returns the edge exactly.
fn bounce_at(from: f32, to: f32, velocity: f32, dt: f64) -> (f32, bool) {
    let w = BOUNCE_W_PER_MS;
    let ms = dt * 1000.0;
    let x0 = (from - to) as f64;
    let v0 = velocity as f64 / 1000.0;
    let e = (-w * ms).exp();
    let x = (x0 + (v0 + w * x0) * ms) * e;
    let v = (v0 - w * (v0 + w * x0) * ms) * e;
    if x.abs() < 0.5 && v.abs() < 0.01 {
        (to, true)
    } else {
        (to + x as f32, false)
    }
}

/// The scroll model of one view: the finger position, the rubber band and the velocity of the last window.
#[derive(Debug, Clone, PartialEq)]
pub struct ScrollPhysics {
    viewport: f32,
    content: f32,
    finger: f32,
    tracker: VelocityTracker,
}

impl ScrollPhysics {
    pub fn new(viewport_h: f32, content_h: f32) -> ScrollPhysics {
        ScrollPhysics { viewport: viewport_h, content: content_h, finger: 0.0, tracker: VelocityTracker::default() }
    }

    /// Sets the viewport and content heights after a layout. The finger keeps its place.
    pub fn set_bounds(&mut self, viewport_h: f32, content_h: f32) {
        self.viewport = viewport_h;
        self.content = content_h;
    }

    /// The largest offset inside the range: content height minus viewport height, never negative.
    pub fn max_offset(&self) -> f32 {
        (self.content - self.viewport).max(0.0)
    }

    /// The offset shown now (rubber-banded outside the range).
    pub fn offset(&self) -> f32 {
        displayed_offset(self.finger, self.max_offset(), self.viewport)
    }

    /// Jumps to `offset` (clamped to the range) and forgets the finger's history.
    pub fn set_offset(&mut self, offset: f32) {
        self.finger = offset.clamp(0.0, self.max_offset());
        self.tracker.clear();
    }

    /// The finger moved `delta` points at `t_ms` (positive scrolls towards the end).
    pub fn drag(&mut self, t_ms: f64, delta: f32) {
        self.finger += delta;
        let shown = self.offset();
        self.tracker.push(t_ms, shown);
    }

    /// The finger lifted at `now_ms`: a fling when the speed is above `STOP_SPEED`, a spring back when the content is
    /// past an edge, nothing otherwise. The model's finger follows the motion's end.
    pub fn release(&mut self, now_ms: f64) -> Option<Motion> {
        let velocity = self.tracker.velocity(now_ms);
        self.tracker.clear();
        let from = self.offset();
        let max = self.max_offset();
        if from < 0.0 || from > max {
            let to = from.clamp(0.0, max);
            self.finger = to;
            return Some(Motion::Bounce { from, to, velocity });
        }
        if velocity.abs() < STOP_SPEED {
            self.finger = from;
            return None;
        }
        let fling = Fling { from, velocity, min: 0.0, max };
        self.finger = fling.offset_at(fling.duration());
        Some(Motion::Fling(fling))
    }
}

#[cfg(test)]
mod physics_tests {
    use super::*;

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn velocity_is_the_displacement_over_the_last_100_ms() {
        // Samples at 0, 50, 100 and 150 ms; at 150 ms the window is 50..=150 ms, from 10 to 40 points: 300 pt/s.
        let mut tracker = VelocityTracker::default();
        tracker.push(0.0, 0.0);
        tracker.push(50.0, 10.0);
        tracker.push(100.0, 20.0);
        tracker.push(150.0, 40.0);
        assert!(close(tracker.velocity(150.0), 300.0, 1e-3));
    }

    #[test]
    fn velocity_is_zero_when_the_finger_has_been_still() {
        let mut tracker = VelocityTracker::default();
        tracker.push(0.0, 0.0);
        tracker.push(10.0, 50.0);
        // The last sample is 290 ms old at 300 ms, outside the window.
        assert_eq!(tracker.velocity(300.0), 0.0);
        let mut single = VelocityTracker::default();
        single.push(0.0, 5.0);
        assert_eq!(single.velocity(0.0), 0.0);
    }

    #[test]
    fn samples_older_than_the_window_are_dropped() {
        let mut tracker = VelocityTracker::default();
        tracker.push(0.0, 0.0);
        tracker.push(120.0, 100.0);
        // 0 ms is 120 ms before 120 ms, so it went; one sample remains and the velocity is zero.
        assert_eq!(tracker.velocity(120.0), 0.0);
    }

    #[test]
    fn rubber_band_follows_the_finger_at_first_and_levels_off() {
        assert_eq!(rubber(0.0, 200.0), 0.0);
        // x = 100, dim = 200: (1 - 1 / (100 * 0.55 / 200 + 1)) * 200 = (1 - 1 / 1.275) * 200 = 43.137
        assert!(close(rubber(100.0, 200.0), 43.137, 1e-3));
        assert!(rubber(1e9, 200.0) < 200.0);
        assert!(rubber(10.0, 200.0) < rubber(20.0, 200.0));
    }

    #[test]
    fn inside_the_range_the_content_follows_the_finger_one_to_one() {
        let mut physics = ScrollPhysics::new(100.0, 500.0);
        physics.drag(0.0, 30.0);
        assert_eq!(physics.offset(), 30.0);
        physics.drag(10.0, -10.0);
        assert_eq!(physics.offset(), 20.0);
        assert_eq!(physics.max_offset(), 400.0);
    }

    #[test]
    fn dragging_past_the_top_is_rubber_banded() {
        let mut physics = ScrollPhysics::new(100.0, 500.0);
        physics.drag(0.0, -100.0);
        // finger -100 past the top of a 100 pt view: -(1 - 1 / (100 * 0.55 / 100 + 1)) * 100 = -35.484
        assert!(close(physics.offset(), -35.484, 0.01));
    }

    #[test]
    fn a_fling_decelerates_by_the_decay_rate_and_stops_at_the_speed_floor() {
        let mut physics = ScrollPhysics::new(100.0, 1000.0);
        physics.drag(0.0, 0.0);
        physics.drag(100.0, 100.0);
        // 100 pt in the last 100 ms: 1000 pt/s, from offset 100 (max 900).
        let motion = physics.release(100.0).expect("a fling");
        let Motion::Fling(fling) = motion else { panic!("expected a fling") };
        assert!(close(fling.velocity, 1000.0, 1e-3));
        // Duration: ln(1000 / 20) / 2.002003 = 1.95405 s.
        assert!(close(fling.duration() as f32, 1.95405, 1e-3));
        assert_eq!(fling.offset_at(0.0), 100.0);
        // Distance until the speed reaches 20 pt/s: (1000 / 2.002003) * (1 - 0.02) = 489.509.
        assert!(close(fling.offset_at(fling.duration()), 589.509, 0.05));
        // The model's finger rests where the fling ends.
        assert!(close(physics.offset(), 589.509, 0.05));
    }

    #[test]
    fn a_fling_is_clamped_at_the_end_of_the_content() {
        let fling = Fling { from: 800.0, velocity: 2000.0, min: 0.0, max: 900.0 };
        let mut last = fling.from;
        let steps = 40;
        for i in 0..=steps {
            let t = fling.duration() * i as f64 / steps as f64;
            let o = fling.offset_at(t);
            assert!(o >= last - 1e-4, "the offset never goes back during a fling");
            assert!(o <= 900.0);
            last = o;
        }
        assert_eq!(fling.offset_at(fling.duration()), 900.0);
    }

    #[test]
    fn a_slow_release_does_not_fling() {
        let mut physics = ScrollPhysics::new(100.0, 1000.0);
        physics.drag(0.0, 0.0);
        physics.drag(100.0, 1.0);
        // 1 pt in 100 ms is 10 pt/s, below the 20 pt/s floor.
        assert_eq!(physics.release(100.0), None);
        assert!(close(physics.offset(), 1.0, 1e-6));
    }

    #[test]
    fn a_release_past_an_edge_springs_back_to_it() {
        let mut physics = ScrollPhysics::new(100.0, 200.0);
        physics.drag(0.0, 0.0);
        physics.drag(100.0, -50.0);
        // finger -50 past the top: shown at -(1 - 1 / (50 * 0.55 / 100 + 1)) * 100 = -21.568
        let motion = physics.release(100.0).expect("a bounce");
        match motion {
            Motion::Bounce { from, to, .. } => {
                assert!(close(from, -21.568, 0.01));
                assert_eq!(to, 0.0);
            }
            other => panic!("expected a bounce, got {other:?}"),
        }
        assert_eq!(physics.offset(), 0.0);
    }

    #[test]
    fn a_bounce_settles_on_its_edge() {
        let bounce = Motion::Bounce { from: -20.0, to: 0.0, velocity: 0.0 };
        assert_eq!(bounce.at(0.0), (-20.0, false));
        let (mid, done) = bounce.at(0.1);
        assert!(mid < 0.0 && mid > -20.0 && !done, "moving back: {mid}");
        assert_eq!(bounce.at(2.0), (0.0, true));
    }

    #[test]
    fn a_new_drag_after_a_fling_takes_over_from_the_release_point() {
        let mut physics = ScrollPhysics::new(100.0, 1000.0);
        physics.drag(0.0, 0.0);
        physics.drag(100.0, 100.0);
        physics.release(100.0);
        physics.set_offset(250.0);
        assert_eq!(physics.offset(), 250.0);
        physics.drag(500.0, 10.0);
        assert_eq!(physics.offset(), 260.0);
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use crate::kit::Font;

    fn ui() -> Ui {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(Font::parse(data).unwrap(), 1.0)
    }

    /// A document of a fixed height, standing in for a long list.
    struct Doc {
        base: ViewBase,
        h: f32,
    }
    impl View for Doc {
        crate::view_base!();
        fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
            (0.0, self.h)
        }
    }

    /// A 100 pt scroll view over a 1000 pt document: a 900 pt range.
    fn list() -> ScrollView {
        let mut sv = ScrollView::new(Box::new(Doc { base: ViewBase::default(), h: 1000.0 }));
        sv.layout(&ui(), Rect { x: 0.0, y: 0.0, w: 200.0, h: 100.0 });
        sv
    }

    /// Advances the frame clock to `t` seconds, then delivers a pointer event.
    fn at(sv: &mut ScrollView, t: f64, ev: Event) {
        sv.tick(t);
        sv.handle(&ViewEvent::Pointer(ev));
    }

    #[test]
    fn a_tap_moves_nothing_and_starts_no_motion() {
        let mut sv = list();
        at(&mut sv, 0.0, Event::Down(10.0, 50.0));
        at(&mut sv, 0.05, Event::Up(10.0, 52.0));
        assert_eq!(sv.content_offset(), 0.0);
        assert!(sv.motion.is_none());
    }

    #[test]
    fn a_fast_drag_flings_the_content_on_and_it_comes_to_rest_at_the_end() {
        let mut sv = list();
        at(&mut sv, 0.000, Event::Down(10.0, 90.0));
        at(&mut sv, 0.016, Event::Move(10.0, 80.0));
        at(&mut sv, 0.032, Event::Move(10.0, 60.0));
        at(&mut sv, 0.048, Event::Move(10.0, 40.0));
        at(&mut sv, 0.064, Event::Up(10.0, 40.0));
        let released = sv.content_offset();
        assert!(released > 0.0 && matches!(sv.motion, Some((Motion::Fling(_), _))), "a fast drag flings");
        sv.tick(0.5);
        assert!(sv.content_offset() > released + 100.0, "coasted on: {}", sv.content_offset());
        sv.tick(10.0);
        assert!(sv.motion.is_none());
        assert!(sv.content_offset() > released && sv.content_offset() <= sv.max_offset());
    }

    #[test]
    fn a_touch_during_a_fling_catches_the_content_where_it_is() {
        let mut sv = list();
        at(&mut sv, 0.000, Event::Down(10.0, 90.0));
        at(&mut sv, 0.016, Event::Move(10.0, 60.0));
        at(&mut sv, 0.032, Event::Move(10.0, 40.0));
        at(&mut sv, 0.048, Event::Up(10.0, 40.0));
        sv.tick(0.2);
        let before = sv.content_offset();
        assert!(sv.motion.is_some());
        at(&mut sv, 0.2, Event::Down(10.0, 50.0));
        assert!(sv.motion.is_none());
        assert!((sv.content_offset() - before).abs() < 1e-3);
    }

    #[test]
    fn a_slow_drag_stops_where_the_finger_leaves_it() {
        let mut sv = list();
        at(&mut sv, 0.0, Event::Down(10.0, 80.0));
        at(&mut sv, 0.4, Event::Move(10.0, 40.0));
        at(&mut sv, 0.8, Event::Move(10.0, 30.0));
        at(&mut sv, 0.9, Event::Up(10.0, 30.0));
        assert!(sv.motion.is_none(), "a slow release does not fling");
        assert!((sv.content_offset() - 38.0).abs() < 1e-3, "stopped at {}", sv.content_offset());
    }

    #[test]
    fn a_pull_past_the_top_rubber_bands_and_springs_back_on_release() {
        let mut sv = list();
        at(&mut sv, 0.0, Event::Down(10.0, 50.0));
        at(&mut sv, 0.016, Event::Move(10.0, 150.0));
        let pulled = sv.content_offset();
        assert!(pulled < 0.0 && pulled > -100.0, "rubber banded: {pulled}");
        at(&mut sv, 0.032, Event::Up(10.0, 150.0));
        assert!(matches!(sv.motion, Some((Motion::Bounce { .. }, _))));
        sv.tick(3.0);
        assert!(sv.motion.is_none());
        assert_eq!(sv.content_offset(), 0.0);
    }
}
