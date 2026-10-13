//! Skeletons and joint animation. A skeleton is a tree of joints; each joint has a parent, a bind pose and an inverse
//! bind matrix. A clip holds tracks that give joints a rotation and a translation over time. Sampling a clip gives a
//! local pose; composing it along the tree gives world transforms; multiplying those by each joint's inverse bind
//! matrix gives the bone matrices that `morph_skin::Skinner` takes. Rotations interpolate with `Quaternion::slerp`,
//! translations linearly. Pure maths: no clock and no drawing.

use crate::linalg::{Matrix4, Quaternion, Vector3};

/// A joint's transform relative to its parent: a rotation, then a translation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Local {
    pub rotation: Quaternion,
    pub translation: Vector3,
}

impl Local {
    pub fn identity() -> Self {
        Local { rotation: Quaternion::identity(), translation: Vector3::ZERO }
    }

    fn matrix(&self) -> Matrix4 {
        let t = self.translation;
        Matrix4::translation(t.x, t.y, t.z) * self.rotation.to_matrix4()
    }
}

/// One joint as declared to a skeleton. `parent` must be an earlier joint in the list.
#[derive(Clone, Debug, PartialEq)]
pub struct JointSpec {
    pub name: String,
    pub parent: Option<usize>,
    pub bind: Local,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SkeletonError {
    Empty,
    ParentOutOfOrder { joint: usize },
    SingularBind { joint: usize },
    EmptyTrack { joint: usize },
    JointOutOfRange { joint: usize },
    UnsortedKeys { joint: usize },
    BadDuration,
    CountMismatch { expected: usize, found: usize },
}

/// A joint tree with the inverse bind matrix of every joint.
#[derive(Clone, Debug, PartialEq)]
pub struct Skeleton {
    joints: Vec<JointSpec>,
    inverse_bind: Vec<Matrix4>,
}

impl Skeleton {
    pub fn new(joints: Vec<JointSpec>) -> Result<Self, SkeletonError> {
        if joints.is_empty() {
            return Err(SkeletonError::Empty);
        }
        for (i, joint) in joints.iter().enumerate() {
            if let Some(p) = joint.parent {
                if p >= i {
                    return Err(SkeletonError::ParentOutOfOrder { joint: i });
                }
            }
        }
        let parents: Vec<Option<usize>> = joints.iter().map(|j| j.parent).collect();
        let binds: Vec<Local> = joints.iter().map(|j| j.bind).collect();
        let world = world_matrices(&parents, &binds);
        let mut inverse_bind = Vec::with_capacity(world.len());
        for (i, m) in world.iter().enumerate() {
            inverse_bind.push(m.inverse().ok_or(SkeletonError::SingularBind { joint: i })?);
        }
        Ok(Skeleton { joints, inverse_bind })
    }

    pub fn joint_count(&self) -> usize {
        self.joints.len()
    }

    pub fn joint(&self, index: usize) -> Option<&JointSpec> {
        self.joints.get(index)
    }

    /// The bind pose of every joint, in joint order.
    pub fn bind_pose(&self) -> Vec<Local> {
        self.joints.iter().map(|j| j.bind).collect()
    }

    /// The bone matrices for a pose: each joint's world transform times its inverse bind matrix. At the bind pose every
    /// bone matrix is the identity.
    pub fn bone_matrices(&self, pose: &[Local]) -> Result<Vec<Matrix4>, SkeletonError> {
        if pose.len() != self.joints.len() {
            return Err(SkeletonError::CountMismatch { expected: self.joints.len(), found: pose.len() });
        }
        let parents: Vec<Option<usize>> = self.joints.iter().map(|j| j.parent).collect();
        let world = world_matrices(&parents, pose);
        Ok(world.iter().zip(&self.inverse_bind).map(|(w, inv)| *w * *inv).collect())
    }
}

/// World transforms for a pose. Parents come before their children, so each parent is already in `world`.
fn world_matrices(parents: &[Option<usize>], locals: &[Local]) -> Vec<Matrix4> {
    let mut world: Vec<Matrix4> = Vec::with_capacity(locals.len());
    for (i, local) in locals.iter().enumerate() {
        let m = match parents[i] {
            Some(p) => world[p] * local.matrix(),
            None => local.matrix(),
        };
        world.push(m);
    }
    world
}

/// A key on a track: the joint's local transform at a time in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub time: f32,
    pub local: Local,
}

