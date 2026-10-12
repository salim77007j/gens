//! Deterministic prop scatter over a region.
//!
//! Placement is **order-independent by construction** (ADR-0031): every
//! candidate point is derived from `(seed, lattice cell)` alone — a
//! jittered lattice where the jitter, the density test and the height
//! all come from pure functions of the cell's world coordinates. Two
//! runs that scatter the same region in any order, or in chunks, get
//! the same points.
//!
//! The pipeline per lattice cell: hash the cell for a uniform `(jx, jz)`
//! jitter inside the cell; evaluate the density noise at the jittered
//! world position; keep the point when density clears the threshold;
//! lift it to the caller's `height_at` closure.

use thiserror::Error;

use crate::noise::Noise2D;

/// Errors from scatter construction.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum ScatterError {
    /// Region extents must be finite and positive.
    #[error("region extents must be finite and > 0, got {extent}x{extent_z}")]
    InvalidRegion {
        /// The offending x extent.
        extent: f32,
        /// The offending z extent.
        extent_z: f32,
    },
    /// Lattice cell size must be finite and > 0.
    #[error("lattice cell size must be finite and > 0, got {cell_size}")]
    InvalidCellSize {
        /// The offending cell size.
        cell_size: f32,
    },
    /// Density threshold must be in `[0, 1]`.
    #[error("density threshold must be in [0, 1], got {threshold}")]
    InvalidThreshold {
        /// The offending threshold.
        threshold: f32,
    },
    /// The region/cell-size ratio would exceed the lattice domain.
    #[error("region needs {cells} lattice cells per axis; the limit is {limit}")]
    RegionTooLarge {
        /// Cells needed per axis.
        cells: i64,
        /// The per-axis limit.
        limit: i64,
    },
}

/// One scattered prop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScatterPoint {
    /// World position `[x, y, z]` (y from the caller's height closure).
    pub position: [f32; 3],
    /// The lattice cell this point came from (diagnostics + tests).
    pub cell: (i32, i32),
}

/// The result of scattering over a region.
#[derive(Debug, Clone, PartialEq)]
pub struct ScatterPlan {
    /// Kept points, in row-major lattice order (deterministic).
    pub points: Vec<ScatterPoint>,
    /// How many candidate cells the region contained.
    pub candidate_cells: u64,
}

