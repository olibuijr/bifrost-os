//! Animations: basic (from/to), keyframe, spring and group, with media timing (begin time, duration, speed, time
//! offset, repeat count, autoreverse, fill mode). Evaluated by `Tree::presentation`.
//!
//! Timeline rules (the reference platform's media timing):
//! - local time is `(t - begin) * speed + time_offset`; before `begin` the animation is not active;
//! - one cycle lasts `duration`, or `2 * duration` with autoreverse (forward leg, then back); `repeat_count` counts
//!   cycles (0 plays once, a fraction plays a partial last cycle, infinity repeats forever);
//! - after the active time the fill mode decides whether the end value stays (`Forwards`, `Both`) or the model shows
//!   again; before it, `Backwards` and `Both` show the start value;
//! - a negative speed plays backwards: it starts at `time_offset`, ends at local time 0, and is not active while
//!   local time is above its active duration.
//!
//! Springs take their duration from the physics (`Spring::settling_duration`), not from `MediaTiming::duration`;
//! their `repeat_count` and `autoreverses` are ignored. A group's `speed` and `time_offset` scale the time its
//! children see; its duration, repeat and fill are not applied (each child carries its own).
//!
//! Owner: wave G1 agent `layers-animation`.

use crate::geometry::{Affine, Color, Point, Rect};
use crate::layer::{Layer, Property, Shadow};
use crate::timing::TimingFunction;

/// An animation's id within a tree; reported back in `animation_finished`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AnimationId(pub u32);

/// An animatable value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Scalar(f32),
    Point(Point),
    Rect(Rect),
    Transform(Affine),
    Color(Color),
    Shadow(Shadow),
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    // Written so that t = 0 gives `a` and t = 1 gives `b` exactly.
    a * (1.0 - t) + b * t
}

fn mix_color(a: Color, b: Color, t: f32) -> Color {
    Color { r: mix(a.r, b.r, t), g: mix(a.g, b.g, t), b: mix(a.b, b.b, t), a: mix(a.a, b.a, t) }
}

impl Value {
    /// Linear interpolation between two values of the same kind (`t` may be outside 0..1 for springs and
    /// overshooting curves). Mismatched kinds return `self` until `t >= 1`, then `other`.
    pub fn lerp(&self, other: &Value, t: f32) -> Value {
        match (*self, *other) {
            (Value::Scalar(a), Value::Scalar(b)) => Value::Scalar(mix(a, b, t)),
            (Value::Point(a), Value::Point(b)) => Value::Point(Point { x: mix(a.x, b.x, t), y: mix(a.y, b.y, t) }),
            (Value::Rect(a), Value::Rect(b)) => {
                Value::Rect(Rect::new(mix(a.x, b.x, t), mix(a.y, b.y, t), mix(a.w, b.w, t), mix(a.h, b.h, t)))
            }
            // Component-wise: not a decomposed rotation, which is acceptable for the transforms apps animate.
            (Value::Transform(a), Value::Transform(b)) => Value::Transform(Affine {
                a: mix(a.a, b.a, t),
                b: mix(a.b, b.b, t),
                c: mix(a.c, b.c, t),
                d: mix(a.d, b.d, t),
                tx: mix(a.tx, b.tx, t),
                ty: mix(a.ty, b.ty, t),
            }),
            (Value::Color(a), Value::Color(b)) => Value::Color(mix_color(a, b, t)),
            (Value::Shadow(a), Value::Shadow(b)) => Value::Shadow(Shadow {
                color: mix_color(a.color, b.color, t),
                opacity: mix(a.opacity, b.opacity, t),
                radius: mix(a.radius, b.radius, t),
                offset: Point { x: mix(a.offset.x, b.offset.x, t), y: mix(a.offset.y, b.offset.y, t) },
            }),
            _ => {
                if t >= 1.0 {
                    *other
                } else {
                    *self
                }
            }
        }
    }

    /// Component-wise sum for additive animations; mismatched kinds keep `self`.
    pub(crate) fn added(&self, other: &Value) -> Value {
        match (*self, *other) {
            (Value::Scalar(a), Value::Scalar(b)) => Value::Scalar(a + b),
            (Value::Point(a), Value::Point(b)) => Value::Point(Point { x: a.x + b.x, y: a.y + b.y }),
            (Value::Rect(a), Value::Rect(b)) => Value::Rect(Rect::new(a.x + b.x, a.y + b.y, a.w + b.w, a.h + b.h)),
            (Value::Transform(a), Value::Transform(b)) => Value::Transform(Affine {
                a: a.a + b.a,
                b: a.b + b.b,
                c: a.c + b.c,
                d: a.d + b.d,
                tx: a.tx + b.tx,
                ty: a.ty + b.ty,
            }),
            (Value::Color(a), Value::Color(b)) => {
                Value::Color(Color { r: a.r + b.r, g: a.g + b.g, b: a.b + b.b, a: a.a + b.a })
            }
            (Value::Shadow(a), Value::Shadow(b)) => Value::Shadow(Shadow {
                color: Color { r: a.color.r + b.color.r, g: a.color.g + b.color.g, b: a.color.b + b.color.b, a: a.color.a + b.color.a },
                opacity: a.opacity + b.opacity,
                radius: a.radius + b.radius,
                offset: Point { x: a.offset.x + b.offset.x, y: a.offset.y + b.offset.y },
            }),
            _ => *self,
        }
    }
}

