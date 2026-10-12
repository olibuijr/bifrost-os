//! Implicit animation (`CATransaction`, `CABasicAnimation`, `CASpringAnimation`, `NativeAnimationContext.runAnimationGroup`).
//!
//! [`Window::animate`] runs a closure; every animatable property that the closure changes (frame, opacity,
//! background) is not applied at once but interpolated from its current on-screen value to the new one, with the
//! curves and springs of `bifrost_ui::anim`. The window diffs the tree before and after the closure, so any code
//! that sets `frame`, `opacity` or `background` (including `layout`) animates without per-view support. A new change
//! that arrives mid-flight retargets from the value on screen, so nothing jumps. [`Window::tick`] advances the tracks,
//! [`Window::next_wake`] says when the next frame is due (`None` when idle). Gaps are in docs/bifrost-ui.md.
use super::*;
use bifrost_ui::anim::{Animation, Curve};
use std::collections::HashMap;

/// `CATransaction.animationDuration` default.
pub const DEFAULT_DURATION: f32 = 0.25;
/// Duration of the cross-fade that replaces motion under Reduce Motion.
pub const REDUCED_DURATION: f32 = 0.12;
/// Frame interval the window asks for while animating.
const FRAME: f64 = 1.0 / 60.0;

/// An animatable property of a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Prop {
    X,
    Y,
    Width,
    Height,
    Opacity,
    Background,
}

impl Prop {
    fn is_frame(self) -> bool {
        matches!(self, Prop::X | Prop::Y | Prop::Width | Prop::Height)
    }
}

/// Duration and curve of a transaction (`CATransaction.animationDuration` + `animationTimingFunction`, or a spring).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub duration: f32,
    pub curve: Curve,
}

impl Timing {
    /// 0.25 s ease-in-out, like an implicit `CATransaction`.
    pub const DEFAULT: Timing = Timing { duration: DEFAULT_DURATION, curve: Curve::EaseInOut };
    pub fn new(duration: f32, curve: Curve) -> Timing {
        Timing { duration, curve }
    }
    /// `CASpringAnimation`: lasts as long as the spring takes to settle.
    pub fn spring(damping_ratio: f32, response: f32) -> Timing {
        Timing { duration: -1.0, curve: Curve::Spring { damping_ratio, response } }
    }
    fn animation(self, from: f32, to: f32) -> Animation {
        match self.curve {
            Curve::Spring { damping_ratio, response } if self.duration < 0.0 => Animation::spring(from, to, 0.0, damping_ratio, response),
            c => Animation::new(from, to, 0.0, self.duration, c),
        }
    }
    fn instant(self) -> bool {
        self.duration == 0.0 && !matches!(self.curve, Curve::Spring { .. })
    }
}

impl Default for Timing {
    fn default() -> Self {
        Timing::DEFAULT
    }
}

/// `CATransaction` / animation group: timing plus completion. `completion(finished)` runs once, after the last track
/// ends (`finished` false when an animation was replaced or interrupted); `action` is queued for
/// [`Window::take_animation_actions`] only when everything finished.
pub struct Transaction {
    pub timing: Timing,
    pub completion: Option<Box<dyn FnOnce(bool)>>,
    pub action: Option<String>,
}

