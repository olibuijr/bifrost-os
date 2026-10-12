//! Animations: basic (from/to), keyframe, spring and group, with media timing (begin time, duration, speed, time
//! offset, repeat count, autoreverse, fill mode). Evaluated by `Tree::presentation`.
//!
//! Owner: wave G1 agent `layers-animation`.

use crate::geometry::{Affine, Color, Point, Rect};
use crate::layer::{Property, Shadow};
use crate::timing::TimingFunction;

/// An animation's id within a tree; reported back in `animation_finished`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AnimationId(pub u32);

/// An animatable value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Scalar(f32),
    Point(Point),
    Rect(Rect),
    Transform(Affine),
    Color(Color),
    Shadow(Shadow),
}

impl Value {
    /// Linear interpolation between two values of the same kind (`t` may be outside 0..1 for springs and
    /// overshooting curves). Mismatched kinds return `self` until `t >= 1`, then `other`.
    pub fn lerp(&self, other: &Value, t: f32) -> Value {
        let _ = (other, t);
        todo!("layers-animation")
    }
}

/// What the animation shows outside its active time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillMode {
    /// The model value before and after (the default: the animation is removed when done).
    #[default]
    Removed,
    Forwards,
    Backwards,
    Both,
}

/// Media timing, as the reference platform's timing protocol defines it. Times are seconds on the tree's clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaTiming {
    /// Absolute start on the tree clock; `None` means "the frame the transaction is applied".
    pub begin_time: Option<f64>,
    pub duration: f64,
    pub speed: f32,
    pub time_offset: f64,
    /// 0 plays once; `f32::INFINITY` repeats forever; fractions play a partial last cycle.
    pub repeat_count: f32,
    pub autoreverses: bool,
    pub fill_mode: FillMode,
}

impl Default for MediaTiming {
    fn default() -> Self {
        MediaTiming { begin_time: None, duration: 0.25, speed: 1.0, time_offset: 0.0, repeat_count: 0.0, autoreverses: false, fill_mode: FillMode::Removed }
    }
}

impl MediaTiming {
    /// The local progress (0..1 within one cycle, reversed on autoreverse legs) at tree time `t` for an animation
    /// that began at `begin`, or `None` when it is not active and its fill mode shows nothing.
    pub fn progress(&self, begin: f64, t: f64) -> Option<f32> {
        let _ = (begin, t);
        todo!("layers-animation")
    }

    /// Whether the animation has ended at tree time `t` (for `animation_finished`).
    pub fn ended(&self, begin: f64, t: f64) -> bool {
        let _ = (begin, t);
        todo!("layers-animation")
    }
}

/// Keyframe values with their key times (0..1, same length as `values`) and per-segment timing functions.
#[derive(Debug, Clone, PartialEq)]
pub struct Keyframes {
    pub values: Vec<Value>,
    pub key_times: Vec<f32>,
    pub timing_functions: Vec<TimingFunction>,
}

/// A damped spring from `from` to `to` (duration comes from the physics, not `MediaTiming::duration`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    pub mass: f32,
    pub stiffness: f32,
    pub damping: f32,
    pub initial_velocity: f32,
}

impl Spring {
    /// The progress (0 at start, settling at 1) after `t` seconds.
    pub fn progress(&self, t: f32) -> f32 {
        let _ = t;
        todo!("layers-animation")
    }

    /// The time after which the spring stays within 0.1% of its target.
    pub fn settling_duration(&self) -> f32 {
        todo!("layers-animation")
    }
}

/// An animation attached to a layer.
#[derive(Debug, Clone, PartialEq)]
pub enum Animation {
    Basic { id: AnimationId, key: Property, from: Option<Value>, to: Option<Value>, timing: MediaTiming, function: TimingFunction, additive: bool },
    Keyframe { id: AnimationId, key: Property, keyframes: Keyframes, timing: MediaTiming },
    Spring { id: AnimationId, key: Property, from: Option<Value>, to: Option<Value>, spring: Spring, timing: MediaTiming },
    Group { id: AnimationId, animations: Vec<Animation>, timing: MediaTiming },
}

impl Animation {
    pub fn id(&self) -> AnimationId {
        match self {
            Animation::Basic { id, .. } | Animation::Keyframe { id, .. } | Animation::Spring { id, .. } | Animation::Group { id, .. } => *id,
        }
    }

    pub fn timing(&self) -> &MediaTiming {
        match self {
            Animation::Basic { timing, .. } | Animation::Keyframe { timing, .. } | Animation::Spring { timing, .. } | Animation::Group { timing, .. } => timing,
        }
    }
}
