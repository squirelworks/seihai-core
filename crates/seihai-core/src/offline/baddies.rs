//! The level's baddies in the session (see [`crate::baddies`]).
//!
//! Offline they come from the level's `BADDY` records; online only from
//! the server's `PLO_BADDYPROPS`. Offline, or online as the level leader,
//! this client runs their AI; online as the leader it sends the props that
//! changed each tick, and as a follower it sends its hits to the leader.

use super::OfflineSession;
use crate::baddies::{self as b, Baddy, Shot, World};
use crate::sound::SoundEvent;

/// The session's view of the game for a baddy.
struct Around<'a> {
    s: &'a OfflineSession,
    rng: crate::items::DelphiRandom,
    player: Option<(f64, f64)>,
    others: Vec<(u16, f64, f64)>,
    sounds: Vec<SoundEvent>,
    shots: Vec<Shot>,
    touches: Vec<(i32, f64, f64)>,
}

impl World for Around<'_> {
    fn blocked(&self, x: f64, y: f64) -> bool {
        self.s.baddy_blocked(x as f32, y as f32)
    }
    fn random(&mut self, n: u32) -> i32 {
        self.rng.next(n) as i32
    }
    fn player(&self) -> Option<(f64, f64)> {
        self.player
    }
    fn others(&self) -> Vec<(u16, f64, f64)> {
        self.others.clone()
    }
    fn sound(&mut self, file: &str, x: f64, y: f64) {
        self.sounds.push(SoundEvent { file: file.into(), x: x as f32, y: y as f32 });
    }
    fn shoot(&mut self, shot: Shot) {
        self.shots.push(shot);
    }
    fn touch_player(&mut self, half: i32, dx: f64, dy: f64) {
        self.touches.push((half, dx, dy));
    }
}

/// How the session's baddies run this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BaddyRole {
    /// Connected to a server.
    pub online: bool,
    /// The server said this client leads the level (`PLO_ISLEADER`).
    pub leader: bool,
}

impl BaddyRole {
    /// Whether this client runs the AI.
    pub fn thinks(self) -> bool {
        !self.online || self.leader
    }

    /// Online, not the leader: hits go to the leader.
    pub fn follows(self) -> bool {
        self.online && !self.leader
    }
}

impl OfflineSession {
    /// Whether a baddy may not stand on `(x, y)` (`0x540140`): off the
    /// board, a wall, a chest, water, a chair or a bed bottom. NPCs that
    /// block are not tested yet.
    pub(crate) fn baddy_blocked(&self, x: f32, y: f32) -> bool {
        if !(0.0..64.0).contains(&x) || !(0.0..64.0).contains(&y) {
            return true;
        }
        let Some(level) = &self.current_level else { return true };
        if let Some(types) = &self.tiletypes {
            if level.kind_at(types, x, y) == crate::assets::TileKind::Blocking {
                return true;
            }
            if let Some(id) = level.tile_id_at(x as i32, y as i32) {
                use crate::assets::{TYPE_BED_BOTTOM, TYPE_CHAIR, TYPE_WATER};
                if matches!(types.types.get(usize::from(id)).copied(), Some(TYPE_WATER | TYPE_CHAIR | TYPE_BED_BOTTOM)) {
                    return true;
                }
            }
        }
        level.chests.iter().any(|c| x >= c.x as f32 && x < c.x as f32 + 2.0 && y >= c.y as f32 && y < c.y as f32 + 2.0)
    }

    fn player_seed(&self) -> (f64, f64, u8) {
        (f64::from(self.player_x), f64::from(self.player_y), self.player_dir)
    }

    /// A new level: the baddies of the old one are gone. Offline, the new
    /// level's `BADDY` records come alive, numbered from 1 as GServer does.
    fn sync_baddy_level(&mut self, role: BaddyRole) {
        if self.baddy_serial == Some(self.level_serial) {
            return;
        }
        self.baddy_serial = Some(self.level_serial);
        self.baddies.clear();
        self.baddy_clock_ms = 0;
        if role.online {
            return;
        }
        let Some(level) = &self.current_level else { return };
        let player = self.player_seed();
        let mut rng = self.item_rng;
        let made: Vec<Baddy> = level.baddies.iter().enumerate().map(|(i, r)| Baddy::from_level((i + 1).min(255) as u8, r, player, &mut rng)).collect();
        self.item_rng = rng;
        self.baddies = made;
    }

    /// Online: the server forgot the level's baddies (a new level).
    pub fn clear_baddies(&mut self) {
        self.baddies.clear();
        self.baddy_serial = Some(self.level_serial);
    }

