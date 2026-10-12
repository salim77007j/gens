//! Deterministic cinematic timelines (Phase 9).
//!
//! # The determinism contract (ADR-0030)
//!
//! A cinematic is **pure data plus pure sampling**: tracks are sorted
//! keyframe lists, sampling is a pure function of local time, and playback
//! advances by the caller-supplied fixed delta — never by a wall clock
//! (the deterministic `FixedSource` discipline of `gens-core`, mirrored
//! here by taking `dt` as a parameter). Like the animation crate, this
//! crate has **no ECS dependency**: the director state is data, and
//! applying sampled values to components is the embedding layer's job
//! (`gens-runtime` examples, the editor). Two runs with the same
//! fixed-step sequence replay byte-identically by construction — and are
//! *proven* to in the phase's replay example, per the ADR-0027 proof
//! pattern.
//!
//! # Module plan (landed by sub-item, each with tests)
//!
//! - `track` — typed keyframe tracks (`Float`/`Vec3`/`Quat` channels)
//!   with `Linear`/`Step`/`Smoothstep` interpolation, clamp-at-ends
//!   semantics and validation (strictly increasing times), mirroring the
//!   `gens-animation` clip vocabulary where the semantics agree.
//! - `timeline` — a named set of tracks plus exactly-once event markers;
//!   logical targets bind to entities at playback start via an ordered
//!   binding map, so the same timeline can drive any scene.
//! - `player` — deterministic playback state machine (`play`/`pause`/
//!   `stop`/`seek`, speed, loop) that returns the events fired by each
//!   `advance(dt)`; event firing is crossing detection over the sampled
//!   interval, exact under fixed steps and documented under loop wrap.
//! - `director` — the playback component plus the camera-cut contract:
//!   while a cinematic owns a bound camera it stages the camera pose the
//!   renderer consumes, and on finish it hands control back — the same
//!   explicit-staging rule the Phase 7 viewport taught (ADR-0026).

#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod track;

pub mod timeline;

pub mod player;
