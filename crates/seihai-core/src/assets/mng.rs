//! MNG reading: the frames of a Graal `.mng` animation, as RGBA, and the
//! clock that plays them on NPC images.
//!
//! Graal's `.mng` files are short looping animations (lamps, glares,
//! emoticons, a few heads and bodies) made by Graal's `gif2mng` tool. They
//! use a small part of MNG: an `MHDR`, `FRAM` chunks that set the framing
//! mode and the delay, a `TERM` that says how often to loop, and one plain
//! PNG stream (`IHDR` … `IEND`) per frame. This reader takes that part:
//!
//! * each PNG stream is one frame, drawn at the `DEFI` position (0, 0 when
//!   there is none) on a canvas of the `MHDR` size;
//! * framing mode 3 or 4 clears the canvas to transparent at the `FRAM`;
//!   modes 1 and 2 draw the next image over what is there;
//! * a frame lasts the delay its `FRAM` set (one-shot or default, in
//!   `MHDR` ticks). A delay of 0 is shown for [`ZERO_DELAY_MS`];
//! * `TERM` action 3 repeats `iteration_max` times (`0x7FFFFFFF` and up is
//!   for ever), then rests on the last frame; any other `TERM`, or none,
//!   plays once and rests on the last frame (action 2: the first).
//!
//! `LOOP`/`ENDL`, `JNG`, `BASI`, delta images and the rest of MNG are not
//! read: a file that needs them still gives the frames it has.

use std::collections::{HashMap, HashSet};
use std::io;

const SIGNATURE: [u8; 8] = [0x8A, b'M', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
/// The most frames kept from one file. Graal's own MNGs have up to a
/// dozen; a file from a server claiming thousands would hold a full canvas
/// for each.
pub const MAX_FRAMES: usize = 256;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// How long a frame whose delay is 0 is shown. The 2.22 lamp
/// `bluelampani.mng` stores 100, 0 and 0 ticks for its three frames, and
/// screenshots of the real 2.22 client on Graal the Adventure caught the
/// lamp on its first and on its last frame, so a 0 frame is shown for a
/// visible time. 100 ms (what GIF viewers use for a 0 delay, and Seihai's
/// GANI step) is Seihai's choice; the client's own value was not read.
pub const ZERO_DELAY_MS: u32 = 100;

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
    /// Play this many times, then rest on [`MngAnimation::rest_frame`].
    Times(u32),
}

