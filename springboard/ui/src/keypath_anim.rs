//! Key-path animation engine: animations that act on named properties of anything [`Animatable`].
//!
//! * Basic animations (`from`, `to`, `by`), keyframe animations (values or a motion path, key times, per-segment
//!   timing functions, calculation modes linear, discrete, paced, cubic and cubic paced, rotation along the path),
//!   spring animations (mass, stiffness, damping, initial velocity, settling duration) and groups.
//! * Media timing: begin time, duration, speed, time offset, repeat count, repeat duration, autoreverse, fill mode.
//!   One cycle lasts `duration`, or twice that with autoreverse (forward leg, then back); `repeat_count` counts
//!   cycles of that length; `repeat_duration` overrides it. Times are seconds since the animation was added
//!   (`begin_time` is a delay); local time is `(t - begin) * speed + time_offset`.
//! * Additive animations add their value to the underlying one (transforms: applied after it); cumulative ones add
//!   the end value once per finished cycle. A value function turns numbers into transforms.
//! * An [`AnimationSet`] holds the animations of one target by key, advances their lifecycle (delegate start and stop,
//!   removal on completion) and computes the presentation values over the model values. Retargeting a spring carries
//!   its velocity.
//!
//! The model values never change here; the target applies the presentation values the set returns. Pure maths and
//! bookkeeping on a caller supplied clock.

use crate::anim::Curve;
use crate::anim_value::{get_with, read_key, with_component, AnimValue, Animatable, ValueFunction, ValueKind};
use std::rc::Rc;

/// Key path the rotation of a path animation is written to.
pub const PATH_ROTATION_KEY: &str = "transform.rotation.z";

/// Easing of progress: a named curve or explicit control points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimingFunction {
    /// The system default ease (0.25, 0.1, 0.25, 1).
    Default,
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// Cubic Bézier through (0,0) and (1,1) with control points (x1,y1), (x2,y2).
    Cubic(f32, f32, f32, f32),
}

impl TimingFunction {
    /// `default`, `linear`, `easeIn`, `easeOut`, `easeInEaseOut` (case-insensitive).
    pub fn named(name: &str) -> Option<TimingFunction> {
        Some(match name.to_ascii_lowercase().as_str() {
            "default" => TimingFunction::Default,
            "linear" => TimingFunction::Linear,
            "easein" => TimingFunction::EaseIn,
            "easeout" => TimingFunction::EaseOut,
            "easeineaseout" | "easeinout" => TimingFunction::EaseInOut,
            _ => return None,
        })
    }
    pub fn control_points(&self) -> (f32, f32, f32, f32) {
        match *self {
            TimingFunction::Default => (0.25, 0.1, 0.25, 1.0),
            TimingFunction::Linear => (0.0, 0.0, 1.0, 1.0),
            TimingFunction::EaseIn => (0.42, 0.0, 1.0, 1.0),
            TimingFunction::EaseOut => (0.0, 0.0, 0.58, 1.0),
            TimingFunction::EaseInOut => (0.42, 0.0, 0.58, 1.0),
            TimingFunction::Cubic(a, b, c, d) => (a, b, c, d),
        }
    }
    pub fn curve(&self) -> Curve {
        match *self {
            TimingFunction::Linear => Curve::Linear,
            TimingFunction::EaseIn => Curve::EaseIn,
            TimingFunction::EaseOut => Curve::EaseOut,
            TimingFunction::EaseInOut => Curve::EaseInOut,
            other => {
                let (a, b, c, d) = other.control_points();
                Curve::Bezier(a, b, c, d)
            }
        }
    }
    /// The timing function of a non-spring curve.
    pub fn from_curve(c: Curve) -> Option<TimingFunction> {
        Some(match c {
            Curve::Linear => TimingFunction::Linear,
            Curve::EaseIn => TimingFunction::EaseIn,
            Curve::EaseOut => TimingFunction::EaseOut,
            Curve::EaseInOut => TimingFunction::EaseInOut,
            Curve::Bezier(a, b, c, d) => TimingFunction::Cubic(a, b, c, d),
            Curve::Spring { .. } => return None,
        })
    }
    /// Eased progress for linear progress `t` (clamped to 0..=1).
    pub fn at(&self, t: f32) -> f32 {
        self.curve().at(t)
    }
}

/// What an animation shows outside its active time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FillMode {
    /// Nothing before or after: the model value shows.
    #[default]
    Removed,
    /// The end value stays after the animation.
    Forwards,
    /// The start value shows during the begin delay.
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

/// Where an animation is on its timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Not begun yet.
    Before,
    /// Running: `progress` (0..=1, reversed on backward legs) through cycle number `cycle`.
    Active { progress: f32, cycle: u32 },
    /// Over: the progress and cycle it ended on.
    After { progress: f32, cycle: u32 },
}

/// Media timing (`beginTime`, `duration`, `speed`, `timeOffset`, `repeatCount`, `repeatDuration`, `autoreverses`,
/// `fillMode`). A `duration` of zero means the animation's own default (0.25 s, or the settling time of a spring).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Media {
    /// Seconds after the animation is added before it begins.
    pub begin_time: f64,
    pub duration: f64,
    pub speed: f32,
    /// Seconds into the active time the animation starts at.
    pub time_offset: f64,
    /// Cycles to play; 0 means 1; `f32::INFINITY` repeats forever; fractions play a partial last cycle.
    pub repeat_count: f32,
    /// Total active seconds; overrides `repeat_count` when positive.
    pub repeat_duration: f64,
    pub autoreverses: bool,
    pub fill: FillMode,
}

impl Default for Media {
    fn default() -> Self {
        Media { begin_time: 0.0, duration: 0.0, speed: 1.0, time_offset: 0.0, repeat_count: 0.0, repeat_duration: 0.0, autoreverses: false, fill: FillMode::Removed }
    }
}

impl Media {
    fn simple(&self, default: f64) -> f64 {
        if self.duration > 0.0 { self.duration } else { default.max(1e-9) }
    }
    fn cycle(&self, default: f64) -> f64 {
        self.simple(default) * if self.autoreverses { 2.0 } else { 1.0 }
    }
    /// Seconds of local time the animation is active for (infinite for an endless repeat).
    pub fn active_duration(&self, default: f64) -> f64 {
        if self.repeat_duration > 0.0 {
            self.repeat_duration
        } else {
            let n = if self.repeat_count > 0.0 { self.repeat_count as f64 } else { 1.0 };
            self.cycle(default) * n
        }
    }
    /// The phase `elapsed` seconds after the animation was added. `default` is the duration used when `duration`
    /// is zero.
    pub fn phase(&self, elapsed: f64, default: f64) -> Phase {
        let local = (elapsed - self.begin_time) * self.speed.max(0.0) as f64 + self.time_offset;
        if local < 0.0 {
            return Phase::Before;
        }
        let d = self.simple(default);
        let cl = self.cycle(default);
        let total = self.active_duration(default);
        let finished = local >= total;
        let l = if finished { total } else { local };
        let mut cycle = (l / cl).floor();
        let mut within = l - cycle * cl;
        if finished && cycle > 0.0 && within <= 1e-9 {
            cycle -= 1.0;
            within = cl;
        }
        let p = if self.autoreverses && within > d { 1.0 - (within - d) / d } else { within / d };
        let progress = p.clamp(0.0, 1.0) as f32;
        let cycle = cycle.min(u32::MAX as f64) as u32;
        if finished { Phase::After { progress, cycle } } else { Phase::Active { progress, cycle } }
    }
    /// Seconds into the simple duration at `progress` (what a group's children see as their clock).
    fn seconds_at(&self, progress: f32, default: f64) -> f64 {
        progress as f64 * self.simple(default)
    }
}

/// One drawing command of a [`MotionPath`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathSeg {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    QuadTo(f32, f32, f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32),
    /// A line back to the start of the sub-path.
    Close,
}

/// A path to move along, flattened to a polyline with cumulative lengths.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionPath {
    pts: Vec<(f32, f32)>,
    cum: Vec<f32>,
}

