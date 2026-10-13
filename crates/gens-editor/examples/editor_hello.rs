//! Gens visual editor hello world (Phase 7).
//!
//! Builds the reference design's scene (docs/design/REFERENCE.png):
//! World/Player/Level/SciFi_Station/AI trees, a Spaceship with a real
//! physics body and script binding, falling Crates, the Guard trio —
//! then drives the full editor (hierarchy, viewport, inspector, asset
//! browser, console) through a scripted, deterministic replay:
//!
//! ```text
//! frame 5  : REAL pointer click on the layout-captured "Spaceship" row
//! frame 6  : selection verified (loud failure if the click missed)
//! frame 8-12: pointer drag orbits the camera
//! frame 15 : inspector-path transform edit (the ship moves)
//! frame 18 : Edit > Duplicate Selected command (menu intent -> app)
//! frame 22 : File > Save Scene writes a REAL GENSSAVE1 autosave
//! ```
//!
//! Two independent runs (fresh worlds, fresh GPU contexts) must produce
//! byte-identical frame hashes — the Phase 7 determinism proof. The
//! final window composite is saved as PNG + a JSON evidence blob.

use gens_assets::{AssetKind, AssetRegistry};
use gens_cinematics::{CinematicPlayer, Timeline, Track};
use gens_core::anyhow as anyhow_crate;
use gens_core::anyhow::{Context, Result, anyhow, ensure};
use gens_core::game_loop::{GameLoop, LoopConfig};
use gens_core::logging::info;
use gens_core::time::FixedSource;
use gens_ecs::ecs::prelude::{Entity, World};
use gens_ecs::hierarchy::attach_children;
use gens_ecs::{Active, EngineSchedules, Name, Transform};
use gens_editor::model::EditorCommand;
use gens_editor::{EditorShell, MeshSlot};
use gens_math::{Quat, Vec3, quat, vec3};
use gens_physics::ecs::sync_transforms_from_physics;
use gens_physics::{
    BodySpec, ColliderShape, ColliderSpec, PhysicsBody, PhysicsResource, PhysicsWorld,
};
use gens_renderer::GpuContext;
use gens_runtime::cinematic::{CameraPose, CinematicStage, advance_and_apply};
use gens_scripting::ScriptComponent;
use gens_ui::{UiEvent, UiInput};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing_subscriber::layer::SubscriberExt;

/// Window size of the composite editor surface (pixels).
const WINDOW_SIZE: [u32; 2] = [1600, 900];
/// Fixed sim step (also the UI clock increment).
const DT: f32 = 1.0 / 60.0;

/// A patrol script for the Spaceship — REAL rhai source registered in
/// the asset browser.
const PATROL_RHAI: &str = r#"
// patrol.rhai — orbit the docking bay when the dock door opens
fn update(ship, ctx) {
    let t = ctx.time;
    let r = 4.0;
    ship.position = [r * cos(t), 0.5, r * sin(t)];
}
"#;

fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let steps: usize = args
        .iter()
        .position(|a| a == "--steps")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    let out_dir = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("editor_hello"));

    // Two independent runs: fresh worlds, fresh GPU contexts, identical
    // event scripts. Byte-identical hashes or loud failure.
    let run_a = run_editor(steps, &out_dir, true)?;
    let run_b = run_editor(steps, &out_dir, false)?;
    ensure!(
        run_a.frame_hashes == run_b.frame_hashes,
        "runs A and B diverged ({} vs {} frames) — determinism broken",
        run_a.frame_hashes.len(),
        run_b.frame_hashes.len()
    );
    info!(
        frames = run_a.frame_hashes.len(),
        final_fnv = format!("{:016x}", run_a.frame_hashes.last().copied().unwrap_or(0)),
        "determinism verified: two runs byte-identical"
    );
    ensure!(
        run_a.validated_clean,
        "the console 'validate' command must report a clean world"
    );
    info!("console command gate: 'validate' reported 0 violations");

    // The profile sidecar is observation-only (ADR-0027): its timings
    // differ between runs BY DESIGN and are never hashed; the frame
    // hashes above are the determinism contract.
    let profile_json =
        serde_json::to_string(&run_a.profile).context("profile sidecar serialize")?;
    let json = format!(
        "{{\n  \"steps\": {steps},\n  \"window\": [{}, {}],\n  \"selection\": \"Spaceship\",\n  \"duplicated\": \"{}\",\n  \"clicked\": {},\n  \"validated_clean\": {},\n  \"cinematic_events\": [{}],\n  \"frame_fnv1a64\": [{}],\n  \"final_fnv1a64\": \"{}\",\n  \"deterministic\": true,\n  \"profile_sidecar\": {}\n}}\n",
        WINDOW_SIZE[0],
        WINDOW_SIZE[1],
        run_a.duplicated_name,
        run_a.clicked,
        run_a.validated_clean,
        run_a
            .cinematic_events
            .iter()
            .map(|e| format!("\"{e}\""))
            .collect::<Vec<_>>()
            .join(", "),
        run_a
            .frame_hashes
            .iter()
            .map(|h| format!("\"{h:016x}\""))
            .collect::<Vec<_>>()
            .join(", "),
        run_a.frame_hashes.last().copied().unwrap_or(0),
        profile_json,
    );
    std::fs::write(format!("{}.json", out_dir.display()), json).context("writing evidence json")?;
    info!(?out_dir, "editor hello complete");
    Ok(())
}

struct RunReport {
    frame_hashes: Vec<u64>,
    duplicated_name: String,
    clicked: bool,
    validated_clean: bool,
    /// The cinematic's fired events (the Animator tab's data trail).
    cinematic_events: Vec<String>,
    /// Observation-only sidecar (ADR-0027): the run's per-phase profile.
    /// Never hashed — two runs' timings differ by design; their PIXELS
    /// must not.
    profile: gens_profiler::ProfileReport,
}

