//! Level file parser (.nw / GLEVNW01 format).

use std::fs;
use std::path::Path;

/// A single tile (2-character code, e.g. "Id").
pub type Tile = [u8; 2];

/// Encode a 12-bit tile id (0–4095) as its 2-char base64 code.
pub fn tile_code(id: u16) -> Tile {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    [B64[((id >> 6) & 0x3F) as usize], B64[(id & 0x3F) as usize]]
}

/// Decode a 2-char base64 tile code into a 12-bit tile id (0–4095).
pub fn tile_id(tile: Tile) -> u16 {
    fn val(c: u8) -> u16 {
        match c {
            b'A'..=b'Z' => (c - b'A') as u16,
            b'a'..=b'z' => (c - b'a' + 26) as u16,
            b'0'..=b'9' => (c - b'0' + 52) as u16,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    }
    (val(tile[0]) << 6) | val(tile[1])
}

/// One BOARD line: a horizontal run of tiles.
#[derive(Debug, Clone)]
pub struct BoardRun {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub layer: i32,
    pub tiles: Vec<Tile>,
}

/// An NPC defined in the level file.
#[derive(Debug, Clone)]
pub struct Npc {
    pub image: String,
    pub x: f32,
    pub y: f32,
    pub script: String,
    /// Visibility flag. `hide`/`show` commands toggle this.
    pub visible: bool,
    /// The part of `image` that is drawn, `(x, y, w, h)` in pixels, set by
    /// `setimgpart` (`setimg` resets it to the whole image).
    pub image_part: Option<(i32, i32, i32, i32)>,
    /// The animation the NPC is drawn with (`setcharani`, `setani`, a
    /// `showcharacter` NPC), lower case and without `.gani`. While set, the
    /// NPC is drawn as that animation instead of its image.
    pub ani: Option<String>,
    /// Step of `ani` being shown, and the milliseconds spent on it.
    pub ani_frame: usize,
    pub ani_ms: u64,
    /// Images the NPC's script put up with `showimg`, by index.
    pub images: Vec<NpcImage>,
    /// Character parts set by `setcharprop`.
    pub head: Option<String>,
    pub body: Option<String>,
    pub shield: Option<String>,
    /// Other character props, by code.
    pub props: std::collections::HashMap<String, String>,
    /// Whether the NPC stops movement. `dontblock` sets this false.
    pub blocks: bool,
    /// `canbepushed` / `canbepulled`: a push or a pull moves the NPC.
    pub pushable: bool,
    pub pullable: bool,
    /// Collision size in pixels, from `setshape`.
    pub shape: Option<(f32, f32)>,
    /// Where the NPC is drawn against the players: `drawunderplayer`,
    /// `drawoverplayer`, `drawaslight`; `blockagain` puts it back.
    pub layer: DrawLayer,
    /// `setcoloreffect` and `setzoomeffect`.
    pub effect: ImageEffect,
    /// The server's id for this NPC online (`PLO_NPCPROPS`), once known.
    pub server_id: Option<u32>,
    /// The text of the NPC's last `message` (empty for none).
    pub message: String,
    /// The script's `dir` (0 up, 1 left, 2 down, 3 right).
    pub dir: u8,
    /// How many times a script ran `setcharani` on it (wrapping): 2.05
    /// sends GANI after each one, even with the same animation.
    pub ani_calls: u32,
}

/// The drawing height of an NPC. The 2.05 client keeps it as one number
/// per NPC: -1 under, 0 with the players, 1 over, 9 as a light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DrawLayer {
    /// `drawunderplayer`: before everything else that moves.
    Under,
    /// The default: before the players.
    #[default]
    Player,
    /// `drawoverplayer`: after the players.
    Over,
    /// `drawaslight`: after the `drawoverplayer` NPCs, under the status bar.
    Light,
}

impl DrawLayer {
    /// The number the 2.05 client keeps: -1, 0, 1 or 9.
    pub fn height(self) -> i32 {
        match self {
            DrawLayer::Under => -1,
            DrawLayer::Player => 0,
            DrawLayer::Over => 1,
            DrawLayer::Light => 9,
        }
    }
}

/// The height `changeimgvis` gives an image of an object at `owner`
/// height: 0, 1, 2 are one under, level with and one over the owner; 3 is
/// the light height (9) and 4 the screen layer (10), whatever the owner.
/// Other values change nothing.
pub fn image_vis_height(vis: i64, owner: i32) -> Option<i32> {
    match vis {
        0..=2 => Some(owner + vis as i32 - 1),
        3 | 4 => Some(vis as i32 + 6),
        _ => None,
    }
}

/// An image a script put up with `showimg index,file,x,y` (index 0 to 199).
/// `hideimg index` removes it.
#[derive(Debug, Clone, PartialEq)]
pub struct NpcImage {
    pub index: i64,
    pub file: String,
    /// Level position in tiles (the same unit as an NPC's x and y).
    pub x: f32,
    pub y: f32,
    pub part: Option<(i32, i32, i32, i32)>,
    /// `changeimgcolors` and `changeimgzoom`.
    pub effect: ImageEffect,
    /// Drawing height, on the scale of [`DrawLayer::height`]: `showimg`
    /// puts the image one above its NPC, `changeimgvis` moves it. 10 and up
    /// is the screen layer, where `x` and `y` are screen pixels.
    pub height: i32,
}

/// The first height of the screen layer (`changeimgvis i,4`).
pub const SCREEN_HEIGHT: i32 = 10;

/// How an NPC's picture, or a `showimg` image, is coloured and scaled:
/// `setcoloreffect` / `changeimgcolors` (red, green, blue, alpha; default
/// 1 each) and `setzoomeffect` / `changeimgzoom` (default 1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageEffect {
    pub color: [f32; 4],
    pub zoom: f32,
}

impl Default for ImageEffect {
    fn default() -> Self {
        ImageEffect { color: [1.0; 4], zoom: 1.0 }
    }
}

impl ImageEffect {
    /// True when drawing needs the effect path: a colour or alpha below 1,
    /// or a zoom other than 1 (the 2.05 test).
    pub fn is_on(&self) -> bool {
        self.color.iter().any(|&c| c < 1.0) || self.zoom != 1.0
    }
}

impl Npc {
    /// An NPC with only its image, position and script.
    pub fn new(image: String, x: f32, y: f32, script: String) -> Npc {
        Npc { image, x, y, script, visible: true, image_part: None, ani: None, ani_frame: 0, ani_ms: 0, images: Vec::new(),
            head: None, body: None, shield: None, props: std::collections::HashMap::new(), blocks: true, pushable: false, pullable: false, shape: None, layer: DrawLayer::Player, effect: ImageEffect::default(), server_id: None,
            message: String::new(), dir: 2, ani_calls: 0 }
    }

    /// Every image file the NPC draws now: its own and its `showimg` ones.
    pub fn image_names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.image.as_str()).chain(self.images.iter().map(|i| i.file.as_str())).filter(|n| !n.is_empty())
    }
}

/// A level link (LINK record).
#[derive(Debug, Clone)]
pub struct Link {
    pub dest: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Arrival position in the destination level. `None` means the
    /// link says `playerx` / `playery`: keep the player's coordinate.
    pub new_x: Option<f32>,
    pub new_y: Option<f32>,
    /// Written after `playerx` / `playery` (`playery+10`, `playerx-15`):
    /// the signed number, as written. Only meaningful when the matching
    /// `new_*` is `None`; 0 otherwise. Whether a client adds it is open
    /// (found in old player-world levels; see `documentation/assets.md`).
    pub x_offset: f32,
    pub y_offset: f32,
}

impl Link {
    /// True if tile `(x, y)` is inside the link's rectangle.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }

    /// True if the box `(x0, y0)..(x1, y1)`, in tiles, overlaps the link's
    /// rectangle. Boxes that only touch an edge do not overlap.
    pub fn overlaps(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
        let (lx0, ly0) = (self.x as f32, self.y as f32);
        let (lx1, ly1) = ((self.x + self.width) as f32, (self.y + self.height) as f32);
        x1 > lx0 && x0 < lx1 && y1 > ly0 && y0 < ly1
    }
}

/// A sign (SIGN / SIGNEND). Text is already plain for GLEVNW01.
#[derive(Debug, Clone)]
pub struct Sign {
    pub x: i32,
    pub y: i32,
    pub text: String,
}

/// A baddy (BADDY / BADDYEND).
#[derive(Debug, Clone)]
pub struct Baddy {
    pub x: f32,
    pub y: f32,
    pub kind: String,
    pub verses: String,
}

/// A chest (CHEST x y item signindex).
#[derive(Debug, Clone)]
pub struct Chest {
    pub x: i32,
    pub y: i32,
    pub item: String,
    pub sign_index: i32,
    /// The server sent this chest as already opened (the short record). A
    /// chest from a level file is closed; one opened in play is tracked by
    /// the session.
    pub opened: bool,
}

/// A 2 x 2 object as the client's tables hold it, `[column][row]`.
pub type Object2x2 = [[u16; 2]; 2];

/// The cut and lift tables of `arrays.dat` (the arrays after the walking
/// ones). Each list is one entry per object type; an entry is the object's
/// 2 x 2 tiles, and the `_into` list holds the 2 x 2 tiles that replace it.
///
/// `arrays.dat` is a run of arrays, each with a big-endian `u32` header whose
/// low 16 bits are the count and whose high 16 bits are 0 for a flat list of
/// `u32` ints and non-zero for a list of nested arrays (each with its own
/// header). In file order, array 9 is the cuttable objects, 10 what they
/// leave, 11 the liftable objects, 12 what they leave.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectTables {
    pub cut: Vec<Object2x2>,
    pub cut_into: Vec<Object2x2>,
    pub lift: Vec<Object2x2>,
    pub lift_into: Vec<Object2x2>,
    /// Array 19: the break effects, `[kind][frame][piece]`, each piece a
    /// packed `u32` (see [`crate::throwing::unpack_piece`]).
    pub leaps: Vec<Vec<Vec<u32>>>,
    /// Array 24: the sprite table, one packed rectangle per sprite id (see
    /// [`crate::throwing::unpack_sprite_rect`]).
    pub sprites: Vec<u32>,
    /// Array 8: the ledge tiles, one list per facing (up, left, down,
    /// right). These are the tiles the 2.05 client calls type 21: running
    /// against one jumps the player off it (see [`crate::ledge`]).
    pub ledges: [Vec<u16>; 4],
    /// Array 16: the stone blocks. Entry 0 can be pushed, entry 1 pulled
    /// (see [`crate::ledge`]).
    pub blocks: Vec<Object2x2>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ArrNode {
    Ints(Vec<u32>),
    List(Vec<ArrNode>),
}

fn parse_arr_node(data: &[u8], off: &mut usize, depth: u32) -> Option<ArrNode> {
    if depth > 4 || *off + 4 > data.len() {
        return None;
    }
    let h = u32::from_be_bytes(data[*off..*off + 4].try_into().ok()?);
    *off += 4;
    let (kind, n) = (h >> 16, (h & 0xFFFF) as usize);
    if kind == 0 {
        let end = off.checked_add(n.checked_mul(4)?)?;
        let ints = data.get(*off..end)?.chunks_exact(4).map(|c| u32::from_be_bytes(c.try_into().unwrap())).collect();
        *off = end;
        Some(ArrNode::Ints(ints))
    } else {
        let mut v = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            v.push(parse_arr_node(data, off, depth + 1)?);
        }
        Some(ArrNode::List(v))
    }
}

fn leaps_from(node: Option<&ArrNode>) -> Vec<Vec<Vec<u32>>> {
    let Some(ArrNode::List(kinds)) = node else { return Vec::new() };
    kinds
        .iter()
        .map(|k| match k {
            ArrNode::List(frames) => frames
                .iter()
                .map(|f| match f {
                    ArrNode::Ints(v) => v.clone(),
                    _ => Vec::new(),
                })
                .collect(),
            _ => Vec::new(),
        })
        .collect()
}

fn ledges_from(node: Option<&ArrNode>) -> [Vec<u16>; 4] {
    let mut out: [Vec<u16>; 4] = Default::default();
    if let Some(ArrNode::List(dirs)) = node {
        for (d, list) in dirs.iter().take(4).enumerate() {
            if let ArrNode::Ints(v) = list {
                out[d] = v.iter().filter(|&&id| id < 4096).map(|&id| id as u16).collect();
            }
        }
    }
    out
}

fn objects_from(node: Option<&ArrNode>) -> Vec<Object2x2> {
    let Some(ArrNode::List(items)) = node else { return Vec::new() };
    items
        .iter()
        .filter_map(|it| {
            let ArrNode::List(cols) = it else { return None };
            let mut o = [[0u16; 2]; 2];
            if cols.len() != 2 {
                return None;
            }
            for (c, col) in cols.iter().enumerate() {
                let ArrNode::Ints(rows) = col else { return None };
                if rows.len() != 2 {
                    return None;
                }
                for (r, &v) in rows.iter().enumerate() {
                    o[c][r] = v as u16;
                }
            }
            Some(o)
        })
        .collect()
}

