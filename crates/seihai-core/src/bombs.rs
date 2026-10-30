//! Bombs: the built-in bomb weapons, `putbomb`, the fuse, and the cross of
//! explosion pieces a bomb leaves.
//!
//! Rules from the 2.05 rev 2 client (2.00 has the same weapon scripts):
//!
//! * The built-in Bomb, Superbomb and Joltbomb weapons are GS1 scripts:
//!   on `weaponfired` with `playerbombs > 0` they `putbomb` power 1, 2 or 3
//!   in front of the player and take one bomb.
//! * `putbomb power,x,y` from a script starts a 60-tick fuse (50 ms ticks).
//!   With water under `(x + 1, y + 1)` there is no bomb, only the splash.
//!   Either way `PLI_BOMBADD` (4) goes out: `x*2`, `y*2`, power (plus 8
//!   when a weapon put it), and the fuse minus 5 ticks. The lab capture of
//!   a real 2.05 bomb reads `x*2, y*2, 1+8, 55`.
//! * In its last 30 ticks a bomb of power 1 to 3 blinks between its two
//!   sprites. When the fuse runs out it plays `bomb.wav` (`thunder.wav`
//!   for power 3) at `(x + 1, y + 1)` and leaves a cross of pieces: the
//!   centre, then every 2 tiles up, left, down and right, the last one of
//!   each arm an end piece. Power 2 is four crosses of radius 2 at the
//!   diagonals plus one of radius 4 in the middle; the others one of
//!   radius 2.
//! * A piece lives 11 ticks and shows its 4 frames, 3 ticks each. While it
//!   lives, a player whose `(x + 1.5, y + 2)` is within 1 tile of the
//!   piece's `(x + 1, y + 1)` on both axes loses half a heart (a whole
//!   heart for power 3), knocked along the piece's arm.
//!
//! * Other players' bombs and explosions come as `PLO_BOMBADD` (11) and
//!   `PLO_EXPLOSION` (36) after the sender's id ([`parse_bomb_add`],
//!   [`parse_explosion`]). The bomb goes through `putbomb` without a send
//!   and burns the fuse it carries; the explosion is one cross of its reach
//!   (at most 15) and type, power `& 7` read as 1 to 3. With bit 8 set the
//!   sender owns the bomb or explosion, and so every piece of it.
//! * An owned piece counts as a player's only while the owner is still in
//!   the player list ([`player_owner`]). A player's piece does not hurt on
//!   a level that said `noplayerkilling`, and when its hit leaves 0 hearts
//!   the owner is the killer. A piece of nobody's, or of a player who left,
//!   always hurts.
//!
//! Not done yet: pieces blowing up bushes and other breakable tiles,
//! chaining into other bombs, the `exploded` NPC event, and hitting
//! baddies.

/// Ticks a scripted `putbomb` burns.
pub const FUSE_TICKS: i32 = 60;
/// The fuse goes out this many ticks shorter (the others start later).
pub const SEND_LEAD: i32 = 5;
/// A piece's lifetime in ticks.
pub const PIECE_TICKS: i32 = 11;
/// The bomb sprite when the power has no pair of its own.
pub const DEFAULT_SPRITE: u16 = 65;
/// The explosion piece in the middle of a cross.
pub const CENTRE: usize = 4;
/// At most this many pieces at once.
pub const MAX_PIECES: usize = 0x400;

/// The two sprites of bombs of power 1, 2 and 3: the first is the normal
/// one, the pair blinks at the end of the fuse.
const BOMB_SPRITES: [[u16; 2]; 3] = [[65, 66], [321, 66], [347, 348]];

/// The four frames of each piece kind.
const PIECE_SPRITES: [[u16; 4]; 9] = [
    [278, 279, 280, 281],
    [282, 283, 284, 285],
    [298, 302, 306, 310],
    [299, 303, 307, 311],
    [286, 287, 288, 289],
    [290, 291, 292, 293],
    [294, 295, 296, 297],
    [300, 304, 308, 312],
    [301, 305, 309, 313],
];

/// The direction each piece kind knocks a player: 0 up, 1 left, 2 down,
/// 3 right.
const PIECE_DIR: [u8; 9] = [0, 0, 1, 1, 2, 2, 2, 3, 3];

/// The four arms of a cross: the step in tiles, then the middle and the
/// end piece kinds.
const ARMS: [((f32, f32), usize, usize); 4] = [((0.0, -2.0), 1, 0), ((-2.0, 0.0), 3, 2), ((0.0, 2.0), 5, 6), ((2.0, 0.0), 7, 8)];

