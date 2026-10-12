//! Pose blending primitives.
//!
//! Two operations cover the controller's needs:
//!
//! - [`blend_into`] — in-place `dst = lerp(dst, src, weight)`; composing
//!   this over several sources builds n-way blends without scratch poses.
//! - [`crossfade`] — a timed, clamped transition between two complete
//!   poses: `t = 0` returns `from` exactly, `t = 1` returns `to` exactly.
//!
//! All math is the pure f32 lerp/slerp already pinned by the track tests,
//! so blending adds no new determinism surface (R-013).

use gens_math::Quat;

use crate::skeleton::{JointPose, LocalPose};

/// In-place partial blend: `dst = lerp(dst, src, weight)`.
///
/// `weight = 0` leaves `dst` untouched, `weight = 1` copies `src`.
/// Poses must cover the same skeleton (debug-asserted).
pub fn blend_into(dst: &mut LocalPose, src: &LocalPose, weight: f32) {
    debug_assert_eq!(
        dst.joint_count(),
        src.joint_count(),
        "poses must cover the same skeleton"
    );
    debug_assert!(
        (0.0..=1.0).contains(&weight),
        "blend weight out of range: {weight}"
    );
    for (i, slot) in dst.poses_mut().iter_mut().enumerate() {
        let s = src.get(i);
        *slot = JointPose {
            translation: slot.translation.lerp(s.translation, weight),
            rotation: slot.rotation.slerp(s.rotation, weight),
            scale: slot.scale.lerp(s.scale, weight),
        };
    }
}

/// Timed crossfade between two complete poses.
///
/// `t` is clamped to `[0, 1]`; rotations take the shortest arc
/// (`Quat::slerp`). Pure: same inputs, same output, every call.
#[must_use]
pub fn crossfade(from: &LocalPose, to: &LocalPose, t: f32) -> LocalPose {
    let w = t.clamp(0.0, 1.0);
    LocalPose::blend(from, 1.0 - w, to, w)
}

/// Quaternion helper shared with blend implementations elsewhere.
///
/// Exposed for tests and tooling: the shortest-arc weight semantics of
/// [`Quat::slerp`] are the crate's rotation-blending contract.
#[must_use]
pub fn slerp_rotation(a: Quat, b: Quat, t: f32) -> Quat {
    a.slerp(b, t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_math::{Quat, vec3};

    fn pose_with(joint: usize, translation: gens_math::Vec3) -> LocalPose {
        let mut pose = LocalPose::identity(joint + 1);
        pose.get_mut(joint).translation = translation;
        pose
    }

    #[test]
    fn blend_into_weight_zero_and_one_are_exact() {
        let mut dst = pose_with(0, vec3(1.0, 0.0, 0.0));
        let src = pose_with(0, vec3(5.0, 0.0, 0.0));

        blend_into(&mut dst, &src, 0.0);
        assert_eq!(dst.get(0).translation, vec3(1.0, 0.0, 0.0));

        blend_into(&mut dst, &src, 1.0);
        assert_eq!(dst.get(0).translation, vec3(5.0, 0.0, 0.0));
    }

    #[test]
    fn blend_into_half_way_is_half_way() {
        let mut dst = pose_with(0, vec3(0.0, 0.0, 0.0));
        let src = pose_with(0, vec3(10.0, 20.0, 0.0));
        blend_into(&mut dst, &src, 0.25);
        assert_eq!(dst.get(0).translation, vec3(2.5, 5.0, 0.0));
    }

    #[test]
    fn crossfade_endpoints_are_exact() {
        let a = pose_with(0, vec3(0.0, 0.0, 0.0));
        let b = pose_with(0, vec3(8.0, 0.0, 0.0));
        let start = crossfade(&a, &b, 0.0);
        let end = crossfade(&a, &b, 1.0);
        let over = crossfade(&a, &b, 7.0); // clamped
        assert_eq!(start.get(0).translation, a.get(0).translation);
        assert_eq!(end.get(0).translation, b.get(0).translation);
        assert_eq!(over.get(0).translation, b.get(0).translation);
    }

    #[test]
    fn crossfade_midpoint_blends_rotation_shortest_arc() {
        let mut a = LocalPose::identity(1);
        let mut b = LocalPose::identity(1);
        a.get_mut(0).rotation = Quat::IDENTITY;
        b.get_mut(0).rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);

        let mid = crossfade(&a, &b, 0.5);
        let expected = Quat::from_rotation_z(std::f32::consts::FRAC_PI_4);
        assert!((mid.get(0).rotation.dot(expected) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn slerp_rotation_helper_agrees_with_quat_slerp() {
        let q1 = Quat::from_rotation_x(0.3);
        let q2 = Quat::from_rotation_y(-0.7);
        assert_eq!(slerp_rotation(q1, q2, 0.4), q1.slerp(q2, 0.4));
    }
}
