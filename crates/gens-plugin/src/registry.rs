//! The plugin registry: ordered, duplicate-rejecting, fully recorded.
//!
//! # The registration contract (ADR-0032)
//!
//! A plugin is an in-process unit of engine extension. It registers
//! **real typed bevy systems** on the composition root's schedules and
//! spawns startup content — the same vocabulary the engine uses on
//! itself, with no reflection, no dynamic libraries and no `dyn`-erased
//! system boxing. Application is deterministic: plugins apply in the
//! order the composition root lists them, duplicate or empty plugin
//! names are loud errors, and the applied record lists what extended
//! the run (evidence output, the same discipline as every format this
//! engine emits).
//!
//! Ordering within a plugin is the plugin author's job (bevy's
//! `.chain()` over a tuple); ordering ACROSS plugins is registration
//! order for data-conflicting systems — the same discipline the
//! engine's own `add_systems` calls rely on (see `gens-physics`).

use std::collections::BTreeSet;

use gens_ecs::EngineSchedules;
use gens_ecs::ecs::world::World;
use thiserror::Error;

/// Errors from plugin registration and application.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum PluginError {
    /// Two plugins claimed the same name.
    #[error("duplicate plugin name '{name}'")]
    DuplicatePlugin {
        /// The duplicated name.
        name: String,
    },
    /// Plugin names are the evidence vocabulary; empty ones are bugs.
    #[error("plugin name must be non-empty")]
    EmptyName,
    /// A plugin's build callback failed.
    #[error("plugin '{plugin}' failed: {message}")]
    BuildFailed {
        /// Which plugin failed.
        plugin: String,
        /// The plugin's own message.
        message: String,
    },
}

/// What a plugin receives during registration.
///
/// Deliberately narrow: the engine's schedules (to add real systems)
/// and the world (for startup spawning). Everything else — resources,
/// events, asset registration — the plugin establishes through its own
/// systems and startup code against this world.
pub struct PluginHost<'a> {
    world: &'a mut World,
    schedules: &'a mut EngineSchedules,
}

impl<'a> PluginHost<'a> {
    /// Wraps the composition root's world and schedules.
    #[must_use]
    pub fn new(world: &'a mut World, schedules: &'a mut EngineSchedules) -> PluginHost<'a> {
        PluginHost { world, schedules }
    }

    /// Mutable access to the world for startup spawning.
    pub fn world(&mut self) -> &mut World {
        self.world
    }

    /// Adds system(s) to the engine's Update schedule.
    ///
    /// Accepts anything `Schedule::add_systems` accepts — a single
    /// system, a tuple (ambiguous order by data conflicts, the engine's
    /// own convention) or a `.chain()`ed tuple (author-controlled
    /// order). Intra-plugin ordering is the author's contract.
    pub fn add_update_systems<M>(
        &mut self,
        systems: impl gens_ecs::ecs::schedule::IntoScheduleConfigs<
            gens_ecs::ecs::system::ScheduleSystem,
            M,
        >,
    ) {
        self.schedules.update.add_systems(systems);
    }

    /// Adds system(s) to the engine's PostUpdate schedule.
    ///
    /// PostUpdate ends with transform propagation (added at schedule
    /// construction); a plugin's post-update systems order against
    /// propagation only through data conflicts. Prefer the Update
    /// schedule unless the system must see post-propagation state.
    pub fn add_post_update_systems<M>(
        &mut self,
        systems: impl gens_ecs::ecs::schedule::IntoScheduleConfigs<
            gens_ecs::ecs::system::ScheduleSystem,
            M,
        >,
    ) {
        self.schedules.post_update.add_systems(systems);
    }
}

/// An in-process engine extension (ADR-0032).
pub trait Plugin {
    /// The plugin's name — the evidence vocabulary. Must be non-empty
    /// and unique across the applied registry.
    fn name(&self) -> &str;

    /// Registers systems and startup content.
    ///
    /// # Errors
    /// A plugin may abort application with [`PluginError::BuildFailed`]
    /// when its own invariants fail — a broken plugin is a loud load
    /// error, never a half-registered one (the ADR-0028 lesson: a
    /// broken export never half-runs).
    fn build(&self, host: &mut PluginHost) -> Result<(), PluginError>;
}

/// An ordered registry of plugins awaiting application.
#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
    names: BTreeSet<String>,
}

impl std::fmt::Debug for PluginRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginRegistry")
            .field("names", &self.names)
            .finish()
    }
}