/// The built-in weapon scripts for the bomb ids of `PLO_DEFAULTWEAPON`: 8
/// Bomb (power 1), 12 Superbomb (2), 23 Joltbomb (3).
pub fn builtin_script(id: u8) -> Option<String> {
    let power = match id {
        8 => 1,
        12 => 2,
        23 => 3,
        _ => return None,
    };
    Some(format!(
        "if (weaponfired && playerbombs>0) {{\n\
         \x20 if (playerdir=0) putbomb {power},playerx+0.5,playery-1;\n\
         \x20 if (playerdir=1) putbomb {power},playerx-1.5,playery+1;\n\
         \x20 if (playerdir=2) putbomb {power},playerx+0.5,playery+2.5;\n\
         \x20 if (playerdir=3) putbomb {power},playerx+2.5,playery+1;\n\
         \x20 playerbombs--;\n\
         }}\n"
    ))
}

/// A bomb on the ground.
#[derive(Debug, Clone, PartialEq)]
pub struct Bomb {
    pub x: f32,
    pub y: f32,
    pub power: u8,
    /// Ticks left on the fuse.
    pub fuse: i32,
    pub sprite: u16,
    /// The player who owns it (a received bomb with bit 8).
    pub owner: Option<u16>,
}

impl Bomb {
    pub fn new(x: f32, y: f32, power: u8, fuse: i32) -> Bomb {
        let sprite = pair(power).map_or(DEFAULT_SPRITE, |p| p[0]);
        Bomb { x, y, power, fuse, sprite, owner: None }
    }

    /// One tick of the fuse. Returns true when the bomb goes off.
    pub fn tick(&mut self) -> bool {
        self.fuse -= 1;
        if self.fuse < 30 {
            if let Some(p) = pair(self.power) {
                self.sprite = p[(self.fuse & 1) as usize];
            }
        }
        self.fuse < 0
    }

    /// The sound of the bang.
    pub fn sound(&self) -> &'static str {
        sound(self.power)
    }

    /// The pieces the bang leaves.
    pub fn pieces(&self) -> Vec<Piece> {
        let mut out = Vec::new();
        if self.power == 2 {
            for (dx, dy) in [(-2.0, -2.0), (2.0, -2.0), (-2.0, 2.0), (2.0, 2.0)] {
                cross(&mut out, self.x + dx, self.y + dy, 2, self.power);
            }
            cross(&mut out, self.x, self.y, 4, self.power);
        } else {
            cross(&mut out, self.x, self.y, 2, self.power);
        }
        for p in &mut out {
            p.owner = self.owner;
        }
        out
    }
}

/// The bang of power `power`: `thunder.wav` for 3, else `bomb.wav`.
pub fn sound(power: u8) -> &'static str {
    if power == 3 {
        "thunder.wav"
    } else {
        "bomb.wav"
    }
}

/// One cross of `radius` (at most 15) and type `power` at `(x, y)`, every
/// piece owned by `owner`: `putexplosion` and `PLO_EXPLOSION`.
pub fn explosion_pieces(radius: i32, x: f32, y: f32, power: u8, owner: Option<u16>) -> Vec<Piece> {
    let mut out = Vec::new();
    cross(&mut out, x, y, radius, power);
    for p in &mut out {
        p.owner = owner;
    }
    out
}

/// The owner a hit names: the piece's owner while that player is still in
/// the player list (`present`), else nobody.
pub fn player_owner(owner: Option<u16>, present: impl Fn(u16) -> bool) -> Option<u16> {
    owner.filter(|&id| present(id))
}

/// `PLO_BOMBADD` (11) after the sender's id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BombAdd {
    pub x: f32,
    pub y: f32,
    pub power: u8,
    pub fuse: i32,
    /// Bit 8 of the power byte: the bomb is the sender's.
    pub owned: bool,
}

/// `x*2`, `y*2`, power (`& 7`, bit 8 owned), fuse in ticks; 4 bytes or more.
pub fn parse_bomb_add(b: &[u8]) -> Option<BombAdd> {
    if b.len() < 4 {
        return None;
    }
    let v = |i: usize| b[i].wrapping_sub(0x20);
    Some(BombAdd { x: f32::from(v(0)) / 2.0, y: f32::from(v(1)) / 2.0, power: v(2) & 7, fuse: i32::from(v(3)), owned: v(2) & 8 != 0 })
}

