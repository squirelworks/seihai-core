//! The level the server sent, kept the way `seihai-mae` keeps it.
//!
//! When the server names a new level, a local copy at the server's mod time
//! stands in if there is one (a same-named file at another mod time is not
//! the server's level). Otherwise the level is built from the board the
//! server sent (`PLO_BOARDPACKET`), and the server's links, chests and signs
//! are put on it as they arrive. This is the same order as the online path
//! in `seihai-mae`'s game loop; nothing here is build-specific, since every
//! build reads these messages with the core readers.

use crate::assets::Level;
use crate::protocol::npcprops::NpcProps;
use crate::protocol::SessionState;
use crate::scripting::gs2::Script;

/// Where a stored level came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A local file at the server's mod time.
    LocalCopy,
    /// The server's board.
    Board,
}

/// What is logged when a level is stored.
#[derive(Debug, Clone, PartialEq)]
pub struct Arrival {
    pub name: String,
    pub source: Source,
    /// The player's position from the server's props, in tiles, if it was
    /// sent by the time the level was stored.
    pub start: Option<(f32, f32)>,
    /// NPCs the server sent (`PLO_NPCPROPS`) for this level.
    pub server_npcs: usize,
    /// Those NPCs' props, by NPC id, for the view to place on the level.
    pub server_npc_props: Vec<(u32, NpcProps)>,
    /// NPCs in the local level file (none for a level built from the board).
    pub file_npcs: usize,
    /// The GS2 scripts the server sent (`PLO_NPCBYTECODE`) for this level's
    /// NPCs, by NPC id, as the reader took them. They are kept, not run.
    pub scripts: Vec<(u32, Result<Script, String>)>,
}

impl Arrival {
    pub fn npcs(&self) -> usize {
        self.server_npcs + self.file_npcs
    }

    /// One log line.
    pub fn line(&self) -> String {
        let source = match self.source {
            Source::LocalCopy => "local copy at the server's mod time",
            Source::Board => "the server's board",
        };
        let start = match self.start {
            Some((x, y)) => format!("({x}, {y})"),
            None => "not sent yet".into(),
        };
        format!(
            "level {}: from {source}, start {start}, {} NPCs ({} from the server, {} in the file)",
            self.name,
            self.npcs(),
            self.server_npcs,
            self.file_npcs
        )
    }

    /// The log lines for the scripts: one per script, then its opcodes.
    pub fn script_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (id, script) in &self.scripts {
            match script {
                Ok(s) => {
                    let names: Vec<&str> = s.functions.iter().map(|f| f.name.as_str()).collect();
                    out.push(format!(
                        "npc {id}: GS2 script, {} ops, {} strings, functions [{}]",
                        s.code.len(),
                        s.strings.len(),
                        names.join(", ")
                    ));
                    out.extend(s.disassemble().lines().map(|l| format!("  {l}")));
                }
                Err(e) => out.push(format!("npc {id}: GS2 script not read: {e}")),
            }
        }
        out
    }
}

/// The current level and what is owed to it.
#[derive(Debug, Default)]
pub struct LevelStore {
    /// The level name the store last acted on, as the server gave it.
    loaded: Option<String>,
    pub level: Option<Level>,
    /// True while `level` was built from the server's board.
    pub from_board: bool,
}

impl LevelStore {
    /// Bring the store up to date with what has been read. `local` gives
    /// the local copy of a level file at the server's mod time, if any.
    /// True when a level was stored by this call.
    pub fn update(
        &mut self,
        st: &mut SessionState,
        local: impl Fn(&str, Option<u32>) -> Option<Level>,
    ) -> bool {
        let mut stored = false;
        if let Some(name) = st.current_level_name() {
            if self
                .loaded
                .as_deref()
                .is_none_or(|l| !l.eq_ignore_ascii_case(&name))
            {
                // The server sends the file name, e.g. "onlinestartlocal.nw".
                let file = if name.ends_with(".nw") {
                    name.clone()
                } else {
                    format!("{name}.nw")
                };
                self.from_board = false;
                self.level = local(&file, st.server_mod_time(&file));
                stored = self.level.is_some();
                self.loaded = Some(name);
            }
        }
        // No local copy: the tiles the server sent. On a gmap the board of
        // the player's level is kept by name; otherwise it is the one just
        // sent.
        if self.level.is_none() {
            let board = st
                .current_level_name()
                .and_then(|n| st.board_of(&n).cloned())
                .or_else(|| st.board.take());
            if let Some(tiles) = board {
                self.level = Some(Level::from_board(&tiles));
                self.from_board = true;
                stored = true;
            }
        }
        // Links, chests and signs can arrive a frame after the board.
        if self.from_board {
            if let Some(level) = self.level.as_mut() {
                if level.links.len() != st.links.len() {
                    level.links.clear();
                    level.add_server_links(&st.links);
                }
                if level.chests.len() != st.chests.iter().filter(|r| r.len() >= 3).count() {
                    level.chests.clear();
                    level.add_server_chests(&st.chests);
                }
                if level.signs.len() != st.signs.len() {
                    level.signs.clear();
                    level.add_server_signs(&st.signs);
                }
            }
        }
        stored
    }

