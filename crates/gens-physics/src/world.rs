//! The physics world facade over rapier3d's `PhysicsWorld` bundle (ADR-0013).
//!
//! Simulation advances in fixed steps ([`PhysicsWorld::timestep`], default
//! 1/60 s) driven explicitly by [`PhysicsWorld::step`] — never by wall time —
//! so results are reproducible in tests, replays, and CI on every platform.

use gens_math::{Quat, Vec3};

use crate::backend::prelude as rp;
use crate::convert;

/// Opaque handle to a rigid body inside a [`PhysicsWorld`].
///
/// Valid only for the world that issued it; Copy + `Eq` so it can live in
/// ECS components and maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BodyHandle(pub(crate) rp::RigidBodyHandle);

/// Opaque handle to a collider inside a [`PhysicsWorld`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColliderHandle(pub(crate) rp::ColliderHandle);

/// How a body participates in the simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BodyKind {
    /// Simulated: gravity, contacts and impulses move it.
    #[default]
    Dynamic,
    /// Immovable: never moved by the solver (floors, walls).
    Fixed,
    /// Moved by setting its position directly (platforms, doors).
    KinematicPosition,
}

/// Description of a rigid body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodySpec {
    /// Simulation role.
    pub kind: BodyKind,
    /// Initial world-space position.
    pub position: Vec3,
    /// Initial linear velocity.
    pub linvel: Vec3,
    /// Whether the solver may put the body to sleep when at rest
    /// (leave `true` for performance; `false` for stable continuous tests).
    pub can_sleep: bool,
    /// Continuous collision detection against fast tunneling.
    pub ccd: bool,
    /// Linear velocity damping (0 = none; 1 ≈ strong drag). Non-negative.
    pub linear_damping: f32,
}

impl Default for BodySpec {
    fn default() -> Self {
        BodySpec {
            kind: BodyKind::Dynamic,
            position: Vec3::ZERO,
            linvel: Vec3::ZERO,
            can_sleep: true,
            ccd: false,
            linear_damping: 0.0,
        }
    }
}

impl BodySpec {
    /// A dynamic body at `position` (the common gameplay case).
    #[must_use]
    pub const fn dynamic_at(position: Vec3) -> BodySpec {
        BodySpec {
            kind: BodyKind::Dynamic,
            position,
            linvel: Vec3::ZERO,
            can_sleep: true,
            ccd: false,
            linear_damping: 0.0,
        }
    }

    /// A fixed (immovable) body at `position`.
    #[must_use]
    pub const fn fixed_at(position: Vec3) -> BodySpec {
        BodySpec {
            kind: BodyKind::Fixed,
            position,
            linvel: Vec3::ZERO,
            can_sleep: false,
            ccd: false,
            linear_damping: 0.0,
        }
    }
}

/// Primitive collider shapes exposed by the facade.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColliderShape {
    /// Sphere with the given radius.
    Ball(f32),
    /// Axis-aligned box with the given half extents.
    Cuboid(Vec3),
}

/// Description of a collider attached to a body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColliderSpec {
    /// Shape geometry.
    pub shape: ColliderShape,
    /// Contact friction in `0..=1`.
    pub friction: f32,
    /// Bounciness in `0..=1`.
    pub restitution: f32,
    /// Mass density (mass derives from shape volume × density).
    pub density: f32,
}

impl Default for ColliderSpec {
    fn default() -> Self {
        ColliderSpec {
            shape: ColliderShape::Ball(0.5),
            friction: 0.5,
            restitution: 0.0,
            density: 1.0,
        }
    }
}

/// The rigid-body simulation (rapier3d 0.36 behind the facade, ADR-0013).
///
/// Stored as an ECS resource via [`crate::ecs::PhysicsResource`]. Rapier's
/// bundled world type does not implement `Debug`, so this facade skips the
/// derive on purpose; state inspection goes through the readback methods.
pub struct PhysicsWorld {
    world: rp::PhysicsWorld,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new(Vec3::new(0.0, -9.81, 0.0))
    }
}

impl PhysicsWorld {
    /// Default fixed timestep: 60 simulation steps per second.
    pub const DEFAULT_DT: f32 = 1.0 / 60.0;

