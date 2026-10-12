//! Worldgen streaming proof (Phase 9, ADR-0031): a procedurally
//! generated chunked world streams in and out around a moving focus,
//! and every layer of it must be deterministic.
//!
//! Four proofs over a 3x3-chunk Chebyshev disc (radius 1, 16-unit
//! chunks) driven through the ECS:
//!
//! - **Generation order independence**: the 3 focus chunks are
//!   generated in row-major, reverse and fixed-permutation orders —
//!   every chunk's terrain hash must be identical across orders.
//! - **Streaming determinism**: the same focus path walked twice (fresh
//!   world + fresh streamer each time) must produce identical
//!   load/unload sequences and identical per-step world hashes.
//! - **Restart determinism**: two straight-through runs replay
//!   byte-identically (the house proof shape since Phase 1).
//! - **Terrain persistence**: a prop spawned on generated terrain keeps
//!   its terrain-derived y for the rest of the run (the world does not
//!   mutate under a deterministic path).
//!
//! ```text
//! cargo run -p gens-runtime --example worldgen_hello -- --steps 300 --out worldgen_hello.json
//! ```

use gens_core::anyhow;
use gens_core::anyhow::Context;
use gens_core::logging::{info, init};
use gens_ecs::{Active, EngineSchedules, Name, Transform};
use gens_math::vec3;
use gens_worldgen::chunk::{ChunkCoord, WorldGen, WorldGenConfig};
use gens_worldgen::streaming::ChunkStreamer;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Fixed step every run uses.
const FIXED_DT: f32 = 1.0 / 60.0;

/// Focus walk: starts inside chunk (0,0) and marches +x at 0.05
/// units/step — 300 steps = 15 units, crossing one chunk boundary.
fn focus_at(step: u32) -> [f32; 2] {
    [8.0 + step as f32 * 0.05, 8.0]
}

/// What one run records.
#[derive(Debug)]
struct RunLedger {
    hashes: Vec<u64>,
    streaming: Vec<(u32, String)>, // (step, "load (x,z)" / "unload (x,z)")
    prop_y: Vec<(u32, f32)>,       // sampled prop height persistence
}

/// Spawns one chunk's content into the world; returns prop entities.
fn apply_load(
    world: &mut gens_ecs::ecs::world::World,
    generator: &WorldGen,
    coord: ChunkCoord,
) -> anyhow::Result<Vec<gens_ecs::ecs::entity::Entity>> {
    let content = generator.generate(coord).context("chunk generation")?;
    world.spawn((
        Name::new(format!("Chunk({},{})", coord.x, coord.z)),
        Transform::IDENTITY,
        Active,
    ));
    let mut props = Vec::with_capacity(content.scatter.len());
    for (i, p) in content.scatter.iter().enumerate() {
        let e = world
            .spawn((
                Name::new(format!("Prop({},{},{})", coord.x, coord.z, i)),
                Transform::from_position(vec3(p.position[0], p.position[1], p.position[2])),
                Active,
            ))
            .id();
        props.push(e);
    }
    Ok(props)
}

