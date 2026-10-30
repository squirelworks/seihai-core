//! Pausing (STATUS bit 0x01).
//!
//! Sources: Graal_analysis `protocol/player-props.md` and
//! `client-internals/feature-specs/11-emotes-ganis.md` (STATUS 0x01 is
//! "editor or paused"), `decomp-findings-2026-10-24-d/03-hit-gates-and-open-points.md`
//! (the hit gate), `versions-133-139.md` (P is the pause key) and the v5
//! and 3.04 labs (`version-profiles/lab-v5-2026-10-30.md`,
//! `lab-304-linux-2026-10-30.md`), where a client whose window has no focus
//! shows to others as `name (paused)`.
//!
//! * **P** toggles pause. A window without the focus is paused as well.
//! * STATUS gets bit 0x01 while paused.
//! * A paused player cannot be hurt, unless the level is a `sparringzone`.
//!   Seihai has no sparring zones, so a paused player is never hurt.
//! * Other players' names get ` (paused)` when their STATUS has the bit
//!   (the 2.00 name label, 0x4ebd1a).
//! * The STATUS the server gives back is kept without bit 0x01. The lab
//!   accounts' template has STATUS 21 (0x15), so copying the bit would show
//!   the player as paused to everyone; only P and the focus set it.
//!
//! Not read yet: whatever else pause stops in the real client (walking,
//! timers). Seihai only marks the state and applies the hit gate.

use super::OfflineSession;

/// STATUS bit 0x01: paused (or in the editor).
pub const PAUSED_STATUS: u8 = 0x01;

impl OfflineSession {
    /// The pause key: toggle the player's own pause. Returns the new state.
    pub fn toggle_pause(&mut self) -> bool {
        self.pause_key = !self.pause_key;
        self.paused = self.pause_key || !self.focused;
        self.paused
    }

    /// The window gained or lost the focus. Without it the player is paused.
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.paused = self.pause_key || !self.focused;
    }

    /// The STATUS bits pausing adds.
    pub fn pause_status(&self) -> u8 {
        if self.paused {
            PAUSED_STATUS
        } else {
            0
        }
    }
}

/// The server's STATUS for our own player, without the paused bit: pause
/// is ours to set (P or the focus), never taken from the account.
pub fn server_status_base(status: u8) -> u8 {
    status & !PAUSED_STATUS
}

/// The name label of another player: `unknown` for an empty name, with
/// ` (paused)` added while its STATUS has the paused bit.
pub fn name_label(nick: &str, status: Option<u8>) -> String {
    let name = if nick.is_empty() { "unknown" } else { nick };
    if status.is_some_and(|s| s & PAUSED_STATUS != 0) {
        format!("{name} (paused)")
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::Vulnerability;

    #[test]
    fn p_toggles_pause_and_the_status_bit() {
        let mut s = OfflineSession::new();
        assert_eq!(s.pause_status(), 0);
        assert!(s.toggle_pause());
        assert_eq!(s.pause_status(), PAUSED_STATUS);
        assert!(!s.toggle_pause());
        assert_eq!(s.pause_status(), 0);
    }

    #[test]
    fn losing_the_focus_pauses_until_it_comes_back() {
        let mut s = OfflineSession::new();
        s.set_focused(false);
        assert!(s.paused);
        s.set_focused(true);
        assert!(!s.paused);
        // The key's pause outlives the focus coming back.
        s.toggle_pause();
        s.set_focused(false);
        s.set_focused(true);
        assert!(s.paused);
    }

    #[test]
    fn a_paused_player_cannot_be_hurt() {
        let mut s = OfflineSession::new();
        assert_eq!(s.victim().state, Vulnerability::Hurtable);
        s.toggle_pause();
        assert_eq!(s.victim().state, Vulnerability::Paused);
        s.toggle_pause();
        assert_eq!(s.victim().state, Vulnerability::Hurtable);
    }

    #[test]
    fn the_accounts_status_does_not_pause_us() {
        // STATUS 21 from the lab accounts' template.
        assert_eq!(server_status_base(21), 0x14);
        assert_eq!(server_status_base(0x02), 0x02);
    }

    #[test]
    fn names_show_paused_from_the_status_bit() {
        assert_eq!(name_label("bob", Some(0x11)), "bob (paused)");
        assert_eq!(name_label("bob", Some(0x10)), "bob");
        assert_eq!(name_label("bob", None), "bob");
        assert_eq!(name_label("", Some(0x01)), "unknown (paused)");
    }
}