impl ObjectTables {
    /// Read the tables from an `arrays.dat`. `None` if the file does not
    /// parse as a run of arrays or has no cut or lift tables.
    pub fn parse(data: &[u8]) -> Option<Self> {
        let mut off = 0;
        let mut nodes = Vec::new();
        while off < data.len() {
            nodes.push(parse_arr_node(data, &mut off, 0)?);
        }
        let t = ObjectTables {
            cut: objects_from(nodes.get(9)),
            cut_into: objects_from(nodes.get(10)),
            lift: objects_from(nodes.get(11)),
            lift_into: objects_from(nodes.get(12)),
            leaps: leaps_from(nodes.get(19)),
            sprites: match nodes.get(24) {
                Some(ArrNode::Ints(v)) => v.clone(),
                _ => Vec::new(),
            },
            ledges: ledges_from(nodes.get(8)),
            blocks: objects_from(nodes.get(16)),
        };
        (!t.cut.is_empty() || !t.lift.is_empty()).then_some(t)
    }
}

/// Tile type byte for water (`isOnWater`).
pub const TYPE_WATER: u8 = 11;
/// Tile type byte for lava: liquid, so the player swims, and it burns.
pub const TYPE_LAVA: u8 = 12;
/// A passable tile in the lava list but not the liquid list: walked on,
/// and it burns. Seihai's own number; the client keeps lists, not bytes,
/// and `tiletype` answers -1 on this tileset anyway.
pub const TYPE_LAVA_FLOOR: u8 = 13;
/// Type bytes from here up are walls (`isOnWall`).
pub const TYPE_WALL_MIN: u8 = 20;
/// Type given to tiles that `arrays.dat` does not list as passable.
const TYPE_HARD_WALL: u8 = 22;
/// Chair (bench, couch): walkable, and the player sits on it.
pub const TYPE_CHAIR: u8 = 3;
/// The top and bottom halves of a bed (`arrays.dat` arrays 5 and 6).
pub const TYPE_BED_TOP: u8 = 4;
pub const TYPE_BED_BOTTOM: u8 = 5;
/// A wall that thrown objects fly over (`arrays.dat` array 2).
pub const TYPE_THROW_THROUGH: u8 = 20;

/// The tile type of `id` in a tileset of the new kind (`addtiledef` with
/// type 1 or 2), where the type follows from where the tile sits on the
/// sheet instead of from `arrays.dat`. This is the 2.05 `tiletype(x, y)`
/// rule; `row` is `id / 16` and `col` is `id % 16`.
///
/// The checks run in this order, so the lava block (192 to 287) sits inside
/// the water block (64 to 287) and the rule never answers 12.
pub fn new_world_tile_type(id: u16) -> u8 {
    let (id, col, row) = (u32::from(id), u32::from(id) % 16, u32::from(id) / 16);
    let wall = id == 32
        || (col >= 2 && (26..=27).contains(&row))
        || (30..=47).contains(&row)
        || (84..=95).contains(&row)
        || (116..=127).contains(&row)
        || (row >= 128 && (row >> 4) & 1 == 1);
    if wall {
        return if (0x200..0x300).contains(&id) {
            TYPE_THROW_THROUGH
        } else if col >= 8 && (30..=31).contains(&row) {
            21
        } else {
            TYPE_HARD_WALL
        };
    }
    match id {
        0x120..=0x13f => 2,
        0x140..=0x17f => TYPE_CHAIR,
        0x180..=0x18f => TYPE_BED_TOP,
        0x190..=0x19f => TYPE_BED_BOTTOM,
        0x40..=0x11f => TYPE_WATER,
        _ if col < 2 && (26..=27).contains(&row) => 6,
        0x300..=0x31f => 6,
        0x320..=0x33f => 7,
        0x340..=0x38f => 8,
        0x390..=0x3af => 9,
        0x3b0..=0x3df => 10,
        _ => 0,
    }
}

/// What a tile does to a walking player.
///
/// The 2.05 client asks one list per question: wall, liquid (swimming)
/// and lava (burning). Water (11) and lava (12) are both liquid, so the
/// player swims in both; lava also burns (see
/// [`TileTypes::is_lava`]). Every other type below 20 is plain floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileKind {
    Open,
    Water,
    Lava,
    Blocking,
}

/// 4096-byte collision table: one type byte per tile id.
/// Wall >= 20, water == 11, lava == 12.
#[derive(Debug, Clone)]
pub struct TileTypes {
    pub types: [u8; 4096],
    /// Bed tops (`arrays.dat` array 5, or a tiledefs `bedtop:` line),
    /// sorted. Kept apart from `types` because the bed top is also a wall
    /// there, and the client asks each list on its own; a tile with type
    /// byte 4 is a bed top as well.
    pub bed_tops: Vec<u16>,
}

impl TileTypes {
    /// A table with these type bytes and no extra bed tops.
    pub fn from_types(types: [u8; 4096]) -> Self {
        TileTypes { types, bed_tops: Vec::new() }
    }

    /// Load a text tiledefs file.
    /// Accepts either a flat comma-separated list or
    /// `index,x,y,type,name` rows.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut types = [0u8; 4096];
        let mut bed_tops = Vec::new();
        let mut i = 0;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // `bedtop: 708-711`: bed tops whose type byte is a wall (ids and
            // ranges, space separated). See `bed_tops`.
            if let Some(ids) = line.strip_prefix("bedtop:") {
                for part in ids.split_whitespace() {
                    let (a, b) = part.split_once('-').unwrap_or((part, part));
                    let (Ok(a), Ok(b)) = (a.parse::<u16>(), b.parse::<u16>()) else {
                        return Err(format!("tiledefs: bad bedtop entry {part}"));
                    };
                    bed_tops.extend((a..=b.min(4095)).filter(|&id| id < 4096));
                }
                continue;
            }
            let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            // index,x,y,type[,name]; longer lines are flat lists
            if (4..=5).contains(&parts.len()) && parts[0].parse::<u16>().is_ok() {
                if let Ok(idx) = parts[0].parse::<usize>() {
                    if idx < 4096 {
                        types[idx] = parts[3].parse().unwrap_or(0);
                        i = i.max(idx + 1);
                        continue;
                    }
                }
            }
            // flat list
            for part in parts {
                if part.is_empty() {
                    continue;
                }
                if i >= 4096 {
                    break;
                }
                types[i] = part.parse().unwrap_or(0);
                i += 1;
            }
        }
        if i < 4096 {
            return Err(format!("tiledefs short: {i} values"));
        }
        bed_tops.sort_unstable();
        bed_tops.dedup();
        Ok(TileTypes { types, bed_tops })
    }

    /// Load a Graal `arrays.dat`. See [`Self::parse_arrays`].
    pub fn load_arrays<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let data = fs::read(path).map_err(|e| e.to_string())?;
        Self::parse_arrays(&data)
    }

    /// Parse a Graal `arrays.dat`.
    ///
    /// Format (big-endian, from the Zelda Online `makeArrays` lineage):
    /// a run of length-prefixed int arrays. The ones used here:
    ///
    /// - array 0: the passable-tile list (`nowall`). Listed tiles are
    ///   type 0; every tile not listed is a hard wall (22).
    /// - array 1: the liquid list. Water and lava tiles are in it, and
    ///   many of the water tiles are *not* in `nowall`, so the first
    ///   version of this loader turned water into wall.
    /// - array 2: the throw-through list. A listed tile that is a wall
    ///   gets type 20, the wall thrown objects fly over.
    /// - array 3: the lava list. A liquid tile that is also here is lava
    ///   (12, swum in and burning); one that is not is water (11). A
    ///   passable tile that is here but not liquid is lava floor (13,
    ///   walked on and burning).
    /// - arrays 4, 5 and 6: chairs (3), bed tops (4) and bed bottoms (5).
    ///   Only a passable tile takes these types.
    ///
    /// The 2.05 client keeps these as lists and asks one list per type
    /// test, so a tile can be in two of them (6 throw-through tiles of the
    /// 2.22 file are also passable). One type byte per tile holds that as
    /// long as walking comes first: a passable tile stays walkable, and a
    /// wall only changes to 20 when it is in the throw-through list. See
    /// `documentation/tile-types.md`.
    ///
    /// Any array that is cut short or implausibly long ends the scan, so a
    /// file with only the first array still loads.
    pub fn parse_arrays(data: &[u8]) -> Result<Self, String> {
        if data.len() < 4 {
            return Err("arrays.dat too short".into());
        }
        let mut arrays: Vec<Vec<usize>> = Vec::new();
        let mut off = 0usize;
        while arrays.len() < 7 && off + 4 <= data.len() {
            let count = u32::from_be_bytes(data[off..off + 4].try_into().unwrap()) as usize;
            let end = off.saturating_add(4).saturating_add(count.saturating_mul(4));
            if end > data.len() || (!arrays.is_empty() && count > 4096) {
                if arrays.is_empty() {
                    return Err(format!("arrays.dat truncated: need {count} ints"));
                }
                break;
            }
            arrays.push(
                data[off + 4..end]
                    .chunks_exact(4)
                    .map(|c| u32::from_be_bytes(c.try_into().unwrap()) as usize)
                    .collect(),
            );
            off = end;
        }
        let mut types = [TYPE_HARD_WALL; 4096];
        if let Some(nowall) = arrays.first() {
            for &id in nowall.iter().filter(|&&id| id < 4096) {
                types[id] = 0;
            }
        }
        let lava: std::collections::HashSet<usize> =
            arrays.get(3).map(|a| a.iter().copied().collect()).unwrap_or_default();
        if let Some(liquid) = arrays.get(1) {
            for &id in liquid.iter().filter(|&&id| id < 4096) {
                types[id] = if lava.contains(&id) { TYPE_LAVA } else { TYPE_WATER };
            }
        }
        if let Some(through) = arrays.get(2) {
            for &id in through.iter().filter(|&&id| id < 4096) {
                if types[id] == TYPE_HARD_WALL {
                    types[id] = TYPE_THROW_THROUGH;
                }
            }
        }
        // Arrays 4 to 6: chairs (benches, chairs, couches) and the two halves
        // of a bed. They keep walking behaviour and only get the type byte of
        // the scripting language's tile-type list.
        for (index, kind) in [(4, TYPE_CHAIR), (5, TYPE_BED_TOP), (6, TYPE_BED_BOTTOM)] {
            for &id in arrays.get(index).into_iter().flatten().filter(|&&id| id < 4096) {
                if types[id] == 0 {
                    types[id] = kind;
                }
            }
        }
        for &id in lava.iter().filter(|&&id| id < 4096) {
            if types[id] == 0 {
                types[id] = TYPE_LAVA_FLOOR;
            }
        }
        let mut bed_tops: Vec<u16> = arrays.get(5).into_iter().flatten().filter(|&&id| id < 4096).map(|&id| id as u16).collect();
        bed_tops.sort_unstable();
        bed_tops.dedup();
        Ok(TileTypes { types, bed_tops })
    }

    /// True if `id` is a chair tile (type 3): standing on it draws the sit
    /// pose.
    pub fn is_chair(&self, id: u16) -> bool {
        self.raw(id) == Some(TYPE_CHAIR)
    }

    /// True if `id` is the top half of a bed (type 4): the head on it with
    /// both probes draws the sleep pose. The bottom half (type 5) is never
    /// tested for that.
    pub fn is_bed_top(&self, id: u16) -> bool {
        self.raw(id) == Some(TYPE_BED_TOP) || self.bed_tops.binary_search(&id).is_ok()
    }

    /// The raw type byte for `id`, if the id is on the table.
    pub fn raw(&self, id: u16) -> Option<u8> {
        self.types.get(id as usize).copied()
    }

    /// What the tile does to a walking player. An id off the table is a
    /// wall, as in the client.
    pub fn kind(&self, id: u16) -> TileKind {
        match self.raw(id) {
            None => TileKind::Blocking,
            Some(t) if t >= TYPE_WALL_MIN => TileKind::Blocking,
            Some(TYPE_WATER) => TileKind::Water,
            Some(TYPE_LAVA) => TileKind::Lava,
            Some(_) => TileKind::Open,
        }
    }

    pub fn is_wall(&self, id: u16) -> bool {
        self.kind(id) == TileKind::Blocking
    }

    pub fn is_water(&self, id: u16) -> bool {
        self.kind(id) == TileKind::Water
    }

    /// True if `id` is in the lava list: lava (12) or lava floor (13). The
    /// client's burn test asks only this list.
    pub fn is_lava(&self, id: u16) -> bool {
        matches!(self.raw(id), Some(TYPE_LAVA | TYPE_LAVA_FLOOR))
    }
}

#[cfg(test)]
mod tiletypes_tests {
    use super::*;