impl Transaction {
    pub fn new(timing: Timing) -> Self {
        Transaction { timing, completion: None, action: None }
    }
    pub fn completion(mut self, f: impl FnOnce(bool) + 'static) -> Self {
        self.completion = Some(Box::new(f));
        self
    }
    pub fn action(mut self, a: &str) -> Self {
        self.action = Some(a.into());
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Props {
    frame: Rect,
    opacity: f32,
    background: Option<u32>,
}

impl Props {
    fn of(b: &ViewBase) -> Props {
        Props { frame: b.frame, opacity: b.opacity, background: b.background }
    }
    fn get(&self, p: Prop) -> f32 {
        match p {
            Prop::X => self.frame.x,
            Prop::Y => self.frame.y,
            Prop::Width => self.frame.w,
            Prop::Height => self.frame.h,
            Prop::Opacity => self.opacity,
            Prop::Background => 0.0,
        }
    }
    fn set(&mut self, p: Prop, v: f32) {
        match p {
            Prop::X => self.frame.x = v,
            Prop::Y => self.frame.y = v,
            Prop::Width => self.frame.w = v,
            Prop::Height => self.frame.h = v,
            Prop::Opacity => self.opacity = v,
            Prop::Background => {}
        }
    }
}

const SCALAR: [Prop; 5] = [Prop::X, Prop::Y, Prop::Width, Prop::Height, Prop::Opacity];

struct Group {
    remaining: usize,
    cancelled: bool,
    completion: Option<Box<dyn FnOnce(bool)>>,
    action: Option<String>,
}

struct Track {
    id: u64,
    prop: Prop,
    anim: Animation,
    started: bool,
    /// the value last written to the view; a different value there means someone else set the property
    shown: f32,
    /// `Background`: from, to, final value
    colors: Option<(u32, u32, Option<u32>)>,
    shown_color: Option<u32>,
    group: u64,
}

#[derive(Default)]
pub(super) struct Animator {
    tracks: Vec<Track>,
    groups: HashMap<u64, Group>,
    next_group: u64,
    snaps: Vec<HashMap<u64, Props>>,
    damage: Option<Rect>,
    actions: Vec<String>,
    /// groups with no track left to run: completed on the next tick
    ready: Vec<u64>,
    pub(super) reduce: Option<bool>,
}

fn union(a: Option<Rect>, b: Rect) -> Rect {
    match a {
        None => b,
        Some(a) => {
            let (x, y) = (a.x.min(b.x), a.y.min(b.y));
            Rect { x, y, w: (a.x + a.w).max(b.x + b.w) - x, h: (a.y + a.h).max(b.y + b.h) - y }
        }
    }
}

/// Linear mix of two premultiplied colours.
pub fn mix_color(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = (((a >> s) & 255) as f32, ((b >> s) & 255) as f32);
        ((x + (y - x) * t).round().clamp(0.0, 255.0) as u32) << s
    };
    ch(0) | ch(8) | ch(16) | ch(24)
}

fn walk(v: &dyn View, f: &mut dyn FnMut(&dyn View)) {
    f(v);
    v.children().iter().for_each(|c| walk(c.as_ref(), f));
}

fn walk_mut(v: &mut dyn View, f: &mut dyn FnMut(&mut dyn View)) {
    f(v);
    v.children_mut().iter_mut().for_each(|c| walk_mut(c.as_mut(), f));
}

fn snapshot(root: &dyn View) -> HashMap<u64, Props> {
    let mut m = HashMap::new();
    walk(root, &mut |v| {
        m.insert(v.id(), Props::of(v.base()));
    });
    m
}

impl Animator {
    fn reduced(&self) -> bool {
        self.reduce.unwrap_or_else(crate::kit::appearance::reduce_motion)
    }
    fn finish(&mut self, group: u64, finished: bool) {
        let Some(g) = self.groups.get_mut(&group) else { return };
        g.cancelled |= !finished;
        g.remaining = g.remaining.saturating_sub(1);
        if g.remaining == 0 {
            self.complete(group);
        }
    }
    fn complete(&mut self, group: u64) {
        if let Some(g) = self.groups.remove(&group) {
            let ok = !g.cancelled;
            if let (true, Some(a)) = (ok, g.action) {
                self.actions.push(a);
            }
            if let Some(c) = g.completion {
                c(ok);
            }
        }
    }
    fn cancel(&mut self, id: u64, prop: Prop) {
        if let Some(i) = self.tracks.iter().position(|t| t.id == id && t.prop == prop) {
            let t = self.tracks.remove(i);
            self.finish(t.group, false);
        }
    }
}

impl<S: Responder> Window<S> {
    /// `NativeView.animate(withDuration:)`: property changes made by `f` run over `timing` (see the module docs).
    pub fn animate(&mut self, timing: Timing, f: impl FnOnce(&mut Self)) {
        self.run(Some(Transaction::new(timing)), f)
    }

    /// `NativeAnimationContext.runAnimationGroup`: like [`Window::animate`] with a completion and/or an action.
    pub fn transaction(&mut self, t: Transaction, f: impl FnOnce(&mut Self)) {
        self.run(Some(t), f)
    }

    /// `CATransaction.setDisableActions(true)`: changes made by `f` apply at once, also inside an `animate` and
    /// cancelling animations already running on the touched properties.
    pub fn disable_actions(&mut self, f: impl FnOnce(&mut Self)) {
        self.run(None, f)
    }

