//! Baddies: the classic level enemies (`BADDY` records, `PLO_BADDYPROPS`).
//!
//! Everything here is read from the 2.05 rev 2 client, where a baddy is a
//! `TComputer` (functions `0x4d5e0c` to `0x4d9588`, the tables at
//! `0x54a7a4` to `0x54aa30` and `arrays.dat` arrays 20, 22 and 23). See
//! `documentation/baddies.md` for the read and what is still open.
//!
//! * A baddy is a 2 x 2 tile box at `(x, y)`; its centre is `(x + 1,
//!   y + 1)` (the hare's is raised by its jump). A player's centre is
//!   `(x + 1.5, y + 2)`.
//! * It thinks every second player tick, 100 ms ([`TICK_MS`]). Offline, or
//!   online when this client is the level leader (`PLO_ISLEADER`), it runs
//!   the AI below and, online, sends the props that changed
//!   (`PLI_BADDYPROPS`, 15). Other clients only draw what the server
//!   forwards (`PLO_BADDYPROPS`, 2) and send their sword hits to the leader
//!   (`PLI_BADDYHURT`, 16, arriving as `PLO_BADDYHURT`, 27).
//! * Modes: 0 walk, 1 look round, 2 hunt, 3 hurt, 4 bumped (sword met
//!   sword), 5 dying, 6 swamp soldier, 7 hare, 8 octopus shooting, 9 dead.
//! * Prop 6 on the wire is the mode's tick counter, not the animation
//!   frame, and prop 7 is `dir * 4 + head`, the body and the head facing
//!   (the 2.22 capture: a walking baddy sends 43 then 42, the hare 2 then 3,
//!   100 ms apart).

use crate::items::DelphiRandom;

/// One baddy tick.
pub const TICK_MS: u64 = 100;
/// The player tick the sword test runs on.
pub const PLAYER_TICK_MS: u64 = 50;
/// Baddy types 0 to 9.
pub const KINDS: usize = 10;
/// The default picture of each type, without the extension (the 2.05
/// client names `.gif`, GServer sends `.png` to 2.1 and later).
pub const IMAGES: [&str; KINDS] =
    ["baddygray", "baddyblue", "baddyred", "baddyblue", "baddygray", "baddyhare", "baddyoctopus", "baddygold", "baddylizardon", "baddydragon"];
/// Starting power of each type (`0x5482cc`), in sword-power units.
pub const POWER: [i32; KINDS] = [2, 3, 4, 3, 2, 1, 1, 6, 12, 8];
/// The archer's and the swamp soldier's bow.
pub const BOW_IMAGE: &str = "wbow1";

pub mod mode {
    pub const WALK: u8 = 0;
    pub const LOOK: u8 = 1;
    pub const HUNT: u8 = 2;
    pub const HURT: u8 = 3;
    pub const BUMPED: u8 = 4;
    pub const DIE: u8 = 5;
    pub const SWAMPSHOT: u8 = 6;
    pub const HAREJUMP: u8 = 7;
    pub const OCTOSHOT: u8 = 8;
    pub const DEAD: u8 = 9;
}

/// One step along each facing (`0x54a924`): up, left, down, right.
const STEP: [(f64, f64); 4] = [(0.0, -0.25), (-0.25, 0.0), (0.0, 0.25), (0.25, 0.0)];
/// The three points ahead of the box that must be free to move (`0x54a7a4`).
const AHEAD: [[(f64, f64); 3]; 4] = [
    [(0.1, -0.5), (1.0, -0.5), (1.9, -0.5)],
    [(-0.5, 0.1), (-0.5, 1.0), (-0.5, 1.9)],
    [(0.1, 2.5), (1.0, 2.5), (1.9, 2.5)],
    [(2.5, 0.1), (2.5, 1.0), (2.5, 1.9)],
];
/// Head turns while looking round, by `counter / 4` (`0x54a964`).
const LOOK_TURN: [i32; 12] = [1, 1, 0, -1, -1, 0, 1, 1, 0, -1, -1, 0];
/// Where the octopus's rock starts, in eighths of a tile (`0x54a990`).
const ROCK_FROM: [(f64, f64); 4] = [(4.0, -3.0), (-8.0, 5.0), (4.0, 12.0), (17.0, 5.0)];
/// Where an arrow starts, in eighths of a tile (`0x54a9f0`).
const ARROW_FROM: [(f64, f64); 4] = [(12.0, -12.0), (-10.0, 0.0), (4.0, 3.0), (11.0, 0.0)];
/// Where a shooter wants to stand, from the target (`0x54a9d0`,
/// `0x54a9e0`). Facing right it is 11 to the target's right, as in the
/// program.
const AIM_X: [f64; 4] = [-1.0, 11.0, 0.0, 11.0];
const AIM_Y: [f64; 4] = [11.0, 1.0, -11.0, 1.0];
/// The hare's height over its 21 jump ticks (`0x547da8`).
const HARE_HEIGHT: [f64; 11] = [0.0, 1.2, 2.4, 3.6, 4.5, 5.4, 6.2, 6.9, 7.4, 7.8, 8.0];
/// Ticks a verse stays over the baddy.
const VERSE_TICKS: i32 = 20;
/// The first arrow sprite; `+ dir` for the others.
pub const ARROW_SPRITE: u16 = 107;
/// The octopus's rock.
pub const ROCK_SPRITE: u16 = 131;

/// Who a baddy is after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// This client's player.
    Local,
    /// Another player in the level, by id.
    Other(u16),
}

/// A shot a baddy fired: an arrow (archer, swamp soldier), a fire shot
/// (dragon, `kind` 2) or the octopus's rock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shot {
    pub sprite: u16,
    pub x: f64,
    pub y: f64,
    pub dir: u8,
    pub kind: u8,
}

/// What a baddy needs from the game around it.
pub trait World {
    /// Whether a baddy may not stand on `(x, y)`: a wall, water, a chair or
    /// a bed (tile types 11, 3 and 5), or off the board.
    fn blocked(&self, x: f64, y: f64) -> bool;
    /// `Random(n)`.
    fn random(&mut self, n: u32) -> i32;
    /// The local player's `(x, y)`, if a baddy may go for it.
    fn player(&self) -> Option<(f64, f64)>;
    /// The other players in the level a baddy may go for.
    fn others(&self) -> Vec<(u16, f64, f64)>;
    fn sound(&mut self, file: &str, x: f64, y: f64);
    fn shoot(&mut self, shot: Shot);
    /// The baddy touched the local player: `half` half hearts, pushed
    /// along `(dx, dy)`.
    fn touch_player(&mut self, half: i32, dx: f64, dy: f64);
}

/// One baddy.
#[derive(Debug, Clone, PartialEq)]
pub struct Baddy {
    pub id: u8,
    pub kind: u8,
    pub x: f64,
    pub y: f64,
    /// The hare's home: it jumps out 4 tiles from here.
    pub home: (f64, f64),
    pub mode: u8,
    pub dir: u8,
    pub head: u8,
    pub ani: i32,
    pub counter: i32,
    pub power: i32,
    /// The picture; empty for the type's default.
    pub image: String,
    /// The sight, hurt and attack verses.
    pub verses: [String; 3],
    /// The verse said now, and its ticks left (`-1` for none).
    pub text: String,
    pub text_ticks: i32,
    say_sight: bool,
    say_hurt: bool,
    say_attack: bool,
    /// Hurt: the body blinks.
    pub blink: bool,
    /// Just shot: the sword is not drawn, and the next look may give up.
    pub shot_out: bool,
    /// A hit sent to the leader that the leader has not answered yet.
    hurt_sent: bool,
    aux: i32,
    aux2: i32,
    knock: (f64, f64),
    target: Target,
    target_dist: f64,
    /// Gone from the level.
    pub removed: bool,
    /// The props as last sent to the server (the leader's diff).
    pub sent: Option<Vec<Vec<u8>>>,
}