    #[test]
    fn arrays_marks_nowall_passable() {
        // count=2, ids 0 and 8
        let mut data = Vec::new();
        data.extend_from_slice(&2u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(&8u32.to_be_bytes());
        let t = TileTypes::parse_arrays(&data).unwrap();
        assert!(!t.is_wall(0));
        assert!(!t.is_wall(8));
        assert!(t.is_wall(1));
    }

    fn arrays_blob(arrays: &[&[u32]]) -> Vec<u8> {
        let mut data = Vec::new();
        for a in arrays {
            data.extend_from_slice(&(a.len() as u32).to_be_bytes());
            for v in *a {
                data.extend_from_slice(&v.to_be_bytes());
            }
        }
        data
    }

    /// Array 4 (chair tiles) marks passable tiles as chairs and never makes a
    /// wall walkable; bed arrays 5 and 6 mark the two halves of a bed.
    #[test]
    fn arrays_chair_list_marks_chairs() {
        // passable: 0, 8, 9, 10; chairs: 8 and 40 (not passable); bed: 9 / 10, 41
        let data = arrays_blob(&[&[0, 8, 9, 10], &[], &[], &[], &[8, 40], &[9], &[10, 41]]);
        let t = TileTypes::parse_arrays(&data).unwrap();
        assert!(t.is_chair(8));
        assert_eq!(t.kind(8), TileKind::Open, "a bench is walkable");
        assert!(!t.is_chair(40) && t.is_wall(40), "a wall in the chair list stays a wall");
        assert!(!t.is_chair(0) && !t.is_chair(9));
        assert_eq!([9, 10, 41].map(|id| t.raw(id)), [Some(TYPE_BED_TOP), Some(TYPE_BED_BOTTOM), Some(TYPE_HARD_WALL)]);
        assert_eq!(t.kind(9), TileKind::Open, "a bed is walkable");
        // A bed top that is not passable (as in the 2.22 file) stays a wall
        // and is still a bed top.
        let t = TileTypes::parse_arrays(&arrays_blob(&[&[0], &[], &[], &[], &[], &[9], &[10]])).unwrap();
        assert!(t.is_wall(9) && t.is_bed_top(9));
        assert!(!t.is_bed_top(10) && !t.is_bed_top(0));
        // A text table marks wall bed tops with a `bedtop:` line.
        let flat = format!("bedtop: 9 20-21\n{}\n", vec!["22"; 4096].join(","));
        let t = TileTypes::parse(&flat).unwrap();
        assert!(t.is_bed_top(9) && t.is_bed_top(20) && t.is_bed_top(21) && t.is_wall(20));
        assert!(!t.is_bed_top(22) && !t.is_bed_top(10));
        assert!(TileTypes::parse(&format!("bedtop: x\n{}\n", vec!["0"; 4096].join(","))).is_err());
        // a file with fewer arrays has no chairs
        let t = TileTypes::parse_arrays(&arrays_blob(&[&[0, 8]])).unwrap();
        assert!(!t.is_chair(8));
    }

    /// The tiles of new4.nw, with their lists in the 2.22 `arrays.dat`:
    /// 398 shallow (passable), 461 water (passable, liquid), 2882 lava
    /// (liquid, lava), 2958 lava floor (passable, lava), 3644 deep water (in
    /// no list).
    #[test]
    fn arrays_new4_tiles() {
        let data = arrays_blob(&[&[398, 461, 2958], &[461, 2882], &[], &[2882, 2958]]);
        let t = TileTypes::parse_arrays(&data).unwrap();
        assert_eq!(t.kind(398), TileKind::Open);
        assert_eq!(t.kind(461), TileKind::Water);
        assert_eq!(t.kind(2882), TileKind::Lava);
        assert_eq!((t.kind(2958), t.raw(2958)), (TileKind::Open, Some(TYPE_LAVA_FLOOR)));
        assert_eq!(t.kind(3644), TileKind::Blocking, "deep water is in no list: a wall");
        assert_eq!([398, 461, 2882, 2958, 3644].map(|id| t.is_lava(id)), [false, false, true, true, false]);
    }

    #[test]
    fn arrays_liquid_list_makes_water_and_lava() {
        // 5 is water and also passable; 6 is water and not passable;
        // 7 is lava (liquid list and lava list).
        let data = arrays_blob(&[&[0, 5], &[5, 6, 7], &[9], &[7]]);
        let t = TileTypes::parse_arrays(&data).unwrap();
        assert_eq!(t.kind(0), TileKind::Open);
        assert_eq!(t.kind(5), TileKind::Water);
        assert_eq!(t.kind(6), TileKind::Water, "water must not be a wall");
        assert_eq!(t.kind(7), TileKind::Lava);
        assert_eq!(t.kind(1), TileKind::Blocking);
        assert!(t.is_water(6) && !t.is_wall(6) && !t.is_wall(7));
    }

    /// Array 2 turns walls into the throw-through type and leaves passable
    /// tiles walkable.
    #[test]
    fn arrays_throw_through_list_marks_walls_type_20() {
        // passable: 0, 5; throw-through: 5 (passable) and 6, 7 (walls)
        let data = arrays_blob(&[&[0, 5], &[], &[5, 6, 7]]);
        let t = TileTypes::parse_arrays(&data).unwrap();
        assert_eq!([5, 6, 7, 8].map(|id| t.raw(id)), [Some(0), Some(20), Some(20), Some(TYPE_HARD_WALL)]);
        assert!(t.is_wall(6), "a throw-through tile still blocks walking");
    }

    #[test]
    fn arrays_tolerates_trailing_data_and_one_array() {
        let mut data = arrays_blob(&[&[0], &[4]]);
        // junk that looks like a huge count
        data.extend_from_slice(&0x0001_0004u32.to_be_bytes());
        data.extend_from_slice(&[1, 2, 3]);
        let t = TileTypes::parse_arrays(&data).unwrap();
        assert_eq!(t.kind(4), TileKind::Water);
        assert!(TileTypes::parse_arrays(&arrays_blob(&[&[0]])).is_ok());
        assert!(TileTypes::parse_arrays(&[0, 0, 1]).is_err());
        assert!(TileTypes::parse_arrays(&[0, 0, 0, 9, 0, 0]).is_err());
    }

    #[test]
    fn new_world_rule_follows_the_sheet_position() {
        let cases: [(u16, u8); 22] = [
            (0, 0),
            (32, 22),
            (64, 11),
            (200, 11), // the lava block is inside the water block
            (0x120, 2),
            (0x150, 3),
            (0x185, 4),
            (0x195, 5),
            (0x1a2, 22), // row 26, col 2
            (0x1a1, 6),  // row 26, col 1
            (0x1e8, 21), // row 30, col 8
            (0x1e7, 22),
            (0x200, 20),
            (0x2ff, 20),
            (0x300, 6),
            (0x320, 7),
            (0x340, 8),
            (0x390, 9),
            (0x3b0, 10),
            (0x3e0, 0),
            (0x800, 0),
            (0x900, 22),
        ];
        for (id, want) in cases {
            assert_eq!(new_world_tile_type(id), want, "tile {id:#x}");
        }
    }

    #[test]
    fn kind_follows_the_type_byte() {
        let mut t = TileTypes::from_types([0; 4096]);
        t.types[1] = TYPE_WATER;
        t.types[2] = TYPE_LAVA;
        t.types[3] = 19;
        t.types[4] = 20;
        t.types[5] = 22;
        t.types[6] = 3; // beds, chairs and the rest of 1..19 do not block
        assert_eq!(
            [0u16, 1, 2, 3, 4, 5, 6].map(|id| t.kind(id)),
            [
                TileKind::Open,
                TileKind::Water,
                TileKind::Lava,
                TileKind::Open,
                TileKind::Blocking,
                TileKind::Blocking,
                TileKind::Open
            ]
        );
        assert_eq!(t.kind(4096), TileKind::Blocking);
    }

    #[test]
    fn text_index_type_rows() {
        let text = "0,0,0,22,blocking\n1,1,0,0,non-blocking\n";
        // pad to 4096 so parse accepts
        let mut full = text.to_string();
        for i in 2..4096 {
            full.push_str(&format!("{i},0,0,0,x\n"));
        }
        let t = TileTypes::parse(&full).unwrap();
        assert!(t.is_wall(0));
        assert!(!t.is_wall(1));
    }
}


use crate::protocol::layouts::BoardModify2;
use crate::protocol::BoardModify;

/// A parsed level.
#[derive(Debug, Clone)]
pub struct Level {
    pub boards: Vec<BoardRun>,
    pub npcs: Vec<Npc>,
    pub links: Vec<Link>,
    pub signs: Vec<Sign>,
    pub baddies: Vec<Baddy>,
    pub chests: Vec<Chest>,
}

impl Level {
    /// Build a level from a 64×64 board of tile ids, row by row, as the
    /// server sends it in `PLO_BOARDPACKET`. Links, signs and chests
    /// arrive in their own packets and start empty here.
    pub fn from_board(tiles: &[u16]) -> Self {
        let boards = tiles
            .chunks(64)
            .take(64)
            .enumerate()
            .map(|(y, row)| BoardRun {
                x: 0,
                y: y as i32,
                width: row.len() as i32,
                layer: 0,
                tiles: row.iter().map(|&t| tile_code(t)).collect(),
            })
            .collect();
        Level {
            boards,
            npcs: Vec::new(),
            links: Vec::new(),
            signs: Vec::new(),
            baddies: Vec::new(),
            chests: Vec::new(),
        }
    }

    /// Add the server's `PLO_LEVELLINK` lines (`dest x y w h newx newy`).
    /// Lines that do not parse are skipped.
    pub fn add_server_links(&mut self, lines: &[String]) {
        self.links
            .extend(lines.iter().filter_map(|l| parse_link(&format!("LINK {l}"))));
    }

    /// Add the server's `PLO_LEVELCHEST` records: `opened, x, y` and, for a
    /// chest the player has not opened, `item, sign index`, every byte
    /// plus 32. An opened chest has no item on the wire, so its item stays
    /// empty. Records too short to hold a position are skipped.
    ///
    /// The client draws the chest from the record (a 2 x 2 blit of the
    /// closed or open chest tiles), see `render::draw_chests`.
    pub fn add_server_chests(&mut self, records: &[String]) {
        for r in records {
            let b = r.as_bytes();
            if b.len() < 3 {
                continue;
            }
            // The client reads any non-zero first byte as "opened".
            let opened = b[0].wrapping_sub(0x20) != 0;
            let (x, y) = (b[1].wrapping_sub(0x20) as i32, b[2].wrapping_sub(0x20) as i32);
            let (item, sign_index) = match b {
                [_, _, _, i, s, ..] if !opened => (
                    CHEST_ITEMS.get(i.wrapping_sub(0x20) as usize).map(|s| s.to_string()).unwrap_or_default(),
                    s.wrapping_sub(0x20) as i32,
                ),
                _ => (String::new(), -1),
            };
            self.chests.push(Chest { x, y, item, sign_index, opened });
        }
    }

    /// Add the server's `PLO_LEVELSIGN` records (`x, y, text`, see
    /// [`decode_sign_record`]).
    pub fn add_server_signs(&mut self, records: &[Vec<u8>]) {
        self.signs.extend(
            records
                .iter()
                .filter_map(|r| decode_sign_record(r))
                .map(|(x, y, text)| Sign { x, y, text }),
        );
    }

    /// Load a .nw or .graal file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let path = path.as_ref();
        let data = fs::read(path).map_err(|e| e.to_string())?;
        if data.len() >= 8 && &data[0..8] == b"GLEVNW01" {
            let text = String::from_utf8_lossy(&data);
            return Self::parse(&text);
        }
        if data.len() >= 8 && (data[0..8].starts_with(b"GR-V1") || data[0..8].starts_with(b"Z3-V1")) {
            return Self::parse_binary(&data);
        }
        Err(format!("unrecognised level header in {}", path.display()))
    }

    /// Parse a binary .graal (GR-V1) bitstream into a Level.
    fn parse_binary(data: &[u8]) -> Result<Self, String> {
        Self::parse_binary_with_layout(data).map(|(level, _)| level)
    }

