//! Actions for views: timed changes of a node's position, rotation, scale and opacity, composed as
//! sequences, groups and repeats. An action is a value; [`Action::pose_at`] says where the node is `t` seconds after
//! the action starts, relative to where it started. The window applies the pose, so the actions stay pure and
//! testable. Timing is linear, the reference's default timing mode; the other timing modes are not modelled.
//!
//! Moves, rotations and scales are relative, as the move-by, rotate-by and scale-by actions are. A fade is absolute: it runs to
//! the given opacity from fully opaque. Composed actions add their offsets and multiply their scales; when two
//! fades overlap, the one that started last sets the opacity.

/// A timed change of a node.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Moves by `(dx, dy)` points over `duration` seconds.
    MoveBy { dx: f32, dy: f32, duration: f32 },
    /// Turns by `radians` over `duration` seconds.
    RotateBy { radians: f32, duration: f32 },
    /// Grows or shrinks by `factor` over `duration` seconds.
    ScaleBy { factor: f32, duration: f32 },
    /// Fades to `alpha` (0 to 1) over `duration` seconds.
    FadeTo { alpha: f32, duration: f32 },
    /// Does nothing for `duration` seconds.
    Wait { duration: f32 },
    /// Runs the actions one after another.
    Sequence(Vec<Action>),
    /// Runs the actions at the same time.
    Group(Vec<Action>),
    /// Runs the action `count` times in turn; each run starts where the previous one ended.
    Repeat { action: Box<Action>, count: u32 },
}

/// Where a node is relative to where its action started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub dx: f32,
    pub dy: f32,
    pub rotation: f32,
    pub scale: f32,
    /// The opacity the action sets, or `None` when no action has set one.
    pub alpha: Option<f32>,
}

impl Pose {
    /// No change: no offset, no rotation, scale one, no opacity set.
    pub const REST: Pose = Pose { dx: 0.0, dy: 0.0, rotation: 0.0, scale: 1.0, alpha: None };

    /// The pose of two changes applied one after the other: offsets and rotations add, scales multiply, and the
    /// opacity of `later` wins when it sets one.
    pub fn then(self, later: Pose) -> Pose {
        Pose {
            dx: self.dx + later.dx,
            dy: self.dy + later.dy,
            rotation: self.rotation + later.rotation,
            scale: self.scale * later.scale,
            alpha: later.alpha.or(self.alpha),
        }
    }
}

impl Action {
    /// How long the action runs, in seconds. A sequence adds its parts, a group takes the longest, and a repeat
    /// multiplies. Negative durations count as zero.
    pub fn duration(&self) -> f32 {
        match self {
            Action::MoveBy { duration, .. }
            | Action::RotateBy { duration, .. }
            | Action::ScaleBy { duration, .. }
            | Action::FadeTo { duration, .. }
            | Action::Wait { duration } => duration.max(0.0),
            Action::Sequence(actions) => actions.iter().map(Action::duration).sum(),
            Action::Group(actions) => actions.iter().map(Action::duration).fold(0.0, f32::max),
            Action::Repeat { action, count } => action.duration() * *count as f32,
        }
    }

    /// The pose `t` seconds after the action starts. Times before the start count as the start, and times after the
    /// end give the final pose.
    pub fn pose_at(&self, t: f32) -> Pose {
        let t = t.max(0.0);
        match self {
            Action::MoveBy { dx, dy, duration } => {
                let p = progress(t, *duration);
                Pose { dx: dx * p, dy: dy * p, ..Pose::REST }
            }
            Action::RotateBy { radians, duration } => Pose { rotation: radians * progress(t, *duration), ..Pose::REST },
            Action::ScaleBy { factor, duration } => {
                Pose { scale: 1.0 + (factor - 1.0) * progress(t, *duration), ..Pose::REST }
            }
            Action::FadeTo { alpha, duration } => {
                Pose { alpha: Some(1.0 + (alpha - 1.0) * progress(t, *duration)), ..Pose::REST }
            }
            Action::Wait { .. } => Pose::REST,
            Action::Sequence(actions) => sequence_pose(actions, t),
            Action::Group(actions) => actions.iter().fold(Pose::REST, |pose, a| pose.then(a.pose_at(t))),
            Action::Repeat { action, count } => repeat_pose(action, *count, t),
        }
    }
}

/// The fraction of `duration` that `t` covers, from 0 to 1. A zero-length action is finished at once.
fn progress(t: f32, duration: f32) -> f32 {
    if duration <= 0.0 {
        1.0
    } else {
        (t / duration).min(1.0)
    }
}

fn sequence_pose(actions: &[Action], t: f32) -> Pose {
    let mut pose = Pose::REST;
    let mut start = 0.0;
    for action in actions {
        let length = action.duration();
        if t - start < length {
            return pose.then(action.pose_at(t - start));
        }
        pose = pose.then(action.pose_at(length));
        start += length;
    }
    pose
}

fn repeat_pose(action: &Action, count: u32, t: f32) -> Pose {
    let length = action.duration();
    if length <= 0.0 {
        return action.pose_at(0.0);
    }
    let finished = ((t / length).floor() as u32).min(count);
    let mut pose = Pose::REST;
    for _ in 0..finished {
        pose = pose.then(action.pose_at(length));
    }
    if finished < count {
        pose = pose.then(action.pose_at(t - finished as f32 * length));
    }
    pose
}
