//! Shape morphing and skeletal skinning as pure models (`NativeMorpher`, `NativeSkinner`). A morpher blends a base
//! vertex array toward named target arrays with weights. A skinner moves each vertex by a weighted mix of bone
//! matrices. Both return new position arrays and own no GPU memory; a renderer uploads the result.
//!
//! Morph rule. Result = base * (1 - sum(w)) + sum(w_i * target_i), which is base + sum(w_i * (target_i - base)).
//! `Additive` uses the weights as given. `Normalized` clamps each weight to 0..=1 and, when the clamped sum exceeds
//! one, divides every weight by that sum, so the base keeps a non-negative share.
//!
//! Skin rule. Each bone has a skin matrix pose * inverse_bind. A vertex first goes through the base bind transform,
//! then through the weighted sum of its bones' skin matrices. A vertex with no influences stays where it is.
//! Weights are used as given; they are not renormalized. Normals are not skinned.

use crate::linalg::{Matrix4, Vector3};

/// The blend formula (`NativeMorpherCalculationMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MorphCalculation {
    /// Weights must lie in 0..=1, and the base keeps the share left over from the weights.
    Normalized,
    /// Weights may take any value; each target's offset from the base is added.
    Additive,
}

/// One target shape: a full vertex array the same length as the base, with its current weight.
#[derive(Clone, Debug, PartialEq)]
pub struct MorphTarget {
    pub name: String,
    pub positions: Vec<Vector3>,
    weight: f32,
}

impl MorphTarget {
    pub fn weight(&self) -> f32 {
        self.weight
    }
}

/// Errors from building morph or skin data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MorphSkinError {
    /// A target or bone array does not match the vertex count or bone count it must agree with.
    CountMismatch { expected: usize, found: usize },
    /// A bone index in an influence list is not a bone of the skeleton.
    BoneIndexOutOfRange { bone: usize, bones: usize },
    /// A weight is negative or not finite.
    InvalidWeight,
}

/// Blends a base vertex array toward its targets (`NativeMorpher`).
#[derive(Clone, Debug, PartialEq)]
pub struct Morpher {
    base: Vec<Vector3>,
    targets: Vec<MorphTarget>,
    calculation: MorphCalculation,
}

impl Morpher {
    pub fn new(base: Vec<Vector3>, calculation: MorphCalculation) -> Self {
        Morpher { base, targets: Vec::new(), calculation }
    }

    pub fn calculation(&self) -> MorphCalculation {
        self.calculation
    }

    pub fn set_calculation(&mut self, calculation: MorphCalculation) {
        self.calculation = calculation;
    }

    pub fn targets(&self) -> &[MorphTarget] {
        &self.targets
    }

    /// Adds a target with weight zero and returns its index. The target must have one position per base vertex.
    pub fn add_target(&mut self, name: &str, positions: Vec<Vector3>) -> Result<usize, MorphSkinError> {
        if positions.len() != self.base.len() {
            return Err(MorphSkinError::CountMismatch { expected: self.base.len(), found: positions.len() });
        }
        self.targets.push(MorphTarget { name: name.to_string(), positions, weight: 0.0 });
        Ok(self.targets.len() - 1)
    }

    /// The weight of the target at `index`, or `None` when there is no such target.
    pub fn weight(&self, index: usize) -> Option<f32> {
        self.targets.get(index).map(|t| t.weight)
    }

    /// Sets the weight of the target at `index`. Returns `false` when there is no such target or the weight is not
    /// finite.
    pub fn set_weight(&mut self, index: usize, weight: f32) -> bool {
        if !weight.is_finite() {
            return false;
        }
        match self.targets.get_mut(index) {
            Some(t) => {
                t.weight = weight;
                true
            }
            None => false,
        }
    }

    /// The index of the first target called `name`.
    pub fn target_index(&self, name: &str) -> Option<usize> {
        self.targets.iter().position(|t| t.name == name)
    }

    pub fn weight_named(&self, name: &str) -> Option<f32> {
        self.target_index(name).and_then(|i| self.weight(i))
    }

    pub fn set_weight_named(&mut self, name: &str, weight: f32) -> bool {
        match self.target_index(name) {
            Some(i) => self.set_weight(i, weight),
            None => false,
        }
    }

    /// The morphed positions for the current weights.
    pub fn positions(&self) -> Vec<Vector3> {
        let mut weights: Vec<f32> = self.targets.iter().map(|t| t.weight).collect();
        if self.calculation == MorphCalculation::Normalized {
            for w in weights.iter_mut() {
                *w = w.clamp(0.0, 1.0);
            }
            let total: f32 = weights.iter().sum();
            if total > 1.0 {
                for w in weights.iter_mut() {
                    *w /= total;
                }
            }
        }
        let total: f32 = weights.iter().sum();
        let mut out: Vec<Vector3> = self.base.iter().map(|b| *b * (1.0 - total)).collect();
        for (t, w) in self.targets.iter().zip(weights.iter()) {
            for (o, p) in out.iter_mut().zip(t.positions.iter()) {
                *o = *o + *p * *w;
            }
        }
        out
    }
}