    /// A world with `gravity` (e.g. `Vec3::new(0.0, -9.81, 0.0)`) and the
    /// default fixed timestep.
    #[must_use]
    pub fn new(gravity: Vec3) -> PhysicsWorld {
        let world = rp::PhysicsWorld {
            gravity: convert::to_rapier_vec(gravity),
            integration_parameters: rp::IntegrationParameters {
                dt: Self::DEFAULT_DT,
                ..Default::default()
            },
            ..Default::default()
        };
        PhysicsWorld { world }
    }

    /// Sets the fixed timestep used by every [`PhysicsWorld::step`].
    ///
    /// # Panics
    /// Panics when `dt` is zero or negative: the loop contract is fixed,
    /// positive steps (see `gens-core` game loop, Phase 1).
    pub fn set_timestep(&mut self, dt: f32) {
        assert!(dt > 0.0, "physics timestep must be positive, got {dt}");
        self.world.integration_parameters.dt = dt;
    }

    /// The fixed timestep in seconds.
    #[must_use]
    pub fn timestep(&self) -> f32 {
        self.world.integration_parameters.dt
    }

    /// Creates a body with an attached collider and returns both handles.
    pub fn create_body(
        &mut self,
        body: BodySpec,
        collider: ColliderSpec,
    ) -> (BodyHandle, ColliderHandle) {
        let mut rb = match body.kind {
            BodyKind::Dynamic => rp::RigidBodyBuilder::dynamic(),
            BodyKind::Fixed => rp::RigidBodyBuilder::fixed(),
            BodyKind::KinematicPosition => rp::RigidBodyBuilder::kinematic_position_based(),
        };
        rb = rb
            .translation(convert::to_rapier_vec(body.position))
            .linvel(convert::to_rapier_vec(body.linvel))
            .can_sleep(body.can_sleep)
            .ccd_enabled(body.ccd)
            .linear_damping(body.linear_damping.max(0.0));

        let mut col = match collider.shape {
            ColliderShape::Ball(radius) => rp::ColliderBuilder::ball(radius),
            ColliderShape::Cuboid(half) => rp::ColliderBuilder::cuboid(half.x, half.y, half.z),
        };
        col = col
            .friction(collider.friction)
            .restitution(collider.restitution)
            .density(collider.density);

        let (rb_handle, col_handle) = self.world.insert(rb, col);
        (BodyHandle(rb_handle), ColliderHandle(col_handle))
    }

    /// Advances the simulation by exactly one fixed timestep.
    pub fn step(&mut self) {
        self.world.step();
    }

    /// World-space position of the body's center of mass frame.
    #[must_use]
    pub fn body_position(&self, handle: BodyHandle) -> Vec3 {
        convert::to_glam_vec(self.world.bodies[handle.0].translation())
    }

    /// World-space rotation of the body.
    #[must_use]
    pub fn body_rotation(&self, handle: BodyHandle) -> Quat {
        convert::to_glam_quat(*self.world.bodies[handle.0].rotation())
    }

    /// Linear velocity of the body.
    #[must_use]
    pub fn body_velocity(&self, handle: BodyHandle) -> Vec3 {
        convert::to_glam_vec(self.world.bodies[handle.0].linvel())
    }

    /// Overwrites the body's linear velocity (wakes the body up).
    pub fn set_body_velocity(&mut self, handle: BodyHandle, velocity: Vec3) {
        self.world.bodies[handle.0].set_linvel(convert::to_rapier_vec(velocity), true);
    }

    /// Applies a linear impulse (kg·m/s) to the body at its center of mass.
    pub fn apply_impulse(&mut self, handle: BodyHandle, impulse: Vec3) {
        self.world.bodies[handle.0].apply_impulse(convert::to_rapier_vec(impulse), true);
    }

    /// Teleports a kinematic or dynamic body to `position` (wakes it up).
    pub fn set_body_position(&mut self, handle: BodyHandle, position: Vec3) {
        self.world.bodies[handle.0].set_translation(convert::to_rapier_vec(position), true);
    }

