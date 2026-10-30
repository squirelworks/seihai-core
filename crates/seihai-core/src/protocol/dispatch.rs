//! Packet dispatcher.
//!
//! Splits a decompressed frame body on `\n` and routes each message by
//! its GCHAR id (wire byte - 0x20). The level-entry set is handled;
//! everything else is recorded as unhandled.

use crate::gmap::{Gmap, GmapSession, LevelSlot};
use super::extras::{
    parse_ghost_icon, parse_level_mod_time, parse_map_spec, parse_signature, parse_start_message,
    parse_text_list, signature_action, MapSpec, SignatureAction,
};
use super::levelcache::{same_level, CachedLevel, LevelCache};
use super::layouts::{
    parse_board_heights, parse_board_modify, parse_board_modify2, parse_flag, parse_npc_del2, parse_npc_id_rest,
    parse_player_toggle, parse_player_warp2, parse_profile, parse_screenshot_name, parse_server_warp,
    parse_shoot, parse_showimg_relay, BoardHeights, BoardModify2, PlayerWarp2, ProfileCard, ServerMode, ServerWarp,
};
use super::profile::{CombatRules, IdSet};
use crate::combat::{self, IncomingHit};
use super::props::parse_player_props_in;
use super::npcprops::{parse_npc_props, NpcProps};
use super::version::ClientVersion;
use crate::chatlists::{BubbleTimer, ChatFilter, Identity};
use std::collections::HashMap;

/// An item the server put down or took away on the current level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ServerItem {
    Add(crate::items::FloorItem),
    Del(f32, f32),
}

/// A single logical message after frame decompression.
#[derive(Debug, Clone)]
pub struct Message {
    /// Logical id (wire byte - 0x20).
    pub id: u8,
    /// Payload bytes (excluding the id and the trailing newline).
    pub payload: Vec<u8>,
}

/// Split a decompressed body into messages.
///
/// Each message is `GCHAR(id)` + payload + `\n`. `PLO_RAWDATA` (100) is
/// `GCHAR(100)` + `GINT3 n` + `\n`, and the next `n` bytes are one message
/// (id + payload) that may contain any byte. A trailing `\n` inside those
/// `n` bytes is part of the payload; `PLO_FILE` for 2.1+ clients ends in one.
///
/// This works on one body. [`SessionState::apply_body`] keeps a partial
/// message across bodies instead.
pub fn split_messages(body: &[u8]) -> Vec<Message> {
    let mut sp = Splitter::default();
    let mut out = sp.push(body);
    out.extend(sp.finish());
    out
}

/// Read a GINT3: three GCHARs, high 7 bits first.
pub fn read_gint3(b: &[u8]) -> Option<usize> {
    if b.len() < 3 {
        return None;
    }
    Some(
        ((b[0].wrapping_sub(0x20) as usize & 0x7F) << 14)
            | ((b[1].wrapping_sub(0x20) as usize & 0x7F) << 7)
            | (b[2].wrapping_sub(0x20) as usize & 0x7F),
    )
}

/// Read a GINT4: four GCHARs, high 7 bits first.
pub fn read_gint4(b: &[u8]) -> Option<u32> {
    if b.len() < 4 {
        return None;
    }
    Some(
        b[..4]
            .iter()
            .fold(0u32, |acc, &c| (acc << 7) | (c.wrapping_sub(0x20) as u32 & 0x7F)),
    )
}

/// A weapon from `PLO_NPCWEAPONADD`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Weapon {
    pub name: String,
    /// NPCPROP_IMAGE (0), if sent.
    pub image: Option<String>,
    /// NPCPROP_SCRIPT (1): the GS1 client script, if sent.
    pub script: Option<String>,
}

/// Parse a `PLO_NPCWEAPONADD` payload: `GCHAR len` + name, then NPC props.
/// Only image (0, `GCHAR len`) and script (1, `GSHORT len`) are read;
/// parsing stops at any other prop.
pub fn parse_weapon_add(b: &[u8]) -> Option<Weapon> {
    let len = b.first()?.wrapping_sub(0x20) as usize;
    let name = latin1(b.get(1..1 + len)?);
    let mut w = Weapon { name, image: None, script: None };
    let mut i = 1 + len;
    while let Some(&id) = b.get(i) {
        match id.wrapping_sub(0x20) {
            0 => {
                let n = b.get(i + 1)?.wrapping_sub(0x20) as usize;
                w.image = Some(latin1(b.get(i + 2..i + 2 + n)?));
                i += 2 + n;
            }
            1 => {
                let n = read_gshort(b.get(i + 1..)?)? as usize;
                w.script = Some(script_text(b.get(i + 3..i + 3 + n)?));
                i += 3 + n;
            }
            _ => break,
        }
    }
    Some(w)
}

/// A weapon script from the wire. The server writes a line break as `§`:
/// the one byte 0xA7 from a Latin-1 server, the two bytes C2 A7 from one
/// that sends UTF-8. Both become `\n`, so the script keeps its lines (as a
/// single `//` comment line it would do nothing). Text that is valid UTF-8
/// is read as UTF-8, the rest as Latin-1.
pub fn script_text(b: &[u8]) -> String {
    let s = match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => latin1(b),
    };
    s.replace('\u{a7}', "\n")
}

/// Message splitter that carries a partial message from one body to the
/// next.
#[derive(Debug, Default)]
pub struct Splitter {
    /// Bytes of an unfinished line or raw block.
    buf: Vec<u8>,
    /// Length of the raw block that starts at `buf[0]`, once its
    /// `PLO_RAWDATA` header has been read.
    raw_len: Option<usize>,
}

impl Splitter {
    /// Add a body and return every message it completes.
    pub fn push(&mut self, body: &[u8]) -> Vec<Message> {
        self.buf.extend_from_slice(body);
        let mut out = Vec::new();
        let mut i = 0;
        loop {
            if let Some(n) = self.raw_len {
                if self.buf.len() - i < n {
                    break;
                }
                if n > 0 {
                    out.push(Message {
                        id: self.buf[i].wrapping_sub(0x20),
                        payload: self.buf[i + 1..i + n].to_vec(),
                    });
                }
                i += n;
                self.raw_len = None;
                continue;
            }
            let Some(nl) = self.buf[i..].iter().position(|&b| b == b'\n') else {
                break;
            };
            let line = &self.buf[i..i + nl];
            i += nl + 1;
            let Some((&id, payload)) = line.split_first() else {
                continue;
            };
            let id = id.wrapping_sub(0x20);
            if id == 100 {
                if let Some(n) = read_gint3(payload) {
                    self.raw_len = Some(n);
                }
                continue;
            }
            out.push(Message {
                id,
                payload: payload.to_vec(),
            });
        }
        self.buf.drain(..i);
        out
    }

    /// Flush a final line that had no `\n`.
    pub fn finish(&mut self) -> Vec<Message> {
        let rest = std::mem::take(&mut self.buf);
        if self.raw_len.take().is_some() {
            // A cut-off raw block (a partial file) is not a message.
            return Vec::new();
        }
        match rest.split_first() {
            Some((&id, payload)) => vec![Message {
                id: id.wrapping_sub(0x20),
                payload: payload.to_vec(),
            }],
            None => Vec::new(),
        }
    }
}

/// The sword power in a SWORDPOWER (8) byte: 0 to 9 is the power itself (the
/// default swords 1 to 4 send their number), from 10 it is power + 30, so
/// -20 to 20 with the image name following. The sender limits the power to
/// that range; so does this.
pub fn sword_power_from_wire(v: u8) -> i8 {
    if v < 10 {
        v as i8
    } else {
        (i16::from(v) - 30).clamp(-20, 20) as i8
    }
}

/// Another player seen via `PLO_OTHERPLPROPS`.
#[derive(Debug, Clone)]
pub struct OtherPlayer {
    pub id: u16,
    pub x: f32,
    pub y: f32,
    pub dir: u8,
    pub nick: Option<String>,
    /// Chat bubble text (CURCHAT, prop 12). Empty clears it.
    pub chat: Option<String>,
    /// How long the bubble has left; see [`SessionState::tick_bubbles`].
    pub chat_timer: BubbleTimer,
    /// Account name (ACCOUNTNAME, prop 34), when the server has sent it.
    pub account: Option<String>,
    /// The gmap cell (props 43/44) the player is in, once the server has
    /// said. `x` and `y` are local to it. `None`: no cell sent, the same
    /// cell as ours.
    pub cell: Option<(i32, i32)>,
    /// SPRITE (17) as received (`dir + 4 * S`), STATUS (18), ALIGNMENT (32,
    /// AP) and SWORDPOWER (8), each once the server has said. The victim's
    /// side of combat reads them.
    pub sprite: Option<u8>,
    pub status: Option<u8>,
    pub ap: Option<u8>,
    pub sword_power: Option<i8>,
    /// What the player holds over the head: the sprite CARRYSPRITE (19)
    /// names ([`crate::throwing::carry_sprite_from_wire`]); `None` empty
    /// hands.
    pub carry: Option<u16>,
    /// The pictures the player is drawn with, once the server has said:
    /// sword (8), shield (9), head (11) and body (35) images, and the GANI
    /// (10, when it is a name). See [`OtherLooks`].
    pub looks: OtherLooks,
}

/// Image names from another player's props. A power with no image of its
/// own names the default picture (`sword2.png`, `shield1.png`,
/// `head3.png`); power 0 is an empty string (nothing drawn).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OtherLooks {
    pub sword: Option<String>,
    pub shield: Option<String>,
    pub head: Option<String>,
    pub body: Option<String>,
    pub gani: Option<String>,
    /// The animation a 2.05 player names in the pre-2.1 bow slot (prop 10,
    /// `c(10 + len)` + name). The real 2.05 client sends its GANI name
    /// there (`walk`, `sword`, `hurt`; the 2026-10-30 lab decode), and
    /// GServer-v2 stores and relays it as a bow image with `.gif` added.
    /// Only the hit judge reads it; drawing a 2.05 player keeps the sprite.
    pub bow_slot_gani: Option<String>,
    /// Body colours (prop 13: skin, coat, sleeves, shoes, belt).
    pub colors: Option<[u8; 5]>,
    /// GANI attributes 1 to 5 (props 37 to 41; 1 is the hat).
    pub attrs: [Option<String>; 5],
}

impl OtherLooks {
    /// The player's animation is a swing ([`combat::is_swing_gani`]): the
    /// GANI (2.1+) or the name in the 2.05 bow slot.
    pub fn swing_gani(&self) -> bool {
        [&self.gani, &self.bow_slot_gani].into_iter().flatten().any(|g| combat::is_swing_gani(g))
    }

    fn update(&mut self, u: OtherLooks) {
        for (old, new) in [(&mut self.sword, u.sword), (&mut self.shield, u.shield), (&mut self.head, u.head), (&mut self.body, u.body), (&mut self.gani, u.gani), (&mut self.bow_slot_gani, u.bow_slot_gani)] {
            if new.is_some() {
                *old = new;
            }
        }
        if u.colors.is_some() {
            self.colors = u.colors;
        }
        for (old, new) in self.attrs.iter_mut().zip(u.attrs) {
            if new.is_some() {
                *old = new;
            }
        }
    }
}

/// The picture a sword (8) or shield (9) prop names. Above `max_plain`
/// the power is followed by the image name; at or below it the power is
/// the number of the default picture, 0 for none.
fn power_image(raw: &[u8], max_plain: u8, stem: &str) -> Option<String> {
    let v = raw.first()?.wrapping_sub(0x20);
    if v > max_plain {
        let n = usize::from(raw.get(1)?.wrapping_sub(0x20));
        Some(latin1(raw.get(2..2 + n).unwrap_or(&[])))
    } else if v == 0 {
        Some(String::new())
    } else {
        Some(format!("{stem}{v}.png"))
    }
}

/// The picture a head prop (11) names: `c(len + 100)` + name, or the
/// number of a default head under 100.
fn head_image(raw: &[u8]) -> Option<String> {
    let v = usize::from(raw.first()?.wrapping_sub(0x20));
    Some(if v >= 100 { latin1(raw.get(1..1 + (v - 100)).unwrap_or(&[])) } else { format!("head{v}.png") })
}

impl OtherPlayer {
    /// The player's position in the coordinates of the cell `own` (tiles):
    /// `x`/`y` plus 64 per cell of distance. With no cell sent, or off a
    /// gmap, the position as it is.
    pub fn position_from(&self, own: Option<(i32, i32)>) -> (f32, f32) {
        match (self.cell, own) {
            (Some(c), Some(o)) => (
                self.x + ((c.0 - o.0) * crate::gmap::CELL_TILES) as f32,
                self.y + ((c.1 - o.1) * crate::gmap::CELL_TILES) as f32,
            ),
            _ => (self.x, self.y),
        }
    }
}

/// Where a chat line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatKind {
    /// A chat bubble in the level (CURCHAT).
    Say,
    /// `PLO_TOALL`.
    Toall,
    /// `PLO_PRIVATEMESSAGE`.
    Private,
    /// `PLO_RC_ADMINMESSAGE`, or a line from Seihai itself.
    Admin,
}

/// One line in the chat history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLine {
    pub kind: ChatKind,
    pub from: String,
    pub text: String,
    /// When the line arrived, on the session's chat clock
    /// ([`SessionState::chat_clock_ms`]). The on-screen overlay fades a line
    /// by its age.
    pub born_ms: u64,
}

/// Lines kept in the chat history.
pub const CHAT_HISTORY: usize = 200;

