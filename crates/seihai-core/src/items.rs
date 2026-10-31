//! Items lying on the floor: what a cut or lifted bush and a lifted pot
//! leave, and what the server puts down (`PLO_ITEMADD`).
//!
//! Read from the 2.05 rev 2 binary (the item object's draw and the level's
//! item tick) and the 1.3 test binary (the drop roll), and checked in the
//! lab with real 1.39, 2.05 rev 2 and 2.22 clients. See
//! `documentation/item-drops.md`.

/// Ticks (50 ms) an item lies before it times out: 164, about 8.2 s. The
/// lab measured 8.20 s (2.05) from the drop to the client's `PLI_ITEMDEL`.
pub const ITEM_LIFE: i32 = 164;

/// An item can be taken only once its life is below this, so not in the
/// first four ticks after it appears.
pub const TAKE_BELOW: i32 = 160;

/// Below this life the item blinks: it is not drawn on odd ticks.
pub const BLINK_BELOW: i32 = 37;

/// The bounce at the drop runs while the life is at least `164 - 7π`
/// (the 2.05 constant, 142.0088), about the first 22 ticks.
pub const BOUNCE_END: f32 = 164.0 - 7.0 * std::f32::consts::PI;

/// The shadow under an item: sprite 42.
pub const ITEM_SHADOW_SPRITE: u16 = 42;

/// Index of each item in the 25-entry item table (the CHEST and
/// `PLO_ITEMADD` numbering).
pub const GREENRUPEE: u8 = 0;
pub const BLUERUPEE: u8 = 1;
pub const REDRUPEE: u8 = 2;
pub const BOMBS: u8 = 3;
pub const DARTS: u8 = 4;
pub const HEART: u8 = 5;

/// The sprite each item is drawn with (2.05 rev 2 table).
pub const ITEM_SPRITES: [u16; 25] =
    [136, 137, 138, 67, 143, 139, 145, 351, 65, 350, 314, 315, 321, 323, 322, 324, 325, 328, 329, 330, 331, 332, 333, 334, 349];

/// Items that lie on the floor as a pickup (2.05 rev 2 table). The others
/// (weapons, swords, shields) are not put down by `PLO_ITEMADD`.
pub const FLOOR_ITEMS: [bool; 25] =
    [true, true, true, true, true, true, false, true, true, false, false, false, false, false, false, false, false, false, false, true, false, false, false, false, true];

/// Delphi's `Random`, which the 1.x clients roll drops with: a 32-bit
/// linear congruential generator, `seed = seed * 0x08088405 + 1`, and
/// `Random(n)` is the high half of `seed * n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelphiRandom {
    pub seed: u32,
}

impl DelphiRandom {
    pub fn new(seed: u32) -> Self {
        Self { seed }
    }

    /// Seeded from the clock, as `Randomize` does.
    pub fn from_clock() -> Self {
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u32).unwrap_or(0);
        Self::new(ms)
    }

    /// A number from 0 to `range - 1`.
    pub fn next(&mut self, range: u32) -> u32 {
        self.seed = self.seed.wrapping_mul(0x0808_8405).wrapping_add(1);
        ((u64::from(self.seed) * u64::from(range)) >> 32) as u32
    }
}

/// What a bush leaves in 1.33 to 1.41, from one `Random(100)`: 0 to 24 a
/// green rupee, 25 and 26 a blue rupee, 27 to 31 a heart, 32 to 36 bombs,
/// and nothing for 37 to 99. The same table is in every build from the
/// 4th alpha to 1.41; no client rolls arrows, and none drops anything for
/// a stone.
pub fn bush_drop(roll: u32) -> Option<u8> {
    match roll {
        0..=24 => Some(GREENRUPEE),
        25..=26 => Some(BLUERUPEE),
        27..=31 => Some(HEART),
        32..=36 => Some(BOMBS),
        _ => None,
    }
}

/// One item on the floor. `(x, y)` is the top-left of the 2 x 2 object it
/// came from, in tiles; the item is drawn centred on `(x + 1, y + 1)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloorItem {
    pub item: u8,
    pub x: f32,
    pub y: f32,
    /// Ticks left; it counts down from [`ITEM_LIFE`].
    pub life: i32,
}

/// What happened to an item on a tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ItemEvent {
    /// The player rolled this item (1.x): the server hears of it from 1.38
    /// on (`PLI_ITEMADD`).
    Rolled { item: u8, x: f32, y: f32 },
    /// The player laid this item (S held and A, then A): the server hears
    /// of it from 1.38 on, for floor items only.
    Laid { item: u8, x: f32, y: f32 },
    /// The player walked over it.
    Taken { item: u8, x: f32, y: f32 },
    /// Its time ran out.
    Expired { x: f32, y: f32 },
}

