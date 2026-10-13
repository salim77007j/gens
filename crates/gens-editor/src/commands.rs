//! Console commands and the shared editor-command executor (P8-6, v1.1).
//!
//! Two layers speak here, both with the same rule: the shell records
//! INTENT ([`EditorState::pending`] / [`EditorState::pending_commands`]);
//! the embedding app drains and executes. The interactive bin and the
//! headless example both call [`execute_editor_command`] so windowed
//! and headless behavior can never drift.
//!
//! Console vocabulary (deliberately small, each command real):
//! - `help` — list the vocabulary
//! - `entities` — live entity count + names in entity-index order
//! - `validate` — run the runtime invariant checker on the live world
//! - `stats` — canonical world state hash + counters
//! - `clear` — clear the console ring
//!
//! Unknown commands are a loud WARN, never a silent no-op.

use crate::model::{AuthoredBody, CreateKind, EditorCommand, MeshSlot, UndoOp};
use crate::playmode;
use crate::shell::EditorState;
use gens_assets::AssetRegistry;
use gens_ecs::ecs::world::World;
use gens_ecs::{Active, Name, Transform};
use gens_math::vec3;
use gens_physics::{BodyKind, ColliderShape, PhysicsResource};
use gens_runtime::debug::validate_world;
use gens_scripting::ScriptComponent;

/// Executes one console command against the live world. Returns true when
/// the command was recognized.
pub fn execute_console_command(
    state: &mut EditorState,
    world: &mut World,
    registry: Option<&AssetRegistry>,
    command: &str,
) -> bool {
    let command = command.trim();
    match command {
        "help" => {
            tracing::info!("commands: help | entities | validate | stats | clear");
        }
        "entities" => {
            let mut rows: Vec<(u32, String, bool)> = world
                .iter_entities()
                .filter_map(|entity_ref| {
                    let entity = entity_ref.id();
                    let name = world.get::<Name>(entity)?;
                    Some((
                        entity.index().index(),
                        name.0.clone(),
                        world.get::<gens_ecs::Active>(entity).is_some(),
                    ))
                })
                .collect();
            rows.sort_by_key(|(index, _, _)| *index);
            tracing::info!(
                "live entities: {} (resource-seed rows excluded by name filter)",
                rows.len()
            );
            const MAX_LISTED: usize = 20;
            for (i, (index, name, active)) in rows.iter().enumerate() {
                if i >= MAX_LISTED {
                    tracing::info!("... and {} more", rows.len() - MAX_LISTED);
                    break;
                }
                tracing::info!("entity #{} '{}' active={}", index, name, active);
            }
        }
        "validate" => {
            let violations = validate_world(world, registry);
            if violations.is_empty() {
                tracing::info!("validation: 0 violations -- the world is clean");
            } else {
                for v in &violations {
                    tracing::warn!(
                        "violation at entity #{}: {} ({})",
                        v.entity,
                        kind_label(&v.kind),
                        v.detail
                    );
                }
                tracing::warn!("validation found {} violation(s)", violations.len());
            }
        }
        "stats" => {
            let hash = gens_runtime::profile::world_state_hash(world);
            tracing::info!(
                "world stats: state_hash={:016x} entities={} steps={} frames={}",
                hash,
                world.iter_entities().count(),
                state.stats.step,
                state.stats.frame
            );
        }
        "clear" => {
            state.console.clear();
        }
        other => {
            tracing::warn!("unknown console command '{}' (try 'help')", other);
            return false;
        }
    }
    true
}

/// Short human label for a violation kind (the console is text).
fn kind_label(kind: &gens_runtime::debug::ViolationKind) -> &'static str {
    use gens_runtime::debug::ViolationKind::*;
    match kind {
        NonFiniteTransform => "non-finite transform",
        NonUnitRotation => "non-unit rotation",
        ZeroScale => "zero scale",
        NonFiniteWorldTransform => "non-finite world transform",
        DanglingChildOf => "dangling parent link",
        TransformWithoutWorldTransform => "missing world transform",
        DuplicateName => "duplicate name",
        MissingPhysicsBody => "missing physics body",
        ScriptNotRegistered => "unregistered script",
    }
}

/// Drains and executes every queued command (the app's per-frame body).
pub fn drain_commands(
    state: &mut EditorState,
    world: &mut World,
    registry: Option<&AssetRegistry>,
) {
    for command in state.take_command_inputs() {
        execute_console_command(state, world, registry, &command);
    }
}

