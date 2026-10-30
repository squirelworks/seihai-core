//! The levels a client already holds, for `PLI_LEVELWARPMOD` (30).
//!
//! A real client that is about to link into a level it has cached sends
//! `PLI_LEVELWARPMOD` with the level's mod time instead of `PLI_LEVELWARP`
//! (0), and the server answers with only what changed: no `PLO_PLAYERWARP`
//! (14), and no board, mod time, links or signs when the mod time is still
//! current (GServer-v2 `sendLevel`; lab, 2026-10-24).
//!
//! What the real client counts as cached is only partly known. Graal_analysis
//! writes it up for 2.05 as "in memory or in its web-level cache" (the
//! `GWEBL001` disk cache); the lab only had revisits inside one session.
//! Seihai has the in-memory part: a level counts as cached after the
//! server has sent its board and its mod time (`PLO_LEVELMODTIME`, 39) and
//! the player has since left it. Nothing is kept on disk and nothing carries
//! over to another login.

use std::collections::HashMap;

/// One level as the server sent it, kept for a later cached link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedLevel {
    /// Unix time from `PLO_LEVELMODTIME` (39).
    pub mod_time: u32,
    /// The 64×64 tiles from `PLO_BOARDPACKET` (101).
    pub board: Vec<u16>,
    pub board_lines: Vec<String>,
    pub links: Vec<String>,
    pub signs: Vec<String>,
}

/// Levels by lower-case name.
#[derive(Debug, Default, Clone)]
pub struct LevelCache {
    levels: HashMap<String, CachedLevel>,
}

/// Whether two names mean the same level file. Names are not
/// case-sensitive, and a link may leave out the `.nw` the server's
/// `PLO_LEVELNAME` carries.
pub fn same_level(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    let strip = |s: &str| -> String {
        for ext in [".nw", ".graal"] {
            if let Some(t) = s.strip_suffix(ext) {
                return t.to_string();
            }
        }
        s.to_string()
    };
    a == b || strip(&a) == strip(&b)
}

impl LevelCache {
    pub fn insert(&mut self, name: &str, level: CachedLevel) {
        self.levels.insert(name.to_ascii_lowercase(), level);
    }

    /// The entry for `name` (see [`same_level`]).
    pub fn get(&self, name: &str) -> Option<&CachedLevel> {
        let key = name.to_ascii_lowercase();
        self.levels
            .get(&key)
            .or_else(|| self.levels.iter().find(|(k, _)| same_level(k, name)).map(|(_, v)| v))
    }

    /// The mod time a `PLI_LEVELWARPMOD` for `name` would carry.
    pub fn mod_time(&self, name: &str) -> Option<u32> {
        self.get(name).map(|l| l.mod_time)
    }

    pub fn len(&self) -> usize {
        self.levels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }

    /// Forget `name`, as when the server says the copy is stale or a warp
    /// failed.
    pub fn remove(&mut self, name: &str) {
        let key = name.to_ascii_lowercase();
        if self.levels.remove(&key).is_none() {
            self.levels.retain(|k, _| !same_level(k, name));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lvl(t: u32) -> CachedLevel {
        CachedLevel { mod_time: t, board: vec![0; 4096], board_lines: vec![], links: vec!["x".into()], signs: vec![] }
    }

    #[test]
    fn names_match_without_case_or_extension() {
        assert!(same_level("Lv_C.NW", "lv_c.nw"));
        assert!(same_level("lv_c", "lv_c.nw"));
        assert!(same_level("lv_c.nw", "lv_c"));
        assert!(!same_level("lv_c.nw", "lv_cc.nw"));
        assert!(!same_level("lv_c.nw", "lv_c.graal.nw"));
    }

    #[test]
    fn lookup_and_remove() {
        let mut c = LevelCache::default();
        assert!(c.is_empty() && c.mod_time("a.nw").is_none());
        c.insert("A.nw", lvl(7));
        assert_eq!(c.mod_time("a.nw"), Some(7));
        assert_eq!(c.mod_time("a"), Some(7));
        assert_eq!(c.len(), 1);
        c.insert("a.NW", lvl(9));
        assert_eq!(c.len(), 1);
        assert_eq!(c.mod_time("A.NW"), Some(9));
        c.remove("a");
        assert!(c.is_empty());
    }
}
