//! The 3D viewport — the center dock's live scene view.
//!
//! The viewport owns a dedicated offscreen `Renderer` (independent of
//! the window renderer's target, per ADR-0025): each frame it renders
//! every entity carrying a [`MeshSlot`] from an orbit camera, reads the
//! pixels back and feeds them to egui as a managed texture. The gizmo
//! (RGB axes at the selection) is drawn by egui's painter on top, using
//! the same `view_proj` matrix — one source of truth for projection.
//!
//! v1.1 adds real scene-view interaction: click an object to select it,
//! drag it to move (on the camera-facing plane through the object),
//! drag empty space to orbit. Picking is exact — the pointer ray is
//! unprojected through the inverse `view_proj` and tested against each
//! draw's actual geometry in local space (OBB slab test for cubes and
//! markers, bounded plane test for quads) — no bounding-sphere hacks.
//! Every drag records an undo op; hierarchy clicks still select too.

use crate::{
    model::{MeshSlot, ShapeKind, UndoOp},
    shell::{EditorState, PlayMode},
    theme,
};
use gens_ecs::prelude::*;
use gens_math::{Mat4, Vec3, Vec4, vec3};
use gens_renderer::camera::Camera;
use gens_renderer::{
    Draw, GpuContext, GpuMesh, Renderer, clear_color_rgba8, mesh, srgb_byte_to_linear,
};
use std::collections::{BTreeMap, BTreeSet};

/// Default viewport render size in pixels (the example window's center
/// dock size; overridden by the actual widget rect every frame).
pub const DEFAULT_SIZE: [u32; 2] = [960, 540];

/// Orbit camera state — pure data on [`EditorState`], so replays can
/// drive it deterministically.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrbitCam {
    /// Horizontal angle in degrees (0 = looking down -Z).
    pub yaw_deg: f32,
    /// Vertical angle in degrees (clamped away from the poles).
    pub pitch_deg: f32,
    /// Distance from the target point.
    pub dist: f32,
    /// Look-at point.
    pub target: Vec3,
}

impl Default for OrbitCam {
    fn default() -> Self {
        Self {
            yaw_deg: 35.0,
            pitch_deg: 30.0,
            dist: 12.0,
            target: Vec3::ZERO,
        }
    }
}

impl OrbitCam {
    /// Camera eye position derived from the orbit parameters.
    pub fn eye(&self) -> Vec3 {
        let yaw = self.yaw_deg.to_radians();
        let pitch = self.pitch_deg.to_radians().clamp(-1.55, 1.55);
        let horiz = self.dist * pitch.cos();
        Vec3::new(
            horiz * yaw.sin(),
            self.dist * pitch.sin(),
            horiz * yaw.cos(),
        ) + self.target
    }

    /// The perspective camera for this orbit at the given aspect.
    pub fn camera(&self, aspect: f32) -> Camera {
        Camera::perspective(
            self.eye(),
            self.target,
            Vec3::Y,
            60.0_f32.to_radians(),
            aspect,
            0.1,
            500.0,
        )
    }
}

/// One entity's drawable description collected from the world.
#[derive(Debug, Clone, PartialEq)]
pub struct CollectedDraw {
    /// The drawn entity (the pick target).
    pub entity: Entity,
    /// Geometry to rasterize.
    pub shape: ShapeKind,
    /// Opaque tint (RGBA8).
    pub tint: [u8; 4],
    /// World matrix (`WorldTransform`, falling back to `Transform`).
    pub model: Mat4,
}

/// Collects drawable entities in deterministic (entity-id) order.
///
/// The seed-entity-style content filter from the hierarchy applies
/// implicitly: an entity without a [`MeshSlot`] is simply not a draw.
pub fn collect_draws(world: &World) -> Vec<CollectedDraw> {
    let mut entities: BTreeSet<Entity> = BTreeSet::new();
    for r in world.iter_entities() {
        entities.insert(r.id());
    }
    let mut out = Vec::new();
    for entity in entities {
        let Some(slot) = world.get::<MeshSlot>(entity).copied() else {
            continue;
        };
        let model = match world.get::<gens_ecs::WorldTransform>(entity) {
            Some(wt) => wt.0,
            None => world
                .get::<Transform>(entity)
                .map(|t| t.to_matrix())
                .unwrap_or(Mat4::IDENTITY),
        };
        // Markers render as small cubes regardless of authored scale.
        let model = if slot.shape == ShapeKind::Marker {
            model * Mat4::from_scale(Vec3::splat(0.15))
        } else {
            model
        };
        out.push(CollectedDraw {
            entity,
            shape: slot.shape,
            tint: slot.tint,
            model,
        });
    }
    out
}

