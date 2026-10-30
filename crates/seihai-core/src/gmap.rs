//! GMAP overworlds: the grid of level names, positions on it, and the
//! edge crossings that move a player from one cell to the next.
//!
//! A gmap is a grid of ordinary 64×64-tile levels. A player is in one cell
//! and has a position local to that cell; the real client keeps the global
//! position (`local + 64 * cell`) on the side and, when the player's x or y
//! leaves `0..64`, changes the cell prop (`GMAPLEVELX` 43 or `GMAPLEVELY`
//! 44) and wraps the coordinate, all in one props message (lab, real 2.22
//! and 2.31 clients, 2026-10-24; Graal_analysis feature spec 06).
//!
//! This module has no I/O. The session side (`SessionState`) feeds it the
//! server's 49/6/186 messages; the walking side (`OfflineSession`) asks it
//! whether a step crosses an edge.
//!
//! Everything version-dependent comes from [`GmapRules`], which is read from
//! the version profile, never from the version number.

use crate::assets::Level;
use crate::offline::OfflineSession;
use crate::protocol::{
    encode_pos_classic, encode_pos_precise, ClientVersion, IdSet, SessionState, TamperSender,
};
use std::collections::{HashMap, HashSet};

/// Tiles along one side of a level, and so of a gmap cell.
pub const CELL_TILES: i32 = 64;

/// `PLI_PLAYERPROPS` (2).
const PLI_PLAYERPROPS: u8 = 2;
/// `PLI_ADJACENTLEVEL` (35).
pub const PLI_ADJACENTLEVEL: u8 = 35;
/// Player props used at a crossing.
const PROP_X: u8 = 15;
const PROP_Y: u8 = 16;
const PROP_GMAPLEVELX: u8 = 43;
const PROP_GMAPLEVELY: u8 = 44;
const PROP_X2: u8 = 78;
const PROP_Y2: u8 = 79;

/// What a version profile says about gmaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GmapRules {
    /// The build understands gmaps: its profile has `.gmap` levels,
    /// `PLO_PLAYERWARP2` (49) and the cell props 43 and 44 in the own-player
    /// prop set. A client without it never enters a gmap and sends no cell
    /// prop.
    pub enabled: bool,
    /// Positions go as X2/Y2 (78/79) instead of X/Y (15/16).
    pub precise_pos: bool,
    /// The build can send `PLI_ADJACENTLEVEL` (35).
    pub adjacent: bool,
}

impl GmapRules {
    /// Read the rules from `version`'s profile.
    pub fn for_version(version: ClientVersion) -> Self {
        let p = version.profile();
        let has = |s: &IdSet, id: u8| s.contains(id);
        Self {
            enabled: p.levels.gmap
                && has(&p.protocol.plo, 49)
                && has(&p.props.own, PROP_GMAPLEVELX)
                && has(&p.props.own, PROP_GMAPLEVELY),
            precise_pos: p.props.precise_pos,
            adjacent: has(&p.protocol.pli, PLI_ADJACENTLEVEL),
        }
    }
}

/// What the server sent for one level while the player was elsewhere: the
/// per-level lists the session keeps for the level it is currently filling.
#[derive(Debug, Default, Clone)]
pub struct LevelSlot {
    pub board: Option<Vec<u16>>,
    pub board_lines: Vec<String>,
    pub links: Vec<String>,
    pub signs: Vec<Vec<u8>>,
    pub chests: Vec<String>,
}

/// The session's view of the gmap the player is on, built from the server's
/// messages: `PLO_PLAYERWARP2` (49) names the gmap and the cell, the `.gmap`
/// file gives the grid, and level data arrives per level (the player's own
/// and the neighbours the client asked for with 35), each introduced by a
/// `PLO_LEVELNAME` (6) and closed by a 6 that names the gmap again.
#[derive(Debug, Default)]
pub struct GmapSession {
    /// The gmap file name from 49.
    pub name: String,
    /// The player's cell.
    pub cell: (i32, i32),
    /// The grid, once the file is known.
    pub map: Option<Gmap>,
    /// 64×64 tile ids of every level the server sent, by lower-case name.
    pub boards: HashMap<String, Vec<u16>>,
    /// The level the first 6 after a 49 named: the player's level until
    /// the grid says which cell is which.
    pub level_hint: Option<String>,
    /// A 49 arrived and its level name has not.
    pub awaiting_level: bool,
    /// Level data parked while another level is the one being filled.
    pub slots: HashMap<String, LevelSlot>,
}

impl GmapSession {
    pub fn new(name: String, cell: (i32, i32)) -> Self {
        Self { name, cell, awaiting_level: true, ..Default::default() }
    }

    /// Whether `name` is this gmap's file name (a 6 that names it means
    /// "the player's level again", not a level).
    pub fn is_name(&self, name: &str) -> bool {
        name.eq_ignore_ascii_case(&self.name)
    }

    /// The level at `cell` per the grid.
    pub fn level_at(&self, cell: (i32, i32)) -> Option<String> {
        self.map.as_ref()?.level_at(cell.0, cell.1).map(str::to_string)
    }

    /// The player's level: the grid's, else the one the server named first.
    pub fn current_level(&self) -> Option<String> {
        self.level_at(self.cell).or_else(|| self.level_hint.clone())
    }
}

/// The levels around the player, as the walking side sees them: the grid,
/// the player's cell, and the loaded levels of the other cells. The level of
/// the player's own cell is `OfflineSession::current_level`.
#[derive(Debug)]
pub struct GmapWorld {
    pub name: String,
    pub map: Gmap,
    pub cell: (i32, i32),
    /// Loaded levels of the other cells (the player's neighbours).
    pub levels: HashMap<(i32, i32), Level>,
    /// The last edge crossing a step made, until it is taken.
    pub crossing: Option<Crossing>,
}

impl GmapWorld {
    pub fn new(name: String, map: Gmap, cell: (i32, i32)) -> Self {
        Self { name, map, cell, levels: HashMap::new(), crossing: None }
    }

    /// Neighbouring cells that hold a level in the grid but have none
    /// loaded.
    pub fn missing(&self) -> Vec<(i32, i32)> {
        self.map.neighbours(self.cell).into_iter().filter(|c| !self.levels.contains_key(c)).collect()
    }
}

/// How close (in tiles) to a big-map level's edge the player gets before the
/// level on that side is asked for. Seihai's choice: the real clients do not
/// ask for big-map neighbours ahead.
pub const BIGMAP_NEAR_EDGE: f32 = 10.0;

/// A parsed `.gmap` file (`GRMAP001`), the reader keys only: the keys a
/// client needs to place levels. Generator keys (`GENERATED`, `HEIGHTMAP`,
/// ...) are skipped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gmap {
    /// Levels across and down.
    pub width: usize,
    pub height: usize,
    /// Level names, row by row, `width * height` of them. An empty name is
    /// a hole.
    pub names: Vec<String>,
    /// `MAPIMG` and `MINIMAPIMG`: overview image file names.
    pub map_img: Option<String>,
    pub minimap_img: Option<String>,
    /// `NOAUTOMAPPING` and `LOADFULLMAP` flags.
    pub no_auto_mapping: bool,
    pub load_full_map: bool,
    /// `LOADATSTART` names.
    pub load_at_start: Vec<String>,
}