impl FloorItem {
    pub fn new(item: u8, x: f32, y: f32) -> Self {
        Self { item, x, y, life: ITEM_LIFE }
    }

    /// The sprite it is drawn with.
    pub fn sprite(&self) -> Option<u16> {
        ITEM_SPRITES.get(usize::from(self.item)).copied()
    }

    /// Whether it is drawn this tick: it blinks (odd ticks off) once its
    /// life is below [`BLINK_BELOW`].
    pub fn visible(&self) -> bool {
        !(self.life < BLINK_BELOW && self.life % 2 != 0)
    }

    /// How far above its place it is drawn, in tiles: a bounce of
    /// `|sin(life / 3)| * (life / 164)^16 * 3` while the life is at least
    /// [`BOUNCE_END`], then none.
    pub fn lift(&self) -> f32 {
        let life = self.life as f32;
        if life < BOUNCE_END {
            return 0.0;
        }
        let f = (life / ITEM_LIFE as f32).powi(16);
        (life / 3.0).sin().abs() * f * 3.0
    }

    /// Whether the player at `(px, py)` takes it: within 2 tiles of the
    /// item's middle `(x + 1, y + 1)`, measured from `(px + 1.5, py + 2)`,
    /// and not in its first ticks.
    pub fn taken_by(&self, px: f32, py: f32) -> bool {
        self.life < TAKE_BELOW && ((px + 1.5) - (self.x + 1.0)).abs() < 2.0 && ((py + 2.0) - (self.y + 1.0)).abs() < 2.0
    }
}

/// One 50 ms tick of every item, in the 2.05 order: each item's life goes
/// down by one, then it is taken if the player stands on it, else it times
/// out once the life is below 0. Taken and timed-out items are removed.
pub fn tick(items: &mut Vec<FloorItem>, px: f32, py: f32, can_take: bool) -> Vec<ItemEvent> {
    let mut out = Vec::new();
    items.retain_mut(|it| {
        it.life -= 1;
        if can_take && it.taken_by(px, py) {
            out.push(ItemEvent::Taken { item: it.item, x: it.x, y: it.y });
            return false;
        }
        if it.life < 0 {
            out.push(ItemEvent::Expired { x: it.x, y: it.y });
            return false;
        }
        true
    });
    out
}

/// `PLO_ITEMADD` (22): `GCHAR x*2, GCHAR y*2, GCHAR item`. Only floor items
/// are kept, as the 2.05 handler does.
pub fn parse_item_add(payload: &[u8]) -> Option<FloorItem> {
    let [x, y, item, ..] = payload else { return None };
    let item = item.wrapping_sub(0x20);
    if !FLOOR_ITEMS.get(usize::from(item)).copied().unwrap_or(false) {
        return None;
    }
    Some(FloorItem::new(item, f32::from(x.wrapping_sub(0x20)) / 2.0, f32::from(y.wrapping_sub(0x20)) / 2.0))
}

/// `PLO_ITEMDEL` (23): `GCHAR x*2, GCHAR y*2`.
pub fn parse_item_del(payload: &[u8]) -> Option<(f32, f32)> {
    let [x, y, ..] = payload else { return None };
    Some((f32::from(x.wrapping_sub(0x20)) / 2.0, f32::from(y.wrapping_sub(0x20)) / 2.0))
}

fn half(v: f32) -> u8 {
    ((v * 2.0).round() as i32).clamp(0, 223) as u8 + 0x20
}

/// `PLI_ITEMADD` (12), `PLI_ITEMTAKE` (32): `GCHAR x*2, GCHAR y*2, GCHAR item`.
fn item_message(id: u8, item: u8, x: f32, y: f32) -> Vec<u8> {
    vec![id + 0x20, half(x), half(y), item + 0x20, b'\n']
}

/// Whether `item` lies on the floor as a pickup ([`FLOOR_ITEMS`]): only
/// these are told to the server when added or taken (2.05).
pub fn is_floor_item(item: u8) -> bool {
    FLOOR_ITEMS.get(usize::from(item)).copied().unwrap_or(false)
}

/// The roll or a lay told to the server: `PLI_ITEMADD` (12).
pub fn item_add_message(item: u8, x: f32, y: f32) -> Vec<u8> {
    item_message(12, item, x, y)
}

