//! Arrows: the built-in Bow, `shootarrow`, and the arrow in flight.
//!
//! Rules from the 2.05 rev 2 client (2.00 has the same weapon script):
//!
//! * The built-in Bow is a GS1 script: on `weaponfired` with
//!   `playerdarts > 0` it freezes the player for 0.3 s, shows the `shoot`
//!   pose and sets a 0.25 s timeout; on the timeout it runs
//!   `shootarrow playerdir` and takes one arrow.
//! * `shootarrow dir` starts sprite `107 + dir` at 1 tile per 50 ms tick
//!   along `dir`, plays `arrow.wav`, and sends `PLI_ARROWADD` (9): `x*2`,
//!   `y*2`, `dir` (plus 8 when a weapon shot it), the sprite and the power
//!   plus 1 (an arrow is power 0).
//! * An arrow is not stopped by walls. It ends when it leaves the level on
//!   its side. On an NPC it fires `wasshot` there, turns round and flies
//!   back for 3 more ticks, then ends. A weapon's own arrow never hits the
//!   player who shot it.
//!
//! Where the arrow starts is inferred, not lab-checked: from the shooter's
//! `(x, y)` it moves back by `(0.3125, 0.9375)` facing up or down and
//! `(0.9375, 0.3125)` facing left or right, then by half the shooter's
//! width (up, down), half its height (left, right), its height (down) or its
//! width (right). For a weapon the shooter is the player, taken here as 2 x
//! 2 tiles.
//!
//! Other players' shots come as `PLO_ARROWADD` (19) after the sender's id
//! ([`parse_arrow_add`]): `x*2`, `y*2`, flags (`& 3` the direction, 8 the
//! sender's), the sprite, and the type (0 arrow, 1 fireball, 2 fireblast,
//! 3 nuke) as `v + 33`. On a level that said `noplayerkilling` such a shot
//! is a player's shot and never hits this player. Otherwise, each tick
//! after it moves, it hits when its sprite's box (array 24: `w / 32` by
//! `h / 32` tiles of the source rectangle) overlaps `(x + 0.5 .. x + 2.5,
//! y + 1 .. y + 3)` of the player ([`Arrow::hits_player`]). It takes half
//! a heart, a whole one for types 1 and 3; the same owner rule as a bomb's
//! piece applies ([`crate::bombs::player_owner`]). The shot is then gone,
//! and types 2 and 3 leave a blast of reach 2 and type 1, owned by the
//! shooter, around the shot's middle less one tile.
//!
//! Not done yet: the freeze and the `shoot` pose, hitting baddies, arrows
//! set alight by an explosion, the arrow count going to the server, a
//! shield stopping or sending back a shot, and flag 4 of `PLO_ARROWADD`
//! (it removes a shot flying the other way on the same line; the line test
//! is not legible in the decompile). Which way a shot knocks the player is
//! not legible either; Seihai pushes along the flight (inferred).

/// The first arrow sprite; `+ dir` for the others.
pub const ARROW_SPRITE: u16 = 107;
/// Ticks an arrow flies back after it hits an NPC.
pub const REFLECT_TICKS: i32 = 3;
/// The player's size in tiles, as the shooter of a weapon's arrow.
pub const SHOOTER_W: f32 = 2.0;
pub const SHOOTER_H: f32 = 2.0;

/// The built-in Bow's weapon script (`PLO_DEFAULTWEAPON` id 7).
pub fn builtin_script(id: u8) -> Option<&'static str> {
    (id == 7).then_some(
        "if (weaponfired && playerdarts>0) {\n  freezeplayer 0.3;\n  setani shoot,wbow1.gif;\n  timeout = 0.25;\n}\nif (timeout) {\n  shootarrow playerdir;\n  playerdarts--;\n}\n",
    )
}

/// Where an arrow starts when a shooter `w` x `h` tiles at `(x, y)` shoots
/// along `dir` (inferred, see the module notes).
pub fn start(x: f32, y: f32, w: f32, h: f32, dir: u8) -> (f32, f32) {
    match dir & 3 {
        0 => (x - 0.3125 + w / 2.0, y - 0.9375),
        1 => (x - 0.9375, y - 0.3125 + h / 2.0),
        2 => (x - 0.3125 + w / 2.0, y - 0.9375 + h),
        _ => (x - 0.9375 + w, y - 0.3125 + h / 2.0),
    }
}

/// An arrow in flight.
#[derive(Debug, Clone, PartialEq)]
pub struct Arrow {
    pub x: f32,
    pub y: f32,
    pub dx: f32,
    pub dy: f32,
    pub sprite: u16,
    /// Ticks left flying back after a hit; `-1` before one.
    pub reflect: i32,
    /// 0 arrow, 1 fireball, 2 fireblast, 3 nuke.
    pub kind: u8,
    /// A player's shot: never hits this player (a weapon's own arrow, or
    /// another player's shot on a `noplayerkilling` level).
    pub harmless: bool,
    /// The player who shot it (a received shot with flag 8).
    pub owner: Option<u16>,
}

