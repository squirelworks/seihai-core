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
//! Not done yet: the freeze and the `shoot` pose, other players' arrows
//! (`PLO_ARROWADD`), hitting baddies, arrows set alight by an explosion,
//! and the arrow count going to the server.

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
        Arrow { x, y, dx, dy, sprite: ARROW_SPRITE + u16::from(dir & 3), reflect: -1 }
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
}