    /// Parse a binary `.graal` / `.zelda` file and also report where each
    /// section sits (derived facts only; see [`GraalLayout`]).
    pub fn parse_binary_with_layout(data: &[u8]) -> Result<(Self, GraalLayout), String> {
        if data.len() < 8 {
            return Err("binary level too short".into());
        }
        // Z3-V1 (`.zelda`) has its own reader: no NPCs, no chests, 4-byte
        // computer records, strict reporting (see `zelda.rs`).
        if super::zelda::is_zelda(data) {
            let z = super::zelda::ZeldaLevel::parse(data)?;
            return Ok((z.to_level(), z.layout.to_graal_layout()));
        }
        let magic = &data[0..8];
        // Text levels and maps are not bitstreams: say so instead of
        // decoding their letters as board codes.
        if magic == b"GLEVNW01" || magic == b"GRMAP001" {
            return Err(format!(
                "{} is a text file, not a binary level (.graal / .zelda)",
                String::from_utf8_lossy(magic)
            ));
        }
        let codebits = graal_code_bits(magic);
        let mut tiles = vec![0u16; 4096];
        let mut boardindex = 0usize;
        let mut bitsread = 0u32;
        let mut buffer = 0u32;
        let mut count = 1u32;
        let mut firstcode = -1i32;
        let mut twicerepeat = false;
        let mut pos = 8usize;
        let mut layout = GraalLayout {
            magic: String::from_utf8_lossy(magic).into_owned(),
            code_bits: codebits,
            file_len: data.len(),
            ..GraalLayout::default()
        };

        while boardindex < 4096 && pos < data.len() {
            while bitsread < codebits && pos < data.len() {
                buffer |= (data[pos] as u32) << bitsread;
                bitsread += 8;
                pos += 1;
            }
            if bitsread < codebits {
                break;
            }
            let mask = (1u32 << codebits) - 1;
            let code = buffer & mask;
            buffer >>= codebits;
            bitsread -= codebits;
            layout.codes += 1;

            if code & (1 << (codebits - 1)) != 0 {
                twicerepeat = (code & 0x100) != 0;
                count = code & 0xFF;
                if twicerepeat {
                    layout.double_runs += 1;
                } else {
                    layout.single_runs += 1;
                }
                layout.max_run = layout.max_run.max(count);
            } else if count == 1 {
                tiles[boardindex] = code as u16;
                boardindex += 1;
            } else if twicerepeat {
                if firstcode < 0 {
                    firstcode = code as i32;
                } else {
                    for _ in 1..=count {
                        if boardindex >= 4095 {
                            break;
                        }
                        tiles[boardindex] = firstcode as u16;
                        tiles[boardindex + 1] = code as u16;
                        boardindex += 2;
                    }
                    firstcode = -1;
                    twicerepeat = false;
                    count = 1;
                }
            } else {
                for _ in 1..=count {
                    if boardindex >= 4096 {
                        break;
                    }
                    tiles[boardindex] = code as u16;
                    boardindex += 1;
                }
                count = 1;
            }
        }
        layout.tiles_decoded = boardindex;
        layout.board_end = pos;
        layout.max_tile = tiles.iter().copied().max().unwrap_or(0);

        let boards = board_runs(&tiles);

        // `pos` is the byte after the last code, so the sections start there.
        let mut level = Level {
            boards,
            npcs: Vec::new(),
            links: Vec::new(),
            signs: Vec::new(),
            baddies: Vec::new(),
            chests: Vec::new(),
        };
        let base = pos.min(data.len());
        parse_binary_sections(magic, &data[base..], &mut level, &mut layout, base);
        Ok((level, layout))
    }

    /// Parse .nw content.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut boards = Vec::new();
        let mut npcs = Vec::new();
        let mut links = Vec::new();
        let mut signs = Vec::new();
        let mut baddies = Vec::new();
        let mut chests = Vec::new();
        let mut lines = text.lines().peekable();

        // Header
        let header = lines.next().unwrap_or("").trim();
        if header != "GLEVNW01" {
            return Err(format!("unexpected header: {header}"));
        }

        while let Some(line) = lines.next() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }

            if line.starts_with("BOARD ") {
                if let Some(run) = parse_board(line) {
                    boards.push(run);
                }
            } else if line.starts_with("NPC ") {
                let mut npc_lines = vec![line.to_string()];
                while let Some(l) = lines.next() {
                    npc_lines.push(l.to_string());
                    if l.trim() == "NPCEND" {
                        break;
                    }
                }
                if let Some(npc) = parse_npc(&npc_lines) {
                    npcs.push(npc);
                }
            } else if line.starts_with("LINK ") {
                if let Some(link) = parse_link(line) {
                    links.push(link);
                }
            } else if line.starts_with("SIGN ") {
                let mut sign_lines = vec![line.to_string()];
                while let Some(l) = lines.next() {
                    sign_lines.push(l.to_string());
                    if l.trim() == "SIGNEND" {
                        break;
                    }
                }
                if let Some(sign) = parse_sign(&sign_lines) {
                    signs.push(sign);
                }
            } else if line.starts_with("BADDY ") {
                let mut baddy_lines = vec![line.to_string()];
                while let Some(l) = lines.next() {
                    baddy_lines.push(l.to_string());
                    if l.trim() == "BADDYEND" {
                        break;
                    }
                }
                if let Some(baddy) = parse_baddy(&baddy_lines) {
                    baddies.push(baddy);
                }
            } else if line.starts_with("CHEST ") {
                if let Some(chest) = parse_chest(line) {
                    chests.push(chest);
                }
            }
        }

        Ok(Level {
            boards,
            npcs,
            links,
            signs,
            baddies,
            chests,
        })
    }

    /// Place a stamp onto the board at (x, y).
    /// Each stamp row becomes a new layer-0 BOARD run.
    /// Collision is decided later by the type table.
    pub fn place_stamp(&mut self, stamp: &crate::assets::Stamp, x: i32, y: i32) {
        for row in 0..stamp.height {
            let mut tiles = Vec::with_capacity(stamp.width);
            for col in 0..stamp.width {
                if let Some(t) = stamp.tile_at(col, row) {
                    tiles.push(t);
                }
            }
            if !tiles.is_empty() {
                self.boards.push(BoardRun {
                    x,
                    y: y + row as i32,
                    width: tiles.len() as i32,
                    layer: 0,
                    tiles,
                });
            }
        }
    }

    /// Apply a board modify (PLO_BOARDMODIFY style) to layer 0.
    /// Creates or extends BOARD runs as needed.
    pub fn apply_board_modify(&mut self, m: &BoardModify) {
        // For simplicity we only handle layer 0 and rebuild affected rows.
        for row in 0..m.height as i32 {
            let y = m.y as i32 + row;
            let row_tiles: Vec<Tile> = (0..m.width as usize)
                .filter_map(|col| {
                    let idx = (row as usize * m.width as usize + col) * 2;
                    if idx + 1 < m.tiles.len() {
                        Some([m.tiles[idx], m.tiles[idx + 1]])
                    } else {
                        None
                    }
                })
                .collect();

            // Remove any existing layer-0 run that overlaps this row segment
            self.boards.retain(|b| {
                !(b.layer == 0 && b.y == y && b.x < m.x as i32 + m.width as i32 && b.x + b.width > m.x as i32)
            });

            // Insert a new run for the modified segment
            if !row_tiles.is_empty() {
                self.boards.push(BoardRun {
                    x: m.x as i32,
                    y,
                    width: row_tiles.len() as i32,
                    layer: 0,
                    tiles: row_tiles,
                });
            }
        }
    }

    /// Layer 0 as a 64×64 board of tile ids, row by row. A tile the level
    /// does not define is 0.
    pub fn layer0_board(&self) -> Vec<u16> {
        let mut board = vec![0u16; 64 * 64];
        for run in self.boards.iter().filter(|r| r.layer == 0) {
            if !(0..64).contains(&run.y) {
                continue;
            }
            for (i, t) in run.tiles.iter().enumerate() {
                let x = run.x + i as i32;
                if (0..64).contains(&x) {
                    board[run.y as usize * 64 + x as usize] = tile_id(*t);
                }
            }
        }
        board
    }

    /// Replace layer 0 with a 64×64 board of tile ids (the other layers,
    /// links, signs and chests stay).
    pub fn set_layer0_board(&mut self, board: &[u16]) {
        self.boards.retain(|r| r.layer != 0);
        let rows = board.chunks(64).take(64).enumerate().map(|(y, row)| BoardRun {
            x: 0,
            y: y as i32,
            width: row.len() as i32,
            layer: 0,
            tiles: row.iter().map(|&t| tile_code(t)).collect(),
        });
        let mut runs: Vec<BoardRun> = rows.collect();
        runs.append(&mut self.boards);
        self.boards = runs;
    }

    /// Apply a `PLO_BOARDMODIFY2` block to layer 0 with its clipping rules
    /// (see [`BoardModify2::apply_to`]). Returns the tiles written.
    pub fn apply_board_modify2(&mut self, m: &BoardModify2) -> usize {
        let mut board = self.layer0_board();
        let n = m.apply_to(&mut board);
        if n > 0 {
            self.set_layer0_board(&board);
        }
        n
    }

    /// Get the tile at (x, y) on layer 0, if present.
    pub fn tile_at(&self, x: i32, y: i32) -> Option<Tile> {
        for run in &self.boards {
            if run.layer == 0 && run.y == y && x >= run.x && x < run.x + run.width {
                let idx = (x - run.x) as usize;
                return run.tiles.get(idx).copied();
            }
        }
        None
    }

    /// Tile id at (x, y), if present.
    pub fn tile_id_at(&self, x: i32, y: i32) -> Option<u16> {
        self.tile_at(x, y).map(tile_id)
    }

    /// The kind of the tile under the point `(x, y)` (in tiles), or
    /// `Blocking` off the board. A point is in the tile it falls in:
    /// the original client truncates its test point to a tile (`int
    /// testx = ...`), so tile 63 runs from 63.0 up to 64.0. A tile the
    /// level does not define is open floor.
    pub fn kind_at(&self, types: &TileTypes, x: f32, y: f32) -> TileKind {
        if x < 0.0 || y < 0.0 || x >= 64.0 || y >= 64.0 {
            return TileKind::Blocking;
        }
        match self.tile_id_at(x as i32, y as i32) {
            Some(id) => types.kind(id),
            None => TileKind::Open,
        }
    }

    /// Put tile `id` at `(x, y)` on layer 0. False if the level has no tile
    /// there.
    pub fn set_tile_id(&mut self, x: i32, y: i32, id: u16) -> bool {
        for run in &mut self.boards {
            if run.layer == 0 && run.y == y && x >= run.x && x < run.x + run.width {
                let idx = (x - run.x) as usize;
                if let Some(t) = run.tiles.get_mut(idx) {
                    *t = tile_code(id);
                    return true;
                }
            }
        }
        false
    }

    /// Find the 2 x 2 object of one of the first `max_types` entries of
    /// `table` that has the tile under `(px, py)` as one of its four tiles
    /// and whose four tiles are all on the board there. Returns the entry
    /// and the top-left tile of the object.
    ///
    /// The client looks for the tile under the point in each entry (entry
    /// by entry, column by column, row by row), then checks the whole 2 x 2
    /// around it; the first full match wins.
    pub fn find_object(&self, table: &[Object2x2], max_types: usize, px: i32, py: i32) -> Option<(usize, i32, i32)> {
        if !(0..64).contains(&px) || !(0..64).contains(&py) {
            return None;
        }
        let here = self.tile_id_at(px, py)?;
        for (ty, obj) in table.iter().enumerate().take(max_types) {
            for c in 0..2 {
                for r in 0..2 {
                    if obj[c][r] != here {
                        continue;
                    }
                    let (ax, ay) = (px - c as i32, py - r as i32);
                    let all = (0..2).all(|i| {
                        (0..2).all(|j| {
                            let (x, y) = (ax + i as i32, ay + j as i32);
                            (0..64).contains(&x) && (0..64).contains(&y) && self.tile_id_at(x, y) == Some(obj[i][j])
                        })
                    });
                    if all {
                        return Some((ty, ax, ay));
                    }
                }
            }
        }
        None
    }

    /// Put a 2 x 2 object's tiles at top-left `(ax, ay)`.
    pub fn put_object(&mut self, ax: i32, ay: i32, tiles: &Object2x2) {
        for i in 0..2 {
            for j in 0..2 {
                self.set_tile_id(ax + i as i32, ay + j as i32, tiles[i][j]);
            }
        }
    }

    /// True if the point is a wall (or off the board).
    pub fn is_on_wall(&self, types: &TileTypes, x: f32, y: f32) -> bool {
        self.kind_at(types, x, y) == TileKind::Blocking
    }

    /// True if the point is in water.
    pub fn is_on_water(&self, types: &TileTypes, x: f32, y: f32) -> bool {
        self.kind_at(types, x, y) == TileKind::Water
    }
}

fn parse_board(line: &str) -> Option<BoardRun> {
    // BOARD x y width layer tiledata
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 6 || parts[0] != "BOARD" {
        return None;
    }
    let x: i32 = parts[1].parse().ok()?;
    let y: i32 = parts[2].parse().ok()?;
    let width: i32 = parts[3].parse().ok()?;
    let layer: i32 = parts[4].parse().ok()?;
    let data = parts[5];

    let mut tiles = Vec::new();
    let bytes = data.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        tiles.push([bytes[i], bytes[i + 1]]);
        i += 2;
    }

    Some(BoardRun {
        x,
        y,
        width,
        layer,
        tiles,
    })
}

fn parse_npc(lines: &[String]) -> Option<Npc> {
    // NPC image x y
    // script...
    // NPCEND
    if lines.is_empty() {
        return None;
    }
    let header: Vec<&str> = lines[0].split_whitespace().collect();
    if header.len() < 4 || header[0] != "NPC" {
        return None;
    }
    let image = header[1].to_string();
    let x: f32 = header[2].parse().ok()?;
    let y: f32 = header[3].parse().ok()?;

    // A block that is only its header line is malformed (no body, no
    // NPCEND): skip it. A closing NPCEND is dropped from the script; a
    // block cut off before NPCEND keeps all its lines.
    if lines.len() < 2 {
        return None;
    }
    let end = if lines[lines.len() - 1].trim() == "NPCEND" { lines.len() - 1 } else { lines.len() };
    let script = lines[1..end]
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    Some(Npc::new(image, x, y, script))
}

/// Names of the chest item indexes 0 to 24 (`LevelItemType`).
pub const CHEST_ITEMS: [&str; 25] = [
    "greenrupee", "bluerupee", "redrupee", "bombs", "darts", "heart", "glove1", "bow", "bomb", "shield", "sword",
    "fullheart", "superbomb", "battleaxe", "goldensword", "mirrorshield", "glove2", "lizardshield", "lizardsword",
    "goldrupee", "fireball", "fireblast", "nukeshot", "joltbomb", "spinattack",
];

