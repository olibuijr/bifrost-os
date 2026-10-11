//! Agents that steer themselves in two dimensions. A goal (seek, flee, reach a speed, wander, avoid circular
//! obstacles, follow a path) produces a steering acceleration. Goals are weighted into a behavior, behaviors are
//! weighted into a composite behavior, and an agent moves under a speed limit and an acceleration limit. This is the
//! agent layer of the reference game SDK, for 2D worlds. Goals name points rather than other agents, so the app
//! updates a target each frame when it moves. Each agent carries its own seeded generator, so a simulation is
//! repeatable from its seed.

use std::ops::{Add, Mul, Sub};

/// A point or direction in 2D space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub fn new(x: f32, y: f32) -> Vec2 {
        Vec2 { x, y }
    }

    /// The unit vector at `angle` radians from the positive x axis.
    pub fn from_angle(angle: f32) -> Vec2 {
        Vec2::new(angle.cos(), angle.sin())
    }

    pub fn dot(self, other: Vec2) -> f32 {
        self.x * other.x + self.y * other.y
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// The unit vector in the same direction, or zero when the vector is (nearly) zero.
    pub fn normalized(self) -> Vec2 {
        let len = self.length();
        if len > 1e-6 {
            self * (1.0 / len)
        } else {
            Vec2::ZERO
        }
    }

    /// The vector scaled down to at most `max` in length. Shorter vectors are returned unchanged.
    pub fn truncated(self, max: f32) -> Vec2 {
        let len = self.length();
        if len > max && len > 0.0 {
            self * (max / len)
        } else {
            self
        }
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, k: f32) -> Vec2 {
        Vec2::new(self.x * k, self.y * k)
    }
}

/// A small deterministic generator (xorshift64), used by wander goals. A zero seed is replaced by a fixed constant.
#[derive(Clone, Copy, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A number in [0, 1).
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / 16_777_216.0
    }

    /// A number in [-half_width, half_width).
    pub fn symmetric(&mut self, half_width: f32) -> f32 {
        (self.unit() * 2.0 - 1.0) * half_width
    }
}

/// A closed disc that agents steer around.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    pub center: Vec2,
    pub radius: f32,
}

/// Where a point sits on a path: the segment it is nearest to, the nearest point, and the distance along the path
/// from the path's first vertex to that point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathPosition {
    pub segment: usize,
    pub point: Vec2,
    pub along: f32,
}

/// A polyline an agent can follow. A cyclical path closes back from its last vertex to its first.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    points: Vec<Vec2>,
    radius: f32,
    cyclical: bool,
}

impl Path {
    /// A path through `points`. Needs at least two vertices and a radius of zero or more. `radius` is how far an
    /// agent may drift from the path before it heads back to the nearest point instead of the next point ahead.
    pub fn new(points: Vec<Vec2>, radius: f32, cyclical: bool) -> Option<Path> {
        if points.len() < 2 || !(radius >= 0.0) {
            return None;
        }
        Some(Path { points, radius, cyclical })
    }

    pub fn num_points(&self) -> usize {
        self.points.len()
    }

    pub fn point(&self, index: usize) -> Option<Vec2> {
        self.points.get(index).copied()
    }

    pub fn radius(&self) -> f32 {
        self.radius
    }

    pub fn is_cyclical(&self) -> bool {
        self.cyclical
    }

    fn segment_count(&self) -> usize {
        if self.cyclical {
            self.points.len()
        } else {
            self.points.len() - 1
        }
    }

    fn segment(&self, i: usize) -> (Vec2, Vec2) {
        (self.points[i], self.points[(i + 1) % self.points.len()])
    }

    /// The total length of the path, including the closing segment when it is cyclical.
    pub fn length(&self) -> f32 {
        (0..self.segment_count())
            .map(|i| {
                let (a, b) = self.segment(i);
                (b - a).length()
            })
            .sum()
    }

    /// The point on the path nearest to `p`.
    pub fn nearest(&self, p: Vec2) -> PathPosition {
        let mut best = PathPosition { segment: 0, point: self.points[0], along: 0.0 };
        let mut best_d2 = f32::INFINITY;
        let mut start = 0.0;
        for i in 0..self.segment_count() {
            let (a, b) = self.segment(i);
            let ab = b - a;
            let seg_len = ab.length();
            let len2 = ab.dot(ab);
            let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let q = a + ab * t;
            let d2 = (p - q).dot(p - q);
            if d2 < best_d2 {
                best_d2 = d2;
                best = PathPosition { segment: i, point: q, along: start + seg_len * t };
            }
            start += seg_len;
        }
        best
    }

