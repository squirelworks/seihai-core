//! Carried and thrown objects: the model.
//!
//! Everything here is derived from the 2.05 client and the 2.22 `arrays.dat`
//! (see `documentation/throwing.md`); values that were not read are named in
//! that file as open and are not guessed here.
//!
//! Units: tiles (16 px). A *flight tick* is one step of the object; the
//! client runs them on its 50 ms game clock ([`TICK_MS`]; the clock is the
//! one the walk and the swing use, the thrown object's own interval was not
//! read separately).

/// Milliseconds per flight tick and per break-effect frame.
pub const TICK_MS: u64 = 50;
/// Ticks of flight before the object comes down by itself.
pub const FLIGHT_TICKS: i32 = 9;

/// The sprite (an id in the sprite table) drawn for each lift-table entry:
/// bush, block, and the three heavier objects.
pub const CARRY_SPRITES: [u16; 5] = [261, 267, 265, 263, 239];

/// The client's carry table, in the order of the script names (`bush`,
/// `sign`, `vase`, `stone`, `blackstone`, `bomb`, `hotbomb`, `superbomb`,
/// `joltbomb`, `hotjoltbomb`, `none`): the carried sprite of each, -1 none
/// (2.05).
pub const CARRY_TABLE: [i32; 11] = [261, 267, 265, 263, 239, 65, 66, 321, 347, 348, -1];

/// The CARRYSPRITE (19) value the 2.05 client sends for what it carries:
/// -1 empty hands, 0 for sprite 65, 200 for 66, 201 for 239, otherwise the
/// sprite id minus 260 (the bush, 261, is 1).
pub fn carry_wire_code(sprite: Option<u16>) -> i32 {
    match sprite {
        None => -1,
        Some(65) => 0,
        Some(66) => 200,
        Some(239) => 201,
        Some(s) => i32::from(s) - 260,
    }
}

/// The carried sprite a received CARRYSPRITE (19) value names (2.05): the
/// inverse of [`carry_wire_code`]. Negative is empty hands. A value whose
/// sprite is not in [`CARRY_TABLE`] becomes the table's first (the bush).
pub fn carry_sprite_from_wire(code: i32) -> Option<u16> {
    if code < 0 {
        return None;
    }
    let s = match code {
        0 => 65,
        200 => 66,
        201 => 239,
        c => c + 260,
    };
    let s = if CARRY_TABLE.contains(&s) { s } else { CARRY_TABLE[0] };
    u16::try_from(s).ok()
}

/// The CARRYSPRITE byte as a GCHAR on the wire (value + 32).
pub fn carry_wire_byte(sprite: Option<u16>) -> u8 {
    (carry_wire_code(sprite) as u8).wrapping_add(0x20)
}

/// The value a received CARRYSPRITE byte holds: GCHAR minus 32, with the
/// byte 31 (the client's -1) as -1. Byte 255 is -1 too: GServer-v2 stores
/// the prop unsigned and clamps it on the way out, so the -1 a client sent
/// reaches the others as 255 (lab, 2026-10-28). Read as 223 it would be no
/// sprite of the table, which the 2.05 receiver turns into the bush; Seihai
/// takes it as empty hands instead (a Seihai choice).
pub fn carry_code_from_byte(b: u8) -> i32 {
    if b == 0xFF {
        return -1;
    }
    i32::from(b) - 0x20
}

/// Carry sprite ids of things that stay on the ground when thrown (bombs).
pub const STAY_SPRITES: [u16; 5] = [65, 66, 321, 347, 348];

/// The tile type objects fly through (a wall type that does not stop a
/// thrown object).
pub const TYPE_THROW_THROUGH: u8 = crate::assets::TYPE_THROW_THROUGH;

/// The shadow under a thrown object: sprite 0.
pub const SHADOW_SPRITE: u16 = 0;

/// Sprite 261 (the bush) breaks into leaves, 267 into pot pieces.
pub const SPRITE_BUSH: u16 = 261;
pub const SPRITE_POT: u16 = 267;