/// `PLO_BOMBDEL` (12), no player id: `x*2`, `y*2`.
pub fn parse_bomb_del(b: &[u8]) -> Option<(f32, f32)> {
    if b.len() < 2 {
        return None;
    }
    Some((f32::from(b[0].wrapping_sub(0x20)) / 2.0, f32::from(b[1].wrapping_sub(0x20)) / 2.0))
}

/// `PLO_EXPLOSION` (36) after the sender's id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Explosion {
    pub radius: i32,
    pub x: f32,
    pub y: f32,
    /// 1 to 3.
    pub power: u8,
    /// Bit 8 of the power byte: the explosion is the sender's.
    pub owned: bool,
}

/// Reach, `x*2`, `y*2`, power (`& 7`, 0 read as 1 and above 3 as 3, bit 8
/// owned); 4 bytes or more.
pub fn parse_explosion(b: &[u8]) -> Option<Explosion> {
    if b.len() < 4 {
        return None;
    }
    let v = |i: usize| b[i].wrapping_sub(0x20);
    let power = (v(3) & 7).clamp(1, 3);
    Some(Explosion { radius: i32::from(v(0)), x: f32::from(v(1)) / 2.0, y: f32::from(v(2)) / 2.0, power, owned: v(3) & 8 != 0 })
}

fn pair(power: u8) -> Option<[u16; 2]> {
    BOMB_SPRITES.get(usize::from(power).checked_sub(1)?).copied()
}

/// A cross of `radius` arm pieces around `(x, y)`.
fn cross(out: &mut Vec<Piece>, x: f32, y: f32, radius: i32, power: u8) {
    let radius = radius.min(15);
    out.push(Piece::new(CENTRE, x, y, power));
    for ((sx, sy), mid, end) in ARMS {
        for i in 1..radius {
            out.push(Piece::new(mid, x + sx * i as f32, y + sy * i as f32, power));
        }
        out.push(Piece::new(end, x + sx * radius as f32, y + sy * radius as f32, power));
    }
}

/// One piece of an explosion.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub kind: usize,
    pub x: f32,
    pub y: f32,
    pub power: u8,
    pub life: i32,
    pub sprite: u16,
    /// The player who owns it (see [`player_owner`]).
    pub owner: Option<u16>,
}

impl Piece {
    pub fn new(kind: usize, x: f32, y: f32, power: u8) -> Piece {
        Piece { kind, x, y, power, life: PIECE_TICKS, sprite: PIECE_SPRITES[kind][0], owner: None }
    }

    /// Whether the player at `(px, py)` stands in this piece.
    pub fn hits_player(&self, px: f32, py: f32) -> bool {
        ((self.x + 1.0) - (px + 1.5)).abs() < 1.0 && ((self.y + 1.0) - (py + 2.0)).abs() < 1.0
    }

    /// Half hearts a hit takes.
    pub fn damage_half(&self) -> i32 {
        if self.power == 3 {
            2
        } else {
            1
        }
    }

    /// The knock direction (0 up, 1 left, 2 down, 3 right).
    pub fn knock_dir(&self) -> u8 {
        PIECE_DIR[self.kind]
    }

    /// One tick after the hit test. Returns false when the piece is gone.
    pub fn tick(&mut self) -> bool {
        self.life -= 1;
        if self.life < 0 {
            return false;
        }
        self.sprite = PIECE_SPRITES[self.kind][(3 - self.life / 3) as usize];
        true
    }
}

