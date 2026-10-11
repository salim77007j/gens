//! The [`Renderer`] facade: camera + model uniforms, draws, offscreen
//! readback (Phase 2).
//!
//! One object owns the whole Phase 2 render state: GPU context, offscreen
//! target, the colored pipeline, one camera uniform and the model-matrix
//! ring. A frame is:
//!
//! 1. `set_camera` (once) and `set_model` per object — both stage uniform
//!    writes that land at the next submission;
//! 2. `draw_and_read` — records one render pass, copies to staging,
//!    submits, waits, and returns packed RGBA8 pixels.
//!
//! The engine's game loop uses exactly this sequence in the `gpu_hello`
//! example; nothing else in the engine touches wgpu types (ADR-0001).

use gens_core::error::Result;
use gens_math::Mat4;
use wgpu::Queue;

use crate::camera;
use crate::context::GpuContext;
use crate::pipeline::{ColoredPipeline, DYNAMIC_OFFSET_ALIGNMENT, model_slot_offset};
use crate::resources::{GpuMesh, ModelUniforms};
use crate::target::RenderTarget;

/// One colored draw: an uploaded mesh positioned by model slot.
#[derive(Clone, Copy, Debug)]
pub struct ColoredDraw<'a> {
    /// Uploaded mesh (vertex + index buffers).
    pub mesh: &'a GpuMesh,
    /// Model-uniform slot holding this draw's model matrix.
    pub slot: u32,
}

/// Owns the full offscreen render state.
pub struct Renderer {
    ctx: GpuContext,
    target: RenderTarget,
    colored: ColoredPipeline,
    camera_buf: wgpu::Buffer,
    camera_bg: wgpu::BindGroup,
    models: ModelUniforms,
}

impl Renderer {
    /// Creates a renderer with an offscreen target of `width` x `height`.
    ///
    /// # Errors
    /// Returns [`EngineError`] when the target size is invalid or pipeline
    /// creation fails.
    pub fn new(ctx: GpuContext, width: u32, height: u32) -> Result<Renderer> {
        let device = ctx.device();
        let target = RenderTarget::new(device, width, height)?;
        let colored = ColoredPipeline::new(device, wgpu::TextureFormat::Rgba8UnormSrgb)?;

        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gens-camera-uniform"),
            size: crate::pipeline::MAT4_UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gens-camera-bind-group"),
            layout: colored.camera_bgl(),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &camera_buf,
                    offset: 0,
                    size: None,
                }),
            }],
        });

        let models =
            ModelUniforms::new(device, ModelUniforms::DEFAULT_CAPACITY, colored.model_bgl());

        Ok(Renderer {
            ctx,
            target,
            colored,
            camera_buf,
            camera_bg,
            models,
        })
    }

    /// The GPU context (device/queue/adapter info).
    #[must_use]
    pub fn context(&self) -> &GpuContext {
        &self.ctx
    }

    /// The offscreen target.
    #[must_use]
    pub fn target(&self) -> &RenderTarget {
        &self.target
    }

    /// Recreates the offscreen target at a new size (uniform state is
    /// preserved; the target's pixels are not).
    ///
    /// # Errors
    /// See [`RenderTarget::new`].
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        self.target = RenderTarget::new(self.ctx.device(), width, height)?;
        Ok(())
    }

    /// Stages the camera matrix for the next submission.
    pub fn set_camera(&self, queue: &Queue, view_proj: Mat4) {
        queue.write_buffer(&self.camera_buf, 0, &camera::uniform::bytes(view_proj));
    }

    /// Stages a model matrix into `slot` for the next submission.
    ///
    /// # Errors
    /// Returns [`EngineError`] when `slot` is out of capacity.
    pub fn set_model(&self, queue: &Queue, slot: u32, model: Mat4) -> Result<()> {
        self.models.write(queue, slot, model)
    }

    /// Renders `draws` into the offscreen target and returns packed RGBA8
    /// pixels (`width * height * 4` bytes, top-to-bottom).
    ///
    /// The clear color is given in the framebuffer's `Rgba8UnormSrgb`
    /// space; only 0.0/1.0 endpoints are byte-exact across backends.
    ///
    /// # Errors
    /// Returns [`EngineError`] on slot overflow or readback failure.
    pub fn draw_and_read(&self, clear: wgpu::Color, draws: &[ColoredDraw<'_>]) -> Result<Vec<u8>> {
        let device = self.ctx.device();
        let queue = self.ctx.queue();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gens-frame-encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gens-frame-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.target.color_view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.target.depth_view(),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(self.colored.raw());
            pass.set_bind_group(0, &self.camera_bg, &[]);
            for draw in draws {
                let offset = model_slot_offset(draw.slot, self.models.capacity())?;
                pass.set_bind_group(1, self.models.bind_group(), &[offset]);
                draw.mesh.bind_to(&mut pass);
                draw.mesh.draw(&mut pass);
            }
        }
        self.target.copy_color_to_staging(&mut encoder);
        queue.submit([encoder.finish()]);
        self.target.read_pixels(device)
    }

    /// Convenience: builds an orthographic camera for the target size and
    /// stages it. Visible view-space z range is `[-1, 1]`.
    pub fn set_camera_2d(&self, queue: &Queue) {
        let cam = crate::camera::Camera::orthographic_2d(
            self.target.width() as f32,
            self.target.height() as f32,
        );
        self.set_camera(queue, cam.view_proj());
    }
}