/// A 64×64 board of tile ids as layer-0 row runs, one full row each. Tile 0
/// is a real tile (the top-left cell of `pics1.png`, grass on the usual
/// sheets), so it is kept: leaving it out made those places draw black.
pub(crate) fn board_runs(tiles: &[u16]) -> Vec<BoardRun> {
    tiles
        .chunks(64)
        .take(64)
        .enumerate()
        .map(|(y, row)| BoardRun { x: 0, y: y as i32, width: row.len() as i32, layer: 0, tiles: row.iter().map(|&t| tile_code(t)).collect() })
        .collect()
}

/// Bits per board code for a binary level magic: 13 for GR-V1.02 to 1.04
/// (the 4096-tile sheet needs the top bit free), 11 for Z3-V1.02, else 12.
pub fn graal_code_bits(magic: &[u8]) -> u32 {
    if matches!(magic, b"GR-V1.02" | b"GR-V1.03" | b"GR-V1.04") {
        13
    } else if magic == b"Z3-V1.02" {
        11
    } else {
        12
    }
}

/// Where the parts of a binary `.graal` / `.zelda` file sit and how many
/// records each held. Every field is a count or an offset, never content.
/// Ranges are byte offsets `(start, end)` into the file; an absent section
/// is an empty range.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraalLayout {
    pub magic: String,
    pub code_bits: u32,
    pub file_len: usize,
    /// Board codes read, split into single runs, double runs and the rest.
    pub codes: usize,
    pub single_runs: usize,
    pub double_runs: usize,
    /// Largest run count seen in a control code.
    pub max_run: u32,
    /// Tiles the board stream produced (4096 when complete).
    pub tiles_decoded: usize,
    pub max_tile: u16,
    /// Offset of the first byte after the board stream.
    pub board_end: usize,
    pub links: (usize, usize),
    pub baddies: (usize, usize),
    pub npcs: (usize, usize),
    pub chests: (usize, usize),
    pub signs: (usize, usize),
    pub link_lines: usize,
    pub baddy_records: usize,
    pub npc_records: usize,
    pub chest_records: usize,
    pub sign_records: usize,
    /// Bytes left after the sign section ended (a `HEIGHTS` block would be here).
    pub unread_tail: usize,
}

impl GraalLayout {
    /// A short derived summary: sizes, offsets and record counts, no level
    /// content. Backs `seihai --level-info`.
    pub fn summary(&self, level: &Level) -> String {
        let span = |r: (usize, usize)| format!("{}..{} ({} bytes)", r.0, r.1, r.1 - r.0);
        let floor = level.layer0_board();
        let distinct: std::collections::BTreeSet<u16> = floor.iter().copied().collect();
        let mut out = String::new();
        out.push_str(&format!("magic {}  code bits {}  file {} bytes\n", self.magic, self.code_bits, self.file_len));
        out.push_str(&format!(
            "board: {} tiles from {} codes ({} single runs, {} double runs, longest run {}), ends at byte {}\n",
            self.tiles_decoded, self.codes, self.single_runs, self.double_runs, self.max_run, self.board_end
        ));
        out.push_str(&format!("board: {} distinct tile ids, highest {}\n", distinct.len(), self.max_tile));
        out.push_str(&format!("links   {}: {} lines, {} read\n", span(self.links), self.link_lines, level.links.len()));
        out.push_str(&format!("baddies {}: {} records\n", span(self.baddies), self.baddy_records));
        out.push_str(&format!("npcs    {}: {} records\n", span(self.npcs), self.npc_records));
        out.push_str(&format!("chests  {}: {} records\n", span(self.chests), self.chest_records));
        out.push_str(&format!("signs   {}: {} records\n", span(self.signs), self.sign_records));
        out.push_str(&format!("unread tail: {} bytes\n", self.unread_tail));
        out
    }
}

/// Bytes to text, one char per byte (the files are Latin-1 era).
fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

/// Line reader over the bytes after a binary level's board.
struct Sections<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Sections<'a> {
    fn at_end(&self) -> bool {
        self.pos >= self.data.len()
    }

    /// The next line without its `\n` (and without a trailing `\r`).
    fn line(&mut self) -> Option<&'a [u8]> {
        if self.at_end() {
            return None;
        }
        let rest = &self.data[self.pos..];
        let (line, used) = match rest.iter().position(|&b| b == b'\n') {
            Some(i) => (&rest[..i], i + 1),
            None => (rest, rest.len()),
        };
        self.pos += used;
        Some(line.strip_suffix(b"\r").unwrap_or(line))
    }
}

/// Read the sections after the board of a GR-V1 or Z3-V1 file: links,
/// baddies, NPCs, chests, signs, in that order, and skip a GR-V1.04
/// `HEIGHTS` block. Layout from Graal_analysis `formats/level-binary.md`
/// and `level-binary-client.md` (2.05, 2.31). A section that is cut short
/// leaves what was read; nothing here fails the whole level.
fn parse_binary_sections(magic: &[u8], tail: &[u8], level: &mut Level, layout: &mut GraalLayout, base: usize) {
    let graal = magic.starts_with(b"GR-V1");
    // GR-V1.00 and later have NPCs; GR-V1.01 and later have chests; Z3-V1
    // has neither, and its baddies have a verse line from Z3-V1.04.
    let has_npcs = graal;
    let has_chests = graal && magic != b"GR-V1.00";
    let baddy_verse = graal || magic == b"Z3-V1.04";
    let mut s = Sections { data: tail, pos: 0 };

    // Links: lines up to "#" or an empty line.
    let start = s.pos;
    while let Some(l) = s.line() {
        if l.is_empty() || l == b"#" {
            break;
        }
        layout.link_lines += 1;
        if let Some(link) = parse_binary_link(&latin1(l)) {
            level.links.push(link);
        }
    }
    layout.links = (base + start, base + s.pos);

    // Baddies: 3 raw bytes x, y, type, then a verse line. The record ends at
    // the first `\n` after those 3 bytes, which may themselves hold 0x0A.
    // A first byte of 0x80 or more (FF FF FF) ends the section.
    let start = s.pos;
    loop {
        if s.pos + 3 > s.data.len() {
            s.pos = s.data.len();
            break;
        }
        let rec = &s.data[s.pos..];
        let end = rec[3..].iter().position(|&b| b == b'\n').map(|i| i + 3);
        let stop = rec[0] >= 0x80;
        let (verse, used) = match (baddy_verse, end) {
            (true, Some(e)) => (&rec[3..e], e + 1),
            (true, None) => (&rec[3..], rec.len()),
            (false, _) => (&rec[3..3], 3),
        };
        let (x, y, kind) = (rec[0], rec[1], rec[2]);
        s.pos += used;
        if stop {
            // Without a verse line the terminator's own line end is next.
            if !baddy_verse && s.data.get(s.pos) == Some(&b'\n') {
                s.pos += 1;
            }
            break;
        }
        layout.baddy_records += 1;
        level.baddies.push(Baddy {
            x: x as f32,
            y: y as f32,
            kind: kind.to_string(),
            verses: latin1(verse).replace('\\', "\n"),
        });
    }

    layout.baddies = (base + start, base + s.pos);

    // NPCs: `x+32, y+32, image#script`, the script's line breaks as 0xA7.
    let start = s.pos;
    if has_npcs {
        while let Some(l) = s.line() {
            if l.is_empty() || l == b"#" {
                break;
            }
            if l.len() < 2 {
                continue;
            }
            layout.npc_records += 1;
            let body = &l[2..];
            let (image, script) = match body.iter().position(|&b| b == b'#') {
                Some(i) => (&body[..i], &body[i + 1..]),
                None => (body, &body[..0]),
            };
            level.npcs.push(Npc::new(
                latin1(image),
                l[0] as f32 - 32.0,
                l[1] as f32 - 32.0,
                latin1(script).replace('\u{a7}', "\n"),
            ));
        }
    }

    layout.npcs = (base + start, base + s.pos);

    // Chests: four GCHARs, `x y item signindex`.
    let start = s.pos;
    if has_chests {
        while let Some(l) = s.line() {
            if l.is_empty() || l == b"#" {
                break;
            }
            if l.len() < 4 {
                continue;
            }
            layout.chest_records += 1;
            let item = l[2] as i32 - 32;
            level.chests.push(Chest {
                x: l[0] as i32 - 32,
                y: l[1] as i32 - 32,
                item: usize::try_from(item)
                    .ok()
                    .and_then(|i| CHEST_ITEMS.get(i))
                    .map_or_else(|| item.to_string(), |n| n.to_string()),
                sign_index: l[3] as i32 - 32,
                opened: false,
            });
        }
    }

    layout.chests = (base + start, base + s.pos);

    // Signs: to an empty line (2.31) or the end (2.05). The sign alphabet is
    // the one used on the wire.
    let start = s.pos;
    while let Some(l) = s.line() {
        if l.is_empty() {
            break;
        }
        layout.sign_records += 1;
        if let Some((x, y, text)) = decode_sign_record(l) {
            level.signs.push(Sign { x, y, text });
        }
    }
    layout.signs = (base + start, base + s.pos);
    layout.unread_tail = s.data.len() - s.pos;
    // A GR-V1.04 `HEIGHTS` block may follow; it is read past and not kept.
}

/// A binary level's link line, `dest x y w h newx newy`. Spaces in the
/// level name are allowed: every token before the last six is the name.
fn parse_binary_link(line: &str) -> Option<Link> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 7 {
        return None;
    }
    let n = parts.len();
    let nx = link_coord(parts[n - 2], "playerx");
    let ny = link_coord(parts[n - 1], "playery");
    Some(Link {
        dest: parts[..n - 6].join(" "),
        x: parts[n - 6].parse().ok()?,
        y: parts[n - 5].parse().ok()?,
        width: parts[n - 4].parse().ok()?,
        height: parts[n - 3].parse().ok()?,
        new_x: nx.0,
        new_y: ny.0,
        x_offset: nx.1,
        y_offset: ny.1,
    })
}

fn parse_link(line: &str) -> Option<Link> {
    // LINK dest x y width height newx newy
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 8 || parts[0] != "LINK" {
        return None;
    }
    let nx = link_coord(parts[6], "playerx");
    let ny = link_coord(parts[7], "playery");
    Some(Link {
        dest: parts[1].to_string(),
        x: parts[2].parse().ok()?,
        y: parts[3].parse().ok()?,
        width: parts[4].parse().ok()?,
        height: parts[5].parse().ok()?,
        new_x: nx.0,
        new_y: ny.0,
        x_offset: nx.1,
        y_offset: ny.1,
    })
}

/// A LINK arrival coordinate as `(absolute, offset)`:
/// * a number gives `(Some(n), 0)`;
/// * `playerx` / `playery` (any case) gives `(None, 0)`;
/// * the same followed by a signed number (`playery+10`, `playerx-1.5`)
///   gives `(None, that number)`;
/// * anything else (a stray word in an old level) is read as "keep the
///   player's coordinate" and the link is kept rather than dropped.
fn link_coord(s: &str, keep: &str) -> (Option<f32>, f32) {
    if let Ok(n) = s.parse::<f32>() {
        // A negative arrival (the editors write -1) keeps the player's own
        // coordinate: 1st Graal and the Java Zelda Online client both set a
        // "constant" flag from `< 0` (805 of 5381 distinct `GR-V1` links in
        // the Drive set use -1, and so do 151 of 338 `Z3-V1` links).
        return (if n < 0.0 { None } else { Some(n) }, 0.0);
    }
    if s.len() >= keep.len() && s.is_char_boundary(keep.len()) && s[..keep.len()].eq_ignore_ascii_case(keep) {
        let rest = &s[keep.len()..];
        if rest.starts_with(['+', '-']) {
            if let Ok(off) = rest.parse::<f32>() {
                return (None, off);
            }
        }
    }
    (None, 0.0)
}

fn parse_sign(lines: &[String]) -> Option<Sign> {
    // SIGN x y
    // text...
    // SIGNEND
    // GLEVNW01 text is already plain. Binary records use decode_sign_record.
    if lines.is_empty() {
        return None;
    }
    let header: Vec<&str> = lines[0].split_whitespace().collect();
    if header.len() < 3 || header[0] != "SIGN" {
        return None;
    }
    let x: i32 = header[1].parse().ok()?;
    let y: i32 = header[2].parse().ok()?;
    let text = lines[1..lines.len().saturating_sub(1)]
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Some(Sign { x, y, text })
}

fn parse_baddy(lines: &[String]) -> Option<Baddy> {
    // BADDY x y type
    // verses...
    // BADDYEND
    if lines.is_empty() {
        return None;
    }
    let header: Vec<&str> = lines[0].split_whitespace().collect();
    if header.len() < 4 || header[0] != "BADDY" {
        return None;
    }
    let x: f32 = header[1].parse().ok()?;
    let y: f32 = header[2].parse().ok()?;
    let kind = header[3].to_string();
    let verses = lines[1..lines.len().saturating_sub(1)]
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Some(Baddy { x, y, kind, verses })
}

