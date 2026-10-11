//! Shared motion contract: curves, durations per kind of animation, and a small `Anim` value.
//! Durations are 0 when Reduce Motion is on, so callers jump to the end state (a cross-fade
//! stays available through `Kind::CrossFade`, which keeps a short 120 ms under Reduce Motion).
use crate::appearance;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    EaseOut,
    EaseInOut,
    /// A damped spring: `response` in seconds, `damping` 0..1+ (1 is critical, no overshoot).
    Spring { response: f32, damping: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    WindowOpen,
    WindowClose,
    Minimize,
    Zoom,
    MenuAppear,
    SheetSlide,
    PanelReveal,
    DockMagnify,
    DockBounce,
    OverviewEnter,
    CrossFade,
}

/// Maps `t` (clamped to 0..1) through the curve; 0 maps to 0 and 1 maps to 1.
pub fn ease(curve: Curve, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match curve {
        Curve::EaseOut => 1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t),
        Curve::EaseInOut => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            }
        }
        Curve::Spring { response, damping } => {
            if t >= 1.0 {
                return 1.0;
            }
            // damped oscillation over the unit interval; a shorter response rings more often
            let zeta = damping.clamp(0.05, 1.5);
            let w = std::f32::consts::TAU * 0.25 / response.max(0.05) * t;
            let decay = (-(zeta * 6.0) * t).exp();
            let v = 1.0 - decay * (w.cos() + zeta * 0.25 * w.sin());
            // blend the last stretch onto 1 so the value lands exactly
            let k = t * t * t * t;
            v * (1.0 - k) + k
        }
    }
}

/// The nominal duration in seconds of `kind`, before Reduce Motion.
pub fn nominal(kind: Kind) -> f32 {
    match kind {
        Kind::WindowOpen => 0.22,
        Kind::WindowClose => 0.16,
        Kind::Minimize => 0.42,
        Kind::Zoom => 0.30,
        Kind::MenuAppear => 0.12,
        Kind::SheetSlide => 0.32,
        Kind::PanelReveal => 0.26,
        Kind::DockMagnify => 0.14,
        Kind::DockBounce => 0.70,
        Kind::OverviewEnter => 0.38,
        Kind::CrossFade => 0.20,
    }
}

/// The duration in seconds; 0 under Reduce Motion (cross-fades keep 120 ms).
pub fn duration(kind: Kind) -> f32 {
    scaled(kind, appearance::reduce_motion())
}

fn scaled(kind: Kind, reduce: bool) -> f32 {
    if !reduce {
        nominal(kind)
    } else if kind == Kind::CrossFade {
        0.12
    } else {
        0.0
    }
}

/// The curve each kind uses by default.
pub fn default_curve(kind: Kind) -> Curve {
    match kind {
        Kind::WindowOpen | Kind::Zoom | Kind::SheetSlide | Kind::DockBounce => Curve::Spring { response: 1.0, damping: 0.9 },
        Kind::Minimize | Kind::OverviewEnter => Curve::EaseInOut,
        _ => Curve::EaseOut,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anim {
    pub from: f32,
    pub to: f32,
    /// start time in seconds on the caller's clock
    pub start: f64,
    pub kind: Kind,
    pub curve: Curve,
}

impl Anim {
    pub fn new(from: f32, to: f32, start: f64, kind: Kind) -> Anim {
        Anim { from, to, start, kind, curve: default_curve(kind) }
    }

    fn progress(&self, now: f64) -> f32 {
        let d = duration(self.kind) as f64;
        if d <= 0.0 {
            1.0
        } else {
            ((now - self.start) / d).clamp(0.0, 1.0) as f32
        }
    }

    pub fn value(&self, now: f64) -> f32 {
        self.from + (self.to - self.from) * ease(self.curve, self.progress(now))
    }

    pub fn done(&self, now: f64) -> bool {
        self.progress(now) >= 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_pin_their_ends_and_clamp() {
        for c in [Curve::EaseOut, Curve::EaseInOut, Curve::Spring { response: 1.0, damping: 0.9 }] {
            assert!(ease(c, 0.0).abs() < 1e-3);
            assert!((ease(c, 1.0) - 1.0).abs() < 1e-4);
            assert_eq!(ease(c, 2.0), ease(c, 1.0));
            assert_eq!(ease(c, -1.0), ease(c, 0.0));
        }
    }

    #[test]
    fn reduce_motion_collapses_durations() {
        assert_eq!(scaled(Kind::WindowOpen, true), 0.0);
        assert!(scaled(Kind::CrossFade, true) > 0.0);
        assert!(scaled(Kind::WindowOpen, false) > 0.0);
    }

    #[test]
    fn anim_runs_from_to() {
        let a = Anim { from: 0.9, to: 1.0, start: 10.0, kind: Kind::WindowOpen, curve: Curve::EaseOut };
        assert!((a.value(20.0) - 1.0).abs() < 1e-4);
        assert!(a.done(20.0));
    }
}