/// Break-effect kinds (rows of the break table).
pub const LEAP_BUSH: usize = 0;
/// Row 1: three pieces of sprite 24, the effect of the second cuttable object.
pub const LEAP_CUT_2: usize = 1;
pub const LEAP_ROCK: usize = 2;
pub const LEAP_POT: usize = 3;
pub const LEAP_SPLASH: usize = 5;

/// The break effect a sword cut of cut-table entry `entry` plays: the entry
/// number is the break-table row (entry 0, the bush, plays the bush's leaves;
/// entry 1 the next row). The spawn routine of the client takes the entry
/// index; that it picks the break row by it is an inference, not read.
pub fn cut_leap_kind(entry: usize) -> usize {
    entry
}

/// Per-tick step of a thrown object by facing (north, west, south, east):
/// `(dx, dy, dz)`. `dy` moves the drawn object, `dz` the ground point under
/// it (the shadow and the hit test).
pub const STEP: [(f32, f32, f32); 4] = [(0.0, -0.8, -1.0), (-1.0, 0.2, 0.0), (0.0, 1.2, 1.0), (1.0, 0.2, 0.0)];

/// Where a carried object is drawn, and where a throw starts, from the
/// player's sprite top-left.
pub fn carry_origin(px: f32, py: f32) -> (f32, f32) {
    (px + 0.5, py - 1.375)
}

/// Where the object is drawn while its carrier hides under it (2.05 on):
/// where the player stands.
pub fn hide_origin(px: f32, py: f32) -> (f32, f32) {
    (px + 0.5, py + 1.0)
}

/// The sprite carried for a lift-table entry, if the table has one for it.
pub fn carry_sprite(kind: usize) -> Option<u16> {
    CARRY_SPRITES.get(kind).copied()
}

/// Whether a carried sprite stays on the ground when it lands.
pub fn stays_on_ground(sprite: Option<u16>) -> bool {
    sprite.is_some_and(|s| STAY_SPRITES.contains(&s))
}

/// Which break effect a landing starts: splash on water, leaves for the
/// bush, pot pieces for the block, rock pieces otherwise.
pub fn leap_kind(sprite: Option<u16>, water: bool) -> usize {
    if water {
        return LEAP_SPLASH;
    }
    match sprite {
        Some(SPRITE_BUSH) => LEAP_BUSH,
        Some(SPRITE_POT) => LEAP_POT,
        _ => LEAP_ROCK,
    }
}

/// An object in flight.
#[derive(Debug, Clone, PartialEq)]
pub struct Thrown {
    /// Lift-table entry it came from.
    pub kind: usize,
    pub sprite: Option<u16>,
    /// Top-left of the drawn sprite.
    pub x: f32,
    pub y: f32,
    /// The ground point's y (the shadow is drawn from it).
    pub ground: f32,
    pub dir: u8,
    pub ticks_left: i32,
    /// The player id of the thrower, for a throw another player made
    /// (`PLO_THROWCARRIED`). `None` for the player's own throws.
    pub thrower: Option<u16>,
    /// The player's own throw: it never hits the player (the 2.05 client
    /// marks its own throws and skips the player test for them).
    pub own: bool,
}

impl Thrown {
    /// A throw from the player at `(px, py)` facing `dir`.
    pub fn launch(kind: usize, px: f32, py: f32, dir: u8) -> Self {
        let (x, y) = carry_origin(px, py);
        Thrown { kind, sprite: carry_sprite(kind), x, y, ground: py + 1.0, dir, ticks_left: FLIGHT_TICKS, thrower: None, own: true }
    }

    /// A throw another player made (`PLO_THROWCARRIED`): it starts at that
    /// player's position `(px, py)` with what they carried, flies their way
    /// for the same 9 ticks, and can hit this player. `kind` is the sprite's
    /// lift-table entry, or `usize::MAX` for a sprite the lift table does
    /// not have (a bomb).
    pub fn launch_remote(sprite: u16, px: f32, py: f32, dir: u8, thrower: u16) -> Self {
        let (x, y) = carry_origin(px, py);
        let kind = CARRY_SPRITES.iter().position(|&s| s == sprite).unwrap_or(usize::MAX);
        Thrown { kind, sprite: Some(sprite), x, y, ground: py + 1.0, dir, ticks_left: FLIGHT_TICKS, thrower: Some(thrower), own: false }
    }

