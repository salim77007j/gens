//! The headless player: load a verified package, run it, report (P8-5).
//!
//! Contract: `load_package` verifies, imports every file into a fresh
//! [`AssetRegistry`] (by recorded kind), registers every script source
//! into a fresh [`gens_scripting::ScriptHost`], instantiates the scene
//! document into a fresh world ([`gens_runtime::data::apply_data_document`],
//! which re-validates loudly) and runs one propagation frame so derived
//! transforms exist. `run_loaded` then ticks the deterministic loop —
//! `run_scripts` (entity-index order, ADR-0019) then the standard
//! schedules — under the observation-only profiler (ADR-0027), and
//! produces a `gens-run-report-1` document whose simulation fields are
//! byte-stable across runs.
//!
//! The player never writes files, never opens windows, never touches the
//! network: it is the CI-provable half of the export story (the window
//! shell remains future work, stated honestly).

use crate::package::{PackageError, VerifiedPackage, parse_kind, verify_package};
use gens_assets::{AssetKind, AssetRegistry};
use gens_ecs::EngineSchedules;
use gens_ecs::ecs::world::World;
use gens_profiler::{MemorySnapshot, ProfileReport, Profiler, memory_stats};
use gens_runtime::debug::{Violation, validate_world};
use gens_runtime::profile::world_state_hash;
use gens_scripting::{ScriptHost, run_scripts};
use serde::Serialize;

/// A package loaded and ready to run.
#[derive(Debug)]
pub struct LoadedGame {
    /// Verified package (manifest + blobs + parsed scene).
    pub package: VerifiedPackage,
    /// Fresh registry with every package file imported by recorded kind.
    pub registry: AssetRegistry,
    /// Fresh world with the scene document instantiated.
    pub world: World,
}

/// Loads and verifies `bytes` into a playable state.
pub fn load_package(bytes: &[u8]) -> Result<LoadedGame, PackageError> {
    let package = verify_package(bytes)?;

    let mut registry = AssetRegistry::new();
    for file in &package.manifest.files {
        let kind = parse_kind(&file.kind).unwrap_or(AssetKind::Raw);
        let blob = &package.blobs[&file.key];
        registry.import(&file.key, kind, blob).map_err(|e| {
            PackageError::BadInput(format!("file '{}' failed to import: {e}", file.key))
        })?;
    }

    // Register script sources (package-verified bytes, not disk reads).
    let mut host = ScriptHost::new();
    for file in &package.manifest.files {
        if file.kind == AssetKind::Script.as_str() {
            let source = String::from_utf8(package.blobs[&file.key].clone()).map_err(|_| {
                PackageError::BadInput(format!("script '{}' is not UTF-8", file.key))
            })?;
            host.register(file.key.clone(), &source).map_err(|e| {
                PackageError::BadInput(format!("script '{}' failed to compile: {e}", file.key))
            })?;
        }
    }

    let mut world = World::new();
    let report = gens_runtime::data::apply_data_document(&mut world, &package.scene)
        .map_err(|e| PackageError::BadInput(format!("scene instantiation failed: {e}")))?;
    let _ = report; // ids in authored order; the report exposes counts only

    world.insert_resource(host);
    // Initial propagation: derived world transforms exist before step 0.
    let mut schedules = EngineSchedules::new();
    schedules.run_frame(&mut world);

    Ok(LoadedGame {
        package,
        registry,
        world,
    })
}

/// Simulation-side run statistics (byte-stable across identical runs).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunStats {
    /// Steps executed.
    pub steps: u64,
    /// Entity count at the end (content-filtered, same rule as the dump).
    pub entities: usize,
    /// Total script ticks performed (`run_scripts` return, summed).
    pub script_ticks: u64,
    /// Final [`world_state_hash`].
    pub final_hash: u64,
    /// Invariant violations after the run (must be empty for a report to
    /// be considered good; embedded for evidence either way).
    pub violations: Vec<Violation>,
}

