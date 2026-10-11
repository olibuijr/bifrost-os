//! Numerical definite integration: adaptive Simpson quadrature over a finite interval, with the infinite ends handled by a
//! change of variables. The caller gives a scalar integrand and absolute and relative tolerances; the result carries the
//! estimated absolute error, so callers can tell a converged value from one that hit the subdivision limit.
//! Pure arithmetic: no allocation beyond the recursion stack.

/// Why an integration was refused or did not converge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum QuadratureError {
    /// A bound is NaN, or both bounds are the same infinity, or the lower bound is above the upper bound.
    InvalidBounds,
    /// A tolerance is negative or NaN.
    InvalidTolerance,
    /// The integrand returned NaN or infinity at a point inside the interval.
    NonFiniteValue,
    /// The subdivision limit was reached before the tolerance was met. The best estimate is attached.
    NotConverged { value: f64, estimated_error: f64 },
}

/// A converged integration result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Integral {
    /// The estimated integral.
    pub value: f64,
    /// The estimated absolute error of `value`.
    pub estimated_error: f64,
}

/// Maximum recursion depth of the adaptive scheme. Each level halves the panel, so this bounds the work.
const MAX_DEPTH: u32 = 40;

/// Integrates `f` over `[lower, upper]`. Either bound may be infinite. For a finite interval the adaptive Simpson rule is
/// used directly. For an infinite end the integral is mapped onto a finite interval (`x = a + t / (1 - t)` for an upper
/// bound of infinity, and a reflection for a lower bound of negative infinity). At the mapped endpoint where x is
/// infinite, the integrand is taken to be zero, which is the limit for any integrable tail.
pub fn integrate(
    lower: f64,
    upper: f64,
    absolute_tolerance: f64,
    relative_tolerance: f64,
    f: impl Fn(f64) -> f64,
) -> Result<Integral, QuadratureError> {
    if lower.is_nan() || upper.is_nan() || lower > upper || (lower == upper && lower.is_infinite()) {
        return Err(QuadratureError::InvalidBounds);
    }
    if !(absolute_tolerance >= 0.0) || !(relative_tolerance >= 0.0) {
        return Err(QuadratureError::InvalidTolerance);
    }
    if lower == upper {
        return Ok(Integral { value: 0.0, estimated_error: 0.0 });
    }
    let tol = Tolerance { absolute: absolute_tolerance, relative: relative_tolerance };
    match (lower.is_infinite(), upper.is_infinite()) {
        (false, false) => finite(lower, upper, &f, tol),
        (false, true) => tail_up(lower, &f, tol),
        (true, false) => {
            // The integral over (-inf, b] equals the integral of f(-u) over [-b, inf).
            let reflected = |u: f64| f(-u);
            tail_up(-upper, &reflected, tol)
        }
        (true, true) => {
            let left = {
                let reflected = |u: f64| f(-u);
                tail_up(0.0, &reflected, tol)?
            };
            let right = tail_up(0.0, &f, tol)?;
            Ok(Integral {
                value: left.value + right.value,
                estimated_error: left.estimated_error + right.estimated_error,
            })
        }
    }
}

#[derive(Clone, Copy)]
struct Tolerance {
    absolute: f64,
    relative: f64,
}

fn finite(a: f64, b: f64, f: &dyn Fn(f64) -> f64, tol: Tolerance) -> Result<Integral, QuadratureError> {
    let (fa, fb) = (checked(f(a))?, checked(f(b))?);
    let m = 0.5 * (a + b);
    let fm = checked(f(m))?;
    let whole = simpson(a, b, fa, fm, fb);
    let eps = tol.absolute.max(tol.relative * whole.abs());
    let mut budget = Budget { remaining: MAX_DEPTH as u64 * 4096 };
    let (value, error, ok) = refine(f, a, b, fa, fm, fb, whole, eps, MAX_DEPTH, &mut budget)?;
    if ok {
        Ok(Integral { value, estimated_error: error })
    } else {
        Err(QuadratureError::NotConverged { value, estimated_error: error })
    }
}

/// Integrates `f` over `[a, inf)` through `x = a + t / (1 - t)`, `t` in `[0, 1]`.
fn tail_up(a: f64, f: &dyn Fn(f64) -> f64, tol: Tolerance) -> Result<Integral, QuadratureError> {
    let g = |t: f64| {
        if t >= 1.0 {
            return 0.0;
        }
        let s = 1.0 - t;
        f(a + t / s) / (s * s)
    };
    finite(0.0, 1.0, &g, tol)
}

