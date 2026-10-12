//! World introspection and invariant checking (Phase 8 debugging, P8-4).
//!
//! Two tools over the live ECS, both pure readers:
//! - [`debug_dump`] — a canonical, JSON-serializable snapshot of every
//!   observed entity (identity, components, hierarchy links, script
//!   summary) in entity-index order. The "what is actually in the world"
//!   answer for bug reports and CI artifacts.
//! - [`validate_world`] — invariant checking with a loud, typed
//!   [`Violation`] vocabulary (no stringly-typed errors): non-finite
//!   transforms, non-unit rotations, zero scale, broken propagation,
//!   dangling hierarchy links, duplicate names, stale physics handles,
//!   unregistered script keys.
//!
//! Both skip the engine's resource-seed entities with the same content
//! filter [`crate::save::capture_world`] uses (ADR-0021 canonical order).
//! Nothing here mutates the world (observation-only, ADR-0027).

use gens_assets::AssetRegistry;
use gens_ecs::ecs::world::World;
use gens_ecs::{Active, Name, Tag, Transform, WorldTransform};
use gens_physics::PhysicsResource;
use gens_scripting::ScriptComponent;
use serde::Serialize;

/// The debug dump's format tag.
pub const DEBUG_FORMAT: &str = "gens-debug-1";

/// A transform triple in plain floats (serde-friendly).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct TripleView {
    /// x component.
    pub x: f32,
    /// y component.
    pub y: f32,
    /// z component.
    pub z: f32,
}

fn triple(v: gens_math::Vec3) -> TripleView {
    TripleView {
        x: v.x,
        y: v.y,
        z: v.z,
    }
}

/// One entity's observed state.
#[derive(Debug, Clone, Serialize)]
pub struct DebugEntity {
    /// Entity index (the canonical order, ADR-0021).
    pub index: u32,
    /// Name, if any.
    pub name: Option<String>,
    /// Whether the `Active` component is present.
    pub active: bool,
    /// Tag, if any.
    pub tag: Option<String>,
    /// Component kind names present (static, greppable).
    pub components: Vec<&'static str>,
    /// Local transform, if any.
    pub transform: Option<TransformView>,
    /// World transform as a column-major 4x4, if propagated.
    pub world_transform: Option<[[f32; 4]; 4]>,
    /// Parent entity index, if parented.
    pub parent: Option<u32>,
    /// Child entity indices, if any.
    pub children: Vec<u32>,
    /// Script summary, if any.
    pub script: Option<ScriptView>,
}

/// Local transform view.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct TransformView {
    /// Position.
    pub position: TripleView,
    /// Rotation (xyzw).
    pub rotation: [f32; 4],
    /// Scale.
    pub scale: TripleView,
}

/// Script summary view (identity + size, not the live vars).
#[derive(Debug, Clone, Serialize)]
pub struct ScriptView {
    /// Asset key of the script.
    pub script: String,
    /// Entry function.
    pub entry: String,
    /// Number of script variables.
    pub vars: usize,
}

