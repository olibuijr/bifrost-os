//! Symbol effects for system symbol images (`SymbolEffect`, `DiscreteSymbolEffect`, `IndefiniteSymbolEffect`,
//! `SymbolEffectOptions`, `ReplaceSymbolEffect`) as pure functions of time. An effect gives a `Pose` (scale, rotation,
//! opacity and offset) for a glyph at `now` seconds; the renderer applies it. Durations and amounts are our own design
//! values: the reference binary sits in a dyld cache that is not unpacked yet, so nothing here is measured.

use crate::anim::Curve;
use crate::symbol_layers::{layer_opacity, Fill, Inactive};
use std::f32::consts::{PI, TAU};

/// How a glyph is drawn at one instant. `REST` is the untouched glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub scale: f32,
    /// Radians, clockwise.
    pub rotation: f32,
    /// 0 is hidden, 1 is fully drawn.
    pub opacity: f32,
    /// Offset as a fraction of the glyph's width and height.
    pub dx: f32,
    pub dy: f32,
}

impl Pose {
    pub const REST: Pose = Pose { scale: 1.0, rotation: 0.0, opacity: 1.0, dx: 0.0, dy: 0.0 };
    pub const HIDDEN: Pose = Pose { opacity: 0.0, ..Pose::REST };
}

/// Repeat preference (`SymbolEffectOptions.RepeatBehavior`): one cycle, a count of extra cycles, or forever.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    NonRepeating,
    Count(u32),
    Indefinite,
}

/// Options that shape a symbol effect (`SymbolEffectOptions`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    pub repeat: Repeat,
    /// Speed multiplier, always above zero.
    pub speed: f32,
}

impl Default for Options {
    fn default() -> Self {
        Options { repeat: Repeat::NonRepeating, speed: 1.0 }
    }
}

impl Options {
    /// Prefers to repeat until the effect is removed (`repeating`).
    pub fn repeating() -> Self {
        Options { repeat: Repeat::Indefinite, ..Options::default() }
    }
    /// Prefers to repeat `count` extra times after the first cycle (`repeat(_:)`).
    pub fn repeat(count: u32) -> Self {
        Options { repeat: Repeat::Count(count), ..Options::default() }
    }
    /// Sets the speed multiplier (`speed(_:)`). Zero, negative and NaN values fall back to 1.
    pub fn speed(self, speed: f32) -> Self {
        let speed = if speed.is_finite() && speed > 0.0 { speed } else { 1.0 };
        Options { speed, ..self }
    }
}

/// The effects this model can run. Each one is a single cycle, repeated as the options ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Fades and grows the glyph in.
    Appear,
    /// Fades and shrinks the glyph out; it stays hidden when the effect ends.
    Disappear,
    /// A transitory scale bump.
    Bounce,
    /// A vertical hop upward and back (`bounce.up`).
    BounceUp,
    /// A vertical dip downward and back (`bounce.down`).
    BounceDown,
    /// Cycles the opacity of each layer in turn; read it with `EffectFrame::layer_opacity`.
    VariableColor,
    /// Lets the system choose: it resolves to `Bounce`.
    Automatic,
    /// Fades the opacity down and back up.
    Pulse,
    /// Shrinks the glyph and holds it small.
    ScaleDown,
    /// Grows the glyph and holds it large.
    ScaleUp,
    /// A slow scale breath, repeating by default.
    Breathe,
    /// One full turn.
    Rotate,
    /// A damped side-to-side rocking rotation.
    Wiggle,
}

impl Effect {
    /// Length of one cycle in seconds at speed 1.
    pub fn cycle_secs(self) -> f32 {
        match self {
            Effect::Appear | Effect::Disappear => 0.4,
            Effect::Bounce | Effect::BounceUp | Effect::BounceDown | Effect::Automatic => 0.6,
            Effect::VariableColor => 1.2,
            Effect::Pulse => 0.9,
            Effect::ScaleDown | Effect::ScaleUp => 0.35,
            Effect::Breathe => 1.6,
            Effect::Rotate => 0.8,
            Effect::Wiggle => 0.5,
        }
    }

