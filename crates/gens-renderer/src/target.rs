//! Offscreen render target: color + depth textures with CPU readback.
//!
//! The primary Gens render path (ADR-0010) draws into engine-owned textures
//! instead of a window surface. This makes the renderer usable on headless
//! CI machines and makes pixels testable: [`RenderTarget::read_pixels`]
//! stages the color buffer with a GPU→CPU copy and returns packed RGBA8
//! bytes ready for PNG encoding.
//!
//! The only subtlety is WebGPU's copy contract: every row in the staging
//! buffer must start at a 256-byte boundary ([`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`]),
//! so the staging buffer is padded and the padding is stripped on readback.
//! Both the padding math ([`bytes_per_row`]) and the unpack
//! ([`unpack_padded_rgba`]) are pure functions with unit tests.

use gens_core::error::{EngineError, Result};
use wgpu::{
    Buffer, COPY_BYTES_PER_ROW_ALIGNMENT, CommandEncoder, Device, Extent3d, Texture,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor,
};

/// Depth-buffer format used by every Gens offscreen target (ADR-0010).
pub const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth24Plus;

/// Bytes per padded row of a `width`-pixel RGBA8 image, rounded up to the
/// 256-byte alignment the WebGPU copy contract requires.
#[must_use]
pub fn bytes_per_row(width: u32) -> Option<u32> {
    let raw = (width as u64)
        .checked_mul(4)?
        .checked_next_multiple_of(COPY_BYTES_PER_ROW_ALIGNMENT as u64)?;
    u32::try_from(raw).ok()
}

/// Strips 256-byte row padding from a staging buffer, returning packed
/// `width * height * 4` RGBA8 bytes (rows top-to-bottom).
///
/// Returns an error instead of panicking when `padded` does not match the
/// padded size implied by `width`/`height`.
pub fn unpack_padded_rgba(padded: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let bpr = bytes_per_row(width).ok_or_else(|| {
        EngineError::invalid(
            "renderer",
            format!("width {width} overflows row padding math"),
        )
    })?;
    let row = width as usize * 4;
    let expected = bpr as usize * height as usize;
    if padded.len() < expected {
        return Err(EngineError::invalid(
            "renderer",
            format!(
                "staging buffer too small: {} bytes, expected {expected} for {width}x{height}",
                padded.len()
            ),
        ));
    }
    let mut out = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        let start = y * bpr as usize;
        out.extend_from_slice(&padded[start..start + row]);
    }
    Ok(out)
}

/// An offscreen color+depth target with a staging buffer for readback.
pub struct RenderTarget {
    width: u32,
    height: u32,
    color: Texture,
    color_view: TextureView,
    depth: Texture,
    depth_view: TextureView,
    staging: Buffer,
}