    /// The body's simulation role.
    ///
    /// Rapier's two kinematic flavors both read back as
    /// [`BodyKind::KinematicPosition`] — the facade only creates the
    /// position-based flavor.
    #[must_use]
    pub fn body_kind(&self, handle: BodyHandle) -> BodyKind {
        use rp::RigidBodyType;
        match self.world.bodies[handle.0].body_type() {
            RigidBodyType::Dynamic => BodyKind::Dynamic,
            RigidBodyType::Fixed => BodyKind::Fixed,
            RigidBodyType::KinematicPositionBased | RigidBodyType::KinematicVelocityBased => {
                BodyKind::KinematicPosition
            }
            // Soft-body frames are a rapier-internal construct outside the
            // facade's scope; they are never solver-driven gameplay bodies.
            RigidBodyType::SoftFrame => BodyKind::Fixed,
        }
    }

    /// Number of rigid bodies in the world.
    #[must_use]
    pub fn body_count(&self) -> usize {
        self.world.bodies.len()
    }

    /// Whether `handle` refers to a live body in this world.
    ///
    /// The safe predicate for debugging/validation code: the positional
    /// accessors index the body set and panic on a stale handle, so
    /// anything that cannot trust its handles must ask first (Phase 8
    /// `validate_world` does exactly that).
    #[must_use]
    pub fn body_exists(&self, handle: BodyHandle) -> bool {
        self.world.bodies.get(handle.0).is_some()
    }

