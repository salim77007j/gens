//! Deterministic procedural world building (Phase 9).
//!
//! # The determinism contract (ADR-0031)
//!
//! Every generator is a **pure function of its integer inputs**: chunk
//! content is derived from `(seed, chunk_coord)` alone, never from
//! generation order, wall-clock time or a shared RNG stream. The noise
//! core is integer-lattice hashing (SplitMix64-style mixing) followed by
//! smoothstep-interpolated value noise and fractal Brownian motion built
//! ONLY from add/multiply/floor/smoothstep — **no transcendental
//! functions anywhere**, because IEEE-754 basic operations are
//! bit-identical across platforms while `sin`/`pow` implementations are
//! not (R-013 discipline, stated as an ADR rule, not a convention).
//! Like the rest of the engine's leaf crates, this crate has no ECS and
//! no I/O dependency: meshes and spawn sets come out as plain data, and
//! the embedding layer applies them.
//!
//! # Module plan (landed by sub-item, each with tests)
//!
//! - `noise` — seeded 2D value noise + FBM with an explicit octave plan;
//!   the FNV-hashable output is what makes terrain reproducible.
//! - `heightfield` — a sampled height grid with cell-size metadata and
//!   the heightfield-to-mesh conversion (positions + central-difference
//!   normals + indices) with a content hash for ledger discipline.
//! - `scatter` — deterministic prop placement: jittered lattice points
//!   where each point is derived from `(seed, cell)`, filtered by the
//!   same noise as a density mask — order-independent by construction.
//! - `chunk` — `ChunkCoord` addressing and the content plan (terrain
//!   mesh + scatter set) a coord generates; `streaming` — the
//!   load/unload set computed from a focus point and a Chebyshev radius,
//!   returned in sorted deterministic order for the caller to apply.

#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
