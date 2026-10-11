//! Rigid body physics on rapier3d (Phase 3, ADR-0013).
//!
//! `gens-physics` wraps the pinned `rapier3d` engine behind a small facade:
//! a [`world::PhysicsWorld`] for simulation state, value-type specs for
//! bodies and colliders, and ECS glue ([`ecs`]) that steps the world and
//! copies body poses into the standard [`gens_ecs::components::Transform`]
//! component so rendering (Phase 2) reads physics results unchanged.
//!
//! Why a facade: rapier 0.36 resolves to its own glam (0.33.x) while gens
//! standardizes on glam 0.34 via `gens-math` (ADR-0001), so all vector and
//! quaternion values cross the boundary through explicit conversions in
//! [`convert`]. Engine code never imports rapier directly; advanced users
//! can reach the raw crate as [`backend`], mirroring the `gens-ecs::ecs`
//! convention (ADR-0002).
//!
//! Determinism: the simulation advances in fixed steps
//! ([`PhysicsWorld::DEFAULT_DT`]) and is deterministic **within one process
//! and platform** (pinned by a unit test). Bit-identical results across
//! different CPU platforms are NOT claimed — see `docs/RISKS.md` R-013.

pub mod convert;
pub mod ecs;
pub mod world;

/// The underlying rapier3d crate (0.36.x), re-exported for advanced use.
///
/// Engine code should prefer the facade; see ADR-0013 for the pinning and
/// churn-containment strategy (mirrors `gens-ecs::ecs`, ADR-0002).
pub use rapier3d as backend;

pub use ecs::{PhysicsBody, PhysicsResource, step_physics_system, sync_transforms_from_physics};
pub use world::{
    BodyHandle, BodyKind, BodySpec, ColliderHandle, ColliderShape, ColliderSpec, PhysicsWorld,
};
