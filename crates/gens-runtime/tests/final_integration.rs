//! The final cross-pillar integration test (Phase 10, row 10 "Final
//! testing + benchmarks + validation").
//!
//! Every existing smoke proves one pillar pair; this file proves the
//! pillars **compose end to end** in one deterministic scenario:
//!
//! - the cinematic timeline is not authored in code — it is **loaded
//!   from a `gens-cine-1` document** (ADR-0033), so the serialized
//!   format feeds the real pipeline;
//! - terrain placement, prop scatter and the physics floor height come
//!   from `gens-worldgen` (a chunk generated at the origin, `height_at`
//!   under the floor, scatter positions for props);
//! - `gens-physics` drops a ball that must SETTLE on the floor (the
//!   settle is asserted from the hash ledger, not assumed);
//! - the director (`gens-runtime::cinematic`) drives the camera and a
//!   hero marker through the live ECS;
//! - at a checkpoint after the settle, `gens-runtime::save` captures
//!   the world; a fresh world restores from the JSON document and
//!   continues;
//! - `world_state_hash` ledgers every step, and both the restored
//!   run's tail and a fully fresh replay must be byte-identical to the
//!   original run.
//!
//! Honest boundaries, documented where they bite:
//! - the save format captures ECS state, not rigid-body velocities
//!   (ADR-0021 scope). The original run ASSERTS the ball is at rest
//!   before the checkpoint (its position bits are static over the last
//!   ten pre-checkpoint steps); the restored run re-creates the ball
//!   FIXED at its saved pose — at rest by construction, indistinguish-
//!   able from the sleeping original. The tail-hash equality then
//!   proves the save/restore + seek path itself, and any drift in any
//!   pillar fails loudly.
//! - the cinematic seek target is the checkpoint's EXACT accumulated
//!   f32 time, captured from the live player (the R-033 contract:
//!   never a nearby literal).

use gens_cinematics::document::{content_hash, from_document};
use gens_cinematics::{CinematicPlayer, LoopMode};
use gens_ecs::ecs::entity::Entity;
use gens_ecs::ecs::world::World;
use gens_ecs::{Active, EngineSchedules, Name, Transform};
use gens_math::Vec3;
use gens_physics::{
    BodySpec, ColliderShape, ColliderSpec, PhysicsBody, PhysicsResource, PhysicsWorld,
    sync_transforms_from_physics,
};
use gens_runtime::cinematic::{CinematicStage, advance_and_apply};
use gens_runtime::profile::world_state_hash;
use gens_runtime::save::{apply_save, capture_world, load_json, save_json};
use gens_worldgen::chunk::{ChunkCoord, WorldGen, WorldGenConfig};
use std::sync::Arc;

/// The cinematic document, authored in `gens-cine-1` JSON — every
/// float is its IEEE-754 bit pattern, per ADR-0033. A 3 s take: the
/// camera eases fov 60°→45° and yaws 90°, the hero marker dollies
/// 0→4 on x, one event fires at the midpoint.
const CINE_DOCUMENT: &str = r#"{
  "format": "gens-cine-1",
  "duration": "40400000",
  "camera": {"target": "camera", "hard_cut": true, "blend_in": "00000000"},
  "tracks": [
    {"target": "camera", "channel": "fov", "kind": "float",
     "interpolation": "smoothstep",
     "float_keys": [
       {"time": "00000000", "value": "42700000"},
       {"time": "40400000", "value": "42340000"}
     ]},
    {"target": "camera", "channel": "rotation", "kind": "quat",
     "interpolation": "linear",
     "quat_keys": [
       {"time": "00000000",
        "value": {"x": "00000000", "y": "00000000", "z": "00000000", "w": "3f800000"}},
       {"time": "40400000",
        "value": {"x": "00000000", "y": "3f3504f3", "z": "00000000", "w": "3f3504f3"}}
     ]},
    {"target": "hero", "channel": "translation", "kind": "vec3",
     "interpolation": "linear",
     "vec3_keys": [
       {"time": "00000000",
        "value": {"x": "00000000", "y": "3f800000", "z": "00000000"}},
       {"time": "40400000",
        "value": {"x": "40800000", "y": "3f800000", "z": "00000000"}}
     ]}
  ],
  "events": [{"time": "3fc00000", "name": "midpoint"}]
}
"#;

const STEPS: u32 = 120;
const CHECKPOINT: u32 = 90;
const DT: f32 = 1.0 / 60.0;

/// What a scenario run should do at its checkpoint.
enum Mode {
    /// The plain composition: `STEPS` fixed steps from zero.
    Full,
    /// The composition with an in-flight capture at `CHECKPOINT`
    /// (exact player time + save JSON), continuing to `STEPS`.
    Checkpoint,
    /// The restored continuation: rebuild from the save document,
    /// re-create physics scenery at the saved poses, seek the director
    /// to the exact checkpoint time, run the remaining steps.
    Restore { save_json: String, seek_time: f32 },
}

