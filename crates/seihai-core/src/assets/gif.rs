//! GIF reading: the first image of a GIF87a or GIF89a file, as RGBA, and
//! every frame as palette indices for gif2mng-style conversion
//! ([`read_gif_frames`]).
//!
//! Old player worlds name their images `.gif` (a pre-2.1 server asks for
//! `.gif` by default), and a server sends the file as it has it. The
//! client draws the first image only; later frames of an animated GIF are
//! not read. The transparent colour of a graphic control block becomes
//! alpha 0, and so does any pixel the first image does not cover.

use std::io;

fn bad(m: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, m.to_string())
}

/// Is this a GIF file (by its signature)?
pub fn is_gif(data: &[u8]) -> bool {
    data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")
}

/// Decode the first image of a GIF into `(width, height, rgba)` on the
/// logical screen.
pub fn read_gif_rgba(data: &[u8]) -> io::Result<(u32, u32, Vec<u8>)> {
    read_gif_rgba_recolored(data, &[])
}

/// Like [`read_gif_rgba`], with some entries of the image's colour table
/// replaced first: `(index, rgb)`, as for a palette PNG (see
/// `read_png_rgba_recolored`). The transparent index stays clear. An index
/// past the table is left alone.
pub fn read_gif_rgba_recolored(
    data: &[u8],
    recolor: &[(usize, [u8; 3])],
) -> io::Result<(u32, u32, Vec<u8>)> {
    if !is_gif(data) || data.len() < 13 {
        return Err(bad("not a GIF"));
    }
    let u16_at = |i: usize| -> io::Result<usize> {
        data.get(i..i + 2)
            .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
            .ok_or_else(|| bad("truncated GIF"))
    };
    let (sw, sh) = (u16_at(6)?, u16_at(8)?);
    let flags = data[10];
    let mut pos = 13;
    let mut global: &[u8] = &[];
    if flags & 0x80 != 0 {
        let n = 3 << ((flags & 7) + 1);
        global = data
            .get(pos..pos + n)
            .ok_or_else(|| bad("truncated colour table"))?;
        pos += n;
    }
    let mut transparent: Option<u8> = None;
    loop {
        match *data.get(pos).ok_or_else(|| bad("no image in GIF"))? {
            // Extension: a graphic control block may name the transparent colour.
            0x21 => {
                let label = *data
                    .get(pos + 1)
                    .ok_or_else(|| bad("truncated extension"))?;
                pos += 2;
                let mut first = true;
                loop {
                    let n = usize::from(*data.get(pos).ok_or_else(|| bad("truncated extension"))?);
                    if n == 0 {
                        pos += 1;
                        break;
                    }
                    let block = data
                        .get(pos + 1..pos + 1 + n)
                        .ok_or_else(|| bad("truncated extension"))?;
                    if first && label == 0xF9 && n >= 4 && block[0] & 1 != 0 {
                        transparent = Some(block[3]);
                    }
                    first = false;
                    pos += 1 + n;
                }
            }
            0x2C => {
                let (ix, iy, iw, ih) = (
                    u16_at(pos + 1)?,
                    u16_at(pos + 3)?,
                    u16_at(pos + 5)?,
                    u16_at(pos + 7)?,
                );
                let iflags = *data.get(pos + 9).ok_or_else(|| bad("truncated image"))?;
                pos += 10;
                let mut table = global;
                if iflags & 0x80 != 0 {
                    let n = 3 << ((iflags & 7) + 1);
                    table = data
                        .get(pos..pos + n)
                        .ok_or_else(|| bad("truncated colour table"))?;
                    pos += n;
                }
                let mut table = table.to_vec();
                for &(idx, rgb) in recolor {
                    if let Some(entry) = table.get_mut(idx * 3..idx * 3 + 3) {
                        entry.copy_from_slice(&rgb);
                    }
                }
                let min_code = *data.get(pos).ok_or_else(|| bad("truncated image"))?;
                pos += 1;
                let mut lzw = Vec::new();
                loop {
                    let n = usize::from(*data.get(pos).ok_or_else(|| bad("truncated image data"))?);
                    pos += 1;
                    if n == 0 {
                        break;
                    }
                    lzw.extend_from_slice(
                        data.get(pos..pos + n)
                            .ok_or_else(|| bad("truncated image data"))?,
                    );
                    pos += n;
                }
                let (w, h) = (sw.max(ix + iw), sh.max(iy + ih));
                super::png::check_image_size(w as u64, h as u64)?;
                let indices = lzw_decode(&lzw, min_code, iw * ih)?;
                let mut rgba = vec![0u8; w * h * 4];
                let rows: Vec<usize> = if iflags & 0x40 != 0 {
                    interlaced_rows(ih)
                } else {
                    (0..ih).collect()
                };
                for (src_row, &y) in rows.iter().enumerate() {
                    for x in 0..iw {
                        let Some(&ci) = indices.get(src_row * iw + x) else {
                            continue;
                        };
                        if transparent == Some(ci) {
                            continue;
                        }
                        let c = usize::from(ci) * 3;
                        let Some(rgb) = table.get(c..c + 3) else {
                            continue;
                        };
                        let o = ((iy + y) * w + ix + x) * 4;
                        rgba[o..o + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
                    }
                }
                return Ok((w as u32, h as u32, rgba));
            }
            0x3B => return Err(bad("no image in GIF")),
            _ => return Err(bad("unknown GIF block")),
        }
    }
}

/// Every frame of a GIF, for converting it to an MNG ([`super::write_mng`])
/// as gif2mng did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GifFrames {
    /// One frame per image block, each the full logical screen.
    pub frames: Vec<super::IndexedFrame>,
    /// The `NETSCAPE2.0` loop count: `Some(0)` loops for ever, `None` when
    /// the file has no loop block (play once).
    pub loops: Option<u16>,
}

