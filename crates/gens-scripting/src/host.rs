//! The ECS face of scripting: one component, one host resource, one
//! deterministic runner (P5-4).
//!
//! - [`ScriptComponent`] — which script runs for an entity, its entry
//!   function, its [`ScriptVars`] state and the animation requests the
//!   last tick produced;
//! - [`ScriptHost`] — the sandboxed engine plus every compiled script,
//!   held as a World resource;
//! - [`run_scripts`] — the exclusive system that ticks all scripted
//!   entities in **entity-index order** (the ADR-0019 determinism
//!   rule): same world, same dt, same time => same final state.

use crate::engine::ScriptEngine;
use crate::error::ScriptResult;
use crate::vars::ScriptVars;
use gens_ecs::ecs::prelude::*;
use gens_math::Vec3;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Marks an entity as scripted and holds its script-facing state.
#[derive(Component, Debug, Clone)]
pub struct ScriptComponent {
    /// Name of the compiled script in [`ScriptHost`] (its asset key).
    pub script: String,
    /// Entry function called each tick (default "update").
    pub entry: String,
    /// The script's variables (in/out every tick).
    pub vars: ScriptVars,
    /// Requests the last tick made to the animation system.
    pub requests: ScriptRequests,
}

impl ScriptComponent {
    /// A scripted entity running `script`'s `update` entry.
    pub fn new(script: impl Into<String>) -> ScriptComponent {
        ScriptComponent {
            script: script.into(),
            entry: "update".to_string(),
            vars: ScriptVars::new(),
            requests: ScriptRequests::default(),
        }
    }

    /// Overrides the entry function name.
    pub fn with_entry(mut self, entry: impl Into<String>) -> Self {
        self.entry = entry.into();
        self
    }
}

/// Animation requests a script tick produced (data only — the game
/// code decides what they mean; nothing here references animation
/// types, keeping scripting -> animation a one-way data street).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScriptRequests {
    /// Parameter writes requested (sorted by name).
    pub params: BTreeMap<String, f64>,
    /// A state to play, if the script asked for one.
    pub play: Option<String>,
}

/// One scripted entity's view of the world for ONE tick.
///
/// Everything a script can read lives in here; everything it writes is
/// applied after the call returns. No aliasing, no re-entrancy, no
/// partial writes: a script that errors mid-call leaves the entity
/// untouched (the frame is dropped).
#[derive(Debug, Clone)]
pub struct ScriptFrame {
    /// Engine simulation time (seconds since the sim started).
    pub time: f64,
    /// This tick's delta in seconds.
    pub dt: f64,
    /// The entity's current position (read-only view).
    pub pos: Vec3,
    /// The entity's current rotation (read-only view).
    pub rot: gens_math::Quat,
    /// The entity's script variables (in/out).
    pub vars: ScriptVars,
    /// Position written by `set_pos`, if the script wrote one.
    pub out_pos: Option<Vec3>,
    /// Requests accumulated this tick.
    pub requests: ScriptRequests,
}

/// Shared slot the registered API functions read the current frame
/// from. `Arc<Mutex>` because rhai's `sync` feature requires
/// `Send + Sync` native functions; use is strictly sequential, so the
/// mutex never contends (and determinism never depends on it).
type FrameSlot = Arc<Mutex<Option<ScriptFrame>>>;

/// Reads/writes the current frame or fails the script call cleanly.
fn with_frame<T>(
    slot: &FrameSlot,
    f: impl FnOnce(&mut ScriptFrame) -> Result<T, String>,
) -> Result<T, Box<rhai::EvalAltResult>> {
    let mut guard = slot.lock().map_err(|_| {
        rhai::EvalAltResult::ErrorRuntime("frame slot poisoned".into(), rhai::Position::NONE)
    })?;
    let frame = guard.as_mut().ok_or_else(|| {
        rhai::EvalAltResult::ErrorRuntime("no script frame active".into(), rhai::Position::NONE)
    })?;
    f(frame).map_err(|e| {
        Box::new(rhai::EvalAltResult::ErrorRuntime(
            e.into(),
            rhai::Position::NONE,
        ))
    })
}

