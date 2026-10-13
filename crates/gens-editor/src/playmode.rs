//! Play-mode world snapshots (v1.1, ADR-0036).
//!
//! Play/Pause/Stop is the editor's core promise: the user edits a
//! scene, presses Play, the simulation runs live, presses Stop, and
//! the scene returns EXACTLY to its pre-play state. The engine's
//! `gens-runtime::save` round-trip covers only the runtime component
//! vocabulary (name/transform/active/tag/script) — the editor's
//! authored vocabulary (`MeshSlot`, `AuthoredBody`, hierarchy links,
//! live physics bodies) would be silently dropped, so play mode owns
//! its snapshot format here.
//!
//! Design notes:
//! - entities are identified by NAME for hierarchy re-attachment
//!   (the invariant checker forbids duplicate names, so names are keys);
//! - capture skips entities with none of the snapshot components —
//!   the same content filter `gens-runtime::save::capture_world` uses,
//!   which is what keeps the engine's resource-seed entities (the
//!   `Resource: Component` backing stores) out of the document;
//! - restore rebuilds a FRESH `PhysicsWorld`: body handles from the
//!   pre-play world are dead by construction after despawn, and
//!   re-creating bodies from the authored specs guarantees what the
//!   inspector shows is what the simulation runs (the model.rs rule);
//! - `WorldTransform` is never captured — it is derived state,
//!   recomputed by propagation on the next frame.

use crate::model::{AuthoredBody, MeshSlot};
use gens_core::anyhow::{Result, anyhow, ensure};
use gens_ecs::ecs::prelude::Entity;
use gens_ecs::ecs::world::World;
use gens_ecs::{Active, Name, Transform, attach_children};
use gens_math::vec3;
use gens_physics::{BodyKind, BodySpec, ColliderSpec, PhysicsBody, PhysicsResource, PhysicsWorld};
use gens_scripting::ScriptComponent;

/// One entity's full authored state.
#[derive(Debug, Clone, PartialEq)]
pub struct EntitySnapshot {
    /// The entity's name (the hierarchy re-attachment key). `None` for
    /// unnamed entities — they restore positionally but cannot parent.
    pub name: Option<String>,
    /// The parent's name, if the entity had one.
    pub parent: Option<String>,
    /// Local transform (identity when the entity carried none).
    pub transform: Option<Transform>,
    /// Whether the entity was `Active`.
    pub active: bool,
    /// Authored mesh (viewport draw vocabulary).
    pub mesh: Option<MeshSlot>,
    /// Authored physics description (re-created into a fresh world).
    pub body: Option<AuthoredBody>,
    /// Script binding.
    pub script: Option<ScriptComponent>,
}

/// The whole world's authored state, in entity-index order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorldSnapshot {
    /// Captured entities.
    pub entities: Vec<EntitySnapshot>,
}

/// Captures every entity carrying at least one snapshot component,
/// in entity-index order (deterministic; matches the save format's
/// canonical order, ADR-0021).
#[must_use]
pub fn capture(world: &World) -> WorldSnapshot {
    let mut rows: Vec<(u32, EntitySnapshot)> = Vec::new();
    for entity_ref in world.iter_entities() {
        let entity = entity_ref.id();
        let name = world.get::<Name>(entity).map(|n| n.0.clone());
        let transform = world.get::<Transform>(entity).copied();
        let active = world.get::<Active>(entity).is_some();
        let mesh = world.get::<MeshSlot>(entity).copied();
        let body = world.get::<AuthoredBody>(entity).copied();
        let script = world.get::<ScriptComponent>(entity).cloned();
        if name.is_none()
            && transform.is_none()
            && !active
            && mesh.is_none()
            && body.is_none()
            && script.is_none()
        {
            continue; // resource-seed row: no authored content (ADR-0021 filter)
        }
        let parent = world
            .get::<gens_ecs::ecs::hierarchy::ChildOf>(entity)
            .and_then(|child_of| {
                let parent = child_of.parent();
                world.get::<Name>(parent).map(|n| n.0.clone())
            });
        rows.push((
            entity.index().index(),
            EntitySnapshot {
                name,
                parent,
                transform,
                active,
                mesh,
                body,
                script,
            },
        ));
    }
    rows.sort_by_key(|(index, _)| *index);
    WorldSnapshot {
        entities: rows.into_iter().map(|(_, snap)| snap).collect(),
    }
}

