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
use bifrost_ui::keypath_anim::{Media, Phase};
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

/// Duration and curve of a transaction (`CATransaction.animationDuration` + `animationTimingFunction`, or a spring),
/// plus the media timing of `CAMediaTiming` (`beginTime` as a delay, `speed`, `repeatCount`, `autoreverses`), which
/// is what the view animation options for a delay, repeating and reversing set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub duration: f32,
    pub curve: Curve,
    /// Springs only: the speed the spring starts at, as a fraction of the distance per second, positive towards the
    /// target. Zero starts from rest, as `CASpringAnimation.initialVelocity` does by default.
    pub initial_velocity: f32,
    /// Seconds after the first tick before the change starts to move; until then the view shows the old value.
    pub delay: f32,
    /// Local time runs this many times faster than the window clock (1 is normal, 0 stands still).
    pub speed: f32,
    /// Cycles to play; 0 means one; `f32::INFINITY` repeats until the property is set again; a fraction plays part of
    /// the last cycle.
    pub repeat_count: f32,
    /// Each cycle plays forwards and then backwards, so it lasts twice the duration.
    pub autoreverses: bool,
}

/// `CASpringAnimation.mass` default.
pub const SPRING_MASS: f32 = 1.0;
/// `CASpringAnimation.stiffness` default.
pub const SPRING_STIFFNESS: f32 = 100.0;
/// `CASpringAnimation.damping` default.
pub const SPRING_DAMPING: f32 = 10.0;

