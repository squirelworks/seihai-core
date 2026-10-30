//! Z3-V1 levels (`.zelda`): the format of Zelda Online and the earliest
//! Graal builds.
//!
//! A Z3-V1 file is a `GR-V1` file without NPCs and chests:
//!
//! ```text
//! char[8]  magic      Z3-V1.03 | Z3-V1.04   (Z3-V1.02 is the Java client only)
//! bits     board      4096 tiles, packed run-length codes (12 bits; 11 for .02)
//! text     links      lines `dest x y w h newx newy`, up to "#" or an empty line
//! binary   computers  records `x y type` + a line end (+ verse text, Z3-V1.04),
//!                     up to a record whose first byte is 0x80 or more
//! text     signs      lines `x+32 y+32 text`, up to an empty line or the end
//! ```
//!
//! "Computer" is the Zelda Online word for what Graal later calls a baddy.
//!
//! The format is settled from three sources that agree: the Java Zelda
//! Online client source shipped in the Drive set, the 1st Graal client and
//! the 1.39 client (both Delphi, read from decompiles for derived facts
//! only), and every real file in the Drive set. The extension does not
//! decide anything: 119 `Z3-V1.04` files are named `.graal`, and 16 `Z3-V1.03`
//! files are named `.editor`; the clients read the magic.
//!
//! This module reads strictly and says what it saw ([`ZeldaLayout`]); it
//! invents nothing. Unknown bytes stay unknown: [`Computer::extra`] and
//! [`ZeldaSign::raw`] keep what the file held. `anomalies` lists every
//! place where a file leaves the layout above. A clean file reaches the end
//! of the file exactly.

use std::collections::BTreeMap;

use super::level::{board_runs, decode_sign_record, Baddy, GraalLayout, Level, Link, Sign};

/// Tiles in a board (64 × 64).
pub const BOARD_TILES: usize = 4096;

/// The Z3-V1 revisions that exist as files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ZeldaMagic {
    /// 11-bit board codes. Read by the Java Zelda Online client only; no
    /// Graal client accepts it and no real file in the Drive set has it.
    V102,
    /// 12-bit board codes; computers are `x y type` and a line end.
    V103,
    /// As 1.03, and each computer carries a verse line (`sight\hurt\attack`).
    V104,
}

impl ZeldaMagic {
    /// The magic of a file, if it is a Z3-V1 magic we know.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        match data.get(..8)? {
            b"Z3-V1.02" => Some(Self::V102),
            b"Z3-V1.03" => Some(Self::V103),
            b"Z3-V1.04" => Some(Self::V104),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::V102 => "Z3-V1.02",
            Self::V103 => "Z3-V1.03",
            Self::V104 => "Z3-V1.04",
        }
    }

    /// Bits per board code.
    pub fn code_bits(self) -> u32 {
        match self {
            Self::V102 => 11,
            _ => 12,
        }
    }

    /// Computers have a verse line from Z3-V1.04.
    pub fn has_verses(self) -> bool {
        self == Self::V104
    }

    /// Do the Graal clients (1st Graal to 2.31) accept it? They compare the
    /// magic against `Z3-V1.03` and `Z3-V1.04` only.
    pub fn graal_clients_read(self) -> bool {
        self != Self::V102
    }
}

/// True if `data` starts with a `Z3-V1` magic of any revision.
pub fn is_zelda(data: &[u8]) -> bool {
    data.starts_with(b"Z3-V1")
}

/// A link line as written: `dest x y w h newx newy`, numbers as decimals.
#[derive(Debug, Clone, PartialEq)]
pub struct ZeldaLink {
    pub dest: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub new_x: f64,
    pub new_y: f64,
}

impl ZeldaLink {
    /// A negative arrival coordinate means "keep the player's own". The Java
    /// client and 1st Graal both set a "constant" flag from `< 0`.
    pub fn keeps_x(&self) -> bool {
        self.new_x < 0.0
    }

    pub fn keeps_y(&self) -> bool {
        self.new_y < 0.0
    }

    /// Does any of x, y, width, height have a fractional part? None did in
    /// the Drive set; [`Link`] holds whole tiles.
    pub fn fractional_geometry(&self) -> bool {
        [self.x, self.y, self.width, self.height].iter().any(|v| v.fract() != 0.0)
    }

    /// As the level's [`Link`]: geometry in whole tiles (rounded down), a
    /// negative arrival as "keep the player's coordinate".
    pub fn to_link(&self) -> Link {
        Link {
            dest: self.dest.clone(),
            x: self.x.floor() as i32,
            y: self.y.floor() as i32,
            width: self.width.floor() as i32,
            height: self.height.floor() as i32,
            new_x: (!self.keeps_x()).then_some(self.new_x as f32),
            new_y: (!self.keeps_y()).then_some(self.new_y as f32),
            x_offset: 0.0,
            y_offset: 0.0,
        }
    }