impl MotionPath {
    pub fn new(segs: &[PathSeg]) -> MotionPath {
        fn push(p: (f32, f32), jump: bool, pts: &mut Vec<(f32, f32)>, cum: &mut Vec<f32>) {
            let add = match pts.last() {
                Some(l) if !jump => ((p.0 - l.0).powi(2) + (p.1 - l.1).powi(2)).sqrt(),
                _ => 0.0,
            };
            let base = cum.last().copied().unwrap_or(0.0);
            pts.push(p);
            cum.push(base + add);
        }
        let mut pts: Vec<(f32, f32)> = Vec::new();
        let mut cum: Vec<f32> = Vec::new();
        let mut start = (0.0f32, 0.0f32);
        for s in segs {
            let last = pts.last().copied().unwrap_or((0.0, 0.0));
            match *s {
                PathSeg::MoveTo(x, y) => {
                    start = (x, y);
                    push((x, y), true, &mut pts, &mut cum);
                }
                PathSeg::LineTo(x, y) => push((x, y), false, &mut pts, &mut cum),
                PathSeg::QuadTo(cx, cy, x, y) => {
                    for i in 1..=16 {
                        let t = i as f32 / 16.0;
                        let u = 1.0 - t;
                        push((u * u * last.0 + 2.0 * u * t * cx + t * t * x, u * u * last.1 + 2.0 * u * t * cy + t * t * y), false, &mut pts, &mut cum);
                    }
                }
                PathSeg::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                    for i in 1..=24 {
                        let t = i as f32 / 24.0;
                        let u = 1.0 - t;
                        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                        push((a * last.0 + b * c1x + c * c2x + d * x, a * last.1 + b * c1y + c * c2y + d * y), false, &mut pts, &mut cum);
                    }
                }
                PathSeg::Close => push(start, false, &mut pts, &mut cum),
            }
        }
        MotionPath { pts, cum }
    }
    /// A path through `points` with straight lines.
    pub fn from_points(points: &[(f32, f32)]) -> MotionPath {
        let segs: Vec<PathSeg> = points.iter().enumerate().map(|(i, p)| if i == 0 { PathSeg::MoveTo(p.0, p.1) } else { PathSeg::LineTo(p.0, p.1) }).collect();
        MotionPath::new(&segs)
    }
    pub fn length(&self) -> f32 {
        self.cum.last().copied().unwrap_or(0.0)
    }
    /// The point at `fraction` (0..=1) of the length and the heading there in radians. `None` for an empty path.
    pub fn sample(&self, fraction: f32) -> Option<((f32, f32), f32)> {
        let first = *self.pts.first()?;
        let target = fraction.clamp(0.0, 1.0) * self.length();
        for i in 1..self.pts.len() {
            let seg = self.cum[i] - self.cum[i - 1];
            if seg <= 1e-9 {
                continue;
            }
            if target <= self.cum[i] {
                let f = (target - self.cum[i - 1]) / seg;
                let (a, b) = (self.pts[i - 1], self.pts[i]);
                return Some(((a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f), (b.1 - a.1).atan2(b.0 - a.0)));
            }
        }
        // zero length, or the target is past the last real segment
        let n = self.pts.len();
        for i in (1..n).rev() {
            if self.cum[i] - self.cum[i - 1] > 1e-9 {
                let (a, b) = (self.pts[i - 1], self.pts[i]);
                return Some((b, (b.1 - a.1).atan2(b.0 - a.0)));
            }
        }
        Some((first, 0.0))
    }
}

/// How keyframe values are mixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CalcMode {
    #[default]
    Linear,
    /// Each value holds until the next key time.
    Discrete,
    /// Constant speed: key times follow the distance between values.
    Paced,
    /// A smooth spline through the values, shaped by tension, continuity and bias.
    Cubic,
    /// A spline with key times set by the distance between values.
    CubicPaced,
}

/// How a path animation turns the object along the path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotationMode {
    /// Rotate to follow the heading of the path.
    Auto,
    /// Follow the heading plus a half turn.
    AutoReverse,
}

/// The keys of a keyframe animation.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct KeyframeSpec {
    pub values: Vec<AnimValue>,
    /// 0..=1, one per value (discrete mode also accepts one more); evenly spaced when `None` or unusable.
    pub key_times: Option<Vec<f32>>,
    /// One per segment, linear mode only; missing entries are linear.
    pub timing_functions: Vec<TimingFunction>,
    pub mode: CalcMode,
    /// When set, the animated value is a point on this path and `values` is ignored.
    pub path: Option<MotionPath>,
    pub rotation_mode: Option<RotationMode>,
    /// Spline shape per value (cubic modes); missing entries are zero.
    pub tension: Vec<f32>,
    pub continuity: Vec<f32>,
    pub bias: Vec<f32>,
}

impl KeyframeSpec {
    fn even_times(&self, n: usize) -> Vec<f32> {
        (0..n).map(|i| if n == 1 { 0.0 } else { i as f32 / (n - 1) as f32 }).collect()
    }
    fn paced_times(&self) -> Vec<f32> {
        let n = self.values.len();
        let mut cum = vec![0.0f32; n];
        for i in 1..n {
            cum[i] = cum[i - 1] + self.values[i - 1].distance(&self.values[i]);
        }
        let total = cum[n - 1];
        if total <= 1e-12 { self.even_times(n) } else { cum.iter().map(|c| c / total).collect() }
    }
    fn times(&self) -> Vec<f32> {
        let n = self.values.len();
        match self.mode {
            CalcMode::Paced | CalcMode::CubicPaced => self.paced_times(),
            _ => match &self.key_times {
                Some(k) if k.len() == n && k.windows(2).all(|w| w[0] <= w[1]) => k.clone(),
                _ => self.even_times(n),
            },
        }
    }
    fn kind(&self) -> Option<ValueKind> {
        let k = self.values.first()?.kind();
        self.values.iter().all(|v| v.kind() == k).then_some(k)
    }
    /// The last key (the value a cumulative animation adds per cycle).
    fn end(&self) -> Option<AnimValue> {
        if let Some(p) = &self.path {
            return p.sample(1.0).map(|(pt, _)| AnimValue::Point(pt.0, pt.1));
        }
        self.values.last().copied()
    }
    fn path_fraction(&self, p: f32) -> f32 {
        match &self.key_times {
            Some(k) if k.len() >= 2 && k.windows(2).all(|w| w[0] <= w[1]) => {
                let n = k.len();
                if p <= k[0] {
                    return 0.0;
                }
                if p >= k[n - 1] {
                    return 1.0;
                }
                let i = (0..n - 1).rev().find(|&i| k[i] <= p).unwrap_or(0);
                let span = k[i + 1] - k[i];
                let f = if span > 0.0 { (p - k[i]) / span } else { 1.0 };
                (i as f32 + f) / (n - 1) as f32
            }
            _ => p,
        }
    }
    /// The value at linear progress `p`, and the heading along the path when there is one.
    pub fn eval(&self, p: f32) -> Option<(AnimValue, Option<f32>)> {
        if let Some(path) = &self.path {
            let ((x, y), heading) = path.sample(self.path_fraction(p))?;
            return Some((AnimValue::Point(x, y), Some(heading)));
        }
        self.eval_values(p).map(|v| (v, None))
    }
    fn eval_values(&self, p: f32) -> Option<AnimValue> {
        let v = &self.values;
        let n = v.len();
        if n == 0 {
            return None;
        }
        self.kind()?;
        if n == 1 {
            return Some(v[0]);
        }
        if self.mode == CalcMode::Discrete {
            let kt: Vec<f32> = match &self.key_times {
                Some(k) if (k.len() == n || k.len() == n + 1) && k.windows(2).all(|w| w[0] <= w[1]) => k.clone(),
                _ => (0..n).map(|i| i as f32 / n as f32).collect(),
            };
            let i = (0..n).rev().find(|&i| kt[i] <= p).unwrap_or(0);
            return Some(v[i]);
        }
        let kt = self.times();
        if p <= kt[0] {
            return Some(v[0]);
        }
        if p >= kt[n - 1] {
            return Some(v[n - 1]);
        }
        let i = (0..n - 1).rev().find(|&i| kt[i] <= p).unwrap_or(0);
        let span = kt[i + 1] - kt[i];
        let s = if span > 1e-9 { (p - kt[i]) / span } else { 1.0 };
        let spline = matches!(self.mode, CalcMode::Cubic | CalcMode::CubicPaced) && self.kind() != Some(ValueKind::Transform);
        if spline {
            return Some(self.hermite(i, s));
        }
        let eased = if self.mode == CalcMode::Linear { self.timing_functions.get(i).map_or(s, |f| f.at(s)) } else { s };
        v[i].lerp(&v[i + 1], eased)
    }
    fn tangents(&self, i: usize) -> (Vec<f32>, Vec<f32>) {
        let n = self.values.len();
        let c = |k: usize| self.values[k].components();
        let (prev, cur, next) = (c(i.saturating_sub(1)), c(i), c((i + 1).min(n - 1)));
        let t = self.tension.get(i).copied().unwrap_or(0.0);
        let k = self.continuity.get(i).copied().unwrap_or(0.0);
        let b = self.bias.get(i).copied().unwrap_or(0.0);
        let (oa, ob) = ((1.0 - t) * (1.0 + b) * (1.0 + k) / 2.0, (1.0 - t) * (1.0 - b) * (1.0 - k) / 2.0);
        let (ia, ib) = ((1.0 - t) * (1.0 + b) * (1.0 - k) / 2.0, (1.0 - t) * (1.0 - b) * (1.0 + k) / 2.0);
        let m = cur.len();
        let mut din = Vec::with_capacity(m);
        let mut dout = Vec::with_capacity(m);
        for j in 0..m {
            let (back, fwd) = (cur[j] - prev[j], next[j] - cur[j]);
            dout.push(oa * back + ob * fwd);
            din.push(ia * back + ib * fwd);
        }
        (din, dout)
    }
    fn hermite(&self, i: usize, s: f32) -> AnimValue {
        let (p0, p1) = (self.values[i].components(), self.values[i + 1].components());
        let (_, m0) = self.tangents(i);
        let (m1, _) = self.tangents(i + 1);
        let (s2, s3) = (s * s, s * s * s);
        let (h00, h10, h01, h11) = (2.0 * s3 - 3.0 * s2 + 1.0, s3 - 2.0 * s2 + s, -2.0 * s3 + 3.0 * s2, s3 - s2);
        let out: Vec<f32> = (0..p0.len()).map(|j| h00 * p0[j] + h10 * m0[j] + h01 * p1[j] + h11 * m1[j]).collect();
        self.values[i].with_components(&out)
    }
}