// ---------------------------------------------------------------------------
// The shared editor-command executor (v1.1): windowed bin and headless
// example both run exactly this — one behavior, two frontends.
// ---------------------------------------------------------------------------

/// Executes one editor command against the live world. Intent that
/// arrived from the toolbar, the menus or the keyboard all lands here.
pub fn execute_editor_command(state: &mut EditorState, world: &mut World, command: &EditorCommand) {
    match command {
        EditorCommand::SaveScene(path) => {
            match playmode::save_scene_document(world).and_then(|json| {
                std::fs::write(path, json)
                    .map_err(|e| gens_core::anyhow::anyhow!("write '{path}': {e}"))
            }) {
                Ok(()) => tracing::info!(path, "Saved scene (gensscene-1)"),
                Err(e) => tracing::error!("save failed: {e}"),
            }
        }
        EditorCommand::LoadScene(path) => match std::fs::read_to_string(path)
            .map_err(|e| gens_core::anyhow::anyhow!("read '{path}': {e}"))
            .and_then(|json| playmode::load_scene_document(world, &json))
        {
            Ok(()) => {
                // All entity ids died in the restore: drop session state
                // that references them.
                state.selected = None;
                state.undo_stack.clear();
                state.viewport_drag = None;
                tracing::info!(path, "Loaded scene (gensscene-1)");
            }
            Err(e) => tracing::error!("load failed: {e}"),
        },
        EditorCommand::DuplicateSelected => {
            if let Some(name) = duplicate_selected(world, state) {
                tracing::info!(name, "Entity spawned");
            }
        }
        EditorCommand::DeleteSelected => delete_selected(state, world),
        EditorCommand::Undo => {
            apply_undo(state, world);
        }
        EditorCommand::Play => match state.play_mode {
            crate::shell::PlayMode::Idle => {
                let snapshot = playmode::capture(world);
                state.play_snapshot = Some(snapshot);
                state.play_mode = crate::shell::PlayMode::Playing;
                state.play_time = 0.0;
                tracing::info!("play mode entered (pre-play snapshot captured)");
            }
            crate::shell::PlayMode::Paused => {
                state.play_mode = crate::shell::PlayMode::Playing;
                tracing::info!("play resumed");
            }
            crate::shell::PlayMode::Playing => {
                tracing::warn!("already playing (Play is a no-op mid-simulation)");
            }
        },
        EditorCommand::Pause => {
            if state.play_mode == crate::shell::PlayMode::Playing {
                state.play_mode = crate::shell::PlayMode::Paused;
                tracing::info!("simulation paused");
            }
        }
        EditorCommand::Stop => {
            if let Some(snapshot) = state.play_snapshot.take() {
                match playmode::restore(world, &snapshot) {
                    Ok(()) => tracing::info!("simulation stopped — pre-play state restored"),
                    Err(e) => tracing::error!("stop/restore failed: {e}"),
                }
            }
            state.play_mode = crate::shell::PlayMode::Idle;
            state.play_time = 0.0;
            // Restored entities carry NEW ids: session state that points
            // at the old ones is stale by construction.
            state.selected = None;
            state.undo_stack.clear();
            state.viewport_drag = None;
        }
        EditorCommand::CreateObject(kind) => {
            let name = create_object(state, world, *kind);
            tracing::info!(name, "Entity spawned");
        }
    }
}

/// Drains every queued editor command in order (the app's per-frame body).
pub fn drain_editor_commands(state: &mut EditorState, world: &mut World) {
    for command in state.take_commands() {
        execute_editor_command(state, world, &command);
    }
}

