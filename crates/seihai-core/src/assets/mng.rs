//! MNG reading: the frames of a Graal `.mng` animation, as RGBA, and the
//! clock that plays them on NPC images.
//!
//! Graal's `.mng` files (lamps, glares, emoticons, a few heads and bodies)
//! are made by Graal's own `gif2mng`, and every client from 1.34 to 2.22
//! reads them with its own `TMNGAnimation` unit, not with a real MNG
//! library. That unit is the model here, read from the 2.22 client
//! (`Graal_222.exe`, the loader at 0x58A3E4, the step at 0x58ACEC, the
//! frame draw at 0x58AE60, the tick at 0x585083) and from `gif2mng.exe`,
//! which carries the same loader. It differs from the MNG standard:
//!
//! * **Chunks.** Names are compared in upper case. Only `MHDR`, `FRAM`,
//!   `TERM`, `IHDR`, `PLTE`, `tRNS`, `IDAT`, `IEND` and `MEND` are read;
//!   `DEFI`, `BACK`, `LOOP` and the rest are skipped, so every frame sits at
//!   (0, 0). The `MHDR` simplicity profile is not looked at.
//! * **Frames are palette images**, colour type 3 at bit depth 4 or 8; any
//!   other `IHDR` makes the whole file fail to load ("MNG loading
//!   failed!"). A frame writes its palette indices over the canvas (the
//!   `MHDR` size): nothing is laid over anything, and the framing mode does
//!   not change the picture, only the timing.
//! * **Palette and transparency.** A frame with its own `PLTE` is drawn with
//!   it, else with the file's palette (a `PLTE` outside any frame, or the
//!   first frame's). `tRNS` gives one transparent index, the first entry
//!   that is 0; other alpha values are opaque. A frame without `tRNS` uses
//!   the file's (a `tRNS` outside any frame), else nothing is transparent.
//! * **Timing.** The loader adds the `FRAM` delays up in its own way (see
//!   [`read_mng`]); a frame's time is in `MHDR` ticks (100 a second when
//!   there is no `MHDR`). The client turns it into hundredths of a second,
//!   then into 50 ms game ticks (`/ 5`), and counts a frame down once per
//!   tick, moving on when the count is 0 or less: a frame lasts
//!   `max(1, centiseconds / 5)` ticks. So a delay of 0 shows one tick.
//! * **Looping.** `TERM`'s action is not looked at: its `iteration_max` is.
//!   The client keeps `iteration_max - 1` as a signed count and loops for
//!   ever while it is below 0: N plays N times and stops on the last frame,
//!   while 0, anything from 2^31 + 1 up (gif2mng's old 0xFFFFFFFF), or a
//!   `TERM` too short to hold the count loops for ever. No `TERM`: once.

use std::collections::{HashMap, HashSet};
use std::io;

const SIGNATURE: [u8; 8] = [0x8A, b'M', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
/// The most frames kept from one file. Graal's own MNGs have up to 14; a
/// file from a server claiming thousands would hold a full canvas for each.
pub const MAX_FRAMES: usize = 256;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The client's game tick, which counts MNG frames down.
pub const TICK_MS: u32 = 50;

/// `MHDR` ticks per second when there is no `MHDR` (the client's default).
const DEFAULT_TICKS: u32 = 100;

fn bad(m: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, m.to_string())
}

/// Is this an MNG file (by its signature)?
pub fn is_mng(data: &[u8]) -> bool {
    data.starts_with(&SIGNATURE)
}

/// One frame: the whole canvas, RGBA, and how long it is shown.
#[derive(Debug, Clone, PartialEq)]
pub struct MngFrame {
    pub rgba: Vec<u8>,
    pub delay_ms: u32,
}

/// How often the animation plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MngRepeat {
    /// Loop for ever.
    Forever,
    /// Play this many times, then rest on the last frame.
    Times(u32),
}

/// The frames of an MNG.
#[derive(Debug, Clone, PartialEq)]
pub struct MngAnimation {
    pub width: u32,
    pub height: u32,
    pub frames: Vec<MngFrame>,
    pub repeat: MngRepeat,
    /// The frame shown once a finite animation has played out (the last).
    pub rest_frame: usize,
}