    /// A `PLO_BADDYPROPS` (2): the id, then props.
    pub fn receive_baddy_props(&mut self, payload: &[u8]) {
        let Some((&id, props)) = payload.split_first() else { return };
        let id = id.wrapping_sub(32);
        // The level the props are for is the one now loaded.
        self.baddy_serial = Some(self.level_serial);
        let player = self.player_seed();
        let i = match self.baddies.iter().position(|b| b.id == id) {
            Some(i) => i,
            None => {
                let mut rng = self.item_rng;
                self.baddies.push(Baddy::from_server(id, player, &mut rng));
                self.item_rng = rng;
                self.baddies.len() - 1
            }
        };
        let mut baddies = std::mem::take(&mut self.baddies);
        let mut w = self.around(Vec::new());
        let died = baddies[i].apply_props(props, &mut w);
        let sounds = std::mem::take(&mut w.sounds);
        self.sound_events.extend(sounds);
        if died {
            baddies.remove(i);
        }
        self.baddies = baddies;
    }

    /// A `PLO_BADDYHURT` (27): another player hit one of our baddies (we
    /// lead the level).
    pub fn receive_baddy_hurt(&mut self, payload: &[u8]) {
        let Some(h) = b::parse_hurt(payload) else { return };
        let mut baddies = std::mem::take(&mut self.baddies);
        if let Some(bd) = baddies.iter_mut().find(|bd| bd.id == h.id) {
            let mut w = self.around(Vec::new());
            bd.hurt(&mut w, false, h.power, h.dx, h.dy, false);
            let sounds = std::mem::take(&mut w.sounds);
            self.sound_events.extend(sounds);
        }
        self.baddies = baddies;
    }