fn run_simulation(steps: u32, generator: &WorldGen) -> anyhow::Result<RunLedger> {
    let mut world = gens_ecs::ecs::world::World::new();
    let mut schedules = EngineSchedules::new();
    let mut streamer = ChunkStreamer::new(1);
    let mut loaded: BTreeMap<ChunkCoord, Vec<gens_ecs::ecs::entity::Entity>> = BTreeMap::new();
    let mut ledger = RunLedger {
        hashes: Vec::with_capacity(steps as usize),
        streaming: Vec::new(),
        prop_y: Vec::new(),
    };
    // A prop to watch: the first spawned one, recorded once available.
    let mut watch: Option<gens_ecs::ecs::entity::Entity> = None;

    for step in 1..=steps {
        let [fx, fz] = focus_at(step);
        let plan = streamer.update(generator.coord_of(fx, fz));
        for coord in &plan.load {
            let props = apply_load(&mut world, generator, *coord)?;
            // Watch a prop from chunk (0,0): with radius 1 and this
            // +x walk, (0,0) stays loaded for the entire run, so the
            // persistence check observes terrain that never streams out.
            if *coord == ChunkCoord::new(0, 0) && watch.is_none() {
                watch = props.first().copied();
            }
            ledger
                .streaming
                .push((step, format!("load ({},{})", coord.x, coord.z)));
            loaded.insert(*coord, props);
        }
        for coord in &plan.unload {
            if let Some(props) = loaded.remove(coord) {
                for e in props {
                    let _ = world.despawn(e);
                }
            }
            ledger
                .streaming
                .push((step, format!("unload ({},{})", coord.x, coord.z)));
        }
        schedules.run_frame(&mut world);
        ledger
            .hashes
            .push(gens_runtime::profile::world_state_hash(&world));
        if step.is_multiple_of(60)
            && let Some(e) = watch
            && let Some(t) = world.get::<Transform>(e)
        {
            ledger.prop_y.push((step, t.position.y));
        }
    }
    Ok(ledger)
}

#[derive(Serialize)]
struct Report {
    format: &'static str,
    environment: Env,
    steps: u32,
    fixed_dt: f64,
    generator: GenInfo,
    order_independence: OrderProof,
    streaming_ledger_a: Vec<(u32, String)>,
    run_a: RunSummary,
    run_b: RunSummary,
    proof: Proof,
    advisory: &'static str,
}

#[derive(Serialize)]
struct Env {
    os: &'static str,
    arch: &'static str,
    profile: &'static str,
    package_version: &'static str,
}

#[derive(Serialize)]
struct GenInfo {
    seed: u64,
    samples_per_chunk: u32,
    cell_size: f32,
    frequency: f32,
    octaves: u32,
    chunk_size: f32,
    density_threshold: f32,
}

#[derive(Serialize)]
struct OrderProof {
    row_major: Vec<String>,
    reverse: Vec<String>,
    permutation: Vec<String>,
    identical: bool,
}

#[derive(Serialize)]
struct RunSummary {
    final_hash: String,
    hashes_every_60: Vec<String>,
    load_events: usize,
    unload_events: usize,
}

#[derive(Serialize)]
struct Proof {
    streaming_identical: bool,
    restart_identical: bool,
    terrain_persistent: bool,
}

fn hex(h: u64) -> String {
    format!("{h:016x}")
}

fn summarize(ledger: &RunLedger) -> RunSummary {
    RunSummary {
        final_hash: hex(*ledger.hashes.last().expect("non-empty")),
        hashes_every_60: ledger
            .hashes
            .iter()
            .enumerate()
            .filter(|(i, _)| (i + 1).is_multiple_of(60))
            .map(|(_, h)| hex(*h))
            .collect(),
        load_events: ledger
            .streaming
            .iter()
            .filter(|(_, e)| e.starts_with("load"))
            .count(),
        unload_events: ledger
            .streaming
            .iter()
            .filter(|(_, e)| e.starts_with("unload"))
            .count(),
    }
}

