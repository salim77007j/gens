//! ECS integration: physics components, stepping, and transform sync.
//!
//! [`PhysicsWorld`] is stored as a bevy resource; [`PhysicsBody`] links an
//! entity to a body handle; [`sync_transforms_from_physics`] copies dynamic
//! and kinematic body poses into the standard
//! [`gens_ecs::components::Transform`] so the Phase 2 renderer consumes
//! physics results without any renderer changes. `WorldTransform` propagation
//! (Phase 1) then works on physics-driven entities unchanged.
//!
//! Fixed bodies are skipped by sync: their `Transform` is authored, and the
//! solver never moves them.

use crate::backend::prelude::RigidBodyType;
use crate::world::{BodyKind, PhysicsWorld};
use gens_ecs::components::Transform;
use gens_ecs::ecs::prelude::*;

/// Links an entity to a rigid body in the [`PhysicsWorld`] resource.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicsBody {
    /// Handle returned by [`PhysicsWorld::create_body`].
    pub handle: crate::world::BodyHandle,
}

/// ECS resource wrapper around the physics world.
///
/// bevy 0.16+ requires resources to also be components, which cannot be
/// retro-implanted on the facade type from a distance — so the resource is
/// this deliberate newtype. Systems take `ResMut<PhysicsResource>`.
#[derive(Resource)]
pub struct PhysicsResource {
    /// The simulation state.
    pub world: PhysicsWorld,
}

/// Steps the simulation once by the fixed timestep.
///
/// Add to a fixed-timestep schedule (or call once per simulation step from
/// the game loop, as `physics_hello` does) — never once per rendered frame.
pub fn step_physics_system(mut physics: ResMut<PhysicsResource>) {
    physics.world.step();
}

/// Copies body poses into [`Transform`] for dynamic and kinematic bodies.
///
/// Scale is untouched (physics is rigid); rotation and position come from
/// the solver. Fixed bodies keep their authored transforms.
pub fn sync_transforms_from_physics(
    bodies: Query<(&PhysicsBody, &mut Transform)>,
    physics: Res<PhysicsResource>,
) {
    let physics = &physics.world;
    for (body, mut transform) in bodies {
        if physics.body_kind(body.handle) == BodyKind::Fixed {
            continue;
        }
        transform.position = physics.body_position(body.handle);
        transform.rotation = physics.body_rotation(body.handle);
    }
}

/// True when a body of this kind is integrated by the solver each step.
#[must_use]
pub fn solver_driven(kind: BodyKind, rapier_type: RigidBodyType) -> bool {
    match kind {
        BodyKind::Fixed => false,
        BodyKind::Dynamic => rapier_type == RigidBodyType::Dynamic,
        BodyKind::KinematicPosition => rapier_type == RigidBodyType::KinematicPositionBased,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::BodySpec;
    use gens_ecs::ecs::world::World;
    use gens_math::Vec3;

    /// One schedule run = one physics step + one transform sync, the exact
    /// pattern gameplay code uses per simulation step.
    fn step_and_sync(schedule: &mut Schedule, world: &mut World) {
        schedule.run(world);
    }

    #[test]
    fn dynamic_body_drives_transform_fixed_body_does_not() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new(Vec3::new(0.0, -9.81, 0.0));
        // Floor: fixed. Ball: dynamic, dropped from y = 4.
        let (floor_body, _) = physics.create_body(
            BodySpec::fixed_at(Vec3::new(0.0, -0.5, 0.0)),
            crate::world::ColliderSpec {
                shape: crate::world::ColliderShape::Cuboid(Vec3::new(10.0, 0.5, 10.0)),
                ..Default::default()
            },
        );
        let (ball_body, _) = physics.create_body(
            BodySpec::dynamic_at(Vec3::new(0.0, 4.0, 0.0)),
            crate::world::ColliderSpec {
                shape: crate::world::ColliderShape::Ball(0.5),
                ..Default::default()
            },
        );
        world.insert_resource(PhysicsResource { world: physics });

        let floor = world
            .spawn((
                PhysicsBody { handle: floor_body },
                Transform::from_position(Vec3::new(0.0, -0.5, 0.0)),
            ))
            .id();
        let ball = world
            .spawn((
                PhysicsBody { handle: ball_body },
                Transform::from_position(Vec3::new(0.0, 4.0, 0.0)),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems((step_physics_system, sync_transforms_from_physics));

        let y_before = world.get::<Transform>(ball).unwrap().position.y;
        for _ in 0..60 {
            step_and_sync(&mut schedule, &mut world);
        }

        // Dynamic ball fell the full 4 m and came to rest on the floor:
        // floor top is y = 0, ball radius 0.5, so the resting center is
        // y ≈ 0.5 (the solver settles a hair below, ~0.494).
        let ball_y = world.get::<Transform>(ball).unwrap().position.y;
        assert!(
            ball_y < y_before - 3.0,
            "ball must fall under gravity: {y_before} -> {ball_y}"
        );
        assert!(
            (ball_y - 0.5).abs() < 0.15,
            "ball must rest on the floor at y≈0.5, got {ball_y}"
        );
        // Fixed floor keeps its authored transform exactly.
        let floor_t = world.get::<Transform>(floor).unwrap();
        assert_eq!(floor_t.position, Vec3::new(0.0, -0.5, 0.0));
    }

    #[test]
    fn sync_respects_scale_and_only_overwrites_pose() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new(Vec3::new(0.0, -9.81, 0.0));
        let (handle, _) = physics.create_body(
            BodySpec::dynamic_at(Vec3::new(0.0, 2.0, 0.0)),
            crate::world::ColliderSpec {
                shape: crate::world::ColliderShape::Ball(0.5),
                ..Default::default()
            },
        );
        world.insert_resource(PhysicsResource { world: physics });
        world.spawn((
            PhysicsBody { handle },
            Transform {
                position: Vec3::new(0.0, 2.0, 0.0),
                rotation: gens_math::Quat::IDENTITY,
                scale: Vec3::new(2.0, 3.0, 4.0),
            },
        ));

        let mut schedule = Schedule::default();
        schedule.add_systems((step_physics_system, sync_transforms_from_physics));
        schedule.run(&mut world);
        schedule.run(&mut world);

        let t = world.query::<&Transform>().single(&world).unwrap();
        assert_eq!(t.scale, Vec3::new(2.0, 3.0, 4.0), "scale must survive sync");
    }

    #[test]
    fn solver_driven_classification() {
        assert!(solver_driven(BodyKind::Dynamic, RigidBodyType::Dynamic));
        assert!(!solver_driven(BodyKind::Fixed, RigidBodyType::Fixed));
        assert!(solver_driven(
            BodyKind::KinematicPosition,
            RigidBodyType::KinematicPositionBased
        ));
        // A mismatch must never claim solver control.
        assert!(!solver_driven(BodyKind::Fixed, RigidBodyType::Dynamic));
    }
}
