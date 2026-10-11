//! Animation curves and springs (`CAMediaTimingFunction`, `NativeView.animate`, `CASpringAnimation`) as pure functions of time.

/// How progress maps to time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// A cubic Bézier through (0,0) and (1,1) with control points (x1,y1), (x2,y2) (`CAMediaTimingFunction`).
    Bezier(f32, f32, f32, f32),
    /// A damped spring from 0 to 1. `damping_ratio` < 1 overshoots, 1 is critical, > 1 creeps. `response` is the
    /// undamped period in seconds (`NativeSpringTimingParameters`).
    Spring { damping_ratio: f32, response: f32 },
}

impl Curve {
    /// Progress (0 at the start, 1 at the end; springs may overshoot) at `t` in 0..=1 of the duration.
    /// For a spring `t` is clamped to the animation's settle time, so the curve is exactly 1 at the end.
    pub fn at(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Curve::Linear => t,
            Curve::EaseIn => bezier(0.42, 0.0, 1.0, 1.0, t),
            Curve::EaseOut => bezier(0.0, 0.0, 0.58, 1.0, t),
            Curve::EaseInOut => bezier(0.42, 0.0, 0.58, 1.0, t),
            Curve::Bezier(a, b, c, d) => bezier(a, b, c, d, t),
            Curve::Spring { damping_ratio, response } => {
                if t >= 1.0 {
                    return 1.0;
                }
                spring(damping_ratio, response, t * settle_time(damping_ratio, response))
            }
        }
    }
}

/// Coefficients of a cubic Bézier through (0,0) and (1,1): x(t) = ((ax t + bx) t + cx) t, likewise y.
struct Poly {
    ax: f32,
    bx: f32,
    cx: f32,
    ay: f32,
    by: f32,
    cy: f32,
}

impl Poly {
    fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Poly {
        let cx = 3.0 * x1;
        let bx = 3.0 * (x2 - x1) - cx;
        let cy = 3.0 * y1;
        let by = 3.0 * (y2 - y1) - cy;
        Poly { ax: 1.0 - cx - bx, bx, cx, ay: 1.0 - cy - by, by, cy }
    }
    fn x(&self, t: f32) -> f32 {
        ((self.ax * t + self.bx) * t + self.cx) * t
    }
    fn dx(&self, t: f32) -> f32 {
        (3.0 * self.ax * t + 2.0 * self.bx) * t + self.cx
    }
    fn y(&self, t: f32) -> f32 {
        ((self.ay * t + self.by) * t + self.cy) * t
    }
    /// The parameter t with x(t) = x: Newton's method until the step in t is negligible (the error in y follows t, so a
    /// small residual in x is not enough where x(t) is nearly flat), then bisection when Newton did not settle.
    fn solve(&self, x: f32) -> f32 {
        let mut t = x;
        for _ in 0..10 {
            let d = self.dx(t);
            if d.abs() < 1e-6 {
                break;
            }
            let step = (self.x(t) - x) / d;
            t -= step;
            if step.abs() < 1e-7 {
                if (0.0..=1.0).contains(&t) && (self.x(t) - x).abs() < 1e-5 {
                    return t;
                }
                break;
            }
        }
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if self.x(mid) < x { lo = mid } else { hi = mid }
        }
        (lo + hi) / 2.0
    }
}

fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    if x <= 0.0 { return 0.0; }
    if x >= 1.0 { return 1.0; }
    let p = Poly::new(x1, y1, x2, y2);
    p.y(p.solve(x))
}

/// Position of a unit-step spring `secs` after release (starts at 0, rests at 1).
fn spring(zeta: f32, response: f32, secs: f32) -> f32 {
    let w = std::f32::consts::TAU / response.max(1e-3); // natural frequency
    let z = zeta.max(1e-3);
    if z < 1.0 {
        let wd = w * (1.0 - z * z).sqrt();
        1.0 - (-z * w * secs).exp() * ((wd * secs).cos() + (z * w / wd) * (wd * secs).sin())
    } else if (z - 1.0).abs() < 1e-4 {
        1.0 - (-w * secs).exp() * (1.0 + w * secs)
    } else {
        let s = (z * z - 1.0).sqrt();
        let (r1, r2) = (-w * (z - s), -w * (z + s));
        1.0 - (r2 * (r1 * secs).exp() - r1 * (r2 * secs).exp()) / (r2 - r1)
    }
}

/// Seconds until the spring stays within 0.1 % of rest.
fn settle_time(zeta: f32, response: f32) -> f32 {
    let w = std::f32::consts::TAU / response.max(1e-3);
    let z = zeta.max(1e-3);
    let decay = if z < 1.0 { z * w } else { w * (z - (z * z - 1.0).sqrt()) };
    (6.9 / decay.max(1e-3)).min(60.0)
}

