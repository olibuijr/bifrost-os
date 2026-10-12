//! Node constraints: rules that move or turn one node after its own position and rotation are set. A node can be kept
//! inside a range on one axis, kept in a distance band from a point, turned to face a point, or have its turn limited.
//! It is the counterpart of the 2D scene framework's node constraints for position, distance and orientation.
//!
//! Pure: a constraint maps a (position, rotation) pair to a new pair. Rotation is in radians, counter-clockwise, Y up,
//! like the physics world. A list is applied in order, each constraint to the result of the one before it. Only one
//! node's position and rotation are constrained: there is no reach or inverse-kinematics solver.
//!
//! Limits are inclusive. Pass the infinities for a side with no limit.

use crate::physics::Vec2;

/// One constraint on a node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NodeConstraint {
    /// The x of the position stays in `lower..=upper`.
    PositionX { lower: f32, upper: f32 },
    /// The y of the position stays in `lower..=upper`.
    PositionY { lower: f32, upper: f32 },
    /// The distance from `target` stays in `lower..=upper`. A node inside the lower radius is pushed out along the
    /// line from the target, and a node beyond the upper radius is pulled in. A node exactly on the target is placed
    /// on the positive x side, at the lower distance.
    Distance { target: Vec2, lower: f32, upper: f32 },
    /// The rotation points at `target`, plus `offset` radians.
    Orient { target: Vec2, offset: f32 },
    /// The rotation stays in `lower..=upper` radians.
    ZRotation { lower: f32, upper: f32 },
}

impl NodeConstraint {
    /// The position and rotation after this constraint.
    pub fn apply(self, position: Vec2, rotation: f32) -> (Vec2, f32) {
        match self {
            NodeConstraint::PositionX { lower, upper } => {
                (Vec2::new(clamp(position.x, lower, upper), position.y), rotation)
            }
            NodeConstraint::PositionY { lower, upper } => {
                (Vec2::new(position.x, clamp(position.y, lower, upper)), rotation)
            }
            NodeConstraint::Distance { target, lower, upper } => {
                let offset = position.minus(target);
                let dist = offset.length();
                let dir = if dist > 0.0 {
                    offset.scaled(1.0 / dist)
                } else {
                    Vec2::new(1.0, 0.0)
                };
                let wanted = clamp(dist, lower, upper);
                (target.plus(dir.scaled(wanted)), rotation)
            }
            NodeConstraint::Orient { target, offset } => {
                let v = target.minus(position);
                (position, v.y.atan2(v.x) + offset)
            }
            NodeConstraint::ZRotation { lower, upper } => (position, clamp(rotation, lower, upper)),
        }
    }
}

/// Applies `constraints` in list order and returns the final position and rotation.
pub fn apply_all(constraints: &[NodeConstraint], position: Vec2, rotation: f32) -> (Vec2, f32) {
    constraints
        .iter()
        .fold((position, rotation), |(p, r), c| c.apply(p, r))
}

fn clamp(v: f32, lower: f32, upper: f32) -> f32 {
    v.max(lower).min(upper)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn close_v(a: Vec2, b: Vec2) -> bool {
        close(a.x, b.x) && close(a.y, b.y)
    }

    #[test]
    fn position_x_clamps_only_the_x_coordinate() {
        let c = NodeConstraint::PositionX { lower: 0.0, upper: 2.0 };
        let (p, r) = c.apply(Vec2::new(5.0, 3.0), 0.7);
        assert!(close_v(p, Vec2::new(2.0, 3.0)));
        assert!(close(r, 0.7));
    }

    #[test]
    fn position_y_keeps_x_and_clamps_y() {
        let c = NodeConstraint::PositionY { lower: -1.0, upper: 1.0 };
        let (p, _) = c.apply(Vec2::new(4.0, -3.0), 0.0);
        assert!(close_v(p, Vec2::new(4.0, -1.0)));
    }

    #[test]
    fn orient_turns_the_node_to_face_the_target_with_an_offset() {
        // From the origin, the target (0, 1) is straight up: atan2(1, 0) = pi / 2.
        let face = NodeConstraint::Orient { target: Vec2::new(0.0, 1.0), offset: 0.0 };
        let (_, r) = face.apply(Vec2::ZERO, 0.0);
        assert!(close(r, PI / 2.0));
        let offset = NodeConstraint::Orient { target: Vec2::new(0.0, 1.0), offset: PI / 2.0 };
        let (_, r2) = offset.apply(Vec2::ZERO, 0.0);
        assert!(close(r2, PI));
    }

    #[test]
    fn distance_pushes_a_node_out_to_the_lower_radius() {
        // Band 1..2 around the origin. The node at distance 0.5 moves out to distance 1 along the same line.
        let c = NodeConstraint::Distance { target: Vec2::ZERO, lower: 1.0, upper: 2.0 };
        let (p, _) = c.apply(Vec2::new(0.5, 0.0), 0.0);
        assert!(close_v(p, Vec2::new(1.0, 0.0)));
    }

    #[test]
    fn distance_pulls_a_node_in_to_the_upper_radius() {
        // (3, 4) is 5 from the origin. Scaled by 2 / 5 it lands at (1.2, 1.6), which is 2 from the origin.
        let c = NodeConstraint::Distance { target: Vec2::ZERO, lower: 0.0, upper: 2.0 };
        let (p, _) = c.apply(Vec2::new(3.0, 4.0), 0.0);
        assert!(close_v(p, Vec2::new(1.2, 1.6)));
    }

    #[test]
    fn distance_inside_the_band_is_left_alone() {
        let c = NodeConstraint::Distance { target: Vec2::ZERO, lower: 1.0, upper: 2.0 };
        let (p, _) = c.apply(Vec2::new(1.5, 0.0), 0.0);
        assert!(close_v(p, Vec2::new(1.5, 0.0)));
    }

    #[test]
    fn z_rotation_limits_the_angle_to_its_range() {
        let c = NodeConstraint::ZRotation { lower: 0.0, upper: 1.0 };
        let (_, high) = c.apply(Vec2::ZERO, 4.0);
        let (_, low) = c.apply(Vec2::ZERO, -1.0);
        assert!(close(high, 1.0));
        assert!(close(low, 0.0));
    }

    #[test]
    fn apply_all_runs_the_constraints_in_order() {
        // Orient gives pi / 2 (about 1.5708). The rotation limit 0..1 then clamps it to 1.
        let list = [
            NodeConstraint::Orient { target: Vec2::new(0.0, 1.0), offset: 0.0 },
            NodeConstraint::ZRotation { lower: 0.0, upper: 1.0 },
        ];
        let (_, r) = apply_all(&list, Vec2::ZERO, 0.0);
        assert!(close(r, 1.0));
        let reversed = [list[1], list[0]];
        let (_, r2) = apply_all(&reversed, Vec2::ZERO, 0.0);
        assert!(close(r2, PI / 2.0));
    }
}
