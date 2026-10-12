//! Heightfields and the heightfield-to-mesh conversion.
//!
//! A [`HeightField`] is a sampled grid of heights over a rectangular
//! world region with cell-size metadata. The [`HeightField::to_mesh`]
//! conversion produces positions, central-difference normals and
//! triangle indices as plain data, plus an FNV-1a content hash over the
//! canonical byte encoding — the ledger discipline the save format and
//! the wire envelopes use, applied to generated terrain.

use thiserror::Error;

/// Errors from heightfield construction.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum HeightFieldError {
    /// Dimensions must be at least 2x2 (a mesh needs triangles).
    #[error("heightfield needs at least 2x2 samples, got {width}x{depth}")]
    TooSmall {
        /// The offending width.
        width: u32,
        /// The offending depth.
        depth: u32,
    },
    /// Cell size must be finite and > 0.
    #[error("cell size must be finite and > 0, got {cell_size}")]
    InvalidCellSize {
        /// The offending cell size.
        cell_size: f32,
    },
    /// A sampled height was non-finite.
    #[error("sampled height at index {index} is non-finite")]
    NonFiniteHeight {
        /// Index into the height buffer.
        index: usize,
    },
}

/// A sampled height grid over a rectangular region.
///
/// `heights` is row-major: index `z * width + x`. World position of
/// sample `(x, z)` is `(x * cell_size, height, z * cell_size)` — the
/// field's origin corner sits at the world origin.
#[derive(Debug, Clone, PartialEq)]
pub struct HeightField {
    width: u32,
    depth: u32,
    cell_size: f32,
    heights: Vec<f32>,
}

impl HeightField {
    /// Builds a field by sampling `height_at` over a `width x depth`
    /// grid.
    ///
    /// # Errors
    /// [`HeightFieldError::TooSmall`], [`HeightFieldError::InvalidCellSize`],
    /// or [`HeightFieldError::NonFiniteHeight`] when the sampler produces
    /// a non-finite height (a NaN in terrain would poison every mesh
    /// hash downstream).
    pub fn sample(
        width: u32,
        depth: u32,
        cell_size: f32,
        mut height_at: impl FnMut(f32, f32) -> f32,
    ) -> Result<HeightField, HeightFieldError> {
        if width < 2 || depth < 2 {
            return Err(HeightFieldError::TooSmall { width, depth });
        }
        if !cell_size.is_finite() || cell_size <= 0.0 {
            return Err(HeightFieldError::InvalidCellSize { cell_size });
        }
        let mut heights = Vec::with_capacity((width * depth) as usize);
        for z in 0..depth {
            for x in 0..width {
                let h = height_at(x as f32 * cell_size, z as f32 * cell_size);
                if !h.is_finite() {
                    return Err(HeightFieldError::NonFiniteHeight {
                        index: heights.len(),
                    });
                }
                heights.push(h);
            }
        }
        Ok(HeightField {
            width,
            depth,
            cell_size,
            heights,
        })
    }

    /// Grid width (x samples).
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Grid depth (z samples).
    #[must_use]
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// World distance between adjacent samples.
    #[must_use]
    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    /// The height buffer, row-major.
    #[must_use]
    pub fn heights(&self) -> &[f32] {
        &self.heights
    }

    /// Converts the field to a [`TerrainMesh`] with central-difference
    /// normals (one-sided at the borders) and a content hash.
    #[must_use]
    pub fn to_mesh(&self) -> TerrainMesh {
        let w = self.width as usize;
        let d = self.depth as usize;
        let c = self.cell_size;
        let mut positions = Vec::with_capacity(w * d);
        let mut normals = Vec::with_capacity(w * d);
        for z in 0..d {
            for x in 0..w {
                let h = self.heights[z * w + x];
                positions.push([x as f32 * c, h, z as f32 * c]);
                // Central difference with clamped neighbors: deterministic
                // edge handling, no special cases beyond the clamp.
                let hx_l = self.heights[z * w + x.saturating_sub(1)];
                let hx_r = self.heights[z * w + (x + 1).min(w - 1)];
                let hz_l = self.heights[(z.saturating_sub(1)) * w + x];
                let hz_r = self.heights[((z + 1).min(d - 1)) * w + x];
                // Normal = normalize((-dh/dx, 2c, -dh/dz)) — the classic
                // heightfield normal; 2c is the central-difference span.
                // sqrt is allowed by ADR-0031: IEEE-754 requires sqrt to
                // be correctly rounded, so it is bit-identical across
                // platforms (the ban covers sin/cos/pow/exp, whose
                // implementations are NOT correctly rounded in general).
                let nx = (hx_l - hx_r) / (2.0 * c);
                let nz = (hz_l - hz_r) / (2.0 * c);
                let len = (nx * nx + 1.0 + nz * nz).sqrt();
                normals.push([nx / len, 1.0 / len, nz / len]);
            }
        }
        let mut indices = Vec::with_capacity((w - 1) * (d - 1) * 6);
        for z in 0..d - 1 {
            for x in 0..w - 1 {
                let tl = (z * w + x) as u32;
                let tr = tl + 1;
                let bl = tl + w as u32;
                let br = bl + 1;
                // Two CCW triangles per quad (viewed from +Y).
                indices.extend_from_slice(&[tl, bl, tr, tr, bl, br]);
            }
        }
        let hash = terrain_hash(&positions, &normals, &indices);
        TerrainMesh {
            positions,
            normals,
            indices,
            hash,
        }
    }
}