/// Split one `LEVELNAMES` row: comma-separated, each name optionally in
/// double quotes. A trailing comma does not add a cell; `""` is a hole.
fn split_row(line: &str) -> Vec<String> {
    let line = line.trim();
    let line = line.strip_suffix(',').unwrap_or(line);
    line.split(',')
        .map(|f| f.trim().trim_matches('"').trim().to_string())
        .collect()
}

impl Gmap {
    /// Parse `GRMAP001` text. `None` if the signature line is missing or the
    /// grid has no cells.
    pub fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
        if lines.by_ref().find(|l| !l.trim().is_empty())?.trim() != "GRMAP001" {
            return None;
        }
        let mut g = Gmap::default();
        let (mut declared_w, mut declared_h) = (None, None);
        let mut rows: Vec<Vec<String>> = Vec::new();
        while let Some(line) = lines.next() {
            let mut words = line.split_whitespace();
            let Some(key) = words.next() else { continue };
            match key {
                "WIDTH" => declared_w = words.next().and_then(|v| v.parse::<usize>().ok()),
                "HEIGHT" => declared_h = words.next().and_then(|v| v.parse::<usize>().ok()),
                "MAPIMG" => g.map_img = words.next().map(str::to_string),
                "MINIMAPIMG" => g.minimap_img = words.next().map(str::to_string),
                "NOAUTOMAPPING" => g.no_auto_mapping = true,
                "LOADFULLMAP" => g.load_full_map = true,
                "LEVELNAMES" => {
                    for row in lines.by_ref() {
                        if row.trim() == "LEVELNAMESEND" {
                            break;
                        }
                        if !row.trim().is_empty() {
                            rows.push(split_row(row));
                        }
                    }
                }
                "LOADATSTART" => {
                    for row in lines.by_ref() {
                        if row.trim() == "LOADATSTARTEND" {
                            break;
                        }
                        g.load_at_start.extend(split_row(row).into_iter().filter(|n| !n.is_empty()));
                    }
                }
                _ => {}
            }
        }
        // The declared size wins; without one, the rows decide.
        g.width = declared_w.unwrap_or_else(|| rows.iter().map(Vec::len).max().unwrap_or(0));
        g.height = declared_h.unwrap_or(rows.len());
        if g.width == 0 || g.height == 0 || g.width > 255 || g.height > 255 {
            return None;
        }
        g.names = vec![String::new(); g.width * g.height];
        for (y, row) in rows.iter().take(g.height).enumerate() {
            for (x, name) in row.iter().take(g.width).enumerate() {
                g.names[y * g.width + x] = name.clone();
            }
        }
        Some(g)
    }

    /// A big map's text (the map screen's `bigmap.txt`): rows of level
    /// names, comma-separated and quoted, with no header. The grid is as
    /// wide as the longest row. `None` when no row names a level.
    pub fn parse_grid(text: &str) -> Option<Self> {
        let rows: Vec<Vec<String>> = text.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.trim().is_empty()).map(split_row).collect();
        let mut g = Gmap { width: rows.iter().map(Vec::len).max().unwrap_or(0), height: rows.len(), ..Gmap::default() };
        if g.width == 0 || g.width > 255 || g.height > 255 {
            return None;
        }
        g.names = vec![String::new(); g.width * g.height];
        for (y, row) in rows.iter().enumerate() {
            for (x, name) in row.iter().enumerate() {
                g.names[y * g.width + x] = name.clone();
            }
        }
        g.names.iter().any(|n| !n.is_empty()).then_some(g)
    }

    /// The level next to `level` on the grid, `(dx, dy)` cells away, if the
    /// grid names one there.
    pub fn neighbour_of(&self, level: &str, dx: i32, dy: i32) -> Option<&str> {
        let (cx, cy) = self.cell_of(level)?;
        self.level_at(cx + dx, cy + dy)
    }

    /// The level at cell `(cx, cy)`, or `None` outside the grid or on a hole.
    pub fn level_at(&self, cx: i32, cy: i32) -> Option<&str> {
        if cx < 0 || cy < 0 || cx as usize >= self.width || cy as usize >= self.height {
            return None;
        }
        let n = self.names[cy as usize * self.width + cx as usize].as_str();
        (!n.is_empty()).then_some(n)
    }

    /// The index of `level` in the gmap's level list: its place among the
    /// named cells in file order (case-insensitive, first match). The real
    /// client's list order and de-duplication are not traced, so only
    /// "not in the list" and "first in the list" can be relied on.
    pub fn list_index(&self, level: &str) -> Option<usize> {
        self.names.iter().filter(|n| !n.is_empty()).position(|n| n.eq_ignore_ascii_case(level))
    }

    /// The cell a level is in (case-insensitive; the first match).
    pub fn cell_of(&self, level: &str) -> Option<(i32, i32)> {
        let i = self.names.iter().position(|n| !n.is_empty() && n.eq_ignore_ascii_case(level))?;
        Some(((i % self.width) as i32, (i / self.width) as i32))
    }

    /// The big-map levels next to `here` on the sides the player at `(x, y)`
    /// (sprite top-left, tiles) is within [`BIGMAP_NEAR_EDGE`] tiles of, the
    /// corner one too when two sides are near. Asked for ahead so a walk
    /// across the edge finds the level already here (laundry list P7).
    /// Nothing when `here` is not on the grid.
    pub fn neighbours_near_edge(&self, here: &str, x: f32, y: f32) -> Vec<&str> {
        // The middle of the player's body, as the edge links test it.
        let (fx, fy) = (x + 1.5, y + 2.0);
        let near = BIGMAP_NEAR_EDGE;
        let dx = if fx < near { -1 } else if fx >= 64.0 - near { 1 } else { 0 };
        let dy = if fy < near { -1 } else if fy >= 64.0 - near { 1 } else { 0 };
        let mut v = Vec::new();
        for (ox, oy) in [(dx, 0), (0, dy), (dx, dy)] {
            if (ox, oy) == (0, 0) {
                continue;
            }
            if let Some(n) = self.neighbour_of(here, ox, oy) {
                if !v.contains(&n) {
                    v.push(n);
                }
            }
        }
        v
    }

    /// The level files to ask the server for now, ahead of a big-map edge:
    /// [`Self::neighbours_near_edge`] for the player at `(x, y)` in `here`,
    /// each name once per `asked` (the caller keeps it per connection), with
    /// `.nw` added to a bare name. The caller sends them as file requests in
    /// the same tick; a link on that edge fires only some steps later, at the
    /// edge itself, so the level is asked for before the link is walked
    /// (laundry item 9, `documentation/bigmap-neighbours.md`).
    pub fn big_map_ahead(&self, here: &str, x: f32, y: f32, asked: &mut HashSet<String>) -> Vec<String> {
        self.neighbours_near_edge(here, x, y)
            .into_iter()
            .filter(|n| asked.insert(format!("near:{}", n.to_ascii_lowercase())))
            .map(|n| if n.contains('.') { n.to_string() } else { format!("{n}.nw") })
            .collect()
    }

    /// The cells around `(cx, cy)` (up to eight) that hold a level.
    pub fn neighbours(&self, (cx, cy): (i32, i32)) -> Vec<(i32, i32)> {
        let mut v = Vec::new();
        // Columns first, then rows: the order the real 2.22 and 2.31 clients
        // send their `PLI_ADJACENTLEVEL` messages in (documentation/gmap-order.md).
        for dx in -1..=1 {
            for dy in -1..=1 {
                if (dx, dy) != (0, 0) && self.level_at(cx + dx, cy + dy).is_some() {
                    v.push((cx + dx, cy + dy));
                }
            }
        }
        v
    }

    /// Where a position that may have left its cell really is. `local` is
    /// the position in tiles relative to `cell`, and may be outside
    /// `0..64` after a step.
    pub fn resolve(&self, cell: (i32, i32), local: (f32, f32)) -> Resolved {
        let (nx, ny, lx, ly) = wrap(cell, local);
        let to = (nx, ny);
        if to == cell {
            return Resolved::Inside;
        }
        if self.level_at(nx, ny).is_none() {
            return Resolved::NoLevel;
        }
        Resolved::Crossed(Crossing { from: cell, to, local_x: lx, local_y: ly })
    }
}

