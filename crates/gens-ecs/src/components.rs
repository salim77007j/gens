//! Engine components (Phase 1).
//!
//! These are the core components every Gens scene is built on:
//! [`Transform`] (local), [`WorldTransform`] (computed), [`Name`],
//! [`Active`], [`Tag`].

use bevy_ecs::component::Component;
use gens_math::{Mat4, Quat, Vec3, compose_trs};
use std::fmt;

/// Local transform relative to the entity's parent (or the world, if root).
///
/// The authoritative transform an author edits; [`WorldTransform`] is derived
/// by the propagation system and must not be written by gameplay code.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Local position.
    pub position: Vec3,
    /// Local rotation.
    pub rotation: Quat,
    /// Local scale.
    pub scale: Vec3,
}

impl Transform {
    /// Identity transform (origin, no rotation, unit scale).
    pub const IDENTITY: Transform = Transform {
        position: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// Transform at `position` with identity rotation and unit scale.
    pub const fn from_position(position: Vec3) -> Transform {
        Transform {
            position,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }

    /// Builder: sets rotation.
    pub const fn with_rotation(mut self, rotation: Quat) -> Transform {
        self.rotation = rotation;
        self
    }

    /// Builder: sets scale.
    pub const fn with_scale(mut self, scale: Vec3) -> Transform {
        self.scale = scale;
        self
    }

    /// Composes the local translate * rotate * scale matrix
    /// (convention documented in `gens_math::compose_trs`).
    pub fn to_matrix(&self) -> Mat4 {
        compose_trs(self.position, self.rotation, self.scale)
    }
}

impl Default for Transform {
    fn default() -> Self {
        Transform::IDENTITY
    }
}

/// Computed world-space transform. Written ONLY by
/// [`transform_propagation_system`](crate::transform_propagation_system).
#[derive(Component, Debug, Clone, Copy, PartialEq, Default)]
pub struct WorldTransform(pub Mat4);

impl WorldTransform {
    /// World-space position extracted from the matrix.
    pub fn position(&self) -> Vec3 {
        self.0.w_axis.truncate()
    }
}

/// Human-readable entity name (authoring aid; lookup is by query, not hash).
#[derive(Component, Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Name(pub String);

impl Name {
    /// New name from any string-like value.
    pub fn new(name: impl Into<String>) -> Name {
        Name(name.into())
    }

    /// Borrowed string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Marker: the entity participates in simulation and rendering.
///
/// Absence means the entity is logically inactive (systems decide what to
/// skip; the component is the single source of truth).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Active;

/// Free-form label for grouping and tooling queries (one tag per entity in
/// Phase 1; multi-tag is a later-phase extension if tooling needs it).
#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
pub struct Tag(pub String);

impl Tag {
    /// New tag from any string-like value.
    pub fn new(tag: impl Into<String>) -> Tag {
        Tag(tag.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_math::{deg_to_rad, eq_eps, vec3};

    #[test]
    fn transform_to_matrix_matches_compose_trs() {
        let t = Transform {
            position: vec3(1.0, 2.0, 3.0),
            rotation: Quat::from_rotation_y(deg_to_rad(90.0)),
            scale: vec3(2.0, 2.0, 2.0),
        };
        let m = t.to_matrix();
        assert!(eq_eps(m.w_axis.x, 1.0));
        assert!(eq_eps(m.w_axis.y, 2.0));
        assert!(eq_eps(m.w_axis.z, 3.0));
    }

    #[test]
    fn transform_builders_chain() {
        let t = Transform::from_position(vec3(1.0, 0.0, 0.0))
            .with_rotation(Quat::from_rotation_x(deg_to_rad(45.0)))
            .with_scale(vec3(2.0, 2.0, 2.0));
        assert!(eq_eps(t.position.x, 1.0));
        assert_eq!(t.scale, vec3(2.0, 2.0, 2.0));
    }

    #[test]
    fn world_transform_position_reads_w_axis() {
        let m = compose_trs(vec3(7.0, 8.0, 9.0), Quat::IDENTITY, Vec3::ONE);
        assert_eq!(WorldTransform(m).position(), vec3(7.0, 8.0, 9.0));
    }

    #[test]
    fn name_display_and_accessors() {
        let n = Name::new("Hero");
        assert_eq!(n.to_string(), "Hero");
        assert_eq!(n.as_str(), "Hero");
        assert_eq!(Name::default().0, "");
    }

    #[test]
    fn tag_new_wraps_string() {
        assert_eq!(Tag::new("enemy").0, "enemy");
    }
}