impl Timing {
    /// 0.25 s ease-in-out, like an implicit `CATransaction`.
    pub const DEFAULT: Timing = Timing {
        duration: DEFAULT_DURATION,
        curve: Curve::EaseInOut,
        initial_velocity: 0.0,
        delay: 0.0,
        speed: 1.0,
        repeat_count: 0.0,
        autoreverses: false,
    };
    pub fn new(duration: f32, curve: Curve) -> Timing {
        Timing { duration, curve, ..Timing::DEFAULT }
    }
    /// `CASpringAnimation`: lasts as long as the spring takes to settle, and starts from rest.
    pub fn spring(damping_ratio: f32, response: f32) -> Timing {
        Timing::spring_with_velocity(damping_ratio, response, 0.0)
    }
    /// `CASpringAnimation` with an `initialVelocity`: the spring starts already moving, as a fling or a drag leaves it.
    pub fn spring_with_velocity(damping_ratio: f32, response: f32, initial_velocity: f32) -> Timing {
        Timing { duration: -1.0, curve: Curve::Spring { damping_ratio, response }, initial_velocity, ..Timing::DEFAULT }
    }
    /// `CASpringAnimation(mass:stiffness:damping:initialVelocity:)`: the physical spring `m x'' = -k x - c x'`. It is
    /// the same motion as a damping ratio `c / (2 sqrt(k m))` and a response (undamped period) `2 pi sqrt(m / k)`, so
    /// mass slows the spring and widens its swing exactly as it does in the physical form. Non-positive or non-finite
    /// values fall back to the defaults ([`SPRING_MASS`], [`SPRING_STIFFNESS`]; damping may be zero).
    pub fn spring_physical(mass: f32, stiffness: f32, damping: f32, initial_velocity: f32) -> Timing {
        let pos = |v: f32, d: f32| if v.is_finite() && v > 0.0 { v } else { d };
        let m = pos(mass, SPRING_MASS);
        let k = pos(stiffness, SPRING_STIFFNESS);
        let c = if damping.is_finite() && damping >= 0.0 { damping } else { SPRING_DAMPING };
        let damping_ratio = c / (2.0 * (k * m).sqrt());
        let response = std::f32::consts::TAU * (m / k).sqrt();
        let v0 = if initial_velocity.is_finite() { initial_velocity } else { 0.0 };
        Timing::spring_with_velocity(damping_ratio, response, v0)
    }
    /// `CASpringAnimation(perceptualDuration:bounce:)`: `duration` is the response; `bounce` 0 is critically damped,
    /// towards 1 bouncier (damping ratio `1 - bounce`), below 0 slower to settle (damping ratio `1 / (1 + bounce)`).
    pub fn spring_perceptual(duration: f32, bounce: f32) -> Timing {
        let b = if bounce.is_finite() { bounce.clamp(-0.99, 0.99) } else { 0.0 };
        let damping_ratio = if b >= 0.0 { 1.0 - b } else { 1.0 / (1.0 + b) };
        Timing::spring(damping_ratio, if duration.is_finite() && duration > 0.0 { duration } else { 0.5 })
    }
    /// `CASpringAnimation.settlingDuration`: seconds one cycle of a spring lasts; the duration itself for a curve.
    pub fn settling_duration(&self) -> f32 {
        match self.curve {
            Curve::Spring { damping_ratio, response } if self.duration < 0.0 => Animation::spring(0.0, 1.0, 0.0, damping_ratio, response).duration,
            _ => self.duration.max(0.0),
        }
    }
    /// Delays the start by `secs` (`beginTime` relative to now, the view animation `delay`).
    pub fn delay(mut self, secs: f32) -> Timing {
        self.delay = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
        self
    }
    /// Plays at `speed` times the window clock (`CAMediaTiming.speed`).
    pub fn speed(mut self, speed: f32) -> Timing {
        self.speed = if speed.is_finite() { speed.max(0.0) } else { 1.0 };
        self
    }
    /// Plays `count` cycles (`repeatCount`; `f32::INFINITY` for ever).
    pub fn repeat_count(mut self, count: f32) -> Timing {
        self.repeat_count = if count.is_nan() { 0.0 } else { count.max(0.0) };
        self
    }
    /// Plays each cycle forwards then backwards (`autoreverses`).
    pub fn autoreverses(mut self, on: bool) -> Timing {
        self.autoreverses = on;
        self
    }
    /// The media timing of this transaction, or `None` when it is plain (start now, normal speed, one cycle).
    fn media(&self) -> Option<Media> {
        let plain = self.delay == 0.0 && self.speed == 1.0 && self.repeat_count == 0.0 && !self.autoreverses;
        (!plain).then(|| Media {
            begin_time: self.delay as f64,
            speed: self.speed,
            repeat_count: self.repeat_count,
            autoreverses: self.autoreverses,
            ..Media::default()
        })
    }
    fn animation(self, from: f32, to: f32) -> Animation {
        match self.curve {
            Curve::Spring { damping_ratio, response } if self.duration < 0.0 => {
                let mut anim = Animation::spring(from, to, 0.0, damping_ratio, response);
                anim.velocity = self.initial_velocity;
                anim
            }
            c => Animation::new(from, to, 0.0, self.duration, c),
        }
    }
    fn instant(self) -> bool {
        self.duration == 0.0 && self.delay == 0.0 && !matches!(self.curve, Curve::Spring { .. })
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
    /// delay, speed, repeat and autoreverse; `None` plays `anim` once from the first tick
    media: Option<Media>,
}

impl Track {
    /// The clock to read `anim` at for window time `t`, and whether the track has ended.
    fn local(&self, t: f64) -> (f64, bool) {
        let Some(m) = self.media else { return (t, self.anim.is_done(t)) };
        let d = self.anim.duration as f64;
        match m.phase(t - self.anim.start, d) {
            Phase::Before => (self.anim.start, false),
            Phase::Active { progress, .. } => (self.anim.start + progress as f64 * d, false),
            Phase::After { .. } => (self.anim.start + d, true),
        }
    }
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
    links: crate::frame_clock::FrameLinks,
    /// key-path animations and the transaction stack (see `keyanim`)
    pub(super) keyed: super::keyanim::Keyed,
    /// the clock of the last tick, seconds
    pub(super) clock: f64,
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
    /// Velocity (value units per second) of the running track of `prop` on `id`, zero when there is none.
    fn velocity_of(&self, id: u64, prop: Prop) -> f32 {
        self.tracks.iter().find(|t| t.id == id && t.prop == prop && t.started).map_or(0.0, |t| t.anim.velocity_at(self.clock))
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
        self.anim.keyed.open_scope(tx.as_ref().map(|t| t.timing));
        f(self);
        self.anim.keyed.tx.commit();
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
            let carried = self.anim.velocity_of(id, p);
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
            let media = t.media();
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
                    media,
                }
            } else {
                let mut anim = t.animation(b.get(p), a.get(p));
                // a retargeted spring keeps the speed of the one it replaces
                let distance = a.get(p) - b.get(p);
                if matches!(anim.curve, Curve::Spring { .. }) && distance.abs() > 1e-6 && t.initial_velocity == 0.0 {
                    anim.velocity = carried / distance;
                }
                Track { id, prop: p, anim, started: false, shown: b.get(p), colors: None, shown_color: None, group, media }
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
        self.anim.tracks.iter().any(|t| id.map_or(true, |i| t.id == i)) || self.anim.keyed.animating(id)
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
        let anim = (!self.anim.tracks.is_empty() || !self.anim.ready.is_empty() || self.anim.keyed.active() || super::any_wants_tick(self.root.as_ref())).then_some(FRAME);
        let link = self.anim.links.wake_interval();
        match (anim, link) {
            (None, None) => None,
            (a, l) => Some(t + a.unwrap_or(f64::MAX).min(l.unwrap_or(f64::MAX))),
        }
    }