    /// The point `distance` along the path from its first vertex. A cyclical path wraps around. An open path stops
    /// at its ends.
    pub fn point_at_distance(&self, distance: f32) -> Vec2 {
        let total = self.length();
        if !(total > 0.0) {
            return self.points[0];
        }
        let mut d = if self.cyclical { distance.rem_euclid(total) } else { distance.clamp(0.0, total) };
        for i in 0..self.segment_count() {
            let (a, b) = self.segment(i);
            let ab = b - a;
            let seg_len = ab.length();
            if d <= seg_len {
                let t = if seg_len > 0.0 { d / seg_len } else { 0.0 };
                return a + ab * t;
            }
            d -= seg_len;
        }
        *self.points.last().unwrap_or(&self.points[0])
    }
}

/// An influence on an agent's movement. Each goal returns a steering acceleration for the agent's current state.
#[derive(Clone, Debug, PartialEq)]
pub enum Goal {
    /// Move toward a point at full speed.
    Seek(Vec2),
    /// Move away from a point at full speed.
    Flee(Vec2),
    /// Speed up or slow down along the current heading until the agent moves at this speed.
    ReachTargetSpeed(f32),
    /// Turn at random by up to `jitter` radians per update, moving at full speed.
    Wander { jitter: f32 },
    /// Steer clear of circles the agent would pass through within `max_prediction_time` seconds at full speed.
    Avoid { obstacles: Vec<Circle>, max_prediction_time: f32 },
    /// Follow the path forward. An agent off the path heads back to the nearest point first.
    FollowPath { path: Path, max_prediction_time: f32 },
}

/// The motion of an agent, as goals see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AgentState {
    pub position: Vec2,
    pub rotation: f32,
    pub velocity: Vec2,
    pub max_speed: f32,
    pub max_acceleration: f32,
    pub radius: f32,
}

/// Anything that yields a steering acceleration for an agent: a goal's weighted set or a composite of them.
pub trait Steering {
    fn acceleration(&self, state: &AgentState, rng: &mut Rng) -> Vec2;
}

/// The direction an agent is moving in, or where it faces when it is standing still.
fn heading(s: &AgentState) -> Vec2 {
    if s.velocity.length() > 1e-4 {
        s.velocity.normalized()
    } else {
        Vec2::from_angle(s.rotation)
    }
}

/// The steering that turns the current velocity into full speed along `dir`.
fn steer_towards(s: &AgentState, dir: Vec2) -> Vec2 {
    dir.normalized() * s.max_speed - s.velocity
}

impl Goal {
    fn acceleration(&self, s: &AgentState, rng: &mut Rng) -> Vec2 {
        match self {
            Goal::Seek(target) => steer_towards(s, *target - s.position),
            Goal::Flee(threat) => steer_towards(s, s.position - *threat),
            Goal::ReachTargetSpeed(speed) => heading(s) * *speed - s.velocity,
            Goal::Wander { jitter } => {
                let angle = {
                    let h = heading(s);
                    h.y.atan2(h.x) + rng.symmetric(*jitter)
                };
                steer_towards(s, Vec2::from_angle(angle))
            }
            Goal::Avoid { obstacles, max_prediction_time } => {
                let dir = heading(s);
                let look = s.max_speed * *max_prediction_time;
                let mut push = Vec2::ZERO;
                for o in obstacles {
                    let min = o.radius + s.radius;
                    let to = o.center - s.position;
                    let ahead = to.dot(dir);
                    let reach = look + min;
                    if ahead <= 0.0 || ahead > reach {
                        continue;
                    }
                    let lateral = to - dir * ahead;
                    let dist = lateral.length();
                    if dist >= min {
                        continue;
                    }
                    // Push away from the obstacle's side. Head-on, pick the left-hand side of the heading.
                    let side = if dist > 1e-6 {
                        lateral * (-1.0 / dist)
                    } else {
                        Vec2::new(-dir.y, dir.x)
                    };
                    let urgency = (1.0 - ahead / reach).clamp(0.0, 1.0);
                    push = push + side * (s.max_acceleration * urgency);
                }
                push
            }
            Goal::FollowPath { path, max_prediction_time } => {
                let here = path.nearest(s.position);
                let target = if (here.point - s.position).length() > path.radius() {
                    here.point
                } else {
                    path.point_at_distance(here.along + s.max_speed * *max_prediction_time)
                };
                steer_towards(s, target - s.position)
            }
        }
    }
}

/// Goals with weights. The weighted sum of their steering is the behavior's steering. Goals are addressed by the
/// index that `add` returns, so the same goal can be added twice with different weights.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Behavior {
    goals: Vec<(Goal, f32)>,
}

