//! Gens assets + scripting + save hello world (Phase 5).
//!
//! One patrol agent where EVERY decision layer is the content this
//! phase is about:
//!
//! ```text
//! walker.gltf (committed fixture, data-URI buffer)
//!   -> import: Skeleton + walk Clip + skinned MeshData   [gens-assets]
//! patrol.rhai (embedded source, registered in the AssetRegistry)
//!   -> sandboxed Rhai script drives the whole agent       [gens-scripting]
//!      (waypoints, movement, walk-phase accumulation,
//!       moving flag -> anim_play request)
//! run_scripts (entity-index order) -> clip sampled at vars.phase
//!   -> FK -> ECS Transform -> WorldTransform propagation   [gens-ecs]
//! mid-run: save_world_to_file -> FRESH world -> load -> continue
//!   -> evidence bit-identical to an uninterrupted run      [gens-runtime::save]
//! ```
//!
//! Two runs must be **bit-identical**: run A goes straight through,
//! run B saves at the midpoint and continues in a fresh world — the
//! Phase 5 proof that scripting + assets + save hold the replay
//! contracts (R-013 scope).
//!
//! ```text
//! cargo run -p gens-runtime --example asset_script_hello -- --steps 1200 --out asset_script_hello.json
//! ```

use gens_animation::{LocalPose, Skeleton};
use gens_assets::gltf::import_gltf_bytes;
use gens_assets::{AssetKind, AssetRegistry};
use gens_core::anyhow::{self, Context};
use gens_core::game_loop::{GameLoop, LoopConfig};
use gens_core::logging::info;
use gens_core::time::FixedSource;
use gens_ecs::{EngineSchedules, Name, Transform};
use gens_math::Vec3;
use gens_runtime::save::{apply_save, load_world_from_file, save_world_to_file};
use gens_scripting::{ScriptComponent, ScriptHost, run_scripts};
use std::path::PathBuf;
use std::time::Duration;

/// The committed glTF fixture (2-joint skin + walk clip + skinned quad).
const WALKER_GLTF: &[u8] = include_bytes!("../../gens-assets/tests/fixtures/walker.gltf");

/// The whole decision layer, as content. Sandbox-visible surface:
/// pos_x/pos_z/set_pos, var_num/set_var_num/var_flag/set_var_flag,
/// anim_play, dt, and the math vocabulary (sqrt/min).
const PATROL_SCRIPT: &str = r#"
    // Patrol the four corners; walk clip phase accumulates while
    // moving; dwell at each waypoint (idle = rest pose).
    fn update() {
        let speed = 1.5;
        let reach = 0.15;
        let dwell = 0.5;

        let px = pos_x();
        let pz = pos_z();
        let idx = var_num("wp");
        let i = to_int(idx);

        let wpx = [1.0, 9.0, 9.0, 1.0];
        let wpz = [1.0, 1.0, 7.0, 7.0];

        let tx = wpx[i];
        let tz = wpz[i];
        let dx = tx - px;
        let dz = tz - pz;
        let dist = sqrt(dx * dx + dz * dz);

        if dist < reach {
            // Arrived: stand (rest pose), count the dwell, then rotate
            // the waypoint index (content decides the loop order).
            set_var_flag("moving", false);
            let t = var_num("dwell") + dt();
            set_var_num("dwell", t);
            set_var_num("phase", 0.0);
            if t >= dwell {
                set_var_num("dwell", 0.0);
                let next = (idx + 1.0) % 4.0;
                set_var_num("wp", next);
            }
        } else {
            set_var_flag("moving", true);
            let step = min(speed * dt(), dist);
            set_pos(px + dx / dist * step, 0.0, pz + dz / dist * step);
            let p = var_num("phase") + dt();
            if p >= 1.0 {
                set_var_num("phase", p - 1.0);
                set_var_flag("wrapped", true);
            } else {
                set_var_num("phase", p);
            }
        }
    }
"#;

/// Evidence sample taken every `SAMPLE_EVERY` steps.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Sample {
    step: u32,
    x: f32,
    y: f32,
    z: f32,
    wp: f64,
    phase: f64,
    moving: bool,
    pose_hash: String,
}

