//! CPU-side scene snapshot rasterizer (Phase 1).
//!
//! Before the GPU renderer exists (Phase 2), the engine can still produce
//! *honest* visual evidence of a running scene: this module rasterizes
//! simple primitives plus a built-in 5x7 bitmap font into an RGBA buffer
//! and encodes PNGs. The hello-world example (and the CI smoke test) use it
//! to prove the game loop, ECS, and transforms actually ran — no window
//! system, no GPU, deterministic bytes on every platform.
//!
//! This is a test/evidence tool, not a rendering path; it keeps serving
//! headless CI even after wgpu lands.

use gens_core::error::{EngineError, Result};

/// An RGBA color as bytes.
pub type Rgba = [u8; 4];

/// Snapshot background (dark blue-gray).
pub const COLOR_BACKGROUND: Rgba = [24, 26, 33, 255];
/// Grid line color.
pub const COLOR_GRID: Rgba = [46, 50, 62, 255];
/// Text color.
pub const COLOR_TEXT: Rgba = [232, 234, 240, 255];
/// Accent (entity markers).
pub const COLOR_ACCENT: Rgba = [86, 182, 255, 255];
/// Trail dots.
pub const COLOR_TRAIL: Rgba = [96, 116, 148, 255];

/// An RGBA8 image that can be drawn into and encoded as PNG.
pub struct Snapshot {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA8 rows, top to bottom, `width * height * 4` bytes.
    pub buffer: Vec<u8>,
}

impl Snapshot {
    /// Allocates a snapshot filled with `background`.
    pub fn new(width: u32, height: u32, background: Rgba) -> Snapshot {
        let mut snap = Snapshot {
            width,
            height,
            buffer: vec![0; (width as usize) * (height as usize) * 4],
        };
        snap.clear(background);
        snap
    }

    /// Fills everything with `color`.
    pub fn clear(&mut self, color: Rgba) {
        for px in self.buffer.as_chunks_mut::<4>().0 {
            px.copy_from_slice(&color);
        }
    }

    /// Writes one pixel; out-of-bounds coordinates are ignored.
    pub fn px(&mut self, x: i32, y: i32, color: Rgba) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let idx = ((y as u32 * self.width + x as u32) * 4) as usize;
        self.buffer[idx..idx + 4].copy_from_slice(&color);
    }

    /// Reads one pixel, if inside bounds.
    pub fn get_px(&self, x: i32, y: i32) -> Option<Rgba> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        let idx = ((y as u32 * self.width + x as u32) * 4) as usize;
        let mut c = [0u8; 4];
        c.copy_from_slice(&self.buffer[idx..idx + 4]);
        Some(c)
    }

    /// Filled axis-aligned rectangle.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Rgba) {
        for dy in 0..h as i32 {
            for dx in 0..w as i32 {
                self.px(x + dx, y + dy, color);
            }
        }
    }

    /// Outline rectangle (1px border).
    pub fn stroke_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Rgba) {
        for dx in 0..w as i32 {
            self.px(x + dx, y, color);
            self.px(x + dx, y + h as i32 - 1, color);
        }
        for dy in 0..h as i32 {
            self.px(x, y + dy, color);
            self.px(x + w as i32 - 1, y + dy, color);
        }
    }

    /// Vertical and horizontal grid lines every `step` pixels.
    pub fn draw_grid(&mut self, step: u32, color: Rgba) {
        let mut x = 0i32;
        while (x as u32) < self.width {
            for y in 0..self.height as i32 {
                self.px(x, y, color);
            }
            x += step as i32;
        }
        let mut y = 0i32;
        while (y as u32) < self.height {
            for x in 0..self.width as i32 {
                self.px(x, y, color);
            }
            y += step as i32;
        }
    }

    /// Draws `text` with the built-in 5x7 font, 6px advance per glyph.
    /// Unknown characters render as a space.
    pub fn draw_text(&mut self, x: i32, y: i32, text: &str, color: Rgba) {
        let mut cursor = x;
        for ch in text.chars() {
            if let Some(glyph) = glyph_for(ch) {
                for (row, bits) in glyph.iter().enumerate() {
                    for col in 0..5i32 {
                        if bits & (0x10 >> col) != 0 {
                            self.px(cursor + col, y + row as i32, color);
                        }
                    }
                }
            }
            cursor += 6;
        }
    }

    /// Measures text width in pixels (6 per glyph, including unknown ones).
    pub fn text_width(text: &str) -> i32 {
        6 * text.chars().count() as i32
    }

    /// Encodes the snapshot as PNG bytes.
    pub fn encode_png(&self) -> Result<Vec<u8>> {
        let img: image::RgbaImage =
            image::ImageBuffer::from_raw(self.width, self.height, self.buffer.clone()).ok_or_else(
                || {
                    EngineError::invalid(
                        "snapshot",
                        format!(
                            "buffer length {} does not match {}x{}x4",
                            self.buffer.len(),
                            self.width,
                            self.height
                        ),
                    )
                },
            )?;
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| EngineError::invalid("snapshot", format!("png encode failed: {e}")))?;
        Ok(png)
    }
}