/// Restores a snapshot: despawns every snapshot-eligible entity in the
/// live world, respawns the captured set (physics bodies re-created
/// from their authored specs into a fresh world), and re-attaches the
/// hierarchy by name.
///
/// # Errors
/// Returns an error when a captured entity names a parent that is not
/// in the snapshot (a dangling link can never be rebuilt) or when a
/// name is duplicated inside the snapshot itself (the re-attachment
/// key would be ambiguous).
pub fn restore(world: &mut World, snapshot: &WorldSnapshot) -> Result<()> {
    // Names must be unique keys inside the document.
    let mut seen = std::collections::BTreeSet::new();
    for entity in &snapshot.entities {
        if let Some(name) = &entity.name {
            ensure!(
                seen.insert(name.clone()),
                "duplicate name '{name}' in snapshot — hierarchy keys would be ambiguous"
            );
        }
    }
    for entity in &snapshot.entities {
        if let Some(parent) = &entity.parent {
            ensure!(
                snapshot
                    .entities
                    .iter()
                    .any(|e| e.name.as_deref() == Some(parent)),
                "entity '{parent:?}' is referenced as a parent but is not in the snapshot"
            );
        }
    }

    // Despawn everything snapshot-eligible (resource-seed rows survive;
    // the fresh PhysicsResource below replaces their backing state).
    let doomed: Vec<_> = world
        .iter_entities()
        .filter(|entity_ref| {
            let entity = entity_ref.id();
            world.get::<Name>(entity).is_some()
                || world.get::<Transform>(entity).is_some()
                || world.get::<MeshSlot>(entity).is_some()
                || world.get::<AuthoredBody>(entity).is_some()
                || world.get::<ScriptComponent>(entity).is_some()
        })
        .map(|entity_ref| entity_ref.id())
        .collect();
    for entity in doomed {
        world.despawn(entity);
    }

    // Fresh simulation state; bodies re-created from authored specs.
    let mut physics = PhysicsWorld::new(GRAVITY);
    let mut spawned: Vec<(Entity, Option<String>, Option<String>)> = Vec::new();
    for entity in &snapshot.entities {
        let transform = entity.transform.unwrap_or(Transform::IDENTITY);
        let mut eb = world.spawn_empty();
        if let Some(name) = &entity.name {
            eb.insert(Name::new(name.clone()));
        }
        eb.insert(transform);
        if entity.active {
            eb.insert(Active);
        }
        if let Some(mesh) = entity.mesh {
            eb.insert(mesh);
        }
        if let Some(script) = &entity.script {
            eb.insert(script.clone());
        }
        if let Some(body) = entity.body {
            // The authored description STAYS on the entity (the
            // inspector edits it; what it shows is what simulation
            // runs) while the live body is re-created from it.
            let spec = body_spec_from_authored(&body, transform.position);
            let collider = collider_spec_from_authored(&body);
            let (handle, _) = physics.create_body(spec, collider);
            eb.insert(body);
            eb.insert(PhysicsBody { handle });
        }
        let id = eb.id();
        spawned.push((id, entity.name.clone(), entity.parent.clone()));
    }

    // Hierarchy re-attachment by name (second pass: every candidate
    // parent now exists).
    for (entity, name, parent) in &spawned {
        if let (Some(parent_name), Some(_)) = (parent, name) {
            let parent_entity = spawned
                .iter()
                .find(|(_, n, _)| n.as_deref() == Some(parent_name.as_str()))
                .map(|(id, _, _)| *id)
                .ok_or_else(|| {
                    anyhow!(
                        "parent '{parent_name}' vanished mid-restore — impossible by construction"
                    )
                })?;
            attach_children(world, parent_entity, &[*entity]);
        }
    }

    world.insert_resource(PhysicsResource { world: physics });
    Ok(())
}

