//! `PLI_TAMPERCHECK` (client to server id 39), sent by 2.17, 2.22 and 2.31.
//!
//! Not to be mixed up with `PLO_LEVELMODTIME`, the server's own id 39.
//!
//! What the payload is (Graal_analysis `client-internals/feature-specs/08-tampercheck.md`
//! and `decomp-findings-2026-10-24-b/01-tamper-check-id39.md`; the six lab
//! values below were reproduced from the two CRC words):
//!
//! - 10 raw bytes: word A then word B, each as 5 bytes in a form that is
//!   **not** GINT5 and not masked, see [`encode_word`].
//! - Each word is a CRC-32 (reflected, polynomial `0xEDB88320`) over ranges of
//!   the client's own memory image, see [`crc_chain`]. Nothing in it depends
//!   on the account, login key, level or time, so for an unmodified build it
//!   is one constant.
//!
//! Seihai is not a Graal executable and has no such image, so the image is
//! pluggable ([`TamperImage`]). The default is the build's constant words
//! (what the real client sends), kept in the version profile. Nothing here
//! sends by itself: the caller opts in (`--send-tampercheck`), because
//! GServer-v2 ignores the packet and four public 2.22 servers did not kick a
//! client that never sent it for 20 s.
//!
//! When the real clients send it again:
//!
//! | build | login | later |
//! |---|---|---|
//! | 2.17 | yes | never |
//! | 2.22 | yes | with every client-initiated link request and gmap edge crossing, not after a server-side warp |
//! | 2.31 | yes | every 3.2 s: a 50 ms timer counts modulo 64 and sends at count 31 |
//!
//! 2.3 (no binary) would send it too, but its constant is unknown, so no
//! value is carried and nothing is sent for it.

use super::version::ClientVersion;
use crate::assets::crc32_update;
use std::time::Duration;

/// Client to server id.
pub const PLI_TAMPERCHECK: u8 = 39;

/// Period of the 2.31 main-form timer.
pub const TICK_MS: u32 = 50;
/// The 2.31 counter wraps at 64 (the length of a 64-character alphabet the
/// timer handler uses), so one cycle is 64 x 50 ms = 3.2 s.
pub const TICKS_PER_CYCLE: u8 = 64;
/// Counter value at which 2.31 sends. The other phases are the anti-debug
/// probe (1), CRC word A (10), the window scan (20) and CRC word B (30);
/// none of them changes what is sent.
pub const SEND_PHASE: u8 = 31;

/// The two CRC words of a build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TamperWords {
    pub a: u32,
    pub b: u32,
}

/// When a build sends id 39 again after login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TamperResend {
    /// Login only (2.17).
    Never,
    /// Every level change (2.22; seen on each gmap edge crossing).
    OnLevelChange,
    /// Every [`TICKS_PER_CYCLE`] ticks of [`TICK_MS`] (2.31).
    Timer,
}

/// What a profile records about id 39.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TamperInfo {
    /// The build's constant words, or `None` where they were never
    /// captured (2.3).
    pub words: Option<TamperWords>,
    pub resend: TamperResend,
}

/// Write one word the way the clients do. Bytes 1 and 5 are range-limited;
/// bytes 2 to 4 are `((v >> n) + 0x20) & 0xFF` and so can be below 0x20 or
/// above 0x7F.
pub fn encode_word(v: u32) -> [u8; 5] {
    [
        (((v >> 28) & 0x0F) as u8) + 0x20,
        ((v >> 21) as u8).wrapping_add(0x20),
        ((v >> 14) as u8).wrapping_add(0x20),
        ((v >> 7) as u8).wrapping_add(0x20),
        ((v & 0x7F) as u8) + 0x20,
    ]
}

/// Read a word written by [`encode_word`]. Each byte carries 7 bits (byte 1
/// carries 4), the rest is redundant.
pub fn decode_word(b: [u8; 5]) -> u32 {
    let seven = |x: u8| (x.wrapping_sub(0x20) & 0x7F) as u32;
    ((b[0].wrapping_sub(0x20) & 0x0F) as u32) << 28
        | seven(b[1]) << 21
        | seven(b[2]) << 14
        | seven(b[3]) << 7
        | seven(b[4])
}

