//! GPU textures created from CPU memory (Phase 2).
//!
//! [`GpuTexture::from_rgba8`] uploads an RGBA8 image (e.g. decoded by the
//! `image` crate) with a linear-filter, clamp-to-edge sampler — the
//! WebGPU-friendly defaults that make non-power-of-two sizes legal.
//! Textures bind through the textured pipeline's group-1 layout; the bind
//! group couples them with the shared model-uniform ring.

use gens_core::error::{EngineError, Result};
use wgpu::{Device, Queue, TextureView};

/// An RGBA8 GPU texture with its view and sampler.
#[derive(Debug)]
pub struct GpuTexture {
    texture: wgpu::Texture,
    view: TextureView,
    sampler: wgpu::Sampler,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// Validates the CPU-side contract of an RGBA8 upload (pure function).
pub fn validate_rgba8(width: u32, height: u32, data: &[u8]) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(EngineError::invalid(
            "renderer",
            format!("texture size must be non-zero, got {width}x{height}"),
        ));
    }
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|wh| wh.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or_else(|| EngineError::invalid("renderer", "texture byte size overflows usize"))?;
    if data.len() != expected {
        return Err(EngineError::invalid(
            "renderer",
            format!(
                "texture data length {} does not match {width}x{height}x4 = {expected}",
                data.len()
            ),
        ));
    }
    Ok(())
}

impl GpuTexture {
    /// Uploads an RGBA8 image. Rows are top-to-bottom (v = 0 is the first
    /// row), matching `image` crate buffers and WGSL sampling conventions.
    ///
    /// # Errors
    /// Returns [`EngineError`] when the size/length contract is violated.
    pub fn from_rgba8(
        device: &Device,
        queue: &Queue,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<GpuTexture> {
        validate_rgba8(width, height, rgba)?;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gens-texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gens-texture-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..wgpu::SamplerDescriptor::default()
        });
        Ok(GpuTexture {
            texture,
            view,
            sampler,
            width,
            height,
        })
    }

    /// The sampled view.
    #[must_use]
    pub fn view(&self) -> &TextureView {
        &self.view
    }

    /// The sampler.
    #[must_use]
    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// The underlying texture (e.g. for future copy operations).
    #[must_use]
    pub fn raw(&self) -> &wgpu::Texture {
        &self.texture
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_enforces_length_contract() {
        assert!(validate_rgba8(2, 2, &[0u8; 16]).is_ok());
        assert!(validate_rgba8(1, 4, &[0u8; 16]).is_ok());
        assert!(validate_rgba8(2, 2, &[0u8; 15]).is_err());
        assert!(validate_rgba8(2, 2, &[0u8; 17]).is_err());
        assert!(validate_rgba8(0, 2, &[0u8; 8]).is_err());
        assert!(validate_rgba8(2, 0, &[0u8; 8]).is_err());
        // Large sizes must overflow-check rather than wrap.
        assert!(validate_rgba8(u32::MAX, u32::MAX, &[]).is_err());
    }
}