/// A value animating from `from` to `to`, started at `start` seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Animation {
    pub from: f32,
    pub to: f32,
    pub start: f64,
    pub duration: f32,
    pub curve: Curve,
}

impl Animation {
    pub fn new(from: f32, to: f32, start: f64, duration: f32, curve: Curve) -> Self {
        Animation { from, to, start, duration: duration.max(0.0), curve }
    }
    /// A spring animation lasts as long as the spring takes to settle.
    pub fn spring(from: f32, to: f32, start: f64, damping_ratio: f32, response: f32) -> Self {
        Animation::new(from, to, start, settle_time(damping_ratio, response), Curve::Spring { damping_ratio, response })
    }
    pub fn progress(&self, now: f64) -> f32 {
        progress_of(self.start, self.duration, now)
    }
    pub fn value(&self, now: f64) -> f32 {
        self.from + (self.to - self.from) * self.curve.at(self.progress(now))
    }
    pub fn is_done(&self, now: f64) -> bool {
        self.progress(now) >= 1.0
    }
    /// Redirects to `to` from wherever the value is now, so an interrupted animation does not jump
    /// (`NativeViewPropertyAnimator` retargeting).
    pub fn retarget(&self, now: f64, to: f32) -> Animation {
        Animation { from: self.value(now), to, start: now, ..*self }
    }
}

/// Fraction of `duration` elapsed at `now`, 0..=1. A zero duration is already complete.
fn progress_of(start: f64, duration: f32, now: f64) -> f32 {
    if duration <= 0.0 { 1.0 } else { ((now - start) as f32 / duration).clamp(0.0, 1.0) }
}

/// How keyframe values are computed between keys (`CAKeyframeAnimation.calculationMode`, the linear and discrete modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyframeMode {
    /// Values interpolate between neighbouring keys, paced by each segment's curve.
    Linear,
    /// Each value holds until the next key time.
    Discrete,
}

/// A value animated through several keys (`CAKeyframeAnimation` with `values`, `keyTimes` and `timingFunctions`).
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframes {
    pub values: Vec<f32>,
    /// Normalised key times in 0..=1, one per value, non-decreasing.
    pub key_times: Vec<f32>,
    /// One curve per segment (`values.len() - 1`). Empty means every segment is linear.
    pub timing: Vec<Curve>,
    pub mode: KeyframeMode,
    pub start: f64,
    pub duration: f32,
}

impl Keyframes {
    /// `None` when the keys cannot be used: no values, a key-time or timing count that does not match, or key times
    /// that are not ascending within 0..=1 (NaN included). Without `key_times` the values are spaced evenly.
    pub fn new(values: Vec<f32>, key_times: Option<Vec<f32>>, timing: Vec<Curve>, mode: KeyframeMode, start: f64, duration: f32) -> Option<Self> {
        let n = values.len();
        if n == 0 {
            return None;
        }
        let key_times = key_times.unwrap_or_else(|| (0..n).map(|i| if n == 1 { 0.0 } else { i as f32 / (n - 1) as f32 }).collect());
        let ascending = key_times.windows(2).all(|w| w[0] <= w[1]);
        let in_range = key_times.iter().all(|k| (0.0..=1.0).contains(k));
        if key_times.len() != n || !ascending || !in_range {
            return None;
        }
        if !(timing.is_empty() || timing.len() == n - 1) {
            return None;
        }
        Some(Keyframes { values, key_times, timing, mode, start, duration: duration.max(0.0) })
    }
    /// The value at `now`. Before the first key time it is the first value, from the last key time on it is the last.
    pub fn value(&self, now: f64) -> f32 {
        let p = progress_of(self.start, self.duration, now);
        let (kt, v) = (&self.key_times, &self.values);
        let last = v.len() - 1;
        if p < kt[0] {
            return v[0];
        }
        if p >= kt[last] {
            return v[last];
        }
        // p < kt[last], so the largest i with kt[i] <= p is a segment whose end lies beyond p, and its span is positive.
        let i = (0..last).rev().find(|&i| kt[i] <= p).unwrap_or(0);
        match self.mode {
            KeyframeMode::Discrete => v[i],
            KeyframeMode::Linear => {
                let t = (p - kt[i]) / (kt[i + 1] - kt[i]);
                let eased = self.timing.get(i).map_or(t, |c| c.at(t));
                v[i] + (v[i + 1] - v[i]) * eased
            }
        }
    }
    pub fn is_done(&self, now: f64) -> bool {
        progress_of(self.start, self.duration, now) >= 1.0
    }
}

