//! Gens physics hello world (Phase 3): input → actions → physics → ECS.
//!
//! A player ball on a floor is driven by a **scripted, deterministic input
//! replay**: a timeline of [`DeviceEvent`]s fed into `gens-input`, whose
//! actions apply impulses through `gens-physics`, whose poses sync into the
//! ECS (`Transform` → `WorldTransform` propagation from Phase 1).
//!
//! The whole simulation runs twice on the fixed-timestep loop; the two runs
//! must produce **bit-identical trajectories** — the Phase 3 in-process
//! determinism proof spanning all three new crates plus the Phase 1 loop
//! and ECS (see `docs/RISKS.md` R-013 for the cross-platform caveat).
//!
//! ```text
//! cargo run -p gens-runtime --example physics_hello -- --steps 600 --out physics_hello.json
//! ```
//!
//! No GPU, no audio device, no window — runs on every CI platform.

use gens_core::anyhow;
use gens_core::anyhow::Context;
use gens_core::game_loop::{GameLoop, LoopConfig};
use gens_core::logging::{info, init};
use gens_core::time::FixedSource;
use gens_ecs::ecs::world::World;
use gens_ecs::{Active, EngineSchedules, Name, WorldTransform};
use gens_input::{DeviceEvent, InputMap, InputState, KeyCode};
use gens_physics::{
    BodyHandle, BodySpec, ColliderShape, ColliderSpec, PhysicsBody, PhysicsResource, PhysicsWorld,
    sync_transforms_from_physics,
};
use std::path::PathBuf;
use std::time::Duration;

/// Gameplay actions for this demo — the user-facing contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum GameAction {
    /// Strafe left.
    MoveLeft,
    /// Strafe right.
    MoveRight,
    /// Jump.
    Jump,
}

/// The scripted input replay: `(simulation step, event)` pairs.
///
/// Deterministic by construction — this is what a recorded replay file
/// becomes in a real game.
fn input_script() -> Vec<(u32, DeviceEvent)> {
    use DeviceEvent::*;
    use KeyCode::*;
    vec![
        (30, KeyDown(KeyD)),
        (90, KeyUp(KeyD)), // ~1 s of strafing right
        (120, KeyDown(KeyA)),
        (170, KeyUp(KeyA)), // ~0.8 s back left (overcomes momentum)
        (200, KeyDown(Space)),
        (204, KeyUp(Space)), // a jump
        (300, KeyDown(KeyD)),
        (360, KeyUp(KeyD)), // another strafe
        (420, KeyDown(Space)),
        (424, KeyUp(Space)), // a second jump
    ]
}

/// Applies every script entry scheduled for `step`.
fn feed_script(input: &mut InputState<GameAction>, script: &[(u32, DeviceEvent)], step: u32) {
    for &(at, ev) in script {
        if at == step {
            input.handle_event(ev);
        }
    }
}

/// The gameplay policy: actions → forces, evaluated once per fixed step.
fn apply_actions(input: &InputState<GameAction>, physics: &mut PhysicsWorld, player: BodyHandle) {
    // Per-step impulses: applied EVERY fixed step, so magnitudes are
    // tiny (0.05 Ns on a ~0.52 kg ball ≈ 5.7 m/s² of control authority).
    const STRAFE_IMPULSE: f32 = 0.05;
    const JUMP_IMPULSE: f32 = 2.2;

    let x_axis = gens_input::axis(input, &GameAction::MoveLeft, &GameAction::MoveRight);
    if x_axis != 0.0 {
        physics.apply_impulse(
            player,
            gens_math::Vec3::new(x_axis * STRAFE_IMPULSE, 0.0, 0.0),
        );
    }
    // Jump only from (near) rest vertically — a cheap ground check for the demo.
    if input.action_just_pressed(&GameAction::Jump) && physics.body_velocity(player).y.abs() < 0.1 {
        physics.apply_impulse(player, gens_math::Vec3::new(0.0, JUMP_IMPULSE, 0.0));
    }
}