/// Edit > Duplicate Selected — clones the known component set with a
/// " copy" name and records a Create undo op. Returns the new name.
pub fn duplicate_selected(world: &mut World, state: &mut EditorState) -> Option<String> {
    let sel = state.selected?;
    let name = world.get::<Name>(sel)?.as_str().to_string();
    let transform = world.get::<Transform>(sel).copied();
    let mesh = world.get::<MeshSlot>(sel).copied();
    let body = world.get::<AuthoredBody>(sel).copied();
    let script = world.get::<ScriptComponent>(sel).cloned();
    let new_name = format!("{name} copy");
    let mut eb = world.spawn((Name::new(new_name.clone()), Active));
    if let Some(t) = transform {
        eb.insert(t);
    }
    if let Some(m) = mesh {
        eb.insert(m);
    }
    if let Some(s) = script {
        eb.insert(s);
    }
    let id = eb.id();
    if let Some(b) = body {
        let position = transform
            .map(|t| t.position)
            .unwrap_or(gens_math::Vec3::ZERO);
        let spec = playmode::body_spec_from_authored(&b, position);
        let collider = playmode::collider_spec_from_authored(&b);
        let (handle, _) = world
            .resource_mut::<PhysicsResource>()
            .world
            .create_body(spec, collider);
        world
            .entity_mut(id)
            .insert(b)
            .insert(gens_physics::PhysicsBody { handle });
    }
    state.undo_stack.push(UndoOp::Create { entity: id });
    state.selected = Some(id);
    Some(new_name)
}

/// Edit > Delete Selected — captures the full authored snapshot FIRST
/// (the undo payload), then despawns. Selection and drag state follow.
pub fn delete_selected(state: &mut EditorState, world: &mut World) {
    let Some(sel) = state.selected else {
        tracing::warn!("delete requested with no selection");
        return;
    };
    // The undo payload is the entity's own row (parent link included).
    let snapshot = playmode::capture(world)
        .entities
        .into_iter()
        .find(|entity| {
            entity.name.is_some()
                && world
                    .get::<Name>(sel)
                    .map(|n| Some(n.0.clone()))
                    .unwrap_or(None)
                    == entity.name
        });
    if let Some(snapshot) = snapshot {
        state.undo_stack.push(UndoOp::Delete { snapshot });
    }
    world.despawn(sel);
    state.selected = None;
    state.viewport_drag = None;
    tracing::info!("Entity deleted");
}

/// GameObject > Create — spawns a new object near the world origin with
/// a unique name, records a Create undo op and selects it. Returns the
/// name.
#[must_use]
pub fn create_object(state: &mut EditorState, world: &mut World, kind: CreateKind) -> String {
    let count = world.iter_entities().count();
    let name = unique_name(world, kind.label(), count);
    let spawn_position = vec3(0.0, 0.5, 0.0);
    let mut eb = world.spawn((Name::new(name.clone()), Active));
    let mut body: Option<AuthoredBody> = None;
    match kind {
        CreateKind::Empty => {
            eb.insert(Transform::from_position(spawn_position));
        }
        CreateKind::Cube => {
            eb.insert((
                Transform::from_position(spawn_position),
                MeshSlot::cube([110, 170, 250, 255]),
            ));
            body = Some(AuthoredBody {
                kind: BodyKind::Dynamic,
                collider: ColliderShape::Cuboid(vec3(0.5, 0.5, 0.5)),
                ..AuthoredBody::default()
            });
        }
        CreateKind::Marker => {
            eb.insert((
                Transform::from_position(spawn_position),
                MeshSlot::marker([250, 210, 110, 255]),
            ));
        }
    }
    let id = eb.id();
    if let Some(body) = body {
        let spec = playmode::body_spec_from_authored(&body, spawn_position);
        let collider = playmode::collider_spec_from_authored(&body);
        let (handle, _) = world
            .resource_mut::<PhysicsResource>()
            .world
            .create_body(spec, collider);
        world
            .entity_mut(id)
            .insert(body)
            .insert(gens_physics::PhysicsBody { handle });
    }
    state.undo_stack.push(UndoOp::Create { entity: id });
    state.selected = Some(id);
    name
}

