//! Skeletal animation: clips, blending and state machines (Phase 4).
//!
//! `gens-animation` is a **pure, deterministic animation core**:
//!
//! - [`skeleton::Skeleton`] — a flat, cycle-free joint hierarchy
//!   (parents are always earlier indices) with forward kinematics;
//! - [`clip::Clip`] — duration-bounded keyframe tracks per joint channel
//!   (translation / rotation / scale) with linear, step and shortest-arc
//!   slerp sampling;
//! - [`blend`] — in-place partial blends and timed crossfades;
//! - [`controller::AnimationController`] — named states, declared
//!   parameters, condition-gated transitions and crossfade bookkeeping.
//!
//! Everything is a function of (state, time): no clocks, no hidden
//! state, so the same call sequence replays identically within a
//! process (see `docs/RISKS.md` R-013; cross-OS bit-identity is not
//! claimed). Controllers are replay-tested bit-for-bit.
//!
//! Mesh-space skinning and clip import formats (glTF et al.) arrive with
//! the assets phase; this crate's data model is designed to be the
//! sampling layer under them.

pub mod blend;
pub mod clip;
pub mod controller;
pub mod skeleton;

pub use blend::{blend_into, crossfade};
pub use clip::{Clip, ClipError, Interpolation, JointTrack, QuatKey, QuatTrack, VecKey, VecTrack};
pub use controller::{
    AnimationController, Condition, ControllerError, NumberOp, ParamValue, StateId, Transition,
};
pub use skeleton::{Joint, JointPose, LocalPose, Skeleton, SkeletonError};