/// One deterministic simulation run; returns the sampled trajectory.
fn run_simulation(steps: u32) -> anyhow::Result<Vec<(u32, f32, f32)>> {
    let mut world = World::new();
    let mut schedules = EngineSchedules::new();
    schedules.update.add_systems(sync_transforms_from_physics);

    let mut physics = PhysicsWorld::new(gens_math::Vec3::new(0.0, -9.81, 0.0));
    let (_floor_body, _) = physics.create_body(
        BodySpec::fixed_at(gens_math::Vec3::new(0.0, -0.5, 0.0)),
        ColliderSpec {
            // 100 m wide: the demo never reaches the edge in 600 steps.
            shape: ColliderShape::Cuboid(gens_math::Vec3::new(50.0, 0.5, 50.0)),
            friction: 0.8,
            ..Default::default()
        },
    );
    let (player_body, _) = physics.create_body(
        BodySpec {
            ccd: true, // fast movers must not tunnel through the floor
            // Rolling has no natural resistance in rapier; damping makes
            // strafing stop when inputs stop (and keeps the demo on the map).
            linear_damping: 0.8,
            ..BodySpec::dynamic_at(gens_math::Vec3::new(0.0, 0.5, 0.0))
        },
        ColliderSpec {
            shape: ColliderShape::Ball(0.5),
            friction: 0.2,
            ..Default::default()
        },
    );
    world.insert_resource(PhysicsResource { world: physics });
    let player = world
        .spawn((
            Name::new("Player"),
            gens_ecs::Transform::IDENTITY,
            Active,
            PhysicsBody {
                handle: player_body,
            },
        ))
        .id();

    let mut input = InputState::with_map(bindings());
    let script = input_script();
    let mut trajectory: Vec<(u32, f32, f32)> = Vec::new();

    let source = FixedSource::new(Duration::from_secs_f64(1.0 / 60.0));
    let mut game_loop = GameLoop::new(LoopConfig::default(), source);
    let mut step_index = 0u32;
    for _ in 0..steps {
        game_loop.run_frame(|_| {
            feed_script(&mut input, &script, step_index);
            // Gameplay queries FIRST (edges are live), THEN the input frame
            // ends: update() rotates just-pressed/released for the next step.
            let mut res = world.resource_mut::<PhysicsResource>();
            apply_actions(&input, &mut res.world, player_body);
            input.update();
            // One fixed solver step, explicitly ordered: control forces
            // first, then the solver, then sync/propagation below.
            res.world.step();
            // `res`'s borrow of `world` ends at its last use (NLL).

            // Transform sync + WorldTransform propagation (Phase 1 pipeline).
            schedules.run_frame(&mut world);
            step_index += 1;
        });
        let wt = world
            .get::<WorldTransform>(player)
            .expect("propagation ran");
        let p = wt.position();
        trajectory.push((step_index, p.x, p.y));
    }
    Ok(trajectory)
}

fn bindings() -> InputMap<GameAction> {
    let mut map = InputMap::new();
    map.bind_key(GameAction::MoveLeft, KeyCode::KeyA);
    map.bind_key(GameAction::MoveLeft, KeyCode::ArrowLeft);
    map.bind_key(GameAction::MoveRight, KeyCode::KeyD);
    map.bind_key(GameAction::MoveRight, KeyCode::ArrowRight);
    map.bind_key(GameAction::Jump, KeyCode::Space);
    map
}

