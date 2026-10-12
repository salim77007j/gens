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
//!
//! Phase 8 contents: [`profile`] — observation-only instrumentation and
//! the canonical [`profile::world_state_hash`] the ADR-0027 proof uses
//! (profiler on vs off must replay byte-identically).

pub mod data;
pub mod net;
pub mod profile;
pub mod save;
pub mod snapshot;
