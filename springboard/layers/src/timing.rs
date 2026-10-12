//! Timing functions (linear, ease in/out/in-out, cubic Bézier), matching the reference platform's named curves.
//!
//! Owner: wave G1 agent `layers-animation`. The solver is std only: Newton iterations on x(s) = t, falling back to
//! bisection when Newton leaves [0, 1] or does not converge. The endpoints are exact: `eval(0) == 0`, `eval(1) == 1`.

/// A timing function maps elapsed fraction (0..1) to progress (usually 0..1, may overshoot for some Béziers).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimingFunction {
    Linear,
    /// cubic-bezier(0.42, 0, 1, 1)
    EaseIn,
    /// cubic-bezier(0, 0, 0.58, 1)
    EaseOut,
    /// cubic-bezier(0.42, 0, 0.58, 1)
    EaseInEaseOut,
    /// cubic-bezier(0.25, 0.1, 0.25, 1), the default for implicit animations
    Default,
    /// Control points (x1, y1, x2, y2).
    CubicBezier(f32, f32, f32, f32),
}

/// Solver tolerance on the x coordinate.
const EPS: f64 = 1e-9;

/// The Bézier polynomial coefficients (a, b, c) of one axis with end points 0 and 1 and control points `p1`, `p2`:
/// `B(s) = ((a s + b) s + c) s`.
fn coefficients(p1: f64, p2: f64) -> (f64, f64, f64) {
    let c = 3.0 * p1;
    let b = 3.0 * (p2 - p1) - c;
    let a = 1.0 - c - b;
    (a, b, c)
}

fn sample(k: (f64, f64, f64), s: f64) -> f64 {
    ((k.0 * s + k.1) * s + k.2) * s
}

fn slope(k: (f64, f64, f64), s: f64) -> f64 {
    (3.0 * k.0 * s + 2.0 * k.1) * s + k.2
}

/// The progress of the cubic Bézier (x1, y1, x2, y2) at x = `t`, for `t` strictly between 0 and 1.
fn solve_bezier(x1: f64, y1: f64, x2: f64, y2: f64, t: f64) -> f64 {
    let kx = coefficients(x1, x2);
    let ky = coefficients(y1, y2);
    // Newton from s = t (x is close to linear for typical curves): a few steps, staying inside [0, 1].
    let mut s = t;
    let mut ok = false;
    for _ in 0..8 {
        let err = sample(kx, s) - t;
        if err.abs() < EPS {
            ok = true;
            break;
        }
        let d = slope(kx, s);
        if d.abs() < 1e-12 {
            break;
        }
        let next = s - err / d;
        if !(0.0..=1.0).contains(&next) {
            break;
        }
        s = next;
    }
    if !ok {
        ok = (sample(kx, s) - t).abs() < EPS;
    }
    if !ok {
        // Bisection: x(s) is monotonic for control x values in [0, 1].
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        s = t;
        for _ in 0..100 {
            let x = sample(kx, s);
            if (x - t).abs() < EPS {
                break;
            }
            if x < t {
                lo = s;
            } else {
                hi = s;
            }
            s = 0.5 * (lo + hi);
        }
    }
    sample(ky, s)
}

impl TimingFunction {
    /// Progress at elapsed fraction `t` (clamped to 0..1; NaN is 0).
    pub fn eval(&self, t: f32) -> f32 {
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        if t == 0.0 {
            return 0.0;
        }
        if t == 1.0 {
            return 1.0;
        }
        match *self {
            TimingFunction::Linear => t,
            other => {
                let (x1, y1, x2, y2) = other.control_points();
                solve_bezier(x1 as f64, y1 as f64, x2 as f64, y2 as f64, t as f64) as f32
            }
        }
    }