    /// Whether the object, after its step, is on the player whose sprite
    /// top-left is `(px, py)`: under 2 tiles from `(px + 0.5, py + 1)` on
    /// each axis, measured from the object's `x` and ground point (2.05).
    pub fn hits_player(&self, px: f32, py: f32) -> bool {
        (self.x - (px + 0.5)).abs() < 2.0 && (self.ground - (py + 1.0)).abs() < 2.0
    }

    /// One flight tick: move by the facing's step and use up a tick.
    pub fn step(&mut self) {
        let (dx, dy, dz) = STEP[usize::from(self.dir & 3)];
        self.x += dx;
        self.y += dy;
        self.ground += dz;
        self.ticks_left -= 1;
    }

    /// The point tested for walls: one tile right of and below the ground
    /// point `(x, ground)`.
    pub fn wall_probe(&self) -> (f32, f32) {
        (self.x + 1.0, self.ground + 1.0)
    }

    /// The point tested for water: one tile right of and below the object's
    /// own `(x, y)`.
    pub fn water_probe(&self) -> (f32, f32) {
        (self.x + 1.0, self.y + 1.0)
    }
}

/// Half hearts a thrown object takes from the player it hits: one heart, or
/// half a heart when the player has exactly one heart left (2.05). So a hit
/// at one heart leaves half a heart, and the next one kills.
pub fn throw_damage_half(hearts_half: i32) -> i32 {
    if hearts_half == 2 {
        1
    } else {
        2
    }
}

/// The knockback a thrown object gives: one tile along the direction it
/// flies (north, west, south, east).
pub fn throw_knock(dir: u8) -> (f32, f32) {
    [(0.0, -1.0), (-1.0, 0.0), (0.0, 1.0), (1.0, 0.0)][usize::from(dir & 3)]
}

/// A break effect (the client calls the class "leap") playing out.
#[derive(Debug, Clone, PartialEq)]
pub struct Leap {
    pub kind: usize,
    /// Anchor: the thrown object's x and ground y at the moment it landed.
    pub x: f32,
    pub y: f32,
    /// Frame being shown; `-1` until the first tick.
    pub frame: i32,
}

/// An object that landed and stays (a bomb).
#[derive(Debug, Clone, PartialEq)]
pub struct Grounded {
    pub sprite: u16,
    pub x: f32,
    pub y: f32,
}

/// One piece of a break-effect frame: sprite id and its offset from the
/// effect's anchor in pixels.
pub type Piece = (u16, i8, i8);

