//! `PLO_NPCPROPS` (3) and `PLI_NPCPROPS` (3): the props of a server NPC.
//!
//! Both ways the payload is `GINT3 id`, then props as `GCHAR prop id` and
//! a value. The prop ids and value shapes are GServer-v2's (`TNPC.h`); the
//! 2.05 behaviour was checked in the lab (`documentation/ledges-push-pull.md`).
//! Image, script, position, the two flag bytes, the message, the
//! character props (GANI attributes 1 to 30) and the NPC's level
//! (`CURLEVEL`) are kept; the other props up to 76 are skipped by their
//! length, and reading stops at a prop whose length is not known (43, 48,
//! and anything past 76; GServer-v2 never sends 43 or 48).

use super::dispatch::{latin1, read_gint3, read_gshort, script_text};

/// `NPCPROP_IMAGE` (0): `GCHAR` length and the file name.
pub const NPCPROP_IMAGE: u8 = 0;
/// `NPCPROP_X` (2) and `NPCPROP_Y` (3): `GCHAR` in half tiles.
pub const NPCPROP_X: u8 = 2;
pub const NPCPROP_Y: u8 = 3;
/// `NPCPROP_VISFLAGS` (13), see [`VIS_VISIBLE`] and the others.
pub const NPCPROP_VISFLAGS: u8 = 13;
/// `NPCPROP_BLOCKFLAGS` (14), see [`BLOCK_DONTBLOCK`] and the others.
pub const NPCPROP_BLOCKFLAGS: u8 = 14;
/// `NPCPROP_MESSAGE` (15): `GCHAR` length and the text of `message`.
pub const NPCPROP_MESSAGE: u8 = 15;
/// The character props a `showcharacter` NPC reports: GANI (12, `GCHAR`
/// length and name, also from the 2.05 client), SPRITE (18, the
/// direction), COLORS (19, five colour indexes), NICKNAME (20), HEAD (22,
/// an index under 100 or `100 + length` and the name), BODY (35) and the
/// GANI attributes 1 to 5 (36 to 40).
pub const NPCPROP_GANI: u8 = 12;
pub const NPCPROP_SPRITE: u8 = 18;
pub const NPCPROP_COLORS: u8 = 19;
pub const NPCPROP_NICKNAME: u8 = 20;
pub const NPCPROP_HEAD: u8 = 22;
pub const NPCPROP_BODY: u8 = 35;
pub const NPCPROP_GATTRIB1: u8 = 36;
/// GANI attributes 6 to 9 (44 to 47) and 10 to 30 (53 to 73), as 1 to 5:
/// `GCHAR` length and the text.
pub const NPCPROP_GATTRIB6: u8 = 44;
pub const NPCPROP_GATTRIB10: u8 = 53;
/// `NPCPROP_CURLEVEL` (52): `GCHAR` length and the level the NPC is on.
pub const NPCPROP_CURLEVEL: u8 = 52;
/// `NPCPROP_X2` (75) and `NPCPROP_Y2` (76): `GSHORT` pixels, doubled, the
/// low bit the sign (2.3 and later).
pub const NPCPROP_X2: u8 = 75;
pub const NPCPROP_Y2: u8 = 76;
/// GANI attributes the NPC record holds (`#P1` to `#P30`).
pub const NPC_GATTRIBS: usize = 30;
/// The image a character NPC reports.
pub const CHARACTER_IMAGE: &str = "#c#";
/// The colours of a character NPC whose script set none (2.05, lab).
pub const DEFAULT_NPC_COLORS: [u8; 5] = [2, 5, 18, 5, 18];

/// HEAD: a numbered head (`head<n>`) or an image name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    Index(u8),
    Name(String),
}

