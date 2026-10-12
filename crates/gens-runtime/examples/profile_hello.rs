//! Gens profiler hello (Phase 8): the ADR-0027 observation-only proof.
//!
//! The SAME deterministic physics replay runs twice — once with the
//! profiler DISABLED, once ENABLED with per-phase spans — and both runs
//! must produce **byte-identical [`world_state_hash`] sequences**. If any
//! measured value ever re-entered the simulation, the hashes would drift;
//! this example is the per-integration proof ADR-0027 demands.
//!
//! The enabled run also emits the phase's profiling evidence:
//! - a `gens-profile-1` report (per-span quartiles, share of frame)
//! - a `gens-mem-1` memory snapshot (current/peak/counts from the
//!   tracking allocator)
//!
//! ```text
//! cargo run -p gens-runtime --example profile_hello -- --steps 600 --out profile_hello.json
//! ```
//!
//! No GPU, no audio device, no window — runs on every CI platform.

use gens_core::anyhow;
use gens_core::anyhow::Context;
use gens_core::logging::{info, init};
use gens_ecs::EngineSchedules;
use gens_ecs::ecs::world::World;
use gens_physics::{
    BodySpec, ColliderShape, ColliderSpec, PhysicsBody, PhysicsResource, PhysicsWorld,
    sync_transforms_from_physics,
};
use gens_profiler::{Profiler, memory_stats};
use gens_runtime::debug::validate_world;
use gens_runtime::profile::{instrumented_frame, world_state_hash};
use std::path::PathBuf;
use std::time::Duration;

/// Deterministic control script: `(step, ball, impulse)` triples.
///
/// Deliberately hand-written — the same contract as physics_hello's input
/// replay, minus the input crate, so the proof isolates *profiling* as the
/// only variable between the runs.
fn impulse_script() -> Vec<(u32, usize, [f32; 3])> {
    vec![
        (30, 0, [0.06, 0.0, 0.0]),    // ball A strafes right ~0.5 s
        (120, 1, [-0.06, 0.0, 0.04]), // ball B strafes left + forward
        (200, 2, [0.0, 2.2, 0.0]),    // ball C jumps
        (300, 0, [-0.06, 0.0, 0.0]),  // ball A comes back
        (420, 2, [0.05, 0.0, -0.05]), // ball C wanders
    ]
}

/// One full deterministic simulation over a fresh world.
///
/// Returns the per-step [`world_state_hash`] sequence; with
/// `profiler: Profiler` the caller decides enabled vs disabled.
fn run_simulation(
    steps: u32,
    mut profiler: Profiler,
) -> anyhow::Result<(Vec<u64>, Profiler, World)> {
    let mut world = World::new();
    let mut physics = PhysicsWorld::new(gens_math::Vec3::new(0.0, -9.81, 0.0));

    // Floor: 100 m wide cuboid — the demo never reaches the edge.
    let (_floor_body, _) = physics.create_body(
        BodySpec::fixed_at(gens_math::Vec3::new(0.0, -0.5, 0.0)),
        ColliderSpec {
            shape: ColliderShape::Cuboid(gens_math::Vec3::new(50.0, 0.5, 50.0)),
            friction: 0.8,
            ..Default::default()
        },
    );

    // Three dynamic balls with distinct starts; CCD on (fast movers must
    // not tunnel; also the phase-3 CCD regressions stay covered here).
    let mut handles = Vec::new();
    for (name, start) in [
        ("Ball_A", [0.0, 0.5, 0.0]),
        ("Ball_B", [-1.0, 0.5, 0.0]),
        ("Ball_C", [1.0, 0.5, 0.0]),
    ]
    .into_iter()
    {
        let (handle, _) = physics.create_body(
            BodySpec {
                ccd: true,
                linear_damping: 0.8,
                ..BodySpec::dynamic_at(gens_math::Vec3::new(start[0], start[1], start[2]))
            },
            ColliderSpec {
                shape: ColliderShape::Ball(0.5),
                friction: 0.2,
                ..Default::default()
            },
        );
        handles.push(handle);
        world.spawn((
            gens_ecs::Name::new(name),
            gens_ecs::Transform::IDENTITY,
            gens_ecs::Active,
            PhysicsBody { handle },
        ));
    }
    world.insert_resource(PhysicsResource { world: physics });

    let mut schedules = EngineSchedules::new();
    schedules.update.add_systems(sync_transforms_from_physics);

    let script = impulse_script();
    let mut hashes: Vec<u64> = Vec::with_capacity(steps as usize);

    let source = gens_core::time::FixedSource::new(Duration::from_secs_f64(1.0 / 60.0));
    let mut game_loop =
        gens_core::game_loop::GameLoop::new(gens_core::game_loop::LoopConfig::default(), source);
    let mut step_index = 0u32;
    for frame in 0..steps {
        profiler.begin_frame(frame as u64);
        game_loop.run_frame(|_| {
            // Control forces: script entries for this step, then one fixed
            // solver step. Spans observe; nothing measured feeds back.
            profiler.time("physics_step", |p| {
                for &(at, ball, imp) in &script {
                    if at == step_index {
                        let mut res = world.resource_mut::<PhysicsResource>();
                        res.world.apply_impulse(
                            handles[ball],
                            gens_math::Vec3::new(imp[0], imp[1], imp[2]),
                        );
                        let _ = p; // nested spans would go here
                    }
                }
                let mut res = world.resource_mut::<PhysicsResource>();
                res.world.step();
            });
            instrumented_frame(&mut schedules, &mut world, &mut profiler);
            hashes.push(world_state_hash(&world));
            step_index += 1;
        });
        profiler.end_frame();
    }

    Ok((hashes, profiler, world))
}