impl MngAnimation {
    /// The time of one pass, in milliseconds.
    pub fn cycle_ms(&self) -> u64 {
        self.frames.iter().map(|f| u64::from(f.delay_ms)).sum()
    }

    /// The frame shown `elapsed_ms` after the animation started.
    pub fn frame_at(&self, elapsed_ms: u64) -> usize {
        let cycle = self.cycle_ms();
        if self.frames.len() < 2 || cycle == 0 {
            return 0;
        }
        if let MngRepeat::Times(n) = self.repeat {
            if elapsed_ms >= cycle * u64::from(n.max(1)) {
                return self.rest_frame;
            }
        }
        let mut t = elapsed_ms % cycle;
        for (i, f) in self.frames.iter().enumerate() {
            if t < u64::from(f.delay_ms) {
                return i;
            }
            t -= u64::from(f.delay_ms);
        }
        self.frames.len() - 1
    }
}

/// How long the client shows a frame of `time` `MHDR` ticks.
pub fn frame_ms(time: i64, ticks_per_second: u32) -> u32 {
    let tps = i64::from(if ticks_per_second == 0 { DEFAULT_TICKS } else { ticks_per_second });
    let centis = time.saturating_mul(100) / tps;
    let ticks = (centis / 5).clamp(1, i64::from(u32::MAX / TICK_MS));
    ticks as u32 * TICK_MS
}

/// One frame as the loader keeps it.
struct Step {
    w: u32,
    h: u32,
    /// Palette indices, one per pixel.
    indices: Vec<u8>,
    palette: Option<Vec<[u8; 3]>>,
    transparent: Option<i32>,
    /// Time in `MHDR` ticks.
    time: i64,
}

/// The first 0 entry of a `tRNS`, or -1.
fn transparent_index(trns: &[u8]) -> i32 {
    trns.iter().position(|&a| a == 0).map_or(-1, |i| i as i32)
}

fn palette_of(plte: &[u8]) -> Vec<[u8; 3]> {
    plte.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()
}

/// Decode a palette PNG stream into its indices: the stream is decoded with
/// a palette whose entry `i` is `(i, 0, 0)`, so red is the index.
fn decode_indices(ihdr_and_idat: &[u8], w: u32, h: u32) -> io::Result<Vec<u8>> {
    let mut png = PNG_SIGNATURE.to_vec();
    png.extend_from_slice(&ihdr_and_idat[..25]);
    let identity: Vec<u8> = (0..=255u8).flat_map(|i| [i, 0, 0]).collect();
    push_chunk(&mut png, b"PLTE", &identity);
    png.extend_from_slice(&ihdr_and_idat[25..]);
    let (pw, ph, rgba) = super::png::read_png_rgba(&png)?;
    if (pw, ph) != (w, h) {
        return Err(bad("MNG frame size"));
    }
    Ok(rgba.chunks_exact(4).map(|p| p[0]).collect())
}