/// Read every image of a GIF as palette indices on the logical screen.
///
/// Each frame starts from the one before as its disposal method leaves it
/// (1 keep, 2 clear, 3 back to the frame before), so a frame that covers
/// only part of the screen still comes out whole. A cleared or uncovered
/// pixel takes the frame's transparent index, or the screen's background
/// index when the frame has none. A frame with its own colour table keeps
/// it; indices carried over from an earlier frame then read through the
/// new table, as a palette player would show them. The delay is in
/// milliseconds (GIF counts hundredths).
pub fn read_gif_frames(data: &[u8]) -> io::Result<GifFrames> {
    if !is_gif(data) || data.len() < 13 {
        return Err(bad("not a GIF"));
    }
    let u16_at = |i: usize| -> io::Result<usize> {
        data.get(i..i + 2)
            .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
            .ok_or_else(|| bad("truncated GIF"))
    };
    let (sw, sh) = (u16_at(6)?, u16_at(8)?);
    super::png::check_image_size(sw as u64, sh as u64)?;
    let flags = data[10];
    let background = data[11];
    let mut pos = 13;
    let mut global: &[u8] = &[];
    if flags & 0x80 != 0 {
        let n = 3 << ((flags & 7) + 1);
        global = data
            .get(pos..pos + n)
            .ok_or_else(|| bad("truncated colour table"))?;
        pos += n;
    }
    let mut out = GifFrames {
        frames: Vec::new(),
        loops: None,
    };
    // The graphic control block for the next image.
    let (mut transparent, mut delay, mut disposal) = (None::<u8>, 0u32, 0u8);
    let mut canvas: Vec<u8> = Vec::new();
    loop {
        match *data.get(pos).ok_or_else(|| bad("truncated GIF"))? {
            0x21 => {
                let label = *data
                    .get(pos + 1)
                    .ok_or_else(|| bad("truncated extension"))?;
                pos += 2;
                let mut blocks: Vec<&[u8]> = Vec::new();
                loop {
                    let n = usize::from(*data.get(pos).ok_or_else(|| bad("truncated extension"))?);
                    if n == 0 {
                        pos += 1;
                        break;
                    }
                    blocks.push(
                        data.get(pos + 1..pos + 1 + n)
                            .ok_or_else(|| bad("truncated extension"))?,
                    );
                    pos += 1 + n;
                }
                match (label, blocks.as_slice()) {
                    (0xF9, [b, ..]) if b.len() >= 4 => {
                        disposal = (b[0] >> 2) & 7;
                        transparent = (b[0] & 1 != 0).then_some(b[3]);
                        delay = u32::from(u16::from_le_bytes([b[1], b[2]])) * 10;
                    }
                    (0xFF, [id, sub, ..])
                        if (*id == b"NETSCAPE2.0" || *id == b"ANIMEXTS1.0")
                            && sub.len() >= 3
                            && sub[0] == 1 =>
                    {
                        out.loops = Some(u16::from_le_bytes([sub[1], sub[2]]));
                    }
                    _ => {}
                }
            }
            0x2C => {
                let (ix, iy, iw, ih) = (
                    u16_at(pos + 1)?,
                    u16_at(pos + 3)?,
                    u16_at(pos + 5)?,
                    u16_at(pos + 7)?,
                );
                let iflags = *data.get(pos + 9).ok_or_else(|| bad("truncated image"))?;
                pos += 10;
                let mut table = global;
                if iflags & 0x80 != 0 {
                    let n = 3 << ((iflags & 7) + 1);
                    table = data
                        .get(pos..pos + n)
                        .ok_or_else(|| bad("truncated colour table"))?;
                    pos += n;
                }
                let min_code = *data.get(pos).ok_or_else(|| bad("truncated image"))?;
                pos += 1;
                let mut lzw = Vec::new();
                loop {
                    let n = usize::from(*data.get(pos).ok_or_else(|| bad("truncated image data"))?);
                    pos += 1;
                    if n == 0 {
                        break;
                    }
                    lzw.extend_from_slice(
                        data.get(pos..pos + n)
                            .ok_or_else(|| bad("truncated image data"))?,
                    );
                    pos += n;
                }
                let blank = transparent.unwrap_or(background);
                if canvas.is_empty() {
                    canvas = vec![blank; sw * sh];
                }
                let before = canvas.clone();
                let indices = lzw_decode(&lzw, min_code, iw * ih)?;
                let rows: Vec<usize> = if iflags & 0x40 != 0 {
                    interlaced_rows(ih)
                } else {
                    (0..ih).collect()
                };
                for (src_row, &y) in rows.iter().enumerate() {
                    for x in 0..iw {
                        let (Some(&ci), true) =
                            (indices.get(src_row * iw + x), ix + x < sw && iy + y < sh)
                        else {
                            continue;
                        };
                        if transparent != Some(ci) {
                            canvas[(iy + y) * sw + ix + x] = ci;
                        }
                    }
                }
                out.frames.push(super::IndexedFrame {
                    width: sw as u32,
                    height: sh as u32,
                    indices: canvas.clone(),
                    palette: table
                        .chunks(3)
                        .filter(|c| c.len() == 3)
                        .map(|c| [c[0], c[1], c[2]])
                        .collect(),
                    transparent,
                    delay_ms: delay,
                });
                match disposal {
                    2 => {
                        for y in iy..(iy + ih).min(sh) {
                            for x in ix..(ix + iw).min(sw) {
                                canvas[y * sw + x] = blank;
                            }
                        }
                    }
                    3 => canvas = before,
                    _ => {}
                }
                (transparent, delay, disposal) = (None, 0, 0);
            }
            0x3B => break,
            _ => return Err(bad("unknown GIF block")),
        }
    }
    if out.frames.is_empty() {
        return Err(bad("no image in GIF"));
    }
    Ok(out)
}