/// Everything the example owns that one run needs.
struct Sim {
    world: gens_ecs::ecs::world::World,
    schedules: EngineSchedules,
    /// The patrol agent; `None` only for a stage-only build awaiting
    /// `apply_save`.
    agent: Option<gens_ecs::ecs::prelude::Entity>,
    skeleton: Skeleton,
    walk: gens_animation::Clip,
    pose: LocalPose,
}

/// FNV-1a over the bits that make one tick's visual state.
fn pose_hash(sim: &Sim) -> String {
    use gens_assets::fnv1a64;
    let agent = sim.agent.expect("agent present");
    let t = sim.world.get::<Transform>(agent).expect("agent alive");
    let jt = sim.pose.get(0);
    let mut bytes = Vec::new();
    for v in [t.position.x, t.position.y, t.position.z] {
        bytes.extend_from_slice(&v.to_bits().to_le_bytes());
    }
    for v in [jt.translation.x, jt.translation.y, jt.translation.z] {
        bytes.extend_from_slice(&v.to_bits().to_le_bytes());
    }
    for v in [jt.rotation.x, jt.rotation.y, jt.rotation.z, jt.rotation.w] {
        bytes.extend_from_slice(&v.to_bits().to_le_bytes());
    }
    let sc = sim.world.get::<ScriptComponent>(agent).expect("scripted");
    let phase = sc.vars.get_number("phase").unwrap_or(f64::NAN);
    bytes.extend_from_slice(&phase.to_bits().to_le_bytes());
    format!("{:016x}", fnv1a64(&bytes))
}

/// Builds a fresh world: assets imported, script registered. When
/// `spawn_agent` is true the patrol agent is spawned with its initial
/// state (run A and the first half of run B); a `false` build is an
/// empty stage for [`apply_save`] to repopulate (run B after the save).
fn build_world(spawn_agent: bool) -> anyhow::Result<Sim> {
    // Assets: the fixture's bytes are the content; the registry gives
    // them identity (the hash joins the evidence via the key lookup).
    let mut registry = AssetRegistry::new();
    let (gltf_id, _) = registry.import("models/walker.gltf", AssetKind::GlTF, WALKER_GLTF)?;
    let (script_id, _) = registry.import(
        "scripts/patrol.rhai",
        AssetKind::Script,
        PATROL_SCRIPT.as_bytes(),
    )?;
    let (gltf_version, gltf_size) = {
        let e = registry.entry(gltf_id).expect("entry");
        (e.version, e.size)
    };
    let (script_version, script_size) = {
        let e = registry.entry(script_id).expect("entry");
        (e.version, e.size)
    };
    info!(
        "asset models/walker.gltf = {gltf_id} (v{gltf_version}, {gltf_size} B); scripts/patrol.rhai = {script_id} (v{script_version}, {script_size} B)",
    );

    let imported = import_gltf_bytes("models/walker.gltf", WALKER_GLTF)?;
    anyhow::ensure!(!imported.skins.is_empty(), "fixture has no skin");
    anyhow::ensure!(!imported.clips.is_empty(), "fixture has no clip");
    let skeleton = imported.skins[0].skeleton.clone();
    let joint_count = skeleton.joint_count();
    let walk = imported.clips[0].clip.clone();

    // Scripting: fresh host, script registered from content.
    let mut host = ScriptHost::new();
    host.register("patrol", PATROL_SCRIPT)?;

    let mut world = gens_ecs::ecs::world::World::new();
    world.insert_resource(host);
    let agent = spawn_agent.then(|| {
        world
            .spawn((
                Name::new("patrol_agent"),
                Transform::from_position(Vec3::new(1.0, 0.0, 1.0)),
                ScriptComponent::new("patrol"),
            ))
            .id()
    });

    Ok(Sim {
        world,
        schedules: EngineSchedules::default(),
        agent,
        skeleton,
        walk,
        pose: LocalPose::identity(joint_count),
    })
}