/// Generated terrain as plain mesh data plus a content hash.
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainMesh {
    /// Vertex positions `[x, y, z]`.
    pub positions: Vec<[f32; 3]>,
    /// Vertex normals (unit length).
    pub normals: Vec<[f32; 3]>,
    /// Triangle indices (CCW from +Y).
    pub indices: Vec<u32>,
    /// FNV-1a over the canonical little-endian byte encoding of all
    /// data — identical content, identical hash, any platform.
    pub hash: u64,
}

/// FNV-1a 64-bit over bytes (the same algorithm the save format and
/// wire envelopes use; local copy keeps this crate dependency-free).
#[must_use]
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

fn terrain_hash(positions: &[[f32; 3]], normals: &[[f32; 3]], indices: &[u32]) -> u64 {
    let mut bytes = Vec::with_capacity((positions.len() + normals.len()) * 12 + indices.len() * 4);
    for v in positions.iter().chain(normals.iter()) {
        for c in v {
            bytes.extend_from_slice(&c.to_bits().to_le_bytes());
        }
    }
    for i in indices {
        bytes.extend_from_slice(&i.to_le_bytes());
    }
    fnv1a64(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::Noise2D;

    fn flat() -> HeightField {
        HeightField::sample(4, 4, 1.0, |_, _| 0.0).expect("valid field")
    }

    #[test]
    fn validates_dimensions_and_cell_size() {
        assert_eq!(
            HeightField::sample(1, 4, 1.0, |_, _| 0.0),
            Err(HeightFieldError::TooSmall { width: 1, depth: 4 })
        );
        assert_eq!(
            HeightField::sample(4, 4, 0.0, |_, _| 0.0),
            Err(HeightFieldError::InvalidCellSize { cell_size: 0.0 })
        );
    }

    #[test]
    fn rejects_non_finite_sampler_output() {
        let result = HeightField::sample(4, 4, 1.0, |x, _| if x > 2.0 { f32::NAN } else { 0.0 });
        assert!(matches!(
            result,
            Err(HeightFieldError::NonFiniteHeight { index: 3 })
        ));
    }

    #[test]
    fn row_major_layout_and_world_positions() {
        let f = HeightField::sample(3, 2, 2.0, |x, z| x + z).expect("ok");
        assert_eq!(f.width(), 3);
        assert_eq!(f.depth(), 2);
        assert_eq!(f.cell_size(), 2.0);
        // heights[z * width + x]
        assert_eq!(f.heights()[0], 0.0); // (x=0,z=0)
        assert_eq!(f.heights()[1], 2.0); // (x=1,z=0)
        assert_eq!(f.heights()[3], 2.0); // (x=0,z=1)
        assert_eq!(f.heights()[4], 4.0); // (x=1,z=1)
    }

    #[test]
    fn flat_mesh_has_up_normals_and_valid_indexing() {
        let mesh = flat().to_mesh();
        assert_eq!(mesh.positions.len(), 16);
        assert_eq!(mesh.indices.len(), (4 - 1) * (4 - 1) * 6);
        for n in &mesh.normals {
            assert!((*n)[1] > 0.999, "flat terrain faces up, got {n:?}");
        }
        for i in &mesh.indices {
            assert!((*i as usize) < mesh.positions.len());
        }
    }

    #[test]
    fn mesh_hash_is_content_addressed() {
        let noise_a = Noise2D::new(17);
        let mesh_a1 = HeightField::sample(8, 8, 1.0, |x, z| {
            noise_a.fbm(x * 0.1, z * 0.1, 4, 2.0, 0.5).expect("ok")
        })
        .expect("ok")
        .to_mesh();
        // Identical generation -> identical hash.
        let mesh_a2 = HeightField::sample(8, 8, 1.0, |x, z| {
            noise_a.fbm(x * 0.1, z * 0.1, 4, 2.0, 0.5).expect("ok")
        })
        .expect("ok")
        .to_mesh();
        assert_eq!(mesh_a1, mesh_a2);
        assert_eq!(mesh_a1.hash, mesh_a2.hash);
        // Different seed -> different hash.
        let noise_b = Noise2D::new(18);
        let mesh_b = HeightField::sample(8, 8, 1.0, |x, z| {
            noise_b.fbm(x * 0.1, z * 0.1, 4, 2.0, 0.5).expect("ok")
        })
        .expect("ok")
        .to_mesh();
        assert_ne!(mesh_a1.hash, mesh_b.hash);
    }

    #[test]
    fn normals_are_unit_length_on_slopes() {
        let noise = Noise2D::new(23);
        let mesh = HeightField::sample(16, 16, 0.5, |x, z| {
            noise.fbm(x * 0.2, z * 0.2, 4, 2.0, 0.5).expect("ok") * 10.0
        })
        .expect("ok")
        .to_mesh();
        for n in &mesh.normals {
            let len2 = n[0] * n[0] + n[1] * n[1] + n[2] * n[2];
            assert!(
                (len2 - 1.0).abs() < 1e-5,
                "normal must be unit length, got {n:?} (len^2={len2})"
            );
        }
    }
}
