//! Render pipelines compiled from WGSL (Phase 2).
//!
//! Every Gens pipeline shares one binding scheme: **group 0** carries the
//! camera uniform, **group 1** carries per-draw resources (a model matrix
//! with a dynamic offset; textures join the same group in the textured
//! pipeline). Keeping the scheme uniform lets the [`crate::Renderer`]
//! switch pipelines without reshuffling bind groups.

use gens_core::error::{EngineError, Result};
use wgpu::{
    BindGroupLayout, BindGroupLayoutEntry, BindingType, BufferBindingType, BufferSize,
    ColorTargetState, ColorWrites, Device, Face, FragmentState, PipelineLayoutDescriptor,
    PrimitiveState, PrimitiveTopology, RenderPipeline, RenderPipelineDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, TextureFormat, VertexState,
};

/// Byte size of one `mat4x4<f32>` uniform (camera and model blocks alike).
pub const MAT4_UNIFORM_SIZE: u64 = 64;

/// Alignment required for dynamic-offset uniform slots
/// (`Limits::min_uniform_buffer_offset_alignment` at defaults).
pub const DYNAMIC_OFFSET_ALIGNMENT: u32 = 256;

/// Computes the byte offset of model-uniform slot `slot`, validating the
/// capacity bound that the underlying buffer was created with.
pub fn model_slot_offset(slot: u32, capacity: u32) -> Result<u32> {
    if slot >= capacity {
        return Err(EngineError::invalid(
            "renderer",
            format!("model slot {slot} out of range (capacity {capacity})"),
        ));
    }
    slot.checked_mul(DYNAMIC_OFFSET_ALIGNMENT).ok_or_else(|| {
        EngineError::invalid("renderer", format!("model slot {slot} overflows offsets"))
    })
}

/// The unlit colored-mesh pipeline (`shaders/colored.wgsl`).
pub struct ColoredPipeline {
    pipeline: RenderPipeline,
    camera_bgl: BindGroupLayout,
    model_bgl: BindGroupLayout,
}

impl ColoredPipeline {
    /// WGSL source, compiled in from `shaders/colored.wgsl`.
    pub const WGSL: &'static str = include_str!("../shaders/colored.wgsl");

    /// Compiles the pipeline for an `Rgba8UnormSrgb` color target.
    ///
    /// # Errors
    /// Returns [`EngineError`] if shader compilation or pipeline creation
    /// fails (a broken WGSL string ships as a test failure, never a panic).
    pub fn new(device: &Device, color_format: TextureFormat) -> Result<ColoredPipeline> {
        let module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("gens-colored-shader"),
            source: ShaderSource::Wgsl(Self::WGSL.into()),
        });

        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gens-camera-bgl"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(MAT4_UNIFORM_SIZE),
                },
                count: None,
            }],
        });

        let model_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gens-model-bgl"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: BufferSize::new(MAT4_UNIFORM_SIZE),
                },
                count: None,
            }],
        });

        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("gens-colored-layout"),
            bind_group_layouts: &[Some(&camera_bgl), Some(&model_bgl)],
            immediate_size: 0,
        });

        let vertex_layout = crate::mesh::Vertex::layout();
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("gens-colored-pipeline"),
            layout: Some(&layout),
            vertex: VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout)],
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                cull_mode: Some(Face::Back),
                ..PrimitiveState::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::target::DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Ok(ColoredPipeline {
            pipeline,
            camera_bgl,
            model_bgl,
        })
    }

    /// The compiled pipeline for render-pass binding.
    #[must_use]
    pub fn raw(&self) -> &RenderPipeline {
        &self.pipeline
    }

    /// Layout for the group-0 camera bind group.
    #[must_use]
    pub fn camera_bgl(&self) -> &BindGroupLayout {
        &self.camera_bgl
    }

    /// Layout for the group-1 model bind group.
    #[must_use]
    pub fn model_bgl(&self) -> &BindGroupLayout {
        &self.model_bgl
    }
}

/// The textured-mesh pipeline (`shaders/textured.wgsl`).
///
/// Group 1 combines the shared model-uniform ring with a sampler and a
/// texture, so one bind group exists per (texture, model-ring) pair. The
/// fragment output is `vertex_color * texture_sample` — white vertex
/// colors mean "untinted".
pub struct TexturedPipeline {
    pipeline: RenderPipeline,
    camera_bgl: BindGroupLayout,
    draw_bgl: BindGroupLayout,
}

impl TexturedPipeline {
    /// WGSL source, compiled in from `shaders/textured.wgsl`.
    pub const WGSL: &'static str = include_str!("../shaders/textured.wgsl");

    /// Compiles the pipeline for an `Rgba8UnormSrgb` color target.
    ///
    /// # Errors
    /// Returns [`EngineError`] if shader compilation or pipeline creation
    /// fails.
    pub fn new(device: &Device, color_format: TextureFormat) -> Result<TexturedPipeline> {
        let module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("gens-textured-shader"),
            source: ShaderSource::Wgsl(Self::WGSL.into()),
        });

        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gens-camera-bgl-textured"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(MAT4_UNIFORM_SIZE),
                },
                count: None,
            }],
        });

        let draw_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gens-textured-draw-bgl"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: BufferSize::new(MAT4_UNIFORM_SIZE),
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("gens-textured-layout"),
            bind_group_layouts: &[Some(&camera_bgl), Some(&draw_bgl)],
            immediate_size: 0,
        });

        let vertex_layout = crate::mesh::Vertex::layout();
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("gens-textured-pipeline"),
            layout: Some(&layout),
            vertex: VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout)],
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                cull_mode: Some(Face::Back),
                ..PrimitiveState::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::target::DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Ok(TexturedPipeline {
            pipeline,
            camera_bgl,
            draw_bgl,
        })
    }

    /// The compiled pipeline for render-pass binding.
    #[must_use]
    pub fn raw(&self) -> &RenderPipeline {
        &self.pipeline
    }

    /// Layout for the group-0 camera bind group.
    #[must_use]
    pub fn camera_bgl(&self) -> &BindGroupLayout {
        &self.camera_bgl
    }

    /// Builds a group-1 bind group binding the model ring's dynamic uniform
    /// together with `texture`'s sampler and view.
    #[must_use]
    pub fn create_draw_bind_group(
        &self,
        device: &Device,
        model_buffer: &wgpu::Buffer,
        texture: &crate::texture::GpuTexture,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gens-textured-draw-bg"),
            layout: &self.draw_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: model_buffer,
                        offset: 0,
                        size: Some(
                            BufferSize::new(MAT4_UNIFORM_SIZE)
                                .expect("MAT4_UNIFORM_SIZE is non-zero"),
                        ),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(texture.sampler()),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(texture.view()),
                },
            ],
        })
    }
}