/// Wrap a cell-relative position into its own cell.
fn wrap(cell: (i32, i32), (x, y): (f32, f32)) -> (i32, i32, f32, f32) {
    let side = CELL_TILES as f32;
    let (kx, ky) = ((x / side).floor(), (y / side).floor());
    (cell.0 + kx as i32, cell.1 + ky as i32, x - kx * side, y - ky * side)
}

/// The global position (tiles) of a cell-local one.
pub fn to_global(cell: (i32, i32), (x, y): (f32, f32)) -> (f32, f32) {
    (x + (cell.0 * CELL_TILES) as f32, y + (cell.1 * CELL_TILES) as f32)
}

/// The cell and cell-local position of a global one.
pub fn to_local((gx, gy): (f32, f32)) -> ((i32, i32), (f32, f32)) {
    let (cx, cy, lx, ly) = wrap((0, 0), (gx, gy));
    ((cx, cy), (lx, ly))
}

/// What [`Gmap::resolve`] found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resolved {
    /// Still in the same cell.
    Inside,
    /// The position is past the edge of the grid or in a hole: there is
    /// nowhere to go.
    NoLevel,
    /// In another cell.
    Crossed(Crossing),
}

/// A move from one cell to another.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crossing {
    pub from: (i32, i32),
    pub to: (i32, i32),
    /// The wrapped position in the new cell, tiles.
    pub local_x: f32,
    pub local_y: f32,
}

impl Crossing {
    /// The cell x changed (prop 43 goes out).
    pub fn x_changed(&self) -> bool {
        self.from.0 != self.to.0
    }

    /// The cell y changed (prop 44 goes out).
    pub fn y_changed(&self) -> bool {
        self.from.1 != self.to.1
    }
}

fn gshort(v: u16) -> [u8; 2] {
    [((v >> 7) as u8 & 0x7F) + 0x20, (v as u8 & 0x7F) + 0x20]
}

/// The props message a client sends at a crossing: `PLI_PLAYERPROPS` (2)
/// with, for each axis that changed, the cell prop (43 for x, 44 for y)
/// followed by the wrapped coordinate of that axis: X2/Y2 (78/79) when the
/// profile uses precise positions, X/Y (15/16) otherwise.
///
/// Real messages, from the lab: 2.31 `43 = 1, X2 = 0` going east, `43 = 0,
/// X2 = 63.5` going west, and the same with 44 and Y2 for south and north;
/// 2.22 `X, 43` (the coordinate first: its id 15 is below 43).
/// Nothing else (no direction, no other axis) is in the message. A step
/// that changes both axes at once was never seen; this writes the pairs
/// one after the other.
pub fn crossing_props_message(rules: &GmapRules, c: &Crossing) -> Vec<u8> {
    let mut m = vec![PLI_PLAYERPROPS + 0x20];
    // Props go out in ascending id order, as the real clients write them: the
    // classic X/Y (15, 16) come before the cell props (43, 44), the precise
    // X2/Y2 (78, 79) after them. A step that changes both axes was never seen;
    // the same rule is applied to it.
    let mut props: Vec<(u8, Vec<u8>)> = Vec::new();
    let mut add = |id: u8, bytes: Vec<u8>| props.push((id, bytes));
    let coord = |classic: u8, precise: u8, v: f32| -> (u8, Vec<u8>) {
        if rules.precise_pos {
            let mut b = vec![precise + 0x20];
            b.extend_from_slice(&gshort(encode_pos_precise(v)));
            (precise, b)
        } else {
            (classic, vec![classic + 0x20, encode_pos_classic(v).wrapping_add(0x20)])
        }
    };
    if c.x_changed() {
        add(PROP_GMAPLEVELX, vec![PROP_GMAPLEVELX + 0x20, c.to.0 as u8 + 0x20]);
        let (id, b) = coord(PROP_X, PROP_X2, c.local_x);
        add(id, b);
    }
    if c.y_changed() {
        add(PROP_GMAPLEVELY, vec![PROP_GMAPLEVELY + 0x20, c.to.1 as u8 + 0x20]);
        let (id, b) = coord(PROP_Y, PROP_Y2, c.local_y);
        add(id, b);
    }
    props.sort_by_key(|(id, _)| *id);
    for (_, b) in props {
        m.extend(b);
    }
    m.push(b'\n');
    m
}

/// The messages of the frame a client sends the moment it crosses an edge
/// (all before the server's answer, lab 2026-10-24):
///
/// 1. `PLI_ENTERLEVEL` (46) when the build sends it (2.3, 2.31);
/// 2. the tamper check (39) when `tamper` is given and the build re-sends it
///    on level changes (2.22);
/// 3. the props message ([`crossing_props_message`]).
///
/// An empty list for a profile without gmaps.
pub fn crossing_frame(
    version: ClientVersion,
    c: &Crossing,
    tamper: Option<&mut TamperSender>,
) -> Vec<Vec<u8>> {
    let rules = GmapRules::for_version(version);
    if !rules.enabled {
        return Vec::new();
    }
    let mut msgs = crossing_lead(version, tamper);
    msgs.push(crossing_props_message(&rules, c));
    msgs
}

