//! Gens step benchmarks (Phase 8): honest, environment-disclosed numbers.
//!
//! Closes R-004's debt ("perf claims wait for Phase 8 benchmarks") the
//! honest way: the numbers are real wall-clock medians over a documented
//! sampling methodology, tagged with the environment they were measured
//! in, and explicitly advisory — CI runners are shared machines, so only
//! comparisons WITHIN one environment class are meaningful (ADR-0029).
//! Nothing here feeds a benchmark result back into the simulation.
//!
//! Methodology (fixed, documented in ADR-0029):
//! - one SAMPLE = wall-clock time of `--steps-per-sample` consecutive
//!   fixed steps of the whole simulation (scripts -> physics ->
//!   propagation, the real composition),
//! - `--warmup` unrecorded samples first,
//! - `--samples` recorded samples; stats over the sample durations.
//!
//! Phase 10 grows the scenario set so the newer pillars are *measured*,
//! not just proven (the Phase 9 handoff ask): `worldgen_stream_r2`
//! samples differential chunk streaming (plans + generation + meshing)
//! and `cinematic_camera_hero` samples the director composition
//! (advance -> sample -> validate -> write through the live ECS). Same
//! sampling methodology, same environment disclosure, same advisory.
//!
//! ```text
//! cargo run -p gens-runtime --example bench_step -- --samples 30 --out bench.json
//! ```

use gens_cinematics::{
    CameraCut, Channel, CinematicPlayer, FloatKey, FloatTrack, Interpolation, LoopMode, QuatKey,
    QuatTrack, Timeline, Vec3Key, Vec3Track,
};
use gens_core::anyhow;
use gens_core::anyhow::Context;
use gens_core::logging::{info, init};
use gens_ecs::EngineSchedules;
use gens_ecs::ecs::world::World;
use gens_ecs::{Active, Name, Transform};
use gens_math::{Quat, Vec3};
use gens_physics::{
    BodySpec, ColliderShape, ColliderSpec, PhysicsBody, PhysicsResource, PhysicsWorld,
    sync_transforms_from_physics,
};
use gens_runtime::cinematic::{CinematicStage, advance_and_apply};
use gens_scripting::{ScriptHost, run_scripts};
use gens_worldgen::chunk::{WorldGen, WorldGenConfig};
use gens_worldgen::streaming::ChunkStreamer;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// Sample statistics in milliseconds (the gens-profiler quartile
/// convention: median = sorted[len/2], p99 = sorted[len*99/100]).
#[derive(Debug, Clone, Serialize)]
pub struct SampleStats {
    /// Scenario name.
    pub name: String,
    /// Recorded samples.
    pub samples: usize,
    /// Fixed steps per sample.
    pub steps_per_sample: usize,
    /// Unrecorded warmup samples.
    pub warmup: usize,
    /// Entities in the scenario.
    pub entities: usize,
    /// Scenario stats (ms).
    pub min_ms: f64,
    pub median_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
    /// Population stddev (ms).
    pub stddev_ms: f64,
}

/// The bench document: format tag + environment + results.
#[derive(Debug, Serialize)]
pub struct BenchReport {
    /// Format tag (`gens-bench-1`).
    pub format: &'static str,
    /// Measured environment (R-004 disclosure).
    pub env: BenchEnv,
    /// Per-scenario stats.
    pub benches: Vec<SampleStats>,
}

/// Environment disclosure.
#[derive(Debug, Serialize)]
pub struct BenchEnv {
    /// OS (`std::env::consts::OS`).
    pub os: &'static str,
    /// Architecture.
    pub arch: &'static str,
    /// Debug or release build (`cfg!(debug_assertions)`).
    pub profile: &'static str,
    /// Logical CPUs available to the process.
    pub parallelism: usize,
    /// Adapter class: these benches are CPU-only by design.
    pub adapter: &'static str,
    /// Package version.
    pub version: &'static str,
    /// The advisory note (R-004): what these numbers may be used for.
    pub advisory: &'static str,
}