/// Playback timing of an animation (`CAMediaTiming`): when it begins, its speed, how often it repeats and whether it
/// reverses. It maps the clock to the 0..=1 input of a curve. Negative speeds are not supported.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediaTiming {
    /// Seconds after the animation's start before its first cycle plays.
    pub begin_time: f64,
    /// Local seconds per clock second; 1 is real time.
    pub speed: f32,
    /// Whole cycles to play, at least 1. `f32::INFINITY` repeats forever.
    pub repeat_count: f32,
    /// Odd-numbered cycles play backwards, so a finish after an even number of cycles returns to the start.
    pub autoreverses: bool,
    /// Seconds per cycle.
    pub duration: f32,
}

impl MediaTiming {
    /// One cycle of `duration` seconds at real time, from the start.
    pub fn new(duration: f32) -> Self {
        MediaTiming { begin_time: 0.0, speed: 1.0, repeat_count: 1.0, autoreverses: false, duration: duration.max(0.0) }
    }
    fn local_time(&self, now: f64, start: f64) -> f64 {
        (now - start - self.begin_time) * self.speed as f64
    }
    /// Progress in 0..=1 through the current cycle at clock time `now`, for an animation started at `start`. `None`
    /// before the animation begins. After the last cycle it is 1, or 0 when the cycles reverse back to the start.
    pub fn progress(&self, now: f64, start: f64) -> Option<f32> {
        let elapsed = self.local_time(now, start);
        if elapsed < 0.0 {
            return None;
        }
        if self.duration <= 0.0 {
            return Some(1.0);
        }
        let cycles = self.repeat_count.max(1.0);
        let d = self.duration as f64;
        if elapsed >= cycles as f64 * d {
            let ends_backwards = self.autoreverses && (cycles.ceil() as u64) % 2 == 0;
            return Some(if ends_backwards { 0.0 } else { 1.0 });
        }
        let pos = elapsed / d;
        let idx = pos.floor();
        let frac = (pos - idx) as f32;
        let backwards = self.autoreverses && (idx as u64) % 2 == 1;
        Some(if backwards { 1.0 - frac } else { frac })
    }
    /// Whether every cycle has played by clock time `now`. An infinite repeat never finishes.
    pub fn is_finished(&self, now: f64, start: f64) -> bool {
        self.local_time(now, start) >= self.repeat_count.max(1.0) as f64 * self.duration as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_curve_starts_at_zero_and_ends_at_one() {
        for c in [Curve::Linear, Curve::EaseIn, Curve::EaseOut, Curve::EaseInOut, Curve::Bezier(0.1, 0.9, 0.2, 1.0), Curve::Spring { damping_ratio: 0.5, response: 0.4 }] {
            assert!(c.at(0.0).abs() < 1e-4, "{c:?}");
            assert_eq!(c.at(1.0), 1.0, "{c:?}");
            assert_eq!(c.at(-3.0), c.at(0.0));
            assert_eq!(c.at(7.0), 1.0);
        }
    }

    /// The previous implementation (40 bisection steps), kept as the oracle for the faster solver.
    fn bezier_bisect(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
        let p = Poly::new(x1, y1, x2, y2);
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if p.x(mid) < x { lo = mid } else { hi = mid }
        }
        p.y((lo + hi) / 2.0)
    }

    #[test]
    fn the_newton_solver_matches_bisection_on_many_curves() {
        let ctl = [0.0f32, 0.1, 0.25, 0.42, 0.58, 0.75, 0.9, 1.0];
        let mut worst = 0.0f32;
        for &x1 in &ctl {
            for &x2 in &ctl {
                for &y1 in &[-0.3f32, 0.0, 0.5, 1.0, 1.4] {
                    for &y2 in &[0.0f32, 0.5, 1.0, 1.3] {
                        for i in 1..100 {
                            let x = i as f32 / 100.0;
                            worst = worst.max((bezier(x1, y1, x2, y2, x) - bezier_bisect(x1, y1, x2, y2, x)).abs());
                        }
                    }
                }
            }
        }
        assert!(worst < 2e-4, "worst difference {worst}");
    }

    #[test]
    fn ease_in_is_slow_first_ease_out_is_fast_first_and_all_are_monotonic() {
        assert!(Curve::EaseIn.at(0.25) < 0.25 && Curve::EaseOut.at(0.25) > 0.25);
        assert!((Curve::EaseInOut.at(0.5) - 0.5).abs() < 1e-3, "symmetric about the middle");
        for c in [Curve::EaseIn, Curve::EaseOut, Curve::EaseInOut, Curve::Linear] {
            let mut last = -1.0;
            for i in 0..=100 {
                let v = c.at(i as f32 / 100.0);
                assert!(v >= last - 1e-5, "{c:?} at {i}");
                last = v;
            }
        }
    }

