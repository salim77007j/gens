//! Window-surface presentation for the interactive editor shell (v1.1,
//! ADR-0036).
//!
//! The engine's render path is offscreen-first (ADR-0010): the editor
//! composes every frame — 3D viewport under egui chrome — into an
//! engine-owned texture and reads the pixels back. Presentation to a
//! real OS window is layered ON TOP of that proven path: the readback
//! bytes are uploaded into the swapchain image
//! (`Queue::write_texture`) and the surface is presented.
//!
//! Why this shape for v1.1:
//! - zero divergence between headless and windowed composition (the
//!   exact same [`crate::EditorShell::frame`] call produces the pixels
//!   a user sees — screenshots are byte-identical to the screen);
//! - no second render-target format to keep in lockstep with the
//!   surface format (the upload path swizzles RGBA→BGRA on the CPU
//!   where the platform demands it);
//! - the cost is one 5–6 MB CPU round-trip per frame, invisible at
//!   editor frame rates and honestly recorded in ADR-0036 as the
//!   known v1.1 limitation (direct-to-surface rendering is future
//!   work, stated as such).
//!
//! The surface type lives in `gens-editor` (not `gens-renderer`)
//! deliberately: the renderer's identity is headless-offscreen
//! (ADR-0010/0011) and its whole test story runs without windowing;
//! only the editor shell consumes a swapchain. The renderer exposes
//! exactly what presentation needs — instance, adapter, device, queue.

use gens_core::error::{EngineError, Result};
use wgpu::{Device, Queue, SurfaceConfiguration, TextureFormat, TextureUsages};
use winit::window::Window;

/// Formats accepted for presentation, in preference order.
///
/// [`TextureFormat::Rgba8UnormSrgb`] first: it matches the offscreen
/// target's byte order exactly (no swizzle). [`TextureFormat::Bgra8UnormSrgb`]
/// is the Metal/DXGI native order and is accepted with a CPU swizzle.
const PREFERRED_FORMATS: [TextureFormat; 2] =
    [TextureFormat::Rgba8UnormSrgb, TextureFormat::Bgra8UnormSrgb];

/// Picks the presentation format from a surface's supported list.
///
/// Preference order is [`PREFERRED_FORMATS`]; `None` when the surface
/// supports neither (callers must refuse to present rather than guess).
#[must_use]
pub fn choose_surface_format(supported: &[TextureFormat]) -> Option<TextureFormat> {
    PREFERRED_FORMATS
        .iter()
        .find(|preferred| supported.contains(preferred))
        .copied()
}

/// Converts readback RGBA8 bytes into the surface's byte order.
///
/// [`TextureFormat::Rgba8UnormSrgb`] is a pass-through (the readback
/// format); [`TextureFormat::Bgra8UnormSrgb`] swaps R and B per pixel.
/// Alpha is never touched.
///
/// # Errors
/// Returns [`EngineError`] when `rgba.len()` is not `width * height * 4`
/// or the format is not a presentation format this module accepts.
pub fn rgba8_to_surface_bytes(
    format: TextureFormat,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(EngineError::invalid(
            "editor",
            format!(
                "presentation input must be {expected} bytes for {width}x{height}, got {}",
                rgba.len()
            ),
        ));
    }
    match format {
        TextureFormat::Rgba8UnormSrgb => Ok(rgba.to_vec()),
        TextureFormat::Bgra8UnormSrgb => {
            let mut out = rgba.to_vec();
            for px in out.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
            Ok(out)
        }
        other => Err(EngineError::invalid(
            "editor",
            format!("unsupported presentation format {other:?}"),
        )),
    }
}

/// Owns the swapchain side of the editor window: the wgpu surface, its
/// configuration, and the upload-present loop.
pub struct SurfaceTarget {
    surface: wgpu::Surface<'static>,
    config: SurfaceConfiguration,
}

impl SurfaceTarget {
    /// Creates the surface for a window and configures it at `width` x
    /// `height`.
    ///
    /// The context must have been built with
    /// [`gens_renderer::GpuContext::new_windowed`] (display handles
    /// present); otherwise surface creation fails loudly here.
    ///
    /// # Errors
    /// Returns [`EngineError`] when the surface cannot be created or no
    /// preferred format is supported.
    pub fn new(
        ctx: &gens_renderer::GpuContext,
        window: std::sync::Arc<Window>,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let surface = ctx
            .instance()
            .create_surface(window)
            .map_err(|e| EngineError::invalid("editor", format!("surface creation failed: {e}")))?;
        let caps = surface.get_capabilities(ctx.adapter());
        let format = choose_surface_format(&caps.formats).ok_or_else(|| {
            EngineError::invalid(
                "editor",
                format!(
                    "no supported presentation format (supported: {:?})",
                    caps.formats
                ),
            )
        })?;
        let config = Self::make_config(&caps, format, width, height);
        surface.configure(ctx.device(), &config);
        Ok(Self { surface, config })
    }