fn checked(v: f64) -> Result<f64, QuadratureError> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(QuadratureError::NonFiniteValue)
    }
}

fn simpson(a: f64, b: f64, fa: f64, fm: f64, fb: f64) -> f64 {
    (b - a) / 6.0 * (fa + 4.0 * fm + fb)
}

struct Budget {
    remaining: u64,
}

/// Splits a panel until the two halves agree with the whole panel to within `eps`. Returns the refined value, the
/// estimated error, and whether every panel met its tolerance.
#[allow(clippy::too_many_arguments)]
fn refine(
    f: &dyn Fn(f64) -> f64,
    a: f64,
    b: f64,
    fa: f64,
    fm: f64,
    fb: f64,
    whole: f64,
    eps: f64,
    depth: u32,
    budget: &mut Budget,
) -> Result<(f64, f64, bool), QuadratureError> {
    let m = 0.5 * (a + b);
    let lm = 0.5 * (a + m);
    let rm = 0.5 * (m + b);
    let flm = checked(f(lm))?;
    let frm = checked(f(rm))?;
    let left = simpson(a, m, fa, flm, fm);
    let right = simpson(m, b, fm, frm, fb);
    let delta = left + right - whole;
    budget.remaining = budget.remaining.saturating_sub(2);
    if delta.abs() <= 15.0 * eps {
        return Ok((left + right + delta / 15.0, delta.abs() / 15.0, true));
    }
    if depth == 0 || budget.remaining == 0 {
        return Ok((left + right + delta / 15.0, delta.abs() / 15.0, false));
    }
    let (lv, le, lok) = refine(f, a, m, fa, flm, fm, left, eps / 2.0, depth - 1, budget)?;
    let (rv, re, rok) = refine(f, m, b, fm, frm, fb, right, eps / 2.0, depth - 1, budget)?;
    Ok((lv + rv, le + re, lok && rok))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f64 = 1e-9;

    fn value(r: Result<Integral, QuadratureError>) -> f64 {
        r.expect("integral should converge").value
    }

    #[test]
    fn polynomial_over_a_finite_interval_is_exact() {
        let v = value(integrate(0.0, 1.0, TOL, TOL, |x| x * x));
        assert!((v - 1.0 / 3.0).abs() < 1e-9, "got {v}");
    }

    #[test]
    fn sine_over_half_a_period_is_two() {
        let v = value(integrate(0.0, std::f64::consts::PI, TOL, TOL, f64::sin));
        assert!((v - 2.0).abs() < 1e-8, "got {v}");
    }

    #[test]
    fn exponential_tail_to_infinity_is_one() {
        let v = value(integrate(0.0, f64::INFINITY, TOL, TOL, |x| (-x).exp()));
        assert!((v - 1.0).abs() < 1e-6, "got {v}");
    }

    #[test]
    fn gaussian_over_the_whole_line_is_root_pi() {
        let v = value(integrate(f64::NEG_INFINITY, f64::INFINITY, TOL, TOL, |x| (-x * x).exp()));
        assert!((v - std::f64::consts::PI.sqrt()).abs() < 1e-6, "got {v}");
    }

    #[test]
    fn lower_infinite_bound_uses_the_reflected_integrand() {
        let v = value(integrate(f64::NEG_INFINITY, 0.0, TOL, TOL, |x| (x).exp()));
        assert!((v - 1.0).abs() < 1e-6, "got {v}");
    }

    #[test]
    fn reversed_or_nan_bounds_are_refused() {
        assert_eq!(integrate(1.0, 0.0, TOL, TOL, |x| x).err(), Some(QuadratureError::InvalidBounds));
        assert_eq!(integrate(f64::NAN, 1.0, TOL, TOL, |x| x).err(), Some(QuadratureError::InvalidBounds));
        assert_eq!(
            integrate(f64::INFINITY, f64::INFINITY, TOL, TOL, |x| x).err(),
            Some(QuadratureError::InvalidBounds)
        );
    }

    #[test]
    fn negative_tolerance_is_refused() {
        assert_eq!(integrate(0.0, 1.0, -1.0, TOL, |x| x).err(), Some(QuadratureError::InvalidTolerance));
    }

    #[test]
    fn non_finite_integrand_inside_the_interval_is_reported() {
        let r = integrate(0.0, 1.0, TOL, TOL, |x| 1.0 / (x - 0.5));
        assert_eq!(r.err(), Some(QuadratureError::NonFiniteValue));
    }

    #[test]
    fn empty_interval_integrates_to_zero() {
        assert_eq!(integrate(2.0, 2.0, TOL, TOL, |x| x).unwrap().value, 0.0);
    }
}
