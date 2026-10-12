//! Actions for scene nodes: a pure model of the reference framework's node actions (`SCNAction`) and of the
//! player that runs them (`SCNActionPlayer` behaviour: run with an optional key, replace by key, remove, step).
//!
//! An `Action` is a tree. A leaf animates one property of a node over a duration with a timing curve (move by or to,
//! rotate by or to, scale by or to, or wait). `Sequence` runs its children one after another, `Group` runs them
//! together and ends with the longest, and `Repeat` runs its child a fixed number of times or forever. The runner
//! reads no clock: the caller passes the frame time to `ActionRunner::step`, which writes the new local pose of each
//! node through `Scene::set_pose`. Nothing is drawn here.
//!
//! Relative leaves (`MoveBy`, `RotateBy`, `ScaleBy`) apply only the change since the previous step, so two relative
//! actions running on one node add up instead of overwriting each other. Absolute leaves (`MoveTo`, `RotateTo`,
//! `ScaleTo`) interpolate from the pose the leaf first saw to the target. Rotations are about the node's local axes.
//! Timing curves are the common quadratic forms (not the reference framework's exact curves). A duration of zero or
//! less completes on the first step.

use crate::linalg::{Quaternion, Vector3};
use crate::scene3d::{NodeId, Pose, Scene};

/// How the fraction of a leaf's duration maps to the fraction of its change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timing {
    Linear,
    EaseIn,
    EaseOut,
    EaseInEaseOut,
}

impl Timing {
    /// Maps a fraction in `0..=1` through the curve. The ends stay fixed at 0 and 1.
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Timing::Linear => t,
            Timing::EaseIn => t * t,
            Timing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Timing::EaseInEaseOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0) * (-2.0 * t + 2.0) * 0.5
                }
            }
        }
    }
}

/// The property change a leaf action makes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// Changes nothing; it only takes time.
    Wait,
    /// Adds an offset to the position.
    MoveBy(Vector3),
    /// Interpolates the position to an absolute point.
    MoveTo(Vector3),
    /// Turns by `radians` about a local axis.
    RotateBy { axis: Vector3, radians: f32 },
    /// Interpolates the orientation to an absolute rotation.
    RotateTo(Quaternion),
    /// Multiplies the scale per axis by the factors.
    ScaleBy(Vector3),
    /// Interpolates the scale to absolute values.
    ScaleTo(Vector3),
}

impl Effect {
    /// Whether the effect adds to the node's current value instead of setting an absolute one.
    fn is_relative(self) -> bool {
        matches!(self, Effect::MoveBy(_) | Effect::RotateBy { .. } | Effect::ScaleBy(_))
    }

    /// The effect that undoes this one, when it is relative. `None` for absolute effects.
    fn reversed(self) -> Option<Effect> {
        match self {
            Effect::Wait => Some(Effect::Wait),
            Effect::MoveBy(d) => Some(Effect::MoveBy(-d)),
            Effect::RotateBy { axis, radians } => Some(Effect::RotateBy { axis, radians: -radians }),
            Effect::ScaleBy(f) => Some(Effect::ScaleBy(Vector3::new(recip(f.x), recip(f.y), recip(f.z)))),
            _ => None,
        }
    }
}

fn recip(x: f32) -> f32 {
    if x == 0.0 {
        0.0
    } else {
        1.0 / x
    }
}

fn lerp_vec(a: Vector3, b: Vector3, t: f32) -> Vector3 {
    a + (b - a) * t
}

fn mul_vec(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(a.x * b.x, a.y * b.y, a.z * b.z)
}

/// An action: a single timed leaf, or a tree of actions.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Animate { effect: Effect, duration: f32, timing: Timing },
    /// Runs the children one after another.
    Sequence(Vec<Action>),
    /// Runs the children together; the group lasts as long as its longest child.
    Group(Vec<Action>),
    /// Runs `action` `count` times, or forever when `count` is `None`.
    Repeat { action: Box<Action>, count: Option<u32> },
}

impl Action {
    pub fn wait(duration: f32) -> Action {
        Action::Animate { effect: Effect::Wait, duration, timing: Timing::Linear }
    }

    pub fn move_by(delta: Vector3, duration: f32) -> Action {
        Action::Animate { effect: Effect::MoveBy(delta), duration, timing: Timing::Linear }
    }

    pub fn move_to(point: Vector3, duration: f32) -> Action {
        Action::Animate { effect: Effect::MoveTo(point), duration, timing: Timing::Linear }
    }