/// What the animation shows outside its active time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillMode {
    /// The model value before and after (the default: the animation is removed when done).
    #[default]
    Removed,
    Forwards,
    Backwards,
    Both,
}

impl FillMode {
    fn forwards(self) -> bool {
        matches!(self, FillMode::Forwards | FillMode::Both)
    }

    fn backwards(self) -> bool {
        matches!(self, FillMode::Backwards | FillMode::Both)
    }
}

/// Media timing, as the reference platform's timing protocol defines it. Times are seconds on the tree's clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaTiming {
    /// Absolute start on the tree clock; `None` means "the frame the transaction is applied".
    pub begin_time: Option<f64>,
    pub duration: f64,
    pub speed: f32,
    pub time_offset: f64,
    /// 0 plays once; `f32::INFINITY` repeats forever; fractions play a partial last cycle.
    pub repeat_count: f32,
    pub autoreverses: bool,
    pub fill_mode: FillMode,
}

impl Default for MediaTiming {
    fn default() -> Self {
        MediaTiming { begin_time: None, duration: 0.25, speed: 1.0, time_offset: 0.0, repeat_count: 0.0, autoreverses: false, fill_mode: FillMode::Removed }
    }
}

/// Where an animation is on its timeline at some tree time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Phase {
    /// Not started (or not started yet, for a negative speed).
    Before,
    /// Inside the active time; the payload is the local time in `0..total`.
    Active(f64),
    /// Past the active time; the payload is the local time it ended at (`total`, or 0 for a negative speed).
    After(f64),
}

impl MediaTiming {
    /// Seconds of one cycle: `duration`, or twice that with autoreverse. Zero for a non-positive duration.
    fn cycle(&self) -> f64 {
        if !(self.duration > 0.0) {
            return 0.0;
        }
        if self.autoreverses {
            2.0 * self.duration
        } else {
            self.duration
        }
    }

    /// The number of cycles: `repeat_count`, or one when it is zero, negative or NaN.
    fn cycles(&self) -> f64 {
        if self.repeat_count > 0.0 {
            self.repeat_count as f64
        } else {
            1.0
        }
    }

    /// The active duration in seconds (infinite for an infinite repeat count).
    pub(crate) fn active_duration(&self) -> f64 {
        let cycle = self.cycle();
        if cycle <= 0.0 {
            return 0.0;
        }
        self.cycles() * cycle
    }

    /// The phase at tree time `t` for an animation that began at `begin` and lasts `total` seconds of local time.
    pub(crate) fn phase_with(&self, begin: f64, t: f64, total: f64) -> Phase {
        if !(t >= begin) {
            return Phase::Before;
        }
        let local = (t - begin) * self.speed as f64 + self.time_offset;
        if self.speed >= 0.0 {
            if local < 0.0 {
                Phase::Before
            } else if local >= total {
                Phase::After(total)
            } else {
                Phase::Active(local)
            }
        } else if local > total {
            Phase::Before
        } else if local <= 0.0 {
            Phase::After(0.0)
        } else {
            Phase::Active(local)
        }
    }

    /// The linear progress (0..1) within the cycle at local time `local`; odd legs run backwards with autoreverse.
    fn cycle_progress(&self, local: f64) -> f32 {
        let cycle = self.cycle();
        if cycle <= 0.0 {
            return 1.0;
        }
        let c = local.rem_euclid(cycle);
        let dur = self.duration;
        let p = if self.autoreverses && c >= dur { 1.0 - (c - dur) / dur } else { c / dur };
        p.clamp(0.0, 1.0) as f32
    }

    /// The progress an animation shows after its active time `total`: the end of the last cycle.
    fn end_progress(&self, total: f64) -> f32 {
        let cycle = self.cycle();
        if cycle <= 0.0 || !total.is_finite() {
            return 1.0;
        }
        let whole = total / cycle;
        if whole.fract() == 0.0 {
            // Finished whole cycles: back at the start after an autoreverse leg, at the end otherwise.
            if self.autoreverses {
                0.0
            } else {
                1.0
            }
        } else {
            self.cycle_progress(total)
        }
    }

    /// The local progress (0..1 within one cycle, reversed on autoreverse legs) at tree time `t` for an animation
    /// that began at `begin`, or `None` when it is not active and its fill mode shows nothing.
    pub fn progress(&self, begin: f64, t: f64) -> Option<f32> {
        let total = self.active_duration();
        match self.phase_with(begin, t, total) {
            Phase::Active(local) => Some(self.cycle_progress(local)),
            Phase::Before => self.fill_mode.backwards().then_some(0.0),
            Phase::After(_) => {
                if !self.fill_mode.forwards() {
                    None
                } else if self.speed < 0.0 {
                    Some(0.0)
                } else {
                    Some(self.end_progress(total))
                }
            }
        }
    }

    /// Whether the animation has ended at tree time `t` (for `animation_finished`).
    pub fn ended(&self, begin: f64, t: f64) -> bool {
        matches!(self.phase_with(begin, t, self.active_duration()), Phase::After(_))
    }
}

/// Keyframe values with their key times (0..1, same length as `values`) and per-segment timing functions.
#[derive(Debug, Clone, PartialEq)]
pub struct Keyframes {
    pub values: Vec<Value>,
    pub key_times: Vec<f32>,
    pub timing_functions: Vec<TimingFunction>,
}