impl PluginRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> PluginRegistry {
        PluginRegistry::default()
    }

    /// Adds a plugin, rejecting duplicate and empty names immediately.
    ///
    /// # Errors
    /// [`PluginError::DuplicatePlugin`] or [`PluginError::EmptyName`].
    pub fn add(&mut self, plugin: impl Plugin + 'static) -> Result<(), PluginError> {
        let name = plugin.name();
        if name.is_empty() {
            return Err(PluginError::EmptyName);
        }
        if !self.names.insert(name.to_string()) {
            return Err(PluginError::DuplicatePlugin {
                name: name.to_string(),
            });
        }
        self.plugins.push(Box::new(plugin));
        Ok(())
    }

    /// How many plugins are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Applies every plugin, in registration order, to the given world
    /// and schedules.
    ///
    /// Deterministic by construction: application order == registration
    /// order, and the applied record lists names in that order. On a
    /// plugin failure, earlier plugins stay applied (their systems are
    /// live) and the error names the failure point — the composition
    /// root decides whether to abort, and the record says what ran.
    ///
    /// # Errors
    /// Propagates [`PluginError::BuildFailed`] from plugin builds.
    pub fn apply(
        mut self,
        world: &mut World,
        schedules: &mut EngineSchedules,
    ) -> Result<AppliedPlugins, PluginError> {
        let mut applied = Vec::with_capacity(self.plugins.len());
        for plugin in self.plugins.drain(..) {
            let name = plugin.name().to_string();
            let mut host = PluginHost::new(world, schedules);
            plugin.build(&mut host)?;
            applied.push(name);
        }
        Ok(AppliedPlugins { plugins: applied })
    }
}

/// The record of what a successful application extended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedPlugins {
    /// Plugin names in application order.
    pub plugins: Vec<String>,
}

impl AppliedPlugins {
    /// Whether `name` is in the applied set.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.plugins.iter().any(|p| p == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_ecs::{Active, Name, Transform};

    /// A test plugin that spawns a marker and registers a system.
    struct MarkerPlugin {
        name: String,
        marker: &'static str,
    }

    impl MarkerPlugin {
        fn new(name: &str, marker: &'static str) -> MarkerPlugin {
            MarkerPlugin {
                name: name.to_string(),
                marker,
            }
        }
    }

    impl Plugin for MarkerPlugin {
        fn name(&self) -> &str {
            &self.name
        }

        fn build(&self, host: &mut PluginHost) -> Result<(), PluginError> {
            host.world()
                .spawn((Name::new(self.marker), Transform::IDENTITY, Active));
            Ok(())
        }
    }

    /// A plugin that always fails its build.
    struct BrokenPlugin;
    impl Plugin for BrokenPlugin {
        fn name(&self) -> &str {
            "broken"
        }
        fn build(&self, _host: &mut PluginHost) -> Result<(), PluginError> {
            Err(PluginError::BuildFailed {
                plugin: "broken".to_string(),
                message: "deliberate failure".to_string(),
            })
        }
    }

    #[test]
    fn applies_in_registration_order() {
        let mut registry = PluginRegistry::new();
        registry
            .add(MarkerPlugin::new("alpha", "AlphaMarker"))
            .expect("add");
        registry
            .add(MarkerPlugin::new("beta", "BetaMarker"))
            .expect("add");
        let mut world = World::new();
        let mut schedules = EngineSchedules::new();
        let applied = registry.apply(&mut world, &mut schedules).expect("apply");
        assert_eq!(applied.plugins, vec!["alpha", "beta"]);
        assert!(applied.contains("alpha"));
        assert!(applied.contains("beta"));
        assert!(!applied.contains("gamma"));
        // Both plugins spawned their markers.
        let names: Vec<String> = {
            let mut q = world.query::<&Name>();
            q.iter(&world).map(|n| n.0.clone()).collect()
        };
        assert!(names.contains(&"AlphaMarker".to_string()));
        assert!(names.contains(&"BetaMarker".to_string()));
    }

    #[test]
    fn rejects_duplicate_and_empty_names() {
        let mut registry = PluginRegistry::new();
        registry.add(MarkerPlugin::new("dup", "A")).expect("first");
        assert_eq!(
            registry.add(MarkerPlugin::new("dup", "B")),
            Err(PluginError::DuplicatePlugin {
                name: "dup".to_string()
            })
        );
        assert_eq!(
            registry.add(MarkerPlugin::new("", "C")),
            Err(PluginError::EmptyName)
        );
        assert_eq!(registry.len(), 1, "rejected plugins are not registered");
    }

    #[test]
    fn failed_build_names_the_failure_and_keeps_earlier_work() {
        let mut registry = PluginRegistry::new();
        registry
            .add(MarkerPlugin::new("good", "GoodMarker"))
            .expect("add");
        registry.add(BrokenPlugin).expect("add");
        let mut world = World::new();
        let mut schedules = EngineSchedules::new();
        let err = registry.apply(&mut world, &mut schedules).unwrap_err();
        assert_eq!(
            err,
            PluginError::BuildFailed {
                plugin: "broken".to_string(),
                message: "deliberate failure".to_string(),
            }
        );
        // The earlier plugin's content is live.
        let names: Vec<String> = {
            let mut q = world.query::<&Name>();
            q.iter(&world).map(|n| n.0.clone()).collect()
        };
        assert!(names.contains(&"GoodMarker".to_string()));
    }
}