    fn make_config(
        // The capability list drove the format choice in `new`; the
        // remaining fields below are engine policy, not platform probes.
        _caps: &wgpu::SurfaceCapabilities,
        format: TextureFormat,
        width: u32,
        height: u32,
    ) -> SurfaceConfiguration {
        SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_DST,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo, // vsync; universally supported
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            // wgpu 30: the color space rides the configuration; Srgb is
            // the only mode our preferred formats accept (the surface
            // family IS sRGB-encoded).
            color_space: wgpu::SurfaceColorSpace::Srgb,
        }
    }

    /// The configured presentation format (what [`rgba8_to_surface_bytes`]
    /// must be told).
    #[must_use]
    pub fn format(&self) -> TextureFormat {
        self.config.format
    }

    /// Configured size in pixels.
    #[must_use]
    pub fn size(&self) -> [u32; 2] {
        [self.config.width, self.config.height]
    }

    /// Reconfigures the surface after a window resize.
    pub fn resize(&mut self, ctx: &gens_renderer::GpuContext, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(ctx.device(), &self.config);
    }

    /// Uploads one frame's RGBA8 readback into the swapchain and presents.
    ///
    /// Transient surface states (`OutOfDate`, `Suboptimal`, `Lost`) are
    /// reported with a "retry: true" marker — the caller reconfigures
    /// (resize) and presents again next frame instead of crashing.
    ///
    /// # Errors
    /// Returns [`EngineError::invalid`] on size mismatch, unsupported
    /// format, or a non-retryable surface error.
    pub fn present_rgba8(
        &self,
        device: &Device,
        queue: &Queue,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<()> {
        if [width, height] != self.size() {
            return Err(EngineError::invalid(
                "editor",
                format!(
                    "frame {width}x{height} does not match surface {}x{} — resize first",
                    self.config.width, self.config.height
                ),
            ));
        }
        let bytes = rgba8_to_surface_bytes(self.config.format, rgba, width, height)?;
        // wgpu 30: acquire returns a status enum, not a Result. Success
        // and Suboptimal both give a drawable frame; Outdated/Lost are
        // recoverable (reconfigure at the caller); Occluded means the
        // window is not visible — skipping the present is the correct,
        // honest no-op; everything else is fatal.
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                return Err(EngineError::invalid(
                    "editor",
                    "surface outdated/lost — reconfigure required (retry: true)",
                ));
            }
            wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
            wgpu::CurrentSurfaceTexture::Timeout => {
                return Err(EngineError::invalid("editor", "surface acquire timed out"));
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(EngineError::invalid(
                    "editor",
                    "surface acquire validation error",
                ));
            }
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &frame.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
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
        let _ = device; // reserved: future blit-based presentation
        // wgpu 30: presentation goes through the queue.
        queue.present(frame);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_preference_uses_rgba_first() {
        let supported = [TextureFormat::Bgra8UnormSrgb, TextureFormat::Rgba8UnormSrgb];
        assert_eq!(
            choose_surface_format(&supported),
            Some(TextureFormat::Rgba8UnormSrgb),
            "rgba must win when both are supported (no swizzle)"
        );
    }

    #[test]
    fn format_preference_falls_back_to_bgra() {
        // Metal and DXGI expose only the BGRA family natively.
        let supported = [TextureFormat::Bgra8Unorm, TextureFormat::Bgra8UnormSrgb];
        assert_eq!(
            choose_surface_format(&supported),
            Some(TextureFormat::Bgra8UnormSrgb)
        );
    }

    #[test]
    fn format_preference_refuses_unsupported_families() {
        // Neither preferred format present -> None (refuse, never guess).
        let supported = [TextureFormat::Rgba16Float];
        assert_eq!(choose_surface_format(&supported), None);
        assert_eq!(choose_surface_format(&[]), None);
    }

    #[test]
    fn rgba_passthrough_and_bgra_swizzle() {
        let px = [1u8, 2, 3, 4, 250, 100, 50, 255];
        // RGBA surface: bytes unchanged.
        assert_eq!(
            rgba8_to_surface_bytes(TextureFormat::Rgba8UnormSrgb, &px, 2, 1).unwrap(),
            px
        );
        // BGRA surface: R and B swap per pixel, G/A stay.
        assert_eq!(
            rgba8_to_surface_bytes(TextureFormat::Bgra8UnormSrgb, &px, 2, 1).unwrap(),
            [3u8, 2, 1, 4, 50, 100, 250, 255]
        );
    }

    #[test]
    fn surface_bytes_reject_bad_sizes_and_formats() {
        let err =
            rgba8_to_surface_bytes(TextureFormat::Rgba8UnormSrgb, &[0u8; 7], 2, 1).unwrap_err();
        assert!(err.to_string().contains("must be 8 bytes"), "{err}");
        let err = rgba8_to_surface_bytes(TextureFormat::Rgba16Float, &[0u8; 8], 2, 1).unwrap_err();
        assert!(
            err.to_string().contains("unsupported presentation"),
            "{err}"
        );
    }
}
