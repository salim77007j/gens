//! UI render path: egui tessellated meshes through a dedicated pipeline.
//!
//! The UI pipeline is deliberately separate from the 3D pipelines:
//!
//! - vertices arrive **already in NDC** — [`UiVertex::ndc_from_pixel`]
//!   converts egui's top-left-origin pixel space (Y down) to WebGPU clip
//!   space on the CPU, so the pipeline needs no camera or model
//!   uniforms, only a texture bind group;
//! - blending is egui's premultiplied convention
//!   (one × src + one-minus-src-alpha), matching egui's vertex colors
//!   and font atlas, which are premultiplied at the source;
//! - no depth test or write: UI draws last over the 3D frame;
//! - each [`UiBatch`] carries its own scissor rectangle in physical
//!   pixels (the pass applies it before drawing).
//!
//! Vertex/index buffers are created per call — fine at HUD scene sizes,
//! documented honestly as in Phase 2's textured bind groups.

use gens_core::error::{EngineError, Result};
use wgpu::Device;

use crate::texture::GpuTexture;

/// One UI vertex: NDC position, normalized UV, premultiplied color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiVertex {
    /// Clip-space position (X right, Y up — WebGPU NDC).
    pub pos: [f32; 2],
    /// Normalized texture coordinates (0,0 top-left).
    pub uv: [f32; 2],
    /// Premultiplied RGBA, channels 0..=1.
    pub color: [f32; 4],
}

impl UiVertex {
    /// 32-byte stride: 2x f32 pos + 2x f32 uv + 4x f32 color.
    pub const STRIDE: usize = 32;

    /// Converts egui pixel-space coordinates to WebGPU NDC.
    ///
    /// egui's origin is the top-left with Y down; NDC's is the center
    /// with Y up. Pure function, pinned by unit tests:
    /// `ndc_from_pixel(0, 0, w, h) == (-1, 1)`,
    /// `ndc_from_pixel(w, h, w, h) == (1, -1)`.
    #[must_use]
    pub fn ndc_from_pixel(x: f32, y: f32, width: f32, height: f32) -> [f32; 2] {
        [(x / width) * 2.0 - 1.0, 1.0 - (y / height) * 2.0]
    }

    /// Packs the vertex into the 32-byte GPU layout.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; Self::STRIDE] {
        let mut out = [0u8; Self::STRIDE];
        for (i, f) in self
            .pos
            .iter()
            .chain(self.uv.iter())
            .chain(self.color.iter())
            .enumerate()
        {
            out[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
        }
        out
    }
}

/// One draw batch: vertices, indices, texture and scissor.
pub struct UiBatch<'a> {
    /// Vertices in NDC.
    pub vertices: &'a [UiVertex],
    /// Triangle indices (u32).
    pub indices: &'a [u32],
    /// Sampled texture (font atlas for text, user uploads otherwise).
    pub texture: &'a GpuTexture,
    /// Scissor rectangle in physical pixels `[x, y, w, h]`, or `None`
    /// for the full target.
    pub scissor_px: Option<[u32; 4]>,
}

/// The UI render pipeline (premultiplied blend, no depth, no uniforms).
pub struct UiPipeline {
    pipeline: wgpu::RenderPipeline,
    texture_bgl: wgpu::BindGroupLayout,
}

impl UiPipeline {
    /// Builds the pipeline for an `Rgba8UnormSrgb` target.
    ///
    /// # Errors
    /// Returns [`EngineError`] when the WGSL fails to compile.
    pub fn new(device: &Device, color_format: wgpu::TextureFormat) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gens-ui-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/ui.wgsl").into()),
        });

        let texture_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gens-ui-texture-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gens-ui-pipeline-layout"),
            bind_group_layouts: &[Some(&texture_bgl)],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: UiVertex::STRIDE as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 8,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 16,
                    shader_location: 2,
                },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gens-ui-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_ui"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_ui"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    // egui convention: premultiplied-alpha over operator.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Ok(Self {
            pipeline,
            texture_bgl,
        })
    }

    /// The raw pipeline for recording passes.
    pub fn raw(&self) -> &wgpu::RenderPipeline {
        &self.pipeline
    }

    /// Bind-group layout for texture+sampler (group 0).
    pub fn texture_bgl(&self) -> &wgpu::BindGroupLayout {
        &self.texture_bgl
    }

    /// Creates the per-batch texture bind group.
    #[must_use]
    pub fn create_texture_bind_group(
        &self,
        device: &Device,
        texture: &GpuTexture,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gens-ui-texture-bind-group"),
            layout: &self.texture_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(texture.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(texture.sampler()),
                },
            ],
        })
    }
}

/// Validates one batch's index/vertex contract.
pub(crate) fn validate_batch(vertex_count: usize, indices: &[u32]) -> Result<()> {
    if let Some(&max) = indices.iter().max()
        && max as usize >= vertex_count
    {
        return Err(EngineError::InvalidState {
            context: "ui draw",
            problem: format!("batch indices exceed vertex count: {max} >= {vertex_count}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndc_conversion_hits_all_four_corners() {
        let (w, h) = (200.0f32, 100.0f32);
        assert_eq!(UiVertex::ndc_from_pixel(0.0, 0.0, w, h), [-1.0, 1.0]);
        assert_eq!(UiVertex::ndc_from_pixel(w, 0.0, w, h), [1.0, 1.0]);
        assert_eq!(UiVertex::ndc_from_pixel(0.0, h, w, h), [-1.0, -1.0]);
        assert_eq!(UiVertex::ndc_from_pixel(w, h, w, h), [1.0, -1.0]);
        // Center maps to the origin.
        assert_eq!(UiVertex::ndc_from_pixel(w / 2.0, h / 2.0, w, h), [0.0, 0.0]);
    }

    #[test]
    fn vertex_bytes_round_trip() {
        let v = UiVertex {
            pos: [-0.5, 0.25],
            uv: [0.0, 1.0],
            color: [0.2, 0.4, 0.6, 0.8],
        };
        let bytes = v.to_bytes();
        assert_eq!(bytes.len(), UiVertex::STRIDE);
        for (i, expected) in [-0.5f32, 0.25, 0.0, 1.0, 0.2, 0.4, 0.6, 0.8]
            .iter()
            .enumerate()
        {
            let got = f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().expect("in range"));
            assert_eq!(got, *expected, "field {i}");
        }
    }

    #[test]
    fn batch_validation_rejects_out_of_range_indices() {
        assert!(validate_batch(3, &[0, 1, 2]).is_ok());
        assert!(matches!(
            validate_batch(3, &[0, 3, 2]),
            Err(EngineError::InvalidState { .. })
        ));
        assert!(validate_batch(0, &[]).is_ok(), "empty batch is valid");
    }
}
