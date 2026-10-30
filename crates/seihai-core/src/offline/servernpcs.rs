//! Server NPCs online: the level NPCs a `PLO_NPCPROPS` names.
//!
//! The 2.05 client gets its level NPCs from the server, runs their scripts
//! itself, and tells the server what the scripts changed (lab capture, see
//! `documentation/ledges-push-pull.md`):
//!
//! * After a push or a pull, or a script's `x = …`, `PLI_NPCPROPS` with the
//!   new X or Y. The server passes it on to the other players in the level,
//!   so everyone sees the NPC where it was pushed.
//! * After `canbepushed`, `canbepulled` or `dontblock`, the NPC's BLOCKFLAGS.
//! * VISFLAGS for every NPC once its script has run (`hide`, `drawoverplayer`
//!   and `drawunderplayer` change it later), IMAGE after `setimg`, and
//!   MESSAGE after `message` (or `setcharprop #c`).
//! * A character NPC (`showcharacter`, `setcharani`): IMAGE `#c#`, its
//!   GANI, SPRITE (its `dir`), COLORS, HEAD, BODY and GANI attributes;
//!   NICKNAME after `setcharprop #n`, character or not. `#1` (sword) and
//!   `#2` (shield) are not sent.
//!
//! Seihai may also have the level file. An NPC of the file that has the
//! server NPC's image and position *is* that NPC and takes its id; a server
//! NPC with no such twin is added to the level.

use std::collections::HashMap;
use std::time::Duration;

use super::OfflineSession;
use crate::assets::{DrawLayer, Level, Npc};
use crate::protocol::npcprops::{
    half_tiles, npc_props_message, Head, NpcProps, PropValue, BLOCK_DONTBLOCK, BLOCK_PULLABLE, BLOCK_PUSHABLE, CHARACTER_IMAGE, DEFAULT_NPC_COLORS,
    NPCPROP_BLOCKFLAGS, NPCPROP_BODY, NPCPROP_COLORS, NPCPROP_GANI, NPCPROP_GATTRIB1, NPCPROP_HEAD, NPCPROP_IMAGE, NPCPROP_MESSAGE, NPCPROP_NICKNAME,
    NPCPROP_SPRITE, NPCPROP_VISFLAGS, NPCPROP_X, NPCPROP_Y, VIS_CLIENT, VIS_DEFAULT, VIS_OVER, VIS_UNDER, VIS_VISIBLE,
};

const OUR_BITS: u8 = BLOCK_DONTBLOCK | BLOCK_PULLABLE | BLOCK_PUSHABLE;

/// The BLOCKFLAGS byte for `n`: the bits Seihai keeps from its script,
/// over any other bits the server gave (`server`).
pub fn block_flags(n: &Npc, server: u8) -> u8 {
    let mut f = server & !OUR_BITS;
    if !n.blocks {
        f |= BLOCK_DONTBLOCK;
    }
    if n.pullable {
        f |= BLOCK_PULLABLE;
    }
    if n.pushable {
        f |= BLOCK_PUSHABLE;
    }
    f
}

const OUR_VIS: u8 = VIS_VISIBLE | VIS_OVER | VIS_UNDER;

/// The VISFLAGS byte for `n`: shown, over or under the players from its
/// script, the client bit, and the server's other bits (`base`).
pub fn vis_flags(n: &Npc, base: u8) -> u8 {
    let mut f = (base & !OUR_VIS) | VIS_CLIENT;
    if n.visible {
        f |= VIS_VISIBLE;
    }
    match n.layer {
        DrawLayer::Over => f |= VIS_OVER,
        DrawLayer::Under => f |= VIS_UNDER,
        _ => {}
    }
    f
}

/// The character props `n` would report, if it is a character NPC: GANI,
/// SPRITE, COLORS, HEAD, BODY and the five GANI attributes.
struct CharProps {
    gani: String,
    sprite: u8,
    colors: [u8; 5],
    head: Head,
    body: String,
    attrs: [String; 5],
}