impl RenderTarget {
    /// Creates a target of `width` x `height` pixels.
    ///
    /// # Errors
    /// Returns [`EngineError`] when the size is zero or exceeds the device's
    /// default texture-dimension limit.
    pub fn new(device: &Device, width: u32, height: u32) -> Result<RenderTarget> {
        Self::validate(width, height)?;
        let bpr = bytes_per_row(width).ok_or_else(|| {
            EngineError::invalid(
                "renderer",
                format!("width {width} overflows row padding math"),
            )
        })?;
        let size = Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let color = device.create_texture(&TextureDescriptor {
            label: Some("gens-target-color"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color.create_view(&TextureViewDescriptor::default());

        let depth = device.create_texture(&TextureDescriptor {
            label: Some("gens-target-depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&TextureViewDescriptor::default());

        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gens-target-staging"),
            size: bpr as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Ok(RenderTarget {
            width,
            height,
            color,
            color_view,
            depth,
            depth_view,
            staging,
        })
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The color attachment view (bind target for render passes).
    #[must_use]
    pub fn color_view(&self) -> &TextureView {
        &self.color_view
    }

    /// The color texture itself (e.g. for later bind-as-input passes).
    #[must_use]
    pub fn color(&self) -> &Texture {
        &self.color
    }

    /// The depth attachment view.
    #[must_use]
    pub fn depth_view(&self) -> &TextureView {
        &self.depth_view
    }

    /// The depth texture itself.
    #[must_use]
    pub fn depth(&self) -> &Texture {
        &self.depth
    }

    /// Records the GPU→staging copy for the color buffer onto `encoder`.
    /// Call before submitting; the copy lands in the same submission as the
    /// render pass so a single wait covers both.
    pub fn copy_color_to_staging(&self, encoder: &mut CommandEncoder) {
        let bpr = bytes_per_row(self.width).expect("validated at construction");
        encoder.copy_texture_to_buffer(
            self.color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bpr),
                    rows_per_image: Some(self.height),
                },
            },
            Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Waits for the staging copy, maps it and returns packed RGBA8 bytes
    /// (padding stripped). Blocks until the GPU is done.
    ///
    /// # Errors
    /// Returns [`EngineError`] when the map fails or the buffer is short.
    pub fn read_pixels(&self, device: &Device) -> Result<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                // The callback must not panic; report through the channel.
                let _ = tx.send(result);
            });
        // Wait for the map callback; PollType::wait_indefinitely blocks on
        // the latest submission, but mapping completion is what we care
        // about, so poll until the channel delivers.
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()
            .map_err(|e| EngineError::invalid("renderer", format!("map callback dropped: {e}")))?
            .map_err(|e| EngineError::invalid("renderer", format!("staging map failed: {e}")))?;

        let data =
            self.staging.slice(..).get_mapped_range().map_err(|e| {
                EngineError::invalid("renderer", format!("staging read failed: {e}"))
            })?;
        let unpacked = unpack_padded_rgba(&data, self.width, self.height);
        drop(data);
        self.staging.unmap();
        unpacked
    }

    fn validate(width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Err(EngineError::invalid(
                "renderer",
                format!("render target size must be non-zero, got {width}x{height}"),
            ));
        }
        let max = wgpu::Limits::default().max_texture_dimension_2d;
        if width > max || height > max {
            return Err(EngineError::invalid(
                "renderer",
                format!("render target {width}x{height} exceeds default limit {max}"),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_per_row_aligns_to_256() {
        // 4-byte-aligned widths that already hit 256: 64 px * 4 = 256.
        assert_eq!(bytes_per_row(64), Some(256));
        // 65 px * 4 = 260 -> next multiple of 256 = 512.
        assert_eq!(bytes_per_row(65), Some(512));
        // Small: 1 px * 4 = 4 -> 256.
        assert_eq!(bytes_per_row(1), Some(256));
        // 127 px * 4 = 508 -> 512; 128 px -> exactly 512.
        assert_eq!(bytes_per_row(127), Some(512));
        assert_eq!(bytes_per_row(128), Some(512));
        // 129 px * 4 = 516 -> 768.
        assert_eq!(bytes_per_row(129), Some(768));
    }

    #[test]
    fn bytes_per_row_rejects_overflow() {
        assert_eq!(bytes_per_row(u32::MAX), None);
    }

    #[test]
    fn unpack_strips_row_padding() {
        // 3 px wide -> bpr 256. Two rows: first 12 bytes red, rest junk;
        // second row 12 bytes blue.
        let w = 3u32;
        let h = 2u32;
        let mut padded = vec![0xEEu8; 256 * 2];
        for px in padded[..12].as_chunks_mut::<4>().0 {
            *px = [255, 0, 0, 255];
        }
        for px in padded[256..268].as_chunks_mut::<4>().0 {
            *px = [0, 0, 255, 255];
        }
        let out = unpack_padded_rgba(&padded, w, h).expect("unpack");
        assert_eq!(out.len(), (w * h * 4) as usize);
        assert_eq!(
            &out[..12],
            &[255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255]
        );
        assert_eq!(
            &out[12..24],
            &[0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255]
        );
    }

    #[test]
    fn unpack_rejects_short_buffer() {
        let err = unpack_padded_rgba(&[0u8; 100], 64, 2).unwrap_err();
        assert!(err.to_string().contains("too small"), "{err}");
    }

    /// Real GPU test: clear + readback round-trip through the offscreen
    /// target (ADR-0010 core loop). Requires an adapter (ADR-0011).
    #[test]
    #[ignore = "requires a GPU adapter (real or software)"]
    fn offscreen_clear_round_trip() {
        let ctx = crate::context::GpuContext::new().expect("GPU context");
        let device = ctx.device();
        let queue = ctx.queue();
        let target = RenderTarget::new(device, 17, 9).expect("target"); // non-256-multiple width
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("test-encoder"),
        });
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.color_view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 1.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            drop(pass);
        }
        target.copy_color_to_staging(&mut encoder);
        queue.submit([encoder.finish()]);
        let px = target.read_pixels(device).expect("readback");
        assert_eq!(px.len(), (17 * 9 * 4) as usize);
        // Pure green clear, exact at the 0/1 sRGB endpoints.
        for chunk in px.as_chunks::<4>().0 {
            assert_eq!(chunk, &[0, 255, 0, 255], "unexpected pixel {chunk:?}");
        }
    }

    /// Real GPU test: zero-size target must be rejected before touching the
    /// device.
    #[test]
    #[ignore = "requires a GPU adapter (real or software)"]
    fn zero_size_target_is_rejected() {
        let ctx = crate::context::GpuContext::new().expect("GPU context");
        assert!(RenderTarget::new(ctx.device(), 0, 8).is_err());
        assert!(RenderTarget::new(ctx.device(), 8, 0).is_err());
    }
}