/// The evidence document: proof + profile report + memory snapshot.
#[derive(serde::Serialize)]
struct ProfileHello {
    format: &'static str,
    steps: u32,
    entities: usize,
    proof: Proof,
    profile: gens_profiler::ProfileReport,
    memory: gens_profiler::MemorySnapshot,
}

#[derive(serde::Serialize)]
struct Proof {
    /// Hash sequence length per run.
    hashes_per_run: usize,
    /// True when the enabled and disabled runs hashed identically.
    identical: bool,
    /// The final state hash (cross-run comparability anchor).
    final_hash: u64,
    /// The profiled run's span names, sorted (sanity evidence).
    span_names: Vec<String>,
    /// Invariant violations on the final world (must be empty).
    violations: Vec<gens_runtime::debug::Violation>,
}

fn main() -> anyhow::Result<()> {
    let mut steps: u32 = 600;
    let mut out = PathBuf::from("profile_hello.json");
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
    info!("Gens engine - profiler hello (ADR-0027 observation-only proof)");
    info!(steps, ?out, %filter, "run configuration");

    // --- The proof: profiler OFF vs ON must hash identically ---------------
    let (hashes_off, _, _) = run_simulation(steps, Profiler::disabled())?;
    let (hashes_on, profiler, world) = run_simulation(steps, Profiler::enabled())?;
    anyhow::ensure!(
        hashes_off.len() == hashes_on.len() && hashes_off.len() == steps as usize,
        "both runs must hash every step"
    );
    for (i, (a, b)) in hashes_off.iter().zip(hashes_on.iter()).enumerate() {
        anyhow::ensure!(
            a == b,
            "ADR-0027 violated: state hash diverges at step {i} ({a:x} vs {b:x})"
        );
    }
    info!(
        steps = hashes_on.len(),
        "determinism: profiler-off and profiler-on runs byte-identical"
    );

    // --- Report + memory evidence ------------------------------------------
    let report = gens_profiler::report::report_from_history(profiler.history());
    let span_names: Vec<String> = report.spans.iter().map(|s| s.name.clone()).collect();
    for name in ["physics_step", "ecs_update", "ecs_post_update"] {
        let stats = report
            .spans
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("span {name} missing from the report"));
        anyhow::ensure!(
            stats.count == steps as u64,
            "span {name} must fire once per step"
        );
        anyhow::ensure!(stats.us.median_us > 0, "span {name} must have real work");
    }
    let memory = memory_stats().snapshot();

    // --- Debugging pillar evidence: the final world must be clean ----------
    let violations = validate_world(&world, None);
    anyhow::ensure!(
        violations.is_empty(),
        "the final world must validate clean, got {violations:?}"
    );
    info!(
        entities = world.iter_entities().count(),
        "final world validates clean"
    );

    let doc = ProfileHello {
        format: "gens-profile-hello-1",
        steps,
        entities: 3,
        proof: Proof {
            hashes_per_run: hashes_on.len(),
            identical: true,
            final_hash: *hashes_on.last().expect("at least one step"),
            span_names,
            violations,
        },
        profile: report,
        memory,
    };
    let json = serde_json::to_string_pretty(&doc)?;
    std::fs::write(&out, &json).with_context(|| format!("writing {}", out.display()))?;
    info!(bytes = json.len(), path = %out.display(), "profiler hello evidence written");
    Ok(())
}
