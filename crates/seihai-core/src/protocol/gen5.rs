//! Gen-5 partial-XOR cipher (cleaned observation of 2.22+).
//!
//! Same LCG family as gen-3 (`it = it * 0x08088405 + key`, seed `0x04A80B38`).
//! Applied to the compressed body, not to individual messages.
//!
//! On send: compress first, then XOR.
//! On receive: undo XOR first, then decompress.
//!
//! The iterator steps once per 4-byte group actually present in the body,
//! up to a cap that depends on the compression type:
//! - uncompressed (`0x02`): at most 12 groups
//! - zlib (`0x04`) / bz2 (`0x06`): at most 4 groups
//!
//! A partial last group is XORed byte by byte and still steps the iterator.
//! A body shorter than the cap steps fewer times.
//! Login frame is plain zlib with no compression byte and no XOR.
//!
//! 6.x adds a long length: a frame of 0xFFFF bytes or more has `FF FF`
//! where the length goes, then the length as a `u32be` (Graal_analysis
//! `protocol/v6x-clients.md`; the 6.03 receive loop reads it the same way).
//! Only builds with [`FrameLen::Long`] read it; for the others `FF FF` is an
//! ordinary length.

use super::gen3::{GEN3_MUL, GEN3_SEED};

/// Compression type byte for gen-5 frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gen5Comp {
    /// Uncompressed (`0x02`).
    None = 0x02,
    /// zlib (`0x04`).
    Zlib = 0x04,
    /// bz2 (`0x06`).
    Bz2 = 0x06,
}

/// How a build reads a frame's length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameLen {
    /// `u16be` only (2.22 to 5.x).
    Short,
    /// `u16be`, or `FF FF` + `u32be` for 0xFFFF bytes or more (6.x).
    Long,
}

impl FrameLen {
    /// The longest long frame Seihai takes. The 6.x client sets no limit of
    /// its own; this is Seihai's, so that a bad `u32` length can't make it
    /// buffer gigabytes.
    pub const MAX_LONG: usize = 64 << 20;

    /// The header length and the frame length from the start of `buf`, or
    /// `None` until enough bytes are in.
    pub fn header(self, buf: &[u8]) -> Option<(usize, usize)> {
        let short = u16::from_be_bytes([*buf.first()?, *buf.get(1)?]);
        if short != 0xFFFF || self == FrameLen::Short {
            return Some((2, short as usize));
        }
        let long = buf.get(2..6)?;
        Some((6, u32::from_be_bytes([long[0], long[1], long[2], long[3]]) as usize))
    }
}

/// Gen-5 partial-XOR state.
#[derive(Debug, Clone)]
pub struct Gen5 {
    it: u32,
    key: u8,
    /// 5.12's step count on zlib and bz2 bodies (see [`Self::round_down`]).
    round_down: bool,
}

impl Gen5 {
    /// Create a new cipher. `key` is the login key byte.
    pub fn new(key: u8) -> Self {
        Self {
            it: GEN3_SEED,
            key,
            round_down: false,
        }
    }

    /// The same cipher with 5.12's step count: on zlib (`0x04`) and bz2
    /// (`0x06`) bodies the iterator steps once per whole 4-byte group
    /// (`len / 4`, rounded down, still at most 4), and a partial last group
    /// is left as it is. Uncompressed bodies step as in every other build.
    /// Read from 5.12's frame decoder; the lab proxy that let 5.12 play on
    /// GServer-v2 decodes it this way (Graal_analysis
    /// `protocol/v5v6-lab-play.md`).
    pub fn round_down(mut self) -> Self {
        self.round_down = true;
        self
    }

    /// True for 5.12's step count.
    pub fn is_round_down(&self) -> bool {
        self.round_down
    }

    /// Resume from a known iterator state (for replaying captures that
    /// start mid-session).
    pub fn with_state(it: u32, key: u8) -> Self {
        Self { it, key, round_down: false }
    }

    /// Current iterator state.
    pub fn state(&self) -> u32 {
        self.it
    }