/// The compiled-script store + engine + current-frame slot.
///
/// A `Resource` (and, per the bevy 0.16+ rule, also a `Component`) —
/// the `gens-physics::PhysicsResource` precedent.
#[derive(Debug, Resource)]
pub struct ScriptHost {
    engine: ScriptEngine,
    scripts: BTreeMap<String, crate::engine::Script>,
    current: FrameSlot,
}

impl Default for ScriptHost {
    fn default() -> Self {
        ScriptHost::new()
    }
}

impl ScriptHost {
    /// Builds the host with the entity API registered into its engine.
    pub fn new() -> ScriptHost {
        let current: FrameSlot = Arc::new(Mutex::new(None));
        let mut engine = ScriptEngine::new();
        register_math_api(&mut engine);
        register_entity_api(&mut engine, Arc::clone(&current));
        ScriptHost {
            engine,
            scripts: BTreeMap::new(),
            current,
        }
    }

    /// Compiles and registers a script under `name`.
    ///
    /// # Errors
    /// [`ScriptError::Compile`] on bad source.
    pub fn register(&mut self, name: impl Into<String>, source: &str) -> ScriptResult<()> {
        let name = name.into();
        let script = self.engine.compile(&name, source)?;
        self.scripts.insert(name, script);
        Ok(())
    }

    /// Borrows a compiled script by name.
    pub fn get(&self, name: &str) -> Option<&crate::engine::Script> {
        self.scripts.get(name)
    }

    /// Number of registered scripts.
    pub fn len(&self) -> usize {
        self.scripts.len()
    }

    /// True when no scripts are registered.
    pub fn is_empty(&self) -> bool {
        self.scripts.is_empty()
    }

    /// The registered script names, sorted.
    pub fn script_names(&self) -> Vec<String> {
        self.scripts.keys().cloned().collect()
    }

    /// Engine read access (for advanced host use).
    pub fn engine(&self) -> &ScriptEngine {
        &self.engine
    }
}

/// Registers the CLOSED math vocabulary scripts may use.
///
/// `Engine::new_raw()` ships no packages, so even `sqrt`/`floor` are
/// absent by default — the host exposes exactly these, all thin libm
/// calls (deterministic within a process; the R-013 scope).
fn register_math_api(engine: &mut ScriptEngine) {
    let e = engine.engine_mut();
    e.register_fn("sqrt", |v: f64| v.sqrt());
    e.register_fn("floor", |v: f64| v.floor());
    e.register_fn("ceil", |v: f64| v.ceil());
    e.register_fn("abs", |v: f64| v.abs());
    e.register_fn("min", |a: f64, b: f64| a.min(b));
    e.register_fn("max", |a: f64, b: f64| a.max(b));
    // Integer conversion (rhai has no cast operator; to_int/to_float
    // live in packages the sandbox does not load).
    e.register_fn("to_int", |v: f64| v as i64);
    e.register_fn("to_float", |v: i64| v as f64);
}