    /// Overrides the Reduce Motion preference (`None` follows `appearance::reduce_motion`). Under Reduce Motion
    /// frame changes are instant and opacity and background cross-fade over 0.12 s.
    pub fn set_reduce_motion(&mut self, on: Option<bool>) {
        self.anim.reduce = on;
    }

    fn run(&mut self, tx: Option<Transaction>, f: impl FnOnce(&mut Self)) {
        let before = snapshot(self.root());
        self.anim.snaps.push(before);
        f(self);
        let before = self.anim.snaps.pop().unwrap_or_default();
        let after = snapshot(self.root());
        let reduced = self.anim.reduced();
        let group = self.anim.next_group;
        self.anim.next_group += 1;
        let mut count = 0;
        let timing = tx.as_ref().map(|t| t.timing);
        let mut changes: Vec<(u64, Prop, Props, Props)> = Vec::new();
        for (id, b) in &before {
            if let Some(a) = after.get(id).filter(|a| *a != b) {
                for p in SCALAR.iter().copied().chain([Prop::Background]) {
                    let differs = if p == Prop::Background { a.background != b.background } else { a.get(p) != b.get(p) };
                    if differs {
                        changes.push((*id, p, *b, *a));
                    }
                }
            }
        }
        for (id, p, b, a) in changes {
            let instant = match timing {
                None => true,
                Some(t) => t.instant() || (reduced && p.is_frame()),
            };
            self.anim.cancel(id, p);
            if instant {
                // outer transactions must not animate what was set without actions
                for snap in &mut self.anim.snaps {
                    if let Some(e) = snap.get_mut(&id) {
                        match p {
                            Prop::Background => e.background = a.background,
                            _ => e.set(p, a.get(p)),
                        }
                    }
                }
                continue;
            }
            let mut t = timing.unwrap_or_default();
            if reduced {
                t = Timing::new(REDUCED_DURATION, Curve::Linear);
            }
            let track = if p == Prop::Background {
                let c = |o: Option<u32>| o.unwrap_or(0);
                Track {
                    id,
                    prop: p,
                    anim: t.animation(0.0, 1.0),
                    started: false,
                    shown: 0.0,
                    colors: Some((c(b.background), c(a.background), a.background)),
                    shown_color: b.background,
                    group,
                }
            } else {
                Track { id, prop: p, anim: t.animation(b.get(p), a.get(p)), started: false, shown: b.get(p), colors: None, shown_color: None, group }
            };
            // the view keeps showing the old value until the first tick starts the clock
            walk_mut(self.root.as_mut(), &mut |v| {
                if v.id() == id {
                    let base = v.base_mut();
                    match p {
                        Prop::X => base.frame.x = b.frame.x,
                        Prop::Y => base.frame.y = b.frame.y,
                        Prop::Width => base.frame.w = b.frame.w,
                        Prop::Height => base.frame.h = b.frame.h,
                        Prop::Opacity => base.opacity = b.opacity,
                        Prop::Background => base.background = b.background,
                    }
                    base.dirty = true;
                }
            });
            self.anim.tracks.push(track);
            count += 1;
        }
        if let Some(t) = tx {
            self.anim.groups.insert(group, Group { remaining: count, cancelled: false, completion: t.completion, action: t.action });
            if count == 0 {
                self.anim.ready.push(group);
            }
        }
        self.full |= count > 0;
    }

    /// Where `id`'s frame is heading: the end of its running animations, else the current frame.
    pub fn target_frame(&self, id: u64) -> Option<Rect> {
        let mut r = None;
        walk(self.root(), &mut |v| {
            if v.id() == id {
                r = Some(v.base().frame);
            }
        });
        let mut r = r?;
        for t in self.anim.tracks.iter().filter(|t| t.id == id) {
            match t.prop {
                Prop::X => r.x = t.anim.to,
                Prop::Y => r.y = t.anim.to,
                Prop::Width => r.w = t.anim.to,
                Prop::Height => r.h = t.anim.to,
                _ => {}
            }
        }
        Some(r)
    }