/// 5x7 glyph rows (5 significant bits, MSB = leftmost column).
type Glyph = [u8; 7];

/// Bitmap font for the characters engine snapshots use. Anything else
/// renders as a space.
fn glyph_for(ch: char) -> Option<Glyph> {
    Some(match ch {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        ':' => [0x00, 0x04, 0x00, 0x00, 0x00, 0x04, 0x00],
        '=' => [0x00, 0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '/' => [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn px_is_bounds_safe() {
        let mut snap = Snapshot::new(4, 4, COLOR_BACKGROUND);
        snap.px(-1, 0, COLOR_ACCENT);
        snap.px(0, -1, COLOR_ACCENT);
        snap.px(4, 0, COLOR_ACCENT);
        snap.px(0, 4, COLOR_ACCENT);
        // Nothing changed.
        assert_eq!(snap.buffer, Snapshot::new(4, 4, COLOR_BACKGROUND).buffer);
        snap.px(2, 1, COLOR_ACCENT);
        assert_eq!(snap.get_px(2, 1), Some(COLOR_ACCENT));
    }

    #[test]
    fn fill_and_stroke_rect_set_expected_pixels() {
        let mut snap = Snapshot::new(10, 10, COLOR_BACKGROUND);
        snap.fill_rect(2, 2, 3, 3, COLOR_ACCENT);
        assert_eq!(snap.get_px(3, 3), Some(COLOR_ACCENT));
        assert_eq!(snap.get_px(5, 5), Some(COLOR_BACKGROUND)); // outside

        let mut s2 = Snapshot::new(10, 10, COLOR_BACKGROUND);
        s2.stroke_rect(0, 0, 10, 10, COLOR_GRID);
        assert_eq!(s2.get_px(0, 0), Some(COLOR_GRID));
        assert_eq!(s2.get_px(9, 9), Some(COLOR_GRID));
        assert_eq!(s2.get_px(5, 5), Some(COLOR_BACKGROUND)); // hollow center
    }

    #[test]
    fn grid_draws_lines_at_step_intervals() {
        let mut snap = Snapshot::new(10, 10, COLOR_BACKGROUND);
        snap.draw_grid(5, COLOR_GRID);
        assert_eq!(snap.get_px(5, 7), Some(COLOR_GRID)); // vertical line
        assert_eq!(snap.get_px(7, 5), Some(COLOR_GRID)); // horizontal line
        assert_eq!(snap.get_px(7, 7), Some(COLOR_BACKGROUND));
    }

    #[test]
    fn text_draws_known_glyph_and_skips_unknown() {
        let mut snap = Snapshot::new(40, 12, COLOR_BACKGROUND);
        snap.draw_text(0, 0, "G1?", COLOR_TEXT);
        // 'G' top-left pixel of the glyph: row0 = 0b01110 -> x=1 lit.
        assert_eq!(snap.get_px(1, 0), Some(COLOR_TEXT));
        // '1' glyph center column: starts at x=6; row0 bits 0b00100 -> x=8.
        assert_eq!(snap.get_px(8, 0), Some(COLOR_TEXT));
        // '?' unknown -> its cell is background, but the cursor still advanced.
        assert_eq!(snap.get_px(18, 0), Some(COLOR_BACKGROUND));
        assert_eq!(Snapshot::text_width("G1?"), 18);
    }

    #[test]
    fn png_encoding_produces_png_signature() {
        let snap = Snapshot::new(8, 8, COLOR_BACKGROUND);
        let png = snap.encode_png().expect("encode");
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    }
}
