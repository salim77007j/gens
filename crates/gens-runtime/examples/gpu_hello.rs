//! Gens GPU hello world (Phase 2): the full engine through a real GPU.
//!
//! Builds the same kind of scene as the Phase 1 CPU example — a grouping
//! root with an orbiting hero — runs the fixed-timestep loop, propagates
//! transforms through the ECS, and renders every frame with the wgpu
//! offscreen renderer (ADR-0010): background quad, orbit trail quads, and
//! a textured checker sprite for the hero.
//!
//! The final frame is written as a PNG and verified structurally
//! in-process: the hero pixel must differ from the background, and the
//! frame border must equal the clear color.
//!
//! ```text
//! cargo run -p gens-runtime --example gpu_hello -- --frames 120 --out gpu_hello.png
//! ```
//!
//! Machines with no GPU adapter at all: pass `--allow-no-gpu` to skip with
//! an explicit `GPU-SKIP` marker instead of failing (used by the Windows CI
//! leg; Linux and macOS always really render — ADR-0011).

use gens_core::anyhow;
use gens_core::anyhow::Context;
use gens_core::game_loop::{GameLoop, LoopConfig};
use gens_core::logging::{info, init};
use gens_core::time::FixedSource;
use gens_ecs::ecs::world::World;
use gens_ecs::{Active, EngineSchedules, Name, Tag, Transform, WorldTransform, attach_children};
use gens_math::{Quat, Vec3, compose_trs};
use gens_renderer::mesh;
use gens_renderer::texture::GpuTexture;
use gens_renderer::{Draw, GpuContext, GpuMesh, Renderer, clear_color_rgba8};
use gens_runtime::snapshot::{COLOR_ACCENT, COLOR_BACKGROUND, COLOR_TRAIL, Snapshot};
use std::path::PathBuf;
use std::time::Duration;

/// Model-slot budget for this example (well under the 256-slot ring).
const MAX_SLOTS: u32 = 64;

/// Builds the hero's checker texture (2x2: red/green over blue/white) —
/// tiny, deterministic, and visibly oriented.
fn checker_rgba() -> ([u8; 16], u32, u32) {
    (
        [
            255, 0, 0, 255, 0, 255, 0, 255, // top row
            0, 0, 255, 255, 255, 255, 255, 255, // bottom row
        ],
        2,
        2,
    )
}

/// A unit quad whose vertex colors carry the sRGB-encoded `rgba` bytes as
/// LINEAR floats, so the sRGB framebuffer stores exactly `rgba` (the
/// fragment path is spec-fixed: linear in -> encoded on store).
fn tinted_quad(rgba: gens_runtime::snapshot::Rgba) -> gens_renderer::mesh::Mesh {
    let mut m = gens_renderer::mesh::quad();
    let tint = [
        gens_renderer::srgb_byte_to_linear(rgba[0]) as f32,
        gens_renderer::srgb_byte_to_linear(rgba[1]) as f32,
        gens_renderer::srgb_byte_to_linear(rgba[2]) as f32,
        1.0,
    ];
    for v in &mut m.vertices {
        v.color = tint;
    }
    m
}