/// The frames of an MNG.
#[derive(Debug, Clone, PartialEq)]
pub struct MngAnimation {
    pub width: u32,
    pub height: u32,
    pub frames: Vec<MngFrame>,
    pub repeat: MngRepeat,
    /// The frame shown once a finite animation has played out.
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

/// Decode an MNG into its frames. The palette of each frame can be changed
/// first, as for a palette PNG (`read_png_rgba_recolored`).
pub fn read_mng(data: &[u8], recolor: &[(usize, [u8; 3])]) -> io::Result<MngAnimation> {
    if !is_mng(data) {
        return Err(bad("not an MNG"));
    }
    let mut pos = 8;
    let (mut width, mut height, mut ticks) = (0u32, 0u32, 1000u32);
    // The framing mode (1 to 4) and the delays, in ticks.
    let mut mode = 1u8;
    let mut default_delay = 1u32;
    let mut frame_delay = 1u32;
    let mut at = (0i32, 0i32);
    let mut canvas: Vec<u8> = Vec::new();
    let mut frames = Vec::new();
    let mut repeat = MngRepeat::Times(1);
    let mut rest_first = false;
    // A palette given outside any image, for images that have none.
    let mut global_plte: Option<Vec<u8>> = None;
    let mut global_trns: Option<Vec<u8>> = None;
    // The PNG stream being collected, and whether it had its own palette.
    let mut png: Option<(Vec<u8>, bool, bool)> = None;
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let tag: [u8; 4] = data[pos + 4..pos + 8].try_into().unwrap();
        let Some(chunk) = data.get(pos + 8..pos + 8 + len) else { break };
        let raw = &data[pos..(pos + 12 + len).min(data.len())];
        pos += 12 + len;
        if let Some((stream, has_plte, has_trns)) = png.as_mut() {
            if &tag == b"PLTE" {
                *has_plte = true;
            } else if &tag == b"tRNS" {
                *has_trns = true;
            } else if &tag == b"IDAT" && !*has_plte {
                // An image that uses the MNG's own palette: put it in
                // before the first image data.
                if let Some(p) = &global_plte {
                    push_chunk(stream, b"PLTE", p);
                    *has_plte = true;
                    if let (Some(t), false) = (&global_trns, *has_trns) {
                        push_chunk(stream, b"tRNS", t);
                        *has_trns = true;
                    }
                }
            }
            stream.extend_from_slice(raw);
            if &tag == b"IEND" {
                let (stream, _, _) = png.take().unwrap();
                if let Ok((w, h, px)) = super::png::read_png_rgba_recolored(&stream, recolor) {
                    if width == 0 || height == 0 {
                        (width, height) = (w, h);
                    }
                    if frames.len() >= MAX_FRAMES {
                        break;
                    }
                    if canvas.len() != (width * height * 4) as usize {
                        canvas = vec![0; (width * height * 4) as usize];
                    }
                    draw_over(&mut canvas, width, height, &px, w, h, at);
                    frames.push(MngFrame { rgba: canvas.clone(), delay_ms: delay_ms(frame_delay, ticks) });
                }
            }
            continue;
        }
        match &tag {
            b"MHDR" if chunk.len() >= 12 => {
                width = u32::from_be_bytes(chunk[0..4].try_into().unwrap());
                height = u32::from_be_bytes(chunk[4..8].try_into().unwrap());
                ticks = u32::from_be_bytes(chunk[8..12].try_into().unwrap());
                if ticks == 0 {
                    ticks = 1000;
                }
                // The size is the file's own claim: refuse a huge one before
                // allocating (0 x 0 means "take the first image's size").
                if width != 0 || height != 0 {
                    super::png::check_image_size(u64::from(width), u64::from(height))?;
                }
                canvas = vec![0; (width as usize) * (height as usize) * 4];
            }
            b"FRAM" => {
                if let Some(&m) = chunk.first() {
                    if (1..=4).contains(&m) {
                        mode = m;
                    }
                }
                frame_delay = default_delay;
                // After the mode: a name ending in 0, then four change
                // flags, then the delay if it changes.
                if chunk.len() > 1 {
                    let name_end = chunk[1..].iter().position(|&b| b == 0).map(|i| i + 1);
                    if let Some(e) = name_end {
                        let flags = &chunk[e + 1..];
                        if flags.len() >= 8 && flags[0] != 0 {
                            let d = u32::from_be_bytes(flags[4..8].try_into().unwrap());
                            frame_delay = d;
                            if flags[0] == 2 {
                                default_delay = d;
                            }
                        }
                    }
                }
                if matches!(mode, 3 | 4) {
                    canvas.iter_mut().for_each(|b| *b = 0);
                }
            }
            b"DEFI" if chunk.len() >= 12 => {
                at = (i32::from_be_bytes(chunk[4..8].try_into().unwrap()), i32::from_be_bytes(chunk[8..12].try_into().unwrap()));
            }
            b"DEFI" => at = (0, 0),
            b"TERM" => {
                let action = chunk.first().copied().unwrap_or(0);
                repeat = if action == 3 && chunk.len() >= 10 {
                    let n = u32::from_be_bytes(chunk[6..10].try_into().unwrap());
                    if n >= 0x7FFF_FFFF { MngRepeat::Forever } else { MngRepeat::Times(n.max(1)) }
                } else {
                    MngRepeat::Times(1)
                };
                rest_first = action == 2;
            }
            b"PLTE" => global_plte = Some(chunk.to_vec()),
            b"tRNS" => global_trns = Some(chunk.to_vec()),
            b"IHDR" => {
                let mut stream = PNG_SIGNATURE.to_vec();
                stream.extend_from_slice(raw);
                png = Some((stream, false, false));
            }
            b"MEND" => break,
            _ => {}
        }
    }
    if frames.is_empty() {
        return Err(bad("MNG has no frame"));
    }
    let rest_frame = if rest_first { 0 } else { frames.len() - 1 };
    Ok(MngAnimation { width, height, frames, repeat, rest_frame })
}

/// The first frame of an MNG as `(width, height, rgba)`, the way every
/// image path reads a picture.
pub fn read_mng_rgba(data: &[u8], recolor: &[(usize, [u8; 3])]) -> io::Result<(u32, u32, Vec<u8>)> {
    let a = read_mng(data, recolor)?;
    let first = a.frames.into_iter().next().ok_or_else(|| bad("MNG has no frame"))?;
    Ok((a.width, a.height, first.rgba))
}

fn delay_ms(d: u32, ticks: u32) -> u32 {
    let ms = (u64::from(d) * 1000 / u64::from(ticks.max(1))).min(u64::from(u32::MAX)) as u32;
    if ms == 0 { ZERO_DELAY_MS } else { ms }
}

fn push_chunk(out: &mut Vec<u8>, tag: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(body);
    let mut crc = super::crc32::crc32_update(0xFFFF_FFFF, tag);
    crc = super::crc32::crc32_update(crc, body);
    out.extend_from_slice(&(crc ^ 0xFFFF_FFFF).to_be_bytes());
}

