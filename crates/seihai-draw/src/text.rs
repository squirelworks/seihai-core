//! Text in a TrueType or OpenType font, for GraalShop's TEXT actors and
//! anything else a tool labels.
//!
//! The font is the caller's: this crate ships none. Glyphs are laid out on
//! one baseline per line with the font's advances and kerning, and their
//! coverage is blended over the buffer. Bold, italic, underline, strikeout
//! and a shadow are drawn by hand (a one-pixel second pass, a shear, and
//! lines), so one regular face is enough.

use ab_glyph::{point, Font, PxScale, ScaleFont};

use crate::PixelBuffer;

/// Where each line sits in the text box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// How [`draw_text`] draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    /// Pixel height of the font (ascent to descent).
    pub size: f32,
    pub color: [u8; 4],
    /// A second pass one pixel to the right.
    pub bold: bool,
    /// Leaned right by a fifth of the height.
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
    /// A black copy one pixel right and down, under the text.
    pub shaded: bool,
    pub align: Align,
    /// Wrap at spaces so no line is wider than this many pixels (a word
    /// longer than that stays whole on its own line).
    pub wrap: Option<u32>,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: 16.0,
            color: [0, 0, 0, 255],
            bold: false,
            italic: false,
            underline: false,
            strikeout: false,
            shaded: false,
            align: Align::Left,
            wrap: None,
        }
    }
}

fn line_width<F: Font>(font: &F, scale: PxScale, line: &str) -> f32 {
    let f = font.as_scaled(scale);
    let mut w = 0.0;
    let mut last = None;
    for c in line.chars() {
        let id = f.glyph_id(c);
        if let Some(prev) = last {
            w += f.kern(prev, id);
        }
        w += f.h_advance(id);
        last = Some(id);
    }
    w
}

/// The lines `text` breaks into: at `\n`, then at spaces to fit `wrap`.
fn lines<F: Font>(font: &F, text: &str, style: &TextStyle) -> Vec<String> {
    let scale = PxScale::from(style.size);
    let mut out = Vec::new();
    for para in text.split('\n') {
        let para = para.trim_end_matches('\r');
        let Some(max) = style.wrap else {
            out.push(para.to_string());
            continue;
        };
        let mut cur = String::new();
        for word in para.split(' ') {
            let candidate = if cur.is_empty() {
                word.to_string()
            } else {
                format!("{cur} {word}")
            };
            if !cur.is_empty() && line_width(font, scale, &candidate) > max as f32 {
                out.push(std::mem::replace(&mut cur, word.to_string()));
            } else {
                cur = candidate;
            }
        }
        out.push(cur);
    }
    out
}

/// The width and height of the box [`draw_text`] fills.
pub fn measure_text<F: Font>(font: &F, text: &str, style: &TextStyle) -> (u32, u32) {
    let scale = PxScale::from(style.size);
    let f = font.as_scaled(scale);
    let lines = lines(font, text, style);
    let widest = lines
        .iter()
        .map(|l| line_width(font, scale, l))
        .fold(0.0f32, f32::max);
    let extra = f32::from(u8::from(style.bold) + u8::from(style.shaded))
        + if style.italic { style.size / 5.0 } else { 0.0 };
    let line_h = f.height() + f.line_gap();
    let w = style.wrap.map_or(widest, |m| widest.max(m as f32));
    (
        (w + extra).ceil() as u32,
        (line_h * lines.len() as f32 - f.line_gap() + f32::from(u8::from(style.shaded))).ceil()
            as u32,
    )
}

fn blend(dst: &mut PixelBuffer, x: i32, y: i32, color: [u8; 4], coverage: f32) {
    if x < 0 || y < 0 || x as u32 >= dst.width || y as u32 >= dst.height {
        return;
    }
    let a = coverage.clamp(0.0, 1.0) * f32::from(color[3]) / 255.0;
    if a <= 0.0 {
        return;
    }
    let i = ((y as u32 * dst.width + x as u32) * 4) as usize;
    let px = &mut dst.rgba[i..i + 4];
    let da = f32::from(px[3]) / 255.0;
    let out_a = a + da * (1.0 - a);
    for c in 0..3 {
        let s = f32::from(color[c]);
        let d = f32::from(px[c]);
        px[c] = ((s * a + d * da * (1.0 - a)) / out_a).round() as u8;
    }
    px[3] = (out_a * 255.0).round() as u8;
}

