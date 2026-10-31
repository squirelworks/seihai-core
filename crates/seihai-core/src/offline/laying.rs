//! Laying items: hold the attack key (S) and press A to pick an item, then A
//! to put it down in front of the player.
//!
//! Read in the 2.05 rev 2 client (its key handlers, the test of which items
//! the player has, what laying takes away, and the item add and its send).
//! The readme of 1.21
//! names it: "Laying Items: Select Item: hold Attack Button (S) + press
//! Action Button (A); Lay Item: Action Button (A)". The same 24-entry lay
//! order and `item2.wav` are in every Windows build from 1.1 to 2.31, so
//! Seihai uses the 2.05 rules for all of them; only 2.05 rev 2 was read.
//!
//! * **Picking.** A with S held, when A does nothing else (nothing to throw,
//!   lift, grab or open) and the point in front of the player
//!   ([`OfflineSession::facing_point`]) is neither a wall nor water: the
//!   player goes into the lay mode (2.05 mode 9) with the first item of
//!   [`LAY_ORDER`] they have, the idle pose, and `item2.wav`. Nothing to
//!   lay: nothing happens.
//! * **In the lay mode** the item is drawn above the head, centred on
//!   `(x + 1.5, y)`, and the player does not walk. A with S held picks the
//!   next item they have (`item2.wav`). A alone lays it. An arrow, D or S
//!   leaves the mode without doing anything else.
//! * **Laying.** The item goes down at `(x + 0.5, y + 1)` moved two tiles
//!   the way the player faces (three up), with the usual 164 ticks of life,
//!   it is taken away from the player ([`lay_cost`]), and `item.wav` plays.
//! * **Wire.** The item add sends `PLI_ITEMADD` (12) `x*2, y*2, item` only
//!   for the items that lie on the floor as pickups
//!   ([`crate::items::FLOOR_ITEMS`]): rupees, bombs, arrows, a heart, the
//!   gold rupee and the spin attack. A laid glove, sword or shield stays on
//!   this client only. The layout is the one the 1.39 capture shows for a
//!   rolled drop. Builds before 1.38 tell the others by UDP only (as for a
//!   bush drop, `item-drops.md`; inferred for the lay, not read), which
//!   Seihai does not have, so they send nothing.
//!
//! Not modelled: a custom sword or shield image (2.05 lays it as a special
//! item that carries the picture), the horse, and the `playerlaysitem`
//! event. The lay bytes were confirmed by a lab capture of the real 2.05 rev 2
//! (`documentation/item-laying.md`).

use super::OfflineSession;
use crate::assets::TileKind;
use crate::items::{self, FloorItem, ItemEvent};
use crate::sound::SoundEvent;

/// The order A with S held goes through the items (2.05 rev 2; the full heart
/// container is not in it).
pub const LAY_ORDER: [u8; 24] = [0, 1, 2, 19, 3, 4, 5, 6, 16, 8, 12, 23, 7, 20, 21, 22, 9, 15, 17, 10, 13, 18, 14, 24];

/// Where a laid item goes, from the player's sprite top-left, by facing
/// (north, west, south, east): `(x + 0.5 + dx, y + 1 + dy)`.
pub const LAY_OFFSET: [(f32, f32); 4] = [(0.0, -3.0), (-2.0, 0.0), (0.0, 2.0), (2.0, 0.0)];

/// The sound of picking an item, and of laying it.
pub const PICK_SOUND: &str = "item2.wav";
pub const LAY_SOUND: &str = "item.wav";

/// What laying `item` takes away, if the player can lay it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayCost {
    Rupees(i32),
    Bombs(i32),
    Darts(i32),
    /// One whole heart; there must be at least one and a half.
    Heart,
    /// The glove, sword or shield power goes down to this.
    Glove(u8),
    Sword(u8),
    Shield(u8),
    SpinAttack,
}

impl OfflineSession {
    /// What laying `item` would take, if the player has it now (2.05). A
    /// sword or shield is layable only as the
    /// exact power it gives: the sword item at sword power 1, the battle axe
    /// at 2, and so on.
    pub fn lay_cost(&self, item: u8) -> Option<LayCost> {
        match item {
            items::GREENRUPEE => (self.rupees >= 1).then_some(LayCost::Rupees(1)),
            items::BLUERUPEE => (self.rupees >= 5).then_some(LayCost::Rupees(5)),
            items::REDRUPEE => (self.rupees >= 30).then_some(LayCost::Rupees(30)),
            19 => (self.rupees >= 100).then_some(LayCost::Rupees(100)),
            items::BOMBS => (self.bombs >= 5).then_some(LayCost::Bombs(5)),
            items::DARTS => (self.darts >= 5).then_some(LayCost::Darts(5)),
            items::HEART => (self.hearts_half() >= 3).then_some(LayCost::Heart),
            6 => (self.glove_power == 2).then_some(LayCost::Glove(1)),
            16 => (self.glove_power == 3).then_some(LayCost::Glove(2)),
            10 => (self.sword_power == 1).then_some(LayCost::Sword(0)),
            13 => (self.sword_power == 2).then_some(LayCost::Sword(1)),
            18 => (self.sword_power == 3).then_some(LayCost::Sword(2)),
            14 => (self.sword_power == 4).then_some(LayCost::Sword(3)),
            9 => (self.shield_power == 1).then_some(LayCost::Shield(0)),
            15 => (self.shield_power == 2).then_some(LayCost::Shield(1)),
            17 => (self.shield_power == 3).then_some(LayCost::Shield(2)),
            24 => self.combat.has_spin().then_some(LayCost::SpinAttack),
            _ => None,
        }
    }

