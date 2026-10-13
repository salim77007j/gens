//! The v1.0.0 example games: two deterministic GENSPKG1 packages built,
//! hash-re-verified and player-run as a release-pipeline gate.
//!
//! Each game is a complete, minimal downstream artifact: a scene
//! (validated by the same constructors the editor/save path uses) plus
//! a real Rhai script, packaged into GENSPKG1, re-verified (every file
//! hash checked) and executed through the real player loop for 120
//! fixed steps with a clean world validation as the pass condition.
//!
//! The release workflow ships the two `.genspkg` files as GitHub
//! Release assets; a user runs them with the released `gens-player`
//! binary — the same run this example performs before attaching.
//!
//! ```text
//! cargo run -p gens-build --example release_games -- --dir dist/games
//! ```

use gens_assets::AssetKind;
use gens_build::{build_package, run_package, verify_package};
use gens_core::anyhow;
use gens_core::anyhow::{Context, Result};
use gens_core::logging::{info, init};
use gens_profiler::Profiler;
use std::path::PathBuf;

/// Patrol script for `hello-patrol` — the proven Phase 8 mover: walk x
/// in [0, 3) at 1 unit/s and wrap (only APIs package_hello pins).
const PATROL_SCRIPT: &str = r#"
    // patrol.rhai — walk x in [0, 3) and wrap (content decides the loop).
    fn update() {
        let x = pos_x();
        if x >= 3.0 {
            set_pos(0.0, pos_y(), pos_z());
        } else {
            set_pos(x + dt(), pos_y(), pos_z());
        }
    }
"#;

const PATROL_SCENE: &str = r#"{
    "format_version": 1,
    "name": "hello-patrol",
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

/// Sink script for `hello-sink`: descend y from 2.0 at 1 unit/s, reset
/// to the drop height (same proven API surface as the patrol script).
const SINK_SCRIPT: &str = r#"
    // sink.rhai — descend y from 2.0 and reset to the drop height.
    fn update() {
        let y = pos_y();
        if y <= 0.0 {
            set_pos(pos_x(), 2.0, pos_z());
        } else {
            set_pos(pos_x(), y - dt(), pos_z());
        }
    }
"#;

const SINK_SCENE: &str = r#"{
    "format_version": 1,
    "name": "hello-sink",
    "entities": [
        {
            "name": "sinker",
            "translation": [0.0, 2.0, 0.0],
            "rotation": [0.0, 0.0, 0.0, 1.0],
            "scale": [1.0, 1.0, 1.0],
            "script": {"script": "scripts/sink.rhai", "entry": "update",
                        "vars": {"drops": {"kind": "number", "value": 0.0}}}
        },
        {
            "name": "anchor",
            "translation": [-2.0, 0.5, 1.0],
            "rotation": [0.0, 0.0, 0.0, 1.0],
            "scale": [1.0, 1.0, 1.0]
        }
    ]
}"#;

/// (package name, scene document, script key, script source)
const GAMES: [(&str, &str, &str, &str); 2] = [
    (
        "hello-patrol",
        PATROL_SCENE,
        "scripts/mover.rhai",
        PATROL_SCRIPT,
    ),
    ("hello-sink", SINK_SCENE, "scripts/sink.rhai", SINK_SCRIPT),
];

/// Player steps per game before the attach gate. Deterministic, so the
/// final hashes are stable run to run and OS to OS (R-013).
const PLAYER_STEPS: u64 = 120;

fn main() -> Result<()> {
    let mut dir = PathBuf::from("dist/games");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dir" => dir = PathBuf::from(args.next().context("--dir needs a value")?),
            other => anyhow::bail!("unknown argument: {other} (use --dir PATH)"),
        }
    }

    let filter = init().unwrap_or_else(|_| "(logging already initialized)".to_string());
    info!("Gens engine - release example games (v1.0.0)");
    info!(?dir, %filter, "run configuration");

    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    for (name, scene, script_key, script) in GAMES {
        let package = build_package(
            name,
            "scenes/main.json",
            scene.as_bytes(),
            vec![(
                script_key.to_string(),
                AssetKind::Script,
                script.as_bytes().to_vec(),
            )],
        )
        .with_context(|| format!("building {name}"))?;

        // Every file hash must match before the package may attach.
        let verified =
            verify_package(&package).with_context(|| format!("{name} failed re-verification"))?;

        // The same run a released gens-player performs: load -> step ->
        // validate; a dirty world refuses to ship.
        let mut profiler = Profiler::enabled();
        let report = run_package(&package, PLAYER_STEPS, &mut profiler)
            .with_context(|| format!("{name}: player run"))?;
        anyhow::ensure!(
            report.run.violations.is_empty(),
            "{name}: world validation failed after the run: {:?}",
            report.run.violations
        );

        let out = dir.join(format!("{name}.genspkg"));
        std::fs::write(&out, &package).with_context(|| format!("writing {}", out.display()))?;
        info!(
            game = name,
            bytes = package.len(),
            files = verified.manifest.files.len(),
            entities = report.run.entities,
            ticks = report.run.script_ticks,
            final_hash = format!("{:016x}", report.run.final_hash),
            path = %out.display(),
            "example game built, verified and player-run clean"
        );
    }
    Ok(())
}
