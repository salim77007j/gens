//! Deterministic chunk streaming: load/unload sets from a focus point.
//!
//! The streamer owns **decisions, not worlds**: `update` computes which
//! chunks should be loaded for a focus point and returns the load and
//! unload sets in sorted coordinate order (ADR-0031). The embedding
//! layer applies them — spawning and despawning is ECS work this pure
//! crate must not do. The desired set is a Chebyshev disc (square of
//! half-width `radius`) around the focus chunk; updates are
//! differential (only the delta is reported) and idempotent (the same
//! focus twice yields empty plans).

use std::collections::BTreeMap;

use crate::chunk::ChunkCoord;

/// The differential result of one streaming update.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StreamingPlan {
    /// Chunks to load, sorted (x, then z).
    pub load: Vec<ChunkCoord>,
    /// Chunks to unload, sorted (x, then z).
    pub unload: Vec<ChunkCoord>,
}

/// Tracks which chunks are loaded and computes the differential plan.
#[derive(Debug, Clone)]
pub struct ChunkStreamer {
    radius: i32,
    loaded: BTreeMap<ChunkCoord, ()>,
}

impl ChunkStreamer {
    /// Creates a streamer with a Chebyshev load radius (0 = only the
    /// focus chunk; nothing is loaded until the first `update`).
    #[must_use]
    pub fn new(radius: i32) -> ChunkStreamer {
        ChunkStreamer {
            radius: radius.max(0),
            loaded: BTreeMap::new(),
        }
    }

    /// The current load radius.
    #[must_use]
    pub fn radius(&self) -> i32 {
        self.radius
    }

    /// Currently loaded chunks, in sorted order.
    #[must_use]
    pub fn loaded(&self) -> Vec<ChunkCoord> {
        self.loaded.keys().copied().collect()
    }

    /// Computes the differential plan for a focus chunk and applies it
    /// to the internal loaded set.
    ///
    /// Deterministic: the desired set is derived from `(focus, radius)`
    /// alone, and both output lists are sorted coordinate-wise, so the
    /// same focus sequence produces the same plan sequence forever.
    pub fn update(&mut self, focus: ChunkCoord) -> StreamingPlan {
        let r = self.radius;
        let mut desired = BTreeMap::new();
        for dz in -r..=r {
            for dx in -r..=r {
                desired.insert(ChunkCoord::new(focus.x + dx, focus.z + dz), ());
            }
        }
        let load: Vec<ChunkCoord> = desired
            .keys()
            .filter(|c| !self.loaded.contains_key(c))
            .copied()
            .collect();
        let unload: Vec<ChunkCoord> = self
            .loaded
            .keys()
            .filter(|c| !desired.contains_key(c))
            .copied()
            .collect();
        self.loaded = desired;
        StreamingPlan { load, unload }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_update_loads_the_whole_disc_sorted() {
        let mut s = ChunkStreamer::new(1);
        let plan = s.update(ChunkCoord::new(5, 5));
        assert_eq!(plan.unload, Vec::new());
        assert_eq!(plan.load.len(), 9);
        // Sorted order: (4,4) < (4,5) < (4,6) < (5,4) < ...
        assert_eq!(
            plan.load[0],
            ChunkCoord::new(4, 4),
            "sorted row-major starts at the NW corner"
        );
        assert_eq!(plan.load[8], ChunkCoord::new(6, 6));
        // Idempotent: the same focus again is a no-op.
        let again = s.update(ChunkCoord::new(5, 5));
        assert_eq!(again, StreamingPlan::default());
    }

    #[test]
    fn crossing_a_boundary_unloads_and_loads_sorted() {
        let mut s = ChunkStreamer::new(1);
        s.update(ChunkCoord::new(0, 0));
        // Move east by one chunk: the -1 column unloads, the +2 loads.
        let plan = s.update(ChunkCoord::new(1, 0));
        assert_eq!(
            plan.unload,
            vec![
                ChunkCoord::new(-1, -1),
                ChunkCoord::new(-1, 0),
                ChunkCoord::new(-1, 1),
            ]
        );
        assert_eq!(
            plan.load,
            vec![
                ChunkCoord::new(2, -1),
                ChunkCoord::new(2, 0),
                ChunkCoord::new(2, 1),
            ]
        );
        assert_eq!(s.loaded().len(), 9);
    }

    #[test]
    fn focus_sequence_determinism() {
        let path = [(0, 0), (0, 0), (1, 0), (2, 1), (2, 1), (1, 1), (-1, -2)];
        // The same focus sequence on two fresh streamers must produce
        // the same plan sequence (restart determinism) — plans depend on
        // prior state, so a REVERSED sequence would legitimately differ.
        let mut a = ChunkStreamer::new(2);
        let mut b = ChunkStreamer::new(2);
        let plans_a: Vec<StreamingPlan> = path
            .iter()
            .map(|&(x, z)| a.update(ChunkCoord::new(x, z)))
            .collect();
        let plans_b: Vec<StreamingPlan> = path
            .iter()
            .map(|&(x, z)| b.update(ChunkCoord::new(x, z)))
            .collect();
        assert_eq!(plans_a, plans_b, "restart determinism over a focus path");
        assert_eq!(a.loaded(), b.loaded());
        // And the loaded set after the path is order-of-traversal
        // invariant for the FINAL focus only when the path ends there —
        // pin the final set explicitly instead.
        assert_eq!(a.loaded().len(), 25);
    }

    #[test]
    fn radius_zero_loads_only_the_focus() {
        let mut s = ChunkStreamer::new(0);
        let plan = s.update(ChunkCoord::new(3, 4));
        assert_eq!(plan.load, vec![ChunkCoord::new(3, 4)]);
        let plan = s.update(ChunkCoord::new(4, 4));
        assert_eq!(plan.load, vec![ChunkCoord::new(4, 4)]);
        assert_eq!(plan.unload, vec![ChunkCoord::new(3, 4)]);
    }

    #[test]
    fn negative_radius_clamps_to_zero() {
        let s = ChunkStreamer::new(-3);
        assert_eq!(s.radius(), 0);
    }
}
