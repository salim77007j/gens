//! Exported-game runtime shell.
//!
//! Phase 1 contents: [`snapshot`] — the CPU rasterizer that produces
//! deterministic scene PNGs for headless runs and CI evidence. The full
//! runtime (windowing, audio output, export pipeline) arrives in later
//! phases.
//!
//! Phase 5 contents: [`save`] — versioned, hash-verified world
//! save/load (see ADR-0021).

pub mod save;
pub mod snapshot;
