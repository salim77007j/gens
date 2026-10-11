//! GPU-side resources: uploaded meshes and the model-matrix uniform ring
//! (Phase 2).
//!
//! [`GpuMesh`] is a CPU [`Mesh`](crate::mesh::Mesh) uploaded once into
//! vertex/index buffers. [`ModelUniforms`] is a fixed-capacity ring of
//! 256-byte-aligned `mat4x4<f32>` slots bound with a **dynamic offset**, so
//! N objects share one buffer and one bind group; the per-draw cost is a
//! `set_bind_group(1, ..., &[offset])`.

use gens_core::error::Result;
use gens_math::Mat4;
use wgpu::util::DeviceExt;
use wgpu::{BindGroup, Device, Queue};

use crate::mesh::Mesh;
use crate::pipeline::{DYNAMIC_OFFSET_ALIGNMENT, MAT4_UNIFORM_SIZE};

/// A mesh uploaded to GPU memory.
#[derive(Debug)]
pub struct GpuMesh {
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    index_count: u32,
}

impl GpuMesh {
    /// Uploads `mesh` (validating it first — an invalid mesh would corrupt
    /// GPU reads otherwise).
    ///
    /// # Errors
    /// Returns [`EngineError`] when the mesh fails [`Mesh::validate`].
    pub fn upload(device: &Device, mesh: &Mesh) -> Result<GpuMesh> {
        mesh.validate()?;
        let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gens-mesh-vertices"),
            contents: &mesh.vertex_bytes(),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gens-mesh-indices"),
            contents: &mesh.index_bytes(),
            usage: wgpu::BufferUsages::INDEX,
        });
        Ok(GpuMesh {
            vertex_buf,
            index_buf,
            index_count: mesh.index_count(),
        })
    }

    /// Number of indices to draw.
    #[must_use]
    pub fn index_count(&self) -> u32 {
        self.index_count
    }

    /// Records the vertex/index bindings onto `pass`.
    pub fn bind_to<'rp>(&self, pass: &mut wgpu::RenderPass<'rp>)
    where
        Self: 'rp,
    {
        pass.set_vertex_buffer(0, self.vertex_buf.slice(..));
        pass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint16);
    }

    /// Records the draw call (index count).
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

/// A ring of per-object model matrices, one 256-byte slot per object,
/// bound via dynamic offset (see [`DYNAMIC_OFFSET_ALIGNMENT`]).
pub struct ModelUniforms {
    buffer: wgpu::Buffer,
    bind_group: BindGroup,
    capacity: u32,
}

impl ModelUniforms {
    /// Default ring capacity (fits comfortably in the default 64 KiB
    /// uniform binding limit: 256 slots x 256 B = 64 KiB).
    pub const DEFAULT_CAPACITY: u32 = 256;

    /// Creates the ring and its (single, reusable) bind group.
    #[must_use]
    pub fn new(device: &Device, capacity: u32, layout: &wgpu::BindGroupLayout) -> ModelUniforms {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gens-model-uniforms"),
            size: u64::from(capacity) * u64::from(DYNAMIC_OFFSET_ALIGNMENT),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gens-model-bind-group"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: Some(
                        wgpu::BufferSize::new(MAT4_UNIFORM_SIZE)
                            .expect("MAT4_UNIFORM_SIZE is non-zero"),
                    ),
                }),
            }],
        });
        ModelUniforms {
            buffer,
            bind_group,
            capacity,
        }
    }

    /// Slot capacity.
    #[must_use]
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Uploads `model` into `slot` (staged for the next submission — call
    /// before the render submission that consumes it).
    ///
    /// # Errors
    /// Returns [`EngineError`] when `slot` is out of capacity.
    pub fn write(&self, queue: &Queue, slot: u32, model: Mat4) -> Result<()> {
        let offset = crate::pipeline::model_slot_offset(slot, self.capacity)?;
        queue.write_buffer(
            &self.buffer,
            u64::from(offset),
            &crate::camera::uniform::bytes(model),
        );
        Ok(())
    }

    /// The bind group shared by every slot (dynamic offsets select slots).
    #[must_use]
    pub fn bind_group(&self) -> &BindGroup {
        &self.bind_group
    }

    /// The underlying uniform buffer (textured pipelines combine it with a
    /// texture into one group-1 bind group).
    #[must_use]
    pub(crate) fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// Byte offset for `slot`, for `set_bind_group(1, bg, &[offset])`.
    ///
    /// # Errors
    /// Returns [`EngineError`] when `slot` is out of capacity.
    pub fn offset(&self, slot: u32) -> Result<u32> {
        crate::pipeline::model_slot_offset(slot, self.capacity)
    }
}

#[cfg(test)]
mod tests {
    use crate::pipeline::model_slot_offset;

    #[test]
    fn slot_offsets_are_256_byte_aligned_and_bounded() {
        assert_eq!(model_slot_offset(0, 256).unwrap(), 0);
        assert_eq!(model_slot_offset(1, 256).unwrap(), 256);
        assert_eq!(model_slot_offset(7, 256).unwrap(), 1792);
        assert!(
            model_slot_offset(256, 256).is_err(),
            "capacity must bound slots"
        );
        assert!(model_slot_offset(u32::MAX, 256).is_err());
    }

    #[test]
    fn mesh_draw_payload_matches_counts() {
        let mesh = crate::mesh::quad();
        // Pure check: index_count feeds draw_indexed ranges.
        assert_eq!(mesh.index_count(), 6);
    }
}