/// Canonical debug snapshot of `world`.
pub fn debug_dump(world: &World) -> DebugDump {
    let mut entities: Vec<DebugEntity> = world
        .iter_entities()
        .filter_map(|entity_ref| {
            let entity = entity_ref.id();
            let name = world.get::<Name>(entity).map(|n| n.0.clone());
            let transform = world.get::<Transform>(entity).copied();
            let world_transform = world.get::<WorldTransform>(entity).map(|w| w.0);
            let active = world.get::<Active>(entity).is_some();
            let tag = world.get::<Tag>(entity).map(|t| t.0.clone());
            let script = world.get::<ScriptComponent>(entity).map(|s| ScriptView {
                script: s.script.clone(),
                entry: s.entry.clone(),
                vars: s.vars.len(),
            });
            let physics = world.get::<gens_physics::PhysicsBody>(entity).is_some();
            let parent = world
                .get::<gens_ecs::ecs::hierarchy::ChildOf>(entity)
                .map(|c| c.parent().index().index());
            let children: Vec<u32> = world
                .get::<gens_ecs::ecs::hierarchy::Children>(entity)
                .map(|children| children.iter().map(|e| e.index().index()).collect())
                .unwrap_or_default();

            if name.is_none()
                && !active
                && tag.is_none()
                && transform.is_none()
                && world_transform.is_none()
                && script.is_none()
                && !physics
                && parent.is_none()
                && children.is_empty()
            {
                return None; // engine-reserved entity with nothing observable
            }

            let mut components = Vec::new();
            if name.is_some() {
                components.push("Name");
            }
            if active {
                components.push("Active");
            }
            if tag.is_some() {
                components.push("Tag");
            }
            if transform.is_some() {
                components.push("Transform");
            }
            if world_transform.is_some() {
                components.push("WorldTransform");
            }
            if parent.is_some() {
                components.push("ChildOf");
            }
            if !children.is_empty() {
                components.push("Children");
            }
            if physics {
                components.push("PhysicsBody");
            }
            if script.is_some() {
                components.push("ScriptComponent");
            }

            Some(DebugEntity {
                index: entity.index().index(),
                name,
                active,
                tag,
                components,
                transform: transform.map(|t| TransformView {
                    position: triple(t.position),
                    rotation: t.rotation.to_array(),
                    scale: triple(t.scale),
                }),
                world_transform: world_transform.map(|m| m.to_cols_array_2d()),
                parent,
                children,
                script,
            })
        })
        .collect();
    entities.sort_by_key(|e| e.index);
    let entity_count = entities.len();
    DebugDump {
        format: DEBUG_FORMAT,
        entity_count,
        entities,
    }
}

/// The dump document.
#[derive(Debug, Clone, Serialize)]
pub struct DebugDump {
    /// Format tag (`gens-debug-1`).
    pub format: &'static str,
    /// Number of observed entities.
    pub entity_count: usize,
    /// Entities in entity-index order.
    pub entities: Vec<DebugEntity>,
}

impl DebugDump {
    /// Compact JSON (byte-stable field order via serde).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("DebugDump serializes")
    }
}

/// The invariant vocabulary. Typed on purpose: matches in tests, greppable
/// in logs, and impossible to typo like a `&str` code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ViolationKind {
    /// A Transform field is NaN or infinite (`field` names which).
    NonFiniteTransform,
    /// A Transform rotation's quaternion length differs from 1 by > 1e-4.
    NonUnitRotation,
    /// A Transform scale component is exactly zero.
    ZeroScale,
    /// A derived WorldTransform matrix is NaN or infinite.
    NonFiniteWorldTransform,
    /// A `ChildOf` relationship points at a despawned entity.
    DanglingChildOf,
    /// A `Transform`-bearing entity has no derived `WorldTransform` —
    /// propagation has not run (or was skipped) for it.
    TransformWithoutWorldTransform,
    /// Two or more entities share this name; authored data documents
    /// require uniqueness (ADR-0023), so duplicates break the export path.
    DuplicateName,
    /// A `PhysicsBody` handle no longer refers to a live body.
    MissingPhysicsBody,
    /// A script key is not in the provided registry (registry given).
    ScriptNotRegistered,
}

/// One invariant violation at one entity.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Violation {
    /// Which invariant failed.
    pub kind: ViolationKind,
    /// Entity index the violation was found at.
    pub entity: u32,
    /// Human-readable detail (which field, which key, ...).
    pub detail: String,
}