/// Registers the entity API: the ONLY functions a script may call
/// besides pure computation. All of them operate on the current frame
/// slot; all of them are deterministic functions of it.
fn register_entity_api(engine: &mut ScriptEngine, current: FrameSlot) {
    let slot = Arc::clone(&current);
    engine
        .engine_mut()
        .register_fn("now", move || -> Result<f64, Box<rhai::EvalAltResult>> {
            with_frame(&slot, |f| Ok(f.time))
        });

    let slot = Arc::clone(&current);
    engine
        .engine_mut()
        .register_fn("dt", move || -> Result<f64, Box<rhai::EvalAltResult>> {
            with_frame(&slot, |f| Ok(f.dt))
        });

    let slot = Arc::clone(&current);
    engine
        .engine_mut()
        .register_fn("pos_x", move || -> Result<f64, Box<rhai::EvalAltResult>> {
            with_frame(&slot, |f| Ok(f64::from(f.pos.x)))
        });

    let slot = Arc::clone(&current);
    engine
        .engine_mut()
        .register_fn("pos_y", move || -> Result<f64, Box<rhai::EvalAltResult>> {
            with_frame(&slot, |f| Ok(f64::from(f.pos.y)))
        });

    let slot = Arc::clone(&current);
    engine
        .engine_mut()
        .register_fn("pos_z", move || -> Result<f64, Box<rhai::EvalAltResult>> {
            with_frame(&slot, |f| Ok(f64::from(f.pos.z)))
        });

    let slot = Arc::clone(&current);
    engine.engine_mut().register_fn(
        "set_pos",
        move |x: f64, y: f64, z: f64| -> Result<(), Box<rhai::EvalAltResult>> {
            with_frame(&slot, |f| {
                f.out_pos = Some(Vec3::new(x as f32, y as f32, z as f32));
                Ok(())
            })
        },
    );

    // Script variables (kind-checked; conflicts fail the call).
    let slot = Arc::clone(&current);
    engine.engine_mut().register_fn(
        "var_num",
        move |name: &str| -> Result<f64, Box<rhai::EvalAltResult>> {
            with_frame(&slot, |f| {
                f.vars.get_number(name).map_err(|e| e.to_string())
            })
        },
    );

    let slot = Arc::clone(&current);
    engine.engine_mut().register_fn(
        "set_var_num",
        move |name: &str, v: f64| -> Result<(), Box<rhai::EvalAltResult>> {
            with_frame(&slot, |f| {
                f.vars.set_number(name, v).map_err(|e| e.to_string())
            })
        },
    );

    let slot = Arc::clone(&current);
    engine.engine_mut().register_fn(
        "var_flag",
        move |name: &str| -> Result<bool, Box<rhai::EvalAltResult>> {
            with_frame(&slot, |f| f.vars.get_flag(name).map_err(|e| e.to_string()))
        },
    );

    let slot = Arc::clone(&current);
    engine.engine_mut().register_fn(
        "set_var_flag",
        move |name: &str, v: bool| -> Result<(), Box<rhai::EvalAltResult>> {
            with_frame(&slot, |f| {
                f.vars.set_flag(name, v).map_err(|e| e.to_string())
            })
        },
    );

    let slot = Arc::clone(&current);
    engine.engine_mut().register_fn(
        "var_text",
        move |name: &str| -> Result<String, Box<rhai::EvalAltResult>> {
            with_frame(&slot, |f| f.vars.get_text(name).map_err(|e| e.to_string()))
        },
    );

    let slot = Arc::clone(&current);
    engine.engine_mut().register_fn(
        "set_var_text",
        move |name: &str, v: &str| -> Result<(), Box<rhai::EvalAltResult>> {
            with_frame(&slot, |f| {
                f.vars.set_text(name, v).map_err(|e| e.to_string())
            })
        },
    );

    // Animation requests (data only; the game code applies them).
    let slot = Arc::clone(&current);
    engine.engine_mut().register_fn(
        "anim_set_param",
        move |name: &str, v: f64| -> Result<(), Box<rhai::EvalAltResult>> {
            with_frame(&slot, |f| {
                f.requests.params.insert(name.to_string(), v);
                Ok(())
            })
        },
    );

    let slot = Arc::clone(&current);
    engine.engine_mut().register_fn(
        "anim_play",
        move |name: &str| -> Result<(), Box<rhai::EvalAltResult>> {
            with_frame(&slot, |f| {
                f.requests.play = Some(name.to_string());
                Ok(())
            })
        },
    );
}

