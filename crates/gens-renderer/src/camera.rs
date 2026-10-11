//! Cameras and the camera uniform layout (Phase 2).
//!
//! Gens uses a right-handed Y-up world with **CCW front faces** and looks
//! down **-Z** in view space (glam's `_rh` matrix family). Two constructors
//! cover the Phase 2 needs: a full look-at perspective camera for 3D scenes
//! and a centered orthographic 2D camera (origin in the middle, +Y up) for
//! sprite-style rendering.
//!
//! The WGSL side consumes exactly one uniform block (see [`uniform`]):
//!
//! ```wgsl
//! struct CameraUniform {
//!     view_proj: mat4x4<f32>,
//! };
//! ```
//!
//! which is 64 bytes with no padding, written from glam's column-major
//! storage in native byte order (all wgpu targets are little-endian).

use gens_math::{Mat4, Vec3};

/// A view/projection pair.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// World -> view matrix.
    pub view: Mat4,
    /// View -> clip projection matrix.
    pub proj: Mat4,
}

impl Camera {
    /// Perspective camera looking from `eye` at `target`.
    ///
    /// `fov_y_radians` is the vertical field of view; `aspect` is
    /// `width / height`; the frustum spans `[near, far]` (both positive).
    #[must_use]
    pub fn perspective(
        eye: Vec3,
        target: Vec3,
        up: Vec3,
        fov_y_radians: f32,
        aspect: f32,
        near: f32,
        far: f32,
    ) -> Camera {
        Camera {
            view: gens_math::look_at_rh(eye, target, up),
            proj: gens_math::perspective_rh(fov_y_radians, aspect, near, far),
        }
    }

    /// Centered 2D orthographic camera: origin in the middle of a
    /// `width` x `height` screen, +Y up, the z = 0 plane in focus and the
    /// visible z range `[-1, 1]`.
    #[must_use]
    pub fn orthographic_2d(width: f32, height: f32) -> Camera {
        Camera {
            view: Mat4::IDENTITY,
            proj: gens_math::orthographic_rh(
                -width * 0.5,
                width * 0.5,
                -height * 0.5,
                height * 0.5,
                -1.0,
                1.0,
            ),
        }
    }

    /// The combined matrix uploaded to the GPU.
    #[must_use]
    pub fn view_proj(&self) -> Mat4 {
        self.proj * self.view
    }
}

/// The camera uniform block consumed by the WGSL shaders.
pub mod uniform {
    use gens_math::Mat4;

    /// Size of `struct CameraUniform { view_proj: mat4x4<f32> }` in bytes.
    pub const SIZE: u64 = 64;

    /// Serializes `view_proj` for `queue.write_buffer`: glam's column-major
    /// `mat4x4<f32>` matches WGSL's uniform layout exactly (64 bytes, no
    /// padding), so this is a straight float-wise encode in native order.
    #[must_use]
    pub fn bytes(view_proj: Mat4) -> [u8; 64] {
        let mut out = [0u8; 64];
        for (i, f) in view_proj.to_cols_array().iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&f.to_ne_bytes());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_math::{Vec4, vec3};

    fn ndc(m: Mat4, p: Vec3) -> (f32, f32, f32) {
        let c = m * Vec4::new(p.x, p.y, p.z, 1.0);
        (c.x / c.w, c.y / c.w, c.z / c.w)
    }

    #[test]
    fn perspective_view_places_eye_at_origin_and_target_ahead() {
        let cam = Camera::perspective(
            vec3(0.0, 0.0, 5.0),
            Vec3::ZERO,
            Vec3::Y,
            std::f32::consts::FRAC_PI_2,
            1.0,
            0.1,
            100.0,
        );
        let eye_in_view = cam.view * Vec4::new(0.0, 0.0, 5.0, 1.0);
        assert!(eye_in_view.abs_diff_eq(Vec4::new(0.0, 0.0, 0.0, 1.0), 1e-5));
        // The target sits 5 units in front (view space -Z).
        let target_in_view = cam.view * Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!(target_in_view.abs_diff_eq(Vec4::new(0.0, 0.0, -5.0, 1.0), 1e-5));
    }