/// Minimal session state built from the level-entry packets.
#[derive(Debug, Default)]
pub struct SessionState {
    pub level_name: Option<String>,
    pub board_lines: Vec<String>,
    pub links: Vec<String>,
    pub chests: Vec<String>,
    pub signs: Vec<String>,
    pub board_modifies: Vec<String>,
    pub baddies: Vec<String>,
    pub npcs: Vec<String>,
    /// Server NPCs by id, every prop merged, with the level they were on
    /// when their last record came (see [`super::npcprops`]).
    pub server_npcs: std::collections::BTreeMap<u32, (Option<String>, NpcProps)>,
    /// `PLO_NPCPROPS` updates in arrival order, taken by the game loop.
    pub npc_changes: Vec<(u32, NpcProps)>,
    /// `PLO_ITEMADD` (22) and `PLO_ITEMDEL` (23) in arrival order, drained
    /// by the game loop onto the floor (see [`crate::items`]).
    pub items: Vec<ServerItem>,
    pub warps: Vec<String>,
    pub player_props: Vec<u8>,
    /// Player position extracted from `PLO_PLAYERPROPS` (classic scale).
    pub player_x: Option<f32>,
    pub player_y: Option<f32>,
    pub player_dir: Option<u8>,
    pub other_props: Vec<(u8, Vec<u8>)>,
    /// Other players parsed from `PLO_OTHERPLPROPS`.
    pub others: Vec<OtherPlayer>,
    /// File names the server asked for or sent.
    pub file_requests: Vec<String>,
    /// Completed files: (name, bytes).
    pub incoming_files: Vec<(String, Vec<u8>)>,
    /// The server's mod time (Unix seconds) of a file in `incoming_files`,
    /// by lower-case name. Only builds whose `PLO_FILE` carries one
    /// (2.1+) fill it; a file the client saves gets this as its file time.
    pub incoming_mod_times: std::collections::HashMap<String, u32>,
    /// In-progress large file: (name, accumulated bytes).
    pub large_file: Option<(String, Vec<u8>)>,
    /// Size the server announced for the large file (`PLO_LARGEFILESIZE`).
    pub large_file_size: Option<u64>,
    /// `PLO_FILE` carries a 5-byte modtime for 2.1+ clients. 2.05 and
    /// older get the name and data only. Set this from the version.
    pub file_has_modtime: bool,
    /// Our own nickname (NICKNAME, prop 0, in `PLO_PLAYERPROPS`).
    pub nickname: Option<String>,
    /// Own hearts and status from `PLO_PLAYERPROPS`, each set when it
    /// arrives and taken by the game loop: MAXPOWER (1, heart containers),
    /// CURPOWER (2, hearts × 2), STATUS (18) and ALIGNMENT (32, AP).
    pub own_max_power: Option<u8>,
    pub own_cur_power: Option<u8>,
    /// GLOVEPOWER (prop 6) the server last gave for the player.
    pub own_glove_power: Option<u8>,
    /// RUPEESCOUNT (3), BOMBSCOUNT (4) and ARROWSCOUNT (5) the server last
    /// gave, taken by the game loop: the login values, and what a pickup
    /// sent as `PLI_ITEMTAKE` gives.
    pub own_rupees: Option<u32>,
    pub own_bombs: Option<u8>,
    pub own_darts: Option<u8>,
    pub own_status: Option<u8>,
    pub own_alignment: Option<u8>,
    /// Own head (11), body (35) and colours (13) from `PLO_PLAYERPROPS`,
    /// taken by the game loop. A server sends these after a `sethead`,
    /// `setbody` or `setcoat` … chat command, for one.
    pub own_head: Option<String>,
    pub own_body: Option<String>,
    pub own_colors: Option<[u8; 5]>,
    /// Own GANI attributes 1 to 5 (props 37 to 41), taken by the game loop.
    pub own_attrs: [Option<String>; 5],
    /// Read prop 10 as the pre-2.1 bow slot.
    pub bow_prop10: bool,
    /// Server→client ids this client version handles. Others are counted
    /// in `ignored` and dropped, as that client would. `None` takes all.
    pub known_plo: Option<IdSet>,
    /// Ids the own-player prop parser accepts; the first other id ends
    /// the stream. `None` takes every id Seihai can decode.
    pub own_prop_ids: Option<IdSet>,
    /// Same for other players' props.
    pub other_prop_ids: Option<IdSet>,
    /// How this build judges hits from other players. `None`: ids 40 and 46
    /// are not decoded and no swing is noticed.
    pub combat: Option<CombatRules>,
    /// Players whose latest props showed a sword-swing sprite, in the order
    /// they arrived. The game loop judges each with
    /// [`crate::offline::OfflineSession::receive_sword`] and drains it.
    pub swings: Vec<u16>,
    /// Decoded `PLO_HURTPLAYER` (40) and `PLO_HITOBJECTS` (46), drained by
    /// the game loop.
    pub hits: Vec<IncomingHit>,
    /// Players who threw what they carried (`PLO_THROWCARRIED`, 21), as
    /// they were when the message came (with their carried sprite), drained
    /// by the game loop. Their carry is cleared on receipt.
    pub throws: Vec<OtherPlayer>,
    /// Messages dropped because this version has no handler for them.
    pub ignored: HashMap<u8, usize>,
    /// Text of `PLO_DISCMESSAGE` (16). The server closes after it.
    pub disconnect_message: Option<String>,
    /// Chat history, oldest first, at most [`CHAT_HISTORY`] lines.
    pub chat_log: Vec<ChatLine>,
    /// Milliseconds since the session began, advanced by
    /// [`Self::tick_bubbles`] (one call per frame). Stamps chat lines.
    pub chat_clock_ms: u64,
    /// Ignored and muted players: their chat, bubbles and PMs are dropped.
    /// Empty by default (nothing hidden); the game loop installs the
    /// lists with [`SessionState::set_filter`].
    pub filter: ChatFilter,
    /// Who each player id was, kept after the player leaves the level so
    /// a PM from someone who has gone can still be matched.
    pub identities: HashMap<u16, Identity>,
    /// Lines and bubbles dropped by the filter.
    pub hidden: usize,
    /// Chests the server reports as opened, by tile position. The game
    /// loop moves them into the session.
    pub opened_chests: Vec<(i32, i32)>,
    /// Server messages received, for the debug overlay.
    pub messages_seen: usize,
    /// Server flags from `PLO_FLAGSET` (28). A flag without `=` has an
    /// empty value.
    pub flags: HashMap<String, String>,
    /// Weapons the player holds, by name, in the order they arrived
    /// (`PLO_NPCWEAPONADD` 33, `PLO_NPCWEAPONDEL` 34).
    pub weapons: Vec<Weapon>,
    /// Built-in weapons from `PLO_DEFAULTWEAPON` (43), by id.
    pub default_weapons: Vec<u8>,
    /// Tiles of the current level from `PLO_BOARDPACKET` (101): 64×64
    /// tile ids, row by row.
    pub board: Option<Vec<u16>>,
    /// Server world time from `PLO_NEWWORLDTIME` (42).
    pub world_time: Option<u32>,
    /// `PLO_SIGNATURE` (25) value; gs2 servers send 73.
    pub signature: Option<u8>,
    /// `PLO_LEVELMODTIME` (39): Unix time of the level file, with the level
    /// it arrived for.
    pub level_mod_time: Option<(String, u32)>,
    /// `PLO_STARTMESSAGE` (41): the server's HTML welcome page. Empty on
    /// some servers.
    pub start_message: Option<String>,
    /// `PLO_HASNPCSERVER` (44) seen: the server runs NPC scripts, so the
    /// client leaves NPC props alone.
    pub has_npc_server: bool,
    /// `PLO_STAFFGUILDS` (47): guild tags that mark staff.
    pub staff_guilds: Vec<String>,
    /// `PLO_SETACTIVELEVEL` (156): the level the following chests, baddies
    /// and NPCs belong to.
    pub active_level: Option<String>,
    /// `PLO_GHOSTICON` (174).
    pub ghost_icon: bool,
    /// `PLO_RPGWINDOW` (179): the strings of the RPG window, in order
    /// (a welcome line and a credits line on the servers seen).
    pub rpg_window: Vec<String>,
    /// `PLO_STATUSLIST` (180): player status names (Online, Away, ...).
    pub status_list: Vec<String>,
    /// `PLO_BIGMAP` (171) and `PLO_MINIMAP` (172).
    pub bigmap: Option<MapSpec>,
    pub minimap: Option<MapSpec>,
    /// `PLO_ISLEADER` (10) seen.
    pub is_leader: bool,
    /// `PLO_LISTPROCESSES` (182) requests seen. Seihai never answers with
    /// a process list.
    pub process_list_requests: usize,
    /// The last `PLO_PLAYERWARP2` (49).
    pub player_warp2: Option<PlayerWarp2>,
    /// The gmap named by the last 49, if any.
    pub gmap_name: Option<String>,
    /// The gmap the player is on, set by a 49 that names a `.gmap` and
    /// cleared by a 49 that names a level. Level data is kept per level
    /// while it is set; see [`GmapSession`].
    pub gmap: Option<GmapSession>,
    /// 186 blocks applied since the last [`Self::take_board_mods`]: the
    /// lower-case level they went to and the block.
    pub board_mods: Vec<(String, BoardModify2)>,
    /// 186 blocks dropped because no level was known for their cell.
    pub board_mods_dropped: usize,
    /// The last `PLO_PROFILE` (75).
    pub profile: Option<ProfileCard>,
    /// Level and id of the last `PLO_NPCDEL2` (150), and how many arrived.
    pub last_npc_del: Option<(String, usize)>,
    pub npc_dels: usize,
    /// `PLO_HIDENPCS` (151) seen: the client removes NPC ids 1 to 999 of the
    /// active level.
    pub hide_npcs: usize,
    /// `PLO_SAY2` (153): the last NPC sign text.
    pub say2: Option<String>,
    /// Frozen by `PLO_FREEZEPLAYER2` (154) until `PLO_UNFREEZEPLAYER` (155).
    pub frozen: bool,
    /// `PLO_UPGRADEURL` (169).
    pub upgrade_url: Option<String>,
    /// `PLO_GHOSTMODE` (170) and `PLO_GHOSTTEXT` (173).
    pub ghost_mode: bool,
    pub ghost_text: Option<String>,
    /// `PLO_SHOOT` (175) messages for a player in the active level, and
    /// `PLO_MOVE`/`PLO_MOVE2` (165, 189) and `PLO_SHOWIMGNPC` (166) counts.
    pub shoots: usize,
    pub npc_moves: usize,
    pub npc_showimgs: usize,
    /// `PLO_KINGDOMSMODE` (176) or `PLO_NEWWORLDMODE` (177).
    pub server_mode: Option<ServerMode>,
    /// `PLO_SHOWIMG` (32) relays: how many, and the last one as sender id
    /// and opaque payload. Seihai draws nothing for it.
    pub showimg_relays: usize,
    pub last_showimg: Option<(u16, Vec<u8>)>,
    /// The images each player in the level shows (id 32 decoded and
    /// applied). Only players in [`Self::others`] are kept. Nothing draws
    /// them yet.
    pub player_images: std::collections::BTreeMap<u16, super::showimg::PlayerImages>,
    /// Whether this build's id 32 carries the image-record text (read from
    /// 2.17.1, 2.22 and 2.31). Earlier builds used the four-field emote
    /// form, which is not decoded here, so their relays stay opaque.
    pub showimg_records: bool,
    /// `PLO_SERVERWARP` (178), decoded: name, host and port. The last one
    /// received; the client takes it with `take()` and reconnects.
    pub server_warp: Option<ServerWarp>,
    /// `PLO_UNKNOWN181` (181): the last player and toggle.
    pub player_toggle: Option<(u16, bool)>,
    /// `PLO_HASPROCESSRUNNING` (183) requests. Never answered.
    pub process_running_requests: usize,
    /// `PLO_TAKESCREENSHOT` (184) names the client would save. Seihai does
    /// not take them.
    pub screenshot_requests: Vec<String>,
    /// The last `PLO_BOARDHEIGHTS` (185) and `PLO_BOARDMODIFY2` (186).
    pub board_heights: Option<BoardHeights>,
    pub board_modify2: Option<BoardModify2>,
    pub unhandled: HashMap<u8, usize>,
    /// Levels the player has left, kept for `PLI_LEVELWARPMOD` (30); see
    /// [`super::levelcache`].
    pub level_cache: LevelCache,
    /// How many level entries were filled from [`Self::level_cache`] because
    /// the server's answer to a cached link left the data out.
    pub cache_restores: usize,
    /// The level a `PLI_LEVELWARPMOD` just asked for: the next level the
    /// server names, if it is this one, starts from the cached copy.
    cached_request: Option<String>,
    /// Parts of the current level that came from the cache and must be
    /// replaced, not added to, if the server sends them after all (a
    /// stale copy gets the full data).
    restored: Restored,
    splitter: Splitter,
}

/// See [`SessionState::restored`].
#[derive(Debug, Default, Clone, Copy)]
struct Restored {
    board_lines: bool,
    links: bool,
    signs: bool,
}

impl SessionState {
    /// A session that reads server traffic the way `version` does.
    pub fn for_version(version: ClientVersion) -> Self {
        let p = version.profile();
        Self {
            file_has_modtime: p.files.modtime,
            bow_prop10: p.props.bow_prop10,
            known_plo: Some(p.protocol.plo),
            own_prop_ids: Some(p.props.own),
            other_prop_ids: Some(p.props.other),
            combat: p.life.combat,
            showimg_records: {
                let all = ClientVersion::all_versions();
                let at = |v: ClientVersion| all.iter().position(|&x| x == v);
                at(version) >= at(ClientVersion::V217)
            },
            ..Default::default()
        }
    }

    /// What the real client would do with the last `PLO_SIGNATURE` (25).
    /// Seihai only reports it; it does not quit.
    pub fn signature_action(&self) -> Option<SignatureAction> {
        self.signature.map(signature_action)
    }

    /// True if this client version would act on message `id`. A raw block
    /// (101 board, 102 file) belongs to the `PLO_RAWDATA` (100) before it.
    pub fn handles(&self, id: u8) -> bool {
        match &self.known_plo {
            None => true,
            Some(set) => set.contains(id) || (matches!(id, 101 | 102) && set.contains(100)),
        }
    }

    /// Apply one message to the session.
    pub fn apply(&mut self, msg: &Message) {
        self.messages_seen += 1;
        if !self.handles(msg.id) {
            *self.ignored.entry(msg.id).or_insert(0) += 1;
            return;
        }
        match msg.id {
            0 => {
                // PLO_LEVELBOARD — rest is a board line
                if std::mem::take(&mut self.restored.board_lines) {
                    self.board_lines.clear();
                }
                if let Ok(s) = String::from_utf8(msg.payload.clone()) {
                    self.board_lines.push(s);
                }
            }
            1 => {
                // PLO_LEVELLINK
                if std::mem::take(&mut self.restored.links) {
                    self.links.clear();
                }
                if let Ok(s) = String::from_utf8(msg.payload.clone()) {
                    self.links.push(s);
                }
            }
            4 => {
                // PLO_LEVELCHEST: c opened, c x, c y [, c item, c sign].
                if let Ok(s) = String::from_utf8(msg.payload.clone()) {
                    self.chests.push(s);
                }
                if let [o, x, y, ..] = msg.payload[..] {
                    if o.wrapping_sub(0x20) == 1 {
                        let pos = (x.wrapping_sub(0x20) as i32, y.wrapping_sub(0x20) as i32);
                        self.opened_chests.push(pos);
                    }
                }
            }
            5 => {
                // PLO_LEVELSIGN
                if std::mem::take(&mut self.restored.signs) {
                    self.signs.clear();
                }
                if let Ok(s) = String::from_utf8(msg.payload.clone()) {
                    self.signs.push(s);
                }
            }
            6 => {
                // PLO_LEVELNAME
                if let Ok(s) = String::from_utf8(msg.payload.clone()) {
                    self.enter_level(s);
                }
            }
            7 => {
                // PLO_BOARDMODIFY
                if let Ok(s) = String::from_utf8(msg.payload.clone()) {
                    self.board_modifies.push(s);
                }
                if let Some(m) = parse_board_modify(&msg.payload) {
                    self.apply_board_modify(&m);
                }
            }
            8 => {
                // PLO_OTHERPLPROPS — parse position and store.
                self.other_props.push((msg.id, msg.payload.clone()));
                if let Some(u) = parse_other_player(&msg.payload, self.bow_prop10, self.other_prop_ids.as_ref()) {
                    self.apply_other(u);
                }
            }
            9 => {
                // PLO_PLAYERPROPS — extract classic position if present.
                // The props are merged by id: a later message with one prop
                // (stock beta4 ends the level frame with `Z` alone) must not
                // drop the head, colours, status and images of the first.
                self.player_props = merge_player_props(&self.player_props, &msg.payload, self.bow_prop10, self.own_prop_ids.as_ref());
                self.apply_player_props(&msg.payload);
            }
            2 => {
                // PLO_BADDYPROPS
                if let Ok(s) = String::from_utf8(msg.payload.clone()) {
                    self.baddies.push(s);
                }
            }
            3 => {
                // PLO_NPCPROPS
                if let Ok(s) = String::from_utf8(msg.payload.clone()) {
                    self.npcs.push(s);
                }
                if let Some((id, p)) = parse_npc_props(&msg.payload, self.bow_prop10) {
                    let level = self.current_level_name();
                    let e = self.server_npcs.entry(id).or_insert_with(|| (level.clone(), NpcProps::default()));
                    e.0 = level;
                    e.1.merge(&p);
                    self.npc_changes.push((id, p));
                }
            }
            14 => {
                // PLO_PLAYERWARP: c x*2, c y*2, rest = level name
                if let [x, y, level @ ..] = msg.payload.as_slice() {
                    self.player_x = Some(x.wrapping_sub(0x20) as f32 / 2.0);
                    self.player_y = Some(y.wrapping_sub(0x20) as f32 / 2.0);
                    let name = latin1(level);
                    // Pre-2.1 clients get no PLO_LEVELNAME on a warp; the
                    // level name here is the only one they see.
                    if !name.is_empty() {
                        self.enter_level(name.clone());
                    }
                    self.warps.push(name);
                }
            }
            22 => {
                // PLO_ITEMADD: x*2, y*2, item. Only floor items are put down,
                // as the 2.05 handler does.
                if let Some(it) = crate::items::parse_item_add(&msg.payload) {
                    self.items.push(ServerItem::Add(it));
                }
            }
            23 => {
                // PLO_ITEMDEL: x*2, y*2.
                if let Some((x, y)) = crate::items::parse_item_del(&msg.payload) {
                    self.items.push(ServerItem::Del(x, y));
                }
            }
            56 => {
                // PLO_DELPLAYER: GSHORT id.
                if let Some(id) = read_gshort(&msg.payload) {
                    self.others.retain(|o| o.id != id);
                    self.player_images.remove(&id);
                }
            }
            13 => {
                // PLO_TOALL: GSHORT from, GSTRING message.
                if let Some(from) = read_gshort(&msg.payload) {
                    // GSTRING; fall back to the rest of the line if the
                    // length runs past it.
                    let rest = msg.payload.get(2..).unwrap_or(&[]);
                    let text = match rest.split_first() {
                        Some((&n, t)) => t.get(..n.wrapping_sub(0x20) as usize).unwrap_or(t),
                        None => rest,
                    };
                    if self.hides(from) {
                        self.hidden += 1;
                    } else {
                        let who = self.name_of(from);
                        self.push_chat(ChatKind::Toall, who, latin1(text));
                    }
                }
            }
            37 => {
                // PLO_PRIVATEMESSAGE: GSHORT from, rest = "type","message".
                if let Some(from) = read_gshort(&msg.payload) {
                    let rest = latin1(msg.payload.get(2..).unwrap_or(&[]));
                    let fields = split_quoted(&rest);
                    let text = fields.last().cloned().unwrap_or_default();
                    if self.hides(from) {
                        self.hidden += 1;
                    } else {
                        let who = self.name_of(from);
                        self.push_chat(ChatKind::Private, who, text);
                    }
                }
            }
            35 => {
                // PLO_RC_ADMINMESSAGE: "<from>:\xa7<text>".
                let rest = latin1(&msg.payload);
                let (who, text) = match rest.split_once('\u{a7}') {
                    Some((w, t)) => (w.trim_end_matches(':').to_string(), t.to_string()),
                    None => ("Server".to_string(), rest),
                };
                self.push_chat(ChatKind::Admin, who, text);
            }
            16 => {
                // PLO_DISCMESSAGE: rest = text, `\r` = line break.
                let text = String::from_utf8_lossy(&msg.payload).replace('\r', "\n");
                self.disconnect_message = Some(text);
            }
            102 => {
                // PLO_FILE, always behind PLO_RAWDATA.
                let parsed = parse_file_packet(&msg.payload, self.file_has_modtime)
                    .or_else(|| parse_file_packet(&msg.payload, !self.file_has_modtime));
                if let Some((name, data, mod_time)) = parsed {
                    self.file_requests.push(format!("FILE:{name}"));
                    if let Some(t) = mod_time.filter(|&t| t != 0) {
                        self.incoming_mod_times.entry(name.to_ascii_lowercase()).or_insert(t);
                    }
                    match self.large_file {
                        Some((ref n, ref mut buf)) if n == &name => buf.extend_from_slice(&data),
                        _ => self.incoming_files.push((name, data)),
                    }
                }
            }
            68 => {
                // PLO_LARGEFILESTART: rest = name. PLO_FILE chunks follow.
                let name = String::from_utf8_lossy(&msg.payload).into_owned();
                self.large_file = Some((name, Vec::new()));
                self.large_file_size = None;
            }
            84 => {
                // PLO_LARGEFILESIZE: GINT5 size.
                self.large_file_size = read_gint5(&msg.payload);
            }
            69 => {
                // PLO_LARGEFILEEND: rest = name.
                if let Some((name, data)) = self.large_file.take() {
                    self.incoming_files.push((name, data));
                }
                self.large_file_size = None;
            }
            30 => {
                // PLO_FILESENDFAILED: rest = name. Before 2.1 the server
                // also uses it for "already up to date".
                let name = String::from_utf8_lossy(&msg.payload);
                self.file_requests.push(format!("FAIL:{name}"));
            }
            101 => {
                // PLO_BOARDPACKET: 64×64 little-endian u16 tile ids,
                // always behind PLO_RAWDATA.
                if msg.payload.len() >= 8192 {
                    self.board = Some(
                        msg.payload[..8192]
                            .chunks_exact(2)
                            .map(|p| u16::from_le_bytes([p[0], p[1]]) & 0x0FFF)
                            .collect(),
                    );
                    // On a gmap every level's board is kept: the player's
                    // and the neighbours' are drawn side by side.
                    if let (Some(g), Some(level), Some(b)) =
                        (self.gmap.as_mut(), self.level_name.as_ref(), self.board.as_ref())
                    {
                        g.boards.insert(level.to_ascii_lowercase(), b.clone());
                    }
                }
            }
            28 => {
                // PLO_FLAGSET: rest = `name` or `name=value`.
                let text = latin1(&msg.payload);
                let (k, v) = text.split_once('=').unwrap_or((text.as_str(), ""));
                self.flags.insert(k.to_string(), v.to_string());
            }
            33 => {
                // PLO_NPCWEAPONADD: replaces a weapon of the same name.
                if let Some(w) = parse_weapon_add(&msg.payload) {
                    match self.weapons.iter_mut().find(|o| o.name == w.name) {
                        Some(old) => *old = w,
                        None => self.weapons.push(w),
                    }
                }
            }
            34 => {
                // PLO_NPCWEAPONDEL: rest = name.
                let name = latin1(&msg.payload);
                self.weapons.retain(|w| w.name != name);
            }
            42 => {
                // PLO_NEWWORLDTIME: GINT4.
                self.world_time = read_gint4(&msg.payload);
            }
            43 => {
                // PLO_DEFAULTWEAPON: GCHAR id.
                if let Some(&c) = msg.payload.first() {
                    let id = c.wrapping_sub(0x20);
                    if !self.default_weapons.contains(&id) {
                        self.default_weapons.push(id);
                    }
                }
            }
            45 => {
                // PLO_FILEUPTODATE: rest = name (2.1+).
                let name = String::from_utf8_lossy(&msg.payload);
                self.file_requests.push(format!("UPTODATE:{name}"));
            }
            21 => {
                // PLO_THROWCARRIED: GSHORT id. The 2.05 client throws that
                // player's carried object from their position and clears
                // their carry. A player it does not know throws nothing.
                if let Some(id) = read_gshort(&msg.payload) {
                    if let Some(o) = self.others.iter_mut().find(|o| o.id == id) {
                        self.throws.push(o.clone());
                        o.carry = None;
                    }
                }
            }
            10 => self.is_leader = true,
            40 if self.combat.is_some() => {
                if let Some(h) = combat::parse_hurtplayer(&msg.payload) {
                    self.hits.push(IncomingHit::Hurt(h));
                }
            }
            46 if self.combat.is_some() => {
                if let Some(h) = combat::parse_hitobjects(&msg.payload) {
                    self.hits.push(IncomingHit::Objects(h));
                }
            }
            25 => self.signature = parse_signature(&msg.payload),
            39 => {
                if let (Some(t), Some(level)) = (parse_level_mod_time(&msg.payload), &self.level_name) {
                    self.level_mod_time = Some((level.clone(), t));
                }
            }
            41 => self.start_message = Some(parse_start_message(&msg.payload).0),
            44 => self.has_npc_server = true,
            47 => self.staff_guilds = parse_text_list(&msg.payload),
            49 => {
                // PLO_PLAYERWARP2 (2.1 test builds, 2.17+).
                if let Some(w) = parse_player_warp2(&msg.payload) {
                    self.player_x = Some(w.x);
                    self.player_y = Some(w.y);
                    if w.is_gmap() {
                        // The player is in a gmap cell. The level it is in
                        // comes with the next PLO_LEVELNAME, and the grid
                        // from the .gmap file.
                        let cell = (i32::from(w.gmap_x), i32::from(w.gmap_y));
                        self.gmap_name = Some(w.level.clone());
                        match self.gmap.as_mut() {
                            Some(g) if g.is_name(&w.level) => {
                                g.cell = cell;
                                g.awaiting_level = true;
                            }
                            _ => self.gmap = Some(GmapSession::new(w.level.clone(), cell)),
                        }
                    } else {
                        self.gmap_name = None;
                        self.gmap = None;
                        if !w.level.is_empty() {
                            self.enter_level(w.level.clone());
                        }
                        self.warps.push(w.level.clone());
                    }
                    self.player_warp2 = Some(w);
                }
            }
            75 => {
                if let Some(c) = parse_profile(&msg.payload) {
                    self.profile = Some(c);
                }
            }
            150 => {
                if let Some(d) = parse_npc_del2(&msg.payload) {
                    self.last_npc_del = Some(d);
                    self.npc_dels += 1;
                }
            }
            151 => self.hide_npcs += 1,
            153 => self.say2 = Some(latin1(&msg.payload)),
            154 => self.frozen = true,
            155 => self.frozen = false,
            165 | 189 => {
                // The client needs an id above 0.
                if parse_npc_id_rest(&msg.payload).is_some_and(|(id, _)| id > 0) {
                    self.npc_moves += 1;
                }
            }
            166 => {
                if parse_npc_id_rest(&msg.payload).is_some() {
                    self.npc_showimgs += 1;
                }
            }
            32 => {
                if let Some(r) = parse_showimg_relay(&msg.payload) {
                    self.showimg_relays += 1;
                    // The client looks the sender up in its level and
                    // ignores a player it does not have.
                    let others = &self.others;
                    self.player_images.retain(|id, _| others.iter().any(|o| o.id == *id));
                    if self.showimg_records && self.others.iter().any(|o| o.id == r.0) {
                        let up = super::showimg::parse_showimg_update(&r.1);
                        super::showimg::apply_showimg_update(self.player_images.entry(r.0).or_default(), &up);
                    }
                    self.last_showimg = Some(r);
                }
            }
            169 => self.upgrade_url = Some(latin1(&msg.payload)),
            170 => self.ghost_mode = parse_flag(&msg.payload) == Some(true),
            173 => self.ghost_text = Some(latin1(&msg.payload)),
            175 => {
                if parse_shoot(&msg.payload).is_some() {
                    self.shoots += 1;
                }
            }
            176 => self.server_mode = Some(ServerMode::Kingdoms),
            177 => self.server_mode = Some(ServerMode::NewWorld),
            178 => {
                if let Some(w) = parse_server_warp(&msg.payload) {
                    self.server_warp = Some(w);
                }
            }
            181 => {
                if let Some(t) = parse_player_toggle(&msg.payload) {
                    self.player_toggle = Some(t);
                }
            }
            183 => self.process_running_requests += 1,
            184 => {
                if let Some(name) = parse_screenshot_name(&msg.payload) {
                    if self.screenshot_requests.len() < 16 {
                        self.screenshot_requests.push(name);
                    }
                }
            }
            185 => {
                if let Some(h) = parse_board_heights(&msg.payload) {
                    self.board_heights = Some(h);
                }
            }
            186 => {
                if let Some(m) = parse_board_modify2(&msg.payload) {
                    self.apply_board_modify2(&m);
                    self.board_modify2 = Some(m);
                }
            }
            156 => {
                // The last message of a level answer: nothing more is coming
                // for a cached link, so what it left out stays as cached.
                self.restored = Restored::default();
                self.cached_request = None;
                let name = latin1(&msg.payload);
                if !name.is_empty() {
                    self.active_level = Some(name);
                }
            }
            168 => {
                // PLO_UNKNOWN168: empty; login servers send it. Nothing to do.
            }
            171 => self.bigmap = parse_map_spec(&msg.payload),
            172 => self.minimap = parse_map_spec(&msg.payload),
            174 => self.ghost_icon = parse_ghost_icon(&msg.payload),
            179 => self.rpg_window = parse_text_list(&msg.payload),
            180 => self.status_list = parse_text_list(&msg.payload),
            182 => self.process_list_requests += 1,
            _ => {
                *self.unhandled.entry(msg.id).or_insert(0) += 1;
            }
        }
    }