/// Decode an MNG into its frames the way the Graal client does (see the
/// module notes). The palettes can be changed first: `(index, rgb)`, as
/// for a palette PNG (`read_png_rgba_recolored`).
///
/// The loader's timing, from the client: it keeps a current delay `cur`
/// and a default `def` (both 1 at the start), a running time `acc`, and
/// whether delays count per layer (framing mode 2 or 4) or per frame (1 or
/// 3; per frame at the start).
/// * At a `FRAM`: when counting per layer, `acc += cur`; then `cur = def`;
///   the mode is taken; if the delay changes, `cur` is the new delay, and
///   also `def` when the change is for good (2).
/// * At an `IHDR`: when counting per frame, `acc += cur`. The previous frame
///   gets `acc` as its time (before the first frame, `acc` is kept as a lead
///   time), then `acc = 0`.
/// * At `MEND`: the last frame gets the lead time plus `acc`, plus `cur`
///   when counting per layer and a `FRAM` came before the first frame.
///
/// For gif2mng's files (a mode 4 `FRAM`, then a mode 2 `FRAM` with a
/// lasting delay before each frame) this gives each frame the delay of the
/// `FRAM` before it, and the last frame 1 tick more (the lead time).
pub fn read_mng(data: &[u8], recolor: &[(usize, [u8; 3])]) -> io::Result<MngAnimation> {
    if !is_mng(data) {
        return Err(bad("not an MNG"));
    }
    let (mut width, mut height, mut tps) = (0u32, 0u32, DEFAULT_TICKS);
    let (mut cur, mut def, mut acc, mut lead) = (1i64, 1i64, 0i64, 0i64);
    let mut per_layer = false;
    let mut fram_before_first = false;
    let mut steps: Vec<Step> = Vec::new();
    let mut global_palette: Option<Vec<[u8; 3]>> = None;
    let mut global_transparent = -1i32;
    // The frame being read: its `IHDR` and `IDAT` chunks as they came.
    let mut open: Option<(Vec<u8>, Option<Vec<[u8; 3]>>, Option<i32>)> = None;
    let mut pos = 8;
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let tag = data[pos + 4..pos + 8].to_ascii_uppercase();
        let Some(chunk) = data.get(pos + 8..pos + 8 + len) else { break };
        let raw = &data[pos..(pos + 12 + len).min(data.len())];
        pos += 12 + len;
        let be = |at: usize| chunk.get(at..at + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()));
        match &tag[..] {
            b"MHDR" => {
                width = be(0).unwrap_or(0);
                height = be(4).unwrap_or(0);
                tps = be(8).filter(|&t| t != 0).unwrap_or(DEFAULT_TICKS);
                super::png::check_image_size(u64::from(width), u64::from(height))?;
            }
            b"FRAM" => {
                if steps.is_empty() && open.is_none() {
                    fram_before_first = true;
                }
                if per_layer {
                    acc += cur;
                }
                cur = def;
                match chunk.first() {
                    Some(1 | 3) => per_layer = false,
                    Some(2 | 4) => per_layer = true,
                    _ => {}
                }
                // After the mode byte, a name ended by 0; then four change
                // flags and the delay. With no 0 the client reads from 2.
                let at = chunk.iter().skip(1).position(|&b| b == 0).map_or(2, |i| i + 2);
                if len >= at + 8 && chunk[at] > 0 {
                    cur = i64::from(be(at + 4).unwrap_or(0) as i32);
                    if chunk[at] == 2 {
                        def = cur;
                    }
                }
            }
            b"TERM" => {}
            b"IHDR" => {
                if chunk.len() < 13 || chunk[9] != 3 || !matches!(chunk[8], 4 | 8) {
                    return Err(bad("MNG loading failed: a frame that is not a 4 or 8 bit palette image"));
                }
                if !per_layer {
                    acc += cur;
                }
                match steps.last_mut() {
                    Some(s) => s.time = acc,
                    None => lead = acc,
                }
                acc = 0;
                open = Some((raw.to_vec(), None, None));
            }
            b"PLTE" => {
                let p = palette_of(chunk);
                match open.as_mut() {
                    Some(o) => {
                        if global_palette.is_none() {
                            global_palette = Some(p.clone());
                        }
                        o.1 = Some(p);
                    }
                    None => global_palette = Some(p),
                }
            }
            b"TRNS" => match open.as_mut() {
                Some(o) => o.2 = Some(transparent_index(chunk)),
                None => global_transparent = transparent_index(chunk),
            },
            b"IDAT" => {
                if let Some(o) = open.as_mut() {
                    o.0.extend_from_slice(raw);
                }
            }
            b"IEND" => {
                if let Some((mut stream, palette, transparent)) = open.take() {
                    push_chunk(&mut stream, b"IEND", &[]);
                    let w = u32::from_be_bytes(stream[8..12].try_into().unwrap());
                    let h = u32::from_be_bytes(stream[12..16].try_into().unwrap());
                    super::png::check_image_size(u64::from(w), u64::from(h))?;
                    let indices = decode_indices(&stream, w, h)?;
                    if steps.len() >= MAX_FRAMES {
                        break;
                    }
                    steps.push(Step { w, h, indices, palette, transparent, time: 0 });
                }
            }
            b"MEND" => {
                lead += acc;
                if fram_before_first && per_layer {
                    lead += cur;
                }
                if let Some(s) = steps.last_mut() {
                    s.time = lead;
                }
                break;
            }
            _ => {}
        }
    }
    if steps.is_empty() {
        return Err(bad("MNG has no frame"));
    }
    if width == 0 || height == 0 {
        (width, height) = (steps[0].w, steps[0].h);
    }
    let repeat = term_repeat(data);
    // The canvas holds indices; each frame writes its own over it, from (0, 0).
    let mut canvas = vec![0u8; width as usize * height as usize];
    let mut frames = Vec::with_capacity(steps.len());
    for s in &steps {
        for y in 0..s.h.min(height) as usize {
            let row = &s.indices[y * s.w as usize..][..s.w.min(width) as usize];
            canvas[y * width as usize..][..row.len()].copy_from_slice(row);
        }
        let mut palette = s.palette.clone().or_else(|| global_palette.clone()).unwrap_or_default();
        palette.resize(256, [0, 0, 0]);
        for &(i, rgb) in recolor {
            if let Some(p) = palette.get_mut(i) {
                *p = rgb;
            }
        }
        let clear = s.transparent.unwrap_or(global_transparent);
        let rgba = canvas
            .iter()
            .flat_map(|&i| if i32::from(i) == clear { [0, 0, 0, 0] } else { let [r, g, b] = palette[usize::from(i)]; [r, g, b, 255] })
            .collect();
        frames.push(MngFrame { rgba, delay_ms: frame_ms(s.time, tps) });
    }
    let rest_frame = frames.len() - 1;
    Ok(MngAnimation { width, height, frames, repeat, rest_frame })
}