/// The image rows in the order an interlaced GIF stores them.
fn interlaced_rows(h: usize) -> Vec<usize> {
    [(0, 8), (4, 8), (2, 4), (1, 2)]
        .iter()
        .flat_map(|&(start, step)| (start..h).step_by(step))
        .collect()
}

/// GIF's variable-width LZW, least significant bit first. Stops at the end
/// code, at the end of the data, or once `want` indices are out.
fn lzw_decode(data: &[u8], min_code: u8, want: usize) -> io::Result<Vec<u8>> {
    if !(1..=11).contains(&min_code) {
        return Err(bad("bad LZW code size"));
    }
    let clear = 1usize << min_code;
    let end = clear + 1;
    let mut size = u32::from(min_code) + 1;
    // Each code is its prefix code and last byte. Roots are the colours.
    let mut prefix: Vec<u16> = Vec::with_capacity(4096);
    let mut suffix: Vec<u8> = Vec::with_capacity(4096);
    let reset = |prefix: &mut Vec<u16>, suffix: &mut Vec<u8>| {
        prefix.clear();
        suffix.clear();
        for i in 0..clear + 2 {
            prefix.push(u16::MAX);
            suffix.push(i as u8);
        }
    };
    reset(&mut prefix, &mut suffix);
    let mut out = Vec::with_capacity(want);
    let (mut bits, mut nbits, mut i) = (0u32, 0u32, 0usize);
    let mut prev: Option<usize> = None;
    let mut buf = Vec::new();
    while out.len() < want {
        while nbits < size {
            let Some(&b) = data.get(i) else {
                return Ok(out);
            };
            bits |= u32::from(b) << nbits;
            nbits += 8;
            i += 1;
        }
        let code = (bits & ((1 << size) - 1)) as usize;
        bits >>= size;
        nbits -= size;
        if code == clear {
            reset(&mut prefix, &mut suffix);
            size = u32::from(min_code) + 1;
            prev = None;
            continue;
        }
        if code == end {
            break;
        }
        let next = prefix.len();
        let first_of = |c: usize, prefix: &[u16]| {
            let mut c = c;
            while prefix[c] != u16::MAX {
                c = usize::from(prefix[c]);
            }
            c
        };
        let known = code < next;
        if !known && (code != next || prev.is_none()) {
            return Err(bad("bad LZW code"));
        }
        if let Some(p) = prev {
            if next < 4096 {
                let first = if known {
                    suffix[first_of(code, &prefix)]
                } else {
                    suffix[first_of(p, &prefix)]
                };
                prefix.push(p as u16);
                suffix.push(first);
            }
        }
        // Write the string of `code`, back to front.
        buf.clear();
        let mut c = code;
        loop {
            buf.push(suffix[c]);
            if prefix[c] == u16::MAX {
                break;
            }
            c = usize::from(prefix[c]);
        }
        out.extend(buf.iter().rev());
        prev = Some(code);
        if prefix.len() == (1 << size) && size < 12 {
            size += 1;
        }
    }
    out.truncate(want);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3 x 2 GIF89a made by hand: colours black, red, green, blue; blue
    /// is transparent. Pixels: red green blue / blue red green.
    fn sample() -> Vec<u8> {
        let mut g = b"GIF89a".to_vec();
        g.extend([3, 0, 2, 0, 0x81, 0, 0]);
        g.extend([0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]);
        g.extend([0x21, 0xF9, 4, 1, 0, 0, 3, 0]);
        g.extend([0x2C, 0, 0, 0, 0, 3, 0, 2, 0, 0]);
        // LZW, min code 2: clear(4) 1 2 3 3 1 2 end(5), 3-bit codes then 4.
        let codes: [(u32, u32); 8] = [
            (4, 3),
            (1, 3),
            (2, 3),
            (3, 3),
            (3, 4),
            (1, 4),
            (2, 4),
            (5, 4),
        ];
        let (mut acc, mut n, mut bytes) = (0u32, 0u32, Vec::new());
        for (c, s) in codes {
            acc |= c << n;
            n += s;
            while n >= 8 {
                bytes.push(acc as u8);
                acc >>= 8;
                n -= 8;
            }
        }
        if n > 0 {
            bytes.push(acc as u8);
        }
        g.push(2);
        g.push(bytes.len() as u8);
        g.extend(&bytes);
        g.extend([0, 0x3B]);
        g
    }

    #[test]
    fn reads_colours_and_the_transparent_index() {
        let (w, h, px) = read_gif_rgba(&sample()).unwrap();
        assert_eq!((w, h), (3, 2));
        let p = |i: usize| &px[i * 4..i * 4 + 4];
        assert_eq!(p(0), [255, 0, 0, 255]);
        assert_eq!(p(1), [0, 255, 0, 255]);
        assert_eq!(p(2)[3], 0, "blue is transparent");
        assert_eq!(p(3)[3], 0);
        assert_eq!(p(4), [255, 0, 0, 255]);
        assert_eq!(p(5), [0, 255, 0, 255]);
    }

    #[test]
    fn recolour_replaces_colour_table_entries() {
        // Red (1) becomes yellow; the transparent blue (3) stays clear.
        let (_, _, px) = read_gif_rgba_recolored(
            &sample(),
            &[(1, [255, 255, 0]), (3, [9, 9, 9]), (200, [1, 1, 1])],
        )
        .unwrap();
        assert_eq!(&px[0..4], [255, 255, 0, 255]);
        assert_eq!(&px[4..8], [0, 255, 0, 255]);
        assert_eq!(px[11], 0);
    }

    #[test]
    fn rejects_what_is_not_a_gif() {
        assert!(read_gif_rgba(b"\x89PNG\r\n\x1a\n").is_err());
        assert!(read_gif_rgba(b"GIF89a").is_err());
    }

    /// Uncompressed-style LZW for `indices` (min code 2): a clear code
    /// before every pixel keeps the codes 3 bits wide.
    fn lzw(indices: &[u8]) -> Vec<u8> {
        let mut codes: Vec<u32> = Vec::new();
        for &i in indices {
            codes.push(4);
            codes.push(u32::from(i));
        }
        codes.push(5);
        let (mut acc, mut n, mut bytes) = (0u32, 0u32, Vec::new());
        for c in codes {
            acc |= c << n;
            n += 3;
            while n >= 8 {
                bytes.push(acc as u8);
                acc >>= 8;
                n -= 8;
            }
        }
        if n > 0 {
            bytes.push(acc as u8);
        }
        let mut out = vec![2, bytes.len() as u8];
        out.extend(bytes);
        out.push(0);
        out
    }

    /// A 2 x 2 screen, three frames: full red, then green on the top-right
    /// pixel (disposal 2), then blue on the bottom-left; loops for ever.
    fn animation() -> Vec<u8> {
        let mut g = b"GIF89a".to_vec();
        g.extend([2, 0, 2, 0, 0x81, 0, 0]);
        g.extend([0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]);
        g.extend([0x21, 0xFF, 11]);
        g.extend(b"NETSCAPE2.0");
        g.extend([3, 1, 0, 0, 0]);
        // Frame 1: 20/100 s, keep.
        g.extend([0x21, 0xF9, 4, 1 << 2, 20, 0, 0, 0]);
        g.extend([0x2C, 0, 0, 0, 0, 2, 0, 2, 0, 0]);
        g.extend(lzw(&[1, 1, 1, 1]));
        // Frame 2: green at (1, 0), transparent 0, then cleared.
        g.extend([0x21, 0xF9, 4, (2 << 2) | 1, 5, 0, 0, 0]);
        g.extend([0x2C, 1, 0, 0, 0, 1, 0, 1, 0, 0]);
        g.extend(lzw(&[2]));
        // Frame 3: blue at (0, 1).
        g.extend([0x2C, 0, 0, 1, 0, 1, 0, 1, 0, 0]);
        g.extend(lzw(&[3]));
        g.push(0x3B);
        g
    }

    #[test]
    fn reads_every_frame_on_the_screen() {
        let a = read_gif_frames(&animation()).unwrap();
        assert_eq!(a.loops, Some(0));
        let f = &a.frames;
        assert_eq!(f.len(), 3);
        assert_eq!((f[0].width, f[0].height), (2, 2));
        assert_eq!(f[0].indices, [1, 1, 1, 1]);
        assert_eq!(f[0].delay_ms, 200);
        assert_eq!(f[0].palette[1], [255, 0, 0]);
        assert_eq!(f[1].indices, [1, 2, 1, 1]);
        assert_eq!(f[1].transparent, Some(0));
        assert_eq!(f[1].delay_ms, 50);
        // Frame 2 was disposed to the background: its pixel is cleared.
        assert_eq!(f[2].indices, [1, 0, 3, 1]);
        assert_eq!(f[2].transparent, None);
        assert_eq!(f[2].delay_ms, 0);
    }

    #[test]
    fn a_still_gif_is_one_frame_and_no_loop() {
        let a = read_gif_frames(&sample()).unwrap();
        assert_eq!(a.loops, None);
        assert_eq!(a.frames.len(), 1);
        assert_eq!(a.frames[0].indices, [1, 2, 3, 3, 1, 2]);
        assert_eq!(a.frames[0].transparent, Some(3));
        assert!(read_gif_frames(b"GIF89a").is_err());
    }

    #[test]
    fn interlace_order() {
        assert_eq!(interlaced_rows(8), vec![0, 4, 2, 6, 1, 3, 5, 7]);
    }
}
