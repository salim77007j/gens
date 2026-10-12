//! Imported mesh data: engine-owned geometry ready for the renderer.
//!
//! [`MeshData`] is the neutral geometry container glTF import produces
//! and the renderer can consume: f32 positions/normals/uvs plus a
//! u32 index buffer (triangle list). It carries no GPU state — the
//! renderer uploads it on demand (the Phase 2 offscreen-first
//! architecture keeps asset data and GPU objects separate).
//!
//! Skin weights come later with the skinned render path (ADR-0016
//! reserved that for Phase 5/8; the asset side lands first — the
//! `SKINS_n`/`JOINTS_n` attributes are parsed and stored here, so the
//! GPU path can adopt them without re-importing).

use gens_math::Vec3;

/// Engine-owned triangle-list mesh.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MeshData {
    /// Vertex positions (length = vertex_count * 3).
    pub positions: Vec<f32>,
    /// Vertex normals, same indexing as `positions` (may be empty).
    pub normals: Vec<f32>,
    /// Vertex UVs, u,v pairs, same indexing as `positions` (may be empty).
    pub uvs: Vec<f32>,
    /// Per-vertex joint indices (4 per vertex; empty when unskinned).
    pub joints: Vec<u16>,
    /// Per-vertex joint weights (4 per vertex; empty when unskinned).
    pub weights: Vec<f32>,
    /// Triangle-list indices (length = 3 * triangle_count).
    pub indices: Vec<u32>,
}

impl MeshData {
    /// Number of vertices.
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// True when the mesh carries skin weights.
    pub fn is_skinned(&self) -> bool {
        !self.joints.is_empty()
    }

    /// Position of vertex `i`.
    pub fn position(&self, i: usize) -> Vec3 {
        Vec3::new(
            self.positions[i * 3],
            self.positions[i * 3 + 1],
            self.positions[i * 3 + 2],
        )
    }

    /// Axis-aligned bounding box of the positions (min, max).
    ///
    /// Deterministic: a pure scan in vertex order; empty meshes return
    /// `(ZERO, ZERO)`.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let mut min = Vec3::ZERO;
        let mut max = Vec3::ZERO;
        let mut first = true;
        for v in 0..self.vertex_count() {
            let p = self.position(v);
            if first {
                min = p;
                max = p;
                first = false;
            } else {
                min = min.min(p);
                max = max.max(p);
            }
        }
        (min, max)
    }

    /// Validates index-buffer invariants (in range, multiple of 3).
    pub fn validate(&self) -> Result<(), MeshError> {
        if !self.indices.len().is_multiple_of(3) {
            return Err(MeshError::IndexCount(self.indices.len()));
        }
        let verts = self.vertex_count();
        if !self.positions.is_empty() && !self.positions.len().is_multiple_of(3) {
            return Err(MeshError::PositionCount(self.positions.len()));
        }
        if !self.normals.is_empty() && self.normals.len() != self.positions.len() {
            return Err(MeshError::LayerMismatch("normals"));
        }
        if !self.uvs.is_empty() && self.uvs.len() != verts * 2 {
            return Err(MeshError::LayerMismatch("uvs"));
        }
        if self.joints.len() != self.weights.len() {
            return Err(MeshError::LayerMismatch("joints/weights"));
        }
        if !self.joints.is_empty() && self.joints.len() != verts * 4 {
            return Err(MeshError::LayerMismatch("joints"));
        }
        for &i in &self.indices {
            if i as usize >= verts {
                return Err(MeshError::IndexOutOfRange(i, verts));
            }
        }
        Ok(())
    }
}

/// Mesh validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MeshError {
    /// Index buffer length is not a multiple of three.
    #[error("index buffer length {0} is not a multiple of 3")]
    IndexCount(usize),
    /// Position buffer length is not a multiple of three.
    #[error("position buffer length {0} is not a multiple of 3")]
    PositionCount(usize),
    /// A per-vertex layer does not match the vertex count.
    #[error("attribute layer {0:?} does not match vertex count")]
    LayerMismatch(&'static str),
    /// An index points past the last vertex.
    #[error("index {0} out of range ({1} vertices)")]
    IndexOutOfRange(u32, usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> MeshData {
        MeshData {
            positions: vec![
                0.0, 0.0, 0.0, // v0
                1.0, 0.0, 0.0, // v1
                1.0, 1.0, 0.0, // v2
                0.0, 1.0, 0.0, // v3
            ],
            normals: [[0.0, 0.0, 1.0f32]; 4].concat(),
            uvs: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
            joints: vec![],
            weights: vec![],
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    #[test]
    fn counts_and_bounds() {
        let m = quad();
        assert_eq!(m.vertex_count(), 4);
        assert_eq!(m.triangle_count(), 2);
        assert!(!m.is_skinned());
        let (min, max) = m.bounds();
        assert_eq!(min, Vec3::ZERO);
        assert_eq!(max, Vec3::new(1.0, 1.0, 0.0));
        m.validate().expect("valid");
    }

    #[test]
    fn out_of_range_index_caught() {
        let mut m = quad();
        m.indices[0] = 4;
        assert_eq!(m.validate(), Err(MeshError::IndexOutOfRange(4, 4)));
    }

    #[test]
    fn layer_mismatch_caught() {
        let mut m = quad();
        m.normals = [[0.0, 0.0, 1.0f32]; 3].concat();
        assert_eq!(m.validate(), Err(MeshError::LayerMismatch("normals")));
    }

    #[test]
    fn index_count_not_triangles_caught() {
        let mut m = quad();
        m.indices.pop();
        assert_eq!(m.validate(), Err(MeshError::IndexCount(5)));
    }

    #[test]
    fn skinned_mesh_validates() {
        let mut m = quad();
        m.joints = vec![0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1];
        m.weights = [[1.0, 0.0, 0.0, 0.0f32]; 4].concat();
        assert!(m.is_skinned());
        m.validate().expect("valid skinned");
    }
}