/// One deterministic scenario: `bodies` dynamic balls over a floor plus
/// `scripts` entities moving along x, stepped `steps` times.
///
/// Fresh world AND fresh schedules per call: bevy 0.20 binds systems to
/// the World they initialize against ("A System cannot be used with
/// Worlds other than the one it was initialized with") — reusing a
/// schedule across worlds panics. Schedule construction is microseconds,
/// negligible against 60+ simulated steps, and it is inside the timed
/// region honestly (documented in the sample stats).
fn run_scenario(bodies: usize, scripts: usize, steps: usize) -> u64 {
    let mut schedules = EngineSchedules::new();
    schedules.update.add_systems(sync_transforms_from_physics);
    let mut world = World::new();
    let mut physics = PhysicsWorld::new(gens_math::Vec3::new(0.0, -9.81, 0.0));
    let (_floor, _) = physics.create_body(
        BodySpec::fixed_at(gens_math::Vec3::new(0.0, -0.5, 0.0)),
        ColliderSpec {
            shape: ColliderShape::Cuboid(gens_math::Vec3::new(50.0, 0.5, 50.0)),
            friction: 0.8,
            ..Default::default()
        },
    );
    for i in 0..bodies {
        let x = (i % 10) as f32 - 4.5;
        let z = (i / 10) as f32 - 4.5;
        let (handle, _) = physics.create_body(
            BodySpec {
                ccd: false,
                linear_damping: 0.2,
                ..BodySpec::dynamic_at(gens_math::Vec3::new(x, 1.0 + (i % 5) as f32, z))
            },
            ColliderSpec {
                shape: ColliderShape::Ball(0.5),
                friction: 0.3,
                ..Default::default()
            },
        );
        world.spawn((
            gens_ecs::Name::new(format!("ball_{i}")),
            gens_ecs::Transform::IDENTITY,
            gens_ecs::Active,
            PhysicsBody { handle },
        ));
    }
    world.insert_resource(PhysicsResource { world: physics });

    if scripts > 0 {
        let mut host = ScriptHost::new();
        host.register(
            "scripts/move.rhai",
            "fn update() { set_pos(pos_x() + dt(), pos_y(), pos_z()); }",
        )
        .expect("bench script compiles");
        world.insert_resource(host);
        for i in 0..scripts {
            world.spawn((
                gens_ecs::Name::new(format!("scripted_{i}")),
                gens_ecs::Transform::from_position(gens_math::Vec3::new(0.0, 2.0, 0.0)),
                gens_ecs::Active,
                gens_scripting::ScriptComponent::new("scripts/move.rhai"),
            ));
        }
    }

    let dt = 1.0 / 60.0;
    let mut checksum: u64 = 0;
    for step in 0..steps {
        let t = step as f64 * dt;
        if scripts > 0 {
            run_scripts(&mut world, dt, t);
        }
        world.resource_mut::<PhysicsResource>().world.step();
        schedules.run_frame(&mut world);
        // A cheap deterministic readback so the optimizer cannot delete
        // the work: fold the entity count (observability, not simulation).
        checksum = checksum.wrapping_add(world.iter_entities().count() as u64);
    }
    checksum
}

/// One deterministic worldgen streaming step: the focus alternates
/// between two chunk coords two Chebyshev radii apart every 10 steps,
/// so both load and unload traffic churn for the whole run.
fn run_worldgen_scenario(steps: usize) -> u64 {
    let generator = WorldGen::from_config(WorldGenConfig {
        seed: 0x9E37_79B9_7F4A_7C15,
        samples_per_chunk: 16,
        cell_size: 1.0,
        frequency: 0.06,
        octaves: 4,
        lacunarity: 2.0,
        gain: 0.5,
        density_threshold: 0.55,
    })
    .expect("bench generator config is valid");
    let mut streamer = ChunkStreamer::new(2);
    let mut checksum: u64 = 0;
    for step in 0..steps {
        // 2 chunks apart at cell 1.0 × 16 samples = 32 world units:
        // radius-2 discs at the two foci overlap, so the differential
        // plan moves ~9 chunks per flip — real generation traffic.
        let fx = if (step / 10) % 2 == 0 { 0.0 } else { 32.0 };
        let plan = streamer.update(generator.coord_of(fx, 0.0));
        for coord in &plan.load {
            let content = generator.generate(*coord).expect("bench chunk generation");
            checksum = checksum.wrapping_add(content.mesh.hash);
        }
        for coord in &plan.unload {
            checksum =
                checksum.wrapping_add((coord.x as i64 as u64) ^ ((coord.z as i64 as u64) << 32));
        }
    }
    checksum
}