    #[test]
    fn an_underdamped_spring_overshoots_a_critical_one_does_not() {
        let peak = |z: f32| (0..=200).map(|i| Curve::Spring { damping_ratio: z, response: 0.5 }.at(i as f32 / 200.0)).fold(f32::MIN, f32::max);
        assert!(peak(0.4) > 1.05, "bouncy: {}", peak(0.4));
        assert!(peak(1.0) <= 1.0 + 1e-3, "critical: {}", peak(1.0));
        assert!(peak(2.0) <= 1.0 + 1e-3, "overdamped: {}", peak(2.0));
    }

    #[test]
    fn a_spring_is_close_to_rest_just_before_its_end() {
        for z in [0.3, 0.7, 1.0, 1.5] {
            let c = Curve::Spring { damping_ratio: z, response: 0.5 };
            assert!((c.at(0.999) - 1.0).abs() < 0.01, "zeta {z}: {}", c.at(0.999));
        }
    }

    #[test]
    fn animation_interpolates_finishes_and_clamps() {
        let a = Animation::new(10.0, 30.0, 5.0, 2.0, Curve::Linear);
        assert_eq!(a.value(4.0), 10.0);
        assert_eq!(a.value(6.0), 20.0);
        assert!(!a.is_done(6.0) && a.is_done(7.0));
        assert_eq!(a.value(99.0), 30.0);
        assert_eq!(Animation::new(1.0, 2.0, 0.0, 0.0, Curve::Linear).value(0.0), 2.0, "a zero duration jumps to the end");
    }

    #[test]
    fn retargeting_continues_from_the_current_value_without_a_jump() {
        let a = Animation::new(0.0, 100.0, 0.0, 1.0, Curve::EaseInOut);
        let mid = a.value(0.3);
        let b = a.retarget(0.3, 0.0);
        assert!((b.value(0.3) - mid).abs() < 1e-4, "no jump at the moment of retargeting");
        assert_eq!(b.value(99.0), 0.0);
        let s = Animation::spring(0.0, 1.0, 0.0, 0.8, 0.4);
        assert!(s.duration > 0.0 && s.is_done(s.duration as f64 + 0.01) && !s.is_done(0.01));
    }

    #[test]
    fn evenly_spaced_linear_keyframes_interpolate_between_neighbours() {
        let k = Keyframes::new(vec![0.0, 10.0, 0.0], None, vec![], KeyframeMode::Linear, 0.0, 2.0).unwrap();
        assert_eq!(k.key_times, vec![0.0, 0.5, 1.0]);
        assert!((k.value(0.5) - 5.0).abs() < 1e-4, "progress 0.25: halfway up the first segment");
        assert!((k.value(1.0) - 10.0).abs() < 1e-4, "progress 0.5: on the middle key");
        assert!((k.value(1.5) - 5.0).abs() < 1e-4, "progress 0.75: halfway down the second segment");
        assert_eq!(k.value(-1.0), 0.0, "before the start");
        assert_eq!(k.value(99.0), 0.0, "after the end");
        assert!(!k.is_done(1.0) && k.is_done(2.0));
    }

    #[test]
    fn explicit_key_times_place_the_keys_and_discrete_mode_holds_each_value() {
        let k = Keyframes::new(vec![0.0, 10.0, 20.0], Some(vec![0.0, 0.2, 1.0]), vec![], KeyframeMode::Linear, 0.0, 1.0).unwrap();
        assert!((k.value(0.6) - 15.0).abs() < 1e-4, "10 + (20 - 10) * 0.5 through the 0.2..1.0 segment");
        let d = Keyframes::new(vec![1.0, 2.0, 3.0], Some(vec![0.0, 0.5, 1.0]), vec![], KeyframeMode::Discrete, 0.0, 1.0).unwrap();
        assert_eq!(d.value(0.49), 1.0);
        assert_eq!(d.value(0.5), 2.0, "the next value takes over at its key time");
        assert_eq!(d.value(0.99), 2.0);
        assert_eq!(d.value(1.0), 3.0);
    }

