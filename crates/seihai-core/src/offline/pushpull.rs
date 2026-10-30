//! Ledge jumps, pushing and pulling, as the 2.05 client does them.
//!
//! Everything here was read from the 2.05 rev 2 client (see
//! `documentation/ledges-push-pull.md` for the full notes). One tick is the
//! 50 ms game tick; a walk step is one arrow key in one tick.
//!
//! * **Pushing.** Every refused step without a corner nudge counts one
//!   blocked step; a free step or a nudge starts the count again. After 17
//!   blocked steps the push pose is shown.
//! * **Ledge jump.** At 18 blocked steps, then every 20 (38 wraps to 18),
//!   the client tests for a ledge: the tile at [`LEDGE_PROBE`] must be one of
//!   the ledge tiles for that facing (`arrays.dat` array 8, the tiles 2.05
//!   calls type 21), and the landing point [`LANDING`] must be free. The
//!   jump then runs for [`JUMP_TICKS`] ticks along fixed curves and plays
//!   `jump.wav`. Arrow keys do nothing until it lands.
//! * **Stone blocks.** At 28 blocked steps (and every 20 after) the client
//!   tries a push. It fires `pushed` at the probe point, then looks for the
//!   pushable block (`arrays.dat` array 16, entry 0) under it. If the two
//!   cells it moves into are free, the block moves one tile, the vacated
//!   cells get the tiles from two ahead, the player moves one tile after it
//!   and `stonemove.wav` plays.
//! * **Grab and pull.** A with nothing liftable in front grabs any wall
//!   tile at [`GRAB_POINT`]. The grab holds while A is held. Holding the
//!   arrow away from the wall starts the pull pose; every 18 ticks of that
//!   the client tries a pull: it fires `pulled` at [`PULL_TRIGGER`], then
//!   looks for the pullable block (array 16, entry 1). A wall or a heavy
//!   rock does not move; the pull pose just goes on (the "futile pull").
//!   Letting go of the arrow stops the pull cycle and letting go of A ends
//!   the grab, but the pull pose stays, looping, until the player moves
//!   (read live in 2.05 rev 2: Graal_analysis `player-mannerisms-live.md`).
//!
//! * **NPCs.** When no block is there, the same two routines look for an
//!   NPC with `canbepushed` (at the probe point) or `canbepulled` (at the
//!   grab point). If every cell along the edge it moves into is free, the
//!   NPC moves one tile, the player one tile with it, and the NPC gets
//!   `waspushed` / `waspulled` (after `actionpushed` / `actionpulled`). The
//!   2.05 client sends nothing to the server for the NPC.

use super::{walk_floor, OfflineSession, DIR_NORTH, DIR_SOUTH, DIR_WEST};
use crate::assets::{Npc, Object2x2, TileKind};
use crate::sound::SoundEvent;

/// Per facing (up, left, down, right): the point, from the player's
/// position, that the ledge test and the push read.
pub const LEDGE_PROBE: [(f32, f32); 4] = [(1.5, 0.5), (0.0, 2.0), (1.5, 3.5), (3.0, 2.0)];
/// Per facing: the landing point, from `(x + 1.5, y + 2)`, that must be
/// free for a jump.
pub const LANDING: [(f32, f32); 4] = [(0.0, -5.0), (-6.5, 3.0), (0.0, 7.0), (6.5, 3.0)];
/// Per facing: the point the A key's grab reads.
pub const GRAB_POINT: [(f32, f32); 4] = [(1.5, 0.9), (0.0, 2.0), (1.5, 3.1), (3.0, 2.0)];
/// Per facing: where the `pulled` trigger goes (one tile out, the same
/// points as the A key's facing point).
pub const PULL_TRIGGER: [(f32, f32); 4] = [(1.5, 0.0), (-0.5, 2.0), (1.5, 4.0), (3.5, 2.0)];
/// One tile in each facing.
pub const DIR_STEP: [(i32, i32); 4] = [(0, -1), (-1, 0), (0, 1), (1, 0)];
/// Per facing: the two cells, from a pushed block's top-left, it moves into.
pub const PUSH_DEST: [[(i32, i32); 2]; 4] = [[(0, -1), (1, -1)], [(-1, 0), (-1, 1)], [(0, 2), (1, 2)], [(2, 0), (2, 1)]];
/// Per facing: the two cells, from a pulled block's top-left, that must be
/// free behind the player (the player steps back one tile).
pub const PULL_DEST: [[(i32, i32); 2]; 4] = [[(0, 4), (1, 4)], [(4, 0), (4, 1)], [(0, -3), (1, -3)], [(-3, 0), (-3, 1)]];

/// Per facing: the two cells, from the grab cell, that must be free behind
/// the player before an NPC is pulled.
pub const NPC_PULL_BEHIND: [[(i32, i32); 2]; 4] = [[(0, 3), (1, 3)], [(3, 0), (3, 1)], [(0, -3), (1, -3)], [(-3, 0), (-3, 1)]];

/// Ticks a jump lasts.
pub const JUMP_TICKS: u8 = 8;
/// Jump curves, one value per tick. Up: tiles above the start.
pub const JUMP_UP: [f32; 8] = [1.5, 2.7, 3.8, 4.5, 5.0, 5.2, 5.2, 5.0];
/// Down: tiles below the start.
pub const JUMP_DOWN: [f32; 8] = [-0.2, 0.2, 0.5, 1.5, 2.8, 4.0, 5.4, 7.0];
/// Left and right: tiles out to the side, and tiles down.
pub const JUMP_SIDE_X: [f32; 8] = [1.0, 2.0, 3.0, 3.8, 4.5, 5.2, 5.8, 6.5];
pub const JUMP_SIDE_Y: [f32; 8] = [-0.2, -0.5, 0.0, 0.5, 1.1, 1.7, 2.3, 3.0];

/// Blocked steps after which the push pose shows (shown from 18 on).
pub const PUSH_POSE_AFTER: u32 = 17;
/// Blocked steps at which the ledge test runs.
pub const LEDGE_TEST_AT: u32 = 18;
/// The count goes back to [`LEDGE_TEST_AT`] when it reaches this.
pub const BLOCKED_WRAP: u32 = 38;
/// Blocked steps (before the step that counts) at which a push is tried.
pub const PUSH_AT: u32 = 28;
/// The grab's pull cycle: the count runs 2..=19 and a pull is tried at 19.
pub const PULL_AT: u32 = 19;
const PULL_CYCLE: u32 = 18;

const TICK_MS: u64 = 50;

