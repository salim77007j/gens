//! Joint hierarchy and forward kinematics.
//!
//! A [`Skeleton`] is a flat list of [`Joint`]s whose parent indices are
//! strictly smaller than the joint's own index. That ordering is the
//! cycle-prevention contract: a joint can never depend on a descendant,
//! so a single forward pass in index order computes every global pose.
//!
//! Local joint poses are TRS ([`JointPose`]); global poses are composed
//! through [`gens_math::compose_trs`] and parent matrix multiplication —
//! pure `f32` math with no hidden state, so results are deterministic
//! within one process (see `docs/RISKS.md` R-013 for the cross-platform
//! bit-identity position, unchanged by this crate).

use gens_math::{Mat4, Quat, Vec3, compose_trs};
use thiserror::Error;

/// Errors from skeleton construction.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SkeletonError {
    /// A parent index did not point to an earlier joint.
    #[error("joint {joint} has parent {parent}: parent must be an earlier joint index (< {joint})")]
    ParentNotEarlier { joint: usize, parent: usize },
    /// Joint and parent name counts did not match.
    #[error("skeleton needs exactly one parent per joint: {joints} joints, {parents} parents")]
    ParentCountMismatch { joints: usize, parents: usize },
}

/// A named node in the joint hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Joint {
    /// Unique within the skeleton; used for lookups and debugging.
    pub name: String,
    /// Index of the parent joint, or `None` for roots.
    /// Always strictly smaller than this joint's own index.
    pub parent: Option<usize>,
}

/// A joint hierarchy: flat, topologically ordered, cycle-free by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skeleton {
    joints: Vec<Joint>,
}

impl Skeleton {
    /// Builds a skeleton from parallel name/parent slices.
    ///
    /// # Errors
    /// Returns [`SkeletonError`] when a parent index points at or past the
    /// joint itself, or the slices differ in length.
    pub fn new(names: &[&str], parents: &[Option<usize>]) -> Result<Self, SkeletonError> {
        if names.len() != parents.len() {
            return Err(SkeletonError::ParentCountMismatch {
                joints: names.len(),
                parents: parents.len(),
            });
        }
        let joints: Vec<Joint> = names
            .iter()
            .enumerate()
            .map(|(i, &name)| Joint {
                name: name.to_owned(),
                parent: parents[i],
            })
            .collect();
        for (i, joint) in joints.iter().enumerate() {
            if let Some(parent) = joint.parent
                && parent >= i
            {
                return Err(SkeletonError::ParentNotEarlier { joint: i, parent });
            }
        }
        Ok(Self { joints })
    }

    /// Builds a single-joint skeleton (the common root-only case).
    #[must_use]
    pub fn single_root(name: &str) -> Self {
        Self {
            joints: vec![Joint {
                name: name.to_owned(),
                parent: None,
            }],
        }
    }

    /// Number of joints.
    #[must_use]
    pub fn joint_count(&self) -> usize {
        self.joints.len()
    }

    /// Index of the joint with this name, if present.
    #[must_use]
    pub fn joint_index(&self, name: &str) -> Option<usize> {
        self.joints.iter().position(|j| j.name == name)
    }

    /// The flat joint list (index order = topological order).
    #[must_use]
    pub fn joints(&self) -> &[Joint] {
        &self.joints
    }

    /// Walks from a joint up to its root, collecting indices root-first.
    #[must_use]
    pub fn ancestor_chain(&self, joint: usize) -> Vec<usize> {
        let mut chain = Vec::new();
        let mut current = Some(joint);
        while let Some(i) = current {
            chain.push(i);
            current = self.joints[i].parent;
        }
        chain.reverse();
        chain
    }
}

/// Local (parent-relative) transform of one joint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointPose {
    /// Offset in the parent's space.
    pub translation: Vec3,
    /// Orientation in the parent's space.
    pub rotation: Quat,
    /// Non-uniform scale applied before rotation.
    pub scale: Vec3,
}

impl JointPose {
    /// The rest-pose-style identity: no translation, no rotation, unit scale.
    pub const IDENTITY: JointPose = JointPose {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// Builds a pose with only a translation (rotation identity, unit scale).
    #[must_use]
    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            ..Self::IDENTITY
        }
    }

    /// Builds a pose with only a rotation.
    #[must_use]
    pub fn from_rotation(rotation: Quat) -> Self {
        Self {
            rotation,
            ..Self::IDENTITY
        }
    }

    /// The local pose as a TRS matrix (T * R * S order, like `compose_trs`).
    #[must_use]
    pub fn to_matrix(&self) -> Mat4 {
        compose_trs(self.translation, self.rotation, self.scale)
    }
}