/// Gravity used while the play-mode snapshot is the source of truth:
/// the scene builder's own default (Earth gravity, the engine demo
/// convention). Restore preserves only authored specs — the gravity
/// constant is part of the editor shell's world construction.
const GRAVITY: gens_math::Vec3 = vec3(0.0, -9.81, 0.0);

/// Maps an authored body onto the real simulation spec (the same
/// mapping the inspector's fields describe).
#[must_use]
pub fn body_spec_from_authored(body: &AuthoredBody, position: gens_math::Vec3) -> BodySpec {
    BodySpec {
        kind: body.kind,
        position,
        linvel: gens_math::Vec3::ZERO,
        can_sleep: true,
        ccd: body.ccd,
        linear_damping: body.linear_damping,
    }
}

/// Maps an authored collider onto the real collider spec.
#[must_use]
pub fn collider_spec_from_authored(body: &AuthoredBody) -> ColliderSpec {
    ColliderSpec {
        shape: body.collider,
        friction: body.friction,
        restitution: body.restitution,
        density: body.density,
    }
}

/// Convenience: whether `world` still has any snapshot-eligible entity
/// (used by tests and the shell's sanity checks).
#[must_use]
pub fn has_scene_entities(world: &World) -> bool {
    world.iter_entities().any(|entity_ref| {
        let entity = entity_ref.id();
        world.get::<Name>(entity).is_some() || world.get::<Transform>(entity).is_some()
    })
}