    pub fn rotate_by(axis: Vector3, radians: f32, duration: f32) -> Action {
        Action::Animate { effect: Effect::RotateBy { axis, radians }, duration, timing: Timing::Linear }
    }

    pub fn rotate_to(orientation: Quaternion, duration: f32) -> Action {
        Action::Animate { effect: Effect::RotateTo(orientation), duration, timing: Timing::Linear }
    }

    pub fn scale_by(factors: Vector3, duration: f32) -> Action {
        Action::Animate { effect: Effect::ScaleBy(factors), duration, timing: Timing::Linear }
    }

    pub fn scale_to(scale: Vector3, duration: f32) -> Action {
        Action::Animate { effect: Effect::ScaleTo(scale), duration, timing: Timing::Linear }
    }

    pub fn sequence(actions: Vec<Action>) -> Action {
        Action::Sequence(actions)
    }

    pub fn group(actions: Vec<Action>) -> Action {
        Action::Group(actions)
    }

    pub fn repeat(action: Action, count: u32) -> Action {
        Action::Repeat { action: Box::new(action), count: Some(count) }
    }

    pub fn repeat_forever(action: Action) -> Action {
        Action::Repeat { action: Box::new(action), count: None }
    }

    /// The same action with its timing curve replaced on every leaf. Other kinds are unchanged.
    pub fn with_timing(self, timing: Timing) -> Action {
        match self {
            Action::Animate { effect, duration, .. } => Action::Animate { effect, duration, timing },
            Action::Sequence(a) => Action::Sequence(a.into_iter().map(|x| x.with_timing(timing)).collect()),
            Action::Group(a) => Action::Group(a.into_iter().map(|x| x.with_timing(timing)).collect()),
            Action::Repeat { action, count } => {
                Action::Repeat { action: Box::new(action.with_timing(timing)), count }
            }
        }
    }

    /// The total time the action takes in seconds. `None` when it never ends.
    pub fn duration(&self) -> Option<f32> {
        match self {
            Action::Animate { duration, .. } => Some(duration.max(0.0)),
            Action::Sequence(actions) => {
                let mut total = 0.0;
                for a in actions {
                    total += a.duration()?;
                }
                Some(total)
            }
            Action::Group(actions) => {
                let mut longest: f32 = 0.0;
                for a in actions {
                    longest = longest.max(a.duration()?);
                }
                Some(longest)
            }
            Action::Repeat { action, count } => {
                let per = action.duration()?;
                count.map(|c| per * c as f32)
            }
        }
    }

    /// The action that undoes this one, or `None` when it contains an absolute change. A sequence runs its children
    /// in reverse order, and a repeat keeps its count.
    pub fn reversed(&self) -> Option<Action> {
        match self {
            Action::Animate { effect, duration, timing } => Some(Action::Animate {
                effect: effect.reversed()?,
                duration: *duration,
                timing: *timing,
            }),
            Action::Sequence(actions) => {
                let mut out = Vec::with_capacity(actions.len());
                for a in actions.iter().rev() {
                    out.push(a.reversed()?);
                }
                Some(Action::Sequence(out))
            }
            Action::Group(actions) => {
                let mut out = Vec::with_capacity(actions.len());
                for a in actions {
                    out.push(a.reversed()?);
                }
                Some(Action::Group(out))
            }
            Action::Repeat { action, count } => {
                Some(Action::Repeat { action: Box::new(action.reversed()?), count: *count })
            }
        }
    }
}

/// The per-run state of an action tree. Built from an `Action` when the action starts and advanced by `step`.
#[derive(Clone, Debug)]
enum Cursor {
    Leaf {
        /// The pose the leaf first saw; absolute effects interpolate from it.
        start: Option<Pose>,
        /// Time spent in the leaf so far, in seconds.
        elapsed: f32,
        /// The eased fraction already applied, so relative effects add only the new part.
        applied: f32,
    },
    Sequence {
        index: usize,
        current: Option<Box<Cursor>>,
    },
    Group {
        children: Vec<Cursor>,
        /// `Some(leftover)` once a child has finished.
        finished: Vec<Option<f32>>,
    },
    Repeat {
        done: u32,
        current: Option<Box<Cursor>>,
    },
}