fn main() -> anyhow::Result<()> {
    let mut steps: u32 = 300;
    let mut out = PathBuf::from("worldgen_hello.json");
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
    info!("Gens engine - worldgen streaming proof (ADR-0031)");
    info!(steps, ?out, %filter, "run configuration");

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
    .context("generator config")?;

    // --- Proof 1: generation order independence over the walked chunks
    // (the x column the focus crosses during a 300-step walk).
    let interesting = [
        ChunkCoord::new(-1, 0),
        ChunkCoord::new(0, 0),
        ChunkCoord::new(1, 0),
        ChunkCoord::new(2, 0),
    ];
    // Each pass collects coord -> terrain hash; the maps must be equal
    // (same coord, same hash) regardless of generation order.
    let mut row_major = std::collections::BTreeMap::new();
    for c in interesting {
        row_major.insert(c, generator.generate(c).expect("gen").mesh.hash);
    }
    let mut reverse = std::collections::BTreeMap::new();
    for c in interesting.iter().rev() {
        reverse.insert(*c, generator.generate(*c).expect("gen").mesh.hash);
    }
    let mut permutation = std::collections::BTreeMap::new();
    for i in [2usize, 0, 3, 1] {
        let c = interesting[i];
        permutation.insert(c, generator.generate(c).expect("gen").mesh.hash);
    }
    let order_identical = row_major == reverse && row_major == permutation;

    // --- Proofs 2-4: two full runs, byte-identical everywhere.
    let run_a = run_simulation(steps, &generator)?;
    let run_b = run_simulation(steps, &generator)?;
    let streaming_identical = run_a.streaming == run_b.streaming;
    let restart_identical = run_a.hashes == run_b.hashes;
    let terrain_persistent = run_a.prop_y.windows(2).all(|w| w[0].1 == w[1].1);

    anyhow::ensure!(
        order_identical,
        "ORDER PROOF FAILED: generation order changed content"
    );
    anyhow::ensure!(
        streaming_identical,
        "STREAMING PROOF FAILED: load/unload sequences diverged"
    );
    anyhow::ensure!(
        restart_identical,
        "RESTART PROOF FAILED: world hashes diverged"
    );
    anyhow::ensure!(
        terrain_persistent,
        "TERRAIN PROOF FAILED: prop heights moved"
    );

    let report = Report {
        format: "gens-worldgen-1",
        environment: Env {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            package_version: env!("CARGO_PKG_VERSION"),
        },
        steps,
        fixed_dt: f64::from(FIXED_DT),
        generator: GenInfo {
            seed: generator.config().seed,
            samples_per_chunk: generator.config().samples_per_chunk,
            cell_size: generator.config().cell_size,
            frequency: generator.config().frequency,
            octaves: generator.config().octaves,
            chunk_size: generator.chunk_size(),
            density_threshold: generator.config().density_threshold,
        },
        order_independence: OrderProof {
            row_major: row_major.values().map(|h| hex(*h)).collect(),
            reverse: reverse.values().map(|h| hex(*h)).collect(),
            permutation: permutation.values().map(|h| hex(*h)).collect(),
            identical: order_identical,
        },
        streaming_ledger_a: run_a.streaming.clone(),
        run_a: summarize(&run_a),
        run_b: summarize(&run_b),
        proof: Proof {
            streaming_identical,
            restart_identical,
            terrain_persistent,
        },
        advisory: "determinism proof, not performance; terrain is procedural, not art-directed",
    };
    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(&out, json).with_context(|| format!("write {}", out.display()))?;
    info!(
        steps,
        final_hash = %run_a.summary_final(),
        "worldgen streaming proof complete (order-independent, streaming- and restart-identical)"
    );
    println!("worldgen proof written to {}", out.display());
    Ok(())
}

impl RunLedger {
    fn summary_final(&self) -> String {
        hex(*self.hashes.last().expect("non-empty"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_path_crosses_one_boundary_in_300_steps() {
        // 8.0 + 300*0.05 = 23.0: chunks 0 then 1 (boundary at 16.0).
        let start = gen_chunk(focus_at(1));
        let end = gen_chunk(focus_at(300));
        assert_eq!(start, 0);
        assert_eq!(end, 1);
    }

    fn gen_chunk(f: [f32; 2]) -> i32 {
        WorldGen::from_config(WorldGenConfig {
            seed: 1,
            samples_per_chunk: 16,
            cell_size: 1.0,
            frequency: 0.06,
            octaves: 4,
            lacunarity: 2.0,
            gain: 0.5,
            density_threshold: 0.55,
        })
        .expect("cfg")
        .coord_of(f[0], f[1])
        .x
    }
}