    /// The pose at `t` in 0..=1 of one cycle.
    fn pose_at(self, t: f32) -> Pose {
        let t = t.clamp(0.0, 1.0);
        let ease = Curve::EaseInOut.at(t);
        let mut p = Pose::REST;
        match self {
            Effect::Appear => {
                p.opacity = ease;
                p.scale = 0.6 + 0.4 * ease;
            }
            Effect::Disappear => {
                p.opacity = 1.0 - ease;
                p.scale = 1.0 - 0.4 * ease;
            }
            Effect::Bounce | Effect::Automatic => p.scale = 1.0 + 0.15 * (PI * t).sin(),
            Effect::BounceUp => p.dy = -0.2 * (PI * t).sin(),
            Effect::BounceDown => p.dy = 0.2 * (PI * t).sin(),
            Effect::VariableColor => {}
            Effect::Pulse => p.opacity = 1.0 - 0.5 * (PI * t).sin(),
            Effect::ScaleDown => p.scale = 1.0 - 0.25 * ease,
            Effect::ScaleUp => p.scale = 1.0 + 0.25 * ease,
            Effect::Breathe => p.scale = 1.0 + 0.08 * (TAU * t).sin(),
            Effect::Rotate => p.rotation = TAU * ease,
            Effect::Wiggle => p.rotation = 0.2 * (2.0 * TAU * t).sin() * (1.0 - t),
        }
        p
    }

    /// The pose after the effect has ended and was not removed.
    fn end_pose(self) -> Pose {
        match self {
            Effect::Disappear => Pose::HIDDEN,
            Effect::Appear => Pose::REST,
            // Scale effects that grow or shrink hold their end value.
            Effect::ScaleDown => Pose { scale: 0.75, ..Pose::REST },
            Effect::ScaleUp => Pose { scale: 1.25, ..Pose::REST },
            _ => Pose::REST,
        }
    }
}

/// A symbol effect applied to one glyph from `start` seconds (`SymbolEffect` with its options).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SymbolEffect {
    pub effect: Effect,
    pub options: Options,
    pub start: f64,
}

impl SymbolEffect {
    pub fn new(effect: Effect, options: Options, start: f64) -> Self {
        SymbolEffect { effect, options, start }
    }

    /// Number of cycles to play, or `None` when the effect repeats forever.
    pub fn cycles(&self) -> Option<u32> {
        match self.options.repeat {
            Repeat::NonRepeating => Some(1),
            Repeat::Count(n) => Some(n.saturating_add(1)),
            Repeat::Indefinite => None,
        }
    }

    /// True for a transient effect (`DiscreteSymbolEffect`): it plays its cycles and then rests.
    pub fn is_discrete(&self) -> bool {
        self.cycles().is_some()
    }

    /// True once a discrete effect has played all its cycles. An indefinite effect never finishes.
    pub fn is_done(&self, now: f64) -> bool {
        match self.cycles() {
            None => false,
            Some(n) => self.elapsed_cycles(now) >= n as f32,
        }
    }

    /// Cycles elapsed at `now`, counting speed. Zero before the start.
    fn elapsed_cycles(&self, now: f64) -> f32 {
        let secs = (now - self.start).max(0.0) as f32 * self.options.speed;
        secs / self.effect.cycle_secs()
    }

    /// The pose at `now` seconds: REST before the start, the cycle's pose while it plays, and the effect's end pose
    /// once a discrete effect is done.
    pub fn pose(&self, now: f64) -> Pose {
        if now < self.start {
            return Pose::REST;
        }
        let cycles = self.elapsed_cycles(now);
        if let Some(n) = self.cycles() {
            if cycles >= n as f32 {
                return self.effect.end_pose();
            }
        }
        self.effect.pose_at(cycles.fract())
    }
}

