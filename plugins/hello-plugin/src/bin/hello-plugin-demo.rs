//! The demo binary: apply `HelloPlugin` to a fresh world, run the
//! engine's schedules for N fixed frames, and prove determinism — two
//! in-process runs must land the orb on the same bits — plus a report
//! recording what the plugin API applied.
//!
//! ```text
//! cargo run --bin hello-plugin-demo -- --frames 120 --out plugin_hello.json
//! ```
//!
//! Exit code 0 = proof passed; nonzero = the plugin API failed the
//! proof (a downstream CI gate, not a warning).

use bevy_ecs::prelude::*;
use gens_ecs::EngineSchedules;
use gens_hello_plugin::{HelloPlugin, Orb, OrbitState};
use gens_plugin::PluginRegistry;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize)]
struct Report {
    format: &'static str,
    environment: Env,
    frames: u32,
    applied: Vec<String>,
    registry_rejected_duplicate: bool,
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
}

#[derive(Serialize)]
struct RunSummary {
    /// Final orb position as raw bit patterns (determinism yardstick).
    orb_position_bits: [String; 3],
    orb_angle_bits: String,
}

#[derive(Serialize)]
struct Proof {
    in_process_identical: bool,
    duplicate_name_rejected: bool,
}

fn hex(f: f32) -> String {
    format!("{:08x}", f.to_bits())
}

fn run(frames: u32, radius: f32) -> [f32; 4] {
    let mut world = World::new();
    let mut schedules = EngineSchedules::new();
    let mut registry = PluginRegistry::new();
    registry
        .add(HelloPlugin { radius })
        .expect("fresh registry accepts the plugin");
    let applied = registry.apply(&mut world, &mut schedules).expect("apply");
    assert!(applied.contains("gens.hello-orbit"));

    for _ in 0..frames {
        schedules.run_frame(&mut world);
    }

    let mut q = world.query::<(&gens_ecs::Transform, &Orb)>();
    let (transform, _) = q
        .iter(&world)
        .next()
        .expect("the plugin spawned exactly one orb");
    let state = world.resource::<OrbitState>();
    [
        transform.position.x,
        transform.position.y,
        transform.position.z,
        state.angle,
    ]
}

fn main() -> std::process::ExitCode {
    let mut frames: u32 = 120;
    let mut out = PathBuf::from("plugin_hello.json");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--frames" => match args.next().map(|v| v.parse::<u32>()) {
                Some(Ok(n)) => frames = n,
                _ => {
                    eprintln!("--frames needs a number");
                    return std::process::ExitCode::FAILURE;
                }
            },
            "--out" => match args.next() {
                Some(p) => out = PathBuf::from(p),
                None => {
                    eprintln!("--out needs a path");
                    return std::process::ExitCode::FAILURE;
                }
            },
            other => {
                eprintln!("unknown argument: {other} (use --frames N, --out PATH)");
                return std::process::ExitCode::FAILURE;
            }
        }
    }

    // Duplicate plugin names must be loud errors (registry discipline).
    let mut dup_registry = PluginRegistry::new();
    let first = dup_registry.add(HelloPlugin { radius: 3.0 });
    let second = dup_registry.add(HelloPlugin { radius: 4.0 });
    let duplicate_name_rejected = first.is_ok() && second.is_err();
    if !duplicate_name_rejected {
        eprintln!("PROOF FAILED: the registry accepted a duplicate plugin name");
        return std::process::ExitCode::FAILURE;
    }

    // Two in-process runs must be bit-identical.
    let run_a = run(frames, 3.0);
    let run_b = run(frames, 3.0);
    let in_process_identical = run_a == run_b;
    if !in_process_identical {
        eprintln!("PROOF FAILED: two runs diverged: {run_a:?} vs {run_b:?}");
        return std::process::ExitCode::FAILURE;
    }

    let report = Report {
        format: "gens-plugin-hello-1",
        environment: Env {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
        },
        frames,
        applied: vec!["gens.hello-orbit".to_string()],
        registry_rejected_duplicate: duplicate_name_rejected,
        run_a: RunSummary {
            orb_position_bits: [hex(run_a[0]), hex(run_a[1]), hex(run_a[2])],
            orb_angle_bits: hex(run_a[3]),
        },
        run_b: RunSummary {
            orb_position_bits: [hex(run_b[0]), hex(run_b[1]), hex(run_b[2])],
            orb_angle_bits: hex(run_b[3]),
        },
        proof: Proof {
            in_process_identical,
            duplicate_name_rejected,
        },
        advisory: "out-of-tree plugin API proof; the engine workspace does not depend on this crate",
    };
    let json = match serde_json::to_string_pretty(&report) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("report serialization failed: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::write(&out, json) {
        eprintln!("write {} failed: {e}", out.display());
        return std::process::ExitCode::FAILURE;
    }
    println!(
        "plugin proof written to {} (orb position bits: {:?})",
        out.display(),
        report.run_a.orb_position_bits
    );
    std::process::ExitCode::SUCCESS
}