/// One bone's share of a vertex (`boneIndices` and `boneWeights` in one pair).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneInfluence {
    pub bone: usize,
    pub weight: f32,
}

/// Moves a base vertex array with a skeleton (`NativeSkinner`).
#[derive(Clone, Debug, PartialEq)]
pub struct Skinner {
    base: Vec<Vector3>,
    bind_transform: Matrix4,
    inverse_bind: Vec<Matrix4>,
    influences: Vec<Vec<BoneInfluence>>,
    pose: Vec<Matrix4>,
}

impl Skinner {
    /// Builds a skinner. `influences` holds one list per base vertex, and `inverse_bind` one matrix per bone. The pose
    /// starts at the bind pose (identity for every bone).
    pub fn new(
        base: Vec<Vector3>,
        bind_transform: Matrix4,
        inverse_bind: Vec<Matrix4>,
        influences: Vec<Vec<BoneInfluence>>,
    ) -> Result<Self, MorphSkinError> {
        if influences.len() != base.len() {
            return Err(MorphSkinError::CountMismatch { expected: base.len(), found: influences.len() });
        }
        let bones = inverse_bind.len();
        for list in &influences {
            for inf in list {
                if inf.bone >= bones {
                    return Err(MorphSkinError::BoneIndexOutOfRange { bone: inf.bone, bones });
                }
                if !(inf.weight.is_finite() && inf.weight >= 0.0) {
                    return Err(MorphSkinError::InvalidWeight);
                }
            }
        }
        let pose = vec![Matrix4::identity(); bones];
        Ok(Skinner { base, bind_transform, inverse_bind, influences, pose })
    }

    pub fn bone_count(&self) -> usize {
        self.inverse_bind.len()
    }

    pub fn pose(&self) -> &[Matrix4] {
        &self.pose
    }

    /// Sets the current bone transforms. The count must match the skeleton.
    pub fn set_pose(&mut self, pose: Vec<Matrix4>) -> Result<(), MorphSkinError> {
        if pose.len() != self.inverse_bind.len() {
            return Err(MorphSkinError::CountMismatch { expected: self.inverse_bind.len(), found: pose.len() });
        }
        self.pose = pose;
        Ok(())
    }