/// Edit > Undo — pops the undo stack and applies the operation.
/// Unrecognized (dead) entities make the op a loud no-op: ids are
/// session-scoped and play-mode restores invalidate them.
pub fn apply_undo(state: &mut EditorState, world: &mut World) {
    let Some(op) = state.undo_stack.pop() else {
        tracing::info!("nothing to undo");
        return;
    };
    match op {
        UndoOp::Transform { entity, before } => {
            if let Some(mut t) = world.get_mut::<Transform>(entity) {
                *t = before;
                // Physics bodies follow the restored position so the
                // next sync doesn't snap the entity back.
                if let Some(body) = world.get::<gens_physics::PhysicsBody>(entity).copied() {
                    world
                        .resource_mut::<PhysicsResource>()
                        .world
                        .set_body_position(body.handle, before.position);
                }
                tracing::info!("undo: transform restored");
            } else {
                tracing::warn!("undo: entity no longer exists (id {entity:?})");
            }
        }
        UndoOp::Delete { snapshot } => {
            let one = crate::playmode::WorldSnapshot {
                entities: vec![snapshot],
            };
            match playmode::restore(world, &one) {
                // The restored entity gets a NEW id; select it so the
                // user sees what came back.
                Ok(()) => {
                    let restored = playmode::capture(world);
                    state.selected = restored
                        .entities
                        .last()
                        .and_then(|e| e.name.clone())
                        .and_then(|name| {
                            world.iter_entities().find_map(|r| {
                                world
                                    .get::<Name>(r.id())
                                    .filter(|n| n.0 == name)
                                    .map(|_| r.id())
                            })
                        });
                    tracing::info!("undo: entity restored (new entity id)");
                }
                Err(e) => tracing::error!("undo/restore failed: {e}"),
            }
        }
        UndoOp::Create { entity } => {
            if world.despawn(entity) {
                tracing::info!("undo: created entity removed");
            } else {
                tracing::warn!("undo: created entity already gone (id {entity:?})");
            }
            if state.selected == Some(entity) {
                state.selected = None;
            }
        }
    }
}