/// The director composition through the live ECS: advance → sample →
/// validate → write per fixed step, then a deterministic readback fold.
/// Every key value is an f32 literal (the platform-independence rule).
fn run_cinematic_scenario(steps: usize) -> u64 {
    let timeline = Arc::new(
        Timeline::builder(2.0)
            .camera(CameraCut {
                target: "camera".to_string(),
                hard_cut: true,
                blend_in: 0.0,
            })
            .float_track(
                "camera",
                Channel::Fov,
                FloatTrack::new(
                    vec![
                        FloatKey {
                            time: 0.0,
                            value: 60.0,
                        },
                        FloatKey {
                            time: 2.0,
                            value: 45.0,
                        },
                    ],
                    Interpolation::Smoothstep,
                )
                .expect("bench fov track"),
            )
            .expect("bench fov binding")
            .vec3_track(
                "hero",
                Channel::Translation,
                Vec3Track::new(
                    vec![
                        Vec3Key {
                            time: 0.0,
                            value: Vec3::new(0.0, 1.0, 0.0),
                        },
                        Vec3Key {
                            time: 2.0,
                            value: Vec3::new(4.0, 1.0, 0.0),
                        },
                    ],
                    Interpolation::Linear,
                )
                .expect("bench translation track"),
            )
            .expect("bench translation binding")
            .quat_track(
                "hero",
                Channel::Rotation,
                QuatTrack::new(
                    vec![
                        QuatKey {
                            time: 0.0,
                            value: Quat::from_xyzw(0.0, 0.0, 0.0, 1.0),
                        },
                        QuatKey {
                            time: 2.0,
                            value: Quat::from_xyzw(0.0, 0.707_106_77, 0.0, 0.707_106_77),
                        },
                    ],
                    Interpolation::Linear,
                )
                .expect("bench rotation track"),
            )
            .expect("bench rotation binding")
            .event(1.0, "bench_event")
            .expect("bench event")
            .build()
            .expect("bench timeline builds"),
    );

    let mut world = World::new();
    world.spawn((Name::new("Hero"), Transform::IDENTITY, Active));
    let camera = world
        .spawn((Name::new("Camera"), Transform::IDENTITY, Active))
        .id();
    // Name-query lookup (iter_entities order is NOT deterministic).
    let hero = {
        let mut q = world.query::<(&Name, gens_ecs::ecs::entity::Entity)>();
        q.iter(&world)
            .find(|(n, _)| n.0 == "Hero")
            .map(|(_, e)| e)
            .expect("hero exists")
    };
    let mut player = CinematicPlayer::new(timeline);
    player.set_loop_mode(LoopMode::Loop);
    let mut stage = CinematicStage::new(
        player,
        [("hero".to_string(), hero), ("camera".to_string(), camera)],
    );
    stage.player.play();

    let dt = 1.0 / 60.0;
    let mut checksum: u64 = 0;
    for _ in 0..steps {
        let outcome =
            advance_and_apply(&mut stage, &mut world, dt).expect("bench cinematic step applies");
        checksum = checksum.wrapping_add(outcome.events.len() as u64);
        let mut q = world.query::<(&Name, &Transform)>();
        if let Some((_, transform)) = q.iter(&world).find(|(n, _)| n.0 == "Hero") {
            checksum = checksum.wrapping_add(u64::from(transform.position.x.to_bits()));
        }
    }
    checksum
}

fn stats_of(
    name: &str,
    entities: usize,
    steps_per_sample: usize,
    warmup: usize,
    samples: &[f64],
) -> SampleStats {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("no NaN durations"));
    let n = sorted.len();
    let mean = sorted.iter().sum::<f64>() / n as f64;
    let variance = sorted.iter().map(|s| (s - mean) * (s - mean)).sum::<f64>() / n as f64;
    SampleStats {
        name: name.to_string(),
        samples: n,
        steps_per_sample,
        warmup,
        entities,
        min_ms: sorted[0],
        median_ms: sorted[n / 2],
        p99_ms: sorted[(n * 99 / 100).min(n - 1)],
        max_ms: sorted[n - 1],
        mean_ms: mean,
        stddev_ms: variance.sqrt(),
    }
}

/// Samples one scenario: `warmup` unrecorded samples, then `samples`
/// recorded ones, each of `steps_per_sample` steps.
fn bench_scenario(
    name: &str,
    bodies: usize,
    scripts: usize,
    steps_per_sample: usize,
    warmup: usize,
    samples: usize,
) -> SampleStats {
    let entities = bodies + scripts + 1 /* floor */;
    for _ in 0..warmup {
        let _ = run_scenario(bodies, scripts, steps_per_sample);
    }
    let mut durations: Vec<f64> = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        let checksum = run_scenario(bodies, scripts, steps_per_sample);
        let elapsed = start.elapsed();
        assert!(checksum > 0, "scenario must do real work");
        durations.push(elapsed.as_secs_f64() * 1000.0);
    }
    stats_of(name, entities, steps_per_sample, warmup, &durations)
}