    fn parse(line: &str) -> Option<Self> {
        let parts: Vec<&str> = line.split(' ').filter(|p| !p.is_empty()).collect();
        if parts.len() < 7 {
            return None;
        }
        let n = parts.len();
        let num = |i: usize| parts[i].parse::<f64>().ok().filter(|v| v.is_finite());
        Some(ZeldaLink {
            dest: parts[..n - 6].join(" "),
            x: num(n - 6)?,
            y: num(n - 5)?,
            width: num(n - 4)?,
            height: num(n - 3)?,
            new_x: num(n - 2)?,
            new_y: num(n - 1)?,
        })
    }

    fn write(&self) -> String {
        format!("{} {:?} {:?} {:?} {:?} {:?} {:?}", self.dest, self.x, self.y, self.width, self.height, self.new_x, self.new_y)
    }
}

/// An enemy record. `x` and `y` are whole tiles stored as raw bytes (not
/// the +32 of signs). `kind` is 0 to 6 in every real file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Computer {
    pub x: u8,
    pub y: u8,
    pub kind: u8,
    /// Whatever followed the three bytes on the record's line: empty in
    /// Z3-V1.03, the verse text (`sight\hurt\attack`) in Z3-V1.04.
    pub extra: Vec<u8>,
}

impl Computer {
    /// Names the Java source gives to types 0 to 6. Later builds' names for
    /// the same numbers are not claimed.
    pub fn java_type_name(kind: u8) -> Option<&'static str> {
        Some(match kind {
            0 => "soldier-green",
            1 => "soldier-blue",
            2 => "soldier-red",
            3 => "shot-soldier",
            4 => "swamp-soldier",
            5 => "jumping-hare",
            6 => "octopus",
            _ => return None,
        })
    }

    /// The verse text split at `\`, as the 1st Graal client splits it into
    /// sight, hurt and attack. Empty when the record has no verse text.
    pub fn verses(&self) -> Vec<String> {
        if self.extra.is_empty() {
            return Vec::new();
        }
        let text: String = self.extra.iter().map(|&b| b as char).collect();
        text.splitn(3, '\\').map(str::to_string).collect()
    }
}

/// A sign record: position in tiles and the packed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZeldaSign {
    pub x: i32,
    pub y: i32,
    /// The bytes after x and y, exactly as stored (alphabet index + 32).
    pub raw: Vec<u8>,
    /// `raw` decoded with the sign alphabet shared with `.graal` files.
    pub text: String,
}

/// Where each part of a Z3-V1 file sits and what was found in it. Counts and
/// offsets only, never content, except `anomalies`, whose lines say what is
/// off in words.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ZeldaLayout {
    pub magic: String,
    pub code_bits: u32,
    pub file_len: usize,
    pub codes: usize,
    pub single_runs: usize,
    pub double_runs: usize,
    pub max_run: u32,
    /// Control codes that arrived while an earlier double-run flag was still
    /// set. The clients keep the flag; see `documentation/assets.md`.
    pub flags_kept: usize,
    pub tiles_decoded: usize,
    pub max_tile: u16,
    pub board_end: usize,
    pub links: (usize, usize),
    pub link_lines: usize,
    /// The links section ended on a `#` line (else on an empty line or the end).
    pub link_hash_end: bool,
    pub link_fractional_numbers: usize,
    pub link_keep_x: usize,
    pub link_keep_y: usize,
    pub computers: (usize, usize),
    pub computer_records: usize,
    /// Computers per type byte.
    pub computer_kinds: BTreeMap<u8, usize>,
    /// Records with text after the three bytes.
    pub computer_extra: usize,
    /// The `FF FF FF` line that ends the section was found.
    pub computer_terminator: bool,
    pub signs: (usize, usize),
    pub sign_records: usize,
    /// Bytes left after the signs section stopped.
    pub unread_tail: usize,
    pub anomalies: Vec<String>,
}

impl ZeldaLayout {
    /// The file parsed to exactly its end with no anomaly.
    pub fn is_clean(&self) -> bool {
        self.anomalies.is_empty() && self.unread_tail == 0
    }