/// The loop count of the file's (last) `TERM`, as the client takes it.
fn term_repeat(data: &[u8]) -> MngRepeat {
    let mut repeat = MngRepeat::Times(1);
    let mut pos = 8;
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        if data[pos + 4..pos + 8].eq_ignore_ascii_case(b"TERM") {
            // The client keeps `iteration_max - 1` as a signed count: below
            // 0 loops for ever, else counts down once per pass. A TERM too
            // short for the field leaves 0 there, so -1.
            let n = data.get(pos + 14..pos + 18).filter(|_| len >= 10).map_or(0, |b| u32::from_be_bytes(b.try_into().unwrap()));
            let left = (n as i32).wrapping_sub(1);
            repeat = if left < 0 { MngRepeat::Forever } else { MngRepeat::Times(left as u32 + 1) };
        }
        if data[pos + 4..pos + 8].eq_ignore_ascii_case(b"MEND") {
            break;
        }
        pos += 12 + len;
    }
    repeat
}

/// The first frame of an MNG as `(width, height, rgba)`, the way every
/// image path reads a picture.
pub fn read_mng_rgba(data: &[u8], recolor: &[(usize, [u8; 3])]) -> io::Result<(u32, u32, Vec<u8>)> {
    let a = read_mng(data, recolor)?;
    let first = a.frames.into_iter().next().ok_or_else(|| bad("MNG has no frame"))?;
    Ok((a.width, a.height, first.rgba))
}

fn push_chunk(out: &mut Vec<u8>, tag: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(body);
    let mut crc = super::crc32::crc32_update(0xFFFF_FFFF, tag);
    crc = super::crc32::crc32_update(crc, body);
    out.extend_from_slice(&(crc ^ 0xFFFF_FFFF).to_be_bytes());
}

/// The MNG images in an image map, played: [`ImageAnimations::update`]
/// puts the frame due now into the map's entry, so everything that draws
/// from the map (NPC images, `showimg`, heads and bodies) animates without
/// knowing about MNG. All animations run on one clock from the moment they
/// are first seen, so two lamps with the same file flicker together, as the
/// real 2.22 client's lamps do on Graal the Adventure.
#[derive(Default)]
pub struct ImageAnimations {
    playing: HashMap<String, Playing>,
    /// `.mng` names already looked at (one frame, or not readable).
    checked: HashSet<String>,
}

struct Playing {
    anim: MngAnimation,
    start_ms: u64,
    shown: usize,
}

impl ImageAnimations {
    pub fn new() -> Self {
        Self::default()
    }

