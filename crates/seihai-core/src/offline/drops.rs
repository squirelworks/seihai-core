//! What a bush or a pot leaves, and the items on the floor.
//!
//! Per build ([`crate::protocol::ItemDrops`], Graal_analysis
//! `client-internals/item-drops.md`):
//!
//! * **1.33 to 1.41** roll the drop themselves: one `Random(100)` for every
//!   cut (any entry of the cut table, tall grass too) and for every lifted
//!   bush, on the table of [`crate::items::bush_drop`]. A lifted pot always
//!   leaves a heart. A sign or a stone leaves nothing.
//! * **1.38 to 1.41** also tell the server of the roll with `PLI_ITEMADD`.
//!   The 1.39 lab capture: C>S 1 (the cut), then C>S 12 `^d `.
//! * **1.42 on** roll nothing: the server drops (`PLO_ITEMADD`), and
//!   offline nothing drops at all.
//! * **Pickup.** Online from 1.38 the client sends `PLI_ITEMTAKE` and the
//!   server gives the item with props; before 1.38, and offline, the client
//!   applies it and (online) sends `PLI_ITEMDEL`.
//! * **Timeout.** After 164 ticks of 50 ms the item goes; online from 1.38
//!   the client sends `PLI_ITEMDEL` (2.05 lab: 8.20 s after the drop).
//! * **Chest.** An opened chest shows its item one tile above it for 24
//!   ticks, and its end sends `PLI_ITEMDEL` at that place too. Real 2.22 on
//!   GServer-RS (2026-10-31): three chests, each `PLI_OPENCHEST` followed
//!   1.19 to 1.20 s later by C>S 13 at `(x, y - 1)`
//!   (`documentation/chest-drawing.md`).
//!
//! The UDP messages 1.33 to 1.41 also send to the other players (19, 20)
//! are not sent: Seihai has no UDP.

use super::OfflineSession;
use crate::sound::SoundEvent;
use crate::items::{self, FloorItem, ItemEvent};

/// The item clock: one life step every 50 ms.
pub const ITEM_TICK_MS: u64 = 50;

/// The item table's names, in the `PLO_ITEMADD` order (the CHEST names).
pub const ITEM_NAMES: [&str; 25] = [
    "greenrupee", "bluerupee", "redrupee", "bombs", "darts", "heart", "glove1", "bow", "bomb", "shield", "sword", "fullheart", "superbomb",
    "battleaxe", "goldensword", "mirrorshield", "glove2", "lizardshield", "lizardsword", "goldrupee", "fireball", "fireblast", "nukeshot",
    "joltbomb", "spinattack",
];

/// The most rupees, bombs and arrows the 2.05 pickup gives up to.
pub const MAX_RUPEES: i32 = 9_999_999;
pub const MAX_BOMBS: i32 = 99;
pub const MAX_DARTS: i32 = 99;

/// Ticks the item of an opened chest is shown: 24, 1.2 s.
pub const CHEST_SHOW_TICKS: i32 = 24;

/// The lift-table entries that drop: the bush rolls, the pot leaves a heart.
const LIFT_BUSH: usize = 0;
const LIFT_POT: usize = 2;

impl OfflineSession {
    /// The drop rules of this build.
    pub fn set_drops(&mut self, d: crate::protocol::ItemDrops) {
        self.drops = d;
    }

    /// A cut at the object whose top-left tile is `(x, y)`: the 1.x roll.
    pub(crate) fn cut_drop(&mut self, x: i32, y: i32) {
        if self.drops.client_roll {
            self.roll_drop(x, y);
        }
    }

    /// A lift of entry `kind` at `(x, y)`: a bush rolls, a pot leaves a heart.
    pub(crate) fn lift_drop(&mut self, kind: usize, x: i32, y: i32) {
        if !self.drops.client_roll {
            return;
        }
        match kind {
            LIFT_BUSH => self.roll_drop(x, y),
            LIFT_POT => self.put_rolled(items::HEART, x, y),
            _ => {}
        }
    }

    fn roll_drop(&mut self, x: i32, y: i32) {
        if let Some(item) = items::bush_drop(self.item_rng.next(100)) {
            self.put_rolled(item, x, y);
        }
    }

    fn put_rolled(&mut self, item: u8, x: i32, y: i32) {
        let (x, y) = (x as f32, y as f32);
        self.floor_items.push(FloorItem::new(item, x, y));
        self.item_events.push(ItemEvent::Rolled { item, x, y });
    }

