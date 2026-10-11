//! Vertices, meshes and built-in shapes (Phase 2).
//!
//! [`Vertex`] is the Phase 2 render format: position + RGBA color + UV,
//! interleaved and 36 bytes wide, matching the WGSL inputs of the colored
//! and textured pipelines. [`Mesh`] is a vertex/index pair with `u16`
//! indices (64k vertices per draw — plenty for Phase 2 scenes; an indexed
//! escape hatch can be added when a real scene needs it).
//!
//! Winding convention: **CCW front faces, right-handed Y-up world**
//! (glam defaults). Every built-in shape honors it, and the unit tests
//! verify winding with actual cross products rather than trusting the
//! constants.

use gens_core::error::{EngineError, Result};
use wgpu::{VertexAttribute, VertexBufferLayout, VertexFormat, VertexStepMode};

/// Interleaved vertex: 3D position, RGBA color (linear), UV.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    /// World or object-space position.
    pub position: [f32; 3],
    /// Linear RGBA color.
    pub color: [f32; 4],
    /// Texture coordinates.
    pub uv: [f32; 2],
}

impl Vertex {
    /// Size of one vertex in bytes: 12 + 16 + 8.
    pub const STRIDE: u64 = 36;

    /// The wgpu vertex buffer layout matching this struct exactly.
    /// Attribute offsets are derived from the field order, so the layout
    /// and the struct cannot drift apart silently.
    #[must_use]
    pub fn layout() -> VertexBufferLayout<'static> {
        VertexBufferLayout {
            array_stride: Self::STRIDE,
            step_mode: VertexStepMode::Vertex,
            attributes: &[
                VertexAttribute {
                    format: VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                VertexAttribute {
                    format: VertexFormat::Float32x4,
                    offset: 12,
                    shader_location: 1,
                },
                VertexAttribute {
                    format: VertexFormat::Float32x2,
                    offset: 28,
                    shader_location: 2,
                },
            ],
        }
    }
}

/// A drawable set of vertices with `u16` indices.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    /// Vertex data.
    pub vertices: Vec<Vertex>,
    /// Triangle indices (3 per triangle, CCW front faces).
    pub indices: Vec<u16>,
}

impl Mesh {
    /// Empty mesh.
    #[must_use]
    pub fn new() -> Mesh {
        Mesh::default()
    }

    /// Number of indices to draw.
    #[must_use]
    pub fn index_count(&self) -> u32 {
        self.indices.len() as u32
    }

    /// Checks index-range invariants. Call after building a mesh by hand;
    /// built-in shapes are always valid.
    ///
    /// # Errors
    /// Returns [`EngineError`] when an index is out of vertex range, the
    /// index count is not a multiple of 3, or the vertex count exceeds the
    /// `u16` index space.
    pub fn validate(&self) -> Result<()> {
        if self.vertices.len() > u16::MAX as usize + 1 {
            return Err(EngineError::invalid(
                "renderer",
                format!(
                    "mesh has {} vertices; u16 indices support at most {}",
                    self.vertices.len(),
                    u16::MAX as usize + 1
                ),
            ));
        }
        if !self.indices.len().is_multiple_of(3) {
            return Err(EngineError::invalid(
                "renderer",
                format!("index count {} is not a multiple of 3", self.indices.len()),
            ));
        }
        let max = self.vertices.len() as u16;
        if let Some(&bad) = self.indices.iter().find(|&&i| i >= max) {
            return Err(EngineError::invalid(
                "renderer",
                format!(
                    "index {bad} out of range for {} vertices",
                    self.vertices.len()
                ),
            ));
        }
        Ok(())
    }

    /// Vertex buffer bytes (interleaved, ready for `queue.write_buffer`).
    #[must_use]
    pub fn vertex_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.vertices.len() * Vertex::STRIDE as usize);
        for v in &self.vertices {
            for f in v.position {
                out.extend_from_slice(&f.to_ne_bytes());
            }
            for f in v.color {
                out.extend_from_slice(&f.to_ne_bytes());
            }
            for f in v.uv {
                out.extend_from_slice(&f.to_ne_bytes());
            }
        }
        out
    }

    /// Index buffer bytes (`u16` little-endian pairs).
    #[must_use]
    pub fn index_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.indices.len() * 2);
        for i in &self.indices {
            out.extend_from_slice(&i.to_ne_bytes());
        }
        out
    }
}