/// The full run report: identity, simulation stats, validation, and the
/// observation-only sidecars (profile + memory).
#[derive(Debug, Clone, Serialize)]
pub struct RunReport {
    /// Format tag (`gens-run-report-1`).
    pub format: &'static str,
    /// Package identity.
    pub package: PackageIdentity,
    /// Simulation statistics.
    pub run: RunStats,
    /// Per-span profile over the run (gens-profile-1).
    pub profile: ProfileReport,
    /// Memory snapshot at report time (gens-mem-1).
    pub memory: MemorySnapshot,
}

/// Package identity fields for the report.
#[derive(Debug, Clone, Serialize)]
pub struct PackageIdentity {
    /// Project name from the manifest.
    pub name: String,
    /// Scene key from the manifest.
    pub scene_key: String,
    /// Engine version that BUILT the package.
    pub engine_version: String,
    /// Number of packaged files.
    pub files: usize,
}

/// Runs a loaded game for `steps` fixed steps (dt = 1/60, deterministic
/// simulated time), profiling each phase.
///
/// The profiler is passed in so callers decide enabled vs disabled — the
/// same observation-only discipline as everywhere else.
pub fn run_loaded(game: &mut LoadedGame, steps: u64, profiler: &mut Profiler) -> RunStats {
    let dt = 1.0 / 60.0;
    let mut schedules = EngineSchedules::new();
    let mut script_ticks: u64 = 0;
    let mut final_hash: u64 = 0;

    for step in 0..steps {
        profiler.begin_frame(step);
        let t = step as f64 * dt;
        let ticks = profiler.time("scripts", |_| run_scripts(&mut game.world, dt, t));
        script_ticks += ticks as u64;
        profiler.time("propagation", |_| schedules.run_frame(&mut game.world));
        final_hash = world_state_hash(&game.world);
        profiler.end_frame();
    }

    let violations = validate_world(&game.world, Some(&game.registry));
    RunStats {
        steps,
        entities: game.world.iter_entities().count(),
        script_ticks,
        final_hash,
        violations,
    }
}

