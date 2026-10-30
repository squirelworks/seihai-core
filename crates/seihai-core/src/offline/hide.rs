//! Hiding under a carried object (2.05 mode 14).
//!
//! Read in the 2.05 rev 2 client (Graal_analysis
//! `client-internals/carry-hide-and-glove.md`) and seen live
//! (`player-mannerisms-live.md`):
//!
//! * **D while carrying** a bush, a light stone or a black stone hides the
//!   player under it. A pot or a sign: D does nothing.
//! * D hides only once the object counts as carried: four ticks into the
//!   lift (the lift counter, seen live). While lifting the player can still
//!   be hit, as a carrier can (lab, 2026-10-30: real 2.05 took a hit while
//!   carrying).
//! * **D again**, or **A**, comes out, back to carrying. Out from under a
//!   bush the leaves fly at `(x + 0.5, y + 1)`.
//! * **S** throws, from hiding too.
//! * Walking hidden goes at a fifth of the speed: 0.1 tile a tick.
//! * Nothing can hit a hidden player. STATUS gets bit 0x02.
//! * **Look, per build** ([`crate::protocol::HidePose`]): 2.05 and 2.22 play
//!   `hiddenstill` / `hidden` and draw only the object, where the player
//!   stands; 1.39 and NewMain 1.41 keep the carry poses.
//! * **Wire.** `hiddenstill` is sprite 18, the same as `carrystill`;
//!   `hidden` walks 19, 20, 19, 18. A 2.00 to 2.05 build sends neither:
//!   the real 2.00 keeps `carrystill` / `carry` in prop 10 and plain
//!   SPRITE, and only STATUS changes (`bow_slot_anim`). A real 2.00
//!   watcher draws a hidden player as the object on the ground at its
//!   feet, and nothing at all with `idle`, `hidden` or `hiddenstill`.

use super::OfflineSession;

/// Lift-table entries that can be hidden under: bush, light stone, black
/// stone (`arrays.dat` array 11 order: bush, sign, pot, stone, black stone).
pub const HIDE_KINDS: [usize; 3] = [0, 3, 4];
/// The STATUS (18) bit of a hidden player.
pub const HIDDEN_STATUS: u8 = 0x02;
/// The tick after a lift from which the object counts as carried (2.05,
/// live: the lift counter runs 4 down to 0 and the carried flag is set as
/// it reaches 0). Before it D does not hide.
pub const LIFT_CARRIED_TICK: u32 = 4;
/// The walk speed factor while hidden.
pub const HIDE_SPEED: f32 = 0.2;
/// Sprite numbers S of the `hidden` walk, one per step.
pub const HIDDEN_WALK_S: [u8; 4] = [19, 20, 19, 18];

impl OfflineSession {
    /// D: hide under what is carried, or come out. True if the state
    /// changed.
    pub fn toggle_hide(&mut self) -> bool {
        // An open sign takes D (2.05): it turns the page instead.
        if self.reading.is_some() {
            return false;
        }
        if self.hidden {
            self.leave_hiding();
            return true;
        }
        let lifting = self.lift_ticks.is_some_and(|t| t < LIFT_CARRIED_TICK);
        let can = !lifting && self.carried.as_ref().is_some_and(|c| HIDE_KINDS.contains(&c.kind));
        if !can || self.life.is_dead() || self.in_water {
            return false;
        }
        self.hidden = true;
        self.hide_steps = 0;
        true
    }

    /// Come out of hiding, back to carrying. Out from under a bush the
    /// leaves fly.
    pub fn leave_hiding(&mut self) {
        if !std::mem::take(&mut self.hidden) {
            return;
        }
        if self.carried.as_ref().is_some_and(|c| c.kind == 0) {
            self.spawn_leap(crate::throwing::cut_leap_kind(0), self.player_x + 0.5, self.player_y + 1.0);
        }
    }

    /// The figure is not drawn: hidden, on a build that shows only the
    /// object.
    pub fn figure_hidden(&self) -> bool {
        self.hidden && self.mannerisms.hide == crate::protocol::HidePose::Hidden
    }

    /// The STATUS bits hiding adds.
    pub fn hide_status(&self) -> u8 {
        if self.hidden {
            HIDDEN_STATUS
        } else {
            0
        }
    }

    /// The sprite number S of the hidden poses, if hiding.
    pub(crate) fn hidden_sprite_s(&self) -> Option<u8> {
        self.hidden.then(|| if self.walking { HIDDEN_WALK_S[self.hide_steps % 4] } else { crate::combat::CARRY_SPRITE_S })
    }
}

#[cfg(test)]
mod tests {
    use crate::offline::{Carried, OfflineSession};
    use crate::protocol::{MANNER_141, MANNER_205};

    fn carrying(kind: usize) -> OfflineSession {
        let mut s = OfflineSession::new();
        s.current_level = Some(crate::assets::Level::from_board(&[0u16; 64 * 64]));
        s.tiletypes = Some(crate::assets::TileTypes::from_types([0; 4096]));
        s.player_x = 30.0;
        s.player_y = 30.0;
        s.carried = Some(Carried { kind, tiles: [[0, 0], [0, 0]] });
        s
    }

    #[test]
    fn d_hides_under_a_bush_or_stone_but_not_a_pot_or_sign() {
        for (kind, ok) in [(0, true), (1, false), (2, false), (3, true), (4, true)] {
            let mut s = carrying(kind);
            assert_eq!(s.toggle_hide(), ok, "kind {kind}");
            assert_eq!(s.hidden, ok);
        }
        let mut empty = OfflineSession::new();
        assert!(!empty.toggle_hide());
    }

    #[test]
    fn d_during_the_lift_does_not_hide_and_the_lifter_can_be_hit() {
        let mut s = carrying(0);
        s.lift_ticks = Some(0);
        assert!(!s.toggle_hide(), "D on the first tick of the lift");
        assert_eq!(s.victim().state, crate::combat::Vulnerability::Hurtable);
        for _ in 0..3 {
            s.tick_poses(50);
            assert!(!s.toggle_hide(), "still lifting at tick {:?}", s.lift_ticks);
        }
        // The fourth tick: the object counts as carried, with or without a
        // `lift.gani`.
        s.tick_poses(50);
        assert!(s.lift_ticks.is_none());
        assert!(s.toggle_hide());
        assert_eq!(s.victim().state, crate::combat::Vulnerability::Hidden);
    }

    #[test]
    fn hidden_walks_a_fifth_as_fast_and_cannot_be_hit() {
        let mut s = carrying(0);
        s.toggle_hide();
        assert_eq!(s.hide_status(), super::HIDDEN_STATUS);
        assert!(s.try_move(0.5, 0.0).is_some());
        assert_eq!(s.player_x, 30.1);
        assert_eq!(s.sprite_value(), s.player_dir + 4 * 20, "the second step of the hidden walk");
        assert_eq!(s.victim().state, crate::combat::Vulnerability::Hidden);
        // D again: out, the leaves fly, full speed again.
        assert!(s.toggle_hide());
        assert!(!s.hidden);
        assert_eq!(s.leaps.len(), 1);
        s.try_move(0.5, 0.0);
        assert_eq!(s.player_x, 30.6);
    }

    #[test]
    fn a_throw_ends_hiding_and_the_look_follows_the_build() {
        let mut s = carrying(3);
        s.set_mannerisms(MANNER_205);
        s.toggle_hide();
        assert!(s.figure_hidden());
        s.set_mannerisms(MANNER_141);
        assert!(!s.figure_hidden(), "NewMain 1.41 keeps the carry poses");
        assert!(s.throw_carried().is_some());
        assert!(!s.hidden);
    }
}
