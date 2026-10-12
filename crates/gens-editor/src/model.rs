//! Editor-side authored component vocabulary.
//!
//! These are REAL ECS components the editor owns: the viewport draws
//! every entity carrying a [`MeshSlot`], the scene builder turns every
//! [`AuthoredBody`] into an actual `gens_physics` body+collider at
//! build time, and the inspector edits both directly in the world.
//! Nothing here is display-only: every field is consumed by engine code.

use gens_ecs::ecs::prelude::Component;
use gens_physics::world::{BodyKind, BodySpec, ColliderShape, ColliderSpec};

/// Primitive the viewport rasterizes for an entity.
///
/// Tint is straight RGBA8 (the colored pipeline's vertex-color space —
/// see ADR-0018 for the sRGB decode-at-boundary convention).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct MeshSlot {
    /// Geometry to draw (scaled by the entity's `Transform.scale`).
    pub shape: ShapeKind,
    /// Opaque tint, RGBA8 bytes.
    pub tint: [u8; 4],
}

impl MeshSlot {
    /// A tinted cube (the default authored mesh).
    pub fn cube(tint: [u8; 4]) -> Self {
        Self {
            shape: ShapeKind::Cube,
            tint,
        }
    }

    /// A tinted ground quad on the XZ plane.
    pub fn quad(tint: [u8; 4]) -> Self {
        Self {
            shape: ShapeKind::Quad,
            tint,
        }
    }

    /// A small light/gizmo marker.
    pub fn marker(tint: [u8; 4]) -> Self {
        Self {
            shape: ShapeKind::Marker,
            tint,
        }
    }
}

/// Which primitive a [`MeshSlot`] draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ShapeKind {
    /// Unit cube centered on the transform.
    Cube,
    /// 1x1 quad on the XZ plane (floors, platforms).
    Quad,
    /// 0.15-unit marker cube (lights, waypoints).
    Marker,
}

impl ShapeKind {
    /// All kinds in inspector dropdown order.
    pub const ALL: [ShapeKind; 3] = [ShapeKind::Cube, ShapeKind::Quad, ShapeKind::Marker];

    /// Stable label for the inspector dropdown.
    pub fn as_str(self) -> &'static str {
        match self {
            ShapeKind::Cube => "Cube",
            ShapeKind::Quad => "Quad",
            ShapeKind::Marker => "Marker",
        }
    }
}

/// Authored physics description — the exact values the scene builder
/// feeds into `BodySpec`/`ColliderSpec` and registers with the real
/// `PhysicsWorld` (the returned handle becomes the entity's
/// `PhysicsBody` component, so what the inspector shows is what the
/// simulation runs).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct AuthoredBody {
    /// Simulation role (Static / Dynamic — `BodyKind` vocabulary).
    pub kind: BodyKind,
    /// Continuous collision detection (mockup: "Use CCD").
    pub ccd: bool,
    /// Linear velocity damping (`BodySpec::linear_damping`).
    pub linear_damping: f32,
    /// Collider geometry (`ColliderSpec::shape`).
    pub collider: ColliderShape,
    /// Contact friction in 0..=1 (`ColliderSpec::friction`).
    pub friction: f32,
    /// Bounciness in 0..=1 (`ColliderSpec::restitution`).
    pub restitution: f32,
    /// Mass density — mass derives from shape volume x density
    /// (`ColliderSpec::density`), so the inspector shows density, never
    /// a fake "mass" number.
    pub density: f32,
}

impl Default for AuthoredBody {
    fn default() -> Self {
        let b = BodySpec::default();
        let c = ColliderSpec::default();
        Self {
            kind: b.kind,
            ccd: b.ccd,
            linear_damping: b.linear_damping,
            collider: c.shape,
            friction: c.friction,
            restitution: c.restitution,
            density: c.density,
        }
    }
}

/// Editor commands queued by the menu bar, drained by the embedding app
/// (example binary / future window shell). The shell never executes
/// world-level commands itself — it only records intent.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorCommand {
    /// File > Save Scene: write the REAL GENSSAVE1 format to this path.
    SaveScene(String),
    /// Edit > Duplicate Selected: clone the known component set.
    DuplicateSelected,
    /// Edit > Delete Selected: despawn the selection.
    DeleteSelected,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The authored-body default must mirror the physics crate's own
    /// defaults (`BodySpec::default()` / `ColliderSpec::default()`) — a
    /// drift here would silently change what simulation runs vs what
    /// the inspector displays.
    #[test]
    fn authored_body_defaults_match_physics_defaults() {
        let ab = AuthoredBody::default();
        let b = BodySpec::default();
        let c = ColliderSpec::default();
        assert_eq!(ab.kind, b.kind);
        assert_eq!(ab.ccd, b.ccd);
        assert_eq!(ab.linear_damping, b.linear_damping);
        assert_eq!(ab.collider, c.shape);
        assert_eq!(ab.friction, c.friction);
        assert_eq!(ab.restitution, c.restitution);
        assert_eq!(ab.density, c.density);
    }

    #[test]
    fn shape_kind_labels_are_stable() {
        assert_eq!(ShapeKind::ALL.len(), 3);
        assert_eq!(ShapeKind::Cube.as_str(), "Cube");
        assert_eq!(ShapeKind::Quad.as_str(), "Quad");
        assert_eq!(ShapeKind::Marker.as_str(), "Marker");
    }
}