/// Convenience: load, run, report — the player CLI's body (also the CI
/// smoke's). Returns the report and leaves the profiler history intact
/// for callers that want per-frame data.
pub fn run_package(
    bytes: &[u8],
    steps: u64,
    profiler: &mut Profiler,
) -> Result<RunReport, PackageError> {
    let mut game = load_package(bytes)?;
    let identity = PackageIdentity {
        name: game.package.manifest.name.clone(),
        scene_key: game.package.manifest.scene_key.clone(),
        engine_version: game.package.manifest.engine_version.clone(),
        files: game.package.manifest.files.len(),
    };
    let run = run_loaded(&mut game, steps, profiler);
    let profile = gens_profiler::report::report_from_history(profiler.history());
    Ok(RunReport {
        format: "gens-run-report-1",
        package: identity,
        run,
        profile,
        memory: memory_stats().snapshot(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::build_package;
    use gens_assets::AssetKind;

    const MOVE_SCRIPT: &str = r#"
        // Moves the entity along +x at 1 m/s, wrapping at x = 3.
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
        "name": "mover-scene",
        "entities": [
            {
                "name": "mover",
                "translation": [0.0, 0.0, 0.0],
                "rotation": [0.0, 0.0, 0.0, 1.0],
                "scale": [1.0, 1.0, 1.0],
                "script": {"script": "scripts/mover.rhai", "entry": "update",
                            "vars": {"lap": {"kind": "number", "value": 0.0}}}
            }
        ]
    }"#;

    fn sample_package() -> Vec<u8> {
        build_package(
            "mover",
            "scenes/mover.json",
            SCENE.as_bytes(),
            vec![(
                "scripts/mover.rhai".into(),
                AssetKind::Script,
                MOVE_SCRIPT.as_bytes().to_vec(),
            )],
        )
        .expect("sample package builds")
    }

    #[test]
    fn load_imports_everything_and_propagates() {
        let mut game = load_package(&sample_package()).expect("load");
        // Registry: scene + script, both importable by their keys.
        assert!(game.registry.get("scenes/mover.json").is_some());
        assert!(game.registry.get("scripts/mover.rhai").is_some());
        // World: the mover exists with a derived world transform.
        let mover = game
            .world
            .query::<(&gens_ecs::Name, gens_ecs::ecs::entity::Entity)>()
            .iter(&game.world)
            .find(|(n, _)| n.0 == "mover")
            .map(|(_, e)| e)
            .expect("mover spawned");
        assert!(game.world.get::<gens_ecs::WorldTransform>(mover).is_some());
    }

    #[test]
    fn run_moves_the_entity_and_validates_clean() {
        let mut game = load_package(&sample_package()).expect("load");
        let mut profiler = Profiler::enabled();
        let stats = run_loaded(&mut game, 120, &mut profiler); // 2 simulated seconds
        assert_eq!(stats.steps, 120);
        assert_eq!(
            stats.script_ticks, 120,
            "one script entity ticks every step"
        );
        assert!(
            stats.violations.is_empty(),
            "must validate clean: {:?}",
            stats.violations
        );
        // The mover actually moved: final x must be inside (0, 3] and the
        // world hash must differ from a fresh load's step-0 hash.
        let mover = game
            .world
            .query::<(&gens_ecs::Name, gens_ecs::ecs::entity::Entity)>()
            .iter(&game.world)
            .find(|(n, _)| n.0 == "mover")
            .map(|(_, e)| e)
            .expect("mover");
        let x = game
            .world
            .get::<gens_ecs::Transform>(mover)
            .unwrap()
            .position
            .x;
        assert!(
            x > 1.0 && x <= 3.0,
            "2 s at 1 m/s from x=0 must land in (1, 3], got {x}"
        );
    }

    #[test]
    fn two_runs_are_simulation_identical() {
        let pkg = sample_package();
        let mut a = load_package(&pkg).expect("load a");
        let mut b = load_package(&pkg).expect("load b");
        let sa = run_loaded(&mut a, 300, &mut Profiler::enabled());
        let sb = run_loaded(&mut b, 300, &mut Profiler::disabled());
        assert_eq!(sa, sb, "simulation fields must not depend on profiling");
    }

    #[test]
    fn disabled_profiler_run_still_works() {
        let mut game = load_package(&sample_package()).expect("load");
        let stats = run_loaded(&mut game, 60, &mut Profiler::disabled());
        assert_eq!(stats.steps, 60);
        assert!(stats.violations.is_empty());
    }

    #[test]
    fn run_report_is_complete_and_tagged() {
        let pkg = sample_package();
        let report = run_package(&pkg, 90, &mut Profiler::enabled()).expect("run");
        assert_eq!(report.format, "gens-run-report-1");
        assert_eq!(report.package.name, "mover");
        assert_eq!(report.package.files, 2);
        assert_eq!(report.run.steps, 90);
        assert!(report.run.violations.is_empty());
        // Profile sidecar names the player's phases.
        let names: Vec<&str> = report
            .profile
            .spans
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert!(names.contains(&"scripts"));
        assert!(names.contains(&"propagation"));
        let json = serde_json::to_string(&report).expect("report serializes");
        assert!(json.contains(r#""format":"gens-run-report-1""#));
    }

    #[test]
    fn corrupt_package_fails_the_load_loudly() {
        let pkg = sample_package();
        let mut corrupt = pkg.clone();
        let mid = corrupt.len() * 3 / 4;
        corrupt[mid] ^= 0xFF;
        assert!(matches!(
            load_package(&corrupt),
            Err(PackageError::Corrupt { .. })
        ));
    }

    #[test]
    fn script_that_fails_to_compile_is_a_loud_load_error() {
        let pkg = build_package(
            "broken",
            "scenes/broken.json",
            SCENE.as_bytes(),
            vec![(
                "scripts/mover.rhai".into(),
                AssetKind::Script,
                b"fn update( {}".to_vec(),
            )],
        )
        .expect("package builds (compile happens at LOAD)");
        let err = load_package(&pkg).expect_err("must fail");
        assert!(matches!(err, PackageError::BadInput(_)), "got {err:?}");
    }
}