/// Vis flag bits, as the 2.05 client reports them: shown 1,
/// `drawoverplayer` 2, `drawunderplayer` 4 (`drawaslight` sets none).
/// It sets [`VIS_CLIENT`] on every NPC it reports, and keeps any other
/// bit the server gave; with none given it starts from [`VIS_DEFAULT`].
pub const VIS_VISIBLE: u8 = 1;
pub const VIS_OVER: u8 = 2;
pub const VIS_UNDER: u8 = 4;
pub const VIS_CLIENT: u8 = 16;
pub const VIS_DEFAULT: u8 = 64 | VIS_VISIBLE;

/// Block flag bits, as the 2.05 client reports them after the NPC's script
/// ran: `dontblock` 1, `canbepulled` 4, `canbepushed` 8.
pub const BLOCK_DONTBLOCK: u8 = 1;
pub const BLOCK_PULLABLE: u8 = 4;
pub const BLOCK_PUSHABLE: u8 = 8;

/// What one `PLO_NPCPROPS` said about an NPC. The server sends a whole
/// record when the NPC first shows, and only what changed after that.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NpcProps {
    pub image: Option<String>,
    /// The GS1 client script, `§` turned into line breaks.
    pub script: Option<String>,
    /// Position in tiles.
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub vis_flags: Option<u8>,
    pub block_flags: Option<u8>,
    /// The NPC's `message` text.
    pub message: Option<String>,
    /// Character props (see [`NPCPROP_GANI`]).
    pub gani: Option<String>,
    pub sprite: Option<u8>,
    pub colors: Option<[u8; 5]>,
    pub nickname: Option<String>,
    pub head: Option<Head>,
    pub body: Option<String>,
    /// GANI attributes 1 to 30.
    pub attrs: [Option<String>; NPC_GATTRIBS],
    /// The level the server says the NPC is on (`CURLEVEL`).
    pub level: Option<String>,
}

impl NpcProps {
    /// Take every prop `other` carries.
    pub fn merge(&mut self, other: &NpcProps) {
        let o = other.clone();
        self.image = o.image.or(self.image.take());
        self.script = o.script.or(self.script.take());
        self.x = o.x.or(self.x);
        self.y = o.y.or(self.y);
        self.vis_flags = o.vis_flags.or(self.vis_flags);
        self.block_flags = o.block_flags.or(self.block_flags);
        self.message = o.message.or(self.message.take());
        self.gani = o.gani.or(self.gani.take());
        self.sprite = o.sprite.or(self.sprite);
        self.colors = o.colors.or(self.colors);
        self.nickname = o.nickname.or(self.nickname.take());
        self.head = o.head.or(self.head.take());
        self.body = o.body.or(self.body.take());
        self.level = o.level.or(self.level.take());
        for (a, b) in self.attrs.iter_mut().zip(o.attrs) {
            if b.is_some() {
                *a = b;
            }
        }
    }
}

