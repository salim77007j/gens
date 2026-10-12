//! Profiling, memory tracking and benchmark support (Phase 8).
//!
//! # The observation-only contract (ADR-0027)
//!
//! Everything this crate measures is **sidecar data**: wall-clock spans and
//! allocation counters are *read* around the simulation, never *fed into*
//! it. The engine's byte-identical replay contracts (Phases 2-7) hash
//! simulation state only; a profiler that merely observes cannot change
//! what is hashed. Phase 8 proves this with a test that replays the same
//! deterministic scene with profiling on and off and demands identical
//! frame-hash sequences.
//!
//! # Module plan (landed by sub-item, each with tests)
//!
//! - `spans` — handle-based nested span capture per frame
//!   ([`Profiler::begin_frame`]/[`Profiler::begin_span`]/[`Profiler::end_span`]),
//!   with a closure wrapper ([`Profiler::time`]) for leaf scopes and a
//!   disabled fast path that costs one branch.
//! - `report` — per-name aggregation over N frames (count, min, median,
//!   p99, max, share of frame) and JSON export.
//! - `memory` — a tracking [`GlobalAlloc`](std::alloc::GlobalAlloc) wrapper
//!   exposing current/peak bytes and allocation counts; the honest,
//!   allocator-level answer to the reference design's "Memory" metric.
//!   Installed as the process `#[global_allocator]` when this crate is
//!   linked (always-on, documented in ADR-0027 and RISKS R-030): every
//!   allocation costs two atomic updates, and the engine gains a live,
//!   honest "Memory" metric everywhere.
//!
//! Benchmarks (`bench_step`, honest environment-disclosed numbers) live in
//! `gens-runtime`'s examples so they can time the real engine step, not a
//! mock.

#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod memory;
pub mod report;
pub mod spans;

pub use memory::{MemorySnapshot, MemoryStats, TrackingAllocator};
pub use report::{PROFILE_FORMAT, ProfileReport, Quartiles, SpanStats};
pub use spans::{FrameProfile, Profiler, SpanId, SpanRecord};

/// The process-wide memory statistics, updated by
/// [`TrackingAllocator`] (the `#[global_allocator]` this crate installs
/// on link). Read this for live memory numbers.
pub fn memory_stats() -> &'static MemoryStats {
    &memory::STATS
}
