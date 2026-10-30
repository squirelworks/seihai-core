//! Developer mode.
//!
//! Everything a developer-only feature needs sits behind one struct, so
//! the whole thing can be removed in one change: delete this module, the
//! `dev` field on `OfflineSession`, and the F9 handler.
//!
//! Rules:
//! - Off at every start. It is never saved in settings.
//! - It switches itself off on a warp, a level change and a disconnect
//!   (callers use [`DevMode::reset`]), so there is no hidden key to turn
//!   it off. F9 still toggles it.
//! - While it is on the screen shows a `DEV MODE` badge.
//! - With the `dev` cargo feature off, [`DevMode::toggle`] does nothing
//!   and the wallhack code is never reachable.

/// Developer-only switches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DevMode {
    /// Walk through walls and chests.
    wallhack: bool,
}

impl DevMode {
    /// True when this build has developer mode at all.
    pub const AVAILABLE: bool = cfg!(feature = "dev");

    /// True while the wallhack is on.
    pub fn wallhack(&self) -> bool {
        Self::AVAILABLE && self.wallhack
    }

    /// True while any developer feature is on.
    pub fn active(&self) -> bool {
        self.wallhack()
    }

    /// F9. Returns the new state. Does nothing without the `dev` feature.
    pub fn toggle(&mut self) -> bool {
        if Self::AVAILABLE {
            self.wallhack = !self.wallhack;
        }
        self.active()
    }

    /// Switch everything off. Returns true if something was on.
    pub fn reset(&mut self) -> bool {
        let was = self.active();
        self.wallhack = false;
        was
    }
}

/// Text for the on-screen badge, when developer mode is on.
pub fn badge(dev: &DevMode) -> Option<&'static str> {
    dev.active().then_some("DEV MODE (F9 off)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_by_default_and_toggles() {
        let mut d = DevMode::default();
        assert!(!d.active());
        assert!(badge(&d).is_none());
        assert_eq!(d.toggle(), DevMode::AVAILABLE);
        if DevMode::AVAILABLE {
            assert!(badge(&d).is_some());
            assert!(!d.toggle());
        }
    }

    #[test]
    fn reset_turns_it_off() {
        let mut d = DevMode::default();
        d.toggle();
        assert_eq!(d.reset(), DevMode::AVAILABLE);
        assert!(!d.active());
        assert!(!d.reset());
    }
}