impl Cursor {
    fn new(action: &Action) -> Cursor {
        match action {
            Action::Animate { .. } => Cursor::Leaf { start: None, elapsed: 0.0, applied: 0.0 },
            Action::Sequence(_) => Cursor::Sequence { index: 0, current: None },
            Action::Group(actions) => Cursor::Group {
                children: actions.iter().map(Cursor::new).collect(),
                finished: vec![None; actions.len()],
            },
            Action::Repeat { .. } => Cursor::Repeat { done: 0, current: None },
        }
    }
}

/// Advances `action` on `node` by `dt` seconds. Returns `None` while the action is still running, or
/// `Some(leftover)` with the time that was left over after it finished.
fn step(scene: &mut Scene, node: NodeId, action: &Action, cursor: &mut Cursor, dt: f32) -> Option<f32> {
    match (action, cursor) {
        (Action::Animate { effect, duration, timing }, Cursor::Leaf { start, elapsed, applied }) => {
            // A node that is gone ends its actions at once.
            let Some(current) = scene.local_pose(node) else {
                return Some(dt);
            };
            let base = *start.get_or_insert(current);
            *elapsed += dt.max(0.0);
            let total = duration.max(0.0);
            let raw = if total <= 0.0 { 1.0 } else { (*elapsed / total).min(1.0) };
            let fraction = timing.apply(raw);
            if *effect != Effect::Wait {
                let mut pose = current;
                let df = fraction - *applied;
                match *effect {
                    Effect::Wait => {}
                    Effect::MoveBy(d) => pose.position = pose.position + d * df,
                    Effect::MoveTo(p) => pose.position = lerp_vec(base.position, p, fraction),
                    Effect::RotateBy { axis, radians } => {
                        pose.orientation =
                            (pose.orientation * Quaternion::from_axis_angle(axis, radians * df)).normalized();
                    }
                    Effect::RotateTo(q) => pose.orientation = Quaternion::slerp(base.orientation, q, fraction),
                    Effect::ScaleBy(f) => {
                        let k = Vector3::new(f.x - 1.0, f.y - 1.0, f.z - 1.0);
                        pose.scale = pose.scale + mul_vec(base.scale, k) * df;
                    }
                    Effect::ScaleTo(s) => pose.scale = lerp_vec(base.scale, s, fraction),
                }
                scene.set_pose(node, pose);
            }
            *applied = fraction;
            if *elapsed >= total {
                Some(*elapsed - total)
            } else {
                None
            }
        }
        (Action::Sequence(actions), Cursor::Sequence { index, current }) => {
            let mut left = dt;
            loop {
                if *index >= actions.len() {
                    return Some(left);
                }
                let cur = current.get_or_insert_with(|| Box::new(Cursor::new(&actions[*index])));
                match step(scene, node, &actions[*index], cur, left) {
                    None => return None,
                    Some(rest) => {
                        *index += 1;
                        *current = None;
                        left = rest;
                    }
                }
            }
        }
        (Action::Group(actions), Cursor::Group { children, finished }) => {
            if actions.is_empty() {
                return Some(dt);
            }
            let mut all_done = true;
            let mut earliest: Option<f32> = None;
            for (i, child) in children.iter_mut().enumerate() {
                if finished[i].is_some() {
                    continue;
                }
                match step(scene, node, &actions[i], child, dt) {
                    None => all_done = false,
                    Some(left) => {
                        finished[i] = Some(left);
                        earliest = Some(earliest.map_or(left, |e: f32| e.min(left)));
                    }
                }
            }
            if all_done {
                Some(earliest.unwrap_or(dt))
            } else {
                None
            }
        }
        (Action::Repeat { action, count }, Cursor::Repeat { done, current }) => {
            // A repeat of a zero-length action forever would never yield; it stays as a no-op until removed.
            let zero_length_forever = count.is_none() && action.duration() == Some(0.0);
            let mut left = dt;
            loop {
                if let Some(c) = count {
                    if *done >= *c {
                        return Some(left);
                    }
                }
                let cur = current.get_or_insert_with(|| Box::new(Cursor::new(action)));
                match step(scene, node, action, cur, left) {
                    None => return None,
                    Some(rest) => {
                        *done += 1;
                        *current = None;
                        if zero_length_forever {
                            return None;
                        }
                        left = rest;
                    }
                }
            }
        }
        // The cursor was built from a different action; treat it as finished rather than guessing.
        _ => Some(dt),
    }
}

/// An action that is running on a node, with the key it was started under.
#[derive(Clone, Debug)]
struct Running {
    node: NodeId,
    key: Option<String>,
    action: Action,
    cursor: Cursor,
}

