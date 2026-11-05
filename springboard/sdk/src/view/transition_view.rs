//! `CATransition` for apps: a view whose one content view is replaced with a transition effect (fade, move in, push,
//! reveal, from any side), the same `bifrost_ui::transition` model the shell's Settings and Files stacks use.
//!
//! [`TransitionView::transition_to`] puts the new content in place at once (it is the model: lookups, layout and the
//! responder chain see it straight away) and keeps the old content aside to draw the effect. While the effect runs the
//! transition view draws both itself, clipped to its frame, and the new content takes no pointer events (it is hidden
//! from the window's own passes until the run ends; its own `hidden` flag is restored then). A second
//! `transition_to` during a run ends the running one at once (its completion hears `false`). Under Reduce Motion
//! every effect becomes the short cross-fade (`Transition::reduced`). The clock starts at the next `Window::tick`.
use super::{draw_subtree, View, ViewBase};
use crate::kit::canvas::{Canvas, RectF, Transform2D};
use crate::kit::ui::BLACK;
use crate::kit::Ui;
use bifrost_ui::transition::{Placement, Run};
pub use bifrost_ui::transition::{Ease as TransitionEase, Kind as TransitionKind, Subtype as TransitionSubtype, Transition};
use bifrost_ui::Rect;

/// A content view swapped with a transition effect (see the module docs).
pub struct TransitionView {
    base: ViewBase,
    /// the current content: empty or one view
    content: Vec<Box<dyn View>>,
    /// the content leaving, drawn until the run ends
    outgoing: Option<Box<dyn View>>,
    /// a transition waiting for its first tick
    pending: Option<Transition>,
    run: Option<Run>,
    clock: f64,
    /// the incoming view's own `hidden` flag, put back when the run ends
    was_hidden: bool,
    completion: Option<Box<dyn FnOnce(bool)>>,
    reduce: Option<bool>,
}

impl TransitionView {
    pub fn new(content: Box<dyn View>) -> Self {
        let mut v = TransitionView::empty();
        v.content.push(content);
        v
    }
    /// A transition view with no content yet; the first `transition_to` brings content in over nothing.
    pub fn empty() -> Self {
        TransitionView {
            base: ViewBase::default(),
            content: Vec::new(),
            outgoing: None,
            pending: None,
            run: None,
            clock: 0.0,
            was_hidden: false,
            completion: None,
            reduce: None,
        }
    }
    /// The current content.
    pub fn content(&self) -> Option<&dyn View> {
        self.content.first().map(|c| c.as_ref())
    }
    pub fn content_mut(&mut self) -> Option<&mut Box<dyn View>> {
        self.content.first_mut()
    }
    /// Overrides the Reduce Motion preference (`None` follows `appearance::reduce_motion`).
    pub fn set_reduce_motion(&mut self, on: Option<bool>) {
        self.reduce = on;
    }
    fn reduced(&self) -> bool {
        self.reduce.unwrap_or_else(crate::kit::appearance::reduce_motion)
    }
    /// True from `transition_to` until the effect has finished.
    pub fn is_transitioning(&self) -> bool {
        self.pending.is_some() || self.run.is_some()
    }
    /// The effect running or about to run (after Reduce Motion is applied).
    pub fn current_transition(&self) -> Option<Transition> {
        self.pending.or(self.run.map(|r| r.transition))
    }

    /// Replaces the content at once, with no effect, and returns the old content. Ends a run in flight.
    pub fn set_content(&mut self, next: Box<dyn View>) -> Option<Box<dyn View>> {
        self.finish(false);
        let mut next = next;
        next.set_frame(self.base.frame);
        let old = self.content.pop();
        self.content.push(next);
        self.base.dirty = true;
        old
    }

    /// Replaces the content with `next` and plays `t` from the old content to it. Call `Window::layout` afterwards as
    /// after any change of content; `next` starts with this view's frame.
    pub fn transition_to(&mut self, next: Box<dyn View>, t: Transition) {
        self.transition_to_then(next, t, |_| {});
    }

    /// Like [`TransitionView::transition_to`]; `completion(finished)` runs once when the effect ends (`false` when a
    /// later change cut it short), as `animationDidStop(_:finished:)` does for a transition.
    pub fn transition_to_then(&mut self, next: Box<dyn View>, t: Transition, completion: impl FnOnce(bool) + 'static) {
        self.finish(false);
        let t = if self.reduced() { Transition::reduced() } else { t };
        let mut next = next;
        next.set_frame(self.base.frame);
        self.outgoing = self.content.pop();
        self.base.dirty = true;
        if t.duration <= 0.0 {
            self.outgoing = None;
            self.content.push(next);
            completion(true);
            return;
        }
        self.was_hidden = next.base().hidden;
        next.base_mut().hidden = true;
        self.content.push(next);
        self.pending = Some(t);
        self.completion = Some(Box::new(completion));
    }

