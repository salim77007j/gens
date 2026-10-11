//! Gens hello world (Phase 1): a full headless engine run.
//!
//! Builds a small scene, runs the fixed-timestep game loop for N frames,
//! animates an entity through the ECS, propagates transforms, and writes a
//! deterministic PNG snapshot of the final scene state. No window, no GPU —
//! this is the CI smoke test binary for Phase 1.
//!
//! ```text
//! cargo run -p gens-runtime --example hello_world -- --frames 120 --out hello_world.png
//! ```

use gens_core::anyhow;
use gens_core::anyhow::Context;
use gens_core::game_loop::{GameLoop, LoopConfig};
use gens_core::logging::{info, init};
use gens_core::time::FixedSource;
// Explicit imports (not the prelude glob) because bevy_ecs 0.20's prelude
// carries a `ContextExt` trait whose `context` method collides with
// `anyhow::Context` used below. See the prelude docs for the caveat.
use gens_ecs::ecs::world::World;
use gens_ecs::{Active, EngineSchedules, Name, Tag, Transform, WorldTransform, attach_children};
use gens_math::Vec3;
use gens_runtime::snapshot::{
    COLOR_ACCENT, COLOR_BACKGROUND, COLOR_GRID, COLOR_TEXT, COLOR_TRAIL, Snapshot,
};
use std::path::PathBuf;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let mut frames: u32 = 120;
    let mut out = PathBuf::from("hello_world.png");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--frames" => {
                frames = args
                    .next()
                    .context("--frames needs a value")?
                    .parse()
                    .context("--frames must be a number")?;
            }
            "--out" => {
                out = PathBuf::from(args.next().context("--out needs a value")?);
            }
            other => anyhow::bail!("unknown argument: {other} (use --frames N, --out PATH)"),
        }
    }

    let filter = init().unwrap_or_else(|_| "(logging already initialized)".to_string());
    info!("Gens engine - hello world");
    info!(frames, ?out, %filter, "run configuration");

    // --- Scene -----------------------------------------------------------
    let mut world = World::new();
    let mut schedules = EngineSchedules::new();

    // Root is a pure grouping node (no Transform) to exercise pass-through
    // propagation; Hero orbits it and carries the player tag.
    let root = world.spawn((Name::new("Root"), Active)).id();
    let hero = world
        .spawn((
            Name::new("Hero"),
            Transform::IDENTITY,
            Active,
            Tag::new("player"),
        ))
        .id();
    attach_children(&mut world, root, &[hero]);
    info!(?root, ?hero, "scene built");

    // --- Simulation ------------------------------------------------------
    // Deterministic: a FixedSource feeds the loop 1/60 s per read, so the
    // run produces identical output on every machine and every CI runner.
    const ORBIT_RADIUS: f32 = 60.0;
    const DEG_PER_STEP: f32 = std::f32::consts::TAU / 60.0;
    let source = FixedSource::new(Duration::from_secs_f64(1.0 / 60.0));
    let mut game_loop = GameLoop::new(LoopConfig::default(), source);
    let mut orbit_angle: f32 = 0.0;
    let mut trail: Vec<(f32, f32)> = Vec::new();
    let mut total_steps = 0u32;
    let mut hitched_frames = 0u32;

    for frame in 0..frames {
        let report = game_loop.run_frame(|_step| {
            orbit_angle += DEG_PER_STEP;
            let mut t = world.get_mut::<Transform>(hero).expect("hero exists");
            t.position = Vec3::new(
                orbit_angle.cos() * ORBIT_RADIUS,
                orbit_angle.sin() * ORBIT_RADIUS,
                0.0,
            );
        });
        schedules.run_frame(&mut world); // update + propagation
        total_steps += report.fixed_steps;
        hitched_frames += u32::from(report.backlog_dropped);

        // Record the ECS-computed world position (not the analytic one) so
        // the snapshot genuinely reflects transform propagation output.
        let wt = world.get::<WorldTransform>(hero).expect("propagation ran");
        trail.push((wt.position().x, wt.position().y));
        if frame + 1 == frames {
            info!(report.fixed_steps, alpha = report.alpha, "final frame");
        }
    }

    let final_pos = trail.last().copied().unwrap_or((0.0, 0.0));
    info!(
        frames,
        total_steps,
        hitched_frames,
        hero_x = final_pos.0,
        hero_y = final_pos.1,
        "simulation complete"
    );

    // --- Snapshot ---------------------------------------------------------
    // World space -> pixels: center origin, 2 px per world unit.
    const W: u32 = 640;
    const H: u32 = 360;
    const SCALE: f32 = 2.0;
    let cx = (W as f32 * 0.5) as i32;
    let cy = (H as f32 * 0.5) as i32;

    let mut snap = Snapshot::new(W, H, COLOR_BACKGROUND);
    snap.draw_grid(32, COLOR_GRID);
    snap.stroke_rect(0, 0, W, H, COLOR_GRID);
    snap.draw_text(12, 10, "GENS ENGINE - PHASE 1 - SCENE SNAPSHOT", COLOR_TEXT);
    snap.draw_text(
        12,
        20,
        &format!("FRAME = {frames}  STEPS = {total_steps}"),
        COLOR_TEXT,
    );
    snap.draw_text(
        12,
        30,
        &format!(
            "HERO = ({:.0}, {:.0})  RADIUS = {ORBIT_RADIUS}",
            final_pos.0, final_pos.1
        ),
        COLOR_TEXT,
    );

    for (x, y) in &trail {
        let px = cx + (x * SCALE) as i32;
        let py = cy - (y * SCALE) as i32; // screen Y is down
        snap.fill_rect(px - 1, py - 1, 2, 2, COLOR_TRAIL);
    }
    let hpx = cx + (final_pos.0 * SCALE) as i32;
    let hpy = cy - (final_pos.1 * SCALE) as i32;
    snap.fill_rect(hpx - 3, hpy - 3, 6, 6, COLOR_ACCENT);
    snap.draw_text(
        12,
        H as i32 - 14,
        "CPU SNAPSHOT - DETERMINISTIC - NO GPU",
        COLOR_TEXT,
    );

    let png = snap.encode_png()?;
    std::fs::write(&out, png).with_context(|| format!("writing {out:?}"))?;
    info!(?out, "snapshot written");
    Ok(())
}
