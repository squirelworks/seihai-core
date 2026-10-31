//! `PLI_SHOWIMGPLAYER` (24) and its relay `PLO_SHOWIMG` (32).
//!
//! # What the message is
//!
//! It is the sender's list of weapon-script images (`showimg` and the
//! `changeimg*` commands), sent as a text that holds only what changed
//! since the last send. The receiver keeps the list per player and draws
//! it. The one byte `GCHAR 9` every real client was seen to send is the
//! first character of that text on its own: the "full list follows" marker
//! with an empty list after it (derived from the 2.17.1, 2.22 and 2.31
//! clients, 2026-10-25; see `documentation/remaining-fields.md`).
//!
//! ```text
//! text    = [ ')' ] record*          ')' (0x29) first = full list
//! record  = (0x2a + index) field*    index 0..=199, one byte
//! field   = tag-byte (0x20 + n) data
//!   n=0  name        GCHAR length, then the name; empty name = remove it
//!   n=1  x           GCHAR (half tiles, or screen units, see `layer`)
//!   n=2  y           GCHAR
//!   n=3  layer       GCHAR, value - 1 (default 1; 10 and over = screen)
//!   n=4  part        0x20 = whole image, or 0x21 then left and top as two
//!                    7-bit bytes each, then width and height as one byte each
//!   n=5  colours     four GCHARs, each over 200
//!   n=6  zoom        GCHAR over 10
//!   n=7  order       one byte, value - 0x52
//!   n=8  mode        GCHAR
//! ```
//!
//! # When it is sent (lab, 2026-10-24, real clients, nothing altered)
//!
//! A periodic handler (about 50 ms, inferred from the period) runs a counter that wraps
//! every 101 calls; after a call that sent something the next call is
//! skipped, so a wrap is 102 ticks, about 5.1 s (the lab measured a median
//! 5.13 s). At each wrap the client builds the full list, so with no images
//! the whole message is the marker `GCHAR 9`. Between wraps it sends only
//! changes, and nothing at all when there are none.
//!
//! | build | period | condition |
//! |---|---|---|
//! | 2.22 | about 5.1 s | the TCP copy goes out only while another player is in the level (0 sends in 3 solo sessions); the client's UDP copy (2.05 UDP id 15) is the other way to reach players |
//! | 2.31 | median 5.13 s (5.11 to 5.17) | always, idle or walking, in every level used |
//! | 2.17.1 | 5.07 to 5.11 s while idle (lab, 2026-10-30) | always: alone in the level in all five runs, on both lab servers, the first one at the end of the post-login frame |
//! | 4.04r2 | 5.142 to 5.151 s (lab, 2026-10-31) | always: alone in the level, the first at the end of the after-burst frames; 4.20r6 and 4.20r8 take it (4.20r8 sent it with the same period in the first 4.x live run) |
//!
//! GServer-v2 relays the payload unchanged to the other players in the
//! level as `PLO_SHOWIMG` (32, `GSHORT sender id` plus the client's bytes).
//! Seihai sends only the marker and only when asked (`--send-showimg`); it
//! has no weapon-script images to list. The period and condition come from
//! the profile, never from the version.

use super::version::ClientVersion;
use std::time::Duration;

/// Client to server id.
pub const PLI_SHOWIMGPLAYER: u8 = 24;
/// The one-byte payload every real client sent: the "full list follows"
/// marker (`)`, 0x29) with no image after it, as a GCHAR.
pub const SHOWIMG_VALUE: u8 = 9;
/// The marker as it appears in the text.
pub const SHOWIMG_FULL_MARKER: u8 = 0x29;
/// A record's first byte is `0x2a + index`.
pub const SHOWIMG_INDEX_BASE: u8 = 0x2a;

/// What a profile records about id 24.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShowImgInfo {
    /// Time between sends.
    pub period: Duration,
    /// True when the client sends only while another player is in its level.
    pub needs_others: bool,
}

/// The wire message: `GCHAR 24`, `GCHAR 9`, newline.
pub fn showimg_message() -> Vec<u8> {
    vec![PLI_SHOWIMGPLAYER + 0x20, SHOWIMG_VALUE + 0x20, b'\n']
}