/// One full editor session (build -> replay -> evidence).
fn run_editor(steps: usize, out_dir: &Path, capture_png: bool) -> Result<RunReport> {
    // --- Scene ----------------------------------------------------------
    let mut world = World::new();
    let mut physics = PhysicsWorld::new(vec3(0.0, -9.81, 0.0));
    let entities = build_scene(&mut world, &mut physics)?;
    world.insert_resource(PhysicsResource { world: physics });
    let mut schedules = EngineSchedules::default();
    schedules.update.add_systems(sync_transforms_from_physics);

    // --- Assets ---------------------------------------------------------
    let assets = build_assets()?;

    // --- Editor ----------------------------------------------------------
    let mut shell = EditorShell::new(GpuContext::new()?, WINDOW_SIZE[0], WINDOW_SIZE[1])?;

    // Console capture (scoped so BOTH runs capture identically).
    let subscriber = tracing_subscriber::registry()
        .with(shell.console_sink())
        .with(tracing_subscriber::fmt::layer().without_time().compact());

    let ship = entities["Spaceship"];

    // --- Cinematic (Phase 9): a small take that dollies a dedicated
    // CineCamera and eases its FOV; the Animator tab reads its live
    // track rows and the Game View tab renders through its pose once
    // the replay switches tabs. Fixed dt only (ADR-0030).
    let timeline = std::sync::Arc::new(director_timeline());
    let mut player = CinematicPlayer::new(timeline);
    player.play();
    let cine_camera = world.spawn((
        Name::new("CineCamera"),
        Transform::from_position(vec3(0.0, 2.0, 8.0)),
        Active,
    ));
    let cine_camera = cine_camera.id();
    let mut stage = CinematicStage::new(player, [("camera".to_string(), cine_camera)]);
    let track_rows = track_rows(&stage);
    let mut cinematic_events: Vec<String> = Vec::new();

    let mut frame_hashes = Vec::new();
    let source = FixedSource::new(Duration::from_secs_f64(DT as f64));
    let mut game_loop = GameLoop::new(LoopConfig::default(), source);
    let mut clicked = false;
    let mut duplicated_name = String::new();
    let mut error: Option<anyhow_crate::Error> = None;
    let mut step_done: usize = 0;
    let mut validated_clean = false;
    // Observation-only profiler (ADR-0027): records per-phase wall-clock
    // spans as SIDECAR data. The replay's rendered pixels stay
    // deterministic — the shell's telemetry panels intentionally stay in
    // their honest "no data" state during hashed replays, because CPU/Mem
    // numbers are wall-clock and would break byte-identity. The UI-with-
    // data path is proven by geometry tests (ADR-0026).
    let mut profiler = gens_profiler::Profiler::enabled();

    tracing::subscriber::with_default(subscriber, || {
        info!("engine initialized");
        info!(entities = entities.len(), "scene loaded: docking bay");
        info!("assets imported: scripts/patrol.rhai, textures/ui/icon_warning.png");
        info!("input devices: keyboard, mouse");
        info!("physics initialized: rapier3d");

        // One run_frame call per scripted frame: FixedSource yields
        // exactly one fixed step per call (deterministic virtual time).
        for _ in 0..steps {
            game_loop.run_frame(|_| {
                if error.is_some() || step_done >= steps {
                    return;
                }
                let step = step_done;
                let time = step as f64 * DT as f64;

                // --- Scripted interactions (real UiEvents / real APIs) ------
                let events = events_for(step, &shell.state);
                if step == 18 {
                    // Same intent the Edit menu writes.
                    shell.state.pending.push(EditorCommand::DuplicateSelected);
                }
                if step == 22 {
                    // Same intent the File menu writes.
                    shell.state.pending.push(EditorCommand::SaveScene(
                        "autosave_001.genssave".to_string(),
                    ));
                }
                if step + 1 == steps {
                    // Same intent the command row's Run button queues.
                    shell.state.pending_commands.push("validate".into());
                }
                for command in shell.state.take_commands() {
                    match command {
                        EditorCommand::SaveScene(path) => {
                            match gens_runtime::save::save_world_to_file(&world, &path) {
                                Ok(bytes) => info!(path, bytes, "Saved scene"),
                                Err(e) => {
                                    error = Some(anyhow!("save failed: {e}"));
                                    return;
                                }
                            }
                        }
                        EditorCommand::DuplicateSelected => {
                            if let Some(name) = duplicate_selected(&mut world, &mut shell.state) {
                                duplicated_name = name.clone();
                                info!(name, "Entity spawned");
                            }
                        }
                        EditorCommand::DeleteSelected => {
                            if let Some(sel) = shell.state.selected.take() {
                                world.despawn(sel);
                                info!("Entity deleted");
                            }
                        }
                        // v1.1 commands route through the SHARED executor
                        // (the interactive bin runs exactly this).
                        other => {
                            gens_editor::commands::execute_editor_command(
                                &mut shell.state,
                                &mut world,
                                &other,
                            );
                        }
                    }
                }
                // Console commands (real execution against the world; the
                // results land in the console via the tracing sink).
                gens_editor::commands::drain_commands(&mut shell.state, &mut world, Some(&assets));
                if step + 1 == steps {
                    validated_clean = shell
                        .state
                        .console
                        .lines()
                        .iter()
                        .any(|l| l.message.contains("validation: 0 violations"));
                }
                if step == 15 {
                    // The inspector's write path: move the ship (visible in
                    // the viewport on the next frame).
                    gens_editor::inspector::set_transform(
                        &mut world,
                        ship,
                        vec3(1.5, 0.8, 0.0),
                        [0.0, 0.0, 0.0],
                        Vec3::ONE,
                    );
                    info!("inspector edit applied: Spaceship moved");
                }
                if step == 6 {
                    // Selection gate: the frame-5 click must have landed.
                    let selected_now = shell
                        .state
                        .selected
                        .and_then(|e| world.get::<Name>(e).map(|n| n.as_str().to_string()));
                    clicked = selected_now.as_deref() == Some("Spaceship");
                    if !clicked {
                        error = Some(anyhow!(
                            "hierarchy click did not select Spaceship (got {selected_now:?})"
                        ));
                        return;
                    }
                    info!("selection: Spaceship (via real pointer click)");
                }

                // --- Simulation ---------------------------------------------
                profiler.begin_frame(step as u64);
                profiler.time("physics", |_| {
                    world.resource_mut::<PhysicsResource>().world.step();
                });
                profiler.time("propagation", |_| schedules.run_frame(&mut world));
                shell.state.stats.step += 1;

                // --- Cinematic advance (before the editor frame) -------------
                let outcome = match advance_and_apply(&mut stage, &mut world, DT) {
                    Ok(outcome) => outcome,
                    Err(e) => {
                        error = Some(anyhow!("cinematic apply failed: {e}"));
                        return;
                    }
                };
                for event in outcome.events {
                    info!(event = %event.name, "cinematic event fired");
                    cinematic_events.push(event.name);
                }
                // Refresh the snapshot the Animator + Game View tabs read.
                let pose = world.get::<CameraPose>(cine_camera).copied();
                shell.state.cinematic = Some(gens_editor::cinema::CinematicPanelData {
                    duration: stage.player.timeline().duration(),
                    local_time: stage.player.local_time(),
                    state: format!("{:?}", stage.player.state()),
                    camera_live: outcome.camera_live,
                    camera_pose: pose,
                    tracks: track_rows.clone(),
                    events: cinematic_events.clone(),
                });
                if step == 20 {
                    // Scripted tab switch: the last frames render through
                    // the cinematic camera (Game View), proving the
                    // director-owned camera path end to end in CI.
                    shell.state.view_tab = Some(gens_editor::shell::ViewTab::GameView);
                    info!("view tab: Game View (cinematic camera)");
                }

                // --- Editor frame -------------------------------------------
                let input = UiInput {
                    time_seconds: time,
                    dt_seconds: DT,
                    size_px: WINDOW_SIZE,
                    events,
                };
                let frame_result =
                    profiler.time("editor_ui", |_| shell.frame(input, &mut world, &assets));
                match frame_result {
                    Ok(pixels) => {
                        frame_hashes.push(fnv1a64(&pixels));
                        if capture_png && step + 1 == steps {
                            match image::RgbaImage::from_raw(WINDOW_SIZE[0], WINDOW_SIZE[1], pixels)
                                .ok_or_else(|| anyhow!("composite size mismatch"))
                            {
                                Ok(image) => {
                                    let png_path = format!("{}.png", out_dir.display());
                                    if let Err(e) = image
                                        .save(&png_path)
                                        .with_context(|| format!("writing {png_path}"))
                                    {
                                        error = Some(e);
                                        return;
                                    }
                                    info!(png_path, "editor snapshot written");
                                }
                                Err(e) => {
                                    error = Some(e);
                                    return;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error = Some(e);
                        return;
                    }
                }
                profiler.end_frame();
                step_done += 1;
            });
            if error.is_some() {
                break;
            }
        }
    });

    if let Some(e) = error {
        return Err(e);
    }
    ensure!(
        step_done == steps,
        "run stopped early: {step_done}/{steps} frames"
    );
    let profile = gens_profiler::report::report_from_history(profiler.history());
    Ok(RunReport {
        frame_hashes,
        duplicated_name,
        clicked,
        validated_clean,
        cinematic_events,
        profile,
    })
}

/// Builds the reference scene; returns a name -> entity map.
fn build_scene(world: &mut World, physics: &mut PhysicsWorld) -> Result<BTreeMap<String, Entity>> {
    let mut names: BTreeMap<String, Entity> = BTreeMap::new();
    fn spawn(
        world: &mut World,
        names: &mut BTreeMap<String, Entity>,
        name: &str,
        bundle: impl gens_ecs::ecs::prelude::Bundle,
    ) -> Entity {
        let e = world.spawn((Name::new(name), Active, bundle)).id();
        names.insert(name.to_string(), e);
        e
    }

    // World
    let world_root = spawn(world, &mut names, "World", Transform::IDENTITY);
    let dir_light = spawn(
        world,
        &mut names,
        "DirectionalLight",
        (Transform::IDENTITY, MeshSlot::marker([255, 230, 120, 255])),
    );
    let skybox = spawn(
        world,
        &mut names,
        "Skybox",
        (Transform::IDENTITY, MeshSlot::marker([90, 140, 255, 255])),
    );
    let environment = spawn(
        world,
        &mut names,
        "Environment",
        (Transform::IDENTITY, MeshSlot::marker([90, 220, 160, 255])),
    );

    // Player
    let player = spawn(world, &mut names, "Player", Transform::IDENTITY);
    let player_character = spawn(
        world,
        &mut names,
        "PlayerCharacter",
        (
            Transform::from_position(vec3(-2.0, 0.5, 2.0)),
            MeshSlot::cube([70, 130, 240, 255]),
        ),
    );
    let camera = spawn(
        world,
        &mut names,
        "Camera",
        (
            Transform::from_position(vec3(0.0, 2.0, 4.0)),
            MeshSlot::marker([200, 200, 210, 255]),
        ),
    );
    let armature = spawn(
        world,
        &mut names,
        "Armature",
        (Transform::IDENTITY, MeshSlot::marker([255, 160, 90, 255])),
    );

    // Level: terrain quad rotated flat (quads face +Z; -90 degrees about X).
    let level = spawn(world, &mut names, "Level", Transform::IDENTITY);
    let terrain = spawn(
        world,
        &mut names,
        "Terrain",
        (
            Transform {
                position: Vec3::ZERO,
                rotation: Quat::from_euler(
                    glam::EulerRot::YXZ,
                    0.0,
                    -std::f32::consts::FRAC_PI_2,
                    0.0,
                ),
                scale: vec3(30.0, 30.0, 1.0),
            },
            MeshSlot::quad([52, 60, 74, 255]),
        ),
    );
    let buildings = spawn(
        world,
        &mut names,
        "Buildings",
        (
            Transform::from_position(vec3(7.0, 1.5, -5.0)),
            MeshSlot::cube([140, 60, 60, 255]),
        ),
    );
    let props = spawn(
        world,
        &mut names,
        "Props",
        (
            Transform::from_position(vec3(-6.0, 0.4, -3.0)),
            MeshSlot::cube([180, 150, 90, 255]),
        ),
    );

    // SciFi_Station
    let station = spawn(world, &mut names, "SciFi_Station", Transform::IDENTITY);
    let bay = spawn(world, &mut names, "DockingBay", Transform::IDENTITY);
    let (ship_body, _) = physics.create_body(
        BodySpec::fixed_at(vec3(0.0, 0.8, 0.0)),
        ColliderSpec {
            shape: ColliderShape::Cuboid(vec3(1.5, 0.4, 0.75)),
            friction: 0.6,
            ..Default::default()
        },
    );
    let ship = spawn(
        world,
        &mut names,
        "Spaceship",
        (
            Transform {
                position: vec3(0.0, 0.8, 0.0),
                rotation: Quat::IDENTITY,
                scale: vec3(3.0, 0.8, 1.5),
            },
            MeshSlot::cube([168, 178, 196, 255]),
            PhysicsBody { handle: ship_body },
            ScriptComponent::new("scripts/patrol.rhai"),
        ),
    );
    let ship_mesh = spawn(
        world,
        &mut names,
        "Mesh",
        (Transform::IDENTITY, MeshSlot::marker([168, 178, 196, 255])),
    );
    let ship_collider = spawn(
        world,
        &mut names,
        "Collider",
        (Transform::IDENTITY, MeshSlot::marker([120, 220, 120, 255])),
    );
    let point_light = spawn(
        world,
        &mut names,
        "PointLight",
        (
            Transform::from_position(vec3(0.0, 2.2, 0.0)),
            MeshSlot::marker([255, 220, 120, 255]),
        ),
    );
    let terminal = spawn(
        world,
        &mut names,
        "Terminal",
        (
            Transform::from_position(vec3(3.5, 0.6, 2.5)),
            MeshSlot::cube([60, 70, 90, 255]),
        ),
    );
    let (crate_body, _) = physics.create_body(
        BodySpec {
            ccd: true,
            ..BodySpec::dynamic_at(vec3(1.0, 4.0, -1.0))
        },
        ColliderSpec {
            shape: ColliderShape::Ball(0.45),
            friction: 0.5,
            ..Default::default()
        },
    );
    let crates = spawn(
        world,
        &mut names,
        "Crates",
        (
            Transform::from_position(vec3(1.0, 4.0, -1.0)),
            MeshSlot::cube([185, 130, 70, 255]),
            PhysicsBody { handle: crate_body },
        ),
    );
    let lights = spawn(
        world,
        &mut names,
        "Lights",
        (
            Transform::from_position(vec3(-3.0, 2.0, 0.0)),
            MeshSlot::marker([255, 240, 180, 255]),
        ),
    );

    // AI
    let ai = spawn(world, &mut names, "AI", Transform::IDENTITY);
    let guard1 = spawn(
        world,
        &mut names,
        "Guard_01",
        (
            Transform::from_position(vec3(-4.0, 0.4, -2.0)),
            MeshSlot::cube([200, 70, 70, 255]),
        ),
    );
    let guard2 = spawn(
        world,
        &mut names,
        "Guard_02",
        (
            Transform::from_position(vec3(-5.0, 0.4, -1.0)),
            MeshSlot::cube([200, 70, 70, 255]),
        ),
    );
    let guard3 = spawn(
        world,
        &mut names,
        "Guard_03",
        (
            Transform::from_position(vec3(-6.0, 0.4, 0.0)),
            MeshSlot::cube([200, 70, 70, 255]),
        ),
    );

    // Hierarchy per the reference.
    attach_children(world, world_root, &[dir_light, skybox, environment]);
    attach_children(world, player, &[player_character, camera, armature]);
    attach_children(world, level, &[terrain, buildings, props]);
    attach_children(world, station, &[bay]);
    attach_children(world, bay, &[ship, ship_mesh, ship_collider, point_light]);
    attach_children(world, station, &[terminal, crates, lights]);
    attach_children(world, ai, &[guard1, guard2, guard3]);
    Ok(names)
}

/// Imports REAL content into the registry (script source + raw bytes).
fn build_assets() -> Result<AssetRegistry> {
    let mut registry = AssetRegistry::new();
    registry
        .import(
            "scripts/patrol.rhai",
            AssetKind::Script,
            PATROL_RHAI.as_bytes(),
        )
        .context("importing patrol script")?;
    // A tiny REAL PNG (8x8 amber checker) — encoded, not fabricated.
    let img = image::RgbaImage::from_fn(8, 8, |x, y| {
        if (x + y) % 2 == 0 {
            image::Rgba([244, 202, 88, 255])
        } else {
            image::Rgba([24, 28, 36, 255])
        }
    });
    let mut png_bytes = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png_bytes, image::ImageFormat::Png)
        .context("encoding demo icon")?;
    registry
        .import(
            "textures/ui/icon_warning.png",
            AssetKind::Raw,
            png_bytes.get_ref(),
        )
        .context("importing demo icon")?;
    Ok(registry)
}

/// Scripted interaction events (real pointer traffic into egui).
fn events_for(step: usize, state: &gens_editor::EditorState) -> Vec<UiEvent> {
    match step {
        // Click the Spaceship row (center of its layout-captured rect).
        5 => state
            .row_rects
            .get("Spaceship")
            .map(|rect| {
                let cx = rect.center().x;
                let cy = rect.center().y;
                vec![
                    UiEvent::PointerMove { x: cx - 4.0, y: cy },
                    UiEvent::PointerMove { x: cx, y: cy },
                    UiEvent::PointerButton {
                        x: cx,
                        y: cy,
                        button: 0,
                        pressed: true,
                    },
                    UiEvent::PointerButton {
                        x: cx,
                        y: cy,
                        button: 0,
                        pressed: false,
                    },
                ]
            })
            .unwrap_or_default(),
        // Orbit drag across the viewport center.
        8 => vec![
            UiEvent::PointerMove { x: 800.0, y: 420.0 },
            UiEvent::PointerButton {
                x: 800.0,
                y: 420.0,
                button: 0,
                pressed: true,
            },
        ],
        9..=11 => vec![UiEvent::PointerMove {
            x: 800.0 + 14.0 * (step - 8) as f32,
            y: 420.0 - 5.0 * (step - 8) as f32,
        }],
        12 => vec![
            UiEvent::PointerMove { x: 842.0, y: 405.0 },
            UiEvent::PointerButton {
                x: 842.0,
                y: 405.0,
                button: 0,
                pressed: false,
            },
        ],
        _ => Vec::new(),
    }
}

/// Edit > Duplicate Selected — clones the known component set with a
/// " copy" name (the app-level implementation of the menu command).
fn duplicate_selected(world: &mut World, state: &mut gens_editor::EditorState) -> Option<String> {
    let sel = state.selected?;
    let name = world.get::<Name>(sel)?.as_str().to_string();
    let transform = world.get::<Transform>(sel).copied();
    let mesh = world.get::<MeshSlot>(sel).copied();
    let body = world.get::<gens_editor::AuthoredBody>(sel).copied();
    let script = world.get::<ScriptComponent>(sel).cloned();
    let new_name = format!("{name} copy");
    let mut eb = world.spawn((Name::new(new_name.clone()), Active));
    if let Some(t) = transform {
        eb.insert(t);
    }
    if let Some(m) = mesh {
        eb.insert(m);
    }
    if let Some(b) = body {
        eb.insert(b);
    }
    if let Some(s) = script {
        eb.insert(s);
    }
    Some(new_name)
}

/// FNV-1a 64 — the engine's standard content hash.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// The director's timeline: a 0.5 s take that dollies the CineCamera,
/// eases its yaw and FOV, and fires one marker. Literal f32 keys only
/// (the same input discipline as the cinematic replay proof).
fn director_timeline() -> Timeline {
    Timeline::builder(0.5)
        .vec3_track(
            "camera",
            gens_cinematics::Channel::Translation,
            gens_cinematics::Vec3Track::new(
                vec![
                    gens_cinematics::Vec3Key {
                        time: 0.0,
                        value: vec3(0.0, 2.0, 8.0),
                    },
                    gens_cinematics::Vec3Key {
                        time: 0.5,
                        value: vec3(1.2, 2.2, 7.2),
                    },
                ],
                gens_cinematics::Interpolation::Smoothstep,
            )
            .expect("camera translation track"),
        )
        .expect("camera translation")
        .quat_track(
            "camera",
            gens_cinematics::Channel::Rotation,
            gens_cinematics::QuatTrack::new(
                vec![
                    gens_cinematics::QuatKey {
                        time: 0.0,
                        value: Quat::IDENTITY,
                    },
                    gens_cinematics::QuatKey {
                        time: 0.5,
                        value: quat(0.0, -0.1, 0.0, 0.995),
                    },
                ],
                gens_cinematics::Interpolation::Smoothstep,
            )
            .expect("camera rotation track"),
        )
        .expect("camera rotation")
        .float_track(
            "camera",
            gens_cinematics::Channel::Fov,
            gens_cinematics::FloatTrack::new(
                vec![
                    gens_cinematics::FloatKey {
                        time: 0.0,
                        value: 60.0,
                    },
                    gens_cinematics::FloatKey {
                        time: 0.5,
                        value: 52.0,
                    },
                ],
                gens_cinematics::Interpolation::Linear,
            )
            .expect("camera fov track"),
        )
        .expect("camera fov")
        .event(0.1, "director-start")
        .expect("event")
        .camera(gens_cinematics::CameraCut {
            target: "camera".to_string(),
            hard_cut: true,
            blend_in: 0.0,
        })
        .build()
        .expect("timeline builds")
}

/// The Animator tab's track rows (target, channel label, key count) in
/// deterministic (target, channel) order.
fn track_rows(stage: &CinematicStage) -> Vec<gens_editor::cinema::TrackRow> {
    stage
        .player
        .timeline()
        .tracks()
        .map(|(addr, track)| gens_editor::cinema::TrackRow {
            target: addr.target.clone(),
            channel: gens_editor::cinema::channel_label(addr.channel),
            keys: match track {
                Track::Float(t) => t.keys().len(),
                Track::Vec3(t) => t.keys().len(),
                Track::Quat(t) => t.keys().len(),
            },
        })
        .collect()
}