fn char_props(n: &Npc) -> Option<CharProps> {
    let gani = n.ani.clone()?;
    let mut colors = DEFAULT_NPC_COLORS;
    for (i, c) in colors.iter_mut().enumerate() {
        if let Some(v) = n.props.get(&format!("#C{i}")).and_then(|v| crate::looks::color_value(v)) {
            *c = v;
        }
    }
    let head = match n.head.as_deref().map(str::trim).filter(|h| !h.is_empty()) {
        Some(h) => Head::Name(h.to_string()),
        None => Head::Index(0),
    };
    let body = n.body.clone().filter(|b| !b.trim().is_empty()).unwrap_or_else(|| "body.png".into());
    let attrs = [1, 2, 3, 4, 5].map(|i| n.props.get(&format!("#P{i}")).cloned().unwrap_or_default());
    Some(CharProps { gani, sprite: n.dir, colors, head, body, attrs })
}

/// Put the props a server record carries on a level NPC.
fn apply(n: &mut Npc, p: &NpcProps) {
    if p.image.as_deref() == Some(CHARACTER_IMAGE) {
        if n.ani.is_none() {
            n.ani = Some("idle".into());
        }
    } else if let Some(image) = &p.image {
        if !n.image.eq_ignore_ascii_case(image) {
            n.image = image.clone();
            n.image_part = None;
        }
    }
    if let Some(v) = p.vis_flags {
        n.visible = v & VIS_VISIBLE != 0;
        n.layer = if v & VIS_OVER != 0 {
            DrawLayer::Over
        } else if v & VIS_UNDER != 0 {
            DrawLayer::Under
        } else if matches!(n.layer, DrawLayer::Over | DrawLayer::Under) {
            DrawLayer::Player
        } else {
            n.layer
        };
    }
    if let Some(m) = &p.message {
        n.message = m.clone();
    }
    if let Some(g) = &p.gani {
        let g = g.trim_end_matches(".gani").to_ascii_lowercase();
        if !g.is_empty() && n.ani.as_deref() != Some(g.as_str()) {
            n.ani = Some(g);
            n.ani_frame = 0;
            n.ani_ms = 0;
        }
    }
    if let Some(d) = p.sprite {
        n.dir = d % 4;
    }
    if let Some(c) = p.colors {
        for (i, v) in c.iter().enumerate() {
            n.props.insert(format!("#C{i}"), v.to_string());
        }
    }
    if let Some(nick) = &p.nickname {
        n.props.insert("#n".into(), nick.clone());
    }
    match &p.head {
        Some(Head::Name(h)) => n.head = Some(h.clone()),
        Some(Head::Index(0)) => n.head = None,
        Some(Head::Index(i)) => n.head = Some(format!("head{i}.png")),
        None => {}
    }
    if let Some(b) = &p.body {
        n.body = Some(b.clone());
    }
    for (i, a) in p.attrs.iter().enumerate() {
        if let Some(a) = a {
            n.props.insert(format!("#P{}", i + 1), a.clone());
        }
    }
    if let Some(x) = p.x {
        n.x = x;
    }
    if let Some(y) = p.y {
        n.y = y;
    }
    if let Some(f) = p.block_flags {
        n.blocks = f & BLOCK_DONTBLOCK == 0;
        n.pullable = f & BLOCK_PULLABLE != 0;
        n.pushable = f & BLOCK_PUSHABLE != 0;
    }
}

/// What became of one server record (see [`OfflineSession::take_server_npc`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Linked {
    /// An NPC the level already had (by id, or the file's twin): its index.
    Known(usize),
    /// A new NPC at the end of the level's list.
    Added(usize),
    /// No level, or a partial record for an NPC the level does not have.
    None,
}