/// Decides when id 24 goes out. The real clients run a free counter: the
/// first tick after login is a wrap and sends, then one wrap per period.
/// Where the profile needs another player in the level, a wrap that finds
/// nobody there is lost; the next one is a full period later (the client
/// builds the message at every wrap and drops the TCP copy when the test
/// fails).
#[derive(Debug, Clone)]
pub struct ShowImgSender {
    info: ShowImgInfo,
    /// Time since the last wrap, capped at one period.
    since: Duration,
}

impl ShowImgSender {
    /// `None` for a profile whose build has no recorded id 24 behaviour.
    pub fn for_version(version: ClientVersion) -> Option<Self> {
        let info = version.profile().protocol.showimg?;
        Some(Self { info, since: info.period })
    }

    pub fn info(&self) -> ShowImgInfo {
        self.info
    }

    /// `dt` has passed; `others_present` is true when another player is in
    /// the level. True when a send is due. At most one wrap per call, and a
    /// stalled caller does not get a burst. A wrap with the profile's
    /// condition unmet is skipped, not kept for later.
    pub fn advance(&mut self, dt: Duration, others_present: bool) -> bool {
        self.since = (self.since + dt).min(self.info.period);
        if self.since < self.info.period {
            return false;
        }
        self.since = Duration::ZERO;
        !self.info.needs_others || others_present
    }
}

/// How an image is placed, as the sender wrote the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImgField {
    /// Tag 0. An empty name removes the image.
    Name(Vec<u8>),
    /// Tag 1, the GCHAR as sent (half tiles from the level corner, or
    /// screen units when the layer is 10 or over).
    X(u8),
    /// Tag 2, as tag 1.
    Y(u8),
    /// Tag 3, GCHAR minus 1. The default is 1.
    Layer(i32),
    /// Tag 4. `None` shows the whole image.
    Part(Option<ImgPart>),
    /// Tag 5, four GCHARs, each over 200 (red, green, blue, alpha).
    Colour([u8; 4]),
    /// Tag 6, a GCHAR over 10.
    Zoom(u8),
    /// Tag 7, the byte minus 0x52.
    Order(i32),
    /// Tag 8, a GCHAR.
    Mode(u8),
}

/// The visible part of an image, clamped the way the client clamps it:
/// left and top to 0..=16000, width and height to 0..=220.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImgPart {
    pub left: u16,
    pub top: u16,
    pub width: u8,
    pub height: u8,
}

/// One record: an image index and the fields that came with it, in order
/// (order matters: a name creates the image, and the other fields only
/// touch an image that exists).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImgRecord {
    pub index: u8,
    pub fields: Vec<ImgField>,
}

/// A decoded id 24 / 32 text (after the sender id for 32).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShowImgUpdate {
    /// The text began with the marker: images of that player that this
    /// text does not name are removed.
    pub full: bool,
    pub records: Vec<ImgRecord>,
}

fn gc(b: u8) -> i32 {
    i32::from(b) - 0x20
}

/// Decode the text. It follows the client's reader: it stops where the
/// reader stops (a tag with no data byte clears the rest), a tag of 10 or
/// more starts the next record, an unknown tag skips its one data byte, and
/// a record the client could not place (index below 0) is dropped.
pub fn parse_showimg_update(text: &[u8]) -> ShowImgUpdate {
    let mut out = ShowImgUpdate::default();
    let mut i = 0;
    let mut first = true;
    while i < text.len() {
        let index = i32::from(text[i]) - i32::from(SHOWIMG_INDEX_BASE);
        i += 1;
        if first && index == -1 {
            out.full = true;
        }
        first = false;
        let mut fields = Vec::new();
        // Fields run until a byte of 0x2a or more (the next record).
        while let Some(&t) = text.get(i) {
            if t >= SHOWIMG_INDEX_BASE {
                break;
            }
            // The client needs the tag and at least one more byte.
            let Some(&d) = text.get(i + 1) else {
                return finish(out, index, fields);
            };
            let tag = t.wrapping_sub(0x20);
            let mut used = 2;
            match tag {
                0 => {
                    let n = gc(d).max(0) as usize;
                    let end = (i + 2 + n).min(text.len());
                    fields.push(ImgField::Name(text[i + 2..end].to_vec()));
                    used = 2 + n;
                }
                1 => fields.push(ImgField::X(d)),
                2 => fields.push(ImgField::Y(d)),
                3 => fields.push(ImgField::Layer(gc(d) - 1)),
                4 if d == 0x20 => fields.push(ImgField::Part(None)),
                4 if d == 0x21 => {
                    used = 8;
                    if text.len() - i > 7 {
                        let two = |a: u8, b: u8| (gc(a) * 0x80 + gc(b)).clamp(0, 16000) as u16;
                        fields.push(ImgField::Part(Some(ImgPart {
                            left: two(text[i + 2], text[i + 3]),
                            top: two(text[i + 4], text[i + 5]),
                            width: gc(text[i + 6]).clamp(0, 220) as u8,
                            height: gc(text[i + 7]).clamp(0, 220) as u8,
                        })));
                    }
                }
                5 => {
                    used = 5;
                    if text.len() - i > 4 {
                        let c = |k: usize| gc(text[i + k]).clamp(0, 255) as u8;
                        fields.push(ImgField::Colour([c(1), c(2), c(3), c(4)]));
                    }
                }
                6 => fields.push(ImgField::Zoom(gc(d).clamp(0, 255) as u8)),
                7 => fields.push(ImgField::Order(i32::from(d) - 0x52)),
                8 => fields.push(ImgField::Mode(gc(d).clamp(0, 255) as u8)),
                _ => {}
            }
            i += used;
            if i >= text.len() {
                break;
            }
        }
        out = finish(out, index, fields);
    }
    out
}

