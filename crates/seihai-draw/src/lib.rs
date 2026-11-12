//! Software drawing of a level tile grid and one gani frame.
//!
//! The buffer is row-major RGBA: `width * height * 4` bytes. There is no
//! window and no sound, so a tool can link this crate without the game
//! client. Levels and ganis are [`seihai_core`] types; nothing here parses
//! a level or a gani of its own. [`Image`] is a view of bytes in the layout
//! `seihai_core::assets::read_png_rgb` or `read_png_rgba` already returns.
//!
//! The live client's camera, chests and HUD stay in `seihai-mae`. This
//! crate only composites tiles and sprites.

mod sprite;
mod tiles;

pub use sprite::{draw_gani_frame, draw_sprite, SpriteSheets};
pub use tiles::{
    draw_level_tiles, draw_tile, tile_cell, BOARD_TILES, SHEET_COLS, SHEET_ROWS, TILE_PX,
};

/// How a copied pixel treats transparency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyMode {
    /// A tile: every source pixel is written, with alpha 255.
    Opaque,
    /// A sprite: alpha 0, and magenta in an RGB image, are left as they were.
    Sprite,
}

/// Row-major RGBA8. `rgba.len()` is `width * height * 4`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PixelBuffer {
    pub width: u32,
    pub height: u32,
    /// `R, G, B, A` for each pixel, left to right, top to bottom.
    pub rgba: Vec<u8>,
}

impl PixelBuffer {
    /// A transparent buffer.
    pub fn new(width: u32, height: u32) -> Self {
        Self::filled(width, height, [0, 0, 0, 0])
    }

    /// A buffer filled with one colour.
    pub fn filled(width: u32, height: u32, color: [u8; 4]) -> Self {
        let n = (width as usize) * (height as usize);
        let mut rgba = Vec::with_capacity(n * 4);
        for _ in 0..n {
            rgba.extend_from_slice(&color);
        }
        Self {
            width,
            height,
            rgba,
        }
    }

    /// The pixel at `(x, y)`, or `None` outside the buffer.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.rgba.get(i..i + 4).map(|p| [p[0], p[1], p[2], p[3]])
    }
}

/// A decoded image: RGB (3 bytes a pixel) or RGBA (4).
///
/// This is not a file parser. Build it from the `(width, height, pixels)`
/// `seihai_core::assets::read_png_rgb` or `read_png_rgba` returns, or from
/// any other buffer in that layout.
#[derive(Debug, Clone, Copy)]
pub struct Image<'a> {
    pub pixels: &'a [u8],
    pub width: u32,
    pub height: u32,
    /// 3 for RGB, 4 for RGBA. Any other count draws nothing.
    pub channels: usize,
}

impl<'a> Image<'a> {
    /// Bytes in the `read_png_rgb` layout.
    pub fn rgb(pixels: &'a [u8], width: u32, height: u32) -> Self {
        Self {
            pixels,
            width,
            height,
            channels: 3,
        }
    }

    /// Bytes in the `read_png_rgba` layout.
    pub fn rgba(pixels: &'a [u8], width: u32, height: u32) -> Self {
        Self {
            pixels,
            width,
            height,
            channels: 4,
        }
    }
}

/// Copy `src_rect` `(x, y, w, h)` so its top-left lands at `at`.
///
/// Pixels that fall outside either image are skipped. A short `pixels`
/// slice skips the samples it does not hold.
pub(crate) fn blit(
    dst: &mut PixelBuffer,
    src: Image<'_>,
    src_rect: (i32, i32, i32, i32),
    at: (i64, i64),
    mode: CopyMode,
) {
    let (sx, sy, sw, sh) = src_rect;
    let (dx, dy) = at;
    if sw <= 0 || sh <= 0 || (src.channels != 3 && src.channels != 4) {
        return;
    }
    for ty in 0..sh {
        for tx in 0..sw {
            let px = dx + i64::from(tx);
            let py = dy + i64::from(ty);
            if px < 0 || py < 0 || px >= i64::from(dst.width) || py >= i64::from(dst.height) {
                continue;
            }
            let ssx = i64::from(sx) + i64::from(tx);
            let ssy = i64::from(sy) + i64::from(ty);
            if ssx < 0 || ssy < 0 || ssx >= i64::from(src.width) || ssy >= i64::from(src.height) {
                continue;
            }
            let sidx = (ssy as usize * src.width as usize + ssx as usize) * src.channels;
            let Some(sample) = src.pixels.get(sidx..sidx + src.channels) else {
                continue;
            };
            if mode == CopyMode::Sprite && src.channels == 4 && sample[3] == 0 {
                continue;
            }
            if mode == CopyMode::Sprite && src.channels == 3 && sample == [255, 0, 255] {
                continue;
            }
            let rgba = if src.channels == 4 && mode == CopyMode::Sprite {
                [sample[0], sample[1], sample[2], sample[3]]
            } else {
                [sample[0], sample[1], sample[2], 255]
            };
            let didx = (py as usize * dst.width as usize + px as usize) * 4;
            if let Some(slot) = dst.rgba.get_mut(didx..didx + 4) {
                slot.copy_from_slice(&rgba);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filled_buffer_reports_its_pixels_and_refuses_the_outside() {
        let buf = PixelBuffer::filled(2, 1, [4, 5, 6, 7]);
        assert_eq!(buf.rgba.len(), 8);
        assert_eq!(buf.pixel(1, 0), Some([4, 5, 6, 7]));
        assert_eq!(buf.pixel(2, 0), None);
        assert_eq!(buf.pixel(0, 1), None);
        assert!(PixelBuffer::new(0, 0).rgba.is_empty());
    }
}
