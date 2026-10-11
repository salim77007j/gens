//! The wgpu render backend: frames to pixels through gens-renderer.
//!
//! [`UiRenderer`] owns the pure [`UiContext`] plus a texture cache and
//! turns [`UiFrame`]s into GPU batches for
//! [`gens_renderer::Renderer::draw_ui_and_read`]:
//!
//! - texture uploads become [`gens_renderer::GpuTexture`]s (the CPU
//!   mirror in the core means the backend only ever uploads whole
//!   textures);
//! - mesh vertices convert from egui pixel space to NDC with the pure,
//!   unit-tested [`gens_renderer::UiVertex::ndc_from_pixel`];
//! - each mesh's clip rectangle becomes a scissor rect;
//! - a mesh referencing an unknown texture is **skipped and counted**,
//!   never silently drawn with the wrong image.

use std::collections::BTreeMap;

use gens_core::error::Result;
use gens_renderer::{GpuTexture, Renderer, UiBatch, UiVertex, clear_color_rgba8};

use crate::context::UiContext;
use crate::frame::{TextureRef, UiFrame, UiInput};

/// Result of one rendered UI pass.
#[derive(Debug, Clone)]
pub struct UiRender {
    /// Packed RGBA8 pixels of the offscreen target.
    pub pixels: Vec<u8>,
    /// Batches drawn.
    pub drawn_batches: usize,
    /// Batches skipped for a missing texture (counted honestly).
    pub skipped_batches: usize,
    /// The tessellated frame that produced the draw.
    pub frame: UiFrame,
}

/// egui UI rendered through the gens-renderer facade.
pub struct UiRenderer {
    core: UiContext,
    textures: BTreeMap<TextureRef, GpuTexture>,
}

impl Default for UiRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl UiRenderer {
    /// A UI renderer with an empty texture cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            core: UiContext::new(),
            textures: BTreeMap::new(),
        }
    }

    /// Mutable access to the pure core (styles, fonts, widget state).
    pub fn core_mut(&mut self) -> &mut UiContext {
        &mut self.core
    }

    /// Runs one UI pass and renders it to the renderer's offscreen target.
    ///
    /// `clear_rgba8` is given in encoded sRGB bytes (the same convention
    /// as [`clear_color_rgba8`]); opaque UI colors round-trip byte-exactly
    /// through the `Rgba8UnormSrgb` target (pinned by the GPU tests).
    ///
    /// # Errors
    /// Returns [`gens_core::error::EngineError`] on texture upload or
    /// readback failure.
    pub fn render(
        &mut self,
        renderer: &Renderer,
        clear_rgba8: [u8; 4],
        input: UiInput,
        build: impl Fn(&mut egui::Ui),
    ) -> Result<UiRender> {
        let frame = self.core.run(input, build);

        // Apply texture lifecycle: uploads replace, frees drop.
        let (device, queue) = (renderer.context().device(), renderer.context().queue());
        for upload in &frame.uploads {
            let texture =
                GpuTexture::from_rgba8(device, queue, upload.width, upload.height, &upload.rgba)?;
            self.textures.insert(upload.id, texture);
        }
        for free in &frame.frees {
            self.textures.remove(&free.id);
        }

        let (w, h) = (
            renderer.target().width() as f32,
            renderer.target().height() as f32,
        );

        /// One drawable mesh mid-conversion: indices + texture + clip
        /// borrowed from the frame, vertices freshly converted to NDC.
        type Drawable<'a> = (&'a [u32], &'a GpuTexture, [f32; 4], Vec<UiVertex>);

        // Phase 1: convert every drawable mesh to NDC vertices (the vec
        // must stop mutating before batches borrow from it).
        let mut drawable: Vec<Drawable<'_>> = Vec::with_capacity(frame.meshes.len());
        let mut skipped = 0usize;
        for mesh in &frame.meshes {
            let Some(texture) = self.textures.get(&mesh.texture) else {
                skipped += 1;
                continue;
            };
            let verts: Vec<UiVertex> = mesh
                .vertices
                .iter()
                .map(|v| UiVertex {
                    pos: UiVertex::ndc_from_pixel(v.pos[0], v.pos[1], w, h),
                    uv: v.uv,
                    color: v.color,
                })
                .collect();
            drawable.push((&mesh.indices, texture, mesh.clip_rect_px, verts));
        }
        // Phase 2: batch references are stable now.
        let batches: Vec<UiBatch<'_>> = drawable
            .iter()
            .map(|(indices, texture, clip, verts)| UiBatch {
                vertices: verts,
                indices,
                texture,
                scissor_px: Some(scissor_from_clip(*clip, w, h)),
            })
            .collect();

        let pixels = renderer.draw_ui_and_read(clear_color_rgba8(clear_rgba8), &batches)?;
        Ok(UiRender {
            pixels,
            drawn_batches: batches.len(),
            skipped_batches: skipped,
            frame,
        })
    }
}

