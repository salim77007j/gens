//! The gens-ecs prelude: one import for typical engine code.
//!
//! ```
//! use gens_ecs::prelude::*;
//!
//! let mut world = World::new();
//! world.spawn((Transform::from_position(Vec3::ONE), Name::new("hero"), Active));
//! ```

// Bevy ECS essentials (Entity, World, Component, Query, Schedule, filters...).
pub use bevy_ecs::prelude::*;
// Math types used with transforms.
pub use gens_math::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4, quat, vec2, vec3};
// Engine components and APIs. Explicit imports shadow any upstream glob
// (e.g. bevy's own Name) so the gens facade always wins.
pub use crate::components::{Active, Name, Tag, Transform, WorldTransform};
pub use crate::hierarchy::{EngineSchedules, attach_children, detach_from_parent};
pub use crate::propagation::transform_propagation_system;
