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
        if self.duration <= 0.0 { 1.0 } else { ((now - self.start) as f32 / self.duration).clamp(0.0, 1.0) }
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
}
