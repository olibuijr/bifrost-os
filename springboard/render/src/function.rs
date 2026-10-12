//! Sampled one-input colour functions and the axial colour built on them. A `Function` maps a
//! domain interval to a colour by linear interpolation between evenly spaced samples. Inputs
//! outside the domain take the nearest end sample. `axial` projects a point onto a line and
//! evaluates the function at the resulting offset, which gives a multi-stop linear shading. Pure:
//! a renderer evaluates it per pixel or per row, the same way it evaluates `gradient::Ramp`.

use crate::colorfx::Rgb;
use crate::gradient::linear_t;

/// A piecewise-linear colour function over the domain `(t0, t1)`, with samples spaced evenly
/// across the domain. Construct it with `new`.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    domain: (f32, f32),
    samples: Vec<Rgb>,
}

impl Function {
    /// A function over `domain` with at least two samples. `None` when the domain is empty or
    /// not finite, or when there are fewer than two samples.
    pub fn new(domain: (f32, f32), samples: Vec<Rgb>) -> Option<Self> {
        let (t0, t1) = domain;
        if !t0.is_finite() || !t1.is_finite() || t1 <= t0 || samples.len() < 2 {
            return None;
        }
        Some(Function { domain, samples })
    }

    /// The colour at input `t`. Inputs below the domain take the first sample, inputs above it
    /// take the last, and NaN is treated as the start of the domain.
    pub fn eval(&self, t: f32) -> Rgb {
        let (t0, t1) = self.domain;
        let t = if t.is_nan() { t0 } else { t.clamp(t0, t1) };
        let last = self.samples.len() - 1;
        // Position in sample units, from 0 to `last`.
        let p = (t - t0) / (t1 - t0) * last as f32;
        let i = (p.floor() as usize).min(last - 1);
        let frac = p - i as f32;
        self.samples[i].mix(self.samples[i + 1], frac)
    }
}

/// The colour of the point (x, y) under an axial shading: the function evaluated at the point's
/// offset along the line from `from` to `to` (0 at `from`, 1 at `to`). A zero-length line gives
/// the start of the function's domain.
pub fn axial(function: &Function, from: (f32, f32), to: (f32, f32), x: f32, y: f32) -> Rgb {
    function.eval(linear_t(from, to, x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Rgb, r: f32, g: f32, b: f32) -> bool {
        (a.r - r).abs() < 1e-5 && (a.g - g).abs() < 1e-5 && (a.b - b).abs() < 1e-5
    }

    fn black_to_white() -> Function {
        Function::new((0.0, 1.0), vec![Rgb::new(0.0, 0.0, 0.0), Rgb::new(1.0, 1.0, 1.0)]).unwrap()
    }

    #[test]
    fn two_samples_interpolate_linearly() {
        let f = black_to_white();
        assert!(close(f.eval(0.0), 0.0, 0.0, 0.0));
        assert!(close(f.eval(0.5), 0.5, 0.5, 0.5));
        assert!(close(f.eval(1.0), 1.0, 1.0, 1.0));
    }

    #[test]
    fn inputs_outside_the_domain_take_the_end_samples() {
        let f = black_to_white();
        assert!(close(f.eval(-3.0), 0.0, 0.0, 0.0));
        assert!(close(f.eval(2.0), 1.0, 1.0, 1.0));
        // NaN is treated as the start of the domain.
        assert!(close(f.eval(f32::NAN), 0.0, 0.0, 0.0));
    }

    #[test]
    fn three_samples_across_a_wider_domain() {
        // Samples at t = 0, 1, 2: red, green, blue.
        let f = Function::new(
            (0.0, 2.0),
            vec![Rgb::new(1.0, 0.0, 0.0), Rgb::new(0.0, 1.0, 0.0), Rgb::new(0.0, 0.0, 1.0)],
        )
        .unwrap();
        // t = 1 is exactly the middle sample.
        assert!(close(f.eval(1.0), 0.0, 1.0, 0.0));
        // t = 0.25 is a quarter of the way from red to green.
        assert!(close(f.eval(0.25), 0.75, 0.25, 0.0));
        // t = 1.5 is halfway from green to blue.
        assert!(close(f.eval(1.5), 0.0, 0.5, 0.5));
    }

    #[test]
    fn axial_uses_the_projection_onto_the_line() {
        let f = black_to_white();
        // (5, 3) projects to 0.5 on the line from (0,0) to (10,0), whatever its distance from it.
        assert!(close(axial(&f, (0.0, 0.0), (10.0, 0.0), 5.0, 3.0), 0.5, 0.5, 0.5));
        // Past the end of the line the colour is clamped to the last sample.
        assert!(close(axial(&f, (0.0, 0.0), (10.0, 0.0), 40.0, 0.0), 1.0, 1.0, 1.0));
    }

    #[test]
    fn invalid_domains_and_sample_counts_are_rejected() {
        let two = vec![Rgb::new(0.0, 0.0, 0.0), Rgb::new(1.0, 1.0, 1.0)];
        assert!(Function::new((1.0, 1.0), two.clone()).is_none());
        assert!(Function::new((1.0, 0.0), two.clone()).is_none());
        assert!(Function::new((0.0, f32::INFINITY), two.clone()).is_none());
        assert!(Function::new((0.0, 1.0), vec![Rgb::new(0.0, 0.0, 0.0)]).is_none());
    }
}
