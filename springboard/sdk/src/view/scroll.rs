//! `NativeScrollView`: one document view inside a clipping frame, panned by dragging, with a thin overlay scroller.
use super::{View, ViewBase, ViewEvent};
use crate::kit::ui::LABEL_2;
use crate::kit::{Event, Surface, Ui};
use bifrost_ui::scroll::Scroller;
use bifrost_ui::Rect;

/// Overlay scroller knob, in points (Golden Gate overlay style: thin, inset from the right edge).
const KNOB_W_PT: f32 = 3.0;
const KNOB_INSET_PT: f32 = 3.0;
const KNOB_MIN_PT: f32 = 24.0;

/// A scroll view (`NativeScrollView` + `NativeClipView`) holding one document view. Vertical only: the document is as wide as
/// the scroll view and as tall as its `intrinsic` height (or the scroll view's, when it has none), so `Surface`'s row
/// clip is enough. Dragging pans (with the `Scroller` fling and rubber band, driven by `View::tick`); the knob shows
/// while the content moves. There is no wheel input yet (the proto delivers only Down, Move, Up and Cancel).
pub struct ScrollView {
    base: ViewBase,
    kids: Vec<Box<dyn View>>,
    scroller: Scroller,
    /// frame time of the last `tick`: `ViewEvent` carries no timestamp, so drags are timed with it
    now: f64,
    last_y: f32,
    doc_h: f32,
    knob: bool,
}

impl ScrollView {
    pub fn new(document: Box<dyn View>) -> Self {
        ScrollView { base: ViewBase::default(), kids: vec![document], scroller: Scroller::new(), now: 0.0, last_y: 0.0, doc_h: 0.0, knob: false }
    }
    /// `NativeClipView.bounds.origin.y`: how far the document is scrolled (may be negative or past the end mid rubber band).
    pub fn content_offset(&self) -> f32 {
        self.scroller.offset()
    }
    /// The largest offset: document height minus the visible height.
    pub fn max_offset(&self) -> f32 {
        (self.doc_h - self.base.frame.h).max(0.0)
    }
    /// `NativeClipView.scroll(to:)`: jumps to `y`, clamped to `0..=max_offset()`, and stops any fling.
    pub fn scroll_to(&mut self, y: f32) {
        self.scroller.set_max(self.max_offset());
        self.scroller.jump_to(y);
        self.base.dirty = true;
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
        (0.0, self.scroller.offset())
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
        self.scroller.set_max(self.max_offset());
        self.base.dirty = true;
    }
    fn handle(&mut self, ev: &ViewEvent) -> bool {
        match ev {
            ViewEvent::Pointer(Event::Down(_, y)) => {
                self.last_y = *y;
                self.scroller.set_max(self.max_offset());
                self.scroller.down(*y, self.now * 1000.0);
            }
            ViewEvent::Pointer(Event::Move(_, y)) => {
                self.last_y = *y;
                self.scroller.moved(*y, self.now * 1000.0);
            }
            ViewEvent::Pointer(Event::Up(_, y)) => {
                self.scroller.up(*y, self.now * 1000.0);
            }
            ViewEvent::Cancel => {
                self.scroller.up(self.last_y, self.now * 1000.0);
            }
            _ => return false,
        }
        self.base.dirty = true;
        true
    }
    fn tick(&mut self, t: f64) {
        self.now = t;
        let moving = self.scroller.dragging() || self.scroller.animating();
        if self.scroller.animating() {
            self.scroller.tick(t * 1000.0);
        }
        if moving || self.knob {
            self.base.dirty = true;
        }
        self.knob = moving;
    }
    fn draw_overlay(&mut self, s: &mut Surface, ui: &Ui) {
        let max = self.max_offset();
        if !self.knob || max <= 0.0 {
            return;
        }
        let f = self.base.frame;
        let knob_h = (f.h * f.h / self.doc_h).max(KNOB_MIN_PT).min(f.h);
        let t = (self.scroller.offset() / max).clamp(0.0, 1.0);
        let y = f.y + t * (f.h - knob_h);
        // opaque colour: overlays repaint every frame, so a translucent knob would build up
        s.fill_rrect(ui.pt(f.right() - KNOB_INSET_PT - KNOB_W_PT), ui.pt(y), ui.pt(KNOB_W_PT), ui.pt(knob_h), ui.pt(KNOB_W_PT / 2.0), LABEL_2.now());
    }
}