/// Keep a record when its index is in range.
fn finish(mut out: ShowImgUpdate, index: i32, fields: Vec<ImgField>) -> ShowImgUpdate {
    if (0..=255).contains(&index) {
        out.records.push(ImgRecord { index: index as u8, fields });
    }
    out
}

/// What the receiver keeps for one shown image (the `TShowImg` of the
/// client): the last value of each field, with the defaults the client
/// gives a new one. Nothing draws it yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerImage {
    pub name: Vec<u8>,
    pub x: u8,
    pub y: u8,
    pub layer: i32,
    pub part: Option<ImgPart>,
    pub colour: [u8; 4],
    pub zoom: u8,
    pub order: Option<i32>,
    pub mode: u8,
}

impl PlayerImage {
    fn new() -> Self {
        Self { name: Vec::new(), x: 0, y: 0, layer: 1, part: None, colour: [200; 4], zoom: 10, order: None, mode: 0 }
    }
}

/// One player's images by index.
pub type PlayerImages = std::collections::BTreeMap<u8, PlayerImage>;

/// Apply an update to one player's images the way the client does: a name
/// creates (or renames) an image, an empty name removes it, other fields
/// touch only an image that exists, and after a full list every image the
/// text did not name (with a non-empty name field) is removed.
pub fn apply_showimg_update(images: &mut PlayerImages, up: &ShowImgUpdate) {
    let mut named = std::collections::BTreeSet::new();
    for rec in &up.records {
        let mut cur = images.get(&rec.index).cloned();
        for f in &rec.fields {
            match f {
                ImgField::Name(n) if n.is_empty() => cur = None,
                ImgField::Name(n) => {
                    let mut img = cur.take().unwrap_or_else(PlayerImage::new);
                    img.name = n.clone();
                    cur = Some(img);
                    named.insert(rec.index);
                }
                other => {
                    let Some(img) = cur.as_mut() else { continue };
                    match other {
                        ImgField::X(v) => img.x = *v,
                        ImgField::Y(v) => img.y = *v,
                        ImgField::Layer(v) => img.layer = *v,
                        ImgField::Part(p) => img.part = *p,
                        ImgField::Colour(c) => img.colour = *c,
                        ImgField::Zoom(z) => img.zoom = *z,
                        ImgField::Order(o) => img.order = Some(*o),
                        ImgField::Mode(m) => img.mode = *m,
                        ImgField::Name(_) => {}
                    }
                }
            }
        }
        match cur {
            Some(img) => {
                images.insert(rec.index, img);
            }
            None => {
                images.remove(&rec.index);
            }
        }
    }
    if up.full {
        images.retain(|k, _| named.contains(k));
    }
}