/// The value of a keyframe track at linear progress `p`. Key times that do not match `values` are spaced evenly;
/// a missing segment timing function is linear.
pub(crate) fn keyframe_value(kf: &Keyframes, p: f32) -> Option<Value> {
    let n = kf.values.len();
    if n == 0 {
        return None;
    }
    if n == 1 {
        return Some(kf.values[0]);
    }
    let times: Vec<f32> = if kf.key_times.len() == n {
        kf.key_times.clone()
    } else {
        (0..n).map(|i| i as f32 / (n - 1) as f32).collect()
    };
    if p <= times[0] {
        return Some(kf.values[0]);
    }
    if p >= times[n - 1] {
        return Some(kf.values[n - 1]);
    }
    for i in 0..n - 1 {
        if p < times[i + 1] {
            let span = times[i + 1] - times[i];
            let f = if span > 0.0 { (p - times[i]) / span } else { 1.0 };
            let curve = kf.timing_functions.get(i).copied().unwrap_or(TimingFunction::Linear);
            return Some(kf.values[i].lerp(&kf.values[i + 1], curve.eval(f)));
        }
    }
    Some(kf.values[n - 1])
}

/// A damped spring from `from` to `to` (duration comes from the physics, not `MediaTiming::duration`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    pub mass: f32,
    pub stiffness: f32,
    pub damping: f32,
    pub initial_velocity: f32,
}

impl Spring {
    /// The parameters as f64 when the spring is physical (positive mass and stiffness, finite inputs).
    fn params(&self) -> Option<(f64, f64, f64, f64)> {
        let v = (self.mass as f64, self.stiffness as f64, self.damping as f64, self.initial_velocity as f64);
        if [v.0, v.1, v.2, v.3].iter().all(|x| x.is_finite()) && v.0 > 0.0 && v.1 > 0.0 {
            Some(v)
        } else {
            None
        }
    }

    /// The displacement from the target, `progress - 1`, at `t` seconds (f64). Starts at -1 with the initial
    /// velocity; the physics is m x'' + c x' + k x = 0.
    fn displacement(&self, t: f64) -> f64 {
        let Some((m, k, c, v0)) = self.params() else {
            return if t > 0.0 { 0.0 } else { -1.0 };
        };
        let w0 = (k / m).sqrt();
        let zeta = c / (2.0 * (k * m).sqrt());
        let x0 = -1.0;
        if (zeta - 1.0).abs() < 1e-9 {
            (-w0 * t).exp() * (x0 + (v0 + w0 * x0) * t)
        } else if zeta < 1.0 {
            let wd = w0 * (1.0 - zeta * zeta).sqrt();
            let s = zeta * w0;
            let b = (v0 + s * x0) / wd;
            (-s * t).exp() * (x0 * (wd * t).cos() + b * (wd * t).sin())
        } else {
            let sq = (zeta * zeta - 1.0).sqrt();
            let r1 = w0 * (-zeta + sq);
            let r2 = w0 * (-zeta - sq);
            let c1 = (v0 - r2 * x0) / (r1 - r2);
            let c2 = x0 - c1;
            c1 * (r1 * t).exp() + c2 * (r2 * t).exp()
        }
    }

    /// The progress (0 at start, settling at 1) after `t` seconds.
    pub fn progress(&self, t: f32) -> f32 {
        if !(t > 0.0) {
            return 0.0;
        }
        (1.0 + self.displacement(t as f64)) as f32
    }

