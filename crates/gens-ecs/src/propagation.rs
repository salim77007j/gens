//! Transform propagation (Phase 1).
//!
//! Exclusive system: computes [`WorldTransform`] for every entity with a
//! [`Transform`], composing parent-after-parent from the roots down:
//!
//! ```text
//! world(child) = world(parent) * local(child)
//! ```
//!
//! Phase-1 baseline recomputes the whole tree on every run in deterministic
//! DFS order (insertion order of `Children`). This is deliberately simple
//! and correct; change-driven partial propagation and parallel traversal are
//! Phase 2+ optimizations tracked in the renderer work (they are invisible
//! to API consumers because this system is the only writer of
//! `WorldTransform`).

use crate::components::{Transform, WorldTransform};
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::prelude::*;
use gens_math::Mat4;

/// Propagates local transforms into world transforms (see module docs).
///
/// Runs as an exclusive system, scheduled in `PostUpdate` by
/// [`EngineSchedules`](crate::EngineSchedules).
pub fn transform_propagation_system(world: &mut World) {
    // Root entities: parentless entities that carry a Transform or have
    // children (pure grouping nodes without either contribute nothing).
    // NOTE: bevy_ecs 0.20 worlds contain one internal seed entity (no
    // Transform, no Children) — the Or filter below excludes it.
    // Collected first so the query borrow ends before mutation begins.
    let mut roots_query =
        world.query_filtered::<Entity, (Without<ChildOf>, Or<(With<Transform>, With<Children>)>)>();
    let roots: Vec<Entity> = roots_query.iter(world).collect();

    for root in roots {
        propagate_subtree(world, root, Mat4::IDENTITY);
    }
}

/// Depth-first composition. A missing `Transform` behaves as identity, so
/// pure grouping nodes pass the parent transform through unchanged; entities
/// with a `Transform` but no `WorldTransform` component yet get one inserted.
fn propagate_subtree(world: &mut World, entity: Entity, parent_world: Mat4) {
    // Transform-bearing entities compose and receive a WorldTransform;
    // grouping nodes (no Transform) pass the parent matrix through
    // unchanged and receive nothing themselves.
    let world_matrix = if let Some(transform) = world.get::<Transform>(entity) {
        let m = parent_world * transform.to_matrix();
        match world.get_mut::<WorldTransform>(entity) {
            Some(mut wt) => wt.0 = m,
            None => {
                world.entity_mut(entity).insert(WorldTransform(m));
            }
        }
        m
    } else {
        parent_world
    };

    let children: Vec<Entity> = world
        .get::<Children>(entity)
        .map(|c| c.to_vec())
        .unwrap_or_default();
    for child in children {
        propagate_subtree(world, child, world_matrix);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Name;
    use gens_math::{Quat, deg_to_rad, eq_eps, vec3};

    fn run(world: &mut World) {
        transform_propagation_system(world);
    }

    #[test]
    fn root_world_transform_equals_local() {
        let mut world = World::new();
        let local = Transform::from_position(vec3(3.0, 4.0, 5.0));
        world.spawn((local, Name::new("root")));
        run(&mut world);
        let wt = world
            .query::<&WorldTransform>()
            .single(&world)
            .expect("one entity");
        assert_eq!(*wt, WorldTransform(local.to_matrix()));
    }

    #[test]
    fn child_composes_parent_times_local() {
        let mut world = World::new();
        // Parent at (1,0,0) rotated 90 deg around Y; child 1 unit forward.
        let parent = world
            .spawn((Transform::from_position(vec3(1.0, 0.0, 0.0))
                .with_rotation(Quat::from_rotation_y(deg_to_rad(90.0))),))
            .id();
        let child = world
            .spawn(Transform::from_position(vec3(0.0, 0.0, 1.0)))
            .id();
        world.entity_mut(parent).add_related::<ChildOf>(&[child]);
        run(&mut world);

        // child local (0,0,1) rotated 90Y -> (1,0,0), then + parent (1,0,0).
        let child_pos = world
            .get::<WorldTransform>(child)
            .expect("propagated")
            .position();
        assert!(eq_eps(child_pos.x, 2.0) && eq_eps(child_pos.y, 0.0) && eq_eps(child_pos.z, 0.0));
    }

    #[test]
    fn three_levels_compose_in_order() {
        let mut world = World::new();
        let gp = world
            .spawn(Transform::from_position(vec3(1.0, 0.0, 0.0)))
            .id();
        let p = world
            .spawn(Transform::from_position(vec3(1.0, 0.0, 0.0)))
            .id();
        let c = world
            .spawn(Transform::from_position(vec3(1.0, 0.0, 0.0)))
            .id();
        world.entity_mut(gp).add_related::<ChildOf>(&[p]);
        world.entity_mut(p).add_related::<ChildOf>(&[c]);
        run(&mut world);
        assert_eq!(
            world.get::<WorldTransform>(c).unwrap().position(),
            vec3(3.0, 0.0, 0.0)
        );
        assert_eq!(
            world.get::<WorldTransform>(p).unwrap().position(),
            vec3(2.0, 0.0, 0.0)
        );
        assert_eq!(
            world.get::<WorldTransform>(gp).unwrap().position(),
            vec3(1.0, 0.0, 0.0)
        );
    }

    #[test]
    fn moving_parent_repropagates_children_next_run() {
        let mut world = World::new();
        let parent = world.spawn(Transform::IDENTITY).id();
        let child = world
            .spawn(Transform::from_position(vec3(1.0, 0.0, 0.0)))
            .id();
        world.entity_mut(parent).add_related::<ChildOf>(&[child]);
        run(&mut world);
        assert_eq!(
            world.get::<WorldTransform>(child).unwrap().position(),
            vec3(1.0, 0.0, 0.0)
        );

        world.get_mut::<Transform>(parent).unwrap().position = vec3(10.0, 0.0, 0.0);
        run(&mut world);
        assert_eq!(
            world.get::<WorldTransform>(child).unwrap().position(),
            vec3(11.0, 0.0, 0.0)
        );
    }

    #[test]
    fn entities_without_transform_are_traversed_but_identity() {
        let mut world = World::new();
        // A pure grouping node with no Transform passes identity to children.
        let group = world.spawn(Name::new("group")).id();
        let child = world
            .spawn(Transform::from_position(vec3(2.0, 0.0, 0.0)))
            .id();
        world.entity_mut(group).add_related::<ChildOf>(&[child]);
        run(&mut world);
        assert_eq!(
            world.get::<WorldTransform>(child).unwrap().position(),
            vec3(2.0, 0.0, 0.0)
        );
    }

    #[test]
    fn deep_hierarchy_in_one_run() {
        let mut world = World::new();
        let mut chain = Vec::new();
        let mut parent: Option<Entity> = None;
        for _ in 0..50 {
            let e = world
                .spawn(Transform::from_position(vec3(1.0, 0.0, 0.0)))
                .id();
            if let Some(p) = parent {
                world.entity_mut(p).add_related::<ChildOf>(&[e]);
            }
            chain.push(e);
            parent = Some(e);
        }
        run(&mut world);
        let last = *chain.last().unwrap();
        assert_eq!(
            world.get::<WorldTransform>(last).unwrap().position(),
            vec3(50.0, 0.0, 0.0)
        );
    }
}