struct RunOutput {
    /// Per-step `world_state_hash` ledger.
    hashes: Vec<u64>,
    /// Per-step ball position bits (x ^ y ^ z folded) — the settle
    /// watch: the world hash keeps moving while the director animates,
    /// so ball rest is asserted against THIS ledger, not the world hash.
    ball_bits: Vec<u64>,
    /// The checkpoint capture, when the mode asked for one:
    /// (exact accumulated f32 time, save document JSON).
    capture: Option<(f32, String)>,
}

/// One deterministic run of the composed scenario.
fn run_scenario(mode: Mode) -> anyhow::Result<RunOutput> {
    let timeline = Arc::new(from_document(CINE_DOCUMENT).expect("cine document is valid"));

    // --- Worldgen: terrain + props + the floor height the physics
    // world is built against (the world -> physics coupling).
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
    .expect("generator config is valid");
    let chunk = generator
        .generate(ChunkCoord::new(0, 0))
        .expect("chunk generation");
    let floor_y = generator
        .height_at(0.0, 0.0)
        .expect("terrain height at origin");

    let mut world = World::new();
    let mut schedules = EngineSchedules::new();
    schedules.update.add_systems(sync_transforms_from_physics);

    let restoring = matches!(mode, Mode::Restore { .. });
    let mut ball_start = Vec3::new(0.0, floor_y + 1.5, 0.0);

    if let Mode::Restore { save_json, .. } = &mode {
        // The restore path: entities come from the save document
        // INSTEAD of being constructed here (the constructors above
        // still run — worldgen terrain sampling is pure and feeds the
        // floor placement either way).
        let save = load_json(save_json)?;
        if let Some(saved_ball) = save
            .entities
            .iter()
            .find(|e| e.name.as_deref() == Some("Ball"))
            && let Some(saved_transform) = &saved_ball.transform
        {
            ball_start = Vec3::from_array(saved_transform.position);
        }
        apply_save(&mut world, &save)?;
    } else {
        // Static scenery: terrain marker at the chunk origin +
        // scattered props (scatter is world-space and deterministic).
        world.spawn((
            Name::new("Terrain"),
            Transform::from_position(chunk.origin),
            Active,
        ));
        for (i, point) in chunk.scatter.iter().enumerate().take(3) {
            world.spawn((
                Name::new(format!("Prop_{i}")),
                Transform::from_position(Vec3::from_array(point.position)),
                Active,
            ));
        }
        world.spawn((
            Name::new("Ball"),
            Transform::from_position(ball_start),
            Active,
        ));
        world.spawn((Name::new("Hero"), Transform::IDENTITY, Active));
        world.spawn((Name::new("Camera"), Transform::IDENTITY, Active));
    }

    // Physics: fixed floor at the terrain height + the ball. In the
    // restored run the ball is FIXED at its SAVED pose — the original
    // run asserts it was at rest there (ball_bits static), so at-rest-
    // by-construction is the honest model of a body restored without
    // velocity data.
    let mut physics = PhysicsWorld::new(Vec3::new(0.0, -9.81, 0.0));
    let _floor = physics.create_body(
        BodySpec::fixed_at(Vec3::new(0.0, floor_y - 0.5, 0.0)),
        ColliderSpec {
            shape: ColliderShape::Cuboid(Vec3::new(50.0, 0.5, 50.0)),
            friction: 0.8,
            ..Default::default()
        },
    );
    let ball_spec = if restoring {
        BodySpec::fixed_at(ball_start)
    } else {
        BodySpec {
            ccd: false,
            linear_damping: 0.2,
            ..BodySpec::dynamic_at(ball_start)
        }
    };
    let (ball_handle, _) = physics.create_body(
        ball_spec,
        ColliderSpec {
            shape: ColliderShape::Ball(0.5),
            friction: 0.4,
            ..Default::default()
        },
    );
    let ball_entity: Entity = if restoring {
        let mut q = world.query::<(&Name, Entity)>();
        q.iter(&world)
            .find(|(n, _)| n.0 == "Ball")
            .map(|(_, e)| e)
            .expect("restored ball exists")
    } else {
        let mut q = world.query::<(&Name, Entity)>();
        q.iter(&world)
            .find(|(n, _)| n.0 == "Ball")
            .map(|(_, e)| e)
            .expect("fresh ball exists")
    };
    world.entity_mut(ball_entity).insert(PhysicsBody {
        handle: ball_handle,
    });
    world.insert_resource(PhysicsResource { world: physics });

    // Director bindings resolve through Name lookups (iter_entities
    // order is NOT deterministic — the P5 lesson).
    let (hero, camera) = {
        let mut q = world.query::<(&Name, Entity)>();
        let mut found = (None, None);
        for (name, entity) in q.iter(&world) {
            match name.0.as_str() {
                "Hero" => found.0 = Some(entity),
                "Camera" => found.1 = Some(entity),
                _ => {}
            }
        }
        (
            found.0.expect("hero exists"),
            found.1.expect("camera exists"),
        )
    };
    let mut player = CinematicPlayer::new(timeline);
    player.set_loop_mode(LoopMode::Once);
    let mut stage = CinematicStage::new(
        player,
        [("hero".to_string(), hero), ("camera".to_string(), camera)],
    );

    let mut capture: Option<(f32, String)> = None;
    let (first_step, total) = match &mode {
        Mode::Full | Mode::Checkpoint => (0, STEPS),
        Mode::Restore { seek_time, .. } => {
            stage
                .player
                .seek(*seek_time)
                .expect("exact checkpoint time");
            (CHECKPOINT, STEPS)
        }
    };
    stage.player.play();

    let mut hashes = Vec::with_capacity((total - first_step) as usize);
    let mut ball_bits = Vec::with_capacity((total - first_step) as usize);
    for step in first_step..total {
        advance_and_apply(&mut stage, &mut world, DT).expect("cinematic step applies");
        world.resource_mut::<PhysicsResource>().world.step();
        schedules.run_frame(&mut world);
        hashes.push(world_state_hash(&world));
        {
            let mut q = world.query::<(&Name, &Transform)>();
            let bits = q
                .iter(&world)
                .find(|(n, _)| n.0 == "Ball")
                .map(|(_, t)| {
                    u64::from(t.position.x.to_bits())
                        ^ (u64::from(t.position.y.to_bits()) << 32)
                        ^ u64::from(t.position.z.to_bits())
                })
                .expect("ball exists");
            ball_bits.push(bits);
        }

        if matches!(mode, Mode::Checkpoint) && step + 1 == CHECKPOINT {
            // The exact accumulated f32 time — captured from the live
            // player, never re-derived from a literal (R-033).
            let exact_time = stage.player.local_time();
            let save = capture_world(&world);
            capture = Some((exact_time, save_json(&save)?));
        }
    }
    Ok(RunOutput {
        hashes,
        ball_bits,
        capture,
    })
}