    #[test]
    fn per_segment_timing_eases_only_that_segment() {
        let eased = Keyframes::new(vec![0.0, 1.0, 2.0], None, vec![Curve::EaseIn, Curve::Linear], KeyframeMode::Linear, 0.0, 2.0).unwrap();
        assert!(eased.value(0.25) < 0.5, "an ease-in first segment is slow at the start");
        assert!((eased.value(1.0) - 1.0).abs() < 1e-4, "the key still lands on its value");
        assert!((eased.value(1.5) - 1.5).abs() < 1e-4, "the linear second segment is unchanged");
    }

    #[test]
    fn a_zero_duration_keyframe_animation_jumps_to_its_last_value() {
        let k = Keyframes::new(vec![4.0, 8.0], None, vec![], KeyframeMode::Linear, 0.0, 0.0).unwrap();
        assert_eq!(k.value(0.0), 8.0);
        assert!(k.is_done(0.0));
    }

    #[test]
    fn keyframes_with_unusable_keys_are_refused() {
        assert!(Keyframes::new(vec![], None, vec![], KeyframeMode::Linear, 0.0, 1.0).is_none(), "no values");
        assert!(Keyframes::new(vec![1.0, 2.0], Some(vec![0.0]), vec![], KeyframeMode::Linear, 0.0, 1.0).is_none(), "key count");
        assert!(Keyframes::new(vec![1.0, 2.0, 3.0], Some(vec![0.0, 0.8, 0.5]), vec![], KeyframeMode::Linear, 0.0, 1.0).is_none(), "descending");
        assert!(Keyframes::new(vec![1.0, 2.0], Some(vec![0.0, f32::NAN]), vec![], KeyframeMode::Linear, 0.0, 1.0).is_none(), "NaN");
        assert!(Keyframes::new(vec![1.0, 2.0, 3.0], None, vec![Curve::Linear], KeyframeMode::Linear, 0.0, 1.0).is_none(), "timing count");
    }

    #[test]
    fn a_repeated_key_time_never_divides_by_zero() {
        let k = Keyframes::new(vec![0.0, 5.0, 5.0, 9.0], Some(vec![0.0, 0.5, 0.5, 1.0]), vec![], KeyframeMode::Linear, 0.0, 1.0).unwrap();
        for i in 0..=100 {
            assert!(k.value(i as f64 / 100.0).is_finite());
        }
        assert_eq!(k.value(0.5), 5.0, "the second key at 0.5 is reached");
    }

    #[test]
    fn media_timing_waits_for_its_begin_time_and_scales_with_speed() {
        let mut t = MediaTiming::new(1.0);
        t.begin_time = 0.5;
        assert_eq!(t.progress(0.2, 0.0), None, "not begun");
        assert_eq!(t.progress(0.5, 0.0), Some(0.0));
        assert!((t.progress(1.0, 0.0).unwrap() - 0.5).abs() < 1e-5);
        t.begin_time = 0.0;
        t.speed = 2.0;
        assert!((t.progress(0.25, 0.0).unwrap() - 0.5).abs() < 1e-5, "double speed is twice as far");
        assert_eq!(t.progress(0.5, 0.0), Some(1.0));
    }

    #[test]
    fn repeats_wrap_each_cycle_and_then_finish() {
        let mut t = MediaTiming::new(1.0);
        t.repeat_count = 3.0;
        assert!((t.progress(1.25, 0.0).unwrap() - 0.25).abs() < 1e-5, "second cycle");
        assert!((t.progress(2.5, 0.0).unwrap() - 0.5).abs() < 1e-5, "third cycle");
        assert_eq!(t.progress(3.0, 0.0), Some(1.0));
        assert!(!t.is_finished(2.9, 0.0) && t.is_finished(3.0, 0.0));
    }

    #[test]
    fn autoreverses_plays_odd_cycles_backwards_and_ends_at_the_start() {
        let mut t = MediaTiming::new(1.0);
        t.repeat_count = 2.0;
        t.autoreverses = true;
        assert!((t.progress(0.25, 0.0).unwrap() - 0.25).abs() < 1e-5, "forward cycle");
        assert!((t.progress(1.25, 0.0).unwrap() - 0.75).abs() < 1e-5, "backward cycle");
        assert_eq!(t.progress(2.0, 0.0), Some(0.0), "an even cycle count ends where it started");
    }

    #[test]
    fn an_infinite_repeat_never_finishes_and_a_zero_duration_is_instant() {
        let mut t = MediaTiming::new(1.0);
        t.repeat_count = f32::INFINITY;
        assert!((t.progress(1000.3, 0.0).unwrap() - 0.3).abs() < 1e-3);
        assert!(!t.is_finished(1e6, 0.0));
        assert_eq!(MediaTiming::new(0.0).progress(0.0, 0.0), Some(1.0));
    }
}