    fn around(&self, others: Vec<(u16, f64, f64)>) -> Around<'_> {
        let player = (!self.life.is_dead() && !self.hidden && self.current_level.is_some()).then(|| self.player_seed()).map(|(x, y, _)| (x, y));
        Around { s: self, rng: self.item_rng, player, others, sounds: Vec::new(), shots: Vec::new(), touches: Vec::new() }
    }

    /// Run the baddies for `ms`: the sword test on each 50 ms player tick
    /// during swing ticks 1 to 3, and the baddy tick on every second one.
    /// `others` are the other players in the level, `(id, x, y)`; the role
    /// is [`OfflineSession::baddy_role`]. True when
    /// something changed on screen.
    pub fn tick_baddies(&mut self, ms: u64, others: &[(u16, f32, f32)]) -> bool {
        let role = self.baddy_role;
        self.sync_baddy_level(role);
        if self.baddies.is_empty() {
            return false;
        }
        let others: Vec<(u16, f64, f64)> = others.iter().map(|&(id, x, y)| (id, f64::from(x), f64::from(y))).collect();
        self.baddy_clock_ms += ms;
        let mut changed = false;
        while self.baddy_clock_ms >= b::PLAYER_TICK_MS {
            self.baddy_clock_ms -= b::PLAYER_TICK_MS;
            self.baddy_phase = !self.baddy_phase;
            self.baddy_sword(role);
            if self.baddy_phase {
                self.baddy_tick(role, &others);
                changed = true;
            }
        }
        changed
    }

    fn baddy_sword(&mut self, role: BaddyRole) {
        let Some(ms) = self.combat.swing_elapsed_ms() else { return };
        if !(b::PLAYER_TICK_MS..4 * b::PLAYER_TICK_MS).contains(&ms) || self.sword_power == 0 {
            return;
        }
        let (px, py, pdir) = self.player_seed();
        let power = i32::from(self.sword_power);
        let mut baddies = std::mem::take(&mut self.baddies);
        let mut w = self.around(Vec::new());
        let mut sends = Vec::new();
        for bd in baddies.iter_mut() {
            let Some(clash) = bd.sword_test(px, py, pdir) else { continue };
            let (cx, cy) = bd.centre();
            let (dx, dy) = (px + 1.5 - cx, py + 2.0 - cy);
            bd.hurt(&mut w, clash, power, dx, dy, role.follows());
            if role.follows() && !clash {
                sends.push(b::hurt_message(bd.id, dx, dy, power));
            }
        }
        let sounds = std::mem::take(&mut w.sounds);
        self.sound_events.extend(sounds);
        self.baddy_sends.extend(sends);
        self.baddies = baddies;
    }

    fn baddy_tick(&mut self, role: BaddyRole, others: &[(u16, f64, f64)]) {
        let mut baddies = std::mem::take(&mut self.baddies);
        let mut w = self.around(others.to_vec());
        let ext = self.baddy_image_ext.clone();
        let mut sends = Vec::new();
        for bd in baddies.iter_mut() {
            bd.tick(&mut w, role.thinks());
            if role.online && role.leader {
                let now = bd.props(&ext);
                match &bd.sent {
                    // The first tick only keeps the props, as the program
                    // does: the server already has them.
                    None => {}
                    Some(before) => sends.extend(b::props_message(bd.id, before, &now)),
                }
                bd.sent = Some(now);
            } else {
                bd.sent = None;
            }
        }
        let Around { rng, sounds, shots, touches, .. } = w;
        self.item_rng = rng;
        baddies.retain(|bd| !bd.removed);
        self.baddies = baddies;
        self.sound_events.extend(sounds);
        self.baddy_sends.extend(sends);
        for s in shots {
            let mut a = crate::arrows::Arrow::new(s.x as f32, s.y as f32, s.dir);
            a.sprite = s.sprite;
            a.kind = s.kind;
            self.arrows.push(a);
        }
        for (half, dx, dy) in touches {
            self.baddy_touch(half, dx, dy);
        }
    }

    /// A baddy ran into the player (`0x4fbc38`): hurt mode with a push away
    /// from the baddy, when the player can be hurt.
    fn baddy_touch(&mut self, half: i32, dx: f64, dy: f64) {
        if self.victim().state != crate::combat::Vulnerability::Hurtable {
            return;
        }
        if self.damage_half(half, None) {
            self.log_hurt("baddy", None, half);
        }
        self.combat.start_hurt_knock(Some((dx as f32, dy as f32)));
        self.throw_hurt = true;
    }

    /// A thrown object's probe point against the baddies (`0x4d668c`): the
    /// first one it hits takes `2 * power` and is thrown along the flight.
    /// True when one was hit (the object stops).
    pub(crate) fn throw_hits_baddy(&mut self, x: f32, y: f32, dir: u8, power: i32) -> bool {
        let role = self.baddy_role;
        let (x, y) = (f64::from(x), f64::from(y));
        let Some(i) = self.baddies.iter().position(|bd| bd.object_test(x, y)) else { return false };
        let mut baddies = std::mem::take(&mut self.baddies);
        let (dx, dy) = Baddy::object_push(dir);
        let mut w = self.around(Vec::new());
        baddies[i].hurt(&mut w, false, power * 2, dx, dy, role.follows());
        let sounds = std::mem::take(&mut w.sounds);
        self.sound_events.extend(sounds);
        if role.follows() {
            self.baddy_sends.push(b::hurt_message(baddies[i].id, dx, dy, power * 2));
        }
        self.baddies = baddies;
        true
    }

    /// The `PLI_BADDYPROPS` and `PLI_BADDYHURT` messages since the last call.
    pub fn take_baddy_sends(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.baddy_sends)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::Level;
    use crate::baddies::mode;
    use crate::protocol::ClientVersion;

    fn session(baddy: &str) -> OfflineSession {
        let mut s = OfflineSession::new();
        s.current_level = Some(Level::parse(&format!("GLEVNW01\nBADDY {baddy}\n\n\n\nBADDYEND\n")).unwrap());
        s.level_serial += 1;
        let profile = ClientVersion::V205.profile();
        s.life.set_rules(profile.life.death);
        s.combat.set_rules(profile.life.combat);
        s.player_x = 30.0;
        s.player_y = 30.0;
        s
    }

    /// Offline the level's baddy spawns, finds the standing player and hurts
    /// them; the hurt time keeps it to one hit at a time.
    #[test]
    fn a_soldier_finds_and_hurts_the_player() {
        let mut s = session("26 30 0");
        let full = s.player_health;
        let mut hits = 0;
        for _ in 0..1200 {
            s.tick_combat(50);
            s.tick_baddies(50, &[]);
            if s.player_health < full && hits == 0 {
                hits = 1;
                assert!(s.combat.is_hurt());
            }
        }
        assert_eq!(s.baddies.len(), 1);
        assert!(s.player_health < full, "never hurt");
        assert!(s.take_baddy_sends().is_empty(), "offline sends nothing");
    }

    /// The sword kills a gray soldier (power 2) in two hits and it is removed
    /// after its death.
    #[test]
    fn the_sword_kills_a_soldier() {
        let mut s = session("26 30 0");
        s.sword_power = 1;
        s.player_dir = crate::offline::DIR_WEST;
        s.tick_baddies(50, &[]);
        for _ in 0..2 {
            // Stand it next to the player, facing away, and swing.
            let b = &mut s.baddies[0];
            (b.x, b.y, b.dir, b.mode, b.counter) = (28.5, 30.5, 1, mode::WALK, 20);
            assert!(s.start_swing());
            for _ in 0..10 {
                s.tick_combat(50);
                s.tick_baddies(50, &[]);
            }
        }
        assert!(s.baddies.is_empty() || s.baddies[0].power < 1, "power {}", s.baddies[0].power);
        for _ in 0..100 {
            s.tick_baddies(50, &[]);
        }
        assert!(s.baddies.iter().all(|b| b.removed) || s.baddies.is_empty());
    }
}
