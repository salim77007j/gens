//! Math facade over glam (Vec/Mat/Quat) with engine-specific utilities.
//!
//! Gens standardizes on [`glam`] for SIMD-friendly linear algebra. This crate
//! re-exports the core types so the rest of the engine depends on `gens-math`
//! (not glam directly — see ADR-0001/0002), and adds the small utility layer
//! engines always end up needing: angle conversions, frame-rate-independent
//! damping, remapping, epsilon comparison, axis-aligned bounding boxes, and a
//! translate-rotate-scale matrix composer used by the ECS transform system.

use glam::{Mat3, Mat4, Quat, Vec3};

pub use glam::{
    IVec2, Mat3 as Mat3x, UVec2, Vec2, Vec3A, Vec4, ivec2, quat, uvec2, vec2, vec3, vec4,
};

/// Default tolerance for floating point comparisons across the engine.
pub const EPSILON: f32 = 1e-5;

/// Converts degrees to radians.
#[inline]
pub fn deg_to_rad(deg: f32) -> f32 {
    deg.to_radians()
}

/// Converts radians to degrees.
#[inline]
pub fn rad_to_deg(rad: f32) -> f32 {
    rad.to_degrees()
}

/// Linear interpolation between `a` and `b` by `t` (unclamped).
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Inverse linear interpolation: where does `v` sit between `a` and `b`?
/// Returns 0.0 when the range is degenerate to avoid division by zero.
#[inline]
pub fn inverse_lerp(a: f32, b: f32, v: f32) -> f32 {
    if (b - a).abs() < EPSILON {
        0.0
    } else {
        (v - a) / (b - a)
    }
}

/// Remaps `v` from the range [`from_a`, `from_b`] to [`to_a`, `to_b`],
/// clamping the interpolation factor so out-of-range inputs stay bounded.
#[inline]
pub fn remap(v: f32, from_a: f32, from_b: f32, to_a: f32, to_b: f32) -> f32 {
    lerp(to_a, to_b, inverse_lerp(from_a, from_b, v).clamp(0.0, 1.0))
}

/// Relative-epsilon equality: robust across magnitudes (1e-8 vs 1e8).
#[inline]
pub fn eq_eps(a: f32, b: f32) -> bool {
    (a - b).abs() <= EPSILON * a.abs().max(b.abs()).max(1.0)
}

/// Frame-rate independent exponential approach from `current` towards
/// `target`. `rate` is the speed (1/s); higher values converge faster.
/// A non-positive `dt` returns `current` unchanged (no time passed).
#[inline]
pub fn damp(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        return current;
    }
    let factor = 1.0 - (-rate.max(0.0) * dt).exp();
    current + (target - current) * factor
}

/// Axis-aligned bounding box in 3D, inclusive of its `min`/`max` corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    /// Lower corner.
    pub min: Vec3,
    /// Upper corner.
    pub max: Vec3,
}

impl Aabb {
    /// A zero-volume box at the origin.
    pub const ZERO: Aabb = Aabb {
        min: Vec3::ZERO,
        max: Vec3::ZERO,
    };

    /// Builds a box from explicit corners; components are min/max-corrected
    /// so callers never need to pre-normalize.
    pub fn new(a: Vec3, b: Vec3) -> Aabb {
        Aabb {
            min: a.min(b),
            max: a.max(b),
        }
    }

    /// Builds a box from a center point and full extents per axis.
    pub fn from_center_extents(center: Vec3, extents: Vec3) -> Aabb {
        let half = extents.abs() * 0.5;
        Aabb {
            min: center - half,
            max: center + half,
        }
    }