/// One dense local-pose array over a whole skeleton (index-aligned).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalPose {
    poses: Vec<JointPose>,
}

impl LocalPose {
    /// All-identity pose for `joint_count` joints.
    #[must_use]
    pub fn identity(joint_count: usize) -> Self {
        Self {
            poses: vec![JointPose::IDENTITY; joint_count],
        }
    }

    /// Joint count this pose is sized for.
    #[must_use]
    pub fn joint_count(&self) -> usize {
        self.poses.len()
    }

    /// Read access to one joint's local pose.
    ///
    /// # Panics
    /// Panics when `joint` is out of range.
    #[must_use]
    pub fn get(&self, joint: usize) -> &JointPose {
        &self.poses[joint]
    }

    /// Mutable access to one joint's local pose.
    ///
    /// # Panics
    /// Panics when `joint` is out of range.
    pub fn get_mut(&mut self, joint: usize) -> &mut JointPose {
        &mut self.poses[joint]
    }

    /// The raw dense array.
    #[must_use]
    pub fn as_slice(&self) -> &[JointPose] {
        &self.poses
    }

    /// Component-wise linear blend `a * w_a + b * w_b` per joint property.
    ///
    /// Rotations blend with [`Quat::slerp`] (shortest arc); weights must be
    /// normalized by the caller (a debug assertion checks their sum).
    #[must_use]
    pub fn blend(a: &LocalPose, w_a: f32, b: &LocalPose, w_b: f32) -> LocalPose {
        debug_assert!(
            (w_a + w_b - 1.0).abs() < 1e-4,
            "blend weights must sum to 1"
        );
        let n = a.poses.len();
        debug_assert_eq!(n, b.poses.len(), "poses must cover the same skeleton");
        let poses = (0..n)
            .map(|i| {
                let pa = a.poses[i];
                let pb = b.poses[i];
                JointPose {
                    translation: pa.translation.lerp(pb.translation, w_b),
                    rotation: pa.rotation.slerp(pb.rotation, w_b),
                    scale: pa.scale.lerp(pb.scale, w_b),
                }
            })
            .collect();
        LocalPose { poses }
    }

    /// Forward kinematics: one joint's global matrix from the local pose array.
    ///
    /// Walks the ancestor chain and composes parent-to-child. `O(depth)`.
    #[must_use]
    pub fn global_matrix(&self, skeleton: &Skeleton, joint: usize) -> Mat4 {
        let chain = skeleton.ancestor_chain(joint);
        let mut global = Mat4::IDENTITY;
        for &i in &chain {
            global *= self.poses[i].to_matrix();
        }
        global
    }

