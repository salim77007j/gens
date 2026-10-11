//! Hierarchy helpers and the standard engine schedules (Phase 1).

use crate::propagation::transform_propagation_system;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use gens_math::Mat4;

/// Attaches `children` under `parent` (maintains both `ChildOf` and the
/// parent's `Children` automatically via bevy relationships).
pub fn attach_children(world: &mut World, parent: Entity, children: &[Entity]) {
    world.entity_mut(parent).add_related::<ChildOf>(children);
}

/// Detaches `child` from its parent, if any (it becomes a root).
pub fn detach_from_parent(world: &mut World, child: Entity) {
    world.entity_mut(child).remove::<ChildOf>();
}

/// World-space position of `entity`'s parent, or the origin for roots.
pub fn parent_world_position(world: &World, entity: Entity) -> Mat4 {
    world
        .get::<ChildOf>(entity)
        .and_then(|c| world.get::<crate::components::WorldTransform>(c.parent()))
        .map(|wt| wt.0)
        .unwrap_or(Mat4::IDENTITY)
}

/// The Phase-1 executor: an `Update` schedule for engine/game systems and a
/// `PostUpdate` schedule that always ends with transform propagation.
///
/// A full multi-schedule app skeleton (fixed/render split, run conditions)
/// arrives with the renderer phase; this is the deterministic core the
/// Phase-1 example and tests use.
pub struct EngineSchedules {
    /// User/engine systems, run first.
    pub update: Schedule,
    /// Post-update; ends with [`transform_propagation_system`].
    pub post_update: Schedule,
}

impl Default for EngineSchedules {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineSchedules {
    /// Builds the standard schedules.
    pub fn new() -> EngineSchedules {
        let mut post_update = Schedule::default();
        post_update.add_systems(transform_propagation_system);
        EngineSchedules {
            update: Schedule::default(),
            post_update,
        }
    }

    /// Runs one full frame: `update`, then `post_update` (propagation).
    pub fn run_frame(&mut self, world: &mut World) {
        self.update.run(world);
        self.post_update.run(world);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Transform, WorldTransform};
    use gens_math::vec3;

    #[test]
    fn attach_and_detach_maintain_children() {
        let mut world = World::new();
        let parent = world.spawn_empty().id();
        let a = world.spawn_empty().id();
        let b = world.spawn_empty().id();
        attach_children(&mut world, parent, &[a, b]);

        assert_eq!(world.get::<Children>(parent).unwrap().to_vec(), vec![a, b]);
        assert_eq!(world.get::<ChildOf>(a).unwrap().parent(), parent);

        detach_from_parent(&mut world, a);
        assert_eq!(world.get::<Children>(parent).unwrap().to_vec(), vec![b]);
        assert!(world.get::<ChildOf>(a).is_none());
    }

    #[test]
    fn schedules_run_propagation_after_update() {
        let mut world = World::new();
        world.spawn(Transform::from_position(vec3(5.0, 0.0, 0.0)));
        let mut schedules = EngineSchedules::new();
        assert!(world.query::<&WorldTransform>().single(&world).is_err());

        schedules.run_frame(&mut world);
        let wt = world.query::<&WorldTransform>().single(&world).unwrap();
        assert_eq!(wt.position(), vec3(5.0, 0.0, 0.0));
    }

    #[test]
    fn parent_world_position_falls_back_to_identity() {
        let mut world = World::new();
        let e = world.spawn_empty().id();
        assert_eq!(parent_world_position(&world, e), Mat4::IDENTITY);
    }
}