/// The 10 payload bytes: word A then word B.
pub fn encode_words(w: TamperWords) -> [u8; 10] {
    let mut out = [0u8; 10];
    out[..5].copy_from_slice(&encode_word(w.a));
    out[5..].copy_from_slice(&encode_word(w.b));
    out
}

/// One word from the ranges of an image, in the order the client covers them.
///
/// Reflected CRC-32 (start `0xFFFFFFFF`). The first range finishes with the
/// usual final inversion and that result is the register the next range
/// continues from; no later call inverts again, so the word comes out
/// without a final inversion. With one range this is `!crc32(range)`'s
/// pre-inversion register, and with none it is `0xFFFFFFFF`. Verified on the
/// 2.17.1 and 2.31 images (all four words).
pub fn crc_chain(ranges: &[&[u8]]) -> u32 {
    let mut reg = 0xFFFF_FFFFu32;
    for (i, r) in ranges.iter().enumerate() {
        reg = crc32_update(reg, r);
        if i == 0 {
            reg ^= 0xFFFF_FFFF;
        }
    }
    reg
}

/// Where the two words come from. The real client reads its own memory;
/// anything that can produce the words can stand in.
pub trait TamperImage {
    /// Words A and B, or `None` if this source has none.
    fn words(&self) -> Option<TamperWords>;
}

/// Fixed words, e.g. a profile's constant.
impl TamperImage for TamperWords {
    fn words(&self) -> Option<TamperWords> {
        Some(*self)
    }
}

/// An image given as the byte ranges each word covers. Word A takes 8 or 9
/// contiguous code ranges, word B one large code range and one block of
/// static data; where those ranges lie is per executable, so the caller
/// supplies them.
#[derive(Debug, Clone, Default)]
pub struct RangeImage {
    pub a: Vec<Vec<u8>>,
    pub b: Vec<Vec<u8>>,
}

impl TamperImage for RangeImage {
    fn words(&self) -> Option<TamperWords> {
        if self.a.is_empty() || self.b.is_empty() {
            return None;
        }
        let word = |rs: &[Vec<u8>]| crc_chain(&rs.iter().map(|r| r.as_slice()).collect::<Vec<_>>());
        Some(TamperWords { a: word(&self.a), b: word(&self.b) })
    }
}

/// Builds the id 39 message and says when to send it again.
#[derive(Debug, Clone)]
pub struct TamperSender {
    payload: [u8; 10],
    resend: TamperResend,
    /// The 2.31 timer counter (0 to 63).
    counter: u8,
    /// Time carried over between [`Self::advance`] calls (less than one tick).
    carry: Duration,
}

impl TamperSender {
    /// A sender for `version` using `image` for the words. `None` if the
    /// version does not send id 39, or the image has no words.
    pub fn with_image(version: ClientVersion, image: &dyn TamperImage) -> Option<Self> {
        let info = version.profile().protocol.tamper?;
        let words = image.words()?;
        Some(Self {
            payload: encode_words(words),
            resend: info.resend,
            // The login send counts as the cycle's send: the next one is a
            // full cycle away.
            counter: SEND_PHASE,
            carry: Duration::ZERO,
        })
    }

    /// A sender for `version` with the build's own constant: what the real
    /// client sends. `None` for a version that does not send id 39 and for
    /// 2.3, whose constant was never captured.
    pub fn for_version(version: ClientVersion) -> Option<Self> {
        let words = version.profile().protocol.tamper?.words?;
        Self::with_image(version, &words)
    }

    /// The 10 payload bytes.
    pub fn payload(&self) -> &[u8; 10] {
        &self.payload
    }

    /// The wire message: `GCHAR 39`, the 10 raw bytes, newline. `None` if
    /// the payload contains a newline: it would split the line, and the real
    /// client's send helper drops such payloads too.
    pub fn message(&self) -> Option<Vec<u8>> {
        if self.payload.contains(&b'\n') {
            return None;
        }
        let mut m = Vec::with_capacity(12);
        m.push(PLI_TAMPERCHECK + 0x20);
        m.extend_from_slice(&self.payload);
        m.push(b'\n');
        Some(m)
    }