    /// True while a property of `id` (or of any view when `None`) is animating.
    pub fn is_animating(&self, id: Option<u64>) -> bool {
        self.anim.tracks.iter().any(|t| id.map_or(true, |i| t.id == i))
    }

    /// Actions of finished transactions ([`Transaction::action`]); send them with [`Window::send`].
    pub fn take_animation_actions(&mut self) -> Vec<String> {
        std::mem::take(&mut self.anim.actions)
    }

    /// The union of the old and new frames of every view moved or faded since the last [`Window::draw`].
    pub fn animation_damage(&self) -> Option<Rect> {
        self.anim.damage
    }

    /// When the next animation frame is due, in the clock of `t` (seconds); `None` when nothing animates.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        (!self.anim.tracks.is_empty() || !self.anim.ready.is_empty() || super::any_wants_tick(self.root.as_ref())).then_some(t + FRAME)
    }

    /// Advances the tracks to `t` (seconds). Called by [`Window::tick`].
    pub(super) fn advance_animations(&mut self, t: f64) {
        if self.anim.tracks.is_empty() && self.anim.ready.is_empty() {
            return;
        }
        let mut tracks = std::mem::take(&mut self.anim.tracks);
        let mut seen = vec![false; tracks.len()];
        let mut done: Vec<(usize, bool)> = Vec::new();
        let mut damage = self.anim.damage;
        let mut moved = false;
        walk_mut(self.root.as_mut(), &mut |v| {
            let id = v.id();
            for (i, tr) in tracks.iter_mut().enumerate().filter(|(_, tr)| tr.id == id) {
                seen[i] = true;
                let base = v.base_mut();
                let old = base.frame;
                let mut p = Props::of(base);
                let external = if tr.prop == Prop::Background { base.background != tr.shown_color } else { p.get(tr.prop) != tr.shown };
                if external {
                    done.push((i, false));
                    continue;
                }
                if !tr.started {
                    tr.started = true;
                    tr.anim.start = t;
                }
                let fin = tr.anim.is_done(t);
                if tr.prop == Prop::Background {
                    let (a, b, end) = tr.colors.unwrap_or((0, 0, None));
                    let c = if fin { end } else { Some(mix_color(a, b, tr.anim.value(t))) };
                    base.background = c;
                    tr.shown_color = c;
                } else {
                    let val = if fin { tr.anim.to } else { tr.anim.value(t) };
                    p.set(tr.prop, val);
                    base.frame = p.frame;
                    base.opacity = p.opacity;
                    tr.shown = val;
                }
                base.dirty = true;
                if tr.prop.is_frame() && base.frame != old {
                    moved = true;
                }
                damage = Some(union(Some(union(damage, old)), base.frame));
                if fin {
                    done.push((i, true));
                }
            }
        });
        for (i, s) in seen.iter().enumerate() {
            if !s {
                done.push((i, false)); // the view left the tree
            }
        }
        done.sort_by(|a, b| b.0.cmp(&a.0));
        done.dedup_by_key(|d| d.0);
        let mut finished = Vec::new();
        for (i, ok) in done {
            let tr = tracks.remove(i);
            finished.push((tr.group, ok));
        }
        let an = &mut self.anim;
        an.tracks = tracks;
        an.damage = damage;
        for (g, ok) in finished {
            an.finish(g, ok);
        }
        for g in std::mem::take(&mut an.ready) {
            an.complete(g);
        }
        if moved {
            self.full = true; // vacated areas need a repaint: the window is redrawn in full
        }
    }

    pub(super) fn clear_animation_damage(&mut self) {
        self.anim.damage = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_render::canvas::RectF;
    use crate::kit::Surface;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn win() -> (Window, u64) {
        let c = Container::new(vec![Box::new(Container::default())]);
        let mut w: Window = Window::new(c, 10);
        let id = w.root().children()[0].id();
        w.root_mut().children_mut()[0].set_frame(rect(0.0, 0.0, 100.0, 50.0));
        w.root_mut().set_frame(rect(0.0, 0.0, 400.0, 400.0));
        w.set_reduce_motion(Some(false));
        (w, id)
    }

    fn frame(w: &Window, id: u64) -> Rect {
        w.view::<Container>(id).unwrap().base().frame
    }

    fn lin(d: f32) -> Timing {
        Timing::new(d, Curve::Linear)
    }

    #[test]
    fn default_timing_is_a_quarter_second_ease() {
        assert_eq!(Timing::default().duration, 0.25);
        assert_eq!(Timing::default().curve, Curve::EaseInOut);
    }

    #[test]
    fn linear_midpoint_and_end() {
        let (mut w, id) = win();
        w.animate(lin(1.0), |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(100.0, 0.0, 100.0, 50.0)));
        assert_eq!(frame(&w, id).x, 0.0, "nothing moves before the clock starts");
        w.tick(10.0);
        assert_eq!(frame(&w, id).x, 0.0);
        w.tick(10.5);
        assert!((frame(&w, id).x - 50.0).abs() < 1e-3);
        w.tick(11.0);
        assert_eq!(frame(&w, id).x, 100.0);
        assert!(!w.is_animating(None));
    }