/// The default picture of type `kind` with extension `ext` (`.png`).
pub fn default_image(kind: u8, ext: &str) -> String {
    format!("{}{}", IMAGES[usize::from(kind).min(KINDS - 1)], ext)
}

fn mod4(v: i32) -> u8 {
    v.rem_euclid(4) as u8
}

impl Baddy {
    /// A new baddy (`0x4d5e0c`). `player` is the local player's `(x, y,
    /// dir)`: a baddy within 30 tiles of it (or a blue one anywhere) starts
    /// facing it, else up or down towards it.
    pub fn new(id: u8, kind: u8, x: f64, y: f64, verses: [String; 3], player: (f64, f64, u8), rng: &mut DelphiRandom) -> Baddy {
        let kind = if usize::from(kind) < KINDS { kind } else { 0 };
        let (px, py, pdir) = player;
        let dir = if (x - px).abs() + (y - py).abs() <= 30.0 || kind == 1 {
            mod4(i32::from(pdir) + 2)
        } else if py <= y {
            0
        } else {
            2
        };
        let mut b = Baddy {
            id,
            kind,
            x,
            y,
            home: (x, y),
            mode: mode::WALK,
            dir,
            head: dir,
            ani: 0,
            counter: 0,
            power: POWER[usize::from(kind)],
            image: String::new(),
            verses,
            text: String::new(),
            text_ticks: -1,
            say_sight: true,
            say_hurt: true,
            say_attack: true,
            blink: false,
            shot_out: false,
            hurt_sent: false,
            aux: 0,
            aux2: 0,
            knock: (0.0, 0.0),
            target: Target::Local,
            target_dist: 0.0,
            removed: false,
            sent: None,
        };
        match kind {
            4 => {
                b.mode = mode::SWAMPSHOT;
                b.ani = 2;
            }
            5 => {
                b.mode = mode::HAREJUMP;
                // The program moves right for both 2 and 3 here.
                match rng.next(4) {
                    0 => b.y -= 4.0,
                    1 => b.x -= 4.0,
                    _ => b.x += 4.0,
                }
            }
            6 => {
                b.dir = 3;
                b.head = 3;
                b.ani = 4;
                b.counter = rng.next(10) as i32 + 20;
            }
            _ => b.counter = rng.next(20) as i32 + 40,
        }
        b
    }

    /// A baddy from a level's `BADDY` record (its verses one per line).
    pub fn from_level(id: u8, rec: &crate::assets::Baddy, player: (f64, f64, u8), rng: &mut DelphiRandom) -> Baddy {
        let kind = rec.kind.trim().parse::<u8>().unwrap_or(0);
        let mut lines = rec.verses.split('\n').map(str::to_string);
        let verses = [lines.next().unwrap_or_default(), lines.next().unwrap_or_default(), lines.next().unwrap_or_default()];
        Baddy::new(id, kind, f64::from(rec.x), f64::from(rec.y), verses, player, rng)
    }

    /// A baddy the server announced: type 0 at (0, 0), then its props.
    pub fn from_server(id: u8, player: (f64, f64, u8), rng: &mut DelphiRandom) -> Baddy {
        Baddy::new(id, 0, 0.0, 0.0, Default::default(), player, rng)
    }

    /// The centre (`0x4d6a44`, `0x4d6a60`): the hare's is raised while it
    /// is up.
    pub fn centre(&self) -> (f64, f64) {
        (self.x + 1.0, self.y + 1.0 - self.hare_height())
    }

    /// How high the hare is, in tiles.
    pub fn hare_height(&self) -> f64 {
        if self.mode == mode::HAREJUMP && (17..=37).contains(&self.counter) {
            HARE_HEIGHT[(10 - (self.counter - 27).abs()) as usize]
        } else {
            0.0
        }
    }

    /// Whether a hit can land now (`0x4d6218`): not while hurt, bumped or
    /// dying, the swamp soldier only while it is up and not just hurt or
    /// shooting, the hare only in the air.
    pub fn hittable(&self) -> bool {
        match self.mode {
            mode::HURT | mode::BUMPED | mode::DIE => false,
            mode::SWAMPSHOT => self.aux < 1 && (22..=36).contains(&self.counter),
            mode::HAREJUMP => (17..=37).contains(&self.counter),
            _ => true,
        }
    }

    fn say(&mut self, verse: usize) {
        self.text = self.verses[verse].clone();
        self.text_ticks = VERSE_TICKS;
    }

    fn blocked_ahead(&self, w: &dyn World, dir: u8) -> bool {
        AHEAD[usize::from(dir & 3)].iter().any(|&(dx, dy)| w.blocked(self.x + dx, self.y + dy))
    }

    /// One baddy tick (`0x4d6bd8`). `think` runs the AI: offline, or online
    /// as the level leader. Returns true when the baddy died and is gone.
    pub fn tick(&mut self, w: &mut dyn World, think: bool) -> bool {
        if think {
            self.think(w);
        }
        self.touch(w);
        if self.kind == 3 {
            self.ani = self.ani.rem_euclid(2) + 2;
        }
        if self.kind == 6 && self.ani < 2 {
            self.ani = self.ani.rem_euclid(2) + 4;
        }
        let died = self.mode == mode::DIE && self.counter < 0;
        if died {
            self.mode = mode::DEAD;
            self.die(w);
        }
        if self.text_ticks >= 0 {
            self.text_ticks -= 1;
        }
        died
    }

    fn die(&mut self, w: &mut dyn World) {
        let (cx, cy) = (self.x + 1.0, self.y + 1.0);
        w.sound("compudead.wav", cx, cy);
        self.removed = true;
    }

    /// The baddy touches the local player (`0x4d6ab0`): half a heart, a
    /// whole one from the golden warrior (Graal_analysis
    /// `damage-sources.md`), and its attack verse after its sight verse.
    fn touch(&mut self, w: &mut dyn World) {
        if !self.hittable() {
            return;
        }
        let Some((px, py)) = w.player() else { return };
        let (cx, cy) = self.centre();
        let (pcx, pcy) = (px + 1.5, py + 2.0);
        if (pcx - cx).abs() <= 1.0 && (pcy - cy).abs() <= 1.0 {
            w.touch_player(if self.kind == 7 { 2 } else { 1 }, pcx - cx, pcy - cy);
            if !self.say_sight && self.say_attack {
                self.say_attack = false;
                self.say(2);
            }
        }
    }