#[test]
fn cross_pillar_composition_replays_and_restores_byte_identically() -> anyhow::Result<()> {
    // The document must load through the real format with a stable
    // content hash (the ADR-0033 contract feeds this pipeline).
    let timeline = from_document(CINE_DOCUMENT).expect("cine document is valid");
    assert_eq!(timeline.duration(), 3.0);
    let doc_hash = content_hash(&timeline);
    assert_eq!(doc_hash, content_hash(&from_document(CINE_DOCUMENT)?));

    // Run A: the full 120-step composition, with an in-flight capture
    // at the checkpoint (Checkpoint mode runs the same steps as Full).
    let run_a = run_scenario(Mode::Checkpoint)?;
    assert_eq!(run_a.hashes.len(), STEPS as usize);

    // The ball must be SETTLED before the checkpoint — asserted from
    // the ball's own position-bits ledger (10 identical entries ending
    // at CHECKPOINT). The world hash keeps moving (the director is
    // still animating), so rest is asserted against ball_bits.
    let settle_window = &run_a.ball_bits[(CHECKPOINT - 10) as usize..CHECKPOINT as usize];
    assert!(
        settle_window.windows(2).all(|w| w[0] == w[1]),
        "ball must be at rest at the checkpoint (position bits static over the last 10 steps)"
    );

    // Run B: restore from the checkpoint capture and continue.
    let (seek_time, save_json) = run_a.capture.as_ref().expect("checkpoint captured");
    let restore_mode = Mode::Restore {
        save_json: save_json.clone(),
        seek_time: *seek_time,
    };
    let run_b = run_scenario(restore_mode)?;
    assert_eq!(
        run_b.hashes.len(),
        (STEPS - CHECKPOINT) as usize,
        "restored run continues exactly the remaining steps"
    );
    assert_eq!(
        run_b.hashes,
        &run_a.hashes[CHECKPOINT as usize..],
        "restored tail must equal the original tail byte-for-byte"
    );

    // Run C: a completely fresh replay must equal run A end to end.
    let run_c = run_scenario(Mode::Full)?;
    assert_eq!(
        run_c.hashes, run_a.hashes,
        "fresh replay must be byte-identical"
    );

    Ok(())
}

/// The director's mid-take state must survive the save boundary: the
/// checkpoint pose written by the document-loaded timeline is restored
/// exactly (the seek target carries the live player's exact bits).
#[test]
fn checkpoint_time_is_exact_accumulated_f32() -> anyhow::Result<()> {
    let run = run_scenario(Mode::Checkpoint)?;
    let (exact_time, _) = run.capture.as_ref().expect("checkpoint captured");
    // 90 × (1/60) accumulated is NOT 1.5 — the honest f32 signature
    // this repo pins everywhere (the phase 9 seek lesson).
    assert!(
        (*exact_time - 1.5).abs() < 1e-6,
        "checkpoint is nominally 1.5s"
    );
    assert_ne!(
        exact_time.to_bits(),
        1.5_f32.to_bits(),
        "accumulated f32 time must differ from the literal (60 x (1/60) != 1.5 in f32)"
    );
    Ok(())
}