/// A pickup that the server gives (1.38 on): `PLI_ITEMTAKE` (32).
pub fn item_take_message(item: u8, x: f32, y: f32) -> Vec<u8> {
    item_message(32, item, x, y)
}

/// An item gone from the floor: `PLI_ITEMDEL` (13), `GCHAR x*2, GCHAR y*2`.
pub fn item_del_message(x: f32, y: f32) -> Vec<u8> {
    vec![13 + 0x20, half(x), half(y), b'\n']
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bush_table_of_the_1x_clients() {
        let counts = (0..100).fold([0; 6], |mut c, r| {
            if let Some(i) = bush_drop(r) {
                c[usize::from(i)] += 1;
            }
            c
        });
        assert_eq!(counts, [25, 2, 0, 5, 0, 5], "green 25 %, blue 2 %, bombs 5 %, heart 5 %, never red or arrows");
        assert_eq!(bush_drop(37), None);
    }

    #[test]
    fn delphi_random_is_the_high_half_of_seed_times_range() {
        let mut r = DelphiRandom::new(0);
        assert_eq!(r.next(100), 0, "seed 1");
        assert_eq!(r.seed, 1);
        assert_eq!(r.next(100), 3, "seed 0x08088406: 0x08088406 * 100 >> 32");
        let mut r = DelphiRandom::new(0xffff_ffff);
        assert_eq!(r.next(100), 96);
    }

    #[test]
    fn an_item_times_out_after_165_ticks() {
        let mut v = vec![FloorItem::new(GREENRUPEE, 10.0, 10.0)];
        for _ in 0..164 {
            assert!(tick(&mut v, 40.0, 40.0, true).is_empty());
        }
        assert_eq!(tick(&mut v, 40.0, 40.0, true), vec![ItemEvent::Expired { x: 10.0, y: 10.0 }]);
        assert!(v.is_empty());
    }

    #[test]
    fn it_is_taken_from_the_fifth_tick_within_two_tiles() {
        let mut v = vec![FloorItem::new(HEART, 31.0, 34.0)];
        // Standing on it: (30, 33.5) has its test point at (31.5, 35.5).
        for _ in 0..4 {
            assert!(tick(&mut v, 30.0, 33.5, true).is_empty(), "not in the first four ticks");
        }
        assert_eq!(tick(&mut v, 30.0, 33.5, true), vec![ItemEvent::Taken { item: HEART, x: 31.0, y: 34.0 }]);
        let it = FloorItem { life: 100, ..FloorItem::new(HEART, 31.0, 34.0) };
        // The test point (px + 1.5, py + 2) against the middle (32, 35).
        assert!(it.taken_by(30.0, 31.1), "1.9 tiles above");
        assert!(!it.taken_by(30.0, 31.0), "2 tiles is out");
        assert!(!it.taken_by(30.0, 35.0), "2 tiles below is out");
        assert!(it.taken_by(28.6, 34.0));
        assert!(!it.taken_by(28.5, 34.0));
        assert!(!it.taken_by(32.5, 34.0));
    }

    #[test]
    fn it_blinks_at_the_end_and_bounces_at_the_start() {
        let mut it = FloorItem::new(GREENRUPEE, 0.0, 0.0);
        assert!(it.visible());
        assert_eq!(it.lift(), (164.0f32 / 3.0).sin().abs() * 3.0);
        it.life = 143;
        assert!(it.lift() > 0.0);
        it.life = 142;
        assert_eq!(it.lift(), 0.0);
        it.life = 37;
        assert!(it.visible());
        it.life = 35;
        assert!(!it.visible());
        it.life = 34;
        assert!(it.visible());
    }

    #[test]
    fn server_items_and_the_messages_match_the_lab_capture() {
        // 2.05 against GServer-v2: S>C 22 `^d"` is a red rupee at (31, 34).
        assert_eq!(parse_item_add(b"^d\""), Some(FloorItem::new(REDRUPEE, 31.0, 34.0)));
        assert_eq!(parse_item_add(&[0x20, 0x20, 0x20 + 9]), None, "a shield is not a floor item");
        assert_eq!(parse_item_del(b"^d"), Some((31.0, 34.0)));
        // The pickup that followed: C>S 32 `^d!`; the timeout: C>S 13 `^d`.
        assert_eq!(item_take_message(BLUERUPEE, 31.0, 34.0), b"@^d!\n");
        assert_eq!(item_del_message(31.0, 34.0), b"-^d\n");
        // 1.39's roll: C>S 12 `^d ` (a green rupee).
        assert_eq!(item_add_message(GREENRUPEE, 31.0, 34.0), b",^d \n");
    }
}
