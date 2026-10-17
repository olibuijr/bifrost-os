//! Inverse kinematics for joint chains: given a chain of joints with fixed bone lengths and a target for the last
//! joint (the end effector), move the joints so the end reaches the target while the root stays put and every bone
//! keeps its length. Two solvers:
//!
//! - [`Chain::solve`] is FABRIK (forward and backward reaching): it works on a chain of any length, in 2D or 3D, by
//!   pulling the end onto the target and then pushing the root back to its place, bone by bone, until the end is within
//!   a tolerance. A target beyond the chain's reach leaves the chain straight and pointing at the target.
//! - [`solve_two_bone`] is the closed form for a limb of two bones (shoulder, elbow, wrist; hip, knee, ankle). A pole
//!   point says which way the middle joint bends.
//!
//! Pure maths over [`Vector3`]; it moves positions, not rotations. A skeleton (`skeleton.rs`) takes the joint positions
//! and turns them into the local rotations of its joints. Joint angle limits are not applied.

use crate::linalg::Vector3;

/// The result of a solve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Solution {
    /// The end joint is within the tolerance of the target.
    pub reached: bool,
    /// Passes over the chain that were needed (0 when the chain was already on the target or the target was out of
    /// reach and the chain was stretched in one move).
    pub iterations: usize,
    /// The distance from the end joint to the target after the solve.
    pub error: f32,
}

/// A chain of joints. Joint 0 is the root and the last joint is the end effector.
#[derive(Clone, Debug, PartialEq)]
pub struct Chain {
    joints: Vec<Vector3>,
    lengths: Vec<f32>,
}

fn direction(from: Vector3, to: Vector3) -> Vector3 {
    let d = (to - from).normalized();
    if d == Vector3::ZERO {
        Vector3::new(1.0, 0.0, 0.0)
    } else {
        d
    }
}

impl Chain {
    /// A chain through `joints`, root first. `None` for fewer than two joints, a joint that is not finite, or two
    /// neighbouring joints at the same place (a bone with no length has no direction).
    pub fn new(joints: &[Vector3]) -> Option<Chain> {
        if joints.len() < 2 || joints.iter().any(|j| !(j.x.is_finite() && j.y.is_finite() && j.z.is_finite())) {
            return None;
        }
        let lengths: Vec<f32> = joints.windows(2).map(|w| (w[1] - w[0]).length()).collect();
        if lengths.iter().any(|l| *l <= f32::EPSILON) {
            return None;
        }
        Some(Chain { joints: joints.to_vec(), lengths })
    }

    /// A straight chain that starts at `root` and runs along `direction`, one bone per entry of `lengths`. `None` for
    /// no bones, a bone that is not positive, or a zero direction.
    pub fn straight(root: Vector3, direction: Vector3, lengths: &[f32]) -> Option<Chain> {
        let dir = direction.normalized();
        if lengths.is_empty() || dir == Vector3::ZERO || lengths.iter().any(|l| !(*l > 0.0 && l.is_finite())) {
            return None;
        }
        let mut joints = vec![root];
        for l in lengths {
            let last = *joints.last().unwrap_or(&root);
            joints.push(last + dir * *l);
        }
        Some(Chain { joints, lengths: lengths.to_vec() })
    }

    /// The joint positions, root first.
    pub fn joints(&self) -> &[Vector3] {
        &self.joints
    }

    /// The bone lengths, root side first.
    pub fn lengths(&self) -> &[f32] {
        &self.lengths
    }

    /// The end effector.
    pub fn end(&self) -> Vector3 {
        self.joints[self.joints.len() - 1]
    }

    /// The farthest the end can be from the root: the sum of the bone lengths.
    pub fn reach(&self) -> f32 {
        self.lengths.iter().sum()
    }

