//! One sprite rectangle, and one frame of a gani.

use seihai_core::assets::Gani;

use crate::{blit, CopyMode, Image, PixelBuffer};

/// The pictures a gani's `SPRITE` line names (`BODY`, `HEAD`, `SPRITES`,
/// `SWORD`, `SHIELD`, `ATTR1` … `ATTR5`). A sprite whose sheet is not one
/// of these, or whose picture was not supplied, is skipped.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpriteSheets<'a> {
    pub body: Option<Image<'a>>,
    pub head: Option<Image<'a>>,
    pub sprites: Option<Image<'a>>,
    pub sword: Option<Image<'a>>,
    pub shield: Option<Image<'a>>,
    /// Index 0 is `ATTR1`.
    pub attrs: [Option<Image<'a>>; 5],
}

/// Copy the rectangle `(x, y, w, h)` of `src` so its top-left is `(dx, dy)`.
///
/// An RGBA pixel with alpha 0 is left out. An RGB pixel of magenta
/// `(255, 0, 255)` is left out. Any other pixel replaces the destination,
/// with no blend; an RGB source is written opaque. A 4-channel source
/// keeps its alpha.
pub fn draw_sprite(
    dst: &mut PixelBuffer,
    src: Image<'_>,
    src_rect: (i32, i32, i32, i32),
    dx: i32,
    dy: i32,
) {
    blit(
        dst,
        src,
        src_rect,
        (i64::from(dx), i64::from(dy)),
        CopyMode::Sprite,
    );
}

/// Draw one step of `gani` for direction `dir`.
///
/// `dir` is the usual order: 0 up, 1 left, 2 down, 3 right. The row for
/// `dir % 4` is drawn, or the first row when that direction is absent, so
/// a `SINGLEDIRECTION` gani uses its one row for every direction. Placements
/// are drawn in order and a later one replaces an earlier one. The frame
/// index wraps, as [`Gani::frame`] does. `(dx, dy)` is where the gani's
/// origin sits on `dst`.
pub fn draw_gani_frame(
    dst: &mut PixelBuffer,
    gani: &Gani,
    frame: usize,
    dir: u8,
    sheets: &SpriteSheets<'_>,
    dx: i32,
    dy: i32,
) {
    let Some(step) = gani.frame(frame) else {
        return;
    };
    let Some(row) = step
        .rows
        .get(usize::from(dir % 4))
        .or_else(|| step.rows.first())
    else {
        return;
    };
    for place in row {
        let Some(sprite) = gani.sprites.iter().find(|s| s.id == place.sprite_id) else {
            continue;
        };
        let Some(image) = sheet_named(sheets, &sprite.sheet) else {
            continue;
        };
        if sprite.x < 0 || sprite.y < 0 || sprite.w <= 0 || sprite.h <= 0 {
            continue;
        }
        draw_sprite(
            dst,
            image,
            (sprite.x, sprite.y, sprite.w, sprite.h),
            dx + place.x,
            dy + place.y,
        );
    }
}

