//! Window effects: open, close, minimize to a dock icon, restore and zoom, as presenter transforms.
//! Pure math: a window's effect is a `WindowFx` (kind, direction, start time, target rect); `fx(now)`
//! gives the `Fx` the GPU presenter applies to the window's content and shadow. Timing comes from
//! `motion::duration`, so Reduce Motion makes every effect a short cross-fade in place.
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
    /// Reduce Motion was on when the effect began: cross-fade only, no scale, slide or settle.
    pub reduced: bool,
}

/// Smallest scale a minimizing window reaches, so the warp ends near the dock icon size.
const MIN_SCALE: f32 = 0.12;
/// Scale a window opens from and closes to.
const POP_SCALE: f32 = 0.9;

impl WindowFx {
    pub fn new(effect: Effect, start: f64, reduced: bool) -> WindowFx {
        WindowFx { effect, start, reduced }
    }

    pub fn kind(&self) -> Kind {
        if self.reduced {
            return Kind::CrossFade;
        }
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
        if self.reduced {
            // a pure cross-fade: the window stays where it is at full size
            let a = match self.effect {
                Effect::Open | Effect::Restore { .. } => t,
                Effect::Close | Effect::Minimize { .. } => 1.0 - t,
                Effect::Zoom => 1.0,
            };
            return Fx { alpha: a.clamp(0.0, 1.0), ..Fx::IDENTITY };
        }
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

/// What a window may animate (`animation_behavior` on a window; the default is `Default`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AnimationBehavior {
    /// The system picks: every effect.
    #[default]
    Default,
    /// A document window: every effect.
    Document,
    /// A utility panel: open and close only.
    Utility,
    /// An alert: open and close only.
    Alert,
    /// Never animates.
    None,
}

impl AnimationBehavior {
    pub fn parse(s: &str) -> AnimationBehavior {
        match s {
            "document" => AnimationBehavior::Document,
            "utility" => AnimationBehavior::Utility,
            "alert" => AnimationBehavior::Alert,
            "none" => AnimationBehavior::None,
            _ => AnimationBehavior::Default,
        }
    }

    pub fn allows(self, e: &Effect) -> bool {
        match self {
            AnimationBehavior::Default | AnimationBehavior::Document => true,
            AnimationBehavior::Utility | AnimationBehavior::Alert => matches!(e, Effect::Open | Effect::Close),
            AnimationBehavior::None => false,
        }
    }
}

/// The two Dock minimize effects. The funnel warp needs a mesh the presenter cannot draw inside the
/// window's own drawable, so both run the scale-and-slide transform today (a recorded gap).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinimizeEffect {
    Funnel,
    Scale,
}

impl MinimizeEffect {
    /// From the `dock_minimize_effect` index (`pages::MINIMIZE_EFFECTS`).
    pub fn from_index(i: usize) -> MinimizeEffect {
        if i == 1 {
            MinimizeEffect::Scale
        } else {
            MinimizeEffect::Funnel
        }
    }
}

/// Offset from the window centre to the dock tile centre, both as (x, y, w, h) in px.
pub fn minimize_target(win: (i32, i32, i32, i32), tile: (i32, i32, i32, i32)) -> (f32, f32) {
    let c = |r: (i32, i32, i32, i32)| (r.0 as f32 + r.2 as f32 / 2.0, r.1 as f32 + r.3 as f32 / 2.0);
    let (wx, wy) = c(win);
    let (tx, ty) = c(tile);
    (tx - wx, ty - wy)
}

/// The running effects, by window id. Holds nothing once they end, so an idle desktop has no work.
#[derive(Default, Debug)]
pub struct FxTracker {
    live: Vec<(u32, WindowFx)>,
}

impl FxTracker {
    /// Starts `effect` for window `id` if `behavior` allows it; `reduce` is Reduce Motion.
    /// Returns whether it started (Zoom is skipped under Reduce Motion).
    pub fn begin(&mut self, id: u32, effect: Effect, behavior: AnimationBehavior, reduce: bool, now: f64) -> bool {
        if !behavior.allows(&effect) || (reduce && effect == Effect::Zoom) {
            return false;
        }
        self.live.retain(|(i, _)| *i != id);
        self.live.push((id, WindowFx::new(effect, now, reduce)));
        true
    }

    pub fn get(&self, id: u32) -> Option<&WindowFx> {
        self.live.iter().find(|(i, _)| *i == id).map(|(_, f)| f)
    }

    /// The transform for `id` at `now`; identity when none runs.
    pub fn fx(&self, id: u32, now: f64, max_shift: f32) -> Fx {
        self.get(id).map_or(Fx::IDENTITY, |f| f.fx(now, max_shift))
    }

    /// True while an effect for `id` still runs, so a minimizing window stays mapped until it ends.
    pub fn keeps_visible(&self, id: u32, now: f64) -> bool {
        self.get(id).is_some_and(|f| !f.done(now))
    }

    /// Drops the effects that ended and returns their ids; an ended effect is never queried again.
    pub fn finish(&mut self, now: f64) -> Vec<u32> {
        let done: Vec<u32> = self.live.iter().filter(|(_, f)| f.done(now)).map(|(i, _)| *i).collect();
        self.live.retain(|(_, f)| !f.done(now));
        done
    }