    #[test]
    fn perspective_frustum_edges_map_to_ndc_unit_square() {
        let fov = std::f32::consts::FRAC_PI_2;
        let cam = Camera::perspective(
            Vec3::ZERO,
            vec3(0.0, 0.0, -1.0),
            Vec3::Y,
            fov,
            1.0,
            0.1,
            100.0,
        );
        let vp = cam.view_proj();
        // Top edge of the frustum at unit distance: y = tan(fov/2) -> NDC y = 1.
        let (nx, ny, _) = ndc(vp, vec3(0.0, (fov * 0.5).tan(), -1.0));
        assert!(
            (nx - 0.0).abs() < 1e-5 && (ny - 1.0).abs() < 1e-5,
            "top edge -> ({nx},{ny})"
        );
        // Right edge: x = tan(fov/2) at z = -1 -> NDC x = 1.
        let (nx, ny, _) = ndc(vp, vec3((fov * 0.5).tan(), 0.0, -1.0));
        assert!(
            (nx - 1.0).abs() < 1e-5 && (ny - 0.0).abs() < 1e-5,
            "right edge -> ({nx},{ny})"
        );
        // Center maps to NDC origin.
        let (nx, ny, _) = ndc(vp, vec3(0.0, 0.0, -1.0));
        assert!((nx - 0.0).abs() < 1e-5 && (ny - 0.0).abs() < 1e-5);
    }

    #[test]
    fn orthographic_2d_maps_screen_corners_to_ndc() {
        let cam = Camera::orthographic_2d(800.0, 600.0);
        let vp = cam.view_proj();
        let (nx, ny, _) = ndc(vp, vec3(-400.0, -300.0, 0.0));
        assert!((nx + 1.0).abs() < 1e-6 && (ny + 1.0).abs() < 1e-6);
        let (nx, ny, _) = ndc(vp, vec3(400.0, 300.0, 0.0));
        assert!((nx - 1.0).abs() < 1e-6 && (ny - 1.0).abs() < 1e-6);
        // Origin stays at the NDC origin, +Y is up.
        let (nx, ny, _) = ndc(vp, vec3(0.0, 100.0, 0.0));
        assert!((nx - 0.0).abs() < 1e-6 && ny > 0.0 && (ny - (200.0 / 600.0)).abs() < 1e-6);
    }

    #[test]
    fn camera_uniform_is_64_bytes_matching_wgsl_layout() {
        let m = Mat4::from_translation(vec3(1.0, 2.0, 3.0));
        let b = uniform::bytes(m);
        assert_eq!(uniform::SIZE, 64);
        assert_eq!(b.len(), 64);
        // Column-major: the first four floats are column 0 of the matrix.
        let cols = m.to_cols_array();
        for (i, f) in cols.iter().enumerate() {
            assert_eq!(&b[i * 4..i * 4 + 4], &f.to_ne_bytes(), "float {i}");
        }
        // Uniform blocks are consumed as little-endian on every wgpu target;
        // native order must therefore be little-endian in practice.
        assert_eq!(f32::to_ne_bytes(1.0), 1.0f32.to_le_bytes());
    }

    #[test]
    fn view_proj_composes_projection_over_view() {
        let cam = Camera::perspective(
            vec3(0.0, 2.0, 5.0),
            Vec3::ZERO,
            Vec3::Y,
            std::f32::consts::FRAC_PI_2,
            16.0 / 9.0,
            0.1,
            100.0,
        );
        let vp = cam.view_proj();
        let manual = cam.proj * cam.view;
        assert_eq!(vp, manual);
        // The look target must project in front of the camera (w > 0).
        let c = vp * Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!(c.w > 0.0);
    }
}