fn sheet_named<'a>(sheets: &SpriteSheets<'a>, name: &str) -> Option<Image<'a>> {
    match name.to_ascii_uppercase().as_str() {
        "BODY" => sheets.body,
        "HEAD" => sheets.head,
        "SPRITES" => sheets.sprites,
        "SWORD" => sheets.sword,
        "SHIELD" => sheets.shield,
        "ATTR1" => sheets.attrs[0],
        "ATTR2" => sheets.attrs[1],
        "ATTR3" => sheets.attrs[2],
        "ATTR4" => sheets.attrs[3],
        "ATTR5" => sheets.attrs[4],
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seihai_core::assets::Gani;

    fn gani(text: &str) -> Gani {
        Gani::parse(text).unwrap()
    }

    #[test]
    fn a_sprite_skips_clear_and_magenta_and_writes_the_rest() {
        let rgba = [
            255, 0, 0, 255, // (0, 0) opaque red
            0, 0, 0, 0, // (1, 0) clear
            1, 2, 3, 128, // (0, 1) kept alpha, not blended
            9, 9, 9, 255, // (1, 1)
        ];
        let mut buf = PixelBuffer::filled(3, 2, [7, 7, 7, 255]);
        draw_sprite(&mut buf, Image::rgba(&rgba, 2, 2), (0, 0, 2, 2), 1, 0);
        assert_eq!(buf.pixel(0, 0), Some([7, 7, 7, 255]), "left of the blit");
        assert_eq!(buf.pixel(1, 0), Some([255, 0, 0, 255]));
        assert_eq!(buf.pixel(2, 0), Some([7, 7, 7, 255]), "alpha 0 stays");
        assert_eq!(buf.pixel(1, 1), Some([1, 2, 3, 128]));
        assert_eq!(buf.pixel(2, 1), Some([9, 9, 9, 255]));

        let rgb = [255, 0, 255, 4, 5, 6];
        let mut keyed = PixelBuffer::filled(2, 1, [7, 7, 7, 255]);
        draw_sprite(&mut keyed, Image::rgb(&rgb, 2, 1), (0, 0, 2, 1), 0, 0);
        assert_eq!(keyed.pixel(0, 0), Some([7, 7, 7, 255]), "magenta stays");
        assert_eq!(keyed.pixel(1, 0), Some([4, 5, 6, 255]));
    }

    #[test]
    fn a_gani_frame_places_its_sprite_on_the_named_sheet() {
        let g = gani(
            "SPRITE 0 BODY 1 0 1 1\n\
             SINGLEDIRECTION\n\
             ANI\n\
             0 4 2\n\
             ANIEND\n",
        );
        let body = [255, 0, 0, 0, 255, 0];
        let sheets = SpriteSheets {
            body: Some(Image::rgb(&body, 2, 1)),
            ..Default::default()
        };
        let mut buf = PixelBuffer::filled(8, 4, [0, 0, 0, 0]);
        draw_gani_frame(&mut buf, &g, 0, 3, &sheets, 0, 0);
        assert_eq!(
            buf.pixel(4, 2),
            Some([0, 255, 0, 255]),
            "source x=1, placed at (4, 2)"
        );
        assert_eq!(
            buf.pixel(3, 2),
            Some([0, 0, 0, 0]),
            "the red neighbour was not the sprite"
        );
        // The single direction row is used for every direction, and the
        // frame index wraps.
        draw_gani_frame(&mut buf, &g, 9, 0, &sheets, 0, 0);
        assert_eq!(buf.pixel(4, 2), Some([0, 255, 0, 255]));
    }

    #[test]
    fn direction_rows_pick_the_sprite_and_a_later_one_covers() {
        let g = gani(
            "SPRITE 0 BODY 0 0 1 1\n\
             SPRITE 1 BODY 1 0 1 1\n\
             ANI\n\
             0 0 0\n\
             1 0 0\n\
             0 0 0\n\
             1 0 0\n\
             ANIEND\n",
        );
        let body = [200, 0, 0, 0, 180, 0];
        let sheets = SpriteSheets {
            body: Some(Image::rgb(&body, 2, 1)),
            ..Default::default()
        };
        let mut up = PixelBuffer::new(1, 1);
        draw_gani_frame(&mut up, &g, 0, 0, &sheets, 0, 0);
        assert_eq!(
            up.pixel(0, 0),
            Some([200, 0, 0, 255]),
            "dir 0 is the first row"
        );
        let mut left = PixelBuffer::new(1, 1);
        draw_gani_frame(&mut left, &g, 0, 1, &sheets, 0, 0);
        assert_eq!(
            left.pixel(0, 0),
            Some([0, 180, 0, 255]),
            "dir 1 is the second row"
        );

        let cover = gani(
            "SPRITE 0 BODY 0 0 1 1\n\
             SPRITE 1 HEAD 0 0 1 1\n\
             SINGLEDIRECTION\n\
             ANI\n\
             0 0 0, 1 0 0\n\
             ANIEND\n",
        );
        let head = [0, 0, 40];
        let covered = SpriteSheets {
            body: Some(Image::rgb(&body, 2, 1)),
            head: Some(Image::rgb(&head, 1, 1)),
            ..Default::default()
        };
        let mut buf = PixelBuffer::new(1, 1);
        draw_gani_frame(&mut buf, &cover, 0, 2, &covered, 0, 0);
        assert_eq!(buf.pixel(0, 0), Some([0, 0, 40, 255]));
    }

    #[test]
    fn a_missing_sheet_or_a_negative_source_draws_nothing() {
        let g = gani(
            "SPRITE 0 BODY -1 0 1 1\n\
             SPRITE 1 NOPE 0 0 1 1\n\
             SPRITE 2 HEAD 0 0 1 1\n\
             SINGLEDIRECTION\n\
             ANI\n\
             0 0 0, 1 0 0, 2 0 0\n\
             ANIEND\n",
        );
        let head = [1, 2, 3];
        let sheets = SpriteSheets {
            head: Some(Image::rgb(&head, 1, 1)),
            ..Default::default()
        };
        // BODY's source x is negative and NOPE is not a sheet token, so a
        // gani of only those two sprites draws nothing even with HEAD loaded.
        let blank = gani(
            "SPRITE 0 BODY -1 0 1 1\n\
             SPRITE 1 NOPE 0 0 1 1\n\
             SINGLEDIRECTION\n\
             ANI\n\
             0 0 0, 1 0 0\n\
             ANIEND\n",
        );
        let mut buf = PixelBuffer::filled(2, 2, [5, 5, 5, 5]);
        draw_gani_frame(&mut buf, &blank, 0, 0, &sheets, 0, 0);
        assert_eq!(buf.pixel(0, 0), Some([5, 5, 5, 5]));
        // The loaded HEAD sheet does draw.
        draw_gani_frame(&mut buf, &g, 0, 0, &sheets, 1, 1);
        assert_eq!(buf.pixel(1, 1), Some([1, 2, 3, 255]));
        assert_eq!(buf.pixel(0, 0), Some([5, 5, 5, 5]));
    }
}
