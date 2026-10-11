//! Skeletal animation: clips, blending and state machines (Phase 4).
//!
//! `gens-animation` is a **pure, deterministic animation core**:
//!
//! - [`skeleton::Skeleton`] — a flat, cycle-free joint hierarchy
//!   (parents are always earlier indices) with forward kinematics;
//! - [`clip::Clip`] — duration-bounded keyframe tracks per joint channel
//!   (translation / rotation / scale) with linear, step and shortest-arc
//!   slerp sampling.
//!
//! Blending and the state-machine controller land in the next sub-item;
//! both stay in this crate and stay pure.
//!
//! Everything is a function of (state, time): sampling allocates no
//! hidden state and never consults wall clocks, so the same input
//! sequence replays identically within a process (see `docs/RISKS.md`
//! R-013; cross-OS bit-identity is not claimed).
//!
//! Mesh-space skinning and clip import formats (glTF et al.) arrive with
//! the assets phase; this crate's data model is designed to be the
//! sampling layer under them.

pub mod clip;
pub mod skeleton;

pub use clip::{Clip, ClipError, Interpolation, JointTrack, QuatKey, QuatTrack, VecKey, VecTrack};
pub use skeleton::{Joint, JointPose, LocalPose, Skeleton, SkeletonError};