    /// The time after which the spring stays within 0.1% of its target. Infinite for an undamped or
    /// non-decaying spring; 0 for a degenerate spring (no mass or no stiffness), which jumps to the target.
    ///
    /// Found by bounding the envelope of the response to find a horizon, then scanning it for the last sample
    /// outside the band and bisecting that exit to 60 steps.
    pub fn settling_duration(&self) -> f32 {
        const BAND: f64 = 1e-3;
        let Some((m, k, c, v0)) = self.params() else {
            return 0.0;
        };
        let w0 = (k / m).sqrt();
        let zeta = c / (2.0 * (k * m).sqrt());
        // (decay rate, envelope bound on |x(t)|, oscillation period for the sampling density)
        let (rate, bound, period): (f64, Box<dyn Fn(f64) -> f64>, f64) = if (zeta - 1.0).abs() < 1e-9 {
            let d = (v0 - w0).abs();
            (w0, Box::new(move |t: f64| (1.0 + d * t) * (-w0 * t).exp()), f64::INFINITY)
        } else if zeta < 1.0 {
            let s = zeta * w0;
            let wd = w0 * (1.0 - zeta * zeta).sqrt();
            let amp = (1.0 + ((v0 - s) / wd).powi(2)).sqrt();
            (s, Box::new(move |t: f64| amp * (-s * t).exp()), 2.0 * std::f64::consts::PI / wd)
        } else {
            let sq = (zeta * zeta - 1.0).sqrt();
            let r1 = w0 * (-zeta + sq);
            let r2 = w0 * (-zeta - sq);
            let c1 = (v0 + r2) / (r1 - r2);
            let c2 = -1.0 - c1;
            let amp = c1.abs() + c2.abs();
            (-r1, Box::new(move |t: f64| amp * (r1 * t).exp()), f64::INFINITY)
        };
        if !(rate > 0.0) || !rate.is_finite() {
            return f32::INFINITY;
        }
        // Horizon: double from one time constant until the envelope is inside the band.
        let mut horizon = 1.0 / rate;
        let mut doublings = 0;
        while bound(horizon) > BAND {
            horizon *= 2.0;
            doublings += 1;
            if doublings > 64 {
                return f32::INFINITY;
            }
        }
        let periods = if period.is_finite() { horizon / period } else { 0.0 };
        let n = (4096.0 + 64.0 * periods).min(2_000_000.0) as usize;
        let at = |i: usize| horizon * i as f64 / n as f64;
        let out = |t: f64| self.displacement(t).abs() > BAND;
        let last_out = (0..=n).rev().find(|&i| out(at(i)));
        match last_out {
            None => 0.0,
            Some(i) if i == n => horizon as f32,
            Some(i) => {
                let (mut lo, mut hi) = (at(i), at(i + 1));
                for _ in 0..60 {
                    let mid = 0.5 * (lo + hi);
                    if out(mid) {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                hi as f32
            }
        }
    }
}

/// An animation attached to a layer.
#[derive(Debug, Clone, PartialEq)]
pub enum Animation {
    Basic { id: AnimationId, key: Property, from: Option<Value>, to: Option<Value>, timing: MediaTiming, function: TimingFunction, additive: bool },
    Keyframe { id: AnimationId, key: Property, keyframes: Keyframes, timing: MediaTiming },
    Spring { id: AnimationId, key: Property, from: Option<Value>, to: Option<Value>, spring: Spring, timing: MediaTiming },
    Group { id: AnimationId, animations: Vec<Animation>, timing: MediaTiming },
}

impl Animation {
    pub fn id(&self) -> AnimationId {
        match self {
            Animation::Basic { id, .. } | Animation::Keyframe { id, .. } | Animation::Spring { id, .. } | Animation::Group { id, .. } => *id,
        }
    }

    pub fn timing(&self) -> &MediaTiming {
        match self {
            Animation::Basic { timing, .. } | Animation::Keyframe { timing, .. } | Animation::Spring { timing, .. } | Animation::Group { timing, .. } => timing,
        }
    }
}

/// The model value of an animatable property, when it has a `Value` kind (one implementation: `Layer::value`).
pub(crate) fn model_value(layer: &Layer, key: Property) -> Option<Value> {
    layer.value(key)
}

/// Writes a value into a property. Returns false when the kind does not match the property.
pub(crate) fn set_value(layer: &mut Layer, key: Property, value: Value) -> bool {
    layer.set_value(key, value)
}

/// One value an animation produces at a time: the property, the value, and whether it adds to the model value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Contribution {
    pub key: Property,
    pub value: Value,
    pub additive: bool,
}

/// Applies a contribution to a layer (added to the layer's current value when additive). Returns false when the
/// property cannot take the value.
pub(crate) fn write(layer: &mut Layer, c: &Contribution) -> bool {
    let value = if c.additive {
        match model_value(layer, c.key) {
            Some(base) => base.added(&c.value),
            None => return false,
        }
    } else {
        c.value
    };
    set_value(layer, c.key, value)
}

/// The fill mode that governs an animation's own value (a group's children carry their own).
pub(crate) fn fill_of(anim: &Animation) -> FillMode {
    match anim {
        Animation::Group { .. } => FillMode::Removed,
        other => other.timing().fill_mode,
    }
}

/// The active total of a spring: its settling duration (infinite when it never settles).
fn spring_total(spring: &Spring) -> f64 {
    let d = spring.settling_duration() as f64;
    if d.is_nan() {
        f64::INFINITY
    } else {
        d
    }
}

/// From and to values, with `None` taken from the model value of the layer.
fn endpoints(key: Property, from: Option<Value>, to: Option<Value>, model: &Layer) -> Option<(Value, Value)> {
    let base = model_value(model, key);
    Some((from.or(base)?, to.or(base)?))
}

/// The local tree time a group's children see at tree time `t`.
fn group_time(timing: &MediaTiming, begin: f64, t: f64) -> f64 {
    begin + (t - begin) * timing.speed as f64 + timing.time_offset
}

/// The values an animation produces at tree time `t`, given that it began at `begin` and acts on `model`.
/// Empty when the animation shows nothing (not started and no backwards fill, or ended and no forwards fill).
pub(crate) fn contributions(anim: &Animation, begin: f64, t: f64, model: &Layer) -> Vec<Contribution> {
    let mut out = Vec::new();
    match anim {
        Animation::Basic { key, from, to, timing, function, additive, .. } => {
            if let (Some((a, b)), Some(p)) = (endpoints(*key, *from, *to, model), timing.progress(begin, t)) {
                out.push(Contribution { key: *key, value: a.lerp(&b, function.eval(p)), additive: *additive });
            }
        }
        Animation::Keyframe { key, keyframes, timing, .. } => {
            if let Some(p) = timing.progress(begin, t) {
                if let Some(value) = keyframe_value(keyframes, p) {
                    out.push(Contribution { key: *key, value, additive: false });
                }
            }
        }
        Animation::Spring { key, from, to, spring, timing, .. } => {
            if let Some((a, b)) = endpoints(*key, *from, *to, model) {
                let total = spring_total(spring);
                let progress = match timing.phase_with(begin, t, total) {
                    Phase::Active(local) => Some(spring.progress(local as f32)),
                    Phase::Before => timing.fill_mode.backwards().then_some(0.0),
                    Phase::After(_) => timing.fill_mode.forwards().then_some(1.0),
                };
                if let Some(s) = progress {
                    out.push(Contribution { key: *key, value: a.lerp(&b, s), additive: false });
                }
            }
        }
        Animation::Group { animations, timing, .. } => {
            let local = group_time(timing, begin, t);
            for child in animations {
                let child_begin = child.timing().begin_time.unwrap_or(begin);
                out.extend(contributions(child, child_begin, local, model));
            }
        }
    }
    out
}

/// Whether an animation has ended at tree time `t` (a group ended when all its children have).
pub(crate) fn ended_at(anim: &Animation, begin: f64, t: f64) -> bool {
    match anim {
        Animation::Basic { timing, .. } | Animation::Keyframe { timing, .. } => timing.ended(begin, t),
        Animation::Spring { spring, timing, .. } => {
            matches!(timing.phase_with(begin, t, spring_total(spring)), Phase::After(_))
        }
        Animation::Group { animations, timing, .. } => {
            if animations.is_empty() {
                return timing.ended(begin, t);
            }
            let local = group_time(timing, begin, t);
            animations.iter().all(|child| {
                let child_begin = child.timing().begin_time.unwrap_or(begin);
                ended_at(child, child_begin, local)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerId;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn basic(key: Property, from: f32, to: f32, timing: MediaTiming) -> Animation {
        Animation::Basic {
            id: AnimationId(1),
            key,
            from: Some(Value::Scalar(from)),
            to: Some(Value::Scalar(to)),
            timing,
            function: TimingFunction::Linear,
            additive: false,
        }
    }

    fn secs(duration: f64) -> MediaTiming {
        MediaTiming { duration, ..MediaTiming::default() }
    }

    fn scalar(c: &[Contribution]) -> f32 {
        match c[0].value {
            Value::Scalar(v) => v,
            other => panic!("not a scalar: {other:?}"),
        }
    }

    #[test]
    fn lerp_scalar_and_endpoints_are_exact() {
        let a = Value::Scalar(2.0);
        let b = Value::Scalar(10.0);
        assert_eq!(a.lerp(&b, 0.0), Value::Scalar(2.0));
        assert_eq!(a.lerp(&b, 0.5), Value::Scalar(6.0));
        assert_eq!(a.lerp(&b, 1.0), Value::Scalar(10.0));
        // Overshoot: 2 + 8 * 1.25 = 12
        assert_eq!(a.lerp(&b, 1.25), Value::Scalar(12.0));
    }

    #[test]
    fn lerp_point_rect_and_colour_componentwise() {
        let p = Value::Point(Point { x: 0.0, y: 10.0 }).lerp(&Value::Point(Point { x: 20.0, y: 0.0 }), 0.25);
        assert_eq!(p, Value::Point(Point { x: 5.0, y: 7.5 }));
        let r = Value::Rect(Rect::new(0.0, 0.0, 100.0, 50.0)).lerp(&Value::Rect(Rect::new(10.0, 0.0, 200.0, 50.0)), 0.5);
        assert_eq!(r, Value::Rect(Rect::new(5.0, 0.0, 150.0, 50.0)));
        let c = Value::Color(Color::BLACK).lerp(&Value::Color(Color::WHITE), 0.5);
        assert_eq!(c, Value::Color(Color { r: 0.5, g: 0.5, b: 0.5, a: 1.0 }));
    }

    #[test]
    fn lerp_transform_is_component_wise_and_shadow_lerps_fields() {
        let t = Value::Transform(Affine::IDENTITY).lerp(&Value::Transform(Affine::translate(10.0, 4.0)), 0.5);
        assert_eq!(t, Value::Transform(Affine { tx: 5.0, ty: 2.0, ..Affine::IDENTITY }));
        let s0 = Shadow { color: Color::BLACK, opacity: 0.0, radius: 0.0, offset: Point::default() };
        let s1 = Shadow { color: Color::BLACK, opacity: 0.5, radius: 8.0, offset: Point { x: 0.0, y: 4.0 } };
        match Value::Shadow(s0).lerp(&Value::Shadow(s1), 0.5) {
            Value::Shadow(s) => {
                assert!(close(s.opacity, 0.25) && close(s.radius, 4.0) && close(s.offset.y, 2.0));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn lerp_mismatched_kinds_steps_at_one() {
        let a = Value::Scalar(1.0);
        let b = Value::Point(Point { x: 2.0, y: 2.0 });
        assert_eq!(a.lerp(&b, 0.99), a);
        assert_eq!(a.lerp(&b, 1.0), b);
    }

    #[test]
    fn progress_linear_within_one_cycle_and_before_after() {
        let t = secs(1.0);
        assert_eq!(t.progress(10.0, 9.0), None, "before begin, removed fill");
        assert_eq!(t.progress(10.0, 10.25), Some(0.25));
        assert_eq!(t.progress(10.0, 11.5), None, "after, removed fill");
        assert!(t.ended(10.0, 11.0));
        assert!(!t.ended(10.0, 10.999));
    }

    #[test]
    fn fill_modes_hold_the_start_and_end_values() {
        let fwd = MediaTiming { fill_mode: FillMode::Forwards, ..secs(1.0) };
        assert_eq!(fwd.progress(0.0, 5.0), Some(1.0));
        assert_eq!(fwd.progress(0.0, -1.0), None);
        let back = MediaTiming { fill_mode: FillMode::Backwards, ..secs(1.0) };
        assert_eq!(back.progress(2.0, 1.0), Some(0.0));
        assert_eq!(back.progress(2.0, 9.0), None);
        let both = MediaTiming { fill_mode: FillMode::Both, ..secs(1.0) };
        assert_eq!(both.progress(2.0, 1.0), Some(0.0));
        assert_eq!(both.progress(2.0, 9.0), Some(1.0));
    }

    #[test]
    fn fractional_repeat_plays_a_partial_last_cycle() {
        // 1.5 cycles of 1 s: 0..1 then 0..0.5 of the second cycle.
        let t = MediaTiming { repeat_count: 1.5, fill_mode: FillMode::Forwards, ..secs(1.0) };
        assert_eq!(t.progress(0.0, 1.25), Some(0.25), "second cycle, 0.25 along");
        // After the 1.5 cycles the forwards fill holds the progress reached at the end of the partial cycle.
        assert_eq!(t.progress(0.0, 2.25), Some(0.5));
        assert_eq!(t.progress(0.0, 2.75), Some(0.5));
        assert!(!t.ended(0.0, 1.4));
        assert!(t.ended(0.0, 1.5));
    }

    #[test]
    fn infinite_repeat_never_ends_and_wraps() {
        let t = MediaTiming { repeat_count: f32::INFINITY, ..secs(0.5) };
        assert!(!t.ended(0.0, 1e6));
        let p = t.progress(0.0, 1000.25).unwrap();
        assert!(close(p, 0.5), "p={p}");
    }

    #[test]
    fn autoreverse_runs_back_on_the_second_leg() {
        // duration 1, autoreverse, once: forward 0..1 then back 1..0 over 2 s.
        let t = MediaTiming { autoreverses: true, ..secs(1.0) };
        assert_eq!(t.progress(0.0, 0.25), Some(0.25));
        assert_eq!(t.progress(0.0, 1.25), Some(0.75));
        assert!(close(t.progress(0.0, 1.9).unwrap(), 0.1));
        assert!(t.ended(0.0, 2.0));
        // Fill forwards after a whole autoreverse cycle shows the start again.
        let f = MediaTiming { fill_mode: FillMode::Forwards, ..t };
        assert_eq!(f.progress(0.0, 5.0), Some(0.0));
    }

    #[test]
    fn speed_and_time_offset_shift_the_local_clock() {
        // speed 2: local = (t - begin) * 2; a 1 s animation ends after 0.5 s of tree time.
        let fast = MediaTiming { speed: 2.0, ..secs(1.0) };
        assert_eq!(fast.progress(0.0, 0.25), Some(0.5));
        assert!(fast.ended(0.0, 0.5));
        // time_offset 0.5 starts the animation half way through.
        let offset = MediaTiming { time_offset: 0.5, ..secs(1.0) };
        assert_eq!(offset.progress(3.0, 3.0), Some(0.5));
        assert!(offset.ended(3.0, 3.5));
        // speed 0 freezes at time_offset.
        let frozen = MediaTiming { speed: 0.0, time_offset: 0.3, ..secs(1.0) };
        assert_eq!(frozen.progress(0.0, 50.0), Some(0.3));
        assert!(!frozen.ended(0.0, 50.0));
    }

    #[test]
    fn negative_speed_plays_backwards_to_zero() {
        let back = MediaTiming { speed: -1.0, time_offset: 1.0, ..secs(1.0) };
        // At begin the local time is 1 (the end); after 0.25 s it is 0.75.
        assert_eq!(back.progress(0.0, 0.25), Some(0.75));
        assert!(back.ended(0.0, 1.0));
    }

    #[test]
    fn zero_duration_is_instant() {
        let t = MediaTiming { duration: 0.0, ..MediaTiming::default() };
        assert!(t.ended(1.0, 1.0));
        assert_eq!(t.progress(1.0, 1.0), None);
    }

    #[test]
    fn spring_critical_matches_the_closed_form() {
        // m = 1, k = 100, c = 20: zeta = 1, w0 = 10, x0 = -1, v0 = 0 gives x = -e^{-10t} (1 + 10t).
        // progress(0.1) = 1 - e^{-1} * 2 = 0.264241.
        let s = Spring { mass: 1.0, stiffness: 100.0, damping: 20.0, initial_velocity: 0.0 };
        assert!(close(s.progress(0.1), 0.264241), "{}", s.progress(0.1));
        assert_eq!(s.progress(0.0), 0.0);
    }

    /// Integrates m x'' + c x' + k x = 0 with RK4 from x0 = -1, x'(0) = v0, as an oracle.
    fn integrate(s: &Spring, t_end: f64) -> f64 {
        let (m, k, c, v0) = (s.mass as f64, s.stiffness as f64, s.damping as f64, s.initial_velocity as f64);
        let f = |x: f64, v: f64| (v, (-c * v - k * x) / m);
        let (mut x, mut v) = (-1.0f64, v0);
        let dt = 1e-4;
        let steps = (t_end / dt).round() as usize;
        for _ in 0..steps {
            let (k1x, k1v) = f(x, v);
            let (k2x, k2v) = f(x + 0.5 * dt * k1x, v + 0.5 * dt * k1v);
            let (k3x, k3v) = f(x + 0.5 * dt * k2x, v + 0.5 * dt * k2v);
            let (k4x, k4v) = f(x + dt * k3x, v + dt * k3v);
            x += dt / 6.0 * (k1x + 2.0 * k2x + 2.0 * k3x + k4x);
            v += dt / 6.0 * (k1v + 2.0 * k2v + 2.0 * k3v + k4v);
        }
        x
    }

    #[test]
    fn spring_matches_numerical_integration_in_all_three_regimes() {
        // Under-damped (zeta 0.1), critical (zeta 1), over-damped (zeta 2), each with an initial velocity.
        let cases = [
            Spring { mass: 1.0, stiffness: 100.0, damping: 2.0, initial_velocity: 0.0 },
            Spring { mass: 1.0, stiffness: 100.0, damping: 20.0, initial_velocity: 3.0 },
            Spring { mass: 1.0, stiffness: 1.0, damping: 4.0, initial_velocity: 0.0 },
        ];
        for s in cases {
            for t in [0.05f32, 0.2, 0.5, 1.0] {
                let oracle = 1.0 + integrate(&s, t as f64) as f32;
                assert!((s.progress(t) - oracle).abs() < 2e-3, "{s:?} t={t}: {} vs {oracle}", s.progress(t));
            }
        }
    }

    #[test]
    fn spring_initial_velocity_moves_towards_the_target() {
        let s = Spring { mass: 1.0, stiffness: 100.0, damping: 20.0, initial_velocity: 5.0 };
        // Small t: progress ~ v0 * t (the initial slope is the initial velocity).
        let t = 1e-3f32;
        assert!((s.progress(t) / t - 5.0).abs() < 1e-2, "slope {}", s.progress(t) / t);
    }

    #[test]
    fn spring_settling_duration_is_the_last_exit_from_the_band() {
        // Critical: |x| = (1 + 10 t) e^{-10 t} is within 0.001 from about 0.92 s.
        let s = Spring { mass: 1.0, stiffness: 100.0, damping: 20.0, initial_velocity: 0.0 };
        let settle = s.settling_duration();
        assert!((0.85..1.0).contains(&settle), "{settle}");
        assert!(((s.progress(settle) - 1.0).abs()) <= 1e-3 + 1e-5);
        assert!(((s.progress(settle - 2e-3) - 1.0).abs()) > 1e-3);
    }

    #[test]
    fn spring_settling_for_an_under_damped_spring_stays_inside_after_it() {
        // zeta 0.1, w0 10: the envelope decays as e^{-t}, so the exit is near ln(1000) = 6.9 s.
        let s = Spring { mass: 1.0, stiffness: 100.0, damping: 2.0, initial_velocity: 0.0 };
        let settle = s.settling_duration();
        assert!((6.5..7.0).contains(&settle), "{settle}");
        for i in 0..50 {
            let t = settle + i as f32 * 0.05;
            assert!((s.progress(t) - 1.0).abs() <= 1e-3 + 1e-5, "t={t}");
        }
    }

    #[test]
    fn undamped_and_degenerate_springs() {
        let undamped = Spring { mass: 1.0, stiffness: 100.0, damping: 0.0, initial_velocity: 0.0 };
        assert_eq!(undamped.settling_duration(), f32::INFINITY);
        let massless = Spring { mass: 0.0, stiffness: 100.0, damping: 1.0, initial_velocity: 0.0 };
        assert_eq!(massless.settling_duration(), 0.0);
        assert_eq!(massless.progress(0.5), 1.0);
    }

    #[test]
    fn keyframes_interpolate_per_segment_with_their_timing() {
        let kf = Keyframes {
            values: vec![Value::Scalar(0.0), Value::Scalar(1.0), Value::Scalar(0.2)],
            key_times: vec![0.0, 0.5, 1.0],
            timing_functions: vec![TimingFunction::Linear, TimingFunction::Linear],
        };
        assert_eq!(keyframe_value(&kf, 0.25), Some(Value::Scalar(0.5)));
        match keyframe_value(&kf, 0.75) {
            Some(Value::Scalar(v)) => assert!(close(v, 0.6), "{v}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(keyframe_value(&kf, 1.0), Some(Value::Scalar(0.2)));
        // The second segment is an ease-in: at its half point the value is the curve's 0.5 mark, not 0.5 of the way.
        let eased = Keyframes { timing_functions: vec![TimingFunction::Linear, TimingFunction::EaseIn], ..kf.clone() };
        // Segment 2 runs from 1 to 0.2 over f = 0.5; the ease-in curve is below 0.5 there, so the value is above 0.6.
        let expected = 1.0 - 0.8 * TimingFunction::EaseIn.eval(0.5);
        match keyframe_value(&eased, 0.75) {
            Some(Value::Scalar(v)) => assert!(close(v, expected) && v > 0.6, "{v} vs {expected}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn keyframes_clamp_outside_and_space_evenly_without_key_times() {
        let kf = Keyframes { values: vec![Value::Scalar(0.0), Value::Scalar(10.0), Value::Scalar(20.0)], key_times: vec![], timing_functions: vec![] };
        assert_eq!(keyframe_value(&kf, 0.25), Some(Value::Scalar(5.0)));
        assert_eq!(keyframe_value(&kf, 0.75), Some(Value::Scalar(15.0)));
        let late = Keyframes { key_times: vec![0.2, 0.4, 0.6], ..kf.clone() };
        assert_eq!(keyframe_value(&late, 0.1), Some(Value::Scalar(0.0)), "before the first key");
        assert_eq!(keyframe_value(&late, 0.9), Some(Value::Scalar(20.0)), "after the last key");
        let empty = Keyframes { values: vec![], key_times: vec![], timing_functions: vec![] };
        assert_eq!(keyframe_value(&empty, 0.5), None);
    }

    #[test]
    fn basic_contribution_uses_the_timing_function_and_model_defaults() {
        let mut model = Layer::new(LayerId(1));
        model.opacity = 0.8;
        let anim = Animation::Basic {
            id: AnimationId(2),
            key: Property::Opacity,
            from: None,
            to: Some(Value::Scalar(0.0)),
            timing: secs(1.0),
            function: TimingFunction::EaseIn,
            additive: false,
        };
        // from comes from the model (0.8); at p = 0.5 the ease-in value is below halfway.
        let c = contributions(&anim, 0.0, 0.5, &model);
        let v = scalar(&c);
        let p = TimingFunction::EaseIn.eval(0.5);
        assert!(close(v, 0.8 * (1.0 - p)), "{v}");
        // Not started and no fill: nothing.
        assert!(contributions(&anim, 5.0, 0.0, &model).is_empty());
    }

    #[test]
    fn additive_basic_adds_its_offset_to_the_model_value() {
        let mut model = Layer::new(LayerId(1));
        model.position = Point { x: 100.0, y: 0.0 };
        let mut layer = model.clone();
        let anim = Animation::Basic {
            id: AnimationId(3),
            key: Property::Position,
            from: Some(Value::Point(Point { x: 0.0, y: 0.0 })),
            to: Some(Value::Point(Point { x: 50.0, y: 0.0 })),
            timing: secs(1.0),
            function: TimingFunction::Linear,
            additive: true,
        };
        for c in contributions(&anim, 0.0, 0.5, &model) {
            assert!(write(&mut layer, &c));
        }
        assert_eq!(layer.position, Point { x: 125.0, y: 0.0 });
    }

    #[test]
    fn spring_contribution_lands_on_the_target_when_forwards() {
        let model = Layer::new(LayerId(1));
        let anim = Animation::Spring {
            id: AnimationId(4),
            key: Property::Opacity,
            from: Some(Value::Scalar(0.0)),
            to: Some(Value::Scalar(1.0)),
            spring: Spring { mass: 1.0, stiffness: 100.0, damping: 20.0, initial_velocity: 0.0 },
            timing: MediaTiming { fill_mode: FillMode::Forwards, ..MediaTiming::default() },
        };
        let end = contributions(&anim, 0.0, 100.0, &model);
        assert_eq!(scalar(&end), 1.0);
        assert!(ended_at(&anim, 0.0, 100.0));
        assert!(!ended_at(&anim, 0.0, 0.5));
    }

    #[test]
    fn group_children_see_the_group_clock_and_their_own_begin_times() {
        let model = Layer::new(LayerId(1));
        let child_a = basic(Property::Opacity, 0.0, 1.0, secs(1.0));
        let child_b = Animation::Basic {
            id: AnimationId(6),
            key: Property::ZPosition,
            from: Some(Value::Scalar(0.0)),
            to: Some(Value::Scalar(10.0)),
            timing: MediaTiming { begin_time: Some(2.0), ..secs(1.0) },
            function: TimingFunction::Linear,
            additive: false,
        };
        let group = Animation::Group { id: AnimationId(5), animations: vec![child_a, child_b], timing: MediaTiming { speed: 2.0, ..MediaTiming::default() } };
        // Group at begin 0 with speed 2: at tree time 0.25 the children see 0.5.
        let c = contributions(&group, 0.0, 0.25, &model);
        assert_eq!(c.len(), 1, "child b has not begun (its begin is 2 on the group clock)");
        assert!(close(scalar(&c), 0.5));
        // Child b begins at group clock 2 (tree time 1.0) and runs 1 s: at tree time 1.25 it is 0.5 along, z = 5.
        let mid = contributions(&group, 0.0, 1.25, &model);
        let zs: Vec<f32> = mid.iter().filter(|c| c.key == Property::ZPosition).map(|c| scalar(std::slice::from_ref(c))).collect();
        assert_eq!(zs.len(), 1);
        assert!(close(zs[0], 5.0), "{zs:?}");
        // The group ends when every child has: child b ends at group clock 3, which is tree time 1.5.
        assert!(!ended_at(&group, 0.0, 1.4));
        assert!(ended_at(&group, 0.0, 1.5));
    }

    #[test]
    fn id_and_timing_accessors_cover_every_variant() {
        let kf = Animation::Keyframe {
            id: AnimationId(7),
            key: Property::Bounds,
            keyframes: Keyframes { values: vec![], key_times: vec![], timing_functions: vec![] },
            timing: MediaTiming { repeat_count: 3.0, ..MediaTiming::default() },
        };
        assert_eq!(kf.id(), AnimationId(7));
        assert_eq!(kf.timing().repeat_count, 3.0);
        let g = Animation::Group { id: AnimationId(8), animations: vec![], timing: MediaTiming::default() };
        assert_eq!(g.id(), AnimationId(8));
    }
}
