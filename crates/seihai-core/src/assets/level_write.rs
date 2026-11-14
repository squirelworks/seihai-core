//! Binary level writer: `GR-V1.03` (`.graal`) and `Z3-V1.04` (`.zelda`).
//!
//! The inverse of the binary reader in `level.rs`, after the layout in the
//! format notes (`formats/level-binary.md` in Graal_analysis): an 8-byte
//! magic, the layer-0 board as a run-length bit stream (13-bit codes for
//! `GR-V1.03`, 12-bit for `Z3-V1.04`, low bit first), then links, baddies
//! (three raw bytes and a verse line, ended by `FF FF FF`), NPCs and chests
//! (`GR-V1` only), and signs in the sign alphabet.
//!
//! The board writer follows the 2.05 editor's rule: a single run for 2 to
//! 64 equal tiles, a double run for an `A B A B` pattern, a plain code
//! otherwise. What a binary level cannot hold is reported, not invented:
//! layers above 0, NPCs and chests in `Z3-V1`, tile ids over 2047 in
//! `Z3-V1`, positions outside 0 to 223, baddy types that are not numbers.

use super::level::{Level, CHEST_ITEMS, SIGN_ALPHABET, SIGN_GLYPHS};

/// Which binary level to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryLevel {
    /// `GR-V1.03`, what the 2.05 and 2.22 editors write for a `.graal`.
    Graal,
    /// `Z3-V1.04`, the `.zelda` level with baddy verses.
    Zelda,
}

impl BinaryLevel {
    pub fn magic(self) -> &'static [u8; 8] {
        match self {
            BinaryLevel::Graal => b"GR-V1.03",
            BinaryLevel::Zelda => b"Z3-V1.04",
        }
    }

    fn code_bits(self) -> u32 {
        match self {
            BinaryLevel::Graal => 13,
            BinaryLevel::Zelda => 12,
        }
    }
}

/// Bytes and what did not fit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Written {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
}

/// Pack a 64 by 64 board as run-length codes, `bits` wide, low bit first.
pub fn encode_board(tiles: &[u16], bits: u32) -> Vec<u8> {
    let flag = 1u32 << (bits - 1);
    let mut codes: Vec<u32> = Vec::new();
    let mut i = 0;
    while i < tiles.len() {
        if i + 1 < tiles.len() && tiles[i] == tiles[i + 1] {
            let mut n = 1;
            while i + n < tiles.len() && tiles[i + n] == tiles[i] && n < 64 {
                n += 1;
            }
            codes.push(flag | n as u32);
            codes.push(u32::from(tiles[i]));
            i += n;
        } else if i + 3 < tiles.len() && tiles[i] == tiles[i + 2] && tiles[i + 1] == tiles[i + 3] {
            let mut n = 1;
            while i + 2 * n + 1 < tiles.len()
                && tiles[i + 2 * n] == tiles[i]
                && tiles[i + 2 * n + 1] == tiles[i + 1]
                && n < 64
            {
                n += 1;
            }
            codes.push(flag | 0x100 | n as u32);
            codes.push(u32::from(tiles[i]));
            codes.push(u32::from(tiles[i + 1]));
            i += 2 * n;
        } else {
            codes.push(u32::from(tiles[i]));
            i += 1;
        }
    }
    let (mut out, mut buf, mut have) = (Vec::new(), 0u64, 0u32);
    for c in codes {
        buf |= u64::from(c) << have;
        have += bits;
        while have >= 8 {
            out.push(buf as u8);
            buf >>= 8;
            have -= 8;
        }
    }
    if have > 0 {
        out.push(buf as u8);
    }
    out
}

/// A `#` slot of the alphabet that is no glyph, used to start a `#K(n)`
/// escape.
const PLAIN_HASH: u8 = 86;

/// Sign text in the packed alphabet (each byte `index + 32`), the inverse
/// of [`super::decode_sign_text`]: glyph tags (`#A`, `#h`, ...) become
/// their codes, characters outside the alphabet become `#K(n)`.
pub fn encode_sign_text(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '#' {
            if let Some(&n) = chars.get(i + 1) {
                let tag: String = ['#', n].iter().collect();
                if let Some((codes, _)) = SIGN_GLYPHS.iter().find(|(_, t)| *t == tag) {
                    out.extend(codes.iter().map(|&k| k + 32));
                    i += 2;
                    continue;
                }
            }
            out.push(PLAIN_HASH + 32);
            i += 1;
            continue;
        }
        let slot = SIGN_ALPHABET
            .iter()
            .enumerate()
            .find(|&(_, &a)| a != b'#' && char::from(a) == c);
        match slot {
            Some((k, _)) => out.push(k as u8 + 32),
            None => {
                let n = u32::from(c).min(255);
                out.push(PLAIN_HASH + 32);
                for ch in format!("K({n})").chars() {
                    let k = SIGN_ALPHABET
                        .iter()
                        .position(|&a| a != b'#' && char::from(a) == ch)
                        .expect("K, ( ) and digits are in the alphabet");
                    out.push(k as u8 + 32);
                }
            }
        }
        i += 1;
    }
    out
}