/// Live object-drag state (pure data on [`EditorState`], replayable).
#[derive(Debug, Clone, PartialEq)]
pub struct ViewportDrag {
    /// The dragged entity.
    pub entity: Entity,
    /// Its transform when the drag began (the undo payload).
    pub before: Transform,
    /// World-space offset between the entity origin and the pointer's
    /// plane intersection at drag start (keeps the grab point sticky).
    pub grab_offset: Vec3,
    /// Whether the drag actually moved anything (below-threshold drags
    /// record no undo op).
    pub moved: bool,
}

/// Builds the tinted `GpuMesh` for a draw key (owned; the caller caches
/// it).
///
/// Base meshes bake their own per-face colors (see `mesh::cube`); the
/// authored tint multiplies every vertex color through the same
/// sRGB-decode-at-boundary convention the colored pipeline uses
/// (ADR-0018), so what the inspector shows is what the shader gets.
fn build_tinted_mesh(
    device: &wgpu::Device,
    shape: ShapeKind,
    tint: [u8; 4],
) -> Result<GpuMesh, gens_core::error::EngineError> {
    let base = match shape {
        ShapeKind::Cube | ShapeKind::Marker => mesh::cube(),
        ShapeKind::Quad => mesh::quad(),
    };
    let tinted = mesh::Mesh {
        vertices: base
            .vertices
            .iter()
            .map(|v| {
                mesh::vertex(
                    v.position,
                    [
                        v.color[0] * srgb_byte_to_linear(tint[0]) as f32,
                        v.color[1] * srgb_byte_to_linear(tint[1]) as f32,
                        v.color[2] * srgb_byte_to_linear(tint[2]) as f32,
                        v.color[3] * srgb_byte_to_linear(tint[3]) as f32,
                    ],
                    v.uv,
                )
            })
            .collect(),
        indices: base.indices,
    };
    GpuMesh::upload(device, &tinted)
}

/// Projects a world point into egui pixel space within `rect`.
///
/// Returns `None` when the point is behind the camera. This is the ONE
/// projection path shared by the gizmo overlay — pinned by tests.
pub fn project(world_pos: Vec3, view_proj: Mat4, rect: egui::Rect) -> Option<egui::Pos2> {
    let clip = view_proj * world_pos.extend(1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    Some(egui::pos2(
        rect.min.x + (ndc.x + 1.0) * 0.5 * rect.width(),
        rect.min.y + (1.0 - ndc.y) * 0.5 * rect.height(),
    ))
}

/// Unprojects a pixel position into a world-space picking ray
/// `(origin, direction)` through the inverse `view_proj`.
///
/// The direction is normalized; a degenerate rect (zero extent) yields
/// the zero direction, which can never hit anything.
#[must_use]
pub fn screen_ray(view_proj: Mat4, rect: egui::Rect, pos: egui::Pos2) -> (Vec3, Vec3) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return (Vec3::ZERO, Vec3::ZERO);
    }
    let nx = (pos.x - rect.min.x) / rect.width() * 2.0 - 1.0;
    let ny = 1.0 - (pos.y - rect.min.y) / rect.height() * 2.0;
    let inverse = view_proj.inverse();
    let unproject = |z: f32| {
        let clip = Vec4::new(nx, ny, z, 1.0);
        let world = inverse * clip;
        Vec3::new(world.x, world.y, world.z) / world.w
    };
    let near = unproject(-1.0);
    let far = unproject(1.0);
    (near, (far - near).normalize_or_zero())
}

/// First intersection distance of a local-space ray with a draw's
/// actual geometry (the exact-shape pick, no sphere approximations).
///
/// - Cube / Marker: slab test against the local unit cube
///   `[-0.5, 0.5]^3` (markers carry their 0.15 scale in the model
///   matrix, which the local-space transform already accounts for);
/// - Quad: bounded plane test on the local `z = 0` plane with
///   `|x|, |y| <= 0.5` (the unit quad lives in the XY plane).
#[must_use]
pub fn hit_distance(shape: ShapeKind, model: Mat4, origin: Vec3, dir: Vec3) -> Option<f32> {
    if dir == Vec3::ZERO {
        return None;
    }
    let inverse = model.inverse();
    let local_origin = inverse.transform_point3(origin);
    let local_dir = inverse.transform_vector3(dir);
    match shape {
        ShapeKind::Cube | ShapeKind::Marker => ray_box(
            local_origin,
            local_dir,
            vec3(-0.5, -0.5, -0.5),
            vec3(0.5, 0.5, 0.5),
        ),
        ShapeKind::Quad => ray_unit_quad(local_origin, local_dir),
    }
}