/// `PLI_BOMBADD` (4): `GCHAR x*2`, `GCHAR y*2`, `GCHAR power` (plus 8 for
/// a weapon's bomb), `GCHAR fuse`, then the newline.
pub fn bomb_add_message(x: f32, y: f32, power: u8, from_weapon: bool, fuse: i32) -> Vec<u8> {
    let flag = if from_weapon { 8 } else { 0 };
    let g = |v: i32| (v.clamp(0, 223) as u8) + 0x20;
    vec![4 + 0x20, g((x * 2.0).round() as i32), g((y * 2.0).round() as i32), g(i32::from(power) + flag), g(fuse), b'\n']
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_matches_the_real_205_bomb() {
        // lab run 4: a 2.05 Bomb at 39,14 sent `6e 3c 29 57`
        assert_eq!(bomb_add_message(39.0, 14.0, 1, true, FUSE_TICKS - SEND_LEAD), vec![0x24, 0x6e, 0x3c, 0x29, 0x57, b'\n']);
    }

    #[test]
    fn the_fuse_blinks_then_goes_off() {
        let mut b = Bomb::new(10.0, 10.0, 1, FUSE_TICKS);
        assert_eq!(b.sprite, 65);
        for _ in 0..30 {
            assert!(!b.tick());
        }
        assert_eq!(b.fuse, 30);
        assert_eq!(b.sprite, 65);
        assert!(!b.tick());
        assert_eq!(b.sprite, 66);
        assert!(!b.tick());
        assert_eq!(b.sprite, 65);
        let mut n = 0;
        while !b.tick() {
            n += 1;
        }
        assert_eq!(n, 28);
    }

    #[test]
    fn a_power_1_bang_is_a_cross_of_radius_2() {
        let p = Bomb::new(10.0, 10.0, 1, 0).pieces();
        assert_eq!(p.len(), 9);
        assert_eq!((p[0].kind, p[0].x, p[0].y), (CENTRE, 10.0, 10.0));
        // up: a middle piece 2 tiles out, the end 4 tiles out
        assert_eq!((p[1].kind, p[1].x, p[1].y), (1, 10.0, 8.0));
        assert_eq!((p[2].kind, p[2].x, p[2].y), (0, 10.0, 6.0));
        assert_eq!((p[8].kind, p[8].x, p[8].y), (8, 14.0, 10.0));
        assert_eq!(Bomb::new(10.0, 10.0, 2, 0).pieces().len(), 4 * 9 + 17);
    }

    #[test]
    fn a_piece_shows_four_frames_and_hurts_the_player_inside() {
        let mut p = Piece::new(CENTRE, 10.0, 10.0, 1);
        let mut frames = Vec::new();
        while p.tick() {
            frames.push(p.sprite);
        }
        assert_eq!(frames, vec![286, 286, 287, 287, 287, 288, 288, 288, 289, 289, 289]);
        assert!(p.hits_player(9.5, 9.0));
        assert!(!p.hits_player(8.5, 9.0));
        assert_eq!(p.knock_dir(), 2);
    }

    #[test]
    fn only_the_bomb_ids_have_a_script() {
        assert!(builtin_script(8).unwrap().contains("putbomb 1,"));
        assert!(builtin_script(23).unwrap().contains("putbomb 3,"));
        assert!(builtin_script(7).is_none());
    }

    #[test]
    fn a_received_bomb_reads_power_owner_and_fuse() {
        // the lab bomb as the server relays it: `6e 3c 29 57` after the id
        let b = parse_bomb_add(&[0x6e, 0x3c, 0x29, 0x57]).unwrap();
        assert_eq!(b, BombAdd { x: 39.0, y: 14.0, power: 1, fuse: 55, owned: true });
        assert!(!parse_bomb_add(&[0x6e, 0x3c, 0x21, 0x57]).unwrap().owned);
        assert_eq!(parse_bomb_add(&[0x6e, 0x3c, 0x29]), None);
        assert_eq!(parse_bomb_del(&[0x6e, 0x3c]), Some((39.0, 14.0)));
    }

    #[test]
    fn an_explosions_power_is_1_to_3_and_bit_8_is_the_owner() {
        let e = |p: u8| parse_explosion(&[0x22, 0x40, 0x40, 0x20 + p]).unwrap();
        assert_eq!((e(0).power, e(1).power, e(3).power, e(5).power, e(7).power), (1, 1, 3, 3, 3));
        assert_eq!((e(2).radius, e(2).x, e(2).y, e(2).owned), (2, 16.0, 16.0, false));
        assert!(e(8 + 2).owned);
        assert_eq!(e(8 + 2).power, 2);
        assert_eq!(parse_explosion(&[0x22, 0x40, 0x40]), None);
    }

    #[test]
    fn an_explosion_is_one_cross_even_for_power_2() {
        let p = explosion_pieces(2, 10.0, 10.0, 2, Some(7));
        assert_eq!(p.len(), 9);
        assert!(p.iter().all(|p| p.owner == Some(7) && p.power == 2));
        assert_eq!(explosion_pieces(40, 10.0, 10.0, 1, None).len(), 1 + 4 * 15);
    }

    #[test]
    fn a_bombs_pieces_carry_its_owner() {
        let mut b = Bomb::new(10.0, 10.0, 2, 0);
        b.owner = Some(3);
        assert!(b.pieces().iter().all(|p| p.owner == Some(3)));
        assert!(Bomb::new(10.0, 10.0, 1, 0).pieces().iter().all(|p| p.owner.is_none()));
    }

    #[test]
    fn an_owner_who_left_counts_as_nobody() {
        assert_eq!(player_owner(Some(3), |id| id == 3), Some(3));
        assert_eq!(player_owner(Some(3), |_| false), None);
        assert_eq!(player_owner(None, |_| true), None);
    }
}