/// A short line for the capture viewer: the marker and one entry per
/// record.
pub fn describe_showimg_update(up: &ShowImgUpdate) -> String {
    let mut parts = Vec::new();
    for r in &up.records {
        let mut s = format!("#{}", r.index);
        for f in &r.fields {
            match f {
                ImgField::Name(n) if n.is_empty() => s.push_str(" remove"),
                ImgField::Name(n) => s.push_str(&format!(" name={}", crate::protocol::latin1(n))),
                ImgField::X(v) => s.push_str(&format!(" x={}", gc(*v))),
                ImgField::Y(v) => s.push_str(&format!(" y={}", gc(*v))),
                ImgField::Layer(v) => s.push_str(&format!(" layer={v}")),
                ImgField::Part(None) => s.push_str(" part=all"),
                ImgField::Part(Some(p)) => s.push_str(&format!(" part={},{} {}x{}", p.left, p.top, p.width, p.height)),
                ImgField::Colour(c) => s.push_str(&format!(" colour={},{},{},{}", c[0], c[1], c[2], c[3])),
                ImgField::Zoom(z) => s.push_str(&format!(" zoom={z}/10")),
                ImgField::Order(o) => s.push_str(&format!(" order={o}")),
                ImgField::Mode(m) => s.push_str(&format!(" mode={m}")),
            }
        }
        parts.push(s);
    }
    if parts.is_empty() { "(none)".to_string() } else { parts.join("; ") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PROFILES;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn message_is_one_gchar_9() {
        assert_eq!(showimg_message(), [24 + 0x20, 9 + 0x20, b'\n']);
    }

    /// Only profiles that can send id 24 carry the record; the two lab
    /// builds are the ones that have it.
    #[test]
    fn profile_records_follow_the_send_set_and_the_lab() {
        for p in PROFILES {
            if p.protocol.showimg.is_some() {
                assert!(p.protocol.pli.contains(PLI_SHOWIMGPLAYER), "{}", p.label);
            }
        }
        let with: Vec<&str> =
            PROFILES.iter().filter(|p| p.protocol.showimg.is_some()).map(|p| p.label).collect();
        assert_eq!(with, ["2.17", "2.22", "2.31", "4.04r2", "4.20r6", "4.20r8"]);
        let i404 = ClientVersion::V404R2.profile().protocol.showimg.unwrap();
        assert!(!i404.needs_others);
        assert_eq!(i404.period, MS(5145));
        assert!(ClientVersion::V500R7.profile().protocol.showimg.is_none());
        let i222 = ClientVersion::V222.profile().protocol.showimg.unwrap();
        let i231 = ClientVersion::V231.profile().protocol.showimg.unwrap();
        assert!(i222.needs_others && !i231.needs_others);
        assert_eq!(i222.period, MS(5100));
        assert_eq!(i231.period, MS(5130));
        let i217 = ClientVersion::V217.profile().protocol.showimg.unwrap();
        assert!(!i217.needs_others);
        assert_eq!(i217.period, MS(5100));
        assert!(ShowImgSender::for_version(ClientVersion::V205).is_none());
    }

    /// 2.31: first send at once, then one per period, alone or not.
    #[test]
    fn v231_sends_at_once_then_every_period() {
        let mut s = ShowImgSender::for_version(ClientVersion::V231).unwrap();
        assert!(s.advance(MS(0), false));
        let mut sends = 0;
        // 20 s in 50 ms steps: sends at 5.13, 10.26 and 15.39 s.
        for _ in 0..400 {
            sends += usize::from(s.advance(MS(50), false));
        }
        assert_eq!(sends, 3);
        // One send per call even after a long stall.
        assert!(s.advance(Duration::from_secs(60), false));
        assert!(!s.advance(MS(50), false));
    }

    /// 2.17.1, alone (`2.17-login-real.trace.tsv`): sends at 0.15, 5.24,
    /// 10.35, 15.45 and 20.55 s, the first with the post-login frame.
    #[test]
    fn v217_sends_alone_every_period() {
        let mut s = ShowImgSender::for_version(ClientVersion::V217).unwrap();
        assert!(s.advance(MS(0), false));
        let mut at = Vec::new();
        for i in 1..=410 {
            if s.advance(MS(50), false) {
                at.push(i * 50);
            }
        }
        assert_eq!(at, [5100, 10200, 15300, 20400]);
    }

    /// 2.22: the counter runs free. Wraps while alone are lost; a player
    /// who arrives mid-period gets the next wrap, a full period after the
    /// last one, not at once.
    #[test]
    fn v222_wraps_free_and_loses_the_ones_spent_alone() {
        let mut s = ShowImgSender::for_version(ClientVersion::V222).unwrap();
        // Alone for 20 s: the wraps at 0, 5.1, 10.2 and 15.3 s are lost.
        for _ in 0..400 {
            assert!(!s.advance(MS(50), false));
        }
        // Someone arrives at 20 s; the last wrap was at 15.3 s, so the next
        // is at 20.4 s, 450 ms later.
        let mut sent_at = None;
        for k in 1..=60 {
            if s.advance(MS(50), true) {
                sent_at = Some(k * 50);
                break;
            }
        }
        assert_eq!(sent_at, Some(450));
        // Then one per period while someone stays.
        let mut sends = 0;
        for _ in 0..400 {
            sends += usize::from(s.advance(MS(50), true));
        }
        assert_eq!(sends, 3);
        // The other player leaves: it stops again.
        for _ in 0..400 {
            assert!(!s.advance(MS(50), false));
        }
    }

    /// 2.22 with a player already in the level at login: the first tick is
    /// a wrap and sends.
    #[test]
    fn v222_sends_on_the_first_tick_when_someone_is_there() {
        let mut s = ShowImgSender::for_version(ClientVersion::V222).unwrap();
        assert!(s.advance(MS(0), true));
        assert!(!s.advance(MS(50), true));
    }

    fn text(s: &str) -> Vec<u8> {
        s.bytes().collect()
    }

    /// The byte every real client sent is the marker alone.
    #[test]
    fn the_real_clients_byte_is_the_full_marker_with_no_images() {
        let up = parse_showimg_update(&[SHOWIMG_VALUE + 0x20]);
        assert!(up.full);
        assert!(up.records.is_empty());
        assert_eq!(describe_showimg_update(&up), "(none)");
        assert_eq!(SHOWIMG_VALUE + 0x20, SHOWIMG_FULL_MARKER);
    }

    /// A full list with one image: name, x, y, layer, part, colours, zoom,
    /// order and mode, then a second record that only names an image.
    #[test]
    fn a_full_list_decodes_every_field() {
        let mut t = vec![SHOWIMG_FULL_MARKER, SHOWIMG_INDEX_BASE + 3];
        // name "a.png", x = 21 half tiles, y = 8
        t.extend([0x20, 0x20 + 5]);
        t.extend(b"a.png");
        t.extend([0x21, 0x20 + 21, 0x22, 0x20 + 8]);
        // layer 2: the byte is layer + 0x21
        t.extend([0x23, 0x21 + 2]);
        // part: 0x21, left 130 (1, 2), top 5 (0, 5), 16 by 32
        t.extend([0x24, 0x21, 0x21, 0x22, 0x20, 0x25, 0x20 + 16, 0x20 + 32]);
        // colours 200, 100, 0, 150
        t.extend([0x25, 0x20 + 200, 0x20 + 100, 0x20, 0x20 + 150]);
        // zoom 15/10, order +3, mode 2
        t.extend([0x26, 0x20 + 15, 0x27, 0x52 + 3, 0x28, 0x20 + 2]);
        // second image: index 0, name only
        t.extend([SHOWIMG_INDEX_BASE, 0x20, 0x20 + 1, b'b']);
        let up = parse_showimg_update(&t);
        assert!(up.full);
        assert_eq!(up.records.len(), 2);
        assert_eq!(up.records[0].index, 3);
        assert_eq!(
            up.records[0].fields,
            vec![
                ImgField::Name(text("a.png")),
                ImgField::X(0x20 + 21),
                ImgField::Y(0x20 + 8),
                ImgField::Layer(2),
                ImgField::Part(Some(ImgPart { left: 130, top: 5, width: 16, height: 32 })),
                ImgField::Colour([200, 100, 0, 150]),
                ImgField::Zoom(15),
                ImgField::Order(3),
                ImgField::Mode(2),
            ]
        );
        assert_eq!(up.records[1], ImgRecord { index: 0, fields: vec![ImgField::Name(text("b"))] });
        let d = describe_showimg_update(&up);
        assert!(d.starts_with("#3 name=a.png x=21 y=8 layer=2 part=130,5 16x32"), "{d}");
    }

    /// Part values are clamped as the client clamps them, and 0x20 resets.
    #[test]
    fn part_is_clamped_and_resets() {
        // left 0x7f * 128 + 0x7f = 16383 -> 16000; width byte 0xff (223) -> 220
        let t = [
            SHOWIMG_INDEX_BASE, 0x24, 0x21, 0x20 + 127, 0x20 + 127, 0x20, 0x20, 0xff, 0x20,
            0x24, 0x20,
        ];
        let up = parse_showimg_update(&t);
        assert_eq!(
            up.records[0].fields,
            vec![
                ImgField::Part(Some(ImgPart { left: 16000, top: 0, width: 220, height: 0 })),
                ImgField::Part(None),
            ]
        );
    }

    /// A dangling tag ends the text; a short part or colour is consumed
    /// but not applied; an unknown tag skips its one data byte; an index
    /// below 0 anywhere but the start is dropped.
    #[test]
    fn malformed_text_stops_where_the_client_stops() {
        // Tag with no data byte.
        let up = parse_showimg_update(&[SHOWIMG_INDEX_BASE, 0x21]);
        assert_eq!(up.records, vec![ImgRecord { index: 0, fields: vec![] }]);
        // Short part (needs 7 more bytes).
        let up = parse_showimg_update(&[SHOWIMG_INDEX_BASE, 0x24, 0x21, 0x20, 0x20]);
        assert_eq!(up.records[0].fields, vec![]);
        // Unknown tag 9 skips one byte; the field after it still reads.
        let up = parse_showimg_update(&[SHOWIMG_INDEX_BASE, 0x29, 0x20, 0x21, 0x20 + 4]);
        assert_eq!(up.records[0].fields, vec![ImgField::X(0x20 + 4)]);
        // A 0x29 after the first byte is tag 9 (skipped with its data
        // byte), not a marker: the text is not a full list.
        let up = parse_showimg_update(&[SHOWIMG_INDEX_BASE, 0x21, 0x20, SHOWIMG_FULL_MARKER, 0x21, 0x20]);
        assert!(!up.full);
        assert_eq!(up.records.len(), 1);
        // Empty text is nothing.
        assert_eq!(parse_showimg_update(&[]), ShowImgUpdate::default());
    }

    /// Names create, other fields need an existing image, an empty name
    /// removes, and a full list removes what it did not name.
    #[test]
    fn applying_follows_the_receiver() {
        let mut imgs = PlayerImages::new();
        // A field for an image that is not there changes nothing.
        apply_showimg_update(&mut imgs, &parse_showimg_update(&[SHOWIMG_INDEX_BASE + 1, 0x21, 0x20 + 9]));
        assert!(imgs.is_empty());
        // Name first, then the field, in one record.
        let mut t = vec![SHOWIMG_INDEX_BASE + 1, 0x20, 0x20 + 1, b'a', 0x21, 0x20 + 9];
        t.extend([SHOWIMG_INDEX_BASE + 2, 0x20, 0x20 + 1, b'b']);
        apply_showimg_update(&mut imgs, &parse_showimg_update(&t));
        assert_eq!(imgs.len(), 2);
        assert_eq!(imgs[&1].x, 0x20 + 9);
        assert_eq!((imgs[&1].layer, imgs[&1].colour, imgs[&1].zoom), (1, [200; 4], 10));
        // A change to image 1 only.
        apply_showimg_update(&mut imgs, &parse_showimg_update(&[SHOWIMG_INDEX_BASE + 1, 0x22, 0x20 + 3]));
        assert_eq!((imgs[&1].x, imgs[&1].y, imgs.len()), (0x20 + 9, 0x20 + 3, 2));
        // A full list that names only image 2 drops image 1.
        let full = [SHOWIMG_FULL_MARKER, SHOWIMG_INDEX_BASE + 2, 0x20, 0x20 + 1, b'b'];
        apply_showimg_update(&mut imgs, &parse_showimg_update(&full));
        assert_eq!(imgs.keys().copied().collect::<Vec<_>>(), vec![2]);
        // The marker alone empties the list.
        apply_showimg_update(&mut imgs, &parse_showimg_update(&[SHOWIMG_VALUE + 0x20]));
        assert!(imgs.is_empty());
        // An empty name removes one image.
        apply_showimg_update(&mut imgs, &parse_showimg_update(&[SHOWIMG_INDEX_BASE, 0x20, 0x20 + 1, b'z']));
        apply_showimg_update(&mut imgs, &parse_showimg_update(&[SHOWIMG_INDEX_BASE, 0x20, 0x20]));
        assert!(imgs.is_empty());
    }
}
