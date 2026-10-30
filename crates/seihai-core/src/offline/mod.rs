//! Offline play support.
//!
//! Allows loading and simulating levels, NPCs, and a local player
//! without a network connection. Behavior is provisional and will
//! be refined against the original client’s offline/tutorial paths.

use crate::assets::{GaniPlayer, Level, Link, Object2x2, ObjectTables, TileKind, TileTypes};
use crate::sound::SoundEvent;
use crate::dev::DevMode;
use crate::gmap::{Crossing, GmapWorld, Resolved};
use crate::life::{respawn_half_hearts, Life, LifeEvent};
use crate::protocol::{BoardModify, PlayerUpdate};
use std::collections::{HashMap, HashSet};

mod drops;
mod hide;
mod pause;
mod pushpull;
mod servernpcs;
pub use hide::{HIDDEN_STATUS, HIDE_KINDS};
pub use pause::{focus_pauses, name_label, pause_message_bits, server_status_base, PAUSED_STATUS};
pub use pushpull::{NpcMoved, Grab, Jump, PushPull, Trigger};
pub use servernpcs::{block_flags, Linked, ServerNpcSync, NPC_SEND_INTERVAL};
use pushpull::StepResult;

/// Facing, in Graal order (ganis and the 2.05 wire): 0 up, 1 left,
/// 2 down, 3 right.
pub const DIR_NORTH: u8 = 0;
/// The client's tick for the lava burn, and the count it must pass.
const LAVA_TICK_MS: u64 = 50;
const LAVA_BURN_TICKS: u32 = 100;
pub const DIR_WEST: u8 = 1;
pub const DIR_SOUTH: u8 = 2;
pub const DIR_EAST: u8 = 3;

/// Where the feet are, from the sprite's top-left, in tiles. This is the
/// one point the original client tests for walls, water and links.
pub const FOOT_DX: f32 = 2.0;
pub const FOOT_DY: f32 = 2.75;

/// Where the 2.05 client tests for a chair (tile type 3), from the sprite's
/// top-left: one point, `(x + 1.5, y + 2)`, the middle of the body.
pub const SIT_PROBE: (f32, f32) = (1.5, 2.0);
/// Where it tests for the top half of a bed (type 4): a left point at
/// `x + 1` and a right one at `x + 1.8`, each at `y` or `y + 0.5`. Both
/// sides must be on a bed top.
pub const BED_PROBE_DX: [f32; 2] = [1.0, 1.8];
pub const BED_PROBE_DY: [f32; 2] = [0.0, 0.5];

/// Where the real client tests for walls while walking, from the sprite's
/// top-left, in tiles, per facing (0 up, 1 left, 2 down, 3 right): six points
/// on the leading edge of the body box (x + 0.5 to x + 2.5, y + 1 to y + 3),
/// taken at the position the step would reach. The same table is in the
/// 2.05, 2.17.1, 2.22 and 2.31 clients. A point is read as the tile
/// `floor(coordinate + 0.0001)`.
///
/// * Points 0 to 3 decide the step: if any is blocked the step is refused.
/// * Points 4 and 5 only steer the corner nudge ([`WALK_NUDGE`]).
///
/// See `documentation/walking-collision.md`.
pub const WALK_PROBES: [[(f64, f64); 6]; 4] = [
    [(0.525, 1.0), (1.475, 1.0), (1.525, 1.0), (2.475, 1.0), (2.525, 1.0), (0.475, 1.0)],
    [(0.5, 1.025), (0.5, 1.975), (0.5, 2.025), (0.5, 2.975), (0.5, 3.025), (0.5, 0.975)],
    [(0.525, 3.0), (1.475, 3.0), (1.525, 3.0), (2.475, 3.0), (2.525, 3.0), (0.475, 3.0)],
    [(2.5, 1.025), (2.5, 1.975), (2.5, 2.025), (2.5, 2.975), (2.5, 3.025), (2.5, 0.975)],
];

/// The corner nudge per facing: `[(dx, dy) when probe 0 is the blocked one,
/// (dx, dy) when probe 3 is]`. A refused step slides the player 0.1 tile
/// along the edge, towards the side that is free.
pub const WALK_NUDGE: [[(f64, f64); 2]; 4] = [
    [(0.1, 0.0), (-0.1, 0.0)],
    [(0.0, 0.1), (0.0, -0.1)],
    [(0.1, 0.0), (-0.1, 0.0)],
    [(0.0, 0.1), (0.0, -0.1)],
];

/// The client floors every coordinate it turns into a tile or a snap line
/// as `floor(v + 0.0001)`.
const WALK_FLOOR_EPS: f64 = 0.0001;
/// After a step the position is pulled to the tile line when it is within
/// this much of it (y on a whole number, x + 0.5 on a whole number).
const WALK_SNAP_EDGE: f64 = 0.0625;

fn walk_floor(v: f64) -> f64 {
    (v + WALK_FLOOR_EPS).floor()
}

/// The client's `Frac`: the part of `v` above `walk_floor(v)`, 0 if `v` is
/// just under a whole number.
fn walk_frac(v: f64) -> f64 {
    let f = walk_floor(v);
    if f > v {
        0.0
    } else {
        v - f
    }
}

/// Walking and swimming step times. A step is half a tile. Swimming is as
/// fast as walking: 0.5 tile a tick, read live in 2.05 rev 2 (Graal_analysis
/// `player-mannerisms-live.md`).
pub const WALK_STEP_MS: u64 = 50;
pub const SWIM_STEP_MS: u64 = 50;

/// When the step just taken counts from, so the pace holds on a loop that
/// ticks every 16 ms or so. A step is due `interval` after the last one; if
/// the loop only noticed it late (a 50 ms step seen after 64 ms), the next
/// step still counts from the time it was due, not from when it was taken. That
/// keeps the average at one step per `interval` (20 per second, 10 tiles a
/// second, as the real clients walk). After a pause of two intervals or more
/// there is nothing to catch up on, and the clock starts from `now`.
pub fn next_step_clock(last: std::time::Instant, now: std::time::Instant, interval: std::time::Duration) -> std::time::Instant {
    if now.saturating_duration_since(last) >= interval * 2 {
        now
    } else {
        last + interval
    }
}

/// The box, as offsets from the player's position in tiles, that the real
/// 2.22 and 2.31 clients test against level links, by walking direction
/// (`DIR_NORTH`, `DIR_WEST`, `DIR_SOUTH`, `DIR_EAST`): `(x0, y0, x1, y1)`.
///
/// Measured in the private lab on 2026-10-25, with the player put on half-tile
/// positions and walking into a rectangle link and the edge links; the only
/// thing the captures show is *where on the half-tile grid the first warp
/// goes out*, so each edge is a value inside the range the captures allow
/// (see `documentation/link-geometry.md`):
///
/// * a left or right step fires once the leading edge, `x + 0.25` or
///   `x + 2.75`, is inside the link; the box is as tall as the feet strip
///   (`y + 2` to `y + 3`);
/// * an up or down step fires once the leading edge, `y + 0.75` or
///   `y + 3.25`, is inside the link; the box is one tile wide, centred on
///   `x + 1.5`.
pub const LINK_BOX: [(f32, f32, f32, f32); 4] = [
    (1.0, 0.75, 2.0, 1.25), // north
    (0.25, 2.0, 0.75, 3.0), // west
    (1.0, 2.75, 2.0, 3.25), // south
    (2.25, 2.0, 2.75, 3.0), // east
];

/// A locally simulated game session (no server).
#[derive(Debug)]
pub struct OfflineSession {
    pub current_level: Option<Level>,
    pub tiletypes: Option<TileTypes>,
    /// The cut and lift tables of `arrays.dat`, if one was loaded.
    pub object_tables: Option<ObjectTables>,
    /// What the player is carrying after a lift: the object's tiles.
    pub carried: Option<Carried>,
    /// Objects in flight, break effects playing, and thrown things that stay
    /// (see [`crate::throwing`]).
    pub thrown: Vec<crate::throwing::Thrown>,
    pub leaps: Vec<crate::throwing::Leap>,
    pub grounded: Vec<crate::throwing::Grounded>,
    /// Bombs on the ground and explosion pieces (see [`crate::bombs`]).
    pub placed_bombs: Vec<crate::bombs::Bomb>,
    pub blast: Vec<crate::bombs::Piece>,
    /// Milliseconds carried over between bomb ticks.
    bomb_ms: u64,
    /// `PLI_BOMBADD` messages since the last [`Self::take_bomb_sends`].
    bomb_sends: Vec<Vec<u8>>,
    /// Arrows in flight (see [`crate::arrows`]).
    pub arrows: Vec<crate::arrows::Arrow>,
    /// Milliseconds carried over between arrow ticks.
    arrow_ms: u64,
    /// `PLI_ARROWADD` messages since the last [`Self::take_arrow_sends`].
    arrow_sends: Vec<Vec<u8>>,
    /// The ids in the server's player list, online (empty offline). A
    /// bomb's piece or a shot names its owner only while the owner is here
    /// ([`crate::bombs::player_owner`]).
    pub present_players: Vec<u16>,
    /// NPCs an arrow hit since the last [`Self::take_shots`].
    shots: Vec<usize>,
    /// Landings since the last [`Self::take_landings`].
    landings: Vec<Landing>,
    /// Milliseconds carried over between flight ticks.
    throw_ms: u64,
    /// NPCs a thrown object hit since the last [`Self::take_pelts`]: the
    /// id the caller's NPC test returned, for the `waspelt` event.
    pelts: Vec<usize>,
    /// Board areas the player changed since the last
    /// [`Self::take_board_changes`].
    board_changes: Vec<BoardChange>,
    /// Another player's thrown object hurt the player since the last
    /// [`Self::take_throw_hurt`].
    throw_hurt: bool,
    pub player_x: f32,
    pub player_y: f32,
    pub player_dir: u8,
    /// True when the player is standing on a water tile (type == 11).
    pub in_water: bool,
    /// The swing lasts the sword GANI (see `CombatState::set_swing_ticks`):
    /// set for 2.22, the build that was checked.
    pub gani_swing: bool,
    /// Optional walk animation.
    pub walk_gani: Option<GaniPlayer>,
    /// Optional swim animation, shown instead of the walk one while the
    /// feet are in water. It runs on the tick clock, moving or not.
    pub swim_gani: Option<GaniPlayer>,
    /// Ticks in the water, for the swim frame.
    pub(crate) swim_ticks: usize,
    /// Ticks since an object was picked up, while `lift.gani` may still
    /// show; `None` after.
    pub lift_ticks: Option<u32>,
    /// The 50 ms clock of [`Self::tick_poses`].
    pub(crate) pose_clock_ms: u64,
    /// A held arrow walked the player out of the hurt pose in this hurt
    /// mode.
    pub(crate) hurt_walked: bool,
    /// Hiding under the carried object (see `offline/hide.rs`).
    pub hidden: bool,
    /// Paused: the pause key, or the window without the focus (see
    /// `offline/pause.rs`).
    pub paused: bool,
    /// The pause key's own state.
    pub(crate) pause_key: bool,
    /// The window has the focus.
    pub(crate) focused: bool,
    /// Steps walked while hidden, for the hidden walk sprite.
    pub(crate) hide_steps: usize,
    /// The swim stroke was running with the player moving, last tick.
    pub(crate) swim_was_moving: bool,
    /// How this build moves and looks (blink, pull release, swim when
    /// still): see [`crate::protocol::Mannerisms`].
    pub mannerisms: crate::protocol::Mannerisms,
    /// Who drops bush and pot items in this build (see `offline/drops.rs`).
    pub drops: crate::protocol::ItemDrops,
    /// Items lying on the floor of the current level.
    pub floor_items: Vec<crate::items::FloorItem>,
    /// The 1.x drop roll's `Random`.
    pub(crate) item_rng: crate::items::DelphiRandom,
    pub(crate) item_ms: u64,
    /// Rolls, pickups and timeouts for [`Self::take_item_messages`].
    pub(crate) item_events: Vec<crate::items::ItemEvent>,
    /// The sword, hurt and sit animations (see [`crate::pose`]).
    pub poses: crate::pose::PoseGanis,
    /// GANIs for animated NPCs, by lower-case name (see [`crate::npcart`]).
    pub npc_ganis: HashMap<String, crate::assets::Gani>,
    /// The body is on a chair tile (type 3): the sit pose is drawn. Set by
    /// [`Self::refresh_rest`].
    pub sitting: bool,
    /// The head is on the top half of a bed (type 4): the sleep pose is
    /// drawn. Set by [`Self::refresh_rest`].
    pub sleeping: bool,
    /// `seteffect red,green,blue,alpha`: the colour the map is shaded with
    /// (default 0,0,0,0, no shading).
    pub screen_effect: [f32; 4],
    /// Images a weapon script put up with `showimg`: weapons act as the
    /// player, so these sit on the player's level at the player's height.
    /// Cleared when a level is loaded.
    pub player_images: Vec<crate::assets::NpcImage>,
    /// Lava burn ticks so far, and time not yet a whole tick.
    lava_ticks: u32,
    lava_ms: u64,
    /// The last step moved the player and no stop has come since: the walk
    /// animation is drawn instead of idle. Cleared by [`Self::stop_walking`].
    pub walking: bool,
    /// Sounds from GANI frames entered and from scripts (`play`,
    /// `playlooped`) since the last [`Self::take_sound_events`].
    pub sound_events: Vec<SoundEvent>,
    /// `stopsound` names since the last [`Self::take_sound_stops`].
    pub sound_stops: Vec<String>,
    /// Player health (for `hurt`).
    pub player_health: i32,
    /// Last message from `say` / `message`.
    pub last_message: Option<String>,
    /// Flags set by `set` / `unset`.
    pub flags: HashMap<String, bool>,
    /// String variables (`setstring` / `addstring`).
    pub strings: HashMap<String, String>,
    /// Player properties (`setplayerprop`).
    pub props: HashMap<String, String>,
    /// Weapons the player holds (`toweapons`).
    pub weapons: Vec<String>,
    /// Pending link destination if the player walked into a LINK area.
    pub pending_link: Option<(String, f32, f32)>,
    /// Rupees (gralats).
    pub rupees: i32,
    pub bombs: i32,
    pub darts: i32,
    /// Sword power 1–4 and shield power 1–3.
    pub sword_power: u8,
    pub shield_power: u8,
    /// Head, body and body colours (`sethead`, `setbody`, `set*color`,
    /// the chat commands, or prop 13 from the server).
    pub looks: crate::looks::Looks,
    /// Set when a script or chat line changed `looks`, so the game loop
    /// can tell the server; cleared by the loop.
    pub looks_changed: bool,
    /// Player flags a script set (`Some(value)`, empty for a plain `set`) or
    /// cleared (`None`) whose value changed, for the server: online they go
    /// out as `PLI_FLAGSET` (18) and `PLI_FLAGDEL` (19). Taken by the game
    /// loop.
    pub flags_out: Vec<(String, Option<String>)>,
    /// Whether the glove is held, and its power.
    pub glove_power: u8,
    /// Ledge jumps, pushing, and the A key's grab and pull (see
    /// [`pushpull`]).
    pub pushpull: PushPull,
    /// Heart containers, in half hearts as Graal counts them.
    pub max_hearts: i32,
    /// Chests opened in the current level, by tile position.
    pub opened_chests: HashSet<(i32, i32)>,
    /// Sign or chest text being read. The game waits until it is closed.
    pub reading: Option<String>,
    /// Developer-only switches (wallhack). Off at start, never saved.
    pub dev: DevMode,
    /// Set from the moment a level warp is asked for until the new level
    /// arrives. Nothing walks and no link fires meanwhile.
    pub warp_pending: Option<String>,
    /// On a gmap: the grid, the player's cell and the neighbouring levels.
    /// `player_x`/`player_y` stay local to that cell. Walking past an edge
    /// moves the player into the next cell (see [`Self::take_gmap_crossing`]).
    pub gmap: Option<GmapWorld>,
    /// The big map (the map screen's grid of level names), when one is
    /// known. Edge links to the level next door on it keep the player's
    /// offset across the seam (see [`edge_link_arrival`]).
    pub big_map: Option<crate::gmap::Gmap>,
    /// False from an arrival until the player has moved: a link cannot
    /// fire on the spot the player was put on.
    armed: bool,
    /// Test links with [`LINK_BOX`] (the profile's `levels.link_boxes`)
    /// instead of the legacy foot point.
    pub link_boxes: bool,
    /// Tile the player arrived on. Links that contain it stay off until
    /// the player steps out of them.
    arrival_tile: Option<(i32, i32)>,
    /// Swimming state last reported to the server.
    swim_sent: bool,
    /// Counts levels loaded with [`OfflineSession::load_level`], so a
    /// caller can tell when its per-level state (scripts) is stale.
    pub level_serial: u64,
    /// The death sequence (timer, input lock, dead bit). It has no rules
    /// until the game loop gives it its version's (see
    /// [`Life::set_rules`]), so a bare session never dies.
    pub life: Life,
    /// What the server knows of the hearts and pose (see
    /// [`Self::take_power_report`]).
    pub power: crate::life::PowerReporter,
    /// The sword swing and hurt mode (see [`crate::combat`]).
    pub combat: crate::combat::CombatState,
    /// Alignment points (AP, 0–100): decide the hearts after a respawn.
    /// Starts at 50, the GServer default account; the server's
    /// `PLPROP_ALIGNMENT` replaces it online.
    pub alignment: u8,
    /// The level entry point a respawn returns to; see [`Self::note_entry`].
    pub entry: Option<EntryPoint>,
    /// The level entered last, whether or not it was saved as the entry
    /// point: the "level left" of the next [`Self::note_entry`].
    entered: Option<String>,
    /// The current level said `noplayerkilling`: sword and packet hits
    /// from other players do nothing. Cleared when a level loads.
    pub no_pk: bool,
    /// Per attacker id: the second of the day it was last judged (see
    /// `CombatRules::judge_throttle`).
    pub judged_at: std::collections::HashMap<u16, u32>,
    /// A hit's knockback moved the player since the last [`OfflineSession::take_knock_moved`].
    pub(crate) knock_moved: bool,
    /// File name of the level loaded last with [`Self::load_level`].
    loaded_file: Option<String>,
}

/// Where the player last entered a level: the point a respawn goes back to.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryPoint {
    pub level: String,
    pub x: f32,
    pub y: f32,
    pub dir: u8,
}

/// Where a respawn put the player.
#[derive(Debug, Clone, PartialEq)]
pub enum RespawnTo {
    /// The saved level entry point. The caller loads the level when it is
    /// not the current one and, online, re-sends the level at `(x, y)`.
    Saved(EntryPoint),
    /// Nothing saved and online: the profile's default position in the
    /// current level. Online, the current level is re-sent there.
    Default { x: f32, y: f32 },
    /// Nothing saved and offline: the player stays where they are. What the
    /// real client does here is not documented (see `health-death.md`).
    InPlace,
}

/// An object the player lifted: which lift entry it was and its tiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carried {
    pub kind: usize,
    pub tiles: Object2x2,
}

/// A thrown object coming down.
#[derive(Debug, Clone, PartialEq)]
pub struct Landing {
    /// Lift-table entry of the object.
    pub kind: usize,
    /// Break effect started (a row of the break table), `None` for an
    /// object that stays on the ground.
    pub effect: Option<usize>,
    pub x: f32,
    pub y: f32,
}

/// What an interact press did.
/// A rectangle of the player's level the client changed, with its new tiles
/// row by row (see [`OfflineSession::take_board_changes`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardChange {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub tiles: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Interaction {
    /// An object (bush, rock, ...) was lifted; `kind` is its entry in the
    /// lift table.
    Lifted { kind: usize },
    /// A faced a wall (a rock too heavy for the glove counts): the player
    /// holds on to it until A is let go, and can pull (see [`pushpull`]).
    Grabbed,
    /// The carried object was thrown; `kind` is its lift-table entry.
    Thrown { kind: usize },
    /// A closed chest was opened at `(x, y)`; `got` describes the item.
    Chest { x: i32, y: i32, item: String, got: String },
    Nothing,
}

/// The message that tells a 2.1+ server the player started or stopped
/// swimming: `PLI_PLAYERPROPS` (2) with prop 10 (`PLPROP_GANI`) as a
/// GSTRING, `swim` or `idle`.
///
/// GServer keeps no swimming status. It stores the gani name and relays
/// it to the other players in the level, so that is how they see the
/// swim animation. Clients before 2.1 read prop 10 as the bow slot, so
/// nothing is sent for them.
pub fn swim_message(swimming: bool) -> Vec<u8> {
    let name: &[u8] = if swimming { b"swim" } else { b"idle" };
    let mut m = vec![2 + 0x20, 10 + 0x20, name.len() as u8 + 0x20];
    m.extend_from_slice(name);
    m.push(b'\n');
    m
}

/// The name group of a level name (2.05's save-entry rule): the name without
/// its folder, lower-cased, up to and including its last `§` or `$`; empty
/// when it has neither.
pub fn name_group(level: &str) -> String {
    let base = level.rsplit(['/', '\\', ':']).next().unwrap_or(level).to_lowercase();
    match base.rfind(['§', '$']) {
        Some(i) => base[..i + base[i..].chars().next().map_or(1, char::len_utf8)].to_string(),
        None => String::new(),
    }
}

/// Where an edge link to the big-map neighbour on that side puts the
/// player, or `None` for any other link. The 2.05 client: a link one tile
/// wide on the west edge (x 0), east edge (x 63), north edge (y 0) or south
/// edge (y 63), whose destination is the level next to `here` on that side
/// of the big map, arrives at the player's position moved by 64 tiles
/// across the seam, the other coordinate kept; the link's own new-x and
/// new-y are not used. (Graal_analysis `03-edge-links.md`.)
///
/// The real client fires such a link once the player has crossed the edge
/// (x past 63.5 east, below -2.5 west, y below -3 north, past 63 south)
/// when the neighbour level is loaded; Seihai fires on the link rectangle,
/// before the player gets that far, so the arrival is held to where the
/// real crossing would put it (a Seihai choice): at least -0.5 going east,
/// at most 61.5 going west, at most 61 going north, at least -1 going
/// south.
pub fn edge_link_arrival(map: &crate::gmap::Gmap, here: &str, link: &Link, x: f32, y: f32) -> Option<(f32, f32)> {
    let next = |dx, dy| map.neighbour_of(here, dx, dy).is_some_and(|n| n.eq_ignore_ascii_case(&link.dest));
    if link.x == 0 && link.width == 1 && next(-1, 0) {
        Some(((x + 64.0).min(61.5), y))
    } else if link.x == 63 && link.width == 1 && next(1, 0) {
        Some(((x - 64.0).max(-0.5), y))
    } else if link.y == 0 && link.height == 1 && next(0, -1) {
        Some((x, (y + 64.0).min(61.0)))
    } else if link.y == 63 && link.height == 1 && next(0, 1) {
        Some((x, (y - 64.0).max(-1.0)))
    } else {
        None
    }
}

impl OfflineSession {
    pub fn new() -> Self {
        Self {
            current_level: None,
            tiletypes: None,
            object_tables: None,
            carried: None,
            thrown: Vec::new(),
            leaps: Vec::new(),
            grounded: Vec::new(),
            placed_bombs: Vec::new(),
            blast: Vec::new(),
            bomb_ms: 0,
            bomb_sends: Vec::new(),
            arrows: Vec::new(),
            arrow_ms: 0,
            arrow_sends: Vec::new(),
            present_players: Vec::new(),
            shots: Vec::new(),
            landings: Vec::new(),
            throw_ms: 0,
            pelts: Vec::new(),
            board_changes: Vec::new(),
            throw_hurt: false,
            player_x: 30.0,
            player_y: 30.0,
            player_dir: DIR_SOUTH,
            in_water: false,
            gani_swing: false,
            walk_gani: None,
            swim_gani: None,
            swim_ticks: 0,
            lift_ticks: None,
            pose_clock_ms: 0,
            hurt_walked: false,
            hidden: false,
            paused: false,
            pause_key: false,
            focused: true,
            hide_steps: 0,
            swim_was_moving: false,
            mannerisms: crate::protocol::MANNER_205,
            drops: crate::protocol::DROPS_142,
            floor_items: Vec::new(),
            item_rng: crate::items::DelphiRandom::from_clock(),
            item_ms: 0,
            item_events: Vec::new(),
            poses: crate::pose::PoseGanis::default(),
            sitting: false,
            sleeping: false,
            screen_effect: [0.0; 4],
            player_images: Vec::new(),
            lava_ticks: 0,
            lava_ms: 0,
            walking: false,
            npc_ganis: HashMap::new(),
            sound_events: Vec::new(),
            sound_stops: Vec::new(),
            player_health: 100,
            last_message: None,
            flags: HashMap::new(),
            strings: HashMap::new(),
            props: HashMap::new(),
            weapons: Vec::new(),
            pending_link: None,
            rupees: 0,
            bombs: 10,
            darts: 10,
            sword_power: 1,
            shield_power: 1,
            looks: crate::looks::Looks::default(),
            looks_changed: false,
            flags_out: Vec::new(),
            // 2.05 starts every player at glove 1 (enough for a pot), as
            // GServer's default account does (`GLOVEP 1`).
            glove_power: 1,
            pushpull: PushPull::default(),
            max_hearts: 6,
            opened_chests: HashSet::new(),
            reading: None,
            dev: DevMode::default(),
            warp_pending: None,
            gmap: None,
            big_map: None,
            armed: true,
            link_boxes: false,
            arrival_tile: None,
            swim_sent: false,
            level_serial: 0,
            life: Life::default(),
            power: crate::life::PowerReporter::default(),
            combat: crate::combat::CombatState::default(),
            alignment: 50,
            entry: None,
            entered: None,
            no_pk: false,
            judged_at: HashMap::new(),
            knock_moved: false,
            loaded_file: None,
        }
    }

    /// Current hearts in half hearts, from the health percentage and
    /// `max_hearts`. 0 only when the health is 0.
    pub fn hearts_half(&self) -> i32 {
        if self.player_health <= 0 {
            return 0;
        }
        ((self.player_health as f64 * self.max_hearts as f64 / 100.0).round() as i32).clamp(1, self.max_hearts.max(1))
    }

    /// Set the hearts in half hearts (clamped to the maximum). A positive
    /// value never rounds down to 0 health, so rounding cannot kill.
    pub fn set_hearts_half(&mut self, half: i32) {
        let max = self.max_hearts.max(1);
        let half = half.clamp(0, max);
        self.player_health = if half == 0 {
            0
        } else {
            ((half as f64 / max as f64 * 100.0).round() as i32).clamp(1, 100)
        };
    }

    /// New maximum in half hearts. Current hearts stay as they are, up to
    /// the new maximum.
    pub fn set_max_hearts_half(&mut self, max: i32) {
        let cur = self.hearts_half();
        self.max_hearts = max.clamp(0, 80);
        self.set_hearts_half(cur);
    }

    /// Hearts and AP the server gave: MAXPOWER (heart containers), CURPOWER
    /// (hearts × 2, so half hearts) and ALIGNMENT. Max first, so a CURPOWER
    /// in the same message is read against the new maximum.
    pub fn apply_server_power(&mut self, max_power: Option<u8>, cur_power: Option<u8>, alignment: Option<u8>) {
        if let Some(m) = max_power {
            // Zero from the server leaves the default. A real account
            // has hearts; zero is either a missing prop or a freeze.
            if m > 0 {
                self.set_max_hearts_half(i32::from(m) * 2);
            } else {
                eprintln!("server sent max power 0; keeping {}", self.max_hearts);
            }
        }
        if let Some(c) = cur_power {
            // Zero is applied, and the player dies: the 2.05 prop reader
            // (case 2 at 0x530dd8) has no zero test, and a 2.22 client sent
            // the dead gani on an injected CURPOWER 0. Respawn gives hearts.
            if c == 0 && self.hearts_half() > 0 {
                eprintln!("server sent cur power 0; hearts {} -> 0", self.hearts_half());
            }
            self.set_hearts_half(i32::from(c));
        }
        if let Some(a) = alignment {
            self.alignment = a.min(100);
        }
        if max_power.is_some() || cur_power.is_some() {
            self.power.server_set(self.hearts_half());
        }
    }

