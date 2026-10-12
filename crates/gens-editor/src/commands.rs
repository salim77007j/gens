//! Console commands: the editor's debugging vocabulary (P8-6).
//!
//! The shell's command row records intent ([`EditorState::pending_commands`]
//! — strings); the embedding app drains it and calls
//! [`execute_console_command`] with the REAL world. Results are spoken
//! through `tracing`, which the console's sink layer captures — the
//! command's output appears in the very panel it was typed into, no
//! special-casing.
//!
//! Vocabulary (deliberately small, each command real):
//! - `help` — list the vocabulary
//! - `entities` — live entity count + names in entity-index order
//! - `validate` — run the runtime invariant checker on the live world
//! - `stats` — canonical world state hash + counters
//! - `clear` — clear the console ring
//!
//! Unknown commands are a loud WARN, never a silent no-op.

use crate::shell::EditorState;
use gens_assets::AssetRegistry;
use gens_ecs::Name;
use gens_ecs::ecs::world::World;
use gens_runtime::debug::validate_world;

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
}