/// `from`, `to` and `by` of a basic animation. Two of the three give the animation; one is completed from the
/// underlying value.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct BasicSpec {
    pub from: Option<AnimValue>,
    pub to: Option<AnimValue>,
    pub by: Option<AnimValue>,
}

/// A spring: `mass * x'' = -stiffness * x - damping * x'`, released with `initial_velocity` (fractions of the
/// distance per second, positive towards the target).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringSpec {
    pub basic: BasicSpec,
    pub mass: f32,
    pub stiffness: f32,
    pub damping: f32,
    pub initial_velocity: f32,
}

impl Default for SpringSpec {
    fn default() -> Self {
        SpringSpec { basic: BasicSpec::default(), mass: 1.0, stiffness: 100.0, damping: 10.0, initial_velocity: 0.0 }
    }
}

impl SpringSpec {
    /// A spring from a damping ratio and an undamped period in seconds (mass 1).
    pub fn from_response(damping_ratio: f32, response: f32) -> SpringSpec {
        let w = std::f64::consts::TAU / response.max(1e-3) as f64;
        SpringSpec { stiffness: (w * w) as f32, damping: (2.0 * damping_ratio as f64 * w) as f32, ..SpringSpec::default() }
    }
    fn physical(&self) -> (f64, f64) {
        // (natural frequency, damping ratio)
        let m = (self.mass as f64).max(1e-6);
        let k = (self.stiffness as f64).max(1e-6);
        let c = (self.damping as f64).max(0.0);
        ((k / m).sqrt(), c / (2.0 * (k * m).sqrt()))
    }
    /// Displacement from rest (1 at release, 0 at rest) and its rate of change at `t` seconds.
    fn state(&self, t: f64) -> (f64, f64) {
        let t = t.max(0.0);
        let (w, z) = self.physical();
        let v0 = self.initial_velocity as f64;
        if z < 1.0 - 1e-6 {
            let wd = w * (1.0 - z * z).sqrt();
            let b = (z * w - v0) / wd;
            let e = (-z * w * t).exp();
            let (cs, sn) = ((wd * t).cos(), (wd * t).sin());
            (e * (cs + b * sn), e * ((b * wd - z * w) * cs - (wd + z * w * b) * sn))
        } else if (z - 1.0).abs() <= 1e-6 {
            let d = w - v0;
            let e = (-w * t).exp();
            (e * (1.0 + d * t), e * (d - w * (1.0 + d * t)))
        } else {
            let s = (z * z - 1.0).sqrt();
            let (r1, r2) = (-w * (z - s), -w * (z + s));
            let c2 = (-v0 - r1) / (r2 - r1);
            let c1 = 1.0 - c2;
            let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
            (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
        }
    }
    /// Progress (0 at release, 1 at rest, may overshoot) `t` seconds after the release.
    pub fn progress(&self, t: f64) -> f64 {
        1.0 - self.state(t).0
    }
    /// Rate of progress in fractions of the distance per second.
    pub fn velocity(&self, t: f64) -> f64 {
        -self.state(t).1
    }
    /// Seconds until the spring stays within 0.1 % of rest (and nearly still); at most 60.
    pub fn settling_duration(&self) -> f64 {
        let step = 1.0 / 240.0;
        let (w, z) = self.physical();
        let rate = if z < 1.0 { z * w } else { w * (z - (z * z - 1.0).max(0.0).sqrt()) };
        let horizon = if rate > 1e-6 { (14.0 / rate).min(60.0) } else { 60.0 };
        let mut last = 0.0;
        let mut t = 0.0;
        while t < horizon {
            if (1.0 - self.progress(t)).abs() > 1e-3 || self.velocity(t).abs() > 1e-2 {
                last = t;
            }
            t += step;
        }
        (last + step).min(60.0)
    }
}

/// What kind of animation it is.
#[derive(Clone)]
pub enum Kind {
    Basic(BasicSpec),
    Keyframe(KeyframeSpec),
    Spring(SpringSpec),
    /// Children run on the group's clock; each has its own media timing and key path.
    Group(Vec<KeyAnimation>),
}

/// Told when an animation begins and ends (`animationDidStart`, `animationDidStop(finished:)`). `finished` is false
/// when the animation was removed or replaced before it ended.
pub trait AnimationDelegate {
    fn did_start(&self, _key: &str) {}
    fn did_stop(&self, _key: &str, _finished: bool) {}
}

/// An animation of one key path (or, for a group, of several).
#[derive(Clone)]
pub struct KeyAnimation {
    pub key_path: String,
    pub kind: Kind,
    pub media: Media,
    /// Eases the whole animation (ignored by springs). Linear unless set.
    pub timing: TimingFunction,
    pub additive: bool,
    pub cumulative: bool,
    pub value_function: Option<ValueFunction>,
    /// Remove the animation when it ends (the default). When false it stays and only a fill mode shows its effect.
    pub removed_on_completion: bool,
    pub delegate: Option<Rc<dyn AnimationDelegate>>,
}

pub const DEFAULT_DURATION: f64 = 0.25;

impl KeyAnimation {
    fn with_kind(key_path: &str, kind: Kind) -> KeyAnimation {
        KeyAnimation { key_path: key_path.into(), kind, media: Media::default(), timing: TimingFunction::Linear, additive: false, cumulative: false, value_function: None, removed_on_completion: true, delegate: None }
    }
    pub fn basic(key_path: &str) -> KeyAnimation {
        Self::with_kind(key_path, Kind::Basic(BasicSpec::default()))
    }
    pub fn keyframes(key_path: &str, values: Vec<AnimValue>) -> KeyAnimation {
        Self::with_kind(key_path, Kind::Keyframe(KeyframeSpec { values, ..KeyframeSpec::default() }))
    }
    /// Moves a point property along `path`.
    pub fn along_path(key_path: &str, path: MotionPath) -> KeyAnimation {
        Self::with_kind(key_path, Kind::Keyframe(KeyframeSpec { path: Some(path), ..KeyframeSpec::default() }))
    }
    pub fn spring(key_path: &str, spring: SpringSpec) -> KeyAnimation {
        Self::with_kind(key_path, Kind::Spring(spring))
    }
    pub fn group(children: Vec<KeyAnimation>) -> KeyAnimation {
        Self::with_kind("", Kind::Group(children))
    }
    /// The animation a transaction makes for an implicit change: `from` to `to` over `duration` with `curve`
    /// (a spring curve gives a spring).
    pub fn implicit(key_path: &str, from: AnimValue, to: AnimValue, duration: f32, curve: Curve) -> KeyAnimation {
        let basic = BasicSpec { from: Some(from), to: Some(to), by: None };
        match curve {
            Curve::Spring { damping_ratio, response } => Self::spring(key_path, SpringSpec { basic, ..SpringSpec::from_response(damping_ratio, response) }),
            c => {
                let mut a = Self::with_kind(key_path, Kind::Basic(basic));
                a.media.duration = duration as f64;
                a.timing = TimingFunction::from_curve(c).unwrap_or(TimingFunction::Linear);
                a
            }
        }
    }
    pub fn from(mut self, v: AnimValue) -> Self {
        self.basic_mut().from = Some(v);
        self
    }
    pub fn to(mut self, v: AnimValue) -> Self {
        self.basic_mut().to = Some(v);
        self
    }
    pub fn by(mut self, v: AnimValue) -> Self {
        self.basic_mut().by = Some(v);
        self
    }
    fn basic_mut(&mut self) -> &mut BasicSpec {
        if !matches!(self.kind, Kind::Spring(_) | Kind::Basic(_)) {
            self.kind = Kind::Basic(BasicSpec::default());
        }
        match &mut self.kind {
            Kind::Spring(s) => &mut s.basic,
            Kind::Basic(b) => b,
            _ => unreachable!(),
        }
    }
    pub fn duration(mut self, secs: f64) -> Self {
        self.media.duration = secs;
        self
    }
    pub fn begin_time(mut self, secs: f64) -> Self {
        self.media.begin_time = secs;
        self
    }
    pub fn speed(mut self, s: f32) -> Self {
        self.media.speed = s;
        self
    }
    pub fn time_offset(mut self, secs: f64) -> Self {
        self.media.time_offset = secs;
        self
    }
    pub fn repeat_count(mut self, n: f32) -> Self {
        self.media.repeat_count = n;
        self
    }
    pub fn repeat_duration(mut self, secs: f64) -> Self {
        self.media.repeat_duration = secs;
        self
    }
    pub fn autoreverses(mut self, on: bool) -> Self {
        self.media.autoreverses = on;
        self
    }
    pub fn fill(mut self, f: FillMode) -> Self {
        self.media.fill = f;
        self
    }
    pub fn timing(mut self, t: TimingFunction) -> Self {
        self.timing = t;
        self
    }
    pub fn additive(mut self, on: bool) -> Self {
        self.additive = on;
        self
    }
    pub fn cumulative(mut self, on: bool) -> Self {
        self.cumulative = on;
        self
    }
    pub fn value_function(mut self, f: ValueFunction) -> Self {
        self.value_function = Some(f);
        self
    }
    pub fn delegate(mut self, d: Rc<dyn AnimationDelegate>) -> Self {
        self.delegate = Some(d);
        self
    }
    pub fn removed_on_completion(mut self, on: bool) -> Self {
        self.removed_on_completion = on;
        self
    }
    /// Sets the keyframe parts of a keyframe animation (no effect on other kinds).
    pub fn with_keyframes(mut self, f: impl FnOnce(&mut KeyframeSpec)) -> Self {
        if let Kind::Keyframe(k) = &mut self.kind {
            f(k);
        }
        self
    }