    /// What the server must hear about the player's hearts and pose since
    /// the last report: a hit, a death, a respawn, a lava burn, a heart.
    /// `None` when nothing changed. The caller sends it online
    /// ([`crate::life::PowerReport::message`]).
    pub fn take_power_report(&mut self) -> Option<crate::life::PowerReport> {
        let half = self.hearts_half();
        let dead = self.life.is_dead() || (half <= 0 && self.life.rules().is_some());
        let hurting = self.combat.is_hurt();
        let sprite = self.sprite_value();
        self.power.report(half, dead, hurting, sprite)
    }

    /// Take damage in half hearts from `attacker` (a player id) or from
    /// nothing known. A player who is dead cannot be hurt. Returns false
    /// when the hit did nothing. The death itself starts at the next
    /// [`Self::tick_life`].
    pub fn damage_half(&mut self, half: i32, attacker: Option<u16>) -> bool {
        if self.life.is_dead() || half <= 0 {
            return false;
        }
        let left = self.hearts_half() - half;
        self.set_hearts_half(left.max(0));
        self.life.note_attacker(attacker);
        true
    }

    /// Advance the death sequence by `ms` and return what happened. At the
    /// start of a death the player faces down. Controls stay locked until
    /// the timer is past the profile's limit (see [`Self::controls_locked`]).
    pub fn tick_life(&mut self, ms: u64) -> Vec<LifeEvent> {
        self.tick_lava(ms);
        let zero = self.player_health <= 0 || self.hearts_half() <= 0;
        self.life.tick(ms, zero);
        let events = self.life.take_events();
        if events.iter().any(|e| matches!(e, LifeEvent::Died { .. })) {
            eprintln!("died (hearts {})", self.hearts_half());
            self.player_dir = DIR_SOUTH;
            self.reading = None;
        }
        events
    }

    /// The lava burn on its 50 ms tick. The 2.05 client tests the point
    /// `(x + 1.5, y + 2)` against the lava list each tick: on lava a count
    /// goes up, and past 100 the player loses half a heart and the count
    /// starts again; off lava the count is 0. Returns the half hearts taken.
    pub fn tick_lava(&mut self, ms: u64) -> i32 {
        self.lava_ms += ms;
        let mut taken = 0;
        while self.lava_ms >= LAVA_TICK_MS {
            self.lava_ms -= LAVA_TICK_MS;
            if self.life.is_dead() || !self.on_lava() {
                self.lava_ticks = 0;
                continue;
            }
            self.lava_ticks += 1;
            if self.lava_ticks > LAVA_BURN_TICKS {
                self.lava_ticks = 0;
                if self.damage_half(1, None) {
                    taken += 1;
                }
            }
        }
        taken
    }

    /// Whether the burn test point is on a tile of the lava list.
    pub fn on_lava(&self) -> bool {
        let (Some(level), Some(types)) = (&self.current_level, &self.tiletypes) else { return false };
        let (px, py) = (self.player_x + 1.5, self.player_y + 2.0);
        if !(0.0..64.0).contains(&px) || !(0.0..64.0).contains(&py) {
            return false;
        }
        level.tile_id_at(px as i32, py as i32).is_some_and(|id| types.is_lava(id))
    }

    /// The player cannot act: dead and the timer not yet past the lock.
    pub fn controls_locked(&self) -> bool {
        self.life.input_locked()
    }

    /// The name of the player's level: on a gmap the level of the current
    /// cell, else the file last loaded.
    pub fn level_name(&self) -> Option<String> {
        match &self.gmap {
            Some(w) => w.map.level_at(w.cell.0, w.cell.1).map(str::to_string),
            None => self.loaded_file.clone(),
        }
    }

    /// True when the level loaded last is the one named `name` (a file name,
    /// with or without `.nw`, any case).
    pub fn is_loaded(&self, name: &str) -> bool {
        let base = |n: &str| n.strip_suffix(".nw").or_else(|| n.strip_suffix(".NW")).unwrap_or(n).to_ascii_lowercase();
        self.loaded_file.as_deref().is_some_and(|f| base(f) == base(name))
    }

    /// A level was entered (or the session started in it): maybe remember
    /// where the player stands as the entry point a respawn returns to.
    /// The client saves when no level was left (the first entry), or when
    /// exactly one of the level left and the new level is in the current
    /// gmap's level list at an index above 0. (The client multiplies the
    /// two list indices, -1 for "not in the list", and saves on a negative
    /// product, so index 0 counts as "same side": a quirk, kept.) Otherwise
    /// builds with [`DeathRules::entry_name_group`] (2.00 to 2.05) save only when
    /// the two names are in different name groups ([`name_group`]); later
    /// builds do not save. A session without death rules saves every entry.
    pub fn note_entry(&mut self, level: &str) {
        let old = self.entered.replace(level.to_string());
        let save = match self.life.rules().copied() {
            None => true,
            Some(rules) => match old.as_deref().filter(|o| !o.is_empty()) {
                None => true,
                Some(old) => {
                    let list = |n: &str| match &self.gmap {
                        Some(g) => g.map.list_index(n).map_or(-1, |i| i as i64),
                        None => -1,
                    };
                    list(level) * list(old) < 0 || (rules.entry_name_group && name_group(old) != name_group(level))
                }
            },
        };
        if save {
            self.entry = Some(EntryPoint { level: level.to_string(), x: self.player_x, y: self.player_y, dir: self.player_dir });
        }
    }

    /// A movement or action key while dead: respawn if the lock is over.
    /// Hearts follow the profile's table by AP, the position is the saved
    /// entry point (or the default), and the death sequence ends. `online`
    /// picks the default when nothing was saved. `None` when the player
    /// could not respawn.
    pub fn respawn(&mut self, online: bool) -> Option<RespawnTo> {
        let rules = *self.life.rules()?;
        if !self.life.respawn() {
            return None;
        }
        let hearts = respawn_half_hearts(&rules, self.alignment, self.max_hearts);
        self.set_hearts_half(hearts.max(1));
        let to = match self.entry.clone() {
            Some(e) => {
                self.player_x = e.x;
                self.player_y = e.y;
                self.player_dir = e.dir;
                RespawnTo::Saved(e)
            }
            None if online => {
                let (x, y) = (f32::from(rules.default_x2) / 2.0, f32::from(rules.default_y2) / 2.0);
                self.player_x = x;
                self.player_y = y;
                self.player_dir = DIR_SOUTH;
                RespawnTo::Default { x, y }
            }
            None => RespawnTo::InPlace,
        };
        self.refresh_water();
        Some(to)
    }

    /// The feet, in tiles: the point used for links, water and scripts
    /// (walking tests the body box, see [`WALK_PROBES`]).
    pub fn foot(&self) -> (f32, f32) {
        (self.player_x + FOOT_DX, self.player_y + FOOT_DY)
    }

    /// The point the A key probes, in tiles: one tile beyond the middle of
    /// the body box's leading edge (north `(x + 1.5, y)`, west `(x - 0.5,
    /// y + 2)`, south `(x + 1.5, y + 4)`, east `(x + 3.5, y + 2)`).
    ///
    /// The 2.05 client reads this table for lifting and for the sword's cut.
    /// Chests use it here too; that is the one assumption, since the point
    /// the client uses for them was not read. It fits the walk: a player
    /// stopped by a chest has exactly this point inside it. Signs are not
    /// read through it (see [`Self::read_sign_ahead`]).
    pub fn facing_point(&self) -> (f32, f32) {
        let (x, y) = (self.player_x, self.player_y);
        match self.player_dir {
            DIR_NORTH => (x + 1.5, y),
            DIR_WEST => (x - 0.5, y + 2.0),
            DIR_EAST => (x + 3.5, y + 2.0),
            _ => (x + 1.5, y + 4.0),
        }
    }

    /// Interact with what the player faces: throw, lift, grab, or open a
    /// closed chest (2×2 tiles) from below.
    pub fn interact(&mut self) -> Interaction {
        if self.life.is_dead() || self.jumping() || self.reading.is_some() {
            return Interaction::Nothing;
        }
        // The A key tries to lift first, then reads a sign or opens a chest.
        // With something in the hands it throws it.
        if let Some(c) = self.throw_carried() {
            return Interaction::Thrown { kind: c };
        }
        if let Some(kind) = self.try_lift() {
            return Interaction::Lifted { kind };
        }
        let Some(level) = &self.current_level else {
            return Interaction::Nothing;
        };
        let (px, py) = self.facing_point();
        let inside = |x: i32, y: i32, w: f32, h: f32| {
            px >= x as f32 && px < x as f32 + w && py >= y as f32 && py < y as f32 + h
        };
        // The real 2.22 client opens a chest only from below, facing up (lab
        // capture, `documentation/chest-drawing.md`); a player stopped against
        // a chest from the west, east or north side and pressing A sends
        // nothing.
        let chest = level
            .chests
            .iter()
            .find(|c| {
                self.player_dir == DIR_NORTH
                    && inside(c.x, c.y, 2.0, 2.0)
                    && !c.opened
                    && !self.opened_chests.contains(&(c.x, c.y))
            })
            .cloned();
        if let Some(c) = chest {
            // A chest's sign index points into the level's signs.
            let note = usize::try_from(c.sign_index)
                .ok()
                .filter(|_| c.sign_index >= 0)
                .and_then(|i| level.signs.get(i))
                .map(|s| s.text.clone());
            self.opened_chests.insert((c.x, c.y));
            let got = self.give_item(&c.item);
            self.reading = note;
            return Interaction::Chest { x: c.x, y: c.y, item: c.item, got };
        }
        // Signs are not read with the A key: see `read_sign_ahead`.
        // Nothing to lift or open: A holds on to a wall (a rock too heavy
        // for the glove is one). The grab lasts while A is held.
        if self.try_grab() {
            return Interaction::Grabbed;
        }
        Interaction::Nothing
    }

    /// Start reading the sign the player walks or pushes into. The 2.05
    /// client does this in its walk step, not on the A key: facing up, the
    /// point `(x + 1.5, y + 0.5)` must lie in a sign's 2×1 box (edges
    /// included) and be blocked (a wall or a chest here). Reading starts on
    /// the first page. Returns the sign's text when one opened.
    pub fn read_sign_ahead(&mut self) -> Option<String> {
        if self.reading.is_some() || self.player_dir != DIR_NORTH || self.life.is_dead() {
            return None;
        }
        let (px, py) = (self.player_x + 1.5, self.player_y + 0.5);
        let level = self.current_level.as_ref()?;
        let sign = level.signs.iter().find(|s| {
            let (sx, sy) = (s.x as f32, s.y as f32);
            px >= sx && px <= sx + 2.0 && py >= sy && py <= sy + 1.0
        })?;
        let text = sign.text.clone();
        if !self.blocked_point(px, py) {
            return None;
        }
        self.reading = Some(text.clone());
        Some(text)
    }

    /// Apply a chest item. Returns a short description.
    ///
    /// Item names are the CHEST record's (`greenrupee`, `fullheart`, ...).
    pub fn give_item(&mut self, item: &str) -> String {
        let item = item.to_ascii_lowercase();
        let mut gain = |what: &mut i32, n: i32, label: &str| {
            *what += n;
            format!("{n} {label}")
        };
        match item.as_str() {
            "greenrupee" => gain(&mut self.rupees, 1, "rupee"),
            "bluerupee" => gain(&mut self.rupees, 5, "rupees"),
            "redrupee" => gain(&mut self.rupees, 30, "rupees"),
            "goldrupee" => gain(&mut self.rupees, 100, "rupees"),
            "bombs" => gain(&mut self.bombs, 5, "bombs"),
            "darts" => gain(&mut self.darts, 5, "arrows"),
            "heart" => {
                self.player_health = 100;
                "a heart".into()
            }
            "fullheart" => {
                self.max_hearts = (self.max_hearts + 2).min(40);
                self.player_health = 100;
                "a heart container".into()
            }
            "spinattack" => {
                self.flags.insert("spinattack".into(), true);
                "the spin attack".into()
            }
            "glove1" | "glove2" => {
                // 2.05 and GServer: glove1 raises the power to 2, glove2 to 3.
                self.glove_power = self.glove_power.max(if item == "glove1" { 2 } else { 3 });
                "a glove".into()
            }
            "sword" | "battleaxe" | "lizardsword" | "goldensword" => {
                let p = match item.as_str() {
                    "sword" => 1,
                    "battleaxe" => 2,
                    "lizardsword" => 3,
                    _ => 4,
                };
                self.sword_power = self.sword_power.max(p);
                format!("sword {p}")
            }
            "shield" | "mirrorshield" | "lizardshield" => {
                let p = match item.as_str() {
                    "shield" => 1,
                    "mirrorshield" => 2,
                    _ => 3,
                };
                self.shield_power = self.shield_power.max(p);
                format!("shield {p}")
            }
            other => {
                self.flags.insert(format!("item:{other}"), true);
                other.to_string()
            }
        }
    }

    /// Load a .nw level from disk.
    pub fn load_level<P: AsRef<std::path::Path>>(&mut self, path: P) -> Result<(), String> {
        self.loaded_file = path.as_ref().file_name().map(|n| n.to_string_lossy().into_owned());
        let level = Level::load(path)?;
        self.current_level = Some(level);
        self.opened_chests.clear();
        self.reading = None;
        self.no_pk = false;
        self.clear_throws();
        self.reset_pushpull();
        self.level_serial += 1;
        Ok(())
    }

    /// Load the 4096-byte collision table from a text tiledefs file.
    pub fn load_tiletypes<P: AsRef<std::path::Path>>(&mut self, path: P) -> Result<(), String> {
        self.tiletypes = Some(TileTypes::load(path)?);
        self.refresh_water();
        Ok(())
    }

    /// Load type-0 collision from `arrays.dat` (first array = nowall list).
    /// Tiles in the list get type 0 (passable); everything else gets type 22.
    pub fn load_tiletypes_arrays<P: AsRef<std::path::Path>>(&mut self, path: P) -> Result<(), String> {
        let path = path.as_ref();
        self.tiletypes = Some(TileTypes::load_arrays(path)?);
        // The same file holds the cut and lift tables.
        self.object_tables = std::fs::read(path).ok().and_then(|d| ObjectTables::parse(&d));
        self.refresh_water();
        Ok(())
    }

    /// The point the 2.05 client probes for lifting and for the sword's cut,
    /// from the sprite's top-left, by facing: north `(x + 1.5, y)`, west
    /// `(x - 0.5, y + 2)`, south `(x + 1.5, y + 4)`, east `(x + 3.5, y + 2)`.
    /// That is one tile beyond the middle of the walking body box's leading
    /// edge (see `documentation/walking-collision.md`).
    pub fn probe_point(&self) -> (f32, f32) {
        let (x, y) = (self.player_x, self.player_y);
        match self.player_dir {
            DIR_NORTH => (x + 1.5, y),
            DIR_WEST => (x - 0.5, y + 2.0),
            DIR_EAST => (x + 3.5, y + 2.0),
            _ => (x + 1.5, y + 4.0),
        }
    }

    /// The tile under [`Self::probe_point`].
    fn probe_tile(&self) -> (i32, i32) {
        let (px, py) = self.probe_point();
        (px.floor() as i32, py.floor() as i32)
    }

    /// Lift the object in front of the player, if the glove allows it.
    ///
    /// The glove power `g` allows the first `g + 2` entries of the lift
    /// table (arrays.dat array 11: bush, sign, pot, light stone, black
    /// stone). Players start at 1, which lifts up to the pot; glove1 (2)
    /// adds the light stone and glove2 (3) the black stone. The object's four tiles on the board
    /// are replaced by the table's "left behind" tiles and the player carries
    /// it. Not while swimming, carrying, or dead.
    pub fn try_lift(&mut self) -> Option<usize> {
        if self.life.is_dead() || self.in_water || self.carried.is_some() {
            return None;
        }
        let tables = self.object_tables.as_ref()?;
        let max = usize::from(self.glove_power) + 2;
        let (px, py) = self.probe_tile();
        let (kind, ax, ay) = self.current_level.as_ref()?.find_object(&tables.lift, max, px, py)?;
        let tiles = tables.lift[kind];
        let left = tables.lift_into.get(kind).copied()?;
        self.current_level.as_mut()?.put_object(ax, ay, &left);
        self.note_board_change(ax, ay, 2, 2);
        self.lift_drop(kind, ax, ay);
        self.carried = Some(Carried { kind, tiles });
        self.lift_ticks = Some(0);
        Some(kind)
    }

    /// Throw what the player carries: it leaves from above the head and flies
    /// the way the player faces ([`crate::throwing`]). Returns the lift-table
    /// entry thrown, `None` with empty hands or when dead.
    pub fn throw_carried(&mut self) -> Option<usize> {
        if self.life.is_dead() {
            return None;
        }
        let c = self.carried.take()?;
        // S throws from hiding too.
        self.hidden = false;
        // The 2.05 client puts a walking player back in the idle mode on a
        // throw (A or S): the carry walk stops, and a held arrow starts the
        // walk again on the next step.
        self.walking = false;
        self.thrown.push(crate::throwing::Thrown::launch(c.kind, self.player_x, self.player_y, self.player_dir));
        Some(c.kind)
    }

    /// Another player threw what they carried (`PLO_THROWCARRIED`): the
    /// object leaves from their position `(px, py)` (in this player's
    /// coordinates) facing `dir`, and can hit this player.
    pub fn throw_remote(&mut self, sprite: u16, px: f32, py: f32, dir: u8, thrower: u16) {
        self.thrown.push(crate::throwing::Thrown::launch_remote(sprite, px, py, dir, thrower));
    }

    /// Drop every object in flight, every effect and every floor item (a
    /// level change).
    fn clear_throws(&mut self) {
        self.thrown.clear();
        self.leaps.clear();
        self.grounded.clear();
        self.landings.clear();
        self.throw_ms = 0;
        self.placed_bombs.clear();
        self.blast.clear();
        self.bomb_ms = 0;
        self.arrows.clear();
        self.arrow_ms = 0;
        // The items on the floor belong to the level.
        self.floor_items.clear();
    }

    /// Whether a thrown object's wall probe at `(fx, fy)` ends its flight:
    /// a wall tile, or off the board with no neighbour. Type 20 is the one
    /// wall type objects fly through. (Hits on NPCs, players and chests are
    /// open: see `documentation/throwing.md`.)
    fn throw_blocked(&self, fx: f32, fy: f32) -> bool {
        let Some(level) = &self.current_level else { return false };
        let outside = !(0.0..64.0).contains(&fx) || !(0.0..64.0).contains(&fy);
        let Some(types) = &self.tiletypes else { return outside && self.gmap.is_none() };
        if self.kind_at_point(level, types, fx, fy) != TileKind::Blocking {
            return false;
        }
        let inside = (0.0..64.0).contains(&fx) && (0.0..64.0).contains(&fy);
        if inside {
            if let Some(id) = level.tile_id_at(fx as i32, fy as i32) {
                return types.types.get(usize::from(id)).copied() != Some(crate::throwing::TYPE_THROW_THROUGH);
            }
        }
        true
    }

    /// Whether the water test point is on a water tile.
    fn throw_in_water(&self, fx: f32, fy: f32) -> bool {
        match (&self.current_level, &self.tiletypes) {
            (Some(level), Some(types)) => self.kind_at_point(level, types, fx, fy) == TileKind::Water,
            _ => false,
        }
    }

    /// Advance flights and break effects by `ms` on their 50 ms tick.
    /// Returns true if anything moved (the scene needs a redraw).
    pub fn tick_throw(&mut self, ms: u64) -> bool {
        self.tick_throw_with(ms, &|_, _| None)
    }

    /// [`Self::tick_throw`] with an NPC test: `npc_at(x, y)` names the
    /// first NPC whose box holds the point, if any. An object that hits one
    /// stops, and the NPC is queued for [`Self::take_pelts`].
    pub fn tick_throw_with(&mut self, ms: u64, npc_at: &dyn Fn(f32, f32) -> Option<usize>) -> bool {
        if self.thrown.is_empty() && self.leaps.is_empty() {
            self.throw_ms = 0;
            return false;
        }
        self.throw_ms += ms;
        let mut moved = false;
        while self.throw_ms >= crate::throwing::TICK_MS {
            self.throw_ms -= crate::throwing::TICK_MS;
            self.throw_tick(npc_at);
            moved = true;
        }
        moved
    }

    /// One flight tick, in the 2.05 order: each object steps, then the wall
    /// test at its probe; if the way is clear, the NPC test at the same
    /// point, then the player test. A hit on either ends the flight like a
    /// wall does.
    fn throw_tick(&mut self, npc_at: &dyn Fn(f32, f32) -> Option<usize>) {
        use crate::throwing as t;
        // Effects: one frame per tick, gone after the last.
        let tables = self.object_tables.as_ref();
        self.leaps.retain_mut(|l| {
            l.frame += 1;
            (l.frame as usize) < t::leap_frames(tables, l.kind).len()
        });
        for mut o in std::mem::take(&mut self.thrown) {
            o.step();
            let (wx, wy) = o.wall_probe();
            let mut stop = self.throw_blocked(wx, wy);
            if !stop {
                if let Some(npc) = npc_at(wx, wy) {
                    self.pelts.push(npc);
                    stop = true;
                }
            }
            if !stop && !o.own && self.victim().state == crate::combat::Vulnerability::Hurtable && o.hits_player(self.player_x, self.player_y) {
                self.thrown_hit(&o);
                stop = true;
            }
            if o.ticks_left > 0 && !stop {
                self.thrown.push(o);
                continue;
            }
            if t::stays_on_ground(o.sprite) {
                if let Some(sprite) = o.sprite {
                    self.grounded.push(t::Grounded { sprite, x: o.x, y: o.y });
                }
                self.landings.push(Landing { kind: o.kind, effect: None, x: o.x, y: o.y });
                continue;
            }
            let (ox, oy) = o.water_probe();
            let kind = t::leap_kind(o.sprite, self.throw_in_water(ox, oy));
            self.spawn_leap(kind, o.x, o.ground);
            self.landings.push(Landing { kind: o.kind, effect: Some(kind), x: o.x, y: o.ground });
        }
    }

    /// Another player's object hit this player. On a level that says
    /// `noplayerkilling` it hurts nobody (it still stops); otherwise it
    /// takes [`crate::throwing::throw_damage_half`], starts hurt mode with a
    /// push along its flight, and names the thrower for the claim a death
    /// sends.
    fn thrown_hit(&mut self, o: &crate::throwing::Thrown) {
        if self.no_pk && o.thrower.is_some() {
            return;
        }
        let half = crate::throwing::throw_damage_half(self.hearts_half());
        if self.damage_half(half, o.thrower) {
            self.log_hurt("thrown object", o.thrower, half);
        }
        self.combat.start_hurt_knock(Some(crate::throwing::throw_knock(o.dir)));
        self.throw_hurt = true;
    }

    /// Note that the player changed the `w` x `h` tiles at `(x, y)` of the
    /// current level (a lift, a cut, a moved stone block), clipped to the
    /// board, with the tiles as they are now.
    pub(crate) fn note_board_change(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let Some(level) = self.current_level.as_ref() else { return };
        let (x0, y0) = (x.max(0), y.max(0));
        let (x1, y1) = ((x + w).min(64), (y + h).min(64));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let tiles = (y0..y1).flat_map(|ty| (x0..x1).map(move |tx| (tx, ty))).map(|(tx, ty)| level.tile_id_at(tx, ty).unwrap_or(0)).collect();
        self.board_changes.push(BoardChange { x: x0, y: y0, width: x1 - x0, height: y1 - y0, tiles });
    }

    /// The board areas the player changed since the last call. Online each
    /// goes to the server as a `PLI_BOARDMODIFY`, which shows the change to
    /// the other players in the level.
    pub fn take_board_changes(&mut self) -> Vec<BoardChange> {
        std::mem::take(&mut self.board_changes)
    }

