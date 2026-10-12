//! Exported-game runtime shell.
//!
//! Phase 1 contents: [`snapshot`] — the CPU rasterizer that produces
//! deterministic scene PNGs for headless runs and CI evidence. The full
//! runtime (windowing, audio output, export pipeline) arrives in later
//! phases.
//!
//! Phase 5 contents: [`save`] — versioned, hash-verified world
//! save/load (see ADR-0021).
//!
//! Phase 6 contents: [`data`] — instantiating data documents
//! ([`gens_assets::DataDocument`]) into live worlds, the mirror of
//! [`save`]; [`net`] lands with the networking pillar (P6-3/P6-4).

pub mod data;
pub mod net;
pub mod save;
pub mod snapshot;