    /// Duration used when `media.duration` is zero.
    fn default_duration(&self) -> f64 {
        match &self.kind {
            Kind::Spring(s) => s.settling_duration(),
            Kind::Group(c) => c
                .iter()
                .map(|a| {
                    let act = a.media.active_duration(a.default_duration());
                    a.media.begin_time + if a.media.speed > 0.0 { act / a.media.speed as f64 } else { f64::INFINITY }
                })
                .filter(|d| d.is_finite())
                .fold(0.0, f64::max)
                .max(1e-9),
            _ => DEFAULT_DURATION,
        }
    }
    /// The phase `elapsed` seconds after the animation was added.
    pub fn phase(&self, elapsed: f64) -> Phase {
        self.media.phase(elapsed, self.default_duration())
    }
    /// True for an animation that never ends by itself.
    pub fn is_endless(&self) -> bool {
        self.media.active_duration(self.default_duration()).is_infinite()
    }
}

/// The values of a target while animations run: model values with the animations' results laid over them.
pub struct Presentation<'a> {
    model: &'a dyn Animatable,
    over: Vec<(String, AnimValue)>,
}

impl<'a> Presentation<'a> {
    pub fn new(model: &'a dyn Animatable) -> Self {
        Presentation { model, over: Vec::new() }
    }
    pub fn get(&self, key: &str) -> Option<AnimValue> {
        get_with(key, &|k| self.over.iter().find(|e| e.0 == k).map(|e| e.1).or_else(|| self.model.value(k)))
    }
    pub fn model_value(&self, key: &str) -> Option<AnimValue> {
        read_key(self.model, key)
    }
    pub fn set(&mut self, key: &str, v: AnimValue) -> bool {
        if let Some(e) = self.over.iter_mut().find(|e| e.0 == key) {
            if e.1.kind() != v.kind() {
                return false;
            }
            e.1 = v;
            return true;
        }
        if let Some(m) = self.model.value(key) {
            if m.kind() != v.kind() {
                return false;
            }
            self.over.push((key.into(), v));
            return true;
        }
        let Some(sc) = v.as_scalar() else { return false };
        for (d, _) in key.match_indices('.').rev() {
            let (parent, suffix) = (&key[..d], &key[d + 1..]);
            if let Some(pv) = self.get(parent) {
                if let Some(nv) = with_component(&pv, suffix, sc) {
                    return self.set(parent, nv);
                }
            }
        }
        false
    }
    /// The animated keys with their presentation values.
    pub fn into_values(self) -> Vec<(String, AnimValue)> {
        self.over
    }
}

fn resolve_basic(b: &BasicSpec, additive: bool, cur: AnimValue, model: AnimValue) -> Option<(AnimValue, AnimValue)> {
    let dflt_from = if additive { cur.zero_like() } else { cur };
    let dflt_to = if additive { cur.zero_like() } else { model };
    let (f, t) = match (b.from, b.to, b.by) {
        (Some(f), Some(t), _) => (f, t),
        (Some(f), None, Some(by)) => (f, f.add(&by)?),
        (None, Some(t), Some(by)) => (t.sub(&by)?, t),
        (Some(f), None, None) => (f, dflt_to),
        (None, Some(t), None) => (dflt_from, t),
        (None, None, Some(by)) => (dflt_from, dflt_from.add(&by)?),
        (None, None, None) => (dflt_from, dflt_to),
    };
    (f.kind() == t.kind()).then_some((f, t))
}

impl KeyAnimation {
    /// Lays this animation's value at `elapsed` over `pres`. Returns false when it contributed nothing.
    fn apply(&self, elapsed: f64, pres: &mut Presentation) -> bool {
        let phase = self.phase(elapsed);
        let (progress, cycle) = match phase {
            Phase::Before if self.media.fill.backwards() => (0.0, 0),
            Phase::Before => return false,
            Phase::Active { progress, cycle } => (progress, cycle),
            Phase::After { progress, cycle } if self.media.fill.forwards() => (progress, cycle),
            Phase::After { .. } => return false,
        };
        if let Kind::Group(children) = &self.kind {
            let local = self.media.seconds_at(progress, self.default_duration());
            let mut any = false;
            for c in children {
                any |= c.apply(local, pres);
            }
            return any;
        }
        let key = self.key_path.as_str();
        let (Some(model), Some(cur)) = (pres.model_value(key), pres.get(key)) else { return false };
        // For a function-driven animation the underlying value is a transform, not what the numbers are.
        let (value, end, heading): (AnimValue, Option<AnimValue>, Option<f32>) = match &self.kind {
            Kind::Basic(b) => {
                let Some((f, t)) = resolve_basic(b, self.additive, cur, model).or_else(|| self.function_endpoints(b)) else { return false };
                let Some(v) = f.lerp(&t, self.timing.at(progress)) else { return false };
                (v, Some(t), None)
            }
            Kind::Spring(s) => {
                let Some((f, t)) = resolve_basic(&s.basic, self.additive, cur, model).or_else(|| self.function_endpoints(&s.basic)) else { return false };
                let secs = if matches!(phase, Phase::After { .. }) { f64::INFINITY } else { self.media.seconds_at(progress, self.default_duration()) };
                let p = if secs.is_finite() { s.progress(secs) as f32 } else { 1.0 };
                let Some(v) = f.lerp(&t, p) else { return false };
                (v, Some(t), None)
            }
            Kind::Keyframe(k) => {
                let Some((v, h)) = k.eval(self.timing.at(progress)) else { return false };
                (v, k.end(), h)
            }
            Kind::Group(_) => return false,
        };
        let mut value = value;
        if self.cumulative && cycle > 0 {
            if let Some(e) = end {
                if let Some(sum) = value.add(&e.scaled(cycle as f32)) {
                    value = sum;
                }
            }
        }
        if let Some(f) = self.value_function {
            match f.apply(&value) {
                Some(t) => value = t,
                None => return false,
            }
        }
        let new = if self.additive {
            match cur.add(&value) {
                Some(v) => v,
                None => return false,
            }
        } else {
            value
        };
        let ok = pres.set(key, new);
        if let (Some(h), Kind::Keyframe(k)) = (heading, &self.kind) {
            if let Some(mode) = k.rotation_mode {
                let angle = h + if mode == RotationMode::AutoReverse { std::f32::consts::PI } else { 0.0 };
                if pres.get(PATH_ROTATION_KEY).is_some() {
                    pres.set(PATH_ROTATION_KEY, AnimValue::Scalar(angle));
                }
            }
        }
        ok
    }
    /// With a value function the endpoints are plain numbers, so a missing one cannot come from the transform:
    /// from defaults to zero for rotations and translations, one for scales; to likewise.
    fn function_endpoints(&self, b: &BasicSpec) -> Option<(AnimValue, AnimValue)> {
        let f = self.value_function?;
        let rest = match f {
            ValueFunction::ScaleX | ValueFunction::ScaleY | ValueFunction::ScaleZ | ValueFunction::Scale => AnimValue::Scalar(1.0),
            _ => AnimValue::Scalar(0.0),
        };
        let from = b.from.or_else(|| b.to.and_then(|t| b.by.and_then(|by| t.sub(&by)))).unwrap_or(rest);
        let to = b.to.or_else(|| b.by.and_then(|by| from.add(&by))).unwrap_or(rest);
        (from.kind() == to.kind()).then_some((from, to))
    }
}

struct Entry {
    key: String,
    anim: KeyAnimation,
    /// Clock time the animation began its timeline (set by the first `advance` after it was added).
    start: Option<f64>,
    started: bool,
    stopped: bool,
}

/// The animations attached to one target, by key (`add(_:forKey:)`, `removeAnimation(forKey:)`).
#[derive(Default)]
pub struct AnimationSet {
    entries: Vec<Entry>,
    counter: u64,
    stopped: Vec<(String, bool)>,
}