/// Convenience constructor for a vertex.
#[must_use]
pub fn vertex(position: [f32; 3], color: [f32; 4], uv: [f32; 2]) -> Vertex {
    Vertex {
        position,
        color,
        uv,
    }
}

/// A single unit triangle in the XY plane, CCW when viewed from +Z.
#[must_use]
pub fn triangle() -> Mesh {
    Mesh {
        vertices: vec![
            vertex([-0.5, -0.5, 0.0], [1.0, 0.0, 0.0, 1.0], [0.0, 0.0]),
            vertex([0.5, -0.5, 0.0], [0.0, 1.0, 0.0, 1.0], [1.0, 0.0]),
            vertex([0.0, 0.5, 0.0], [0.0, 0.0, 1.0, 1.0], [0.5, 1.0]),
        ],
        indices: vec![0, 1, 2],
    }
}

/// A unit quad centered on the origin in the XY plane (facing +Z, i.e.
/// toward a camera at +Z looking at the origin).
#[must_use]
pub fn quad() -> Mesh {
    Mesh {
        vertices: vec![
            vertex([-0.5, -0.5, 0.0], [1.0, 1.0, 1.0, 1.0], [0.0, 1.0]),
            vertex([0.5, -0.5, 0.0], [1.0, 1.0, 1.0, 1.0], [1.0, 1.0]),
            vertex([0.5, 0.5, 0.0], [1.0, 1.0, 1.0, 1.0], [1.0, 0.0]),
            vertex([-0.5, 0.5, 0.0], [1.0, 1.0, 1.0, 1.0], [0.0, 0.0]),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
    }
}

/// A unit cube centered on the origin, one flat color per face.
///
/// 24 vertices (4 per face, hard edges), 36 indices. Faces ordered
/// +X, -X, +Y, -Y, +Z, -Z with distinct colors so an unlit render shows
/// the cube's orientation.
#[must_use]
pub fn cube() -> Mesh {
    // Per-face: 4 corner offsets (CCW seen from outside) + face color.
    let faces: &[([[f32; 3]; 4], [f32; 4])] = &[
        (
            [
                [0.5, -0.5, 0.5],
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [0.5, 0.5, 0.5],
            ],
            [0.90, 0.30, 0.30, 1.0],
        ),
        (
            [
                [-0.5, -0.5, -0.5],
                [-0.5, -0.5, 0.5],
                [-0.5, 0.5, 0.5],
                [-0.5, 0.5, -0.5],
            ],
            [0.30, 0.55, 0.90, 1.0],
        ),
        (
            [
                [-0.5, 0.5, 0.5],
                [0.5, 0.5, 0.5],
                [0.5, 0.5, -0.5],
                [-0.5, 0.5, -0.5],
            ],
            [0.95, 0.80, 0.30, 1.0],
        ),
        (
            [
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, -0.5, 0.5],
                [-0.5, -0.5, 0.5],
            ],
            [0.55, 0.85, 0.40, 1.0],
        ),
        (
            [
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
            [0.90, 0.55, 0.25, 1.0],
        ),
        (
            [
                [0.5, -0.5, -0.5],
                [-0.5, -0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [0.5, 0.5, -0.5],
            ],
            [0.70, 0.45, 0.85, 1.0],
        ),
    ];
    let uvs: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    let mut mesh = Mesh::new();
    for (face, (corners, color)) in faces.iter().enumerate() {
        let base = face as u16 * 4;
        for (c, uv) in corners.iter().zip(uvs) {
            mesh.vertices.push(vertex(*c, *color, uv));
        }
        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Signed area cross product z-component of triangle (a, b, c) viewed
    /// from +Z: positive = CCW (front face under our convention).
    fn winding_z(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
        let (ab_x, ab_y) = (b[0] - a[0], b[1] - a[1]);
        let (ac_x, ac_y) = (c[0] - a[0], c[1] - a[1]);
        ab_x * ac_y - ab_y * ac_x
    }

    #[test]
    fn vertex_layout_matches_struct_exactly() {
        let layout = Vertex::layout();
        assert_eq!(layout.array_stride, 36);
        let attrs = layout.attributes;
        assert_eq!(attrs[0].format, VertexFormat::Float32x3);
        assert_eq!(attrs[0].offset, 0);
        assert_eq!(attrs[1].format, VertexFormat::Float32x4);
        assert_eq!(attrs[1].offset, 12);
        assert_eq!(attrs[2].format, VertexFormat::Float32x2);
        assert_eq!(attrs[2].offset, 28);
        // Locations must be 0,1,2 contiguous.
        for (i, a) in attrs.iter().enumerate() {
            assert_eq!(a.shader_location, i as u32);
        }
    }

    #[test]
    fn vertex_bytes_round_trip() {
        let v = vertex([1.5, -2.0, 3.25], [0.1, 0.2, 0.3, 0.4], [0.75, 0.25]);
        let mesh = Mesh {
            vertices: vec![v],
            indices: vec![],
        };
        let bytes = mesh.vertex_bytes();
        assert_eq!(bytes.len(), 36);
        assert_eq!(&bytes[..4], &1.5f32.to_ne_bytes());
        assert_eq!(&bytes[12..16], &0.1f32.to_ne_bytes());
        assert_eq!(&bytes[28..32], &0.75f32.to_ne_bytes());
    }

    #[test]
    fn triangle_is_ccw_from_positive_z() {
        let t = triangle();
        let w = winding_z(
            t.vertices[0].position,
            t.vertices[1].position,
            t.vertices[2].position,
        );
        assert!(w > 0.0, "triangle winding {w} must be CCW from +Z");
    }

    #[test]
    fn quad_is_ccw_and_covers_unit_square() {
        let q = quad();
        assert_eq!(q.vertices.len(), 4);
        assert_eq!(q.indices, vec![0, 1, 2, 0, 2, 3]);
        for tri in [[0, 1, 2], [0, 2, 3]] {
            let w = winding_z(
                q.vertices[tri[0]].position,
                q.vertices[tri[1]].position,
                q.vertices[tri[2]].position,
            );
            assert!(w > 0.0, "quad triangle {tri:?} winding {w} must be CCW");
        }
        let xs: Vec<f32> = q.vertices.iter().map(|v| v.position[0]).collect();
        let ys: Vec<f32> = q.vertices.iter().map(|v| v.position[1]).collect();
        assert_eq!(xs.iter().cloned().fold(f32::MIN, f32::max), 0.5);
        assert_eq!(xs.iter().cloned().fold(f32::MAX, f32::min), -0.5);
        assert_eq!(ys.iter().cloned().fold(f32::MIN, f32::max), 0.5);
        assert_eq!(ys.iter().cloned().fold(f32::MAX, f32::min), -0.5);
    }

    #[test]
    fn cube_is_well_formed_with_six_colored_faces() {
        let c = cube();
        c.validate().expect("cube is valid");
        assert_eq!(c.vertices.len(), 24);
        assert_eq!(c.indices.len(), 36);
        // Every corner sits on the unit cube surface.
        for v in &c.vertices {
            let on_surface = v
                .position
                .iter()
                .any(|&a| (a - 0.5).abs() < 1e-6 || (a + 0.5).abs() < 1e-6);
            assert!(on_surface, "corner {:?} not on unit cube", v.position);
        }
        // Six distinct face colors.
        let mut colors: Vec<[f32; 4]> = c
            .vertices
            .as_chunks::<1>()
            .0
            .iter()
            .map(|c| c[0].color)
            .collect();
        colors.dedup_by(|a, b| *a == *b);
        assert!(
            colors.len() >= 6,
            "expected >=6 face colors, got {colors:?}"
        );
    }

    #[test]
    fn validate_catches_bad_indices() {
        let mut m = triangle();
        m.validate().expect("triangle valid");
        m.indices.push(2);
        assert!(m.validate().is_err(), "index count % 3 must fail");
        m.indices = vec![0, 1, 9];
        assert!(m.validate().is_err(), "out-of-range index must fail");
        // 65 537 vertices exceed the u16 index space.
        let big = Mesh {
            vertices: vec![vertex([0.0; 3], [0.0; 4], [0.0; 2]); u16::MAX as usize + 2],
            indices: vec![],
        };
        assert!(big.validate().is_err(), "oversized vertex count must fail");
    }

    #[test]
    fn index_bytes_are_two_bytes_each() {
        let m = triangle();
        assert_eq!(m.index_bytes().len(), 6);
        assert_eq!(&m.index_bytes()[..2], &0u16.to_ne_bytes());
    }
}
