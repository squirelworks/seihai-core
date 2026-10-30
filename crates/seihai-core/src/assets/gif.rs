//! GIF reading: the first image of a GIF87a or GIF89a file, as RGBA.
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
pub fn read_gif_rgba_recolored(data: &[u8], recolor: &[(usize, [u8; 3])]) -> io::Result<(u32, u32, Vec<u8>)> {
    if !is_gif(data) || data.len() < 13 {
        return Err(bad("not a GIF"));
    }
    let u16_at = |i: usize| -> io::Result<usize> {
        data.get(i..i + 2).map(|b| usize::from(u16::from_le_bytes([b[0], b[1]]))).ok_or_else(|| bad("truncated GIF"))
    };
    let (sw, sh) = (u16_at(6)?, u16_at(8)?);
    let flags = data[10];
    let mut pos = 13;
    let mut global: &[u8] = &[];
    if flags & 0x80 != 0 {
        let n = 3 << ((flags & 7) + 1);
        global = data.get(pos..pos + n).ok_or_else(|| bad("truncated colour table"))?;
        pos += n;
    }
    let mut transparent: Option<u8> = None;
    loop {
        match *data.get(pos).ok_or_else(|| bad("no image in GIF"))? {
            // Extension: a graphic control block may name the transparent colour.
            0x21 => {
                let label = *data.get(pos + 1).ok_or_else(|| bad("truncated extension"))?;
                pos += 2;
                let mut first = true;
                loop {
                    let n = usize::from(*data.get(pos).ok_or_else(|| bad("truncated extension"))?);
                    if n == 0 {
                        pos += 1;
                        break;
                    }
                    let block = data.get(pos + 1..pos + 1 + n).ok_or_else(|| bad("truncated extension"))?;
                    if first && label == 0xF9 && n >= 4 && block[0] & 1 != 0 {
                        transparent = Some(block[3]);
                    }
                    first = false;
                    pos += 1 + n;
                }
            }
            0x2C => {
                let (ix, iy, iw, ih) = (u16_at(pos + 1)?, u16_at(pos + 3)?, u16_at(pos + 5)?, u16_at(pos + 7)?);
                let iflags = *data.get(pos + 9).ok_or_else(|| bad("truncated image"))?;
                pos += 10;
                let mut table = global;
                if iflags & 0x80 != 0 {
                    let n = 3 << ((iflags & 7) + 1);
                    table = data.get(pos..pos + n).ok_or_else(|| bad("truncated colour table"))?;
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
                    lzw.extend_from_slice(data.get(pos..pos + n).ok_or_else(|| bad("truncated image data"))?);
                    pos += n;
                }
                let indices = lzw_decode(&lzw, min_code, iw * ih)?;
                let (w, h) = (sw.max(ix + iw), sh.max(iy + ih));
                let mut rgba = vec![0u8; w * h * 4];
                let rows: Vec<usize> = if iflags & 0x40 != 0 { interlaced_rows(ih) } else { (0..ih).collect() };
                for (src_row, &y) in rows.iter().enumerate() {
                    for x in 0..iw {
                        let Some(&ci) = indices.get(src_row * iw + x) else { continue };
                        if transparent == Some(ci) {
                            continue;
                        }
                        let c = usize::from(ci) * 3;
                        let Some(rgb) = table.get(c..c + 3) else { continue };
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

/// The image rows in the order an interlaced GIF stores them.
fn interlaced_rows(h: usize) -> Vec<usize> {
    [(0, 8), (4, 8), (2, 4), (1, 2)].iter().flat_map(|&(start, step)| (start..h).step_by(step)).collect()
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
            let Some(&b) = data.get(i) else { return Ok(out) };
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
                let first = if known { suffix[first_of(code, &prefix)] } else { suffix[first_of(p, &prefix)] };
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
        let codes: [(u32, u32); 8] = [(4, 3), (1, 3), (2, 3), (3, 3), (3, 4), (1, 4), (2, 4), (5, 4)];
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
        let (_, _, px) = read_gif_rgba_recolored(&sample(), &[(1, [255, 255, 0]), (3, [9, 9, 9]), (200, [1, 1, 1])]).unwrap();
        assert_eq!(&px[0..4], [255, 255, 0, 255]);
        assert_eq!(&px[4..8], [0, 255, 0, 255]);
        assert_eq!(px[11], 0);
    }

    #[test]
    fn rejects_what_is_not_a_gif() {
        assert!(read_gif_rgba(b"\x89PNG\r\n\x1a\n").is_err());
        assert!(read_gif_rgba(b"GIF89a").is_err());
    }

    #[test]
    fn interlace_order() {
        assert_eq!(interlaced_rows(8), vec![0, 4, 2, 6, 1, 3, 5, 7]);
    }
}