/// Lay `src` (`sw` × `sh`) over the canvas with its top-left at `at`.
fn draw_over(canvas: &mut [u8], cw: u32, ch: u32, src: &[u8], sw: u32, sh: u32, at: (i32, i32)) {
    for y in 0..sh as i32 {
        let cy = y + at.1;
        if cy < 0 || cy >= ch as i32 {
            continue;
        }
        for x in 0..sw as i32 {
            let cx = x + at.0;
            if cx < 0 || cx >= cw as i32 {
                continue;
            }
            let si = ((y as u32 * sw + x as u32) * 4) as usize;
            let di = ((cy as u32 * cw + cx as u32) * 4) as usize;
            let a = u32::from(src[si + 3]);
            if a == 255 {
                canvas[di..di + 4].copy_from_slice(&src[si..si + 4]);
            } else if a > 0 {
                let da = u32::from(canvas[di + 3]);
                let out_a = a + da * (255 - a) / 255;
                for k in 0..3 {
                    let v = (u32::from(src[si + k]) * a + u32::from(canvas[di + k]) * da * (255 - a) / 255) / out_a.max(1);
                    canvas[di + k] = v.min(255) as u8;
                }
                canvas[di + 3] = out_a.min(255) as u8;
            }
        }
    }
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

    #[test]
    fn frames_delays_and_forever_loop() {
        let term = vec![3, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];
        let data = mng(
            2,
            2,
            1000,
            &[
                (b"TERM", term),
                (b"FRAM", fram(2, 2, 100)),
                (b"PNG ", png_chunks(2, 2, [255, 0, 0])),
                (b"FRAM", fram(2, 2, 0)),
                (b"PNG ", png_chunks(2, 2, [0, 255, 0])),
                (b"FRAM", fram(2, 2, 250)),
                (b"PNG ", png_chunks(2, 2, [0, 0, 255])),
                (b"FRAM", fram(2, 2, 0)),
            ],
        );
        let a = read_mng(&data, &[]).unwrap();
        assert_eq!((a.width, a.height, a.frames.len()), (2, 2, 3));
        assert_eq!(a.frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>(), vec![100, ZERO_DELAY_MS, 250]);
        assert_eq!(&a.frames[0].rgba[..4], &[255, 0, 0, 255]);
        assert_eq!(&a.frames[2].rgba[..4], &[0, 0, 255, 255]);
        assert_eq!(a.repeat, MngRepeat::Forever);
        assert_eq!(a.frame_at(0), 0);
        assert_eq!(a.frame_at(99), 0);
        assert_eq!(a.frame_at(100), 1);
        assert_eq!(a.frame_at(200), 2);
        assert_eq!(a.frame_at(449), 2);
        assert_eq!(a.frame_at(450), 0);
        assert_eq!(read_mng_rgba(&data, &[]).unwrap(), (2, 2, a.frames[0].rgba.clone()));
    }

    #[test]
    fn finite_loop_rests_on_the_last_frame() {
        let term = vec![3, 0, 0, 0, 0, 0, 0, 0, 0, 2];
        let data = mng(
            1,
            1,
            1000,
            &[(b"TERM", term), (b"FRAM", fram(2, 2, 100)), (b"PNG ", png_chunks(1, 1, [1, 1, 1])), (b"PNG ", png_chunks(1, 1, [2, 2, 2]))],
        );
        let a = read_mng(&data, &[]).unwrap();
        assert_eq!(a.repeat, MngRepeat::Times(2));
        assert_eq!(a.frame_at(150), 1);
        assert_eq!(a.frame_at(250), 0);
        assert_eq!(a.frame_at(400), 1);
        assert_eq!(a.frame_at(100_000), 1);
    }

    #[test]
    fn mode_four_clears_and_defi_places() {
        let mut defi = vec![0, 1, 0, 0];
        defi.extend_from_slice(&1i32.to_be_bytes());
        defi.extend_from_slice(&0i32.to_be_bytes());
        let data = mng(
            2,
            1,
            1000,
            &[
                (b"FRAM", fram(1, 2, 10)),
                (b"PNG ", png_chunks(1, 1, [9, 9, 9])),
                (b"DEFI", defi),
                (b"PNG ", png_chunks(1, 1, [7, 7, 7])),
                (b"FRAM", fram(4, 0, 0)),
                (b"PNG ", png_chunks(1, 1, [5, 5, 5])),
            ],
        );
        let a = read_mng(&data, &[]).unwrap();
        assert_eq!(a.frames.len(), 3);
        // Mode 1: the second image goes over the first, at x 1.
        assert_eq!(a.frames[1].rgba, vec![9, 9, 9, 255, 7, 7, 7, 255]);
        // Mode 4 cleared the canvas; the DEFI place still holds.
        assert_eq!(a.frames[2].rgba, vec![0, 0, 0, 0, 5, 5, 5, 255]);
    }

    #[test]
    fn image_animations_swap_the_frame_in_the_map() {
        let data = mng(
            1,
            1,
            1000,
            &[
                (b"TERM", vec![3, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]),
                (b"FRAM", fram(2, 2, 100)),
                (b"PNG ", png_chunks(1, 1, [10, 0, 0])),
                (b"PNG ", png_chunks(1, 1, [20, 0, 0])),
            ],
        );
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
        assert_eq!(a.frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>(), vec![100, ZERO_DELAY_MS, ZERO_DELAY_MS]);
        let yellow = |f: &MngFrame| f.rgba.chunks(4).filter(|p| p[0] > 200 && p[1] > 200 && p[2] < 80).count();
        assert!(yellow(&a.frames[0]) < yellow(&a.frames[1]) && yellow(&a.frames[1]) < yellow(&a.frames[2]));
    }
}