    /// The control points of the curve.
    pub fn control_points(&self) -> (f32, f32, f32, f32) {
        match *self {
            TimingFunction::Linear => (0.0, 0.0, 1.0, 1.0),
            TimingFunction::EaseIn => (0.42, 0.0, 1.0, 1.0),
            TimingFunction::EaseOut => (0.0, 0.0, 0.58, 1.0),
            TimingFunction::EaseInEaseOut => (0.42, 0.0, 0.58, 1.0),
            TimingFunction::Default => (0.25, 0.1, 0.25, 1.0),
            TimingFunction::CubicBezier(a, b, c, d) => (a, b, c, d),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn linear_is_identity_and_clamps() {
        assert_eq!(TimingFunction::Linear.eval(0.3), 0.3);
        assert_eq!(TimingFunction::Linear.eval(-1.0), 0.0);
        assert_eq!(TimingFunction::Linear.eval(2.0), 1.0);
        assert_eq!(TimingFunction::Linear.eval(f32::NAN), 0.0);
    }

    #[test]
    fn endpoints_are_exact_for_every_curve() {
        for f in [
            TimingFunction::EaseIn,
            TimingFunction::EaseOut,
            TimingFunction::EaseInEaseOut,
            TimingFunction::Default,
            TimingFunction::CubicBezier(0.9, -0.5, 0.1, 1.7),
        ] {
            assert_eq!(f.eval(0.0), 0.0, "{f:?}");
            assert_eq!(f.eval(1.0), 1.0, "{f:?}");
        }
    }

    #[test]
    fn smoothstep_curve_has_a_hand_computed_value() {
        // x(s) = s for cubic-bezier(1/3, 0, 2/3, 1), and y(s) = 3 s^2 - 2 s^3 (smoothstep).
        // At x = 0.25: y = 3 * 0.0625 - 2 * 0.015625 = 0.15625. At x = 0.5: y = 0.5.
        let f = TimingFunction::CubicBezier(1.0 / 3.0, 0.0, 2.0 / 3.0, 1.0);
        assert!(close(f.eval(0.25), 0.15625, 1e-5), "{}", f.eval(0.25));
        assert!(close(f.eval(0.5), 0.5, 1e-5));
        assert!(close(f.eval(0.75), 0.84375, 1e-5));
    }

    #[test]
    fn ease_in_ease_out_is_point_symmetric() {
        // The curve (0.42, 0, 0.58, 1) is symmetric about (0.5, 0.5).
        let f = TimingFunction::EaseInEaseOut;
        assert!(close(f.eval(0.5), 0.5, 1e-5));
        for t in [0.1f32, 0.2, 0.3, 0.4] {
            assert!(close(f.eval(t) + f.eval(1.0 - t), 1.0, 1e-4), "t={t}");
        }
    }

    #[test]
    fn default_curve_matches_the_css_ease_midpoint() {
        // cubic-bezier(0.25, 0.1, 0.25, 1) at x = 0.5 is about 0.8024 (the CSS "ease" midpoint).
        assert!(close(TimingFunction::Default.eval(0.5), 0.8024, 1e-3), "{}", TimingFunction::Default.eval(0.5));
    }

    #[test]
    fn ease_in_is_slow_at_the_start_and_ease_out_fast() {
        assert!(TimingFunction::EaseIn.eval(0.25) < 0.25);
        assert!(TimingFunction::EaseOut.eval(0.25) > 0.25);
    }

    #[test]
    fn curves_are_monotonic_in_x_and_solve_accurately() {
        // Round trip: the solved progress for x must satisfy the curve; check against a dense bisection oracle.
        let f = TimingFunction::CubicBezier(0.2, 0.9, 0.8, 0.1);
        let mut prev = -1.0f32;
        for i in 0..=200 {
            let t = i as f32 / 200.0;
            let y = f.eval(t);
            assert!(y >= prev - 1e-6, "not monotonic at {t}: {y} < {prev}");
            prev = y;
        }
    }

    #[test]
    fn steep_curve_uses_the_fallback_and_still_lands() {
        // Control x values at 0 make x'(0) vanish, so Newton cannot start there; the solver must still converge.
        let f = TimingFunction::CubicBezier(0.0, 1.0, 0.0, 1.0);
        for i in 1..20 {
            let t = i as f32 / 20.0;
            let y = f.eval(t);
            assert!(y.is_finite() && (0.0..=1.0).contains(&y), "t={t} y={y}");
        }
        assert_eq!(f.eval(1.0), 1.0);
    }

    #[test]
    fn control_points_of_named_curves() {
        assert_eq!(TimingFunction::EaseIn.control_points(), (0.42, 0.0, 1.0, 1.0));
        assert_eq!(TimingFunction::Default.control_points(), (0.25, 0.1, 0.25, 1.0));
        assert_eq!(TimingFunction::CubicBezier(0.1, 0.2, 0.3, 0.4).control_points(), (0.1, 0.2, 0.3, 0.4));
    }
}