/// Sprite numbers S on the wire (SPRITE prop: direction + 4 * S), from
/// 2.05's animation step table: `push` steps 14 + frame, `grab` step 19,
/// `pull` steps 20 + frame.
pub const PUSH_SPRITE_S: [u8; 5] = [11, 12, 13, 11, 12];
pub const GRAB_SPRITE_S: u8 = 14;
pub const PULL_SPRITE_S: [u8; 3] = [15, 16, 17];

/// A ledge jump in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Jump {
    pub dir: u8,
    /// Where the player stood when it started.
    pub x0: f32,
    pub y0: f32,
    /// Ticks shown so far (1 after the start, [`JUMP_TICKS`] on landing).
    pub shown: u8,
}

/// A grab in progress: 1 while just holding, 2..=19 while pulling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grab {
    pub count: u32,
}

impl Grab {
    pub fn pulling(&self) -> bool {
        self.count > 1
    }
}

/// A `pushed` or `pulled` trigger action at a point, for the NPCs there.
#[derive(Debug, Clone, PartialEq)]
pub struct Trigger {
    /// `pushed` or `pulled`; scripts see `actionpushed` / `actionpulled`.
    pub action: &'static str,
    pub x: f32,
    pub y: f32,
    /// The direction parameter: the facing for a push, the opposite for a
    /// pull (the way the thing would move).
    pub dir: u8,
}

/// An NPC a push or a pull moved: its index in the level's NPC list, where
/// it is now, and the event its script gets.
#[derive(Debug, Clone, PartialEq)]
pub struct NpcMoved {
    pub index: usize,
    pub x: f32,
    pub y: f32,
    /// `waspushed` or `waspulled`.
    pub event: &'static str,
}

/// The box an NPC is hit in, `(x, y, w, h)` in tiles, as the 2.05 hit test
/// has it: a character the fixed box `x + 0.5 .. x + 2.5`, `y + 1 .. y + 3`;
/// otherwise its `setshape` size, else the part of its image it shows. An
/// NPC whose image size Seihai does not know counts as 2 x 2 tiles (32
/// pixels, the script world's default).
pub fn npc_hit_box(n: &Npc) -> (f32, f32, f32, f32) {
    if n.ani.is_some() {
        return (n.x + 0.5, n.y + 1.0, 2.0, 2.0);
    }
    let (w, h) = npc_size_px(n);
    (n.x, n.y, w / 16.0, h / 16.0)
}

/// The size, in pixels, a moving NPC's edge is walked along: 48 x 48 for a
/// character, else as [`npc_hit_box`].
fn npc_move_size(n: &Npc) -> (f32, f32) {
    if n.ani.is_some() {
        return (3.0, 3.0);
    }
    let (w, h) = npc_size_px(n);
    (w / 16.0, h / 16.0)
}

fn npc_size_px(n: &Npc) -> (f32, f32) {
    if let Some((w, h)) = n.shape {
        return (w, h);
    }
    if let Some((_, _, w, h)) = n.image_part {
        return (w as f32, h as f32);
    }
    (32.0, 32.0)
}

/// The points along the edge an NPC at `(x, y)`, `w` by `h` tiles, moves
/// into going `d` (0 up, 1 left, 2 down, 3 right): one per tile, starting
/// at its left or top.
fn npc_edge(d: usize, x: f32, y: f32, w: f32, h: f32) -> Vec<(f32, f32)> {
    let along = |from: f32, len: f32| (0..).map(move |i| from + i as f32).take_while(move |&t| t < from + len);
    match d {
        0 => along(x, w).map(|t| (t, y - 1.0)).collect(),
        1 => along(y, h).map(|t| (x - 1.0, t)).collect(),
        2 => along(x, w).map(|t| (t, y + h)).collect(),
        _ => along(y, h).map(|t| (x + w, t)).collect(),
    }
}

/// The push, jump and grab state of the player.
#[derive(Debug, Clone, Default)]
pub struct PushPull {
    /// Blocked steps in a row (see the module notes).
    pub blocked_steps: u32,
    /// The last step was refused (2.05's "pushing" mode).
    pub pushing: bool,
    pub jump: Option<Jump>,
    pub grab: Option<Grab>,
    /// Ticks the pull pose has been shown, for its frame.
    pub pull_ticks: u32,
    /// The pull pose is on: from the first pull until the player moves.
    pub pull_shown: bool,
    /// The grab pose stays after A is let go (NewMain 1.41), until the
    /// next action.
    pub grab_shown: bool,
    clock_ms: u64,
    triggers: Vec<Trigger>,
    npc_moves: Vec<NpcMoved>,
    moved: bool,
}

/// What a single walk step did (see [`OfflineSession::walk_step`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepResult {
    Free,
    Refused { nudged: bool },
}

fn opposite(dir: u8) -> u8 {
    (dir + 2) % 4
}

fn tile_of(v: f32) -> i32 {
    walk_floor(f64::from(v)) as i32
}

impl PushPull {
    /// The pull pose is on: pulling now, or a pull since the last move.
    pub fn pull_pose(&self) -> bool {
        self.pull_shown || self.grab.is_some_and(|g| g.pulling())
    }
}

impl OfflineSession {
    /// Count one walk step in facing `dir` and act on the count: the ledge
    /// test and the push. Returns the position to go on from (a push moves
    /// the player one tile), or `None` when a jump started.
    pub(crate) fn count_step(&mut self, dir: u8, res: StepResult, x: f64, y: f64) -> Option<(f64, f64)> {
        let (mut x, mut y) = (x, y);
        match res {
            StepResult::Free => {
                self.pushpull.pull_shown = false;
                self.pushpull.grab_shown = false;
                self.pushpull.pushing = false;
                self.pushpull.blocked_steps = 0;
            }
            StepResult::Refused { nudged } => {
                if !self.pushpull.pushing {
                    self.pushpull.blocked_steps = 0;
                    self.pushpull.pushing = true;
                }
                if !nudged && self.pushpull.blocked_steps == PUSH_AT {
                    if let Some((nx, ny)) = self.try_push(dir, x as f32, y as f32) {
                        (x, y) = (f64::from(nx), f64::from(ny));
                    }
                }
                self.pushpull.blocked_steps = if nudged { 0 } else { self.pushpull.blocked_steps + 1 };
                if self.pushpull.blocked_steps >= BLOCKED_WRAP {
                    self.pushpull.blocked_steps = LEDGE_TEST_AT;
                }
                if self.pushpull.blocked_steps == LEDGE_TEST_AT && self.ledge_ahead(dir, x as f32, y as f32) {
                    self.start_jump(dir, x as f32, y as f32);
                    return None;
                }
            }
        }
        Some((x, y))
    }

    /// The push pose is on: 18 or more blocked steps in a row.
    pub fn push_pose(&self) -> bool {
        self.pushpull.pushing && self.pushpull.blocked_steps > PUSH_POSE_AFTER && self.pushpull.jump.is_none()
    }