    /// XOR `body` in place. `comp` selects the group limit.
    ///
    /// The iterator steps once per 4-byte group that the body actually
    /// has, up to 12 groups for uncompressed (`0x02`) bodies and 4 for
    /// zlib (`0x04`) or bz2 (`0x06`). A partial last group is XORed byte
    /// by byte and still steps the iterator. A short body steps fewer
    /// times: a 7-byte `0x02` body steps twice, not 12 times. The lab
    /// 2.22 captures only chain from frame to frame with this count.
    pub fn xor_body(&mut self, body: &mut [u8], comp: Gen5Comp) {
        let limit = match comp {
            Gen5Comp::None => 12,
            Gen5Comp::Zlib | Gen5Comp::Bz2 => 4,
        };
        let groups = if self.round_down && comp != Gen5Comp::None { body.len() / 4 } else { body.len().div_ceil(4) };
        for chunk in body.chunks_mut(4).take(groups.min(limit)) {
            self.it = self.it.wrapping_mul(GEN3_MUL).wrapping_add(self.key as u32);
            for (b, k) in chunk.iter_mut().zip(self.it.to_le_bytes()) {
                *b ^= k;
            }
        }
    }

    /// Encode a body: XOR then return. Caller supplies the already-compressed
    /// bytes and the compression type that was used.
    pub fn encode(&mut self, mut body: Vec<u8>, comp: Gen5Comp) -> Vec<u8> {
        self.xor_body(&mut body, comp);
        body
    }

    /// Decode a body: undo the XOR. Caller then decompresses.
    pub fn decode(&mut self, mut body: Vec<u8>, comp: Gen5Comp) -> Vec<u8> {
        self.xor_body(&mut body, comp);
        body
    }
}

/// Choose the gen-5 compression type for a body of the given length.
///
/// - ≥ 0x1000 → bz2
/// - ≥ 0x30 → zlib
/// - < 0x30 → uncompressed (caller may still prefer zlib if it is smaller)
pub fn choose_comp(len: usize) -> Gen5Comp {
    if len >= 0x1000 {
        Gen5Comp::Bz2
    } else if len >= 0x30 {
        Gen5Comp::Zlib
    } else {
        Gen5Comp::None
    }
}

/// Write a gen-5 frame: `u16be (1 + body.len())` + comp byte + body.
pub fn write_frame(comp: Gen5Comp, body: &[u8]) -> Vec<u8> {
    let total = 1 + body.len();
    let mut out = Vec::with_capacity(2 + total);
    out.extend_from_slice(&(total as u16).to_be_bytes());
    out.push(comp as u8);
    out.extend_from_slice(body);
    out
}

/// Read one gen-5 frame: `u16be (1+body)` + comp byte + XORed body.
///
/// Returns the decompressed body. `cipher` is advanced.
pub fn read_frame<R: std::io::Read>(
    r: &mut R,
    cipher: &mut Gen5,
) -> std::io::Result<Vec<u8>> {
    read_frame_inner(r, cipher, false, FrameLen::Short)
}

/// As [`read_frame`], with the length read as `len` says.
pub fn read_frame_len<R: std::io::Read>(
    r: &mut R,
    cipher: &mut Gen5,
    len: FrameLen,
) -> std::io::Result<Vec<u8>> {
    read_frame_inner(r, cipher, false, len)
}

/// As [`read_frame`], but a frame whose first byte is the zlib header
/// (`0x78`, never a valid gen-5 compression byte) is read as a plain zlib
/// frame and leaves the cipher alone. For listserver replies: the public
/// listserver on 14923 answers a classic login with plain zlib frames and a
/// gen-5 login with gen-5 frames, and one 2.05 run against it failed with
/// `unknown gen-5 compression byte 0x78` (cause not captured).
pub fn read_frame_or_plain<R: std::io::Read>(
    r: &mut R,
    cipher: &mut Gen5,
) -> std::io::Result<Vec<u8>> {
    read_frame_inner(r, cipher, true, FrameLen::Short)
}