/// What a tick did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Flying,
    Gone,
}

impl Arrow {
    pub fn new(x: f32, y: f32, dir: u8) -> Arrow {
        let (dx, dy) = [(0.0, -1.0), (-1.0, 0.0), (0.0, 1.0), (1.0, 0.0)][usize::from(dir & 3)];
        Arrow { x, y, dx, dy, sprite: ARROW_SPRITE + u16::from(dir & 3), reflect: -1, kind: 0, harmless: false, owner: None }
    }

    /// The way it flies: 0 up, 1 left, 2 down, 3 right.
    pub fn dir(&self) -> u8 {
        if self.dy.abs() <= self.dx.abs() {
            if self.dx >= 0.0 {
                3
            } else {
                1
            }
        } else if self.dy >= 0.0 {
            2
        } else {
            0
        }
    }

    /// Move one tick. Leaving the level on its side, or the last tick of
    /// the flight back, ends it.
    pub fn step(&mut self) -> Step {
        let dir = self.dir();
        self.x += self.dx;
        self.y += self.dy;
        let out = match dir {
            0 => self.y < -2.0,
            1 => self.x < -2.0,
            2 => self.y >= 64.0,
            _ => self.x >= 64.0,
        };
        if out {
            return Step::Gone;
        }
        if self.reflect >= 0 {
            self.reflect -= 1;
            if self.reflect < 0 {
                return Step::Gone;
            }
        }
        Step::Flying
    }

    /// A received shot: its sprite, type and owner as sent.
    pub fn received(a: &ArrowAdd, sender: Option<u16>, no_pk: bool) -> Arrow {
        let mut arrow = Arrow::new(a.x, a.y, a.dir);
        arrow.sprite = a.sprite;
        arrow.kind = a.kind;
        arrow.harmless = no_pk;
        arrow.owner = sender.filter(|_| a.owned);
        arrow
    }

    /// Whether its box, `w` x `h` pixels of the sprite, overlaps the player
    /// at `(px, py)`. Not while it flies back, nor for a harmless shot.
    pub fn hits_player(&self, w: i32, h: i32, px: f32, py: f32) -> bool {
        if self.reflect >= 0 || self.harmless {
            return false;
        }
        let (w, h) = (w as f32 / 32.0, h as f32 / 32.0);
        self.x + w > px + 0.5 && self.x < px + 2.5 && self.y + h > py + 1.0 && self.y < py + 3.0
    }

    /// Half hearts a hit takes: 2 for a fireball or nuke, else 1.
    pub fn damage_half(&self) -> i32 {
        if matches!(self.kind, 1 | 3) {
            2
        } else {
            1
        }
    }

    /// Where a fireblast or nuke that hit leaves its blast (reach 2, type
    /// 1): the middle of its `w` x `h` pixel box less one tile. `None` for
    /// an arrow or a fireball.
    pub fn blast_at(&self, w: i32, h: i32) -> Option<(f32, f32)> {
        (self.kind >= 2).then(|| (self.x + w as f32 / 64.0 - 1.0, self.y + h as f32 / 64.0 - 1.0))
    }

    /// It hit an NPC: back the way it came for [`REFLECT_TICKS`].
    pub fn bounce(&mut self) {
        self.reflect = REFLECT_TICKS;
        self.dx = -self.dx;
        self.dy = -self.dy;
    }
}

/// `PLI_ARROWADD` (9): `GCHAR x*2`, `GCHAR y*2`, `GCHAR dir` (plus 8 for a
/// weapon's arrow), `GCHAR sprite`, `GCHAR power + 1`, then the newline.
pub fn arrow_add_message(x: f32, y: f32, dir: u8, from_weapon: bool, sprite: u16, power: u8) -> Vec<u8> {
    let g = |v: i32| (v.clamp(0, 223) as u8) + 0x20;
    let flags = i32::from(dir & 3) + if from_weapon { 8 } else { 0 };
    vec![9 + 0x20, g((x * 2.0).round() as i32), g((y * 2.0).round() as i32), g(flags), g(i32::from(sprite)), g(i32::from(power) + 1), b'\n']
}

/// `PLO_ARROWADD` (19) after the sender's id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrowAdd {
    pub x: f32,
    pub y: f32,
    pub dir: u8,
    /// Flag 4: remove a shot flying the other way first (not done).
    pub replaces: bool,
    /// Flag 8: the shot is the sender's.
    pub owned: bool,
    pub sprite: u16,
    pub kind: u8,
}