    #[test]
    fn opacity_animates_and_instant_changes_are_not_animated() {
        let (mut w, id) = win();
        w.animate(lin(1.0), |w| w.view_mut::<Container>(id).unwrap().base_mut().opacity = 0.0);
        w.tick(0.0);
        w.tick(0.25);
        assert!((w.view::<Container>(id).unwrap().base().opacity - 0.75).abs() < 1e-3);
        w.disable_actions(|w| w.view_mut::<Container>(id).unwrap().base_mut().opacity = 1.0);
        assert_eq!(w.view::<Container>(id).unwrap().base().opacity, 1.0);
        w.tick(0.5);
        assert_eq!(w.view::<Container>(id).unwrap().base().opacity, 1.0);
        assert!(!w.is_animating(None));
    }

    #[test]
    fn background_mixes_colours() {
        let (mut w, id) = win();
        w.animate(lin(1.0), |w| w.view_mut::<Container>(id).unwrap().base_mut().background = Some(0xff00_00ff));
        w.tick(0.0);
        w.tick(0.5);
        let c = w.view::<Container>(id).unwrap().base().background.unwrap();
        assert_eq!(c & 0xff, 0x80);
        w.tick(1.0);
        assert_eq!(w.view::<Container>(id).unwrap().base().background, Some(0xff00_00ff));
    }