/// One fixed step: scripts decide, the host samples the imported clip
/// at the script-driven phase, FK writes the animation offset, ECS
/// propagation computes world transforms.
fn step_sim(sim: &mut Sim, dt: f64, time: f64) {
    run_scripts(&mut sim.world, dt, time);
    sample_animation(sim);
    sim.schedules.run_frame(&mut sim.world);
}

/// Host-side animation: the script's state selects and phases the
/// imported clip (data-only street: vars -> clip sample -> pose).
/// Called every step AND once after a load — the pose is DERIVED
/// state (a pure function of the restored vars), never saved.
fn sample_animation(sim: &mut Sim) {
    let agent = sim.agent.expect("agent present");
    let (moving, phase) = {
        let sc = sim.world.get::<ScriptComponent>(agent).expect("scripted");
        (
            sc.vars.get_flag("moving").unwrap_or(false),
            sc.vars.get_number("phase").unwrap_or(0.0),
        )
    };
    sim.pose.set_identity();
    if moving {
        sim.walk
            .sample_clamped((phase as f32) * sim.walk.duration(), &mut sim.pose);
    }
    let root = sim.pose.global_matrix(&sim.skeleton, 0);
    let bob = root.w_axis.y;
    if let Some(mut t) = sim.world.get_mut::<Transform>(agent) {
        t.position.y = bob;
    }
}

/// Runs `steps` steps, sampling evidence every [`SAMPLE_EVERY`].
/// `save_at` (run B): save the world to `save_path` right after that
/// step, rebuild a FRESH world, load the save, and continue.
fn run(
    steps: u32,
    save_at: Option<u32>,
    save_path: &PathBuf,
) -> anyhow::Result<(Vec<Sample>, gens_assets::AssetRegistry)> {
    let mut sim = build_world(true)?;
    let mut registry = AssetRegistry::new();
    let (_, script_entry) = registry.import(
        "scripts/patrol.rhai",
        AssetKind::Script,
        PATROL_SCRIPT.as_bytes(),
    )?;
    let _ = script_entry;

    let source = FixedSource::new(Duration::from_secs_f64(1.0 / 60.0));
    let mut game_loop = GameLoop::new(LoopConfig::default(), source);
    let mut samples = Vec::new();
    let mut loaded = false;

    for step in 1..=steps {
        game_loop.run_frame(|_| {
            step_sim(&mut sim, 1.0 / 60.0, f64::from(step) / 60.0);
        });

        if Some(step) == save_at && !loaded {
            let bytes = save_world_to_file(&sim.world, save_path).context("mid-run save")?;
            info!(
                "saved world at step {step} ({bytes} B) -> {}",
                save_path.display()
            );
            // FRESH stage (no agent); the save restores every
            // saveable component, spawning the agent at its index.
            let mut fresh = build_world(false)?;
            let save = load_world_from_file(save_path).context("mid-run load")?;
            apply_save(&mut fresh.world, &save)?;
            // The loaded world holds the saved entities PLUS bevy's
            // resource entities (resources are components in bevy
            // 0.16+ and occupy entity slots) — find the agent by name.
            fresh.agent = fresh.world.iter_entities().find_map(|r| {
                r.get::<Name>()
                    .filter(|n| n.as_str() == "patrol_agent")
                    .map(|_| r.id())
            });
            // The fresh world's host/scripts were rebuilt; the agent
            // identity survives via spawn order (save == fresh order).
            // Re-derive the animation pose from the restored vars so
            // the continuation is indistinguishable from run A.
            sample_animation(&mut fresh);
            sim = fresh;
            loaded = true;
        }

        if step % 20 == 0 {
            let agent = sim.agent.expect("agent present");
            let t = sim.world.get::<Transform>(agent).expect("alive");
            let sc = sim.world.get::<ScriptComponent>(agent).expect("scripted");
            samples.push(Sample {
                step,
                x: t.position.x,
                y: t.position.y,
                z: t.position.z,
                wp: sc.vars.get_number("wp").unwrap_or(f64::NAN),
                phase: sc.vars.get_number("phase").unwrap_or(f64::NAN),
                moving: sc.vars.get_flag("moving").unwrap_or(false),
                pose_hash: pose_hash(&sim),
            });
        }
    }
    Ok((samples, registry))
}