/// The messages that open a crossing frame, ahead of the flags and the
/// props: `PLI_ENTERLEVEL` (46) when the build sends it, then the tamper
/// check (39) when `tamper` is given and the build re-sends it on level
/// changes. A build has one or the other, never both (2.31 sends 46 and
/// keeps 39 on its timer; 2.22 has no 46 and re-sends 39).
///
/// A build with a walk cadence (2.17, 2.22, 3.04) sends these in the frame
/// of the step that crosses and leaves the cell prop to its next props
/// message ([`crate::protocol::walkcadence::WalkCadence::cross`]).
pub fn crossing_lead(version: ClientVersion, tamper: Option<&mut TamperSender>) -> Vec<Vec<u8>> {
    if !GmapRules::for_version(version).enabled {
        return Vec::new();
    }
    let mut msgs = Vec::new();
    if version.sends_enter_level() {
        msgs.push(crate::net::enter_level_message());
    }
    if let Some(t) = tamper {
        if t.level_changed() {
            msgs.extend(t.message());
        }
    }
    msgs
}

/// `PLI_ADJACENTLEVEL` (35): `GINT5` mod time of the cached copy (0 when
/// there is none) and the level name. The server answers with that level's
/// data, then names the gmap again.
pub fn adjacent_level_message(mod_time: u32, level: &str) -> Vec<u8> {
    let mut m = vec![PLI_ADJACENTLEVEL + 0x20];
    for shift in [28, 21, 14, 7, 0] {
        m.push(((mod_time >> shift) as u8 & 0x7F) + 0x20);
    }
    m.extend(level.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }));
    m.push(b'\n');
    m
}

/// The `PLI_ADJACENTLEVEL` messages for the cells around `cell`: one per
/// neighbouring level that has no entry in `have` (lower-case names), mod
/// time 0 as on a cold cache. Empty when the profile cannot send 35.
pub fn adjacent_requests(
    rules: &GmapRules,
    map: &Gmap,
    cell: (i32, i32),
    have: &HashMap<String, u32>,
) -> Vec<Vec<u8>> {
    if !rules.enabled || !rules.adjacent {
        return Vec::new();
    }
    map.neighbours(cell)
        .into_iter()
        .filter_map(|(x, y)| map.level_at(x, y))
        .filter(|n| !have.contains_key(&n.to_ascii_lowercase()))
        .map(|n| adjacent_level_message(0, n))
        .collect()
}

/// Where [`sync_world`] finds files. The GUI backs it with the asset
/// folder; tests with maps in memory.
pub trait WorldLoader {
    /// The text of the `.gmap` file `name`, if it is available locally.
    fn gmap_text(&self, name: &str) -> Option<String>;
    /// The level file `name`, if it is available locally and may stand in
    /// for the server's: `server_mod_time` is the time the server gave for
    /// it (`PLO_LEVELMODTIME`), if any.
    fn level(&self, name: &str, server_mod_time: Option<u32>) -> Option<Level>;
}

/// What the network side has to do after [`sync_world`].
#[derive(Debug, Default, PartialEq)]
pub struct WorldRequests {
    /// The `.gmap` is not local: ask the server for this file (`WANTFILE`).
    /// Returned once per file.
    pub gmap_file: Option<String>,
    /// The `.gmap` is local: the file-cache rules decide whether to ask the
    /// server for a newer copy (`UPDATEFILE`). Returned once per file.
    pub gmap_local: Option<String>,
    /// `PLI_ADJACENTLEVEL` messages for neighbours the server has not sent
    /// yet, to go out in one frame. Each level is asked for once.
    pub messages: Vec<Vec<u8>>,
}

/// Apply the 186 blocks the session took since the last call to the levels
/// the walking side holds: the player's and the loaded neighbours.
pub fn apply_board_mods(session: &mut OfflineSession, net: &mut SessionState) {
    let mods = net.take_board_mods();
    if mods.is_empty() {
        return;
    }
    let current = net.current_level_name().map(|n| n.to_ascii_lowercase());
    for (level, m) in mods {
        if current.as_deref() == Some(level.as_str()) {
            if let Some(l) = session.current_level.as_mut() {
                l.apply_board_modify2(&m);
            }
        } else if let Some(w) = session.gmap.as_mut() {
            if let Some(cell) = w.map.cell_of(&level) {
                if let Some(l) = w.levels.get_mut(&cell) {
                    l.apply_board_modify2(&m);
                }
            }
        }
    }
}

/// Bring the walking session's gmap world in line with what the server has
/// said: the grid (from a local file, or a request for the server's), the
/// player's cell, the neighbouring levels (a local file, else the board the
/// server sent) and a `PLI_ADJACENTLEVEL` for each neighbour the server has
/// not sent. Off a gmap, or for a profile without gmaps, the world is
/// dropped. `asked` remembers what was already requested.
pub fn sync_world(
    session: &mut OfflineSession,
    net: &mut SessionState,
    rules: &GmapRules,
    loader: &dyn WorldLoader,
    asked: &mut HashSet<String>,
) -> WorldRequests {
    let mut req = WorldRequests::default();
    apply_board_mods(session, net);
    let Some(g) = net.gmap.as_ref().filter(|_| rules.enabled) else {
        session.leave_gmap();
        return req;
    };
    if g.map.is_none() {
        let grid = loader.gmap_text(&g.name).and_then(|t| Gmap::parse(&t));
        if let Some(m) = grid {
            let name = g.name.clone();
            net.set_gmap_grid(m);
            if asked.insert(format!("file:{}", name.to_ascii_lowercase())) {
                req.gmap_local = Some(name);
            }
        }
    }
    let g = net.gmap.as_ref().expect("checked above");
    let Some(map) = g.map.clone() else {
        if asked.insert(format!("file:{}", g.name.to_ascii_lowercase())) {
            req.gmap_file = Some(g.name.clone());
        }
        return req;
    };
    // The client clamps the cell a 49 gives to the grid.
    let cell = (g.cell.0.clamp(0, map.width as i32 - 1), g.cell.1.clamp(0, map.height as i32 - 1));
    let moved = cell != g.cell;
    match session.gmap.as_mut() {
        Some(w) if w.name.eq_ignore_ascii_case(&g.name) => {
            if w.cell != cell {
                // The server moved the player (a warp inside the gmap).
                w.levels.clear();
                w.cell = cell;
            }
        }
        _ => session.enter_gmap(g.name.clone(), map.clone(), cell),
    }
    let around = map.neighbours(cell);
    let map = map.clone();
    if moved {
        net.set_gmap_cell(cell);
    }
    let w = session.gmap.as_mut().expect("just set");
    w.levels.retain(|c, _| around.contains(c));
    for c in w.missing() {
        let Some(name) = map.level_at(c.0, c.1) else { continue };
        // The server's board first, as the real client draws it; a local
        // file only as the loader allows.
        let level = net
            .board_of(name)
            .map(|b| Level::from_board(b))
            .or_else(|| loader.level(name, net.server_mod_time(name)));
        if let Some(l) = level {
            w.levels.insert(c, l);
        }
    }
    if rules.adjacent {
        for &(x, y) in &around {
            let Some(name) = map.level_at(x, y) else { continue };
            if net.board_of(name).is_none() && asked.insert(format!("adj:{}", name.to_ascii_lowercase())) {
                req.messages.push(adjacent_level_message(0, name));
            }
        }
    }
    req
}