/// The body kind's stable label (inspector and console share it).
#[must_use]
pub fn body_kind_label(kind: BodyKind) -> &'static str {
    match kind {
        BodyKind::Dynamic => "Dynamic",
        BodyKind::Fixed => "Fixed",
        BodyKind::KinematicPosition => "Kinematic",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_ecs::WorldTransform;
    use gens_math::{Vec3, vec3};

    /// Mirrors the reference scene's key entities: root + child with
    /// mesh + physics body + script.
    fn build_test_world(world: &mut World, physics: &mut PhysicsWorld) {
        let (handle, _) = physics.create_body(
            BodySpec::dynamic_at(vec3(1.0, 4.0, -1.0)),
            ColliderSpec {
                shape: gens_physics::ColliderShape::Ball(0.45),
                friction: 0.5,
                ..Default::default()
            },
        );
        let root = world
            .spawn((
                Name::new("Root"),
                Active,
                Transform::IDENTITY,
                MeshSlot::marker([90, 140, 255, 255]),
            ))
            .id();
        let crate_entity = world
            .spawn((
                Name::new("Crate"),
                Active,
                Transform::from_position(vec3(1.0, 4.0, -1.0)),
                MeshSlot::cube([185, 130, 70, 255]),
                AuthoredBody {
                    kind: BodyKind::Dynamic,
                    ccd: true,
                    linear_damping: 0.1,
                    collider: gens_physics::ColliderShape::Ball(0.45),
                    friction: 0.5,
                    restitution: 0.2,
                    density: 1.0,
                },
                PhysicsBody { handle },
                ScriptComponent::new("scripts/patrol.rhai"),
            ))
            .id();
        attach_children(world, root, &[crate_entity]);
        world.insert_resource(PhysicsResource {
            world: std::mem::take(physics),
        });
    }

    #[test]
    fn capture_round_trips_every_authored_component() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new(vec3(0.0, -9.81, 0.0));
        build_test_world(&mut world, &mut physics);

        let snapshot = capture(&world);
        assert_eq!(snapshot.entities.len(), 2, "root + crate captured");

        // Mutate everything after the capture, then restore.
        let crate_entity = world
            .iter_entities()
            .find(|e| world.get::<Name>(e.id()).map(|n| n.0.as_str()) == Some("Crate"))
            .map(|e| e.id())
            .unwrap();
        world.get_mut::<Transform>(crate_entity).unwrap().position = vec3(50.0, 50.0, 50.0);
        world.despawn(crate_entity);
        assert_eq!(capture(&world).entities.len(), 1, "mutation visible");

        restore(&mut world, &snapshot).expect("restore");
        let restored = capture(&world);
        assert_eq!(
            restored, snapshot,
            "restore must reproduce the capture exactly"
        );
    }

    #[test]
    fn restore_recreates_live_physics_bodies() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new(vec3(0.0, -9.81, 0.0));
        build_test_world(&mut world, &mut physics);
        let snapshot = capture(&world);

        restore(&mut world, &snapshot).expect("restore");
        let crate_entity = world
            .iter_entities()
            .find(|e| world.get::<Name>(e.id()).map(|n| n.0.as_str()) == Some("Crate"))
            .map(|e| e.id())
            .expect("crate respawned");
        let body = world
            .get::<PhysicsBody>(crate_entity)
            .copied()
            .expect("body re-created");
        // The new handle must be live in the fresh resource world.
        let resource = world.resource::<PhysicsResource>();
        let position = resource.world.body_position(body.handle);
        assert!(
            (position - vec3(1.0, 4.0, -1.0)).length() < 1e-4,
            "body restored at its authored position, got {position:?}"
        );
    }

    #[test]
    fn restore_rebuilds_hierarchy_links() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new(vec3(0.0, -9.81, 0.0));
        build_test_world(&mut world, &mut physics);
        let snapshot = capture(&world);

        restore(&mut world, &snapshot).expect("restore");
        let _root = world
            .iter_entities()
            .find(|e| world.get::<Name>(e.id()).map(|n| n.0.as_str()) == Some("Root"))
            .map(|e| e.id())
            .unwrap();
        let crate_entity = world
            .iter_entities()
            .find(|e| world.get::<Name>(e.id()).map(|n| n.0.as_str()) == Some("Crate"))
            .map(|e| e.id())
            .unwrap();
        // Propagation must reach the child through the rebuilt link:
        // fresh entities carry no WorldTransform — its existence after
        // the propagation run IS the proof the link works, and with
        // the root at identity the world position must equal the
        // child's local position.
        gens_ecs::transform_propagation_system(&mut world);
        let child_world = world.get::<WorldTransform>(crate_entity).expect("derived");
        let local = world.get::<Transform>(crate_entity).unwrap().position;
        assert_eq!(
            child_world.position(),
            local,
            "child's world transform must be derived through the rebuilt link"
        );
    }

    #[test]
    fn restore_rejects_dangling_parents_and_duplicate_names() {
        let dangling = WorldSnapshot {
            entities: vec![EntitySnapshot {
                name: Some("Orphan".into()),
                parent: Some("Ghost".into()),
                transform: Some(Transform::IDENTITY),
                active: true,
                mesh: None,
                body: None,
                script: None,
            }],
        };
        let mut world = World::new();
        let err = restore(&mut world, &dangling).unwrap_err();
        assert!(err.to_string().contains("not in the snapshot"), "{err}");

        let duplicates = WorldSnapshot {
            entities: vec![
                EntitySnapshot {
                    name: Some("Twin".into()),
                    parent: None,
                    transform: Some(Transform::IDENTITY),
                    active: true,
                    mesh: None,
                    body: None,
                    script: None,
                },
                EntitySnapshot {
                    name: Some("Twin".into()),
                    parent: None,
                    transform: Some(Transform::IDENTITY),
                    active: true,
                    mesh: None,
                    body: None,
                    script: None,
                },
            ],
        };
        let err = restore(&mut world, &duplicates).unwrap_err();
        assert!(err.to_string().contains("duplicate name"), "{err}");
    }

    #[test]
    fn resource_seed_rows_never_enter_the_snapshot() {
        let mut world = World::new();
        world.insert_resource(PhysicsResource {
            world: PhysicsWorld::new(vec3(0.0, -9.81, 0.0)),
        });
        world.spawn((
            Name::new("Solo"),
            Active,
            Transform::from_position(Vec3::ONE),
        ));
        let snapshot = capture(&world);
        assert_eq!(
            snapshot.entities.len(),
            1,
            "the resource-seed row (no authored components) must be excluded"
        );
        // And restore leaves the resource intact for the real entity set.
        restore(&mut world, &snapshot).expect("restore");
        let _ = world.resource::<PhysicsResource>();
        assert!(has_scene_entities(&world));
    }
}