fn parse_chest(line: &str) -> Option<Chest> {
    // CHEST x y item signindex
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 5 || parts[0] != "CHEST" {
        return None;
    }
    Some(Chest {
        x: parts[1].parse().ok()?,
        y: parts[2].parse().ok()?,
        item: parts[3].to_string(),
        sign_index: parts[4].parse().ok()?,
        opened: false,
    })
}

/// Binary sign alphabet: each byte is `index + 32` into this table. `#`
/// slots are glyph and escape codes (`formats/sign-text.md`). The same table
/// serves `.graal` sign sections and `PLO_LEVELSIGN`.
const SIGN_ALPHABET: &[u8; 97] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!?-.,#>()#####\"####':/~&### <####;\n";

/// Glyph codes: one or two alphabet codes for `#` plus a symbol. Two-code
/// glyphs come first so they win.
const SIGN_GLYPHS: [(&[u8], &str); 14] = [
    (&[74, 75], "#h"),
    (&[87, 88], "#4"),
    (&[91], "#A"),
    (&[92], "#B"),
    (&[93], "#X"),
    (&[94], "#Y"),
    (&[77], "#u"),
    (&[78], "#d"),
    (&[79], "#l"),
    (&[80], "#r"),
    (&[71], "#x"),
    (&[72], "#y"),
    (&[73], "#z"),
    (&[67], "#."),
];

/// Decode sign text (the bytes after x and y) with the packed alphabet.
/// Glyph codes become `#A`-style tags; `#K(n)` escapes become the character
/// `n`, except `#K(13)` (the CR of the editor's CRLF), which is dropped.
/// Bytes outside the table are controls and are not shown. Not for NPC
/// scripts (plain Latin-1) or GLEVNW01 signs (plain text).
pub fn decode_sign_text(payload: &[u8]) -> String {
    let codes: Vec<i32> = payload.iter().map(|&b| b as i32 - 32).collect();
    let mut raw = String::new();
    let mut i = 0;
    while i < codes.len() {
        let c = codes[i];
        if c < 0 || c as usize >= SIGN_ALPHABET.len() {
            i += 1;
            continue;
        }
        let glyph = SIGN_GLYPHS
            .iter()
            .find(|(seq, _)| seq.iter().enumerate().all(|(k, &v)| codes.get(i + k) == Some(&(v as i32))));
        if let Some((seq, tag)) = glyph {
            raw.push_str(tag);
            i += seq.len();
            continue;
        }
        raw.push(SIGN_ALPHABET[c as usize] as char);
        i += 1;
    }
    expand_sign_escapes(&raw)
}