#[cfg(test)]
mod tests {
    #[test]
    fn big_map_neighbours_are_asked_for_near_an_edge() {
        let g = super::Gmap::parse_grid("\"nw.nw\",\"n.nw\",\"ne.nw\"\n\"w.nw\",\"mid.nw\",\"e.nw\"\n\"sw.nw\",\"\",\"se.nw\"\n").unwrap();
        // The middle of the level: nothing.
        assert!(g.neighbours_near_edge("mid.nw", 30.0, 30.0).is_empty());
        // Near the east edge: east only.
        assert_eq!(g.neighbours_near_edge("MID.nw", 55.0, 30.0), ["e.nw"]);
        // Near the north-west corner: west, north and the corner.
        assert_eq!(g.neighbours_near_edge("mid.nw", 2.0, 1.0), ["w.nw", "n.nw", "nw.nw"]);
        // South: a hole below, so only the corner to the south-east.
        assert_eq!(g.neighbours_near_edge("mid.nw", 60.0, 60.0), ["e.nw", "se.nw"]);
        // At the grid's edge there is nothing beyond.
        assert!(g.neighbours_near_edge("n.nw", 30.0, 0.0).is_empty());
        // A level not on the big map.
        assert!(g.neighbours_near_edge("house.nw", 0.0, 0.0).is_empty());
    }

    use super::*;

    #[test]
    fn a_big_map_text_is_a_grid_of_quoted_names() {
        let g = Gmap::parse_grid("\"a.nw\",\"b.nw\"\r\n\"c.nw\",\"\",\"e.nw\"\r\n\r\n").unwrap();
        assert_eq!((g.width, g.height), (3, 2));
        assert_eq!(g.neighbour_of("A.NW", 1, 0), Some("b.nw"));
        assert_eq!(g.neighbour_of("a.nw", 0, 1), Some("c.nw"));
        assert_eq!(g.neighbour_of("c.nw", 1, 0), None, "a hole");
        assert_eq!(g.neighbour_of("a.nw", -1, 0), None, "off the grid");
        assert_eq!(g.neighbour_of("x.nw", 1, 0), None);
        assert!(Gmap::parse_grid("\r\n\"\"\r\n").is_none());
    }

    const TWO_BY_TWO: &str = "GRMAP001\nWIDTH 2\nHEIGHT 2\nLEVELNAMES\n\"gm_nw.nw\",\"gm_ne.nw\",\n\"gm_sw.nw\",\"gm_se.nw\",\nLEVELNAMESEND\nMAPIMG over.png\n";

    fn map() -> Gmap {
        Gmap::parse(TWO_BY_TWO).unwrap()
    }

    #[test]
    fn parses_a_two_by_two_file() {
        let g = map();
        assert_eq!((g.width, g.height), (2, 2));
        assert_eq!(g.level_at(0, 0), Some("gm_nw.nw"));
        assert_eq!(g.level_at(1, 0), Some("gm_ne.nw"));
        assert_eq!(g.level_at(0, 1), Some("gm_sw.nw"));
        assert_eq!(g.level_at(1, 1), Some("gm_se.nw"));
        assert_eq!(g.map_img.as_deref(), Some("over.png"));
        assert_eq!(g.level_at(2, 0), None);
        assert_eq!(g.level_at(-1, 0), None);
    }

    #[test]
    fn size_comes_from_the_rows_without_width_and_height() {
        let g = Gmap::parse("GRMAP001\nLEVELNAMES\na.nw,b.nw,c.nw\nd.nw,e.nw,f.nw\nLEVELNAMESEND\n").unwrap();
        assert_eq!((g.width, g.height), (3, 2));
        assert_eq!(g.level_at(2, 1), Some("f.nw"));
    }

    #[test]
    fn holes_short_rows_and_crlf() {
        let t = "GRMAP001\r\nWIDTH 3\r\nHEIGHT 3\r\nLEVELNAMES\r\n\"a.nw\",\"\",\"c.nw\"\r\n\"d.nw\"\r\nLEVELNAMESEND\r\n";
        let g = Gmap::parse(t).unwrap();
        assert_eq!(g.level_at(0, 0), Some("a.nw"));
        assert_eq!(g.level_at(1, 0), None, "a quoted empty name is a hole");
        assert_eq!(g.level_at(2, 0), Some("c.nw"));
        assert_eq!(g.level_at(1, 1), None, "a short row leaves holes");
        assert_eq!(g.level_at(0, 2), None, "a missing row too");
    }

    #[test]
    fn flags_images_and_load_list() {
        let t = "GRMAP001\nWIDTH 1\nHEIGHT 1\nMINIMAPIMG mini.png\nNOAUTOMAPPING\nLOADFULLMAP\nLOADATSTART\n\"a.nw\",\"b.nw\"\nLOADATSTARTEND\nLEVELNAMES\n\"a.nw\"\nLEVELNAMESEND\n";
        let g = Gmap::parse(t).unwrap();
        assert!(g.no_auto_mapping && g.load_full_map);
        assert_eq!(g.minimap_img.as_deref(), Some("mini.png"));
        assert_eq!(g.load_at_start, vec!["a.nw", "b.nw"]);
    }

    #[test]
    fn bad_signature_or_empty_grid_is_none() {
        assert!(Gmap::parse("GLEVNW01\nWIDTH 1\n").is_none());
        assert!(Gmap::parse("GRMAP001\nWIDTH 2\nHEIGHT 2\n").is_some(), "declared size, all holes");
        assert!(Gmap::parse("GRMAP001\n").is_none());
        assert!(Gmap::parse("").is_none());
    }

    #[test]
    fn cell_lookup_is_case_insensitive() {
        let g = map();
        assert_eq!(g.cell_of("GM_SE.NW"), Some((1, 1)));
        assert_eq!(g.cell_of("nope.nw"), None);
        assert_eq!(g.neighbours((0, 0)), vec![(0, 1), (1, 0), (1, 1)]);
        assert_eq!(g.neighbours((1, 1)).len(), 3);
    }

    #[test]
    fn global_and_local_round_trip() {
        assert_eq!(to_global((1, 0), (0.0, 30.5)), (64.0, 30.5));
        assert_eq!(to_local((64.0, 30.5)), ((1, 0), (0.0, 30.5)));
        assert_eq!(to_local((63.5, 94.5)), ((0, 1), (63.5, 30.5)));
        assert_eq!(to_local((-0.5, 0.0)), ((-1, 0), (63.5, 0.0)));
        for cell in [(0, 0), (1, 0), (3, 2)] {
            for local in [(0.0, 0.0), (63.5, 12.0), (30.5, 63.5)] {
                assert_eq!(to_local(to_global(cell, local)), (cell, local));
            }
        }
    }