fn main() -> anyhow::Result<()> {
    let mut steps: u32 = 600;
    let mut out = PathBuf::from("physics_hello.json");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--steps" => {
                steps = args
                    .next()
                    .context("--steps needs a value")?
                    .parse()
                    .context("--steps must be a number")?;
            }
            "--out" => {
                out = PathBuf::from(args.next().context("--out needs a value")?);
            }
            other => anyhow::bail!("unknown argument: {other} (use --steps N, --out PATH)"),
        }
    }

    let filter = init().unwrap_or_else(|_| "(logging already initialized)".to_string());
    info!("Gens engine - physics hello world");
    info!(steps, ?out, %filter, "run configuration");

    // --- Two identical runs must be bit-identical (R-013, in-process) ------
    let run_a = run_simulation(steps)?;
    let run_b = run_simulation(steps)?;

    assert_eq!(
        run_a.len(),
        run_b.len(),
        "both runs must execute the same step count"
    );
    for (a, b) in run_a.iter().zip(run_b.iter()) {
        assert_eq!(a.0, b.0, "step index mismatch");
        assert_eq!(a.1, b.1, "x must be bit-identical at step {}", a.0);
        assert_eq!(a.2, b.2, "y must be bit-identical at step {}", a.0);
    }
    info!(samples = run_a.len(), "determinism: two runs bit-identical");

    // --- Behavioral contract checks on the trajectory ----------------------
    let y_at = |step: u32| -> f32 {
        run_a
            .iter()
            .rev()
            .find(|&&(s, _, _)| s <= step)
            .map(|&(_, _, y)| y)
            .expect("trajectory sampled every step")
    };
    let x_at = |step: u32| -> f32 {
        run_a
            .iter()
            .rev()
            .find(|&&(s, _, _)| s <= step)
            .map(|&(_, x, _)| x)
            .expect("trajectory sampled every step")
    };

    // 1) The player never falls through the floor (radius 0.5 above y=0).
    let min_y = run_a
        .iter()
        .map(|&(_, _, y)| y)
        .fold(f32::INFINITY, f32::min);
    assert!(
        min_y > 0.4,
        "player must stay above the floor, min y = {min_y}"
    );

    // 2) The scripted strafe moved the player to +x, then partially back.
    assert!(
        x_at(89) > 0.5,
        "1 s of MoveRight must move the player right, x = {}",
        x_at(89)
    );
    let x_before_left = x_at(119);
    let min_x_during_left = run_a
        .iter()
        .filter(|&&(s, _, _)| (120..=199).contains(&s))
        .map(|&(_, x, _)| x)
        .fold(f32::INFINITY, f32::min);
    assert!(
        min_x_during_left < x_before_left,
        "MoveLeft must pull the player back: min {min_x_during_left} < {x_before_left}"
    );

    // 3) The jump lifted the player, who then came back down to rest.
    let pre_jump = y_at(199);
    let mut peak = f32::NEG_INFINITY;
    for &(_, _, y) in &run_a {
        if y > peak {
            peak = y;
        }
    }
    assert!(
        peak > pre_jump + 0.2,
        "jump must lift the player: peak {peak} vs {pre_jump}"
    );
    let final_y = y_at(steps);
    assert!(
        (final_y - 0.5).abs() < 0.05,
        "player must come to rest at y≈0.5, got {final_y}"
    );

    info!(
        final_x = x_at(steps),
        final_y = final_y,
        peak_y = peak,
        min_y = min_y,
        "physics hello verification passed"
    );

    // --- Write the trajectory as JSON evidence ------------------------------
    let mut json = String::from("{\"trajectory\": [\n");
    for (i, &(s, x, y)) in run_a.iter().enumerate() {
        let comma = if i + 1 == run_a.len() { "" } else { "," };
        json.push_str(&format!(
            "  {{\"step\": {s}, \"x\": {x}, \"y\": {y}}}{comma}\n"
        ));
    }
    json.push_str("]}\n");
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&out, json).with_context(|| format!("writing {out:?}"))?;
    info!(?out, "trajectory written");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_is_step_ordered_and_bounded() {
        let script = input_script();
        assert!(!script.is_empty());
        for pair in script.windows(2) {
            assert!(pair[0].0 <= pair[1].0, "script must be non-decreasing");
        }
        // Every event lands inside a 600-step default run.
        assert!(script.last().unwrap().0 < 600);
    }

    #[test]
    fn bindings_cover_all_actions() {
        let map = bindings();
        assert!(!map.bindings(&GameAction::MoveLeft).is_empty());
        assert!(!map.bindings(&GameAction::MoveRight).is_empty());
        assert!(!map.bindings(&GameAction::Jump).is_empty());
    }

    #[test]
    fn short_simulation_is_deterministic() {
        // A compact in-test slice of the main contract: two 120-step runs.
        let a = run_simulation(120).unwrap();
        let b = run_simulation(120).unwrap();
        assert_eq!(a, b, "identical inputs must replay identically");
    }
}

#[cfg(test)]
mod probe_y {
    use super::*;

    #[test]
    fn probe_trajectory() {
        let t = run_simulation(600).unwrap();
        let (mut min_step, mut min_y, mut peak_y) = (0, f32::INFINITY, f32::NEG_INFINITY);
        for &(s, x, y) in &t {
            if y < min_y {
                min_y = y;
                min_step = s;
            }
            peak_y = peak_y.max(y);
            if s == 89 || s == 179 || s == 260 || s == 300 || s == 500 || s == 599 {
                println!("step {s}: x={x:.3} y={y:.3}");
            }
        }
        println!("min y {min_y} at step {min_step}; peak y {peak_y}");
        // Find first step where y < 0.3 (falling off/through).
        if let Some(&(s, x, y)) = t.iter().find(|&&(_, _, y)| y < 0.3) {
            println!("FIRST y<0.3 at step {s}: x={x:.3} y={y:.3}");
        } else {
            println!("no y<0.3 anywhere");
        }
    }
}