/// The break table of the 2.22 `arrays.dat` (array 19) for the kinds this
/// client plays: `[frame][piece]`. Used when no `arrays.dat` is loaded.
pub fn builtin_leap(kind: usize) -> &'static [&'static [Piece]] {
    match kind {
        LEAP_BUSH => &[
            &[(25, 0, 4), (25, 6, 0), (26, 6, 8), (26, 9, 17)],
            &[(25, -2, 4), (25, 4, -4), (26, 8, 10), (26, 9, 19)],
            &[(26, -4, 4), (25, 2, -8), (26, 10, 12), (26, 9, 21)],
            &[(25, -5, 4), (25, 1, -10), (25, 9, 13), (25, 9, 22)],
            &[(25, -6, 4), (25, 0, -12), (25, 10, 14), (25, 10, 23)],
            &[(26, -7, 4), (25, -1, -14), (26, 10, 15), (26, 10, 24)],
            &[(26, -8, 4), (25, -2, -16), (26, 10, 22), (26, 10, 25)],
        ],
        LEAP_CUT_2 => &[
            &[(24, 0, 8), (24, 8, 0), (24, 12, 8)],
            &[(24, -2, 8), (24, 8, -2), (24, 14, 8)],
            &[(24, -4, 8), (24, 8, -4), (24, 16, 8)],
            &[(24, -6, 8), (24, 8, -6), (24, 18, 8)],
        ],
        LEAP_ROCK => &[
            &[(96, -1, 0), (96, 0, 9), (98, 9, -1), (98, 10, 8)],
            &[(99, -3, -2), (99, -2, 11), (97, 11, -3), (97, 12, 10)],
            &[(96, -5, -4), (96, -4, 13), (98, 13, -5), (98, 14, 12)],
            &[(99, -7, -6), (99, -6, 15), (97, 15, -7), (97, 16, 14)],
        ],
        LEAP_POT => &[
            &[(100, -1, 0), (100, 0, 9), (102, 9, -1), (102, 10, 8)],
            &[(103, -3, -2), (103, -2, 11), (101, 11, -3), (101, 12, 10)],
            &[(100, -5, -4), (100, -4, 13), (102, 13, -5), (102, 14, 12)],
            &[(103, -7, -6), (103, -6, 15), (101, 15, -7), (101, 16, 14)],
        ],
        LEAP_SPLASH => &[
            &[(326, -6, 3), (327, 7, 3)],
            &[(326, -8, 1), (327, 9, 1)],
            &[(326, -9, -1), (327, 10, -1)],
            &[(326, -10, -3), (327, 11, -3)],
            &[(326, -13, -4), (327, 14, -4)],
            &[(326, -15, -4), (327, 16, -4)],
            &[(326, -17, -3), (327, 18, -3)],
            &[(326, -19, -1), (327, 20, -1)],
        ],
        _ => &[],
    }
}

/// Source rectangles `(x, y, w, h)` of the sprite ids this client draws, for
/// when no `arrays.dat` sprite table is loaded (the 2.22 table's values).
pub fn builtin_sprite_rect(id: u16) -> Option<(i32, i32, i32, i32)> {
    Some(match id {
        0 => (0, 0, 24, 12),
        24 => (34, 8, 10, 8),
        25 => (44, 0, 16, 16),
        26 => (60, 0, 16, 16),
        96 => (24, 82, 16, 16),
        97 => (40, 82, 16, 16),
        98 => (24, 98, 16, 16),
        99 => (40, 98, 16, 16),
        100 => (56, 82, 16, 14),
        101 => (72, 82, 16, 14),
        102 => (56, 96, 16, 14),
        103 => (72, 96, 16, 14),
        239 => (96, 370, 32, 32),
        261 => (0, 338, 32, 32),
        263 => (96, 338, 32, 32),
        265 => (64, 338, 32, 32),
        267 => (32, 338, 32, 32),
        326 => (24, 114, 30, 30),
        327 => (54, 114, 30, 30),
        _ => return None,
    })
}

/// Unpack a sprite-table entry (array 24): `x = (v >> 24) * 2`,
/// `y = ((v >> 12) & 0xfff) * 2`, `w = ((v >> 6) & 0x3f) * 2`,
/// `h = (v & 0x3f) * 2`.
pub fn unpack_sprite_rect(v: u32) -> (i32, i32, i32, i32) {
    (((v >> 24) * 2) as i32, (((v >> 12) & 0xfff) * 2) as i32, (((v >> 6) & 0x3f) * 2) as i32, ((v & 0x3f) * 2) as i32)
}

/// Unpack a break-table piece (array 19): sprite id in the high 16 bits,
/// then the signed pixel offsets x and y.
pub fn unpack_piece(v: u32) -> Piece {
    ((v >> 16) as u16, ((v >> 8) & 0xff) as u8 as i8, (v & 0xff) as u8 as i8)
}

/// Whether the keys pressed this frame make an interact press: A or Space
/// always, and S too while something is carried (the 2.05 client's sword
/// key throws the carried object instead of swinging; its weapon key, D,
/// does not throw).
pub fn interact_pressed(carrying: bool, a: bool, space: bool, s: bool) -> bool {
    a || space || (carrying && s)
}

/// The source rectangle of sprite `id`: the loaded sprite table, else the
/// built-in value for the ids this client draws.
pub fn sprite_rect(tables: Option<&crate::assets::ObjectTables>, id: u16) -> Option<(i32, i32, i32, i32)> {
    if let Some(v) = tables.and_then(|t| t.sprites.get(usize::from(id))) {
        let r = unpack_sprite_rect(*v);
        if r.2 > 0 && r.3 > 0 {
            return Some(r);
        }
    }
    builtin_sprite_rect(id)
}