    /// NPCs hit by thrown objects since the last call (for `waspelt`).
    pub fn take_pelts(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.pelts)
    }

    /// True once if another player's thrown object hurt the player since the
    /// last call: the caller sends the hurt sprite.
    pub fn take_throw_hurt(&mut self) -> bool {
        std::mem::take(&mut self.throw_hurt)
    }

    /// The landings since the last call.
    pub fn take_landings(&mut self) -> Vec<Landing> {
        std::mem::take(&mut self.landings)
    }

    /// `putbomb power,x,y`: a bomb with a 60-tick fuse at `(x, y)`, or the
    /// splash when water is under `(x + 1, y + 1)`. Either way a
    /// `PLI_BOMBADD` is queued; `from_weapon` marks a weapon's bomb.
    pub fn put_bomb(&mut self, power: u8, x: f32, y: f32, from_weapon: bool) {
        use crate::bombs as b;
        if self.throw_in_water(x + 1.0, y + 1.0) {
            self.spawn_leap(crate::throwing::LEAP_SPLASH, x, y);
        } else {
            self.placed_bombs.push(b::Bomb::new(x, y, power, b::FUSE_TICKS));
        }
        self.bomb_sends.push(b::bomb_add_message(x, y, power, from_weapon, b::FUSE_TICKS - b::SEND_LEAD));
    }

    /// The `PLI_BOMBADD` messages since the last call.
    pub fn take_bomb_sends(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.bomb_sends)
    }

    /// Advance fuses and explosions by `ms` on their 50 ms tick. Returns
    /// true if anything changed (the scene needs a redraw).
    pub fn tick_bombs(&mut self, ms: u64) -> bool {
        if self.placed_bombs.is_empty() && self.blast.is_empty() {
            self.bomb_ms = 0;
            return false;
        }
        self.bomb_ms += ms;
        let mut changed = false;
        while self.bomb_ms >= crate::throwing::TICK_MS {
            self.bomb_ms -= crate::throwing::TICK_MS;
            self.bomb_tick();
            changed = true;
        }
        changed
    }

    /// One tick: the pieces hit and age, then the fuses burn; a bomb that
    /// goes off plays its sound and leaves its pieces.
    fn bomb_tick(&mut self) {
        for p in std::mem::take(&mut self.blast) {
            let mut p = p;
            if self.victim().state == crate::combat::Vulnerability::Hurtable && p.hits_player(self.player_x, self.player_y) {
                let owner = self.owner_named(p.owner);
                if !(self.no_pk && owner.is_some()) {
                    if self.damage_half(p.damage_half(), owner) {
                        self.log_hurt("explosion", owner, p.damage_half());
                    }
                    self.combat.start_hurt_knock(Some(crate::throwing::throw_knock(p.knock_dir())));
                    self.throw_hurt = true;
                }
            }
            if p.tick() {
                self.blast.push(p);
            }
        }
        for mut bomb in std::mem::take(&mut self.placed_bombs) {
            if !bomb.tick() {
                self.placed_bombs.push(bomb);
                continue;
            }
            self.sound_events.push(SoundEvent { file: bomb.sound().into(), x: bomb.x + 1.0, y: bomb.y + 1.0 });
            let room = crate::bombs::MAX_PIECES.saturating_sub(self.blast.len());
            self.blast.extend(bomb.pieces().into_iter().take(room));
        }
    }

    /// The stderr line for a hit that is not a sword or hit packet, so a lab
    /// log shows where the hearts went.
    fn log_hurt(&self, source: &str, owner: Option<u16>, half: i32) {
        eprintln!("hurt by {source} (owner {owner:?}): {half} half hearts, now {}", self.hearts_half());
    }

    /// The owner a hit names: `owner` while that player is in the server's
    /// player list, else nobody.
    fn owner_named(&self, owner: Option<u16>) -> Option<u16> {
        crate::bombs::player_owner(owner, |id| self.present_players.contains(&id))
    }

    /// Another player's bomb (`PLO_BOMBADD`), in this level's coordinates:
    /// `putbomb` with no send, the fuse it carries, and the sender as the
    /// owner when bit 8 says so.
    pub fn receive_bomb(&mut self, b: &crate::bombs::BombAdd, sender: Option<u16>) {
        if self.throw_in_water(b.x + 1.0, b.y + 1.0) {
            self.spawn_leap(crate::throwing::LEAP_SPLASH, b.x, b.y);
            return;
        }
        let mut bomb = crate::bombs::Bomb::new(b.x, b.y, b.power, b.fuse);
        bomb.owner = sender.filter(|_| b.owned);
        self.placed_bombs.push(bomb);
    }

    /// `PLO_BOMBDEL`: the bomb at exactly `(x, y)` is gone.
    pub fn remove_bomb(&mut self, x: f32, y: f32) {
        self.placed_bombs.retain(|b| (b.x, b.y) != (x, y));
    }

    /// Another player's explosion (`PLO_EXPLOSION`), in this level's
    /// coordinates: its sound and one cross. The sound names are taken to be
    /// a bomb's (inferred).
    pub fn receive_explosion(&mut self, e: &crate::bombs::Explosion, sender: Option<u16>) {
        use crate::bombs as b;
        self.sound_events.push(SoundEvent { file: b::sound(e.power).into(), x: e.x + 1.0, y: e.y + 1.0 });
        let room = b::MAX_PIECES.saturating_sub(self.blast.len());
        let owner = sender.filter(|_| e.owned);
        self.blast.extend(b::explosion_pieces(e.radius, e.x, e.y, e.power, owner).into_iter().take(room));
    }

    /// Another player's shot (`PLO_ARROWADD`), in this level's coordinates.
    pub fn receive_arrow(&mut self, a: &crate::arrows::ArrowAdd, sender: Option<u16>) {
        self.arrows.push(crate::arrows::Arrow::received(a, sender, self.no_pk));
    }

    /// A shot hit this player: the hearts and the push unless it is a
    /// player's on a `noplayerkilling` level, then its blast if it has one.
    fn shot_hit(&mut self, a: &crate::arrows::Arrow, w: i32, h: i32) {
        let owner = self.owner_named(a.owner);
        if !(self.no_pk && owner.is_some()) {
            if self.damage_half(a.damage_half(), owner) {
                let name = ["arrow", "fireball", "fireblast", "nuke"].get(usize::from(a.kind)).copied().unwrap_or("shot");
                self.log_hurt(name, owner, a.damage_half());
            }
            self.combat.start_hurt_knock(Some(crate::throwing::throw_knock(a.dir())));
            self.throw_hurt = true;
        }
        if let Some((bx, by)) = a.blast_at(w, h) {
            let room = crate::bombs::MAX_PIECES.saturating_sub(self.blast.len());
            self.blast.extend(crate::bombs::explosion_pieces(2, bx, by, 1, a.owner).into_iter().take(room));
        }
    }

    /// `shootarrow dir` from a shooter `w` x `h` tiles at `(x, y)`: the
    /// arrow, `arrow.wav`, and a queued `PLI_ARROWADD`. `from_weapon`
    /// marks a weapon's arrow.
    pub fn shoot_arrow(&mut self, x: f32, y: f32, w: f32, h: f32, dir: u8, from_weapon: bool) {
        use crate::arrows as a;
        let (sx, sy) = a::start(x, y, w, h, dir);
        let mut arrow = a::Arrow::new(sx, sy, dir);
        arrow.harmless = from_weapon;
        self.sound_events.push(SoundEvent { file: "arrow.wav".into(), x: sx + 0.5, y: sy + 0.5 });
        self.arrow_sends.push(a::arrow_add_message(sx, sy, dir, from_weapon, arrow.sprite, 0));
        self.arrows.push(arrow);
    }

    /// The `PLI_ARROWADD` messages since the last call.
    pub fn take_arrow_sends(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.arrow_sends)
    }

    /// The NPCs an arrow hit since the last call (they get `wasshot`).
    pub fn take_shots(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.shots)
    }

    /// Advance arrows by `ms` on their 50 ms tick. An arrow whose middle
    /// is on an NPC (`npc_at`) shoots it and turns round. Returns true if
    /// anything moved.
    pub fn tick_arrows(&mut self, ms: u64, npc_at: &dyn Fn(f32, f32) -> Option<usize>) -> bool {
        use crate::arrows::Step;
        if self.arrows.is_empty() {
            self.arrow_ms = 0;
            return false;
        }
        self.arrow_ms += ms;
        let mut moved = false;
        while self.arrow_ms >= crate::throwing::TICK_MS {
            self.arrow_ms -= crate::throwing::TICK_MS;
            moved = true;
            let tables = self.object_tables.as_ref();
            let mut kept = Vec::new();
            let mut hits = Vec::new();
            let mut hurt_now = false;
            for mut a in std::mem::take(&mut self.arrows) {
                if a.step() == Step::Gone {
                    continue;
                }
                // The player is hit only by a shot whose size the sprite
                // table gives, and once hurt (by an earlier shot of this
                // tick too) no more.
                if let Some((_, _, w, h)) = crate::throwing::sprite_rect(tables, a.sprite) {
                    if !hurt_now && self.victim().state == crate::combat::Vulnerability::Hurtable && a.hits_player(w, h, self.player_x, self.player_y) {
                        hurt_now = !(self.no_pk && self.owner_named(a.owner).is_some());
                        hits.push((a, w, h));
                        continue;
                    }
                }
                if a.reflect < 0 {
                    let (w, h) = crate::throwing::sprite_rect(tables, a.sprite).map_or((16, 16), |(_, _, w, h)| (w, h));
                    let (mx, my) = (a.x + w as f32 / 32.0, a.y + h as f32 / 32.0);
                    if let Some(npc) = npc_at(mx, my) {
                        self.shots.push(npc);
                        a.bounce();
                    }
                }
                kept.push(a);
            }
            self.arrows = kept;
            for (a, w, h) in hits {
                self.shot_hit(&a, w, h);
            }
        }
        moved
    }

    /// The sword's cut: the cuttable object in front of the player (the same
    /// probe point as the lift) is replaced by what it leaves behind.
    /// Returns the table entry that was cut.
    pub fn try_cut(&mut self) -> Option<usize> {
        let tables = self.object_tables.as_ref()?;
        let (px, py) = self.probe_tile();
        let (kind, ax, ay) = self.current_level.as_ref()?.find_object(&tables.cut, tables.cut.len(), px, py)?;
        let left = tables.cut_into.get(kind).copied()?;
        self.current_level.as_mut()?.put_object(ax, ay, &left);
        self.note_board_change(ax, ay, 2, 2);
        // The cut plays the break effect of its table entry (bush: the leaves
        // of the thrown bush), anchored on the object's top-left tile.
        // Which effect each cut entry plays is inferred, see
        // `documentation/cut-and-lift.md`.
        self.spawn_leap(crate::throwing::cut_leap_kind(kind), ax as f32, ay as f32);
        self.cut_drop(ax, ay);
        Some(kind)
    }

    /// Start a break effect at `(x, y)` and its sound, as the 2.05 client
    /// does for every leap: `water.wav` for the splash, `crush.wav` for the
    /// others, played at `(x + 1, y + 1)`.
    pub fn spawn_leap(&mut self, kind: usize, x: f32, y: f32) {
        self.leaps.push(crate::throwing::Leap { kind, x, y, frame: -1 });
        let file = if kind == crate::throwing::LEAP_SPLASH { "water.wav" } else { "crush.wav" };
        self.sound_events.push(SoundEvent { file: file.into(), x: x + 1.0, y: y + 1.0 });
    }

    /// Current player state as a protocol update.
    pub fn player_update(&self) -> PlayerUpdate {
        PlayerUpdate {
            x: self.player_x,
            y: self.player_y,
            dir: self.player_dir,
        }
    }

    /// Apply a GS1 command against this offline session.
    ///
    /// Handles the high-frequency commands from the scraped set:
    /// `hide`/`show` (NPC visibility), `warp` (local position),
    /// `setlevel` (noted), `hurt` (health), `say`/`message` (last message),
    /// `set`/`unset` (flags).
    /// Returns true if the command was recognized and applied.
    pub fn handle_command(&mut self, name: &str, args: &[crate::scripting::Value]) -> bool {
        match name {
            "hide" => {
                if let Some(level) = self.current_level.as_mut() {
                    for npc in &mut level.npcs {
                        npc.visible = false;
                    }
                }
                true
            }
            "show" => {
                if let Some(level) = self.current_level.as_mut() {
                    for npc in &mut level.npcs {
                        npc.visible = true;
                    }
                }
                true
            }
            "warp" => {
                if args.len() >= 2 {
                    if let (crate::scripting::Value::Num(x), crate::scripting::Value::Num(y)) =
                        (&args[0], &args[1])
                    {
                        self.player_x = *x;
                        self.player_y = *y;
                    }
                }
                true
            }
            "setlevel" => {
                // Level switch is recorded; actual load is done by the caller
                // once a level path or name is resolved.
                true
            }
            "hurt" => {
                let amt = args
                    .first()
                    .and_then(|v| match v {
                        crate::scripting::Value::Num(n) => Some(*n as i32),
                        _ => None,
                    })
                    .unwrap_or(1);
                self.player_health = (self.player_health - amt).max(0);
                true
            }
            "say" | "message" => {
                if let Some(crate::scripting::Value::Str(s)) = args.first() {
                    self.last_message = Some(s.clone());
                }
                true
            }
            "set" => {
                if let Some(crate::scripting::Value::Str(name)) = args.first() {
                    self.flags.insert(name.clone(), true);
                }
                true
            }
            "unset" => {
                if let Some(crate::scripting::Value::Str(name)) = args.first() {
                    self.flags.insert(name.clone(), false);
                }
                true
            }
            "setstring" => {
                if args.len() >= 2 {
                    if let (crate::scripting::Value::Str(name), crate::scripting::Value::Str(val)) =
                        (&args[0], &args[1])
                    {
                        self.strings.insert(name.clone(), val.clone());
                    }
                }
                true
            }
            "addstring" => {
                if args.len() >= 2 {
                    if let (crate::scripting::Value::Str(name), crate::scripting::Value::Str(val)) =
                        (&args[0], &args[1])
                    {
                        let entry = self.strings.entry(name.clone()).or_default();
                        entry.push_str(val);
                    }
                }
                true
            }
            "setplayerprop" => {
                if args.len() >= 2 {
                    if let (crate::scripting::Value::Str(name), crate::scripting::Value::Str(val)) =
                        (&args[0], &args[1])
                    {
                        self.props.insert(name.clone(), val.clone());
                    }
                }
                true
            }
            "setimg" | "setshape" => {
                // Image/shape change is recorded as a prop for now.
                if let Some(crate::scripting::Value::Str(name)) = args.first() {
                    self.props.insert("image".into(), name.clone());
                }
                true
            }
            "move" => {
                // Relative move: args are dx, dy.
                if args.len() >= 2 {
                    if let (crate::scripting::Value::Num(dx), crate::scripting::Value::Num(dy)) =
                        (&args[0], &args[1])
                    {
                        self.player_x += *dx;
                        self.player_y += *dy;
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Place a named stamp from an object set onto the current level.
    /// Returns true if the stamp was found and written.
    pub fn place_named_stamp(&mut self, set: &crate::assets::ObjectSet, name: &str, x: i32, y: i32) -> bool {
        if let Some(stamp) = set.find(name) {
            if let Some(level) = self.current_level.as_mut() {
                level.place_stamp(stamp, x, y);
                return true;
            }
        }
        false
    }
    /// If the player's feet are in a link, record the destination in
    /// `pending_link`. Returns true if a link was entered.
    ///
    /// A link does not fire while a level is loading or a warp is
    /// pending, before the player has moved since arriving, or while the
    /// player is still inside a link that contained the arrival tile.
    pub fn check_links(&mut self) -> bool {
        self.pending_link = None;
        if self.warp_pending.is_some() || !self.armed {
            return false;
        }
        let (fx, fy) = self.foot();
        let (px, py) = (fx as i32, fy as i32);
        let Some(level) = &self.current_level else {
            return false;
        };
        let (px0, py0) = (self.player_x, self.player_y);
        let b = LINK_BOX[usize::from(self.player_dir & 3)];
        let boxes = self.link_boxes;
        let hit = |l: &Link| {
            if boxes {
                l.overlaps(px0 + b.0, py0 + b.1, px0 + b.2, py0 + b.3)
            } else {
                l.contains(px, py)
            }
        };
        if let Some(t) = self.arrival_tile {
            // Still on the way out of the arrival link?
            if level.links.iter().any(|l| l.contains(t.0, t.1) && hit(l)) {
                return false;
            }
            self.arrival_tile = None;
        }
        let here = self.loaded_file.as_deref().unwrap_or("");
        let big_map = self.big_map.as_ref();
        let (x, y) = (self.player_x, self.player_y);
        self.pending_link = level.links.iter().find(|l| hit(l)).map(|link| {
            let (nx, ny) = big_map
                .and_then(|m| edge_link_arrival(m, here, link, x, y))
                .unwrap_or((link.new_x.unwrap_or(x + link.x_offset), link.new_y.unwrap_or(y + link.y_offset)));
            (link.dest.clone(), nx, ny)
        });
        self.pending_link.is_some()
    }

    /// The player is put at `(x, y)` in a level that is now loaded: the
    /// end of a warp, online or offline. Ends a pending warp, switches
    /// developer mode off (returns true if it was on), and holds links
    /// back until the player moves (see [`Self::check_links`]).
    pub fn arrive(&mut self, x: f32, y: f32) -> bool {
        self.player_x = x;
        self.player_y = y;
        self.warp_pending = None;
        self.pending_link = None;
        self.armed = false;
        self.reset_pushpull();
        let (fx, fy) = self.foot();
        self.arrival_tile = Some((fx as i32, fy as i32));
        let dev_was_on = self.dev.reset();
        self.refresh_water();
        dev_was_on
    }

    /// Ask for a warp: from now until [`Self::arrive`] or
    /// [`Self::cancel_warp`] no link fires and the player stays put.
    pub fn begin_warp(&mut self, dest: &str) {
        self.warp_pending = Some(dest.to_string());
        self.pending_link = None;
    }

    /// Give up a pending warp (the server refused or never answered).
    pub fn cancel_warp(&mut self) {
        self.warp_pending = None;
    }

    /// Recompute `in_water` and `sitting` from the feet.
    ///
    /// Going into the water or out of it splashes, as in the 2.05 client:
    /// one splash where the player stands, three when the player did not
    /// walk in (a warp, a knock), and one on the way out.
    pub fn refresh_water(&mut self) {
        let (fx, fy) = self.foot();
        let was = self.in_water;
        self.in_water = match (&self.current_level, &self.tiletypes) {
            // The liquid list holds lava too: the 2.05 client swims in both.
            (Some(level), Some(types)) => matches!(self.kind_at_point(level, types, fx, fy), TileKind::Water | TileKind::Lava),
            _ => false,
        };
        if self.in_water != was {
            // The stroke starts from its first step in the water; on land the
            // walk comes back at its first step.
            self.swim_ticks = 0;
            if let Some(g) = self.swim_gani.as_mut() {
                g.reset();
            }
            if !self.in_water {
                if let Some(g) = self.walk_gani.as_mut() {
                    g.reset();
                }
            }
            let n = if self.in_water && !self.walking { 3 } else { 1 };
            for _ in 0..n {
                self.spawn_leap(crate::throwing::LEAP_SPLASH, self.player_x, self.player_y);
            }
        }
        self.refresh_rest();
    }

    /// True if the tile of the current level under a point passes `test`.
    /// Off the board nothing does.
    fn tile_is(&self, fx: f32, fy: f32, test: fn(&TileTypes, u16) -> bool) -> bool {
        let (Some(level), Some(types)) = (&self.current_level, &self.tiletypes) else { return false };
        if !(0.0..64.0).contains(&fx) || !(0.0..64.0).contains(&fy) {
            return false;
        }
        level.tile_id_at(fx as i32, fy as i32).is_some_and(|id| test(types, id))
    }

    /// Sit on a chair or lie down in a bed, as the 2.05 client decides it.
    ///
    /// The client runs this test on level entry, when an arrow key is let
    /// go, and on every tick the player walked or a script moved them, so
    /// after the walk has set the walk pose: on a chair the sit pose wins,
    /// even while walking. Not while swimming or carrying (the client also
    /// skips it on a horse).
    ///
    /// * Chair: the point `(x + 1.5, y + 2)` is type 3.
    /// * Otherwise bed: `(x + 1, y)` or `(x + 1, y + 0.5)` is type 4, and
    ///   `(x + 1.8, y)` or `(x + 1.8, y + 0.5)` is too. The client then
    ///   puts the head on the bed's top row: y moves down half a tile when
    ///   neither point at `y` is on the bed top, and is then cut to a whole
    ///   tile (`floor(y + 0.0001)`). x is left alone. Walking up or down in
    ///   a bed therefore snaps straight back; the way out is to the side.
    pub fn refresh_rest(&mut self) {
        self.sitting = false;
        self.sleeping = false;
        if self.in_water || self.carried.is_some() {
            return;
        }
        let (x, y) = (self.player_x, self.player_y);
        if self.tile_is(x + SIT_PROBE.0, y + SIT_PROBE.1, TileTypes::is_chair) {
            self.sitting = true;
            return;
        }
        let bed_top = |s: &Self, dx: f32, dy: f32| s.tile_is(x + dx, y + dy, TileTypes::is_bed_top);
        if !BED_PROBE_DX.iter().all(|&dx| BED_PROBE_DY.iter().any(|&dy| bed_top(self, dx, dy))) {
            return;
        }
        self.sleeping = true;
        let mut ny = y;
        if !BED_PROBE_DX.iter().any(|&dx| bed_top(self, dx, 0.0)) {
            ny += 0.5;
        }
        self.player_y = (ny + 0.0001).floor();
    }

    /// Time between steps now (swimming and walking are the same pace).
    pub fn step_interval(&self) -> std::time::Duration {
        std::time::Duration::from_millis(if self.in_water { SWIM_STEP_MS } else { WALK_STEP_MS })
    }

    /// `Some(swimming)` when the swimming state changed since the last
    /// call, so the caller can tell the server once per change.
    pub fn take_swim_change(&mut self) -> Option<bool> {
        if self.in_water != self.swim_sent {
            self.swim_sent = self.in_water;
            Some(self.in_water)
        } else {
            None
        }
    }

    /// The kind of tile under a point given relative to the current cell.
    /// On a gmap a point past the edge is in the next cell's level, and in
    /// no level (blocking) when that cell has none loaded.
    fn kind_at_point(&self, level: &Level, types: &TileTypes, fx: f32, fy: f32) -> TileKind {
        let Some(w) = &self.gmap else {
            return level.kind_at(types, fx, fy);
        };
        let side = crate::gmap::CELL_TILES as f32;
        let (kx, ky) = ((fx / side).floor() as i32, (fy / side).floor() as i32);
        if (kx, ky) == (0, 0) {
            return level.kind_at(types, fx, fy);
        }
        match w.levels.get(&(w.cell.0 + kx, w.cell.1 + ky)) {
            Some(l) => l.kind_at(types, fx - kx as f32 * side, fy - ky as f32 * side),
            None => TileKind::Blocking,
        }
    }

    /// Whether `(fx, fy)` is off the board with no level there: the wall
    /// test when there are no tile types (`--no-builtin` with nothing
    /// cached). The 2.22 client's wall test checks the bounds before the
    /// tile, so the edge is a wall whatever the tiles are; on a gmap a loaded
    /// neighbour is open ground.
    fn off_board(&self, fx: f32, fy: f32) -> bool {
        let side = crate::gmap::CELL_TILES as f32;
        let (kx, ky) = ((fx / side).floor() as i32, (fy / side).floor() as i32);
        if (kx, ky) == (0, 0) {
            return false;
        }
        match &self.gmap {
            Some(w) => !w.levels.contains_key(&(w.cell.0 + kx, w.cell.1 + ky)),
            None => true,
        }
    }

    /// Whether a sprite position (current-cell tiles) is a legal place to be
    /// as far as the gmap goes: inside the cell, or across an edge into a
    /// loaded level. A step that would cross both edges at once is refused
    /// (it was never seen from a real client; the walk slides along one
    /// axis instead).
    fn gmap_step_ok(&self, nx: f32, ny: f32) -> bool {
        let Some(w) = &self.gmap else { return true };
        match w.map.resolve(w.cell, (nx, ny)) {
            Resolved::Inside => true,
            Resolved::NoLevel => false,
            Resolved::Crossed(c) => !(c.x_changed() && c.y_changed()) && w.levels.contains_key(&c.to),
        }
    }

    /// Start a gmap walk: the grid, and the cell the player is in.
    /// `player_x`/`player_y` must be local to `cell`.
    pub fn enter_gmap(&mut self, name: String, map: crate::gmap::Gmap, cell: (i32, i32)) {
        self.gmap = Some(GmapWorld::new(name, map, cell));
    }

    /// Leave the gmap (a plain level warp).
    pub fn leave_gmap(&mut self) {
        self.gmap = None;
    }

    /// The edge crossing the last step made, if any. The caller sends the
    /// crossing frame ([`crate::gmap::crossing_frame`]); the player is
    /// already in the new cell with the wrapped position.
    pub fn take_gmap_crossing(&mut self) -> Option<Crossing> {
        self.gmap.as_mut()?.crossing.take()
    }

    /// Move into the cell `c` names: the loaded level there becomes the
    /// current one, the old one is kept as a neighbour.
    fn cross_to(&mut self, c: Crossing) {
        let Some(w) = self.gmap.as_mut() else { return };
        if let Some(old) = self.current_level.take() {
            w.levels.insert(c.from, old);
        }
        self.current_level = w.levels.remove(&c.to);
        w.cell = c.to;
        w.crossing = Some(c);
        // A new cell is a new level to the script engine too.
        self.clear_throws();
        self.reset_pushpull();
        self.level_serial += 1;
        self.player_x = c.local_x;
        self.player_y = c.local_y;
        self.opened_chests.clear();
        self.pending_link = None;
    }

    /// True if the walking test finds the point `(px, py)` (tiles,
    /// current-cell coordinates) blocked: a wall tile (or off the board), or
    /// a chest record covering it (2 x 2 tiles, edges included, open or
    /// not). The tile is `floor(coordinate + 0.0001)`, as in the real client.
    /// Water and lava are open. Developer mode skips the test.
    fn walk_blocked(&self, px: f64, py: f64) -> bool {
        self.walk_blocked_except(px, py, None)
    }

    /// [`Self::walk_blocked`], with NPC `except` left out (an NPC that is
    /// being moved does not stop itself).
    fn walk_blocked_except(&self, px: f64, py: f64, except: Option<usize>) -> bool {
        if self.dev.wallhack() {
            return false;
        }
        if self.npc_blocks_at(px as f32, py as f32, except) {
            return true;
        }
        let (fx, fy) = ((px + WALK_FLOOR_EPS) as f32, (py + WALK_FLOOR_EPS) as f32);
        match (&self.current_level, &self.tiletypes) {
            (Some(level), Some(types)) if self.kind_at_point(level, types, fx, fy) == TileKind::Blocking => return true,
            (Some(_), None) if self.off_board(fx, fy) => return true,
            _ => {}
        }
        match &self.current_level {
            Some(level) => level.chests.iter().any(|c| {
                let (cx, cy) = (c.x as f64, c.y as f64);
                px >= cx && px <= cx + 2.0 && py >= cy && py <= cy + 2.0
            }),
            None => false,
        }
    }

    /// How many of the nine points inside the body box (x + 0.525, 1.5 and
    /// 2.475 across, y + 1.025, 2.0 and 2.975 down) are blocked. Zero for any
    /// position walking reaches. Used only to let a body that a warp put
    /// inside a wall walk out ([`Self::walk_step`]).
    fn body_overlap(&self, x: f64, y: f64) -> usize {
        const XS: [f64; 3] = [0.525, 1.5, 2.475];
        const YS: [f64; 3] = [1.025, 2.0, 2.975];
        XS.iter().flat_map(|&px| YS.iter().map(move |&py| (px, py))).filter(|&(px, py)| self.walk_blocked(x + px, y + py)).count()
    }

    /// One arrow key of the real client's walking routine: the sprite at
    /// `(x, y)` takes the step `(mx, my)` facing `dir` (0 up, 1 left, 2
    /// down, 3 right). Returns the position it ends at.
    ///
    /// 1. The six [`WALK_PROBES`] of the facing are read at the stepped
    ///    position. The step is kept if probes 0 to 3 are free.
    /// 2. Otherwise two corner rules may add a [`WALK_NUDGE`] of 0.1 tile
    ///    (probe 0 blocked, 2 to 4 free: nudge one way; 0 and 1 free, 3
    ///    blocked, 5 free: the other way). Then the step is taken back, and
    ///    the position along the facing is pulled flush to the tile line:
    ///    up to `floor(y)`, left to `floor(x + 0.5) - 0.5`, down to
    ///    `floor(y + 1)`, right to `floor(x + 1.5) - 0.5` (only if it was not
    ///    on the line already). The right and bottom probes sit one tile
    ///    line beyond the body, so this is what lets the body close the last
    ///    half tile to a wall on its right or below.
    /// 3. In every case the position then snaps to a tile line it is within
    ///    0.0625 of (y on whole numbers, x on whole numbers minus 0.5).
    ///
    /// A body already overlapping a wall (put there by a warp) may still take
    /// a step that leaves it less inside.
    fn walk_step(&self, x: f64, y: f64, dir: usize, mx: f64, my: f64) -> (f64, f64, StepResult) {
        let (nx, ny) = (x + mx, y + my);
        if !self.gmap_step_ok(nx as f32, ny as f32) {
            return (x, y, StepResult::Refused { nudged: false });
        }
        let hit: Vec<bool> = WALK_PROBES[dir].iter().map(|&(px, py)| self.walk_blocked(nx + px, ny + py)).collect();
        let (mut fx, mut fy) = (nx, ny);
        let refused = hit[0] || hit[1] || hit[2] || hit[3];
        if refused {
            let here = self.body_overlap(x, y);
            if here > 0 && self.body_overlap(nx, ny) < here {
                // Walking out of a wall: no nudge, no snap.
                return (nx, ny, StepResult::Free);
            }
        }
        let mut nudged = false;
        if refused {
            if hit[0] && !hit[2] && !hit[3] && !hit[4] {
                fx += WALK_NUDGE[dir][0].0;
                fy += WALK_NUDGE[dir][0].1;
                nudged = true;
            } else if !hit[0] && !hit[1] && hit[3] && !hit[5] {
                fx += WALK_NUDGE[dir][1].0;
                fy += WALK_NUDGE[dir][1].1;
                nudged = true;
            }
            fx -= mx;
            fy -= my;
            match dir {
                0 if walk_frac(fy) > 0.0 => fy = walk_floor(fy),
                1 if walk_frac(fx + 0.5) > 0.0 => fx = walk_floor(fx + 0.5) - 0.5,
                2 if walk_frac(fy) > 0.0 => fy = walk_floor(fy + 1.0),
                3 if walk_frac(fx + 0.5) > 0.0 => fx = walk_floor(fx + 1.5) - 0.5,
                _ => {}
            }
        }
        let f = fy - walk_floor(fy);
        if !(WALK_SNAP_EDGE..=1.0 - WALK_SNAP_EDGE).contains(&f) {
            fy = walk_floor(fy + WALK_SNAP_EDGE);
        }
        let t = fx + 0.5;
        let f = t - walk_floor(t);
        if !(WALK_SNAP_EDGE..=1.0 - WALK_SNAP_EDGE).contains(&f) {
            fx = walk_floor(t + WALK_SNAP_EDGE) - 0.5;
        }
        (fx, fy, if refused { StepResult::Refused { nudged } } else { StepResult::Free })
    }

    /// Move the player by (dx, dy) as the real client's walking routine
    /// does ([`Self::walk_step`]): each arrow key on its own, in the order up,
    /// left, down, right, each tested at the position the earlier ones left,
    /// so a diagonal is two steps and slides along a wall. A refused step can
    /// still move the player: the 0.1-tile corner nudge round a corner, and
    /// the snap flush to the wall. Facing comes from the dominant axis.
    /// Nothing moves while a warp is pending.
    ///
    /// Each step also counts towards a push or a ledge jump, and while A
    /// holds a wall the arrows only pull (see [`pushpull`]); in the air they
    /// do nothing.
    /// Returns the emitted `PlayerUpdate` if the position changed.
    pub fn try_move(&mut self, dx: f32, dy: f32) -> Option<PlayerUpdate> {
        // An open sign takes the arrows (2.05): no step and no turn.
        if dx == 0.0 && dy == 0.0 || self.life.is_dead() || self.jumping() || self.reading.is_some() {
            return None;
        }
        // Hurt: the arrows are ignored for the first ticks; after that a
        // held arrow walks again and the walk replaces the hurt pose.
        if let Some(ms) = self.combat.hurt_elapsed_ms() {
            if ms / crate::pose::GANI_STEP_MS < crate::pose::HURT_WALK_AFTER_TICKS {
                return None;
            }
            self.hurt_walked = true;
        }
        if self.pushpull.grab.is_some() {
            self.grab_arrow(dx, dy);
            return None;
        }
        // Facing from dominant axis.
        if dx.abs() > dy.abs() {
            self.player_dir = if dx > 0.0 { DIR_EAST } else { DIR_WEST };
        } else {
            self.player_dir = if dy > 0.0 { DIR_SOUTH } else { DIR_NORTH };
        }
        if self.warp_pending.is_some() {
            return None;
        }

        let (x0, y0) = (self.player_x as f64, self.player_y as f64);
        let (mut x, mut y) = (x0, y0);
        // Hidden under a carried object: a fifth of the speed.
        let k = if self.hidden { hide::HIDE_SPEED } else { 1.0 };
        let (dx, dy) = ((dx * k) as f64, (dy * k) as f64);
        let steps = [(dy < 0.0, DIR_NORTH, 0.0, dy), (dx < 0.0, DIR_WEST, dx, 0.0), (dy > 0.0, DIR_SOUTH, 0.0, dy), (dx > 0.0, DIR_EAST, dx, 0.0)];
        for (pressed, dir, mx, my) in steps {
            if !pressed {
                continue;
            }
            let (sx, sy, res) = self.walk_step(x, y, dir as usize, mx, my);
            match self.count_step(dir, res, sx, sy) {
                Some(p) => (x, y) = p,
                // A ledge jump started: it has placed the player.
                None => {
                    self.walking = true;
                    return Some(self.player_update());
                }
            }
        }
        // Positions are kept to a thousandth of a tile, so the 0.1 nudges do
        // not drift (7.5000005) and a half tile stays a half tile on the wire.
        let round = |v: f64| ((v * 1000.0).round() / 1000.0) as f32;
        let (nx, ny) = (round(x), round(y));
        if nx == self.player_x && ny == self.player_y {
            // Running in place: 2.05 sets the walk gani on every walk tick,
            // moved or not, so the legs keep going against a wall or a ledge
            // until the push pose takes over. No footsteps then (2.05 plays
            // them only in modes 0 and 10, and a refused step is mode 1).
            self.walking = true;
            if !(self.in_water && self.swim_gani.is_some()) {
                if let Some(gani) = self.walk_gani.as_mut() {
                    gani.step();
                }
            }
            // Pushing against a sign reads it.
            self.read_sign_ahead();
            return None;
        }

        if self.hidden {
            self.hide_steps += 1;
        }
        self.player_x = nx;
        self.player_y = ny;
        if let Some(w) = &self.gmap {
            if let Resolved::Crossed(c) = w.map.resolve(w.cell, (self.player_x, self.player_y)) {
                self.cross_to(c);
            }
        }
        self.armed = true;
        self.walking = true;
        self.refresh_water();
        self.read_sign_ahead();
        // One walk GANI step per move; each frame entered with a PLAYSOUND
        // cue queues that sound once, at the cue's offset from the player.
        // The swim GANI runs on the tick clock instead (`tick_poses`).
        let swimming = self.in_water && self.swim_gani.is_some();
        if let Some(gani) = self.walk_gani.as_mut().filter(|_| !swimming) {
            let cues = gani.step_sounds();
            for c in cues {
                self.sound_events.push(SoundEvent { file: c.file, x: self.player_x + c.x, y: self.player_y + c.y });
            }
        }
        Some(self.player_update())
    }

    /// True if the point `(fx, fy)` (tiles, current-cell coordinates) is
    /// blocked for a knocked-back body: a wall tile (or off the board) or
    /// inside a chest. Developer mode skips the test.
    fn blocked_point(&self, fx: f32, fy: f32) -> bool {
        if self.dev.wallhack() {
            return false;
        }
        match (&self.current_level, &self.tiletypes) {
            (Some(level), Some(types)) if self.kind_at_point(level, types, fx, fy) == TileKind::Blocking => return true,
            (Some(_), None) if self.off_board(fx, fy) => return true,
            _ => {}
        }
        match &self.current_level {
            Some(level) => level.chests.iter().any(|c| fx >= c.x as f32 && fx < c.x as f32 + 2.0 && fy >= c.y as f32 && fy < c.y as f32 + 2.0),
            None => false,
        }
    }

    /// One knockback step along the unit vector `(ux, uy)`: one tile, then
    /// the twelve probe points of the body box ([`crate::combat::KNOCKBACK_PROBES`])
    /// must all be free, or the step is taken back. Returns true if the
    /// player moved. Not while a warp is pending.
    pub(crate) fn knock_step(&mut self, ux: f32, uy: f32) -> bool {
        if self.warp_pending.is_some() || self.life.is_dead() {
            return false;
        }
        let (nx, ny) = (self.player_x + ux * crate::combat::KNOCKBACK_STEP_TILES, self.player_y + uy * crate::combat::KNOCKBACK_STEP_TILES);
        if !self.gmap_step_ok(nx, ny) {
            return false;
        }
        if crate::combat::KNOCKBACK_PROBES.iter().any(|&(px, py)| self.blocked_point(nx + px, ny + py)) {
            return false;
        }
        self.player_x = nx;
        self.player_y = ny;
        if let Some(w) = &self.gmap {
            if let Resolved::Crossed(c) = w.map.resolve(w.cell, (self.player_x, self.player_y)) {
                self.cross_to(c);
            }
        }
        self.armed = true;
        self.refresh_water();
        true
    }

    /// True once if a hit's knockback moved the player since the last call:
    /// the caller sends the new position like a step.
    pub fn take_knock_moved(&mut self) -> bool {
        std::mem::take(&mut self.knock_moved)
    }

    /// Attach a walk GANI. Stepped on every successful move.
    pub fn set_walk_gani(&mut self, gani: GaniPlayer) {
        self.walk_gani = Some(gani);
    }

    /// Attach a swim GANI, shown instead of the walk one in water.
    pub fn set_swim_gani(&mut self, gani: GaniPlayer) {
        self.swim_gani = Some(gani);
    }

    /// Sounds queued by GANI frames since the last call.
    pub fn take_sound_events(&mut self) -> Vec<SoundEvent> {
        std::mem::take(&mut self.sound_events)
    }

    /// Sounds scripts stopped (`stopsound`) since the last call.
    pub fn take_sound_stops(&mut self) -> Vec<String> {
        std::mem::take(&mut self.sound_stops)
    }

    /// Current walk-frame index, if a GANI is attached.
    pub fn walk_frame(&self) -> Option<usize> {
        self.walk_gani.as_ref().map(|g| g.frame)
    }

    /// Apply a board update to the current level (if any).
    pub fn apply_board_update(&mut self, m: &BoardModify) {
        if let Some(level) = self.current_level.as_mut() {
            level.apply_board_modify(m);
        }
    }

    /// Whether a level is currently loaded.
    pub fn has_level(&self) -> bool {
        self.current_level.is_some()
    }
}

impl Default for OfflineSession {
    fn default() -> Self {
        Self::new()
    }
}

/// The debug log's line for a level change: `level: <name>` when `now`
/// differs from the level last logged (any case), and remembers it.
pub fn level_change_line(last: &mut Option<String>, now: Option<&str>) -> Option<String> {
    let now = now?;
    if last.as_deref().is_some_and(|l| l.eq_ignore_ascii_case(now)) {
        return None;
    }
    *last = Some(now.to_string());
    Some(format!("level: {now}"))
}

#[cfg(test)]
mod tests {
    /// A 16 ms loop (the frame limit) and a 50 ms step: counting from the due time gives 20 steps
    /// a second (the real clients' pace); counting from the moment of the step
    /// gave a step every 64 ms (15.6 a second).
    #[test]
    fn step_clock_holds_the_pace_on_a_16_ms_loop() {
        use std::time::{Duration, Instant};
        let interval = Duration::from_millis(WALK_STEP_MS);
        let t0 = Instant::now();
        let simulate = |catch_up: bool| {
            let mut last = t0;
            let mut steps = 0;
            // ticks of 16 ms for ten seconds
            for k in 1..=625u64 {
                let now = t0 + Duration::from_millis(k * 16);
                if now - last >= interval {
                    last = if catch_up { next_step_clock(last, now, interval) } else { now };
                    steps += 1;
                }
            }
            steps
        };
        let old = simulate(false);
        let new = simulate(true);
        assert_eq!(old, 156, "the old rule steps every fourth tick (64 ms)");
        assert!((199..=201).contains(&new), "{new} steps in ten seconds");
    }

    #[test]
    fn step_clock_restarts_after_a_pause() {
        use std::time::{Duration, Instant};
        let interval = Duration::from_millis(50);
        let t0 = Instant::now();
        assert_eq!(next_step_clock(t0, t0 + Duration::from_millis(60), interval), t0 + interval);
        let late = t0 + Duration::from_millis(100);
        assert_eq!(next_step_clock(t0, late, interval), late);
    }

    use super::*;
    use crate::assets::ObjectTables;

    #[test]
    fn the_debug_log_names_each_level_change_once() {
        let mut last = None;
        assert_eq!(level_change_line(&mut last, None), None);
        assert_eq!(level_change_line(&mut last, Some("onlinestartlocal.nw")).as_deref(), Some("level: onlinestartlocal.nw"));
        assert_eq!(level_change_line(&mut last, Some("onlinestartlocal.nw")), None);
        assert_eq!(level_change_line(&mut last, Some("OnlineStartLocal.nw")), None);
        assert_eq!(level_change_line(&mut last, Some("new3.nw")).as_deref(), Some("level: new3.nw"));
        // Back again is a change too.
        assert_eq!(level_change_line(&mut last, Some("onlinestartlocal.nw")).as_deref(), Some("level: onlinestartlocal.nw"));
        let mut s = OfflineSession::new();
        assert_eq!(s.level_name(), None);
        s.loaded_file = Some("new3.nw".into());
        assert_eq!(s.level_name().as_deref(), Some("new3.nw"));
    }

    #[test]
    fn chest_and_sign_interact() {
        let text = "GLEVNW01\nCHEST 20 24 greenrupee 0\nCHEST 10 10 fullheart -1\nSIGN 30 10\nWelcome\nSIGNEND\n";
        let mut s = OfflineSession::new();
        s.current_level = Some(Level::parse(text).unwrap());

        // Walk up into the chest at (20, 24): it stops the player below it.
        s.player_x = 19.5;
        s.player_y = 25.0;
        while s.try_move(0.0, -0.5).is_some() {}
        // The body's top edge (y + 1) stops at the chest's bottom row.
        assert_eq!(s.player_y, 25.0);
        assert_eq!(s.player_dir, DIR_NORTH);
        let got = s.interact();
        assert_eq!(
            got,
            Interaction::Chest { x: 20, y: 24, item: "greenrupee".into(), got: "1 rupee".into() }
        );
        assert_eq!(s.rupees, 1);
        // Sign 0 is the chest's note.
        assert_eq!(s.reading.as_deref(), Some("Welcome"));
        // Opened once only.
        s.reading = None;
        assert_eq!(s.interact(), Interaction::Nothing);

        // The A key does not read the sign at (30, 10).
        s.player_x = 28.5;
        s.player_y = 10.0;
        assert_eq!(s.interact(), Interaction::Nothing);
        assert!(s.reading.is_none());

        // A heart container with no note.
        s.reading = None;
        s.player_x = 8.5;
        s.player_y = 11.0;
        assert!(matches!(s.interact(), Interaction::Chest { x: 10, y: 10, .. }));
        assert_eq!(s.max_hearts, 8);
        assert!(s.reading.is_none());
    }

    /// A level with a sign at (30, 10) on wall tiles (30..31, 10).
    fn sign_on_wall() -> OfflineSession {
        let mut lvl = level(|x, y| if (x == 30 || x == 31) && y == 10 { WALL } else { 0 });
        lvl.signs.push(crate::assets::Sign { x: 30, y: 10, text: "Hello".into() });
        let mut s = OfflineSession::new();
        s.current_level = Some(lvl);
        s.tiletypes = Some(types());
        s
    }

    #[test]
    fn walking_up_into_a_sign_opens_it() {
        // 2.05: facing up, (x + 1.5, y + 0.5) in the sign's box and blocked.
        let mut s = sign_on_wall();
        s.player_x = 29.0;
        s.player_y = 13.0;
        while s.reading.is_none() && s.try_move(0.0, -0.5).is_some() {}
        assert_eq!(s.reading.as_deref(), Some("Hello"));
        assert_eq!(s.player_y, 10.0, "stopped with the body under the wall");
        // Closed, then pushed against again: it opens again.
        s.reading = None;
        assert!(s.try_move(0.0, -0.5).is_none());
        assert_eq!(s.reading.as_deref(), Some("Hello"));
    }

    #[test]
    fn a_sign_opens_only_facing_up_and_against_something_solid() {
        // Beside the sign, walking east into the wall: not read.
        let mut s = sign_on_wall();
        s.player_x = 26.0;
        s.player_y = 9.5;
        while s.try_move(0.5, 0.0).is_some() {}
        assert!(s.reading.is_none());
        // The same sign over open ground: the point is not blocked.
        let mut s = sign_on_wall();
        s.tiletypes = Some(TileTypes::from_types([0; 4096]));
        s.player_x = 29.0;
        s.player_y = 11.0;
        s.try_move(0.0, -0.5);
        assert!(s.reading.is_none());
        // The box edges count: x + 1.5 = 32 (the right edge) still reads.
        let mut s = sign_on_wall();
        assert!(s.current_level.as_mut().unwrap().set_tile_id(32, 10, WALL));
        s.player_x = 30.5;
        s.player_y = 10.0;
        s.player_dir = DIR_NORTH;
        assert!(s.try_move(0.0, -0.5).is_none());
        assert_eq!(s.reading.as_deref(), Some("Hello"));
    }

    #[test]
    fn an_open_sign_takes_the_walk_the_a_key_and_hiding() {
        // 2.05: walking and the A / S / D handler are only reached outside
        // reading (Graal_analysis `client-internals/sign-panel.md`).
        let mut s = sign_on_wall();
        s.player_x = 29.0;
        s.player_y = 10.0;
        s.player_dir = DIR_NORTH;
        s.reading = Some("Hello".into());
        assert!(s.try_move(0.5, 0.0).is_none(), "no step");
        assert_eq!((s.player_x, s.player_y, s.player_dir), (29.0, 10.0, DIR_NORTH), "nor a turn");
        assert!(!s.walking);
        // The wall ahead would be grabbed outside reading.
        assert_eq!(s.interact(), Interaction::Nothing);
        assert!(s.pushpull.grab.is_none());
        s.carried = Some(Carried { kind: 0, tiles: [[0, 0], [0, 0]] });
        assert!(!s.toggle_hide(), "D does not hide");
        assert!(!s.hidden);
        assert_eq!(s.interact(), Interaction::Nothing, "A does not throw");
        assert!(s.carried.is_some());
        // Closed, the same keys act again.
        s.reading = None;
        assert!(s.toggle_hide());
        assert!(matches!(s.interact(), Interaction::Thrown { kind: 0 }));
    }

    #[test]
    fn chest_opens_only_facing_up() {
        // Lab capture: the real client opens a chest only when the player is
        // below it facing up; west, east and south sides send nothing.
        let text = "GLEVNW01\nCHEST 20 24 greenrupee -1\n";
        let new = || {
            let mut s = OfflineSession::new();
            s.current_level = Some(Level::parse(text).unwrap());
            s
        };
        // Facing north under the chest: opens.
        let mut s = new();
        s.player_x = 18.5;
        s.player_y = 25.0;
        s.player_dir = DIR_NORTH;
        assert!(matches!(s.interact(), Interaction::Chest { x: 20, y: 24, .. }));
        // Chest to the east of the player, facing east: probe point is inside, nothing happens.
        let mut s = new();
        s.player_x = 16.5;
        s.player_y = 23.0;
        s.player_dir = DIR_EAST;
        let (px, py) = s.facing_point();
        assert!(px >= 20.0 && px < 22.0 && py >= 24.0 && py < 26.0);
        assert_eq!(s.interact(), Interaction::Nothing);
        assert!(s.opened_chests.is_empty());
        // West side, facing west.
        let mut s = new();
        s.player_x = 22.0;
        s.player_y = 23.0;
        s.player_dir = DIR_WEST;
        let (px, py) = s.facing_point();
        assert!(px >= 20.0 && px < 22.0 && py >= 24.0 && py < 26.0);
        assert_eq!(s.interact(), Interaction::Nothing);
        // Above the chest, facing south.
        let mut s = new();
        s.player_x = 18.5;
        s.player_y = 20.0;
        s.player_dir = DIR_SOUTH;
        let (_, py) = s.facing_point();
        assert!(py >= 24.0 && py < 26.0);
        assert_eq!(s.interact(), Interaction::Nothing);
    }

    // ---- tile movement and warp arrival ------------------------------------


    const OPEN: u16 = 0;
    const WALL: u16 = 1;
    const CHAIR: u16 = 7;
    const BED_TOP: u16 = 8;
    const BED_BOTTOM: u16 = 9;
    const WATER: u16 = 2;
    const LAVA: u16 = 3;

    fn types() -> TileTypes {
        let mut t = TileTypes::from_types([0; 4096]);
        t.types[WALL as usize] = 22;
        t.types[WATER as usize] = 11;
        t.types[LAVA as usize] = 12;
        t.types[CHAIR as usize] = 3;
        t.types[BED_TOP as usize] = 4;
        t.types[BED_BOTTOM as usize] = 5;
        t
    }

    /// A 64x64 level where `f(x, y)` picks each tile.
    fn level(f: impl Fn(i32, i32) -> u16) -> Level {
        let tiles: Vec<u16> = (0..64 * 64).map(|i| f(i % 64, i / 64)).collect();
        Level::from_board(&tiles)
    }

    /// A session whose feet stand at tile-space `(fx, fy)`.
    fn session_at(level: Level, fx: f32, fy: f32) -> OfflineSession {
        let mut s = OfflineSession::new();
        s.current_level = Some(level);
        s.tiletypes = Some(types());
        s.player_x = fx - FOOT_DX;
        s.player_y = fy - FOOT_DY;
        s
    }

    /// A session whose sprite's top-left is at `(x, y)`.
    fn body_at(level: Level, x: f32, y: f32) -> OfflineSession {
        let mut s = OfflineSession::new();
        s.current_level = Some(level);
        s.tiletypes = Some(types());
        s.player_x = x;
        s.player_y = y;
        s
    }

    fn walk(s: &mut OfflineSession, dx: f32, dy: f32, steps: usize) -> usize {
        (0..steps).filter(|_| s.try_move(dx, dy).is_some()).count()
    }

    // ---- cutting and lifting ------------------------------------------------

    /// An `arrays.dat` with the structure of the client's: 9 walking lists
    /// (here empty), then the cut table (array 9), what a cut leaves (10),
    /// the lift table (11) and what a lift leaves (12), each a nested list
    /// of 2 x 2 objects stored `[column][row]`.
    fn object_arrays(cut: &[[[u32; 2]; 2]], cut_into: &[[[u32; 2]; 2]], lift: &[[[u32; 2]; 2]], lift_into: &[[[u32; 2]; 2]]) -> Vec<u8> {
        fn objs(out: &mut Vec<u8>, list: &[[[u32; 2]; 2]]) {
            out.extend_from_slice(&((1u32 << 16) | list.len() as u32).to_be_bytes());
            for o in list {
                out.extend_from_slice(&((1u32 << 16) | 2).to_be_bytes());
                for col in o {
                    out.extend_from_slice(&2u32.to_be_bytes());
                    for v in col {
                        out.extend_from_slice(&v.to_be_bytes());
                    }
                }
            }
        }
        let mut out = Vec::new();
        for _ in 0..9 {
            out.extend_from_slice(&0u32.to_be_bytes());
        }
        objs(&mut out, cut);
        objs(&mut out, cut_into);
        objs(&mut out, lift);
        objs(&mut out, lift_into);
        out
    }

    const BUSH: [[u32; 2]; 2] = [[2, 18], [3, 19]];
    const BUSH_CUT: [[u32; 2]; 2] = [[677, 693], [678, 694]];
    const BLOCK: [[u32; 2]; 2] = [[512, 528], [513, 529]];
    const BLOCK_LEFT: [[u32; 2]; 2] = [[1802, 1818], [1803, 1819]];
    const ROCK: [[u32; 2]; 2] = [[34, 50], [35, 51]];
    const ROCK_LEFT: [[u32; 2]; 2] = [[1834, 1850], [1835, 1851]];

    fn put(l: &mut Level, ax: i32, ay: i32, o: [[u32; 2]; 2]) {
        for i in 0..2 {
            for j in 0..2 {
                assert!(l.set_tile_id(ax + i as i32, ay + j as i32, o[i][j] as u16));
            }
        }
    }

    fn get(l: &Level, ax: i32, ay: i32) -> [[u32; 2]; 2] {
        let mut o = [[0; 2]; 2];
        for i in 0..2 {
            for j in 0..2 {
                o[i][j] = u32::from(l.tile_id_at(ax + i as i32, ay + j as i32).unwrap());
            }
        }
        o
    }

    fn object_session(glove: u8) -> OfflineSession {
        let data = object_arrays(&[BUSH, [[420, 436], [421, 437]]], &[BUSH_CUT, [[679, 695], [680, 696]]], &[BUSH, BLOCK, ROCK], &[BUSH_CUT, BLOCK_LEFT, ROCK_LEFT]);
        let mut lvl = level(|_, _| 2047);
        put(&mut lvl, 20, 20, BUSH);
        put(&mut lvl, 30, 20, BLOCK);
        put(&mut lvl, 40, 20, ROCK);
        let mut s = OfflineSession::new();
        s.current_level = Some(lvl);
        s.tiletypes = Some(types());
        s.object_tables = ObjectTables::parse(&data);
        s.glove_power = glove;
        s
    }

    /// Stand below the object whose top-left tile is `(ax, ay)`, facing it:
    /// the probe point (x + 1.5, y) falls in the object's bottom row.
    fn face_up_at(s: &mut OfflineSession, ax: i32, ay: i32) {
        s.player_x = ax as f32 - 0.5;
        s.player_y = ay as f32 + 1.5;
        s.player_dir = DIR_NORTH;
    }

    #[test]
    fn object_tables_parse_the_nested_arrays() {
        let t = ObjectTables::parse(&object_arrays(&[BUSH], &[BUSH_CUT], &[ROCK], &[ROCK_LEFT])).unwrap();
        assert_eq!(t.cut, vec![[[2, 18], [3, 19]]]);
        assert_eq!(t.cut_into, vec![[[677, 693], [678, 694]]]);
        assert_eq!(t.lift[0], [[34, 50], [35, 51]]);
        assert_eq!(t.lift_into[0], [[1834, 1850], [1835, 1851]]);
        // A file with only walking lists has no tables; a cut-off one is refused.
        assert!(ObjectTables::parse(&[0, 0, 0, 0]).is_none());
        let mut bad = object_arrays(&[BUSH], &[BUSH_CUT], &[ROCK], &[ROCK_LEFT]);
        bad.truncate(bad.len() - 3);
        assert!(ObjectTables::parse(&bad).is_none());
    }

    #[test]
    fn the_sword_cuts_a_bush_by_its_tiles_and_not_grass_or_a_rock() {
        let mut s = object_session(0);
        face_up_at(&mut s, 20, 20);
        assert_eq!(s.try_cut(), Some(0));
        assert_eq!(get(s.current_level.as_ref().unwrap(), 20, 20), BUSH_CUT);
        // Nothing left to cut.
        assert_eq!(s.try_cut(), None);
        // A point beside the object cuts nothing.
        let mut s = object_session(0);
        face_up_at(&mut s, 22, 20);
        assert_eq!(s.try_cut(), None);
        // A rock is not in the cut table.
        face_up_at(&mut s, 40, 20);
        assert_eq!(s.try_cut(), None);
    }

    /// A cut bush plays the bush's leaves (break row 0, seven frames of 50 ms),
    /// anchored on the object's top-left tile; a swing that cuts nothing plays
    /// nothing.
    #[test]
    fn a_cut_bush_plays_the_leaf_effect_where_it_stood() {
        let mut s = object_session(0);
        face_up_at(&mut s, 22, 20);
        assert_eq!(s.try_cut(), None);
        assert!(s.leaps.is_empty(), "nothing cut, no effect");
        face_up_at(&mut s, 20, 20);
        assert_eq!(s.try_cut(), Some(0));
        assert_eq!(s.leaps, vec![crate::throwing::Leap { kind: crate::throwing::LEAP_BUSH, x: 20.0, y: 20.0, frame: -1 }]);
        // not drawn before its first tick; then one frame per tick
        assert!(s.tick_throw(50));
        for f in 0..7 {
            assert_eq!(s.leaps.len(), 1, "frame {f} is shown");
            assert_eq!(s.leaps[0].frame, f);
            s.tick_throw(50);
        }
        assert!(s.leaps.is_empty(), "gone after the last frame");
    }

    #[test]
    fn a_swing_that_cuts_plays_the_effect_and_a_level_change_drops_it() {
        let mut s = object_session(0);
        s.life.set_rules(Some(crate::protocol::DEATH_205));
        s.combat.set_rules(Some(crate::protocol::COMBAT_205));
        s.sword_power = 1;
        face_up_at(&mut s, 20, 20);
        assert!(s.start_swing());
        assert_eq!(s.leaps.len(), 1);
        s.clear_throws();
        assert!(s.leaps.is_empty());
    }

    #[test]
    fn a_swing_cuts_when_it_starts() {
        let mut s = object_session(0);
        s.life.set_rules(Some(crate::protocol::DEATH_205));
        s.combat.set_rules(Some(crate::protocol::COMBAT_205));
        s.sword_power = 1;
        face_up_at(&mut s, 20, 20);
        assert!(s.start_swing());
        assert_eq!(get(s.current_level.as_ref().unwrap(), 20, 20), BUSH_CUT);
        let c = s.take_board_changes();
        assert_eq!(c.len(), 1, "the cut goes to the server as one board modify");
        assert_eq!((c[0].x, c[0].y, c[0].width, c[0].height), (20, 20, 2, 2));
        assert_eq!(c[0].tiles, [BUSH_CUT[0][0], BUSH_CUT[1][0], BUSH_CUT[0][1], BUSH_CUT[1][1]].map(|t| t as u16), "row by row");
    }

    #[test]
    fn a_throw_while_walking_puts_the_player_back_to_idle() {
        let mut s = object_session(0);
        face_up_at(&mut s, 20, 20);
        assert_eq!(s.interact(), Interaction::Lifted { kind: 0 });
        s.walking = true;
        assert_eq!(s.interact(), Interaction::Thrown { kind: 0 });
        assert!(!s.walking, "the throw ends the carry walk");
        // A hidden player is thrown out of hiding the same way.
        face_up_at(&mut s, 30, 20);
        s.carried = Some(Carried { kind: 0, tiles: [[2, 18], [3, 19]] });
        assert!(s.toggle_hide());
        s.walking = true;
        assert!(s.throw_carried().is_some());
        assert!(!s.walking && !s.hidden);
    }

    #[test]
    fn a_bare_hand_lifts_a_bush_and_the_block_but_not_the_rock() {
        let mut s = object_session(0);
        face_up_at(&mut s, 20, 20);
        assert_eq!(s.interact(), Interaction::Lifted { kind: 0 });
        assert_eq!(get(s.current_level.as_ref().unwrap(), 20, 20), BUSH_CUT, "the bush leaves its stump");
        let c = s.take_board_changes();
        assert_eq!((c.len(), c[0].x, c[0].y, c[0].width, c[0].height), (1, 20, 20, 2, 2), "the lift is a 2 x 2 board modify");
        assert_eq!(c[0].tiles, [BUSH_CUT[0][0], BUSH_CUT[1][0], BUSH_CUT[0][1], BUSH_CUT[1][1]].map(|t| t as u16), "row by row");
        assert_eq!(s.carried.as_ref().unwrap().tiles, [[2u16, 18], [3, 19]]);
        // Hands full: A throws it.
        assert_eq!(s.interact(), Interaction::Thrown { kind: 0 });
        assert!(s.carried.is_none());
        assert_eq!(s.thrown.len(), 1);
        s.thrown.clear();

        face_up_at(&mut s, 30, 20);
        assert_eq!(s.interact(), Interaction::Lifted { kind: 1 });
        assert_eq!(get(s.current_level.as_ref().unwrap(), 30, 20), BLOCK_LEFT);
        s.carried = None;

        // A rock blocks like a wall. Standing flush against it (the body's
        // top edge on its bottom edge), A grabs it: the grab point is 0.1
        // tile above the body.
        for id in ROCK.iter().flatten() {
            s.tiletypes.as_mut().unwrap().types[*id as usize] = 22;
        }
        face_up_at(&mut s, 40, 20);
        s.player_y = 21.0;
        assert_eq!(s.interact(), Interaction::Grabbed, "the rock is grabbed, not lifted");
        assert!(s.carried.is_none());
        assert_eq!(s.pushpull.grab, Some(Grab { count: 1 }), "held until A is let go");
        assert_eq!(get(s.current_level.as_ref().unwrap(), 40, 20), ROCK, "the rock does not move");
    }

    /// arrays.dat array 11's third entry is the pot: 2.05 players start at
    /// glove 1, which reaches it, and the glove items raise it to 2 and 3.
    #[test]
    fn players_start_at_glove_1_and_the_gloves_raise_it() {
        assert_eq!(OfflineSession::new().glove_power, 1);
        let mut s = object_session(OfflineSession::new().glove_power);
        face_up_at(&mut s, 40, 20);
        assert_eq!(s.interact(), Interaction::Lifted { kind: 2 }, "the third entry without a glove item");
        let mut s = OfflineSession::new();
        s.glove_power = 1;
        s.give_item("glove1");
        assert_eq!(s.glove_power, 2);
        s.give_item("glove2");
        assert_eq!(s.glove_power, 3);
        s.give_item("glove1");
        assert_eq!(s.glove_power, 3, "never lowered");
    }

    #[test]
    fn a_glove_lifts_the_rock() {
        let mut s = object_session(1);
        face_up_at(&mut s, 40, 20);
        assert_eq!(s.interact(), Interaction::Lifted { kind: 2 });
        assert_eq!(get(s.current_level.as_ref().unwrap(), 40, 20), ROCK_LEFT);
        assert_eq!(s.carried.as_ref().unwrap().kind, 2);
    }

    #[test]
    fn no_lift_while_swimming_or_without_the_tables() {
        let mut s = object_session(2);
        face_up_at(&mut s, 20, 20);
        s.in_water = true;
        assert_eq!(s.try_lift(), None);
        s.in_water = false;
        s.object_tables = None;
        assert_eq!(s.try_lift(), None);
        assert_eq!(s.try_cut(), None);
    }

    /// Walking into the water splashes once where the player stands, with
    /// `water.wav`; walking out splashes once more. Being put into the water
    /// (not walking) splashes three times.
    #[test]
    fn entering_and_leaving_water_splashes() {
        let tiles = |x: i32, _y: i32| if x >= 20 { WATER } else { OPEN };
        let mut s = session_at(level(tiles), 19.75, 30.0);
        s.refresh_water();
        assert!(s.leaps.is_empty());
        assert!(s.try_move(0.5, 0.0).is_some());
        assert!(s.in_water);
        assert_eq!(s.leaps.len(), 1);
        assert_eq!(s.leaps[0].kind, crate::throwing::LEAP_SPLASH);
        assert_eq!((s.leaps[0].x, s.leaps[0].y), (s.player_x, s.player_y));
        assert!(s.sound_events.iter().any(|e| e.file == "water.wav"));
        assert!(s.try_move(0.5, 0.0).is_some());
        assert_eq!(s.leaps.len(), 1, "no splash while swimming on");
        assert!(s.try_move(-1.0, 0.0).is_some());
        assert!(!s.in_water);
        assert_eq!(s.leaps.len(), 2);

        let mut put = session_at(level(tiles), 30.0, 30.0);
        put.refresh_water();
        assert_eq!(put.leaps.len(), 3);
    }

    /// A bench (chair tiles, walkable): the body's middle `(x + 1.5, y + 2)`
    /// on it sits, walking off it stands. The flag is set while walking
    /// too: the 2.05 client sets the sit pose after the walk pose.
    #[test]
    fn the_body_on_a_chair_tile_sits() {
        // Chair tiles in column 20, rows 10..=11 (a 1 x 2 bench).
        let tiles = |x: i32, y: i32| if x == 20 && (10..=11).contains(&y) { CHAIR } else { OPEN };
        // probe at (19.5, 10.5): tile 19, not a chair
        let mut s = body_at(level(tiles), 18.0, 8.5);
        s.refresh_water();
        assert!(!s.sitting);
        // one step right: the probe is at (20.0, 10.5), on the bench
        assert!(s.try_move(0.5, 0.0).is_some());
        assert!(s.sitting, "probe at ({}, {})", s.player_x + SIT_PROBE.0, s.player_y + SIT_PROBE.1);
        // walking on: still on it at 20.5, then off at 21.0
        assert!(s.try_move(0.5, 0.0).is_some());
        assert!(s.sitting);
        assert!(s.try_move(0.5, 0.0).is_some());
        assert!(!s.sitting);
        // the bench is not a wall: it did not block the walk
    }

    /// Both rows of a 2 x 2 chair sit (the old feet point, y + 2.75, missed
    /// the lower row whenever the body was drawn over it).
    #[test]
    fn both_halves_of_a_chair_sit() {
        let tiles = |x: i32, y: i32| if (20..=21).contains(&x) && (10..=11).contains(&y) { CHAIR } else { OPEN };
        let probe_y = |s: &OfflineSession| s.player_y + SIT_PROBE.1;
        // y + 2 runs over rows 10 and 11: y from 8.0 up to (not with) 10.0.
        for y in [8.0, 8.5, 9.0, 9.5] {
            let mut s = body_at(level(tiles), 19.0, y);
            s.refresh_water();
            assert!(s.sitting, "probe y {}", probe_y(&s));
        }
        for y in [7.5, 10.0] {
            let mut s = body_at(level(tiles), 19.0, y);
            s.refresh_water();
            assert!(!s.sitting, "probe y {}", probe_y(&s));
        }
    }

    /// A player who spawns on a bench sits as soon as the tile types are
    /// loaded; carrying or swimming does not sit.
    #[test]
    fn a_player_placed_on_a_chair_sits() {
        let tiles = |x: i32, y: i32| if (20..=21).contains(&x) && (10..=11).contains(&y) { CHAIR } else { OPEN };
        let mut s = OfflineSession::new();
        s.current_level = Some(level(tiles));
        s.player_x = 20.0 - SIT_PROBE.0 + 0.5;
        s.player_y = 10.0 - SIT_PROBE.1 + 0.5;
        s.tiletypes = Some(types());
        s.refresh_water();
        assert!(s.sitting);
        s.carried = Some(Carried { kind: 0, tiles: [[0, 0], [0, 0]] });
        s.refresh_rest();
        assert!(!s.sitting, "not while carrying");
        s.carried = None;
        // off the board is not a chair
        s.player_x = -10.0;
        s.refresh_water();
        assert!(!s.sitting);
    }

    /// A 2-wide bed at columns 20..=21: top row 10, bottom row 11.
    fn bed(x: i32, y: i32) -> u16 {
        match (x, y) {
            (20..=21, 10) => BED_TOP,
            (20..=21, 11) => BED_BOTTOM,
            _ => OPEN,
        }
    }

    /// The head on the bed's top row, both probes (x + 1 and x + 1.8) on it,
    /// lies down; one probe off the bed does not.
    #[test]
    fn the_head_on_a_bed_top_sleeps() {
        // x + 1 and x + 1.8 both in columns 20..=21: x from 19.0 up to 20.2.
        for x in [19.0, 19.5, 20.0] {
            let mut s = body_at(level(bed), x, 10.0);
            s.refresh_water();
            assert!(s.sleeping, "x {x}");
            assert!(!s.sitting);
            assert_eq!((s.player_x, s.player_y), (x, 10.0), "x is left alone");
        }
        for x in [18.5, 20.5] {
            let mut s = body_at(level(bed), x, 10.0);
            s.refresh_water();
            assert!(!s.sleeping, "x {x}");
        }
        // The bottom half alone is never tested: the head on row 11 stands.
        let mut s = body_at(level(bed), 19.5, 11.0);
        s.refresh_water();
        assert!(!s.sleeping);
    }

    /// Lying down puts the head on the top row: half a tile down when the
    /// head is just above it, then cut to a whole tile.
    #[test]
    fn lying_down_snaps_the_head_to_the_bed_top_row() {
        // y = 9.5: the probes at y (row 9) miss, at y + 0.5 (row 10) hit.
        let mut s = body_at(level(bed), 19.5, 9.5);
        s.refresh_water();
        assert!(s.sleeping);
        assert_eq!(s.player_y, 10.0);
        // y = 10.5: the probes at y are on row 10 already: cut to 10.
        let mut s = body_at(level(bed), 19.5, 10.5);
        s.refresh_water();
        assert!(s.sleeping);
        assert_eq!(s.player_y, 10.0);
    }

    /// The sit and sleep poses are drawn walking or not, and only with
    /// their GANI loaded.
    #[test]
    fn sit_and_sleep_poses_win_over_the_walk() {
        use crate::pose::PoseKind;
        let one = || crate::assets::Gani::parse("ANI\n1 0 0\n1 0 0\n1 0 0\n1 0 0\nANIEND\n").unwrap();
        let tiles = |x: i32, y: i32| if (20..=21).contains(&x) && (10..=11).contains(&y) { CHAIR } else { bed(x, y - 20) };
        let mut s = body_at(level(tiles), 19.0, 8.5);
        s.set_pose_gani(PoseKind::Idle, one());
        s.refresh_water();
        assert!(s.sitting);
        assert_eq!(s.current_pose().map(|p| p.kind), Some(PoseKind::Idle), "no sit.gani: idle");
        s.set_pose_gani(PoseKind::Sit, one());
        s.set_pose_gani(PoseKind::Sleep, one());
        s.walking = true;
        assert_eq!(s.current_pose().map(|p| p.kind), Some(PoseKind::Sit));
        s.player_y = 30.0;
        s.refresh_water();
        assert!(s.sleeping && !s.sitting);
        assert_eq!(s.current_pose().map(|p| p.kind), Some(PoseKind::Sleep));
    }

    /// In bed, a step down snaps straight back; a step to the side gets out.
    #[test]
    fn the_way_out_of_a_bed_is_to_the_side() {
        let mut s = body_at(level(bed), 19.5, 10.0);
        s.refresh_water();
        assert!(s.sleeping);
        s.try_move(0.0, 0.5);
        assert_eq!(s.player_y, 10.0);
        assert!(s.sleeping);
        assert!(s.try_move(0.5, 0.0).is_some());
        assert!(s.sleeping, "x + 1.8 = 21.8 is still on the bed");
        assert!(s.try_move(0.5, 0.0).is_some());
        assert!(!s.sleeping, "x + 1.8 = 22.3 is off it");
    }

    #[test]
    fn wall_blocks_a_walk() {
        let mut s = body_at(level(|x, _| if x == 20 { WALL } else { OPEN }), 16.5, 8.0);
        // The right edge of the body is x + 2.5: 19.0 -> 19.5 is tile 19, 20.0 is the wall.
        // The right probe sits on the tile line beyond the edge, so the step
        // to 17.5 (edge 20.0) is refused, and the client then snaps the body
        // flush to the wall: the push to 17.5 is the second change.
        assert_eq!(walk(&mut s, 0.5, 0.0, 10), 2);
        assert_eq!(s.player_x, 17.5);
        assert_eq!(s.player_dir, DIR_EAST);
        // and it is open again going back
        assert_eq!(walk(&mut s, -0.5, 0.0, 2), 2);
    }

    #[test]
    fn last_row_and_column_are_floor_not_wall() {
        // The original client truncates its test point to a tile, so tile
        // 63 runs from 63.0 to 64.0; only 64.0 and beyond is off the board.
        // The body's right edge is x + 2.5 and its left edge x + 0.5.
        let mut s = body_at(level(|_, _| OPEN), 60.5, 10.0);
        assert_eq!(walk(&mut s, 0.5, 0.0, 10), 2, "edge 63.0 -> 63.5; 64.0 is outside, and the snap closes the last half tile");
        assert_eq!(s.player_x, 61.5, "right edge flush with the board edge at 64.0");
        let mut s = body_at(level(|_, _| OPEN), 0.0, 10.0);
        assert_eq!(walk(&mut s, -0.5, 0.0, 4), 1, "edge 0.5 -> 0.0 ok, -0.5 is off the board");
        assert_eq!(s.player_x, -0.5);
    }

    #[test]
    fn wall_edge_is_the_tile_edge_not_half_a_tile_earlier() {
        // Wall column at x = 20: a right edge at 19.5 must be allowed, since
        // tile 19 is floor (a rounded test point would call it tile 20).
        let mut s = body_at(level(|x, _| if x == 20 { WALL } else { OPEN }), 16.5, 8.0);
        assert!(s.try_move(0.5, 0.0).is_some());
        assert_eq!(s.player_x + 2.5, 19.5);
        // The next step is refused (its right probe is on the wall's tile
        // line); the body is then pulled flush against the wall.
        assert!(s.try_move(0.5, 0.0).is_some());
        assert_eq!(s.player_x + 2.5, 20.0);
        assert!(s.try_move(0.5, 0.0).is_none());
    }

    #[test]
    fn water_is_swimmable_not_a_wall() {
        // Water at x 10..=13 on every row.
        let mut s = session_at(level(|x, _| if (10..=13).contains(&x) { WATER } else { OPEN }), 8.5, 20.5);
        assert!(!s.in_water);
        assert_eq!(s.take_swim_change(), None);
        assert!(s.try_move(0.5, 0.0).is_some()); // 9.0
        assert!(!s.in_water);
        assert!(s.try_move(0.5, 0.0).is_some()); // 9.5
        assert!(s.try_move(0.5, 0.0).is_some()); // 10.0: in the water
        assert!(s.in_water);
        assert_eq!(s.take_swim_change(), Some(true));
        assert_eq!(s.take_swim_change(), None, "reported once per change");
        // swim across and leave
        walk(&mut s, 0.5, 0.0, 8); // 10.5 .. 14.0
        assert!(!s.in_water);
        assert_eq!(s.take_swim_change(), Some(false));
        assert_eq!(s.foot().0, 14.0);
    }

    #[test]
    fn swimming_is_as_fast_as_walking_and_reports_the_gani() {
        let mut s = session_at(level(|_, _| WATER), 30.5, 30.5);
        assert!(s.try_move(0.5, 0.0).is_some());
        assert!(s.in_water);
        // 2.05 rev 2 swims 0.5 tile a tick, the walking pace.
        assert_eq!(s.step_interval(), std::time::Duration::from_millis(WALK_STEP_MS));
        let mut dry = session_at(level(|_, _| OPEN), 30.5, 30.5);
        dry.try_move(0.5, 0.0);
        assert_eq!(dry.step_interval(), std::time::Duration::from_millis(WALK_STEP_MS));
        // PLI_PLAYERPROPS, PLPROP_GANI, GSTRING
        assert_eq!(swim_message(true), b"\x22\x2a\x24swim\n");
        assert_eq!(swim_message(false), b"\x22\x2a\x24idle\n");
    }

    #[test]
    fn lava_does_not_block_and_is_swum() {
        let mut s = session_at(level(|x, _| if x >= 12 { LAVA } else { OPEN }), 10.5, 20.5);
        assert_eq!(walk(&mut s, 0.5, 0.0, 6), 6);
        // 2.05: the liquid list holds lava, so the swim test is true there.
        assert!(s.in_water, "lava is swum in");
    }

    #[test]
    fn diagonal_into_a_corner_goes_nowhere() {
        // A wall row at y = 30 and a wall column at x = 20: the body below
        // the row and left of the column cannot go down-right, since both of
        // its axis steps are blocked.
        let mut s = body_at(level(|x, y| if y == 30 || x == 20 { WALL } else { OPEN }), 17.5, 27.0);
        let before = (s.player_x, s.player_y);
        assert!(s.try_move(0.5, 0.5).is_none());
        assert_eq!((s.player_x, s.player_y), before);
        // From half a tile short on both sides, the body is pulled flush into
        // the corner (the snap of each refused step) and then stays.
        let mut s = body_at(level(|x, y| if y == 30 || x == 20 { WALL } else { OPEN }), 17.0, 26.5);
        assert!(s.try_move(0.5, 0.5).is_some());
        assert_eq!((s.player_x, s.player_y), (17.5, 27.0));
        assert!(s.try_move(0.5, 0.5).is_none());
    }

    #[test]
    fn diagonal_slides_along_a_wall() {
        // Wall row at y = 30: the bottom edge (y + 3) is at 29.5, so going
        // down-right slides right.
        // (Starting a half tile higher, the down step is refused and the body
        // is pulled flush to the row first: 26.5 -> 27.0.)
        let mut s = body_at(level(|_, y| if y == 30 { WALL } else { OPEN }), 10.0, 27.0);
        let r = s.try_move(0.5, 0.5);
        assert!(r.is_some());
        assert_eq!((s.player_x, s.player_y), (10.5, 27.0));
        let mut s = body_at(level(|_, y| if y == 30 { WALL } else { OPEN }), 10.0, 26.5);
        assert!(s.try_move(0.5, 0.5).is_some());
        assert_eq!((s.player_x, s.player_y), (10.5, 27.0));
    }

    #[test]
    fn the_leading_edge_is_four_points_wide_not_one() {
        // A single wall tile at (21, 20), two tiles left of where the old
        // foot point (x + 2.0) would have met it: the left half of the body
        // walks into it from below. Going up, the edge is y + 1 and spans
        // x + 0.5 .. x + 2.5.
        let tiles = |x: i32, y: i32| if (x, y) == (21, 20) { WALL } else { OPEN };
        let mut s = body_at(level(tiles), 19.0, 20.5);
        // top edge 21.5, x range 19.5..21.5: the point at 21.475 is in tile 21.
        assert!(s.try_move(0.0, -0.5).is_some()); // edge 21.0: row 21, open
        // Edge 20.5 is row 20 and 21.475 is the wall: the step is refused. Only
        // the right-hand probe is blocked, so the client nudges the body 0.1
        // to the left instead of leaving it stuck on the corner.
        let before = s.player_y;
        assert!(s.try_move(0.0, -0.5).is_some());
        assert_eq!((s.player_x, s.player_y), (18.9, before), "nudged left, not up");
        // One tile to the left it is clear.
        let mut s = body_at(level(tiles), 18.0, 20.5);
        assert_eq!(walk(&mut s, 0.0, -0.5, 4), 4);
        assert_eq!(s.player_x, 18.0);
    }

    #[test]
    fn rocks_block_with_a_nested_arrays_file() {
        // The client's arrays.dat: array 0 is the passable list, and the
        // rock's tiles (34 35 50 51) are not in it.
        let mut data = Vec::new();
        for list in [vec![0u32, 2047], vec![], vec![], vec![]] {
            data.extend_from_slice(&(list.len() as u32).to_be_bytes());
            for v in list {
                data.extend_from_slice(&v.to_be_bytes());
            }
        }
        let t = TileTypes::parse_arrays(&data).unwrap();
        let tiles = |x: i32, y: i32| match (x, y) {
            (40, 10) => 34,
            (41, 10) => 35,
            (40, 11) => 50,
            (41, 11) => 51,
            _ => 2047,
        };
        let mut s = body_at(level(tiles), 36.5, 9.0);
        s.tiletypes = Some(t.clone());
        // Right edge 39.0 -> 39.5 free, 40.0 is the rock; the body ends flush
        // against it (edge 40.0, x = 37.5).
        assert_eq!(walk(&mut s, 0.5, 0.0, 10), 2);
        assert_eq!(s.player_x, 37.5);
        // From above, the bottom edge y + 3 reaches the rock's top row (10):
        // flush at y = 7.0.
        let mut s = body_at(level(tiles), 39.0, 6.0);
        s.tiletypes = Some(t);
        assert_eq!(walk(&mut s, 0.0, 0.5, 10), 2);
        assert_eq!(s.player_y, 7.0);
    }

    #[test]
    fn a_player_inside_a_wall_can_walk_out() {
        // Walls in columns 20 and 21; a body at x = 19.0 spans 19.5 to 21.5,
        // so it is half in the wall. It may step back out (left) ...
        let mut s = body_at(level(|x, _| if (20..=21).contains(&x) { WALL } else { OPEN }), 19.0, 10.0);
        assert!(s.try_move(-0.5, 0.0).is_some());
        assert_eq!(s.player_x, 18.5);
        // ... and, once out, the wall blocks it again.
        assert_eq!(walk(&mut s, -0.5, 0.0, 3), 3);
        assert_eq!(s.player_x, 17.0);
        // Clear of the wall, it blocks again: one more step reaches the wall
        // (the body ends flush, edge 20.0) and no further.
        assert_eq!(walk(&mut s, 0.5, 0.0, 10), 1);
        assert_eq!(s.player_x, 17.5);
    }

    /// The screenshots' bug: a body resting against a wall by its right or
    /// bottom edge was counted as "inside" it (the right and bottom probes
    /// sit on the tile line outside the box), and a body counted as inside
    /// walked through anything. Walking down along a wall at x = 17.5 (right
    /// edge on the tile line, wall column 20) must not open the wall.
    #[test]
    fn resting_against_a_wall_does_not_unlock_walking_through_it() {
        let tiles = |x: i32, y: i32| if x == 20 && (10..=12).contains(&y) { WALL } else { OPEN };
        let mut s = body_at(level(tiles), 17.5, 5.0);
        // walk down past the wall: nothing in the way for a body at 17.5 (right probes are not used going down)
        assert_eq!(walk(&mut s, 0.0, 0.5, 20), 20);
        assert_eq!(s.player_x, 17.5);
        // now beside the wall's rows again from below; go back up and stop next to it
        let mut s = body_at(level(tiles), 17.5, 8.0);
        assert!(s.try_move(0.0, 0.5).is_some());
        assert!(s.try_move(0.0, 0.5).is_some());
        // beside the wall now (box rows overlap 10..12): the wall still blocks stepping right
        assert_eq!(walk(&mut s, 0.5, 0.0, 10), 0, "x = {}", s.player_x);
        assert_eq!(s.player_x, 17.5);
        // and diagonal pushes slide along it instead of through it
        assert_eq!(walk(&mut s, 0.5, 0.5, 3), 3);
        assert_eq!(s.player_x, 17.5);
    }

    /// Same at the bottom edge: a body whose bottom edge rests on a wall row.
    #[test]
    fn resting_on_a_wall_row_does_not_unlock_walking_through_it() {
        let tiles = |x: i32, y: i32| if y == 20 && (10..=12).contains(&x) { WALL } else { OPEN };
        // y + 3.0 = 20.0: the bottom edge is on the tile line above the wall row.
        let mut s = body_at(level(tiles), 9.5, 17.0);
        assert_eq!(walk(&mut s, 0.0, 0.5, 10), 0);
        // slide sideways above the wall, then try down again
        assert_eq!(walk(&mut s, 0.5, 0.0, 2), 2);
        assert_eq!(walk(&mut s, 0.0, 0.5, 10), 0, "y = {}", s.player_y);
        assert_eq!(s.player_y, 17.0);
    }

    /// Rocks one tile apart: a two-tile body does not fit between them, and a
    /// body beside one is not freed by it.
    #[test]
    fn a_body_overlap_of_zero_for_every_position_reached_by_walking() {
        let tiles = |x: i32, y: i32| if (x * 7 + y * 13) % 5 == 0 { WALL } else { OPEN };
        let mut s = body_at(level(|x, y| if (6..40).contains(&x) && (6..40).contains(&y) { tiles(x, y) } else { OPEN }), 2.0, 2.0);
        // a long scripted walk of mixed directions
        let dirs = [(0.5, 0.0), (0.0, 0.5), (-0.5, 0.0), (0.0, -0.5), (0.5, 0.5), (-0.5, 0.5), (0.5, -0.5)];
        for i in 0..4000usize {
            let (dx, dy) = dirs[(i / 7 + i / 31) % dirs.len()];
            s.try_move(dx, dy);
            assert_eq!(s.body_overlap(s.player_x as f64, s.player_y as f64), 0, "step {i} at ({}, {})", s.player_x, s.player_y);
        }
    }

    // ---- the real client's walking rule ------------------------------------
    //
    // These encode what the 2.05, 2.17.1, 2.22 and 2.31 clients do (the same
    // routine in all four): six probe points per facing, a 0.1-tile corner
    // nudge, a snap flush to the wall after a refused step, and a snap to
    // the tile line within 0.0625 of it. See documentation/walking-collision.md.

    /// A 2 x 2 rock of wall tiles with its top-left tile at `(rx, ry)`.
    fn rock_at(rx: i32, ry: i32) -> Level {
        level(move |x, y| if (rx..rx + 2).contains(&x) && (ry..ry + 2).contains(&y) { WALL } else { OPEN })
    }

    /// The body (x + 0.5 to x + 2.5, y + 1 to y + 3) must never contain a
    /// blocked point, and the six probe table must be what the clients carry.
    #[test]
    fn the_probe_table_is_the_clients() {
        assert_eq!(WALK_PROBES[0], [(0.525, 1.0), (1.475, 1.0), (1.525, 1.0), (2.475, 1.0), (2.525, 1.0), (0.475, 1.0)]);
        assert_eq!(WALK_PROBES[2][3], (2.475, 3.0));
        assert_eq!(WALK_PROBES[1][0], (0.5, 1.025));
        assert_eq!(WALK_PROBES[3][5], (2.5, 0.975));
        // Nudges: sideways for up and down, up/down for left and right.
        assert_eq!(WALK_NUDGE[0], [(0.1, 0.0), (-0.1, 0.0)]);
        assert_eq!(WALK_NUDGE[3], [(0.0, 0.1), (0.0, -0.1)]);
    }

    /// Whichever half-tile the body starts on, walking at a rock from any
    /// side ends flush against it: the edge on the rock's tile line. (Before,
    /// the right and bottom stopped half a tile short.)
    #[test]
    fn every_side_of_a_rock_ends_flush_from_both_alignments() {
        let lvl = || rock_at(30, 20); // tiles 30..31 x 20..21
        // from the west, rows 20.0 to 22.0 exactly on the rock's rows: right
        // edge x + 2.5 -> 30.0
        for x0 in [24.0, 24.5] {
            let mut s = body_at(lvl(), x0, 19.5);
            walk(&mut s, 0.5, 0.0, 30);
            assert_eq!((s.player_x, s.player_y), (27.5, 19.5), "from x = {x0}");
        }
        // from the east: left edge x + 0.5 -> 32.0
        for x0 in [38.0, 38.5] {
            let mut s = body_at(lvl(), x0, 19.5);
            walk(&mut s, -0.5, 0.0, 30);
            assert_eq!((s.player_x, s.player_y), (31.5, 19.5), "from x = {x0}");
        }
        // from above: bottom edge y + 3 -> 20.0
        for y0 in [10.0, 10.5] {
            let mut s = body_at(lvl(), 29.5, y0);
            walk(&mut s, 0.0, 0.5, 30);
            assert_eq!((s.player_x, s.player_y), (29.5, 17.0), "from y = {y0}");
        }
        // from below: top edge y + 1 -> 22.0
        for y0 in [30.0, 30.5] {
            let mut s = body_at(lvl(), 29.5, y0);
            walk(&mut s, 0.0, -0.5, 30);
            assert_eq!((s.player_x, s.player_y), (29.5, 21.0), "from y = {y0}");
        }
    }

    /// The report: "blocks me from walking onto grass next to an object". A
    /// rock at tiles 30..31; the body flush against its west side (right edge
    /// 30.0) can still walk down the column beside it onto the grass, and
    /// then back up, with the rock never in the way.
    #[test]
    fn stepping_onto_grass_next_to_a_rock() {
        let mut s = body_at(rock_at(30, 20), 24.0, 19.5);
        walk(&mut s, 0.5, 0.0, 30);
        assert_eq!(s.player_x, 27.5, "right edge flush at 30.0");
        // Down the rock's west side: nothing on the leading (bottom) edge.
        assert_eq!(walk(&mut s, 0.0, 0.5, 20), 20);
        assert_eq!((s.player_x, s.player_y), (27.5, 29.5));
        // and back up to the rock's rows, still flush
        assert_eq!(walk(&mut s, 0.0, -0.5, 20), 20);
        assert_eq!((s.player_x, s.player_y), (27.5, 19.5));
    }

    /// A 2-wide body does not fit a 1-tile gap, and does fit a 2-tile gap.
    /// Walking down along a wall into the gap: the right-hand wall at column
    /// 22 is open from row 20 down.
    #[test]
    fn a_body_walks_along_a_wall_into_a_two_tile_gap_and_not_a_one_tile_gap() {
        // Walls at columns 19 and 22 for rows 10..=29; the gap is 20..=21.
        let two = |x: i32, y: i32| if (10..30).contains(&y) && (x == 19 || x == 22) { WALL } else { OPEN };
        let mut s = body_at(level(two), 19.5, 5.0);
        assert_eq!(walk(&mut s, 0.0, 0.5, 60), 60, "the body (20.0 to 22.0) goes straight through");
        assert_eq!((s.player_x, s.player_y), (19.5, 35.0));
        // Walls at columns 19 and 21 (gap is column 20 only): the body is
        // two wide; it stops with its bottom edge flush on the walls' top.
        let one = |x: i32, y: i32| if (10..30).contains(&y) && (x == 19 || x == 21) { WALL } else { OPEN };
        let mut s = body_at(level(one), 19.5, 5.0);
        walk(&mut s, 0.0, 0.5, 60);
        assert_eq!(s.player_y, 7.0, "bottom edge flush at row 10, not inside the gap");
    }

    /// A wall that ends: walking east along a wall's end, the body slides
    /// round the corner by 0.1 per step instead of stopping on it, and
    /// continues east once clear. (Wall column 22, rows 0..=19.)
    #[test]
    fn the_body_slides_round_a_corner_by_a_tenth_of_a_tile() {
        let lvl = || level(|x, y| if x == 22 && y < 20 { WALL } else { OPEN });
        // The right edge reaches column 22 at x = 19.5. The upper probe (y +
        // 1.025 = 19.525) is in the wall, the rest are free: nudge down.
        let mut s = body_at(lvl(), 19.5, 18.5);
        assert!(s.try_move(0.5, 0.0).is_some());
        assert_eq!((s.player_x, s.player_y), (19.5, 18.6), "nudged 0.1 down, not moved east");
        // Four more nudges clear the wall (18.9 -> 19.0 by the snap) ...
        walk(&mut s, 0.5, 0.0, 4);
        assert_eq!((s.player_x, s.player_y), (19.5, 19.0));
        // ... and then the walk goes east through the opening.
        assert_eq!(walk(&mut s, 0.5, 0.0, 10), 10);
        assert_eq!(s.player_x, 24.5);
        // Mirror: the wall's end is below, the lower probe is the one blocked:
        // nudge up (-0.1). Wall column 22 rows 21..; the body's lower edge
        // y + 2.975 = 21.475 at y = 18.5.
        let mut s = body_at(level(|x, y| if x == 22 && y >= 21 { WALL } else { OPEN }), 19.5, 18.5);
        assert!(s.try_move(0.5, 0.0).is_some());
        assert_eq!((s.player_x, s.player_y), (19.5, 18.4), "nudged 0.1 up");
        walk(&mut s, 0.5, 0.0, 30);
        assert_eq!(s.player_y, 18.0);
        assert!(s.player_x > 22.0, "x = {}", s.player_x);
    }

    /// A rock that blocks the middle of the leading edge, not an end of it,
    /// is not slid round: the body stops.
    #[test]
    fn a_rock_straight_ahead_does_not_nudge() {
        // One wall tile (30, 21). A body at x = 27.5 going east reads the
        // column x = 30.0 at rows y + 1.025 .. y + 2.975 (20.525 to 22.475
        // at y = 19.5): point 0 (row 20) is free, point 1 (row 21) is blocked.
        let mut s = body_at(level(|x, y| if (x, y) == (30, 21) { WALL } else { OPEN }), 27.5, 19.5);
        walk(&mut s, 0.5, 0.0, 10);
        assert_eq!((s.player_x, s.player_y), (27.5, 19.5));
        // Point 0 and point 3 both in a two-row rock: no nudge either.
        let mut s = body_at(rock_at(30, 20), 27.5, 19.5);
        walk(&mut s, 0.5, 0.0, 10);
        assert_eq!((s.player_x, s.player_y), (27.5, 19.5));
    }

    /// A position within 0.0625 of a tile line is pulled onto it by any
    /// step, accepted or not (x on the body's left edge, x + 0.5).
    #[test]
    fn positions_near_a_tile_line_snap_to_it() {
        let mut s = body_at(level(|_, _| OPEN), 10.46, 10.04);
        assert!(s.try_move(0.0, 0.5).is_some());
        assert_eq!((s.player_x, s.player_y), (10.5, 10.54), "x 10.46 -> 10.5; y 10.04 + 0.5 = 10.54 is not near a line");
        let mut s = body_at(level(|_, _| OPEN), 10.0, 10.02);
        s.try_move(0.5, 0.0);
        assert_eq!((s.player_x, s.player_y), (10.5, 10.0), "y 10.02 -> 10.0");
        // 0.1 is further than 0.0625 and stays.
        let mut s = body_at(level(|_, _| OPEN), 10.0, 10.1);
        s.try_move(0.5, 0.0);
        assert_eq!((s.player_x, s.player_y), (10.5, 10.1));
    }

    /// The floor and fractional-part helpers: the client floors `v + 0.0001`.
    #[test]
    fn floor_and_frac_follow_the_client() {
        assert_eq!(walk_floor(2.99995), 3.0);
        assert_eq!(walk_floor(2.9998), 2.0);
        assert_eq!(walk_floor(-0.00005), 0.0);
        assert_eq!(walk_frac(5.0), 0.0);
        assert_eq!(walk_frac(4.99995), 0.0);
        assert!((walk_frac(4.25) - 0.25).abs() < 1e-12);
    }

    /// A long scripted walk through a field of rocks, nudges and snaps
    /// included, never leaves the body overlapping a wall.
    #[test]
    fn walking_in_every_direction_never_enters_a_rock() {
        let tiles = |x: i32, y: i32| if (x * 7 + y * 13) % 5 == 0 { WALL } else { OPEN };
        let mut s = body_at(level(|x, y| if (6..40).contains(&x) && (6..40).contains(&y) { tiles(x, y) } else { OPEN }), 2.0, 2.0);
        let dirs = [(0.5, 0.0), (0.0, 0.5), (-0.5, 0.0), (0.0, -0.5), (0.5, 0.5), (-0.5, 0.5), (0.5, -0.5)];
        for i in 0..6000usize {
            let (dx, dy) = dirs[(i / 5 + i / 29) % dirs.len()];
            s.try_move(dx, dy);
            assert_eq!(s.body_overlap(s.player_x as f64, s.player_y as f64), 0, "step {i} at ({}, {})", s.player_x, s.player_y);
        }
    }

    #[test]
    fn chests_block_the_feet() {
        let chest = crate::assets::Chest { x: 20, y: 10, item: "greenrupee".into(), sign_index: -1, opened: false };
        // The body (rows 9.0 + 1.0 to + 3.0) covers the chest's rows 10 to 12
        // with its whole right edge: it stops flush against it (edge 20.0).
        let mut s = body_at(level(|_, _| OPEN), 16.5, 9.0);
        s.current_level.as_mut().unwrap().chests.push(chest.clone());
        assert_eq!(walk(&mut s, 0.5, 0.0, 10), 2);
        assert_eq!(s.player_x, 17.5);
        // The chest record covers its edges too: going up under it, the probe
        // on the chest's bottom line (y + 1 = 12.0) is blocked, so the body
        // from 11.5 only closes up to flush (11.0).
        let mut s = body_at(level(|_, _| OPEN), 19.5, 11.5);
        s.current_level.as_mut().unwrap().chests.push(chest);
        assert_eq!(walk(&mut s, 0.0, -0.5, 10), 1);
        assert_eq!(s.player_y, 11.0);
    }

    #[test]
    fn dev_mode_is_off_at_start_and_walks_through_walls() {
        let mut s = body_at(level(|x, _| if x == 20 { WALL } else { OPEN }), 17.5, 10.0);
        assert!(!s.dev.active());
        assert!(s.try_move(0.5, 0.0).is_none());
        if DevMode::AVAILABLE {
            s.dev.toggle();
            assert!(s.try_move(0.5, 0.0).is_some());
            // a warp or level change switches it off again
            assert!(s.arrive(10.0, 10.0));
            assert!(!s.dev.active());
        }
    }

    fn link(dest: &str, x: i32, y: i32, w: i32, h: i32, nx: Option<f32>, ny: Option<f32>) -> Link {
        Link { dest: dest.into(), x, y, width: w, height: h, new_x: nx, new_y: ny, x_offset: 0.0, y_offset: 0.0 }
    }

    #[test]
    fn link_uses_its_destination_and_keeps_player_axes() {
        let mut lvl = level(|_, _| OPEN);
        lvl.links.push(link("b.nw", 40, 0, 4, 64, Some(1.0), None)); // x = 1, y = playery
        lvl.links.push(link("c.nw", 0, 50, 64, 2, None, Some(2.5))); // x = playerx, y = 2.5
        let mut s = session_at(lvl, 41.0, 20.0);
        assert!(s.check_links());
        assert_eq!(s.pending_link, Some(("b.nw".into(), 1.0, s.player_y)));
        let mut s2 = session_at(s.current_level.clone().unwrap(), 12.0, 50.5);
        assert!(s2.check_links());
        assert_eq!(s2.pending_link, Some(("c.nw".into(), s2.player_x, 2.5)));
    }

    /// A 3 x 3 big map with `mid.nw` in the middle.
    fn big3() -> crate::gmap::Gmap {
        crate::gmap::Gmap::parse_grid("\"nw.nw\",\"n.nw\",\"ne.nw\"\r\n\"w.nw\",\"mid.nw\",\"e.nw\"\r\n\"sw.nw\",\"s.nw\",\"se.nw\"\r\n").unwrap()
    }

    #[test]
    fn an_edge_link_to_the_big_map_neighbour_keeps_the_offset_across_the_seam() {
        let m = big3();
        // East edge, the link says x = 0 and y = playery: the 2.05 client
        // uses x - 64 instead (held at -0.5, where its own crossing lands).
        let east = link("e.nw", 63, 0, 1, 64, Some(0.0), None);
        assert_eq!(edge_link_arrival(&m, "mid.nw", &east, 63.75, 20.0), Some((-0.25, 20.0)));
        assert_eq!(edge_link_arrival(&m, "mid.nw", &east, 61.5, 20.0), Some((-0.5, 20.0)));
        // A playerx link on the east edge does not land on the far edge.
        let east_px = link("E.NW", 63, 10, 1, 20, None, None);
        assert_eq!(edge_link_arrival(&m, "MID.nw", &east_px, 63.75, 12.0), Some((-0.25, 12.0)));
        let west = link("w.nw", 0, 0, 1, 64, Some(61.0), None);
        assert_eq!(edge_link_arrival(&m, "mid.nw", &west, -2.75, 7.0), Some((61.25, 7.0)));
        assert_eq!(edge_link_arrival(&m, "mid.nw", &west, 0.5, 7.0), Some((61.5, 7.0)));
        let north = link("n.nw", 0, 0, 64, 1, None, Some(61.0));
        assert_eq!(edge_link_arrival(&m, "mid.nw", &north, 9.0, -3.5), Some((9.0, 60.5)));
        let south = link("s.nw", 0, 63, 64, 1, None, Some(0.0));
        assert_eq!(edge_link_arrival(&m, "mid.nw", &south, 9.0, 63.5), Some((9.0, -0.5)));
        assert_eq!(edge_link_arrival(&m, "mid.nw", &south, 9.0, 61.0), Some((9.0, -1.0)));
    }

    #[test]
    fn other_links_keep_their_own_arrival() {
        let m = big3();
        // Not the neighbour on that side, not one tile wide, or no big-map
        // cell for the level: the link's own coordinates.
        assert_eq!(edge_link_arrival(&m, "mid.nw", &link("w.nw", 63, 0, 1, 64, Some(0.0), None), 63.5, 5.0), None);
        assert_eq!(edge_link_arrival(&m, "mid.nw", &link("e.nw", 62, 0, 2, 64, Some(0.0), None), 63.5, 5.0), None);
        assert_eq!(edge_link_arrival(&m, "house.nw", &link("e.nw", 63, 0, 1, 64, Some(0.0), None), 63.5, 5.0), None);
        // The corner level is not a side neighbour.
        assert_eq!(edge_link_arrival(&m, "mid.nw", &link("ne.nw", 63, 0, 1, 64, Some(0.0), None), 63.5, 5.0), None);
    }

    /// Laundry item 9: walking east to the edge link of `mid.nw` on a big
    /// map, `e.nw` is asked for while the player is still well inside the
    /// level, and nothing more is asked when the link fires. Each tick does
    /// what the main loop does: step, check the links, then the ahead
    /// request (so a request in the link's own tick would count as late).
    #[test]
    fn a_big_map_neighbour_is_asked_for_before_its_edge_link_fires() {
        let mut lvl = level(|_, _| OPEN);
        lvl.links.push(link("e.nw", 63, 0, 1, 64, Some(0.0), None));
        let mut s = session_at(lvl, 30.0, 20.0);
        s.loaded_file = Some("mid.nw".into());
        let map = big3();
        let mut asked = std::collections::HashSet::new();
        let (mut asked_at, mut link_at) = (None, None);
        for step in 0..200 {
            s.try_move(0.5, 0.0);
            if s.check_links() {
                link_at = Some(step);
                assert_eq!(s.pending_link.as_ref().map(|l| l.0.as_str()), Some("e.nw"));
                assert!(map.big_map_ahead("mid.nw", s.player_x, s.player_y, &mut asked).is_empty(), "nothing new at the link");
                break;
            }
            let ahead = map.big_map_ahead("mid.nw", s.player_x, s.player_y, &mut asked);
            if !ahead.is_empty() {
                assert_eq!(ahead, ["e.nw"]);
                assert!(asked_at.is_none(), "asked once");
                asked_at = Some(step);
            }
        }
        let (asked_at, link_at) = (asked_at.expect("asked ahead"), link_at.expect("the link fired"));
        // Half-tile steps: at least five tiles (ten steps, 500 ms) ahead.
        assert!(link_at - asked_at >= 10, "asked at step {asked_at}, link at {link_at}");
    }

    /// The ahead request covers the corner level too, and a bare big-map
    /// name gets `.nw`.
    #[test]
    fn big_map_ahead_names_files_once() {
        let map = crate::gmap::Gmap::parse_grid("\"mid\",\"e\"\r\n\"s\",\"se\"\r\n").unwrap();
        let mut asked = std::collections::HashSet::new();
        assert_eq!(map.big_map_ahead("mid", 60.0, 60.0, &mut asked), ["e.nw", "s.nw", "se.nw"]);
        assert!(map.big_map_ahead("MID", 61.0, 61.0, &mut asked).is_empty());
    }

    #[test]
    fn check_links_uses_the_big_map_rule_and_falls_back_without_one() {
        let mut lvl = level(|_, _| OPEN);
        lvl.links.push(link("e.nw", 63, 0, 1, 64, None, None));
        // Feet on tile 63: the player's own x is 61.5.
        let mut s = session_at(lvl, 63.5, 20.0);
        s.loaded_file = Some("mid.nw".into());
        assert!(s.check_links());
        assert_eq!(s.pending_link, Some(("e.nw".into(), 61.5, 17.25)), "without a big map: playerx");
        s.big_map = Some(big3());
        assert!(s.check_links());
        assert_eq!(s.pending_link, Some(("e.nw".into(), -0.5, 17.25)));
    }

    /// The server's board with no tile types (`--no-builtin`, nothing
    /// cached): off the board is still a wall, as in the 2.22 client's wall
    /// test (bounds first). The Windows lab run walked to y 75 in `new1.nw`.
    #[test]
    fn off_the_board_is_a_wall_without_tile_types() {
        let mut s = body_at(level(|_, _| OPEN), 30.0, 32.0);
        s.tiletypes = None;
        walk(&mut s, 0.0, 0.5, 200);
        assert!(s.player_y <= 61.0, "south: stopped at the edge, got {}", s.player_y);
        walk(&mut s, 0.5, 0.0, 200);
        assert!(s.player_x <= 61.5, "east: got {}", s.player_x);
        walk(&mut s, 0.0, -0.5, 200);
        assert!(s.player_y >= -1.0, "north: got {}", s.player_y);
        walk(&mut s, -0.5, 0.0, 200);
        assert!(s.player_x >= -0.5, "west: got {}", s.player_x);
    }

    /// The lab's `new1.nw`: the server's board, its one west-edge link from
    /// `PLO_LEVELLINK`, no tile types, 2.22 link boxes. Walking west from the
    /// start fires the link, with the link's own arrival x.
    #[test]
    fn a_server_edge_link_fires_without_tile_types() {
        let mut lvl = level(|_, _| OPEN);
        lvl.add_server_links(&["testworld.nw 0 0 1 64 61 playery".to_string()]);
        let mut s = body_at(lvl, 30.0, 32.0);
        s.tiletypes = None;
        s.link_boxes = true;
        let mut fired = None;
        for _ in 0..200 {
            if s.try_move(-0.5, 0.0).is_some() && s.check_links() {
                fired = s.pending_link.take();
                break;
            }
        }
        let (dest, x, y) = fired.expect("the west edge link fires");
        assert_eq!((dest.as_str(), x, y), ("testworld.nw", 61.0, 32.0));
    }

    /// A Z3-V1 level read from disk drives the same link rules: a negative
    /// arrival (`-1` in the file) keeps the player's coordinate on that axis.
    #[test]
    fn zelda_level_links_work_in_the_engine() {
        use crate::assets::{ZeldaLevel, ZeldaLink, ZeldaMagic};
        let z = ZeldaLevel {
            magic: ZeldaMagic::V103,
            board: vec![0; 4096],
            links: vec![
                ZeldaLink { dest: "b.zelda".into(), x: 40.0, y: 0.0, width: 4.0, height: 64.0, new_x: 1.5, new_y: -1.0 },
                ZeldaLink { dest: "c.zelda".into(), x: 0.0, y: 50.0, width: 64.0, height: 2.0, new_x: -1.0, new_y: 2.0 },
            ],
            computers: vec![],
            signs: vec![],
            layout: Default::default(),
        };
        let dir = std::env::temp_dir().join(format!("seihai-zelda-engine-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.zelda");
        std::fs::write(&path, z.to_bytes().unwrap()).unwrap();
        let mut s = session_at(level(|_, _| OPEN), 41.0, 20.0);
        s.load_level(&path).unwrap();
        s.armed = true;
        assert!(s.check_links());
        assert_eq!(s.pending_link, Some(("b.zelda".into(), 1.5, s.player_y)));
        let mut s2 = session_at(level(|_, _| OPEN), 12.0, 50.5);
        s2.load_level(&path).unwrap();
        s2.armed = true;
        assert!(s2.check_links());
        assert_eq!(s2.pending_link, Some(("c.zelda".into(), s2.player_x, 2.0)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// B has a link back to A across its west edge, and the arrival spot
    /// from A's link is inside it.
    fn level_b() -> Level {
        let mut b = level(|_, _| OPEN);
        b.links.push(link("a.nw", 0, 10, 4, 6, Some(60.0), None));
        b
    }

    #[test]
    fn arriving_inside_a_link_does_not_warp_straight_back() {
        let mut s = session_at(level_b(), 40.0, 40.0);
        // The server puts the player at x = 1, y = 12: feet at (3.0, 14.75),
        // inside the return link.
        s.begin_warp("b.nw");
        assert!(!s.check_links(), "no link fires while a warp is pending");
        assert!(s.try_move(0.5, 0.0).is_none(), "and nothing walks");
        s.arrive(1.0, 12.0);
        assert_eq!((s.player_x, s.player_y), (1.0, 12.0), "arrival is the destination, exactly");
        assert!(s.warp_pending.is_none());
        assert!(!s.check_links(), "not before the player has moved");
        // Holding the key toward the edge keeps the player in that link.
        for _ in 0..6 {
            s.try_move(-0.5, 0.0);
            assert!(!s.check_links());
        }
        assert!(s.player_x <= 0.0, "the left edge (x + 0.5) stops at the board edge");
    }

    #[test]
    fn stepping_off_the_arrival_link_and_back_on_warps() {
        let mut s = session_at(level_b(), 40.0, 40.0);
        s.arrive(1.0, 12.0); // feet (3.0, 14.75)
        assert_eq!(walk(&mut s, 0.5, 0.0, 2), 2); // feet x 4.0: out of the link
        assert!(!s.check_links());
        assert!(s.try_move(-0.5, 0.0).is_some()); // feet x 3.5: back in
        assert!(s.check_links());
        assert_eq!(s.pending_link, Some(("a.nw".into(), 60.0, s.player_y)));
    }

    #[test]
    fn a_link_elsewhere_still_fires_after_the_first_move() {
        let mut b = level_b();
        b.links.push(link("c.nw", 30, 0, 4, 64, Some(5.0), Some(5.0)));
        let mut s = session_at(b, 28.0, 30.0);
        s.arrive(26.0, 27.25);
        assert!(!s.check_links());
        assert_eq!(walk(&mut s, 0.5, 0.0, 4), 4); // feet x 30.0
        assert!(s.check_links());
        assert_eq!(s.pending_link.as_ref().map(|l| l.0.as_str()), Some("c.nw"));
    }

    /// A level with the lab's links: a rectangle (10, 28, 2 by 5) and the
    /// four edge strips, and a session with probe boxes on that walks from
    /// `(x, y)` in direction `(dx, dy)`; returns the position the first link
    /// fires at, or `None` after `steps` steps.
    fn first_fire(x: f32, y: f32, dx: f32, dy: f32, steps: usize) -> Option<(f32, f32)> {
        let mut lvl = level(|_, _| OPEN);
        lvl.links.push(link("lv_b.nw", 0, 0, 64, 2, None, Some(60.0)));
        lvl.links.push(link("lv_c.nw", 10, 28, 2, 5, Some(60.0), Some(30.0)));
        lvl.links.push(link("lv_w.nw", 0, 62, 64, 2, None, Some(4.0)));
        lvl.links.push(link("lv_s.nw", 62, 0, 2, 64, Some(4.0), None));
        let mut s = session_at(lvl, 0.0, 0.0);
        s.link_boxes = true;
        s.player_x = x;
        s.player_y = y;
        s.arrive(x, y);
        for _ in 0..steps {
            s.try_move(dx, dy)?;
            if s.check_links() {
                return Some((s.player_x, s.player_y));
            }
        }
        None
    }

    // The positions below are the real 2.22 and 2.31 clients' first warps in
    // the lab's `linkgeom` and `linkgeom2` scenarios (the player is put on a
    // half tile and walks): the warp goes out at the step *after* the last
    // position the client reported.

    #[test]
    fn probe_boxes_fire_where_the_real_client_does_on_the_rectangle_link() {
        // Down at x 8.5 ... 10.5 reached the link after reporting y 24.5.
        for x in [8.5, 9.0, 9.5, 10.0, 10.5] {
            assert_eq!(first_fire(x, 20.0, 0.0, 0.5, 40), Some((x, 25.0)), "down at x {x}");
        }
        // Up at x 8.5 and 9.5 from y 40 went on after y 32.5.
        for x in [8.5, 9.5] {
            assert_eq!(first_fire(x, 40.0, 0.0, -0.5, 40), Some((x, 32.0)), "up at x {x}");
        }
        // Left from x 14 after x 12; right from x 6 after x 7.
        for y in [26.0, 26.5, 27.0, 27.5] {
            assert_eq!(first_fire(14.0, y, -0.5, 0.0, 40), Some((11.5, y)), "left at y {y}");
        }
        for y in [26.5, 27.0, 27.5] {
            assert_eq!(first_fire(6.0, y, 0.5, 0.0, 40), Some((7.5, y)), "right at y {y}");
        }
    }

    #[test]
    fn probe_boxes_miss_the_rectangle_link_where_the_real_client_does() {
        // Beside the link (x 8.0 and 11.0 or more going down; 7 and 7.5 and
        // 11 to 12 in the first lab run).
        for x in [7.0, 7.5, 8.0, 11.0, 11.5, 12.0] {
            assert_eq!(first_fire(x, 20.0, 0.0, 0.5, 20), None, "down at x {x}");
        }
        assert_eq!(first_fire(11.0, 40.0, 0.0, -0.5, 20), None);
        // Left and right below the link's bottom edge (y 32 and 33.5).
        for y in [32.0, 32.5, 33.0, 33.5] {
            assert_eq!(first_fire(14.0, y, -0.5, 0.0, 20), None, "left at y {y}");
        }
        for y in [32.5, 33.0] {
            assert_eq!(first_fire(6.0, y, 0.5, 0.0, 20), None, "right at y {y}");
        }
    }

    #[test]
    fn probe_boxes_fire_where_the_real_client_does_on_the_edges() {
        assert_eq!(first_fire(30.0, 56.0, 0.0, 0.5, 40), Some((30.0, 59.0))); // south edge
        assert_eq!(first_fire(30.0, 6.0, 0.0, -0.5, 40), Some((30.0, 1.0))); // north edge
        assert_eq!(first_fire(56.0, 30.5, 0.5, 0.0, 40), Some((59.5, 30.5))); // east edge
    }

    #[test]
    fn the_foot_point_stays_for_profiles_without_probe_boxes() {
        // The legacy rule passes by the rectangle link at its far edge.
        let mut lvl = level(|_, _| OPEN);
        lvl.links.push(link("lv_c.nw", 10, 28, 2, 5, Some(60.0), Some(30.0)));
        let mut s = session_at(lvl, 0.0, 0.0);
        s.player_x = 8.5;
        s.player_y = 20.0;
        s.arrive(8.5, 20.0);
        assert!(!s.link_boxes);
        let mut fired = None;
        for _ in 0..40 {
            s.try_move(0.0, 0.5);
            if s.check_links() {
                fired = Some(s.player_y);
                break;
            }
        }
        // foot y = y + 2.75 reaches row 28 at y 25.5, and foot x = x + 2 = 10.5
        assert_eq!(fired, Some(25.5));
    }

    #[test]
    fn no_link_while_the_level_is_loading() {
        let mut s = session_at(level_b(), 2.0, 12.0);
        s.arrive(0.0, 9.25);
        s.try_move(0.5, 0.0);
        s.current_level = None;
        assert!(!s.check_links());
    }

    #[test]
    fn a_cancelled_warp_frees_the_player() {
        let mut s = session_at(level(|_, _| OPEN), 10.0, 10.0);
        s.begin_warp("x.nw");
        assert!(s.try_move(0.5, 0.0).is_none());
        s.cancel_warp();
        assert!(s.try_move(0.5, 0.0).is_some());
    }

    // ---- gmap walking ------------------------------------------------------

    use crate::gmap::Gmap;

    const GRID: &str = "GRMAP001\nWIDTH 2\nHEIGHT 2\nLEVELNAMES\n\"nw.nw\",\"ne.nw\",\n\"sw.nw\",\"se.nw\",\nLEVELNAMESEND\n";
    /// A tile id no wall or water test confuses with the open floor: each
    /// cell's level is open floor with its own marker in the top-left tile.
    const MARK: [u16; 4] = [10, 11, 12, 13];

    /// `wall_at` is the top of a run of six wall tiles in its column.
    fn marked(cell: usize, wall_at: Option<(i32, i32)>) -> Level {
        level(move |x, y| {
            if wall_at.is_some_and(|(wx, wy)| x == wx && (wy..wy + 6).contains(&y)) {
                WALL
            } else if (x, y) == (0, 0) {
                MARK[cell]
            } else {
                OPEN
            }
        })
    }

    /// A 2x2 world, the player in the north-west cell with the sprite at
    /// `(x, y)`. `load` says which neighbour cells have a level.
    fn gmap_session(x: f32, y: f32, load: &[(i32, i32)], wall_ne: Option<(i32, i32)>) -> OfflineSession {
        let mut s = OfflineSession::new();
        s.tiletypes = Some(types());
        s.current_level = Some(marked(0, None));
        s.player_x = x;
        s.player_y = y;
        s.enter_gmap("g.gmap".into(), Gmap::parse(GRID).unwrap(), (0, 0));
        let w = s.gmap.as_mut().unwrap();
        for &(cx, cy) in load {
            let i = (cy * 2 + cx) as usize;
            w.levels.insert((cx, cy), marked(i, if (cx, cy) == (1, 0) { wall_ne } else { None }));
        }
        s
    }

    fn marker(s: &OfflineSession) -> Option<u16> {
        s.current_level.as_ref().and_then(|l| l.tile_id_at(0, 0))
    }

    #[test]
    fn walking_east_crosses_into_the_next_level() {
        let mut s = gmap_session(60.0, 30.5, &[(1, 0), (0, 1), (1, 1)], None);
        // 60.0 -> 63.5 is seven half-steps; the eighth reaches 64.0.
        assert_eq!(walk(&mut s, 0.5, 0.0, 7), 7);
        assert_eq!((s.player_x, marker(&s)), (63.5, Some(10)));
        assert!(s.take_gmap_crossing().is_none());
        assert!(s.try_move(0.5, 0.0).is_some());
        let c = s.take_gmap_crossing().expect("the step crossed");
        assert_eq!((c.from, c.to, c.local_x, c.local_y), ((0, 0), (1, 0), 0.0, 30.5));
        assert_eq!((s.player_x, s.player_y), (0.0, 30.5));
        assert_eq!(marker(&s), Some(11), "the north-east level is current");
        assert_eq!(s.gmap.as_ref().unwrap().cell, (1, 0));
        assert!(s.gmap.as_ref().unwrap().levels.contains_key(&(0, 0)), "the old level stays as a neighbour");
        assert!(s.take_gmap_crossing().is_none(), "taken once");
    }

    #[test]
    fn walking_back_west_returns_with_a_wrapped_position() {
        let mut s = gmap_session(63.0, 30.5, &[(1, 0), (0, 1), (1, 1)], None);
        walk(&mut s, 0.5, 0.0, 2);
        s.take_gmap_crossing().unwrap();
        assert_eq!((s.player_x, marker(&s)), (0.0, Some(11)));
        assert!(s.try_move(-0.5, 0.0).is_some());
        let c = s.take_gmap_crossing().unwrap();
        assert_eq!((c.to, c.local_x), ((0, 0), 63.5));
        assert_eq!((s.player_x, marker(&s)), (63.5, Some(10)));
    }

    #[test]
    fn walking_south_and_north_cross_the_y_edge() {
        let mut s = gmap_session(30.5, 63.0, &[(1, 0), (0, 1), (1, 1)], None);
        walk(&mut s, 0.0, 0.5, 2);
        let c = s.take_gmap_crossing().unwrap();
        assert_eq!((c.to, c.local_y, c.y_changed(), c.x_changed()), ((0, 1), 0.0, true, false));
        assert_eq!(marker(&s), Some(12));
        s.try_move(0.0, -0.5);
        let c = s.take_gmap_crossing().unwrap();
        assert_eq!((c.to, c.local_y), ((0, 0), 63.5));
    }

    #[test]
    fn the_grid_edge_stops_the_walk() {
        let mut s = gmap_session(61.0, 30.5, &[(1, 0), (0, 1), (1, 1)], None);
        walk(&mut s, 0.5, 0.0, 6);
        s.take_gmap_crossing().unwrap();
        // Now in the east column: nothing further east. The body's right
        // edge (x + 2.5) reaches the board's edge 64.0 (sprite 61.5) and stops.
        walk(&mut s, 0.5, 0.0, 200);
        assert_eq!(s.player_x, 61.5);
        assert!(s.take_gmap_crossing().is_none());
        // Same at the north edge.
        let mut s = gmap_session(30.5, 1.0, &[(1, 0), (0, 1), (1, 1)], None);
        walk(&mut s, 0.0, -0.5, 20);
        assert!(s.take_gmap_crossing().is_none());
        assert_eq!(s.player_y, 0.0);
    }

    #[test]
    fn a_neighbour_that_is_not_loaded_blocks() {
        let mut s = gmap_session(60.0, 30.5, &[], None);
        walk(&mut s, 0.5, 0.0, 20);
        assert!(s.take_gmap_crossing().is_none());
        assert_eq!(s.player_x, 61.5, "stops with the body edge flush at the seam");
        assert_eq!(marker(&s), Some(10));
        // Once it is loaded the walk goes on.
        s.gmap.as_mut().unwrap().levels.insert((1, 0), marked(1, None));
        walk(&mut s, 0.5, 0.0, 20);
        assert!(s.take_gmap_crossing().is_some());
    }

    /// The feet test reads the neighbour's tiles across the seam, so a wall
    /// in column 0 of the next level stops the player in this one.
    #[test]
    fn a_wall_across_the_seam_blocks() {
        let mut s = gmap_session(60.0, 30.5, &[(1, 0), (0, 1), (1, 1)], Some((0, 31)));
        // The body spans rows 31.5 to 33.5, all in the wall's rows 31 to 36.
        walk(&mut s, 0.5, 0.0, 20);
        assert_eq!(s.player_x, 61.5);
        assert!(s.take_gmap_crossing().is_none());
        // Well above it the same walk is free.
        let mut s = gmap_session(60.0, 27.5, &[(1, 0), (0, 1), (1, 1)], Some((0, 31)));
        walk(&mut s, 0.5, 0.0, 8);
        assert!(s.take_gmap_crossing().is_some());
    }

    #[test]
    fn water_across_the_seam_is_read_from_the_neighbour() {
        let mut s = gmap_session(60.0, 30.5, &[(1, 0), (0, 1), (1, 1)], None);
        s.gmap.as_mut().unwrap().levels.insert((1, 0), level(|_, _| WATER));
        walk(&mut s, 0.5, 0.0, 4);
        assert!(s.in_water, "feet at 64.0 are in the east level's water");
    }

    /// A step across both edges at once is refused; the walk slides along
    /// one axis, so the player crosses one edge at a time.
    #[test]
    fn a_corner_step_does_not_cross_both_edges() {
        let mut s = gmap_session(63.5, 63.5, &[(1, 0), (0, 1), (1, 1)], None);
        let r = s.try_move(0.5, 0.5);
        assert!(r.is_some(), "it slides");
        let c = s.take_gmap_crossing().unwrap();
        assert!(c.x_changed() != c.y_changed(), "one axis only: {c:?}");
    }

    #[test]
    fn without_a_gmap_the_edge_is_a_wall_as_before() {
        let mut s = session_at(level(|_, _| OPEN), 62.5, 10.5);
        assert!(s.gmap.is_none());
        walk(&mut s, 0.5, 0.0, 10);
        assert!(s.take_gmap_crossing().is_none());
        assert_eq!(s.player_x, 61.5);
    }

    #[test]
    fn crossing_clears_the_opened_chests_and_pending_link() {
        let mut s = gmap_session(63.0, 30.5, &[(1, 0), (0, 1), (1, 1)], None);
        s.opened_chests.insert((3, 3));
        walk(&mut s, 0.5, 0.0, 2);
        assert!(s.opened_chests.is_empty());
    }

    #[test]
    fn level_serial_counts_loaded_levels() {
        let dir = std::env::temp_dir().join(format!("seihai-serial-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.nw");
        std::fs::write(&path, "GLEVNW01\n").unwrap();
        let mut s = OfflineSession::new();
        assert_eq!(s.level_serial, 0);
        s.load_level(&path).unwrap();
        s.load_level(&path).unwrap();
        assert_eq!(s.level_serial, 2);
        assert!(s.load_level(dir.join("missing.nw")).is_err());
        assert_eq!(s.level_serial, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- health and death --------------------------------------------------

    use crate::life::LifeEvent;
    use crate::protocol::DEATH_205;

    fn mortal() -> OfflineSession {
        let mut s = OfflineSession::new();
        s.life.set_rules(Some(DEATH_205));
        s.current_level = Some(level(|_, _| OPEN));
        s.tiletypes = Some(types());
        s
    }

    fn die(s: &mut OfflineSession, attacker: Option<u16>) -> Vec<LifeEvent> {
        assert!(s.damage_half(s.max_hearts, attacker));
        s.tick_life(0)
    }

    #[test]
    fn half_hearts_round_trip_and_never_round_to_zero() {
        let mut s = OfflineSession::new();
        assert_eq!((s.max_hearts, s.hearts_half()), (6, 6));
        for h in 0..=6 {
            s.set_hearts_half(h);
            assert_eq!(s.hearts_half(), h);
        }
        s.set_max_hearts_half(60);
        s.set_hearts_half(1);
        assert!(s.player_health >= 1, "1 of 60 half hearts is still alive");
        assert_eq!(s.hearts_half(), 1);
    }

    #[test]
    fn hearts_zero_starts_the_death_and_faces_down() {
        let mut s = mortal();
        s.player_dir = DIR_EAST;
        assert!(s.damage_half(2, Some(9)));
        assert!(s.tick_life(50).is_empty(), "4 half hearts left");
        let ev = die(&mut s, Some(9));
        assert_eq!(ev, vec![LifeEvent::Died { attacker: Some(9) }]);
        assert!(s.life.is_dead());
        assert_eq!(s.player_dir, DIR_SOUTH);
    }

    #[test]
    fn no_rules_means_no_death() {
        let mut s = OfflineSession::new();
        s.damage_half(100, Some(1));
        assert!(s.tick_life(5000).is_empty());
        assert!(!s.life.is_dead() && !s.controls_locked());
        assert_eq!(s.player_health, 0);
    }

    #[test]
    fn nothing_moves_or_interacts_while_dead() {
        let mut s = mortal();
        die(&mut s, None);
        let (x, y) = (s.player_x, s.player_y);
        assert!(s.try_move(0.5, 0.0).is_none());
        assert_eq!((s.player_x, s.player_y), (x, y));
        assert_eq!(s.interact(), Interaction::Nothing);
        assert!(s.controls_locked());
        // The lock is over after the timer passes 27 ticks (1.4 s).
        s.tick_life(27 * 50);
        assert!(s.controls_locked());
        s.tick_life(50);
        assert!(!s.controls_locked());
        assert!(s.try_move(0.5, 0.0).is_none(), "dead until a respawn, even unlocked");
    }

    #[test]
    fn a_dead_player_cannot_be_hurt() {
        let mut s = mortal();
        die(&mut s, None);
        assert!(!s.damage_half(2, Some(3)));
    }

    #[test]
    fn respawn_waits_for_the_lock() {
        let mut s = mortal();
        die(&mut s, None);
        s.tick_life(27 * 50);
        assert!(s.respawn(true).is_none());
        assert!(s.life.is_dead());
    }

    #[test]
    fn respawn_hearts_follow_ap() {
        // (ap, max half hearts) -> half hearts after the respawn.
        for (ap, max, want) in [(0, 20, 6), (19, 20, 6), (20, 20, 10), (39, 8, 8), (40, 20, 20), (100, 12, 12)] {
            let mut s = mortal();
            s.set_max_hearts_half(max);
            s.set_hearts_half(max);
            s.alignment = ap;
            die(&mut s, None);
            s.tick_life(28 * 50);
            let _ = s.life.take_events();
            assert!(s.respawn(false).is_some(), "ap {ap}");
            assert_eq!(s.hearts_half(), want, "ap {ap} max {max}");
            assert!(!s.life.is_dead());
        }
    }

    #[test]
    fn respawn_goes_to_the_saved_entry_point() {
        let mut s = mortal();
        s.player_x = 12.5;
        s.player_y = 20.0;
        s.player_dir = DIR_WEST;
        s.note_entry("a.nw");
        s.player_x = 40.0;
        s.player_y = 41.0;
        die(&mut s, None);
        s.tick_life(28 * 50);
        let to = s.respawn(true).unwrap();
        let want = EntryPoint { level: "a.nw".into(), x: 12.5, y: 20.0, dir: DIR_WEST };
        assert_eq!(to, RespawnTo::Saved(want));
        assert_eq!((s.player_x, s.player_y, s.player_dir), (12.5, 20.0, DIR_WEST));
        // A later entry of the same name group (plain names share the empty
        // group) does not replace it in the 2.05 rules.
        s.player_x = 3.0;
        s.note_entry("b.nw");
        assert_eq!(s.entry.as_ref().map(|e| (e.level.as_str(), e.x)), Some(("a.nw", 12.5)));
        // A different group does.
        s.note_entry("cave$1.nw");
        assert_eq!(s.entry.as_ref().map(|e| (e.level.as_str(), e.x)), Some(("cave$1.nw", 3.0)));
    }

    #[test]
    fn name_groups_are_the_name_up_to_the_last_delimiter() {
        assert_eq!(name_group("Cave$1.nw"), "cave$");
        assert_eq!(name_group("cave$2.nw"), "cave$");
        assert_eq!(name_group("a§b§c.nw"), "a§b§");
        assert_eq!(name_group("levels/Town§x.graal"), "town§");
        assert_eq!(name_group("town.nw"), "", "no delimiter: the empty group");
        assert_eq!(name_group("a.nw"), name_group("b.nw"));
        assert_ne!(name_group("a$1.nw"), name_group("b$1.nw"));
    }

    fn gmap_world(names: &str) -> crate::gmap::GmapWorld {
        let text = format!("GRMAP001\nWIDTH 2\nHEIGHT 1\nLEVELNAMES\n{names}\nLEVELNAMESEND\n");
        crate::gmap::GmapWorld::new("w.gmap".into(), crate::gmap::Gmap::parse(&text).unwrap(), (0, 0))
    }

    #[test]
    fn the_first_entry_is_always_saved() {
        let mut s = mortal();
        s.player_x = 7.0;
        s.note_entry("a.nw");
        assert_eq!(s.entry.as_ref().map(|e| e.x), Some(7.0));
    }

    #[test]
    fn entering_or_leaving_a_gmap_saves_even_in_the_same_name_group() {
        // The list is [first, second]: "second" is at index 1, "outside" is not in it.
        let mut s = mortal();
        s.gmap = Some(gmap_world("first.nw,second.nw"));
        s.player_x = 1.0;
        s.note_entry("outside.nw");
        s.player_x = 2.0;
        s.note_entry("second.nw");
        assert_eq!(s.entry.as_ref().map(|e| (e.level.as_str(), e.x)), Some(("second.nw", 2.0)), "into the gmap");
        s.player_x = 3.0;
        s.note_entry("outside.nw");
        assert_eq!(s.entry.as_ref().map(|e| (e.level.as_str(), e.x)), Some(("outside.nw", 3.0)), "out of it");
        // Both in the list (indices 0 and 1, or 1 and 1): the product is not
        // negative, so the group test decides: same group, not saved.
        s.player_x = 4.0;
        s.note_entry("second.nw");
        s.player_x = 5.0;
        s.note_entry("first.nw");
        assert_eq!(s.entry.as_ref().map(|e| e.x), Some(4.0));
    }

    #[test]
    fn index_zero_of_the_gmap_list_counts_as_the_same_side() {
        // The client multiplies the indices; 0 times -1 is 0, not negative.
        let mut s = mortal();
        s.gmap = Some(gmap_world("first.nw,second.nw"));
        s.player_x = 1.0;
        s.note_entry("outside.nw");
        s.player_x = 2.0;
        s.note_entry("first.nw");
        assert_eq!(s.entry.as_ref().map(|e| e.x), Some(1.0), "the quirk: entering at index 0 does not save");
    }

    #[test]
    fn later_builds_have_no_name_group_test() {
        use crate::protocol::DEATH_21;
        let mut s = OfflineSession::new();
        s.life.set_rules(Some(DEATH_21));
        s.player_x = 1.0;
        s.note_entry("a$1.nw");
        s.player_x = 2.0;
        s.note_entry("b$1.nw");
        assert_eq!(s.entry.as_ref().map(|e| e.x), Some(1.0), "different groups, still not saved");
    }

    #[test]
    fn the_name_group_test_is_the_2_00_to_2_05_builds_only() {
        use crate::protocol::ClientVersion as V;
        for v in [V::V200, V::V201, V::V202, V::V202R2, V::V203, V::V204, V::V205] {
            assert!(v.death_rules().unwrap().entry_name_group, "{}", v.label());
        }
        for v in [V::V217, V::V222, V::V231] {
            assert!(!v.death_rules().unwrap().entry_name_group, "{}", v.label());
        }
        for l in ["2.1", "2.12", "2.13", "2.14", "2.15", "2.16", "2.3"] {
            let p = crate::protocol::by_label(l).unwrap_or_else(|| panic!("{l}"));
            assert!(!p.life.death.unwrap().entry_name_group, "{l}");
        }
    }

    #[test]
    fn respawn_without_an_entry_point() {
        let mut s = mortal();
        die(&mut s, None);
        s.tick_life(28 * 50);
        s.player_x = 5.0;
        assert_eq!(s.respawn(true), Some(RespawnTo::Default { x: 30.5, y: 30.0 }));
        assert_eq!((s.player_x, s.player_y, s.player_dir), (30.5, 30.0, DIR_SOUTH));
        let mut s = mortal();
        die(&mut s, None);
        s.tick_life(28 * 50);
        s.player_x = 5.0;
        assert_eq!(s.respawn(false), Some(RespawnTo::InPlace));
        assert_eq!(s.player_x, 5.0, "offline with nothing saved: not moved");
    }

    #[test]
    fn dying_twice_in_a_row() {
        let mut s = mortal();
        for _ in 0..2 {
            let ev = die(&mut s, Some(4));
            assert_eq!(ev, vec![LifeEvent::Died { attacker: Some(4) }]);
            s.tick_life(28 * 50);
            assert!(s.respawn(false).is_some());
            let _ = s.life.take_events();
        }
    }

    #[test]
    fn server_power_and_ap_apply() {
        let mut s = OfflineSession::new();
        // MAXPOWER 5 hearts, CURPOWER 7 half hearts, AP 60.
        s.apply_server_power(Some(5), Some(7), Some(60));
        assert_eq!((s.max_hearts, s.hearts_half(), s.alignment), (10, 7, 60));
        s.apply_server_power(None, Some(0), None);
        assert_eq!(s.player_health, 0);
        s.apply_server_power(None, None, Some(250));
        assert_eq!(s.alignment, 100);
    }

    #[test]
    fn server_cur_power_zero_kills_and_respawn_frees_controls() {
        let mut s = mortal();
        s.apply_server_power(Some(3), Some(6), Some(50));
        s.apply_server_power(None, Some(0), None);
        assert_eq!(s.tick_life(50), vec![LifeEvent::Died { attacker: None }]);
        assert!(s.controls_locked());
        s.tick_life(28 * 50);
        assert!(!s.controls_locked());
        assert!(s.respawn(false).is_some());
        assert_eq!(s.hearts_half(), 6, "AP 50: full hearts back");
        assert!(!s.controls_locked());
    }

    #[test]
    fn a_hit_is_reported_with_the_new_hearts_and_hurt() {
        let mut s = mortal();
        s.combat.set_rules(Some(crate::protocol::COMBAT_205));
        s.apply_server_power(Some(3), Some(6), Some(50));
        assert_eq!(s.take_power_report(), None, "the server's own hearts");
        let h = crate::combat::HurtPlayer { attacker: 2, dx: 1.0, dy: 0.0, half: 1, npc: None };
        assert!(s.receive_hurtplayer(&h, Some((s.player_x, s.player_y))).outcome.is_ok());
        let r = s.take_power_report().unwrap();
        assert_eq!(r.cur_half, Some(5));
        assert_eq!(r.pose, Some(crate::life::ReportPose::Hurt));
        assert_eq!(r.sprite, Some(s.sprite_value()));
        // Hurt mode over: idle.
        s.tick_combat(40 * 50);
        let r = s.take_power_report().unwrap();
        assert_eq!((r.cur_half, r.pose), (None, Some(crate::life::ReportPose::Idle)));
    }

    #[test]
    fn death_and_respawn_are_reported() {
        let mut s = mortal();
        s.apply_server_power(Some(3), Some(6), Some(30));
        assert!(s.damage_half(6, None));
        // Before the life tick: already reported as dead, not hurt.
        let r = s.take_power_report().unwrap();
        assert_eq!((r.cur_half, r.pose), (Some(0), Some(crate::life::ReportPose::Dead)));
        s.tick_life(0);
        s.tick_life(28 * 50);
        assert_eq!(s.take_power_report(), None);
        assert!(s.respawn(false).is_some());
        // AP 30: min(5 hearts, max 3 hearts) = 6 half hearts.
        let r = s.take_power_report().unwrap();
        assert_eq!((r.cur_half, r.pose), (Some(6), Some(crate::life::ReportPose::Idle)));
    }

    #[test]
    fn a_lava_burn_is_reported() {
        let mut s = mortal();
        s.apply_server_power(Some(3), Some(6), Some(50));
        s.damage_half(1, None);
        let r = s.take_power_report().unwrap();
        assert_eq!((r.cur_half, r.pose, r.sprite), (Some(5), None, None));
    }

    #[test]
    fn is_loaded_ignores_extension_and_case() {
        let dir = std::env::temp_dir().join(format!("seihai-loaded-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Town.nw"), "GLEVNW01\n").unwrap();
        let mut s = OfflineSession::new();
        assert!(!s.is_loaded("town"));
        s.load_level(dir.join("Town.nw")).unwrap();
        assert!(s.is_loaded("town.nw") && s.is_loaded("TOWN") && !s.is_loaded("other.nw"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- GANI sounds -------------------------------------------------------

    use crate::assets::Gani;

    /// A single-direction gani with `n` frames; frame `k` has a cue for
    /// each `(k, name)` in `cues`.
    fn gani_with(n: usize, cues: &[(usize, &str)]) -> GaniPlayer {
        let mut text = String::from("SINGLEDIRECTION\nANI\n");
        for k in 0..n {
            text.push_str("1 0 0, 2 0 0\n");
            for (at, name) in cues {
                if *at == k {
                    text.push_str(&format!("PLAYSOUND {name} 1.5 2\n"));
                }
            }
            text.push('\n');
        }
        text.push_str("ANIEND\n");
        GaniPlayer::new(Gani::parse(&text).unwrap())
    }

    #[test]
    fn a_gani_frame_with_a_sound_queues_it_once_per_entry() {
        let mut s = session_at(level(|_, _| OPEN), 10.5, 10.5);
        s.set_walk_gani(gani_with(4, &[(2, "steps.wav")]));
        // Frames entered: 1, 2 (cue), 3, 0, 1, 2 (cue).
        walk(&mut s, 0.5, 0.0, 1);
        assert!(s.sound_events.is_empty(), "frame 1 has no cue");
        walk(&mut s, 0.5, 0.0, 1);
        let ev = s.take_sound_events();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].file, "steps.wav");
        // The cue is 1.5, 2 from the player: the listener's own point.
        assert_eq!((ev[0].x, ev[0].y), (s.player_x + 1.5, s.player_y + 2.0));
        assert!(s.take_sound_events().is_empty(), "taken events are gone");
        walk(&mut s, 0.5, 0.0, 3); // 3, 0, 1
        assert!(s.take_sound_events().is_empty());
        walk(&mut s, 0.5, 0.0, 1); // 2 again
        assert_eq!(s.take_sound_events().len(), 1);
    }

    #[test]
    fn a_blocked_move_runs_in_place_without_a_sound() {
        let mut s = body_at(level(|x, _| if x == 20 { WALL } else { OPEN }), 17.5, 10.0);
        s.set_walk_gani(gani_with(4, &[(1, "steps.wav"), (2, "steps.wav")]));
        assert!(s.try_move(0.5, 0.0).is_none());
        assert!(s.try_move(0.5, 0.0).is_none());
        // 2.05 keeps the walk gani going against the wall (the legs run),
        // but plays no footsteps: the player has not moved.
        assert!(s.walking);
        assert_eq!(s.walk_frame(), Some(2));
        assert!(s.sound_events.is_empty());
        assert_eq!(s.player_x, 17.5);
    }

    #[test]
    fn no_gani_means_no_sound_events() {
        let mut s = session_at(level(|_, _| OPEN), 10.5, 10.5);
        walk(&mut s, 0.5, 0.0, 4);
        assert!(s.sound_events.is_empty());
    }

    #[test]
    fn swimming_steps_the_swim_gani_not_the_walk_one() {
        let mut s = session_at(level(|x, _| if (10..=13).contains(&x) { WATER } else { OPEN }), 8.5, 20.5);
        s.set_walk_gani(gani_with(1, &[(0, "steps.wav")]));
        s.set_swim_gani(gani_with(1, &[(0, "stroke.wav")]));
        walk(&mut s, 0.5, 0.0, 1); // 9.0, still on land
        assert_eq!(s.take_sound_events()[0].file, "steps.wav");
        walk(&mut s, 0.5, 0.0, 4); // into the water
        assert!(s.in_water);
        let files: Vec<String> = s.take_sound_events().into_iter().map(|e| e.file).collect();
        assert!(!files.iter().any(|f| f == "stroke.wav"), "the stroke is not stepped by moves: {files:?}");
        // The stroke runs on the tick clock: its cue comes with the tick.
        s.tick_poses(50);
        let files: Vec<String> = s.take_sound_events().into_iter().map(|e| e.file).collect();
        assert_eq!(files, ["stroke.wav"]);
    }

    #[test]
    fn the_swim_stroke_goes_on_while_floating_still() {
        let mut s = session_at(level(|_, _| WATER), 30.5, 30.5);
        // Eight steps of two ticks (WAIT 1), as the stock swim.gani.
        let text = format!("SINGLEDIRECTION\nLOOP\nANI\n{}ANIEND\n", "1 0 0\nWAIT 1\n\n".repeat(8));
        s.set_swim_gani(GaniPlayer::new(Gani::parse(&text).unwrap()));
        s.try_move(0.5, 0.0);
        assert!(s.in_water);
        let frames: Vec<usize> = (0..17).map(|_| {
            s.tick_poses(50);
            s.swim_gani.as_ref().unwrap().frame
        }).collect();
        assert_eq!(&frames[..5], [0, 1, 1, 2, 2]);
        assert_eq!(frames[15], 0, "a 16-tick stroke");
    }

    #[test]
    fn floating_still_the_stroke_runs_restarts_or_freezes_by_build() {
        use crate::protocol::{MANNER_141, MANNER_205, MANNER_222};
        let run = |m| {
            let mut s = session_at(level(|_, _| WATER), 30.5, 30.5);
            s.set_mannerisms(m);
            let text = format!("SINGLEDIRECTION\nLOOP\nANI\n{}ANIEND\n", "1 0 0\nWAIT 1\n\n".repeat(8));
            s.set_swim_gani(GaniPlayer::new(Gani::parse(&text).unwrap()));
            s.try_move(0.5, 0.0);
            for _ in 0..6 {
                s.tick_poses(50);
            }
            s.stop_walking();
            (0..4).map(|_| {
                s.tick_poses(50);
                s.swim_gani.as_ref().unwrap().frame
            }).collect::<Vec<_>>()
        };
        assert_eq!(run(MANNER_205), [3, 4, 4, 5], "2.05 runs on");
        assert_eq!(run(MANNER_222), [0, 0, 1, 1], "2.22 restarts at the stop");
        assert_eq!(run(MANNER_141), [0, 0, 0, 0], "NewMain 1.41 freezes");
    }
    // ---- throwing -----------------------------------------------------

    /// Lift the bush at (20, 20) and stand at `(px, py)` facing `dir`.
    fn carrying_bush(px: f32, py: f32, dir: u8) -> OfflineSession {
        let mut s = object_session(0);
        face_up_at(&mut s, 20, 20);
        assert_eq!(s.interact(), Interaction::Lifted { kind: 0 });
        s.player_x = px;
        s.player_y = py;
        s.player_dir = dir;
        s
    }

    #[test]
    fn the_sword_key_throws_what_is_carried_instead_of_swinging() {
        let mut s = carrying_bush(10.0, 30.0, DIR_EAST);
        s.sword_power = 1;
        s.combat.set_rules(Some(crate::protocol::COMBAT_205));
        assert!(!s.start_swing(), "no swing with something in the hands");
        assert!(!s.combat.is_swinging());
        assert!(s.carried.is_some(), "the refused swing leaves the object carried");
        // The same key press then throws (the key goes on to the interact).
        assert_eq!(s.interact(), Interaction::Thrown { kind: 0 });
        assert!(s.start_swing(), "empty hands swing again");
    }

    #[test]
    fn the_interact_key_throws_what_is_carried_and_nothing_else_when_empty() {
        let mut s = carrying_bush(10.0, 30.0, DIR_EAST);
        assert_eq!(s.interact(), Interaction::Thrown { kind: 0 });
        assert!(s.carried.is_none());
        assert_eq!(s.thrown.len(), 1);
        let o = &s.thrown[0];
        assert_eq!((o.x, o.y, o.ground, o.dir, o.ticks_left), (10.5, 28.625, 31.0, DIR_EAST, 9));
        // Empty hands, nothing in front: no throw.
        s.thrown.clear();
        s.player_x = 5.0;
        s.player_y = 5.0;
        assert_eq!(s.interact(), Interaction::Nothing);
        assert!(s.thrown.is_empty());
        assert_eq!(s.throw_carried(), None);
    }

    /// A player at (14, 30) with three hearts and the 2.05 rules, and
    /// another player's bush thrown east from (10, 30) by player 7.
    fn pelted() -> OfflineSession {
        let mut s = object_session(0);
        s.combat.set_rules(Some(crate::protocol::COMBAT_205));
        s.life.set_rules(Some(crate::protocol::DEATH_205));
        s.set_max_hearts_half(6);
        s.set_hearts_half(6);
        s.player_x = 14.0;
        s.player_y = 30.0;
        s.throw_remote(261, 10.0, 30.0, DIR_EAST, 7);
        s
    }

    #[test]
    fn another_players_throw_hits_the_player_for_a_heart_and_pushes_along_its_flight() {
        let mut s = pelted();
        // x starts at 10.5 and moves a tile a tick: 13.5 is the first step
        // under 2 tiles from the player's 14.5.
        s.tick_throw(2 * 50);
        assert_eq!(s.thrown.len(), 1, "two ticks: still in the air");
        assert_eq!(s.hearts_half(), 6);
        s.tick_throw(50);
        assert!(s.thrown.is_empty(), "the hit ends the flight");
        assert_eq!(s.hearts_half(), 4, "one heart");
        assert!(s.combat.is_hurt());
        assert!(s.take_throw_hurt());
        assert!(!s.take_throw_hurt(), "reported once");
        let l = s.take_landings();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].effect, Some(crate::throwing::LEAP_BUSH), "the bush breaks where it hit");
        // The push runs east, the way the bush flew.
        s.tick_combat(50);
        assert!(s.player_x > 14.0);
    }

    #[test]
    fn a_thrown_object_takes_half_a_heart_at_one_heart_and_the_kill_claims_the_thrower() {
        let mut s = pelted();
        s.set_hearts_half(2);
        s.tick_throw(3 * 50);
        assert_eq!(s.hearts_half(), 1, "half a heart at exactly one heart");
        // Out of hurt mode, the next one kills and names player 7.
        s.combat.set_rules(Some(crate::protocol::COMBAT_205));
        s.throw_remote(261, 10.0, 30.0, DIR_EAST, 7);
        s.tick_throw(3 * 50);
        assert_eq!(s.hearts_half(), 0);
        let events = s.tick_life(50);
        assert!(events.contains(&LifeEvent::Died { attacker: Some(7) }), "{events:?}");
    }

    #[test]
    fn the_players_own_throw_never_hits_the_player() {
        let mut s = pelted();
        s.thrown.clear();
        s.player_dir = DIR_EAST;
        s.thrown.push(crate::throwing::Thrown::launch(0, 10.0, 30.0, DIR_EAST));
        s.tick_throw(3 * 50);
        assert_eq!(s.thrown.len(), 1, "flies on through the player");
        assert_eq!(s.hearts_half(), 6);
    }

    #[test]
    fn on_a_no_player_killing_level_a_throw_stops_on_the_player_and_hurts_nothing() {
        let mut s = pelted();
        s.no_pk = true;
        s.tick_throw(3 * 50);
        assert!(s.thrown.is_empty());
        assert_eq!(s.hearts_half(), 6);
        assert!(!s.combat.is_hurt());
    }

    #[test]
    fn a_hurt_player_is_not_hit_and_the_object_flies_on() {
        let mut s = pelted();
        s.combat.start_hurt();
        s.tick_throw(3 * 50);
        assert_eq!(s.thrown.len(), 1);
        assert_eq!(s.hearts_half(), 6);
    }

    #[test]
    fn an_npc_in_the_way_is_pelted_first_and_stops_the_object() {
        let mut s = pelted();
        // An NPC whose box holds the probe from x = 13 on (the probe is one
        // tile right of the object: 12.5 + 1 after two ticks).
        let npc = |x: f32, _y: f32| (x >= 13.0).then_some(4);
        s.tick_throw_with(2 * 50, &npc);
        assert!(s.thrown.is_empty());
        assert_eq!(s.take_pelts(), vec![4]);
        assert!(s.take_pelts().is_empty());
        assert_eq!(s.hearts_half(), 6, "the NPC took it, not the player");
    }

    #[test]
    fn a_dead_player_does_not_throw() {
        let mut s = carrying_bush(10.0, 30.0, DIR_EAST);
        s.player_health = 0;
        s.life.set_rules(Some(crate::protocol::DEATH_205));
        s.tick_life(10);
        if s.life.is_dead() {
            assert_eq!(s.throw_carried(), None);
            assert!(s.carried.is_some());
        }
    }

    #[test]
    fn a_throw_flies_nine_ticks_then_the_bush_breaks_into_leaves() {
        let mut s = carrying_bush(10.0, 30.0, DIR_EAST);
        s.throw_carried();
        // Less than a tick: nothing moves.
        assert!(!s.tick_throw(49));
        assert_eq!(s.thrown[0].x, 10.5);
        assert!(s.tick_throw(1));
        assert_eq!(s.thrown[0].x, 11.5);
        for _ in 0..7 {
            s.tick_throw(50);
        }
        assert_eq!(s.thrown[0].ticks_left, 1);
        assert!(s.take_landings().is_empty());
        s.tick_throw(50);
        assert!(s.thrown.is_empty());
        let l = s.take_landings();
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].kind, l[0].effect), (0, Some(crate::throwing::LEAP_BUSH)));
        assert_eq!((l[0].x, l[0].y), (19.5, 31.0));
        // The effect shows its 7 frames, one per tick, then is gone.
        assert_eq!(s.leaps.len(), 1);
        assert_eq!(s.leaps[0].frame, -1);
        for f in 0..7 {
            s.tick_throw(50);
            assert_eq!(s.leaps.len(), 1, "frame {f} is shown");
            assert_eq!(s.leaps[0].frame, f);
        }
        s.tick_throw(50);
        assert!(s.leaps.is_empty(), "removed once past the last frame");
        assert!(!s.tick_throw(50));
    }

    #[test]
    fn a_wall_ends_the_flight_early() {
        let mut s = carrying_bush(10.0, 30.0, DIR_EAST);
        // After k steps the wall probe is (11.5 + k, 32): a wall at tile
        // (14, 32) is hit on the third step.
        s.current_level.as_mut().unwrap().set_tile_id(14, 32, WALL);
        s.throw_carried();
        for _ in 0..2 {
            s.tick_throw(50);
        }
        assert_eq!(s.thrown.len(), 1);
        s.tick_throw(50);
        assert!(s.thrown.is_empty());
        let l = s.take_landings();
        assert_eq!(l[0].x, 13.5);
    }

    #[test]
    fn the_type_the_throw_passes_is_not_a_wall_to_it() {
        let mut s = carrying_bush(10.0, 30.0, DIR_EAST);
        s.tiletypes.as_mut().unwrap().types[WALL as usize] = 20;
        s.current_level.as_mut().unwrap().set_tile_id(14, 32, WALL);
        s.throw_carried();
        for _ in 0..4 {
            s.tick_throw(50);
        }
        assert_eq!(s.thrown.len(), 1, "type 20 is flown through");
        // The same tile as a real wall (22) stops it.
        let mut s = carrying_bush(10.0, 30.0, DIR_EAST);
        s.current_level.as_mut().unwrap().set_tile_id(14, 32, WALL);
        s.throw_carried();
        for _ in 0..4 {
            s.tick_throw(50);
        }
        assert!(s.thrown.is_empty());
    }

    #[test]
    fn the_board_edge_ends_the_flight() {
        let mut s = carrying_bush(58.0, 30.0, DIR_EAST);
        s.throw_carried();
        for _ in 0..9 {
            s.tick_throw(50);
        }
        // the probe 59.5 + k leaves the board at k = 5
        assert!(s.thrown.is_empty());
        let l = s.take_landings();
        assert_eq!(l.len(), 1);
        assert!(l[0].x < 64.0, "stopped at the edge, x {}", l[0].x);
    }

    #[test]
    fn landing_in_water_splashes_and_a_rock_breaks_into_rock_pieces() {
        // Water where the object's own probe point ends up.
        let mut s = carrying_bush(10.0, 30.0, DIR_EAST);
        s.throw_carried();
        // end: x = 19.5, y = 28.625 + 1.8 = 30.425 -> probe (20.5, 31.425)
        s.current_level.as_mut().unwrap().set_tile_id(20, 31, WATER);
        for _ in 0..9 {
            s.tick_throw(50);
        }
        assert_eq!(s.take_landings()[0].effect, Some(crate::throwing::LEAP_SPLASH));
        // A rock (lift entry 2 -> sprite 265) breaks into the rock pieces.
        let mut s = object_session(1);
        face_up_at(&mut s, 40, 20);
        assert_eq!(s.interact(), Interaction::Lifted { kind: 2 });
        assert_eq!(s.interact(), Interaction::Thrown { kind: 2 });
        for _ in 0..9 {
            s.tick_throw(50);
        }
        assert_eq!(s.take_landings()[0].effect, Some(crate::throwing::LEAP_ROCK));
    }

    #[test]
    fn a_bomb_stays_where_it_lands() {
        let mut s = object_session(0);
        s.player_x = 10.0;
        s.player_y = 30.0;
        let mut o = crate::throwing::Thrown::launch(0, 10.0, 30.0, DIR_SOUTH);
        o.sprite = Some(65);
        s.thrown.push(o);
        for _ in 0..9 {
            s.tick_throw(50);
        }
        assert!(s.thrown.is_empty());
        assert!(s.leaps.is_empty(), "no break effect for a bomb");
        assert_eq!(s.grounded.len(), 1);
        assert_eq!(s.grounded[0].sprite, 65);
        assert_eq!(s.take_landings()[0].effect, None);
    }

    #[test]
    fn the_four_directions_fly_their_own_way() {
        for (dir, dx, dz) in [(DIR_NORTH, 0.0, -9.0), (DIR_WEST, -9.0, 0.0), (DIR_SOUTH, 0.0, 9.0), (DIR_EAST, 9.0, 0.0)] {
            let mut s = carrying_bush(30.0, 30.0, dir);
            s.throw_carried();
            let (x0, z0) = (s.thrown[0].x, s.thrown[0].ground);
            for _ in 0..4 {
                s.tick_throw(50);
            }
            let o = &s.thrown[0];
            assert!((o.x - x0 - dx * 4.0 / 9.0).abs() < 1e-3 && (o.ground - z0 - dz * 4.0 / 9.0).abs() < 1e-3, "dir {dir}");
        }
    }

    #[test]
    fn a_new_level_drops_flights_and_effects() {
        let mut s = carrying_bush(10.0, 30.0, DIR_EAST);
        s.throw_carried();
        s.tick_throw(50);
        s.clear_throws();
        assert!(s.thrown.is_empty() && s.leaps.is_empty() && s.take_landings().is_empty());
    }

    #[test]
    fn break_tables_come_from_arrays_dat_when_it_has_them() {
        // a minimal file: nine empty arrays, four object arrays, then a break table
        // (array 19) with one kind of one frame, and a sprite table (array 24)
        let mut data = object_arrays(&[BUSH], &[BUSH_CUT], &[BUSH], &[BUSH_CUT]);
        for _ in 13..19 {
            data.extend_from_slice(&0u32.to_be_bytes());
        }
        data.extend_from_slice(&((1u32 << 16) | 1).to_be_bytes()); // kinds
        data.extend_from_slice(&((1u32 << 16) | 1).to_be_bytes()); // frames
        data.extend_from_slice(&1u32.to_be_bytes()); // pieces
        data.extend_from_slice(&((7u32 << 16) | (3 << 8) | 0xfe).to_be_bytes());
        for _ in 20..24 {
            data.extend_from_slice(&0u32.to_be_bytes());
        }
        data.extend_from_slice(&1u32.to_be_bytes());
        data.extend_from_slice(&((1u32 << 24) | (2 << 12) | (3 << 6) | 4).to_be_bytes());
        let t = ObjectTables::parse(&data).unwrap();
        assert_eq!(t.leaps, vec![vec![vec![(7u32 << 16) | (3 << 8) | 0xfe]]]);
        assert_eq!(crate::throwing::leap_frames(Some(&t), 0), vec![vec![(7u16, 3i8, -2i8)]]);
        assert_eq!(crate::throwing::sprite_rect(Some(&t), 0), Some((2, 4, 6, 8)));
        // ids the file lacks fall back to the built-in rect
        assert_eq!(crate::throwing::sprite_rect(Some(&t), 261), Some((0, 338, 32, 32)));
    }

    // -- Bush and pot drops (`offline/drops.rs`). ---------------------------

    /// The object session with this build's drop rules and a roll that
    /// gives a green rupee (seed 0: the first `Random(100)` is 0).
    fn drop_session(d: crate::protocol::ItemDrops) -> OfflineSession {
        let mut s = object_session(1);
        s.set_drops(d);
        s.item_rng = crate::items::DelphiRandom::new(0);
        s
    }

    fn ticks(s: &mut OfflineSession, n: usize, online: bool) {
        for _ in 0..n {
            s.tick_items(drops::ITEM_TICK_MS, online);
        }
    }

    #[test]
    fn a_1x_cut_rolls_a_rupee_onto_the_bush_and_tells_the_server_from_138() {
        let mut s = drop_session(crate::protocol::DROPS_138);
        face_up_at(&mut s, 20, 20);
        assert_eq!(s.try_cut(), Some(0));
        assert_eq!(s.floor_items, vec![crate::items::FloorItem::new(crate::items::GREENRUPEE, 20.0, 20.0)], "at the bush's top-left tile");
        // The 1.39 capture: C>S 12 x*2, y*2, item.
        assert_eq!(s.take_item_messages(true), vec![crate::items::item_add_message(0, 20.0, 20.0)]);
        // 1.33 to 1.37 roll the same but tell nobody over TCP.
        let mut s = drop_session(crate::protocol::DROPS_133);
        face_up_at(&mut s, 20, 20);
        s.try_cut();
        assert_eq!(s.floor_items.len(), 1);
        assert!(s.take_item_messages(true).is_empty());
    }

    #[test]
    fn a_roll_of_37_or_more_drops_nothing_and_2x_never_rolls() {
        let mut s = drop_session(crate::protocol::DROPS_138);
        s.item_rng = crate::items::DelphiRandom::new(0xffff_ffff);
        face_up_at(&mut s, 20, 20);
        s.try_cut();
        assert!(s.floor_items.is_empty(), "roll 96");
        // 1.42 on: the server drops, and offline nothing does.
        let mut s = drop_session(crate::protocol::DROPS_142);
        face_up_at(&mut s, 20, 20);
        s.try_cut();
        assert!(s.floor_items.is_empty());
        assert_eq!(s.item_rng, crate::items::DelphiRandom::new(0), "no roll at all");
    }

    #[test]
    fn a_lifted_bush_rolls_entry_2_leaves_a_heart_and_the_others_nothing() {
        let mut s = drop_session(crate::protocol::DROPS_138);
        face_up_at(&mut s, 20, 20);
        assert_eq!(s.try_lift(), Some(0));
        assert_eq!(s.floor_items.len(), 1, "the bush rolled");
        s.carried = None;
        s.floor_items.clear();
        face_up_at(&mut s, 30, 20);
        assert_eq!(s.try_lift(), Some(1));
        assert!(s.floor_items.is_empty(), "entry 1 (the sign) leaves nothing");
        s.carried = None;
        s.item_rng = crate::items::DelphiRandom::new(0xffff_ffff);
        face_up_at(&mut s, 40, 20);
        assert_eq!(s.try_lift(), Some(2));
        assert_eq!(s.floor_items, vec![crate::items::FloorItem::new(crate::items::HEART, 40.0, 20.0)], "entry 2 (the pot) always a heart");
    }

    #[test]
    fn offline_a_pickup_is_applied_and_nothing_is_sent() {
        let mut s = drop_session(crate::protocol::DROPS_138);
        face_up_at(&mut s, 20, 20);
        s.try_cut();
        s.take_item_messages(false);
        // Walk onto it: (19.5, 19.5) tests (21, 21.5) against its middle (21, 21).
        s.player_y = 19.5;
        ticks(&mut s, 4, false);
        assert_eq!(s.rupees, 0, "not in the first four ticks");
        ticks(&mut s, 1, false);
        assert_eq!(s.rupees, 1);
        assert!(s.floor_items.is_empty());
        assert!(s.take_sound_events().iter().any(|e| e.file == "extra.wav"));
        assert!(s.take_item_messages(false).is_empty());
    }

    #[test]
    fn online_from_138_the_server_gives_the_pickup_and_hears_the_timeout() {
        let mut s = drop_session(crate::protocol::DROPS_142);
        face_up_at(&mut s, 20, 20);
        s.player_y = 19.5;
        s.add_server_item(crate::items::FloorItem::new(crate::items::BLUERUPEE, 20.0, 20.0));
        ticks(&mut s, 5, true);
        assert_eq!(s.rupees, 0, "the server sends the rupees with props");
        assert_eq!(s.take_item_messages(true), vec![crate::items::item_take_message(1, 20.0, 20.0)]);
        // Out of reach it times out after 165 ticks: PLI_ITEMDEL.
        s.add_server_item(crate::items::FloorItem::new(crate::items::BOMBS, 50.0, 50.0));
        ticks(&mut s, 164, true);
        assert!(s.take_item_messages(true).is_empty());
        ticks(&mut s, 1, true);
        assert_eq!(s.take_item_messages(true), vec![crate::items::item_del_message(50.0, 50.0)]);
        // PLO_ITEMDEL takes an item away; a level change clears the floor.
        s.add_server_item(crate::items::FloorItem::new(crate::items::BOMBS, 50.0, 50.0));
        s.remove_server_item(50.0, 50.0);
        assert!(s.floor_items.is_empty());
        s.add_server_item(crate::items::FloorItem::new(crate::items::BOMBS, 50.0, 50.0));
        s.clear_throws();
        assert!(s.floor_items.is_empty());
    }

    #[test]
    fn before_138_an_online_pickup_is_applied_and_sent_as_itemdel_and_a_timeout_is_silent() {
        let mut s = drop_session(crate::protocol::DROPS_133);
        face_up_at(&mut s, 20, 20);
        s.player_y = 19.5;
        s.add_server_item(crate::items::FloorItem::new(crate::items::BOMBS, 20.0, 20.0));
        s.bombs = 97;
        ticks(&mut s, 5, true);
        assert_eq!(s.bombs, 99, "five bombs, up to 99");
        assert_eq!(s.take_item_messages(true), vec![crate::items::item_del_message(20.0, 20.0)]);
        s.add_server_item(crate::items::FloorItem::new(crate::items::BOMBS, 50.0, 50.0));
        ticks(&mut s, 165, true);
        assert!(s.floor_items.is_empty());
        assert!(s.take_item_messages(true).is_empty());
    }

    #[test]
    fn a_floor_heart_heals_one_heart() {
        let mut s = OfflineSession::new();
        s.set_hearts_half(2);
        s.apply_pickup(crate::items::HEART);
        assert_eq!(s.hearts_half(), 4);
        s.apply_pickup(crate::items::REDRUPEE);
        s.apply_pickup(19);
        assert_eq!(s.rupees, 130);
    }

    /// The player at (14, 30) with 3 hearts, combat rules on, and player 7
    /// in the player list.
    fn bombed() -> OfflineSession {
        let mut s = pelted();
        s.thrown.clear();
        s.present_players = vec![7];
        s
    }

    fn explosion(owned: bool) -> crate::bombs::Explosion {
        // one cross of reach 2 whose centre piece covers the player:
        // (13.5 + 1, 32 + 1) against (14 + 1.5, 30 + 2)
        crate::bombs::Explosion { radius: 2, x: 14.0, y: 31.0, power: 1, owned }
    }

    #[test]
    fn another_players_explosion_hurts_and_names_the_owner() {
        let mut s = bombed();
        s.receive_explosion(&explosion(true), Some(7));
        assert_eq!(s.blast.len(), 9);
        assert_eq!(s.take_sound_events().first().map(|e| e.file.clone()), Some("bomb.wav".into()));
        s.tick_bombs(50);
        assert_eq!(s.hearts_half(), 5);
        assert!(s.combat.is_hurt());
        s.set_hearts_half(1);
        s.combat = crate::combat::CombatState::new(Some(crate::protocol::COMBAT_205));
        s.blast.clear();
        s.receive_explosion(&explosion(true), Some(7));
        s.tick_bombs(50);
        let events = s.tick_life(50);
        assert!(events.contains(&LifeEvent::Died { attacker: Some(7) }), "{events:?}");
    }

    #[test]
    fn on_a_no_player_killing_level_an_owned_piece_does_not_hurt() {
        let mut s = bombed();
        s.no_pk = true;
        s.receive_explosion(&explosion(true), Some(7));
        s.tick_bombs(50);
        assert_eq!(s.hearts_half(), 6);
        assert!(!s.combat.is_hurt());
    }

    #[test]
    fn an_unowned_piece_or_one_whose_owner_left_hurts_even_with_no_pk() {
        for (owned, present) in [(false, vec![7]), (true, vec![])] {
            let mut s = bombed();
            s.no_pk = true;
            s.present_players = present;
            s.receive_explosion(&explosion(owned), Some(7));
            s.tick_bombs(50);
            assert_eq!(s.hearts_half(), 5, "owned {owned}");
            s.set_hearts_half(0);
            let events = s.tick_life(50);
            assert!(events.contains(&LifeEvent::Died { attacker: None }), "{events:?}");
        }
    }

    #[test]
    fn a_received_bomb_burns_its_own_fuse_then_hurts_and_bombdel_removes_it() {
        let mut s = bombed();
        let add = crate::bombs::BombAdd { x: 14.0, y: 31.0, power: 3, fuse: 2, owned: true };
        s.receive_bomb(&add, Some(7));
        assert_eq!(s.placed_bombs[0].owner, Some(7));
        s.tick_bombs(2 * 50);
        assert!(s.blast.is_empty(), "fuse 2: still burning after two ticks");
        s.tick_bombs(50);
        assert_eq!(s.blast.len(), 9);
        s.tick_bombs(50);
        assert_eq!(s.hearts_half(), 4, "power 3: a whole heart");
        let mut s = bombed();
        s.receive_bomb(&add, Some(7));
        s.remove_bomb(13.5, 31.0);
        assert_eq!(s.placed_bombs.len(), 1, "only exactly that point");
        s.remove_bomb(14.0, 31.0);
        assert!(s.placed_bombs.is_empty());
    }

    /// Sprite table entries of 16 x 16 pixels for the arrow sprites.
    fn arrow_tables() -> ObjectTables {
        let mut t = ObjectTables::default();
        t.sprites = vec![0; 111];
        for id in 107..=110 {
            t.sprites[id] = (8 << 6) | 8;
        }
        t
    }

    fn shot(kind: u8, owned: bool) -> crate::arrows::ArrowAdd {
        // flying east along y 31 from x 10, a tile a tick; its half-tile
        // box is past the player's x + 0.5 = 14.5 first at x 15 (tick 5)
        crate::arrows::ArrowAdd { x: 10.0, y: 31.0, dir: 3, replaces: false, owned, sprite: 110, kind }
    }

    #[test]
    fn another_players_arrow_hits_once_its_box_is_on_the_body() {
        let mut s = bombed();
        s.object_tables = Some(arrow_tables());
        s.receive_arrow(&shot(0, true), Some(7));
        let none = |_: f32, _: f32| None;
        s.tick_arrows(4 * 50, &none);
        assert_eq!(s.hearts_half(), 6, "x 14: 14 + 0.5 is not past 14.5");
        s.tick_arrows(50, &none);
        assert_eq!(s.hearts_half(), 5);
        assert!(s.arrows.is_empty(), "the shot is gone");
        assert!(s.blast.is_empty(), "an arrow leaves no blast");
    }

    #[test]
    fn a_nuke_takes_a_heart_and_leaves_a_blast() {
        let mut s = bombed();
        s.object_tables = Some(arrow_tables());
        s.receive_arrow(&shot(3, true), Some(7));
        s.tick_arrows(5 * 50, &|_, _| None);
        assert_eq!(s.hearts_half(), 4);
        assert_eq!(s.blast.len(), 9);
        assert!(s.blast.iter().all(|p| p.owner == Some(7) && p.power == 1));
        assert_eq!((s.blast[0].x, s.blast[0].y), (14.25, 30.25));
    }

    #[test]
    fn arrows_on_a_no_pk_level_and_our_own_weapons_never_hit() {
        let mut s = bombed();
        s.object_tables = Some(arrow_tables());
        s.no_pk = true;
        s.receive_arrow(&shot(0, false), Some(7));
        s.tick_arrows(5 * 50, &|_, _| None);
        assert_eq!(s.hearts_half(), 6);
        assert_eq!(s.arrows.len(), 1, "it flies on");
        let mut s = bombed();
        s.object_tables = Some(arrow_tables());
        s.player_dir = DIR_WEST;
        s.shoot_arrow(17.0, 30.0, 2.0, 2.0, DIR_WEST, true);
        s.tick_arrows(3 * 50, &|_, _| None);
        assert_eq!(s.hearts_half(), 6);
    }

    #[test]
    fn without_a_sprite_table_a_shot_does_not_hit_the_player() {
        let mut s = bombed();
        s.receive_arrow(&shot(0, true), Some(7));
        s.tick_arrows(6 * 50, &|_, _| None);
        assert_eq!(s.hearts_half(), 6);
    }
}
