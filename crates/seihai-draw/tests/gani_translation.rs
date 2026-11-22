//! Authored public-API GANI translation and clipping controls.
//! Tiny caller-owned images and buffers; no external assets.
use seihai_core::assets::Gani;
use seihai_draw::{draw_gani_frame, Image, PixelBuffer, SpriteSheets};

fn frame(x: i32, y: i32, width: i32, height: i32) -> Gani {
    Gani::parse(&format!(
        "SPRITE 0 HEAD 0 0 {width} {height}\nSINGLEDIRECTION\nANI\n0 {x} {y}\nANIEND\n"
    ))
    .unwrap()
}

#[test]
fn negative_x_translation_does_not_wrap_into_the_destination() {
    let data = [19, 37, 53];
    let sheets = SpriteSheets {
        head: Some(Image::rgb(&data, 1, 1)),
        ..Default::default()
    };
    let mut dst = PixelBuffer::filled(1, 1, [7, 11, 13, 17]);
    draw_gani_frame(
        &mut dst,
        &frame(i32::MIN, 0, 1, 1),
        0,
        0,
        &sheets,
        i32::MIN,
        0,
    );
    assert_eq!(dst.pixel(0, 0), Some([7, 11, 13, 17]));
}

#[test]
fn negative_y_translation_does_not_wrap_into_the_destination() {
    let data = [19, 37, 53];
    let sheets = SpriteSheets {
        head: Some(Image::rgb(&data, 1, 1)),
        ..Default::default()
    };
    let mut dst = PixelBuffer::filled(1, 1, [7, 11, 13, 17]);
    draw_gani_frame(
        &mut dst,
        &frame(0, i32::MIN, 1, 1),
        0,
        0,
        &sheets,
        0,
        i32::MIN,
    );
    assert_eq!(dst.pixel(0, 0), Some([7, 11, 13, 17]));
}

#[test]
fn positive_translation_overflow_remains_offscreen() {
    let data = [19, 37, 53, 71, 89, 107, 127, 149, 173];
    for (x, y, dx, dy, width, height) in [
        (i32::MAX, 0, i32::MAX, 0, 3, 1),
        (0, i32::MAX, 0, i32::MAX, 1, 3),
    ] {
        let sheets = SpriteSheets {
            head: Some(Image::rgb(&data, width as u32, height as u32)),
            ..Default::default()
        };
        let mut dst = PixelBuffer::filled(1, 1, [7, 11, 13, 17]);
        // A wrapping sum is -2; the third source pixel would then paint
        // the destination. The mathematical sum is wholly offscreen.
        draw_gani_frame(&mut dst, &frame(x, y, width, height), 0, 0, &sheets, dx, dy);
        assert_eq!(dst.pixel(0, 0), Some([7, 11, 13, 17]));
    }
}

#[test]
fn translated_partial_sprite_and_visible_pixel_keep_their_source_pixels() {
    let data = [19, 37, 53, 71, 89, 107];
    let sheets = SpriteSheets {
        head: Some(Image::rgb(&data, 2, 1)),
        ..Default::default()
    };
    let mut clipped = PixelBuffer::filled(1, 1, [7, 11, 13, 17]);
    // Huge operands cancel to -1: an origin-only cull would lose the
    // visible second pixel of this legitimate partially clipped sprite.
    draw_gani_frame(
        &mut clipped,
        &frame(i32::MAX, 0, 2, 1),
        0,
        0,
        &sheets,
        i32::MIN,
        0,
    );
    assert_eq!(clipped.pixel(0, 0), Some([71, 89, 107, 255]));
    let mut visible = PixelBuffer::filled(3, 1, [7, 11, 13, 17]);
    draw_gani_frame(&mut visible, &frame(1, 0, 1, 1), 0, 0, &sheets, 1, 0);
    assert_eq!(visible.pixel(2, 0), Some([19, 37, 53, 255]));
    assert_eq!(visible.pixel(1, 0), Some([7, 11, 13, 17]));
}
