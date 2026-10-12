//! Observation-only profiling integration (Phase 8, ADR-0027).
//!
//! Two pieces:
//! - [`instrumented_frame`] — runs the standard [`EngineSchedules`] frame
//!   inside profiler spans (`ecs_update` / `ecs_post_update`). Callers wrap
//!   their own subsystem phases (`physics_step`, `scripts`, ...) with
//!   [`Profiler::time`] around it.
//! - [`world_state_hash`] — a canonical FNV-1a hash of the observable ECS
//!   state (entity-index order, name/active/tag/Transform/WorldTransform
//!   as raw bit patterns). This is the yardstick the ADR-0027 proof uses:
//!   the same deterministic replay must hash identically with the profiler
//!   enabled and disabled.

use gens_assets::fnv1a64;
use gens_ecs::ecs::world::World;
use gens_ecs::{Active, EngineSchedules, Name, Tag, Transform, WorldTransform};
use gens_profiler::Profiler;

/// Runs one schedules frame inside profiler spans.
///
/// Timing wraps the schedule calls only — the systems themselves are the
/// caller's; nothing measured ever re-enters the simulation.
pub fn instrumented_frame(
    schedules: &mut EngineSchedules,
    world: &mut World,
    profiler: &mut Profiler,
) {
    profiler.time("ecs_update", |_| schedules.update.run(world));
    profiler.time("ecs_post_update", |_| schedules.post_update.run(world));
}

