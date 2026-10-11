//! Conversions between `gens-math` (glam 0.34) and rapier's math types
//! (glam 0.33 via parry's `glamx`).
//!
//! The two glam majors produce distinct types, so every value crossing the
//! physics boundary passes through one of these explicit, zero-allocation
//! helpers. Keeping them in one module makes the boundary auditable and
//! unit-testable.

use gens_math::{Quat, Vec3};

use crate::backend::math as rm;

/// Converts a gens vector into rapier's `Vector` (glam 0.33 `Vec3`).
#[inline]
#[must_use]
pub fn to_rapier_vec(v: Vec3) -> rm::Vector {
    rm::Vector::new(v.x, v.y, v.z)
}

/// Converts rapier's `Vector` into a gens vector.
#[inline]
#[must_use]
pub fn to_glam_vec(v: rm::Vector) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

/// Converts a gens quaternion into rapier's `Rotation` (`Rot3`, glam 0.33).
#[inline]
#[must_use]
pub fn to_rapier_quat(q: Quat) -> rm::Rotation {
    rm::Rotation::from_xyzw(q.x, q.y, q.z, q.w)
}

/// Converts rapier's `Rotation` into a gens quaternion.
#[inline]
#[must_use]
pub fn to_glam_quat(q: rm::Rotation) -> Quat {
    Quat::from_xyzw(q.x, q.y, q.z, q.w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_math::eq_eps;

    #[test]
    fn vectors_round_trip_component_wise() {
        let v = Vec3::new(1.5, -2.25, 3.125);
        let back = to_glam_vec(to_rapier_vec(v));
        assert_eq!(v, back);
    }

    #[test]
    fn quaternions_round_trip_component_wise() {
        let q = Quat::from_rotation_y(0.75);
        let back = to_glam_quat(to_rapier_quat(q));
        for (a, b) in q.to_array().iter().zip(back.to_array().iter()) {
            assert!(eq_eps(*a, *b));
        }
    }

    #[test]
    fn rapier_vector_is_three_component_f32() {
        // Guards against rapier silently switching to f64 vectors (Real = f64)
        // without the facade noticing: exact f32 bits must survive the hop.
        let v = to_rapier_vec(Vec3::new(0.1, 0.2, 0.3));
        assert_eq!(v.x, 0.1_f32);
        assert_eq!(v.y, 0.2_f32);
        assert_eq!(v.z, 0.3_f32);
    }
}