/// Builds a unique name: `base`, `base 2`, `base 3`, ...
fn unique_name(world: &World, base: &str, attempt: usize) -> String {
    let mut candidate = base.to_string();
    let mut n = attempt.max(1);
    loop {
        let taken = world.iter_entities().any(|r| {
            world
                .get::<Name>(r.id())
                .map(|name| name.0 == candidate)
                .unwrap_or(false)
        });
        if !taken {
            return candidate;
        }
        n += 1;
        candidate = format!("{base} {n}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{EditorState, FrameStats};
    use gens_ecs::{Active, Transform};
    use gens_math::Vec3;
    use std::sync::Arc;
    use tracing_subscriber::layer::SubscriberExt;

    fn populated_state() -> (EditorState, World) {
        let mut state = EditorState::new();
        state.stats = FrameStats {
            frame: 3,
            step: 2,
            fps: 60.0,
            entities: 1,
        };
        let mut world = World::new();
        let hero = world
            .spawn((
                Name::new("Hero"),
                Transform::from_position(Vec3::ONE),
                Active,
            ))
            .id();
        // Derived transform present (what propagation computes) — the test
        // world starts CLEAN; individual tests then break invariants.
        world
            .entity_mut(hero)
            .insert(gens_ecs::WorldTransform(gens_math::Mat4::from_translation(
                Vec3::ONE,
            )));
        (state, world)
    }

    /// Commands speak through tracing; capture with the console's own sink
    /// so the assertion checks the REAL pipeline (sink -> ring -> lines).
    fn captured_lines<F: FnOnce()>(
        buffer: &Arc<crate::console::ConsoleBuffer>,
        f: F,
    ) -> Vec<String> {
        let sink = crate::console::ConsoleSink::new(buffer.clone());
        let subscriber = tracing_subscriber::registry().with(sink);
        tracing::subscriber::with_default(subscriber, f);
        buffer.lines().into_iter().map(|l| l.message).collect()
    }

    #[test]
    fn help_entities_stats_are_recognized_and_speak() {
        let (mut state, mut world) = populated_state();
        let buffer = state.console.clone();
        let lines = captured_lines(&buffer, || {
            assert!(execute_console_command(
                &mut state, &mut world, None, "help"
            ));
            assert!(execute_console_command(
                &mut state, &mut world, None, "entities"
            ));
            assert!(execute_console_command(
                &mut state, &mut world, None, "stats"
            ));
        });
        assert!(
            lines
                .iter()
                .any(|m| m.contains("help | entities | validate")),
            "help text spoken"
        );
        assert!(
            lines.iter().any(|m| m.contains("live entities")),
            "entity count spoken"
        );
        assert!(
            lines.iter().any(|m| m.contains("Hero")),
            "entity name spoken"
        );
        assert!(
            lines.iter().any(|m| m.contains("state_hash=")),
            "hash spoken"
        );
    }

    #[test]
    fn unknown_command_is_loud_and_reports_false() {
        let (mut state, mut world) = populated_state();
        let buffer = state.console.clone();
        let lines = captured_lines(&buffer, || {
            assert!(!execute_console_command(
                &mut state,
                &mut world,
                None,
                "frobnicate"
            ));
        });
        assert!(lines.iter().any(|m| m.contains("unknown console command")));
    }

    #[test]
    fn validate_reports_clean_world_and_real_violations() {
        let (mut state, mut world) = populated_state();
        let buffer = state.console.clone();
        let lines = captured_lines(&buffer, || {
            assert!(execute_console_command(
                &mut state, &mut world, None, "validate"
            ));
        });
        assert!(lines.iter().any(|m| m.contains("0 violations")));

        // Now break an invariant: a non-finite position field.
        let hero = world
            .query::<(&Name, gens_ecs::ecs::entity::Entity)>()
            .iter(&world)
            .find(|(n, _)| n.0 == "Hero")
            .map(|(_, e)| e)
            .unwrap();
        world.get_mut::<Transform>(hero).unwrap().position = Vec3::new(f32::NAN, 0.0, 0.0);
        let lines = captured_lines(&buffer, || {
            assert!(execute_console_command(
                &mut state, &mut world, None, "validate"
            ));
        });
        assert!(lines.iter().any(|m| m.contains("violation")));
    }

    #[test]
    fn clear_empties_the_ring() {
        let (mut state, mut world) = populated_state();
        let buffer = state.console.clone();
        buffer.push(crate::console::ConsoleLine {
            time_seconds: 0.0,
            level: tracing::Level::INFO,
            target: "test".into(),
            message: "seed line".into(),
        });
        assert!(!buffer.is_empty());
        let _ = captured_lines(&buffer, || {
            assert!(execute_console_command(
                &mut state, &mut world, None, "clear"
            ));
        });
        assert!(buffer.is_empty(), "clear must empty the ring");
    }

    #[test]
    fn drain_executes_every_queued_command_in_order() {
        let (mut state, mut world) = populated_state();
        let buffer = state.console.clone();
        state.pending_commands = vec!["help".into(), "frobnicate".into(), "stats".into()];
        let _ = captured_lines(&buffer, || {
            drain_commands(&mut state, &mut world, None);
        });
        assert!(state.pending_commands.is_empty(), "queue must be drained");
    }

    /// A tiny world with one physics-backed entity (the play/undo test
    /// fixture).
    fn physics_world_with_crate() -> (World, gens_ecs::ecs::prelude::Entity) {
        let mut world = World::new();
        world.insert_resource(PhysicsResource {
            world: gens_physics::PhysicsWorld::new(gens_math::vec3(0.0, -9.81, 0.0)),
        });
        let (handle, _) = world.resource_mut::<PhysicsResource>().world.create_body(
            gens_physics::BodySpec::dynamic_at(gens_math::vec3(1.0, 4.0, -1.0)),
            gens_physics::ColliderSpec {
                shape: gens_physics::ColliderShape::Ball(0.45),
                ..Default::default()
            },
        );
        let entity = world
            .spawn((
                Name::new("Crate"),
                Active,
                Transform::from_position(gens_math::vec3(1.0, 4.0, -1.0)),
                crate::model::MeshSlot::cube([185, 130, 70, 255]),
                crate::model::AuthoredBody {
                    kind: gens_physics::BodyKind::Dynamic,
                    collider: gens_physics::ColliderShape::Ball(0.45),
                    ..AuthoredBody::default()
                },
                gens_physics::PhysicsBody { handle },
            ))
            .id();
        (world, entity)
    }

    #[test]
    fn play_stop_cycle_restores_the_pre_play_world() {
        let (mut state, mut world) = (EditorState::new(), World::new());
        let mut physics = gens_physics::PhysicsWorld::new(gens_math::vec3(0.0, -9.81, 0.0));
        let (handle, _) = physics.create_body(
            gens_physics::BodySpec::dynamic_at(gens_math::vec3(1.0, 4.0, -1.0)),
            gens_physics::ColliderSpec::default(),
        );
        world.insert_resource(PhysicsResource { world: physics });
        world.spawn((
            Name::new("Crate"),
            Active,
            Transform::from_position(gens_math::vec3(1.0, 4.0, -1.0)),
            gens_physics::PhysicsBody { handle },
        ));
        let before = playmode::capture(&world);

        // Play, mutate (the simulation ran; things fell), then Stop.
        execute_editor_command(&mut state, &mut world, &EditorCommand::Play);
        assert_eq!(state.play_mode, crate::shell::PlayMode::Playing);
        let crate_entity = world
            .iter_entities()
            .find(|e| world.get::<Name>(e.id()).map(|n| n.0.as_str()) == Some("Crate"))
            .map(|e| e.id())
            .unwrap();
        world.get_mut::<Transform>(crate_entity).unwrap().position =
            gens_math::vec3(40.0, 40.0, 40.0);
        execute_editor_command(&mut state, &mut world, &EditorCommand::Stop);
        assert_eq!(state.play_mode, crate::shell::PlayMode::Idle);
        let after = playmode::capture(&world);
        assert_eq!(
            before, after,
            "Stop must restore the pre-play world exactly"
        );
        assert!(state.play_snapshot.is_none(), "the snapshot is consumed");
        assert!(
            state.undo_stack.is_empty(),
            "session undo dies with the ids"
        );
    }

    #[test]
    fn create_then_undo_removes_the_object() {
        let (mut world, _) = physics_world_with_crate();
        let mut state = EditorState::new();
        let name = create_object(&mut state, &mut world, CreateKind::Cube);
        let created_entity = state.undo_stack.last().and_then(|op| match op {
            UndoOp::Create { entity } => Some(*entity),
            _ => None,
        });
        assert_eq!(
            state.selected, created_entity,
            "the new object is selected"
        );
        let created = world
            .iter_entities()
            .find(|e| {
                world
                    .get::<Name>(e.id())
                    .map(|n| n.0 == name)
                    .unwrap_or(false)
            })
            .map(|e| e.id())
            .expect("cube spawned");
        assert!(
            world.get::<crate::model::MeshSlot>(created).is_some(),
            "cube carries a mesh"
        );
        assert!(
            world.get::<gens_physics::PhysicsBody>(created).is_some(),
            "cube carries a live body"
        );

        apply_undo(&mut state, &mut world);
        assert!(
            world.iter_entities().all(|e| world
                .get::<Name>(e.id())
                .map(|n| n.0 != name)
                .unwrap_or(true)),
            "undo removes the created object"
        );
    }

    #[test]
    fn delete_then_undo_restores_the_entity_with_a_new_id() {
        let (mut world, _) = physics_world_with_crate();
        let mut state = EditorState::new();
        let victim = world
            .iter_entities()
            .find(|e| world.get::<Name>(e.id()).map(|n| n.0.as_str()) == Some("Crate"))
            .map(|e| e.id())
            .unwrap();
        state.selected = Some(victim);

        delete_selected(&mut state, &mut world);
        assert!(state.selected.is_none(), "selection cleared by delete");
        assert!(
            world.iter_entities().all(|e| e.id() != victim),
            "the victim is gone"
        );

        apply_undo(&mut state, &mut world);
        let restored = world
            .iter_entities()
            .find(|e| world.get::<Name>(e.id()).map(|n| n.0.as_str()) == Some("Crate"))
            .map(|e| e.id())
            .expect("entity restored");
        assert_ne!(restored, victim, "the restored id is new (documented)");
        assert!(
            world.get::<crate::model::MeshSlot>(restored).is_some(),
            "mesh survived the round-trip"
        );
        let body = world
            .get::<gens_physics::PhysicsBody>(restored)
            .copied()
            .expect("body re-created");
        let position = world
            .resource::<PhysicsResource>()
            .world
            .body_position(body.handle);
        assert!(
            (position - gens_math::vec3(1.0, 4.0, -1.0)).length() < 1e-4,
            "restored body sits at its authored position, got {position:?}"
        );
        assert_eq!(state.selected, Some(restored), "restoration is selected");
    }

    #[test]
    fn duplicate_records_undo_and_selects_the_copy() {
        let (mut world, _) = physics_world_with_crate();
        let mut state = EditorState::new();
        let original = world
            .iter_entities()
            .find(|e| world.get::<Name>(e.id()).map(|n| n.0.as_str()) == Some("Crate"))
            .map(|e| e.id())
            .unwrap();
        state.selected = Some(original);
        let name = duplicate_selected(&mut world, &mut state).expect("duplicate");
        assert_eq!(name, "Crate copy");
        assert_eq!(state.undo_stack.len(), 1);
        apply_undo(&mut state, &mut world);
        assert!(
            world.iter_entities().all(|e| world
                .get::<Name>(e.id())
                .map(|n| n.0 != name)
                .unwrap_or(true)),
            "undo removes the copy"
        );
    }
}