/// Everything a renderer needs for one glyph at one instant: the pose, the variable colour progress and whether the
/// effect has ended (`frame`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectFrame {
    pub scale: f32,
    pub opacity: f32,
    /// Vertical offset as a fraction of the glyph height; negative is up.
    pub y_offset: f32,
    pub rotation: f32,
    /// Progress through the current variable colour cycle, 0..=1; 0 for other effects.
    pub progress: f32,
    pub finished: bool,
}

impl EffectFrame {
    /// Opacity of layer `layer` of `layers`: the layer fill for `VariableColor`, the whole-glyph opacity otherwise.
    pub fn layer_opacity(&self, effect: Effect, layer: usize, layers: usize, fill: Fill, inactive: Inactive) -> f32 {
        if effect == Effect::VariableColor && !self.finished {
            layer_opacity(layer, layers, self.progress, fill, inactive)
        } else if layer < layers {
            self.opacity
        } else {
            0.0
        }
    }
}

/// The frame of `effect` with `options` at `t` seconds after it started.
pub fn frame(effect: Effect, options: Options, t: f64) -> EffectFrame {
    let fx = SymbolEffect::new(effect, options, 0.0);
    let pose = fx.pose(t);
    let finished = fx.is_done(t);
    let progress = if effect == Effect::VariableColor && !finished { fx.elapsed_cycles(t).fract() } else { 0.0 };
    EffectFrame { scale: pose.scale, opacity: pose.opacity, y_offset: pose.dy, rotation: pose.rotation, progress, finished }
}

/// How a symbol is replaced by another (`ReplaceSymbolEffect` variants).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replace {
    /// The old symbol shrinks away, then the new one grows in.
    DownUp,
    /// The old symbol fades out, then the new one grows in.
    OffUp,
    /// The old symbol grows and fades, then the new one grows in from smaller.
    UpUp,
}