    /// Short derived summary (sizes, offsets, counts). Backs
    /// `seihai --level-info` for Z3-V1 files.
    pub fn summary(&self) -> String {
        let span = |r: (usize, usize)| format!("{}..{} ({} bytes)", r.0, r.1, r.1 - r.0);
        let mut o = String::new();
        o.push_str(&format!("magic {}  code bits {}  file {} bytes  (Z3-V1: no npcs, no chests)\n", self.magic, self.code_bits, self.file_len));
        o.push_str(&format!(
            "board: {} tiles from {} codes ({} single runs, {} double runs, longest run {}), ends at byte {}, highest tile {}\n",
            self.tiles_decoded, self.codes, self.single_runs, self.double_runs, self.max_run, self.board_end, self.max_tile
        ));
        o.push_str(&format!(
            "links     {}: {} lines, ended by {}; {} negative arrival x, {} negative arrival y, {} fractional numbers\n",
            span(self.links),
            self.link_lines,
            if self.link_hash_end { "#" } else { "an empty line or the end" },
            self.link_keep_x,
            self.link_keep_y,
            self.link_fractional_numbers
        ));
        let kinds: Vec<String> = self.computer_kinds.iter().map(|(k, n)| format!("type {k} x{n}")).collect();
        o.push_str(&format!(
            "computers {}: {} records ({}), {} with text, terminator {}\n",
            span(self.computers),
            self.computer_records,
            if kinds.is_empty() { "none".to_string() } else { kinds.join(", ") },
            self.computer_extra,
            if self.computer_terminator { "found" } else { "missing" }
        ));
        o.push_str(&format!("signs     {}: {} records\n", span(self.signs), self.sign_records));
        o.push_str(&format!("unread tail: {} bytes\n", self.unread_tail));
        if self.anomalies.is_empty() {
            o.push_str("anomalies: none\n");
        } else {
            for a in &self.anomalies {
                o.push_str(&format!("anomaly: {a}\n"));
            }
        }
        o
    }

    /// The same facts in the layout record `.graal` levels use, so a caller
    /// that holds a [`GraalLayout`] can show a Z3-V1 file too. Computers are
    /// reported as baddies; the NPC and chest ranges are empty.
    pub fn to_graal_layout(&self) -> GraalLayout {
        GraalLayout {
            magic: self.magic.clone(),
            code_bits: self.code_bits,
            file_len: self.file_len,
            codes: self.codes,
            single_runs: self.single_runs,
            double_runs: self.double_runs,
            max_run: self.max_run,
            tiles_decoded: self.tiles_decoded,
            max_tile: self.max_tile,
            board_end: self.board_end,
            links: self.links,
            baddies: self.computers,
            npcs: (self.computers.1, self.computers.1),
            chests: (self.computers.1, self.computers.1),
            signs: self.signs,
            link_lines: self.link_lines,
            baddy_records: self.computer_records,
            npc_records: 0,
            chest_records: 0,
            sign_records: self.sign_records,
            unread_tail: self.unread_tail,
        }
    }
}

/// A parsed Z3-V1 level.
#[derive(Debug, Clone, PartialEq)]
pub struct ZeldaLevel {
    pub magic: ZeldaMagic,
    /// 4096 tile ids, row by row.
    pub board: Vec<u16>,
    pub links: Vec<ZeldaLink>,
    pub computers: Vec<Computer>,
    pub signs: Vec<ZeldaSign>,
    pub layout: ZeldaLayout,
}

/// Line reader over a byte buffer from a start offset.
struct Lines<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Lines<'a> {
    /// The next line without its `\n`, and where it began.
    fn line(&mut self) -> Option<&'a [u8]> {
        if self.pos >= self.data.len() {
            return None;
        }
        let rest = &self.data[self.pos..];
        let (line, used) = match rest.iter().position(|&b| b == b'\n') {
            Some(i) => (&rest[..i], i + 1),
            None => (rest, rest.len()),
        };
        self.pos += used;
        Some(line)
    }
}

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

struct Board {
    tiles: Vec<u16>,
    end: usize,
}

/// Decode the board stream the way the clients do: codes are read least
/// significant bit first; the top bit of a code marks a control code whose
/// low byte is a count and whose 0x100 bit asks for a double run. The
/// double-run flag stays set until a double run completes (Java and both
/// Delphi clients agree), which the `.graal` reader does not model.
fn decode_board(data: &[u8], start: usize, bits: u32, lay: &mut ZeldaLayout) -> Result<Board, String> {
    let flag = 1u32 << (bits - 1);
    let mask = (1u32 << bits) - 1;
    let mut tiles = vec![0u16; BOARD_TILES];
    let mut n = 0usize;
    let (mut buf, mut have, mut pos) = (0u32, 0u32, start);
    let (mut count, mut twice, mut first) = (1u32, false, -1i32);
    while n < BOARD_TILES {
        while have < bits {
            let Some(&b) = data.get(pos) else {
                return Err(format!("board stream ends after {n} of {BOARD_TILES} tiles"));
            };
            buf |= u32::from(b) << have;
            have += 8;
            pos += 1;
        }
        let code = buf & mask;
        buf >>= bits;
        have -= bits;
        lay.codes += 1;
        if code & flag != 0 {
            if twice {
                lay.flags_kept += 1;
            }
            if code & 0x100 != 0 {
                twice = true;
                lay.double_runs += 1;
            } else {
                lay.single_runs += 1;
            }
            count = code & 0xFF;
            lay.max_run = lay.max_run.max(count);
        } else if count == 1 {
            tiles[n] = code as u16;
            n += 1;
        } else if twice {
            if first < 0 {
                first = code as i32;
            } else {
                if n + 2 * count as usize > BOARD_TILES {
                    return Err("a double run runs past the end of the board".into());
                }
                for _ in 0..count {
                    tiles[n] = first as u16;
                    tiles[n + 1] = code as u16;
                    n += 2;
                }
                first = -1;
                twice = false;
                count = 1;
            }
        } else {
            if n + count as usize > BOARD_TILES {
                return Err("a run runs past the end of the board".into());
            }
            for _ in 0..count {
                tiles[n] = code as u16;
                n += 1;
            }
            count = 1;
        }
    }
    lay.tiles_decoded = n;
    lay.board_end = pos;
    lay.max_tile = tiles.iter().copied().max().unwrap_or(0);
    Ok(Board { tiles, end: pos })
}