/// Samples the worldgen streaming scenario (same warmup/sample pattern
/// as [`bench_scenario`]; the streamed composition is CPU-only by
/// design, so no adapter check is needed).
fn bench_worldgen(
    name: &str,
    steps_per_sample: usize,
    warmup: usize,
    samples: usize,
) -> SampleStats {
    for _ in 0..warmup {
        let _ = run_worldgen_scenario(steps_per_sample);
    }
    let mut durations: Vec<f64> = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        let checksum = run_worldgen_scenario(steps_per_sample);
        let elapsed = start.elapsed();
        assert!(checksum > 0, "scenario must do real work");
        durations.push(elapsed.as_secs_f64() * 1000.0);
    }
    // entities semantics: peak desired-set size (radius 2 = 5×5 chunks).
    stats_of(name, 25, steps_per_sample, warmup, &durations)
}

/// Samples the cinematic director scenario (same warmup/sample
/// pattern; `advance_and_apply` runs through the live ECS world).
fn bench_cinematic(
    name: &str,
    steps_per_sample: usize,
    warmup: usize,
    samples: usize,
) -> SampleStats {
    for _ in 0..warmup {
        let _ = run_cinematic_scenario(steps_per_sample);
    }
    let mut durations: Vec<f64> = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        let checksum = run_cinematic_scenario(steps_per_sample);
        let elapsed = start.elapsed();
        assert!(checksum > 0, "scenario must do real work");
        durations.push(elapsed.as_secs_f64() * 1000.0);
    }
    stats_of(name, 2, steps_per_sample, warmup, &durations)
}

fn main() -> anyhow::Result<()> {
    let mut samples: usize = 30;
    let mut steps_per_sample: usize = 100;
    let mut warmup: usize = 5;
    let mut out = PathBuf::from("bench_step.json");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--samples" => {
                samples = args
                    .next()
                    .context("--samples needs a value")?
                    .parse()
                    .context("--samples must be a number")?;
            }
            "--steps-per-sample" => {
                steps_per_sample = args
                    .next()
                    .context("--steps-per-sample needs a value")?
                    .parse()
                    .context("--steps-per-sample must be a number")?;
            }
            "--warmup" => {
                warmup = args
                    .next()
                    .context("--warmup needs a value")?
                    .parse()
                    .context("--warmup must be a number")?;
            }
            "--out" => out = PathBuf::from(args.next().context("--out needs a value")?),
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    let filter = init().unwrap_or_else(|_| "(logging already initialized)".to_string());
    info!("Gens engine - step benchmarks (advisory, environment-disclosed)");
    info!(samples, steps_per_sample, warmup, ?out, %filter, "run configuration");

    let parallelism = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let env = BenchEnv {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        parallelism,
        adapter: "cpu-only (no GPU in the timed path)",
        version: env!("CARGO_PKG_VERSION"),
        advisory: "advisory only: shared runners; compare within one environment class, never across",
    };

    // The core loop, three load points: light, medium, script-heavy.
    let benches = vec![
        bench_scenario(
            "physics_10_bodies",
            10,
            0,
            steps_per_sample,
            warmup,
            samples,
        ),
        bench_scenario(
            "physics_50_bodies",
            50,
            0,
            steps_per_sample,
            warmup,
            samples,
        ),
        bench_scenario(
            "scripts_20_plus_physics_20",
            20,
            20,
            steps_per_sample,
            warmup,
            samples,
        ),
        bench_worldgen("worldgen_stream_r2", steps_per_sample, warmup, samples),
        bench_cinematic("cinematic_camera_hero", steps_per_sample, warmup, samples),
    ];

    for b in &benches {
        info!(
            name = %b.name,
            median_ms = format!("{:.3}", b.median_ms),
            p99_ms = format!("{:.3}", b.p99_ms),
            stddev_ms = format!("{:.3}", b.stddev_ms),
            "bench result"
        );
    }

    let report = BenchReport {
        format: "gens-bench-1",
        env,
        benches,
    };
    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(&out, &json).with_context(|| format!("writing {}", out.display()))?;
    info!(bytes = json.len(), path = %out.display(), "bench evidence written");
    Ok(())
}