    #[test]
    fn retarget_continues_from_the_value_on_screen() {
        let (mut w, id) = win();
        w.animate(lin(1.0), |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(100.0, 0.0, 100.0, 50.0)));
        w.tick(0.0);
        w.tick(0.5);
        let mid = frame(&w, id).x;
        w.animate(lin(1.0), |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(0.0, 0.0, 100.0, 50.0)));
        assert_eq!(frame(&w, id).x, mid, "no jump when the new change arrives");
        w.tick(0.5);
        assert!((frame(&w, id).x - mid).abs() < 1e-3);
        w.tick(1.0);
        assert!((frame(&w, id).x - mid / 2.0).abs() < 1e-3);
        assert_eq!(w.target_frame(id).unwrap().x, 0.0);
        w.tick(1.5);
        assert_eq!(frame(&w, id).x, 0.0);
    }

    #[test]
    fn setting_a_property_outside_a_transaction_cancels_its_animation() {
        let (mut w, id) = win();
        w.animate(lin(1.0), |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(100.0, 0.0, 100.0, 50.0)));
        w.tick(0.0);
        w.tick(0.5);
        w.view_mut::<Container>(id).unwrap().set_frame(rect(7.0, 0.0, 100.0, 50.0));
        w.tick(0.75);
        assert_eq!(frame(&w, id).x, 7.0);
        assert!(!w.is_animating(None));
    }

    #[test]
    fn completion_fires_once_with_finished_flag() {
        use std::{cell::RefCell, rc::Rc};
        let (mut w, id) = win();
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        let tx = Transaction::new(lin(1.0)).completion(move |ok| l.borrow_mut().push(ok)).action("done");
        w.transaction(tx, |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(10.0, 5.0, 100.0, 50.0)));
        w.tick(0.0);
        w.tick(0.5);
        assert!(log.borrow().is_empty());
        w.tick(1.0);
        w.tick(1.5);
        w.tick(2.0);
        assert_eq!(*log.borrow(), vec![true]);
        assert_eq!(w.take_animation_actions(), vec!["done".to_string()]);
        // an interrupted group completes once with false and queues no action
        let l = log.clone();
        let tx = Transaction::new(lin(1.0)).completion(move |ok| l.borrow_mut().push(ok)).action("again");
        w.transaction(tx, |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(50.0, 5.0, 100.0, 50.0)));
        w.tick(3.0);
        w.animate(lin(1.0), |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(0.0, 5.0, 100.0, 50.0)));
        assert_eq!(*log.borrow(), vec![true, false]);
        assert!(w.take_animation_actions().is_empty());
    }

    #[test]
    fn empty_transaction_completes_on_the_next_tick() {
        use std::{cell::Cell, rc::Rc};
        let (mut w, _) = win();
        let n = Rc::new(Cell::new(0));
        let c = n.clone();
        w.transaction(Transaction::new(lin(1.0)).completion(move |_| c.set(c.get() + 1)), |_| {});
        assert_eq!(n.get(), 0);
        assert!(w.next_wake(0.0).is_some());
        w.tick(0.0);
        w.tick(0.1);
        assert_eq!(n.get(), 1);
    }

    #[test]
    fn damage_is_the_union_of_old_and_new_frames() {
        let (mut w, id) = win();
        w.animate(lin(1.0), |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(100.0, 100.0, 100.0, 50.0)));
        w.tick(0.0);
        w.tick(1.0);
        let d = w.animation_damage().unwrap();
        assert!(d.x <= 0.0 && d.y <= 0.0 && d.x + d.w >= 200.0 && d.y + d.h >= 150.0);
        assert!(w.needs_display());
    }

    #[test]
    fn spring_settles_on_the_target() {
        let (mut w, id) = win();
        w.animate(Timing::spring(0.6, 0.3), |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(80.0, 0.0, 100.0, 50.0)));
        w.tick(0.0);
        w.tick(0.1);
        assert!(frame(&w, id).x > 0.0);
        w.tick(60.0);
        assert_eq!(frame(&w, id).x, 80.0);
        assert_eq!(w.next_wake(60.0), None);
    }

    #[test]
    fn idle_window_has_no_wake() {
        let (mut w, id) = win();
        assert_eq!(w.next_wake(1.0), None);
        w.animate(lin(1.0), |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(5.0, 0.0, 100.0, 50.0)));
        assert!(w.next_wake(1.0).unwrap() > 1.0);
        w.tick(1.0);
        w.tick(3.0);
        assert_eq!(w.next_wake(3.0), None);
    }

    #[test]
    fn reduce_motion_makes_frames_instant_and_fades_short() {
        let (mut w, id) = win();
        w.set_reduce_motion(Some(true));
        w.animate(lin(1.0), |w| {
            let v = w.view_mut::<Container>(id).unwrap();
            v.set_frame(rect(60.0, 0.0, 100.0, 50.0));
            v.base_mut().opacity = 0.0;
        });
        assert_eq!(frame(&w, id).x, 60.0);
        w.tick(0.0);
        w.tick(1.0);
        assert_eq!(w.view::<Container>(id).unwrap().base().opacity, 0.0);
        assert!(!w.is_animating(None));
    }

    #[test]
    fn disabled_block_inside_a_transaction_is_not_animated_by_the_outer_one() {
        let (mut w, id) = win();
        w.animate(lin(1.0), |w| {
            w.disable_actions(|w| w.view_mut::<Container>(id).unwrap().set_frame(rect(40.0, 0.0, 100.0, 50.0)));
        });
        assert_eq!(frame(&w, id).x, 40.0);
        assert!(!w.is_animating(None));
    }

    #[test]
    fn faded_view_blends_over_its_backdrop() {
        // group opacity: a white square at 50% over black is mid grey, the rest of the surface is untouched
        let mut px = vec![0xff00_0000u32; 16];
        let mut s = Surface { w: 4, h: 4, stride: 4, px: &mut px, y0: 0 };
        {
            let c: &mut dyn Canvas = &mut s;
            c.push_opacity(0.5);
            c.fill_rect(RectF::new(0.0, 0.0, 2.0, 2.0), 0xffff_ffff);
            c.pop_opacity();
        }
        let p = px[0];
        assert!((p & 0xff).abs_diff(0x80) <= 1 && p >> 24 == 0xff);
        assert_eq!(px[3], 0xff00_0000);
    }
}