    #[test]
    fn crossing_wraps_like_the_lab_captures() {
        let g = map();
        // East: global x reaches 64.0.
        assert_eq!(g.resolve((0, 0), (63.5, 30.5)), Resolved::Inside);
        let Resolved::Crossed(c) = g.resolve((0, 0), (64.0, 30.5)) else { panic!() };
        assert_eq!((c.to, c.local_x, c.local_y), ((1, 0), 0.0, 30.5));
        assert!(c.x_changed() && !c.y_changed());
        // West: from cell 1 at local 0.0, one half-step back.
        let Resolved::Crossed(c) = g.resolve((1, 0), (-0.5, 30.5)) else { panic!() };
        assert_eq!((c.to, c.local_x), ((0, 0), 63.5));
        // South and north.
        let Resolved::Crossed(c) = g.resolve((0, 0), (10.0, 64.0)) else { panic!() };
        assert_eq!((c.to, c.local_y), ((0, 1), 0.0));
        assert!(c.y_changed() && !c.x_changed());
        let Resolved::Crossed(c) = g.resolve((0, 1), (10.0, -0.5)) else { panic!() };
        assert_eq!((c.to, c.local_y), ((0, 0), 63.5));
    }

    #[test]
    fn nothing_beyond_the_grid_or_into_a_hole() {
        let g = map();
        assert_eq!(g.resolve((0, 0), (-0.5, 5.0)), Resolved::NoLevel);
        assert_eq!(g.resolve((1, 1), (64.0, 5.0)), Resolved::NoLevel);
        let holey = Gmap::parse("GRMAP001\nWIDTH 2\nHEIGHT 1\nLEVELNAMES\n\"a.nw\",\"\"\nLEVELNAMESEND\n").unwrap();
        assert_eq!(holey.resolve((0, 0), (64.0, 5.0)), Resolved::NoLevel);
    }

    #[test]
    fn diagonal_crossing_changes_both_axes() {
        let g = map();
        let Resolved::Crossed(c) = g.resolve((0, 0), (64.0, 64.0)) else { panic!() };
        assert_eq!(c.to, (1, 1));
        assert!(c.x_changed() && c.y_changed());
    }

    #[test]
    fn rules_come_from_the_profile() {
        use ClientVersion::*;
        // 2.05 has no 49 handler and no cell props; 2.22 and 2.31 do.
        assert!(!GmapRules::for_version(V205).enabled);
        let r222 = GmapRules::for_version(V222);
        let r231 = GmapRules::for_version(V231);
        assert!(r222.enabled && r231.enabled);
        assert!(!r222.precise_pos && r231.precise_pos);
        assert!(r222.adjacent && r231.adjacent);
        for v in [V138, V139, V200] {
            assert!(!GmapRules::for_version(v).enabled, "{v:?}");
        }
    }

    fn cross(from: (i32, i32), to: (i32, i32), lx: f32, ly: f32) -> Crossing {
        Crossing { from, to, local_x: lx, local_y: ly }
    }

    /// The four props messages a real 2.31 sent in the lab (the id byte is
    /// the PLAYERPROPS id): `43 = 1, X2 = 0`, `43 = 0, X2 = 63.5`,
    /// `44 = 1, Y2 = 0`, `44 = 0, Y2 = 63.5`.
    #[test]
    fn props_message_matches_the_real_231_bytes() {
        let r = GmapRules::for_version(ClientVersion::V231);
        let east = crossing_props_message(&r, &cross((0, 0), (1, 0), 0.0, 30.5));
        assert_eq!(east, [&[0x22, 0x4b, 0x21, 0x6e, 0x20, 0x20][..], b"\n"].concat());
        let west = crossing_props_message(&r, &cross((1, 0), (0, 0), 63.5, 30.5));
        assert_eq!(west, [&[0x22, 0x4b, 0x20, 0x6e, 0x2f, 0x90][..], b"\n"].concat());
        let south = crossing_props_message(&r, &cross((0, 0), (0, 1), 10.0, 0.0));
        assert_eq!(south, [&[0x22, 0x4c, 0x21, 0x6f, 0x20, 0x20][..], b"\n"].concat());
        let north = crossing_props_message(&r, &cross((0, 1), (0, 0), 10.0, 63.5));
        assert_eq!(north, [&[0x22, 0x4c, 0x20, 0x6f, 0x2f, 0x90][..], b"\n"].concat());
    }

    /// A profile without precise positions writes the classic X/Y (x2 as a
    /// GCHAR), the cell prop first.
    #[test]
    fn props_message_uses_classic_coordinates_for_222() {
        let r = GmapRules::for_version(ClientVersion::V222);
        let east = crossing_props_message(&r, &cross((0, 0), (1, 0), 0.0, 30.5));
        assert_eq!(east, [&[0x22, 0x2f, 0x20, 0x4b, 0x21][..], b"\n"].concat());
        let west = crossing_props_message(&r, &cross((1, 0), (0, 0), 63.5, 30.5));
        assert_eq!(west, [&[0x22, 0x2f, 127 + 0x20, 0x4b, 0x20][..], b"\n"].concat());
        let south = crossing_props_message(&r, &cross((0, 0), (0, 1), 10.0, 0.5));
        assert_eq!(south, [&[0x22, 0x30, 0x21, 0x4c, 0x21][..], b"\n"].concat());
    }

    #[test]
    fn diagonal_props_message_writes_the_props_in_id_order() {
        let r = GmapRules::for_version(ClientVersion::V231);
        let m = crossing_props_message(&r, &cross((0, 0), (1, 1), 0.0, 0.0));
        let ids: Vec<u8> = crate::protocol::parse_player_props(&m[1..m.len() - 1]).iter().map(|p| p.id).collect();
        // Inferred: a diagonal step was never captured; ascending ids is the rule
        // both single-axis captures follow.
        assert_eq!(ids, vec![43, 44, 78, 79]);
    }

    /// The messages the props message parses back to what was crossed.
    #[test]
    fn props_message_round_trips_through_the_prop_parser() {
        use crate::protocol::{decode_pos_precise, parse_player_props_in};
        let v = ClientVersion::V231;
        let r = GmapRules::for_version(v);
        let own = v.profile().props.own;
        for (to, lx, ly) in [((1, 0), 0.0, 30.5), ((0, 1), 12.0, 0.0), ((0, 0), 63.5, 1.5)] {
            let from = if to == (0, 0) { (1, 0) } else { (0, 0) };
            let m = crossing_props_message(&r, &cross(from, to, lx, ly));
            let props = parse_player_props_in(&m[1..m.len() - 1], false, Some(&own));
            assert_eq!(props.len(), 2, "{to:?}");
            let (cell_prop, coord) = (&props[0], &props[1]);
            let cell = cell_prop.gchar().unwrap() as i32;
            assert_eq!(if cell_prop.id == 43 { cell } else { to.0 }, to.0);
            assert_eq!(if cell_prop.id == 44 { cell } else { to.1 }, to.1);
            let raw = crate::protocol::read_gshort(&coord.raw).unwrap();
            let want = if cell_prop.id == 43 { lx } else { ly };
            assert_eq!(decode_pos_precise(raw), want);
        }
    }

