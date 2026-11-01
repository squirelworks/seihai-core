//! A level's tile grid, blitted from a `pics1.png`-style sheet.

use seihai_core::assets::{tile_id, Level};

use crate::{blit, CopyMode, Image, PixelBuffer};

/// Width and height of one tile, in pixels.
pub const TILE_PX: u32 = 16;
/// Columns of tiles on a full `pics1.png` sheet (2048 px).
pub const SHEET_COLS: u32 = 128;
/// Rows of tiles on a full `pics1.png` sheet (512 px).
pub const SHEET_ROWS: u32 = 32;
/// Edge of a level board, in tiles.
pub const BOARD_TILES: u32 = 64;

/// Where tile `id` sits on the sheet, as `(column, row)` in tiles.
///
/// The sheet is eight blocks of 16 columns and 512 tiles. Tile `n` is at
/// column `n % 16 + (n / 512) * 16`, row `(n / 16) % 32`. The same mapping
/// the client's renderer and the offline level editor use.
pub fn tile_cell(id: u16) -> (u32, u32) {
    let t = u32::from(id);
    (t % 16 + (t / 512) * 16, (t / 16) % SHEET_ROWS)
}

/// Blit tile `id` from `sheet` so its top-left is `(dx, dy)`.
///
/// The tile is skipped when its 16×16 block does not lie entirely on the
/// sheet. Copied pixels are opaque, including a magenta pixel: colour-key
/// is for sprites, not for the board.
pub fn draw_tile(dst: &mut PixelBuffer, sheet: Image<'_>, id: u16, dx: i32, dy: i32) {
    let (col, row) = tile_cell(id);
    let sx = col * TILE_PX;
    let sy = row * TILE_PX;
    if sx + TILE_PX > sheet.width || sy + TILE_PX > sheet.height {
        return;
    }
    blit(
        dst,
        sheet,
        (sx as i32, sy as i32, TILE_PX as i32, TILE_PX as i32),
        (i64::from(dx), i64::from(dy)),
        CopyMode::Opaque,
    );
}

/// Draw `level`'s tiles into `dst`.
///
/// The buffer's top-left is level pixel `(origin_x, origin_y)`. Tiles are
/// [`TILE_PX`] pixels, and only the 64×64 board is drawn. Layers go from
/// the lowest number to the highest, so a layer 1 tile covers the layer 0
/// tile under it. On one layer the first `BOARD` run that covers a cell
/// wins, which is what [`Level::tile_id_at`] answers for layer 0. A cell a
/// layer does not define is left as it was. A tile that does not fit on
/// `sheet` is skipped.
///
/// This is the board only. Chest records, the camera and the "dead player
/// sees a black board" rule stay with the live client.
pub fn draw_level_tiles(
    dst: &mut PixelBuffer,
    level: &Level,
    sheet: Image<'_>,
    origin_x: i32,
    origin_y: i32,
) {
    let (Some((x0, x1)), Some((y0, y1))) = (
        visible_tiles(origin_x, dst.width),
        visible_tiles(origin_y, dst.height),
    ) else {
        return;
    };
    let mut layers: Vec<i32> = level.boards.iter().map(|run| run.layer).collect();
    layers.sort_unstable();
    layers.dedup();
    let tile = i64::from(TILE_PX);
    for layer in layers {
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                let Some(id) = tile_on_layer(level, layer, tx, ty) else {
                    continue;
                };
                let dx = i64::from(tx) * tile - i64::from(origin_x);
                let dy = i64::from(ty) * tile - i64::from(origin_y);
                let (col, row) = tile_cell(id);
                let sx = col * TILE_PX;
                let sy = row * TILE_PX;
                if sx + TILE_PX > sheet.width || sy + TILE_PX > sheet.height {
                    continue;
                }
                blit(
                    dst,
                    sheet,
                    (sx as i32, sy as i32, TILE_PX as i32, TILE_PX as i32),
                    (dx, dy),
                    CopyMode::Opaque,
                );
            }
        }
    }
}

/// The first run on `layer` that covers `(x, y)`, as a tile id.
fn tile_on_layer(level: &Level, layer: i32, x: i32, y: i32) -> Option<u16> {
    for run in &level.boards {
        if run.layer == layer && run.y == y && x >= run.x && x < run.x + run.width {
            let idx = (x - run.x) as usize;
            return run.tiles.get(idx).copied().map(tile_id);
        }
    }
    None
}

/// Tile columns (or rows) that overlap a span of `span` pixels starting at
/// `origin`. `None` when the span misses the 64-tile board.
fn visible_tiles(origin: i32, span: u32) -> Option<(i32, i32)> {
    if span == 0 {
        return None;
    }
    let tile = i64::from(TILE_PX);
    let origin = i64::from(origin);
    let first = origin.div_euclid(tile);
    let last = (origin + i64::from(span) - 1).div_euclid(tile);
    if last < 0 || first >= i64::from(BOARD_TILES) {
        return None;
    }
    let first = first.max(0) as i32;
    let last = last.min(i64::from(BOARD_TILES) - 1) as i32;
    (first <= last).then_some((first, last))
}

#[cfg(test)]
mod tests {
    use super::*;
    use seihai_core::assets::{read_png_rgb, write_png_rgb, Level};