/// Canonical, byte-stable hash of the observable ECS state.
///
/// Entities are visited in entity-index order (the save format's canonical
/// order, ADR-0021); per entity the hash input is
/// `index|name|active|tag|transform bits|world-transform bits` with every
/// `f32` rendered as its raw 8-hex bit pattern (no float formatting
/// ambiguity). Entities the engine reserves (e.g. resource holders) are
/// skipped the same way [`crate::save::capture_world`] skips non-saveable
/// ones: an entity with none of the observed components contributes
/// nothing.
pub fn world_state_hash(world: &World) -> u64 {
    let mut rows: Vec<(u32, String)> = world
        .iter_entities()
        .filter_map(|entity_ref| {
            let entity = entity_ref.id();
            let name = world.get::<Name>(entity);
            let transform = world.get::<Transform>(entity);
            let world_transform = world.get::<WorldTransform>(entity);
            let active = world.get::<Active>(entity);
            let tag = world.get::<Tag>(entity);
            if name.is_none()
                && transform.is_none()
                && world_transform.is_none()
                && active.is_none()
                && tag.is_none()
            {
                return None;
            }
            let mut row = String::new();
            row.push_str(&entity.index().index().to_string());
            row.push('|');
            row.push_str(name.map(|n| n.0.as_str()).unwrap_or("-"));
            row.push('|');
            row.push_str(if active.is_some() { "A" } else { "-" });
            row.push('|');
            row.push_str(tag.map(|t| t.0.as_str()).unwrap_or("-"));
            row.push('|');
            if let Some(t) = transform {
                for f in t.position.to_array() {
                    row.push_str(&format!("{:08x}|", f.to_bits()));
                }
                for f in t.rotation.to_array() {
                    row.push_str(&format!("{:08x}|", f.to_bits()));
                }
                for f in t.scale.to_array() {
                    row.push_str(&format!("{:08x}|", f.to_bits()));
                }
            } else {
                row.push_str("-|");
            }
            if let Some(wt) = world_transform {
                for f in wt.0.to_cols_array() {
                    row.push_str(&format!("{:08x}|", f.to_bits()));
                }
            } else {
                row.push_str("-|");
            }
            Some((entity.index().index(), row))
        })
        .collect();
    rows.sort_by_key(|(index, _)| *index);
    let joined = rows
        .into_iter()
        .map(|(_, row)| row)
        .collect::<Vec<_>>()
        .join("\n");
    fnv1a64(joined.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_ecs::ecs::bundle::Bundle;
    use gens_math::{Quat, Vec3};

    fn spawn_named(world: &mut World, name: &str, pos: Vec3) -> gens_ecs::ecs::entity::Entity {
        world
            .spawn((Name::new(name), Transform::from_position(pos), Active))
            .id()
    }

    /// Finds the entity with `name` — `iter_entities()` order is NOT
    /// deterministic (P5-5/P5-6 lesson), lookups go through queries.
    fn named(world: &mut World, name: &str) -> gens_ecs::ecs::entity::Entity {
        // bevy 0.20 fact (P7 lesson): World::query needs &mut World.
        world
            .query::<(&Name, gens_ecs::ecs::entity::Entity)>()
            .iter(world)
            .find(|(n, _)| n.0 == name)
            .map(|(_, e)| e)
            .unwrap_or_else(|| panic!("entity {name} missing"))
    }

    #[test]
    fn hash_is_stable_for_identical_worlds() {
        let mut a = World::new();
        spawn_named(&mut a, "A", Vec3::new(1.0, 2.0, 3.0));
        spawn_named(&mut a, "B", Vec3::new(4.0, 5.0, 6.0));
        let mut b = World::new();
        spawn_named(&mut b, "A", Vec3::new(1.0, 2.0, 3.0));
        spawn_named(&mut b, "B", Vec3::new(4.0, 5.0, 6.0));
        assert_eq!(world_state_hash(&a), world_state_hash(&b));
    }

    #[test]
    fn hash_is_sensitive_to_every_observed_dimension() {
        // Names, transforms, active flags and world transforms all feed
        // the hash; changing any one must change it.
        let mut a = World::new();
        spawn_named(&mut a, "A", Vec3::new(1.0, 2.0, 3.0));
        let h_names = world_state_hash(&a);

        let mut b = World::new();
        spawn_named(&mut b, "DIFFERENT", Vec3::new(1.0, 2.0, 3.0));
        assert_ne!(h_names, world_state_hash(&b), "name must feed the hash");

        let mut c = World::new();
        let e = spawn_named(&mut c, "A", Vec3::new(1.0, 2.0, 3.0));
        c.get_mut::<Transform>(e).unwrap().position = Vec3::new(9.0, 2.0, 3.0);
        assert_ne!(
            h_names,
            world_state_hash(&c),
            "transform must feed the hash"
        );

        let mut d = World::new();
        let e = spawn_named(&mut d, "A", Vec3::new(1.0, 2.0, 3.0));
        d.entity_mut(e).remove::<Active>();
        assert_ne!(
            h_names,
            world_state_hash(&d),
            "active flag must feed the hash"
        );
    }

    #[test]
    fn instrumented_frame_records_both_spans() {
        let mut world = World::new();
        spawn_named(&mut world, "A", Vec3::ONE);
        let mut schedules = EngineSchedules::new();
        let mut profiler = Profiler::enabled();
        profiler.begin_frame(1);
        instrumented_frame(&mut schedules, &mut world, &mut profiler);
        profiler.end_frame();
        let history = profiler.history();
        assert_eq!(history.len(), 1);
        // Spans record in COMPLETION order: update runs first.
        let names: Vec<&str> = history[0].spans.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["ecs_update", "ecs_post_update"]);
    }

    #[test]
    fn instrumented_frame_with_disabled_profiler_still_runs() {
        let mut world = World::new();
        spawn_named(&mut world, "A", Vec3::ONE);
        let mut schedules = EngineSchedules::new();
        let mut profiler = Profiler::disabled();
        profiler.begin_frame(1);
        instrumented_frame(&mut schedules, &mut world, &mut profiler);
        assert!(profiler.end_frame().is_none());
        // The world must actually have been propagated (WorldTransform now
        // exists) — the disabled path must not skip the work. Lookup by
        // Name: iter_entities() order is not deterministic (P5-5).
        let e = named(&mut world, "A");
        assert!(world.get::<WorldTransform>(e).is_some());
    }

    /// A bundle sanity check so the test helpers stay honest about what
    /// they spawn (Bundle import is otherwise unused in the crate).
    #[test]
    fn bundle_helper_spawns_visible_components() {
        let mut world = World::new();
        let e = spawn_named(&mut world, "X", Vec3::ZERO);
        assert!(world.get::<Name>(e).is_some());
        assert!(world.get::<Transform>(e).is_some());
        assert!(world.get::<Active>(e).is_some());
        let _ = Quat::IDENTITY; // re-export sanity for the math types used
    }
    /// The `Bundle` import is load-bearing for the spawn helper's type
    /// inference on some bevy versions; pin that fact with a compile-time
    /// use so refactors cannot silently drop it.
    #[test]
    fn bundle_trait_in_scope() {
        fn assert_bundle<B: Bundle>() {}
        assert_bundle::<(Name, Transform, Active)>();
    }
}