    #[test]
    fn crossing_frame_order_follows_the_profile() {
        let c = cross((0, 0), (1, 0), 0.0, 30.5);
        let ids = |f: &[Vec<u8>]| -> Vec<u8> { f.iter().map(|m| m[0] - 0x20).collect() };
        // 2.31: 46 then props. No tamper check at a crossing.
        let mut t231 = TamperSender::for_version(ClientVersion::V231);
        let f = crossing_frame(ClientVersion::V231, &c, t231.as_mut());
        assert_eq!(ids(&f), vec![46, 2]);
        assert_eq!(f[0], crate::net::enter_level_message());
        // 2.22: the tamper check, then props; without the opt-in just props.
        let mut t222 = TamperSender::for_version(ClientVersion::V222);
        let f = crossing_frame(ClientVersion::V222, &c, t222.as_mut());
        assert_eq!(ids(&f), vec![39, 2]);
        let f = crossing_frame(ClientVersion::V222, &c, None);
        assert_eq!(ids(&f), vec![2]);
        // A profile without gmaps sends nothing.
        assert!(crossing_frame(ClientVersion::V205, &c, None).is_empty());
    }

    #[test]
    fn adjacent_level_message_layout() {
        let m = adjacent_level_message(0, "gm_ne.nw");
        assert_eq!(m, [&[0x43, 0x20, 0x20, 0x20, 0x20, 0x20][..], b"gm_ne.nw\n"].concat());
        // A mod time round-trips through the GINT5 reader.
        let m = adjacent_level_message(1_781_284_436, "a.nw");
        let t = m[1..6].iter().fold(0u64, |a, &c| (a << 7) | (c - 0x20) as u64);
        assert_eq!(t, 1_781_284_436);
    }

    #[test]
    fn adjacent_requests_skip_cached_levels_and_follow_the_profile() {
        let g = map();
        let r = GmapRules::for_version(ClientVersion::V222);
        let none = HashMap::new();
        let all = adjacent_requests(&r, &g, (0, 0), &none);
        assert_eq!(all.len(), 3);
        assert!(all[0].ends_with(b"gm_sw.nw\n"), "columns first: sw, ne, se");
        assert!(all[1].ends_with(b"gm_ne.nw\n") && all[2].ends_with(b"gm_se.nw\n"));
        let mut have = HashMap::new();
        have.insert("gm_ne.nw".to_string(), 5);
        assert_eq!(adjacent_requests(&r, &g, (0, 0), &have).len(), 2);
        let off = GmapRules { adjacent: false, ..r };
        assert!(adjacent_requests(&off, &g, (0, 0), &none).is_empty());
        let disabled = GmapRules { enabled: false, ..r };
        assert!(adjacent_requests(&disabled, &g, (0, 0), &none).is_empty());
    }

    // ---- sync_world ----

    struct Files {
        grid: Option<String>,
        levels: HashMap<String, Level>,
    }

    impl WorldLoader for Files {
        fn gmap_text(&self, _: &str) -> Option<String> {
            self.grid.clone()
        }
        fn level(&self, name: &str, _: Option<u32>) -> Option<Level> {
            self.levels.get(name).cloned()
        }
    }

    fn msg(id: u8, payload: &[u8]) -> Vec<u8> {
        [&[id + 0x20][..], payload, b"\n"].concat()
    }

    fn login(net: &mut SessionState) {
        let p = [&[0x20 + 116, 0x20 + 61, 0x20 + 50, 0x20, 0x20][..], b"g.gmap"].concat();
        net.apply_body(&[msg(49, &p), msg(6, b"gm_nw.nw"), msg(6, b"g.gmap")].concat());
    }

    fn flat(mark: u16) -> Level {
        Level::from_board(&vec![mark; 64 * 64])
    }

    #[test]
    fn world_waits_for_the_gmap_file_and_asks_once() {
        let mut net = SessionState::for_version(ClientVersion::V222);
        login(&mut net);
        let mut session = OfflineSession::new();
        let rules = GmapRules::for_version(ClientVersion::V222);
        let files = Files { grid: None, levels: HashMap::new() };
        let mut asked = HashSet::new();
        let r = sync_world(&mut session, &mut net, &rules, &files, &mut asked);
        assert_eq!(r.gmap_file.as_deref(), Some("g.gmap"));
        assert!(r.messages.is_empty() && session.gmap.is_none());
        let r = sync_world(&mut session, &mut net, &rules, &files, &mut asked);
        assert_eq!(r, WorldRequests::default(), "asked only once");
    }

    #[test]
    fn a_local_gmap_is_offered_for_a_check_once() {
        let mut net = SessionState::for_version(ClientVersion::V222);
        login(&mut net);
        let mut session = OfflineSession::new();
        let rules = GmapRules::for_version(ClientVersion::V222);
        let files = Files { grid: Some(TWO_BY_TWO.into()), levels: HashMap::new() };
        let mut asked = HashSet::new();
        let r = sync_world(&mut session, &mut net, &rules, &files, &mut asked);
        assert_eq!(r.gmap_local.as_deref(), Some("g.gmap"));
        assert_eq!(r.gmap_file, None);
        let r = sync_world(&mut session, &mut net, &rules, &files, &mut asked);
        assert_eq!(r.gmap_local, None, "once");
    }

    #[test]
    fn world_loads_neighbours_and_asks_the_server_for_the_rest() {
        let mut net = SessionState::for_version(ClientVersion::V222);
        login(&mut net);
        let mut session = OfflineSession::new();
        let rules = GmapRules::for_version(ClientVersion::V222);
        let mut files = Files { grid: Some(TWO_BY_TWO.into()), levels: HashMap::new() };
        files.levels.insert("gm_ne.nw".into(), flat(1));
        let mut asked = HashSet::new();
        let r = sync_world(&mut session, &mut net, &rules, &files, &mut asked);
        // The grid came from the local file and went to the session too.
        assert!(net.gmap.as_ref().unwrap().map.is_some());
        let w = session.gmap.as_ref().unwrap();
        assert_eq!(w.cell, (0, 0));
        assert!(w.levels.contains_key(&(1, 0)), "from the local file");
        assert!(!w.levels.contains_key(&(0, 1)));
        // 35 for the three neighbours, once.
        assert_eq!(r.messages.len(), 3);
        assert!(r.messages[0].ends_with(b"gm_sw.nw\n"), "sw, ne, se");
        assert!(sync_world(&mut session, &mut net, &rules, &files, &mut asked).messages.is_empty());
        // The server's answer for gm_sw (name, board, gmap) fills the gap.
        let mut board = vec![100 + 0x20, 0x20, 64 + 0x20, 2 + 0x20, b'\n', 101 + 0x20];
        board.extend((0..64 * 64).flat_map(|_| 7u16.to_le_bytes()));
        board.push(b'\n');
        net.apply_body(&[msg(6, b"gm_sw.nw"), board, msg(6, b"g.gmap")].concat());
        sync_world(&mut session, &mut net, &rules, &files, &mut asked);
        let sw = session.gmap.as_ref().unwrap().levels.get(&(0, 1)).expect("built from the server board");
        assert_eq!(sw.tile_id_at(5, 5), Some(7));
    }

