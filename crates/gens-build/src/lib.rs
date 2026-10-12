//! Build pipeline: package a game project, verify it, run it (Phase 8).
//!
//! The pipeline in one breath: a project is a **scene data document**
//! ([`gens_assets::DataDocument`], the Phase-6 authored format) plus the
//! asset files its scripts and content reference. [`package`] wraps both
//! into a versioned, FNV-hash-verified `GENSPKG1` container — the same
//! framing discipline as the save format (magic line, canonical manifest,
//! length-prefixed data, hash trailer). [`player`] loads a verified
//! package into a fresh registry + world, ticks the scripts and the
//! standard schedules headlessly, validates invariants, and emits a
//! `gens-run-report-1` JSON — the export story, proven on CI.
//!
//! Everything here is deterministic and CPU-only: two player runs over
//! the same package produce identical simulation fields (final hash,
//! tick counts, entity counts). Timing fields (profile, memory) are
//! observation-only sidecars (ADR-0027).

pub mod package;
pub mod player;

pub use package::{
    PackageError, PackageFile, PackageManifest, VerifiedPackage, build_package, verify_package,
};
pub use player::{LoadedGame, RunReport, load_package, run_loaded, run_package};