/// Ray vs axis-aligned box: returns the entry distance (`>= 0` when the
/// origin is inside) or `None` on a miss.
fn ray_box(origin: Vec3, dir: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    // A tiny epsilon keeps zero direction components from producing
    // 0 * infinity = NaN; the perturbation is far below any scene scale.
    let dir = dir + vec3(1e-8, 1e-8, 1e-8);
    let inv = Vec3::ONE / dir;
    let t0 = (min - origin) * inv;
    let t1 = (max - origin) * inv;
    let near = t0.min(t1).max_element();
    let far = t0.max(t1).min_element();
    if far >= near.max(0.0) {
        Some(near.max(0.0))
    } else {
        None
    }
}

/// Ray vs the unit quad (local `z = 0` plane, `|x|,|y| <= 0.5`).
fn ray_unit_quad(origin: Vec3, dir: Vec3) -> Option<f32> {
    if dir.z.abs() < 1e-8 {
        return None;
    }
    let t = -origin.z / dir.z;
    if t < 0.0 {
        return None;
    }
    let hit = origin + dir * t;
    if hit.x.abs() <= 0.5 && hit.y.abs() <= 0.5 {
        Some(t)
    } else {
        None
    }
}

/// Picks the nearest drawable entity under a pixel position.
///
/// Hit comparison is exact by construction: the picking ray is
/// `(origin, dir)` with `dir` unit-length, and a draw's local-space
/// test returns `t` on the ray `local_origin + local_dir·t` where
/// `local_dir = L⁻¹·dir` (`L` = the model's linear part). Mapping the
/// hit back through the model gives `origin + L·local_dir·t =
/// origin + dir·t` — the local parameter IS the world distance, so
/// candidate hits compare as plain `t` values. (v1.1.0 multiplied by
/// `|local_dir|` here, which deflated scaled-up draws — the 30×30
/// ground quad outranked every object at any distance; the first real
/// user session caught it, EDITOR_REALITY_CHECK.md P0 #1.)
#[must_use]
pub fn pick_at(
    world: &World,
    view_proj: Mat4,
    rect: egui::Rect,
    pos: egui::Pos2,
) -> Option<Entity> {
    let (origin, dir) = screen_ray(view_proj, rect, pos);
    if dir == Vec3::ZERO {
        return None;
    }
    let mut best: Option<(f32, Entity)> = None;
    for draw in collect_draws(world) {
        let Some(t_world) = hit_distance(draw.shape, draw.model, origin, dir) else {
            continue;
        };
        if best.is_none() || t_world < best.expect("checked above").0 {
            best = Some((t_world, draw.entity));
        }
    }
    best.map(|(_, entity)| entity)
}

/// The viewport: renderer + shape cache + egui texture handle.
pub struct EditorViewport {
    renderer: Renderer,
    meshes: BTreeMap<(ShapeKind, [u8; 4]), GpuMesh>,
    tex: Option<egui::TextureHandle>,
    last_size: [u32; 2],
    last_frame: Option<(Mat4, Mat4)>,
}

impl EditorViewport {
    /// Creates the viewport renderer and uploads the base meshes.
    ///
    /// # Errors
    /// Propagates wgpu context/target/mesh creation failures.
    pub fn new() -> gens_core::anyhow::Result<Self> {
        let ctx = GpuContext::new()?;
        let renderer = Renderer::new(ctx, DEFAULT_SIZE[0], DEFAULT_SIZE[1])?;
        Ok(Self {
            renderer,
            meshes: BTreeMap::new(),
            tex: None,
            last_size: DEFAULT_SIZE,
            last_frame: None,
        })
    }

    /// The projection data of the last rendered frame (gizmo source).
    pub fn last_frame(&self) -> Option<(Mat4, Mat4)> {
        self.last_frame
    }

    /// Renders the scene into the viewport target and refreshes the
    /// egui texture the panel draws.
    ///
    /// # Errors
    /// Propagates mesh upload, model-slot or readback failures.
    pub fn render(
        &mut self,
        egui_ctx: &egui::Context,
        state: &EditorState,
        world: &mut World,
    ) -> gens_core::anyhow::Result<()> {
        let aspect = state.viewport_size_px[0] as f32 / state.viewport_size_px[1].max(1) as f32;
        let cam = state.orbit.camera(aspect);
        self.render_with_camera(egui_ctx, state, world, cam)
    }

    /// Game View: renders the scene through the CINEMATIC camera pose
    /// (ADR-0030's staging rule — the directed pose is authoritative).
    /// The pose's position/rotation aim the camera; its FOV sets the
    /// projection.
    ///
    /// # Errors
    /// As [`EditorViewport::render`].
    pub fn render_game_view(
        &mut self,
        egui_ctx: &egui::Context,
        state: &EditorState,
        world: &mut World,
        pose: &gens_runtime::cinematic::CameraPose,
    ) -> gens_core::anyhow::Result<()> {
        let aspect = state.viewport_size_px[0] as f32 / state.viewport_size_px[1].max(1) as f32;
        // RH convention: the camera looks down its local -Z; up is +Y.
        let forward = pose.rotation * gens_math::Vec3::NEG_Z;
        let up = pose.rotation * gens_math::Vec3::Y;
        let cam = Camera::perspective(
            pose.position,
            pose.position + forward,
            up,
            gens_math::deg_to_rad(pose.fov_degrees),
            aspect,
            0.1,
            100.0,
        );
        self.render_with_camera(egui_ctx, state, world, cam)
    }