    /// Whether any effect still needs frames (the shell wakes only while this is true).
    pub fn active(&self, now: f64) -> bool {
        self.live.iter().any(|(_, f)| !f.done(now))
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    pub fn forget(&mut self, id: u32) {
        self.live.retain(|(i, _)| *i != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_ends_at_identity_and_close_ends_invisible() {
        let o = WindowFx::new(Effect::Open, 0.0, false);
        let f = o.fx(100.0, 40.0);
        assert!((f.scale - 1.0).abs() < 1e-4 && (f.alpha - 1.0).abs() < 1e-4 && (f.shadow - 1.0).abs() < 1e-4);
        let c = WindowFx::new(Effect::Close, 0.0, false);
        assert!(c.fx(100.0, 40.0).alpha < 0.01);
        assert!(c.ends_hidden() && o.done(100.0));
    }

    #[test]
    fn minimize_moves_toward_the_dock_and_is_clamped() {
        let m = WindowFx::new(Effect::Minimize { to_dx: 500.0, to_dy: 900.0 }, 0.0, false);
        let f = m.fx(100.0, 40.0);
        assert_eq!((f.dx, f.dy), (40.0, 40.0));
        assert!(f.scale < 0.2 && f.alpha < 0.01);
    }

    #[test]
    fn restore_returns_to_identity() {
        let r = WindowFx::new(Effect::Restore { from_dx: 10.0, from_dy: 20.0 }, 0.0, false);
        let f = r.fx(100.0, 40.0);
        assert!((f.scale - 1.0).abs() < 1e-4 && (f.alpha - 1.0).abs() < 1e-4 && f.dx.abs() < 1e-4);
    }

    #[test]
    fn effects_are_running_just_after_the_start_and_done_after_their_duration() {
        for e in [Effect::Open, Effect::Close, Effect::Minimize { to_dx: 5.0, to_dy: 5.0 }, Effect::Restore { from_dx: 5.0, from_dy: 5.0 }] {
            let f = WindowFx::new(e, 10.0, false);
            assert!(!f.done(10.01), "{e:?} runs at first");
            assert!(f.done(11.0), "{e:?} ends within a second");
        }
    }

    #[test]
    fn minimize_target_is_the_centre_offset() {
        // window centre (200, 150), tile centre (420, 520)
        assert_eq!(minimize_target((100, 100, 200, 100), (400, 500, 40, 40)), (220.0, 370.0));
    }

    #[test]
    fn reduce_motion_cross_fades_in_place() {
        let m = WindowFx::new(Effect::Minimize { to_dx: 300.0, to_dy: 300.0 }, 0.0, true);
        assert_eq!(m.kind(), Kind::CrossFade);
        let mid = m.fx(0.06, 40.0);
        assert_eq!((mid.scale, mid.dx, mid.dy), (1.0, 0.0, 0.0));
        assert!(mid.alpha > 0.0 && mid.alpha < 1.0);
        assert!(m.done(0.5) && !m.done(0.05));
        let mut t = FxTracker::default();
        assert!(!t.begin(1, Effect::Zoom, AnimationBehavior::Default, true, 0.0));
        assert!(t.begin(1, Effect::Open, AnimationBehavior::Default, true, 0.0));
    }

    #[test]
    fn behavior_none_never_animates() {
        let mut t = FxTracker::default();
        for e in [Effect::Open, Effect::Close, Effect::Zoom, Effect::Minimize { to_dx: 1.0, to_dy: 1.0 }] {
            assert!(!t.begin(1, e, AnimationBehavior::None, false, 0.0));
        }
        assert!(!t.active(0.0) && t.fx(1, 0.0, 40.0).is_identity());
        assert!(!t.begin(2, Effect::Minimize { to_dx: 1.0, to_dy: 1.0 }, AnimationBehavior::Alert, false, 0.0));
        assert!(t.begin(2, Effect::Open, AnimationBehavior::Alert, false, 0.0));
    }

    #[test]
    fn tracker_is_empty_after_effects_end() {
        let mut t = FxTracker::default();
        t.begin(7, Effect::Open, AnimationBehavior::Document, false, 0.0);
        assert!(t.active(0.05) && t.keeps_visible(7, 0.05));
        assert_eq!(t.finish(0.05), Vec::<u32>::new());
        assert_eq!(t.finish(5.0), vec![7]);
        assert!(!t.active(5.0) && t.get(7).is_none());
    }

    #[test]
    fn a_closing_window_shrinks_and_fades_over_its_close_time() {
        // Close lasts 0.16 s on an ease-out curve: halfway (0.08 s) the progress is 1 - (1 - 0.5)^3 = 0.875
        let c = WindowFx::new(Effect::Close, 0.0, false);
        let f = c.fx(0.08, 40.0);
        assert!((f.alpha - 0.125).abs() < 1e-3, "alpha {}", f.alpha);
        assert!((f.scale - (1.0 - 0.1 * 0.875)).abs() < 1e-3, "scale {}", f.scale);
        assert!(!c.done(0.08) && c.done(0.5), "the close is kept only while it runs");
        let mut t = FxTracker::default();
        assert!(t.begin(3, Effect::Close, AnimationBehavior::Utility, false, 0.0), "utility windows close with an effect");
        assert!(!t.begin(4, Effect::Minimize { to_dx: 1.0, to_dy: 1.0 }, AnimationBehavior::Utility, false, 0.0));
    }

    #[test]
    fn minimize_effect_index_follows_the_settings_list() {
        assert_eq!(MinimizeEffect::from_index(0), MinimizeEffect::Funnel);
        assert_eq!(MinimizeEffect::from_index(1), MinimizeEffect::Scale);
        assert_eq!(AnimationBehavior::parse("none"), AnimationBehavior::None);
        assert_eq!(AnimationBehavior::parse("x"), AnimationBehavior::Default);
    }
}