/// Checks the world's invariants, returning every violation (empty = the
/// world is clean). `registry` enables the script-key check; pass `None`
/// when no registry is available (the check is then skipped honestly, not
/// silently failed).
///
/// Physics handle checks run only when a [`PhysicsResource`] is present.
pub fn validate_world(world: &World, registry: Option<&AssetRegistry>) -> Vec<Violation> {
    let mut violations = Vec::new();

    // Pass 1: per-entity invariants.
    for entity_ref in world.iter_entities() {
        let entity = entity_ref.id();
        let index = entity.index().index();
        let transform = world.get::<Transform>(entity);
        if let Some(t) = transform {
            if !t.position.is_finite() {
                violations.push(Violation {
                    kind: ViolationKind::NonFiniteTransform,
                    entity: index,
                    detail: "position".into(),
                });
            }
            if !t.rotation.is_finite() {
                violations.push(Violation {
                    kind: ViolationKind::NonFiniteTransform,
                    entity: index,
                    detail: "rotation".into(),
                });
            }
            if !t.scale.is_finite() {
                violations.push(Violation {
                    kind: ViolationKind::NonFiniteTransform,
                    entity: index,
                    detail: "scale".into(),
                });
            }
            if (t.rotation.length() - 1.0).abs() > 1e-4 {
                violations.push(Violation {
                    kind: ViolationKind::NonUnitRotation,
                    entity: index,
                    detail: format!("length = {}", t.rotation.length()),
                });
            }
            if t.scale.x == 0.0 || t.scale.y == 0.0 || t.scale.z == 0.0 {
                violations.push(Violation {
                    kind: ViolationKind::ZeroScale,
                    entity: index,
                    detail: format!("scale = {:?}", t.scale.to_array()),
                });
            }
            if world.get::<WorldTransform>(entity).is_none() {
                violations.push(Violation {
                    kind: ViolationKind::TransformWithoutWorldTransform,
                    entity: index,
                    detail: "propagation has not run for this entity".into(),
                });
            }
        }
        if let Some(wt) = world.get::<WorldTransform>(entity)
            && !wt.0.to_cols_array().iter().all(|f| f.is_finite())
        {
            violations.push(Violation {
                kind: ViolationKind::NonFiniteWorldTransform,
                entity: index,
                detail: "world transform matrix contains non-finite values".into(),
            });
        }
        if let Some(child_of) = world.get::<gens_ecs::ecs::hierarchy::ChildOf>(entity) {
            let parent = child_of.parent();
            if world.get_entity(parent).is_err() {
                violations.push(Violation {
                    kind: ViolationKind::DanglingChildOf,
                    entity: index,
                    detail: format!("parent index {}", parent.index().index()),
                });
            }
        }
        if let Some(script) = world.get::<ScriptComponent>(entity)
            && let Some(registry) = registry
            && registry.get(&script.script).is_none()
        {
            violations.push(Violation {
                kind: ViolationKind::ScriptNotRegistered,
                entity: index,
                detail: format!("script key '{}'", script.script),
            });
        }
    }

    // Pass 2: name uniqueness (authored data documents require it).
    let mut by_name: std::collections::BTreeMap<&str, Vec<u32>> = Default::default();
    for entity_ref in world.iter_entities() {
        let entity = entity_ref.id();
        if let Some(name) = world.get::<Name>(entity) {
            by_name
                .entry(name.0.as_str())
                .or_default()
                .push(entity.index().index());
        }
    }
    for (name, holders) in by_name {
        if holders.len() > 1 {
            for index in holders {
                violations.push(Violation {
                    kind: ViolationKind::DuplicateName,
                    entity: index,
                    detail: format!("'{name}' is used by multiple entities"),
                });
            }
        }
    }

    // Pass 3: physics handles (only when a physics world is present).
    if let Some(physics) = world.get_resource::<PhysicsResource>() {
        for entity_ref in world.iter_entities() {
            let entity = entity_ref.id();
            if let Some(body) = world.get::<gens_physics::PhysicsBody>(entity)
                && !physics.world.body_exists(body.handle)
            {
                violations.push(Violation {
                    kind: ViolationKind::MissingPhysicsBody,
                    entity: entity.index().index(),
                    detail: "physics body handle is not live".into(),
                });
            }
        }
    }

    violations.sort_by(|a, b| {
        a.entity
            .cmp(&b.entity)
            .then(serde_name(&a.kind).cmp(serde_name(&b.kind)))
            .then(a.detail.cmp(&b.detail))
    });
    violations
}