    /// The next item the player can lay after `after` in [`LAY_ORDER`]
    /// (from the start with `None`), wrapping round to `after` itself.
    pub fn next_layable(&self, after: Option<u8>) -> Option<u8> {
        let start = after.and_then(|a| LAY_ORDER.iter().position(|&i| i == a)).map_or(0, |p| p + 1);
        (0..LAY_ORDER.len()).map(|k| LAY_ORDER[(start + k) % LAY_ORDER.len()]).find(|&i| self.lay_cost(i).is_some())
    }

    /// A with S held, when A did nothing else: go into the lay mode with the
    /// first item the player has. True if it started.
    pub fn start_laying(&mut self) -> bool {
        if self.laying.is_some()
            || self.life.is_dead()
            || self.carried.is_some()
            || self.hidden
            || self.reading.is_some()
            || self.warp_pending.is_some()
            || self.in_water
            || self.jumping()
            || self.combat.is_swinging()
            || self.combat.spin_elapsed_ms().is_some()
            || !self.lay_spot_free()
        {
            return false;
        }
        let Some(item) = self.next_layable(None) else { return false };
        self.laying = Some(item);
        self.walking = false;
        self.lay_sound(PICK_SOUND);
        true
    }

    /// A with S held in the lay mode: the next item. True if it changed.
    pub fn next_lay_item(&mut self) -> bool {
        let Some(cur) = self.laying else { return false };
        // 2.05 picks again even when nothing else is there.
        self.laying = self.next_layable(Some(cur));
        self.lay_sound(PICK_SOUND);
        self.laying.is_none_or(|i| i != cur)
    }

    /// A alone in the lay mode: put the item down, if the player still has
    /// it, and leave the mode. Returns the item laid.
    pub fn lay_item(&mut self) -> Option<u8> {
        let item = self.laying.take()?;
        let cost = self.lay_cost(item)?;
        if self.current_level.is_some() {
            let (dx, dy) = LAY_OFFSET[usize::from(self.player_dir & 3)];
            let (x, y) = (self.player_x + 0.5 + dx, self.player_y + 1.0 + dy);
            self.floor_items.push(FloorItem::new(item, x, y));
            self.item_events.push(ItemEvent::Laid { item, x, y });
        }
        match cost {
            LayCost::Rupees(n) => self.rupees -= n,
            LayCost::Bombs(n) => self.bombs -= n,
            LayCost::Darts(n) => self.darts -= n,
            LayCost::Heart => {
                let h = self.hearts_half();
                self.set_hearts_half(h - 2);
            }
            LayCost::Glove(p) => self.glove_power = p,
            LayCost::Sword(p) => self.sword_power = p,
            LayCost::Shield(p) => self.shield_power = p,
            LayCost::SpinAttack => self.combat.set_has_spin(false),
        }
        self.lay_sound(LAY_SOUND);
        Some(item)
    }

    /// Any other game key in the lay mode: leave it. True if it was on.
    pub fn stop_laying(&mut self) -> bool {
        self.laying.take().is_some()
    }

    /// The point in front of the player is neither a wall (or a chest) nor
    /// water (2.05).
    fn lay_spot_free(&self) -> bool {
        let (fx, fy) = self.facing_point();
        if self.blocked_point(fx, fy) {
            return false;
        }
        match (&self.current_level, &self.tiletypes) {
            (Some(level), Some(types)) => self.kind_at_point(level, types, fx, fy) != TileKind::Water,
            _ => true,
        }
    }

    fn lay_sound(&mut self, file: &str) {
        self.sound_events.push(SoundEvent { file: file.into(), x: self.player_x + 1.5, y: self.player_y + 2.0 });
    }
}