/// Parse a `PLO_NPCPROPS` payload into the NPC id and its props. `bow_prop12`
/// reads prop 12 as the pre-2.1 bow (power, or `GCHAR 10 + len` and the
/// image) instead of a GANI name.
pub fn parse_npc_props(b: &[u8], bow_prop12: bool) -> Option<(u32, NpcProps)> {
    let id = read_gint3(b)? as u32;
    let mut p = NpcProps::default();
    let mut i = 3;
    let gc = |i: usize| b.get(i).map(|c| c.wrapping_sub(0x20) as usize);
    // A GCHAR length, then that many bytes.
    let text = |i: usize| -> Option<(String, usize)> {
        let n = gc(i)?;
        Some((latin1(b.get(i + 1..i + 1 + n)?), 1 + n))
    };
    while let Some(prop) = gc(i) {
        i += 1;
        match prop {
            0 => {
                let (s, n) = text(i)?;
                p.image = Some(s);
                i += n;
            }
            1 => {
                let n = read_gshort(b.get(i..)?)? as usize;
                p.script = Some(script_text(b.get(i + 2..i + 2 + n)?));
                i += 2 + n;
            }
            2 => {
                p.x = Some(gc(i)? as f32 / 2.0);
                i += 1;
            }
            3 => {
                p.y = Some(gc(i)? as f32 / 2.0);
                i += 1;
            }
            13 => {
                p.vis_flags = Some(gc(i)? as u8);
                i += 1;
            }
            14 => {
                p.block_flags = Some(gc(i)? as u8);
                i += 1;
            }
            15 => {
                let (s, n) = text(i)?;
                p.message = Some(s);
                i += n;
            }
            18 => {
                p.sprite = Some(gc(i)? as u8);
                i += 1;
            }
            19 => {
                let c = b.get(i..i + 5)?;
                p.colors = Some([0, 1, 2, 3, 4].map(|k| c[k].wrapping_sub(0x20)));
                i += 5;
            }
            20 => {
                let (s, n) = text(i)?;
                p.nickname = Some(s);
                i += n;
            }
            35 => {
                let (s, n) = text(i)?;
                p.body = Some(s);
                i += n;
            }
            36..=40 => {
                let (s, n) = text(i)?;
                p.attrs[prop - 36] = Some(s);
                i += n;
            }
            44..=47 | 53..=73 => {
                let (s, n) = text(i)?;
                p.attrs[if prop < 53 { prop - 44 + 5 } else { prop - 53 + 9 }] = Some(s);
                i += n;
            }
            52 => {
                let (s, n) = text(i)?;
                p.level = Some(s);
                i += n;
            }
            // Scripter, name, type: `GCHAR` length and the text.
            49..=51 => i += text(i)?.1,
            // Class list: `GSHORT` length and the text.
            74 => i += 2 + read_gshort(b.get(i..)?)? as usize,
            75 | 76 => {
                let v = Some(super::props::decode_pos_precise(read_gshort(b.get(i..)?)?));
                if prop == 75 {
                    p.x = v;
                } else {
                    p.y = v;
                }
                i += 2;
            }
            12 if !bow_prop12 => {
                let (s, n) = text(i)?;
                p.gani = Some(s);
                i += n;
            }
            // Power, arrows, bombs, glove and bomb power, saves,
            // alignment, gmap level x / y: one byte.
            4 | 6..=9 | 23..=33 | 41 | 42 => i += 1,
            // Rupees and id: GINT3.
            5 | 17 => i += 3,
            // Hurt dx / dy: two bytes. Image part: six.
            16 => i += 2,
            34 => i += 6,
            // Sword: power 0..=4, or 30 + power then the image.
            10 => {
                let sp = gc(i)?;
                i += 1;
                if sp > 4 {
                    i += text(i)?.1;
                }
            }
            // Shield: power 0..=3, or 10 + power then the image.
            11 => {
                let sp = gc(i)?;
                i += 1;
                if sp > 3 {
                    i += text(i)?.1;
                }
            }
            12 if bow_prop12 => {
                let sp = gc(i)?;
                i += 1 + sp.saturating_sub(10);
            }
            // Horse.
            21 => i += text(i)?.1,
            // Head: an index under 100, else 100 + the name's length.
            22 => {
                let len = gc(i)?;
                if len < 100 {
                    p.head = Some(Head::Index(len as u8));
                    i += 1;
                } else {
                    p.head = Some(Head::Name(latin1(b.get(i + 1..i + 1 + len - 100)?)));
                    i += 1 + len - 100;
                }
            }
            _ => break,
        }
        if i > b.len() {
            return None;
        }
    }
    Some((id, p))
}

/// One prop value for [`npc_props_message`].
#[derive(Debug, Clone, PartialEq)]
pub enum PropValue {
    /// One `GCHAR`.
    Byte(u8),
    /// Several `GCHAR`s (COLORS).
    Bytes(Vec<u8>),
    /// HEAD.
    Head(Head),
    /// `GCHAR` length, then the text (Latin-1, at most 223 bytes).
    Text(String),
}