/// Scatters props over the region `[0, extent) x [0, extent_z)` on a
/// jittered `cell_size` lattice.
///
/// The `density` noise and the `height` closure must be pure (the crate
/// cannot enforce that; ADR-0031 states the contract the caller signs).
///
/// # Errors
/// [`ScatterError`] on invalid extents, cell size or threshold.
pub fn scatter(
    jitter_seed: u64,
    density: &Noise2D,
    threshold: f32,
    extent: f32,
    extent_z: f32,
    cell_size: f32,
    mut height: impl FnMut(f32, f32) -> f32,
) -> Result<ScatterPlan, ScatterError> {
    if !extent.is_finite() || extent <= 0.0 || !extent_z.is_finite() || extent_z <= 0.0 {
        return Err(ScatterError::InvalidRegion { extent, extent_z });
    }
    if !cell_size.is_finite() || cell_size <= 0.0 {
        return Err(ScatterError::InvalidCellSize { cell_size });
    }
    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        return Err(ScatterError::InvalidThreshold { threshold });
    }
    let cells_x = (extent / cell_size).ceil() as i64;
    let cells_z = (extent_z / cell_size).ceil() as i64;
    // Lattice addressing is i32 (hash_cell); a region exceeding it is a
    // misconfigured generator — loud error, never a silent wrap.
    const CELL_LIMIT: i64 = i32::MAX as i64;
    if cells_x > CELL_LIMIT || cells_z > CELL_LIMIT {
        return Err(ScatterError::RegionTooLarge {
            cells: cells_x.max(cells_z),
            limit: CELL_LIMIT,
        });
    }
    // Two independent jitter channels: separate noise sources instead of
    // coordinate-offset arithmetic (no overflow paths at all).
    let jitter_x = Noise2D::new(jitter_seed);
    let jitter_z = Noise2D::new(jitter_seed ^ 0xABCD_1234_5678_90EF);
    let mut points = Vec::new();
    for cz in 0..cells_z {
        for cx in 0..cells_x {
            // The guard above proves the i64 -> i32 cast is lossless.
            let (cx, cz) = (cx as i32, cz as i32);
            // Jitter inside the cell from two independent channels.
            let jx = jitter_x.hash_cell(cx, cz);
            let jz = jitter_z.hash_cell(cx, cz);
            let wx = (cx as f32 + jx) * cell_size;
            let wz = (cz as f32 + jz) * cell_size;
            if wx >= extent || wz >= extent_z {
                continue; // ceil() over-allocation: outside the region
            }
            let d = density.value(wx, wz);
            if d >= threshold {
                let y = height(wx, wz);
                points.push(ScatterPoint {
                    position: [wx, y, wz],
                    cell: (cx, cz),
                });
            }
        }
    }
    Ok(ScatterPlan {
        candidate_cells: (cells_x * cells_z) as u64,
        points,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::Noise2D;

    fn height_flat(x: f32, _z: f32) -> f32 {
        x * 0.0 // flat ground at y=0, uses the signature for realism
    }

    #[test]
    fn validates_region_cell_and_threshold() {
        let density = Noise2D::new(1);
        assert_eq!(
            scatter(1, &density, 0.5, 0.0, 10.0, 1.0, height_flat),
            Err(ScatterError::InvalidRegion {
                extent: 0.0,
                extent_z: 10.0
            })
        );
        assert_eq!(
            scatter(1, &density, 0.5, 10.0, 10.0, -1.0, height_flat),
            Err(ScatterError::InvalidCellSize { cell_size: -1.0 })
        );
        assert_eq!(
            scatter(1, &density, 1.5, 10.0, 10.0, 1.0, height_flat),
            Err(ScatterError::InvalidThreshold { threshold: 1.5 })
        );
    }

    #[test]
    fn placement_is_order_independent() {
        let density = Noise2D::new(4);
        let a = scatter(9, &density, 0.3, 16.0, 16.0, 2.0, height_flat).expect("ok");
        let b = scatter(9, &density, 0.3, 16.0, 16.0, 2.0, height_flat).expect("ok");
        assert_eq!(a, b, "same inputs, any call order, same points");
    }

    #[test]
    fn threshold_is_monotone() {
        let density = Noise2D::new(4);
        let loose = scatter(9, &density, 0.2, 16.0, 16.0, 2.0, height_flat).expect("ok");
        let strict = scatter(9, &density, 0.8, 16.0, 16.0, 2.0, height_flat).expect("ok");
        assert!(
            loose.points.len() >= strict.points.len(),
            "a lower threshold keeps at least as many points ({} vs {})",
            loose.points.len(),
            strict.points.len()
        );
    }

    #[test]
    fn points_stay_inside_the_region() {
        let density = Noise2D::new(4);
        let plan = scatter(9, &density, 0.05, 16.0, 16.0, 2.0, height_flat).expect("ok");
        assert!(!plan.points.is_empty(), "low threshold keeps points");
        for p in &plan.points {
            assert!((0.0..16.0).contains(&p.position[0]), "x in region");
            assert!((0.0..16.0).contains(&p.position[2]), "z in region");
        }
    }

    #[test]
    fn cell_coordinates_are_unique_and_tracked() {
        let density = Noise2D::new(4);
        let plan = scatter(9, &density, 0.4, 16.0, 16.0, 2.0, height_flat).expect("ok");
        let mut cells: Vec<(i32, i32)> = plan.points.iter().map(|p| p.cell).collect();
        cells.sort_unstable();
        cells.dedup();
        assert_eq!(cells.len(), plan.points.len(), "one point per cell at most");
    }

    #[test]
    fn height_closure_lifts_points() {
        let density = Noise2D::new(4);
        let plan = scatter(9, &density, 0.05, 16.0, 16.0, 2.0, |x, z| x + z).expect("ok");
        for p in &plan.points {
            let expected = p.position[0] + p.position[2];
            assert_eq!(p.position[1], expected, "y comes from the closure");
        }
    }
}