/// Where the item being picked is drawn: centred on this point.
pub fn lay_held_centre(px: f32, py: f32) -> (f32, f32) {
    (px + 1.5, py)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offline::{DIR_EAST, DIR_NORTH};

    fn session() -> OfflineSession {
        let mut s = OfflineSession::new();
        s.player_x = 30.0;
        s.player_y = 30.0;
        s.rupees = 0;
        s.bombs = 0;
        s.darts = 0;
        s.sword_power = 0;
        s.shield_power = 0;
        s.glove_power = 1;
        // One heart: too few to lay one.
        s.set_hearts_half(2);
        s
    }

    #[test]
    fn the_order_skips_what_the_player_lacks() {
        let mut s = session();
        assert_eq!(s.next_layable(None), None);
        s.rupees = 40;
        s.bombs = 5;
        assert_eq!(s.next_layable(None), Some(items::GREENRUPEE));
        assert_eq!(s.next_layable(Some(items::GREENRUPEE)), Some(items::BLUERUPEE));
        assert_eq!(s.next_layable(Some(items::BLUERUPEE)), Some(items::REDRUPEE));
        // No gold rupee under 100: bombs next, then round to the start.
        assert_eq!(s.next_layable(Some(items::REDRUPEE)), Some(items::BOMBS));
        assert_eq!(s.next_layable(Some(items::BOMBS)), Some(items::GREENRUPEE));
    }

    #[test]
    fn swords_shields_and_gloves_go_down_one_step() {
        let mut s = session();
        s.sword_power = 3;
        s.shield_power = 2;
        s.glove_power = 3;
        assert_eq!(s.lay_cost(10), None, "the plain sword only at power 1");
        assert_eq!(s.lay_cost(18), Some(LayCost::Sword(2)));
        assert_eq!(s.lay_cost(15), Some(LayCost::Shield(1)));
        assert_eq!(s.lay_cost(16), Some(LayCost::Glove(2)));
        assert_eq!(s.lay_cost(6), None);
        assert_eq!(s.lay_cost(11), None, "the heart container is never laid");
    }

    #[test]
    fn a_heart_needs_one_and_a_half() {
        let mut s = session();
        s.max_hearts = 6;
        s.set_hearts_half(2);
        assert_eq!(s.lay_cost(items::HEART), None);
        s.set_hearts_half(3);
        assert_eq!(s.lay_cost(items::HEART), Some(LayCost::Heart));
    }

    #[test]
    fn s_and_a_pick_then_a_lays_in_front() {
        let mut s = session();
        s.rupees = 7;
        s.player_dir = DIR_NORTH;
        assert!(s.start_laying());
        assert_eq!(s.laying, Some(items::GREENRUPEE));
        assert!(s.next_lay_item());
        assert_eq!(s.laying, Some(items::BLUERUPEE));
        assert_eq!(s.sound_events.iter().filter(|e| e.file == PICK_SOUND).count(), 2);

        s.current_level = Some(crate::assets::Level::parse("GLEVNW01\n").unwrap());
        s.sound_events.clear();
        assert_eq!(s.lay_item(), Some(items::BLUERUPEE));
        assert_eq!(s.laying, None);
        assert_eq!(s.rupees, 2);
        assert_eq!(s.floor_items, vec![FloorItem::new(items::BLUERUPEE, 30.5, 28.0)]);
        assert_eq!(s.sound_events.last().map(|e| e.file.as_str()), Some(LAY_SOUND));
    }

    #[test]
    fn east_lays_two_tiles_right() {
        let mut s = session();
        s.current_level = Some(crate::assets::Level::parse("GLEVNW01\n").unwrap());
        s.bombs = 9;
        s.player_dir = DIR_EAST;
        s.laying = Some(items::BOMBS);
        assert_eq!(s.lay_item(), Some(items::BOMBS));
        assert_eq!(s.bombs, 4);
        assert_eq!(s.floor_items[0].x, 32.5);
        assert_eq!(s.floor_items[0].y, 31.0);
    }

    #[test]
    fn nothing_to_lay_does_not_start() {
        let mut s = session();
        assert!(!s.start_laying());
        assert_eq!(s.laying, None);
    }

    #[test]
    fn carrying_does_not_start() {
        let mut s = session();
        s.rupees = 1;
        s.carried = Some(crate::offline::Carried { kind: 0, tiles: [[0, 0], [0, 0]] });
        assert!(!s.start_laying());
    }

    #[test]
    fn a_floor_item_goes_to_the_server_and_a_sword_does_not() {
        let mut s = session();
        s.current_level = Some(crate::assets::Level::parse("GLEVNW01\n").unwrap());
        s.set_drops(crate::protocol::DROPS_142);
        s.rupees = 1;
        s.sword_power = 1;
        s.player_dir = DIR_NORTH;
        s.laying = Some(items::GREENRUPEE);
        s.lay_item();
        s.laying = Some(10);
        s.lay_item();
        assert_eq!(s.sword_power, 0);
        let msgs = s.take_item_messages(true);
        assert_eq!(msgs, vec![items::item_add_message(items::GREENRUPEE, 30.5, 28.0)]);
    }

    #[test]
    fn before_138_a_lay_sends_nothing() {
        let mut s = session();
        s.current_level = Some(crate::assets::Level::parse("GLEVNW01\n").unwrap());
        s.set_drops(crate::protocol::DROPS_133);
        s.rupees = 1;
        s.laying = Some(items::GREENRUPEE);
        s.lay_item();
        assert!(s.take_item_messages(true).is_empty());
    }
}