    /// Show the item of the chest at `(x, y)` one tile above it; its end is
    /// an item timeout there.
    pub(crate) fn show_chest_item(&mut self, x: i32, y: i32) {
        self.chest_shows.push((x as f32, (y - 1) as f32, CHEST_SHOW_TICKS));
    }

    /// An item the server put down (`PLO_ITEMADD`).
    pub fn add_server_item(&mut self, it: FloorItem) {
        self.floor_items.push(it);
    }

    /// An item the server took away (`PLO_ITEMDEL`): every item at `(x, y)`.
    pub fn remove_server_item(&mut self, x: f32, y: f32) {
        self.floor_items.retain(|it| it.x != x || it.y != y);
    }

    /// Run the items' 50 ms clock by `ms`. A pickup plays `extra.wav`; it is
    /// applied here offline, and online before 1.38 (from 1.38 the server
    /// gives it). Nothing is taken while dead.
    pub fn tick_items(&mut self, ms: u64, online: bool) {
        self.item_ms += ms;
        while self.item_ms >= ITEM_TICK_MS {
            self.item_ms -= ITEM_TICK_MS;
            let events = &mut self.item_events;
            self.chest_shows.retain_mut(|(x, y, left)| {
                *left -= 1;
                if *left > 0 {
                    return true;
                }
                events.push(ItemEvent::Expired { x: *x, y: *y });
                false
            });
            if self.floor_items.is_empty() {
                continue;
            }
            let can_take = !self.life.is_dead();
            for e in items::tick(&mut self.floor_items, self.player_x, self.player_y, can_take) {
                if let ItemEvent::Taken { item, x, y } = e {
                    self.sound_events.push(SoundEvent { file: "extra.wav".into(), x: x + 1.0, y: y + 1.0 });
                    // A laid glove, sword or shield is no server item: it
                    // is given here and nothing is sent (2.05).
                    if !online || !self.drops.server_take || !items::is_floor_item(item) {
                        self.apply_pickup(item);
                    }
                }
                self.item_events.push(e);
            }
        }
    }

    /// What the item events since the last call send to the server, by the
    /// build's rules. Offline nothing is sent (the events are dropped).
    pub fn take_item_messages(&mut self, online: bool) -> Vec<Vec<u8>> {
        let events = std::mem::take(&mut self.item_events);
        if !online {
            return Vec::new();
        }
        let d = self.drops;
        events
            .into_iter()
            .filter_map(|e| match e {
                ItemEvent::Rolled { item, x, y } => d.add_to_server.then(|| items::item_add_message(item, x, y)),
                // A lay: 1.38 on (the builds that tell the server of an item
                // they add), floor items only.
                ItemEvent::Laid { item, x, y } => (lay_to_server(d) && items::is_floor_item(item)).then(|| items::item_add_message(item, x, y)),
                ItemEvent::Taken { item, .. } if !items::is_floor_item(item) => None,
                ItemEvent::Taken { item, x, y } if d.server_take => Some(items::item_take_message(item, x, y)),
                ItemEvent::Taken { x, y, .. } => Some(items::item_del_message(x, y)),
                ItemEvent::Expired { x, y } => d.del_on_expiry.then(|| items::item_del_message(x, y)),
            })
            .collect()
    }

    /// Give a floor item, as the 2.05 pickup does: rupees 1, 5, 30 and 100
    /// up to 9,999,999, five bombs or arrows up to 99, a heart one whole
    /// heart. The rest go as the chest item of the same name.
    pub fn apply_pickup(&mut self, item: u8) {
        let add = |v: &mut i32, n: i32, max: i32| *v = (*v + n).min(max);
        match item {
            items::GREENRUPEE => add(&mut self.rupees, 1, MAX_RUPEES),
            items::BLUERUPEE => add(&mut self.rupees, 5, MAX_RUPEES),
            items::REDRUPEE => add(&mut self.rupees, 30, MAX_RUPEES),
            19 => add(&mut self.rupees, 100, MAX_RUPEES),
            items::BOMBS => add(&mut self.bombs, 5, MAX_BOMBS),
            items::DARTS => add(&mut self.darts, 5, MAX_DARTS),
            items::HEART => {
                let h = self.hearts_half();
                self.set_hearts_half(h + 2);
            }
            other => {
                if let Some(name) = ITEM_NAMES.get(usize::from(other)) {
                    self.give_item(name);
                }
            }
        }
    }
}

/// Whether this build tells the server of an item it lays: 1.38 to 1.41
/// send the roll, and 1.42 on have the server drop; both send
/// `PLI_ITEMADD` from their item add. 1.33 to 1.37 use UDP only.
fn lay_to_server(d: crate::protocol::ItemDrops) -> bool {
    d.add_to_server || !d.client_roll
}