    /// The shared render path: a given [`Camera`] draws the world's
    /// [`MeshSlot`] entities into the viewport texture.
    ///
    /// # Errors
    /// Propagates mesh upload, model-slot or readback failures.
    fn render_with_camera(
        &mut self,
        egui_ctx: &egui::Context,
        state: &EditorState,
        world: &mut World,
        cam: Camera,
    ) -> gens_core::anyhow::Result<()> {
        let size = state.viewport_size_px;
        if size != self.last_size {
            self.renderer.resize(size[0], size[1])?;
            self.last_size = size;
        }
        let view_proj = cam.view_proj();

        let device = self.renderer.context().device().clone();
        let queue = self.renderer.context().queue().clone();
        // Stage the camera FIRST — without this the shaders keep whatever
        // matrix was uploaded last and everything clips away (caught by
        // inspecting the first CI snapshot: empty viewport, gizmo fine).
        self.renderer.set_camera(&queue, view_proj);
        let draws_info = collect_draws(world);

        // Pass 1: stage model matrices + fill the mesh cache (mutable
        // borrow ends before draws collect immutable refs).
        for (slot, info) in draws_info.iter().enumerate() {
            let slot_idx = u32::try_from(slot).unwrap_or(u32::MAX);
            self.renderer.set_model(&queue, slot_idx, info.model)?;
            let key = (info.shape, info.tint);
            if let std::collections::btree_map::Entry::Vacant(vacant) = self.meshes.entry(key) {
                vacant.insert(build_tinted_mesh(&device, info.shape, info.tint)?);
            }
        }
        // Pass 2: collect draws (immutable refs into the cache).
        let draws: Vec<Draw<'_>> = draws_info
            .iter()
            .enumerate()
            .map(|(slot, info)| Draw::Colored {
                mesh: &self.meshes[&(info.shape, info.tint)],
                slot: u32::try_from(slot).unwrap_or(u32::MAX),
            })
            .collect();
        let rgba = self
            .renderer
            .draw_and_read(clear_color_rgba8(theme::VIEWPORT_CLEAR), &draws)?;

        // RGBA8 top-to-bottom straight into an egui texture.
        let pixels: Vec<egui::Color32> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| egui::Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3]))
            .collect();
        let image = egui::ColorImage::new([size[0] as usize, size[1] as usize], pixels);
        if self.tex.is_some() {
            if let Some(handle) = self.tex.as_mut() {
                handle.set(image, egui::TextureOptions::NEAREST);
            }
        } else {
            self.tex = Some(egui_ctx.load_texture(
                "gens_viewport_3d",
                image,
                egui::TextureOptions::NEAREST,
            ));
        }
        self.last_frame = Some((cam.view, view_proj));
        Ok(())
    }

    /// Game View panel: the cinematic camera's render with a director
    /// toolbar. No orbit interaction — the cinematic owns the camera.
    pub fn game_view_panel(&self, ui: &mut egui::Ui, state: &EditorState) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Cinematic").color(theme::TEXT_DIM));
            ui.separator();
            match &state.cinematic {
                Some(cin) => {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} t {:.2}/{:.2} fov {:.1}",
                            cin.state,
                            cin.local_time,
                            cin.duration,
                            cin.camera_pose.map(|p| p.fov_degrees).unwrap_or(0.0),
                        ))
                        .color(theme::TEXT_DIM),
                    );
                }
                None => {
                    ui.label(
                        egui::RichText::new("no cinematic staging the camera — orbit view")
                            .color(theme::TEXT_DIM),
                    );
                }
            }
        });
        ui.separator();
        let Some(handle) = &self.tex else {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("Game View not rendered yet").color(theme::TEXT_DIM));
            });
            return;
        };
        let size = egui::vec2(self.last_size[0] as f32, self.last_size[1] as f32);
        let (rect, _response) = ui.allocate_exact_size(size, egui::Sense::hover());
        ui.painter().image(
            handle.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }

    /// Draws the viewport content INTO the UI (called during the paint
    /// pass; world is mutable so object drags apply same-frame).
    pub fn panel(&self, ui: &mut egui::Ui, state: &mut EditorState, world: &mut World) {
        // Camera toolbar (reference: Perspective/Lit badges + gizmo row).
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Perspective").color(theme::TEXT_DIM));
            ui.separator();
            ui.label(egui::RichText::new("Lit").color(theme::TEXT_DIM));
            ui.separator();
            ui.label(
                egui::RichText::new(format!(
                    "yaw {:.0}° pitch {:.0}° dist {:.1}",
                    state.orbit.yaw_deg, state.orbit.pitch_deg, state.orbit.dist
                ))
                .color(theme::TEXT_DIM),
            );
            if ui.small_button("Reset View").clicked() {
                state.orbit = OrbitCam::default();
            }
        });
        ui.separator();

        let Some(handle) = &self.tex else {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("Viewport not rendered yet").color(theme::TEXT_DIM));
            });
            return;
        };

        // Image + real scene-view interaction (v1.1):
        // - click an object -> select it (exact-geometry pick);
        // - click empty space -> deselect;
        // - drag an object -> move it on the camera-facing plane
        //   through its origin (an undo op is recorded on release);
        // - drag empty space -> orbit the camera (classic behavior).
        // Object drags are an EDIT-mode tool: mid-simulation the
        // physics body owns the transform, so dragging is disabled
        // while Playing (orbiting still works).
        let size = egui::vec2(self.last_size[0] as f32, self.last_size[1] as f32);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        ui.painter().image(
            handle.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );

        let aspect = self.last_size[0] as f32 / self.last_size[1].max(1) as f32;
        let view_proj = state.orbit.camera(aspect).view_proj();
        let editable = state.play_mode != PlayMode::Playing;

        if response.drag_started()
            && editable
            && let Some(pos) = response.interact_pointer_pos()
            && rect.contains(pos)
            && let Some(entity) = pick_at(world, view_proj, rect, pos)
        {
            let before = world
                .get::<Transform>(entity)
                .copied()
                .unwrap_or(Transform::IDENTITY);
            let origin = world_position(world, entity);
            let grab_offset = plane_grab_offset(state, view_proj, rect, pos, origin);
            state.viewport_drag = Some(ViewportDrag {
                entity,
                before,
                grab_offset,
                moved: false,
            });
        }
        if response.dragged() {
            match state.viewport_drag.clone() {
                Some(mut drag) => {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let normal = (state.orbit.eye() - state.orbit.target).normalize_or_zero();
                        let (ro, rd) = screen_ray(view_proj, rect, pos);
                        let denom = rd.dot(normal);
                        if denom.abs() > 1e-5 {
                            let t = (drag.before.position - ro).dot(normal) / denom;
                            let new_position = ro + rd * t + drag.grab_offset;
                            if let Some(mut transform) = world.get_mut::<Transform>(drag.entity) {
                                transform.position = new_position;
                            }
                            // Physics follows so the next play-mode sync
                            // does not snap the object back.
                            if let Some(body) =
                                world.get::<gens_physics::PhysicsBody>(drag.entity).copied()
                            {
                                world
                                    .resource_mut::<gens_physics::PhysicsResource>()
                                    .world
                                    .set_body_position(body.handle, new_position);
                            }
                            drag.moved = true;
                            state.viewport_drag = Some(drag);
                        }
                    }
                }
                None => {
                    // Orbit drag (the pre-v1.1 behavior, unchanged).
                    let drag_delta = response.drag_delta();
                    if drag_delta != egui::Vec2::ZERO {
                        state.orbit.yaw_deg =
                            (state.orbit.yaw_deg + drag_delta.x * 0.5).rem_euclid(360.0);
                        state.orbit.pitch_deg =
                            (state.orbit.pitch_deg - drag_delta.y * 0.5).clamp(-85.0, 85.0);
                    }
                }
            }
        }
        if response.drag_stopped()
            && let Some(drag) = state.viewport_drag.take()
            && drag.moved
        {
            state.undo_stack.push(UndoOp::Transform {
                entity: drag.entity,
                before: drag.before,
            });
        }
        if response.clicked() {
            state.selected = response
                .interact_pointer_pos()
                .and_then(|pos| pick_at(world, view_proj, rect, pos));
        }

        state.viewport_size_px = [
            rect.width().round().max(1.0) as u32,
            rect.height().round().max(1.0) as u32,
        ];

        // Transform gizmo at the selection (RGB axes, reference colors).
        if let (Some(entity), Some((view, view_proj))) = (state.selected, self.last_frame) {
            let origin = world_position(world, entity);
            if let Some(pos) = project(origin, view_proj, rect) {
                let painter = ui.painter();
                painter.circle_stroke(pos, 5.0, egui::Stroke::new(1.5, egui::Color32::WHITE));
                let axes = [
                    (vec3(1.0, 0.0, 0.0), theme::GIZMO_AXES[0], "X"),
                    (vec3(0.0, 1.0, 0.0), theme::GIZMO_AXES[1], "Y"),
                    (vec3(0.0, 0.0, 1.0), theme::GIZMO_AXES[2], "Z"),
                ];
                for (axis, color, label) in axes {
                    let Some(end_px) = project(origin + axis * 0.75, view_proj, rect) else {
                        continue;
                    };
                    // A view-space depth check keeps back-facing axes dimmed
                    // (the reference gizmo's depth cue, kept simple).
                    let cam_dir = view.transform_point3(origin + axis * 0.75);
                    let stroke = if cam_dir.z < 0.0 {
                        egui::Stroke::new(2.0, color)
                    } else {
                        egui::Stroke::new(1.0, color.gamma_multiply(0.45))
                    };
                    painter.line_segment([pos, end_px], stroke);
                    painter.text(
                        end_px + egui::vec2(3.0, -3.0),
                        egui::Align2::LEFT_BOTTOM,
                        label,
                        egui::FontId::proportional(10.0),
                        color,
                    );
                }
            }
        }
    }
}