    /// Ticks the push pose has been on: 0 on the 18th blocked step, back to
    /// 0 each time the count wraps from 38 to 18.
    pub fn push_tick(&self) -> Option<usize> {
        self.push_pose().then(|| (self.pushpull.blocked_steps - PUSH_POSE_AFTER - 1) as usize)
    }

    /// The ledge test: the probe tile is a ledge for this facing, and the
    /// landing point is free.
    fn ledge_ahead(&self, dir: u8, x: f32, y: f32) -> bool {
        let d = usize::from(dir & 3);
        let (Some(level), Some(tables)) = (&self.current_level, &self.object_tables) else { return false };
        let (px, py) = (x + LEDGE_PROBE[d].0, y + LEDGE_PROBE[d].1);
        if !(0.0..64.0).contains(&px) || !(0.0..64.0).contains(&py) {
            return false;
        }
        let Some(id) = level.tile_id_at(tile_of(px), tile_of(py)) else { return false };
        if !tables.ledges[d].contains(&id) {
            return false;
        }
        let (lx, ly) = (x + 1.5 + LANDING[d].0, y + 2.0 + LANDING[d].1);
        !self.walk_blocked(f64::from(lx), f64::from(ly))
    }

    fn start_jump(&mut self, dir: u8, x: f32, y: f32) {
        self.player_dir = dir;
        self.pushpull.pushing = false;
        self.pushpull.blocked_steps = 0;
        self.pushpull.pull_shown = false;
        self.pushpull.grab_shown = false;
        self.pushpull.jump = Some(Jump { dir, x0: x, y0: y, shown: 0 });
        self.sound_events.push(SoundEvent { file: "jump.wav".into(), x: x + 1.5, y: y + 2.0 });
        // 2.05 shows the first step of the flight in the tick the jump starts.
        self.jump_step();
    }

    /// One tick of the flight. Ends the jump after the last one.
    fn jump_step(&mut self) {
        let Some(mut j) = self.pushpull.jump else { return };
        let i = usize::from(j.shown.min(JUMP_TICKS - 1));
        let (x, y) = match j.dir {
            DIR_NORTH => (j.x0, j.y0 - JUMP_UP[i]),
            DIR_WEST => (j.x0 - JUMP_SIDE_X[i], j.y0 + JUMP_SIDE_Y[i]),
            DIR_SOUTH => (j.x0, j.y0 + JUMP_DOWN[i]),
            _ => (j.x0 + JUMP_SIDE_X[i], j.y0 + JUMP_SIDE_Y[i]),
        };
        let round = |v: f32| (v * 1000.0).round() / 1000.0;
        self.player_x = round(x);
        self.player_y = round(y);
        j.shown += 1;
        self.pushpull.jump = (j.shown < JUMP_TICKS).then_some(j);
        self.pushpull.moved = true;
        if self.pushpull.jump.is_none() {
            self.land();
        }
    }

    /// The jump is over: on a gmap the landing may be in the next cell.
    fn land(&mut self) {
        if let Some(w) = &self.gmap {
            if let crate::gmap::Resolved::Crossed(c) = w.map.resolve(w.cell, (self.player_x, self.player_y)) {
                self.cross_to(c);
            }
        }
        self.armed = true;
        self.refresh_water();
    }

    /// A ledge jump is in the air: arrow keys do nothing.
    pub fn jumping(&self) -> bool {
        self.pushpull.jump.is_some()
    }

    /// The push: `pushed` at the probe point, then the pushable block under
    /// it. Returns the player's new position if the block moved.
    fn try_push(&mut self, dir: u8, x: f32, y: f32) -> Option<(f32, f32)> {
        let d = usize::from(dir & 3);
        let (px, py) = (x + LEDGE_PROBE[d].0, y + LEDGE_PROBE[d].1);
        let (tx, ty) = (tile_of(px), tile_of(py));
        if !(0..64).contains(&tx) || !(0..64).contains(&ty) {
            return None;
        }
        self.pushpull.triggers.push(Trigger { action: "pushed", x: px, y: py, dir });
        let block = self.object_tables.as_ref().and_then(|t| t.blocks.first().copied());
        let at = block.and_then(|b| self.block_origin(&b, tx, ty));
        if let Some((ax, ay)) = at {
            if PUSH_DEST[d].iter().any(|&(dx, dy)| self.block_way_shut(ax + dx, ay + dy)) {
                return None;
            }
            self.move_block(ax, ay, DIR_STEP[d]);
        } else if !self.move_npc(d, px, py, false) {
            return None;
        }
        let (sx, sy) = DIR_STEP[d];
        let (nx, ny) = (x + sx as f32, y + sy as f32);
        self.sound_events.push(SoundEvent { file: "stonemove.wav".into(), x: nx + 1.5, y: ny + 2.0 });
        Some((nx, ny))
    }

    /// The NPC with `canbepushed` (or, pulling, `canbepulled`) whose hit
    /// box holds `(px, py)`.
    fn movable_npc_at(&self, px: f32, py: f32, pull: bool) -> Option<usize> {
        self.current_level.as_ref()?.npcs.iter().position(|n| {
            let (x, y, w, h) = npc_hit_box(n);
            n.visible && (if pull { n.pullable } else { n.pushable }) && px >= x && px < x + w && py >= y && py < y + h
        })
    }

    /// A pushable or pullable NPC stops walking at `(px, py)` (other than
    /// `except`). The 2.05 client stops at every NPC that blocks; Seihai
    /// only at these, since it does not know most NPCs' image sizes.
    pub(crate) fn npc_blocks_at(&self, px: f32, py: f32, except: Option<usize>) -> bool {
        self.current_level.as_ref().is_some_and(|l| {
            l.npcs.iter().enumerate().any(|(i, n)| {
                let (x, y, w, h) = npc_hit_box(n);
                Some(i) != except && n.visible && n.blocks && (n.pushable || n.pullable) && px >= x && px < x + w && py >= y && py < y + h
            })
        })
    }

    /// Move the pushable (or pullable) NPC at `(px, py)` one tile, the 2.05
    /// way: every point along the edge it moves into must be free of walls,
    /// chests and other NPCs; then it moves one tile in facing `d` (against
    /// it when pulling) and the player with it. True if it moved; the
    /// player's own move is the caller's.
    fn move_npc(&mut self, d: usize, px: f32, py: f32, pull: bool) -> bool {
        let Some(i) = self.movable_npc_at(px, py, pull) else { return false };
        let Some(n) = self.current_level.as_ref().map(|l| &l.npcs[i]) else { return false };
        let (w, h) = npc_move_size(n);
        let way = if pull { (d + 2) % 4 } else { d };
        let edge = npc_edge(way, n.x, n.y, w, h);
        if edge.iter().any(|&(cx, cy)| self.npc_way_shut(cx, cy, i)) {
            return false;
        }
        let (sx, sy) = DIR_STEP[way];
        let Some(n) = self.current_level.as_mut().map(|l| &mut l.npcs[i]) else { return false };
        n.x += sx as f32;
        n.y += sy as f32;
        let (x, y) = (n.x, n.y);
        self.pushpull.npc_moves.push(NpcMoved { index: i, x, y, event: if pull { "waspulled" } else { "waspushed" } });
        true
    }