/// Runs actions on the nodes of a `Scene`. Several actions can run on one node at once; each may carry a key that
/// names it, so it can be replaced or removed later.
#[derive(Clone, Debug, Default)]
pub struct ActionRunner {
    running: Vec<Running>,
}

impl ActionRunner {
    pub fn new() -> ActionRunner {
        ActionRunner { running: Vec::new() }
    }

    /// Starts `action` on `node`. A key that a running action on the same node already uses replaces that action.
    pub fn run(&mut self, node: NodeId, action: Action, key: Option<&str>) {
        if let Some(k) = key {
            self.running.retain(|r| !(r.node == node && r.key.as_deref() == Some(k)));
        }
        let cursor = Cursor::new(&action);
        self.running.push(Running { node, key: key.map(|k| k.to_string()), action, cursor });
    }

    pub fn has_actions(&self, node: NodeId) -> bool {
        self.running.iter().any(|r| r.node == node)
    }

    /// Stops the action with this key on `node`. `true` when one was running.
    pub fn remove_action(&mut self, node: NodeId, key: &str) -> bool {
        let before = self.running.len();
        self.running.retain(|r| !(r.node == node && r.key.as_deref() == Some(key)));
        self.running.len() != before
    }

    /// Stops every action on `node`.
    pub fn remove_all_actions(&mut self, node: NodeId) {
        self.running.retain(|r| r.node != node);
    }

    /// The number of actions still running.
    pub fn len(&self) -> usize {
        self.running.len()
    }

    pub fn is_empty(&self) -> bool {
        self.running.is_empty()
    }

