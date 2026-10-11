//! Window effects: open, close, minimize to a dock icon, restore and zoom, as presenter transforms.
//! Pure math: a window's effect is a `WindowFx` (kind, direction, start time, target rect); `fx(now)`
//! gives the `Fx` the GPU presenter applies to the window's content and shadow. Timing comes from
//! `motion::duration`, so Reduce Motion makes every effect jump to its end state.
use bifrost_render::glass::Fx;
use bifrost_render::motion::{self, Anim, Kind};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    Open,
    Close,
    /// Minimize toward the dock icon centre (px, window-centre relative offset is derived by the caller).
    Minimize { to_dx: f32, to_dy: f32 },
    Restore { from_dx: f32, from_dy: f32 },
    /// Zoom (maximize / restore size): a short settle pulse, the size itself comes from the compositor.
    Zoom,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowFx {
    pub effect: Effect,
    pub start: f64,
}

/// Smallest scale a minimizing window reaches, so the warp ends near the dock icon size.
const MIN_SCALE: f32 = 0.12;
/// Scale a window opens from and closes to.
const POP_SCALE: f32 = 0.9;

impl WindowFx {
    pub fn kind(&self) -> Kind {
        match self.effect {
            Effect::Open => Kind::WindowOpen,
            Effect::Close => Kind::WindowClose,
            Effect::Minimize { .. } | Effect::Restore { .. } => Kind::Minimize,
            Effect::Zoom => Kind::Zoom,
        }
    }

    pub fn done(&self, now: f64) -> bool {
        Anim::new(0.0, 1.0, self.start, self.kind()).done(now)
    }

    /// True once the effect leaves the window invisible (close, minimize): the caller unmaps it.
    pub fn ends_hidden(&self) -> bool {
        matches!(self.effect, Effect::Close | Effect::Minimize { .. })
    }

    /// The transform at `now`. `max_shift` bounds dx, dy (the drawable's shadow margin, px).
    pub fn fx(&self, now: f64, max_shift: f32) -> Fx {
        let t = Anim::new(0.0, 1.0, self.start, self.kind()).value(now);
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        let mut f = match self.effect {
            Effect::Open => Fx { scale: lerp(POP_SCALE, 1.0), alpha: lerp(0.0, 1.0), dx: 0.0, dy: 0.0, shadow: lerp(0.0, 1.0) },
            Effect::Close => Fx { scale: lerp(1.0, POP_SCALE), alpha: lerp(1.0, 0.0), dx: 0.0, dy: 0.0, shadow: lerp(1.0, 0.0) },
            Effect::Minimize { to_dx, to_dy } => {
                // scale shrinks on the curve, the window slides along it; fades over the last third
                let fade = ((t - 0.66) / 0.34).clamp(0.0, 1.0);
                Fx { scale: lerp(1.0, MIN_SCALE), alpha: 1.0 - fade, dx: lerp(0.0, to_dx), dy: lerp(0.0, to_dy), shadow: lerp(1.0, 0.3) }
            }
            Effect::Restore { from_dx, from_dy } => {
                let fade = (t / 0.34).clamp(0.0, 1.0);
                Fx { scale: lerp(MIN_SCALE, 1.0), alpha: fade, dx: lerp(from_dx, 0.0), dy: lerp(from_dy, 0.0), shadow: lerp(0.3, 1.0) }
            }
            // the window dips to 0.985 and returns, the spring gives the settle
            Effect::Zoom => Fx { scale: 1.0 - 0.015 * (std::f32::consts::PI * t).sin(), alpha: 1.0, dx: 0.0, dy: 0.0, shadow: 1.0 },
        };
        f.dx = f.dx.clamp(-max_shift, max_shift);
        f.dy = f.dy.clamp(-max_shift, max_shift);
        f.scale = f.scale.clamp(0.05, 1.0);
        f.alpha = f.alpha.clamp(0.0, 1.0);
        f
    }
}

/// Dimming of an inactive window's shadow: the presenter's `Shadow { focused }` already halves it;
/// this is the extra content-independent factor used while the key window changes (0..1).
pub fn inactive_shadow_factor(focused: bool) -> f32 {
    if focused {
        1.0
    } else {
        0.6
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_ends_at_identity_and_close_ends_invisible() {
        let o = WindowFx { effect: Effect::Open, start: 0.0 };
        let f = o.fx(100.0, 40.0);
        assert!((f.scale - 1.0).abs() < 1e-4 && (f.alpha - 1.0).abs() < 1e-4 && (f.shadow - 1.0).abs() < 1e-4);
        let c = WindowFx { effect: Effect::Close, start: 0.0 };
        assert!(c.fx(100.0, 40.0).alpha < 0.01);
        assert!(c.ends_hidden() && o.done(100.0));
    }

    #[test]
    fn minimize_moves_toward_the_dock_and_is_clamped() {
        let m = WindowFx { effect: Effect::Minimize { to_dx: 500.0, to_dy: 900.0 }, start: 0.0 };
        let f = m.fx(100.0, 40.0);
        assert_eq!((f.dx, f.dy), (40.0, 40.0));
        assert!(f.scale < 0.2 && f.alpha < 0.01);
    }

    #[test]
    fn restore_returns_to_identity() {
        let r = WindowFx { effect: Effect::Restore { from_dx: 10.0, from_dy: 20.0 }, start: 0.0 };
        let f = r.fx(100.0, 40.0);
        assert!((f.scale - 1.0).abs() < 1e-4 && (f.alpha - 1.0).abs() < 1e-4 && f.dx.abs() < 1e-4);
    }
}