    /// 2.05's blocked-point test for a moving NPC: a wall tile, a chest,
    /// or another NPC (`except` is the one moving). Water does not stop it.
    fn npc_way_shut(&self, cx: f32, cy: f32, except: usize) -> bool {
        if !(0.0..64.0).contains(&cx) || !(0.0..64.0).contains(&cy) {
            return true;
        }
        self.walk_blocked_except(f64::from(cx), f64::from(cy), Some(except))
    }

    /// The pull: `pulled` one tile out, then the pullable block at the grab
    /// point. True if the block moved (the player stepped back with it).
    fn try_pull(&mut self) -> bool {
        let dir = self.player_dir & 3;
        let d = usize::from(dir);
        let (x, y) = (self.player_x, self.player_y);
        let (gx, gy) = (x + GRAB_POINT[d].0, y + GRAB_POINT[d].1);
        let (tx, ty) = (tile_of(gx), tile_of(gy));
        if !(0..64).contains(&tx) || !(0..64).contains(&ty) {
            return false;
        }
        let (qx, qy) = (x + PULL_TRIGGER[d].0, y + PULL_TRIGGER[d].1);
        self.pushpull.triggers.push(Trigger { action: "pulled", x: qx, y: qy, dir: opposite(dir) });
        let block = self.object_tables.as_ref().and_then(|t| t.blocks.get(1).copied());
        let at = block.and_then(|b| self.block_origin(&b, tx, ty));
        let (sx, sy) = DIR_STEP[d];
        if let Some((ax, ay)) = at {
            if PULL_DEST[d].iter().any(|&(dx, dy)| self.block_way_shut(ax + dx, ay + dy)) {
                return false;
            }
            self.move_block(ax, ay, (-sx, -sy));
        } else {
            // An NPC: the two cells behind the player, from the grab cell,
            // must be free first.
            if self.movable_npc_at(gx, gy, true).is_none()
                || NPC_PULL_BEHIND[d].iter().any(|&(dx, dy)| self.block_way_shut(tx + dx, ty + dy))
                || !self.move_npc(d, gx, gy, true)
            {
                return false;
            }
        }
        self.player_x = x - sx as f32;
        self.player_y = y - sy as f32;
        self.pushpull.moved = true;
        self.refresh_water();
        // 2.05 picks `lift.wav` or `lift2.wav` by a player flag that was not
        // traced; the first is the one played here.
        self.sound_events.push(SoundEvent { file: "lift.wav".into(), x: self.player_x + 1.5, y: self.player_y + 2.0 });
        true
    }

    /// The block's top-left if the tile at `(tx, ty)` is one of its four
    /// tiles. Like 2.05, only that one tile is compared.
    fn block_origin(&self, block: &Object2x2, tx: i32, ty: i32) -> Option<(i32, i32)> {
        let id = self.current_level.as_ref()?.tile_id_at(tx, ty)?;
        for (c, col) in block.iter().enumerate() {
            for (r, &t) in col.iter().enumerate() {
                if t == id {
                    return Some((tx - c as i32, ty - r as i32));
                }
            }
        }
        None
    }

    /// A block cannot move into or past this cell: a wall (or a chest, or
    /// off the board), water, or a chair. (2.05 also refuses the lower half
    /// of a bed, which Seihai's tile table does not load.)
    fn block_way_shut(&self, cx: i32, cy: i32) -> bool {
        let (px, py) = (f64::from(cx), f64::from(cy));
        if self.walk_blocked(px, py) {
            return true;
        }
        match (&self.current_level, &self.tiletypes) {
            (Some(level), Some(types)) => {
                level.kind_at(types, cx as f32, cy as f32) == TileKind::Water
                    || level.tile_id_at(cx, cy).is_some_and(|id| types.is_chair(id))
            }
            _ => false,
        }
    }

    /// Move the 2 x 2 block at `(ax, ay)` one tile by `(sx, sy)`, the 2.05
    /// way: each of its cells takes the tile two cells along first, then the
    /// block is written one cell along. The 4 x 4 area around the old spot
    /// is what changed.
    fn move_block(&mut self, ax: i32, ay: i32, (sx, sy): (i32, i32)) {
        let Some(level) = self.current_level.as_mut() else { return };
        let mut saved = [[0u16; 2]; 2];
        for (c, col) in saved.iter_mut().enumerate() {
            for (r, cell) in col.iter_mut().enumerate() {
                let (x, y) = (ax + c as i32, ay + r as i32);
                *cell = level.tile_id_at(x, y).unwrap_or(0);
                let under = level.tile_id_at(x + 2 * sx, y + 2 * sy).unwrap_or(*cell);
                level.set_tile_id(x, y, under);
            }
        }
        level.put_object(ax + sx, ay + sy, &saved);
        self.note_board_change(ax - 1, ay - 1, 4, 4);
    }

    /// A on a wall: start holding it. The point is [`GRAB_POINT`]; anything
    /// that blocks there as a tile can be grabbed. Not while swimming,
    /// carrying, dead or jumping.
    pub fn try_grab(&mut self) -> bool {
        if self.life.is_dead() || self.in_water || self.carried.is_some() || self.jumping() {
            return false;
        }
        let d = usize::from(self.player_dir & 3);
        let (gx, gy) = (self.player_x + GRAB_POINT[d].0, self.player_y + GRAB_POINT[d].1);
        let wall = match (&self.current_level, &self.tiletypes) {
            (Some(level), Some(types)) => self.kind_at_point(level, types, gx, gy) == TileKind::Blocking,
            _ => false,
        } || self.npc_blocks_at(gx, gy, None);
        if wall {
            self.pushpull.grab = Some(Grab { count: 1 });
            self.pushpull.pull_ticks = 0;
            self.pushpull.pull_shown = false;
            self.pushpull.grab_shown = false;
            self.pushpull.pushing = false;
        }
        wall
    }