// serde_name helper: the enum's serde tag is the snake_case name; derive
// does not expose it, so this little shim keeps the sort deterministic.
fn serde_name(kind: &ViolationKind) -> &'static str {
    match kind {
        ViolationKind::NonFiniteTransform => "non_finite_transform",
        ViolationKind::NonUnitRotation => "non_unit_rotation",
        ViolationKind::ZeroScale => "zero_scale",
        ViolationKind::NonFiniteWorldTransform => "non_finite_world_transform",
        ViolationKind::DanglingChildOf => "dangling_child_of",
        ViolationKind::TransformWithoutWorldTransform => "transform_without_world_transform",
        ViolationKind::DuplicateName => "duplicate_name",
        ViolationKind::MissingPhysicsBody => "missing_physics_body",
        ViolationKind::ScriptNotRegistered => "script_not_registered",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_ecs::{Active, Name, Transform};
    use gens_math::{Quat, Vec3};

    fn clean_world() -> World {
        let mut world = World::new();
        let e = world
            .spawn((Name::new("A"), Transform::from_position(Vec3::ONE), Active))
            .id();
        // Give the entity its derived transform (what propagation does).
        world
            .entity_mut(e)
            .insert(WorldTransform(gens_math::Mat4::from_translation(Vec3::ONE)));
        world
    }

    #[test]
    fn clean_world_validates_empty() {
        let world = clean_world();
        assert!(
            validate_world(&world, None).is_empty(),
            "clean world must have no violations"
        );
    }

    #[test]
    fn dump_lists_identity_components_and_hierarchy() {
        let mut world = World::new();
        let parent = world
            .spawn((Name::new("Parent"), Transform::IDENTITY, Active))
            .id();
        world
            .entity_mut(parent)
            .insert(WorldTransform(gens_math::Mat4::IDENTITY));
        let child = world
            .spawn((
                Name::new("Child"),
                Transform::from_position(Vec3::new(1.0, 0.0, 0.0)),
                Active,
            ))
            .id();
        gens_ecs::attach_children(&mut world, parent, &[child]);
        world
            .entity_mut(child)
            .insert(WorldTransform(gens_math::Mat4::IDENTITY));

        let dump = debug_dump(&world);
        assert_eq!(dump.entity_count, 2);
        let child_row = dump
            .entities
            .iter()
            .find(|e| e.name.as_deref() == Some("Child"))
            .unwrap();
        assert_eq!(child_row.parent, Some(parent.index().index()));
        let parent_row = dump
            .entities
            .iter()
            .find(|e| e.name.as_deref() == Some("Parent"))
            .unwrap();
        assert_eq!(parent_row.children, vec![child.index().index()]);
        assert!(child_row.components.contains(&"ChildOf"));
        assert!(parent_row.components.contains(&"Children"));
        assert!(child_row.world_transform.is_some());
    }

    #[test]
    fn dump_json_is_stable_and_tagged() {
        let world = clean_world();
        let a = debug_dump(&world).to_json();
        let b = debug_dump(&world).to_json();
        assert_eq!(a, b);
        assert!(a.contains(r#""format":"gens-debug-1""#));
    }

    #[test]
    fn non_finite_transform_is_caught_per_field() {
        let mut world = clean_world();
        let e = world
            .query::<(&Name, gens_ecs::ecs::entity::Entity)>()
            .iter(&world)
            .find(|(n, _)| n.0 == "A")
            .map(|(_, e)| e)
            .unwrap();
        world.get_mut::<Transform>(e).unwrap().position = Vec3::new(f32::NAN, 0.0, 0.0);
        let violations = validate_world(&world, None);
        assert!(
            violations
                .iter()
                .any(|v| v.kind == ViolationKind::NonFiniteTransform && v.detail == "position")
        );
    }

    #[test]
    fn non_unit_rotation_is_caught() {
        let mut world = clean_world();
        let e = world
            .query::<(&Name, gens_ecs::ecs::entity::Entity)>()
            .iter(&world)
            .find(|(n, _)| n.0 == "A")
            .map(|(_, e)| e)
            .unwrap();
        world.get_mut::<Transform>(e).unwrap().rotation = Quat::from_xyzw(1.0, 1.0, 0.0, 0.0);
        let violations = validate_world(&world, None);
        assert!(
            violations
                .iter()
                .any(|v| v.kind == ViolationKind::NonUnitRotation)
        );
    }

    #[test]
    fn zero_scale_is_caught() {
        let mut world = clean_world();
        let e = world
            .query::<(&Name, gens_ecs::ecs::entity::Entity)>()
            .iter(&world)
            .find(|(n, _)| n.0 == "A")
            .map(|(_, e)| e)
            .unwrap();
        world.get_mut::<Transform>(e).unwrap().scale = Vec3::new(1.0, 0.0, 1.0);
        let violations = validate_world(&world, None);
        assert!(
            violations
                .iter()
                .any(|v| v.kind == ViolationKind::ZeroScale)
        );
    }

    #[test]
    fn missing_world_transform_is_caught() {
        let mut world = World::new();
        world.spawn((Name::new("A"), Transform::from_position(Vec3::ONE), Active));
        let violations = validate_world(&world, None);
        assert!(
            violations
                .iter()
                .any(|v| v.kind == ViolationKind::TransformWithoutWorldTransform),
            "a transform-bearing entity without propagation must be flagged"
        );
    }

    #[test]
    #[should_panic(expected = "not spawned")]
    fn bevy_rejects_relationships_to_unspawned_entities() {
        // Engine fact pinned here (verified from bevy_ecs 0.20 behavior):
        // inserting a ChildOf whose parent is not spawned panics. The
        // public API therefore cannot produce a dangling ChildOf — the
        // validate_world branch stays as defense-in-depth for corrupted
        // states (hand-crafted worlds, future engine changes), not for
        // everyday editing mistakes.
        let mut world = World::new();
        let orphan = world
            .spawn((Name::new("Orphan"), Transform::IDENTITY, Active))
            .id();
        world
            .entity_mut(orphan)
            .insert(gens_ecs::ecs::hierarchy::ChildOf(
                gens_ecs::ecs::entity::Entity::PLACEHOLDER,
            ));
    }

    #[test]
    fn duplicate_names_are_caught_for_every_holder() {
        let mut world = World::new();
        for i in 0..2 {
            let e = world
                .spawn((
                    Name::new("Twin"),
                    Transform::from_position(Vec3::splat(i as f32)),
                    Active,
                ))
                .id();
            world
                .entity_mut(e)
                .insert(WorldTransform(gens_math::Mat4::IDENTITY));
        }
        let violations = validate_world(&world, None);
        let dups: Vec<_> = violations
            .iter()
            .filter(|v| v.kind == ViolationKind::DuplicateName)
            .collect();
        assert_eq!(
            dups.len(),
            2,
            "every holder of a duplicated name is flagged"
        );
    }

    #[test]
    fn unregistered_script_key_is_caught_only_with_a_registry() {
        let mut world = World::new();
        let e = world
            .spawn((Name::new("Bot"), Transform::IDENTITY, Active))
            .id();
        world
            .entity_mut(e)
            .insert(WorldTransform(gens_math::Mat4::IDENTITY));
        world
            .entity_mut(e)
            .insert(ScriptComponent::new("scripts/missing.rhai"));

        // Without a registry: the check is skipped honestly.
        assert!(validate_world(&world, None).is_empty());

        // With an empty registry: loud.
        let registry = gens_assets::AssetRegistry::new();
        let violations = validate_world(&world, Some(&registry));
        assert!(
            violations
                .iter()
                .any(|v| v.kind == ViolationKind::ScriptNotRegistered)
        );
    }

    #[test]
    fn missing_physics_body_is_caught_when_physics_resource_present() {
        let mut world = clean_world();
        let e = world
            .query::<(&Name, gens_ecs::ecs::entity::Entity)>()
            .iter(&world)
            .find(|(n, _)| n.0 == "A")
            .map(|(_, e)| e)
            .unwrap();
        let mut physics = gens_physics::PhysicsWorld::new(Vec3::ZERO);
        // A handle from a DIFFERENT (discarded) world: never live here.
        let (stale, _) = physics.create_body(
            gens_physics::BodySpec::fixed_at(Vec3::ZERO),
            gens_physics::ColliderSpec {
                shape: gens_physics::ColliderShape::Ball(0.1),
                ..Default::default()
            },
        );
        drop(physics);
        let fresh_physics = gens_physics::PhysicsWorld::new(Vec3::ZERO);
        world.insert_resource(gens_physics::PhysicsResource {
            world: fresh_physics,
        });
        world
            .entity_mut(e)
            .insert(gens_physics::PhysicsBody { handle: stale });
        let violations = validate_world(&world, None);
        assert!(
            violations
                .iter()
                .any(|v| v.kind == ViolationKind::MissingPhysicsBody)
        );
    }
}