impl ZeldaLevel {
    /// Parse a Z3-V1 file. `Err` for a bad magic or a board stream that is
    /// cut short or overruns; every other deviation is kept in
    /// `layout.anomalies` and the parts read are returned.
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < 8 {
            return Err("Z3-V1 level shorter than its magic".into());
        }
        let magic = ZeldaMagic::from_bytes(data)
            .ok_or_else(|| format!("unknown Z3-V1 revision {:?}", latin1(&data[..8])))?;
        let mut lay = ZeldaLayout {
            magic: magic.name().to_string(),
            code_bits: magic.code_bits(),
            file_len: data.len(),
            ..ZeldaLayout::default()
        };
        let board = decode_board(data, 8, magic.code_bits(), &mut lay)?;
        let mut s = Lines { data, pos: board.end };

        // Links: lines up to "#" or an empty line.
        let start = s.pos;
        let mut links = Vec::new();
        let mut bad_links = 0;
        while let Some(l) = s.line() {
            if l.is_empty() {
                break;
            }
            if l == b"#" {
                lay.link_hash_end = true;
                break;
            }
            lay.link_lines += 1;
            match ZeldaLink::parse(&latin1(l)) {
                Some(link) => {
                    let tail: Vec<&[u8]> = l.rsplit(|&b| b == b' ').filter(|t| !t.is_empty()).take(6).collect();
                    lay.link_fractional_numbers += tail.iter().filter(|t| t.contains(&b'.') && !t.ends_with(b".0")).count();
                    lay.link_keep_x += usize::from(link.keeps_x());
                    lay.link_keep_y += usize::from(link.keeps_y());
                    if link.fractional_geometry() {
                        lay.anomalies.push("a link has a fractional x, y, width or height".into());
                    }
                    links.push(link);
                }
                None => bad_links += 1,
            }
        }
        lay.links = (start, s.pos);
        if bad_links > 0 {
            lay.anomalies.push(format!("{bad_links} link line(s) do not read as `dest x y w h newx newy`"));
        }
        if !lay.link_hash_end {
            lay.anomalies.push("the links section has no `#` line".into());
        }

        // Computers: three raw bytes and the rest of the line. A first byte
        // of 0x80 or more is the terminator; its line end is read with it.
        // A 0x0A among the first three bytes is data, not a line end.
        let start = s.pos;
        let mut computers = Vec::new();
        loop {
            let rest = &data[s.pos.min(data.len())..];
            if rest.len() < 3 {
                s.pos = data.len();
                lay.anomalies.push("the computers section ends before its terminator".into());
                break;
            }
            let end = rest[3..].iter().position(|&b| b == b'\n').map(|i| i + 3);
            let (extra, used) = match end {
                Some(e) => (&rest[3..e], e + 1),
                None => (&rest[3..], rest.len()),
            };
            if end.is_none() {
                lay.anomalies.push("a computer record has no line end".into());
            }
            s.pos += used;
            if rest[0] >= 0x80 {
                lay.computer_terminator = true;
                if rest[..3] != [0xFF, 0xFF, 0xFF] || !extra.is_empty() {
                    lay.anomalies.push("the computers terminator is not `FF FF FF` and a line end".into());
                }
                break;
            }
            if !extra.is_empty() && !magic.has_verses() {
                lay.anomalies.push("a Z3-V1.03 computer record has text after its type".into());
            }
            *lay.computer_kinds.entry(rest[2]).or_insert(0) += 1;
            lay.computer_extra += usize::from(!extra.is_empty());
            computers.push(Computer { x: rest[0], y: rest[1], kind: rest[2], extra: extra.to_vec() });
        }
        lay.computer_records = computers.len();
        lay.computers = (start, s.pos);

