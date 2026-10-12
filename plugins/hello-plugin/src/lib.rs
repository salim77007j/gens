//! A downstream plugin for the Gens engine — the ADR-0032 out-of-tree
//! proof.
//!
//! This crate is deliberately EXCLUDED from the engine workspace: it
//! path-depends on the engine's leaf crates exactly the way any
//! third-party plugin crate would, and CI builds and runs it as its own
//! step. If the plugin API only worked from inside the workspace, this
//! crate would not compile — that is the proof.
//!
//! [`HelloPlugin`] exercises the full registration vocabulary:
//!
//! - a plugin-defined component ([`Orb`]) and resource ([`OrbitState`])
//!   via plain bevy derives (no engine-side registration hooks needed),
//! - startup spawning through `PluginHost::world()`,
//! - a real typed system on the Update schedule through
//!   `PluginHost::add_update_systems`.
//!
//! The orbit itself is driven by the resource's fixed `dt` — a plugin
//! that hid a wall clock would be the plugin author's determinism bug,
//! not the engine's; this one keeps the engine's discipline anyway.

use bevy_ecs::prelude::*;
use gens_math::vec3;
use gens_plugin::{Plugin, PluginError, PluginHost};

/// Marks the entities the plugin orbits.
#[derive(Component, Debug)]
pub struct Orb;

/// The plugin's own simulation state: angle, radius and the FIXED step.
#[derive(Resource, Debug)]
pub struct OrbitState {
    /// Current orbital angle (radians).
    pub angle: f32,
    /// Orbital radius (world units).
    pub radius: f32,
    /// The fixed delta the orbit system advances per frame.
    pub dt: f32,
}

/// The downstream plugin: one spawn, one resource, one system.
#[derive(Debug, Default)]
pub struct HelloPlugin {
    /// Orbital radius handed to the state (default 3.0 when 0).
    pub radius: f32,
}

impl Plugin for HelloPlugin {
    fn name(&self) -> &str {
        "gens.hello-orbit"
    }

    fn build(&self, host: &mut PluginHost) -> Result<(), PluginError> {
        let radius = if self.radius > 0.0 { self.radius } else { 3.0 };
        host.world().insert_resource(OrbitState {
            angle: 0.0,
            radius,
            dt: 1.0 / 60.0,
        });
        host.world().spawn((
            Name::new("PluginOrb"),
            Orb,
            gens_ecs::Transform::from_position(vec3(radius, 1.0, 0.0)),
            gens_ecs::Active,
        ));
        host.add_update_systems(orbit_system);
        Ok(())
    }
}

/// Advances the orb deterministically: angle += dt, position = r * (cos,
/// sin) on the XZ plane. In-process determinism is what the demo proves;
/// the engine's cross-platform no-transcendental rule (ADR-0031) binds
/// ENGINE content, while a plugin's own content is its author's contract.
fn orbit_system(
    mut state: ResMut<OrbitState>,
    mut orbs: Query<&mut gens_ecs::Transform, With<Orb>>,
) {
    state.angle += state.dt;
    for mut transform in &mut orbs {
        transform.position = vec3(
            state.radius * state.angle.cos(),
            1.0,
            state.radius * state.angle.sin(),
        );
    }
}