    /// Switch to level `name`. When it differs from the current level,
    /// drop the old level's board, links, signs and chests: the server
    /// sends the new ones after the name.
    ///
    /// On a gmap the lists are parked per level instead, and a `name` that
    /// is the gmap's own file name means "back to the player's level": the
    /// server names the gmap after each level it sends, the neighbours the
    /// client asked for included.
    fn enter_level(&mut self, name: String) {
        let Some(mut g) = self.gmap.take() else {
            if self.level_name.as_deref() != Some(name.as_str()) {
                self.keep_level_in_cache();
                self.board = None;
                self.board_lines.clear();
                self.links.clear();
                self.signs.clear();
                self.chests.clear();
                self.restored = Restored::default();
                if self.cached_request.take().is_some_and(|want| same_level(&want, &name)) {
                    self.restore_from_cache(&name);
                }
            }
            self.level_name = Some(name);
            return;
        };
        let target = if g.is_name(&name) {
            g.current_level()
        } else {
            if std::mem::take(&mut g.awaiting_level) {
                g.level_hint = Some(name.clone());
            }
            Some(name)
        };
        if let Some(target) = target {
            let same = self.level_name.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&target));
            if !same {
                if let Some(old) = self.level_name.take() {
                    g.slots.insert(
                        old.to_ascii_lowercase(),
                        LevelSlot {
                            board: self.board.take(),
                            board_lines: std::mem::take(&mut self.board_lines),
                            links: std::mem::take(&mut self.links),
                            signs: std::mem::take(&mut self.signs),
                            chests: std::mem::take(&mut self.chests),
                        },
                    );
                }
                let slot = g.slots.remove(&target.to_ascii_lowercase()).unwrap_or_default();
                self.board = slot.board;
                self.board_lines = slot.board_lines;
                self.links = slot.links;
                self.signs = slot.signs;
                self.chests = slot.chests;
            }
            self.level_name = Some(target);
        }
        self.gmap = Some(g);
    }

    /// Put the level being left into [`Self::level_cache`], when the server
    /// has sent its board and its mod time. Off a gmap only.
    fn keep_level_in_cache(&mut self) {
        let (Some(old), Some((lv, t)), Some(board)) = (&self.level_name, &self.level_mod_time, &self.board) else {
            return;
        };
        if !same_level(old, lv) {
            return;
        }
        self.level_cache.insert(
            old,
            CachedLevel {
                mod_time: *t,
                board: board.clone(),
                board_lines: self.board_lines.clone(),
                links: self.links.clone(),
                signs: self.signs.clone(),
            },
        );
    }

    /// Fill the new level from the cache (the server's answer to a
    /// `PLI_LEVELWARPMOD` leaves out what the copy already has).
    fn restore_from_cache(&mut self, name: &str) {
        let Some(c) = self.level_cache.get(name).cloned() else {
            return;
        };
        self.board = Some(c.board);
        self.board_lines = c.board_lines;
        self.links = c.links;
        self.signs = c.signs;
        self.level_mod_time = Some((name.to_string(), c.mod_time));
        self.restored = Restored { board_lines: true, links: true, signs: true };
        self.cache_restores += 1;
    }

    /// The mod time a `PLI_LEVELWARPMOD` into `dest` would carry: the level
    /// is cached, is not the one the player is in, and play is not on a
    /// gmap (a gmap keeps its own per-level slots and asks with
    /// `PLI_ADJACENTLEVEL`). `None` means ask with `PLI_LEVELWARP`.
    pub fn cached_mod_time(&self, dest: &str) -> Option<u32> {
        if self.gmap.is_some() || self.level_name.as_deref().is_some_and(|c| same_level(c, dest)) {
            return None;
        }
        self.level_cache.mod_time(dest)
    }

    /// A `PLI_LEVELWARPMOD` for `dest` went out: when the server names that
    /// level next, start from the cached copy. Call
    /// [`Self::cancel_cached_request`] if no answer comes.
    pub fn expect_cached_level(&mut self, dest: &str) {
        self.cached_request = Some(dest.to_string());
    }

    /// The cached link got no answer (or the warp failed): forget the copy,
    /// so the next link asks with a plain `PLI_LEVELWARP`.
    pub fn cancel_cached_request(&mut self) {
        if let Some(dest) = self.cached_request.take() {
            self.level_cache.remove(&dest);
        }
    }

    /// The level the player is in: on a gmap the one at the player's cell,
    /// otherwise the one the server named last.
    pub fn current_level_name(&self) -> Option<String> {
        match &self.gmap {
            Some(g) => g.current_level().or_else(|| self.level_name.clone()),
            None => self.level_name.clone(),
        }
    }

    /// The mod time the server gave for `level`: the current level's
    /// `PLO_LEVELMODTIME` (39), else the one of its cached copy.
    pub fn server_mod_time(&self, level: &str) -> Option<u32> {
        match &self.level_mod_time {
            Some((n, t)) if same_level(n, level) => Some(*t),
            _ => self.level_cache.mod_time(level),
        }
    }

    /// The 64×64 tiles the server sent for `level` (a gmap keeps them for
    /// every level it was sent; without one, only the current level's board
    /// is known).
    pub fn board_of(&self, level: &str) -> Option<&Vec<u16>> {
        let key = level.to_ascii_lowercase();
        match &self.gmap {
            Some(g) => g.boards.get(&key),
            None => (self.level_name.as_deref().map(str::to_ascii_lowercase) == Some(key))
                .then_some(self.board.as_ref())
                .flatten(),
        }
    }

    /// Move the player's cell (the client crossed an edge: the real client
    /// changes the cell at once and the server follows). No-op off a gmap.
    pub fn set_gmap_cell(&mut self, cell: (i32, i32)) {
        if let Some(g) = self.gmap.as_mut() {
            g.cell = cell;
        }
    }

    /// Hand the grid to the session (read from a local file, or from the
    /// file the server sent). Ignored off a gmap.
    pub fn set_gmap_grid(&mut self, map: Gmap) {
        if let Some(g) = self.gmap.as_mut() {
            g.map = Some(map);
        }
    }

    /// Apply a 186 block to the level its `mapX`/`mapY` cell holds. On a
    /// gmap that is the grid's level at that cell; off a gmap there is one
    /// level, cell (0, 0). A cell with no known level drops the block.
    ///
    /// The cell other than the player's own was never tested against a
    /// real client; this follows the field's name.
    fn apply_board_modify2(&mut self, m: &BoardModify2) {
        let cell = (i32::from(m.map_x), i32::from(m.map_y));
        let level = match &self.gmap {
            Some(g) => g.level_at(cell).or_else(|| (cell == g.cell).then(|| g.current_level()).flatten()),
            None => (cell == (0, 0)).then(|| self.level_name.clone()).flatten(),
        };
        let Some(level) = level else {
            self.board_mods_dropped += 1;
            return;
        };
        self.apply_board_block(&level, m);
    }

    /// Apply a `PLO_BOARDMODIFY` (7) to the player's level: the server
    /// sends it for a change another player made (a bush lifted or cut, a
    /// stone block pushed), and GServer echoes some back to the sender. The
    /// block has no cell of its own; it goes where 186 goes for the
    /// player's cell.
    fn apply_board_modify(&mut self, m: &BoardModify2) {
        let level = match &self.gmap {
            Some(g) => g.current_level(),
            None => self.level_name.clone(),
        };
        let Some(level) = level else {
            self.board_mods_dropped += 1;
            return;
        };
        self.apply_board_block(&level, m);
    }

    /// Write a board block into `level`'s board (and the live board when it
    /// is the player's level), and queue it for whoever draws the levels.
    fn apply_board_block(&mut self, level: &str, m: &BoardModify2) {
        let key = level.to_ascii_lowercase();
        let is_live = self.level_name.as_deref().map(str::to_ascii_lowercase).as_deref() == Some(key.as_str());
        if let Some(g) = self.gmap.as_mut() {
            // The level object exists on the real client before its board
            // arrives: start from an empty board.
            let board = g.boards.entry(key.clone()).or_insert_with(|| vec![0; 64 * 64]);
            m.apply_to(board);
        }
        if is_live {
            if let Some(b) = self.board.as_mut() {
                m.apply_to(b);
            }
        }
        self.board_mods.push((key, m.clone()));
    }

    /// The 186 blocks applied since the last call, for whoever draws the
    /// levels.
    pub fn take_board_mods(&mut self) -> Vec<(String, BoardModify2)> {
        std::mem::take(&mut self.board_mods)
    }

    /// Apply every message in a decompressed body.
    ///
    /// A message cut off at the end of the body (a raw block or a line
    /// without its `\n`) is kept and finished by the next body.
    pub fn apply_body(&mut self, body: &[u8]) {
        for msg in self.splitter.push(body) {
            self.apply(&msg);
        }
    }

    /// Add a line to the chat history.
    pub fn push_chat(&mut self, kind: ChatKind, from: String, text: String) {
        self.chat_log.push(ChatLine { kind, from, text, born_ms: self.chat_clock_ms });
        if self.chat_log.len() > CHAT_HISTORY {
            let extra = self.chat_log.len() - CHAT_HISTORY;
            self.chat_log.drain(..extra);
        }
    }

    /// Install the ignore and mute sets, and take down the bubbles of
    /// players they now hide.
    pub fn set_filter(&mut self, filter: ChatFilter) {
        self.filter = filter;
        for i in 0..self.others.len() {
            if self.hides(self.others[i].id) {
                let o = &mut self.others[i];
                o.chat = None;
                o.chat_timer = BubbleTimer::default();
            }
        }
    }

    /// Whether the filter hides player `id`.
    pub fn hides(&self, id: u16) -> bool {
        !self.filter.is_empty() && self.identities.get(&id).is_some_and(|w| self.filter.hides(w))
    }

    /// What is known of the player with this account name or nickname,
    /// among the players seen so far: (id, identity).
    pub fn find_identity(&self, name: &str) -> Option<(u16, &Identity)> {
        // Prefer a player who is here now.
        let here = self.others.iter().filter_map(|o| self.identities.get(&o.id).map(|w| (o.id, w)));
        let all = self.identities.iter().map(|(id, w)| (*id, w));
        here.chain(all).find(|(_, w)| w.is_named(name))
    }

    /// Whether the player named `name` (account or nickname) is in the
    /// level now, with their id.
    pub fn present_id(&self, name: &str) -> Option<u16> {
        self.others
            .iter()
            .find(|o| self.identities.get(&o.id).is_some_and(|w| w.is_named(name)))
            .map(|o| o.id)
    }

    /// Advance the chat bubbles by `dt_ms`. Returns true if one ended, so
    /// the view needs a redraw.
    pub fn tick_bubbles(&mut self, dt_ms: u32) -> bool {
        self.chat_clock_ms += u64::from(dt_ms);
        let mut ended = false;
        for o in &mut self.others {
            if o.chat_timer.tick(dt_ms) {
                o.chat = None;
                ended = true;
            }
        }
        ended
    }

    /// Nickname for a player id, or the id itself.
    fn name_of(&self, id: u16) -> String {
        self.others
            .iter()
            .find(|o| o.id == id)
            .and_then(|o| o.nick.clone())
            .unwrap_or_else(|| format!("#{id}"))
    }

    fn apply_other(&mut self, mut u: OtherUpdate) {
        if u.left {
            self.others.retain(|o| o.id != u.id);
            return;
        }
        let i = match self.others.iter().position(|o| o.id == u.id) {
            Some(i) => i,
            None => {
                self.others.push(OtherPlayer {
                    id: u.id,
                    x: 0.0,
                    y: 0.0,
                    dir: 2,
                    nick: None,
                    chat: None,
                    chat_timer: BubbleTimer::default(),
                    account: None,
                    cell: None,
                    sprite: None,
                    status: None,
                    ap: None,
                    sword_power: None,
                    carry: None,
                    looks: OtherLooks::default(),
                });
                self.others.len() - 1
            }
        };
        let o = &mut self.others[i];
        if let Some(v) = u.x {
            o.x = v;
        }
        if let Some(v) = u.y {
            o.y = v;
        }
        if let Some(v) = u.dir {
            o.dir = v;
        }
        if u.sprite.is_some() {
            o.sprite = u.sprite;
        }
        if u.status.is_some() {
            o.status = u.status;
        }
        if u.ap.is_some() {
            o.ap = u.ap;
        }
        if u.sword_power.is_some() {
            o.sword_power = u.sword_power;
        }
        if let Some(c) = u.carry {
            o.carry = c;
        }
        let moved = u.x.is_some() || u.y.is_some() || u.dir.is_some() || u.sprite.is_some() || u.looks.gani.is_some() || u.looks.bow_slot_gani.is_some();
        o.looks.update(std::mem::take(&mut u.looks));
        // A 2.05 swing is a swing sprite in the update. A 2.1+ client sends
        // GANI `sword` once and then only what changes, so any update of
        // its place or animation while that GANI is on is judged again.
        let gani_swing = moved && o.looks.swing_gani();
        let sprite_swing = u.sprite.zip(self.combat).is_some_and(|(sp, r)| combat::is_swing_sprite(&r, sp));
        let swing = self.combat.is_some() && (sprite_swing || gani_swing);
        if swing {
            self.swings.push(u.id);
        }
        let o = &mut self.others[i];
        if u.cell_x.is_some() || u.cell_y.is_some() {
            let (cx, cy) = o.cell.unwrap_or((0, 0));
            o.cell = Some((u.cell_x.unwrap_or(cx), u.cell_y.unwrap_or(cy)));
        }
        if u.nick.is_some() {
            o.nick = u.nick;
        }
        if u.account.is_some() {
            o.account = u.account;
        }
        let ident = self.identities.entry(u.id).or_default();
        ident.account = o.account.clone().or(ident.account.take());
        ident.nick = o.nick.clone().or(ident.nick.take());
        if let Some(text) = u.chat {
            if self.hides(u.id) {
                // Ignored or muted: no bubble and no history line.
                let o = &mut self.others[i];
                o.chat = None;
                o.chat_timer = BubbleTimer::default();
                if !text.is_empty() {
                    self.hidden += 1;
                }
            } else {
                let o = &mut self.others[i];
                // As in 2.05: a line shows for at least 60 frames from its
                // arrival and until the speaker's client sends an empty
                // CURCHAT, which it does when its own bubble ends
                // ([`BubbleTimer::start_floor`]).
                if text.is_empty() {
                    if o.chat.as_deref().is_some_and(|c| !c.is_empty()) && o.chat_timer.left_ms() > 0 {
                        o.chat_timer.release();
                    } else {
                        o.chat = None;
                        o.chat_timer = BubbleTimer::default();
                    }
                } else {
                    o.chat = Some(text.clone());
                    o.chat_timer = BubbleTimer::start_floor();
                }
                if !text.is_empty() {
                    let who = self.name_of(u.id);
                    self.push_chat(ChatKind::Say, who, text);
                }
            }
        }
    }

    /// Pick position and direction out of a `PLO_PLAYERPROPS` stream.
    ///
    /// X/Y are props 15/16 (GCHAR, half-tiles); direction is SPRITE
    /// (17) mod 4.
    fn apply_player_props(&mut self, payload: &[u8]) {
        for prop in parse_player_props_in(payload, self.bow_prop10, self.own_prop_ids.as_ref()) {
            match (prop.id, prop.gchar()) {
                (15, Some(v)) => self.player_x = Some(v as f32 / 2.0),
                (16, Some(v)) => self.player_y = Some(v as f32 / 2.0),
                (17, Some(v)) => self.player_dir = Some(v % 4),
                // The server moves the player's cell (GMAPLEVELX/Y).
                (43, Some(v)) => {
                    if let Some(g) = self.gmap.as_mut() {
                        g.cell.0 = i32::from(v);
                    }
                }
                (44, Some(v)) => {
                    if let Some(g) = self.gmap.as_mut() {
                        g.cell.1 = i32::from(v);
                    }
                }
                (1, Some(v)) => self.own_max_power = Some(v),
                (2, Some(v)) => self.own_cur_power = Some(v),
                (6, Some(v)) => self.own_glove_power = Some(v),
                (3, _) => self.own_rupees = read_gint3(&prop.raw).map(|v| v as u32),
                (4, Some(v)) => self.own_bombs = Some(v),
                (5, Some(v)) => self.own_darts = Some(v),
                (18, Some(v)) => self.own_status = Some(v),
                (32, Some(v)) => self.own_alignment = Some(v),
                (0, _) => self.nickname = Some(latin1(prop.raw.get(1..).unwrap_or(&[]))),
                (11, _) => self.own_head = head_image(&prop.raw),
                (35, _) => self.own_body = Some(latin1(prop.raw.get(1..).unwrap_or(&[]))).filter(|b| !b.is_empty()),
                (13, _) => self.own_colors = crate::looks::colors_from_prop(&prop.raw),
                (37..=41, _) => self.own_attrs[usize::from(prop.id - 37)] = Some(latin1(prop.raw.get(1..).unwrap_or(&[]))),
                _ => {}
            }
        }
    }

    /// Take every completed file, with the server's mod time where the
    /// build sends one. The gmap's own file gives the grid on the way.
    pub fn take_files(&mut self) -> Vec<IncomingFile> {
        let mut out = Vec::new();
        for (name, data) in self.incoming_files.drain(..) {
            if let Some(g) = self.gmap.as_mut().filter(|g| g.is_name(&name)) {
                if let Some(map) = Gmap::parse(&String::from_utf8_lossy(&data)) {
                    g.map = Some(map);
                }
            }
            let mod_time = self.incoming_mod_times.remove(&name.to_ascii_lowercase());
            out.push(IncomingFile { name, data, mod_time });
        }
        out
    }

    /// Write every completed file into `dir` and clear the list.
    /// Returns the names written. (The client saves by the real clients'
    /// folder rules instead: see `filecache`.)
    pub fn flush_files(&mut self, dir: &std::path::Path) -> Vec<String> {
        let mut written = Vec::new();
        for f in self.take_files() {
            let path = dir.join(&f.name);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::write(&path, &f.data).is_ok() {
                written.push(f.name);
            }
        }
        written
    }
}

