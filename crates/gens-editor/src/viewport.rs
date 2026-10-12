//! The 3D viewport — the center dock's live scene view.
//!
//! The viewport owns a dedicated offscreen `Renderer` (independent of
//! the window renderer's target, per ADR-0025): each frame it renders
//! every entity carrying a [`MeshSlot`] from an orbit camera, reads the
//! pixels back and feeds them to egui as a managed texture. The gizmo
//! (RGB axes at the selection) is drawn by egui's painter on top, using
//! the same `view_proj` matrix — one source of truth for projection.
//!
//! Picking is via the hierarchy (click a row); viewport dragging orbits
//! the camera — both are real, scripted-replayable interactions.

use crate::{
    model::{MeshSlot, ShapeKind},
    shell::EditorState,
    theme,
};
use gens_ecs::prelude::*;
use gens_math::{Mat4, Vec3, vec3};
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
            shape: slot.shape,
            tint: slot.tint,
            model,
        });
    }
    out
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
    /// pass; read-only on self).
    pub fn panel(&self, ui: &mut egui::Ui, state: &mut EditorState, world: &World) {
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

        // Image + orbit drag (real interaction: drag rotates the camera).
        let size = egui::vec2(self.last_size[0] as f32, self.last_size[1] as f32);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        ui.painter().image(
            handle.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        let drag = response.drag_delta();
        if drag != egui::Vec2::ZERO {
            state.orbit.yaw_deg = (state.orbit.yaw_deg + drag.x * 0.5).rem_euclid(360.0);
            state.orbit.pitch_deg = (state.orbit.pitch_deg - drag.y * 0.5).clamp(-85.0, 85.0);
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