/// The `PLI_NPCPROPS` (3) message: `GINT3 id`, then each prop id and its
/// value, and the newline. The 2.05 client sends the props of a server NPC
/// that its client-side script changed, in prop id order: IMAGE after
/// `setimg`, X and Y (half tiles) after a push, a pull or `x = …`,
/// VISFLAGS, BLOCKFLAGS after `canbepushed` and the like, MESSAGE.
pub fn npc_props_message(id: u32, props: &[(u8, PropValue)]) -> Vec<u8> {
    let mut m = vec![3 + 0x20, ((id >> 14) & 0x7F) as u8 + 0x20, ((id >> 7) & 0x7F) as u8 + 0x20, (id & 0x7F) as u8 + 0x20];
    for (prop, v) in props {
        m.push(prop + 0x20);
        match v {
            PropValue::Byte(b) => m.push((*b).min(223) + 0x20),
            PropValue::Bytes(bs) => m.extend(bs.iter().map(|b| (*b).min(223) + 0x20)),
            PropValue::Text(t) => {
                let bytes = latin1_bytes(t, 223);
                m.push(bytes.len() as u8 + 0x20);
                m.extend(bytes);
            }
            PropValue::Head(Head::Index(i)) => m.push((*i).min(99) + 0x20),
            PropValue::Head(Head::Name(n)) => {
                let bytes = latin1_bytes(n, 123);
                m.push(bytes.len() as u8 + 100 + 0x20);
                m.extend(bytes);
            }
        }
    }
    m.push(b'\n');
    m
}

fn latin1_bytes(t: &str, max: usize) -> Vec<u8> {
    let mut bytes: Vec<u8> = t.chars().map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')).collect();
    bytes.truncate(max);
    bytes
}