    fn think(&mut self, w: &mut dyn World) {
        match self.mode {
            mode::WALK => self.walk(w),
            mode::LOOK => {
                self.counter -= 1;
                if self.counter < 0 {
                    let d = w.random(4) as u8;
                    self.dir = d;
                    self.head = d;
                    self.mode = mode::WALK;
                    self.counter = w.random(20) + 40;
                } else {
                    let turn = LOOK_TURN[(self.counter / 4).clamp(0, 11) as usize];
                    self.head = mod4(turn + i32::from(self.dir) + 4);
                }
                self.look(w);
            }
            mode::HUNT => {
                if self.counter < 6 {
                    self.blink = false;
                    if self.kind == 3 || self.kind == 9 {
                        self.shooter(w);
                    } else {
                        self.chase(w);
                    }
                } else {
                    self.counter -= 1;
                    if !self.blink {
                        self.ani = self.counter.rem_euclid(2);
                    }
                }
            }
            mode::HURT | mode::BUMPED => self.knocked(w),
            mode::DIE => {
                if self.counter >= 0 {
                    self.counter -= 1;
                }
            }
            mode::SWAMPSHOT => self.swamp(w),
            mode::HAREJUMP => {
                self.counter = (self.counter + 1) % 46;
                if self.counter == 0 {
                    self.aux += 1;
                    if self.aux % 2 == 0 {
                        (self.x, self.y) = self.home;
                        match w.random(4) {
                            0 => self.y -= 4.0,
                            1 => self.x -= 4.0,
                            2 => self.y += 4.0,
                            _ => self.x += 4.0,
                        }
                    }
                }
                if self.counter == 16 {
                    w.sound("frog.wav", self.x + 1.0, self.y + 1.0);
                }
            }
            mode::OCTOSHOT => {
                self.counter -= 1;
                match self.counter {
                    0 => {
                        let d = w.random(4) as u8;
                        self.dir = d;
                        self.head = d;
                        self.mode = mode::WALK;
                        self.counter = w.random(10) + 20;
                    }
                    2 => self.ani = 4,
                    6 => {
                        self.ani = 7;
                        let (ox, oy) = ROCK_FROM[usize::from(self.dir)];
                        w.shoot(Shot { sprite: ROCK_SPRITE, x: self.x + ox / 8.0, y: self.y + oy / 8.0, dir: self.dir, kind: 0 });
                    }
                    10 => self.ani = 6,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn walk(&mut self, w: &mut dyn World) {
        if self.head == self.dir {
            self.counter -= 1;
            if self.kind != 6 && self.counter % 5 == 0 {
                w.sound("steps.wav", self.x + 1.0, self.y + 1.0);
            }
            if self.counter < 0 {
                if self.kind == 6 {
                    self.mode = mode::OCTOSHOT;
                    self.counter = 13;
                } else {
                    self.mode = mode::LOOK;
                    self.counter = 44;
                }
            } else {
                self.ani = (self.counter % 4) / 2 + if self.kind == 6 { 4 } else { 0 };
            }
            if self.mode == mode::WALK {
                if !self.blocked_ahead(w, self.dir) {
                    let speed = if self.kind == 6 || self.kind == 8 { 2.0 } else { 1.0 };
                    let (sx, sy) = STEP[usize::from(self.dir)];
                    self.x += sx * speed;
                    self.y += sy * speed;
                } else {
                    self.head = mod4(i32::from(self.dir) + 2);
                    self.dir = if self.kind == 6 { self.head } else { mod4(i32::from(self.dir) + 3) };
                    if self.counter & 1 == 0 {
                        self.counter -= 1;
                    }
                }
            }
        } else if self.counter & 1 == 0 {
            self.dir = self.head;
        } else {
            self.counter -= 1;
        }
        if self.kind != 6 {
            self.look(w);
        }
    }

    /// Where a target stands (its centre), if it is still there.
    fn target_at(&self, w: &dyn World, t: Target) -> Option<(f64, f64)> {
        let (x, y) = match t {
            Target::Local => w.player()?,
            Target::Other(id) => w.others().into_iter().find(|o| o.0 == id).map(|o| (o.1, o.2))?,
        };
        Some((x + 1.5, y + 2.0))
    }

    /// Whether the baddy facing `dir` sees `to` from `from` (`0x4f7b64`):
    /// in the quarter it faces, at most 20 tiles off, with no wall on the
    /// line between. The points the program tests along the line are not
    /// legible; Seihai tests one a tile.
    fn sees(&self, w: &dyn World, dir: u8, from: (f64, f64), to: (f64, f64)) -> bool {
        let (dx, dy) = ((from.0 - to.0).abs(), (from.1 - to.1).abs());
        let ahead = match dir & 3 {
            0 => from.1 > to.1 && dy >= dx,
            1 => from.0 > to.0 && dx >= dy,
            2 => to.1 > from.1 && dy >= dx,
            _ => to.0 > from.0 && dx >= dy,
        };
        if !ahead {
            return false;
        }
        let m = dx.max(dy);
        if m == 0.0 {
            return true;
        }
        if m > 20.0 {
            return false;
        }
        let mut i = 0.0;
        while i < m {
            let t = i / m;
            if w.blocked(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t) {
                return false;
            }
            i += 1.0;
        }
        true
    }

    /// Pick the nearest player (`0x4d8278`) and the way to it. The first
    /// candidate is taken without the sight test, as in the program.
    fn choose_target(&mut self, w: &dyn World) -> u8 {
        let c = (self.x + 1.0, self.y + 1.0);
        let mut best = self.dir;
        self.target = Target::Local;
        self.target_dist = 1000.0;
        let way = |t: (f64, f64)| {
            let (dx, dy) = (t.0 - c.0, t.1 - c.1);
            let d = if dy.abs() <= dx.abs() {
                if dx <= 0.0 { 1 } else { 3 }
            } else if dy <= 0.0 {
                0
            } else {
                2
            };
            (dx.abs().max(dy.abs()), d)
        };
        let mut candidates: Vec<(Target, (f64, f64))> = w.others().into_iter().map(|(id, x, y)| (Target::Other(id), (x + 1.5, y + 2.0))).collect();
        if let Some((x, y)) = w.player() {
            candidates.push((Target::Local, (x + 1.5, y + 2.0)));
        }
        for (t, at) in candidates {
            let (m, d) = way(at);
            if m < self.target_dist && (self.target_dist == 1000.0 || self.sees(w, d, c, at)) {
                self.target = t;
                self.target_dist = m;
                best = d;
            }
        }
        best
    }

    /// Look for a player while walking or looking round (`0x4d73e4`).
    fn look(&mut self, w: &mut dyn World) {
        let d = self.choose_target(w);
        let Some(at) = self.target_at(w, self.target) else { return };
        if self.sees(w, self.head, (self.x + 1.0, self.y + 1.0), at) && self.target_dist < 20.0 {
            self.mode = mode::HUNT;
            self.counter = 16;
            self.dir = d;
            self.head = d;
            self.aux = 0;
            self.aux2 = 0;
            if self.say_sight {
                self.say_sight = false;
                self.say(0);
            }
        }
    }

    /// Snap to the half-tile grid and step out of any wall touched.
    fn unstick(&mut self, w: &dyn World) {
        self.x = (self.x * 2.0 + 0.0001).floor() / 2.0;
        self.y = (self.y * 2.0 + 0.0001).floor() / 2.0;
        for d in 0..4u8 {
            if self.blocked_ahead(w, d) {
                match d {
                    0 => self.y += 0.5,
                    1 => self.x += 0.5,
                    2 => self.y -= 0.5,
                    _ => self.x -= 0.5,
                }
            }
        }
    }

    /// Run at the target (`0x4d7538`).
    fn chase(&mut self, w: &mut dyn World) {
        self.counter = (self.counter + 3).rem_euclid(4);
        self.ani = self.counter / 2;
        if self.counter == 0 {
            w.sound("steps.wav", self.x + 1.0, self.y + 1.0);
        }
        self.choose_target(w);
        let b = (self.x + 1.0, self.y + 1.0);
        let Some(t) = self.target_at(w, self.target) else { return };
        let (dx, dy) = (t.0 - b.0, t.1 - b.1);
        let (ax, ay) = (dx.abs(), dy.abs());
        let seen = self.sees(w, self.dir, b, t);
        if !seen {
            self.aux += 1;
            if self.aux > 40 {
                self.mode = mode::LOOK;
                self.counter = 44;
            }
        } else {
            self.aux = 0;
        }
        if self.mode != mode::HUNT {
            return;
        }
        let (mut major, minor, ratio) = if ay <= ax {
            (if dx <= 0.0 { 1 } else { 3 }, if dy <= 0.0 { 0 } else { 2 }, if ax <= 0.0 { 0.0 } else { ay / ax })
        } else {
            (if dy <= 0.0 { 0 } else { 2 }, if dx <= 0.0 { 1 } else { 3 }, ax / ay)
        };
        let wanted = major;
        if ay < 2.0 && ax < 2.0 && !seen {
            major = self.dir;
        }
        let mut b1 = self.blocked_ahead(w, major);
        let mut b2 = self.blocked_ahead(w, minor);
        if wanted != major && b1 {
            major = wanted;
            b1 = self.blocked_ahead(w, major);
            b2 = self.blocked_ahead(w, minor);
        }
        self.dir = major;
        self.head = major;
        let speed = if self.kind == 8 { 3.0 } else { 2.0 };
        let (mx, my) = STEP[usize::from(major)];
        let (nx, ny) = STEP[usize::from(minor)];
        match (b1, b2) {
            (false, false) => {
                self.x += speed * nx * ratio + speed * mx;
                self.y += speed * ny * ratio + speed * my;
            }
            (true, false) => {
                self.x += speed * nx;
                self.y += speed * ny;
            }
            (false, true) => {
                self.x += speed * mx;
                self.y += speed * my;
            }
            (true, true) => {}
        }
        if b1 {
            self.unstick(w);
        }
    }

    /// The archer and the dragon (`0x4d7bd0`): get in line with the target
    /// 11 tiles off, then shoot.
    fn shooter(&mut self, w: &mut dyn World) {
        if self.kind == 9 {
            self.counter = (self.counter + 3).rem_euclid(4);
            self.ani = self.counter / 2;
        } else {
            self.ani = 2;
        }
        if self.aux >= 1 {
            self.aux -= 1;
            if self.aux == 0 {
                self.counter = 12;
            }
            return;
        }
        let d = self.choose_target(w);
        self.dir = d;
        self.head = d;
        let b = (self.x + 1.0, self.y + 1.0);
        let Some(t) = self.target_at(w, self.target) else { return };
        let aim = (AIM_X[usize::from(d)] + t.0, AIM_Y[usize::from(d)] + t.1);
        if self.shot_out {
            self.shot_out = false;
            if self.sees(w, d, b, t) {
                self.mode = mode::LOOK;
                self.counter = 44;
                return;
            }
        }
        let (ex, ey) = (aim.0 - b.0, aim.1 - b.1);
        let (ax, ay) = (ex.abs(), ey.abs());
        if (ax < 1.0 && ay < 1.0) || self.aux2 > 40 {
            self.shoot(w);
            return;
        }
        self.aux2 += 1;
        w.sound("steps.wav", self.x + 1.0, self.y + 1.0);
        let (major, minor) = if ay <= ax {
            (if ex <= 0.0 { 1 } else { 3 }, if ey <= 0.0 { 0 } else { 2 })
        } else {
            (if ey <= 0.0 { 0 } else { 2 }, if ex <= 0.0 { 1 } else { 3 })
        };
        let b1 = self.blocked_ahead(w, major);
        let b2 = self.blocked_ahead(w, minor);
        let before = (self.x, self.y);
        for (blocked, way) in [(b1, major), (b2, minor)] {
            if !blocked {
                let (sx, sy) = STEP[usize::from(way)];
                if ax >= 1.0 {
                    self.x += sx * 2.0;
                }
                if ay >= 1.0 {
                    self.y += sy * 2.0;
                }
            }
        }
        if b1 {
            self.unstick(w);
        }
        let across = (self.dir == 1 || self.dir == 3) && ay < 1.0 && ax >= 11.0;
        let along = (self.dir == 0 || self.dir == 2) && ax < 1.0 && ay >= 11.0;
        if (self.x, self.y) == before || across || along {
            self.shoot(w);
        }
    }

    /// Fire along the facing (`0x4d81f0`): an arrow, the dragon a fire shot.
    fn shoot(&mut self, w: &mut dyn World) {
        self.aux = 15;
        self.shot_out = true;
        let (ox, oy) = ARROW_FROM[usize::from(self.dir)];
        let kind = if self.kind == 9 { 2 } else { 0 };
        w.shoot(Shot { sprite: ARROW_SPRITE + u16::from(self.dir), x: self.x + ox / 8.0, y: self.y + oy / 8.0, dir: self.dir, kind });
        self.aux2 = 0;
    }

    /// The swamp soldier's 43-tick round (`0x4d6c70` mode 6): up at 17, faces
    /// the target at 22, shoots at 25, down from 33.
    fn swamp(&mut self, w: &mut dyn World) {
        self.counter = (self.counter + 1) % 43;
        match self.counter {
            11 | 33 => {
                self.dir = 2;
                if self.counter == 33 {
                    self.shot_out = false;
                }
            }
            22 => self.dir = self.choose_target(w),
            25 => self.shoot(w),
            _ => {}
        }
        self.head = self.dir;
        if self.aux > 0 {
            self.aux -= 1;
            if self.aux < 1 {
                self.blink = false;
                if self.power < 1 {
                    self.mode = mode::DIE;
                    self.counter = 14;
                }
            }
        }
    }

    /// Thrown back after a hit or a sword clash (`0x4d8594`).
    fn knocked(&mut self, w: &mut dyn World) {
        if self.counter > 2 {
            self.x += self.knock.0;
            self.y += self.knock.1;
            if (0..4u8).any(|d| self.blocked_ahead(w, d)) {
                self.x -= self.knock.0;
                self.y -= self.knock.1;
            }
        }
        self.counter -= 1;
        if self.counter < 1 {
            if self.power < 1 {
                self.mode = mode::DIE;
                self.counter = 14;
            } else {
                if self.mode == mode::HURT && self.say_hurt {
                    self.say_hurt = false;
                    self.say_sight = false;
                    self.say(1);
                }
                self.mode = mode::HUNT;
                self.counter = 12;
                self.head = self.choose_target(w);
                self.aux = 0;
            }
        } else if self.counter < 4 && self.kind == 6 {
            self.mode = mode::DIE;
            self.counter = 9;
        }
    }

    /// A hit (`0x4d6810`): `clash` when the sword met the baddy's sword
    /// (it is bumped, no damage), else `power` is taken and the baddy is
    /// thrown away from `(dx, dy)`, the hitter's centre less its own. A
    /// client that is online but not the leader only takes the power and
    /// shows the hurt mode until the leader's props come back.
    pub fn hurt(&mut self, w: &mut dyn World, clash: bool, power: i32, dx: f64, dy: f64, follower: bool) {
        if follower {
            if !clash {
                self.power -= power;
                self.mode = mode::HURT;
                self.hurt_sent = true;
            }
            return;
        }
        let speed;
        if !clash {
            self.blink = true;
            self.power -= power;
            if self.kind == 4 {
                self.aux = 8;
                return;
            }
            if self.kind == 5 {
                self.mode = mode::DIE;
                self.counter = 9;
                self.y = self.centre().1 - 1.0;
                return;
            }
            self.mode = mode::HURT;
            speed = 2.0;
        } else {
            self.mode = mode::BUMPED;
            speed = 0.8;
            w.sound("swordon.wav", self.x + 1.0, self.y + 1.0);
            self.blink = false;
        }
        self.counter = 8;
        let (vx, vy) = (-dx * 2.0, -dy * 2.0);
        let len = (vx * vx + vy * vy).sqrt();
        self.knock = if len <= 0.0 { (-speed, vy) } else { (vx * speed / len, vy * speed / len) };
    }

    /// The local player's sword against this baddy (`0x4d6268`), during
    /// swing ticks 1 to 3. `None` if it misses, else whether it was a clash.
    /// A soldier (types 0, 1, 2, 7) facing the player blocks with its own
    /// sword from 2 to 5 tiles ahead.
    pub fn sword_test(&self, px: f64, py: f64, pdir: u8) -> Option<bool> {
        if !self.hittable() {
            return None;
        }
        let (cx, cy) = self.centre();
        let (pcx, pcy) = (px + 1.5, py + 2.0);
        let (adx, ady) = ((pcx - cx).abs(), (pcy - cy).abs());
        let soldier = matches!(self.kind, 0 | 1 | 2 | 7);
        let clash = soldier
            && mod4(i32::from(pdir) + 2) == self.dir
            && match pdir & 3 {
                0 => !(cy + 5.0 < pcy || pcy <= cy + 2.0 || 2.0 < adx || cx + 0.5 <= pcx),
                1 => !(cx + 5.0 < pcx || pcx <= cx + 2.0 || 2.0 < ady || pcy <= cy - 0.5),
                2 => !(pcy < cy - 5.0 || cy - 2.0 <= pcy || 2.0 < adx || pcx <= cx - 0.5),
                _ => !(pcx < cx - 5.0 || cx - 2.0 <= pcx || 2.0 < ady || pcy <= cy - 0.5),
            };
        if clash {
            return Some(true);
        }
        let hit = match pdir & 3 {
            0 => !(cy + 4.0 < pcy || pcy < cy || 2.0 <= adx),
            1 => !(cx + 4.0 < pcx || pcx < cx || 2.0 <= ady),
            2 => !(pcy < cy - 4.0 || cy < pcy || 2.0 < adx),
            _ => !(pcx < cx - 4.0 || cx < pcx || 2.0 < ady),
        };
        hit.then_some(false)
    }

    /// A thrown object, bomb piece or shot at `(x, y)` flying along `dir`
    /// (`0x4d668c`): it hits within a tile of the centre across and from 2
    /// above to 1 below, takes `2 * power` and throws the baddy along the
    /// flight.
    pub fn object_test(&self, x: f64, y: f64) -> bool {
        let (cx, cy) = self.centre();
        self.hittable() && (cx - 1.0..=cx + 1.0).contains(&x) && (cy - 2.0..=cy + 1.0).contains(&y)
    }

    /// The flight push of [`Self::object_test`] for `dir`, as the `(dx, dy)`
    /// of [`Self::hurt`].
    pub fn object_push(dir: u8) -> (f64, f64) {
        match dir & 3 {
            0 => (0.0, 1.0),
            1 => (1.0, 0.0),
            2 => (0.0, -1.0),
            _ => (-1.0, 0.0),
        }
    }

    /// Prop `i` as the leader sends it (`0x4d86fc`). `ext` is the build's
    /// picture extension.
    pub fn prop(&self, i: u8, ext: &str) -> Vec<u8> {
        let g = |v: i32| (v + 32) as u8;
        let text = |s: &str| {
            let mut v = vec![g(s.len().min(223) as i32)];
            v.extend(s.bytes().take(223));
            v
        };
        match i {
            0 => vec![g(i32::from(self.id))],
            1 => vec![g((self.x * 2.0 + 0.5 + 0.0001).floor() as i32)],
            2 => vec![g((self.y * 2.0 + 0.5 + 0.0001).floor() as i32)],
            3 => vec![g(i32::from(self.kind))],
            4 => {
                // As the program does: a picture other than the default goes
                // out as the default name, the default as no name (GServer
                // notes that a hurt baddy loses its picture).
                let def = default_image(self.kind, ext);
                let name = if self.image.is_empty() || self.image.eq_ignore_ascii_case(&def) { String::new() } else { def };
                let mut v = vec![g(self.power)];
                v.extend(text(&name));
                v
            }
            5 => vec![g(i32::from(self.mode))],
            6 => vec![g(self.counter)],
            7 => vec![g(i32::from(self.dir) * 4 + i32::from(self.head))],
            8..=10 => text(&self.verses[usize::from(i - 8)]),
            _ => Vec::new(),
        }
    }

    /// All props, 0 to 10.
    pub fn props(&self, ext: &str) -> Vec<Vec<u8>> {
        (0..=10).map(|i| self.prop(i, ext)).collect()
    }

    /// Take a prop stream from the server (`0x4d8940`). True when it
    /// killed the baddy (mode 9).
    pub fn apply_props(&mut self, data: &[u8], w: &mut dyn World) -> bool {
        let mut d = data;
        let mut died = false;
        let v = |b: u8| i32::from(b) - 32;
        while d.len() > 1 {
            let id = v(d[0]);
            let mut used = 2;
            match id {
                0 => self.id = v(d[1]).clamp(0, 255) as u8,
                1 | 2 => {
                    let p = (f64::from(v(d[1])) / 2.0).clamp(0.0, 63.0);
                    if id == 1 {
                        self.x = p;
                        self.home.0 = p;
                    } else {
                        self.y = p;
                        self.home.1 = p;
                    }
                }
                3 => {
                    let k = v(d[1]);
                    self.kind = if (0..KINDS as i32).contains(&k) { k as u8 } else { 0 };
                    match self.kind {
                        4 => self.ani = 2,
                        6 => self.ani = 4,
                        _ => {}
                    }
                }
                4 => {
                    self.power = v(d[1]);
                    let len = d.get(2).map_or(0, |&b| v(b)).max(0) as usize;
                    used = 3 + len;
                    self.image = d.get(3..(3 + len).min(d.len())).map(crate::protocol::latin1).unwrap_or_default();
                }
                5 => {
                    let m = v(d[1]);
                    self.blink = m == i32::from(mode::HURT);
                    if self.blink {
                        self.hurt_sent = false;
                    }
                    if !self.hurt_sent {
                        self.mode = m.clamp(0, 255) as u8;
                    }
                    if m == i32::from(mode::HUNT) && self.say_sight {
                        self.say_sight = false;
                        self.say(0);
                    }
                    if m == i32::from(mode::HURT) && self.say_hurt {
                        self.say_hurt = false;
                        self.say_sight = false;
                        self.say(1);
                    }
                    if m == i32::from(mode::DEAD) {
                        self.die(w);
                        died = true;
                    }
                }
                6 => self.counter = v(d[1]),
                7 => {
                    let b = v(d[1]);
                    self.dir = ((b >> 2) & 3) as u8;
                    self.head = (b & 3) as u8;
                }
                8..=10 => {
                    let len = v(d[1]).max(0) as usize;
                    used = 2 + len;
                    self.verses[(id - 8) as usize] = d.get(2..(2 + len).min(d.len())).map(crate::protocol::latin1).unwrap_or_default();
                }
                _ => break,
            }
            d = d.get(used..).unwrap_or(&[]);
        }
        died
    }

    /// What the picture is: the custom one, else the type's default.
    pub fn image_name(&self, ext: &str) -> String {
        if self.image.is_empty() { default_image(self.kind, ext) } else { self.image.clone() }
    }
}

/// The `PLI_BADDYPROPS` (15) message for the props that changed since
/// `before` (`0x4d8e38`): the id, then each changed prop. `None` when
/// nothing changed.
pub fn props_message(id: u8, before: &[Vec<u8>], now: &[Vec<u8>]) -> Option<Vec<u8>> {
    let mut changed = Vec::new();
    for (i, p) in now.iter().enumerate() {
        if before.get(i) != Some(p) {
            changed.push(i as u8 + 32);
            changed.extend_from_slice(p);
        }
    }
    if changed.is_empty() {
        return None;
    }
    let mut m = vec![15 + 32, id + 32];
    m.extend(changed);
    m.push(b'\n');
    Some(m)
}

/// The `PLI_BADDYHURT` (16) message (`0x50c7b8`): the id, the hitter's
/// centre less the baddy's on each axis (limited to 4 tiles, in sixteenths
/// from -4) and the power. Read from the program, not seen on the wire yet.
pub fn hurt_message(id: u8, dx: f64, dy: f64, power: i32) -> Vec<u8> {
    let q = |v: f64| ((v.clamp(-4.0, 4.0) + 4.0) * 16.0 + 0.0001).floor() as i32;
    let g = |v: i32| (v.clamp(0, 223) + 32) as u8;
    vec![16 + 32, g(i32::from(id)), g(q(dx)), g(q(dy)), g(power), b'\n']
}

/// A `PLO_BADDYHURT` (27) the leader gets: id, `dx`, `dy`, power.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HurtIn {
    pub id: u8,
    pub dx: f64,
    pub dy: f64,
    pub power: i32,
}

pub fn parse_hurt(payload: &[u8]) -> Option<HurtIn> {
    let [id, a, b, p, ..] = payload else { return None };
    Some(HurtIn {
        id: id.wrapping_sub(32),
        dx: (f64::from(*a) - 96.0) / 16.0,
        dy: (f64::from(*b) - 96.0) / 16.0,
        power: i32::from(*p) - 32,
    })
}

/// Where a piece of a baddy's picture comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PieceSheet {
    /// The baddy's own picture.
    Baddy,
    /// `wbow1`.
    Bow,
    /// `sprites.png`.
    Sprites,
}

/// One piece to draw: a sprite id and its place in pixels from the
/// baddy's `(x, y)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Piece {
    pub sprite: u16,
    pub dx: i32,
    pub dy: i32,
    pub sheet: PieceSheet,
}

/// The baddy draw tables: `arrays.dat` arrays 20 (poses by `ani * 4 +
/// dir`), 22 (the death puff) and 23 (the swamp soldier in the water).
/// Each entry is a sprite id in the high 16 bits and signed offsets in
/// eighths of a tile.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DrawTables {
    pub poses: Vec<Vec<u32>>,
    pub puff: Vec<Vec<u32>>,
    pub swamp: Vec<Vec<u32>>,
}

