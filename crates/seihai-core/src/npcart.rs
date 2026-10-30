//! NPC animations (item 23): the state side of `setcharani`, `setani` and
//! `showcharacter`. The drawing is `render::draw_npcs`.
//!
//! An NPC whose script chose an animation (`ani` on [`crate::assets::Npc`])
//! is drawn as that GANI instead of its image. The session keeps the loaded
//! GANIs by name; [`OfflineSession::wanted_npc_anis`] lists the ones an NPC
//! asks for and no one has loaded yet, so the host can load them.
//!
//! Timing is Seihai's own convention, because the real clients' is not
//! documented here: an animation advances one step every
//! [`NPC_ANI_STEP_MS`]. A `LOOP` animation wraps; any other stays on its
//! last step unless it has `SETBACKTO name`, which then switches the NPC to
//! that animation (the sword swing's `SETBACKTO idle` does the same for the
//! player).

use crate::assets::Gani;
use crate::offline::OfflineSession;

/// One animation step of an NPC.
pub const NPC_ANI_STEP_MS: u64 = 100;

/// The animation name a script gave: lower case, no `.gani`, no trailing
/// parameters. `None` for an empty name.
pub fn ani_name(raw: &str) -> Option<String> {
    let n = raw.trim().trim_matches('"').to_ascii_lowercase();
    let n = n.strip_suffix(".gani").unwrap_or(&n).trim().to_string();
    (!n.is_empty()).then_some(n)
}

impl OfflineSession {
    /// Attach a GANI for the NPCs, by name (`idle`, `walk`, ...).
    pub fn set_npc_gani(&mut self, name: &str, gani: Gani) {
        self.npc_ganis.insert(name.to_ascii_lowercase(), gani);
    }

    /// Animation names that NPCs of this level use and that are not loaded.
    pub fn wanted_npc_anis(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for n in self.current_level.iter().flat_map(|l| l.npcs.iter()) {
            if let Some(a) = &n.ani {
                if !self.npc_ganis.contains_key(a) && !v.contains(a) {
                    v.push(a.clone());
                }
            }
        }
        v
    }

    /// Advance the NPC animations by `ms`. True when any NPC now shows a
    /// different step or animation (so the scene needs drawing again).
    pub fn tick_npc_anis(&mut self, ms: u64) -> bool {
        let Some(level) = self.current_level.as_mut() else { return false };
        let mut changed = false;
        for npc in level.npcs.iter_mut() {
            let Some(name) = npc.ani.clone() else { continue };
            let Some(gani) = self.npc_ganis.get(&name) else { continue };
            let frames = gani.frame_count();
            if frames == 0 {
                continue;
            }
            npc.ani_ms += ms;
            // At most one pass through the animation per call.
            let mut steps = (npc.ani_ms / NPC_ANI_STEP_MS).min(frames as u64 + 1);
            npc.ani_ms %= NPC_ANI_STEP_MS;
            while steps > 0 {
                steps -= 1;
                let next = npc.ani_frame + 1;
                if next < frames {
                    npc.ani_frame = next;
                    changed = true;
                } else if gani.looped {
                    npc.ani_frame = 0;
                    changed = true;
                } else if let Some(back) = gani.setback_to.as_deref().and_then(ani_name) {
                    npc.ani = Some(back);
                    npc.ani_frame = 0;
                    npc.ani_ms = 0;
                    changed = true;
                    break;
                } else {
                    npc.ani_ms = 0;
                    break;
                }
            }
        }
        changed
    }
}