        // Signs: lines `x+32 y+32 text` up to an empty line or the end.
        let start = s.pos;
        let mut signs = Vec::new();
        while let Some(l) = s.line() {
            if l.is_empty() {
                break;
            }
            match decode_sign_record(l) {
                Some((x, y, text)) => signs.push(ZeldaSign { x, y, raw: l[2..].to_vec(), text }),
                None => lay.anomalies.push("a sign line is shorter than its two coordinate bytes".into()),
            }
        }
        lay.sign_records = signs.len();
        lay.signs = (start, s.pos);
        lay.unread_tail = data.len() - s.pos;
        if lay.unread_tail > 0 {
            lay.anomalies.push(format!("{} byte(s) follow the signs section", lay.unread_tail));
        }
        lay.anomalies.dedup();

        Ok(ZeldaLevel { magic, board: board.tiles, links, computers, signs, layout: lay })
    }

    /// The level as the engine holds it: board runs for layer 0, links,
    /// computers as baddies, signs. No NPCs and no chests.
    pub fn to_level(&self) -> Level {
        Level {
            boards: board_runs(&self.board),
            npcs: Vec::new(),
            links: self.links.iter().map(ZeldaLink::to_link).collect(),
            signs: self.signs.iter().map(|s| Sign { x: s.x, y: s.y, text: s.text.clone() }).collect(),
            baddies: self
                .computers
                .iter()
                .map(|c| Baddy {
                    x: f32::from(c.x),
                    y: f32::from(c.y),
                    kind: c.kind.to_string(),
                    verses: latin1(&c.extra).replace('\\', "\n"),
                })
                .collect(),
            chests: Vec::new(),
        }
    }

    /// Write the level back as a Z3-V1 file. This is Seihai's own encoder:
    /// the structure round-trips, the bytes need not match what an original
    /// editor wrote (it uses single runs and double runs of up to 255).
    /// Fails if a tile id does not fit the code width.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let bits = self.magic.code_bits();
        let mut out = self.magic.name().as_bytes().to_vec();
        out.extend(encode_board(&self.board, bits)?);
        for l in &self.links {
            out.extend(l.write().bytes());
            out.push(b'\n');
        }
        out.extend(b"#\n");
        for c in &self.computers {
            out.extend([c.x, c.y, c.kind]);
            out.extend(&c.extra);
            out.push(b'\n');
        }
        out.extend([0xFF, 0xFF, 0xFF, b'\n']);
        for s in &self.signs {
            out.push((s.x + 32) as u8);
            out.push((s.y + 32) as u8);
            out.extend(&s.raw);
            out.push(b'\n');
        }
        Ok(out)
    }
}