impl AnimationSet {
    pub fn new() -> Self {
        Self::default()
    }
    /// Adds `anim` under `key` (an unused generated key when `None`) and returns the key. An animation already
    /// under that key is replaced and told it did not finish.
    pub fn add(&mut self, anim: KeyAnimation, key: Option<&str>) -> String {
        let key = match key {
            Some(k) => k.to_string(),
            None => {
                self.counter += 1;
                format!("animation#{}", self.counter)
            }
        };
        self.remove(&key);
        self.entries.push(Entry { key: key.clone(), anim, start: None, started: false, stopped: false });
        key
    }
    /// Removes the animation under `key`; a delegate that saw it start (and not stop) is told it did not finish.
    pub fn remove(&mut self, key: &str) -> bool {
        match self.entries.iter().position(|e| e.key == key) {
            Some(i) => {
                let e = self.entries.remove(i);
                self.note_stop(e, false);
                true
            }
            None => false,
        }
    }
    pub fn remove_all(&mut self) {
        for e in std::mem::take(&mut self.entries) {
            self.note_stop(e, false);
        }
    }
    fn note_stop(&mut self, e: Entry, finished: bool) {
        if !e.stopped {
            if e.started {
                if let Some(d) = &e.anim.delegate {
                    d.did_stop(&e.key, finished);
                }
            }
            self.stopped.push((e.key, finished));
        }
    }
    /// Keys of the animations that ended or were removed since the last call, with whether they finished.
    pub fn take_stopped(&mut self) -> Vec<(String, bool)> {
        std::mem::take(&mut self.stopped)
    }
    pub fn keys(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.key.clone()).collect()
    }
    pub fn get(&self, key: &str) -> Option<&KeyAnimation> {
        self.entries.iter().find(|e| e.key == key).map(|e| &e.anim)
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// True while some animation can still change its target's presentation: running, waiting to begin, or filling.
    pub fn is_active(&self) -> bool {
        self.entries.iter().any(|e| !e.stopped || e.anim.media.fill.forwards())
    }
    /// True when a running or waiting animation (not just a held fill) is left.
    pub fn is_running(&self) -> bool {
        self.entries.iter().any(|e| !e.stopped)
    }
    /// Moves lifecycles to clock time `now`: starts timelines, calls delegates, removes ended animations that are
    /// removed on completion.
    pub fn advance(&mut self, now: f64) {
        let mut ended = Vec::new();
        for (i, e) in self.entries.iter_mut().enumerate() {
            let start = *e.start.get_or_insert(now);
            let phase = e.anim.phase(now - start);
            if !e.started && !matches!(phase, Phase::Before) {
                e.started = true;
                if let Some(d) = &e.anim.delegate {
                    d.did_start(&e.key);
                }
            }
            if matches!(phase, Phase::After { .. }) && !e.stopped {
                e.stopped = true;
                if let Some(d) = &e.anim.delegate {
                    d.did_stop(&e.key, true);
                }
                self.stopped.push((e.key.clone(), true));
                if e.anim.removed_on_completion {
                    ended.push(i);
                }
            }
        }
        for i in ended.into_iter().rev() {
            self.entries.remove(i);
        }
    }
    /// The model values with every animation laid over them in the order they were added, at clock time `now`.
    /// An animation not yet started counts as starting now. Returns only the keys that animations touch.
    pub fn presentation(&self, now: f64, model: &dyn Animatable) -> Vec<(String, AnimValue)> {
        let mut pres = Presentation::new(model);
        for e in &self.entries {
            e.anim.apply(now - e.start.unwrap_or(now), &mut pres);
        }
        pres.into_values()
    }
    /// The presentation value of one key (a sub-path included).
    pub fn presentation_value(&self, key: &str, now: f64, model: &dyn Animatable) -> Option<AnimValue> {
        let mut pres = Presentation::new(model);
        for e in &self.entries {
            e.anim.apply(now - e.start.unwrap_or(now), &mut pres);
        }
        pres.get(key)
    }
    /// Redirects the spring under `key` to `to` from where it is now, keeping its velocity. A basic animation
    /// restarts from its presentation value instead. Returns false when `key` has no animation or `to` does not
    /// match the property.
    pub fn retarget(&mut self, key: &str, to: AnimValue, now: f64, model: &dyn Animatable) -> bool {
        let Some(i) = self.entries.iter().position(|e| e.key == key) else { return false };
        let prop = self.entries[i].anim.key_path.clone();
        let Some(m) = read_key(model, &prop) else { return false };
        if m.kind() != to.kind() {
            return false;
        }
        let cur = self.presentation_value(&prop, now, model).unwrap_or(m);
        let old = self.entries[i].anim.clone();
        let elapsed = now - self.entries[i].start.unwrap_or(now);
        let mut fraction = 0.0f32;
        if let Kind::Spring(s) = &old.kind {
            if let Some((f, t)) = resolve_basic(&s.basic, old.additive, cur, m) {
                let secs = old.media.seconds_at(
                    match old.phase(elapsed) {
                        Phase::Active { progress, .. } => progress,
                        _ => 1.0,
                    },
                    old.default_duration(),
                );
                let dp = s.velocity(secs) as f32;
                let (fc, tc, nc) = (f.components(), t.components(), to.components());
                let cc = cur.components();
                let vel: Vec<f32> = fc.iter().zip(&tc).map(|(a, b)| (b - a) * dp).collect();
                let dist: Vec<f32> = nc.iter().zip(&cc).map(|(a, b)| a - b).collect();
                let dd: f32 = dist.iter().map(|x| x * x).sum();
                if dd > 1e-12 {
                    fraction = vel.iter().zip(&dist).map(|(a, b)| a * b).sum::<f32>() / dd;
                }
            }
        }
        let mut next = old;
        let speed = next.media.speed;
        next.media = Media { speed, ..Media::default() };
        next.additive = false;
        next.cumulative = false;
        match &mut next.kind {
            Kind::Spring(s) => {
                s.basic = BasicSpec { from: Some(cur), to: Some(to), by: None };
                s.initial_velocity = fraction;
            }
            _ => {
                let d = old_duration(&self.entries[i].anim);
                next.kind = Kind::Basic(BasicSpec { from: Some(cur), to: Some(to), by: None });
                next.media.duration = d;
            }
        }
        self.add(next, Some(key));
        true
    }
}

fn old_duration(a: &KeyAnimation) -> f64 {
    a.media.simple(a.default_duration())
}

/// Wraps closures as an [`AnimationDelegate`].
pub struct ClosureDelegate {
    pub on_start: Box<dyn Fn(&str)>,
    pub on_stop: Box<dyn Fn(&str, bool)>,
}

impl AnimationDelegate for ClosureDelegate {
    fn did_start(&self, key: &str) {
        (self.on_start)(key)
    }
    fn did_stop(&self, key: &str, finished: bool) {
        (self.on_stop)(key, finished)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform3d::Transform3D;
    use crate::Rect;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Obj {
        opacity: f32,
        pos: (f32, f32),
        frame: Rect,
        transform: Option<Transform3D>,
    }

    impl Obj {
        fn new() -> Obj {
            Obj { opacity: 1.0, pos: (0.0, 0.0), frame: Rect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 }, transform: Some(Transform3D::IDENTITY) }
        }
    }

    impl Animatable for Obj {
        fn value(&self, key: &str) -> Option<AnimValue> {
            match key {
                "opacity" => Some(AnimValue::Scalar(self.opacity)),
                "position" => Some(AnimValue::Point(self.pos.0, self.pos.1)),
                "frame" => Some(AnimValue::Rect(self.frame)),
                "transform" => self.transform.map(AnimValue::Transform),
                _ => None,
            }
        }
        fn set_value(&mut self, key: &str, v: AnimValue) -> bool {
            match (key, v) {
                ("opacity", AnimValue::Scalar(s)) => self.opacity = s,
                ("position", AnimValue::Point(x, y)) => self.pos = (x, y),
                ("frame", AnimValue::Rect(r)) => self.frame = r,
                ("transform", AnimValue::Transform(t)) => self.transform = Some(t),
                _ => return false,
            }
            true
        }
    }

    fn val(set: &AnimationSet, key: &str, now: f64, o: &Obj) -> f32 {
        set.presentation_value(key, now, o).and_then(|v| v.as_scalar()).unwrap()
    }