    /// Moves the joints so the end is within `tolerance` of `target`, in at most `max_iterations` passes. The root does
    /// not move and the bone lengths are kept. A target beyond [`Chain::reach`] straightens the chain towards it.
    pub fn solve(&mut self, target: Vector3, tolerance: f32, max_iterations: usize) -> Solution {
        let n = self.joints.len();
        let root = self.joints[0];
        let tolerance = tolerance.max(0.0);
        if (target - root).length() >= self.reach() {
            // Out of reach: lay the bones in a line from the root towards the target.
            let dir = direction(root, target);
            for i in 0..n - 1 {
                self.joints[i + 1] = self.joints[i] + dir * self.lengths[i];
            }
            let error = (self.end() - target).length();
            return Solution { reached: error <= tolerance, iterations: 0, error };
        }
        let mut iterations = 0;
        while (self.end() - target).length() > tolerance && iterations < max_iterations {
            // Backward: the end goes onto the target and each joint is pulled to its bone length from the next one.
            self.joints[n - 1] = target;
            for i in (0..n - 1).rev() {
                let d = direction(self.joints[i + 1], self.joints[i]);
                self.joints[i] = self.joints[i + 1] + d * self.lengths[i];
            }
            // Forward: the root goes back to its place and each joint follows it.
            self.joints[0] = root;
            for i in 0..n - 1 {
                let d = direction(self.joints[i], self.joints[i + 1]);
                self.joints[i + 1] = self.joints[i] + d * self.lengths[i];
            }
            iterations += 1;
        }
        let error = (self.end() - target).length();
        Solution { reached: error <= tolerance, iterations, error }
    }
}