impl Behavior {
    pub fn new() -> Behavior {
        Behavior { goals: Vec::new() }
    }

    /// Adds a goal and returns its index.
    pub fn add(&mut self, goal: Goal, weight: f32) -> usize {
        self.goals.push((goal, weight));
        self.goals.len() - 1
    }

    /// Sets the weight of the goal at `index`. Returns false when there is no such goal.
    pub fn set_weight(&mut self, index: usize, weight: f32) -> bool {
        match self.goals.get_mut(index) {
            Some(entry) => {
                entry.1 = weight;
                true
            }
            None => false,
        }
    }

    pub fn weight(&self, index: usize) -> Option<f32> {
        self.goals.get(index).map(|e| e.1)
    }

    pub fn goal(&self, index: usize) -> Option<&Goal> {
        self.goals.get(index).map(|e| &e.0)
    }

    pub fn goal_count(&self) -> usize {
        self.goals.len()
    }

    /// Removes the goal at `index`, shifting later goals down by one.
    pub fn remove(&mut self, index: usize) -> Option<Goal> {
        if index < self.goals.len() {
            Some(self.goals.remove(index).0)
        } else {
            None
        }
    }
}

impl Steering for Behavior {
    fn acceleration(&self, state: &AgentState, rng: &mut Rng) -> Vec2 {
        let mut total = Vec2::ZERO;
        for (goal, weight) in &self.goals {
            total = total + goal.acceleration(state, rng) * *weight;
        }
        total
    }
}

/// Behaviors with weights, combined by weighted sum in the same way goals are within a behavior.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompositeBehavior {
    behaviors: Vec<(Behavior, f32)>,
}

impl CompositeBehavior {
    pub fn new() -> CompositeBehavior {
        CompositeBehavior { behaviors: Vec::new() }
    }

    /// Adds a behavior and returns its index.
    pub fn add(&mut self, behavior: Behavior, weight: f32) -> usize {
        self.behaviors.push((behavior, weight));
        self.behaviors.len() - 1
    }

    /// Sets the weight of the behavior at `index`. Returns false when there is no such behavior.
    pub fn set_weight(&mut self, index: usize, weight: f32) -> bool {
        match self.behaviors.get_mut(index) {
            Some(entry) => {
                entry.1 = weight;
                true
            }
            None => false,
        }
    }

    pub fn weight(&self, index: usize) -> Option<f32> {
        self.behaviors.get(index).map(|e| e.1)
    }

    pub fn behavior_count(&self) -> usize {
        self.behaviors.len()
    }

    /// Removes the behavior at `index`, shifting later ones down by one.
    pub fn remove(&mut self, index: usize) -> Option<Behavior> {
        if index < self.behaviors.len() {
            Some(self.behaviors.remove(index).0)
        } else {
            None
        }
    }
}

impl Steering for CompositeBehavior {
    fn acceleration(&self, state: &AgentState, rng: &mut Rng) -> Vec2 {
        let mut total = Vec2::ZERO;
        for (behavior, weight) in &self.behaviors {
            total = total + behavior.acceleration(state, rng) * *weight;
        }
        total
    }
}

/// Receives a call before and after each simulation step of an agent, so the app can sync its view.
pub trait AgentDelegate {
    fn agent_will_update(&mut self, _agent: &Agent) {}
    fn agent_did_update(&mut self, _agent: &Agent) {}
}

struct NoDelegate;

impl AgentDelegate for NoDelegate {}

/// A 2D agent. Each update adds the steering acceleration (limited to `max_acceleration`), limits the velocity to
/// `max_speed`, moves the position and turns the agent to face its velocity.
#[derive(Clone, Debug)]
pub struct Agent {
    pub position: Vec2,
    pub rotation: f32,
    pub velocity: Vec2,
    pub max_speed: f32,
    pub max_acceleration: f32,
    pub radius: f32,
    rng: Rng,
}

impl Agent {
    /// An agent at the origin, at rest, with a speed and acceleration limit of 100 and a radius of 10.
    pub fn new(seed: u64) -> Agent {
        Agent {
            position: Vec2::ZERO,
            rotation: 0.0,
            velocity: Vec2::ZERO,
            max_speed: 100.0,
            max_acceleration: 100.0,
            radius: 10.0,
            rng: Rng::new(seed),
        }
    }

    pub fn state(&self) -> AgentState {
        AgentState {
            position: self.position,
            rotation: self.rotation,
            velocity: self.velocity,
            max_speed: self.max_speed,
            max_acceleration: self.max_acceleration,
            radius: self.radius,
        }
    }