/// The keys for one joint, in strictly increasing time order.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub joint: usize,
    pub keys: Vec<Key>,
}

impl Track {
    fn value_at(&self, t: f32) -> Local {
        let keys = &self.keys;
        if t <= keys[0].time {
            return keys[0].local;
        }
        let last = keys[keys.len() - 1];
        if t >= last.time {
            return last.local;
        }
        // The first key is at or before t, and the last key is after it, so the index is in range.
        let i = keys.partition_point(|k| k.time <= t) - 1;
        let (a, b) = (keys[i], keys[i + 1]);
        let alpha = (t - a.time) / (b.time - a.time);
        Local {
            rotation: Quaternion::slerp(a.local.rotation, b.local.rotation, alpha),
            translation: a.local.translation * (1.0 - alpha) + b.local.translation * alpha,
        }
    }
}

/// An animation clip over a skeleton. Joints without a track keep their bind pose.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    duration: f32,
    bind: Vec<Local>,
    tracks: Vec<Track>,
}

impl Clip {
    /// Checks the tracks against the skeleton: every joint exists, every track has keys, and the keys are in time order.
    pub fn new(skeleton: &Skeleton, duration: f32, tracks: Vec<Track>) -> Result<Self, SkeletonError> {
        if !(duration.is_finite() && duration > 0.0) {
            return Err(SkeletonError::BadDuration);
        }
        for track in &tracks {
            let joint = track.joint;
            if joint >= skeleton.joint_count() {
                return Err(SkeletonError::JointOutOfRange { joint });
            }
            if track.keys.is_empty() {
                return Err(SkeletonError::EmptyTrack { joint });
            }
            let finite = track.keys.iter().all(|k| k.time.is_finite());
            let ordered = track.keys.windows(2).all(|w| w[0].time < w[1].time);
            if !(finite && ordered) {
                return Err(SkeletonError::UnsortedKeys { joint });
            }
        }
        Ok(Clip { duration, bind: skeleton.bind_pose(), tracks })
    }

    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// The local pose of every joint at `time` seconds. Times outside the clip are clamped to its ends.
    pub fn sample(&self, time: f32) -> Vec<Local> {
        let t = if time.is_finite() { time.clamp(0.0, self.duration) } else { 0.0 };
        let mut pose = self.bind.clone();
        for track in &self.tracks {
            pose[track.joint] = track.value_at(t);
        }
        pose
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn at(x: f32, y: f32, z: f32) -> Local {
        Local { rotation: Quaternion::identity(), translation: Vector3::new(x, y, z) }
    }

    fn joint(name: &str, parent: Option<usize>, bind: Local) -> JointSpec {
        JointSpec { name: name.to_string(), parent, bind }
    }

    /// A root at (1, 0, 0) with a child 1 unit above it.
    fn two_joints() -> Skeleton {
        Skeleton::new(vec![joint("root", None, at(1.0, 0.0, 0.0)), joint("child", Some(0), at(0.0, 1.0, 0.0))])
            .expect("valid skeleton")
    }

    #[test]
    fn bind_pose_gives_identity_bone_matrices() {
        let skeleton = two_joints();
        let bones = skeleton.bone_matrices(&skeleton.bind_pose()).expect("pose matches");
        for m in bones {
            assert_eq!(m, Matrix4::identity());
        }
    }

    #[test]
    fn moving_a_child_joint_moves_its_bone_by_the_same_amount() {
        let skeleton = two_joints();
        let mut pose = skeleton.bind_pose();
        pose[1].translation = Vector3::new(0.0, 3.0, 0.0);
        let bones = skeleton.bone_matrices(&pose).expect("pose matches");
        assert!(close(bones[0].m[12], 0.0) && close(bones[0].m[13], 0.0));
        assert!(close(bones[1].m[12], 0.0));
        assert!(close(bones[1].m[13], 2.0));
    }

    #[test]
    fn clip_interpolates_translation_between_keys() {
        let skeleton = two_joints();
        let track = Track {
            joint: 1,
            keys: vec![Key { time: 0.0, local: at(0.0, 1.0, 0.0) }, Key { time: 1.0, local: at(0.0, 3.0, 0.0) }],
        };
        let clip = Clip::new(&skeleton, 1.0, vec![track]).expect("valid clip");
        let pose = clip.sample(0.5);
        assert!(close(pose[1].translation.y, 2.0));
        assert!(close(pose[0].translation.x, 1.0), "root without a track keeps its bind pose");
    }

    #[test]
    fn clip_slerps_rotation_halfway_to_a_quarter_turn() {
        let skeleton = two_joints();
        let z = Vector3::new(0.0, 0.0, 1.0);
        let end = Local { rotation: Quaternion::from_axis_angle(z, FRAC_PI_2), translation: Vector3::ZERO };
        let track = Track {
            joint: 0,
            keys: vec![Key { time: 0.0, local: Local::identity() }, Key { time: 1.0, local: end }],
        };
        let clip = Clip::new(&skeleton, 1.0, vec![track]).expect("valid clip");
        let got = clip.sample(0.5)[0].rotation;
        let want = Quaternion::from_axis_angle(z, FRAC_PI_2 * 0.5);
        assert!(close(got.z, want.z) && close(got.w, want.w));
        assert!(close(got.x, 0.0) && close(got.y, 0.0));
    }

    #[test]
    fn times_outside_the_clip_are_clamped_to_its_ends() {
        let skeleton = two_joints();
        let track = Track {
            joint: 1,
            keys: vec![Key { time: 0.0, local: at(0.0, 1.0, 0.0) }, Key { time: 1.0, local: at(0.0, 5.0, 0.0) }],
        };
        let clip = Clip::new(&skeleton, 1.0, vec![track]).expect("valid clip");
        assert!(close(clip.sample(-4.0)[1].translation.y, 1.0));
        assert!(close(clip.sample(9.0)[1].translation.y, 5.0));
        assert!(close(clip.sample(f32::NAN)[1].translation.y, 1.0));
    }

    #[test]
    fn a_child_joint_must_come_after_its_parent() {
        let err = Skeleton::new(vec![joint("a", Some(0), Local::identity())]).unwrap_err();
        assert_eq!(err, SkeletonError::ParentOutOfOrder { joint: 0 });
        assert_eq!(Skeleton::new(Vec::new()).unwrap_err(), SkeletonError::Empty);
    }

    #[test]
    fn clip_rejects_bad_tracks_and_durations() {
        let skeleton = two_joints();
        let key = |t: f32| Key { time: t, local: Local::identity() };
        assert_eq!(Clip::new(&skeleton, 0.0, Vec::new()).unwrap_err(), SkeletonError::BadDuration);
        let missing = Track { joint: 9, keys: vec![key(0.0)] };
        assert_eq!(
            Clip::new(&skeleton, 1.0, vec![missing]).unwrap_err(),
            SkeletonError::JointOutOfRange { joint: 9 }
        );
        let empty = Track { joint: 0, keys: Vec::new() };
        assert_eq!(Clip::new(&skeleton, 1.0, vec![empty]).unwrap_err(), SkeletonError::EmptyTrack { joint: 0 });
        let unsorted = Track { joint: 0, keys: vec![key(1.0), key(0.5)] };
        assert_eq!(Clip::new(&skeleton, 1.0, vec![unsorted]).unwrap_err(), SkeletonError::UnsortedKeys { joint: 0 });
    }

    #[test]
    fn bone_matrices_reject_a_pose_of_the_wrong_length() {
        let skeleton = two_joints();
        assert_eq!(
            skeleton.bone_matrices(&[Local::identity()]).unwrap_err(),
            SkeletonError::CountMismatch { expected: 2, found: 1 }
        );
    }
}