    /// Geometric center of the box.
    #[inline]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Full size per axis (always non-negative).
    #[inline]
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    /// True when the point lies inside the box (inclusive).
    #[inline]
    pub fn contains(&self, p: Vec3) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }

    /// True when the two boxes overlap (inclusive on touching faces).
    #[inline]
    pub fn intersects(&self, other: &Aabb) -> bool {
        self.min.cmple(other.max).all() && self.max.cmpge(other.min).all()
    }

    /// Smallest box containing both boxes.
    #[inline]
    pub fn union(&self, other: &Aabb) -> Aabb {
        Aabb {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Grows the box to include a point.
    #[inline]
    pub fn expand_to_include(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }
}

/// Composes a translation * rotation * scale matrix, the exact convention
/// used by the ECS `Transform` propagation system (local-to-world).
#[inline]
pub fn compose_trs(translation: Vec3, rotation: Quat, scale: Vec3) -> Mat4 {
    Mat4::from_scale_rotation_translation(scale, rotation, translation)
}

/// Extracts the rotation part of a model matrix as a 3x3 basis.
#[inline]
pub fn rotation_submatrix(m: &Mat4) -> Mat3 {
    Mat3::from_cols(
        m.x_axis.truncate(),
        m.y_axis.truncate(),
        m.z_axis.truncate(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::vec3;

    #[test]
    fn angle_conversions_round_trip() {
        for deg in [0.0, 45.0, 90.0, 180.0, 359.9] {
            assert!(eq_eps(deg_to_rad(deg).to_degrees(), deg));
        }
        assert!(eq_eps(deg_to_rad(180.0), std::f32::consts::PI));
    }

    #[test]
    fn lerp_hits_endpoints_and_midpoint() {
        assert!(eq_eps(lerp(0.0, 10.0, 0.0), 0.0));
        assert!(eq_eps(lerp(0.0, 10.0, 1.0), 10.0));
        assert!(eq_eps(lerp(0.0, 10.0, 0.5), 5.0));
        // Extrapolation is allowed (t unclamped).
        assert!(eq_eps(lerp(0.0, 10.0, 2.0), 20.0));
    }

    #[test]
    fn inverse_lerp_and_remap() {
        assert!(eq_eps(inverse_lerp(0.0, 10.0, 5.0), 0.5));
        assert!(eq_eps(inverse_lerp(5.0, 5.0, 5.0), 0.0)); // degenerate range
        assert!(eq_eps(remap(5.0, 0.0, 10.0, 100.0, 200.0), 150.0));
        // Out-of-range input is clamped.
        assert!(eq_eps(remap(20.0, 0.0, 10.0, 100.0, 200.0), 200.0));
    }

    #[test]
    fn epsilon_comparison_across_magnitudes() {
        assert!(eq_eps(1.0, 1.0 + 1e-7));
        assert!(!eq_eps(1.0, 1.1));
        assert!(eq_eps(1e8, 1e8 + 1.0)); // relative tolerance
        assert!(!eq_eps(1e-8, 1e-4));
    }

    #[test]
    fn damp_is_framerate_independent() {
        // One big step and many small steps over the same time converge
        // to (nearly) the same value.
        let one_shot = damp(0.0, 100.0, 4.0, 0.5);
        let mut stepped = 0.0;
        for _ in 0..50 {
            stepped = damp(stepped, 100.0, 4.0, 0.01);
        }
        assert!(eq_eps(one_shot, stepped) || (one_shot - stepped).abs() < 0.5);
    }

    #[test]
    fn damp_guards() {
        assert!(eq_eps(damp(3.0, 100.0, 4.0, 0.0), 3.0)); // dt = 0
        assert!(eq_eps(damp(3.0, 100.0, 4.0, -1.0), 3.0)); // negative dt
        assert!(eq_eps(damp(3.0, 100.0, -4.0, 1.0), 3.0)); // negative rate: no move
    }

    #[test]
    fn aabb_new_normalizes_corners() {
        let box_ = Aabb::new(vec3(2.0, 2.0, 2.0), vec3(-1.0, -1.0, -1.0));
        assert_eq!(box_.min, vec3(-1.0, -1.0, -1.0));
        assert_eq!(box_.max, vec3(2.0, 2.0, 2.0));
    }

    #[test]
    fn aabb_contains_and_intersects() {
        let a = Aabb::from_center_extents(vec3(0.0, 0.0, 0.0), vec3(2.0, 2.0, 2.0));
        assert!(a.contains(vec3(0.5, 0.5, 0.5)));
        assert!(!a.contains(vec3(2.0, 0.0, 0.0)));
        let b = Aabb::from_center_extents(vec3(1.5, 0.0, 0.0), vec3(2.0, 2.0, 2.0));
        assert!(a.intersects(&b));
        let c = Aabb::from_center_extents(vec3(10.0, 0.0, 0.0), vec3(2.0, 2.0, 2.0));
        assert!(!a.intersects(&c));
    }

    #[test]
    fn aabb_union_and_expand() {
        let a = Aabb::new(vec3(0.0, 0.0, 0.0), vec3(1.0, 1.0, 1.0));
        let b = Aabb::new(vec3(5.0, 5.0, 5.0), vec3(6.0, 6.0, 6.0));
        let u = a.union(&b);
        assert_eq!(u.min, vec3(0.0, 0.0, 0.0));
        assert_eq!(u.max, vec3(6.0, 6.0, 6.0));
        assert_eq!(u.center(), vec3(3.0, 3.0, 3.0));
        assert_eq!(u.size(), vec3(6.0, 6.0, 6.0));
    }

    #[test]
    fn compose_trs_matches_manual_composition() {
        let t = vec3(1.0, 2.0, 3.0);
        let r = Quat::from_rotation_y(deg_to_rad(90.0));
        let s = vec3(2.0, 2.0, 2.0);
        let manual = Mat4::from_translation(t) * Mat4::from_quat(r) * Mat4::from_scale(s);
        let composed = compose_trs(t, r, s);
        for i in 0..4 {
            for j in 0..4 {
                assert!(eq_eps(
                    composed.to_cols_array()[i * 4 + j],
                    manual.to_cols_array()[i * 4 + j]
                ));
            }
        }
        // Rotation by 90 deg around Y maps local +Z to world +X, scaled by 2
        // and translated by (1,2,3): (0,0,1) -> (0,0,2) -> (2,0,0) -> (3,2,3).
        let point = composed.transform_point3(vec3(0.0, 0.0, 1.0));
        assert!(eq_eps(point.x, 3.0) && eq_eps(point.y, 2.0) && eq_eps(point.z, 3.0));
    }

    #[test]
    fn rotation_submatrix_drops_translation() {
        let m = compose_trs(vec3(10.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ONE);
        let r = rotation_submatrix(&m);
        assert!(eq_eps(r.x_axis.x, 1.0));
        assert!(eq_eps(r.to_cols_array()[3], 0.0)); // no translation leaked
    }
}