    /// Advances the agent by `dt` seconds under `steering`. A step with no positive, finite `dt` does nothing.
    pub fn update(&mut self, steering: &dyn Steering, dt: f32) {
        self.update_with_delegate(steering, dt, &mut NoDelegate);
    }

    /// As `update`, and calls the delegate before and after the step.
    pub fn update_with_delegate(&mut self, steering: &dyn Steering, dt: f32, delegate: &mut dyn AgentDelegate) {
        if !(dt > 0.0) || !dt.is_finite() {
            return;
        }
        delegate.agent_will_update(&*self);
        let state = self.state();
        let accel = steering.acceleration(&state, &mut self.rng).truncated(self.max_acceleration);
        self.velocity = (self.velocity + accel * dt).truncated(self.max_speed);
        self.position = self.position + self.velocity * dt;
        if self.velocity.length() > 1e-4 {
            self.rotation = self.velocity.y.atan2(self.velocity.x);
        }
        delegate.agent_did_update(&*self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn state(position: Vec2, velocity: Vec2, max_speed: f32, max_acc: f32) -> AgentState {
        AgentState { position, rotation: 0.0, velocity, max_speed, max_acceleration: max_acc, radius: 1.0 }
    }

    fn close(a: Vec2, b: Vec2) -> bool {
        (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3
    }

    #[test]
    fn seek_moves_toward_target_within_speed_limit() {
        let mut agent = Agent::new(1);
        agent.max_speed = 50.0;
        agent.max_acceleration = 200.0;
        let target = Vec2::new(300.0, 0.0);
        let mut behavior = Behavior::new();
        behavior.add(Goal::Seek(target), 1.0);
        for _ in 0..120 {
            agent.update(&behavior, DT);
            assert!(agent.velocity.length() <= 50.0 + 1e-3);
        }
        // Two seconds at a 50 speed with a one-second response: about 57 units covered.
        let remaining = (target - agent.position).length();
        assert!(remaining < 260.0, "agent should have closed in, remaining {remaining}");
        assert!(agent.position.x > 0.0);
    }

    #[test]
    fn flee_moves_away_from_threat() {
        let mut agent = Agent::new(2);
        agent.max_speed = 40.0;
        agent.max_acceleration = 200.0;
        let mut behavior = Behavior::new();
        behavior.add(Goal::Flee(Vec2::new(10.0, 0.0)), 1.0);
        for _ in 0..60 {
            agent.update(&behavior, DT);
        }
        assert!(agent.position.x < -5.0, "agent should be pushed away, at {:?}", agent.position);
    }

    #[test]
    fn acceleration_limit_bounds_the_first_step() {
        let mut agent = Agent::new(3);
        agent.max_speed = 100.0;
        agent.max_acceleration = 20.0;
        let mut behavior = Behavior::new();
        behavior.add(Goal::Seek(Vec2::new(1000.0, 0.0)), 1.0);
        agent.update(&behavior, 0.1);
        // From rest, one step at the limit gives at most max_acceleration * dt of speed.
        assert!(agent.velocity.length() <= 20.0 * 0.1 + 1e-4, "speed {}", agent.velocity.length());
        assert!(agent.velocity.length() > 0.0);
    }

    #[test]
    fn reach_target_speed_converges_on_the_speed() {
        let mut agent = Agent::new(4);
        agent.max_speed = 100.0;
        agent.max_acceleration = 100.0;
        let mut behavior = Behavior::new();
        behavior.add(Goal::ReachTargetSpeed(30.0), 1.0);
        for _ in 0..300 {
            agent.update(&behavior, DT);
        }
        assert!((agent.velocity.length() - 30.0).abs() < 0.5, "speed {}", agent.velocity.length());
    }

    #[test]
    fn behavior_weight_scales_goal_influence() {
        let s = state(Vec2::ZERO, Vec2::ZERO, 10.0, 100.0);
        let mut one = Behavior::new();
        one.add(Goal::Seek(Vec2::new(10.0, 0.0)), 1.0);
        let mut two = Behavior::new();
        two.add(Goal::Seek(Vec2::new(10.0, 0.0)), 2.0);
        let a1 = one.acceleration(&s, &mut Rng::new(1));
        let a2 = two.acceleration(&s, &mut Rng::new(1));
        assert!(close(a1, Vec2::new(10.0, 0.0)));
        assert!(close(a2, Vec2::new(20.0, 0.0)));
        assert!(two.set_weight(0, 0.0));
        assert!(close(two.acceleration(&s, &mut Rng::new(1)), Vec2::ZERO));
    }

    #[test]
    fn composite_sums_weighted_behaviors() {
        let s = state(Vec2::ZERO, Vec2::ZERO, 10.0, 100.0);
        let mut a = Behavior::new();
        a.add(Goal::Seek(Vec2::new(10.0, 0.0)), 1.0);
        let mut b = Behavior::new();
        b.add(Goal::Seek(Vec2::new(0.0, 10.0)), 1.0);
        let mut composite = CompositeBehavior::new();
        composite.add(a, 1.0);
        composite.add(b, 0.5);
        let acc = composite.acceleration(&s, &mut Rng::new(1));
        assert!(close(acc, Vec2::new(10.0, 5.0)), "got {:?}", acc);
        assert_eq!(composite.behavior_count(), 2);
    }

    #[test]
    fn path_nearest_point_and_distance_walk() {
        let path = Path::new(vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)], 2.0, false)
            .expect("valid path");
        assert_eq!(path.num_points(), 3);
        assert!((path.length() - 20.0).abs() < 1e-4);
        let near = path.nearest(Vec2::new(5.0, 3.0));
        assert_eq!(near.segment, 0);
        assert!(close(near.point, Vec2::new(5.0, 0.0)));
        assert!((near.along - 5.0).abs() < 1e-4);
        assert!(close(path.point_at_distance(15.0), Vec2::new(10.0, 5.0)));
        assert!(close(path.point_at_distance(100.0), Vec2::new(10.0, 10.0)));
        assert!(Path::new(vec![Vec2::ZERO], 1.0, false).is_none());
        assert!(Path::new(vec![Vec2::ZERO, Vec2::new(1.0, 0.0)], -1.0, false).is_none());
    }

    #[test]
    fn cyclical_path_wraps_distance() {
        let path = Path::new(vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)], 0.0, true)
            .expect("valid path");
        let total = path.length();
        assert!(total > 20.0, "closing segment adds length, total {total}");
        assert!(close(path.point_at_distance(3.0), path.point_at_distance(3.0 + total)));
    }

    #[test]
    fn follow_path_carries_agent_along_open_path() {
        let path = Path::new(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 5.0, false).expect("valid path");
        let mut agent = Agent::new(5);
        agent.max_speed = 20.0;
        agent.max_acceleration = 50.0;
        let mut behavior = Behavior::new();
        behavior.add(Goal::FollowPath { path, max_prediction_time: 0.5 }, 1.0);
        for _ in 0..300 {
            agent.update(&behavior, DT);
        }
        assert!(agent.position.x > 50.0, "agent should advance, at {:?}", agent.position);
        assert!(agent.position.y.abs() < 1.0, "agent should stay on the line, at {:?}", agent.position);
    }

    #[test]
    fn avoid_pushes_sideways_when_obstacle_is_ahead() {
        let s = state(Vec2::ZERO, Vec2::new(10.0, 0.0), 10.0, 50.0);
        let mut avoid = Behavior::new();
        avoid.add(
            Goal::Avoid { obstacles: vec![Circle { center: Vec2::new(20.0, 0.0), radius: 3.0 }], max_prediction_time: 3.0 },
            1.0,
        );
        let acc = avoid.acceleration(&s, &mut Rng::new(1));
        assert!(acc.y > 0.0, "should steer left of the heading, got {:?}", acc);
        assert!(acc.x.abs() < 1e-4, "avoidance should not slow the agent, got {:?}", acc);
    }

    #[test]
    fn delegate_is_called_around_each_step() {
        struct Recorder(Vec<&'static str>);
        impl AgentDelegate for Recorder {
            fn agent_will_update(&mut self, _agent: &Agent) {
                self.0.push("will");
            }
            fn agent_did_update(&mut self, _agent: &Agent) {
                self.0.push("did");
            }
        }
        let behavior = Behavior::new();
        let mut agent = Agent::new(6);
        let mut rec = Recorder(Vec::new());
        agent.update_with_delegate(&behavior, 0.1, &mut rec);
        assert_eq!(rec.0, vec!["will", "did"]);
        agent.update_with_delegate(&behavior, 0.0, &mut rec);
        assert_eq!(rec.0.len(), 2, "a zero step does nothing");
    }

    #[test]
    fn wander_is_repeatable_for_a_seed() {
        let run = |seed: u64| {
            let mut agent = Agent::new(seed);
            agent.max_speed = 30.0;
            agent.max_acceleration = 60.0;
            let mut behavior = Behavior::new();
            behavior.add(Goal::Wander { jitter: 0.5 }, 1.0);
            for _ in 0..100 {
                agent.update(&behavior, DT);
            }
            agent.position
        };
        assert_eq!(run(7), run(7));
        assert_ne!(run(7), run(8));
    }
}