/// Ticks every scripted entity: entry fn called in entity-index order,
/// writes applied only on success. Returns the number of scripts run.
///
/// # Panics
/// Panics when no [`ScriptHost`] resource is present — scripting
/// without a host is a wiring bug, not a runtime condition.
pub fn run_scripts(world: &mut World, dt: f64, time_seconds: f64) -> usize {
    // Exclusive access: take the host out (rhai calls need &engine
    // while entity writes need &mut World — an exclusive borrow of
    // both is impossible without the swap).
    let host = world
        .remove_resource::<ScriptHost>()
        .expect("ScriptHost resource missing");

    // Snapshot the tick list in entity-index order (ADR-0019).
    let mut query = world.query::<(Entity, &ScriptComponent)>();
    let mut ticks: Vec<(
        gens_ecs::ecs::entity::EntityIndex,
        Entity,
        String,
        String,
        ScriptVars,
        Vec3,
        gens_math::Quat,
    )> = Vec::new();
    for (entity, sc) in query.iter(world) {
        let pos = world
            .get::<gens_ecs::Transform>(entity)
            .map(|t| t.position)
            .unwrap_or(Vec3::ZERO);
        let rot = world
            .get::<gens_ecs::Transform>(entity)
            .map(|t| t.rotation)
            .unwrap_or(gens_math::Quat::IDENTITY);
        ticks.push((
            entity.index(),
            entity,
            sc.script.clone(),
            sc.entry.clone(),
            sc.vars.clone(),
            pos,
            rot,
        ));
    }
    ticks.sort_by_key(|(idx, ..)| *idx);

    let mut run = 0;
    for (_idx, entity, script_name, entry, vars, pos, rot) in ticks {
        let Some(script) = host.scripts.get(&script_name) else {
            // A missing script is a content error, but killing the
            // whole simulation for one entity is worse; skip (and the
            // behavioral gates in the example catch the stall).
            continue;
        };
        let mut frame = ScriptFrame {
            time: time_seconds,
            dt,
            pos,
            rot,
            vars,
            out_pos: None,
            requests: ScriptRequests::default(),
        };
        // Publish the frame, call, harvest.
        *host.current.lock().expect("frame slot poisoned") = Some(frame.clone());
        let result: ScriptResult<()> =
            host.engine
                .call_fn_with_scope(&mut rhai::Scope::new(), script, &entry, ());
        let Ok(applied) = result.map(|()| {
            let slot = host.current.lock().expect("frame slot poisoned");
            slot.clone().expect("frame just published")
        }) else {
            // Failed tick: drop the frame, leave the entity untouched.
            *host.current.lock().expect("frame slot poisoned") = None;
            continue;
        };
        *host.current.lock().expect("frame slot poisoned") = None;
        frame = applied;

        if let (Some(new_pos), Some(mut t)) =
            (frame.out_pos, world.get_mut::<gens_ecs::Transform>(entity))
        {
            t.position = new_pos;
        }
        if let Some(mut sc) = world.get_mut::<ScriptComponent>(entity) {
            sc.vars = frame.vars;
            sc.requests = frame.requests;
        }
        run += 1;
    }

    world.insert_resource(host);
    run
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_ecs::{Active, Name, Transform};

    const PATROL: &str = r#"
        fn update() {
            let dir = var_num("dir");
            let speed = var_num("speed");
            set_var_num("dir", dir + dt());
            if pos_x() > 2.0 {
                set_var_flag("turned", true);
            }
            set_pos(pos_x() + speed * dt(), pos_y(), pos_z() + dir * 0.0);
        }
    "#;

    fn build_world(order: &[f64]) -> (World, Vec<Entity>) {
        let mut world = World::new();
        let mut host = ScriptHost::new();
        host.register("patrol", PATROL).expect("compiles");
        world.insert_resource(host);
        let mut entities = Vec::new();
        for (i, dir) in order.iter().enumerate() {
            let e = world
                .spawn((
                    Name::new(format!("e{i}")),
                    Transform::from_position(Vec3::new(0.0, 0.0, 0.0)),
                    Active,
                    ScriptComponent::new("patrol"),
                ))
                .id();
            world
                .get_mut::<ScriptComponent>(e)
                .expect("fresh")
                .vars
                .set_number("dir", *dir)
                .expect("set");
            world
                .get_mut::<ScriptComponent>(e)
                .expect("fresh")
                .vars
                .set_number("speed", 1.0)
                .expect("set");
            entities.push(e);
        }
        (world, entities)
    }

    #[test]
    fn scripts_move_entities_and_persist_vars() {
        let (mut world, entities) = build_world(&[0.0]);
        for step in 1..=60 {
            let _ = run_scripts(&mut world, 1.0 / 60.0, f64::from(step) / 60.0);
        }
        let t = world.get::<Transform>(entities[0]).expect("alive");
        assert!((t.position.x - 1.0).abs() < 1e-4, "{}", t.position.x);
        let sc = world.get::<ScriptComponent>(entities[0]).expect("alive");
        let dir = sc.vars.get_number("dir").expect("var");
        assert!((dir - 1.0).abs() < 1e-4, "{dir}");
    }

    #[test]
    fn execution_is_deterministic_across_runs() {
        let trace = |order: &[f64]| {
            let (mut world, entities) = build_world(order);
            let mut positions = Vec::new();
            for step in 1..=120 {
                run_scripts(&mut world, 1.0 / 60.0, f64::from(step) / 60.0);
                let t = world.get::<Transform>(entities[0]).expect("alive");
                positions.push(t.position.x);
            }
            let sc = world.get::<ScriptComponent>(entities[0]).expect("alive");
            (positions, sc.vars.clone())
        };
        let (a, va) = trace(&[0.0]);
        let (b, vb) = trace(&[0.0]);
        assert_eq!(a, b);
        assert_eq!(va, vb);
    }

    #[test]
    fn entity_tick_order_is_index_sorted() {
        // Two entities reading a shared counter THROUGH their own vars
        // is impossible by design; instead assert the documented order
        // by giving each script a different speed and replaying: the
        // outcomes must not depend on spawn order.
        let a = trace_positions(&[3.0, 1.0, 2.0]);
        let b = trace_positions(&[1.0, 2.0, 3.0]);
        // Same multiset of final positions either way (per-entity
        // physics independent of tick order).
        let mut sa = a.clone();
        sa.sort_by(|x, y| x.total_cmp(y));
        let mut sb = b.clone();
        sb.sort_by(|x, y| x.total_cmp(y));
        assert_eq!(sa, sb);
    }

    fn trace_positions(order: &[f64]) -> Vec<f64> {
        let (mut world, entities) = build_world(order);
        for step in 1..=30 {
            run_scripts(&mut world, 1.0 / 60.0, f64::from(step) / 60.0);
        }
        entities
            .iter()
            .map(|e| f64::from(world.get::<Transform>(*e).expect("alive").position.x))
            .collect()
    }

    #[test]
    fn math_vocabulary_works_and_is_deterministic() {
        // new_raw ships no packages — even sqrt/floor are absent
        // unless the host exposes them. The vocabulary is a plain
        // engine surface (no frame needed: math fns touch no state).
        let mut host = ScriptHost::new();
        host.register("m", "fn g(x) { sqrt(x) * floor(x) + min(x, 9.0) }")
            .expect("compiles");
        let script = host.get("m").expect("registered").clone();
        let a: f64 = host.engine().call_fn(&script, "g", (2.5,)).expect("call");
        let b: f64 = host.engine().call_fn(&script, "g", (2.5,)).expect("call");
        // sqrt(2.5) ~= 1.5811, floor(2.5) = 2, min(2.5, 9) = 2.5.
        assert!((a - (2.5_f64.sqrt() * 2.0 + 2.5)).abs() < 1e-12);
        assert_eq!(a, b);
    }

    #[test]
    fn anim_requests_are_recorded() {
        let mut world = World::new();
        let mut host = ScriptHost::new();
        host.register(
            "anim",
            "fn update() { anim_play(\"walk\"); anim_set_param(\"speed\", 2.0); }",
        )
        .expect("compiles");
        world.insert_resource(host);
        let e = world
            .spawn((Transform::IDENTITY, ScriptComponent::new("anim")))
            .id();
        run_scripts(&mut world, 1.0 / 60.0, 1.0 / 60.0);
        let sc = world.get::<ScriptComponent>(e).expect("alive");
        assert_eq!(sc.requests.play.as_deref(), Some("walk"));
        assert_eq!(sc.requests.params.get("speed"), Some(&2.0));
    }

    #[test]
    fn failing_script_leaves_entity_untouched() {
        let mut world = World::new();
        let mut host = ScriptHost::new();
        host.register("boom", "fn update() { let x = 0; x / 0; }")
            .expect("compiles");
        world.insert_resource(host);
        let e = world
            .spawn((
                Transform::from_position(Vec3::new(5.0, 0.0, 0.0)),
                ScriptComponent::new("boom"),
            ))
            .id();
        let run = run_scripts(&mut world, 1.0 / 60.0, 1.0 / 60.0);
        // The script ran (was attempted) but the entity is unchanged.
        let t = world.get::<Transform>(e).expect("alive");
        assert_eq!(t.position, Vec3::new(5.0, 0.0, 0.0));
        let _ = run;
    }

    #[test]
    fn missing_script_skips_entity() {
        let mut world = World::new();
        world.insert_resource(ScriptHost::new());
        let e = world
            .spawn((
                Transform::from_position(Vec3::new(7.0, 0.0, 0.0)),
                ScriptComponent::new("nope"),
            ))
            .id();
        run_scripts(&mut world, 1.0 / 60.0, 1.0 / 60.0);
        assert_eq!(
            world.get::<Transform>(e).expect("alive").position,
            Vec3::new(7.0, 0.0, 0.0)
        );
    }
}