fn unpack(v: u32) -> (u16, i32, i32) {
    ((v >> 16) as u16, i32::from(((v >> 8) & 0xff) as u8 as i8), i32::from((v & 0xff) as u8 as i8))
}

impl Baddy {
    /// The pieces of the picture now (`TComputer.virt_002`). `frame` is the
    /// drawing frame counter (the hurt blink is off on odd frames).
    pub fn pieces(&self, t: &DrawTables, frame: u64) -> Vec<Piece> {
        let mut raw: Vec<u32> = Vec::new();
        let get = |list: &Vec<Vec<u32>>, i: i32| list.get(i.max(0) as usize).cloned().unwrap_or_default();
        let pose = (self.ani * 4 + i32::from(self.dir)).max(0);
        if self.mode != mode::DIE || self.counter > 3 {
            match self.kind {
                4 if self.counter > 10 => {
                    let c = self.counter;
                    if !(17..=44).contains(&c) {
                        raw = get(&t.swamp, (c - 11).rem_euclid(2));
                    } else if c < 22 || (33..35).contains(&c) {
                        raw = get(&t.swamp, 2);
                    } else if c < 33 {
                        raw = get(&t.poses, pose).into_iter().skip(1).collect();
                        raw.extend(get(&t.swamp, 0).first());
                    } else if c < 37 {
                        raw = get(&t.swamp, 3);
                    }
                }
                4 => {}
                5 => {
                    if self.mode != mode::DIE && self.counter > 10 {
                        if !(17..=37).contains(&self.counter) {
                            raw = vec![(22 + (self.counter.rem_euclid(2)) as u32) << 16 | 8];
                        } else {
                            let up = (-self.hare_height() * 8.0 + 0.0001).floor() as i32;
                            raw = vec![0x30_0000 | (up as u8 as u32), 0x16_0008];
                        }
                    }
                }
                _ => raw = get(&t.poses, pose),
            }
        }
        let mut out = Vec::new();
        for v in raw {
            let (mut s, ox, oy) = unpack(v);
            if (48..52).contains(&s) && self.kind != 5 {
                s = 48 + u16::from(self.head);
            }
            if self.shot_out && (107..=110).contains(&s) {
                continue;
            }
            if self.kind == 9 && s >= 64 {
                continue;
            }
            if self.blink && (1..=106).contains(&s) && frame & 1 == 1 {
                continue;
            }
            let sheet = match s {
                48..=63 | 104..=106 | 117..=130 => PieceSheet::Baddy,
                111..=114 => PieceSheet::Bow,
                _ => PieceSheet::Sprites,
            };
            out.push(Piece { sprite: s, dx: ox * 2, dy: oy * 2, sheet });
        }
        if self.mode == mode::DIE && (0..10).contains(&self.counter) {
            for v in get(&t.puff, 4 - self.counter / 2) {
                let (s, ox, oy) = unpack(v);
                out.push(Piece { sprite: s, dx: ox * 2, dy: oy * 2, sheet: PieceSheet::Sprites });
            }
        }
        out
    }