    /// A was let go: the grab ends. A build whose pull lets go to `grab`
    /// (NewMain 1.41) keeps showing the grab pose until the next action.
    pub fn release_grab(&mut self) -> bool {
        let released = self.pushpull.grab.take().is_some();
        if released && !self.pushpull.pull_shown && self.mannerisms.pull_release == crate::protocol::PullRelease::BackToGrab {
            self.pushpull.grab_shown = true;
        }
        released
    }

    /// An arrow key was let go: a push ends, and a pull stops its cycle (the
    /// pull pose stays). True if a push or a pull cycle ended.
    pub fn arrow_released(&mut self) -> bool {
        let was_pushing = self.push_pose();
        self.pushpull.pushing = false;
        let back_to_grab = self.mannerisms.pull_release == crate::protocol::PullRelease::BackToGrab;
        match &mut self.pushpull.grab {
            Some(g) if g.pulling() => {
                g.count = 1;
                // NewMain 1.41 goes back to the grab pose; 2.05 keeps pulling.
                if back_to_grab {
                    self.pushpull.pull_shown = false;
                }
                true
            }
            _ => was_pushing,
        }
    }

    /// An arrow key while grabbing: the arrow away from the wall starts the
    /// pull. Nothing moves and the facing stays.
    pub(crate) fn grab_arrow(&mut self, dx: f32, dy: f32) {
        let Some(g) = self.pushpull.grab.as_mut() else { return };
        let away = match opposite(self.player_dir & 3) {
            DIR_NORTH => dy < 0.0,
            DIR_WEST => dx < 0.0,
            DIR_SOUTH => dy > 0.0,
            _ => dx > 0.0,
        };
        if !away {
            return;
        }
        if g.count == 1 {
            g.count = 2;
            if !self.pushpull.pull_shown {
                self.pushpull.pull_ticks = 0;
            }
            self.pushpull.pull_shown = true;
        } else if g.count == PULL_AT {
            // A pulled block moves the player, and the grab goes on.
            self.try_pull();
        }
    }

    /// Advance the jump, the pull cycle and the pull pose by `ms` on the
    /// 50 ms tick. Returns true if the player moved (the caller sends the
    /// position).
    pub fn tick_pushpull(&mut self, ms: u64) -> bool {
        // A swing, a hit or a lift sets its own gani: the stale pull is gone.
        if self.combat.swing_frame().is_some() || self.combat.is_hurt() || self.carried.is_some() || self.life.is_dead() {
            self.pushpull.pull_shown = false;
            self.pushpull.grab_shown = false;
        }
        if self.pushpull.jump.is_none() && !self.pushpull.pull_pose() {
            self.pushpull.clock_ms = 0;
            return std::mem::take(&mut self.pushpull.moved);
        }
        self.pushpull.clock_ms += ms;
        while self.pushpull.clock_ms >= TICK_MS {
            self.pushpull.clock_ms -= TICK_MS;
            if self.pushpull.jump.is_some() {
                self.jump_step();
            }
            if let Some(g) = self.pushpull.grab.as_mut() {
                if g.count > 1 {
                    g.count = (g.count - 1) % PULL_CYCLE + 2;
                }
            }
            if self.pushpull.pull_pose() {
                self.pushpull.pull_ticks += 1;
            }
        }
        std::mem::take(&mut self.pushpull.moved)
    }

    /// The sprite number S of the push, grab or pull pose, if one is on.
    pub fn pushpull_sprite_s(&self) -> Option<u8> {
        self.pushpull_sprite_s_stepped(true)
    }

    /// As [`Self::pushpull_sprite_s`]; with `stepped` false the push and
    /// pull poses keep their first frame's number.
    pub fn pushpull_sprite_s_stepped(&self, stepped: bool) -> Option<u8> {
        let frame = |g: &Option<crate::assets::Gani>, t: usize| if stepped { g.as_ref().map_or(0, |g| g.frame_at_tick(t, true)) } else { 0 };
        if self.pushpull.pull_pose() {
            return Some(PULL_SPRITE_S[frame(&self.poses.pull, self.pushpull.pull_ticks as usize).min(2)]);
        }
        if self.pushpull.grab.is_some() || self.pushpull.grab_shown {
            return Some(GRAB_SPRITE_S);
        }
        self.push_tick().map(|t| PUSH_SPRITE_S[frame(&self.poses.push, t).min(4)])
    }

    /// The NPCs pushes and pulls moved since the last call.
    pub fn take_npc_moves(&mut self) -> Vec<NpcMoved> {
        std::mem::take(&mut self.pushpull.npc_moves)
    }

    /// The `pushed` / `pulled` triggers since the last call.
    pub fn take_triggers(&mut self) -> Vec<Trigger> {
        std::mem::take(&mut self.pushpull.triggers)
    }