    /// A new copy of `name` arrived: read it again on the next update.
    pub fn forget(&mut self, name: &str) {
        self.playing.remove(name);
        self.checked.remove(name);
    }

    /// How many images are animating.
    pub fn len(&self) -> usize {
        self.playing.len()
    }

    pub fn is_empty(&self) -> bool {
        self.playing.is_empty()
    }

    /// Start the `.mng` entries of `images` not seen before (their bytes
    /// come from `read`), drop the ones no longer in the map, and put each
    /// animation's frame due at `now_ms` into its entry. True when a
    /// picture changed (the frame needs drawing again).
    pub fn update(&mut self, images: &mut HashMap<String, (u32, Vec<u8>)>, now_ms: u64, mut read: impl FnMut(&str) -> Option<Vec<u8>>) -> bool {
        for name in images.keys() {
            if !name.to_ascii_lowercase().ends_with(".mng") || self.checked.contains(name) {
                continue;
            }
            self.checked.insert(name.clone());
            let Some(anim) = read(name).and_then(|d| read_mng(&d, &[]).ok()) else { continue };
            if anim.frames.len() > 1 {
                self.playing.insert(name.clone(), Playing { anim, start_ms: now_ms, shown: usize::MAX });
            }
        }
        self.checked.retain(|n| images.contains_key(n));
        self.playing.retain(|n, _| images.contains_key(n));
        let mut changed = false;
        for (name, p) in self.playing.iter_mut() {
            let i = p.anim.frame_at(now_ms.saturating_sub(p.start_ms));
            if i == p.shown {
                continue;
            }
            let Some(entry) = images.get_mut(name) else { continue };
            // The entry is this file's picture only while it has the
            // animation's size (a recoloured body is kept under another key).
            if entry.0 != p.anim.width || entry.1.len() != p.anim.frames[i].rgba.len() {
                continue;
            }
            entry.1.clone_from(&p.anim.frames[i].rgba);
            p.shown = i;
            changed = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A palette PNG stream of `w` × `h` filled with palette index 0 of
    /// colour `rgb`, without the signature (the chunks only).
    fn png_chunks(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 3, 0, 0, 0]);
        push_chunk(&mut out, b"IHDR", &ihdr);
        push_chunk(&mut out, b"PLTE", &rgb);
        let mut rows = Vec::new();
        for _ in 0..h {
            rows.push(0u8);
            rows.extend(std::iter::repeat(0u8).take(w as usize));
        }
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut z, &rows).unwrap();
        push_chunk(&mut out, b"IDAT", &z.finish().unwrap());
        push_chunk(&mut out, b"IEND", &[]);
        out
    }

    fn fram(mode: u8, change: u8, delay: u32) -> Vec<u8> {
        let mut c = vec![mode, 0, change, 0, 0, 0];
        c.extend_from_slice(&delay.to_be_bytes());
        c
    }

    fn mng(w: u32, h: u32, ticks: u32, body: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = SIGNATURE.to_vec();
        let mut mhdr = Vec::new();
        for v in [w, h, ticks, 0, 0, 0, 1] {
            mhdr.extend_from_slice(&v.to_be_bytes());
        }
        push_chunk(&mut out, b"MHDR", &mhdr);
        for (tag, c) in body {
            if *tag == b"PNG " {
                out.extend_from_slice(c);
            } else {
                push_chunk(&mut out, tag, c);
            }
        }
        push_chunk(&mut out, b"MEND", &[]);
        out
    }

    /// A gif2mng-shaped file: `MHDR`, `FRAM` mode 4, `TERM`, then a `FRAM`
    /// mode 2 with the delay before each frame, and a closing `FRAM`.
    fn gif2mng(w: u32, h: u32, term: Option<u32>, frames: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut body: Vec<(&[u8; 4], Vec<u8>)> = vec![(b"FRAM", vec![4, 0, 0, 0, 0, 0, 0, 0, 0, 0])];
        if let Some(n) = term {
            let mut t = vec![3, 0, 0, 0, 0, 0];
            t.extend_from_slice(&n.to_be_bytes());
            body.push((b"TERM", t));
        }
        for (d, png) in frames {
            body.push((b"FRAM", fram(2, 2, *d)));
            body.push((b"PNG ", png.clone()));
        }
        body.push((b"FRAM", fram(2, 2, 0)));
        mng(w, h, 1000, &body)
    }

    #[test]
    fn gif2mng_timing_follows_the_client() {
        // As bluelampani.mng: 100, 0, 0 ms. The client shows 100, 50, 50:
        // a 0 delay is one 50 ms tick, and the last frame gets the 1-tick
        // lead time plus the closing FRAM's 0.
        let data = gif2mng(2, 2, Some(0xFFFF_FFFF), &[(100, png_chunks(2, 2, [255, 0, 0])), (0, png_chunks(2, 2, [0, 255, 0])), (0, png_chunks(2, 2, [0, 0, 255]))]);
        let a = read_mng(&data, &[]).unwrap();
        assert_eq!((a.width, a.height, a.frames.len()), (2, 2, 3));
        assert_eq!(a.frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>(), vec![100, 50, 50]);
        assert_eq!(&a.frames[0].rgba[..4], &[255, 0, 0, 255]);
        assert_eq!(&a.frames[2].rgba[..4], &[0, 0, 255, 255]);
        assert_eq!(a.repeat, MngRepeat::Forever);
        assert_eq!((a.frame_at(99), a.frame_at(100), a.frame_at(150), a.frame_at(199), a.frame_at(200)), (0, 1, 2, 2, 0));
        assert_eq!(read_mng_rgba(&data, &[]).unwrap(), (2, 2, a.frames[0].rgba.clone()));
    }

    #[test]
    fn delays_round_down_to_whole_ticks() {
        assert_eq!(frame_ms(0, 1000), 50);
        assert_eq!(frame_ms(70, 1000), 50);
        assert_eq!(frame_ms(99, 1000), 50);
        assert_eq!(frame_ms(100, 1000), 100);
        assert_eq!(frame_ms(149, 1000), 100);
        assert_eq!(frame_ms(2250, 1000), 2250);
        assert_eq!(frame_ms(10, 100), 100);
        assert_eq!(frame_ms(1, 0), 50, "no MHDR rate: 100 a second");
    }

    #[test]
    fn term_count_and_action() {
        let f = || vec![(100, png_chunks(1, 1, [1, 1, 1])), (100, png_chunks(1, 1, [2, 2, 2]))];
        let a = read_mng(&gif2mng(1, 1, Some(2), &f()), &[]).unwrap();
        assert_eq!(a.repeat, MngRepeat::Times(2));
        assert_eq!(a.frame_at(150), 1);
        assert_eq!(a.frame_at(250), 0);
        assert_eq!(a.frame_at(100_000), 1);
        assert_eq!(read_mng(&gif2mng(1, 1, Some(0), &f()), &[]).unwrap().repeat, MngRepeat::Forever, "0 loops for ever");
        assert_eq!(read_mng(&gif2mng(1, 1, Some(0x8000_0001), &f()), &[]).unwrap().repeat, MngRepeat::Forever);
        assert_eq!(read_mng(&gif2mng(1, 1, None, &f()), &[]).unwrap().repeat, MngRepeat::Times(1), "no TERM: once");
        // The action byte is not read: action 0 with a count of 3 still loops.
        let mut data = gif2mng(1, 1, Some(3), &f());
        let at = data.windows(4).position(|w| w == b"TERM").unwrap() + 4;
        data[at] = 0;
        assert_eq!(read_mng(&data, &[]).unwrap().repeat, MngRepeat::Times(3));
    }

    #[test]
    fn frames_replace_and_defi_is_ignored() {
        let mut defi = vec![0, 1, 0, 0];
        defi.extend_from_slice(&1i32.to_be_bytes());
        defi.extend_from_slice(&0i32.to_be_bytes());
        let data = mng(
            2,
            1,
            1000,
            &[
                (b"FRAM", fram(1, 2, 100)),
                (b"PNG ", png_chunks(2, 1, [9, 9, 9])),
                (b"DEFI", defi),
                (b"PNG ", png_chunks(1, 1, [7, 7, 7])),
            ],
        );
        let a = read_mng(&data, &[]).unwrap();
        // The second frame, 1 x 1, goes to (0, 0) whatever the DEFI says,
        // and the canvas keeps the first frame's index to its right (drawn
        // with the second frame's palette, which has only index 0).
        assert_eq!(a.frames[1].rgba, vec![7, 7, 7, 255, 7, 7, 7, 255]);
    }

    #[test]
    fn one_transparent_index_and_case_blind_names() {
        // Two-colour frame, tRNS [128, 0]: index 0 (alpha 128) is opaque,
        // index 1 is clear. The chunk names are written in odd case.
        let mut png = Vec::new();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&2u32.to_be_bytes());
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 3, 0, 0, 0]);
        push_chunk(&mut png, b"IHDR", &ihdr);
        push_chunk(&mut png, b"plte", &[10, 20, 30, 40, 50, 60]);
        push_chunk(&mut png, b"tRNS", &[128, 0]);
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut z, &[0, 0, 1]).unwrap();
        push_chunk(&mut png, b"IDAT", &z.finish().unwrap());
        push_chunk(&mut png, b"IEND", &[]);
        let a = read_mng(&mng(2, 1, 1000, &[(b"PNG ", png)]), &[]).unwrap();
        assert_eq!(a.frames[0].rgba, vec![10, 20, 30, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn non_palette_frame_fails_like_the_client() {
        let mut png = Vec::new();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        push_chunk(&mut png, b"IHDR", &ihdr);
        assert!(read_mng(&mng(1, 1, 1000, &[(b"PNG ", png)]), &[]).is_err());
    }

    #[test]
    fn image_animations_swap_the_frame_in_the_map() {
        let data = gif2mng(1, 1, Some(0xFFFF_FFFF), &[(100, png_chunks(1, 1, [10, 0, 0])), (100, png_chunks(1, 1, [20, 0, 0]))]);
        let mut images = HashMap::new();
        images.insert("Lamp.mng".to_string(), read_mng_rgba(&data, &[]).map(|(w, _, px)| (w, px)).unwrap());
        images.insert("still.png".to_string(), (1, vec![1, 2, 3, 255]));
        let mut anims = ImageAnimations::new();
        let reads = std::cell::Cell::new(0);
        let mut read = |_: &str| {
            reads.set(reads.get() + 1);
            Some(data.clone())
        };
        anims.update(&mut images, 1000, &mut read);
        assert_eq!(anims.len(), 1);
        assert_eq!(images["Lamp.mng"].1[0], 10);
        assert!(!anims.update(&mut images, 1050, &mut read));
        assert!(anims.update(&mut images, 1100, &mut read));
        assert_eq!(images["Lamp.mng"].1[0], 20);
        // The last frame: its own 100 plus the 1-tick lead, 101 ms, so two
        // 50 ms ticks.
        assert!(!anims.update(&mut images, 1150, &mut read));
        assert!(anims.update(&mut images, 1200, &mut read));
        assert_eq!(images["Lamp.mng"].1[0], 10);
        assert_eq!(reads.get(), 1, "a name is read once");
        images.remove("Lamp.mng");
        anims.update(&mut images, 1300, &mut read);
        assert!(anims.is_empty());
    }

    /// The real lamp of the 2.22 data folder, when `SEIHAI_DATA_222`
    /// points at it: three 32 × 64 frames, the yellow light growing.
    #[test]
    #[ignore]
    fn real_bluelampani() {
        let Ok(root) = std::env::var("SEIHAI_DATA_222") else { return };
        let data = std::fs::read(std::path::Path::new(&root).join("levels/images/bluelampani.mng")).unwrap();
        let a = read_mng(&data, &[]).unwrap();
        assert_eq!((a.width, a.height, a.frames.len()), (32, 64, 3));
        assert_eq!(a.repeat, MngRepeat::Forever);
        assert_eq!(a.frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>(), vec![100, 50, 50]);
        let yellow = |f: &MngFrame| f.rgba.chunks(4).filter(|p| p[0] > 200 && p[1] > 200 && p[2] < 80).count();
        assert!(yellow(&a.frames[0]) < yellow(&a.frames[1]) && yellow(&a.frames[1]) < yellow(&a.frames[2]));
    }
}