    /// The skinned vertex positions for the current pose.
    pub fn positions(&self) -> Vec<Vector3> {
        let skins: Vec<Matrix4> = self
            .pose
            .iter()
            .zip(self.inverse_bind.iter())
            .map(|(p, inv)| *p * *inv)
            .collect();
        let mut out = Vec::with_capacity(self.base.len());
        for (v, list) in self.base.iter().zip(self.influences.iter()) {
            if list.is_empty() {
                out.push(*v);
                continue;
            }
            let mut acc = [0.0f32; 16];
            for inf in list {
                let s = &skins[inf.bone];
                for k in 0..16 {
                    acc[k] += s.m[k] * inf.weight;
                }
            }
            let blended = Matrix4 { m: acc };
            let bound = self.bind_transform.transform_point(*v).unwrap_or(*v);
            out.push(blended.transform_point(bound).unwrap_or(*v));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    fn assert_vec3(actual: Vector3, expected: Vector3) {
        assert!(
            close(actual.x, expected.x) && close(actual.y, expected.y) && close(actual.z, expected.z),
            "expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn additive_morph_adds_each_offset_scaled_by_its_weight() {
        let mut m = Morpher::new(vec![Vector3::ZERO], MorphCalculation::Additive);
        let i = m.add_target("push", vec![Vector3::new(2.0, 0.0, 0.0)]).unwrap();
        assert!(m.set_weight(i, 0.5));
        assert_vec3(m.positions()[0], Vector3::new(1.0, 0.0, 0.0));
        assert!(m.set_weight(i, 1.0));
        assert_vec3(m.positions()[0], Vector3::new(2.0, 0.0, 0.0));
        assert!(m.set_weight(i, 2.0));
        assert_vec3(m.positions()[0], Vector3::new(4.0, 0.0, 0.0));
    }

    #[test]
    fn normalized_morph_gives_the_base_the_share_the_weights_leave() {
        let mut m = Morpher::new(vec![Vector3::ZERO], MorphCalculation::Normalized);
        let a = m.add_target("a", vec![Vector3::new(10.0, 0.0, 0.0)]).unwrap();
        let b = m.add_target("b", vec![Vector3::new(0.0, 10.0, 0.0)]).unwrap();
        m.set_weight(a, 0.25);
        m.set_weight(b, 0.25);
        // Base weight is 1 - 0.5 = 0.5 and the targets contribute 2.5 each.
        assert_vec3(m.positions()[0], Vector3::new(2.5, 2.5, 0.0));
    }

    #[test]
    fn normalized_weights_are_clamped_and_rescaled_when_their_sum_passes_one() {
        let mut m = Morpher::new(vec![Vector3::ZERO], MorphCalculation::Normalized);
        let a = m.add_target("a", vec![Vector3::new(10.0, 0.0, 0.0)]).unwrap();
        let b = m.add_target("b", vec![Vector3::new(0.0, 10.0, 0.0)]).unwrap();
        m.set_weight(a, 1.0);
        m.set_weight(b, 1.0);
        // Each weight becomes 0.5, the base share is zero.
        assert_vec3(m.positions()[0], Vector3::new(5.0, 5.0, 0.0));
        m.set_weight(a, -3.0);
        m.set_weight(b, 2.0);
        // Negative clamps to zero, and 2.0 clamps to one.
        assert_vec3(m.positions()[0], Vector3::new(0.0, 10.0, 0.0));
    }

    #[test]
    fn target_with_wrong_vertex_count_is_rejected_and_named_weights_work() {
        let mut m = Morpher::new(vec![Vector3::ZERO], MorphCalculation::Additive);
        assert_eq!(
            m.add_target("bad", vec![Vector3::ZERO, Vector3::ZERO]),
            Err(MorphSkinError::CountMismatch { expected: 1, found: 2 })
        );
        m.add_target("smile", vec![Vector3::new(1.0, 0.0, 0.0)]).unwrap();
        assert!(m.set_weight_named("smile", 0.5));
        assert_eq!(m.weight_named("smile"), Some(0.5));
        assert!(!m.set_weight_named("frown", 0.5));
        assert_eq!(m.weight(7), None);
        assert!(!m.set_weight(0, f32::NAN));
    }

    #[test]
    fn one_bone_translation_moves_every_influenced_vertex() {
        let mut s = Skinner::new(
            vec![Vector3::new(0.0, 1.0, 0.0)],
            Matrix4::identity(),
            vec![Matrix4::identity()],
            vec![vec![BoneInfluence { bone: 0, weight: 1.0 }]],
        )
        .unwrap();
        s.set_pose(vec![Matrix4::translation(1.0, 0.0, 0.0)]).unwrap();
        assert_vec3(s.positions()[0], Vector3::new(1.0, 1.0, 0.0));
    }

    #[test]
    fn two_equal_bones_average_their_transforms() {
        let s0 = Matrix4::identity();
        let s1 = Matrix4::translation(2.0, 0.0, 0.0);
        let mut s = Skinner::new(
            vec![Vector3::ZERO],
            Matrix4::identity(),
            vec![Matrix4::identity(), Matrix4::identity()],
            vec![vec![BoneInfluence { bone: 0, weight: 0.5 }, BoneInfluence { bone: 1, weight: 0.5 }]],
        )
        .unwrap();
        s.set_pose(vec![s0, s1]).unwrap();
        assert_vec3(s.positions()[0], Vector3::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn skin_matrix_is_pose_times_inverse_bind() {
        let mut s = Skinner::new(
            vec![Vector3::ZERO],
            Matrix4::identity(),
            vec![Matrix4::translation(-1.0, 0.0, 0.0)],
            vec![vec![BoneInfluence { bone: 0, weight: 1.0 }]],
        )
        .unwrap();
        s.set_pose(vec![Matrix4::translation(3.0, 0.0, 0.0)]).unwrap();
        // Inverse bind moves the origin to (-1, 0, 0), then the pose adds (3, 0, 0).
        assert_vec3(s.positions()[0], Vector3::new(2.0, 0.0, 0.0));
    }

    #[test]
    fn invalid_skin_data_is_rejected_and_unweighted_vertices_stay_put() {
        let bad_bone = Skinner::new(
            vec![Vector3::ZERO],
            Matrix4::identity(),
            vec![Matrix4::identity()],
            vec![vec![BoneInfluence { bone: 1, weight: 1.0 }]],
        );
        assert_eq!(bad_bone.err(), Some(MorphSkinError::BoneIndexOutOfRange { bone: 1, bones: 1 }));
        let bad_weight = Skinner::new(
            vec![Vector3::ZERO],
            Matrix4::identity(),
            vec![Matrix4::identity()],
            vec![vec![BoneInfluence { bone: 0, weight: -1.0 }]],
        );
        assert_eq!(bad_weight.err(), Some(MorphSkinError::InvalidWeight));
        let mut s = Skinner::new(
            vec![Vector3::new(4.0, 5.0, 6.0)],
            Matrix4::identity(),
            vec![Matrix4::identity()],
            vec![vec![]],
        )
        .unwrap();
        s.set_pose(vec![Matrix4::translation(9.0, 9.0, 9.0)]).unwrap();
        assert_vec3(s.positions()[0], Vector3::new(4.0, 5.0, 6.0));
        assert!(s.set_pose(vec![]).is_err());
    }
}