/// Clip rectangle `[x, y, w, h]` (pixels) to a clamped scissor rect.
///
/// Rectangles outside the target clamp to zero extent (the pass then
/// skips the batch); partially visible rectangles clamp to the target.
#[must_use]
pub fn scissor_from_clip(clip: [f32; 4], width: f32, height: f32) -> [u32; 4] {
    let x0 = clip[0].clamp(0.0, width).floor();
    let y0 = clip[1].clamp(0.0, height).floor();
    let x1 = (clip[0] + clip[2]).min(width).ceil();
    let y1 = (clip[1] + clip[3]).min(height).ceil();
    let w = (x1 - x0).max(0.0);
    let mut h = (y1 - y0).max(0.0);
    // A degenerate extent in one dimension makes the whole rect empty.
    if w == 0.0 {
        h = 0.0;
    }
    [x0 as u32, y0 as u32, w as u32, h as u32]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scissor_clamps_to_target_and_floor_or_ceils() {
        // Fully inside: exact.
        assert_eq!(
            scissor_from_clip([10.0, 20.0, 30.0, 40.0], 100.0, 100.0),
            [10, 20, 30, 40]
        );
        // Negative origin clamps to 0.
        assert_eq!(
            scissor_from_clip([-5.0, -5.0, 20.0, 20.0], 100.0, 100.0),
            [0, 0, 15, 15]
        );
        // Extending past the target clamps to the edge.
        assert_eq!(
            scissor_from_clip([90.0, 90.0, 50.0, 50.0], 100.0, 100.0),
            [90, 90, 10, 10]
        );
        // Fully outside collapses to zero extent.
        assert_eq!(
            scissor_from_clip([200.0, 0.0, 10.0, 10.0], 100.0, 100.0),
            [100, 0, 0, 0]
        );
    }

    // ---- GPU integration tests (run via `cargo test -- --ignored`) ----
    // CI renders them for real on Linux (lavapipe) and macOS (Metal);
    // Windows runners use their software adapter when present (ADR-0011).

    use gens_renderer::GpuContext;

    const PANEL_FILL: [u8; 4] = [20, 24, 33, 255];

    fn pixel(pixels: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    }

    #[test]
    #[ignore = "needs a GPU adapter (CI: lavapipe/Metal)"]
    fn ui_paints_panel_text_and_button() {
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("GPU-SKIP: no adapter for ui paint test");
            return;
        };
        let renderer = Renderer::new(ctx, 160, 120).expect("renderer");
        let mut ui = UiRenderer::new();

        let frame = ui
            .render(
                &renderer,
                [12, 12, 20, 255],
                UiInput::new([160, 120], 0.0, 0.016),
                |ui| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::new().fill(egui::Color32::from_rgb(
                            PANEL_FILL[0],
                            PANEL_FILL[1],
                            PANEL_FILL[2],
                        )))
                        .show(ui, |ui| {
                            ui.label("gens");
                            let _ = ui.button("ok");
                        });
                },
            )
            .expect("render");

        assert!(frame.drawn_batches > 0, "panel + text + button must draw");
        assert_eq!(frame.skipped_batches, 0);

        // Opaque panel fill round-trips byte-exactly through the sRGB
        // target (same physics as the Phase 2 clear-color evidence).
        let bg = pixel(&frame.pixels, 160, 3, 3);
        assert_eq!(bg, PANEL_FILL, "panel background must be exact");

        // Text row (top of the central panel) must contain non-panel pixels.
        let mut text_pixels = 0u32;
        for x in 8..150 {
            if pixel(&frame.pixels, 160, x, 12) != PANEL_FILL {
                text_pixels += 1;
            }
        }
        assert!(text_pixels > 0, "label glyphs must paint over the fill");

        // Button area (just below the label) must also differ somewhere.
        let mut button_pixels = 0u32;
        for x in 8..150 {
            for y in 24..40 {
                if pixel(&frame.pixels, 160, x, y) != PANEL_FILL {
                    button_pixels += 1;
                }
            }
        }
        assert!(button_pixels > 0, "button must paint over the fill");
    }

    #[test]
    #[ignore = "needs a GPU adapter (CI: lavapipe/Metal)"]
    fn ui_hover_changes_pixels_under_pointer() {
        let Ok(ctx) = GpuContext::new() else {
            eprintln!("GPU-SKIP: no adapter for ui hover test");
            return;
        };
        let renderer = Renderer::new(ctx, 160, 120).expect("renderer");
        let mut ui = UiRenderer::new();
        use std::cell::Cell;
        use std::rc::Rc;

        let rect_cell = Rc::new(Cell::new(egui::Rect::NOTHING));
        let rc = rect_cell.clone();
        let _ = ui.render(
            &renderer,
            [12, 12, 20, 255],
            UiInput::new([160, 120], 0.0, 0.016),
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let response = ui.button("hover me");
                    rc.set(response.rect);
                });
            },
        );
        let rect = rect_cell.get();
        let cx = rect.center().x;
        let cy = rect.center().y;

        let paint_at = |ui: &mut UiRenderer, events: Vec<crate::frame::UiEvent>| {
            ui.render(
                &renderer,
                [12, 12, 20, 255],
                crate::frame::UiInput {
                    time_seconds: 1.0,
                    dt_seconds: 0.016,
                    size_px: [160, 120],
                    events,
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let _ = ui.button("hover me");
                    });
                },
            )
            .expect("render")
        };

        let idle = paint_at(&mut ui, vec![]);
        let hovered = paint_at(
            &mut ui,
            vec![crate::frame::UiEvent::PointerMove { x: cx, y: cy }],
        );

        // Widget ID ordering is stable, so frame 0's first button and
        // frame 2's first button occupy the same rect; the hover state
        // must change the pixels at its center.
        let before = pixel(&idle.pixels, 160, cx as u32, cy as u32);
        let after = pixel(&hovered.pixels, 160, cx as u32, cy as u32);
        assert_ne!(
            before, after,
            "hover state must repaint the button center: {before:?} vs {after:?}"
        );
    }
}