    /// Ends the effect: the old content goes, the new one is shown and takes events again.
    fn finish(&mut self, finished: bool) {
        if !self.is_transitioning() {
            return;
        }
        self.pending = None;
        self.run = None;
        self.outgoing = None;
        let hidden = self.was_hidden;
        if let Some(c) = self.content.first_mut() {
            c.base_mut().hidden = hidden;
            c.base_mut().dirty = true;
        }
        self.base.dirty = true;
        if let Some(f) = self.completion.take() {
            f(finished);
        }
    }
}

/// Draws `v` (shown even when hidden) moved by `(dx, dy)` device pixels at `alpha`.
fn draw_moved(v: &mut dyn View, c: &mut dyn Canvas, ui: &Ui, dx: f32, dy: f32, alpha: f32, unhide: bool) {
    if alpha <= 0.0 {
        return;
    }
    let hidden = v.base().hidden;
    if unhide {
        v.base_mut().hidden = false;
    }
    c.push_transform(Transform2D::translate(dx, dy));
    if alpha < 1.0 {
        c.push_opacity(alpha);
    }
    draw_subtree(v, c, ui);
    if alpha < 1.0 {
        c.pop_opacity();
    }
    c.pop_transform();
    v.base_mut().hidden = hidden;
}

/// The outgoing content and its shade.
fn draw_under(o: Option<&mut Box<dyn View>>, s: &mut dyn Canvas, ui: &Ui, pl: &Placement, area: RectF) {
    let Some(o) = o else { return };
    draw_moved(o.as_mut(), s, ui, ui.pt(pl.under_x), ui.pt(pl.under_y), pl.under_alpha, false);
    if pl.shade > 0.0 {
        let moved = RectF::new(area.x + ui.pt(pl.under_x), area.y + ui.pt(pl.under_y), area.w, area.h);
        s.push_opacity(pl.shade);
        s.fill_rect(moved, BLACK);
        s.pop_opacity();
    }
}

/// The incoming content.
fn draw_over(c: Option<&mut Box<dyn View>>, s: &mut dyn Canvas, ui: &Ui, pl: &Placement) {
    if let Some(c) = c {
        draw_moved(c.as_mut(), s, ui, ui.pt(pl.over_x), ui.pt(pl.over_y), pl.over_alpha, true);
    }
}