impl OfflineSession {
    /// Take one `PLO_NPCPROPS` record for NPC `id` into the current level.
    pub fn take_server_npc(&mut self, id: u32, p: &NpcProps) -> Linked {
        let Some(level) = self.current_level.as_mut() else { return Linked::None };
        if let Some(i) = level.npcs.iter().position(|n| n.server_id == Some(id)) {
            apply(&mut level.npcs[i], p);
            return Linked::Known(i);
        }
        let (Some(image), Some(x), Some(y)) = (p.image.as_deref(), p.x, p.y) else { return Linked::None };
        if let Some(i) = file_twin(level, image, x, y) {
            level.npcs[i].server_id = Some(id);
            apply(&mut level.npcs[i], p);
            return Linked::Known(i);
        }
        let mut n = Npc::new(image.to_string(), x, y, p.script.clone().unwrap_or_default());
        n.server_id = Some(id);
        apply(&mut n, p);
        level.npcs.push(n);
        Linked::Added(level.npcs.len() - 1)
    }
}

/// The level-file NPC with no server id yet, this image (any case) and this
/// position (to the half tile the server sends).
fn file_twin(level: &Level, image: &str, x: f32, y: f32) -> Option<usize> {
    level.npcs.iter().position(|n| {
        n.server_id.is_none() && n.image.eq_ignore_ascii_case(image) && half_tiles(n.x) == half_tiles(x) && half_tiles(n.y) == half_tiles(y)
    })
}

/// What the server last had for one linked NPC.
#[derive(Debug, Clone)]
struct Known {
    /// X and Y in half tiles.
    x: u8,
    y: u8,
    block: u8,
    /// VISFLAGS: `None` until the server or Seihai has said one.
    vis: Option<u8>,
    /// The server's VISFLAGS, for the bits Seihai does not set.
    vis_base: u8,
    image: String,
    message: String,
    nickname: String,
    gani: String,
    /// The NPC's `ani_calls` when GANI last went out.
    ani_calls: u32,
    sprite: u8,
    colors: Option<[u8; 5]>,
    head: Option<Head>,
    body: String,
    attrs: [String; 5],
}

/// How often the 2.05 client sends NPC props: at most every 100 ms. In
/// the lab an NPC that a 0.05 s timeout moved half a tile went out every
/// 0.1 s, a whole tile at a time (X), and an idle NPC sent nothing at all.
pub const NPC_SEND_INTERVAL: Duration = Duration::from_millis(100);

/// The values the server last had for each linked NPC. Anything the
/// level's NPC has that differs goes out.
#[derive(Debug, Default)]
pub struct ServerNpcSync {
    known: HashMap<u32, Known>,
    /// Time since the last send round ([`ServerNpcSync::tick`]).
    since: Duration,
}

impl ServerNpcSync {
    /// Forget everything (a new level).
    pub fn clear(&mut self) {
        self.known.clear();
    }

    /// The server sent `p` for `id`: those are now the values it has.
    pub fn heard(&mut self, id: u32, p: &NpcProps, n: &Npc) {
        let k = self.known.entry(id).or_insert_with(|| Known {
            x: half_tiles(n.x),
            y: half_tiles(n.y),
            block: 0,
            vis: None,
            vis_base: VIS_DEFAULT,
            image: n.image.clone(),
            message: String::new(),
            nickname: String::new(),
            gani: String::new(),
            ani_calls: n.ani_calls,
            sprite: 2,
            colors: None,
            head: None,
            body: String::new(),
            attrs: Default::default(),
        });
        if let Some(x) = p.x {
            k.x = half_tiles(x);
        }
        if let Some(y) = p.y {
            k.y = half_tiles(y);
        }
        if let Some(f) = p.block_flags {
            k.block = f;
        }
        if let Some(v) = p.vis_flags {
            k.vis = Some(v);
            k.vis_base = v;
        }
        if let Some(i) = &p.image {
            k.image = i.clone();
        }
        if let Some(m) = &p.message {
            k.message = m.clone();
        }
        if let Some(v) = &p.nickname {
            k.nickname = v.clone();
        }
        if let Some(v) = &p.gani {
            k.gani = v.clone();
        }
        if let Some(v) = p.sprite {
            k.sprite = v;
        }
        if p.colors.is_some() {
            k.colors = p.colors;
        }
        if p.head.is_some() {
            k.head = p.head.clone();
        }
        if let Some(v) = &p.body {
            k.body = v.clone();
        }
        for (k, a) in k.attrs.iter_mut().zip(&p.attrs) {
            if let Some(a) = a {
                *k = a.clone();
            }
        }
    }