    /// The name of the level the store last acted on.
    pub fn level_name(&self) -> Option<&str> {
        self.loaded.as_deref()
    }

    /// The stored level as it stands: the server's NPCs and the player's
    /// position can come after the board, so this is read when the server
    /// has gone quiet, not at once.
    pub fn arrival(&self, st: &SessionState) -> Option<Arrival> {
        let level = self.level.as_ref()?;
        let name = self.loaded.clone().unwrap_or_default();
        let here = |l: &Option<String>| l.as_deref().is_none_or(|l| l.eq_ignore_ascii_case(&name));
        let server_npc_props: Vec<(u32, NpcProps)> = st
            .server_npcs
            .iter()
            .filter(|(_, (l, _))| here(l))
            .map(|(&id, (_, p))| (id, p.clone()))
            .collect();
        let server_npcs = server_npc_props.len();
        let scripts = st
            .npc_bytecode
            .iter()
            .filter(|(id, _)| st.server_npcs.get(id).is_some_and(|(l, _)| here(l)))
            .map(|(&id, code)| (id, Script::parse(code)))
            .collect();
        Some(Arrival {
            name,
            source: if self.from_board {
                Source::Board
            } else {
                Source::LocalCopy
            },
            start: st.player_x.zip(st.player_y),
            server_npcs,
            server_npc_props,
            file_npcs: level.npcs.len(),
            scripts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ClientVersion;

    fn state() -> SessionState {
        SessionState::for_version(ClientVersion::V3153)
    }

    fn none(_: &str, _: Option<u32>) -> Option<Level> {
        None
    }

    #[test]
    fn nothing_before_a_level() {
        let mut st = state();
        let mut s = LevelStore::default();
        assert!(!s.update(&mut st, none));
        assert!(s.level.is_none() && s.arrival(&st).is_none());
    }

    #[test]
    fn a_board_is_stored_once_and_logged_once() {
        let mut st = state();
        st.level_name = Some("start.nw".into());
        st.player_x = Some(30.5);
        st.player_y = Some(31.0);
        let mut s = LevelStore::default();
        // The name came first; the board has not.
        assert!(!s.update(&mut st, none));
        st.board = Some(vec![0; 4096]);
        assert!(s.update(&mut st, none));
        let a = s.arrival(&st).unwrap();
        assert_eq!(a.name, "start.nw");
        assert_eq!(a.source, Source::Board);
        assert_eq!(a.start, Some((30.5, 31.0)));
        assert_eq!(a.npcs(), 0);
        assert!(s.from_board);
        assert!(!s.update(&mut st, none));
        assert!(s.level.is_some());
    }

    #[test]
    fn server_links_and_signs_join_a_board_level() {
        let mut st = state();
        st.level_name = Some("start.nw".into());
        st.board = Some(vec![0; 4096]);
        let mut s = LevelStore::default();
        s.update(&mut st, none);
        st.links.push("next.nw 0 0 1 64 61 playery".into());
        s.update(&mut st, none);
        assert_eq!(s.level.as_ref().unwrap().links.len(), 1);
    }

    #[test]
    fn a_local_copy_stands_in_for_the_board() {
        let mut st = state();
        st.level_name = Some("start".into());
        let mut s = LevelStore::default();
        assert!(s.update(&mut st, |n, _| (n == "start.nw")
            .then(|| Level::from_board(&[0; 4096]))));
        let a = s.arrival(&st).unwrap();
        assert_eq!(a.source, Source::LocalCopy);
        assert_eq!(a.start, None);
        assert!(a.line().contains("start not sent yet"));
        assert!(!s.from_board);
    }

    #[test]
    fn a_new_level_name_starts_over() {
        let mut st = state();
        st.level_name = Some("a.nw".into());
        st.board = Some(vec![0; 4096]);
        let mut s = LevelStore::default();
        assert!(s.update(&mut st, none));
        // As `PLO_LEVELNAME` does: the old board goes with the old name.
        st.level_name = Some("b.nw".into());
        st.board = None;
        assert!(!s.update(&mut st, none), "b's board has not come");
        assert!(s.level.is_none());
        st.board = Some(vec![1; 4096]);
        assert!(s.update(&mut st, none));
        assert_eq!(s.arrival(&st).unwrap().name, "b.nw");
    }
}