impl View for TransitionView {
    crate::view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.content
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.content
    }
    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        self.content.first().map_or((0.0, 0.0), |c| c.intrinsic(ui))
    }
    fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.set_frame(bounds);
        for c in &mut self.content {
            c.layout(ui, bounds);
        }
        if let Some(o) = self.outgoing.as_mut() {
            o.layout(ui, bounds);
        }
    }
    fn wants_tick(&self) -> bool {
        self.is_transitioning()
    }
    fn tick(&mut self, t: f64) {
        self.clock = t;
        if let Some(tr) = self.pending.take() {
            self.run = Some(Run::new(tr, t));
        }
        match self.run {
            Some(r) if r.is_done(t) => self.finish(true),
            Some(_) => self.base.dirty = true,
            None => {}
        }
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        if !self.is_transitioning() {
            return;
        }
        let f = self.base.frame;
        // before the first tick the effect is at its start
        let pl = match self.run {
            Some(r) => r.placement(self.clock, f.w, f.h),
            None => self.pending.map(|t| t.placement(0.0, f.w, f.h)).unwrap_or_else(|| Transition::fade().placement(1.0, f.w, f.h)),
        };
        let area = RectF::new(ui.pt(f.x).round(), ui.pt(f.y).round(), ui.pt(f.w).round(), ui.pt(f.h).round());
        s.push_clip(area, 0.0);
        if pl.over_on_top {
            draw_under(self.outgoing.as_mut(), s, ui, &pl, area);
            draw_over(self.content.first_mut(), s, ui, &pl);
        } else {
            draw_over(self.content.first_mut(), s, ui, &pl);
            draw_under(self.outgoing.as_mut(), s, ui, &pl, area);
        }
        s.pop_clip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::Surface;
    use crate::view::{Container, Window};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn swatch(color: u32) -> Box<dyn View> {
        let mut c = Container::default();
        c.base_mut().background = Some(color);
        Box::new(c)
    }

    const RED: u32 = 0xffff_0000;
    const GREEN: u32 = 0xff00_ff00;

    /// A 4 x 4 window whose only child is a transition view showing a red swatch.
    fn win() -> (Window, u64) {
        let mut tv = TransitionView::new(swatch(RED));
        tv.set_reduce_motion(Some(false));
        let mut w: Window = Window::new(Container::new(vec![Box::new(tv)]), 10);
        let id = w.root().children()[0].id();
        w.root_mut().set_frame(rect(0.0, 0.0, 4.0, 4.0));
        let tv = &mut w.root_mut().children_mut()[0];
        tv.set_frame(rect(0.0, 0.0, 4.0, 4.0));
        tv.children_mut()[0].set_frame(rect(0.0, 0.0, 4.0, 4.0));
        (w, id)
    }

    fn tv(w: &mut Window, id: u64) -> &mut TransitionView {
        w.view_mut::<TransitionView>(id).unwrap()
    }

    fn test_ui() -> Ui {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).expect("SFUI.ttf").into_boxed_slice());
        Ui::new(crate::kit::Font::parse(data).expect("font"), 1.0)
    }

    fn row(w: &mut Window, ui: &Ui) -> Vec<u32> {
        let mut px = vec![0xff00_0000u32; 16];
        let mut s = Surface { w: 4, h: 4, stride: 4, px: &mut px, y0: 0 };
        w.draw(&mut s, ui);
        px[0..4].to_vec()
    }

    #[test]
    fn the_new_content_is_the_model_at_once_and_the_old_one_leaves_when_done() {
        let (mut w, id) = win();
        let next = swatch(GREEN);
        let next_id = next.id();
        tv(&mut w, id).transition_to(next, Transition::push(TransitionSubtype::FromRight));
        assert!(w.view::<Container>(next_id).is_some(), "lookups find the new content during the run");
        assert!(tv(&mut w, id).is_transitioning());
        assert!(w.next_wake(0.0).is_some());
        w.tick(0.0);
        w.tick(0.1);
        assert!(w.view::<Container>(next_id).unwrap().base().hidden, "drawn by the transition view while it runs");
        w.tick(0.3);
        assert!(!tv(&mut w, id).is_transitioning());
        assert!(!w.view::<Container>(next_id).unwrap().base().hidden);
        assert_eq!(w.next_wake(0.3), None);
    }

    #[test]
    fn headless_push_from_the_right_shows_both_halves_at_the_midpoint() {
        let ui = test_ui();
        let (mut w, id) = win();
        let t = Transition::push(TransitionSubtype::FromRight).ease(TransitionEase::Linear).duration(1.0);
        tv(&mut w, id).transition_to(swatch(GREEN), t);
        w.tick(0.0);
        w.tick(0.5);
        assert_eq!(row(&mut w, &ui), vec![RED, RED, GREEN, GREEN], "the old content pushed half out, the new half in");
        w.tick(1.0);
        assert_eq!(row(&mut w, &ui), vec![GREEN; 4]);
    }

    #[test]
    fn headless_reveal_slides_the_old_content_off_the_new_one() {
        let ui = test_ui();
        let (mut w, id) = win();
        let t = Transition::reveal(TransitionSubtype::FromLeft).ease(TransitionEase::Linear).duration(1.0);
        tv(&mut w, id).transition_to(swatch(GREEN), t);
        w.tick(0.0);
        w.tick(0.25);
        // from the left: the old content moves right by a quarter, uncovering the first column
        assert_eq!(row(&mut w, &ui), vec![GREEN, RED, RED, RED]);
    }

    #[test]
    fn completion_hears_whether_the_effect_finished() {
        let (mut w, id) = win();
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        tv(&mut w, id).transition_to_then(swatch(GREEN), Transition::fade(), move |ok| l.borrow_mut().push(ok));
        w.tick(0.0);
        let l = log.clone();
        tv(&mut w, id).transition_to_then(swatch(RED), Transition::fade(), move |ok| l.borrow_mut().push(ok));
        assert_eq!(*log.borrow(), vec![false], "cut short by the next change");
        w.tick(0.1);
        w.tick(1.0);
        assert_eq!(*log.borrow(), vec![false, true]);
    }

    #[test]
    fn reduce_motion_turns_every_effect_into_the_short_fade_and_zero_is_instant() {
        let (mut w, id) = win();
        tv(&mut w, id).set_reduce_motion(Some(true));
        tv(&mut w, id).transition_to(swatch(GREEN), Transition::push(TransitionSubtype::FromBottom));
        assert_eq!(tv(&mut w, id).current_transition(), Some(Transition::reduced()));
        let done = Rc::new(RefCell::new(None));
        let d = done.clone();
        tv(&mut w, id).set_reduce_motion(Some(false));
        tv(&mut w, id).transition_to_then(swatch(RED), Transition::fade().duration(0.0), move |ok| *d.borrow_mut() = Some(ok));
        assert_eq!(*done.borrow(), Some(true));
        assert!(!tv(&mut w, id).is_transitioning());
    }

    #[test]
    fn set_content_swaps_without_an_effect() {
        let (mut w, id) = win();
        let old = tv(&mut w, id).set_content(swatch(GREEN));
        assert_eq!(old.unwrap().base().background, Some(RED));
        assert!(!tv(&mut w, id).is_transitioning());
        assert_eq!(tv(&mut w, id).content().unwrap().base().background, Some(GREEN));
    }
}