fn read_frame_inner<R: std::io::Read>(
    r: &mut R,
    cipher: &mut Gen5,
    allow_plain: bool,
    len: FrameLen,
) -> std::io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 6];
    r.read_exact(&mut len_buf[..2])?;
    if len.header(&len_buf[..2]).is_none() {
        r.read_exact(&mut len_buf[2..])?;
    }
    let (_, total) = len.header(&len_buf).expect("six bytes are enough");
    if total > FrameLen::MAX_LONG {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("gen-5 frame length {total} is over {}", FrameLen::MAX_LONG),
        ));
    }
    if total == 0 {
        return Ok(Vec::new());
    }
    let mut raw = vec![0u8; total];
    r.read_exact(&mut raw)?;
    let comp = match raw[0] {
        0x02 => Gen5Comp::None,
        0x04 => Gen5Comp::Zlib,
        0x06 => Gen5Comp::Bz2,
        0x78 if allow_plain => {
            return super::zlib_decompress(&raw)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e));
        }
        other => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown gen-5 compression byte {other:#04x}"),
            ))
        }
    };
    let mut body = raw[1..].to_vec();
    cipher.xor_body(&mut body, comp);
    match comp {
        Gen5Comp::None => Ok(body),
        Gen5Comp::Zlib => super::zlib_decompress(&body)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        Gen5Comp::Bz2 => super::bz2_decompress(&body)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_roundtrip() {
        let key = 0x42;
        let mut enc = Gen5::new(key);
        let mut dec = Gen5::new(key);
        let body = b"hello world, this is a test payload for gen5";
        let encoded = enc.encode(body.to_vec(), Gen5Comp::Zlib);
        assert_ne!(encoded, body);
        let decoded = dec.decode(encoded, Gen5Comp::Zlib);
        assert_eq!(decoded, body);
    }

    /// 5.12: a 7-byte zlib body steps once and leaves bytes 4 to 6 alone; a
    /// raw body steps as before.
    #[test]
    fn round_down_on_compressed_only() {
        let mut down = Gen5::new(0x4B).round_down();
        let mut up = Gen5::new(0x4B);
        let mut a = vec![0u8; 7];
        let mut b = vec![0u8; 7];
        down.xor_body(&mut a, Gen5Comp::Zlib);
        up.xor_body(&mut b, Gen5Comp::Zlib);
        assert_eq!(a[..4], b[..4]);
        assert_eq!(a[4..], [0, 0, 0]);
        assert_ne!(down.state(), up.state());
        let mut once = Gen5::new(0x4B);
        once.xor_body(&mut [0u8; 4], Gen5Comp::Zlib);
        assert_eq!(down.state(), once.state());
        // A whole number of groups, or a raw body, steps the same either way.
        let (mut d2, mut u2) = (Gen5::new(9).round_down(), Gen5::new(9));
        d2.xor_body(&mut [0u8; 8], Gen5Comp::Zlib);
        u2.xor_body(&mut [0u8; 8], Gen5Comp::Zlib);
        d2.xor_body(&mut [0u8; 7], Gen5Comp::None);
        u2.xor_body(&mut [0u8; 7], Gen5Comp::None);
        assert_eq!(d2.state(), u2.state());
    }

    #[test]
    fn uncompressed_limit() {
        let mut g = Gen5::new(1);
        let mut body = vec![0u8; 100];
        g.xor_body(&mut body, Gen5Comp::None);
        // 12 groups * 4 = 48 bytes should be touched; rest stay zero.
        assert!(body[..48].iter().any(|&b| b != 0));
        assert!(body[48..].iter().all(|&b| b == 0));
    }

    #[test]
    fn short_body_steps_per_group() {
        let mut g = Gen5::new(1);
        g.xor_body(&mut [0u8; 7], Gen5Comp::None);
        let mut h = Gen5::new(1);
        h.xor_body(&mut [0u8; 8], Gen5Comp::None);
        assert_eq!(g.state(), h.state());
        h.xor_body(&mut [], Gen5Comp::None);
        assert_eq!(g.state(), h.state());
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// Lab 2.22 session, client→server frame 1 (the first frame after the
    /// plain zlib login line). The iterator is still at the seed, and the
    /// next frame's exported starting state follows from two groups.
    #[test]
    fn lab_222_first_client_frame() {
        let raw = hex("02065dbfb1166cc3b487443600a1ca0db3c4609eed04");
        let mut g = Gen5::new(43);
        let mut body = raw[1..].to_vec();
        g.xor_body(&mut body, Gen5Comp::None);
        assert_eq!(&body[..8], b"EEnglish");
        assert_eq!(body[8], b'\n');
        assert_eq!(body[9], 39 + 0x20);
        assert_eq!(*body.last().unwrap(), b'\n');
        assert_eq!(g.state(), 497_901_838);
    }

    /// Lab 2.22 session, server→client frames 2–4: a 7-byte uncompressed
    /// frame between zlib frames. Each frame's end state is the next
    /// frame's exported starting state.
    #[test]
    fn lab_222_server_chain() {
        let frames = [
            "02a124beb5d097",
            "044d003e203aae18995ee1e89e7da02751cec84ccd817192f2532ac12cbbf2a4fcdc2488a89dae5e7a661a97a6825a414e62656a111700c76614f9",
            "0461a02179c7da8f6387a6cd63c16e8e77c8c92c2ec94ccecf4b4d2cc9cc4b4713ccc84cc114cccb5728c82d060b02002a201bfd",
        ];
        let states = [3_340_213_666u32, 2_094_352_182];
        let mut g = Gen5::with_state(194_979_936, 11);
        let mut bodies = Vec::new();
        for (i, f) in frames.iter().enumerate() {
            let mut wire = hex(f);
            let len = wire.len() as u16;
            let mut framed = len.to_be_bytes().to_vec();
            framed.append(&mut wire);
            bodies.push(read_frame(&mut framed.as_slice(), &mut g).unwrap());
            if let Some(s) = states.get(i) {
                assert_eq!(g.state(), *s, "after frame {}", i + 2);
            }
        }
        assert_eq!(bodies[0][0], 42 + 0x20);
        assert!(bodies[1].starts_with(b"Msword1.png\n"));
        assert!(bodies[2].ends_with(b"plisticonno pms.png\n"));
    }

    #[test]
    fn unknown_ctype_is_an_error() {
        let framed = [0u8, 2, 0x09, 0x00];
        let mut g = Gen5::new(1);
        assert!(read_frame(&mut &framed[..], &mut g).is_err());
    }

    /// A 6.x long frame: `FF FF`, the `u32be` length, then the frame. A
    /// 2.22 to 5.x build reads the same `FF FF` as a plain length.
    #[test]
    fn long_frame_length() {
        let body = vec![b'a'; 0x10000];
        let mut framed = vec![0xFF, 0xFF];
        framed.extend(((body.len() + 1) as u32).to_be_bytes());
        framed.push(0x02);
        framed.extend(&body);
        let mut enc = framed.clone();
        Gen5::new(9).xor_body(&mut enc[7..], Gen5Comp::None);
        let got = read_frame_len(&mut enc.as_slice(), &mut Gen5::new(9), FrameLen::Long).unwrap();
        assert_eq!(got, body);

        assert_eq!(FrameLen::Long.header(&framed[..5]), None);
        assert_eq!(FrameLen::Long.header(&framed), Some((6, 0x10001)));
        assert_eq!(FrameLen::Short.header(&framed), Some((2, 0xFFFF)));
        assert_eq!(FrameLen::Long.header(&[0x12, 0x34]), Some((2, 0x1234)));
        assert_eq!(FrameLen::Long.header(&[0x12]), None);

        let mut huge = vec![0xFF, 0xFF];
        huge.extend(u32::MAX.to_be_bytes());
        assert!(read_frame_len(&mut huge.as_slice(), &mut Gen5::new(9), FrameLen::Long).is_err());
    }

    #[test]
    fn frame_header() {
        let frame = write_frame(Gen5Comp::Zlib, b"abc");
        assert_eq!(&frame[..2], &4u16.to_be_bytes());
        assert_eq!(frame[2], 0x04);
        assert_eq!(&frame[3..], b"abc");
    }
}
