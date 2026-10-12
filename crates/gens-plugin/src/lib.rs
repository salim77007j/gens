//! The Gens plugin API (Phase 9).
//!
//! # The registration contract (ADR-0032)
//!
//! A plugin is an in-process unit of engine extension: it registers
//! **real typed bevy systems** onto the engine's schedules and spawns
//! startup content into the world at composition time — the same
//! vocabulary the engine itself uses, with no reflection, no dynamic
//! libraries and no `dyn`-erased system boxing. Registration is
//! **deterministic by construction**: plugins apply in the order the
//! composition root lists them, duplicate plugin names are loud errors,
//! and the applied set is recorded so a run can report exactly what
//! extended it. The API surface lives on [`gens_ecs`] types
//! ([`gens_ecs::EngineSchedules`], the bevy `World`) so a plugin crate
//! outside this workspace can depend on the same leaf crates and
//! register systems that schedule identically to built-ins.
//!
//! # Module plan (landed by sub-item, each with tests)
//!
//! - `registry` — the [`Plugin`](crate::registry::Plugin) trait, the
//!   ordered duplicate-name-rejecting registry, and `apply` — which
//!   walks the registry, hands each plugin a host exposing the
//!   schedules and the world, and returns an applied record for
//!   evidence output.
//!
//! The out-of-tree proof (`plugins/hello-plugin`, excluded from the
//! workspace) lands as its own sub-item and CI step: a downstream crate
//! that path-depends on the engine crates and extends a running world.

#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
pub mod registry;

pub use registry::{AppliedPlugins, Plugin, PluginError, PluginHost, PluginRegistry};