    /// Drop a jump or grab in progress (a level change).
    pub(crate) fn reset_pushpull(&mut self) {
        self.pushpull = PushPull::default();
        self.board_changes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{Level, ObjectTables, TileTypes};

    const WALL: u16 = 100;
    const WATER: u16 = 101;
    const LEDGE_UP: u16 = 190;
    const LEDGE_LEFT: u16 = 212;
    const PUSH_BLOCK: [[u16; 2]; 2] = [[1764, 1780], [1765, 1781]];
    const PULL_BLOCK: [[u16; 2]; 2] = [[1998, 2014], [1999, 2015]];
    const FLOOR: u16 = 0;
    const FLOOR2: u16 = 7;

    fn types() -> TileTypes {
        let mut t = TileTypes::from_types([0; 4096]);
        for id in [WALL, LEDGE_UP, LEDGE_LEFT] {
            t.types[id as usize] = 22;
        }
        for o in [PUSH_BLOCK, PULL_BLOCK] {
            for col in o {
                for id in col {
                    t.types[id as usize] = 22;
                }
            }
        }
        t.types[WATER as usize] = 11;
        t
    }

    /// `arrays.dat` with array 8 (ledges per facing) and array 16 (blocks),
    /// the rest empty but for one cut and one lift entry so it parses.
    fn arrays() -> Vec<u8> {
        fn ints(out: &mut Vec<u8>, v: &[u32]) {
            out.extend_from_slice(&(v.len() as u32).to_be_bytes());
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
        fn objs(out: &mut Vec<u8>, list: &[[[u16; 2]; 2]]) {
            out.extend_from_slice(&((1u32 << 16) | list.len() as u32).to_be_bytes());
            for o in list {
                out.extend_from_slice(&((1u32 << 16) | 2).to_be_bytes());
                for col in o {
                    ints(out, &[u32::from(col[0]), u32::from(col[1])]);
                }
            }
        }
        let mut out = Vec::new();
        for i in 0..25 {
            match i {
                8 => {
                    out.extend_from_slice(&((1u32 << 16) | 4).to_be_bytes());
                    ints(&mut out, &[u32::from(LEDGE_UP)]);
                    ints(&mut out, &[u32::from(LEDGE_LEFT)]);
                    ints(&mut out, &[]);
                    ints(&mut out, &[]);
                }
                9..=12 => objs(&mut out, &[[[2, 18], [3, 19]]]),
                16 => objs(&mut out, &[PUSH_BLOCK, PULL_BLOCK]),
                _ => ints(&mut out, &[]),
            }
        }
        out
    }

    fn session(f: impl Fn(i32, i32) -> u16) -> OfflineSession {
        let tiles: Vec<u16> = (0..64 * 64).map(|i| f(i % 64, i / 64)).collect();
        let mut s = OfflineSession::new();
        s.current_level = Some(Level::from_board(&tiles));
        s.tiletypes = Some(types());
        s.object_tables = ObjectTables::parse(&arrays());
        assert!(s.object_tables.as_ref().is_some_and(|t| t.blocks.len() == 2), "the test arrays parse");
        s
    }

    fn tile(s: &OfflineSession, x: i32, y: i32) -> u16 {
        s.current_level.as_ref().unwrap().tile_id_at(x, y).unwrap()
    }

    fn put(s: &mut OfflineSession, ax: i32, ay: i32, o: Object2x2) {
        s.current_level.as_mut().unwrap().put_object(ax, ay, &o);
    }

    #[test]
    fn the_tables_load_from_arrays_dat() {
        let t = ObjectTables::parse(&arrays()).unwrap();
        assert_eq!(t.ledges[0], vec![LEDGE_UP]);
        assert_eq!(t.ledges[1], vec![LEDGE_LEFT]);
        assert_eq!(t.blocks, vec![PUSH_BLOCK, PULL_BLOCK]);
    }

    /// A row of up-ledges at y = 14 with open ground above and below: the
    /// player walks up into it.
    fn ledge_row() -> OfflineSession {
        let mut s = session(|_, y| if y == 14 { LEDGE_UP } else { FLOOR });
        s.player_x = 20.5;
        s.player_y = 14.0;
        s.player_dir = DIR_NORTH;
        s
    }

    #[test]
    fn running_into_an_up_ledge_jumps_after_18_blocked_steps() {
        let mut s = ledge_row();
        for i in 1..18 {
            assert!(s.try_move(0.0, -0.5).is_none(), "step {i} is refused");
            assert!(!s.jumping());
        }
        assert_eq!(s.pushpull.blocked_steps, 17);
        assert!(!s.push_pose(), "the push pose starts after 17");
        assert!(s.walking, "the player runs in place against the ledge");
        s.try_move(0.0, -0.5);
        assert!(s.jumping(), "the 18th blocked step jumps");
        // The first step of the flight is shown at once.
        assert_eq!((s.player_x, s.player_y), (20.5, 12.5));
        assert!(s.take_sound_events().iter().any(|e| e.file == "jump.wav" && e.x == 22.0 && e.y == 16.0));
        // Arrow keys do nothing in the air.
        assert!(s.try_move(0.5, 0.0).is_none());
        let ys: Vec<f32> = (0..7).map(|_| {
            s.tick_pushpull(50);
            s.player_y
        }).collect();
        assert_eq!(ys, vec![11.3, 10.2, 9.5, 9.0, 8.8, 8.8, 9.0]);
        assert!(!s.jumping(), "landed after 8 ticks");
        assert!(s.try_move(0.0, -0.5).is_some(), "walking again");
    }

    #[test]
    fn a_blocked_landing_spot_stops_the_jump() {
        let mut s = ledge_row();
        // Landing point (x + 1.5, y + 2 - 5) = (22, 11).
        s.current_level.as_mut().unwrap().set_tile_id(22, 11, WALL);
        for _ in 0..60 {
            s.try_move(0.0, -0.5);
        }
        assert!(!s.jumping());
        assert_eq!(s.player_y, 14.0);
    }

    #[test]
    fn a_plain_wall_shows_the_push_pose_but_never_jumps() {
        let mut s = session(|_, y| if y == 14 { WALL } else { FLOOR });
        s.player_x = 20.5;
        s.player_y = 14.0;
        for _ in 0..18 {
            s.try_move(0.0, -0.5);
        }
        assert!(s.push_pose());
        for _ in 0..60 {
            s.try_move(0.0, -0.5);
        }
        assert!(!s.jumping());
        // The count wraps at 38 to 18 and keeps going.
        assert!((LEDGE_TEST_AT..BLOCKED_WRAP).contains(&s.pushpull.blocked_steps));
        // A free step ends it.
        s.try_move(0.0, 0.5);
        assert!(!s.push_pose());
        assert_eq!(s.pushpull.blocked_steps, 0);
    }

    #[test]
    fn the_push_pose_holds_each_step_for_its_wait_and_restarts_at_the_wrap() {
        let mut s = session(|_, y| if y == 14 { WALL } else { FLOOR });
        // The stock push.gani: five steps of WAIT 3 (4 ticks), looping.
        let step = "1 0 0\n1 0 0\n1 0 0\n1 0 0\nWAIT 3\n\n";
        s.poses.push = Some(crate::assets::Gani::parse(&format!("LOOP\nANI\n{}ANIEND\n", step.repeat(5))).unwrap());
        s.player_x = 20.5;
        s.player_y = 14.0;
        let mut seen = Vec::new();
        for _ in 0..40 {
            s.try_move(0.0, -0.5);
            seen.push(s.current_pose().map(|p| (p.kind, p.frame)));
        }
        use crate::pose::PoseKind::{Push, Walk};
        // Steps 1..17 run in place (no walk gani here: the idle fallback).
        assert!(seen[..17].iter().all(|p| p.map_or(true, |p| p.0 != Push && p.0 != Walk)));
        let push: Vec<usize> = seen[17..].iter().map(|p| p.unwrap().1).collect();
        assert_eq!(&push[..9], [0, 0, 0, 0, 1, 1, 1, 1, 2]);
        assert_eq!(push[20], 0, "the count wraps from 38 to 18: frame 0 again");
        // The sprite the others see follows the same frame.
        assert_eq!(s.pushpull_sprite_s(), Some(PUSH_SPRITE_S[push[22]]));
    }

    #[test]
    fn a_gani_client_sends_one_push_sprite_while_it_pushes() {
        let mut s = session(|_, y| if y == 14 { WALL } else { FLOOR });
        let step = "1 0 0\n1 0 0\n1 0 0\n1 0 0\nWAIT 3\n\n";
        s.poses.push = Some(crate::assets::Gani::parse(&format!("LOOP\nANI\n{}ANIEND\n", step.repeat(5))).unwrap());
        s.player_x = 20.5;
        s.player_y = 14.0;
        let mut gani = Vec::new();
        let mut stepped = Vec::new();
        for _ in 0..40 {
            s.try_move(0.0, -0.5);
            gani.push(s.sprite_value_for(true));
            stepped.push(s.sprite_value_for(false));
        }
        gani.dedup();
        stepped.dedup();
        assert_eq!(gani, [DIR_NORTH, DIR_NORTH + 4 * PUSH_SPRITE_S[0]], "walk, then the push sprite once");
        assert!(stepped.len() > 2, "the 2.05 numbers step with the frame");
    }

    #[test]
    fn a_left_ledge_jumps_out_and_down() {
        let mut s = session(|x, _| if x == 16 { LEDGE_LEFT } else { FLOOR });
        s.player_x = 16.5;
        s.player_y = 20.0;
        s.player_dir = DIR_WEST;
        for _ in 0..18 {
            s.try_move(-0.5, 0.0);
        }
        assert!(s.jumping());
        for _ in 0..7 {
            s.tick_pushpull(50);
        }
        assert_eq!((s.player_x, s.player_y), (10.0, 23.0));
    }

    #[test]
    fn the_push_block_moves_one_tile_on_the_29th_blocked_step() {
        let mut s = session(|_, y| if y < 10 { FLOOR2 } else { FLOOR });
        put(&mut s, 20, 10, PUSH_BLOCK);
        s.player_x = 19.5;
        s.player_y = 11.0;
        s.player_dir = DIR_NORTH;
        for _ in 0..28 {
            s.try_move(0.0, -0.5);
        }
        assert_eq!(tile(&s, 20, 10), PUSH_BLOCK[0][0], "not yet");
        s.try_move(0.0, -0.5);
        assert_eq!(tile(&s, 20, 9), PUSH_BLOCK[0][0], "the block went up a tile");
        assert_eq!(tile(&s, 21, 10), PUSH_BLOCK[1][1]);
        assert_eq!(tile(&s, 20, 11), FLOOR2, "the vacated row takes the tile from two ahead");
        assert_eq!(s.player_y, 10.0, "the player follows one tile");
        assert!(s.take_sound_events().iter().any(|e| e.file == "stonemove.wav"));
        let c = s.take_board_changes();
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].x, c[0].y, c[0].width, c[0].height), (19, 9, 4, 4), "the 4 x 4 around the old spot");
        assert_eq!(c[0].tiles[1], PUSH_BLOCK[0][0], "the new tiles, row by row");
        let t = s.take_triggers();
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].action, t[0].dir), ("pushed", DIR_NORTH));
    }

    #[test]
    fn a_push_block_against_water_or_a_wall_stays() {
        for blocker in [WALL, WATER] {
            let mut s = session(|_, _| FLOOR);
            put(&mut s, 20, 10, PUSH_BLOCK);
            s.current_level.as_mut().unwrap().set_tile_id(21, 9, blocker);
            s.player_x = 19.5;
            s.player_y = 11.0;
            for _ in 0..40 {
                s.try_move(0.0, -0.5);
            }
            assert_eq!(tile(&s, 20, 10), PUSH_BLOCK[0][0]);
            assert_eq!(s.player_y, 11.0);
            assert!(!s.take_triggers().is_empty(), "the push still fires `pushed`");
        }
    }

    /// Below a wall at y = 10, holding A, facing up.
    fn grabbing_a_wall() -> OfflineSession {
        let mut s = session(|_, y| if y == 10 { WALL } else { FLOOR });
        s.player_x = 19.5;
        s.player_y = 10.0;
        s.player_dir = DIR_NORTH;
        assert!(s.try_grab(), "A grabs a wall");
        s
    }

    #[test]
    fn a_futile_pull_cycles_without_moving_anything() {
        let mut s = grabbing_a_wall();
        assert_eq!(s.pushpull.grab, Some(Grab { count: 1 }));
        // Arrows other than the one away from the wall do nothing.
        assert!(s.try_move(0.5, 0.0).is_none());
        assert_eq!(s.player_dir, DIR_NORTH, "the facing stays");
        assert!(!s.pushpull.grab.unwrap().pulling());
        // Down (away from the wall) starts the pull.
        s.try_move(0.0, 0.5);
        assert!(s.pushpull.grab.unwrap().pulling());
        let mut pulls = 0;
        for _ in 0..36 {
            s.tick_pushpull(50);
            s.try_move(0.0, 0.5);
            pulls += s.take_triggers().iter().filter(|t| t.action == "pulled").count();
        }
        assert_eq!(pulls, 2, "one pull try every 18 ticks");
        assert_eq!((s.player_x, s.player_y), (19.5, 10.0), "nothing moved");
        assert!(s.take_board_changes().is_empty());
        // Letting go of the arrow: the cycle stops, the pull pose goes on.
        // Letting go of A ends the grab; the pull pose still goes on, and
        // keeps its frame clock, until the player moves.
        assert!(s.arrow_released());
        assert_eq!(s.pushpull.grab, Some(Grab { count: 1 }));
        assert!(s.pushpull.pull_pose());
        assert!(s.release_grab());
        let t = s.pushpull.pull_ticks;
        s.tick_pushpull(100);
        assert_eq!(s.pushpull.pull_ticks, t + 2);
        assert!(s.pushpull.pull_pose());
        assert_eq!(s.pushpull_sprite_s().map(|v| PULL_SPRITE_S.contains(&v)), Some(true));
        assert!(s.try_move(0.0, 0.5).is_some(), "walks again");
        assert!(!s.pushpull.pull_pose());
        assert_eq!(s.pushpull_sprite_s(), None);
    }

    #[test]
    fn newmain_141_lets_a_pull_go_back_to_the_grab() {
        let mut s = grabbing_a_wall();
        s.set_mannerisms(crate::protocol::MANNER_141);
        s.try_move(0.0, 0.5);
        assert!(s.pushpull.pull_pose());
        // Letting go of the arrow: back to the grab pose.
        s.arrow_released();
        assert!(!s.pushpull.pull_pose());
        assert_eq!(s.pushpull_sprite_s(), Some(GRAB_SPRITE_S));
        // Letting go of A: the grab pose stays until the player moves.
        assert!(s.release_grab());
        assert_eq!(s.pushpull_sprite_s(), Some(GRAB_SPRITE_S));
        assert!(s.try_move(0.0, 0.5).is_some());
        assert_eq!(s.pushpull_sprite_s(), None);
    }

    #[test]
    fn the_pull_block_comes_along_one_tile() {
        let mut s = session(|_, y| if y < 10 { FLOOR2 } else { FLOOR });
        put(&mut s, 20, 8, PULL_BLOCK);
        s.player_x = 19.5;
        s.player_y = 9.0;
        s.player_dir = DIR_NORTH;
        assert!(s.try_grab());
        s.try_move(0.0, 0.5);
        for _ in 0..17 {
            s.tick_pushpull(50);
        }
        assert_eq!(s.pushpull.grab.unwrap().count, PULL_AT);
        s.try_move(0.0, 0.5);
        assert_eq!(tile(&s, 20, 9), PULL_BLOCK[0][0], "the block came down a tile");
        assert_eq!(tile(&s, 20, 8), FLOOR, "its old top row takes the tile from two behind");
        assert_eq!(s.player_y, 10.0, "the player stepped back");
        let t = s.take_triggers();
        assert_eq!((t[0].action, t[0].dir), ("pulled", DIR_SOUTH));
        assert!(s.tick_pushpull(0), "the step back goes out");
    }

    fn add_npc(s: &mut OfflineSession, x: f32, y: f32, push: bool, pull: bool) {
        let mut n = Npc::new(String::new(), x, y, String::new());
        n.pushable = push;
        n.pullable = pull;
        s.current_level.as_mut().unwrap().npcs.push(n);
    }

    fn npc_pos(s: &OfflineSession) -> (f32, f32) {
        let n = &s.current_level.as_ref().unwrap().npcs[0];
        (n.x, n.y)
    }

    #[test]
    fn a_canbepushed_npc_moves_one_tile_and_the_player_follows() {
        let mut s = session(|_, _| FLOOR);
        add_npc(&mut s, 20.0, 10.0, true, false);
        s.player_x = 19.5;
        s.player_y = 11.0;
        s.player_dir = DIR_NORTH;
        for _ in 0..28 {
            s.try_move(0.0, -0.5);
        }
        assert_eq!(npc_pos(&s), (20.0, 10.0), "it blocks, and has not moved yet");
        assert_eq!(s.player_y, 11.0);
        s.try_move(0.0, -0.5);
        assert_eq!(npc_pos(&s), (20.0, 9.0), "one tile up");
        assert_eq!(s.player_y, 10.0, "the player follows");
        assert_eq!(s.take_npc_moves(), vec![NpcMoved { index: 0, x: 20.0, y: 9.0, event: "waspushed" }]);
        assert_eq!(s.take_triggers()[0].action, "pushed", "actionpushed comes first");
        assert!(s.take_sound_events().iter().any(|e| e.file == "stonemove.wav"));
    }

    #[test]
    fn a_pushed_npc_stops_at_a_wall_and_a_plain_npc_does_not_move() {
        let mut s = session(|_, y| if y == 9 { WALL } else { FLOOR });
        add_npc(&mut s, 20.0, 10.0, true, false);
        s.player_x = 19.5;
        s.player_y = 11.0;
        s.player_dir = DIR_NORTH;
        for _ in 0..29 {
            s.try_move(0.0, -0.5);
        }
        assert_eq!(npc_pos(&s), (20.0, 10.0), "the cells above it are wall");
        assert!(s.take_npc_moves().is_empty());

        let mut s = session(|_, _| FLOOR);
        add_npc(&mut s, 20.0, 10.0, false, true);
        s.player_x = 19.5;
        s.player_y = 11.0;
        s.player_dir = DIR_NORTH;
        for _ in 0..29 {
            s.try_move(0.0, -0.5);
        }
        assert_eq!(npc_pos(&s), (20.0, 10.0), "canbepulled only: a push does not move it");
    }

    #[test]
    fn a_canbepulled_npc_comes_along_one_tile() {
        let mut s = session(|_, _| FLOOR);
        add_npc(&mut s, 20.0, 8.0, false, true);
        s.player_x = 19.5;
        s.player_y = 9.0;
        s.player_dir = DIR_NORTH;
        assert!(s.try_grab(), "a pullable NPC can be held");
        s.try_move(0.0, 0.5);
        for _ in 0..17 {
            s.tick_pushpull(50);
        }
        s.try_move(0.0, 0.5);
        assert_eq!(npc_pos(&s), (20.0, 9.0), "one tile towards the player");
        assert_eq!(s.player_y, 10.0, "the player stepped back");
        assert_eq!(s.take_npc_moves(), vec![NpcMoved { index: 0, x: 20.0, y: 9.0, event: "waspulled" }]);
    }

    /// `new3.nw` with the ledge and block lists of the 2.22 install's
    /// `arrays.dat` (array 8 and array 16). Ledge and block tiles block; the
    /// rest of the level is open here.
    fn new3() -> OfflineSession {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/levels/new3.nw");
        let mut s = OfflineSession::new();
        s.current_level = Some(Level::load(path).expect("new3.nw loads"));
        let mut t = ObjectTables::default();
        t.ledges = [vec![190, 191, 4045, 4046], vec![212, 228, 563, 579, 4028, 4044], vec![185, 186, 4061, 4062], vec![273, 289, 4031, 4047]];
        t.blocks = vec![PUSH_BLOCK, PULL_BLOCK];
        let mut types = TileTypes::from_types([0; 4096]);
        for id in t.ledges.iter().flatten().copied().chain(t.blocks.iter().flatten().flatten().copied()) {
            types.types[usize::from(id)] = 22;
        }
        s.object_tables = Some(t);
        s.tiletypes = Some(types);
        s
    }

    #[test]
    fn new3_jumps_up_the_ledge_row_and_pushes_its_block() {
        let mut s = new3();
        assert_eq!(tile(&s, 22, 14), 190, "the up-ledge row is at y = 14");
        s.player_x = 21.5;
        s.player_y = 14.0;
        s.player_dir = DIR_NORTH;
        for _ in 0..18 {
            s.try_move(0.0, -0.5);
        }
        assert!(s.jumping(), "running up into the ledge row jumps");
        for _ in 0..7 {
            s.tick_pushpull(50);
        }
        assert_eq!((s.player_x, s.player_y), (21.5, 9.0), "five tiles up");

        // The push block at 34,19 (2 x 2), pushed from below.
        let mut s = new3();
        assert_eq!(tile(&s, 34, 19), PUSH_BLOCK[0][0]);
        s.player_x = 33.5;
        s.player_y = 20.0;
        for _ in 0..29 {
            s.try_move(0.0, -0.5);
        }
        assert_eq!(tile(&s, 34, 18), PUSH_BLOCK[0][0], "moved up one tile");
        assert_eq!(s.player_y, 19.0);
    }

    #[test]
    fn grab_needs_a_wall_in_front() {
        let mut s = session(|_, _| FLOOR);
        s.player_x = 19.5;
        s.player_y = 10.0;
        assert!(!s.try_grab());
        assert!(s.pushpull.grab.is_none());
    }
}