    /// Time has passed. True when the build is due to send again (2.31
    /// only: every 64 ticks of 50 ms, i.e. 3.2 s after the last send).
    /// At most one send is reported per call, so a stalled caller does not
    /// get a burst.
    pub fn advance(&mut self, dt: Duration) -> bool {
        if self.resend != TamperResend::Timer {
            return false;
        }
        // A long stall is at most one cycle's worth of ticks.
        let cycle = Duration::from_millis(u64::from(TICK_MS) * u64::from(TICKS_PER_CYCLE));
        let tick = Duration::from_millis(u64::from(TICK_MS));
        self.carry += dt.min(cycle);
        let mut due = false;
        while self.carry >= tick {
            self.carry -= tick;
            self.counter = (self.counter + 1) % TICKS_PER_CYCLE;
            due |= self.counter == SEND_PHASE;
        }
        due
    }

    /// The player asked for a level link (not a server-side warp). True when
    /// the build sends again in the same frame, right after the request
    /// (2.22; see `Connection::send_link_request`).
    pub fn level_changed(&mut self) -> bool {
        self.resend == TamperResend::OnLevelChange
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAB: [(ClientVersion, u32, u32, &str); 3] = [
        (ClientVersion::V217, 0xab637cbe, 0x3e48dc0c, "2a7bad195e231243d82c"),
        (ClientVersion::V222, 0x5dcb77f6, 0xbc736311, "250e4d0f962b03ede631"),
        (ClientVersion::V231, 0x9ecd4bc0, 0xecea65e2, "291655b7602e87c9eb82"),
    ];

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// The six lab values: two words per build, three builds.
    #[test]
    fn encoder_reproduces_the_lab_values() {
        for (v, a, b, bytes) in LAB {
            assert_eq!(hex(&encode_words(TamperWords { a, b })), bytes, "{v:?}");
            assert_eq!(hex(&encode_word(a)), bytes[..10], "{v:?} word A");
            assert_eq!(hex(&encode_word(b)), bytes[10..], "{v:?} word B");
        }
    }

    /// The middle bytes are not masked, so they can fall below 0x20.
    #[test]
    fn middle_bytes_are_not_masked() {
        let w = encode_words(TamperWords { a: 0xab637cbe, b: 0x3e48dc0c });
        assert!(w.iter().any(|&b| b < 0x20), "{w:02x?}");
        assert_eq!(encode_word(0), [0x20; 5]);
        assert_eq!(encode_word(0xFFFF_FFFF), [0x2f, 0x1f, 0x1f, 0x1f, 0x9f]);
    }

    #[test]
    fn words_round_trip() {
        for v in [0u32, 1, 0x7F, 0x80, 0xdead_beef, 0xFFFF_FFFF, 0x8000_0000, 0x1234_5678] {
            assert_eq!(decode_word(encode_word(v)), v, "{v:#x}");
        }
        for (_, a, b, _) in LAB {
            assert_eq!(decode_word(encode_word(a)), a);
            assert_eq!(decode_word(encode_word(b)), b);
        }
    }

    /// Vectors made with an independent implementation of the derivation.
    #[test]
    fn crc_chain_vectors() {
        assert_eq!(crc_chain(&[b"123456789"]), 0xcbf43926);
        assert_eq!(crc_chain(&[b"hello", b"world!!", b"graal"]), 0x5deaf019);
        assert_eq!(crc_chain(&[b"abc", b"def"]), 0xb4cfe0fd);
        let big = [(0..=255u8).collect::<Vec<_>>().repeat(3), vec![b'x'; 100]];
        assert_eq!(crc_chain(&[&big[0], &big[1]]), 0xb8c5f682);
        assert_eq!(crc_chain(&[]), 0xFFFF_FFFF);
    }

    #[test]
    fn range_image_feeds_the_encoder() {
        let img = RangeImage {
            a: vec![b"hello".to_vec(), b"world!!".to_vec(), b"graal".to_vec()],
            b: vec![b"abc".to_vec(), b"def".to_vec()],
        };
        assert_eq!(img.words(), Some(TamperWords { a: 0x5deaf019, b: 0xb4cfe0fd }));
        let s = TamperSender::with_image(ClientVersion::V222, &img).unwrap();
        assert_eq!(s.payload(), &encode_words(TamperWords { a: 0x5deaf019, b: 0xb4cfe0fd }));
        assert!(RangeImage::default().words().is_none());
        assert!(TamperSender::with_image(ClientVersion::V222, &RangeImage::default()).is_none());
    }

    /// The default is each build's constant, byte for byte what the lab saw.
    #[test]
    fn default_value_is_the_lab_value() {
        for (v, _, _, bytes) in LAB {
            let s = TamperSender::for_version(v).unwrap();
            assert_eq!(hex(s.payload()), bytes, "{v:?}");
            let m = s.message().unwrap();
            assert_eq!(m[0], 0x20 + 39);
            assert_eq!(&m[1..11], s.payload());
            assert_eq!(m[11], b'\n');
            assert_eq!(m.len(), 12);
            // No newline in any of them, so the line framing is safe.
            assert!(!s.payload().contains(&b'\n'));
        }
    }

    #[test]
    fn versions_without_a_value_send_nothing() {
        for v in ClientVersion::all_versions() {
            let has = matches!(v, ClientVersion::V217 | ClientVersion::V222 | ClientVersion::V231);
            assert_eq!(TamperSender::for_version(*v).is_some(), has, "{v:?}");
        }
        // 2.3 would send it, but its constant is unknown.
        assert!(ClientVersion::V230.sends_tampercheck());
        assert!(TamperSender::for_version(ClientVersion::V230).is_none());
        assert!(TamperSender::with_image(ClientVersion::V205, &TamperWords { a: 1, b: 2 }).is_none());
    }

    #[test]
    fn a_newline_in_the_payload_is_not_sent() {
        // Word whose first byte is 0x0a is impossible (>= 0x20), so make the
        // newline appear in a middle byte: (v >> 14) + 0x20 == 0x0a.
        let v = 0xEA << 14; // 0xEA + 0x20 = 0x10a -> 0x0a
        assert!(encode_word(v).contains(&b'\n'));
        let s = TamperSender::with_image(ClientVersion::V222, &TamperWords { a: v, b: 0 }).unwrap();
        assert!(s.message().is_none());
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn v231_resends_every_3_2_seconds() {
        let mut s = TamperSender::for_version(ClientVersion::V231).unwrap();
        // Nothing before a full cycle after the login send.
        assert!(!s.advance(ms(3150)));
        assert!(s.advance(ms(50)));
        // Then every 3.2 s, in 50 ms steps too.
        let mut sends = 0;
        for _ in 0..(64 * 3) {
            sends += s.advance(ms(50)) as u32;
        }
        assert_eq!(sends, 3);
        // Odd step sizes carry the remainder.
        let mut s = TamperSender::for_version(ClientVersion::V231).unwrap();
        let mut t = 0;
        let mut at = Vec::new();
        for _ in 0..1000 {
            t += 20;
            if s.advance(ms(20)) {
                at.push(t);
            }
        }
        assert_eq!(at, vec![3200, 6400, 9600, 12800, 16000, 19200]);
        // Calls much shorter than a tick still add up (a busy caller).
        let mut s = TamperSender::for_version(ClientVersion::V231).unwrap();
        let mut sends = 0;
        for _ in 0..3200 {
            sends += s.advance(Duration::from_micros(1000)) as u32;
        }
        assert_eq!(sends, 1);
        // A long stall gives one send, not a burst.
        let mut s = TamperSender::for_version(ClientVersion::V231).unwrap();
        assert!(s.advance(Duration::from_secs(3600)));
        assert!(!s.advance(ms(0)));
    }

    #[test]
    fn v222_resends_on_level_change_and_v217_never() {
        let mut s = TamperSender::for_version(ClientVersion::V222).unwrap();
        assert!(s.level_changed());
        assert!(!s.advance(Duration::from_secs(600)));
        let mut s = TamperSender::for_version(ClientVersion::V217).unwrap();
        assert!(!s.level_changed());
        assert!(!s.advance(Duration::from_secs(600)));
        let mut s = TamperSender::for_version(ClientVersion::V231).unwrap();
        assert!(!s.level_changed());
    }
}