fn main() -> anyhow::Result<()> {
    gens_core::logging::init().expect("logging init");
    let args: Vec<String> = std::env::args().collect();
    let mut steps = 1200_u32;
    let mut out = PathBuf::from("asset_script_hello.json");
    let mut save_path = std::env::temp_dir().join("gens_asset_script_save.bin");
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--steps" => {
                i += 1;
                steps = args.get(i).context("--steps needs a value")?.parse()?;
            }
            "--out" => {
                i += 1;
                out = PathBuf::from(args.get(i).context("--out needs a value")?);
            }
            "--save-path" => {
                i += 1;
                save_path = PathBuf::from(args.get(i).context("--save-path needs a value")?);
            }
            other => anyhow::bail!("unknown arg {other} (use --steps N --out FILE)"),
        }
        i += 1;
    }

    // Run A: straight through. Run B: save at the midpoint, continue
    // in a fresh world.
    let (run_a, _) = run(steps, None, &save_path)?;
    let midpoint = steps / 2;
    let (run_b, _) = run(steps, Some(midpoint), &save_path)?;

    anyhow::ensure!(
        run_a.len() == run_b.len(),
        "evidence lengths differ: {} vs {}",
        run_a.len(),
        run_b.len()
    );
    let mut mismatches = Vec::new();
    for (a, b) in run_a.iter().zip(&run_b) {
        if a.x.to_bits() != b.x.to_bits()
            || a.y.to_bits() != b.y.to_bits()
            || a.z.to_bits() != b.z.to_bits()
            || a.wp.to_bits() != b.wp.to_bits()
            || a.phase.to_bits() != b.phase.to_bits()
            || a.moving != b.moving
            || a.pose_hash != b.pose_hash
        {
            mismatches.push(format!("step {}: A={a:?} B={b:?}", a.step));
        }
    }
    anyhow::ensure!(
        mismatches.is_empty(),
        "save/load continuation diverged ({} samples):\n{}",
        mismatches.len(),
        mismatches
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );

    // Behavioral gates (measured, not assumed). The patrol loops back
    // to its start, so the span is the FULL excursion (max - min),
    // not first-vs-last.
    let xs: Vec<f32> = run_a.iter().map(|s| s.x).collect();
    let span_x =
        xs.iter().fold(f32::MIN, |m, v| m.max(*v)) - xs.iter().fold(f32::MAX, |m, v| m.min(*v));
    let max_wp = run_a.iter().map(|s| s.wp).fold(f64::MIN, f64::max);
    let ever_moving = run_a.iter().any(|s| s.moving);
    let ever_idle = run_a.iter().any(|s| !s.moving);
    let ever_wrapped = {
        let mut wrapped = false;
        for w in run_a.windows(2) {
            if w[1].phase < w[0].phase {
                wrapped = true;
            }
        }
        wrapped
    };
    anyhow::ensure!(
        span_x > 7.0,
        "patrol did not span the map (x span {span_x})"
    );
    anyhow::ensure!(max_wp >= 3.0, "waypoints never cycled (max wp {max_wp})");
    anyhow::ensure!(ever_moving && ever_idle, "both states must be observed");
    anyhow::ensure!(ever_wrapped, "walk phase must wrap");

    // Evidence document (deterministic: serde_json BTreeMap + fixed
    // sample order).
    let doc = serde_json::json!({
        "example": "asset_script_hello",
        "steps": steps,
        "save_at": serde_json::Value::Null,
        "runs": {
            "straight": run_a,
            "save_load_continue": run_b,
        },
        "gates": {
            "save_load_bit_identical": true,
            "x_span": span_x,
            "max_waypoint": max_wp,
            "both_states_observed": ever_moving && ever_idle,
            "phase_wrapped": ever_wrapped,
        },
    });
    let json = serde_json::to_string_pretty(&doc)?;
    std::fs::write(&out, json.as_bytes()).with_context(|| format!("write {}", out.display()))?;
    info!(
        "asset_script_hello: {} steps, {} samples, save/load bit-identical, x span {span_x:.3}, max wp {max_wp}, evidence {}",
        steps,
        run_a.len(),
        out.display()
    );
    Ok(())
}
