//! Entity component system facade over bevy_ecs (ADR-0002).
//!
//! Engine code uses gens-ecs types and the [`prelude`]; direct bevy_ecs
//! imports are confined to this crate. The raw crate remains reachable as
//! [`ecs`] for advanced cases (rendering internals in later phases), which
//! keeps the facade a convention rather than a wall.

pub mod components;
pub mod hierarchy;
pub mod prelude;
pub mod propagation;

/// The underlying bevy_ecs crate (0.20.x), re-exported for advanced use.
/// Engine code should prefer the facade; see ADR-0002 for the pinning and
/// churn-containment strategy.
pub use bevy_ecs as ecs;

pub use components::{Active, Name, Tag, Transform, WorldTransform};
pub use hierarchy::{EngineSchedules, attach_children, detach_from_parent, parent_world_position};
pub use propagation::transform_propagation_system;