    fn started(a: KeyAnimation) -> AnimationSet {
        let mut s = AnimationSet::new();
        s.add(a, Some("a"));
        s.advance(0.0);
        s
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn named_timing_functions_and_control_points() {
        assert_eq!(TimingFunction::named("easeInEaseOut"), Some(TimingFunction::EaseInOut));
        assert_eq!(TimingFunction::named("EASEIN"), Some(TimingFunction::EaseIn));
        assert_eq!(TimingFunction::named("x"), None);
        assert_eq!(TimingFunction::Default.control_points(), (0.25, 0.1, 0.25, 1.0));
        assert!(close(TimingFunction::Linear.at(0.3), 0.3));
        assert!(close(TimingFunction::Cubic(0.0, 0.0, 1.0, 1.0).at(0.3), 0.3));
        assert!(TimingFunction::EaseIn.at(0.3) < 0.3);
        assert_eq!(TimingFunction::from_curve(Curve::EaseOut), Some(TimingFunction::EaseOut));
        assert_eq!(TimingFunction::from_curve(Curve::Spring { damping_ratio: 1.0, response: 1.0 }), None);
    }

    #[test]
    fn basic_from_to_by_combinations() {
        let o = Obj::new();
        let mk = || KeyAnimation::basic("opacity").duration(1.0);
        let s = started(mk().from(AnimValue::Scalar(0.0)).to(AnimValue::Scalar(10.0)));
        assert!(close(val(&s, "opacity", 0.5, &o), 5.0));
        let s = started(mk().from(AnimValue::Scalar(2.0)).by(AnimValue::Scalar(4.0)));
        assert!(close(val(&s, "opacity", 1.0 - 1e-6, &o), 6.0));
        let s = started(mk().to(AnimValue::Scalar(10.0)).by(AnimValue::Scalar(4.0)));
        assert!(close(val(&s, "opacity", 0.0, &o), 6.0));
        let s = started(mk().to(AnimValue::Scalar(0.0)));
        assert!(close(val(&s, "opacity", 0.5, &o), 0.5), "from the current value");
        let s = started(mk().from(AnimValue::Scalar(0.0)));
        assert!(close(val(&s, "opacity", 0.5, &o), 0.5), "to the model value");
        let s = started(mk().by(AnimValue::Scalar(3.0)));
        assert!(close(val(&s, "opacity", 0.5, &o), 2.5));
    }

    #[test]
    fn the_model_is_untouched_and_the_animation_is_removed_when_done() {
        let o = Obj::new();
        let mut s = started(KeyAnimation::basic("opacity").duration(1.0).from(AnimValue::Scalar(0.0)).to(AnimValue::Scalar(1.0)));
        assert!(s.is_running());
        s.advance(1.5);
        assert!(s.is_empty());
        assert!(s.presentation(1.5, &o).is_empty());
        assert_eq!(o.opacity, 1.0);
        assert_eq!(s.take_stopped(), vec![("a".to_string(), true)]);
    }

    #[test]
    fn key_paths_reach_sub_components() {
        let o = Obj::new();
        let s = started(KeyAnimation::basic("frame.size.width").duration(1.0).to(AnimValue::Scalar(200.0)));
        let p = s.presentation(0.5, &o);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].0, "frame");
        assert_eq!(p[0].1, AnimValue::Rect(Rect { x: 0.0, y: 0.0, w: 150.0, h: 50.0 }));
        let s = started(KeyAnimation::basic("position.x").duration(1.0).to(AnimValue::Scalar(10.0)));
        assert!(close(val(&s, "position.x", 0.5, &o), 5.0));
    }

    #[test]
    fn two_animations_on_different_components_of_one_property_compose() {
        let o = Obj::new();
        let mut s = AnimationSet::new();
        s.add(KeyAnimation::basic("position.x").duration(1.0).to(AnimValue::Scalar(10.0)), Some("x"));
        s.add(KeyAnimation::basic("position.y").duration(1.0).to(AnimValue::Scalar(20.0)), Some("y"));
        s.advance(0.0);
        assert_eq!(s.presentation_value("position", 0.5, &o), Some(AnimValue::Point(5.0, 10.0)));
    }

    #[test]
    fn additive_animations_add_to_the_model_and_stack() {
        let mut o = Obj::new();
        o.pos = (100.0, 0.0);
        let mut s = AnimationSet::new();
        s.add(KeyAnimation::basic("position.x").duration(1.0).from(AnimValue::Scalar(-50.0)).to(AnimValue::Scalar(0.0)).additive(true), Some("a"));
        s.add(KeyAnimation::basic("position.x").duration(1.0).from(AnimValue::Scalar(10.0)).to(AnimValue::Scalar(10.0)).additive(true), Some("b"));
        s.advance(0.0);
        assert!(close(val(&s, "position.x", 0.0, &o), 60.0));
        assert!(close(val(&s, "position.x", 0.5, &o), 85.0));
    }

    #[test]
    fn additive_by_adds_zero_to_by() {
        let mut o = Obj::new();
        o.opacity = 5.0;
        let s = started(KeyAnimation::basic("opacity").duration(1.0).by(AnimValue::Scalar(4.0)).additive(true));
        assert!(close(val(&s, "opacity", 0.5, &o), 7.0));
    }

    #[test]
    fn cumulative_adds_the_end_value_each_cycle() {
        let o = Obj::new();
        let a = KeyAnimation::basic("opacity").duration(1.0).from(AnimValue::Scalar(0.0)).to(AnimValue::Scalar(10.0)).repeat_count(3.0).cumulative(true);
        let s = started(a);
        assert!(close(val(&s, "opacity", 0.5, &o), 5.0));
        assert!(close(val(&s, "opacity", 1.5, &o), 15.0));
        assert!(close(val(&s, "opacity", 2.5, &o), 25.0));
        let a = KeyAnimation::basic("opacity").duration(1.0).from(AnimValue::Scalar(0.0)).to(AnimValue::Scalar(10.0)).repeat_count(2.0);
        let s = started(a);
        assert!(close(val(&s, "opacity", 1.5, &o), 5.0), "not cumulative: every cycle starts over");
    }

    #[test]
    fn media_timing_begin_speed_offset() {
        let m = Media { duration: 2.0, begin_time: 1.0, ..Media::default() };
        assert_eq!(m.phase(0.5, 1.0), Phase::Before);
        assert_eq!(m.phase(2.0, 1.0), Phase::Active { progress: 0.5, cycle: 0 });
        assert_eq!(m.phase(3.0, 1.0), Phase::After { progress: 1.0, cycle: 0 });
        let fast = Media { duration: 2.0, speed: 2.0, ..Media::default() };
        assert_eq!(fast.phase(0.5, 1.0), Phase::Active { progress: 0.5, cycle: 0 });
        let off = Media { duration: 2.0, time_offset: 1.0, ..Media::default() };
        assert_eq!(off.phase(0.0, 1.0), Phase::Active { progress: 0.5, cycle: 0 });
        assert_eq!(off.phase(1.0, 1.0), Phase::After { progress: 1.0, cycle: 0 });
        let zero = Media::default();
        assert_eq!(zero.phase(0.125, 0.25), Phase::Active { progress: 0.5, cycle: 0 }, "default duration");
    }

    #[test]
    fn repeats_autoreverse_and_repeat_duration() {
        let m = Media { duration: 1.0, repeat_count: 2.0, ..Media::default() };
        assert_eq!(m.phase(1.25, 1.0), Phase::Active { progress: 0.25, cycle: 1 });
        assert_eq!(m.phase(2.0, 1.0), Phase::After { progress: 1.0, cycle: 1 });
        let r = Media { duration: 1.0, autoreverses: true, ..Media::default() };
        assert_eq!(r.phase(0.5, 1.0), Phase::Active { progress: 0.5, cycle: 0 });
        assert_eq!(r.phase(1.25, 1.0), Phase::Active { progress: 0.75, cycle: 0 });
        assert_eq!(r.phase(2.0, 1.0), Phase::After { progress: 0.0, cycle: 0 }, "a full forth and back ends at the start");
        let rd = Media { duration: 1.0, repeat_duration: 2.5, repeat_count: 100.0, ..Media::default() };
        assert_eq!(rd.phase(2.25, 1.0), Phase::Active { progress: 0.25, cycle: 2 });
        assert!(matches!(rd.phase(2.5, 1.0), Phase::After { .. }));
        let inf = Media { duration: 1.0, repeat_count: f32::INFINITY, ..Media::default() };
        assert!(matches!(inf.phase(1000.5, 1.0), Phase::Active { cycle: 1000, .. }));
        assert!(inf.active_duration(1.0).is_infinite());
    }

    #[test]
    fn fill_modes() {
        let o = Obj::new();
        let mk = |f| KeyAnimation::basic("opacity").duration(1.0).begin_time(1.0).from(AnimValue::Scalar(0.2)).to(AnimValue::Scalar(0.8)).fill(f);
        let mut s = AnimationSet::new();
        s.add(mk(FillMode::Removed), Some("a"));
        s.advance(0.0);
        assert!(s.presentation(0.5, &o).is_empty(), "nothing before");
        let mut s = AnimationSet::new();
        s.add(mk(FillMode::Backwards), Some("a"));
        s.advance(0.0);
        assert!(close(val(&s, "opacity", 0.5, &o), 0.2));
        let mut s = AnimationSet::new();
        s.add(mk(FillMode::Forwards).removed_on_completion(false), Some("a"));
        s.advance(0.0);
        s.advance(5.0);
        assert_eq!(s.len(), 1, "kept");
        assert!(close(val(&s, "opacity", 5.0, &o), 0.8), "the end value stays");
        assert!(!s.is_running() && s.is_active());
        let mut s = AnimationSet::new();
        s.add(mk(FillMode::Removed).removed_on_completion(false), Some("a"));
        s.advance(0.0);
        s.advance(5.0);
        assert!(s.presentation(5.0, &o).is_empty(), "kept but no effect");
    }

    #[test]
    fn autoreverse_returns_to_the_start_value() {
        let o = Obj::new();
        let a = KeyAnimation::basic("opacity").duration(1.0).from(AnimValue::Scalar(0.0)).to(AnimValue::Scalar(10.0)).autoreverses(true);
        let s = started(a);
        assert!(close(val(&s, "opacity", 0.5, &o), 5.0));
        assert!(close(val(&s, "opacity", 1.5, &o), 5.0));
        assert!(close(val(&s, "opacity", 1.9, &o), 1.0));
    }

    #[test]
    fn timing_function_eases_the_animation() {
        let o = Obj::new();
        let a = KeyAnimation::basic("opacity").duration(1.0).from(AnimValue::Scalar(0.0)).to(AnimValue::Scalar(1.0)).timing(TimingFunction::EaseIn);
        let s = started(a);
        assert!(val(&s, "opacity", 0.25, &o) < 0.25);
    }

    fn sc(v: f32) -> AnimValue {
        AnimValue::Scalar(v)
    }

    #[test]
    fn linear_keyframes_with_key_times_and_segment_timing() {
        let o = Obj::new();
        let a = KeyAnimation::keyframes("opacity", vec![sc(0.0), sc(10.0), sc(0.0)]).duration(1.0).with_keyframes(|k| k.key_times = Some(vec![0.0, 0.25, 1.0]));
        let s = started(a);
        assert!(close(val(&s, "opacity", 0.125, &o), 5.0));
        assert!(close(val(&s, "opacity", 0.25, &o), 10.0));
        assert!(close(val(&s, "opacity", 0.625, &o), 5.0));
        let a = KeyAnimation::keyframes("opacity", vec![sc(0.0), sc(1.0)]).duration(1.0).with_keyframes(|k| k.timing_functions = vec![TimingFunction::EaseIn]);
        let s = started(a);
        assert!(val(&s, "opacity", 0.25, &o) < 0.25);
    }

    #[test]
    fn discrete_keyframes_hold_each_value() {
        let o = Obj::new();
        let a = KeyAnimation::keyframes("opacity", vec![sc(1.0), sc(2.0), sc(3.0), sc(4.0)]).duration(1.0).with_keyframes(|k| k.mode = CalcMode::Discrete);
        let s = started(a);
        assert_eq!(val(&s, "opacity", 0.1, &o), 1.0);
        assert_eq!(val(&s, "opacity", 0.3, &o), 2.0);
        assert_eq!(val(&s, "opacity", 0.6, &o), 3.0);
        assert_eq!(val(&s, "opacity", 0.9, &o), 4.0);
        let a = KeyAnimation::keyframes("opacity", vec![sc(1.0), sc(2.0)]).duration(1.0).with_keyframes(|k| {
            k.mode = CalcMode::Discrete;
            k.key_times = Some(vec![0.0, 0.8, 1.0]);
        });
        let s = started(a);
        assert_eq!(val(&s, "opacity", 0.5, &o), 1.0);
        assert_eq!(val(&s, "opacity", 0.9, &o), 2.0);
    }

    #[test]
    fn paced_keyframes_move_at_constant_speed() {
        let o = Obj::new();
        // distances 10 and 30: the middle key lands at a quarter of the time
        let a = KeyAnimation::keyframes("opacity", vec![sc(0.0), sc(10.0), sc(40.0)]).duration(1.0).with_keyframes(|k| {
            k.mode = CalcMode::Paced;
            k.key_times = Some(vec![0.0, 0.9, 1.0]);
        });
        let s = started(a);
        assert!(close(val(&s, "opacity", 0.25, &o), 10.0));
        assert!(close(val(&s, "opacity", 0.5, &o), 20.0));
        assert!(close(val(&s, "opacity", 0.125, &o), 5.0));
    }

    #[test]
    fn cubic_keyframes_pass_through_the_keys_and_stay_smooth() {
        let o = Obj::new();
        let a = KeyAnimation::keyframes("opacity", vec![sc(0.0), sc(10.0), sc(0.0), sc(10.0)]).duration(3.0).with_keyframes(|k| k.mode = CalcMode::Cubic);
        let s = started(a);
        assert!(close(val(&s, "opacity", 1.0, &o), 10.0));
        assert!(close(val(&s, "opacity", 2.0, &o), 0.0));
        let lin = KeyAnimation::keyframes("opacity", vec![sc(0.0), sc(10.0), sc(0.0), sc(10.0)]).duration(3.0);
        let sl = started(lin);
        assert!((val(&s, "opacity", 0.5, &o) - val(&sl, "opacity", 0.5, &o)).abs() > 0.1, "a spline differs from straight lines");
        let a = KeyAnimation::keyframes("opacity", vec![sc(0.0), sc(10.0), sc(40.0)]).duration(1.0).with_keyframes(|k| k.mode = CalcMode::CubicPaced);
        let s = started(a);
        assert!(close(val(&s, "opacity", 0.25, &o), 10.0), "cubic paced keys follow distance");
    }

    #[test]
    fn mixed_kind_keyframes_do_nothing() {
        let o = Obj::new();
        let a = KeyAnimation::keyframes("opacity", vec![sc(0.0), AnimValue::Point(1.0, 1.0)]).duration(1.0);
        let s = started(a);
        assert!(s.presentation(0.5, &o).is_empty());
    }

    #[test]
    fn path_animation_follows_the_path_and_rotates() {
        let o = Obj::new();
        let path = MotionPath::new(&[PathSeg::MoveTo(0.0, 0.0), PathSeg::LineTo(10.0, 0.0), PathSeg::LineTo(10.0, 10.0)]);
        assert!(close(path.length(), 20.0));
        let a = KeyAnimation::along_path("position", path).duration(1.0).with_keyframes(|k| k.rotation_mode = Some(RotationMode::Auto));
        let s = started(a);
        assert_eq!(s.presentation_value("position", 0.25, &o), Some(AnimValue::Point(5.0, 0.0)));
        assert_eq!(s.presentation_value("position", 0.75, &o), Some(AnimValue::Point(10.0, 5.0)));
        assert!(close(val(&s, PATH_ROTATION_KEY, 0.75, &o), std::f32::consts::FRAC_PI_2));
        assert!(close(val(&s, PATH_ROTATION_KEY, 0.25, &o), 0.0));
        let a = KeyAnimation::along_path("position", MotionPath::from_points(&[(0.0, 0.0), (10.0, 0.0)])).duration(1.0).with_keyframes(|k| k.rotation_mode = Some(RotationMode::AutoReverse));
        let s = started(a);
        assert!((val(&s, PATH_ROTATION_KEY, 0.5, &o).abs() - std::f32::consts::PI).abs() < 1e-3);
    }

    #[test]
    fn curved_paths_are_flattened_with_the_right_length() {
        let p = MotionPath::new(&[PathSeg::MoveTo(0.0, 0.0), PathSeg::CubicTo(0.0, 0.0, 10.0, 0.0, 10.0, 0.0)]);
        assert!(close(p.length(), 10.0));
        let q = MotionPath::new(&[PathSeg::MoveTo(0.0, 0.0), PathSeg::QuadTo(5.0, 0.0, 10.0, 0.0), PathSeg::Close]);
        assert!(close(q.length(), 20.0));
        assert!(MotionPath::new(&[]).sample(0.5).is_none());
        let z = MotionPath::from_points(&[(3.0, 4.0)]);
        assert_eq!(z.sample(0.5), Some(((3.0, 4.0), 0.0)));
    }

    #[test]
    fn groups_run_children_on_the_group_clock() {
        let o = Obj::new();
        let a = KeyAnimation::basic("opacity").duration(1.0).from(sc(0.0)).to(sc(1.0));
        let b = KeyAnimation::basic("position.x").duration(1.0).begin_time(1.0).from(sc(0.0)).to(sc(10.0)).fill(FillMode::Both);
        let g = KeyAnimation::group(vec![a, b]);
        assert!(close(g.default_duration() as f32, 2.0));
        let s = started(g);
        assert!(close(val(&s, "opacity", 0.5, &o), 0.5));
        assert_eq!(val(&s, "position.x", 0.5, &o), 0.0, "backwards fill of the late child");
        assert!(close(val(&s, "position.x", 1.5, &o), 5.0));
        let g = KeyAnimation::group(vec![KeyAnimation::basic("opacity").duration(1.0).from(sc(0.0)).to(sc(1.0))]).duration(2.0).speed(2.0);
        let s = started(g);
        assert!(close(val(&s, "opacity", 0.25, &o), 0.5), "the group's speed scales its children's time");
    }

    #[test]
    fn value_functions_drive_a_transform() {
        let o = Obj::new();
        let a = KeyAnimation::basic("transform").duration(1.0).from(sc(0.0)).to(sc(2.0)).value_function(ValueFunction::RotateZ);
        let s = started(a);
        let v = s.presentation_value("transform.rotation.z", 0.5, &o).unwrap().as_scalar().unwrap();
        assert!(close(v, 1.0));
        let a = KeyAnimation::basic("transform").duration(1.0).to(sc(3.0)).value_function(ValueFunction::ScaleX);
        let s = started(a);
        let v = s.presentation_value("transform.scale.x", 0.5, &o).unwrap().as_scalar().unwrap();
        assert!(close(v, 2.0), "scale starts from one");
        let a = KeyAnimation::basic("transform").duration(1.0).to(sc(8.0)).value_function(ValueFunction::TranslateY).additive(true);
        let s = started(a);
        let v = s.presentation_value("transform.translation.y", 0.5, &o).unwrap().as_scalar().unwrap();
        assert!(close(v, 4.0));
    }

    #[test]
    fn rotation_by_key_path_without_a_value_function() {
        let o = Obj::new();
        let a = KeyAnimation::basic("transform.rotation.z").duration(1.0).from(sc(0.0)).to(sc(2.0));
        let s = started(a);
        assert!(close(val(&s, "transform.rotation.z", 0.5, &o), 1.0));
    }

    #[test]
    fn a_spring_settles_overshoots_and_ends_exactly() {
        let o = Obj::new();
        let spring = SpringSpec { stiffness: 200.0, damping: 8.0, ..SpringSpec::default() };
        let d = spring.settling_duration();
        assert!(d > 0.3 && d < 5.0, "{d}");
        let a = KeyAnimation::spring("opacity", spring).from(sc(0.0)).to(sc(1.0));
        let s = started(a);
        let peak = (1..200).map(|i| val(&s, "opacity", i as f64 * d / 200.0, &o)).fold(0.0f32, f32::max);
        assert!(peak > 1.05, "underdamped overshoots, got {peak}");
        let mut s = s;
        s.advance(d + 0.1);
        assert!(s.is_empty());
        let crit = SpringSpec { stiffness: 100.0, damping: 20.0, ..SpringSpec::default() };
        let peak = (1..100).map(|i| crit.progress(i as f64 * 0.05)).fold(0.0f64, f64::max);
        assert!(peak <= 1.0 + 1e-6, "critical damping does not overshoot");
        let over = SpringSpec { stiffness: 100.0, damping: 40.0, ..SpringSpec::default() };
        assert!(over.progress(0.2) < 1.0 && over.progress(30.0) > 0.999);
        assert!(close(spring.progress(0.0) as f32, 0.0));
    }

    #[test]
    fn spring_initial_velocity_changes_the_start() {
        let base = SpringSpec::default();
        let fast = SpringSpec { initial_velocity: 5.0, ..base };
        assert!(fast.progress(0.02) > base.progress(0.02));
        assert!(close(fast.velocity(0.0) as f32, 5.0));
        for z in [5.0f32, 20.0, 60.0] {
            let s = SpringSpec { damping: z, initial_velocity: 3.0, ..base };
            assert!((s.velocity(0.0) - 3.0).abs() < 0.01, "damping {z}");
        }
    }

    #[test]
    fn spring_from_response_matches_the_damping_ratio() {
        let s = SpringSpec::from_response(1.0, 0.5);
        let (w, z) = s.physical();
        assert!((z - 1.0).abs() < 1e-3);
        assert!((w - std::f64::consts::TAU / 0.5).abs() < 1e-3);
    }

    #[test]
    fn retargeting_a_spring_carries_its_velocity() {
        let o = Obj::new();
        let spring = SpringSpec { stiffness: 100.0, damping: 12.0, ..SpringSpec::default() };
        let mut s = AnimationSet::new();
        s.add(KeyAnimation::spring("opacity", spring).from(sc(0.0)).to(sc(100.0)), Some("a"));
        s.advance(0.0);
        let t = 0.15;
        let before = val(&s, "opacity", t, &o);
        let h = 1e-3;
        let v_before = (val(&s, "opacity", t + h, &o) - val(&s, "opacity", t - h, &o)) / (2.0 * h as f32);
        assert!(v_before > 50.0);
        assert!(s.retarget("a", sc(200.0), t, &o));
        s.advance(t);
        let after = val(&s, "opacity", t, &o);
        assert!(close(after, before), "no jump: {before} vs {after}");
        let h = 2e-4;
        let v_after = (val(&s, "opacity", t + 2.0 * h, &o) - val(&s, "opacity", t, &o)) / (2.0 * h as f32);
        assert!((v_after - v_before).abs() / v_before < 0.1, "{v_before} vs {v_after}");
        assert!(!s.retarget("missing", sc(1.0), t, &o));
        assert!(!s.retarget("a", AnimValue::Point(0.0, 0.0), t, &o), "wrong kind");
    }

    #[test]
    fn retargeting_a_basic_animation_restarts_from_where_it_is() {
        let o = Obj::new();
        let mut s = AnimationSet::new();
        s.add(KeyAnimation::basic("opacity").duration(1.0).from(sc(0.0)).to(sc(10.0)), Some("a"));
        s.advance(0.0);
        assert!(s.retarget("a", sc(0.0), 0.5, &o));
        s.advance(0.5);
        assert!(close(val(&s, "opacity", 0.5, &o), 5.0));
        assert!(close(val(&s, "opacity", 1.0, &o), 2.5));
    }

    #[derive(Default)]
    struct Log {
        events: RefCell<Vec<String>>,
    }
    impl AnimationDelegate for Log {
        fn did_start(&self, key: &str) {
            self.events.borrow_mut().push(format!("start {key}"));
        }
        fn did_stop(&self, key: &str, finished: bool) {
            self.events.borrow_mut().push(format!("stop {key} {finished}"));
        }
    }

    #[test]
    fn the_delegate_hears_start_and_stop() {
        let log = Rc::new(Log::default());
        let mut s = AnimationSet::new();
        s.add(KeyAnimation::basic("opacity").duration(1.0).begin_time(1.0).from(sc(0.0)).to(sc(1.0)).delegate(log.clone()), Some("a"));
        s.advance(0.0);
        assert!(log.events.borrow().is_empty(), "not begun");
        s.advance(1.5);
        assert_eq!(*log.events.borrow(), vec!["start a".to_string()]);
        s.advance(3.0);
        assert_eq!(*log.events.borrow(), vec!["start a".to_string(), "stop a true".to_string()]);
    }

    #[test]
    fn removing_or_replacing_reports_not_finished() {
        let log = Rc::new(Log::default());
        let mk = || KeyAnimation::basic("opacity").duration(10.0).from(sc(0.0)).to(sc(1.0)).delegate(log.clone());
        let mut s = AnimationSet::new();
        s.add(mk(), Some("a"));
        s.advance(0.0);
        s.add(mk(), Some("a"));
        assert_eq!(*log.events.borrow(), vec!["start a".to_string(), "stop a false".to_string()]);
        assert!(s.remove("a"));
        assert!(!s.remove("a"));
        assert_eq!(log.events.borrow().len(), 2, "the replacement never started, so it reports nothing");
        s.add(mk(), Some("b"));
        s.advance(1.0);
        s.remove_all();
        assert_eq!(log.events.borrow().last().unwrap(), "stop b false");
        assert!(s.is_empty());
    }

    #[test]
    fn keys_add_remove_and_generated_keys() {
        let mut s = AnimationSet::new();
        let k1 = s.add(KeyAnimation::basic("opacity"), None);
        let k2 = s.add(KeyAnimation::basic("opacity"), None);
        assert_ne!(k1, k2);
        s.add(KeyAnimation::basic("position"), Some("move"));
        assert_eq!(s.len(), 3);
        assert!(s.get("move").is_some() && s.get("zz").is_none());
        assert_eq!(s.keys().len(), 3);
        assert!(s.remove("move"));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn animations_start_at_the_first_advance_after_they_were_added() {
        let o = Obj::new();
        let mut s = AnimationSet::new();
        s.add(KeyAnimation::basic("opacity").duration(1.0).from(sc(0.0)).to(sc(1.0)), Some("a"));
        assert!(close(val(&s, "opacity", 50.0, &o), 0.0), "before the clock starts it shows its start");
        s.advance(50.0);
        assert!(close(val(&s, "opacity", 50.5, &o), 0.5));
    }

    #[test]
    fn presentation_ignores_unknown_keys_and_kind_mismatches() {
        let o = Obj::new();
        let s = started(KeyAnimation::basic("nothing").duration(1.0).from(sc(0.0)).to(sc(1.0)));
        assert!(s.presentation(0.5, &o).is_empty());
        let s = started(KeyAnimation::basic("opacity").duration(1.0).from(AnimValue::Point(0.0, 0.0)).to(AnimValue::Point(1.0, 1.0)));
        assert!(s.presentation(0.5, &o).is_empty());
    }

    #[test]
    fn implicit_animations_use_the_transaction_curve() {
        let o = Obj::new();
        let a = KeyAnimation::implicit("opacity", sc(0.0), sc(1.0), 1.0, Curve::EaseIn);
        let s = started(a);
        assert!(val(&s, "opacity", 0.25, &o) < 0.25);
        let a = KeyAnimation::implicit("opacity", sc(0.0), sc(1.0), 1.0, Curve::Spring { damping_ratio: 0.5, response: 0.4 });
        assert!(matches!(a.kind, Kind::Spring(_)));
    }

    #[test]
    fn the_closure_delegate_forwards() {
        let seen = Rc::new(RefCell::new(Vec::<String>::new()));
        let (a, b) = (seen.clone(), seen.clone());
        let d = ClosureDelegate { on_start: Box::new(move |k| a.borrow_mut().push(format!("s{k}"))), on_stop: Box::new(move |k, f| b.borrow_mut().push(format!("e{k}{f}"))) };
        d.did_start("x");
        d.did_stop("x", true);
        assert_eq!(*seen.borrow(), vec!["sx".to_string(), "extrue".to_string()]);
    }
}