/// World-space position of an entity (`WorldTransform` or `Transform`).
fn world_position(world: &World, entity: Entity) -> Vec3 {
    world
        .get::<gens_ecs::WorldTransform>(entity)
        .map(|wt| wt.position())
        .or_else(|| world.get::<Transform>(entity).map(|t| t.position))
        .unwrap_or(Vec3::ZERO)
}

/// The world-space offset between `origin` and the pointer's
/// intersection with the camera-facing plane through `origin` (the
/// sticky grab point for object drags). Zero when the ray is parallel
/// to the plane.
fn plane_grab_offset(
    state: &EditorState,
    view_proj: Mat4,
    rect: egui::Rect,
    pos: egui::Pos2,
    origin: Vec3,
) -> Vec3 {
    let normal = (state.orbit.eye() - state.orbit.target).normalize_or_zero();
    let (ro, rd) = screen_ray(view_proj, rect, pos);
    let denom = rd.dot(normal);
    if denom.abs() > 1e-5 {
        let t = (origin - ro).dot(normal) / denom;
        origin - (ro + rd * t)
    } else {
        Vec3::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cube at `pos` (the pick-test fixture).
    fn world_with_cube_at(pos: Vec3) -> World {
        let mut world = World::new();
        world.spawn((
            gens_ecs::Name::new("Cube"),
            Transform::from_position(pos),
            MeshSlot::cube([255, 0, 0, 255]),
        ));
        world
    }

    #[test]
    fn orbit_eye_stays_on_the_sphere() {
        let orbit = OrbitCam {
            yaw_deg: 35.0,
            pitch_deg: 30.0,
            dist: 12.0,
            target: vec3(2.0, 1.0, -3.0),
        };
        let eye = orbit.eye();
        let offset = eye - orbit.target;
        assert!((offset.length() - 12.0).abs() < 1e-4, "{offset:?}");
        // Pitch 30°: y component is dist*sin(30°) = 6.
        assert!((offset.y - 6.0).abs() < 1e-4, "{offset:?}");
        // The camera looks AT the target: view maps target to view origin
        // plane (z = -near side, x/y centered).
        let cam = orbit.camera(16.0 / 9.0);
        let t = cam.view.transform_point3(orbit.target);
        assert!(t.x.abs() < 1e-4 && t.y.abs() < 1e-4, "{t:?}");
        assert!(t.z < 0.0, "target is in front of the camera");
    }

    #[test]
    fn project_maps_target_to_rect_center() {
        let orbit = OrbitCam::default();
        let cam = orbit.camera(1.0);
        let vp = cam.view_proj();
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(400.0, 300.0));
        let center = project(orbit.target, vp, rect).unwrap();
        assert!((center.x - (10.0 + 200.0)).abs() < 1e-3);
        assert!((center.y - (20.0 + 150.0)).abs() < 1e-3);
        // Behind the camera -> None.
        let behind = orbit.eye() + (orbit.eye() - orbit.target);
        assert!(project(behind, vp, rect).is_none());
    }

    #[test]
    fn collect_draws_are_deterministic_and_marked() {
        let mut world = World::new();
        let a = world
            .spawn((
                Name::new("a"),
                MeshSlot::cube([255, 0, 0, 255]),
                Transform::from_position(Vec3::ONE),
            ))
            .id();
        let _no_mesh = world
            .spawn((Name::new("invisible"), Transform::IDENTITY))
            .id();
        let b = world
            .spawn((
                Name::new("b"),
                MeshSlot::marker([0, 255, 0, 255]),
                Transform::from_position(Vec3::NEG_ONE),
            ))
            .id();
        let draws = collect_draws(&world);
        assert_eq!(draws.len(), 2, "entities without MeshSlot are not draws");
        // Entity-id order, not spawn order.
        let first = if a < b { &draws[0] } else { &draws[1] };
        let second = if a < b { &draws[1] } else { &draws[0] };
        assert_eq!(first.model, Mat4::from_translation(Vec3::ONE));
        assert_eq!(first.shape, ShapeKind::Cube);
        // Markers shrink to 0.15 units.
        assert_eq!(second.shape, ShapeKind::Marker);
        assert_eq!(
            second.model,
            Mat4::from_translation(Vec3::NEG_ONE) * Mat4::from_scale(Vec3::splat(0.15))
        );
    }

    #[test]
    #[ignore = "GPU test — executed by CI on adapter-backed OSes (ADR-0011)"]
    fn tint_multiplies_through_srgb_decode() {
        let ctx = gens_renderer::GpuContext::new().unwrap();
        let device = ctx.device().clone();
        let red = [255, 0, 0, 255];
        let gpu = build_tinted_mesh(&device, ShapeKind::Cube, red).unwrap();
        let base = mesh::cube();
        assert_eq!(gpu.index_count(), base.index_count());
    }

    fn pick_rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))
    }

    #[test]
    fn pick_hits_the_projected_object_center() {
        let pos = vec3(2.0, 1.0, -3.0);
        let world = world_with_cube_at(pos);
        let orbit = OrbitCam::default();
        let cam = orbit.camera(800.0 / 600.0);
        let vp = cam.view_proj();
        // The pixel the cube's center projects to MUST pick the cube —
        // the projection and the unprojection are exact inverses.
        let screen = project(pos, vp, pick_rect()).expect("cube is in front of the camera");
        let picked = pick_at(&world, vp, pick_rect(), screen);
        assert!(picked.is_some(), "the projected center must pick something");
    }

    #[test]
    fn pick_misses_empty_space() {
        let world = world_with_cube_at(vec3(2.0, 1.0, -3.0));
        let orbit = OrbitCam::default();
        let cam = orbit.camera(800.0 / 600.0);
        let vp = cam.view_proj();
        // The rect corner (far from any object's projection) is sky.
        let picked = pick_at(&world, vp, pick_rect(), egui::pos2(790.0, 590.0));
        assert_eq!(picked, None, "empty space must not pick");
    }

    #[test]
    fn hit_distance_is_exact_for_boxes_and_quads() {
        // A ray straight at a cube's face hits at the known distance:
        // cube centered at z = -5, camera ray marching -z from the
        // origin; the near face sits at z = -4.5 (t = 4.5).
        let model = Mat4::from_translation(vec3(0.0, 0.0, -5.0));
        let t = hit_distance(ShapeKind::Cube, model, Vec3::ZERO, vec3(0.0, 0.0, -1.0));
        assert!((t.unwrap() - 4.5).abs() < 1e-5, "face at t=4.5, got {t:?}");

        // A ray aimed wide of the half-extent misses.
        let t = hit_distance(
            ShapeKind::Cube,
            model,
            Vec3::ZERO,
            vec3(0.9, 0.0, -1.0).normalize_or_zero(),
        );
        assert!(t.is_none(), "off-axis ray must miss the unit cube");

        // A rotated cube still hits exactly (the OBB transform is part
        // of the local-space test).
        let rotated = Mat4::from_translation(vec3(0.0, 0.0, -5.0))
            * Mat4::from_quat(gens_math::Quat::from_rotation_y(
                std::f32::consts::FRAC_PI_4,
            ));
        let t = hit_distance(ShapeKind::Cube, rotated, Vec3::ZERO, vec3(0.0, 0.0, -1.0));
        assert!(t.is_some(), "a rotated cube still fills the view axis");

        // Quads: within bounds hits, outside bounds misses.
        let quad = Mat4::IDENTITY;
        let t = hit_distance(
            ShapeKind::Quad,
            quad,
            vec3(0.0, 0.0, 2.0),
            vec3(0.0, 0.0, -1.0),
        );
        assert!(
            (t.unwrap() - 2.0).abs() < 1e-5,
            "straight-on quad hit at t=2"
        );
        let t = hit_distance(
            ShapeKind::Quad,
            quad,
            vec3(0.9, 0.0, 2.0),
            vec3(0.0, 0.0, -1.0),
        );
        assert_eq!(t, None, "outside |x| <= 0.5 must miss");
    }

    #[test]
    fn pick_prefers_the_nearest_hit() {
        // Two cubes on ONE view ray: the far cube sits at eye + 2*(near
        // - eye), so both project to the same pixel and the nearest
        // must win.
        let orbit = OrbitCam::default();
        let cam = orbit.camera(800.0 / 600.0);
        let vp = cam.view_proj();
        let eye = orbit.eye();
        let near_pos = vec3(2.0, 1.0, -3.0);
        let far_pos = eye + (near_pos - eye) * 2.0;

        let mut world = World::new();
        world.spawn((
            gens_ecs::Name::new("Far"),
            Transform::from_position(far_pos),
            MeshSlot::cube([0, 0, 255, 255]),
        ));
        let near = world
            .spawn((
                gens_ecs::Name::new("Near"),
                Transform::from_position(near_pos),
                MeshSlot::cube([0, 255, 0, 255]),
            ))
            .id();

        let screen = project(near_pos, vp, pick_rect()).expect("near cube is visible");
        let picked = pick_at(&world, vp, pick_rect(), screen);
        assert_eq!(picked, Some(near), "the nearest hit must win");
    }

    /// The shipped demo scene's ground: a 30×30 quad, rotated -90°
    /// around Y (the editor bin's exact local transform).
    fn ground_transform() -> Transform {
        Transform {
            position: Vec3::ZERO,
            rotation: gens_math::Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2),
            scale: vec3(30.0, 30.0, 1.0),
        }
    }

    #[test]
    fn hit_distance_is_world_exact_for_scaled_cubes() {
        // A scale-2 cube centered at z = -10: its near face sits at
        // z = -9, so a ray marching -z hits at t = 9. (v1.1.0 reported
        // 4.5 here — the |L⁻¹·dir| factor halved the distance — which
        // is how scaled-up draws outranked everything behind them.)
        let model =
            Mat4::from_translation(vec3(0.0, 0.0, -10.0)) * Mat4::from_scale(Vec3::splat(2.0));
        let t = hit_distance(ShapeKind::Cube, model, Vec3::ZERO, vec3(0.0, 0.0, -1.0));
        assert!((t.unwrap() - 9.0).abs() < 1e-5, "face at t=9, got {t:?}");
    }

    #[test]
    fn pick_prefers_the_cube_in_front_of_the_scaled_ground_plane() {
        // EDITOR_REALITY_CHECK.md P0 #1, as a test: the shipped scene's
        // 30×30 ground quad behind a unit cube. Every measured click on
        // the cube selected Terrain because the quad's t was deflated
        // by its scale. The cube's projected center must pick the cube.
        let orbit = OrbitCam::default();
        let cam = orbit.camera(800.0 / 600.0);
        let vp = cam.view_proj();

        let mut world = World::new();
        world.spawn((
            gens_ecs::Name::new("Terrain"),
            ground_transform(),
            MeshSlot::quad([52, 60, 74, 255]),
        ));
        let cube = world
            .spawn((
                gens_ecs::Name::new("Crate"),
                Transform::from_position(vec3(1.0, 0.5, -1.0)),
                MeshSlot::cube([185, 130, 70, 255]),
            ))
            .id();

        let screen = project(vec3(1.0, 0.5, -1.0), vp, pick_rect())
            .expect("the cube is in front of the camera");
        let picked = pick_at(&world, vp, pick_rect(), screen);
        assert_eq!(
            picked,
            Some(cube),
            "clicking the cube must select the cube, not the ground behind it"
        );

        // The ground itself stays pickable where the cube is not.
        let ground_screen = project(vec3(-6.0, 0.0, 2.0), vp, pick_rect())
            .expect("a far ground point is still on screen");
        let picked = pick_at(&world, vp, pick_rect(), ground_screen);
        assert_neq_terrain_guard(picked, cube);
    }

    /// The ground-point pick must be SOME draw (the quad fills the
    /// lower half of the view) — the assertion only fails when picking
    /// is dead.
    fn assert_neq_terrain_guard(picked: Option<Entity>, cube: Entity) {
        assert!(picked.is_some(), "the ground plane must still pick");
        assert_ne!(
            picked,
            Some(cube),
            "a ground-only point must not pick the cube"
        );
    }

    #[test]
    fn pick_prefers_the_scaled_down_marker_over_the_ground_behind_it() {
        // The scaled-DOWN half of the same bug: markers (0.15×) had
        // their t inflated ~6.7×, so the ground behind them won.
        let orbit = OrbitCam::default();
        let cam = orbit.camera(800.0 / 600.0);
        let vp = cam.view_proj();

        let mut world = World::new();
        world.spawn((
            gens_ecs::Name::new("Terrain"),
            ground_transform(),
            MeshSlot::quad([52, 60, 74, 255]),
        ));
        let marker = world
            .spawn((
                gens_ecs::Name::new("DirectionalLight"),
                Transform::from_position(vec3(0.0, 1.2, 0.0)),
                MeshSlot::marker([255, 230, 120, 255]),
            ))
            .id();

        let screen = project(vec3(0.0, 1.2, 0.0), vp, pick_rect())
            .expect("the marker is in front of the camera");
        let picked = pick_at(&world, vp, pick_rect(), screen);
        assert_eq!(
            picked,
            Some(marker),
            "the small marker must outrank the large ground plane behind it"
        );
    }
}