/// Poses of the outgoing and incoming symbols at `progress` in 0..=1 of a replace transition. The first half moves the
/// outgoing symbol, the second half the incoming one. At 0 only the outgoing symbol is drawn; at 1 only the incoming.
pub fn replace_poses(variant: Replace, progress: f32) -> (Pose, Pose) {
    let p = progress.clamp(0.0, 1.0);
    let first = Curve::EaseInOut.at((p * 2.0).min(1.0));
    let second = Curve::EaseInOut.at(((p - 0.5) * 2.0).clamp(0.0, 1.0));
    let (out, incoming) = match variant {
        Replace::DownUp => (
            Pose { scale: 1.0 - 0.4 * first, opacity: 1.0 - first, ..Pose::REST },
            Pose { scale: 0.6 + 0.4 * second, opacity: second, ..Pose::REST },
        ),
        Replace::OffUp => (
            Pose { opacity: 1.0 - first, ..Pose::REST },
            Pose { scale: 0.6 + 0.4 * second, opacity: second, ..Pose::REST },
        ),
        Replace::UpUp => (
            Pose { scale: 1.0 + 0.4 * first, opacity: 1.0 - first, ..Pose::REST },
            Pose { scale: 1.4 - 0.4 * second, opacity: second, ..Pose::REST },
        ),
    };
    // Before the handover the outgoing symbol is the only one drawn; after it, only the incoming one.
    let out = if p >= 0.5 { Pose::HIDDEN } else { out };
    let incoming = if p < 0.5 { Pose::HIDDEN } else { incoming };
    (out, incoming)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn options_default_to_one_cycle_at_normal_speed() {
        let o = Options::default();
        assert_eq!(o.repeat, Repeat::NonRepeating);
        assert_eq!(o.speed, 1.0);
        assert_eq!(Options::repeating().repeat, Repeat::Indefinite);
        assert_eq!(Options::repeat(2).repeat, Repeat::Count(2));
        // Speed must stay usable: zero, negative and NaN fall back to 1.
        assert_eq!(Options::default().speed(0.0).speed, 1.0);
        assert_eq!(Options::default().speed(-2.0).speed, 1.0);
        assert_eq!(Options::default().speed(f32::NAN).speed, 1.0);
        assert_eq!(Options::default().speed(2.5).speed, 2.5);
    }

    #[test]
    fn discrete_effect_is_at_rest_before_and_after_its_cycle() {
        let e = SymbolEffect::new(Effect::Bounce, Options::default(), 10.0);
        assert_eq!(e.pose(9.0), Pose::REST);
        assert!(!e.is_done(10.2));
        assert!(e.is_done(10.7));
        // After the one cycle a bounce holds rest, not the mid-cycle bump.
        assert_eq!(e.pose(11.0), Pose::REST);
        assert!(e.is_discrete());
    }

    #[test]
    fn indefinite_effect_never_finishes_and_stays_bounded() {
        let e = SymbolEffect::new(Effect::Breathe, Options::repeating(), 0.0);
        assert_eq!(e.cycles(), None);
        assert!(!e.is_discrete());
        for k in 0..200 {
            let now = k as f64 * 0.37;
            assert!(!e.is_done(now));
            let s = e.pose(now).scale;
            assert!((0.92..=1.08).contains(&s), "scale {s} at {now}");
        }
    }

    #[test]
    fn counted_repeat_plays_one_plus_count_cycles() {
        // repeat(2) plays three cycles of 0.4 s (Appear).
        let e = SymbolEffect::new(Effect::Appear, Options::repeat(2), 0.0);
        assert_eq!(e.cycles(), Some(3));
        assert!(!e.is_done(1.1));
        assert!(e.is_done(1.3));
    }

    #[test]
    fn disappear_ends_hidden_and_appear_ends_drawn() {
        let gone = SymbolEffect::new(Effect::Disappear, Options::default(), 0.0);
        assert_eq!(gone.pose(5.0).opacity, 0.0);
        let mid = SymbolEffect::new(Effect::Disappear, Options::default(), 0.0);
        assert!(mid.pose(0.2).opacity > 0.0 && mid.pose(0.2).opacity < 1.0);
        let shown = SymbolEffect::new(Effect::Appear, Options::default(), 0.0);
        assert_eq!(shown.pose(5.0).opacity, 1.0);
        assert!(close(shown.pose(0.0).opacity, 0.0));
    }

    #[test]
    fn speed_two_halves_the_wall_time_of_a_cycle() {
        let slow = SymbolEffect::new(Effect::Rotate, Options::default(), 0.0);
        let fast = SymbolEffect::new(Effect::Rotate, Options::default().speed(2.0), 0.0);
        // Half of the 0.8 s cycle at speed 1 is the same point in the cycle as 0.2 s at speed 2.
        assert!(close(slow.pose(0.4).rotation, fast.pose(0.2).rotation));
        // Speed 2 finishes the 0.8 s cycle at 0.4 s of wall time; speed 1 has not yet.
        assert!(fast.is_done(0.41));
        assert!(!fast.is_done(0.39));
        assert!(!slow.is_done(0.39));
    }

    #[test]
    fn rotate_turns_a_full_circle_and_wiggle_rocks_both_ways() {
        let spin = SymbolEffect::new(Effect::Rotate, Options::default(), 0.0);
        assert!(spin.pose(0.2).rotation > 0.0 && spin.pose(0.2).rotation < TAU);
        let rock = SymbolEffect::new(Effect::Wiggle, Options::repeating(), 0.0);
        let (mut neg, mut pos) = (false, false);
        for k in 0..100 {
            let r = rock.pose(k as f64 * 0.005).rotation;
            neg |= r < -0.01;
            pos |= r > 0.01;
        }
        assert!(neg && pos);
    }

    #[test]
    fn replace_hands_the_glyph_over_at_half_way() {
        let (out0, in0) = replace_poses(Replace::DownUp, 0.0);
        assert_eq!(out0.opacity, 1.0);
        assert_eq!(in0, Pose::HIDDEN);
        let (out1, in1) = replace_poses(Replace::DownUp, 1.0);
        assert_eq!(out1, Pose::HIDDEN);
        assert_eq!(in1.opacity, 1.0);
        assert!(close(in1.scale, 1.0));
        // Only one symbol is drawn at any progress value.
        let (out, incoming) = replace_poses(Replace::OffUp, 0.3);
        assert!(out.opacity > 0.0 && incoming.opacity == 0.0);
        let (out, incoming) = replace_poses(Replace::UpUp, 0.8);
        assert!(out.opacity == 0.0 && incoming.opacity > 0.0);
    }

    #[test]
    fn bounce_up_and_down_hop_in_opposite_directions_and_rest_at_the_ends() {
        let up = frame(Effect::BounceUp, Options::default(), 0.3);
        let down = frame(Effect::BounceDown, Options::default(), 0.3);
        assert!(up.y_offset < 0.0 && down.y_offset > 0.0);
        assert!(close(up.y_offset, -down.y_offset));
        assert!(close(frame(Effect::BounceUp, Options::default(), 0.0).y_offset, 0.0));
        let end = frame(Effect::BounceDown, Options::default(), 0.61);
        assert!(end.finished && close(end.y_offset, 0.0) && close(end.scale, 1.0));
    }

    #[test]
    fn counted_bounce_finishes_after_two_periods_and_speed_halves_it() {
        let o = Options::repeat(1);
        assert!(!frame(Effect::BounceUp, o, 1.1).finished);
        assert!(frame(Effect::BounceUp, o, 1.21).finished);
        let fast = o.speed(2.0);
        assert!(!frame(Effect::BounceUp, fast, 0.55).finished);
        assert!(frame(Effect::BounceUp, fast, 0.61).finished);
    }

    #[test]
    fn continuous_effect_never_finishes() {
        for k in 0..50 {
            assert!(!frame(Effect::VariableColor, Options::repeating(), k as f64 * 3.7).finished);
        }
    }

    #[test]
    fn automatic_resolves_to_the_bounce_scale_bump() {
        for t in [0.0, 0.15, 0.3, 0.45, 0.6, 1.0] {
            assert_eq!(frame(Effect::Automatic, Options::default(), t), frame(Effect::Bounce, Options::default(), t));
        }
    }

    #[test]
    fn frame_of_appear_and_disappear_ends_match_the_pose() {
        let a = frame(Effect::Appear, Options::default(), 0.0);
        assert!(close(a.opacity, 0.0) && !a.finished);
        let a = frame(Effect::Appear, Options::default(), 1.0);
        assert!(a.finished && a.opacity == 1.0 && a.scale == 1.0);
        assert_eq!(frame(Effect::Disappear, Options::default(), 1.0).opacity, 0.0);
    }

    #[test]
    fn variable_colour_frame_lights_layers_in_turn() {
        let o = Options::repeating();
        let start = frame(Effect::VariableColor, o, 0.0);
        assert_eq!(start.progress, 0.0);
        assert!(close(start.layer_opacity(Effect::VariableColor, 0, 3, Fill::Cumulative, Inactive::Dim), 0.3));
        let late = frame(Effect::VariableColor, o, 1.1);
        assert!(late.progress > 0.9);
        assert!(close(late.layer_opacity(Effect::VariableColor, 0, 3, Fill::Cumulative, Inactive::Dim), 1.0));
        // Other effects report the whole-glyph opacity for every layer; a layer past the count is 0.
        let pulse = frame(Effect::Pulse, o, 0.45);
        assert!(close(pulse.layer_opacity(Effect::Pulse, 1, 3, Fill::Cumulative, Inactive::Dim), pulse.opacity));
        assert_eq!(pulse.layer_opacity(Effect::Pulse, 3, 3, Fill::Cumulative, Inactive::Dim), 0.0);
    }
}