    /// Attaches a per-frame callback to this window; it runs from [`Window::tick`] at its preferred rate.
    pub fn add_frame_link(&mut self, link: crate::frame_clock::FrameLink) -> crate::frame_clock::LinkId {
        self.anim.links.add(link)
    }
    pub fn remove_frame_link(&mut self, id: crate::frame_clock::LinkId) -> Option<crate::frame_clock::FrameLink> {
        self.anim.links.remove(id)
    }
    pub fn frame_link_mut(&mut self, id: crate::frame_clock::LinkId) -> Option<&mut crate::frame_clock::FrameLink> {
        self.anim.links.get_mut(id)
    }
    /// The refresh rate of the display the window is on, which frame-rate ranges resolve against (default 60).
    pub fn set_display_refresh(&mut self, hz: f32) {
        if hz.is_finite() && hz > 0.0 {
            self.anim.links.display_hz = hz;
        }
    }
    /// Runs the frame links due at `t`; true when any ran.
    pub(super) fn fire_frame_links(&mut self, t: f64) -> bool {
        self.anim.links.fire(t) > 0
    }

    /// Advances the tracks to `t` (seconds). Called by [`Window::tick`].
    pub(super) fn advance_animations(&mut self, t: f64) {
        self.anim.clock = t;
        self.advance_key_animations(t);
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
                let (at, fin) = tr.local(t);
                if tr.prop == Prop::Background {
                    let (a, b, end) = tr.colors.unwrap_or((0, 0, None));
                    let c = if fin { end } else { Some(mix_color(a, b, tr.anim.value(at))) };
                    base.background = c;
                    tr.shown_color = c;
                } else {
                    let val = if fin { tr.anim.to } else { tr.anim.value(at) };
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
    fn a_spring_with_initial_velocity_moves_ahead_of_one_from_rest() {
        let (mut rest, rest_id) = win();
        rest.animate(Timing::spring(0.6, 0.5), |w| w.view_mut::<Container>(rest_id).unwrap().set_frame(rect(100.0, 0.0, 100.0, 50.0)));
        let (mut fling, fling_id) = win();
        let pushed = Timing::spring_with_velocity(0.6, 0.5, 4.0);
        assert_eq!(pushed.initial_velocity, 4.0);
        assert_eq!(Timing::spring(0.6, 0.5).initial_velocity, 0.0);
        fling.animate(pushed, |w| w.view_mut::<Container>(fling_id).unwrap().set_frame(rect(100.0, 0.0, 100.0, 50.0)));
        rest.tick(0.0);
        fling.tick(0.0);
        rest.tick(0.05);
        fling.tick(0.05);
        assert!(frame(&fling, fling_id).x > frame(&rest, rest_id).x, "a fling of four distances a second starts ahead");
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

    #[test]
    fn a_retargeted_spring_keeps_its_speed() {
        let (mut w, id) = win();
        let sp = Timing::spring(0.6, 0.5);
        w.animate(sp, |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(100.0, 0.0, 100.0, 50.0)));
        w.tick(0.0);
        w.tick(0.1);
        let x1 = frame(&w, id).x;
        w.tick(0.11);
        let x2 = frame(&w, id).x;
        let v = (x2 - x1) / 0.01;
        assert!(v > 0.0);
        w.animate(sp, |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(300.0, 0.0, 100.0, 50.0)));
        w.tick(0.11);
        assert!((frame(&w, id).x - x2).abs() < 1e-3, "no jump");
        w.tick(0.12);
        let v2 = (frame(&w, id).x - x2) / 0.01;
        assert!((v2 - v).abs() / v < 0.3, "{v} vs {v2}");
    }

    // Headless: the tests below read model values or draw into an in-memory Surface. No display, device or clock is used.
    fn test_ui() -> crate::kit::Ui {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).expect("SFUI.ttf").into_boxed_slice());
        crate::kit::Ui::new(crate::kit::Font::parse(data).expect("font"), 1.0)
    }

    /// A 4 x 4 window whose only child is a green 2 x 4 swatch at the origin.
    fn swatch() -> (Window, u64) {
        let mut w: Window = Window::new(Container::new(vec![Box::new(Container::default())]), 10);
        let id = w.root().children()[0].id();
        w.root_mut().set_frame(rect(0.0, 0.0, 4.0, 4.0));
        let child = &mut w.root_mut().children_mut()[0];
        child.set_frame(rect(0.0, 0.0, 2.0, 4.0));
        child.base_mut().background = Some(0xff00_ff00);
        w.set_reduce_motion(Some(false));
        (w, id)
    }

    #[test]
    fn headless_swatch_is_drawn_where_the_animation_has_moved_it() {
        let ui = test_ui();
        let (mut w, id) = swatch();
        w.animate(lin(1.0), |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(2.0, 0.0, 2.0, 4.0)));
        w.tick(0.0);
        w.tick(0.5);
        let mut px = vec![0xff00_0000u32; 16];
        let mut s = Surface { w: 4, h: 4, stride: 4, px: &mut px, y0: 0 };
        w.draw(&mut s, &ui);
        assert_eq!(s.px[1], 0xff00_ff00, "the swatch covers columns 1 and 2 at the midpoint");
        assert_eq!(s.px[2], 0xff00_ff00);
        assert_ne!(s.px[0], 0xff00_ff00);
        assert_ne!(s.px[3], 0xff00_ff00);
    }

    #[test]
    fn headless_swatch_settles_on_the_new_frame_when_the_animation_ends() {
        let ui = test_ui();
        let (mut w, id) = swatch();
        w.animate(lin(1.0), |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(2.0, 0.0, 2.0, 4.0)));
        w.tick(0.0);
        w.tick(1.0);
        let mut px = vec![0xff00_0000u32; 16];
        let mut s = Surface { w: 4, h: 4, stride: 4, px: &mut px, y0: 0 };
        w.draw(&mut s, &ui);
        assert_eq!(s.px[2], 0xff00_ff00);
        assert_eq!(s.px[3], 0xff00_ff00);
        assert_ne!(s.px[1], 0xff00_ff00, "the old column is no longer covered");
    }

    #[test]
    fn headless_frame_opacity_and_size_interpolate_together() {
        let (mut w, id) = win();
        w.animate(lin(1.0), |w| {
            let v = w.view_mut::<Container>(id).unwrap();
            v.set_frame(rect(100.0, 40.0, 200.0, 90.0));
            v.base_mut().opacity = 0.0;
        });
        w.tick(0.0);
        w.tick(0.5);
        let b = w.view::<Container>(id).unwrap().base();
        assert_eq!(b.frame, rect(50.0, 20.0, 150.0, 70.0));
        assert!((b.opacity - 0.5).abs() < 1e-4);
    }

    #[test]
    fn headless_underdamped_spring_passes_its_target_then_rests_on_it() {
        let (mut w, id) = win();
        w.animate(Timing::spring(0.3, 0.4), |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(100.0, 0.0, 100.0, 50.0)));
        let mut peak = f32::MIN;
        for i in 0..=400 {
            w.tick(i as f64 * 0.01);
            peak = peak.max(frame(&w, id).x);
        }
        assert!(peak > 110.0, "an underdamped spring passes its target: {peak}");
        assert_eq!(frame(&w, id).x, 100.0);
        assert!(!w.is_animating(None));
    }

    #[test]
    fn headless_group_completes_once_after_all_of_its_tracks() {
        use std::{cell::Cell, rc::Rc};
        let (mut w, id) = win();
        let n = Rc::new(Cell::new(0));
        let c = n.clone();
        let tx = Transaction::new(lin(1.0)).completion(move |ok| {
            assert!(ok);
            c.set(c.get() + 1);
        });
        w.transaction(tx, |w| {
            let v = w.view_mut::<Container>(id).unwrap();
            v.set_frame(rect(100.0, 0.0, 100.0, 50.0));
            v.base_mut().opacity = 0.0;
        });
        w.tick(0.0);
        w.tick(0.5);
        assert_eq!(n.get(), 0, "still running at the midpoint");
        w.tick(1.0);
        assert_eq!(n.get(), 1);
        w.tick(2.0);
        assert_eq!(n.get(), 1, "the completion runs once");
    }

    #[test]
    fn a_physical_spring_matches_its_damping_ratio_and_response() {
        // the defaults: omega = 10 rad/s, zeta = 10 / (2 * 10) = 0.5
        let t = Timing::spring_physical(SPRING_MASS, SPRING_STIFFNESS, SPRING_DAMPING, 0.0);
        let Curve::Spring { damping_ratio, response } = t.curve else { panic!("a spring") };
        assert!((damping_ratio - 0.5).abs() < 1e-5);
        assert!((response - std::f32::consts::TAU / 10.0).abs() < 1e-5);
        assert!(t.duration < 0.0, "lasts until it settles");
        // four times the mass halves the frequency and halves the damping ratio
        let heavy = Timing::spring_physical(4.0, 100.0, 10.0, 0.0);
        let Curve::Spring { damping_ratio: z4, response: r4 } = heavy.curve else { panic!("a spring") };
        assert!((r4 - 2.0 * response).abs() < 1e-4 && (z4 - 0.25).abs() < 1e-5);
        assert!(heavy.settling_duration() > t.settling_duration(), "a heavier, less damped spring settles later");
        // nonsense falls back to the defaults
        assert_eq!(Timing::spring_physical(0.0, -1.0, f32::NAN, f32::INFINITY), t);
    }

    #[test]
    fn a_heavier_spring_lags_behind_a_light_one() {
        let (mut light, a) = win();
        let (mut heavy, b) = win();
        light.animate(Timing::spring_physical(1.0, 100.0, 20.0, 0.0), |w| w.view_mut::<Container>(a).unwrap().set_frame(rect(100.0, 0.0, 100.0, 50.0)));
        heavy.animate(Timing::spring_physical(5.0, 100.0, 20.0, 0.0), |w| w.view_mut::<Container>(b).unwrap().set_frame(rect(100.0, 0.0, 100.0, 50.0)));
        light.tick(0.0);
        heavy.tick(0.0);
        light.tick(0.1);
        heavy.tick(0.1);
        assert!(frame(&heavy, b).x < frame(&light, a).x);
        light.tick(120.0);
        heavy.tick(120.0);
        assert_eq!((frame(&light, a).x, frame(&heavy, b).x), (100.0, 100.0));
    }

    #[test]
    fn perceptual_springs_map_bounce_to_damping() {
        let ratio = |t: Timing| match t.curve {
            Curve::Spring { damping_ratio, response } => (damping_ratio, response),
            _ => panic!("a spring"),
        };
        assert_eq!(ratio(Timing::spring_perceptual(0.5, 0.0)), (1.0, 0.5));
        let (z, _) = ratio(Timing::spring_perceptual(0.5, 0.3));
        assert!((z - 0.7).abs() < 1e-6);
        let (z, _) = ratio(Timing::spring_perceptual(0.5, -0.5));
        assert!((z - 2.0).abs() < 1e-6);
        assert_eq!(Timing::new(0.4, Curve::Linear).settling_duration(), 0.4);
    }

    #[test]
    fn a_delay_holds_the_old_value_then_plays() {
        let (mut w, id) = win();
        w.animate(lin(1.0).delay(0.5), |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(100.0, 0.0, 100.0, 50.0)));
        w.tick(0.0);
        w.tick(0.4);
        assert_eq!(frame(&w, id).x, 0.0, "still waiting");
        w.tick(1.0);
        assert!((frame(&w, id).x - 50.0).abs() < 1e-3);
        w.tick(1.5);
        assert_eq!(frame(&w, id).x, 100.0);
        assert!(!w.is_animating(None));
    }

    #[test]
    fn autoreverse_and_repeat_play_back_and_forth_then_rest_on_the_new_value() {
        let (mut w, id) = win();
        w.animate(lin(1.0).autoreverses(true).repeat_count(2.0), |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(100.0, 0.0, 100.0, 50.0)));
        w.tick(0.0);
        w.tick(0.5);
        assert!((frame(&w, id).x - 50.0).abs() < 1e-3);
        w.tick(1.5);
        assert!((frame(&w, id).x - 50.0).abs() < 1e-3, "on the way back");
        w.tick(1.9);
        assert!((frame(&w, id).x - 10.0).abs() < 1e-3);
        w.tick(2.5);
        assert!((frame(&w, id).x - 50.0).abs() < 1e-3, "the second cycle");
        assert!(w.is_animating(Some(id)));
        w.tick(4.0);
        assert_eq!(frame(&w, id).x, 100.0, "the model value once the cycles are over");
        assert!(!w.is_animating(None));
    }

    #[test]
    fn speed_scales_the_clock_and_an_endless_repeat_runs_until_replaced() {
        let (mut w, id) = win();
        w.animate(lin(1.0).speed(2.0), |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(100.0, 0.0, 100.0, 50.0)));
        w.tick(0.0);
        w.tick(0.25);
        assert!((frame(&w, id).x - 50.0).abs() < 1e-3);
        w.tick(0.5);
        assert_eq!(frame(&w, id).x, 100.0);
        w.animate(lin(1.0).repeat_count(f32::INFINITY), |w| w.view_mut::<Container>(id).unwrap().set_frame(rect(0.0, 0.0, 100.0, 50.0)));
        w.tick(1.0);
        w.tick(100.25);
        assert!((frame(&w, id).x - 75.0).abs() < 1e-2, "{}", frame(&w, id).x);
        assert!(w.is_animating(Some(id)));
        w.disable_actions(|w| w.view_mut::<Container>(id).unwrap().set_frame(rect(5.0, 0.0, 100.0, 50.0)));
        w.tick(100.5);
        assert_eq!(frame(&w, id).x, 5.0);
        assert!(!w.is_animating(None));
    }

    #[test]
    fn plain_timing_has_no_media_and_reduce_motion_drops_it() {
        assert!(Timing::DEFAULT.media().is_none());
        assert!(lin(1.0).delay(0.1).media().is_some());
        assert_eq!(lin(1.0).speed(f32::NAN).speed, 1.0);
        let (mut w, id) = win();
        w.set_reduce_motion(Some(true));
        w.animate(lin(1.0).repeat_count(f32::INFINITY), |w| w.view_mut::<Container>(id).unwrap().base_mut().opacity = 0.0);
        w.tick(0.0);
        w.tick(1.0);
        assert!(!w.is_animating(None), "under Reduce Motion the cross-fade plays once");
    }

}