/// Turn `#K(n)` (1 to 3 digits) into the character `n`; `#K(13)` vanishes.
fn expand_sign_escapes(raw: &str) -> String {
    let b = raw.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"#K(") {
            let digits = b[i + 3..].iter().take_while(|d| d.is_ascii_digit()).count();
            if (1..=3).contains(&digits) && b.get(i + 3 + digits) == Some(&b')') {
                let n: u32 = raw[i + 3..i + 3 + digits].parse().unwrap_or(0);
                if n < 256 {
                    if n != 13 {
                        out.push(char::from_u32(n).unwrap_or('?'));
                    }
                    i += 4 + digits;
                    continue;
                }
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

/// Full binary sign record: byte0 = x+0x20, byte1 = y+0x20, rest = alphabet.
pub fn decode_sign_record(line: &[u8]) -> Option<(i32, i32, String)> {
    if line.len() < 2 {
        return None;
    }
    let x = line[0] as i32 - 0x20;
    let y = line[1] as i32 - 0x20;
    let text = decode_sign_text(&line[2..]);
    Some((x, y, text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::crc32;

    #[test]
    fn server_chest_and_sign_records_reach_the_level() {
        let mut board = Level::from_board(&[0; 4096]);
        // Opened chest at (23, 19): three bytes. Closed chest at (5, 6):
        // item 0 (green rupee), sign 2.
        let opened: String = [0x21u8, 0x20 + 23, 0x20 + 19].iter().map(|&b| b as char).collect();
        let closed: String = [0x20u8, 0x20 + 5, 0x20 + 6, 0x20, 0x22].iter().map(|&b| b as char).collect();
        board.add_server_chests(&[opened, closed, "!".into()]);
        assert_eq!(board.chests.len(), 2);
        assert_eq!((board.chests[0].x, board.chests[0].y, board.chests[0].item.as_str()), (23, 19, ""));
        assert!(board.chests[0].opened && !board.chests[1].opened);
        assert_eq!((board.chests[1].x, board.chests[1].y, board.chests[1].item.as_str(), board.chests[1].sign_index), (5, 6, "greenrupee", 2));
        // A sign: x 7, y 8, text "AB" (alphabet codes 0, 1).
        let sign = vec![0x20u8 + 7, 0x20 + 8, 0x20, 0x21];
        // The beta4 lab gserver's record for `SIGN 30 22` with the text
        // "Hello" (2026-10-30): the last byte is 0x80.
        let beta4 = vec![0x3eu8, 0x36, 0x27, 0x3e, 0x45, 0x45, 0x48, 0x80];
        board.add_server_signs(&[sign, beta4]);
        assert_eq!(board.signs.len(), 2);
        assert_eq!((board.signs[0].x, board.signs[0].y, board.signs[0].text.as_str()), (7, 8, "AB"));
        assert_eq!((board.signs[1].x, board.signs[1].y), (30, 22));
        assert!(board.signs[1].text.starts_with("Hello"), "{:?}", board.signs[1].text);
    }

    #[test]
    fn link_keeps_player_coordinate() {
        let lvl = Level::parse("GLEVNW01\nLINK new1.nw 0 10 1 4 62 playery\n").unwrap();
        assert_eq!(lvl.links.len(), 1);
        assert_eq!((lvl.links[0].new_x, lvl.links[0].new_y), (Some(62.0), None));
        // The server's PLO_LEVELLINK form has no LINK keyword.
        let mut board = Level::from_board(&[0; 4096]);
        board.add_server_links(&["new1.nw 33 28 2 6 20 12.5".to_string(), "bad".to_string()]);
        assert_eq!(board.links.len(), 1);
        assert_eq!((board.links[0].x, board.links[0].new_y), (33, Some(12.5)));
    }

    #[test]
    fn board_ids_round_trip() {
        let tiles: Vec<u16> = (0..4096).collect();
        let lvl = Level::from_board(&tiles);
        assert_eq!(tile_code(3 * 64 + 29), *b"Dd");
        for (x, y) in [(0, 0), (63, 0), (5, 9), (63, 63)] {
            assert_eq!(lvl.tile_id_at(x, y), Some((y * 64 + x) as u16));
        }
    }

    #[test]
    fn parse_minimal() {
        let src = r#"GLEVNW01
BOARD 0 0 4 0 IdIdIdAb
NPC - 10 20
if (created) { }
NPCEND
LINK town 0 63 64 1 30 1
SIGN 5 5
Hello
SIGNEND
BADDY 10 12 0
grrrr
BADDYEND
CHEST 8 8 greenrupee 0
"#;
        let lvl = Level::parse(src).unwrap();
        assert_eq!(lvl.boards.len(), 1);
        assert_eq!(lvl.boards[0].tiles.len(), 4);
        assert_eq!(lvl.npcs.len(), 1);
        assert_eq!(lvl.npcs[0].x, 10.0);
        assert_eq!(lvl.links.len(), 1);
        assert_eq!(lvl.links[0].dest, "town");
        assert_eq!((lvl.links[0].new_x, lvl.links[0].new_y), (Some(30.0), Some(1.0)));
        assert_eq!(lvl.signs.len(), 1);
        assert_eq!(lvl.signs[0].text, "Hello");
        assert_eq!(lvl.baddies.len(), 1);
        assert_eq!(lvl.baddies[0].kind, "0");
        assert_eq!(lvl.chests.len(), 1);
        assert_eq!(lvl.chests[0].item, "greenrupee");
    }

    #[test]
    fn header_only_npc_block_is_skipped_not_a_panic() {
        // Synthetic: an NPC header at the end of the file, no body, no NPCEND.
        let text = "GLEVNW01\nNPC image.png 10 12\n";
        let lvl = Level::parse(text).unwrap();
        assert!(lvl.npcs.is_empty());
        // The same in the middle: the block swallows lines up to the next NPCEND
        // or the end of the file, and parsing still finishes.
        let text = "GLEVNW01\nNPC image.png 10 12";
        assert!(Level::parse(text).unwrap().npcs.is_empty());
    }

    #[test]
    fn npc_block_shapes() {
        // Header and NPCEND only: an NPC with an empty script.
        let lvl = Level::parse("GLEVNW01\nNPC a.png 1 2\nNPCEND\n").unwrap();
        assert_eq!(lvl.npcs.len(), 1);
        assert_eq!(lvl.npcs[0].script, "");
        // Normal block.
        let lvl = Level::parse("GLEVNW01\nNPC a.png 1 2\nif (created) {\n}\nNPCEND\n").unwrap();
        assert_eq!(lvl.npcs[0].script, "if (created) {\n}");
        // Cut off before NPCEND: the lines read so far are kept.
        let lvl = Level::parse("GLEVNW01\nNPC a.png 1 2\nsetimg x;").unwrap();
        assert_eq!(lvl.npcs[0].script, "setimg x;");
        // Bad header numbers: skipped.
        assert!(Level::parse("GLEVNW01\nNPC a.png x y\nNPCEND\n").unwrap().npcs.is_empty());
    }

    #[test]
    fn binary_reader_refuses_text_levels_and_maps() {
        for magic in ["GLEVNW01", "GRMAP001"] {
            let data = format!("{magic}\nWIDTH 2\nHEIGHT 2\n");
            let e = Level::parse_binary_with_layout(data.as_bytes()).unwrap_err();
            assert!(e.contains("text file"), "{e}");
        }
    }

    #[test]
    fn binary_boards_keep_tile_0() {
        // GR-V1.01: 12-bit codes, low bits first. Rows alternate 0 and 5,
        // so every row starts with tile 0 and has 0s after other tiles.
        let mut data = b"GR-V1.01".to_vec();
        let (mut buf, mut bits) = (0u32, 0u32);
        for i in 0..4096u32 {
            buf |= (if i % 2 == 0 { 0 } else { 5 }) << bits;
            bits += 12;
            while bits >= 8 {
                data.push(buf as u8);
                buf >>= 8;
                bits -= 8;
            }
        }
        data.push(buf as u8);
        let lvl = Level::parse_binary(&data).unwrap();
        for (x, y) in [(0, 0), (2, 0), (0, 63), (62, 63)] {
            assert_eq!(lvl.tile_id_at(x, y), Some(0), "({x}, {y})");
        }
        assert_eq!(lvl.tile_id_at(63, 63), Some(5));
    }

    #[test]
    fn tile_id_roundtrip() {
        assert_eq!(tile_id([b'A', b'A']), 0);
        assert_eq!(tile_id([b'/', b'/']), 4095);
    }

    #[test]
    fn tiletypes_wall_water() {
        let mut text = String::from("# comment\n");
        for i in 0..4096 {
            let v = if i == 10 { 11 } else if i >= 100 { 20 } else { 0 };
            text.push_str(&format!("{v},"));
            if (i + 1) % 128 == 0 {
                text.push('\n');
            }
        }
        let tt = TileTypes::parse(&text).unwrap();
        assert!(tt.is_water(10));
        assert!(tt.is_wall(100));
        assert!(!tt.is_wall(0));
    }

    #[test]
    fn layer0_board_round_trips_and_keeps_other_layers() {
        let mut board = vec![0u16; 64 * 64];
        board[0] = 7;
        board[64 * 63 + 63] = 4095;
        let mut level = Level::from_board(&board);
        level.boards.push(BoardRun { x: 0, y: 0, width: 1, layer: 1, tiles: vec![tile_code(9)] });
        assert_eq!(level.layer0_board(), board);
        board[5] = 12;
        level.set_layer0_board(&board);
        assert_eq!(level.layer0_board(), board);
        assert_eq!(level.boards.iter().filter(|r| r.layer == 1).count(), 1);
        assert_eq!(level.tile_id_at(5, 0), Some(12));
    }

    #[test]
    fn board_modify2_clips_at_column_63_on_a_level() {
        let mut level = Level::from_board(&vec![1u16; 64 * 64]);
        let m = BoardModify2 { map_x: 0, map_y: 0, x: 62, y: 3, width: 4, height: 2, tiles: vec![9; 8], trailer: vec![] };
        assert_eq!(level.apply_board_modify2(&m), 4);
        assert_eq!(level.tile_id_at(61, 3), Some(1));
        assert_eq!(level.tile_id_at(62, 3), Some(9));
        assert_eq!(level.tile_id_at(63, 4), Some(9));
        assert_eq!(level.tile_id_at(0, 4), Some(1), "no wrap to the next row");
        assert_eq!(level.tile_id_at(62, 5), Some(1));
    }
    /// A GR-V1 board writer after the 2.05 rules (`level-binary-client.md`):
    /// a single run for equal neighbours (up to 64), a double run for an
    /// `A B A B` pattern, plain tiles otherwise; codes packed LSB first.
    fn encode_board(tiles: &[u16], bits: u32) -> Vec<u8> {
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
                codes.push(tiles[i] as u32);
                i += n;
            } else if i + 3 < tiles.len() && tiles[i] == tiles[i + 2] && tiles[i + 1] == tiles[i + 3] {
                let mut n = 1;
                while i + 2 * n + 1 < tiles.len() && tiles[i + 2 * n] == tiles[i] && tiles[i + 2 * n + 1] == tiles[i + 1] && n < 64 {
                    n += 1;
                }
                codes.push(flag | 0x100 | n as u32);
                codes.push(tiles[i] as u32);
                codes.push(tiles[i + 1] as u32);
                i += 2 * n;
            } else {
                codes.push(tiles[i] as u32);
                i += 1;
            }
        }
        let (mut out, mut buf, mut have) = (Vec::new(), 0u64, 0u32);
        for c in codes {
            buf |= (c as u64) << have;
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

    fn sample_board() -> Vec<u16> {
        let mut t = vec![0u16; 4096];
        for (i, v) in t.iter_mut().enumerate() {
            *v = match i % 64 {
                0..=9 => 2,
                10..=19 => if i % 2 == 0 { 7 } else { 9 },
                20..=21 => 1500,
                _ => (i % 5) as u16 * 3,
            };
        }
        t[63] = 4095;
        t[4095] = 2047;
        t
    }

    fn binary_level(magic: &[u8; 8], bits: u32, tail: &[u8]) -> Vec<u8> {
        let mut d = magic.to_vec();
        d.extend(encode_board(&sample_board(), bits));
        d.extend_from_slice(tail);
        d
    }

    fn board_of(l: &Level) -> Vec<u16> {
        l.layer0_board()
    }

    #[test]
    fn graal_boards_decode_at_every_code_width() {
        let want = sample_board();
        for (magic, bits) in [
            (b"GR-V1.00", 12u32),
            (b"GR-V1.01", 12),
            (b"GR-V1.02", 13),
            (b"GR-V1.03", 13),
            (b"GR-V1.04", 13),
        ] {
            let mut b = want.clone();
            if bits == 12 {
                // 12-bit files cannot hold tiles of 2048 or more.
                for t in b.iter_mut() {
                    *t &= 0x7FF;
                }
            }
            let mut d = magic.to_vec();
            d.extend(encode_board(&b, bits));
            d.extend_from_slice(b"\n");
            let l = Level::parse_binary(&d).unwrap();
            assert_eq!(board_of(&l), b, "{}", String::from_utf8_lossy(magic));
        }
    }

    #[test]
    fn graal_sections_are_read_in_order() {
        let mut tail: Vec<u8> = Vec::new();
        // Links, with a space in a level name.
        tail.extend_from_slice(b"house 1.graal 3 4 2 1 10 11\nnew1.nw 0 10 1 4 62 playery\n#\n");
        // Baddies: x, y, type, verses; a 0x0A inside the 3 raw bytes; then FF FF FF.
        tail.extend_from_slice(&[5, 6, 2]);
        tail.extend_from_slice(b"saw you\\ouch\\grr\n");
        tail.extend_from_slice(&[10, 7, 0]);
        tail.extend_from_slice(b"a\\b\\c\n");
        tail.extend_from_slice(&[0xFF, 0xFF, 0xFF, b'\n']);
        // NPCs: x+32, y+32, image#script with 0xA7 for line breaks.
        tail.extend_from_slice(&[32 + 30, 32 + 28]);
        tail.extend_from_slice(b"door.png#if (playerenters) {\xA7  say hi;\xA7}\n");
        tail.extend_from_slice(&[32 + 1, 32 + 2]);
        tail.extend_from_slice(b"#message x;\n");
        tail.extend_from_slice(b"#\n");
        // Chests: x, y, item, sign index, each +32.
        tail.extend_from_slice(&[32 + 7, 32 + 8, 32 + 1, 32 + 0, b'\n']);
        tail.extend_from_slice(&[32 + 9, 32 + 9, 32 + 99, 32 + 3, b'\n']);
        tail.extend_from_slice(b"#\n");
        // A sign: x+32, y+32, then text in the sign alphabet (each byte is its alphabet index + 32, so `a` is 58).
        tail.extend_from_slice(&[32 + 12, 32 + 13, 58, b'\n']);
        let d = binary_level(b"GR-V1.03", 13, &tail);
        let l = Level::parse_binary(&d).unwrap();
        assert_eq!(l.links.len(), 2);
        assert_eq!((l.links[0].dest.as_str(), l.links[0].x, l.links[0].y, l.links[0].width, l.links[0].height), ("house 1.graal", 3, 4, 2, 1));
        assert_eq!((l.links[0].new_x, l.links[0].new_y), (Some(10.0), Some(11.0)));
        assert_eq!((l.links[1].new_x, l.links[1].new_y), (Some(62.0), None));
        assert_eq!(l.baddies.len(), 2);
        assert_eq!((l.baddies[0].x, l.baddies[0].y, l.baddies[0].kind.as_str()), (5.0, 6.0, "2"));
        assert_eq!(l.baddies[0].verses, "saw you\nouch\ngrr");
        assert_eq!((l.baddies[1].x, l.baddies[1].y), (10.0, 7.0));
        assert_eq!(l.npcs.len(), 2);
        assert_eq!((l.npcs[0].image.as_str(), l.npcs[0].x, l.npcs[0].y), ("door.png", 30.0, 28.0));
        assert_eq!(l.npcs[0].script, "if (playerenters) {\n  say hi;\n}");
        assert_eq!(l.npcs[1].image, "");
        assert_eq!((l.npcs[1].x, l.npcs[1].y), (1.0, 2.0));
        assert_eq!(l.chests.len(), 2);
        assert_eq!((l.chests[0].x, l.chests[0].y, l.chests[0].item.as_str(), l.chests[0].sign_index), (7, 8, "bluerupee", 0));
        // An item index with no name stays a number.
        assert_eq!(l.chests[1].item, "99");
        assert_eq!(l.signs.len(), 1);
        assert_eq!((l.signs[0].x, l.signs[0].y, l.signs[0].text.as_str()), (12, 13, "a"));
        // The board is untouched by the tail.
        assert_eq!(board_of(&l), sample_board());
    }

    #[test]
    fn graal_00_has_no_chest_section_and_heights_are_skipped() {
        // GR-V1.00: links, baddies, NPCs, then the signs directly.
        let mut tail: Vec<u8> = Vec::new();
        tail.extend_from_slice(b"#\n");
        tail.extend_from_slice(&[0xFF, 0xFF, 0xFF, b'\n']);
        tail.extend_from_slice(b"#\n");
        tail.extend_from_slice(&[32 + 1, 32 + 1, 58, b'\n']);
        let mut b = sample_board();
        for t in b.iter_mut() {
            *t &= 0x7FF;
        }
        let mut d = b"GR-V1.00".to_vec();
        d.extend(encode_board(&b, 12));
        d.extend_from_slice(&tail);
        let l = Level::parse_binary(&d).unwrap();
        assert!(l.chests.is_empty());
        assert_eq!(l.signs.len(), 1);

        // GR-V1.04: a HEIGHTS block after the signs is not read as a sign.
        let mut tail: Vec<u8> = Vec::new();
        tail.extend_from_slice(b"#\n");
        tail.extend_from_slice(&[0xFF, 0xFF, 0xFF, b'\n']);
        tail.extend_from_slice(b"#\n#\n");
        tail.extend_from_slice(&[32 + 2, 32 + 2, 58, b'\n']);
        tail.extend_from_slice(b"\nHEIGHTS\n0,0,0,0,0,0,0,0,0\nHEIGHTSEND\n");
        let d = binary_level(b"GR-V1.04", 13, &tail);
        let l = Level::parse_binary(&d).unwrap();
        assert_eq!(l.signs.len(), 1);
    }

    #[test]
    fn graal_tail_that_is_cut_short_keeps_the_board() {
        for cut in [0usize, 1, 5, 30] {
            let tail = b"dest.nw 1 2 3 4 5 6\n#\n\x05\x06\x02verse\n".to_vec();
            let d = binary_level(b"GR-V1.03", 13, &tail[..cut.min(tail.len())]);
            let l = Level::parse_binary(&d).unwrap();
            assert_eq!(board_of(&l), sample_board(), "cut {cut}");
        }
        // No tail at all.
        let l = Level::parse_binary(&binary_level(b"GR-V1.03", 13, b"")).unwrap();
        assert!(l.links.is_empty() && l.npcs.is_empty() && l.signs.is_empty());
    }

    #[test]
    fn load_reads_a_graal_file_from_disk() {
        let dir = std::env::temp_dir().join(format!("seihai-graal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.graal");
        std::fs::write(&p, binary_level(b"GR-V1.03", 13, b"a.graal 0 0 1 1 5 5\n#\n")).unwrap();
        let l = Level::load(&p).unwrap();
        assert_eq!(l.links.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }


    fn unhex(h: &str) -> Vec<u8> {
        (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
    }

    /// Sign records from real GR-V1.03 levels (Graal_analysis
    /// `formats/sign-decoder/vectors.json`): the packed alphabet, glyph
    /// codes and the `#K(13)` + newline line break.
    #[test]
    fn sign_text_uses_the_packed_alphabet() {
        let hex = "314848464c6f762a6555576680273e3a4542474079314848466f6d762a655557668032443a4d3e79493a4b446d762a655557668032493a4b706d762a6555576680213a4b793a473d794c4e494545524c70";
        let t = decode_sign_text(&unhex(hex));
        assert_eq!(t, "Rooms#l\nHealing Room#l#u\nSkate park#u\nSpar#r#u\nBar and supllys#r");
        let t = decode_sign_text(&unhex("363e453c48463e79334879363a4d3e4b36484b453d61"));
        assert_eq!(t, "Welcome To WaterWorld.");
        // Punctuation and the apostrophe sit in the table's upper half.
        let t = decode_sign_text(&unhex("413e454548793a473d79503e453c48463e794d4879762a6555576680233e4b3a47403e3d714c7941484e4c3e"));
        assert_eq!(t, "hello and welcome to \nDeranged's house");
        // A `#K(n)` escape is the character n; bytes below 32 are not shown.
        let p = [32, 5, 32 + 86, 32 + 10, 32 + 69, 32 + 57, 32 + 56, 32 + 70];
        assert_eq!(decode_sign_text(&p), "A6");
        // A sign record is x+32, y+32, then the text.
        let mut rec = vec![32 + 12, 32 + 13];
        rec.extend(unhex("3641424c4d453e7932414849"));
        assert_eq!(decode_sign_record(&rec), Some((12, 13, "Whistle Shop".to_string())));
    }

    /// A GR-V1.03 file shaped like the two real ones: one link with a
    /// half-tile arrival, an empty baddy section, NPCs (one with no image),
    /// a chest with sign index 0x1F ("none"), and no sign section at all.
    #[test]
    fn graal_03_without_signs_ends_at_the_chest_terminator() {
        let mut tail: Vec<u8> = Vec::new();
        tail.extend_from_slice(b"house2.graal 31 26 2 1 24.5 35\n#\n");
        tail.extend_from_slice(&[0xFF, 0xFF, 0xFF, b'\n']);
        tail.extend_from_slice(&[32 + 34, 32 + 25]);
        tail.extend_from_slice(b"lamp.gif#if (playerenters) {\xa7  show;\xa7}\n");
        tail.extend_from_slice(&[32 + 13, 32 + 33]);
        tail.extend_from_slice(b"#// no image\xa7if (created) {\xa7}\n#\n");
        tail.extend_from_slice(&[32 + 54, 32 + 9, 32 + 2, 31, b'\n']);
        tail.extend_from_slice(b"#\n");
        let d = binary_level(b"GR-V1.03", 13, &tail);
        let (l, lay) = Level::parse_binary_with_layout(&d).unwrap();
        assert_eq!((lay.magic.as_str(), lay.code_bits, lay.tiles_decoded, lay.max_tile), ("GR-V1.03", 13, 4096, 4095));
        assert_eq!(lay.file_len, d.len());
        let b = lay.board_end;
        assert_eq!(lay.links, (b, b + 33));
        assert_eq!(lay.baddies, (b + 33, b + 37));
        assert_eq!((lay.link_lines, lay.baddy_records, lay.npc_records, lay.chest_records, lay.sign_records), (1, 0, 2, 1, 0));
        assert_eq!(lay.chests.1, d.len());
        assert_eq!((lay.signs, lay.unread_tail), ((d.len(), d.len()), 0));
        assert_eq!((l.links[0].new_x, l.links[0].new_y), (Some(24.5), Some(35.0)));
        assert_eq!(l.npcs.len(), 2);
        assert_eq!((l.npcs[0].image.as_str(), l.npcs[0].x, l.npcs[0].y), ("lamp.gif", 34.0, 25.0));
        assert_eq!(l.npcs[0].script, "if (playerenters) {\n  show;\n}");
        assert_eq!(l.npcs[1].image, "");
        assert_eq!((l.chests[0].x, l.chests[0].y, l.chests[0].item.as_str(), l.chests[0].sign_index), (54, 9, "redrupee", -1));
        assert!(l.signs.is_empty());
        assert!(lay.summary(&l).contains("npcs"));
    }

    /// Derived facts of real `.graal` levels, keyed by the CRC-32 of the
    /// file. Set `SEIHAI_GRAAL_LEVELS` to a folder of `.graal` files: every
    /// file must parse and tile up (the generic checks); the two levels the
    /// project owner supplied are also checked against counts, offsets and a
    /// few tile ids. No level content is stored in the repository.
    #[test]
    #[ignore = "needs real .graal files; set SEIHAI_GRAAL_LEVELS to a folder of them"]
    fn real_graal_levels_match_the_format_notes() {
        let Some(dir) = std::env::var_os("SEIHAI_GRAAL_LEVELS") else {
            panic!("set SEIHAI_GRAAL_LEVELS to a folder of .graal files");
        };
        let mut seen = 0;
        for e in std::fs::read_dir(&dir).unwrap() {
            let path = e.unwrap().path();
            if path.extension().and_then(|x| x.to_str()) != Some("graal") {
                continue;
            }
            let data = std::fs::read(&path).unwrap();
            let (l, lay) = Level::parse_binary_with_layout(&data).unwrap();
            seen += 1;
            // Generic: the board is complete, the sections tile the rest of
            // the file in order, and nothing is left over.
            assert_eq!(lay.tiles_decoded, 4096, "{path:?}");
            assert!(lay.max_tile < 4096);
            assert_eq!(lay.board_end, lay.links.0);
            assert_eq!(lay.links.1, lay.baddies.0);
            assert_eq!(lay.baddies.1, lay.npcs.0);
            assert_eq!(lay.npcs.1, lay.chests.0);
            assert_eq!(lay.chests.1, lay.signs.0);
            assert_eq!(lay.signs.1 + lay.unread_tail, lay.file_len);
            assert_eq!(l.layer0_board().len(), 4096);
            assert_eq!(l.links.len(), lay.link_lines, "every link line parsed");
            let t = |x: i32, y: i32| l.tile_id_at(x, y).unwrap();
            match crc32(&data) {
                0x6271_1327 => {
                    assert_eq!((lay.magic.as_str(), lay.code_bits, lay.file_len), ("GR-V1.03", 13, 8289));
                    assert_eq!((lay.codes, lay.single_runs, lay.double_runs, lay.max_run), (333, 71, 17, 64));
                    assert_eq!((lay.board_end, lay.links, lay.baddies, lay.npcs, lay.chests), (550, (550, 583), (583, 587), (587, 8287), (8287, 8289)));
                    assert_eq!((l.links.len(), l.baddies.len(), l.npcs.len(), l.chests.len(), l.signs.len()), (1, 0, 8, 0, 0));
                    assert_eq!((lay.max_tile, l.layer0_board().iter().filter(|&&v| v == 632).count()), (1899, 3704));
                    // The link is the door: two tiles of the same id, and
                    // the arrival is a half tile.
                    let k = &l.links[0];
                    assert_eq!((k.x, k.y, k.width, k.height, k.new_x, k.new_y), (31, 26, 2, 1, Some(24.5), Some(35.0)));
                    assert_eq!((t(0, 0), t(31, 26), t(32, 26), t(63, 63)), (632, 541, 541, 632));
                    let at: Vec<(f32, f32)> = l.npcs.iter().map(|n| (n.x, n.y)).collect();
                    assert_eq!(at[0], (34.0, 25.0));
                    assert_eq!(at[7], (36.0, 30.0));
                    assert!(l.npcs.iter().all(|n| n.image.ends_with(".gif") && !n.script.is_empty()));
                    assert_eq!(l.npcs.iter().map(|n| n.script.matches('\n').count()).sum::<usize>(), 280);
                }
                0xa489_f475 => {
                    assert_eq!((lay.magic.as_str(), lay.code_bits, lay.file_len), ("GR-V1.03", 13, 8611));
                    assert_eq!((lay.codes, lay.single_runs, lay.double_runs, lay.max_run), (990, 144, 38, 64));
                    assert_eq!((lay.board_end, lay.links, lay.baddies, lay.npcs, lay.chests), (1617, (1617, 1650), (1650, 1654), (1654, 8604), (8604, 8611)));
                    assert_eq!((l.links.len(), l.baddies.len(), l.npcs.len(), l.chests.len(), l.signs.len()), (1, 0, 3, 1, 0));
                    // 2135 is above 2047: only a 13-bit code can carry it.
                    assert_eq!((lay.max_tile, l.layer0_board().iter().filter(|&&v| v == 632).count()), (2135, 2660));
                    let k = &l.links[0];
                    assert_eq!((k.x, k.y, k.width, k.height, k.new_x, k.new_y), (14, 28, 2, 1, Some(21.5), Some(28.0)));
                    let c = &l.chests[0];
                    assert_eq!((c.x, c.y, c.item.as_str(), c.sign_index), (54, 9, "redrupee", -1));
                    // The chest sits on its own tile.
                    assert_eq!((t(54, 9), t(13, 33), t(0, 0)), (2089, 2118, 632));
                    assert_eq!(l.npcs.iter().map(|n| (n.x, n.y)).collect::<Vec<_>>(), vec![(13.0, 33.0), (53.0, 46.0), (31.0, 49.0)]);
                    assert!(l.npcs.iter().all(|n| n.image.is_empty() && n.script.starts_with("// Graal v1.3 NPC")));
                    assert_eq!(l.npcs.iter().map(|n| n.script.matches('\n').count()).sum::<usize>(), 372);
                }
                other => eprintln!("{path:?}: crc {other:08x}, generic checks only"),
            }
        }
        assert!(seen > 0, "no .graal files in the folder");
    }

    /// Bulk run over a folder tree of old `.graal` levels (an old player
    /// world): `SEIHAI_PLAYERWORLD=<dir>`. Asserts only derived facts, never
    /// content. Set `SEIHAI_CORPUS_TSV=<file>` to also write one derived row
    /// per file (CRC-32, sizes, counts, flags).
    #[test]
    #[ignore = "needs a folder tree of .graal files; set SEIHAI_PLAYERWORLD"]
    fn playerworld_graal_corpus_parses() {
        let Some(dir) = std::env::var_os("SEIHAI_PLAYERWORLD") else {
            panic!("set SEIHAI_PLAYERWORLD to a folder tree with .graal files");
        };
        fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let mut es: Vec<_> = std::fs::read_dir(d).unwrap().filter_map(Result::ok).map(|e| e.path()).collect();
            es.sort();
            for p in es {
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("graal")) {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(std::path::Path::new(&dir), &mut files);
        let mut tsv = String::from("crc32\tlen\tmagic\tbits\tcodes\tsingle\tdouble\tmaxrun\ttiles\tmaxtile\tboard_end\tlink_lines\tlinks\tbaddies\tnpcs\tchests\tsigns\tunread\tflags\n");
        let (mut ok, mut clean, mut wide) = (0usize, 0usize, 0usize);
        let mut sums = [0usize; 5];
        let mut by_flag: std::collections::BTreeMap<String, usize> = Default::default();
        for p in &files {
            let data = std::fs::read(p).unwrap();
            let crc = crc32(&data);
            if data.len() < 8 || !(data.starts_with(b"GR-V1") || data.starts_with(b"Z3-V1")) {
                let kind = if data.len() < 8 { "short" } else if data.starts_with(b"GLEVNW01") { "text-level" } else { "other-magic" };
                *by_flag.entry(kind.into()).or_default() += 1;
                tsv.push_str(&format!("{crc:08x}\t{}\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t{kind}\n", data.len()));
                continue;
            }
            // The public entry point agrees with the layout parser.
            assert!(Level::load(p).is_ok(), "Level::load failed for crc {crc:08x}");
            match Level::parse_binary_with_layout(&data) {
                Err(e) => {
                    *by_flag.entry("error".into()).or_default() += 1;
                    tsv.push_str(&format!("{crc:08x}\t{}\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\terror:{e}\n", data.len()));
                }
                Ok((l, y)) => {
                    ok += 1;
                    sums[0] += l.links.len();
                    sums[1] += l.baddies.len();
                    sums[2] += l.npcs.len();
                    sums[3] += l.chests.len();
                    sums[4] += l.signs.len();
                    if y.max_tile >= 2048 {
                        wide += 1;
                    }
                    let mut flags: Vec<&str> = Vec::new();
                    if y.tiles_decoded < 4096 {
                        flags.push("board-short");
                    }
                    if l.links.len() != y.link_lines {
                        flags.push("link-unparsed");
                    }
                    if y.unread_tail > 0 {
                        flags.push("tail");
                    }
                    if y.baddies.1 >= y.file_len && y.npcs.0 >= y.file_len {
                        flags.push("cut-in-baddies");
                    } else if y.npcs.1 >= y.file_len && y.chests.0 >= y.file_len {
                        flags.push("cut-in-npcs");
                    }
                    if flags.is_empty() {
                        clean += 1;
                        flags.push("clean");
                    }
                    let f = flags.join(",");
                    for x in &flags {
                        *by_flag.entry((*x).into()).or_default() += 1;
                    }
                    tsv.push_str(&format!(
                        "{crc:08x}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{f}\n",
                        y.file_len, y.magic, y.code_bits, y.codes, y.single_runs, y.double_runs, y.max_run, y.tiles_decoded,
                        y.max_tile, y.board_end, y.link_lines, l.links.len(), l.baddies.len(), l.npcs.len(), l.chests.len(),
                        l.signs.len(), y.unread_tail
                    ));
                }
            }
        }
        eprintln!("graal files {}, parsed {ok}, clean {clean}; flags {by_flag:?}", files.len());
        if let Some(out) = std::env::var_os("SEIHAI_CORPUS_TSV") {
            std::fs::write(out, tsv).unwrap();
        }
        assert!(!files.is_empty());
        assert_eq!(clean, ok, "files that parsed with a flag: {by_flag:?}");
        // The owner's player world (3141 `.graal` names): 27 are empty and
        // one is a text level under a `.graal` name; the rest are GR-V1.03.
        if files.len() == 3141 {
            assert_eq!((ok, by_flag.get("short"), by_flag.get("text-level")), (3113, Some(&27), Some(&1)));
            // links, baddies, NPCs, chests, signs over all files
            assert_eq!(sums, [9355, 902, 14857, 375, 2381]);
            // 1998 files hold a tile id of 2048 or more: 13-bit codes.
            assert_eq!(wide, 1998);
        }
    }

    /// Old levels write arrival coordinates as `playerx`, `playery+10`,
    /// `playerx-15` and, in one case, a stray word. None of them drops the
    /// link; the signed number is kept as an offset.
    #[test]
    fn link_arrival_expressions_are_kept() {
        let tail = b"a.graal 20 26 4 1 playerx playery+10\nb.graal 12 22 4 1 playerx+15 53\nc.graal 5 5 1 1 PlayerX-1.5 61.5\nd.graal 23 29 2 1 X's Y's\ne.graal 0 0 1 1 7 PLAYERY\n#\n";
        let mut d = binary_level(b"GR-V1.03", 13, tail);
        d.extend_from_slice(&[0xFF, 0xFF, 0xFF, b'\n', b'#', b'\n', b'#', b'\n']);
        let (l, lay) = Level::parse_binary_with_layout(&d).unwrap();
        assert_eq!((l.links.len(), lay.link_lines), (5, 5));
        let v: Vec<_> = l.links.iter().map(|k| (k.new_x, k.new_y, k.x_offset, k.y_offset)).collect();
        assert_eq!(v[0], (None, None, 0.0, 10.0));
        assert_eq!(v[1], (None, Some(53.0), 15.0, 0.0));
        assert_eq!(v[2], (None, Some(61.5), -1.5, 0.0));
        assert_eq!(v[3], (None, None, 0.0, 0.0));
        assert_eq!(v[4], (Some(7.0), None, 0.0, 0.0));
    }

    /// A baddy at x = 0 is a baddy: only a first byte of 0x80 or more ends
    /// the section (proved by two real levels; a reference decoder that
    /// also stopped at 0 lost them and misread the NPCs after them).
    #[test]
    fn baddy_at_column_zero_does_not_end_the_section() {
        let mut tail: Vec<u8> = b"#\n".to_vec();
        tail.extend_from_slice(&[43, 5, 7]);
        tail.extend_from_slice(b"\\\\\n");
        tail.extend_from_slice(&[0, 62, 6]);
        tail.extend_from_slice(b"\\\\\n");
        tail.extend_from_slice(&[0xFF, 0xFF, 0xFF, b'\n']);
        tail.extend_from_slice(&[32 + 3, 32 + 4]);
        tail.extend_from_slice(b"x.gif#show;\n#\n#\n");
        let (l, lay) = Level::parse_binary_with_layout(&binary_level(b"GR-V1.03", 13, &tail)).unwrap();
        assert_eq!((lay.baddy_records, lay.npc_records), (2, 1));
        assert_eq!((l.baddies[1].x, l.baddies[1].y, l.baddies[1].kind.as_str()), (0.0, 62.0, "6"));
        assert_eq!(l.npcs[0].image, "x.gif");
    }
}