    fn sheet(colors: &[[u8; 3]]) -> Vec<u8> {
        let w = colors.len() as u32 * TILE_PX;
        let mut pixels = vec![0u8; (w * TILE_PX * 3) as usize];
        for (id, rgb) in colors.iter().enumerate() {
            let col = id as u32;
            for y in 0..TILE_PX {
                for x in 0..TILE_PX {
                    let i = ((y * w + col * TILE_PX + x) * 3) as usize;
                    pixels[i..i + 3].copy_from_slice(rgb);
                }
            }
        }
        pixels
    }

    #[test]
    fn tile_cells_follow_the_pics1_blocks() {
        assert_eq!(tile_cell(0), (0, 0));
        assert_eq!(tile_cell(16), (0, 1));
        assert_eq!(tile_cell(511), (15, 31));
        assert_eq!(tile_cell(512), (16, 0));
        // Block 1, column 1, row 0.
        assert_eq!(tile_cell(513), (17, 0));
        assert_eq!(tile_cell(4095), (127, 31));
    }

    #[test]
    fn a_parsed_level_paints_its_tiles_and_a_higher_layer_covers() {
        // AA is tile 0, AB is tile 1, AC is tile 2. The second layer-0 run
        // on (0, 0) loses to the first. Layer 1 covers (1, 0) only. AQ is
        // tile 16, one row down, so it does not fit this 48×16 sheet.
        let level = Level::parse(
            "GLEVNW01\n\
             BOARD 0 0 1 0 AA\n\
             BOARD 0 0 1 0 AB\n\
             BOARD 1 0 1 0 AB\n\
             BOARD 2 0 1 0 AQ\n\
             BOARD 1 0 1 1 AC\n",
        )
        .unwrap();
        assert_eq!(
            level.tile_id_at(0, 0),
            Some(0),
            "core lookup is the first layer-0 run"
        );
        assert_eq!(level.tile_id_at(1, 0), Some(1));
        let colors = [[200, 10, 10], [10, 180, 10], [10, 10, 220]];
        let w = colors.len() as u32 * TILE_PX;
        let mut px = sheet(&colors);
        // One pixel of tile 0 is not the flat colour, so a blit has to
        // sample the sheet rather than flood the cell.
        let mark = ((5 * w + 3) * 3) as usize;
        px[mark..mark + 3].copy_from_slice(&[255, 255, 255]);

        let mut buf = PixelBuffer::filled(64, 16, [1, 2, 3, 4]);
        draw_level_tiles(&mut buf, &level, Image::rgb(&px, w, TILE_PX), 0, 0);

        assert_eq!(
            buf.pixel(0, 0),
            Some([200, 10, 10, 255]),
            "tile 0, and the later run does not cover it"
        );
        assert_eq!(
            buf.pixel(3, 5),
            Some([255, 255, 255, 255]),
            "the marked pixel of tile 0"
        );
        assert_eq!(buf.pixel(15, 15), Some([200, 10, 10, 255]));
        assert_eq!(
            buf.pixel(16, 0),
            Some([10, 10, 220, 255]),
            "layer 1 covers tile 1"
        );
        assert_eq!(buf.pixel(31, 15), Some([10, 10, 220, 255]));
        assert_eq!(
            buf.pixel(32, 0),
            Some([1, 2, 3, 4]),
            "tile 16 is off this sheet"
        );
        assert_eq!(buf.pixel(48, 0), Some([1, 2, 3, 4]), "past the runs");
    }

    #[test]
    fn the_origin_selects_which_level_pixels_land_in_the_buffer() {
        let level = Level::parse("GLEVNW01\nBOARD 0 0 2 0 AAAB\n").unwrap();
        let px = sheet(&[[200, 10, 10], [10, 180, 10]]);
        let mut buf = PixelBuffer::filled(16, 16, [9, 9, 9, 9]);
        draw_level_tiles(&mut buf, &level, Image::rgb(&px, 32, 16), 16, 0);
        assert_eq!(buf.pixel(0, 0), Some([10, 180, 10, 255]));
        assert_eq!(buf.pixel(15, 15), Some([10, 180, 10, 255]));

        let mut missed = PixelBuffer::filled(8, 8, [9, 9, 9, 9]);
        draw_level_tiles(&mut missed, &level, Image::rgb(&px, 32, 16), -32, 0);
        assert_eq!(
            missed.pixel(0, 0),
            Some([9, 9, 9, 9]),
            "the board starts at pixel 0"
        );
    }

    #[test]
    fn a_core_png_decodes_as_the_tile_sheet() {
        let mut raw = vec![0u8; 16 * 16 * 3];
        raw[0..3].copy_from_slice(&[7, 8, 9]);
        let mut png = Vec::new();
        write_png_rgb(&mut png, 16, 16, &raw).unwrap();
        let (w, h, pixels) = read_png_rgb(&png).unwrap();
        let level = Level::parse("GLEVNW01\nBOARD 0 0 1 0 AA\n").unwrap();
        let mut buf = PixelBuffer::new(16, 16);
        draw_level_tiles(&mut buf, &level, Image::rgb(&pixels, w, h), 0, 0);
        assert_eq!(buf.pixel(0, 0), Some([7, 8, 9, 255]));
        assert_eq!(buf.pixel(1, 0), Some([0, 0, 0, 255]));
    }
}