    /// Forward kinematics for every joint, in index order.
    ///
    /// Each global matrix is composed from its parent's *global* matrix, so
    /// the result is identical to per-joint [`Self::global_matrix`] calls
    /// while doing the work in one pass.
    #[must_use]
    pub fn global_matrices(&self, skeleton: &Skeleton) -> Vec<Mat4> {
        let mut out = Vec::with_capacity(self.poses.len());
        for (i, pose) in self.poses.iter().enumerate() {
            let global = match skeleton.joints[i].parent {
                Some(parent) => out[parent] * pose.to_matrix(),
                None => pose.to_matrix(),
            };
            out.push(global);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_math::vec3;

    fn two_joint_skeleton() -> Skeleton {
        Skeleton::new(&["root", "child"], &[None, Some(0)]).expect("valid hierarchy")
    }

    #[test]
    fn skeleton_rejects_forward_or_self_parent() {
        // Parent == own index and parent > own index are both cycles-in-waiting.
        assert_eq!(
            Skeleton::new(&["a"], &[Some(0)]),
            Err(SkeletonError::ParentNotEarlier {
                joint: 0,
                parent: 0
            })
        );
        assert!(matches!(
            Skeleton::new(&["a", "b"], &[None, Some(2)]),
            Err(SkeletonError::ParentNotEarlier {
                joint: 1,
                parent: 2
            })
        ));
    }

    #[test]
    fn skeleton_rejects_parent_count_mismatch() {
        assert_eq!(
            Skeleton::new(&["a", "b"], &[None]),
            Err(SkeletonError::ParentCountMismatch {
                joints: 2,
                parents: 1
            })
        );
    }

    #[test]
    fn ancestor_chain_is_root_first() {
        let s = Skeleton::new(
            &["hip", "spine", "head", "arm"],
            &[None, Some(0), Some(1), Some(0)],
        )
        .expect("valid");
        assert_eq!(s.ancestor_chain(2), vec![0, 1, 2]);
        assert_eq!(s.ancestor_chain(3), vec![0, 3]);
        assert_eq!(s.ancestor_chain(0), vec![0]);
    }

    #[test]
    fn joint_names_resolve_to_indices() {
        let s = two_joint_skeleton();
        assert_eq!(s.joint_index("child"), Some(1));
        assert_eq!(s.joint_index("missing"), None);
    }

    #[test]
    fn identity_pose_composes_to_identity_matrix() {
        let pose = JointPose::IDENTITY;
        assert_eq!(pose.to_matrix(), Mat4::IDENTITY);
    }

    #[test]
    fn translation_only_pose_keeps_axis_aligned_matrix() {
        let pose = JointPose::from_translation(vec3(1.0, 2.0, 3.0));
        assert_eq!(
            pose.to_matrix(),
            Mat4::from_translation(vec3(1.0, 2.0, 3.0))
        );
    }

    #[test]
    fn fk_two_joint_chain_matches_hand_composition() {
        let skeleton = two_joint_skeleton();
        let mut pose = LocalPose::identity(2);
        // Root moves right; child rotates 90 deg around Z at its local origin.
        pose.get_mut(0).translation = vec3(5.0, 0.0, 0.0);
        let quarter_turn = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        pose.get_mut(1).rotation = quarter_turn;

        let root_local = pose.get(0).to_matrix();
        let child_local = pose.get(1).to_matrix();
        let expected = root_local * child_local;
        assert_eq!(pose.global_matrix(&skeleton, 1), expected);

        // The root's global matrix is just its local one.
        assert_eq!(pose.global_matrix(&skeleton, 0), root_local);
    }

    #[test]
    fn fk_batch_equals_per_joint_composition() {
        let skeleton =
            Skeleton::new(&["root", "mid", "leaf"], &[None, Some(0), Some(1)]).expect("valid");
        let mut pose = LocalPose::identity(3);
        pose.get_mut(0).translation = vec3(1.0, 0.0, 0.0);
        pose.get_mut(1).translation = vec3(0.0, 2.0, 0.0);
        pose.get_mut(1).scale = vec3(2.0, 2.0, 2.0);
        pose.get_mut(2).rotation = Quat::from_rotation_y(0.5);

        let batch = pose.global_matrices(&skeleton);
        for (joint, batch_matrix) in batch.iter().enumerate() {
            assert_eq!(*batch_matrix, pose.global_matrix(&skeleton, joint));
        }
        // Root translation survives to the leaf through both hops.
        let leaf_translation = batch[2].w_axis.truncate();
        assert_eq!(leaf_translation, vec3(1.0, 2.0, 0.0));
    }

    #[test]
    fn parent_scale_scales_child_translation() {
        let skeleton = two_joint_skeleton();
        let mut pose = LocalPose::identity(2);
        pose.get_mut(0).scale = vec3(2.0, 2.0, 2.0);
        pose.get_mut(1).translation = vec3(1.0, 0.0, 0.0);

        let child_global = pose.global_matrix(&skeleton, 1);
        assert_eq!(child_global.w_axis.truncate(), vec3(2.0, 0.0, 0.0));
    }

    #[test]
    fn blend_half_way_between_translations() {
        let mut a = LocalPose::identity(1);
        let mut b = LocalPose::identity(1);
        a.get_mut(0).translation = vec3(0.0, 0.0, 0.0);
        b.get_mut(0).translation = vec3(4.0, 8.0, 0.0);
        b.get_mut(0).rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);

        let mid = LocalPose::blend(&a, 0.5, &b, 0.5);
        assert_eq!(mid.get(0).translation, vec3(2.0, 4.0, 0.0));
        // Slerp at t=0.5 of a 90 deg z-rotation is a 45 deg z-rotation.
        let expected = Quat::from_rotation_z(std::f32::consts::FRAC_PI_4);
        assert!((mid.get(0).rotation.dot(expected) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn blend_identity_pose_scales_linearly() {
        let mut b = LocalPose::identity(1);
        b.get_mut(0).scale = vec3(3.0, 3.0, 3.0);
        let quarter = LocalPose::blend(&LocalPose::identity(1), 0.75, &b, 0.25);
        assert_eq!(quarter.get(0).scale, vec3(1.5, 1.5, 1.5));
    }
}