    /// Gravity vector (readback for tests and tooling).
    #[must_use]
    pub fn gravity(&self) -> Vec3 {
        convert::to_glam_vec(self.world.gravity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_math::eq_eps;

    const G: f32 = 9.81;

    fn gravity_world() -> PhysicsWorld {
        PhysicsWorld::new(Vec3::new(0.0, -G, 0.0))
    }

    fn floor(world: &mut PhysicsWorld) -> BodyHandle {
        let (handle, _) = world.create_body(
            BodySpec::fixed_at(Vec3::new(0.0, -0.5, 0.0)),
            ColliderSpec {
                shape: ColliderShape::Cuboid(Vec3::new(20.0, 0.5, 20.0)),
                friction: 0.9,
                ..Default::default()
            },
        );
        handle
    }

    #[test]
    fn free_fall_matches_newtonian_kinematics() {
        // The contract is physics, not rapier's exact integration scheme
        // (0.36 uses a higher-order integrator; pinning its coefficients
        // would couple us to internals). After t seconds from rest:
        // velocity must be exactly g·t and position within 5 cm of the
        // closed-form ½g·t² free-fall distance.
        let mut world = gravity_world();
        let (ball, _) = world.create_body(
            BodySpec::dynamic_at(Vec3::new(0.0, 10.0, 0.0)),
            ColliderSpec {
                shape: ColliderShape::Ball(0.25),
                ..Default::default()
            },
        );
        let dt = world.timestep();
        let steps = 60;
        let t = dt * steps as f32;
        for _ in 0..steps {
            world.step();
        }
        let y = world.body_position(ball).y;
        let expected = 10.0 - 0.5 * G * t * t;
        assert!(
            (y - expected).abs() < 0.05,
            "free-fall y {y} must match ½g·t² = {expected}"
        );
        let vy = world.body_velocity(ball).y;
        assert!(
            eq_eps(vy, -G * t),
            "velocity must be exactly g·t: {vy} vs {}",
            -G * t
        );
    }

    #[test]
    fn ball_rests_on_floor() {
        let mut world = gravity_world();
        let _floor = floor(&mut world);
        let (ball, _) = world.create_body(
            BodySpec::dynamic_at(Vec3::new(0.0, 5.0, 0.0)),
            ColliderSpec {
                shape: ColliderShape::Ball(0.5),
                ..Default::default()
            },
        );
        for _ in 0..300 {
            world.step();
        }
        // Floor top surface is y = 0; the ball (r = 0.5) rests at y = 0.5.
        let y = world.body_position(ball).y;
        assert!(
            (y - 0.5).abs() < 0.02,
            "ball must come to rest at y≈0.5, got {y}"
        );
        let speed = world.body_velocity(ball).length();
        assert!(speed < 1e-2, "resting ball must have ~zero speed: {speed}");
    }

    #[test]
    fn same_process_runs_are_bit_identical() {
        // Determinism contract (R-013, in-process scope): two identical
        // simulations must produce bit-identical trajectories.
        let mut a = gravity_world();
        let mut b = gravity_world();
        let _floor_a = floor(&mut a);
        let _floor_b = floor(&mut b);
        let spec = BodySpec {
            linear_damping: 0.4,
            ..BodySpec::dynamic_at(Vec3::new(0.0, 5.0, 0.0))
        };
        let (ball_a, _) = a.create_body(spec, ColliderSpec::default());
        let (ball_b, _) = b.create_body(spec, ColliderSpec::default());

        for step in 0..600 {
            if step % 90 == 0 {
                let impulse = Vec3::new(0.37, 0.11, -0.53);
                a.apply_impulse(ball_a, impulse);
                b.apply_impulse(ball_b, impulse);
            }
            a.step();
            b.step();
        }

        assert_eq!(
            a.body_position(ball_a),
            b.body_position(ball_b),
            "identical inputs must yield bit-identical positions"
        );
        assert_eq!(
            a.body_velocity(ball_a),
            b.body_velocity(ball_b),
            "identical inputs must yield bit-identical velocities"
        );
    }

    #[test]
    fn impulse_changes_velocity_and_respects_mass() {
        let mut world = gravity_world();
        // Density 1: a unit-diameter ball has mass m = ρ·(4/3)πr³ ≈ 0.5236.
        let (light, _) = world.create_body(
            BodySpec::dynamic_at(Vec3::ZERO),
            ColliderSpec {
                shape: ColliderShape::Ball(0.5),
                density: 1.0,
                ..Default::default()
            },
        );
        let before = world.body_velocity(light);
        world.apply_impulse(light, Vec3::new(1.0, 0.0, 0.0));
        let after = world.body_velocity(light);
        let dv = (after - before).x;
        // v' = v + J/m → dv = 1.0 / 0.5236 ≈ 1.9099.
        assert!(
            (dv - 1.0 / (1.0 * std::f32::consts::PI * 4.0 / 3.0 * 0.125)).abs() < 1e-2,
            "impulse must convert to velocity by 1/m, got dv={dv}"
        );
    }

    #[test]
    fn kinematic_body_teleports_and_reads_back_kind() {
        let mut world = gravity_world();
        let (kin, _) = world.create_body(
            BodySpec {
                kind: BodyKind::KinematicPosition,
                position: Vec3::ZERO,
                ..Default::default()
            },
            ColliderSpec {
                shape: ColliderShape::Cuboid(Vec3::new(1.0, 1.0, 1.0)),
                ..Default::default()
            },
        );
        assert_eq!(world.body_kind(kin), BodyKind::KinematicPosition);
        world.set_body_position(kin, Vec3::new(3.0, 4.0, 5.0));
        assert_eq!(world.body_position(kin), Vec3::new(3.0, 4.0, 5.0));
        // Kinematic bodies are not gravity-driven.
        world.step();
        assert_eq!(world.body_position(kin), Vec3::new(3.0, 4.0, 5.0));
    }

    #[test]
    fn timestep_guard_rejects_non_positive() {
        let mut world = gravity_world();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                world.set_timestep(0.0);
            }))
            .is_err()
        );
    }

    #[test]
    fn gravity_and_counts_readback() {
        let mut world = PhysicsWorld::default();
        assert_eq!(world.gravity(), Vec3::new(0.0, -9.81, 0.0));
        assert_eq!(world.body_count(), 0);
        let _ = floor(&mut world);
        assert_eq!(world.body_count(), 1);
        assert!(eq_eps(world.timestep(), PhysicsWorld::DEFAULT_DT));
    }

    #[test]
    fn body_exists_distinguishes_live_and_stale_handles() {
        let mut world = PhysicsWorld::default();
        let floor_handle = floor(&mut world);
        assert!(world.body_exists(floor_handle), "fresh handle must exist");
        // A handle that was never created (max index + max generation) must
        // report missing — this is the predicate validate_world uses
        // instead of the panicking positional accessors. The engine never
        // removes bodies (worlds are rebuilt fresh), so "stale" means
        // "never existed or from a discarded world".
        let bogus = BodyHandle(rp::RigidBodyHandle::from_raw_parts(u32::MAX, u32::MAX));
        assert!(!world.body_exists(bogus));
    }
}
