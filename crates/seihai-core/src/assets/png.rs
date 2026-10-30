//! Minimal PNG writer matching the Graal client’s image export routines.
//! Supports 8-bit RGB and indexed (palette) images.
//! Uses the same CRC-32 as the client.

use super::crc32::crc32;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{self, Read, Write};

fn write_chunk<W: Write>(w: &mut W, tag: &[u8; 4], data: &[u8]) -> io::Result<()> {
    let len = data.len() as u32;
    w.write_all(&len.to_be_bytes())?;
    w.write_all(tag)?;
    w.write_all(data)?;
    let mut crc_data = Vec::with_capacity(4 + data.len());
    crc_data.extend_from_slice(tag);
    crc_data.extend_from_slice(data);
    let crc = crc32(&crc_data);
    w.write_all(&crc.to_be_bytes())?;
    Ok(())
}

/// Write an 8-bit RGB PNG (no palette).
/// `pixels` must be width * height * 3 bytes, row-major, no padding.
pub fn write_png_rgb<W: Write>(
    w: &mut W,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> io::Result<()> {
    if pixels.len() != (width as usize) * (height as usize) * 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pixel buffer size mismatch",
        ));
    }

    // Signature
    w.write_all(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])?;

    // IHDR
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(2); // color type: RGB
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // interlace
    write_chunk(w, b"IHDR", &ihdr)?;

    write_chunk(w, b"IDAT", &zlib_rows(pixels, width as usize * 3)?)?;

    // IEND
    write_chunk(w, b"IEND", &[])?;
    Ok(())
}

/// Write an indexed-color PNG (palette).
/// `indices` is width * height bytes; `palette` is RGB triples (up to 256).
pub fn write_png_indexed<W: Write>(
    w: &mut W,
    width: u32,
    height: u32,
    indices: &[u8],
    palette: &[u8],
) -> io::Result<()> {
    if indices.len() != (width as usize) * (height as usize) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "index buffer size mismatch",
        ));
    }
    if palette.len() % 3 != 0 || palette.len() > 256 * 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid palette",
        ));
    }

    w.write_all(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])?;

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8);
    ihdr.push(3); // indexed
    ihdr.push(0);
    ihdr.push(0);
    ihdr.push(0);
    write_chunk(w, b"IHDR", &ihdr)?;

    write_chunk(w, b"PLTE", palette)?;

    write_chunk(w, b"IDAT", &zlib_rows(indices, width as usize)?)?;

    write_chunk(w, b"IEND", &[])?;
    Ok(())
}

/// zlib-compress image rows, each prefixed with filter type 0.
fn zlib_rows(data: &[u8], stride: usize) -> io::Result<Vec<u8>> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    for row in data.chunks(stride.max(1)) {
        enc.write_all(&[0])?;
        enc.write_all(row)?;
    }
    enc.finish()
}

/// Minimal PNG reader.
///
/// Handles non-interlaced grayscale, RGB, palette (1/2/4/8-bit, with
/// `tRNS`), gray+alpha and RGBA images at 8 bits per channel, and all
/// five row filters. Returns `(width, height, pixels)` where `pixels`
/// is row-major RGBA. GIF data is read too (the first image, see
/// [`super::gif`]).
pub fn read_png_rgba(data: &[u8]) -> io::Result<(u32, u32, Vec<u8>)> {
    // A server sends an image as it has it: a `.gif` (or a GIF under a
    // `.png` name) is read here too, so every image path takes both.
    if super::gif::is_gif(data) {
        return super::gif::read_gif_rgba(data);
    }
    read_png_rgba_recolored(data, &[])
}