/// `x*2`, `y*2`, flags, sprite, type + 33; 5 bytes or more.
pub fn parse_arrow_add(b: &[u8]) -> Option<ArrowAdd> {
    if b.len() < 5 {
        return None;
    }
    let v = |i: usize| b[i].wrapping_sub(0x20);
    let flags = v(2);
    Some(ArrowAdd {
        x: f32::from(v(0)) / 2.0,
        y: f32::from(v(1)) / 2.0,
        dir: flags & 3,
        replaces: flags & 4 != 0,
        owned: flags & 8 != 0,
        sprite: u16::from(v(3)),
        kind: b[4].wrapping_sub(0x21),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_arrow_flies_a_tile_a_tick_until_it_leaves_the_level() {
        let mut a = Arrow::new(10.0, 3.0, 0);
        assert_eq!(a.sprite, 107);
        let mut n = 0;
        while a.step() == Step::Flying {
            n += 1;
        }
        // 3 -> -2 is still in; -3 is out
        assert_eq!(n, 5);
        assert_eq!(Arrow::new(10.0, 3.0, 3).sprite, 110);
    }

    #[test]
    fn a_hit_turns_it_round_for_three_ticks() {
        let mut a = Arrow::new(10.0, 10.0, 3);
        a.bounce();
        assert_eq!(a.dir(), 1);
        assert_eq!(a.step(), Step::Flying);
        assert_eq!(a.step(), Step::Flying);
        assert_eq!(a.step(), Step::Flying);
        assert_eq!(a.step(), Step::Gone);
        assert_eq!(a.x, 6.0);
    }

    #[test]
    fn the_message_layout() {
        assert_eq!(arrow_add_message(30.5, 29.0, 2, true, 109, 0), vec![9 + 0x20, 61 + 0x20, 58 + 0x20, 10 + 0x20, 109 + 0x20, 0x21, b'\n']);
    }

    #[test]
    fn the_start_is_in_front_of_the_player() {
        let at = |dir| start(30.0, 30.0, SHOOTER_W, SHOOTER_H, dir);
        assert_eq!(at(0), (30.6875, 29.0625));
        assert_eq!(at(2), (30.6875, 31.0625));
        assert_eq!(at(3), (31.0625, 30.6875));
    }

    #[test]
    fn a_received_shot_reads_like_our_own_message() {
        let m = arrow_add_message(30.5, 29.0, 2, true, 109, 0);
        let a = parse_arrow_add(&m[1..6]).unwrap();
        assert_eq!(a, ArrowAdd { x: 30.5, y: 29.0, dir: 2, replaces: false, owned: true, sprite: 109, kind: 0 });
        assert_eq!(parse_arrow_add(&m[1..5]), None);
        let fire = parse_arrow_add(&[0x60, 0x60, 0x20 + 4 + 3, 0x20 + 110, 0x21 + 1]).unwrap();
        assert_eq!((fire.dir, fire.replaces, fire.owned, fire.kind), (3, true, false, 1));
    }

    #[test]
    fn the_shot_box_overlaps_the_players_body() {
        // a 16 x 16 pixel sprite is half a tile each way
        let at = |x, y| Arrow { x, y, ..Arrow::new(0.0, 0.0, 3) };
        let (px, py) = (30.0, 30.0);
        assert!(at(30.5, 31.0).hits_player(16, 16, px, py));
        assert!(!at(30.0, 31.0).hits_player(16, 16, px, py), "x + 0.5 == px + 0.5 is out");
        assert!(at(32.4, 32.9).hits_player(16, 16, px, py));
        assert!(!at(32.5, 31.0).hits_player(16, 16, px, py), "x == px + 2.5 is out");
        assert!(!at(31.0, 33.0).hits_player(16, 16, px, py), "y == py + 3 is out");
        assert!(!at(31.0, 30.5).hits_player(16, 16, px, py), "y + h == py + 1 is out");
    }

    #[test]
    fn harmless_and_returning_shots_do_not_hit() {
        let mut a = Arrow { x: 31.0, y: 31.0, ..Arrow::new(0.0, 0.0, 3) };
        a.harmless = true;
        assert!(!a.hits_player(16, 16, 30.0, 30.0));
        a.harmless = false;
        a.bounce();
        assert!(!a.hits_player(16, 16, 30.0, 30.0));
    }

    #[test]
    fn the_owner_is_kept_only_with_flag_8_and_no_pk_makes_it_harmless() {
        let add = parse_arrow_add(&[0x60, 0x60, 0x20 + 8, 0x20 + 107, 0x21]).unwrap();
        assert_eq!(Arrow::received(&add, Some(4), false).owner, Some(4));
        assert!(Arrow::received(&add, Some(4), true).harmless);
        let unowned = ArrowAdd { owned: false, ..add };
        assert_eq!(Arrow::received(&unowned, Some(4), false).owner, None);
    }

    #[test]
    fn damage_and_blast_by_type() {
        let k = |kind| Arrow { kind, ..Arrow::new(10.0, 10.0, 0) };
        assert_eq!((k(0).damage_half(), k(1).damage_half(), k(2).damage_half(), k(3).damage_half()), (1, 2, 1, 2));
        assert_eq!(k(0).blast_at(16, 16), None);
        assert_eq!(k(1).blast_at(16, 16), None);
        assert_eq!(k(2).blast_at(16, 16), Some((9.25, 9.25)));
    }
}
