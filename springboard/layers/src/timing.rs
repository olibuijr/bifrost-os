//! Timing functions (linear, ease in/out/in-out, cubic Bézier), matching the reference platform's named curves.
//!
//! Owner: wave G1 agent `layers-animation`. Reuse the cubic solver in `bifrost-render::motion` semantics; this crate
//! is std only, so the solver is implemented here (Newton iterations with a bisection fallback).

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

impl TimingFunction {
    /// Progress at elapsed fraction `t` (clamped to 0..1).
    pub fn eval(&self, t: f32) -> f32 {
        let _ = t;
        todo!("layers-animation")
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