/// Draw `text` with its box's top-left at `(x, y)`; returns the box size
/// ([`measure_text`]). Characters the font lacks draw as its missing glyph.
pub fn draw_text<F: Font>(
    dst: &mut PixelBuffer,
    font: &F,
    text: &str,
    x: i32,
    y: i32,
    style: &TextStyle,
) -> (u32, u32) {
    let size = measure_text(font, text, style);
    if style.shaded {
        let shadow = TextStyle {
            shaded: false,
            color: [0, 0, 0, style.color[3]],
            ..*style
        };
        paint(dst, font, text, x + 1, y + 1, &shadow, size.0);
    }
    paint(dst, font, text, x, y, style, size.0);
    size
}

fn paint<F: Font>(
    dst: &mut PixelBuffer,
    font: &F,
    text: &str,
    x: i32,
    y: i32,
    style: &TextStyle,
    box_w: u32,
) {
    let scale = PxScale::from(style.size);
    let f = font.as_scaled(scale);
    let ascent = f.ascent();
    let line_h = f.height() + f.line_gap();
    let lean = if style.italic { 0.2 } else { 0.0 };
    let passes: &[i32] = if style.bold { &[0, 1] } else { &[0] };
    for (n, line) in lines(font, text, style).iter().enumerate() {
        let w = line_width(font, scale, line);
        let free = box_w as f32 - w - f32::from(u8::from(style.bold));
        let left = x as f32
            + match style.align {
                Align::Left => 0.0,
                Align::Center => (free / 2.0).floor().max(0.0),
                Align::Right => free.max(0.0),
            };
        let baseline = y as f32 + ascent + line_h * n as f32;
        let mut caret = left;
        let mut last = None;
        for c in line.chars() {
            let id = f.glyph_id(c);
            if let Some(prev) = last {
                caret += f.kern(prev, id);
            }
            let glyph = id.with_scale_and_position(scale, point(caret, baseline));
            caret += f.h_advance(id);
            last = Some(id);
            let Some(outline) = font.outline_glyph(glyph) else {
                continue;
            };
            let b = outline.px_bounds();
            outline.draw(|gx, gy, cov| {
                let py = b.min.y + gy as f32;
                let shear = ((baseline - py) * lean).round() as i32;
                for dx in passes {
                    blend(
                        dst,
                        b.min.x as i32 + gx as i32 + shear + dx,
                        py as i32,
                        style.color,
                        cov,
                    );
                }
            });
        }
        let thick = (style.size / 14.0).round().max(1.0) as i32;
        let x0 = left.round() as i32;
        let x1 = (left + w).round() as i32 + i32::from(u8::from(style.bold));
        let mut rule = |at: f32| {
            let ry = at.round() as i32;
            for yy in ry..ry + thick {
                for xx in x0..x1 {
                    blend(dst, xx, yy, style.color, 1.0);
                }
            }
        };
        if style.underline {
            rule(baseline + (style.size / 10.0).max(1.0));
        }
        if style.strikeout {
            rule(baseline - ascent * 0.35);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ab_glyph::{GlyphId, Outline, OutlineCurve, Rect};

    /// A font whose every glyph is a filled square, so a test needs no
    /// font file: advance 10, ascent 8, descent -2 at size 10.
    struct Boxes;

    impl Font for Boxes {
        fn units_per_em(&self) -> Option<f32> {
            Some(10.0)
        }
        fn ascent_unscaled(&self) -> f32 {
            8.0
        }
        fn descent_unscaled(&self) -> f32 {
            -2.0
        }
        fn line_gap_unscaled(&self) -> f32 {
            0.0
        }
        fn italic_angle(&self) -> f32 {
            0.0
        }
        fn glyph_id(&self, c: char) -> GlyphId {
            GlyphId(if c == ' ' { 0 } else { 1 })
        }
        fn h_advance_unscaled(&self, _: GlyphId) -> f32 {
            10.0
        }
        fn h_side_bearing_unscaled(&self, _: GlyphId) -> f32 {
            0.0
        }
        fn v_advance_unscaled(&self, _: GlyphId) -> f32 {
            10.0
        }
        fn v_side_bearing_unscaled(&self, _: GlyphId) -> f32 {
            0.0
        }
        fn kern_unscaled(&self, _: GlyphId, _: GlyphId) -> f32 {
            0.0
        }
        fn outline(&self, id: GlyphId) -> Option<Outline> {
            if id.0 == 0 {
                return None;
            }
            // 2..8 across, 0..6 up from the baseline.
            let p = |x, y| point(x, y);
            let line = OutlineCurve::Line;
            Some(Outline {
                // As the TrueType reader gives it: min is the top-left.
                bounds: Rect {
                    min: p(2.0, 6.0),
                    max: p(8.0, 0.0),
                },
                curves: vec![
                    line(p(2.0, 0.0), p(8.0, 0.0)),
                    line(p(8.0, 0.0), p(8.0, 6.0)),
                    line(p(8.0, 6.0), p(2.0, 6.0)),
                    line(p(2.0, 6.0), p(2.0, 0.0)),
                ],
            })
        }
        fn glyph_count(&self) -> usize {
            2
        }
        fn codepoint_ids(&self) -> ab_glyph::CodepointIdIter<'_> {
            unimplemented!()
        }
        fn glyph_raster_image2(&self, _: GlyphId, _: u16) -> Option<ab_glyph::v2::GlyphImage<'_>> {
            None
        }
        fn glyph_svg_image(&self, _: GlyphId) -> Option<ab_glyph::GlyphSvg<'_>> {
            None
        }
        fn font_data(&self) -> &[u8] {
            &[]
        }
    }

    fn style() -> TextStyle {
        TextStyle {
            size: 10.0,
            color: [255, 0, 0, 255],
            ..TextStyle::default()
        }
    }

    fn red(buf: &PixelBuffer, x: u32, y: u32) -> bool {
        buf.pixel(x, y).is_some_and(|p| p[0] > 200 && p[3] > 200)
    }

    #[test]
    fn draws_glyphs_on_the_baseline_and_measures_the_box() {
        let mut buf = PixelBuffer::new(40, 24);
        let size = draw_text(&mut buf, &Boxes, "a b", 1, 1, &style());
        assert_eq!(size, (30, 10));
        // The first box: x 3..9, y from baseline 9 up 6 (3..9).
        assert!(red(&buf, 3, 3) && red(&buf, 8, 8));
        assert!(!red(&buf, 2, 5) && !red(&buf, 9, 5) && !red(&buf, 5, 2));
        // The space draws nothing; the third character is at 21.
        assert!(!red(&buf, 15, 5) && red(&buf, 23, 5));
    }

    #[test]
    fn wraps_aligns_and_decorates() {
        let wrapped = TextStyle {
            wrap: Some(25),
            align: Align::Right,
            ..style()
        };
        assert_eq!(measure_text(&Boxes, "aa a", &wrapped), (25, 20));
        let mut buf = PixelBuffer::new(30, 24);
        draw_text(&mut buf, &Boxes, "aa a", 0, 0, &wrapped);
        // Line 2 ("a") is pushed right: its box starts at 15 + 2.
        assert!(red(&buf, 18, 14) && !red(&buf, 4, 14));

        let mut buf = PixelBuffer::new(20, 16);
        let under = TextStyle {
            underline: true,
            ..style()
        };
        draw_text(&mut buf, &Boxes, "a", 0, 0, &under);
        assert!(red(&buf, 0, 9), "the rule runs the whole advance");

        let mut buf = PixelBuffer::new(20, 16);
        let shaded = TextStyle {
            shaded: true,
            ..style()
        };
        draw_text(&mut buf, &Boxes, "a", 0, 0, &shaded);
        assert_eq!(buf.pixel(8, 7), Some([0, 0, 0, 255]), "shadow at +1,+1");
        assert!(red(&buf, 7, 7));

        let mut buf = PixelBuffer::new(20, 16);
        let bold = TextStyle {
            bold: true,
            ..style()
        };
        draw_text(&mut buf, &Boxes, "a", 0, 0, &bold);
        assert!(red(&buf, 8, 5), "one pixel wider");
    }
}