    /// `dt` more went by: every [`NPC_SEND_INTERVAL`], the
    /// [`ServerNpcSync::changes`] since the last round; else nothing.
    pub fn tick(&mut self, level: &Level, dt: Duration) -> Vec<Vec<u8>> {
        self.since += dt;
        if self.since < NPC_SEND_INTERVAL {
            return Vec::new();
        }
        // A long frame does not owe the rounds it skipped.
        self.since = Duration::from_nanos((self.since.as_nanos() % NPC_SEND_INTERVAL.as_nanos()) as u64);
        self.changes(level)
    }

    /// `PLI_NPCPROPS` messages for every linked NPC of `level` whose props
    /// differ from what the server has; those become known.
    pub fn changes(&mut self, level: &Level) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for n in &level.npcs {
            let Some(id) = n.server_id else { continue };
            let Some(k) = self.known.get_mut(&id) else { continue };
            let mut props = Vec::new();
            let ch = char_props(n);
            let image = if ch.is_some() { CHARACTER_IMAGE } else { n.image.as_str() };
            if !image.eq_ignore_ascii_case(&k.image) {
                k.image = image.to_string();
                props.push((NPCPROP_IMAGE, PropValue::Text(image.to_string())));
            }
            for (prop, now, was) in [(NPCPROP_X, half_tiles(n.x), &mut k.x), (NPCPROP_Y, half_tiles(n.y), &mut k.y)] {
                if now != *was {
                    *was = now;
                    props.push((prop, PropValue::Byte(now)));
                }
            }
            let vis = vis_flags(n, k.vis_base);
            if k.vis != Some(vis) {
                k.vis = Some(vis);
                props.push((NPCPROP_VISFLAGS, PropValue::Byte(vis)));
            }
            let block = block_flags(n, k.block);
            if block != k.block {
                k.block = block;
                props.push((NPCPROP_BLOCKFLAGS, PropValue::Byte(block)));
            }
            if n.message != k.message {
                k.message = n.message.clone();
                props.push((NPCPROP_MESSAGE, PropValue::Text(n.message.clone())));
            }
            if let Some(c) = &ch {
                // A `setcharani` sends GANI again even when it is the same.
                let again = std::mem::replace(&mut k.ani_calls, n.ani_calls) != n.ani_calls;
                if again || !c.gani.eq_ignore_ascii_case(k.gani.trim_end_matches(".gani")) {
                    k.gani = c.gani.clone();
                    props.push((NPCPROP_GANI, PropValue::Text(c.gani.clone())));
                }
            }
            if let Some(c) = &ch {
                if c.sprite != k.sprite {
                    k.sprite = c.sprite;
                    props.push((NPCPROP_SPRITE, PropValue::Byte(c.sprite)));
                }
                if k.colors.unwrap_or(DEFAULT_NPC_COLORS) != c.colors {
                    k.colors = Some(c.colors);
                    props.push((NPCPROP_COLORS, PropValue::Bytes(c.colors.to_vec())));
                }
            }
            let nick = n.props.get("#n").cloned().unwrap_or_default();
            if nick != k.nickname {
                k.nickname = nick.clone();
                props.push((NPCPROP_NICKNAME, PropValue::Text(nick)));
            }
            if let Some(c) = ch {
                if k.head.as_ref() != Some(&c.head) {
                    k.head = Some(c.head.clone());
                    props.push((NPCPROP_HEAD, PropValue::Head(c.head)));
                }
                if !c.body.eq_ignore_ascii_case(&k.body) {
                    k.body = c.body.clone();
                    props.push((NPCPROP_BODY, PropValue::Text(c.body)));
                }
                for (i, (now, was)) in c.attrs.into_iter().zip(k.attrs.iter_mut()).enumerate() {
                    if now != *was {
                        *was = now.clone();
                        props.push((NPCPROP_GATTRIB1 + i as u8, PropValue::Text(now)));
                    }
                }
            }
            // The 2.05 client sends props in id order.
            props.sort_by_key(|(p, _)| *p);
            if !props.is_empty() {
                out.push(npc_props_message(id, &props));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(level: &str) -> OfflineSession {
        let mut s = OfflineSession::new();
        s.current_level = Some(Level::parse(level).unwrap());
        s
    }

    fn record(image: &str, x: f32, y: f32) -> NpcProps {
        NpcProps { image: Some(image.into()), script: Some("if (created) { canbepushed; }\n".into()), x: Some(x), y: Some(y), ..NpcProps::default() }
    }

    #[test]
    fn a_server_npc_with_no_twin_is_added() {
        let mut s = session("GLEVNW01\n");
        assert_eq!(s.take_server_npc(1001, &record("labblock.gif", 30.0, 25.0)), Linked::Added(0));
        let n = &s.current_level.as_ref().unwrap().npcs[0];
        assert_eq!((n.image.as_str(), n.x, n.y, n.server_id), ("labblock.gif", 30.0, 25.0, Some(1001)));
        assert!(n.script.contains("canbepushed"));
        // A later update moves it; a partial record for an unknown id adds nothing.
        assert_eq!(s.take_server_npc(1001, &NpcProps { y: Some(24.0), ..NpcProps::default() }), Linked::Known(0));
        assert_eq!(s.current_level.as_ref().unwrap().npcs[0].y, 24.0);
        assert_eq!(s.take_server_npc(7, &NpcProps { y: Some(1.0), ..NpcProps::default() }), Linked::None);
    }

    #[test]
    fn the_level_files_twin_takes_the_id() {
        let mut s = session("GLEVNW01\nNPC LabBlock.gif 30 25\n//\nNPCEND\nNPC other.png 30 25\n//\nNPCEND\n");
        assert_eq!(s.take_server_npc(1001, &record("labblock.gif", 30.0, 25.0)), Linked::Known(0));
        let l = s.current_level.as_ref().unwrap();
        assert_eq!(l.npcs.len(), 2);
        assert_eq!((l.npcs[0].server_id, l.npcs[1].server_id), (Some(1001), None));
        // Moved on the server: no twin, so it is the server's NPC.
        assert_eq!(s.take_server_npc(1002, &record("other.png", 31.0, 25.0)), Linked::Added(2));
    }

    #[test]
    fn server_block_flags_set_push_pull_and_block() {
        let mut s = session("GLEVNW01\n");
        let p = NpcProps { block_flags: Some(BLOCK_PULLABLE | BLOCK_DONTBLOCK), ..record("a.png", 1.0, 1.0) };
        s.take_server_npc(5, &p);
        let n = &s.current_level.as_ref().unwrap().npcs[0];
        assert!(n.pullable && !n.pushable && !n.blocks);
        assert_eq!(block_flags(n, 0x40), 0x40 | 5);
    }

    #[test]
    fn changes_send_what_the_script_and_a_push_changed_and_nothing_twice() {
        let mut s = session("GLEVNW01\n");
        let p = record("labblock.gif", 30.0, 25.0);
        s.take_server_npc(1001, &p);
        let mut sync = ServerNpcSync::default();
        sync.heard(1001, &p, &s.current_level.as_ref().unwrap().npcs[0]);
        // The first look: VISFLAGS 81 (the server gave none), as 2.05.
        assert_eq!(sync.changes(s.current_level.as_ref().unwrap()), vec![b"# '\x89-q\n".to_vec()]);
        assert!(sync.changes(s.current_level.as_ref().unwrap()).is_empty());
        // `canbepushed` ran: BLOCKFLAGS 8, as the 2.05 client sent it.
        s.current_level.as_mut().unwrap().npcs[0].pushable = true;
        assert_eq!(sync.changes(s.current_level.as_ref().unwrap()), vec![b"# '\x89.(\n".to_vec()]);
        // Pushed up one tile: Y only (48 half tiles).
        s.current_level.as_mut().unwrap().npcs[0].y = 24.0;
        assert_eq!(sync.changes(s.current_level.as_ref().unwrap()), vec![b"# '\x89#P\n".to_vec()]);
        assert!(sync.changes(s.current_level.as_ref().unwrap()).is_empty());
        // Another player moved it: the server's value is not sent back.
        let moved = NpcProps { x: Some(31.0), ..NpcProps::default() };
        s.take_server_npc(1001, &moved);
        sync.heard(1001, &moved, &s.current_level.as_ref().unwrap().npcs[0]);
        assert!(sync.changes(s.current_level.as_ref().unwrap()).is_empty());
    }

    #[test]
    fn npcs_without_a_server_id_send_nothing() {
        let s = session("GLEVNW01\nNPC a.png 3 3\n//\nNPCEND\n");
        let mut sync = ServerNpcSync::default();
        assert!(sync.changes(s.current_level.as_ref().unwrap()).is_empty());
    }

    /// The lab's VISFLAGS replies: server none → 81, 0 → 16, 64 → 80,
    /// 16 → nothing; `hide` 80, `drawoverplayer` 83, `drawunderplayer` 85.
    #[test]
    fn vis_flags_follow_the_2_05_client() {
        let first = |server: Option<u8>, f: &dyn Fn(&mut Npc)| {
            let mut s = session("GLEVNW01\n");
            let p = NpcProps { vis_flags: server, ..record("a.png", 1.0, 1.0) };
            s.take_server_npc(5, &p);
            f(&mut s.current_level.as_mut().unwrap().npcs[0]);
            let mut sync = ServerNpcSync::default();
            sync.heard(5, &p, &s.current_level.as_ref().unwrap().npcs[0]);
            sync.changes(s.current_level.as_ref().unwrap())
        };
        let vis = |v: u8| vec![[&b"#  %-"[..], &[v + 0x20], b"\n"].concat()];
        assert_eq!(first(None, &|_| {}), vis(81));
        assert_eq!(first(Some(0), &|_| {}), vis(16));
        assert_eq!(first(Some(64), &|_| {}), vis(80));
        assert!(first(Some(16), &|_| {}).is_empty());
        assert!(first(Some(81), &|_| {}).is_empty());
        assert_eq!(first(None, &|n| n.visible = false), vis(80));
        assert_eq!(first(None, &|n| n.layer = DrawLayer::Over), vis(83));
        assert_eq!(first(None, &|n| n.layer = DrawLayer::Under), vis(85));
        assert_eq!(first(None, &|n| n.layer = DrawLayer::Light), vis(81));
        assert_eq!(first(Some(1), &|n| n.visible = false), vis(16));
    }

    #[test]
    fn image_and_message_go_out_in_prop_order() {
        let mut s = session("GLEVNW01\n");
        let p = NpcProps { vis_flags: Some(81), ..record("a.png", 1.0, 1.0) };
        s.take_server_npc(1118, &p);
        let mut sync = ServerNpcSync::default();
        sync.heard(1118, &p, &s.current_level.as_ref().unwrap().npcs[0]);
        let n = &mut s.current_level.as_mut().unwrap().npcs[0];
        n.image = "bomb1.png".into();
        n.message = "hi".into();
        assert_eq!(sync.changes(s.current_level.as_ref().unwrap()), vec![b"# (~ )bomb1.png/\"hi\n".to_vec()]);
        // The server's message and image are taken, not sent back.
        let back = NpcProps { message: Some("yo".into()), image: Some("c.png".into()), ..NpcProps::default() };
        s.take_server_npc(1118, &back);
        sync.heard(1118, &back, &s.current_level.as_ref().unwrap().npcs[0]);
        assert_eq!(s.current_level.as_ref().unwrap().npcs[0].message, "yo");
        assert!(sync.changes(s.current_level.as_ref().unwrap()).is_empty());
    }
}