/// A position in tiles as the `GCHAR` half tiles of `NPCPROP_X` / `Y`.
pub fn half_tiles(v: f32) -> u8 {
    (v * 2.0).round().clamp(0.0, 223.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gc(n: usize) -> u8 {
        n as u8 + 0x20
    }

    /// The record the lab stub sent the 2.05 client: id 1001, image, script
    /// (with `§` breaks), x 30, y 25.
    fn lab_record() -> Vec<u8> {
        let mut p = vec![gc(0), gc(7), gc(105)];
        p.extend([gc(0), gc(12)]);
        p.extend_from_slice(b"labblock.gif");
        let script = b"if (created) { canbepushed; }\xa7";
        p.extend([gc(1), gc(0), gc(script.len())]);
        p.extend_from_slice(script);
        p.extend([gc(2), gc(60), gc(3), gc(50)]);
        p
    }

    #[test]
    fn a_full_record_gives_id_image_script_and_position() {
        let (id, p) = parse_npc_props(&lab_record(), true).unwrap();
        assert_eq!(id, 1001);
        assert_eq!(p.image.as_deref(), Some("labblock.gif"));
        assert_eq!(p.script.as_deref(), Some("if (created) { canbepushed; }\n"));
        assert_eq!((p.x, p.y), (Some(30.0), Some(25.0)));
        assert_eq!(p.block_flags, None);
    }

    #[test]
    fn skipped_props_keep_the_reader_in_step() {
        // Sword with an image, shield by power, rupees, colours, a message,
        // a head by name, then the position.
        let mut p = vec![gc(0), gc(0), gc(5)];
        p.extend([gc(10), gc(31), gc(3)]);
        p.extend_from_slice(b"s.p");
        p.extend([gc(11), gc(2), gc(5), gc(0), gc(1), gc(2), gc(19), 1, 2, 3, 4, 5]);
        p.extend([gc(15), gc(2)]);
        p.extend_from_slice(b"hi");
        p.extend([gc(22), gc(103)]);
        p.extend_from_slice(b"h.p");
        p.extend([gc(14), gc(12), gc(2), gc(10), gc(3), gc(11)]);
        let (id, p) = parse_npc_props(&p, true).unwrap();
        assert_eq!(id, 5);
        assert_eq!((p.x, p.y, p.block_flags), (Some(5.0), Some(5.5), Some(12)));
    }

    #[test]
    fn an_unknown_prop_ends_the_record_and_a_short_one_is_dropped() {
        let p = [gc(0), gc(0), gc(1), gc(2), gc(8), gc(77), gc(1), gc(3), gc(9)];
        assert_eq!(parse_npc_props(&p, true).unwrap().1.x, Some(4.0));
        assert_eq!(parse_npc_props(&p[..2], true), None);
        assert_eq!(parse_npc_props(&[gc(0), gc(0), gc(1), gc(0), gc(9), b'a'], true), None);
    }

    #[test]
    fn prop_12_is_the_bow_before_2_1_and_a_gani_after() {
        let p = [gc(0), gc(0), gc(1), gc(12), gc(12), b'b', b'w', gc(2), gc(4)];
        assert_eq!(parse_npc_props(&p, true).unwrap().1.x, Some(2.0));
        let p = [gc(0), gc(0), gc(1), gc(12), gc(4), b'i', b'd', b'l', b'e', gc(2), gc(4)];
        assert_eq!(parse_npc_props(&p, false).unwrap().1.x, Some(2.0));
    }

    #[test]
    fn the_message_matches_the_2_05_client() {
        // Lab capture: after a push up the 2.05 client sent `3`, id 1001,
        // Y = 48 half tiles; after `canbepushed; canbepulled;` BLOCKFLAGS 12.
        assert_eq!(npc_props_message(1001, &[(NPCPROP_Y, PropValue::Byte(48))]), b"# '\x89#P\n");
        assert_eq!(npc_props_message(1001, &[(NPCPROP_BLOCKFLAGS, PropValue::Byte(12))]), b"# '\x89.,\n");
        // NPC 1121 (`message hi;`): VISFLAGS 81, MESSAGE "hi".
        let m = npc_props_message(1121, &[(NPCPROP_VISFLAGS, PropValue::Byte(81)), (NPCPROP_MESSAGE, PropValue::Text("hi".into()))]);
        assert_eq!(m, b"# (\x81-q/\"hi\n");
        // NPC 1118 (`setimg bomb1.png;`): IMAGE, then VISFLAGS.
        let m = npc_props_message(1118, &[(NPCPROP_IMAGE, PropValue::Text("bomb1.png".into())), (NPCPROP_VISFLAGS, PropValue::Byte(81))]);
        assert_eq!(m, b"# (~ )bomb1.png-q\n");
    }

    #[test]
    fn character_props_read_and_write_as_the_2_05_client_sent_them() {
        // NPC 3002 (`showcharacter; setcharprop #3,head7.png; #8,body2.png`).
        let props = [
            (0, PropValue::Text("#c#".into())),
            (NPCPROP_GANI, PropValue::Text("idle".into())),
            (NPCPROP_VISFLAGS, PropValue::Byte(81)),
            (NPCPROP_HEAD, PropValue::Head(Head::Name("head7.png".into()))),
            (NPCPROP_BODY, PropValue::Text("body2.png".into())),
        ];
        let m = npc_props_message(3002, &props);
        assert_eq!(m, b"# 7Z ##c#,$idle-q6\x8dhead7.pngC)body2.png\n");
        let (id, p) = parse_npc_props(&m[1..m.len() - 1], false).unwrap();
        assert_eq!(id, 3002);
        assert_eq!((p.image.as_deref(), p.gani.as_deref(), p.body.as_deref()), (Some("#c#"), Some("idle"), Some("body2.png")));
        assert_eq!(p.head, Some(Head::Name("head7.png".into())));
        // NPC 3005: colours; 3001: head index 0; 3006: SPRITE 1; 3007: #P1.
        let m = npc_props_message(1, &[(NPCPROP_COLORS, PropValue::Bytes(vec![4, 10, 18, 5, 18])), (NPCPROP_HEAD, PropValue::Head(Head::Index(0)))]);
        assert_eq!(m, b"#  !3$*2%26 \n");
        let mut tail = vec![gc(0), gc(0), gc(1)];
        tail.extend_from_slice(&m[4..m.len() - 1]);
        tail.extend([gc(18), gc(1), gc(36), gc(8)]);
        tail.extend_from_slice(b"hat0.png");
        let (_, p) = parse_npc_props(&tail, false).unwrap();
        assert_eq!((p.colors, p.head, p.sprite), (Some([4, 10, 18, 5, 18]), Some(Head::Index(0)), Some(1)));
        assert_eq!(p.attrs[0].as_deref(), Some("hat0.png"));
    }

    #[test]
    fn the_message_prop_is_read() {
        let p = [gc(0), gc(0), gc(1), gc(15), gc(4), b't', b'i', b'c', b'k', gc(13), gc(81)];
        let (_, p) = parse_npc_props(&p, true).unwrap();
        assert_eq!((p.message.as_deref(), p.vis_flags), (Some("tick"), Some(81)));
    }

    /// The order GServer-v2 sends a moved NPC in: GANI attributes 1 to 30
    /// (6 to 30 past prop 43), the saves, the gmap level, then X2 and Y2.
    /// The reader used to stop at prop 44 and drop the rest.
    #[test]
    fn props_past_43_are_read_and_keep_the_reader_in_step() {
        let mut p = vec![gc(0), gc(0), gc(9)];
        let attr = |p: &mut Vec<u8>, prop: usize, t: &[u8]| {
            p.extend([gc(prop), gc(t.len())]);
            p.extend_from_slice(t);
        };
        attr(&mut p, 36, b"a1.png");
        attr(&mut p, 44, b"a6.png");
        attr(&mut p, 47, b"a9.png");
        attr(&mut p, 53, b"a10.png");
        attr(&mut p, 73, b"a30.png");
        attr(&mut p, 49, b"someone");
        attr(&mut p, 50, b"door");
        attr(&mut p, 51, b"LOCALN");
        attr(&mut p, 52, b"onlinestartlocal.nw");
        // Class list: GSHORT length.
        p.extend([gc(74), gc(0), gc(3)]);
        p.extend_from_slice(b"a,b");
        p.extend([gc(23), gc(7), gc(41), gc(1), gc(42), gc(0)]);
        // X2 = 30.5 tiles = 488 px, Y2 = -2 tiles = -32 px.
        let x2 = 488u16 << 1;
        let y2 = (32u16 << 1) | 1;
        p.extend([gc(75), gc((x2 >> 7) as usize), gc((x2 & 0x7F) as usize)]);
        p.extend([gc(76), gc((y2 >> 7) as usize), gc((y2 & 0x7F) as usize)]);
        p.extend([gc(15), gc(2)]);
        p.extend_from_slice(b"hi");
        let (id, n) = parse_npc_props(&p, false).unwrap();
        assert_eq!(id, 9);
        let a = |i: usize| n.attrs[i - 1].as_deref();
        assert_eq!((a(1), a(6), a(9), a(10), a(30), a(7)), (Some("a1.png"), Some("a6.png"), Some("a9.png"), Some("a10.png"), Some("a30.png"), None));
        assert_eq!(n.level.as_deref(), Some("onlinestartlocal.nw"));
        assert_eq!((n.x, n.y), (Some(30.5), Some(-2.0)));
        assert_eq!(n.message.as_deref(), Some("hi"));
    }

    #[test]
    fn props_43_48_and_past_76_still_end_the_record() {
        for prop in [43, 48, 77] {
            let p = vec![gc(0), gc(0), gc(9), gc(prop), gc(1), gc(15), gc(1), b'x'];
            let (_, n) = parse_npc_props(&p, false).unwrap();
            assert_eq!(n.message, None, "prop {prop}");
        }
    }

    #[test]
    fn merge_keeps_what_an_update_leaves_out() {
        let (_, mut full) = parse_npc_props(&lab_record(), true).unwrap();
        full.merge(&NpcProps { y: Some(24.0), ..NpcProps::default() });
        assert_eq!((full.x, full.y), (Some(30.0), Some(24.0)));
        assert!(full.image.is_some() && full.script.is_some());
    }
}