    /// The verse to show over the baddy now, if any. It is drawn centred on
    /// `(x + 1, y - 2)`.
    pub fn shown_text(&self) -> Option<&str> {
        (self.text_ticks >= 0 && !self.text.is_empty()).then_some(self.text.as_str())
    }
}

impl DrawTables {
    /// The tables of a loaded `arrays.dat`, else the built-in copy of the
    /// 2.22 file's.
    pub fn from_tables(t: Option<&crate::assets::ObjectTables>) -> DrawTables {
        match t.filter(|t| !t.baddy_poses.is_empty()) {
            Some(t) => DrawTables { poses: t.baddy_poses.clone(), puff: t.baddy_puff.clone(), swamp: t.baddy_swamp.clone() },
            None => DrawTables::builtin(),
        }
    }

    /// The 2.22 `arrays.dat` arrays 20, 22 and 23.
    pub fn builtin() -> DrawTables {
        let own = |l: &[&[u32]]| l.iter().map(|e| e.to_vec()).collect();
        DrawTables { poses: own(BUILTIN_POSES), puff: own(BUILTIN_PUFF), swamp: own(BUILTIN_SWAMP) }
    }
}

// From the 2.22 `arrays.dat` (Graal Reborn copy); 2.05 reads the same arrays.
const BUILTIN_POSES: &[&[u32]] = &[
    &[0x00020a, 0x3c0df0, 0x68fdfd, 0x34fdff, 0x3000f7],
    &[0x00020a, 0x6afefe, 0x350203, 0x3100f6, 0x3df507],
    &[0x00020a, 0x3afd02, 0x3200f8, 0x680a04, 0x3efd0a],
    &[0x00020a, 0x690dfe, 0x370103, 0x3300f6, 0x3f0b07],
    &[0x00020a, 0x3c0df5, 0x68fdfe, 0x38fd02, 0x3000f7],
    &[0x00020a, 0x6afffe, 0x39ff00, 0x3100f5, 0x3df005],
    &[0x00020a, 0x36fd02, 0x3200f7, 0x680a04, 0x3efd08],
    &[0x00020a, 0x690dfe, 0x3b0100, 0x3300f5, 0x3f1005],
    &[0x00020a, 0x6f06fa, 0x6b0cf4, 0x34fdff, 0x3000f7],
    &[0x00020a, 0x350203, 0x3100f6, 0x70fcfb, 0x6cf600],
    &[0x00020a, 0x3afd02, 0x3200f8, 0x71ff06, 0x6d0403],
    &[0x00020a, 0x370103, 0x3300f6, 0x720dfb, 0x6e0b00],
    &[0x00020a, 0x6f06fa, 0x6b0cf4, 0x38fd02, 0x3000f7],
    &[0x00020a, 0x39ff00, 0x3100f5, 0x70fcfa, 0x6cf6ff],
    &[0x00020a, 0x36fd02, 0x3200f7, 0x71ff05, 0x6d0402],
    &[0x00020a, 0x3b0100, 0x3300f5, 0x720dfa, 0x6e0bff],
    &[0x00020a, 0x7500ff],
    &[0x00020a, 0x7600ff],
    &[0x00020a, 0x7700ff],
    &[0x00020a, 0x7800ff],
    &[0x00020a, 0x7900ff],
    &[0x00020a, 0x7a00ff],
    &[0x00020a, 0x7b00ff],
    &[0x00020a, 0x7c00ff],
    &[0x00020a, 0x7500ff],
    &[0x00020a, 0x7d00ff],
    &[0x00020a, 0x7e00ff],
    &[0x00020a, 0x7f00ff],
    &[0x00020a, 0x7500ff],
    &[0x00020a, 0x800000],
    &[0x00020a, 0x8100ff],
    &[0x00020a, 0x820000],
];
const BUILTIN_PUFF: &[&[u32]] = &[
    &[0x1f00ff],
    &[0x2000ff],
    &[0x210000],
    &[0x22fdfc, 0x230bfc, 0x24fd0a, 0x250b0a],
    &[0x26faf9, 0x270ef9, 0x28fa0d, 0x290e0d],
];
const BUILTIN_SWAMP: &[&[u32]] = &[
    &[0x730007],
    &[0x740007],
    &[0x3000fc, 0x730007],
    &[0x300000, 0x730007],
];
/// Source rectangles (array 24) of the sprites baddies use, from the
/// same file, for a client without one.
pub fn builtin_rect(id: u16) -> Option<(i32, i32, i32, i32)> {
    Some(match id {
        0 => (0, 0, 24, 12),
        22 => (0, 274, 32, 16),
        23 => (0, 290, 32, 16),
        31 => (0, 50, 32, 32),
        32 => (32, 50, 32, 32),
        33 => (64, 50, 32, 30),
        34 => (96, 18, 16, 16),
        35 => (112, 18, 16, 16),
        36 => (96, 34, 16, 16),
        37 => (112, 34, 16, 16),
        38 => (96, 50, 16, 16),
        39 => (112, 50, 16, 16),
        40 => (96, 66, 16, 16),
        41 => (112, 66, 16, 16),
        48 => (0, 0, 32, 32),
        49 => (32, 0, 32, 32),
        50 => (64, 0, 32, 32),
        51 => (96, 0, 32, 32),
        52 => (0, 58, 44, 32),
        53 => (88, 56, 26, 24),
        54 => (0, 32, 44, 26),
        55 => (88, 32, 26, 24),
        56 => (44, 58, 44, 26),
        57 => (32, 90, 32, 28),
        58 => (44, 32, 44, 26),
        59 => (0, 90, 32, 28),
        60 => (114, 32, 10, 32),
        61 => (64, 94, 32, 10),
        62 => (114, 64, 10, 32),
        63 => (64, 84, 32, 10),
        104 => (104, 96, 16, 22),
        105 => (120, 96, 8, 20),
        106 => (96, 96, 8, 20),
        107 => (10, 422, 10, 30),
        108 => (0, 412, 30, 10),
        109 => (0, 422, 10, 30),
        110 => (0, 402, 30, 10),
        111 => (0, 0, 32, 16),
        112 => (0, 32, 16, 32),
        113 => (0, 16, 32, 16),
        114 => (16, 32, 16, 32),
        115 => (96, 82, 32, 24),
        116 => (96, 106, 32, 24),
        117 => (0, 0, 32, 32),
        118 => (32, 0, 32, 32),
        119 => (64, 0, 32, 32),
        120 => (96, 0, 32, 32),
        121 => (0, 32, 32, 32),
        122 => (32, 32, 32, 30),
        123 => (64, 32, 32, 32),
        124 => (96, 32, 32, 30),
        125 => (32, 62, 32, 32),
        126 => (64, 64, 32, 32),
        127 => (96, 62, 32, 32),
        128 => (32, 94, 32, 30),
        129 => (0, 64, 32, 32),
        130 => (96, 94, 32, 30),
        131 => (64, 144, 16, 26),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An open field: walls only off the board.
    struct Field {
        player: Option<(f64, f64)>,
        rng: DelphiRandom,
        walls: Vec<(i32, i32)>,
        sounds: Vec<String>,
        shots: Vec<Shot>,
        touched: Vec<i32>,
    }

    impl Field {
        fn new(player: Option<(f64, f64)>) -> Field {
            Field { player, rng: DelphiRandom::new(1), walls: Vec::new(), sounds: Vec::new(), shots: Vec::new(), touched: Vec::new() }
        }
    }

    impl World for Field {
        fn blocked(&self, x: f64, y: f64) -> bool {
            !(0.0..64.0).contains(&x) || !(0.0..64.0).contains(&y) || self.walls.contains(&(x.floor() as i32, y.floor() as i32))
        }
        fn random(&mut self, n: u32) -> i32 {
            self.rng.next(n) as i32
        }
        fn player(&self) -> Option<(f64, f64)> {
            self.player
        }
        fn others(&self) -> Vec<(u16, f64, f64)> {
            Vec::new()
        }
        fn sound(&mut self, file: &str, _: f64, _: f64) {
            self.sounds.push(file.into());
        }
        fn shoot(&mut self, shot: Shot) {
            self.shots.push(shot);
        }
        fn touch_player(&mut self, half: i32, _: f64, _: f64) {
            self.touched.push(half);
        }
    }

    fn baddy(kind: u8, x: f64, y: f64) -> Baddy {
        let verses = ["Halt, intruder!".to_string(), "Ouch!".to_string(), "You got me...".to_string()];
        Baddy::new(1, kind, x, y, verses, (50.0, 50.0, 2), &mut DelphiRandom::new(7))
    }

    /// The six props of the testworld capture (real 2.22, S>C f14), taken
    /// as the program takes them.
    #[test]
    fn server_props_of_the_capture_make_the_baddies() {
        let hex = "21 21 7c 22 7c 23 20 24 22 2d 62 61 64 64 79 67 72 61 79 2e 70 6e 67 25 20 26 20 27 2a 28 2f 48 61 6c 74 2c 20 69 6e 74 72 75 64 65 72 21 29 25 4f 75 63 68 21 2a 2d 59 6f 75 20 67 6f 74 20 6d 65 2e 2e 2e";
        let bytes: Vec<u8> = hex.split(' ').map(|b| u8::from_str_radix(b, 16).unwrap()).collect();
        let mut w = Field::new(None);
        // the id first, then the props
        let mut b = Baddy::from_server(bytes[0] - 32, (0.0, 0.0, 2), &mut DelphiRandom::new(0));
        assert!(!b.apply_props(&bytes[1..], &mut w));
        assert_eq!((b.id, b.kind, b.x, b.y, b.power), (1, 0, 46.0, 46.0, 2));
        assert_eq!(b.image, "baddygray.png");
        assert_eq!((b.mode, b.counter, b.dir, b.head), (0, 0, 2, 2));
        assert_eq!(b.verses, ["Halt, intruder!".to_string(), "Ouch!".into(), "You got me...".into()]);
        // the hare (id 5): mode 7, prop 4 len 13 + name
        let hare = "25 21 90 22 8e 23 25 24 21 2d 62 61 64 64 79 68 61 72 65 2e 70 6e 67 25 27 26 20 27 2a";
        let hb: Vec<u8> = hare.split(' ').map(|b| u8::from_str_radix(b, 16).unwrap()).collect();
        let mut h = Baddy::from_server(hb[0] - 32, (0.0, 0.0, 2), &mut DelphiRandom::new(0));
        h.apply_props(&hb[1..], &mut w);
        assert_eq!((h.id, h.kind, h.x, h.y, h.mode, h.power), (5, 5, 56.0, 55.0, 7, 1));
    }

    /// The leader's answer in the capture: props 6 and 7 for a walker
    /// (`21 26 4b 27 29`), prop 6 alone for the hare (`25 26 22`).
    #[test]
    fn leader_diff_has_the_capture_shape() {
        let mut b = baddy(0, 46.0, 46.0);
        (b.counter, b.dir, b.head) = (0, 2, 2);
        let before = b.props(".png");
        b.counter = 43;
        b.dir = 2;
        b.head = 1;
        let m = props_message(1, &before, &b.props(".png")).unwrap();
        assert_eq!(m, vec![0x2f, 0x21, 0x26, 0x4b, 0x27, 0x29, b'\n']);
        assert_eq!(props_message(1, &b.props(".png"), &b.props(".png")), None);
    }

    #[test]
    fn a_walker_walks_a_quarter_tile_a_tick_and_counts_down() {
        let mut w = Field::new(None);
        let mut b = baddy(0, 20.0, 20.0);
        b.dir = 3;
        b.head = 3;
        b.counter = 43;
        b.tick(&mut w, true);
        assert_eq!((b.x, b.y, b.counter), (20.25, 20.0, 42));
        // the head turns before the body at a wall
        w.walls.push((23, 20));
        b.x = 20.5;
        b.tick(&mut w, true);
        assert_eq!((b.dir, b.head), (2, 1));
    }

    #[test]
    fn a_walker_that_sees_the_player_hunts_and_says_its_verse() {
        // facing down, the player 6 tiles below
        let mut w = Field::new(Some((19.5, 26.0)));
        let mut b = baddy(0, 20.0, 20.0);
        b.dir = 2;
        b.head = 2;
        b.counter = 40;
        b.tick(&mut w, true);
        assert_eq!(b.mode, mode::HUNT);
        assert_eq!(b.counter, 16);
        assert_eq!(b.shown_text(), Some("Halt, intruder!"));
        // ten alert ticks, then it runs at 0.5 a tick
        for _ in 0..13 {
            b.tick(&mut w, true);
        }
        assert!(b.y > 20.0, "{}", b.y);
    }

    #[test]
    fn the_sword_hurts_then_kills_a_gray_soldier() {
        let mut w = Field::new(Some((19.5, 22.0)));
        let mut b = baddy(0, 20.0, 20.0);
        b.dir = 2;
        b.head = 2;
        // player below facing up, centre (21, 24), baddy centre (21, 21):
        // a soldier facing the player clashes
        assert_eq!(b.sword_test(19.5, 22.0, 0), Some(true));
        b.dir = 1;
        assert_eq!(b.sword_test(19.5, 22.0, 0), Some(false));
        b.hurt(&mut w, false, 1, 0.0, 3.0, false);
        assert_eq!((b.mode, b.power, b.counter), (mode::HURT, 1, 8));
        assert!(!b.hittable());
        b.tick(&mut w, true);
        assert_eq!(b.y, 18.0, "thrown 2 tiles up");
        while b.mode == mode::HURT {
            b.tick(&mut w, true);
        }
        assert_eq!(b.mode, mode::HUNT);
        assert_eq!(b.verses[1], b.text);
        b.hurt(&mut w, false, 1, 0.0, 3.0, false);
        let mut died = false;
        for _ in 0..40 {
            died |= b.tick(&mut w, true);
        }
        assert!(died && b.removed);
        assert!(w.sounds.contains(&"compudead.wav".to_string()));
    }

    #[test]
    fn touching_takes_half_a_heart_the_golden_warrior_one() {
        let mut w = Field::new(Some((19.5, 19.0)));
        let mut b = baddy(0, 20.0, 20.0);
        b.mode = mode::LOOK;
        b.counter = 30;
        b.tick(&mut w, false);
        let mut g = baddy(7, 20.0, 20.0);
        g.tick(&mut w, false);
        assert_eq!(w.touched, vec![1, 2]);
    }

    #[test]
    fn baddyhurt_round_trips() {
        let m = hurt_message(3, 1.5, -2.0, 2);
        let h = parse_hurt(&m[1..m.len() - 1]).unwrap();
        assert_eq!((h.id, h.dx, h.dy, h.power), (3, 1.5, -2.0, 2));
    }

    #[test]
    fn the_hare_jumps_out_and_is_only_hit_in_the_air() {
        let mut w = Field::new(None);
        let mut h = baddy(5, 30.0, 30.0);
        assert_eq!(h.mode, mode::HAREJUMP);
        assert!(!h.hittable());
        for _ in 0..27 {
            h.tick(&mut w, true);
        }
        assert_eq!(h.counter, 27);
        assert!(h.hittable());
        assert_eq!(h.hare_height(), 8.0);
        assert!(w.sounds.contains(&"frog.wav".to_string()));
    }
}