/// The frames of break effect `kind`, `[frame][piece]`: the loaded table,
/// else the built-in one.
pub fn leap_frames(tables: Option<&crate::assets::ObjectTables>, kind: usize) -> Vec<Vec<Piece>> {
    if let Some(k) = tables.and_then(|t| t.leaps.get(kind)).filter(|k| !k.is_empty()) {
        return k.iter().map(|f| f.iter().map(|&v| unpack_piece(v)).collect()).collect();
    }
    builtin_leap(kind).iter().map(|f| f.to_vec()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thrown_object_hits_inside_two_tiles_of_the_players_middle_and_feet() {
        let mut o = Thrown::launch_remote(261, 10.0, 30.0, 3, 7);
        assert_eq!((o.kind, o.thrower, o.own), (0, Some(7), false));
        // Player at (14, 30): the test point is (14.5, 31).
        o.x = 12.6;
        assert!(o.hits_player(14.0, 30.0));
        o.x = 12.5;
        assert!(!o.hits_player(14.0, 30.0), "2 tiles is out");
        o.x = 14.5;
        o.ground = 32.9;
        assert!(o.hits_player(14.0, 30.0));
        o.ground = 33.0;
        assert!(!o.hits_player(14.0, 30.0));
        assert_eq!(Thrown::launch_remote(65, 0.0, 0.0, 0, 1).kind, usize::MAX, "a bomb is no lift entry");
        assert!(Thrown::launch(0, 0.0, 0.0, 0).own);
    }

    #[test]
    fn a_thrown_object_takes_a_heart_or_half_of_the_last_one_and_pushes_its_way() {
        assert_eq!(throw_damage_half(6), 2);
        assert_eq!(throw_damage_half(3), 2);
        assert_eq!(throw_damage_half(2), 1);
        assert_eq!(throw_damage_half(1), 2);
        assert_eq!(throw_knock(0), (0.0, -1.0));
        assert_eq!(throw_knock(1), (-1.0, 0.0));
        assert_eq!(throw_knock(2), (0.0, 1.0));
        assert_eq!(throw_knock(3), (1.0, 0.0));
    }

    #[test]
    fn launch_starts_above_the_head_and_ground_under_the_feet() {
        let t = Thrown::launch(0, 10.0, 20.0, 2);
        assert_eq!((t.x, t.y, t.ground), (10.5, 18.625, 21.0));
        assert_eq!((t.sprite, t.ticks_left), (Some(261), 9));
    }

    #[test]
    fn a_s_and_space_throw_while_carrying_and_s_only_then() {
        assert!(interact_pressed(true, true, false, false));
        assert!(interact_pressed(true, false, true, false));
        assert!(interact_pressed(true, false, false, true), "the sword key throws what is carried");
        assert!(!interact_pressed(false, false, false, true), "S empty-handed is a swing, not an interact");
        assert!(interact_pressed(false, true, false, false));
        assert!(!interact_pressed(true, false, false, false));
    }

    #[test]
    fn carry_codes_round_trip_the_2_05_table() {
        assert_eq!(carry_wire_code(None), -1);
        assert_eq!(carry_wire_byte(None), 31);
        let codes: Vec<i32> = CARRY_SPRITES.iter().map(|&s| carry_wire_code(Some(s))).collect();
        assert_eq!(codes, vec![1, 7, 5, 3, 201]);
        assert_eq!(carry_wire_code(Some(65)), 0);
        assert_eq!(carry_wire_code(Some(66)), 200);
        assert_eq!(carry_wire_code(Some(321)), 61);
        for &s in CARRY_TABLE.iter().filter(|&&s| s >= 0) {
            let s = s as u16;
            assert_eq!(carry_sprite_from_wire(carry_code_from_byte(carry_wire_byte(Some(s)))), Some(s), "sprite {s}");
        }
        assert_eq!(carry_sprite_from_wire(carry_code_from_byte(31)), None);
        assert_eq!(carry_sprite_from_wire(carry_code_from_byte(255)), None, "GServer-v2's relayed -1");
        assert_eq!(carry_sprite_from_wire(2), Some(261), "not in the table: the bush");
    }

    #[test]
    fn nine_steps_cover_the_documented_distances() {
        for (dir, dx, dy, dz) in [(0u8, 0.0, -7.2, -9.0), (1, -9.0, 1.8, 0.0), (2, 0.0, 10.8, 9.0), (3, 9.0, 1.8, 0.0)] {
            let mut t = Thrown::launch(1, 30.0, 30.0, dir);
            let start = (t.x, t.y, t.ground);
            for _ in 0..FLIGHT_TICKS {
                t.step();
            }
            assert_eq!(t.ticks_left, 0);
            assert!((t.x - start.0 - dx).abs() < 1e-4, "dir {dir} x");
            assert!((t.y - start.1 - dy).abs() < 1e-4, "dir {dir} y");
            assert!((t.ground - start.2 - dz).abs() < 1e-4, "dir {dir} z");
        }
    }

    #[test]
    fn landing_effect_depends_on_the_object_and_the_water() {
        assert_eq!(leap_kind(Some(261), false), LEAP_BUSH);
        assert_eq!(leap_kind(Some(267), false), LEAP_POT);
        assert_eq!(leap_kind(Some(263), false), LEAP_ROCK);
        assert_eq!(leap_kind(None, false), LEAP_ROCK);
        assert_eq!(leap_kind(Some(261), true), LEAP_SPLASH);
        assert_eq!([0, 1, 2, 3, 4].map(|k| carry_sprite(k).unwrap()), CARRY_SPRITES);
        assert_eq!(carry_sprite(5), None);
    }

    /// The effect rows are those of the 2.22 table: the bush cut plays row 0
    /// (7 frames of 4 pieces), the second cuttable row 1 (4 frames of 3).
    #[test]
    fn cut_entries_play_their_own_break_row() {
        assert_eq!(cut_leap_kind(0), LEAP_BUSH);
        assert_eq!(cut_leap_kind(1), LEAP_CUT_2);
        assert_eq!(builtin_leap(cut_leap_kind(0)).len(), 7);
        assert_eq!(builtin_leap(cut_leap_kind(1)).len(), 4);
        assert!(builtin_leap(cut_leap_kind(1)).iter().all(|f| f.len() == 3 && f.iter().all(|p| p.0 == 24)));
        assert_eq!(builtin_sprite_rect(24), Some((34, 8, 10, 8)));
    }

    #[test]
    fn bombs_stay_and_everything_else_breaks() {
        for s in STAY_SPRITES {
            assert!(stays_on_ground(Some(s)));
        }
        for s in CARRY_SPRITES {
            assert!(!stays_on_ground(Some(s)));
        }
        assert!(!stays_on_ground(None));
    }

    #[test]
    fn packed_table_entries_unpack_to_the_documented_rects() {
        // x = 48 / 2 = 24 -> 0x0c, y = 82 / 2 = 41, w = 8, h = 8
        let v = (12u32 << 24) | (41 << 12) | (8 << 6) | 8;
        assert_eq!(unpack_sprite_rect(v), (24, 82, 16, 16));
        assert_eq!(builtin_sprite_rect(96), Some((24, 82, 16, 16)));
        let p = (96u32 << 16) | (0xff << 8) | 0x09;
        assert_eq!(unpack_piece(p), (96, -1, 9));
    }

    #[test]
    fn break_effects_have_the_documented_frame_counts() {
        assert_eq!(builtin_leap(LEAP_BUSH).len(), 7);
        assert_eq!(builtin_leap(LEAP_ROCK).len(), 4);
        assert_eq!(builtin_leap(LEAP_POT).len(), 4);
        assert_eq!(builtin_leap(LEAP_SPLASH).len(), 8);
        assert!(builtin_leap(LEAP_ROCK).iter().all(|f| f.len() == 4));
        assert!(builtin_leap(9).is_empty());
    }
}
