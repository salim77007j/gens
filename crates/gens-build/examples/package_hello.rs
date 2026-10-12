//! Gens build-pipeline hello (Phase 8): the export story, proven end to
//! end on CPU.
//!
//! Builds a GENSPKG1 package from an in-memory project (a mover scene +
//! a real Rhai script), verifies every hash, loads it into a fresh
//! registry/world, runs 300 deterministic fixed steps through the real
//! player loop (scripts -> schedules), validates invariants, and writes a
//! `gens-run-report-1` artifact.
//!
//! ```text
//! cargo run -p gens-build --example package_hello -- --steps 300 --out package_hello.json
//! ```

use gens_assets::AssetKind;
use gens_build::{build_package, run_package};
use gens_core::anyhow;
use gens_core::anyhow::Context;
use gens_core::logging::{info, init};
use gens_profiler::Profiler;
use std::path::PathBuf;

const MOVE_SCRIPT: &str = r#"
    // Patrol along x between 0 and 3 (content decides the loop).
    fn update() {
        let x = pos_x();
        if x >= 3.0 {
            set_pos(0.0, pos_y(), pos_z());
        } else {
            set_pos(x + dt(), pos_y(), pos_z());
        }
    }
"#;

const SCENE: &str = r#"{
    "format_version": 1,
    "name": "package-hello",
    "entities": [
        {
            "name": "mover",
            "translation": [0.0, 0.5, 0.0],
            "rotation": [0.0, 0.0, 0.0, 1.0],
            "scale": [1.0, 1.0, 1.0],
            "script": {"script": "scripts/mover.rhai", "entry": "update",
                        "vars": {"laps": {"kind": "number", "value": 0.0}}}
        },
        {
            "name": "anchor",
            "translation": [4.0, 0.5, -1.0],
            "rotation": [0.0, 0.0, 0.0, 1.0],
            "scale": [1.0, 1.0, 1.0]
        }
    ]
}"#;

fn main() -> anyhow::Result<()> {
    let mut steps: u64 = 300;
    let mut out = PathBuf::from("package_hello.json");
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
            "--out" => out = PathBuf::from(args.next().context("--out needs a value")?),
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    let filter = init().unwrap_or_else(|_| "(logging already initialized)".to_string());
    info!("Gens engine - build pipeline hello (package -> verify -> player)");
    info!(steps, ?out, %filter, "run configuration");

    // --- Build: project parts -> GENSPKG1 ----------------------------------
    let package = build_package(
        "package-hello",
        "scenes/hello.json",
        SCENE.as_bytes(),
        vec![(
            "scripts/mover.rhai".into(),
            AssetKind::Script,
            MOVE_SCRIPT.as_bytes().to_vec(),
        )],
    )
    .context("package build")?;
    info!(
        bytes = package.len(),
        "package built (scene validated at build time)"
    );

    // --- Run: verify -> load -> step -> validate ----------------------------
    let report = run_package(&package, steps, &mut Profiler::enabled()).context("package run")?;
    anyhow::ensure!(
        report.run.violations.is_empty(),
        "world must validate clean after the run: {:?}",
        report.run.violations
    );
    anyhow::ensure!(
        report.run.script_ticks == steps,
        "the mover must tick every step ({} vs {steps})",
        report.run.script_ticks
    );
    info!(
        entities = report.run.entities,
        ticks = report.run.script_ticks,
        final_hash = format!("{:016x}", report.run.final_hash),
        "package run complete and clean"
    );

    // --- Evidence ------------------------------------------------------------
    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(&out, &json).with_context(|| format!("writing {}", out.display()))?;
    info!(bytes = json.len(), path = %out.display(), "package hello evidence written");
    Ok(())
}