/// Pack a board with single runs (equal neighbours) and double runs (an
/// `A B A B ...` pattern), codes least significant bit first.
fn encode_board(tiles: &[u16], bits: u32) -> Result<Vec<u8>, String> {
    let flag = 1u32 << (bits - 1);
    if let Some(t) = tiles.iter().find(|&&t| u32::from(t) >= flag) {
        return Err(format!("tile id {t} does not fit a {bits}-bit code"));
    }
    let mut codes: Vec<u32> = Vec::new();
    let mut i = 0;
    while i < tiles.len() {
        let same = tiles[i..].iter().take(255).take_while(|&&t| t == tiles[i]).count();
        let pairs = if i + 3 < tiles.len() {
            let mut n = 0;
            while n < 255 && i + 2 * n + 1 < tiles.len() && tiles[i + 2 * n] == tiles[i] && tiles[i + 2 * n + 1] == tiles[i + 1] {
                n += 1;
            }
            n
        } else {
            0
        };
        if same >= 2 && same >= 2 * pairs {
            codes.push(flag | same as u32);
            codes.push(u32::from(tiles[i]));
            i += same;
        } else if pairs >= 2 {
            codes.push(flag | 0x100 | pairs as u32);
            codes.push(u32::from(tiles[i]));
            codes.push(u32::from(tiles[i + 1]));
            i += 2 * pairs;
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
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Vec<u16> {
        let mut t = vec![0u16; BOARD_TILES];
        for (i, v) in t.iter_mut().enumerate() {
            *v = match i % 64 {
                0..=9 => 2,
                10..=19 => {
                    if i % 2 == 0 {
                        7
                    } else {
                        9
                    }
                }
                20..=21 => 1500,
                _ => (i % 5) as u16 * 3,
            };
        }
        t[63] = 2047;
        t[4095] = 2047;
        t
    }

    fn file(magic: &str, tail: &[u8]) -> Vec<u8> {
        let bits = if magic == "Z3-V1.02" { 11 } else { 12 };
        let mut d = magic.as_bytes().to_vec();
        let b: Vec<u16> = board().iter().map(|&t| if bits == 11 { t & 0x3FF } else { t }).collect();
        d.extend(encode_board(&b, bits).unwrap());
        d.extend_from_slice(tail);
        d
    }

    #[test]
    fn board_round_trips_at_both_widths() {
        for (m, bits) in [("Z3-V1.02", 11u32), ("Z3-V1.03", 12), ("Z3-V1.04", 12)] {
            let want: Vec<u16> = board().iter().map(|&t| if bits == 11 { t & 0x3FF } else { t }).collect();
            let d = file(m, b"#\n\xff\xff\xff\n");
            let z = ZeldaLevel::parse(&d).unwrap();
            assert_eq!(z.board, want, "{m}");
            assert_eq!(z.layout.code_bits, bits);
            assert_eq!(z.layout.tiles_decoded, 4096);
            assert!(z.layout.is_clean(), "{m}: {:?}", z.layout.anomalies);
            assert!(z.layout.double_runs > 0 && z.layout.single_runs > 0);
        }
    }

    #[test]
    fn z3_103_computers_are_three_bytes_and_a_line_end() {
        // Two computers: the second has 0x0A as its y, which is data.
        let d = file("Z3-V1.03", b"#\n\x17\x27\x04\n\x05\x0a\x02\n\xff\xff\xff\n");
        let z = ZeldaLevel::parse(&d).unwrap();
        assert_eq!(z.computers.len(), 2);
        assert_eq!((z.computers[0].x, z.computers[0].y, z.computers[0].kind), (23, 39, 4));
        assert_eq!((z.computers[1].x, z.computers[1].y, z.computers[1].kind), (5, 10, 2));
        assert!(z.computers.iter().all(|c| c.extra.is_empty()));
        assert!(z.layout.computer_terminator && z.layout.is_clean());
        assert_eq!(z.layout.computer_kinds.get(&4), Some(&1));
        assert!(z.signs.is_empty());
    }

    #[test]
    fn z3_104_computers_carry_verses() {
        let d = file("Z3-V1.04", b"#\n\x0a\x0b\x01Hi!\\Oh no!\\Need help\n\x01\x02\x00\\\\\n\xff\xff\xff\n");
        let z = ZeldaLevel::parse(&d).unwrap();
        assert_eq!(z.computers.len(), 2);
        assert_eq!(z.computers[0].verses(), ["Hi!", "Oh no!", "Need help"]);
        assert_eq!(z.computers[1].verses(), ["", "", ""]);
        let l = z.to_level();
        assert_eq!(l.baddies[0].verses, "Hi!\nOh no!\nNeed help");
        assert_eq!(l.baddies[0].x, 10.0);
        assert!(z.layout.is_clean());
    }

    #[test]
    fn links_take_decimals_and_negative_arrivals_keep_the_player() {
        let d = file(
            "Z3-V1.03",
            b"level11.zelda 31.0 42.0 2.0 1.0 43.5 41.0\nroad.graal 0 29 1 27 61 -1\nbig house.zelda 5 6 2 1 -1.0 -1.0\n#\n\xff\xff\xff\n",
        );
        let z = ZeldaLevel::parse(&d).unwrap();
        assert_eq!(z.links.len(), 3);
        let l = z.to_level();
        assert_eq!((l.links[0].x, l.links[0].y, l.links[0].width, l.links[0].height), (31, 42, 2, 1));
        assert_eq!((l.links[0].new_x, l.links[0].new_y), (Some(43.5), Some(41.0)));
        assert_eq!((l.links[1].new_x, l.links[1].new_y), (Some(61.0), None));
        assert_eq!(l.links[2].dest, "big house.zelda");
        assert_eq!((l.links[2].new_x, l.links[2].new_y), (None, None));
        assert_eq!(z.layout.link_keep_x, 1);
        assert_eq!(z.layout.link_keep_y, 2);
        assert_eq!(z.layout.link_fractional_numbers, 1, "43.5 only; 31.0 is whole");
        assert!(z.layout.is_clean(), "{:?}", z.layout.anomalies);
    }

    #[test]
    fn signs_use_the_packed_alphabet_and_end_at_the_end() {
        // x=3 y=4 "Hi" in alphabet codes (H = 7, i = 34): bytes code + 32.
        let d = file("Z3-V1.03", b"#\n\xff\xff\xff\n#$'B\n%&\x80!\n");
        let z = ZeldaLevel::parse(&d).unwrap();
        assert_eq!(z.signs.len(), 2);
        assert_eq!((z.signs[0].x, z.signs[0].y), (3, 4));
        assert_eq!(z.signs[0].text, "Hi");
        assert_eq!(z.signs[0].raw, b"'B");
        assert_eq!(z.to_level().signs[0].text, "Hi");
        assert_eq!(z.layout.sign_records, 2);
        assert!(z.layout.is_clean());
    }

    #[test]
    fn deviations_are_reported_not_fixed() {
        // no `#` line, no terminator, text after the last sign's empty line
        let d = file("Z3-V1.03", b"a.zelda 1 2 3 4 5 6\n");
        let z = ZeldaLevel::parse(&d).unwrap();
        assert!(!z.layout.is_clean());
        assert!(z.layout.anomalies.iter().any(|a| a.contains("no `#`")));
        assert!(z.layout.anomalies.iter().any(|a| a.contains("before its terminator")));
        let d = file("Z3-V1.03", b"#\n\xff\xff\xff\n\n#$'B\n");
        let z = ZeldaLevel::parse(&d).unwrap();
        assert_eq!(z.layout.unread_tail, 5);
        assert!(!z.layout.is_clean());
        // a record cut off
        let d = file("Z3-V1.04", b"#\n\x01\x02\x03verse");
        let z = ZeldaLevel::parse(&d).unwrap();
        assert!(z.layout.anomalies.iter().any(|a| a.contains("no line end")));
    }

    #[test]
    fn hard_errors() {
        assert!(ZeldaLevel::parse(b"Z3-V1").is_err());
        assert!(ZeldaLevel::parse(b"Z3-V1.05xxxxxxxx").unwrap_err().contains("unknown Z3-V1 revision"));
        assert!(ZeldaLevel::parse(b"GR-V1.03xxxxxxxx").is_err());
        let mut d = file("Z3-V1.03", b"");
        d.truncate(100);
        assert!(ZeldaLevel::parse(&d).unwrap_err().contains("board stream ends"));
    }

    /// Pack 12-bit codes, least significant bit first.
    fn pack12(codes: &[u32]) -> Vec<u8> {
        let (mut out, mut buf, mut have) = (Vec::new(), 0u64, 0u32);
        for &c in codes {
            buf |= u64::from(c) << have;
            have += 12;
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

    #[test]
    fn a_double_flag_stays_set_until_a_double_run_completes() {
        // [double, count 1] 7: a count of 1 writes the tile alone and the
        // double flag stays set. [count 2] 9 4: the pair is written twice.
        // The `.graal` reader clears the flag on a control code without 0x100.
        let flag = 0x800u32;
        let mut codes = vec![flag | 0x100 | 1, 7, flag | 2, 9, 4];
        let mut left = BOARD_TILES - 5;
        while left > 0 {
            let n = left.min(255);
            codes.extend([flag | n as u32, 3]);
            left -= n;
        }
        let mut bytes = b"Z3-V1.03".to_vec();
        bytes.extend(pack12(&codes));
        bytes.extend(b"#\n\xff\xff\xff\n");
        let z = ZeldaLevel::parse(&bytes).unwrap();
        assert_eq!(&z.board[..6], &[7, 9, 4, 9, 4, 3]);
        assert_eq!(z.layout.flags_kept, 1);
        assert!(z.layout.is_clean());
    }

    #[test]
    fn level_matches_the_board_runs_of_a_graal_file() {
        let d = file("Z3-V1.04", b"#\n\xff\xff\xff\n");
        let z = ZeldaLevel::parse(&d).unwrap();
        assert_eq!(z.to_level().layer0_board(), z.board);
        let via = Level::parse_binary_with_layout(&d).unwrap();
        assert_eq!(via.0.layer0_board(), z.board);
        assert_eq!(via.1.magic, "Z3-V1.04");
        assert_eq!(via.1.npc_records + via.1.chest_records, 0);
    }

    #[test]
    fn to_bytes_round_trips_the_structure() {
        let d = file(
            "Z3-V1.04",
            b"a.zelda 1.0 2.0 3.0 4.0 5.5 -1.0\n#\n\x01\x02\x03Hi\\there\\you\n\xff\xff\xff\n#$'B\n",
        );
        let z = ZeldaLevel::parse(&d).unwrap();
        let again = ZeldaLevel::parse(&z.to_bytes().unwrap()).unwrap();
        assert_eq!(again.board, z.board);
        assert_eq!(again.links, z.links);
        assert_eq!(again.computers, z.computers);
        assert_eq!(again.signs, z.signs);
        assert!(again.layout.is_clean());
        let mut bad = z.clone();
        bad.board[0] = 2048;
        assert!(bad.to_bytes().is_err());
    }

    #[test]
    fn magic_facts() {
        assert!(ZeldaMagic::V103.graal_clients_read() && ZeldaMagic::V104.graal_clients_read());
        assert!(!ZeldaMagic::V102.graal_clients_read());
        assert!(ZeldaMagic::V104.has_verses() && !ZeldaMagic::V103.has_verses());
        assert_eq!(ZeldaMagic::from_bytes(b"Z3-V1.03zzzz"), Some(ZeldaMagic::V103));
        assert_eq!(Computer::java_type_name(6), Some("octopus"));
        assert_eq!(Computer::java_type_name(7), None);
    }

    #[test]
    fn summary_reads_without_content() {
        let d = file("Z3-V1.03", b"a.zelda 1 2 3 4 5 6\n#\n\x01\x02\x03\n\xff\xff\xff\n");
        let z = ZeldaLevel::parse(&d).unwrap();
        let s = z.layout.summary();
        assert!(s.contains("magic Z3-V1.03") && s.contains("computers") && s.contains("anomalies: none"), "{s}");
        assert!(!s.contains("a.zelda"));
    }

    /// Every Z3-V1 file under `$SEIHAI_DRIVE_CLIENTS` (the folder that holds
    /// the build folders), whatever its extension. Asserts derived facts
    /// only; nothing from the files is printed except counts.
    #[test]
    #[ignore = "needs SEIHAI_DRIVE_CLIENTS=<folder with the old client builds>"]
    fn real_z3_files_match_the_format_notes() {
        let Some(root) = std::env::var_os("SEIHAI_DRIVE_CLIENTS") else {
            eprintln!("SEIHAI_DRIVE_CLIENTS not set; skipping");
            return;
        };
        fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(rd) = std::fs::read_dir(d) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.metadata().is_ok_and(|m| m.len() >= 8 && m.len() < 1_000_000) {
                    out.push(p);
                }
            }
        }
        let mut all = Vec::new();
        walk(std::path::Path::new(&root), &mut all);
        let (mut files, mut v102, mut v103, mut v104, mut unique) = (0, 0, 0, 0, std::collections::HashSet::new());
        let (mut links, mut computers, mut signs, mut hash_kept, mut flags_kept) = (0, 0, 0, 0, 0);
        let mut bad = Vec::new();
        for p in &all {
            let Ok(data) = std::fs::read(p) else { continue };
            if !is_zelda(&data) {
                continue;
            }
            files += 1;
            let z = match ZeldaLevel::parse(&data) {
                Ok(z) => z,
                Err(e) => {
                    bad.push(format!("{}: {e}", p.display()));
                    continue;
                }
            };
            let first_time = unique.insert(data.clone());
            match z.magic {
                ZeldaMagic::V102 => v102 += 1,
                ZeldaMagic::V103 => v103 += 1,
                ZeldaMagic::V104 => v104 += 1,
            }
            if !z.layout.is_clean() {
                bad.push(format!("{}: {:?}", p.display(), z.layout.anomalies));
            }
            // Derived facts that hold for every file.
            assert_eq!(z.layout.tiles_decoded, BOARD_TILES);
            assert_eq!(z.layout.board_end + (z.layout.signs.1 - z.layout.links.0), data.len(), "sections tile the file");
            assert!(z.layout.max_tile < 2048, "12-bit codes cannot hold more");
            assert!(z.computers.iter().all(|c| c.kind <= 6 && c.x < 64 && c.y < 64), "types 0 to 6, positions on the board");
            assert!(z.signs.iter().all(|s| (0..64).contains(&s.x) && (0..64).contains(&s.y)));
            assert!(z.links.iter().all(|l| !l.dest.is_empty() && !l.fractional_geometry()));
            assert_eq!(z.layout.computer_extra > 0, z.magic == ZeldaMagic::V104 && z.computers.iter().any(|c| !c.extra.is_empty()));
            if z.magic == ZeldaMagic::V103 {
                assert_eq!(z.layout.computer_extra, 0, "1.03 records have no text after the type");
            }
            if first_time {
                // The structure survives our own writer.
                let again = ZeldaLevel::parse(&z.to_bytes().unwrap()).unwrap();
                assert_eq!((again.board, again.links, again.computers, again.signs), (z.board.clone(), z.links.clone(), z.computers.clone(), z.signs.clone()));
                // Through the engine's level type.
                let lvl = z.to_level();
                assert_eq!(lvl.layer0_board(), z.board);
                assert_eq!(lvl.links.len(), z.links.len());
            }
            links += z.links.len();
            computers += z.computers.len();
            signs += z.signs.len();
            hash_kept += usize::from(z.layout.link_hash_end);
            flags_kept += z.layout.flags_kept;
        }
        eprintln!(
            "{files} Z3-V1 files ({} distinct): {v102} 1.02, {v103} 1.03, {v104} 1.04; {links} links, {computers} computers, {signs} signs; {flags_kept} sticky double flags",
            unique.len()
        );
        assert!(bad.is_empty(), "files that do not parse clean:\n{}", bad.join("\n"));
        assert_eq!(v102, 0, "no file has Z3-V1.02");
        assert_eq!(hash_kept, files, "every links section ends on `#`");
        assert_eq!(flags_kept, 0, "no real file leaves a double flag set");
    }
}