/// A file the server finished sending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingFile {
    pub name: String,
    pub data: Vec<u8>,
    /// The server's mod time (Unix seconds); `None` before 2.1.
    pub mod_time: Option<u32>,
}

/// Read a GSHORT: two GCHARs, high 7 bits first.
pub fn read_gshort(b: &[u8]) -> Option<u16> {
    let (hi, lo) = (*b.first()?, *b.get(1)?);
    Some(((hi.wrapping_sub(0x20) as u16 & 0x7F) << 7) | (lo.wrapping_sub(0x20) as u16 & 0x7F))
}

/// The props one `PLO_OTHERPLPROPS` carried. The server sends only the
/// props that changed, so anything absent keeps its old value.
#[derive(Debug, Default)]
struct OtherUpdate {
    id: u16,
    x: Option<f32>,
    y: Option<f32>,
    dir: Option<u8>,
    nick: Option<String>,
    account: Option<String>,
    chat: Option<String>,
    /// GMAPLEVELX (43) and GMAPLEVELY (44).
    cell_x: Option<i32>,
    cell_y: Option<i32>,
    sprite: Option<u8>,
    status: Option<u8>,
    ap: Option<u8>,
    sword_power: Option<i8>,
    /// CARRYSPRITE (19): the carried sprite, `Some(None)` empty hands.
    carry: Option<Option<u16>>,
    looks: OtherLooks,
    /// PCONNECTED (51) or JOINLEAVELVL (50) = 0: the player left.
    left: bool,
}

/// Parse one `PLO_OTHERPLPROPS` payload: `GSHORT id` + prop stream.
fn parse_other_player(payload: &[u8], bow_prop10: bool, accepted: Option<&IdSet>) -> Option<OtherUpdate> {
    let id = read_gshort(payload)?;
    let mut u = OtherUpdate { id, ..Default::default() };
    let text = |raw: &[u8]| latin1(raw.get(1..).unwrap_or(&[]));
    for prop in parse_player_props_in(&payload[2..], bow_prop10, accepted) {
        match prop.id {
            0 => u.nick = Some(text(&prop.raw)),
            34 => u.account = Some(text(&prop.raw)).filter(|a| !a.is_empty()),
            12 => u.chat = Some(text(&prop.raw)),
            15 => u.x = prop.gchar().map(|v| v as f32 / 2.0),
            16 => u.y = prop.gchar().map(|v| v as f32 / 2.0),
            17 => {
                u.dir = prop.gchar().map(|v| v % 4);
                u.sprite = prop.gchar();
            }
            8 => {
                u.sword_power = prop.gchar().map(sword_power_from_wire);
                u.looks.sword = power_image(&prop.raw, 4, "sword");
            }
            9 => u.looks.shield = power_image(&prop.raw, 3, "shield"),
            10 if !bow_prop10 => u.looks.gani = Some(text(&prop.raw)).filter(|g| !g.is_empty()),
            // The bow slot: a byte under 10 is a bow power (no name).
            // A 2.x server's GANI string in the bow slot
            // (`props::bow_slot_holds_gani`) names it the same way.
            10 => {
                u.looks.bow_slot_gani =
                    (super::props::bow_slot_holds_gani(&prop.raw) || prop.gchar().is_some_and(|v| v >= 10)).then(|| text(&prop.raw))
            }
            11 => u.looks.head = head_image(&prop.raw),
            35 => u.looks.body = Some(text(&prop.raw)).filter(|b| !b.is_empty()),
            13 => u.looks.colors = crate::looks::colors_from_prop(&prop.raw),
            37..=41 => u.looks.attrs[usize::from(prop.id - 37)] = Some(text(&prop.raw)),
            18 => u.status = prop.gchar(),
            19 => u.carry = prop.raw.first().map(|&b| crate::throwing::carry_sprite_from_wire(crate::throwing::carry_code_from_byte(b))),
            32 => u.ap = prop.gchar(),
            43 => u.cell_x = prop.gchar().map(i32::from),
            44 => u.cell_y = prop.gchar().map(i32::from),
            50 => u.left |= prop.gchar() == Some(0),
            51 => u.left = true,
            // X2/Y2: precise position, GSHORT.
            78 => u.x = read_gshort(&prop.raw).map(super::props::decode_pos_precise),
            79 => u.y = read_gshort(&prop.raw).map(super::props::decode_pos_precise),
            _ => {}
        }
    }
    Some(u)
}

/// Graal text is single-byte (Windows code page); map each byte to the
/// same code point, which is exact for Latin-1.
pub fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

/// Split a `"a","b",c` list, as GServer's gtokenize writes it.
pub(super) fn split_quoted(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() || s.ends_with(',') {
        out.push(cur);
    }
    out
}