/// The closed form for a two-bone limb: the middle joint and the end of a limb whose root is `root`, with bones of
/// `upper` and `lower` length, reaching for `target`. The middle joint bends towards `pole`. A target that is too far
/// is reached as far as it goes with the limb straight; one that is too near is held at the shortest distance the
/// bones allow. Returns `[root, middle, end]`, or `None` for a bone that is not positive.
pub fn solve_two_bone(root: Vector3, upper: f32, lower: f32, target: Vector3, pole: Vector3) -> Option<[Vector3; 3]> {
    if !(upper > 0.0 && lower > 0.0 && upper.is_finite() && lower.is_finite()) {
        return None;
    }
    let to_target = target - root;
    let min_reach = (upper - lower).abs() + 1e-4;
    let max_reach = upper + lower - 1e-4;
    let d = to_target.length().clamp(min_reach, max_reach.max(min_reach));
    let u = direction(root, target);
    // The middle joint lies on a circle around the root-target line. `a` is how far along the line the circle sits,
    // `h` its radius.
    let a = (upper * upper - lower * lower + d * d) / (2.0 * d);
    let h = (upper * upper - a * a).max(0.0).sqrt();
    let toward_pole = pole - root;
    let mut bend = toward_pole - u * toward_pole.dot(u);
    if bend.length() <= 1e-6 {
        bend = u.cross(Vector3::new(0.0, 1.0, 0.0));
        if bend.length() <= 1e-6 {
            bend = u.cross(Vector3::new(1.0, 0.0, 0.0));
        }
    }
    let bend = bend.normalized();
    let middle = root + u * a + bend * h;
    let end = root + u * d;
    Some([root, middle, end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32, z: f32) -> Vector3 {
        Vector3::new(x, y, z)
    }

    fn close(a: Vector3, b: Vector3, eps: f32) -> bool {
        (a - b).length() <= eps
    }

    fn arm() -> Chain {
        Chain::straight(v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), &[2.0, 2.0, 1.0]).unwrap()
    }

    #[test]
    fn a_chain_needs_two_joints_with_distinct_places() {
        assert!(Chain::new(&[v(0.0, 0.0, 0.0)]).is_none());
        assert!(Chain::new(&[v(0.0, 0.0, 0.0), v(0.0, 0.0, 0.0)]).is_none());
        assert!(Chain::new(&[v(0.0, 0.0, 0.0), v(f32::NAN, 0.0, 0.0)]).is_none());
        let c = Chain::new(&[v(0.0, 0.0, 0.0), v(3.0, 4.0, 0.0)]).unwrap();
        assert_eq!(c.lengths(), &[5.0]);
        assert!(Chain::straight(v(0.0, 0.0, 0.0), Vector3::ZERO, &[1.0]).is_none());
        assert!(Chain::straight(v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), &[1.0, 0.0]).is_none());
    }

    #[test]
    fn a_reachable_target_is_reached_with_bones_and_root_kept() {
        let mut c = arm();
        let target = v(2.0, 2.0, 1.0);
        let s = c.solve(target, 1e-3, 64);
        assert!(s.reached, "{s:?}");
        assert!(close(c.end(), target, 2e-3));
        assert_eq!(c.joints()[0], v(0.0, 0.0, 0.0));
        for (i, w) in c.joints().windows(2).enumerate() {
            assert!(((w[1] - w[0]).length() - c.lengths()[i]).abs() < 1e-3, "bone {i}");
        }
    }

    #[test]
    fn a_target_out_of_reach_straightens_the_chain_towards_it() {
        let mut c = arm();
        let s = c.solve(v(0.0, 50.0, 0.0), 1e-3, 64);
        assert!(!s.reached);
        assert_eq!(s.iterations, 0);
        assert!(close(c.end(), v(0.0, 5.0, 0.0), 1e-4));
        assert!(close(c.joints()[1], v(0.0, 2.0, 0.0), 1e-4));
        assert!((s.error - 45.0).abs() < 1e-3);
    }

    #[test]
    fn a_chain_already_on_its_target_does_no_work() {
        let mut c = arm();
        let s = c.solve(v(5.0 - 1e-6, 0.0, 0.0), 1e-3, 64);
        assert!(s.reached);
        assert_eq!(s.iterations, 0);
    }

    #[test]
    fn the_iteration_cap_is_honoured() {
        let mut c = arm();
        let s = c.solve(v(1.0, 1.0, 0.0), 0.0, 2);
        assert!(s.iterations <= 2);
        let again = c.solve(v(1.0, 1.0, 0.0), 1e-2, 200);
        assert!(again.reached);
    }

    #[test]
    fn a_target_on_the_root_side_still_converges() {
        let mut c = arm();
        let s = c.solve(v(-1.0, 0.5, 0.0), 5e-3, 400);
        assert!(s.reached, "{s:?}");
        assert!(close(c.end(), v(-1.0, 0.5, 0.0), 1e-2));
    }

    #[test]
    fn a_two_bone_limb_reaches_with_its_bone_lengths() {
        let root = v(0.0, 0.0, 0.0);
        let target = v(2.0, 1.0, 0.0);
        let [r, mid, end] = solve_two_bone(root, 2.0, 2.0, target, v(0.0, 0.0, 5.0)).unwrap();
        assert_eq!(r, root);
        assert!(close(end, target, 1e-3));
        assert!(((mid - r).length() - 2.0).abs() < 1e-3);
        assert!(((end - mid).length() - 2.0).abs() < 1e-3);
    }

    #[test]
    fn the_pole_picks_the_side_the_middle_joint_bends_to() {
        let root = v(0.0, 0.0, 0.0);
        let target = v(3.0, 0.0, 0.0);
        let toward = solve_two_bone(root, 2.0, 2.0, target, v(1.0, 4.0, 0.0)).unwrap()[1];
        let away = solve_two_bone(root, 2.0, 2.0, target, v(1.0, -4.0, 0.0)).unwrap()[1];
        assert!(toward.y > 0.5);
        assert!(away.y < -0.5);
    }

    #[test]
    fn a_two_bone_limb_out_of_reach_is_straight_and_a_bad_bone_is_refused() {
        let [_, mid, end] = solve_two_bone(v(0.0, 0.0, 0.0), 1.0, 1.0, v(10.0, 0.0, 0.0), v(0.0, 1.0, 0.0)).unwrap();
        assert!(close(end, v(2.0, 0.0, 0.0), 5e-2));
        assert!(close(mid, v(1.0, 0.0, 0.0), 5e-2));
        assert!(solve_two_bone(v(0.0, 0.0, 0.0), 0.0, 1.0, v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)).is_none());
    }

    #[test]
    fn a_pole_on_the_limb_line_still_gives_a_bend() {
        let [_, mid, _] = solve_two_bone(v(0.0, 0.0, 0.0), 2.0, 2.0, v(3.0, 0.0, 0.0), v(9.0, 0.0, 0.0)).unwrap();
        assert!(mid.length() > 0.0);
        assert!((mid.length() - 2.0).abs() < 1e-3);
        assert!(mid.y.abs() + mid.z.abs() > 0.5);
    }
}