/// Dynamic-offset slot stride, re-exported for call sites doing their own
/// slot math (tests, editor panels).
pub const MODEL_SLOT_STRIDE: u32 = DYNAMIC_OFFSET_ALIGNMENT;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{self, Mesh};

    /// sRGB channel encoding for a linear value in [0, 1] (the inverse of
    /// the Rgba8UnormSrgb hardware conversion, used to set tolerance bands).
    fn srgb_encode(linear: f32) -> f32 {
        if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        }
    }

    fn renderer() -> Renderer {
        let ctx = crate::context::GpuContext::new().expect("GPU adapter (ADR-0011)");
        Renderer::new(ctx, 64, 64).expect("renderer")
    }

    /// Full pipeline check: a colored triangle renders with interpolated
    /// vertex colors at the centroid and leaves the clear color outside.
    /// Structural expectations only (ADR-0011): channel relations and a
    /// tolerance band, not byte-exact equality across drivers.
    #[test]
    #[ignore = "requires a GPU adapter (real or software)"]
    fn colored_triangle_renders_with_interpolation() {
        let r = renderer();
        let device = r.context().device();
        let queue = r.context().queue();
        r.set_camera_2d(queue);
        r.set_model(queue, 0, Mat4::IDENTITY).unwrap();
        let tri = GpuMesh::upload(device, &mesh::triangle()).unwrap();

        let px = r
            .draw_and_read(
                wgpu::Color::WHITE,
                &[ColoredDraw {
                    mesh: &tri,
                    slot: 0,
                }],
            )
            .unwrap();
        assert_eq!(px.len(), (64 * 64 * 4) as usize);

        let at = |x: u32, y: u32| {
            let i = ((y * 64 + x) * 4) as usize;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };

        // Corner: pure white clear (endpoint colors are byte-exact).
        assert_eq!(at(2, 2), [255, 255, 255, 255]);

        // Centroid: linear (1/3, 1/3, 1/3) -> equal sRGB channels well
        // below white; every channel equal within rounding.
        let c = at(32, 32);
        let expect = srgb_encode(1.0 / 3.0) * 255.0;
        for &ch in &c[..3] {
            assert!(
                (f32::from(ch) - expect).abs() <= 3.0,
                "centroid {c:?} vs {expect}"
            );
        }
        assert_eq!(c[3], 255);
    }

    /// Model matrices + depth: two blue quads at different depths drawn in
    /// both orders must produce identical pixels (depth test wins over
    /// draw order for opaque geometry).
    #[test]
    #[ignore = "requires a GPU adapter (real or software)"]
    fn model_matrices_and_depth_are_draw_order_independent() {
        let r = renderer();
        let device = r.context().device();
        let queue = r.context().queue();
        r.set_camera_2d(queue);

        let mut blue_quad: Mesh = mesh::quad();
        for v in &mut blue_quad.vertices {
            v.color = [0.0, 0.0, 1.0, 1.0];
        }
        let quad = GpuMesh::upload(device, &blue_quad).unwrap();

        // Quad near the right half; nearer quad at z = 0, farther at z = -0.5.
        let near = gens_math::compose_trs(
            gens_math::vec3(0.4, 0.0, 0.0),
            gens_math::Quat::IDENTITY,
            gens_math::Vec3::ONE,
        );
        let far = gens_math::compose_trs(
            gens_math::vec3(0.4, 0.0, -0.5),
            gens_math::Quat::IDENTITY,
            gens_math::Vec3::ONE,
        );

        let set_models = |slot_near: u32, slot_far: u32| {
            r.set_model(queue, slot_near, near).unwrap();
            r.set_model(queue, slot_far, far).unwrap();
        };

        // Order A: near submitted first, far second.
        set_models(0, 1);
        let draws_a = [
            ColoredDraw {
                mesh: &quad,
                slot: 0,
            }, // near
            ColoredDraw {
                mesh: &quad,
                slot: 1,
            }, // far
        ];
        let px_near_first = r.draw_and_read(wgpu::Color::BLACK, &draws_a).unwrap();

        // Order B: far submitted first, near second — same slots, reversed
        // draw order. Depth, not submission order, must decide the pixels.
        set_models(0, 1);
        let draws_b = [
            ColoredDraw {
                mesh: &quad,
                slot: 1,
            }, // far
            ColoredDraw {
                mesh: &quad,
                slot: 0,
            }, // near
        ];
        let px_far_first = r.draw_and_read(wgpu::Color::BLACK, &draws_b).unwrap();

        assert_eq!(px_near_first, px_far_first);

        let at = |px: &[u8], x: u32, y: u32| {
            let i = ((y * 64 + x) * 4) as usize;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        // Inside the quad: pure blue (endpoint). Depth semantics: nearer
        // quad's color, same whichever order they were submitted.
        assert_eq!(at(&px_near_first, 46, 32), [0, 0, 255, 255]);
        // Left of the quad: background black.
        assert_eq!(at(&px_near_first, 4, 32), [0, 0, 0, 255]);
    }
}