fn latin1(s: &str, out: &mut Vec<u8>) {
    out.extend(
        s.chars()
            .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')),
    );
}

fn number(n: f32) -> String {
    if n.is_finite() && n.fract() == 0.0 {
        format!("{}", n as i32)
    } else {
        n.to_string()
    }
}

/// A link's arrival coordinate. `Z3-V1` readers want a number, and a
/// negative one keeps the player's coordinate, so `playerx` is `-1` there
/// (an offset after it cannot be stored and is dropped).
fn arrival(abs: Option<f32>, offset: f32, keep: &str, zelda: bool) -> String {
    match abs {
        Some(n) => number(n),
        None if zelda => "-1".to_string(),
        None if offset == 0.0 => keep.to_string(),
        None if offset > 0.0 => format!("{keep}+{}", number(offset)),
        None => format!("{keep}{}", number(offset)),
    }
}

/// One whole-tile coordinate as a `GCHAR` (`n + 32`), if it fits.
fn gchar(n: f32) -> Option<u8> {
    let n = n.round();
    (0.0..=223.0).contains(&n).then(|| n as u8 + 32)
}

/// Write `level` as a binary level.
pub fn encode_binary(level: &Level, format: BinaryLevel) -> Written {
    let mut w = Written::default();
    let zelda = format == BinaryLevel::Zelda;
    let out = &mut w.bytes;
    let warn = &mut w.warnings;
    out.extend_from_slice(format.magic());

    if level.boards.iter().any(|r| r.layer != 0) {
        warn.push("layers above 0 are not kept: a binary level has one layer".into());
    }
    let mut board: Vec<u16> = level.layer0_board();
    if zelda {
        let over = board.iter().filter(|&&t| t > 2047).count();
        if over > 0 {
            warn.push(format!(
                "{over} tiles above 2047 kept as their low 11 bits (Z3-V1 codes are 12-bit)"
            ));
            for t in &mut board {
                *t &= 0x7FF;
            }
        }
    }
    out.extend(encode_board(&board, format.code_bits()));

    for link in &level.links {
        let offset = (link.new_x.is_none() && link.x_offset != 0.0)
            || (link.new_y.is_none() && link.y_offset != 0.0);
        if zelda && offset {
            warn.push(format!(
                "link to {}: the offset after playerx/playery is dropped (Z3-V1 stores -1)",
                link.dest
            ));
        }
        let line = format!(
            "{} {} {} {} {} {} {}\n",
            link.dest,
            link.x,
            link.y,
            link.width,
            link.height,
            arrival(link.new_x, link.x_offset, "playerx", zelda),
            arrival(link.new_y, link.y_offset, "playery", zelda),
        );
        latin1(&line, out);
    }
    out.extend_from_slice(b"#\n");

    for b in &level.baddies {
        // Raw bytes; a first byte of 0x80 or more would end the section.
        let byte = |n: f32| {
            let n = n.round();
            (0.0..=127.0).contains(&n).then_some(n as u8)
        };
        let (Some(x), Some(y)) = (byte(b.x), byte(b.y)) else {
            warn.push(format!(
                "baddy at {},{}: outside 0 to 127, dropped",
                b.x, b.y
            ));
            continue;
        };
        let Ok(kind) = b.kind.trim().parse::<u8>() else {
            warn.push(format!(
                "baddy type \"{}\" is not a number, dropped",
                b.kind
            ));
            continue;
        };
        out.extend_from_slice(&[x, y, kind]);
        latin1(&b.verses.replace('\n', "\\"), out);
        out.push(b'\n');
    }
    out.extend_from_slice(b"\xff\xff\xff\n");

    if zelda {
        if !level.npcs.is_empty() {
            warn.push(format!(
                "{} NPCs dropped: Z3-V1 has no NPC section",
                level.npcs.len()
            ));
        }
        if !level.chests.is_empty() {
            warn.push(format!(
                "{} chests dropped: Z3-V1 has no chest section",
                level.chests.len()
            ));
        }
    } else {
        for npc in &level.npcs {
            let (Some(x), Some(y)) = (gchar(npc.x), gchar(npc.y)) else {
                warn.push(format!(
                    "NPC at {},{}: outside 0 to 223, dropped",
                    npc.x, npc.y
                ));
                continue;
            };
            out.extend_from_slice(&[x, y]);
            latin1(&npc.image, out);
            out.push(b'#');
            let script: String = npc
                .script
                .replace("\r\n", "\n")
                .chars()
                .map(|c| if c == '\n' { '\u{a7}' } else { c })
                .collect();
            latin1(&script, out);
            out.push(b'\n');
        }
        out.extend_from_slice(b"#\n");
        for chest in &level.chests {
            let item = CHEST_ITEMS
                .iter()
                .position(|n| n.eq_ignore_ascii_case(&chest.item))
                .or_else(|| chest.item.parse::<usize>().ok().filter(|&i| i < 224));
            let (Some(x), Some(y), Some(item), Some(sign)) = (
                gchar(chest.x as f32),
                gchar(chest.y as f32),
                item,
                gchar(chest.sign_index as f32),
            ) else {
                warn.push(format!(
                    "chest at {},{} ({}): cannot be stored, dropped",
                    chest.x, chest.y, chest.item
                ));
                continue;
            };
            out.extend_from_slice(&[x, y, item as u8 + 32, sign]);
            out.push(b'\n');
        }
        out.extend_from_slice(b"#\n");
    }

    for sign in &level.signs {
        let (Some(x), Some(y)) = (gchar(sign.x as f32), gchar(sign.y as f32)) else {
            warn.push(format!(
                "sign at {},{}: outside 0 to 223, dropped",
                sign.x, sign.y
            ));
            continue;
        };
        out.extend_from_slice(&[x, y]);
        out.extend(encode_sign_text(&sign.text));
        out.push(b'\n');
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::decode_sign_text;

    const TEXT: &str = "GLEVNW01
BOARD 0 0 4 0 AAABAAAB
BOARD 0 1 3 0 /w/w/w
LINK next.nw 63 0 1 64 0 playery
LINK house.nw 30 30 2 1 30.5 playery+10
NPC block.png 10 12
if (playertouchsme) {
  say2 Hello;
}
NPCEND
BADDY 20 21 3
see you
ouch
attack
BADDYEND
CHEST 5 6 bombs 0
SIGN 7 8
Hello, #A world?
New line!
SIGNEND
";

    #[test]
    fn graal_round_trips_through_the_reader() {
        let level = Level::parse(TEXT).unwrap();
        let w = encode_binary(&level, BinaryLevel::Graal);
        assert!(w.warnings.is_empty(), "{:?}", w.warnings);
        assert_eq!(&w.bytes[..8], b"GR-V1.03");
        let back = Level::from_bytes(&w.bytes).unwrap();
        assert_eq!(back.layer0_board(), level.layer0_board());
        assert_eq!(back.links.len(), 2);
        assert_eq!(back.links[1].new_x, Some(30.5));
        assert_eq!(back.links[1].y_offset, 10.0);
        assert_eq!(back.npcs.len(), 1);
        assert_eq!(back.npcs[0].image, "block.png");
        assert_eq!((back.npcs[0].x, back.npcs[0].y), (10.0, 12.0));
        assert_eq!(back.npcs[0].script.trim(), level.npcs[0].script.trim());
        assert_eq!(back.baddies.len(), 1);
        assert_eq!(back.baddies[0].kind, "3");
        assert_eq!(back.baddies[0].verses, "see you\nouch\nattack");
        assert_eq!(back.chests.len(), 1);
        assert_eq!(back.chests[0].item, "bombs");
        assert_eq!(back.signs.len(), 1);
        assert_eq!(back.signs[0].text, level.signs[0].text);
    }

    #[test]
    fn zelda_drops_what_it_cannot_hold_and_says_so() {
        let mut level = Level::parse(TEXT).unwrap();
        level.boards.push(crate::assets::BoardRun {
            x: 3,
            y: 3,
            width: 1,
            layer: 0,
            tiles: vec![crate::assets::tile_code(3000)],
        });
        let w = encode_binary(&level, BinaryLevel::Zelda);
        assert_eq!(&w.bytes[..8], b"Z3-V1.04");
        assert_eq!(w.warnings.len(), 4, "{:?}", w.warnings);
        let back = Level::from_bytes(&w.bytes).unwrap();
        assert_eq!(back.tile_id_at(3, 3), Some(3000 & 0x7FF));
        assert_eq!(back.tile_id_at(0, 1), Some(4080 & 0x7FF));
        assert!(back.npcs.is_empty() && back.chests.is_empty());
        assert_eq!(back.links.len(), 2);
        assert_eq!(back.baddies[0].verses, "see you\nouch\nattack");
        assert_eq!(back.signs[0].text, level.signs[0].text);
    }

    #[test]
    fn sign_text_round_trips_glyphs_and_escapes() {
        for text in ["Hi #A and #h!", "a = b + 5%", "#K is #", "line\nline"] {
            assert_eq!(decode_sign_text(&encode_sign_text(text)), text, "{text:?}");
        }
    }

    #[test]
    fn long_runs_and_patterns_use_runs() {
        let mut board = vec![7u16; 4096];
        for (i, t) in board.iter_mut().enumerate().take(64) {
            *t = if i % 2 == 0 { 1 } else { 2 };
        }
        let bytes = encode_board(&board, 13);
        // 64 tiles of ABAB (one double run, 3 codes) and 63 runs of 64.
        assert_eq!(bytes.len(), ((3 + 63 * 2) * 13usize).div_ceil(8));
        let mut file = b"GR-V1.03".to_vec();
        file.extend(bytes);
        file.extend_from_slice(b"#\n\xff\xff\xff\n#\n#\n");
        assert_eq!(Level::from_bytes(&file).unwrap().layer0_board(), board);
    }
}