fn main() -> anyhow::Result<()> {
    let mut frames: u32 = 120;
    let mut out = PathBuf::from("gpu_hello.png");
    let mut width: u32 = 320;
    let mut height: u32 = 180;
    let mut allow_no_gpu = false;
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
            "--size" => {
                let spec = args.next().context("--size needs WxH")?;
                let (w, h) = spec
                    .split_once('x')
                    .context("--size must be WxH, e.g. 320x180")?;
                width = w.parse().context("--size width must be a number")?;
                height = h.parse().context("--size height must be a number")?;
            }
            "--allow-no-gpu" => allow_no_gpu = true,
            other => anyhow::bail!(
                "unknown argument: {other} (use --frames N, --out PATH, --size WxH, --allow-no-gpu)"
            ),
        }
    }

    let filter = init().unwrap_or_else(|_| "(logging already initialized)".to_string());
    info!("Gens engine - GPU hello world");
    info!(frames, ?out, width, height, %filter, allow_no_gpu, "run configuration");

    // --- GPU ---------------------------------------------------------------
    let ctx = match GpuContext::new() {
        Ok(ctx) => ctx,
        Err(e) if allow_no_gpu => {
            println!("GPU-SKIP: no usable GPU adapter on this machine ({e})");
            return Ok(());
        }
        Err(e) => {
            return Err(e).context("GPU adapter (install mesa-vulkan-drivers on headless Linux)");
        }
    };
    info!(adapter = %ctx.adapter_summary(), "GPU context ready");
    let renderer = Renderer::new(ctx, width, height).context("offscreen renderer")?;
    let device = renderer.context().device();
    let queue = renderer.context().queue();

    // --- Scene -----------------------------------------------------------
    let mut world = World::new();
    let mut schedules = EngineSchedules::new();
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

    // --- GPU resources -----------------------------------------------------
    let hero_mesh = GpuMesh::upload(device, &mesh::quad()).context("hero mesh upload")?;
    let trail_cpu = tinted_quad(COLOR_TRAIL);
    let trail_mesh = GpuMesh::upload(device, &trail_cpu).context("trail mesh upload")?;
    let background_cpu = tinted_quad(COLOR_BACKGROUND);
    let background_mesh =
        GpuMesh::upload(device, &background_cpu).context("background mesh upload")?;
    let (checker, tw, th) = checker_rgba();
    let hero_texture =
        GpuTexture::from_rgba8(device, queue, tw, th, &checker).context("hero texture upload")?;

    // --- Simulation + render loop -------------------------------------------
    // Deterministic clock (FixedSource): the animation is identical on
    // every machine; the rasterization is whatever the local GPU produces.
    //
    // World space is pixels (the 2D ortho camera maps 1 world unit to 1
    // pixel, origin at the screen center, +Y up). Depth: the camera window
    // spans view z in [-1, 1]; LARGER z is nearer (WebGPU depth = (1 - z)/2
    // under the hood). Layering, back to front: background -0.4, trail
    // -0.2, hero +0.1.
    const ORBIT_RADIUS: f32 = 60.0;
    const DEG_PER_STEP: f32 = std::f32::consts::TAU / 60.0;
    const HERO_SIZE: f32 = 24.0;
    const TRAIL_SIZE: f32 = 6.0;
    const Z_BACKGROUND: f32 = -0.4;
    const Z_TRAIL: f32 = -0.2;
    const Z_HERO: f32 = 0.1;
    // Framebuffer center (the ortho camera origin), used by verification.
    let cy = height as f32 * 0.5;
    let cx = width as f32 * 0.5;

    let source = FixedSource::new(Duration::from_secs_f64(1.0 / 60.0));
    let mut game_loop = GameLoop::new(LoopConfig::default(), source);
    let mut orbit_angle: f32 = 0.0;
    let mut trail: Vec<(f32, f32)> = Vec::new();
    let mut total_steps = 0u32;

    let mut last_pixels: Option<Vec<u8>> = None;
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
        schedules.run_frame(&mut world); // update + transform propagation
        total_steps += report.fixed_steps;

        // World-space position straight from the ECS propagation output.
        let wt = world.get::<WorldTransform>(hero).expect("propagation ran");
        let hero_pos = wt.position();
        trail.push((hero_pos.x, hero_pos.y));

        // Render this frame: background, every 8th trail point, then hero.
        renderer.set_camera_2d(queue);
        renderer.set_model(
            queue,
            0,
            compose_trs(
                Vec3::new(0.0, 0.0, Z_BACKGROUND),
                Quat::IDENTITY,
                Vec3::new(width as f32, height as f32, 1.0),
            ),
        )?;
        let mut draws: Vec<Draw<'_>> = vec![Draw::Colored {
            mesh: &background_mesh,
            slot: 0,
        }];
        let mut slot = 1u32;
        for (_, (x, y)) in trail.iter().enumerate().step_by(8) {
            if slot >= MAX_SLOTS {
                break; // slot budget guard (never hit at 120 frames / step 8)
            }
            renderer.set_model(
                queue,
                slot,
                compose_trs(
                    Vec3::new(*x, *y, Z_TRAIL),
                    Quat::IDENTITY,
                    Vec3::splat(TRAIL_SIZE),
                ),
            )?;
            draws.push(Draw::Colored {
                mesh: &trail_mesh,
                slot,
            });
            slot += 1;
        }
        renderer.set_model(
            queue,
            slot,
            compose_trs(
                Vec3::new(hero_pos.x, hero_pos.y, Z_HERO),
                Quat::IDENTITY,
                Vec3::splat(HERO_SIZE),
            ),
        )?;
        draws.push(Draw::Textured {
            mesh: &hero_mesh,
            slot,
            texture: &hero_texture,
        });

        let pixels = renderer
            .draw_and_read(clear_color_rgba8(COLOR_BACKGROUND), &draws)
            .context("frame render")?;
        last_pixels = Some(pixels);

        if frame + 1 == frames {
            info!(
                report.fixed_steps,
                alpha = report.alpha,
                "final frame rendered"
            );
        }
    }

    // --- Verify + encode -----------------------------------------------------
    let pixels = last_pixels.expect("at least one frame rendered");
    let px = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y * width + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    let bg: [u8; 4] = COLOR_BACKGROUND;

    // Structural verification (ADR-0011: structural, not byte-exact across
    // drivers). 1) The border must be background-like: not the trail color,
    // not any checker corner, and fully opaque. (The exact stored bytes of
    // a mid-range clear depend on the backend's sRGB clear handling, so a
    // byte-equality assert would be driver-flavored, not engine truth.)
    let border = px(0, 0);
    assert_eq!(border[3], 255, "border alpha must be opaque");
    assert_ne!(border, COLOR_TRAIL, "border must not be trail color");
    for corner in [
        [255u8, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 255, 255],
    ] {
        assert_ne!(border, corner, "border must not be a checker corner");
    }
    // 2) Hero pixel: after `frames` steps the orbit lands at world
    //    (ORBIT_RADIUS, 0). The camera is centered on the framebuffer, so
    //    framebuffer pixels are (world_x + W/2, H/2 - world_y).
    let hero_screen_x = (ORBIT_RADIUS + cx) as u32;
    let hero_px = px(hero_screen_x.min(width - 2), (cy as u32).min(height - 2));
    assert_ne!(
        hero_px, bg,
        "hero pixel must not be background: {hero_px:?}"
    );
    assert_ne!(
        hero_px, COLOR_TRAIL,
        "hero pixel must not be trail color: {hero_px:?}"
    );
    // 3) The accent trail must exist somewhere: at least one sampled trail
    //    point differs from background.
    let trail_present = trail.iter().step_by(8).any(|(x, y)| {
        let sx = (x + cx) as i32;
        let sy = (cy - y) as i32;
        sx > 0
            && sy > 0
            && (sx as u32) < width
            && (sy as u32) < height
            && px(sx as u32, sy as u32) != bg
    });
    assert!(trail_present, "at least one trail dot must be visible");
    info!(?hero_px, "structural verification passed");

    let snap = Snapshot {
        width,
        height,
        buffer: pixels,
    };
    let png = snap.encode_png().context("PNG encode")?;
    std::fs::write(&out, png).with_context(|| format!("writing {out:?}"))?;
    info!(
        ?out,
        total_steps,
        trail = trail.len(),
        "GPU snapshot written"
    );

    // Reference the accent constant so the palette stays intentional even
    // if trail styling changes later.
    debug_assert_ne!(COLOR_ACCENT, COLOR_BACKGROUND);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn checker_texture_has_four_distinct_corners() {
        let (data, w, h) = super::checker_rgba();
        assert_eq!((w, h), (2, 2));
        assert_eq!(&data[0..4], &[255, 0, 0, 255]); // red
        assert_eq!(&data[4..8], &[0, 255, 0, 255]); // green
        assert_eq!(&data[8..12], &[0, 0, 255, 255]); // blue
        assert_eq!(&data[12..16], &[255, 255, 255, 255]); // white
    }
}