/// Split comma text the way the clients' own list class does (the
/// `CommaText` setter of 2.17.1, 2.22 and 2.31; read from the three
/// binaries, 2026-10-25):
///
/// - Leading white space (bytes 1 to 0x20) is skipped before each item.
/// - An item that starts with `"` is quoted: it runs to the closing `"`,
///   a doubled `""` is one quote, and an unclosed quote runs to the end.
/// - Any other item runs up to the next byte of 0x20 or less or `,`. So
///   white space ends an item, and the text after it (if it is not a
///   comma) starts the next item: `a b,c` is three items.
/// - After an item, white space is skipped, then one comma, then white
///   space again. A comma that ends the text adds no empty item; `a,,b`
///   keeps the empty one in the middle.
/// - A NUL byte ends the text.
///
/// This differs from [`split_quoted`], which keeps spaces inside an item.
pub(super) fn delphi_comma_text(s: &[u8]) -> Vec<Vec<u8>> {
    let ws = |c: u8| (1..=0x20).contains(&c);
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    let s = &s[..end];
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() && ws(s[i]) {
        i += 1;
    }
    while i < s.len() {
        let mut item = Vec::new();
        if s[i] == b'"' {
            i += 1;
            while i < s.len() {
                if s[i] == b'"' {
                    if s.get(i + 1) == Some(&b'"') {
                        item.push(b'"');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                item.push(s[i]);
                i += 1;
            }
        } else {
            let start = i;
            while i < s.len() && s[i] > 0x20 && s[i] != b',' {
                i += 1;
            }
            item.extend_from_slice(&s[start..i]);
        }
        out.push(item);
        while i < s.len() && ws(s[i]) {
            i += 1;
        }
        if s.get(i) == Some(&b',') {
            i += 1;
            while i < s.len() && ws(s[i]) {
                i += 1;
            }
        }
    }
    out
}

/// Read a GINT5: five GCHARs, high 7 bits first.
pub(super) fn read_gint5(b: &[u8]) -> Option<u64> {
    if b.len() < 5 {
        return None;
    }
    Some(
        b[..5]
            .iter()
            .fold(0u64, |acc, &c| (acc << 7) | (c.wrapping_sub(0x20) as u64 & 0x7F)),
    )
}

/// Parse a `PLO_FILE` payload (the bytes after the id).
///
/// - 2.1+: `GINT5 modtime` + `GCHAR name_len` + name + data + `\n`
/// - before 2.1 (2.05): `GCHAR name_len` + name + data, with no `\n`
fn parse_file_packet(payload: &[u8], has_modtime: bool) -> Option<(String, Vec<u8>, Option<u32>)> {
    let mod_time = if has_modtime { read_gint5(payload).map(|t| t.min(u64::from(u32::MAX)) as u32) } else { None };
    let p = if has_modtime { payload.get(5..)? } else { payload };
    let (&len, rest) = p.split_first()?;
    let name_len = len.wrapping_sub(0x20) as usize;
    if rest.len() < name_len {
        return None;
    }
    // GServer decides by its own version table whether the modtime is
    // there, so a name that is not a plain file name means the other
    // layout.
    let name_bytes = &rest[..name_len];
    let plain = |b: &u8| b.is_ascii_graphic() || *b == b' ';
    if name_len == 0 || !name_bytes.iter().all(plain) || !name_bytes.contains(&b'.') {
        return None;
    }
    let name = String::from_utf8_lossy(name_bytes).into_owned();
    let mut data = &rest[name_len..];
    if has_modtime {
        data = data.strip_suffix(b"\n").unwrap_or(data);
    }
    Some((name, data.to_vec(), mod_time))
}

/// `old` with the props of `new` put in by id: a prop in `new` replaces the
/// one of the same id, the others are kept, in id order. The real 2.22 and
/// 2.31 answered a full `PLO_PLAYERPROPS` followed by one with `Z` alone with
/// the head, colours and status of the first (lab, 2026-10-30, stock beta4).
/// Only the props this build reads are kept from `old` (the parse stops at
/// the first id it does not know). When `new` does not parse to its end, it
/// replaces `old` whole, as before.
pub fn merge_player_props(old: &[u8], new: &[u8], bow_prop10: bool, accepted: Option<&IdSet>) -> Vec<u8> {
    let new_props = parse_player_props_in(new, bow_prop10, accepted);
    if new_props.iter().map(|p| 1 + p.raw.len()).sum::<usize>() != new.len() {
        return new.to_vec();
    }
    let old_props = parse_player_props_in(old, bow_prop10, accepted);
    let mut merged: std::collections::BTreeMap<u8, Vec<u8>> = old_props.into_iter().map(|p| (p.id, p.raw)).collect();
    merged.extend(new_props.into_iter().map(|p| (p.id, p.raw)));
    let mut out = Vec::new();
    for (id, raw) in merged {
        out.push(id + 0x20);
        out.extend(raw);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carrysprite_19_names_what_another_player_holds() {
        let mut u = parse_other_player(&[0x20, 0x22, 19 + 0x20, 1 + 0x20], false, None).unwrap();
        assert_eq!(u.carry, Some(Some(261)), "1 is the bush");
        u = parse_other_player(&[0x20, 0x22, 19 + 0x20, 31], false, None).unwrap();
        assert_eq!(u.carry, Some(None), "-1: empty hands");
        u = parse_other_player(&[0x20, 0x22, 19 + 0x20, 201 + 0x20], false, None).unwrap();
        assert_eq!(u.carry, Some(Some(239)));
        u = parse_other_player(&[0x20, 0x22, 15 + 0x20, 0x40], false, None).unwrap();
        assert_eq!(u.carry, None, "no prop 19: unchanged");
    }

    #[test]
    fn sword_shield_head_body_and_gani_props_name_the_pictures() {
        let g = |v: u8| v + 0x20;
        let mut p = vec![0x20, 0x22];
        p.extend([g(8), g(33), g(9)]);
        p.extend(b"sabre.png");
        p.extend([g(9), g(2), g(11), g(107)]);
        p.extend(b"hat.png");
        p.extend([g(35), g(7)]);
        p.extend(b"red.png");
        p.extend([g(10), g(4)]);
        p.extend(b"walk");
        let u = parse_other_player(&p, false, None).unwrap();
        assert_eq!(u.sword_power, Some(3));
        let want = |s: &str| Some(s.to_string());
        assert_eq!(
            u.looks,
            OtherLooks { sword: want("sabre.png"), shield: want("shield2.png"), head: want("hat.png"), body: want("red.png"), gani: want("walk"), bow_slot_gani: None, colors: None, attrs: Default::default() }
        );
        // Plain powers name the default pictures; 0 is none.
        let u = parse_other_player(&[0x20, 0x22, g(8), g(2), g(9), g(0), g(11), g(4)], false, None).unwrap();
        assert_eq!((u.looks.sword, u.looks.shield, u.looks.head), (want("sword2.png"), want(""), want("head4.png")));
        // A later update without the props keeps them.
        let mut st = SessionState::default();
        st.apply_other(parse_other_player(&p, false, None).unwrap());
        st.apply_other(parse_other_player(&[0x20, 0x22, g(15), g(20)], false, None).unwrap());
        assert_eq!(st.others[0].looks.shield.as_deref(), Some("shield2.png"));
    }

    #[test]
    fn split_and_route() {
        // wire: id 6 (LEVELNAME) = 0x26, payload "start", newline
        // wire: id 0 (LEVELBOARD) = 0x20, payload "BOARD 0 ...", newline
        let body = b"&start\n BOARD 0\n";
        let msgs = split_messages(body);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].id, 6);
        assert_eq!(msgs[1].id, 0);

        let mut state = SessionState::default();
        state.apply_body(body);
        assert_eq!(state.level_name.as_deref(), Some("start"));
        assert_eq!(state.board_lines.len(), 1);
    }

    /// GServer's 2.05 file reply: `PLO_RAWDATA` + GINT3 length + `\n`,
    /// then `PLO_FILE` + GCHAR name length + name + data, with no `\n`.
    fn raw_file_205(name: &str, data: &[u8]) -> Vec<u8> {
        let n = 1 + 1 + name.len() + data.len();
        let mut out = vec![100 + 0x20];
        out.extend([((n >> 14) & 0x7F) as u8 + 0x20, ((n >> 7) & 0x7F) as u8 + 0x20, (n & 0x7F) as u8 + 0x20]);
        out.push(b'\n');
        out.push(102 + 0x20);
        out.push(name.len() as u8 + 0x20);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn gint3_matches_lab_board_header() {
        // Lab 2.22 board packet: RAWDATA length 8194 = 1 + 64*64*2 + 1.
        assert_eq!(read_gint3(b" `\""), Some(8194));
    }

    #[test]
    fn rawdata_file_205() {
        // Data holds a \n and ends without one; the next message follows.
        let data = b"GLEVNW01\nBOARD 0 0 64 0 aa";
        let mut body = raw_file_205("new1.nw", data);
        body.extend_from_slice(b"&new1.nw\n");
        let msgs = split_messages(&body);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].id, 102);
        assert_eq!(msgs[1].id, 6);

        let mut state = SessionState::default();
        state.apply_body(&body);
        assert_eq!(state.incoming_files, vec![("new1.nw".to_string(), data.to_vec())]);
        assert_eq!(state.level_name.as_deref(), Some("new1.nw"));
    }

    #[test]
    fn rawdata_file_with_modtime_split_across_frames() {
        // 2.1+ layout: GINT5 modtime, GCHAR name length, name, data, \n.
        let name = "a.png";
        let data: Vec<u8> = (0..=255u8).collect();
        let mut file = vec![102 + 0x20];
        file.extend_from_slice(b"  !\"#");
        file.push(name.len() as u8 + 0x20);
        file.extend_from_slice(name.as_bytes());
        file.extend_from_slice(&data);
        file.push(b'\n');
        let n = file.len();
        let mut body = vec![100 + 0x20, ((n >> 14) as u8) + 0x20, ((n >> 7) & 0x7F) as u8 + 0x20, (n & 0x7F) as u8 + 0x20, b'\n'];
        body.extend_from_slice(&file);
        body.extend_from_slice(b"&a\n");

        let mut state = SessionState::default();
        state.file_has_modtime = true;
        let (one, two) = body.split_at(40);
        state.apply_body(one);
        assert!(state.incoming_files.is_empty());
        state.apply_body(two);
        assert_eq!(state.incoming_files, vec![(name.to_string(), data)]);
        assert_eq!(state.level_name.as_deref(), Some("a"));
    }

    #[test]
    fn the_servers_file_time_comes_out_with_the_file() {
        // GINT5 bytes 0,0,1,2,3 are 1*128*128 + 2*128 + 3.
        let name = "a.png";
        let mut file = vec![102 + 0x20];
        file.extend_from_slice(b"  !\"#");
        file.push(name.len() as u8 + 0x20);
        file.extend_from_slice(name.as_bytes());
        file.extend_from_slice(b"xyz\n");
        let n = file.len();
        let mut body = vec![100 + 0x20, 0x20, (n >> 7) as u8 + 0x20, (n & 0x7F) as u8 + 0x20, b'\n'];
        body.extend_from_slice(&file);
        let mut st = SessionState::for_version(ClientVersion::V222);
        st.apply_body(&body);
        let got = st.take_files();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].name.as_str(), got[0].data.as_slice(), got[0].mod_time), (name, &b"xyz"[..], Some(16643)));
        assert!(st.take_files().is_empty());
    }

    #[test]
    fn file_layout_follows_the_server() {
        // A 2.1+ reply while the client expected the 2.05 layout.
        let name = "new1.nw";
        let mut file = vec![102 + 0x20];
        file.extend_from_slice(b"  !\"#");
        file.push(name.len() as u8 + 0x20);
        file.extend_from_slice(name.as_bytes());
        file.extend_from_slice(b"GLEVNW01\n");
        file.push(b'\n');
        let n = file.len();
        let mut body = vec![100 + 0x20, 0x20, (n >> 7) as u8 + 0x20, (n & 0x7F) as u8 + 0x20, b'\n'];
        body.extend_from_slice(&file);
        let mut st = SessionState::default();
        st.apply_body(&body);
        assert_eq!(st.incoming_files, vec![(name.to_string(), b"GLEVNW01\n".to_vec())]);
    }

    #[test]
    fn large_file_chunks_join() {
        let mut body = b"dbig.nw\n".to_vec();
        body.extend_from_slice(&raw_file_205("big.nw", b"one,"));
        body.extend_from_slice(&raw_file_205("big.nw", b"two"));
        body.extend_from_slice(b"ebig.nw\n");
        let mut state = SessionState::default();
        state.apply_body(&body);
        assert_eq!(state.incoming_files, vec![("big.nw".to_string(), b"one,two".to_vec())]);
    }

    #[test]
    fn disconnect_message_is_kept() {
        let mut state = SessionState::default();
        state.apply_body(b"0Your client version is not allowed.\rAllowed: 2.22\n");
        assert_eq!(
            state.disconnect_message.as_deref(),
            Some("Your client version is not allowed.\nAllowed: 2.22")
        );
    }

    #[test]
    fn board_packet_behind_rawdata() {
        let n = 1 + 8192 + 1;
        let mut body = vec![
            100 + 0x20,
            ((n >> 14) & 0x7F) as u8 + 0x20,
            ((n >> 7) & 0x7F) as u8 + 0x20,
            (n & 0x7F) as u8 + 0x20,
            b'\n',
            101 + 0x20,
        ];
        for i in 0..4096u16 {
            body.extend_from_slice(&(i % 512).to_le_bytes());
        }
        body.push(b'\n');
        let mut state = SessionState::default();
        state.apply_body(&body);
        let board = state.board.expect("board");
        assert_eq!(board.len(), 4096);
        assert_eq!((board[0], board[1], board[600]), (0, 1, 88));
        assert!(state.unhandled.is_empty());
    }

    /// A level answer as GServer-v2 sends it for a full level: name, board,
    /// mod time, a link and a sign, then the active level.
    fn full_answer(name: &str, t: u32, seed: u16, link: &str, sign: &str) -> Vec<u8> {
        answer(name, Some(t), seed, link, sign)
    }

    fn answer(name: &str, t: Option<u32>, seed: u16, link: &str, sign: &str) -> Vec<u8> {
        let n = 1 + 8192 + 1;
        let mut body = vec![
            100 + 0x20,
            ((n >> 14) & 0x7F) as u8 + 0x20,
            ((n >> 7) & 0x7F) as u8 + 0x20,
            (n & 0x7F) as u8 + 0x20,
            b'\n',
            101 + 0x20,
        ];
        for i in 0..4096u16 {
            body.extend_from_slice(&((i + seed) % 512).to_le_bytes());
        }
        body.push(b'\n');
        let gint5: Vec<u8> = t.map_or(Vec::new(), |t| {
            msg(39, &[28, 21, 14, 7, 0].iter().map(|s| ((t >> s) as u8 & 0x7F) + 0x20).collect::<Vec<u8>>())
        });
        [
            msg(6, name.as_bytes()),
            body,
            gint5,
            msg(1, link.as_bytes()),
            msg(5, sign.as_bytes()),
            msg(156, name.as_bytes()),
        ]
        .concat()
    }

    /// What the server sends for a cached link whose mod time is current:
    /// the name and the closing messages, no 14, no board, no mod time, no
    /// links, no signs (lab, 2.31).
    fn cached_answer(name: &str) -> Vec<u8> {
        [msg(6, name.as_bytes()), msg(174, &[0x20]), msg(156, name.as_bytes())].concat()
    }

    /// A level the player left is cached with its mod time; a cached link's
    /// answer without data restores board, links and signs from it.
    #[test]
    fn cached_answer_restores_the_level() {
        let mut st = SessionState::for_version(ClientVersion::V231);
        st.apply_body(&full_answer("a.nw", 1_000, 0, "b.nw 0 0 2 2 1 1", "sign a"));
        assert!(st.cached_mod_time("b.nw").is_none(), "nothing cached yet");
        st.apply_body(&full_answer("b.nw", 2_000, 3, "a.nw 0 0 2 2 1 1", "sign b"));
        assert_eq!(st.level_cache.len(), 1, "a was kept when the player left it");
        assert_eq!(st.cached_mod_time("a.nw"), Some(1_000));
        assert_eq!(st.cached_mod_time("A.NW"), Some(1_000));
        assert_eq!(st.cached_mod_time("a"), Some(1_000));
        assert!(st.cached_mod_time("b.nw").is_none(), "the level the player is in is not asked for");
        // Link back to a with the copy.
        st.expect_cached_level("a.nw");
        st.apply_body(&cached_answer("a.nw"));
        assert_eq!(st.level_name.as_deref(), Some("a.nw"));
        assert_eq!(st.cache_restores, 1);
        assert_eq!(st.links, vec!["b.nw 0 0 2 2 1 1".to_string()]);
        assert_eq!(st.signs, vec!["sign a".to_string()]);
        let board = st.board.as_ref().expect("board from the cache");
        assert_eq!((board[0], board[1]), (0, 1));
        assert_eq!(st.level_mod_time, Some(("a.nw".to_string(), 1_000)));
        // b is cached now, and a (the player is in it) is not offered.
        assert_eq!(st.cached_mod_time("b.nw"), Some(2_000));
        assert!(st.cached_mod_time("a.nw").is_none());
    }

    /// The server's mod time for a level: the current one's 39, else the
    /// cached copy's; none for a level the server has not timed.
    #[test]
    fn server_mod_time_of_current_and_cached_levels() {
        let mut st = SessionState::for_version(ClientVersion::V231);
        assert_eq!(st.server_mod_time("a.nw"), None);
        st.apply_body(&full_answer("a.nw", 1_000, 0, "b.nw 0 0 2 2 1 1", "sign a"));
        assert_eq!(st.server_mod_time("a.nw"), Some(1_000));
        assert_eq!(st.server_mod_time("A"), Some(1_000));
        st.apply_body(&full_answer("b.nw", 2_000, 3, "a.nw 0 0 2 2 1 1", "sign b"));
        assert_eq!(st.server_mod_time("b.nw"), Some(2_000));
        assert_eq!(st.server_mod_time("a.nw"), Some(1_000), "from the cache");
        assert_eq!(st.server_mod_time("new1.nw"), None);
    }

    /// The copy was stale: the server sends the whole level after all. The
    /// new data replaces the cached links and signs instead of adding to them.
    #[test]
    fn full_answer_to_a_cached_link_replaces_the_copy() {
        let mut st = SessionState::for_version(ClientVersion::V231);
        st.apply_body(&full_answer("a.nw", 1_000, 0, "old 0 0 2 2 1 1", "old sign"));
        st.apply_body(&full_answer("b.nw", 2_000, 3, "x", "y"));
        st.expect_cached_level("a.nw");
        st.apply_body(&full_answer("a.nw", 1_500, 7, "new 0 0 2 2 1 1", "new sign"));
        assert_eq!(st.links, vec!["new 0 0 2 2 1 1".to_string()]);
        assert_eq!(st.signs, vec!["new sign".to_string()]);
        assert_eq!(st.level_mod_time, Some(("a.nw".to_string(), 1_500)));
        assert_eq!(st.board.as_ref().map(|b| b[0]), Some(7));
        // Leaving a again caches the new copy.
        st.apply_body(&full_answer("b.nw", 2_000, 3, "x", "y"));
        assert_eq!(st.cached_mod_time("a.nw"), Some(1_500));
    }

    /// Without a request armed, a level the player has been in is entered
    /// from the server's data only, and a request nobody answers is dropped
    /// with its copy.
    #[test]
    fn restore_needs_a_request() {
        let mut st = SessionState::for_version(ClientVersion::V231);
        st.apply_body(&full_answer("a.nw", 1_000, 0, "l", "s"));
        st.apply_body(&full_answer("b.nw", 2_000, 3, "x", "y"));
        // Server-side warp back to a, no request: nothing is restored.
        st.apply_body(&cached_answer("a.nw"));
        assert_eq!(st.cache_restores, 0);
        assert!(st.links.is_empty() && st.board.is_none());
        // A request that is never answered: cancelling forgets the copy.
        let mut st = SessionState::for_version(ClientVersion::V231);
        st.apply_body(&full_answer("a.nw", 1_000, 0, "l", "s"));
        st.apply_body(&full_answer("b.nw", 2_000, 3, "x", "y"));
        st.expect_cached_level("a.nw");
        st.cancel_cached_request();
        assert!(st.cached_mod_time("a.nw").is_none());
        // A request for one level answered with another restores nothing.
        st.apply_body(&full_answer("a.nw", 1_000, 0, "l", "s"));
        st.apply_body(&full_answer("c.nw", 3_000, 5, "z", "z"));
        st.expect_cached_level("a.nw");
        st.apply_body(&cached_answer("c.nw"));
        assert_eq!(st.cache_restores, 0);
    }

    /// A level whose board or mod time never arrived is not cached.
    #[test]
    fn incomplete_levels_are_not_cached() {
        let mut st = SessionState::for_version(ClientVersion::V231);
        st.apply_body(&msg(6, b"a.nw"));
        st.apply_body(&msg(6, b"b.nw"));
        assert!(st.level_cache.is_empty());
        // Board but no mod time.
        let mut st = SessionState::for_version(ClientVersion::V231);
        let a = answer("a.nw", None, 0, "l", "s");
        st.apply_body(&a);
        st.apply_body(&msg(6, b"b.nw"));
        assert!(st.level_cache.is_empty());
    }

    #[test]
    fn warp_to_another_level_resets_level_lists() {
        let mut state = SessionState::default();
        state.apply_body(b"&onlinestartlocal.nw\n!new1.nw 26 32 12 4 20 12.5\n");
        assert_eq!(state.links.len(), 1);
        // A pre-2.1 warp: PLO_PLAYERWARP carries the only level name.
        state.apply_body(b".HYnew1.nw\n");
        assert_eq!(state.level_name.as_deref(), Some("new1.nw"));
        assert_eq!((state.player_x, state.player_y), (Some(20.0), Some(28.5)));
        assert!(state.links.is_empty());
        // The same name again (2.1+ sends both) keeps what followed it.
        state.apply_body(b"!a 0 0 1 1 1 1\n&new1.nw\n");
        assert_eq!(state.links.len(), 1);
    }

    #[test]
    fn flags_weapons_and_world_time() {
        let mut state = SessionState::default();
        // PLO_FLAGSET (28) with and without a value.
        state.apply_body(b"<gr.fileerror=none\n<quest1\n");
        assert_eq!(state.flags.get("gr.fileerror").map(String::as_str), Some("none"));
        assert_eq!(state.flags.get("quest1").map(String::as_str), Some(""));
        // PLO_NPCWEAPONADD (33): name, NPCPROP_IMAGE, NPCPROP_SCRIPT.
        let mut add = vec![b'A', 0x20 + 3];
        add.extend_from_slice(b"Bow");
        add.extend_from_slice(&[0x20, 0x20 + 7]);
        add.extend_from_slice(b"bow.png");
        add.push(0x21);
        add.extend_from_slice(&gs(2));
        add.extend_from_slice(b"//\n");
        state.apply_body(&add);
        assert_eq!(
            state.weapons,
            vec![Weapon { name: "Bow".into(), image: Some("bow.png".into()), script: Some("//".into()) }]
        );
        // PLO_DEFAULTWEAPON (43) and PLO_NEWWORLDTIME (42).
        state.apply_body(b"K!\nK!\n");
        assert_eq!(state.default_weapons, vec![1]);
        let mut t = vec![b'J'];
        t.extend_from_slice(&[0x20, 0x20, 0x21, 0x20 + 5]);
        t.push(b'\n');
        state.apply_body(&t);
        assert_eq!(state.world_time, Some(128 + 5));
        // PLO_NPCWEAPONDEL (34).
        state.apply_body(b"BBow\n");
        assert!(state.weapons.is_empty());
        assert!(state.unhandled.is_empty());
    }

    fn gs(v: u16) -> [u8; 2] {
        [((v >> 7) as u8) + 0x20, (v & 0x7F) as u8 + 0x20]
    }

    /// `PLO_OTHERPLPROPS` for player `id` with a prop stream.
    fn other(id: u16, props: &[u8]) -> Vec<u8> {
        [&[8 + 0x20][..], &gs(id), props, b"\n"].concat()
    }

    #[test]
    fn throwcarried_21_throws_what_that_player_held_and_empties_their_hands() {
        let mut st = SessionState::default();
        // Player 200 at (30, 12) facing east, holding the bush.
        st.apply_body(&other(200, &[15 + 0x20, 60 + 0x20, 16 + 0x20, 24 + 0x20, 17 + 0x20, 3 + 0x20, 19 + 0x20, 1 + 0x20]));
        assert_eq!(st.others[0].carry, Some(261));
        st.apply_body(&[&[21 + 0x20][..], &gs(200), b"\n"].concat());
        assert_eq!(st.throws.len(), 1);
        let t = &st.throws[0];
        assert_eq!((t.id, t.carry, t.x, t.y, t.dir), (200, Some(261), 30.0, 12.0, 3));
        assert_eq!(st.others[0].carry, None, "the thrower's hands are empty");
        // A player the client does not know throws nothing.
        st.apply_body(&[&[21 + 0x20][..], &gs(9), b"\n"].concat());
        assert_eq!(st.throws.len(), 1);
    }

    #[test]
    fn other_player_updates_merge() {
        let mut st = SessionState::default();
        // Nick "bob" (prop 0), x 30 (15 = 60), y 12 (16 = 24), dir 3.
        let first = other(200, &[0x20, 0x23, b'b', b'o', b'b', 15 + 0x20, 60 + 0x20, 16 + 0x20, 24 + 0x20, 17 + 0x20, 3 + 0x20]);
        st.apply_body(&first);
        // Later: only the chat bubble (prop 12).
        st.apply_body(&other(200, &[12 + 0x20, 0x22, b'h', b'i']));
        let o = &st.others[0];
        assert_eq!((o.id, o.x, o.y, o.dir), (200, 30.0, 12.0, 3));
        assert_eq!(o.nick.as_deref(), Some("bob"));
        assert_eq!(o.chat.as_deref(), Some("hi"));
        assert_eq!(st.chat_log.last().map(|l| (l.from.as_str(), l.text.as_str())), Some(("bob", "hi")));

        // PCONNECTED (51) with no value: gone.
        st.apply_body(&other(200, &[51 + 0x20]));
        assert!(st.others.is_empty());
    }

    #[test]
    fn own_nickname_from_player_props() {
        let mut st = SessionState::default();
        st.apply_body(b") 'default\n");
        assert_eq!(st.nickname.as_deref(), Some("default"));
    }

    #[test]
    fn own_hearts_status_and_ap() {
        let mut st = SessionState::default();
        // MAXPOWER 3, CURPOWER 5 (2.5 hearts), STATUS 0x14, ALIGNMENT 50.
        let props = [1 + 0x20, 3 + 0x20, 2 + 0x20, 5 + 0x20, 18 + 0x20, 0x14 + 0x20, 32 + 0x20, 50 + 0x20];
        st.apply_body(&[&[9 + 0x20][..], &props, b"\n"].concat());
        assert_eq!(
            (st.own_max_power, st.own_cur_power, st.own_status, st.own_alignment),
            (Some(3), Some(5), Some(0x14), Some(50))
        );
        // GLOVEPOWER (6).
        st.apply_body(&[9 + 0x20, 6 + 0x20, 3 + 0x20, b'\n']);
        assert_eq!(st.own_glove_power, Some(3));
        // RUPEESCOUNT 1 after a 1.39 pickup (lab capture S>C 9 `#  !`),
        // then BOMBSCOUNT 15 and ARROWSCOUNT 7.
        st.apply_body(b")#  !\n");
        st.apply_body(&[9 + 0x20, 4 + 0x20, 15 + 0x20, 5 + 0x20, 7 + 0x20, b'\n']);
        assert_eq!((st.own_rupees, st.own_bombs, st.own_darts), (Some(1), Some(15), Some(7)));
    }

    #[test]
    fn own_and_others_looks_from_props() {
        let mut st = SessionState::default();
        // HEADGIF "head927.png" (length + 100), BODYIMG "body13.png", COLORS.
        let mut props = vec![11 + 0x20, 11 + 100 + 0x20];
        props.extend(b"head927.png");
        props.extend([35 + 0x20, 10 + 0x20]);
        props.extend(b"body13.png");
        props.extend([13 + 0x20, 0x32, 0x2f, 0x2b, 0x2b, 0x32]);
        st.apply_body(&[&[9 + 0x20][..], &props, b"\n"].concat());
        assert_eq!(st.own_head.as_deref(), Some("head927.png"));
        assert_eq!(st.own_body.as_deref(), Some("body13.png"));
        assert_eq!(st.own_colors, Some([18, 15, 11, 11, 18]));
        st.apply_body(&other(7, &[13 + 0x20, 0x22, 0x20, 0x2a, 0x24, 0x32]));
        assert_eq!(st.others[0].looks.colors, Some([2, 0, 10, 4, 18]));
        // A later update without prop 13 keeps the colours.
        st.apply_body(&other(7, &[15 + 0x20, 0x30]));
        assert_eq!(st.others[0].looks.colors, Some([2, 0, 10, 4, 18]));
    }

    #[test]
    fn attribute_props_37_to_41_name_the_hat_and_others() {
        let mut st = SessionState::default();
        let mut props = vec![37 + 0x20, 8 + 0x20];
        props.extend(b"hat3.png");
        st.apply_body(&[&[9 + 0x20][..], &props, b"\n"].concat());
        assert_eq!(st.own_attrs[0].as_deref(), Some("hat3.png"));
        let mut props = vec![39 + 0x20, 5 + 0x20];
        props.extend(b"x.png");
        st.apply_body(&other(7, &props));
        assert_eq!(st.others[0].looks.attrs[2].as_deref(), Some("x.png"));
        // A later update without the prop keeps it.
        st.apply_body(&other(7, &[15 + 0x20, 0x30]));
        assert_eq!(st.others[0].looks.attrs[2].as_deref(), Some("x.png"));
    }

    #[test]
    fn delplayer_reads_gshort() {
        let mut st = SessionState::default();
        st.apply_body(&other(300, &[15 + 0x20, 0x30]));
        st.apply_body(&[&[56 + 0x20][..], &gs(300), b"\n"].concat());
        assert!(st.others.is_empty());
    }

    #[test]
    fn toall_pm_and_admin_lines() {
        let mut st = SessionState::default();
        st.apply_body(&other(5, &[0x20, 0x21, b'z']));
        st.apply_body(&[&[13 + 0x20][..], &gs(5), b"%hello\n"].concat());
        st.apply_body(&[&[37 + 0x20][..], &gs(5), b"\"\",\"Private message:\",\"psst, \"\"x\"\"\"\n"].concat());
        st.apply_body(b"CServer message:\xa7Shutting down\n");
        let got: Vec<_> = st.chat_log.iter().map(|l| (l.kind, l.from.as_str(), l.text.as_str())).collect();
        assert_eq!(
            got,
            vec![
                (ChatKind::Toall, "z", "hello"),
                (ChatKind::Private, "z", "psst, \"x\""),
                (ChatKind::Admin, "Server message", "Shutting down"),
            ]
        );
    }

    /// A player's props: nickname (0) and account (34).
    fn who_props(nick: &str, account: &str) -> Vec<u8> {
        let mut v = vec![0x20, 0x20 + nick.len() as u8];
        v.extend_from_slice(nick.as_bytes());
        v.extend_from_slice(&[34 + 0x20, 0x20 + account.len() as u8]);
        v.extend_from_slice(account.as_bytes());
        v
    }

    fn say(id: u16, text: &str) -> Vec<u8> {
        let mut v = vec![12 + 0x20, 0x20 + text.len() as u8];
        v.extend_from_slice(text.as_bytes());
        other(id, &v)
    }

    fn toall(from: u16, text: &str) -> Vec<u8> {
        [&[13 + 0x20][..], &gs(from), &[0x20 + text.len() as u8], text.as_bytes(), b"\n"].concat()
    }

    fn pm(from: u16, text: &str) -> Vec<u8> {
        [&[37 + 0x20][..], &gs(from), format!("\"\",\"{text}\"").as_bytes(), b"\n"].concat()
    }

    fn ignoring(names: &[&str], muted: &[&str]) -> crate::chatlists::ChatFilter {
        let mut f = crate::chatlists::ChatFilter::default();
        for n in names {
            f.ignore.add(n);
        }
        for n in muted {
            f.mute.add(n);
        }
        f
    }

    #[test]
    fn account_name_prop_is_read() {
        let mut st = SessionState::default();
        st.apply_body(&other(5, &who_props("Nick", "acct5")));
        assert_eq!(st.others[0].account.as_deref(), Some("acct5"));
        assert_eq!(st.identities[&5].account.as_deref(), Some("acct5"));
        assert_eq!(st.identities[&5].nick.as_deref(), Some("Nick"));
    }

    #[test]
    fn ignored_players_chat_toall_and_pm_are_dropped() {
        let mut st = SessionState::default();
        st.set_filter(ignoring(&["JERK"], &[]));
        st.apply_body(&other(5, &who_props("Nice Name", "jerk")));
        st.apply_body(&other(6, &who_props("Friend", "friend")));
        // Bubble: none for the ignored player, one for the friend.
        st.apply_body(&say(5, "hello"));
        st.apply_body(&say(6, "hi"));
        assert_eq!(st.others[0].chat, None);
        assert_eq!(st.others[1].chat.as_deref(), Some("hi"));
        st.apply_body(&toall(5, "buy gold"));
        st.apply_body(&toall(6, "party"));
        st.apply_body(&pm(5, "psst"));
        st.apply_body(&pm(6, "hey"));
        let got: Vec<_> = st.chat_log.iter().map(|l| (l.kind, l.from.as_str(), l.text.as_str())).collect();
        assert_eq!(
            got,
            vec![
                (ChatKind::Say, "Friend", "hi"),
                (ChatKind::Toall, "Friend", "party"),
                (ChatKind::Private, "Friend", "hey"),
            ]
        );
        assert_eq!(st.hidden, 3, "a bubble, a toall and a PM");
    }

    #[test]
    fn pm_from_a_player_who_left_the_level_is_still_filtered() {
        let mut st = SessionState::default();
        st.set_filter(ignoring(&["jerk"], &[]));
        st.apply_body(&other(5, &who_props("J", "jerk")));
        st.apply_body(&other(5, &[51 + 0x20]));
        assert!(st.others.is_empty());
        st.apply_body(&pm(5, "still here"));
        assert!(st.chat_log.is_empty());
        assert_eq!(st.hidden, 1);
    }

    #[test]
    fn nothing_is_hidden_by_default() {
        let mut st = SessionState::default();
        st.apply_body(&other(5, &who_props("J", "jerk")));
        st.apply_body(&say(5, "yo"));
        st.apply_body(&pm(5, "pm"));
        assert_eq!(st.chat_log.len(), 2);
        assert_eq!(st.hidden, 0);
    }

    #[test]
    fn mute_hides_the_same_way_and_new_filter_clears_bubbles() {
        let mut st = SessionState::default();
        st.apply_body(&other(7, &who_props("Loud", "loud")));
        st.apply_body(&say(7, "AAAA"));
        assert_eq!(st.others[0].chat.as_deref(), Some("AAAA"));
        // Muting takes the bubble down at once.
        st.set_filter(ignoring(&[], &["Loud"]));
        assert_eq!(st.others[0].chat, None);
        st.apply_body(&say(7, "BBBB"));
        st.apply_body(&toall(7, "CCCC"));
        assert_eq!(st.chat_log.len(), 1, "only the first line, from before the mute");
        // Unmuted: back.
        st.set_filter(ignoring(&[], &[]));
        st.apply_body(&say(7, "DDDD"));
        assert_eq!(st.others[0].chat.as_deref(), Some("DDDD"));
    }

    #[test]
    fn nickname_decides_while_no_account_is_known() {
        let mut st = SessionState::default();
        st.set_filter(ignoring(&["Jerk"], &[]));
        st.apply_body(&other(5, &[0x20, 0x24, b'j', b'e', b'r', b'k']));
        st.apply_body(&say(5, "hi"));
        assert_eq!(st.others[0].chat, None);
        // Once the account arrives and differs, the account decides.
        st.apply_body(&other(5, &[34 + 0x20, 0x20 + 5, b'o', b't', b'h', b'e', b'r']));
        st.apply_body(&say(5, "hi again"));
        assert_eq!(st.others[0].chat.as_deref(), Some("hi again"));
    }

    #[test]
    fn bubble_shows_three_seconds_and_until_the_speaker_clears_it() {
        // Measured on the real 2.05 rev 2 (Wine lab, 2026-10-30).
        use crate::chatlists::BUBBLE_MS;
        let mut st = SessionState::default();
        st.apply_body(&other(5, &who_props("A", "a")));
        st.apply_body(&say(5, "hello"));
        assert_eq!(st.others[0].chat.as_deref(), Some("hello"));
        // Not cleared: it stays however long.
        assert!(!st.tick_bubbles(600_000));
        assert_eq!(st.others[0].chat.as_deref(), Some("hello"));
        // Cleared after its 3 s are up: it goes at once.
        st.apply_body(&other(5, &[12 + 0x20, 0x20]));
        assert_eq!(st.others[0].chat, None);
        // The history line stays.
        assert_eq!(st.chat_log.len(), 1);
        assert!(!st.tick_bubbles(1000), "nothing more to end");
        // An empty text with no line up does nothing visible.
        st.apply_body(&other(5, &[12 + 0x20, 0x20]));
        assert_eq!(st.others[0].chat, None);
        assert!(!st.tick_bubbles(10));
        // Cleared 0.5 s in: it still shows until 3 s after it arrived.
        st.apply_body(&say(5, "again"));
        assert!(!st.tick_bubbles(500));
        st.apply_body(&other(5, &[12 + 0x20, 0x20]));
        assert_eq!(st.others[0].chat.as_deref(), Some("again"));
        assert!(!st.tick_bubbles(BUBBLE_MS - 550), "still up");
        assert!(st.tick_bubbles(50), "this tick takes it down");
        assert_eq!(st.others[0].chat, None);
        // A new line starts its own 3 s and waits for its own clear.
        st.apply_body(&say(5, "and more"));
        assert!(!st.tick_bubbles(BUBBLE_MS * 2));
        assert_eq!(st.others[0].chat.as_deref(), Some("and more"));
    }

    #[test]
    fn player_warp_sets_position() {
        // id 14, x = 30.5 (61), y = 12 (24), level "start.nw"
        let body = [&[0x2e, 0x20 + 61, 0x20 + 24][..], b"start.nw\n"].concat();
        let mut state = SessionState::default();
        state.apply_body(&body);
        assert_eq!(state.player_x, Some(30.5));
        assert_eq!(state.player_y, Some(12.0));
        assert_eq!(state.warps, vec!["start.nw".to_string()]);
    }

    /// 1.39 has no handler for world time (42); 2.05 does.
    #[test]
    fn version_drops_unknown_ids() {
        let body = [&[42 + 0x20][..], b"    \n", &[6 + 0x20][..], b"start.nw\n"].concat();
        let mut old = SessionState::for_version(ClientVersion::V139);
        old.apply_body(&body);
        assert_eq!(old.ignored.get(&42), Some(&1));
        assert_eq!(old.level_name.as_deref(), Some("start.nw"));
        let mut new = SessionState::for_version(ClientVersion::V205);
        new.apply_body(&body);
        assert!(new.ignored.is_empty());
    }

    /// A file inside a raw block reaches a version whose switch has 100
    /// but not 102.
    #[test]
    fn raw_block_ids_follow_rawdata() {
        let st = SessionState::for_version(ClientVersion::V139);
        assert!(st.handles(101) && st.handles(102));
        assert!(!st.handles(169));
    }

    fn msg(id: u8, payload: &[u8]) -> Vec<u8> {
        [&[id + 0x20][..], payload, b"\n"].concat()
    }

    /// The messages every public 2.22 gserver sent around the level (ids
    /// 10, 25, 39, 41, 44, 47, 156, 168, 174, 179, 180, 182), shaped like
    /// the captured ones, plus the big-map and mini-map specs.
    #[test]
    fn public_login_extras_are_decoded() {
        let mut st = SessionState::for_version(ClientVersion::V222);
        st.apply_body(&msg(6, b"start.nw"));
        let mut body = Vec::new();
        for (id, p) in [
            (25u8, &b"i"[..]),
            (44, b""),
            (168, b""),
            (171, b"bigmap2.txt,bigmap.png,0,0"),
            (172, b"minimap.txt,map2.png,0,0"),
            (10, b""),
            (174, b" "),
            (156, b"start.nw"),
            (179, b"\"Welcome to X.\",\"Credits, with a comma.\""),
            (41, b"<html><body>hi</body></html>"),
            (47, b"\"Server\",\"Manager\",\"GP\""),
            (180, b"Online,Away,DND"),
            (182, b""),
        ] {
            body.extend(msg(id, p));
        }
        // LEVELMODTIME: GINT5 1_781_284_436.
        let t: u64 = 1_781_284_436;
        let g5: Vec<u8> = (0..5).rev().map(|i| (((t >> (7 * i)) & 0x7F) as u8) + 0x20).collect();
        body.extend(msg(39, &g5));
        st.apply_body(&body);

        assert!(st.unhandled.is_empty(), "{:?}", st.unhandled);
        assert!(st.ignored.is_empty(), "{:?}", st.ignored);
        assert_eq!(st.signature, Some(73));
        assert!(st.has_npc_server && st.is_leader && !st.ghost_icon);
        assert_eq!(st.bigmap.as_ref().map(|m| m.png.as_str()), Some("bigmap.png"));
        assert_eq!(st.minimap.as_ref().map(|m| m.txt.as_str()), Some("minimap.txt"));
        assert_eq!(st.active_level.as_deref(), Some("start.nw"));
        assert_eq!(st.rpg_window, vec!["Welcome to X.", "Credits, with a comma."]);
        assert_eq!(st.start_message.as_deref(), Some("<html><body>hi</body></html>"));
        assert_eq!(st.staff_guilds, vec!["Server", "Manager", "GP"]);
        assert_eq!(st.status_list, vec!["Online", "Away", "DND"]);
        assert_eq!(st.process_list_requests, 1);
        assert_eq!(st.level_mod_time, Some(("start.nw".to_string(), 1_781_284_436)));
    }

    /// Ghost icon on, and the map specs are ignored by a version without
    /// handlers for them (2.05 has no 171 or 172).
    #[test]
    fn extras_follow_the_version() {
        let mut old = SessionState::for_version(ClientVersion::V205);
        old.apply_body(&msg(171, b"a.txt,b.png,1,2"));
        assert!(old.bigmap.is_none());
        assert_eq!(old.ignored.get(&171), Some(&1));
        let mut new = SessionState::for_version(ClientVersion::V222);
        new.apply_body(&[msg(171, b"a.txt,b.png,1,2"), msg(174, b"!")].concat());
        assert_eq!(new.bigmap.as_ref().map(|m| (m.x, m.y)), Some((1.0, 2.0)));
        assert!(new.ghost_icon);
    }

    /// No id and no payload shape may panic any version's session, and a
    /// bad payload must leave the state as it was.
    #[test]
    fn no_message_panics() {
        let payloads: [&[u8]; 6] = [b"", b" ", b"\xff", b"\x00\x00\x00", b"\"unterminated,", &[0x7f; 40]];
        for v in ClientVersion::all_versions() {
            for id in 0..=255u8 {
                for p in payloads {
                    let mut st = SessionState::for_version(*v);
                    st.apply(&Message { id, payload: p.to_vec() });
                }
            }
        }
        let mut st = SessionState::default();
        st.apply_body(&msg(171, b"not a map spec"));
        st.apply_body(&msg(39, b"abc"));
        assert!(st.bigmap.is_none() && st.level_mod_time.is_none());
    }

    /// Another player's stream stops at MAXPOWER (1) for 2.05.
    #[test]
    fn other_player_props_per_version() {
        let mut st = SessionState::for_version(ClientVersion::V205);
        st.apply_body(&other(7, &[15 + 0x20, 0x20 + 20, 1 + 0x20, 0x23, 16 + 0x20, 0x20 + 8]));
        assert_eq!(st.others[0].x, 10.0);
        assert_eq!(st.others[0].y, 0.0);
        let mut all = SessionState::default();
        all.apply_body(&other(7, &[15 + 0x20, 0x20 + 20, 1 + 0x20, 0x23, 16 + 0x20, 0x20 + 8]));
        assert_eq!(all.others[0].y, 4.0);
    }

    // ---- decompile layouts (2.17.1 / 2.22 / 2.31) ----

    fn gc(v: u8) -> u8 {
        v + 0x20
    }

    /// Every id above 48 that the 2.22 receive dispatcher handles, with a
    /// payload shaped as the decompile describes, reaches `SessionState`
    /// and none is counted as unhandled.
    #[test]
    fn decompiled_layouts_reach_the_session() {
        let mut st = SessionState::for_version(ClientVersion::V222);
        st.apply_body(&msg(6, b"gm_nw.nw"));
        let mut body = Vec::new();
        // 49: x 30.5, y 12, z, gmap cell (1,0), then a gmap name.
        body.extend(msg(49, &[&[gc(61), gc(24), gc(50), gc(1), gc(0)][..], b"gm2x2.gmap"].concat()));
        body.extend(msg(75, &[&[gc(4)][..], b"acct", &[gc(5)], b"hello"].concat()));
        body.extend(msg(150, &[&[gc(4)][..], b"l.nw", &[gc(0), gc(0), gc(9)]].concat()));
        body.extend(msg(151, b""));
        body.extend(msg(153, b"sign text"));
        body.extend(msg(154, b""));
        body.extend(msg(165, &[gc(0), gc(0), gc(3), b'm']));
        body.extend(msg(166, &[gc(0), gc(0), gc(3), b'p']));
        body.extend(msg(169, b"http://up.example"));
        body.extend(msg(170, b"!"));
        body.extend(msg(173, b"ghost"));
        body.extend(msg(175, &[gc(0), gc(5), b's']));
        body.extend(msg(176, b""));
        body.extend(msg(178, b"x,Name,host,14900"));
        body.extend(msg(181, &[gc(0), gc(8), gc(1)]));
        body.extend(msg(183, b"proc.exe"));
        body.extend(msg(184, b"shot.png"));
        body.extend(msg(185, &[gc(0), gc(0), gc(0), gc(0), gc(0), gc(0), gc(50), gc(64)]));
        body.extend(msg(186, &[gc(0), gc(0), gc(1), gc(2), gc(1), gc(1), gc(0), gc(7)]));
        st.apply_body(&body);
        assert!(st.unhandled.is_empty(), "{:?}", st.unhandled);
        assert!(st.ignored.is_empty(), "{:?}", st.ignored);

        assert_eq!(st.gmap_name.as_deref(), Some("gm2x2.gmap"));
        // A gmap name is not a level: the level stays the one 6 named.
        assert_eq!(st.level_name.as_deref(), Some("gm_nw.nw"));
        assert_eq!((st.player_x, st.player_y), (Some(30.5), Some(12.0)));
        assert_eq!(st.player_warp2.as_ref().map(|w| (w.gmap_x, w.gmap_y)), Some((1, 0)));
        let p = st.profile.as_ref().unwrap();
        assert_eq!((p.account.as_str(), p.lines.as_slice()), ("acct", &["hello".to_string()][..]));
        assert_eq!((st.last_npc_del.clone(), st.npc_dels, st.hide_npcs), (Some(("l.nw".to_string(), 9)), 1, 1));
        assert_eq!(st.say2.as_deref(), Some("sign text"));
        assert!(st.frozen);
        assert_eq!((st.npc_moves, st.npc_showimgs, st.shoots), (1, 1, 1));
        assert_eq!(st.upgrade_url.as_deref(), Some("http://up.example"));
        assert!(st.ghost_mode);
        assert_eq!(st.ghost_text.as_deref(), Some("ghost"));
        assert_eq!(st.server_mode, Some(ServerMode::Kingdoms));
        assert_eq!(st.server_warp.as_ref().map(|w| w.host.as_str()), Some("host"));
        assert_eq!(st.player_toggle, Some((8, true)));
        assert_eq!(st.process_running_requests, 1);
        assert_eq!(st.screenshot_requests, vec!["shot.png"]);
        assert_eq!(st.board_heights.as_ref().map(|h| h.cells.clone()), Some(vec![0.5]));
        assert_eq!(st.board_modify2.as_ref().map(|m| m.tiles.clone()), Some(vec![7]));
        // Unfreeze, and a plain level name in 49 enters that level.
        st.apply_body(&[msg(155, b""), msg(49, &[&[gc(2), gc(4), gc(50), gc(0), gc(0)][..], b"b.nw"].concat())].concat());
        assert!(!st.frozen);
        assert_eq!(st.level_name.as_deref(), Some("b.nw"));
        assert_eq!(st.gmap_name, None);
    }

    /// Ids a version has no handler for are dropped, per the decompiled
    /// dispatch tables: 186 arrives with 2.22, 44 and 189/84 move at 2.31,
    /// 2.17 has none of 186/189/84.
    #[test]
    fn layout_ids_follow_the_version_gates() {
        let handled = |v: ClientVersion, id: u8| SessionState::for_version(v).handles(id);
        for id in [186u8, 189, 84] {
            assert!(!handled(ClientVersion::V217, id), "2.17 id {id}");
        }
        assert!(handled(ClientVersion::V222, 186) && !handled(ClientVersion::V222, 189) && !handled(ClientVersion::V222, 84));
        assert!(handled(ClientVersion::V231, 189) && handled(ClientVersion::V231, 84) && handled(ClientVersion::V231, 186));
        assert!(handled(ClientVersion::V217, 44) && handled(ClientVersion::V222, 44) && !handled(ClientVersion::V231, 44));
        // 2.05 has none of 169 to 189.
        for id in 169..=189u8 {
            assert!(!handled(ClientVersion::V205, id), "2.05 id {id}");
        }
        // And a dropped message leaves the state alone.
        let mut st = SessionState::for_version(ClientVersion::V222);
        st.apply_body(&msg(189, &[gc(0), gc(0), gc(3), b'm']));
        assert_eq!(st.ignored.get(&189), Some(&1));
        assert_eq!(st.npc_moves, 0);
        // 2.31 ignores 44.
        let mut st = SessionState::for_version(ClientVersion::V231);
        st.apply_body(&msg(44, b""));
        assert!(!st.has_npc_server);
        assert_eq!(st.ignored.get(&44), Some(&1));
    }

    /// Bad payloads leave every new field alone.
    #[test]
    fn bad_layout_payloads_change_nothing() {
        let mut st = SessionState::for_version(ClientVersion::V231);
        for id in [49u8, 75, 150, 165, 166, 175, 178, 181, 184, 185, 186, 189] {
            st.apply_body(&msg(id, b""));
        }
        st.apply_body(&msg(184, b"shot.jpg"));
        st.apply_body(&msg(165, &[gc(0), gc(0), gc(0), b'm'])); // id 0
        assert!(st.player_warp2.is_none() && st.profile.is_none() && st.last_npc_del.is_none());
        assert!(st.server_warp.is_none() && st.player_toggle.is_none() && st.screenshot_requests.is_empty());
        assert!(st.board_heights.is_none() && st.board_modify2.is_none());
        assert_eq!((st.npc_moves, st.npc_showimgs, st.shoots, st.npc_dels), (0, 0, 0, 0));
        assert!(st.unhandled.is_empty());
    }

    /// The decoded images of a player the client knows are kept; a sender it
    /// does not know is ignored; a player who leaves takes them along.
    #[test]
    fn showimg_relay_updates_the_images_of_known_players() {
        let mut st = SessionState { showimg_records: true, ..Default::default() };
        let relay = |id: u8, text: &[u8]| {
            let mut p = vec![gc(0), gc(id)];
            p.extend_from_slice(text);
            msg(32, &p)
        };
        // Player 2 is not in the level yet: nothing is kept.
        st.apply_body(&relay(2, &[0x29, 0x2a, 0x20, 0x21, b'a']));
        assert!(st.player_images.is_empty());
        st.others.push(OtherPlayer {
            id: 2,
            x: 0.0,
            y: 0.0,
            dir: 0,
            nick: None,
            chat: None,
            chat_timer: Default::default(),
            account: None,
            cell: None,
            sprite: None,
            status: None,
            ap: None,
            sword_power: None,
            carry: None,
            looks: Default::default(),
        });
        st.apply_body(&relay(2, &[0x29, 0x2a, 0x20, 0x21, b'a']));
        assert_eq!(st.player_images[&2][&0].name, b"a");
        // The real clients' byte empties the list.
        st.apply_body(&relay(2, &[gc(9)]));
        assert!(st.player_images[&2].is_empty());
        st.apply_body(&relay(2, &[0x2b, 0x20, 0x21, b'q']));
        assert_eq!(st.player_images[&2].len(), 1);
        st.apply_body(&msg(56, &[gc(0), gc(2)]));
        assert!(st.player_images.is_empty());
        // Only the builds read for this form decode it.
        let on = |v| SessionState::for_version(v).showimg_records;
        assert!(!on(ClientVersion::V205) && !on(ClientVersion::V133));
        assert!(on(ClientVersion::V217) && on(ClientVersion::V222) && on(ClientVersion::V231));
    }

    /// 32 is `GSHORT id` plus opaque bytes: the real clients' one-byte 9, any
    /// other payload and an empty one are kept; a payload too short for the
    /// id changes nothing and does not panic.
    #[test]
    fn showimg_relay_is_kept_opaque() {
        for v in [ClientVersion::V222, ClientVersion::V231] {
            let mut st = SessionState::for_version(v);
            st.apply_body(&msg(32, &[gc(0), gc(2), gc(9)]));
            assert_eq!((st.showimg_relays, st.last_showimg.clone()), (1, Some((2, vec![gc(9)]))), "{v:?}");
            st.apply_body(&msg(32, &[gc(0), gc(3)]));
            assert_eq!(st.last_showimg, Some((3, vec![])));
            st.apply_body(&msg(32, &[gc(9)]));
            st.apply_body(&msg(32, b""));
            assert_eq!(st.showimg_relays, 2);
            assert!(st.unhandled.is_empty(), "{:?}", st.unhandled);
        }
    }

    /// 178 is decoded with the lab's field order; a 3-field text is ignored,
    /// and a later good one replaces the earlier.
    #[test]
    fn server_warp_is_decoded_and_three_fields_are_ignored() {
        let mut st = SessionState::for_version(ClientVersion::V231);
        st.apply_body(&msg(178, b"labtwo,labtwo,127.0.0.2"));
        assert!(st.server_warp.is_none());
        st.apply_body(&msg(178, b"labtwo,labtwo,127.0.0.2,14803"));
        let w = st.server_warp.take().unwrap();
        assert_eq!((w.name.as_str(), w.host.as_str(), w.port.as_str()), ("labtwo", "127.0.0.2", "14803"));
        assert_eq!(w.address().as_deref(), Some("127.0.0.2:14803"));
        assert!(st.server_warp.is_none());
    }

    /// The signature rule is reported, not enforced.
    #[test]
    fn signature_value_reports_the_client_rule() {
        let mut st = SessionState::for_version(ClientVersion::V222);
        assert_eq!(st.signature_action(), None);
        st.apply_body(&msg(25, b"i"));
        assert_eq!(st.signature_action(), Some(SignatureAction::Ignore));
        st.apply_body(&msg(25, b"!"));
        assert_eq!(st.signature_action(), Some(SignatureAction::Quit));
        st.apply_body(&msg(25, b" "));
        assert_eq!(st.signature_action(), Some(SignatureAction::QuitUnlessLoopback));
    }

    fn id_set(s: &str) -> IdSet {
        let mut set = IdSet::EMPTY;
        for part in s.split(',') {
            let (lo, hi) = part.split_once('-').unwrap_or((part, part));
            set = set.with_range(lo.parse().unwrap(), hi.parse().unwrap());
        }
        set
    }

    fn table_version(label: &str) -> Option<ClientVersion> {
        let l = label.to_ascii_lowercase();
        Some(match l.split(|c| c == ' ' || c == '(').next()? {
            "2.00" => ClientVersion::V200,
            "2.01" => ClientVersion::V201,
            "2.02" if l.contains("rev2") => ClientVersion::V202R2,
            "2.02" => ClientVersion::V202,
            "2.03" => ClientVersion::V203,
            "2.04" => ClientVersion::V204,
            "2.05" => ClientVersion::V205,
            "2.17.1" => ClientVersion::V217,
            "2.22" => ClientVersion::V222,
            "2.31" => ClientVersion::V231,
            "newmain" => ClientVersion::V142,
            _ => return None,
        })
    }

    /// The per-build tables read from the 2.00 to 2.31 binaries
    /// (`decomp_plo_handled.tsv`, `decomp_pli_sent.tsv`) agree with the
    /// profile rows: same handled server ids, same sendable client ids.
    /// The 2.1 test builds are skipped (the 2.1 row is inferred and wider).
    #[test]
    fn profile_sets_match_the_decompiled_tables() {
        let mut plo_rows = 0;
        for line in include_str!("decomp_plo_handled.tsv").lines().skip(1) {
            let c: Vec<&str> = line.split('\t').collect();
            let Some(v) = table_version(c[0]) else { continue };
            let want = id_set(c[3]);
            assert_eq!(want.len(), c[2].parse::<usize>().unwrap(), "{}: count column", c[0]);
            assert_eq!(v.profile().protocol.plo, want, "plo {}", c[0]);
            plo_rows += 1;
        }
        let mut pli_rows = 0;
        for line in include_str!("decomp_pli_sent.tsv").lines().skip(1) {
            let c: Vec<&str> = line.split('\t').collect();
            let Some(v) = table_version(c[0]) else { continue };
            assert_eq!(v.profile().protocol.pli, id_set(c[1]), "pli {}", c[0]);
            pli_rows += 1;
        }
        // 19 handled-id rows and 8 client-send rows (the 2.1 test builds are skipped).
        assert_eq!((plo_rows, pli_rows), (19, 8));
        // The 2.1 test builds' handled set is inside the 2.1 row.
        for line in include_str!("decomp_plo_handled.tsv").lines().skip(1) {
            let c: Vec<&str> = line.split('\t').collect();
            if c[0].starts_with("2.1 test") {
                let want = id_set(c[3]);
                assert_eq!(ClientVersion::V210.profile().protocol.plo.and(want), want, "{}", c[0]);
            }
        }
    }

    /// Every id from every version, with payloads of awkward lengths and
    /// bytes, leaves every session intact (no panic).
    #[test]
    fn no_layout_panics_any_id_any_version() {
        let mut payloads: Vec<Vec<u8>> = vec![vec![], vec![0xff; 300], vec![0x7f; 5000]];
        for n in 1..=14usize {
            payloads.push(vec![0x20; n]);
            payloads.push(vec![0xff; n]);
            payloads.push((0..n as u8).map(|i| 0x20 + i).collect());
        }
        for v in ClientVersion::all_versions() {
            for id in 0..=255u8 {
                for p in &payloads {
                    let mut st = SessionState::for_version(*v);
                    st.apply(&Message { id, payload: p.clone() });
                    // And through the splitter, with a raw-data prefix.
                    st.apply_body(&[&[id.wrapping_add(0x20)][..], p, b"\n"].concat());
                }
            }
        }
    }

    // ---- gmap sessions ----

    fn gsh(v: u16) -> [u8; 2] {
        [gc((v >> 7) as u8 & 0x7F), gc(v as u8 & 0x7F)]
    }

    /// A `PLO_RAWDATA` + `PLO_BOARDPACKET` pair for a board filled with `fill`.
    fn raw_board(fill: u16) -> Vec<u8> {
        let mut out = vec![100 + 0x20];
        out.extend([gc(0), gc(64), gc(2)]); // GINT3 8194
        out.push(b'\n');
        out.push(101 + 0x20);
        for _ in 0..64 * 64 {
            out.extend(fill.to_le_bytes());
        }
        out.push(b'\n');
        out
    }

    const GRID: &str = "GRMAP001\nWIDTH 2\nHEIGHT 2\nLEVELNAMES\n\"gm_nw.nw\",\"gm_ne.nw\",\n\"gm_sw.nw\",\"gm_se.nw\",\nLEVELNAMESEND\n";

    fn warp49(cell: (u8, u8), name: &str) -> Vec<u8> {
        msg(49, &[&[gc(116), gc(61), gc(50), gc(cell.0), gc(cell.1)][..], name.as_bytes()].concat())
    }

    /// The 2.22 lab login order: 49 names the gmap, 6 names the level, 6
    /// names the gmap again.
    fn gmap_login(st: &mut SessionState) {
        st.apply_body(&[warp49((0, 0), "gm2x2.gmap"), msg(6, b"gm_nw.nw"), raw_board(1), msg(6, b"gm2x2.gmap")].concat());
    }

    fn grid() -> Gmap {
        Gmap::parse(GRID).unwrap()
    }

    #[test]
    fn gmap_login_keeps_the_players_level_not_the_gmap_name() {
        let mut st = SessionState::for_version(ClientVersion::V222);
        gmap_login(&mut st);
        let g = st.gmap.as_ref().unwrap();
        assert_eq!((g.name.as_str(), g.cell), ("gm2x2.gmap", (0, 0)));
        assert_eq!(st.level_name.as_deref(), Some("gm_nw.nw"));
        // No grid yet: the first level named after the 49 stands in.
        assert_eq!(st.current_level_name().as_deref(), Some("gm_nw.nw"));
        st.set_gmap_grid(grid());
        assert_eq!(st.current_level_name().as_deref(), Some("gm_nw.nw"));
        st.set_gmap_cell((1, 0));
        assert_eq!(st.current_level_name().as_deref(), Some("gm_ne.nw"));
        assert_eq!(st.board_of("GM_NW.NW").map(|b| b[0]), Some(1));
    }

    #[test]
    fn neighbour_levels_are_parked_and_the_players_level_comes_back() {
        let mut st = SessionState::for_version(ClientVersion::V222);
        gmap_login(&mut st);
        st.set_gmap_grid(grid());
        st.apply_body(&msg(1, b"gm_ne.nw 0 0 1 1 5 5"));
        // The server answers a PLI 35 for gm_ne: name, board, link, gmap.
        st.apply_body(
            &[msg(6, b"gm_ne.nw"), raw_board(2), msg(1, b"other.nw 1 1 2 2 3 3"), msg(6, b"gm2x2.gmap")].concat(),
        );
        // The player's level is current again, with its own link and board.
        assert_eq!(st.level_name.as_deref(), Some("gm_nw.nw"));
        assert_eq!(st.links, vec!["gm_ne.nw 0 0 1 1 5 5"]);
        assert_eq!(st.board.as_ref().map(|b| b[0]), Some(1));
        assert_eq!(st.board_of("gm_ne.nw").map(|b| b[0]), Some(2));
        assert_eq!(st.board_of("gm_nw.nw").map(|b| b[0]), Some(1));
        // The neighbour's link did not leak into the player's level.
        assert!(!st.links.iter().any(|l| l.starts_with("other.nw")));
    }

    /// After a crossing the server names the new level, sends its data (an
    /// empty board packet when the client has it), then the gmap.
    #[test]
    fn crossing_answer_moves_the_current_level() {
        let mut st = SessionState::for_version(ClientVersion::V222);
        gmap_login(&mut st);
        st.set_gmap_grid(grid());
        st.apply_body(&[msg(6, b"gm_ne.nw"), raw_board(2), msg(6, b"gm2x2.gmap")].concat());
        st.set_gmap_cell((1, 0));
        assert_eq!(st.current_level_name().as_deref(), Some("gm_ne.nw"));
        st.apply_body(&[msg(6, b"gm_ne.nw"), msg(39, &[gc(0); 5]), msg(0, b"BOARD 0 0 1 0 AA"), msg(6, b"gm2x2.gmap")].concat());
        assert_eq!(st.level_name.as_deref(), Some("gm_ne.nw"));
        assert_eq!(st.board_lines.len(), 1);
        // Going back restores the first level's own board.
        st.set_gmap_cell((0, 0));
        st.apply_body(&[msg(6, b"gm_nw.nw"), msg(6, b"gm2x2.gmap")].concat());
        assert_eq!(st.level_name.as_deref(), Some("gm_nw.nw"));
        assert_eq!(st.board.as_ref().map(|b| b[0]), Some(1));
    }

    /// A cached-level crossing on a gmap, as the lab captured it (2.22 and
    /// 2.31, `gmap_cross`: east, back west, south, back north). The real
    /// client crosses back into a level it holds with the same props frame
    /// as a first visit, not with `PLI_LEVELWARPMOD` (30). The server's
    /// answer then leaves out its mod time (39): level name, an empty
    /// board packet, the gmap name. The level comes back from its parked
    /// slot.
    #[test]
    fn revisit_answer_without_mod_time_restores_the_parked_level() {
        let mut st = SessionState::for_version(ClientVersion::V231);
        st.apply_body(
            &[warp49((0, 0), "gm2x2.gmap"), msg(6, b"gm_nw.nw"), msg(39, &[gc(0), gc(0), gc(0), gc(1), gc(2)]), raw_board(1)]
                .concat(),
        );
        st.apply_body(&[msg(1, b"lv_a.nw 0 0 1 64 60 30"), msg(6, b"gm2x2.gmap")].concat());
        st.set_gmap_grid(grid());
        // East, first visit: the answer carries the mod time and the board.
        st.set_gmap_cell((1, 0));
        st.apply_body(&[msg(6, b"gm_ne.nw"), msg(39, &[gc(0); 5]), raw_board(2), msg(0, b""), msg(6, b"gm2x2.gmap")].concat());
        assert_eq!(st.board.as_ref().map(|b| b[0]), Some(2));
        assert!(st.links.is_empty(), "the first level's link stays with it");
        // Back west into the level the client holds: the gmap never asks
        // for a cached link, whatever the cache holds.
        assert_eq!(st.cached_mod_time("gm_nw.nw"), None);
        st.set_gmap_cell((0, 0));
        st.apply_body(&[msg(6, b"gm_nw.nw"), msg(0, b""), msg(6, b"gm2x2.gmap")].concat());
        assert_eq!(st.level_name.as_deref(), Some("gm_nw.nw"));
        assert_eq!(st.board.as_ref().map(|b| b[0]), Some(1));
        assert_eq!(st.board_of("gm_nw.nw").map(|b| b[0]), Some(1));
        assert_eq!(st.links, vec!["lv_a.nw 0 0 1 64 60 30".to_string()]);
        // And the neighbour's own data is parked again, not lost.
        assert_eq!(st.board_of("gm_ne.nw").map(|b| b[0]), Some(2));
    }

    #[test]
    fn plain_49_leaves_the_gmap() {
        let mut st = SessionState::for_version(ClientVersion::V222);
        gmap_login(&mut st);
        st.apply_body(&warp49((0, 0), "b.nw"));
        assert!(st.gmap.is_none());
        assert_eq!(st.level_name.as_deref(), Some("b.nw"));
        assert_eq!(st.current_level_name().as_deref(), Some("b.nw"));
    }

    #[test]
    fn gmap_file_from_the_server_gives_the_grid() {
        let mut st = SessionState::for_version(ClientVersion::V222);
        gmap_login(&mut st);
        st.incoming_files.push(("gm2x2.gmap".into(), GRID.as_bytes().to_vec()));
        st.incoming_files.push(("other.png".into(), vec![1, 2, 3]));
        let dir = std::env::temp_dir().join(format!("seihai-gmap-{}", std::process::id()));
        let written = st.flush_files(&dir);
        assert_eq!(written.len(), 2);
        assert_eq!(st.gmap.as_ref().unwrap().map.as_ref().unwrap().width, 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn own_cell_props_move_the_cell() {
        let mut st = SessionState::for_version(ClientVersion::V222);
        gmap_login(&mut st);
        st.apply_body(&msg(9, &[gc(43), gc(1), gc(44), gc(1)]));
        assert_eq!(st.gmap.as_ref().unwrap().cell, (1, 1));
    }

    /// 186 goes onto the level at its mapX/mapY cell. A block at x 62 is
    /// clipped at column 63, as the real 2.22 and 2.31 clients drew it.
    #[test]
    fn board_modify2_lands_on_the_level_at_its_cell() {
        let mut st = SessionState::for_version(ClientVersion::V222);
        gmap_login(&mut st);
        st.set_gmap_grid(grid());
        st.apply_body(&[msg(6, b"gm_ne.nw"), raw_board(2), msg(6, b"gm2x2.gmap")].concat());
        let mut p = vec![gc(1), gc(0), gc(62), gc(1), gc(4), gc(1)];
        for t in [10u16, 11, 12, 13] {
            p.extend(gsh(t));
        }
        st.apply_body(&msg(186, &p));
        let ne = st.board_of("gm_ne.nw").unwrap();
        assert_eq!(&ne[64 + 60..64 + 64], &[2, 2, 10, 11], "columns 62 and 63 only");
        assert_eq!(ne[2 * 64 + 62], 2, "no wrap onto the next row");
        // The player's own level is untouched, and so is its live board.
        assert_eq!(st.board_of("gm_nw.nw").unwrap()[64 + 62], 1);
        assert_eq!(st.board.as_ref().unwrap()[64 + 62], 1);
        let mods = st.take_board_mods();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].0, "gm_ne.nw");
        assert!(st.take_board_mods().is_empty());
    }

    #[test]
    fn board_modify2_on_the_live_level_updates_its_board_too() {
        let mut st = SessionState::for_version(ClientVersion::V231);
        gmap_login(&mut st);
        st.set_gmap_grid(grid());
        let mut p = vec![gc(0), gc(0), gc(1), gc(1), gc(1), gc(1)];
        p.extend(gsh(0x0ABC));
        st.apply_body(&msg(186, &p));
        assert_eq!(st.board.as_ref().unwrap()[64 + 1], 0x0ABC);
        assert_eq!(st.board_of("gm_nw.nw").unwrap()[64 + 1], 0x0ABC);
        // The 12-bit mask of the 101 board applies.
        let mut p = vec![gc(0), gc(0), gc(0), gc(0), gc(1), gc(1)];
        p.extend(gsh(0xFABC));
        st.apply_body(&msg(186, &p));
        assert_eq!(st.board.as_ref().unwrap()[0], 0x0ABC);
    }

    #[test]
    fn board_modify2_for_an_unknown_cell_is_dropped() {
        let mut st = SessionState::for_version(ClientVersion::V222);
        gmap_login(&mut st);
        st.set_gmap_grid(grid());
        let mut p = vec![gc(5), gc(5), gc(0), gc(0), gc(1), gc(1)];
        p.extend(gsh(9));
        st.apply_body(&msg(186, &p));
        assert_eq!(st.board_mods_dropped, 1);
        assert!(st.take_board_mods().is_empty());
        assert_eq!(st.board_modify2.as_ref().map(|m| m.map_x), Some(5), "still recorded");
    }

    /// `PLO_BOARDMODIFY` (7) has no cell: it goes to the player's level.
    #[test]
    fn board_modify_goes_to_the_live_level() {
        let mut st = SessionState::for_version(ClientVersion::V205);
        st.apply_body(&[msg(6, b"a.nw"), raw_board(3)].concat());
        let mut p = vec![gc(2), gc(2), gc(2), gc(1)];
        p.extend(gsh(7));
        p.extend(gsh(8));
        st.apply_body(&msg(7, &p));
        assert_eq!(&st.board.as_ref().unwrap()[2 * 64 + 2..2 * 64 + 4], &[7, 8]);
        let mods = st.take_board_mods();
        assert_eq!((mods.len(), mods[0].0.as_str()), (1, "a.nw"));
        // Cut short: dropped.
        st.apply_body(&msg(7, &p[..5]));
        assert!(st.take_board_mods().is_empty());
    }

    /// Off a gmap there is one level, cell (0, 0).
    #[test]
    fn board_modify2_off_a_gmap_is_cell_zero() {
        let mut st = SessionState::for_version(ClientVersion::V222);
        st.apply_body(&[msg(6, b"a.nw"), raw_board(3)].concat());
        let mut p = vec![gc(0), gc(0), gc(2), gc(2), gc(1), gc(1)];
        p.extend(gsh(7));
        st.apply_body(&msg(186, &p));
        assert_eq!(st.board.as_ref().unwrap()[2 * 64 + 2], 7);
        assert_eq!(st.take_board_mods().len(), 1);
        let mut p = vec![gc(1), gc(0), gc(2), gc(2), gc(1), gc(1)];
        p.extend(gsh(8));
        st.apply_body(&msg(186, &p));
        assert_eq!(st.board.as_ref().unwrap()[2 * 64 + 2], 7);
        assert_eq!(st.board_mods_dropped, 1);
    }

    /// Props 43 and 44 of another player give its cell, one axis at a time;
    /// its position is then drawn relative to ours.
    #[test]
    fn other_player_cell_props() {
        let mut st = SessionState::default();
        st.apply_body(&other(7, &[15 + 0x20, 10 + 0x20, 16 + 0x20, 20 + 0x20]));
        assert_eq!(st.others[0].cell, None);
        assert_eq!(st.others[0].position_from(Some((0, 0))), (5.0, 10.0));
        st.apply_body(&other(7, &[43 + 0x20, 1 + 0x20]));
        assert_eq!(st.others[0].cell, Some((1, 0)));
        st.apply_body(&other(7, &[44 + 0x20, 1 + 0x20, 15 + 0x20, 2 + 0x20]));
        assert_eq!(st.others[0].cell, Some((1, 1)));
        // Cell (1,1) seen from cell (0,0): 64 tiles east and south.
        assert_eq!(st.others[0].position_from(Some((0, 0))), (65.0, 74.0));
        assert_eq!(st.others[0].position_from(Some((1, 1))), (1.0, 10.0));
        assert_eq!(st.others[0].position_from(None), (1.0, 10.0));
    }

    // --- Script-initiated warps: the online request (item 2) ---

    mod script_warp {
        use super::*;
        use crate::net::{start_online_link, EnterLevelSender};
        use crate::offline::OfflineSession;
        use crate::protocol::TamperSender;
        use std::cell::RefCell;

        const T: u32 = 1_781_284_436;

        /// What `send` was given, one entry per frame.
        fn run(
            v: ClientVersion,
            sess: &mut OfflineSession,
            st: &mut SessionState,
            el: Option<&mut EnterLevelSender>,
            tamper: Option<&mut TamperSender>,
        ) -> (Option<crate::net::LinkStart>, Vec<Vec<Vec<u8>>>) {
            let sent = RefCell::new(Vec::new());
            let r = start_online_link(
                |m| {
                    sent.borrow_mut().push(m.to_vec());
                    Ok(())
                },
                v,
                sess,
                st,
                el,
                tamper,
                "c.nw",
                12.0,
                3.0,
            )
            .unwrap();
            (r, sent.into_inner())
        }

        fn warp_to_c(code: u8) -> Vec<u8> {
            // 12 tiles is 24 half-tiles, 3 is 6.
            [&[code + 0x20, 24 + 0x20, 6 + 0x20][..], b"c.nw\n"].concat()
        }

        /// 2.31, level not held: PLI 0 alone, one frame; the warp is
        /// pending (no walking, no further link); the server's answer names
        /// the level and the 46 is owed after it.
        #[test]
        fn uncached_script_warp_sends_pli_0_then_loads_on_the_answer() {
            let v = ClientVersion::V231;
            let mut st = SessionState::for_version(v);
            st.apply_body(&full_answer("a.nw", 1_000, 0, "", ""));
            let mut el = EnterLevelSender::for_version(v);
            assert!(!el.note_level("a.nw"));
            let mut sess = OfflineSession::new();
            let (r, sent) = run(v, &mut sess, &mut st, Some(&mut el), None);
            let r = r.expect("sent");
            assert_eq!(sent, vec![vec![warp_to_c(0)]]);
            assert_eq!(r.arrival, Some((12.0, 3.0)), "GServer-v2 sends no PLO_PLAYERWARP for a plain level");
            assert_eq!(sess.warp_pending.as_deref(), Some("c.nw"));
            // The server's answer loads the new level; 46 is owed once.
            st.apply_body(&full_answer("c.nw", 2_000, 5, "", ""));
            assert_eq!(st.level_name.as_deref(), Some("c.nw"));
            assert!(st.board.is_some());
            assert!(el.note_level("c.nw") && el.is_pending());
            sess.arrive(12.0, 3.0);
            assert!(sess.warp_pending.is_none());
        }

        /// 2.31 with a copy of the level: 30, 46, props in one frame, in
        /// that order, and the arrival position is the requested one.
        #[test]
        fn cached_script_warp_sends_30_46_props_in_one_frame() {
            let v = ClientVersion::V231;
            let mut st = SessionState::for_version(v);
            st.apply_body(&full_answer("c.nw", T, 0, "", ""));
            st.apply_body(&full_answer("a.nw", 1_000, 1, "", ""));
            assert_eq!(st.cached_mod_time("c.nw"), Some(T));
            let mut el = EnterLevelSender::for_version(v);
            assert!(!el.note_level("a.nw"));
            let mut sess = OfflineSession::new();
            sess.player_dir = 1;
            let (r, sent) = run(v, &mut sess, &mut st, Some(&mut el), None);
            let r = r.expect("sent");
            assert_eq!(sent.len(), 1, "one frame");
            let ids: Vec<u8> = sent[0].iter().map(|m| m[0] - 0x20).collect();
            assert_eq!(ids, vec![30, 46, 2]);
            assert_eq!(r.arrival, Some((12.0, 3.0)));
            assert_eq!(r.cached_mod_time, Some(T));
            // The 46 went along: none owed after the answer.
            st.expect_cached_level("c.nw");
            st.apply_body(&cached_answer("c.nw"));
            assert_eq!(st.level_name.as_deref(), Some("c.nw"));
            assert!(!el.note_level("c.nw"));
        }

        /// 2.22 holds the tamper check in the same frame, after the request,
        /// and never sends 46. A profile without cached links asks with 0.
        #[test]
        fn script_warp_follows_the_profile() {
            let v = ClientVersion::V222;
            let mut st = SessionState::for_version(v);
            st.apply_body(&full_answer("c.nw", T, 0, "", ""));
            st.apply_body(&full_answer("a.nw", 1_000, 1, "", ""));
            let mut tamper = TamperSender::for_version(v).unwrap();
            let mut sess = OfflineSession::new();
            let (_, sent) = run(v, &mut sess, &mut st, None, Some(&mut tamper));
            let ids: Vec<u8> = sent[0].iter().map(|m| m[0] - 0x20).collect();
            assert_eq!(ids, vec![30, 39]);

            let v = ClientVersion::V204;
            assert!(!v.sends_cached_links());
            let mut st = SessionState::for_version(v);
            st.apply_body(&full_answer("c.nw", T, 0, "", ""));
            st.apply_body(&full_answer("a.nw", 1_000, 1, "", ""));
            let mut sess = OfflineSession::new();
            let (r, sent) = run(v, &mut sess, &mut st, None, None);
            assert_eq!(sent, vec![vec![warp_to_c(0)]]);
            assert_eq!(r.unwrap().arrival, Some((12.0, 3.0)));
        }

        /// A warp already waiting for the server: a second script warp
        /// sends nothing and changes nothing.
        #[test]
        fn a_second_script_warp_while_pending_sends_nothing() {
            let v = ClientVersion::V231;
            let mut st = SessionState::for_version(v);
            let mut sess = OfflineSession::new();
            sess.begin_warp("b.nw");
            let (r, sent) = run(v, &mut sess, &mut st, None, None);
            assert!(r.is_none() && sent.is_empty());
            assert_eq!(sess.warp_pending.as_deref(), Some("b.nw"));
        }

        /// The send fails (a profile that cannot send the request, or a
        /// dead socket): the warp is not left pending and the error
        /// comes back to the caller.
        #[test]
        fn a_failed_send_cancels_the_warp() {
            let v = ClientVersion::V231;
            let mut st = SessionState::for_version(v);
            let mut sess = OfflineSession::new();
            let e = start_online_link(
                |_| Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no")),
                v,
                &mut sess,
                &mut st,
                None,
                None,
                "c.nw",
                1.0,
                1.0,
            )
            .unwrap_err();
            assert_eq!(e.kind(), std::io::ErrorKind::Unsupported);
            assert!(sess.warp_pending.is_none());
        }
    }
}