/// Like [`read_png_rgba`], with some palette entries of a palette PNG
/// replaced first: `(index, rgb)`. Alpha still comes from the image. Other
/// colour types ignore `recolor`. This is how a body picture is drawn in
/// the player's colours (see `looks.rs`).
pub fn read_png_rgba_recolored(data: &[u8], recolor: &[(usize, [u8; 3])]) -> io::Result<(u32, u32, Vec<u8>)> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    if data.len() < 8 || &data[..8] != &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return Err(bad("not a PNG"));
    }
    let mut pos = 8;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut interlace = 0u8;
    let mut palette = Vec::new();
    let mut trns = Vec::new();
    let mut idat = Vec::new();
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let tag = &data[pos + 4..pos + 8];
        let chunk = data.get(pos + 8..pos + 8 + len).ok_or_else(|| bad("truncated chunk"))?;
        if tag == b"IHDR" && chunk.len() >= 13 {
            width = u32::from_be_bytes(chunk[0..4].try_into().unwrap());
            height = u32::from_be_bytes(chunk[4..8].try_into().unwrap());
            bit_depth = chunk[8];
            color_type = chunk[9];
            interlace = chunk[12];
        } else if tag == b"PLTE" {
            palette = chunk.to_vec();
        } else if tag == b"tRNS" {
            trns = chunk.to_vec();
        } else if tag == b"IDAT" {
            idat.extend_from_slice(chunk);
        } else if tag == b"IEND" {
            break;
        }
        pos += 12 + len;
    }
    let channels = match color_type {
        0 => 1,
        2 => 3,
        3 => 1,
        4 => 2,
        6 => 4,
        _ => return Err(bad("unknown PNG color type")),
    };
    let depth_ok = match color_type {
        3 => matches!(bit_depth, 1 | 2 | 4 | 8),
        _ => bit_depth == 8,
    };
    if !depth_ok || interlace != 0 {
        return Err(bad("unsupported PNG bit depth or interlace"));
    }
    if color_type == 3 && palette.is_empty() {
        return Err(bad("palette PNG without PLTE"));
    }
    for &(idx, rgb) in recolor {
        if let Some(entry) = palette.get_mut(idx * 3..idx * 3 + 3) {
            entry.copy_from_slice(&rgb);
        }
    }
    let mut dec = ZlibDecoder::new(&idat[..]);
    let mut raw = Vec::new();
    dec.read_to_end(&mut raw)?;

    // Filters work on whole bytes; sub-byte pixels use a 1-byte step.
    let bpp = (channels * bit_depth as usize).div_ceil(8);
    let stride = (width as usize * channels * bit_depth as usize).div_ceil(8);
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    let mut prev = vec![0u8; stride];
    let mut row = vec![0u8; stride];
    let mut i = 0;
    for _ in 0..height {
        let line = raw.get(i..i + 1 + stride).ok_or_else(|| bad("truncated image data"))?;
        i += 1 + stride;
        let filter = line[0];
        for x in 0..stride {
            let v = line[1 + x];
            let a = if x >= bpp { row[x - bpp] } else { 0 };
            let b = prev[x];
            let c = if x >= bpp { prev[x - bpp] } else { 0 };
            row[x] = match filter {
                0 => v,
                1 => v.wrapping_add(a),
                2 => v.wrapping_add(b),
                3 => v.wrapping_add(((a as u16 + b as u16) / 2) as u8),
                4 => v.wrapping_add(paeth(a, b, c)),
                _ => return Err(bad("unknown PNG filter")),
            };
        }
        if color_type == 3 {
            let per_byte = 8 / bit_depth as usize;
            let mask = ((1u16 << bit_depth) - 1) as u8;
            for x in 0..width as usize {
                let byte = row[x / per_byte];
                let shift = 8 - bit_depth as usize * (x % per_byte + 1);
                let idx = ((byte >> shift) & mask) as usize;
                let rgb = palette.get(idx * 3..idx * 3 + 3).unwrap_or(&[0, 0, 0]);
                pixels.extend_from_slice(rgb);
                pixels.push(trns.get(idx).copied().unwrap_or(255));
            }
        } else {
            for px in row.chunks(channels) {
                match color_type {
                    0 => pixels.extend_from_slice(&[px[0], px[0], px[0], 255]),
                    4 => pixels.extend_from_slice(&[px[0], px[0], px[0], px[1]]),
                    2 => pixels.extend_from_slice(&[px[0], px[1], px[2], 255]),
                    _ => pixels.extend_from_slice(px),
                }
            }
        }
        std::mem::swap(&mut prev, &mut row);
    }
    Ok((width, height, pixels))
}

/// Like [`read_png_rgba`], with alpha dropped. Returns row-major RGB.
pub fn read_png_rgb(data: &[u8]) -> io::Result<(u32, u32, Vec<u8>)> {
    let (w, h, rgba) = read_png_rgba(data)?;
    let rgb = rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    Ok((w, h, rgb))
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let pa = (p - a as i16).abs();
    let pb = (p - b as i16).abs();
    let pc = (p - c as i16).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server's `.gif` goes through the same reader as a PNG (laundry
    /// list P6: such images were never drawn).
    #[test]
    fn gif_data_is_read_too() {
        let gif = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\xff\x00\x00\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;";
        assert_eq!(read_png_rgba(gif).unwrap(), (1, 1, vec![255, 0, 0, 255]));
    }

    #[test]
    fn rgb_roundtrip_header() {
        let pixels = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]; // 2x2
        let mut buf = Vec::new();
        write_png_rgb(&mut buf, 2, 2, &pixels).unwrap();
        assert!(buf.starts_with(&[0x89, b'P', b'N', b'G']));
        assert!(buf.windows(4).any(|w| w == b"IHDR"));
        assert!(buf.windows(4).any(|w| w == b"IEND"));
        let (w, h, back) = read_png_rgb(&buf).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(back, pixels);
    }

    #[test]
    fn indexed_4bit_with_trns() {
        // 3x1, 4-bit palette: indices 0, 1, 2; index 0 transparent.
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&3u32.to_be_bytes());
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.extend_from_slice(&[4, 3, 0, 0, 0]);
        let mut buf = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        write_chunk(&mut buf, b"IHDR", &ihdr).unwrap();
        write_chunk(&mut buf, b"PLTE", &[0, 0, 0, 255, 0, 0, 0, 255, 0]).unwrap();
        write_chunk(&mut buf, b"tRNS", &[0]).unwrap();
        let idat = zlib_rows(&[0x01, 0x20], 2).unwrap();
        write_chunk(&mut buf, b"IDAT", &idat).unwrap();
        write_chunk(&mut buf, b"IEND", &[]).unwrap();
        let (w, h, px) = read_png_rgba(&buf).unwrap();
        assert_eq!((w, h), (3, 1));
        assert_eq!(px, vec![0, 0, 0, 0, 255, 0, 0, 255, 0, 255, 0, 255]);
    }
}