    /// A local file of the same name does not hide the board the server
    /// sent for a level.
    #[test]
    fn the_server_board_wins_over_a_local_file() {
        let mut net = SessionState::for_version(ClientVersion::V222);
        login(&mut net);
        let mut board = vec![100 + 0x20, 0x20, 64 + 0x20, 2 + 0x20, b'\n', 101 + 0x20];
        board.extend((0..64 * 64).flat_map(|_| 7u16.to_le_bytes()));
        board.push(b'\n');
        net.apply_body(&[msg(6, b"gm_ne.nw"), board, msg(6, b"g.gmap")].concat());
        let mut session = OfflineSession::new();
        let rules = GmapRules::for_version(ClientVersion::V222);
        let mut files = Files { grid: Some(TWO_BY_TWO.into()), levels: HashMap::new() };
        files.levels.insert("gm_ne.nw".into(), flat(1));
        let mut asked = HashSet::new();
        sync_world(&mut session, &mut net, &rules, &files, &mut asked);
        let ne = session.gmap.as_ref().unwrap().levels.get(&(1, 0)).expect("gm_ne is drawn");
        assert_eq!(ne.tile_id_at(5, 5), Some(7), "the server's board, not the local file");
    }

    #[test]
    fn world_follows_a_crossing_and_keeps_only_neighbours() {
        let mut net = SessionState::for_version(ClientVersion::V231);
        login(&mut net);
        let mut session = OfflineSession::new();
        let rules = GmapRules::for_version(ClientVersion::V231);
        let mut files = Files { grid: Some(TWO_BY_TWO.into()), levels: HashMap::new() };
        for n in ["gm_nw.nw", "gm_ne.nw", "gm_sw.nw", "gm_se.nw"] {
            files.levels.insert(n.into(), flat(2));
        }
        let mut asked = HashSet::new();
        session.current_level = files.level("gm_nw.nw", None);
        sync_world(&mut session, &mut net, &rules, &files, &mut asked);
        // Walk east across the edge: the session and the net follow.
        session.player_x = 63.5;
        session.player_y = 10.0;
        session.tiletypes = None;
        assert!(session.try_move(0.5, 0.0).is_some());
        let c = session.take_gmap_crossing().unwrap();
        net.set_gmap_cell(c.to);
        sync_world(&mut session, &mut net, &rules, &files, &mut asked);
        assert_eq!(net.current_level_name().as_deref(), Some("gm_ne.nw"));
        let w = session.gmap.as_ref().unwrap();
        assert_eq!(w.cell, (1, 0));
        assert!(w.levels.contains_key(&(0, 0)) && w.levels.contains_key(&(0, 1)) && w.levels.contains_key(&(1, 1)));
        assert_eq!(w.levels.len(), 3);
    }

    #[test]
    fn world_is_dropped_off_a_gmap_or_without_gmap_rules() {
        let mut net = SessionState::for_version(ClientVersion::V222);
        login(&mut net);
        let mut session = OfflineSession::new();
        let files = Files { grid: Some(TWO_BY_TWO.into()), levels: HashMap::new() };
        let mut asked = HashSet::new();
        let rules = GmapRules::for_version(ClientVersion::V222);
        sync_world(&mut session, &mut net, &rules, &files, &mut asked);
        assert!(session.gmap.is_some());
        let off = GmapRules { enabled: false, ..rules };
        sync_world(&mut session, &mut net, &off, &files, &mut asked);
        assert!(session.gmap.is_none());
        sync_world(&mut session, &mut net, &rules, &files, &mut asked);
        net.apply_body(&msg(49, &[&[0x20 + 2, 0x20 + 4, 0x20 + 50, 0x20, 0x20][..], b"b.nw"].concat()));
        sync_world(&mut session, &mut net, &rules, &files, &mut asked);
        assert!(session.gmap.is_none(), "a plain 49 leaves the gmap");
    }

    /// 186 reaches the walking side's copy of the right level.
    #[test]
    fn board_mods_reach_the_current_and_neighbour_levels() {
        let mut net = SessionState::for_version(ClientVersion::V222);
        login(&mut net);
        let mut session = OfflineSession::new();
        let rules = GmapRules::for_version(ClientVersion::V222);
        let mut files = Files { grid: Some(TWO_BY_TWO.into()), levels: HashMap::new() };
        files.levels.insert("gm_ne.nw".into(), flat(1));
        session.current_level = Some(flat(1));
        let mut asked = HashSet::new();
        sync_world(&mut session, &mut net, &rules, &files, &mut asked);
        let block = |mx: u8| {
            let mut p = vec![0x20 + mx, 0x20, 0x20 + 62, 0x20 + 2, 0x20 + 4, 0x20 + 1];
            for _ in 0..4 {
                p.extend([0x20, 0x20 + 9]);
            }
            msg(186, &p)
        };
        net.apply_body(&[block(0), block(1)].concat());
        apply_board_mods(&mut session, &mut net);
        let own = session.current_level.as_ref().unwrap();
        assert_eq!((own.tile_id_at(61, 2), own.tile_id_at(62, 2), own.tile_id_at(63, 2)), (Some(1), Some(9), Some(9)));
        let ne = session.gmap.as_ref().unwrap().levels.get(&(1, 0)).unwrap();
        assert_eq!((ne.tile_id_at(62, 2), ne.tile_id_at(0, 3)), (Some(9), Some(1)));
    }

    /// A 49 whose cell is outside the grid is clamped to it, as the real
    /// client does.
    #[test]
    fn a_cell_outside_the_grid_is_clamped() {
        let mut net = SessionState::for_version(ClientVersion::V222);
        let p = [&[0x20 + 116, 0x20 + 61, 0x20 + 50, 0x20 + 5, 0x20 + 9][..], b"g.gmap"].concat();
        net.apply_body(&[msg(49, &p), msg(6, b"gm_se.nw"), msg(6, b"g.gmap")].concat());
        let mut session = OfflineSession::new();
        let rules = GmapRules::for_version(ClientVersion::V222);
        let files = Files { grid: Some(TWO_BY_TWO.into()), levels: HashMap::new() };
        sync_world(&mut session, &mut net, &rules, &files, &mut HashSet::new());
        assert_eq!(net.gmap.as_ref().unwrap().cell, (1, 1));
        assert_eq!(session.gmap.as_ref().unwrap().cell, (1, 1));
        assert_eq!(net.current_level_name().as_deref(), Some("gm_se.nw"));
    }
}