    /// Advances every running action by `dt` seconds (negative or NaN counts as zero) and drops the ones that
    /// finished. Returns how many finished during this step.
    pub fn step(&mut self, scene: &mut Scene, dt: f32) -> usize {
        let dt = if dt.is_nan() { 0.0 } else { dt.max(0.0) };
        let before = self.running.len();
        self.running.retain_mut(|r| step(&mut *scene, r.node, &r.action, &mut r.cursor, dt).is_none());
        before - self.running.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn v(x: f32, y: f32, z: f32) -> Vector3 {
        Vector3::new(x, y, z)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn close_v(a: Vector3, b: Vector3) -> bool {
        close(a.x, b.x) && close(a.y, b.y) && close(a.z, b.z)
    }

    fn scene_with_node() -> (Scene, NodeId) {
        let mut scene = Scene::new();
        let node = scene.add_child(scene.root(), "box").unwrap();
        (scene, node)
    }

    #[test]
    fn timing_curves_keep_their_ends() {
        for t in [Timing::Linear, Timing::EaseIn, Timing::EaseOut, Timing::EaseInEaseOut] {
            assert!(close(t.apply(0.0), 0.0));
            assert!(close(t.apply(1.0), 1.0));
        }
        assert!(close(Timing::EaseIn.apply(0.5), 0.25));
        assert!(close(Timing::EaseOut.apply(0.5), 0.75));
        assert!(close(Timing::EaseInEaseOut.apply(0.5), 0.5));
        assert!(close(Timing::Linear.apply(2.0), 1.0));
    }

    #[test]
    fn durations_compose_like_the_reference_actions() {
        let a = Action::move_by(v(1.0, 0.0, 0.0), 0.5);
        let b = Action::wait(1.5);
        assert_eq!(Action::sequence(vec![a.clone(), b.clone()]).duration(), Some(2.0));
        assert_eq!(Action::group(vec![a.clone(), b.clone()]).duration(), Some(1.5));
        assert_eq!(Action::repeat(a.clone(), 4).duration(), Some(2.0));
        assert_eq!(Action::repeat_forever(a.clone()).duration(), None);
        assert_eq!(Action::sequence(vec![a, Action::repeat_forever(b)]).duration(), None);
    }

    #[test]
    fn move_by_reaches_the_offset_and_moves_linearly() {
        let (mut scene, node) = scene_with_node();
        let mut runner = ActionRunner::new();
        runner.run(node, Action::move_by(v(4.0, 0.0, -2.0), 2.0), None);
        runner.step(&mut scene, 1.0);
        assert!(close_v(scene.local_pose(node).unwrap().position, v(2.0, 0.0, -1.0)));
        assert_eq!(runner.step(&mut scene, 1.0), 1);
        assert!(close_v(scene.local_pose(node).unwrap().position, v(4.0, 0.0, -2.0)));
        assert!(runner.is_empty());
    }

    #[test]
    fn move_to_interpolates_from_the_start_pose() {
        let (mut scene, node) = scene_with_node();
        let mut p = scene.local_pose(node).unwrap();
        p.position = v(10.0, 0.0, 0.0);
        scene.set_pose(node, p);
        let mut runner = ActionRunner::new();
        runner.run(node, Action::move_to(v(0.0, 0.0, 0.0), 1.0), None);
        runner.step(&mut scene, 0.25);
        assert!(close_v(scene.local_pose(node).unwrap().position, v(7.5, 0.0, 0.0)));
        runner.step(&mut scene, 5.0);
        assert!(close_v(scene.local_pose(node).unwrap().position, v(0.0, 0.0, 0.0)));
    }

    #[test]
    fn sequence_runs_children_in_order_and_carries_leftover_time() {
        let (mut scene, node) = scene_with_node();
        let seq = Action::sequence(vec![Action::move_by(v(1.0, 0.0, 0.0), 1.0), Action::move_by(v(0.0, 2.0, 0.0), 1.0)]);
        let mut runner = ActionRunner::new();
        runner.run(node, seq, None);
        // 1.5 s: the first move ends at 1.0 and the second has 0.5 s of its 1.0 s.
        runner.step(&mut scene, 1.5);
        assert!(close_v(scene.local_pose(node).unwrap().position, v(1.0, 1.0, 0.0)));
        runner.step(&mut scene, 0.5);
        assert!(close_v(scene.local_pose(node).unwrap().position, v(1.0, 2.0, 0.0)));
        assert!(runner.is_empty());
    }

    #[test]
    fn group_ends_with_its_longest_child() {
        let (mut scene, node) = scene_with_node();
        let g = Action::group(vec![Action::move_by(v(1.0, 0.0, 0.0), 1.0), Action::move_by(v(0.0, 0.0, 3.0), 3.0)]);
        let mut runner = ActionRunner::new();
        runner.run(node, g, None);
        runner.step(&mut scene, 2.0);
        assert!(close_v(scene.local_pose(node).unwrap().position, v(1.0, 0.0, 2.0)));
        assert!(runner.has_actions(node));
        runner.step(&mut scene, 1.0);
        assert!(close_v(scene.local_pose(node).unwrap().position, v(1.0, 0.0, 3.0)));
        assert!(!runner.has_actions(node));
    }

    #[test]
    fn relative_actions_on_one_node_add_up() {
        let (mut scene, node) = scene_with_node();
        let mut runner = ActionRunner::new();
        runner.run(node, Action::move_by(v(1.0, 0.0, 0.0), 1.0), Some("a"));
        runner.run(node, Action::move_by(v(0.0, 1.0, 0.0), 1.0), Some("b"));
        runner.step(&mut scene, 0.5);
        runner.step(&mut scene, 0.5);
        assert!(close_v(scene.local_pose(node).unwrap().position, v(1.0, 1.0, 0.0)));
    }

    #[test]
    fn repeat_runs_the_child_count_times() {
        let (mut scene, node) = scene_with_node();
        let mut runner = ActionRunner::new();
        runner.run(node, Action::repeat(Action::move_by(v(1.0, 0.0, 0.0), 1.0), 3), None);
        runner.step(&mut scene, 2.5);
        assert!(close_v(scene.local_pose(node).unwrap().position, v(2.5, 0.0, 0.0)));
        assert!(runner.has_actions(node));
        runner.step(&mut scene, 1.0);
        assert!(close_v(scene.local_pose(node).unwrap().position, v(3.0, 0.0, 0.0)));
        assert!(!runner.has_actions(node));
    }

    #[test]
    fn repeat_forever_never_finishes_and_zero_length_forever_is_a_no_op() {
        let (mut scene, node) = scene_with_node();
        let mut runner = ActionRunner::new();
        runner.run(node, Action::repeat_forever(Action::rotate_by(v(0.0, 1.0, 0.0), FRAC_PI_2, 1.0)), None);
        assert_eq!(runner.step(&mut scene, 10.0), 0);
        assert!(runner.has_actions(node));

        let mut idle = ActionRunner::new();
        idle.run(node, Action::repeat_forever(Action::wait(0.0)), None);
        assert_eq!(idle.step(&mut scene, 1.0), 0);
        assert!(idle.has_actions(node));
    }

    #[test]
    fn rotate_by_turns_about_the_local_axis() {
        let (mut scene, node) = scene_with_node();
        let mut runner = ActionRunner::new();
        runner.run(node, Action::rotate_by(v(0.0, 1.0, 0.0), FRAC_PI_2, 1.0), None);
        runner.step(&mut scene, 1.0);
        let m = scene.local_pose(node).unwrap().matrix();
        let p = m.transform_point(v(1.0, 0.0, 0.0)).unwrap();
        // A quarter turn about +Y takes +X to -Z.
        assert!(close_v(p, v(0.0, 0.0, -1.0)), "{:?}", p);
    }

    #[test]
    fn scale_by_and_scale_to() {
        let (mut scene, node) = scene_with_node();
        let mut runner = ActionRunner::new();
        runner.run(node, Action::scale_by(v(3.0, 1.0, 1.0), 1.0), None);
        runner.step(&mut scene, 1.0);
        assert!(close_v(scene.local_pose(node).unwrap().scale, v(3.0, 1.0, 1.0)));

        let mut runner = ActionRunner::new();
        runner.run(node, Action::scale_to(v(2.0, 2.0, 2.0), 1.0), None);
        runner.step(&mut scene, 0.5);
        assert!(close_v(scene.local_pose(node).unwrap().scale, v(2.5, 1.5, 1.5)));
        runner.step(&mut scene, 0.5);
        assert!(close_v(scene.local_pose(node).unwrap().scale, v(2.0, 2.0, 2.0)));
    }

    #[test]
    fn keys_replace_and_remove_actions() {
        let (mut scene, node) = scene_with_node();
        let mut runner = ActionRunner::new();
        runner.run(node, Action::repeat_forever(Action::wait(1.0)), Some("spin"));
        runner.run(node, Action::move_by(v(1.0, 0.0, 0.0), 1.0), Some("spin"));
        assert_eq!(runner.len(), 1);
        assert!(runner.remove_action(node, "spin"));
        assert!(!runner.remove_action(node, "spin"));
        assert!(!runner.has_actions(node));

        runner.run(node, Action::wait(5.0), None);
        runner.run(node, Action::wait(5.0), None);
        runner.remove_all_actions(node);
        assert!(runner.is_empty());
    }

    #[test]
    fn actions_on_missing_nodes_finish_and_bad_time_is_ignored() {
        let (mut scene, node) = scene_with_node();
        let mut runner = ActionRunner::new();
        runner.run(node, Action::move_by(v(1.0, 0.0, 0.0), 1.0), None);
        assert_eq!(runner.step(&mut scene, f32::NAN), 0);
        assert_eq!(runner.step(&mut scene, -5.0), 0);
        assert!(runner.has_actions(node));
        let other = Scene::new();
        let mut scene2 = other;
        assert_eq!(runner.step(&mut scene2, 0.1), 1);
    }

    #[test]
    fn reversed_undoes_relative_actions_and_refuses_absolute_ones() {
        let (mut scene, node) = scene_with_node();
        let forward = Action::sequence(vec![
            Action::move_by(v(2.0, 0.0, 0.0), 1.0),
            Action::rotate_by(v(0.0, 0.0, 1.0), 1.0, 1.0),
        ]);
        let back = forward.reversed().unwrap();
        let mut runner = ActionRunner::new();
        runner.run(node, forward, None);
        runner.step(&mut scene, 2.0);
        runner.run(node, back, None);
        runner.step(&mut scene, 2.0);
        assert!(close_v(scene.local_pose(node).unwrap().position, v(0.0, 0.0, 0.0)));
        let m = scene.local_pose(node).unwrap().matrix();
        assert!(close_v(m.transform_point(v(1.0, 0.0, 0.0)).unwrap(), v(1.0, 0.0, 0.0)));

        assert!(Action::move_to(v(1.0, 1.0, 1.0), 1.0).reversed().is_none());
        assert!(Action::sequence(vec![Action::wait(1.0), Action::scale_to(v(1.0, 1.0, 1.0), 1.0)]).reversed().is_none());
    }

    #[test]
    fn with_timing_replaces_the_curve_on_every_leaf() {
        let a = Action::sequence(vec![Action::move_by(v(1.0, 0.0, 0.0), 1.0)]).with_timing(Timing::EaseIn);
        match a {
            Action::Sequence(children) => match &children[0] {
                Action::Animate { timing, .. } => assert_eq!(*timing, Timing::EaseIn),
                other => panic!("unexpected {:?}", other),
            },
            other => panic!("unexpected {:?}", other),
        }
    }
}
