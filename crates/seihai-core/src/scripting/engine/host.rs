//! Binds a [`ScriptWorld`] to an offline [`OfflineSession`]: level NPCs
//! become script objects, the player's numbers are copied in before each
//! event, and effects are applied back afterwards.

use super::vocab::Era;
use super::world::{Effect, ObjId, ObjKind, ObjSpec, ScriptWorld};
use crate::offline::{OfflineSession, FOOT_DX, FOOT_DY};
use crate::protocol::ClientVersion;

/// A weapon as the server sent it: name, image and client script.
pub struct WeaponScript<'a> {
    pub name: &'a str,
    pub image: &'a str,
    pub script: &'a str,
}

/// Whether a player flag a script changed goes to the server. `local.`
/// flags stay in the client (2.02 and later, Graal_analysis
/// `packet-layouts.md`), and `this.` flags belong to the NPC.
fn sent_to_server(name: &str) -> bool {
    !name.starts_with("local.") && !name.starts_with("this.")
}

/// Scripts for one running session.
pub struct SessionScripts {
    pub world: ScriptWorld,
    was_reading: bool,
    /// Effects the session cannot apply itself (sound, network, effects
    /// on the picture), for the caller to act on.
    pub unhandled: Vec<Effect>,
    /// Set when a script changed an NPC image (`setimg`, `showimg`), so the
    /// host can load the new picture. The host clears it.
    pub images_changed: bool,
    /// Lines from `sendrpgmessage`, for the player's message window.
    pub rpg_messages: Vec<String>,
}


/// Where the player's cell starts in gmap coordinates (tiles); `(0, 0)` off
/// a gmap.
fn gmap_origin(s: &OfflineSession) -> (f64, f64) {
    let side = crate::gmap::CELL_TILES as f64;
    s.gmap.as_ref().map_or((0.0, 0.0), |w| (w.cell.0 as f64 * side, w.cell.1 as f64 * side))
}

impl SessionScripts {
    pub fn new(version: ClientVersion) -> Self {
        SessionScripts { world: ScriptWorld::new(Era::for_version(version)), was_reading: false, unhandled: Vec::new(), images_changed: false, rpg_messages: Vec::new() }
    }

    /// Replace the level's scripts with those of `session`'s current
    /// level, run `created` on each NPC, then `playerenters` on all
    /// objects (weapons included).
    pub fn load_level(&mut self, session: &mut OfflineSession, level_name: &str) {
        self.world.clear_level();
        self.world.core.level = level_name.to_string();
        // A weapon's images belong to the level the player was on.
        session.player_images.clear();
        if let Some(level) = &session.current_level {
            for (i, n) in level.npcs.iter().enumerate() {
                let mut spec = ObjSpec::npc(&n.image, n.x as f64, n.y as f64, &n.script).with_tag(i);
                spec.kind = ObjKind::Npc;
                let id = self.world.add(spec);
                if let Some(o) = self.world.obj_mut(id) {
                    o.visible = n.visible;
                }
            }
        }
        let kinds: Option<Vec<crate::assets::TileKind>> = session.current_level.as_ref().zip(session.tiletypes.as_ref()).map(|(l, t)| {
            (0..64 * 64).map(|i| l.kind_at(t, (i % 64) as f32 + 0.5, (i / 64) as f32 + 0.5)).collect()
        });
        use crate::assets::TileKind::{Blocking, Lava, Water};
        self.world.core.walls = kinds.as_ref().map(|k| k.iter().map(|&k| k == Blocking).collect());
        self.world.core.water = kinds.map(|k| k.into_iter().map(|k| k == Water || k == Lava).collect());
        // The map for `onmapx` / `onmapy`: the gmap the player is on, else the big map.
        self.world.core.map = session.gmap.as_ref().map(|g| g.map.clone()).or_else(|| session.big_map.clone());
        self.sync_player(session);
        self.world.start_all();
        self.world.broadcast("playerenters");
        self.apply(session);
    }

    /// Make the world's weapon objects match the weapons the server sent.
    /// New ones run `created`.
    pub fn sync_weapons(&mut self, session: &mut OfflineSession, weapons: &[WeaponScript]) {
        let have: Vec<String> =
            self.world.ids().into_iter().filter_map(|i| self.world.obj(i)).filter(|o| o.kind == ObjKind::Weapon).map(|o| o.name.clone()).collect();
        for name in &have {
            if !weapons.iter().any(|w| w.name == name) {
                self.world.remove_weapon(name);
            }
        }
        let mut added = false;
        for w in weapons {
            if !have.iter().any(|h| h == w.name) {
                self.world.add(ObjSpec::weapon(w.name, w.image, w.script));
                added = true;
            }
        }
        if added {
            self.sync_player(session);
            self.world.start_all();
            self.apply(session);
        }
    }

    /// Copy the player's state into the world.
    ///
    /// On a gmap `playerx` and `playery` are gmap coordinates (cell * 64 +
    /// local): the real 2.22's `gr.x`, set by `-gr_movement` from `playerx`,
    /// runs past 128 in cell (2, 1) (lab, 2026-10-30).
    pub fn sync_player(&mut self, s: &OfflineSession) {
        // The board as it is now: cuts, lifts and the server's board
        // changes all show in `board[]` reads.
        self.world.core.board = s.current_level.as_ref().map(|l| (0..64 * 64).map(|i| l.tile_id_at(i % 64, i / 64).unwrap_or(0)).collect());
        let p = &mut self.world.core.player;
        let (ox, oy) = gmap_origin(s);
        p.x = s.player_x as f64 + ox;
        p.y = s.player_y as f64 + oy;
        p.dir = s.player_dir as f64;
        p.hearts = s.player_health as f64 / 100.0 * s.max_hearts as f64 / 2.0;
        p.hp = p.hearts;
        p.max_hp = s.max_hearts as f64 / 2.0;
        p.rupees = s.rupees as f64;
        p.bombs = s.bombs as f64;
        p.darts = s.darts as f64;
        p.sword_power = s.sword_power as f64;
        p.shield_power = s.shield_power as f64;
        let set = |st: &mut std::collections::BTreeSet<String>, name: &str, on: bool| {
            if on {
                st.insert(name.to_string());
            } else {
                st.remove(name);
            }
        };
        set(&mut p.status, "playerswimming", s.in_water);
        set(&mut p.status, "playerreading", s.reading.is_some());
        // Player flags set earlier in the session.
        for (k, v) in &s.flags {
            if *v && !self.world.core.flags.contains_key(k) {
                self.world.core.flags.insert(k.clone(), String::new());
            }
        }
    }

    /// The player's feet are where they are: fire `playertouchsme` on
    /// NPCs just touched. Returns how many fired.
    pub fn touch(&mut self, session: &mut OfflineSession) -> usize {
        self.sync_player(session);
        let n = self.world.touch((session.player_x + FOOT_DX) as f64, (session.player_y + FOOT_DY) as f64);
        self.apply(session);
        n
    }

    /// An arrow hit NPC `id`: `wasshot` there.
    pub fn shot(&mut self, session: &mut OfflineSession, id: ObjId) {
        self.sync_player(session);
        self.world.fire(id, "wasshot", &[]);
        self.apply(session);
    }

    /// A thrown object hit the NPC `id`: `waspelt` on it.
    pub fn pelt(&mut self, session: &mut OfflineSession, id: ObjId) {
        self.sync_player(session);
        self.world.fire(id, "waspelt", &[]);
        self.apply(session);
    }

    /// The player pushed or pulled at a point (2.05's `pushed` / `pulled`
    /// trigger action): `actionpushed` or `actionpulled` on the NPC there,
    /// with the direction as the parameter. True if an NPC was there.
    pub fn trigger(&mut self, session: &mut OfflineSession, t: &crate::offline::Trigger) -> bool {
        let Some(id) = self.world.npc_at(f64::from(t.x), f64::from(t.y)) else { return false };
        self.sync_player(session);
        self.world.fire(id, &format!("action{}", t.action), &[t.dir.to_string()]);
        self.apply(session);
        true
    }

    /// A push or a pull moved a level NPC: its script object follows, and
    /// gets `waspushed` / `waspulled`.
    pub fn npc_moved(&mut self, session: &mut OfflineSession, m: &crate::offline::NpcMoved) {
        let id = self.world.ids().into_iter().find(|&id| self.world.obj(id).is_some_and(|o| o.kind == ObjKind::Npc && o.tag == m.index));
        let Some(id) = id else { return };
        if let Some(o) = self.world.obj_mut(id) {
            o.x = f64::from(m.x);
            o.y = f64::from(m.y);
        }
        self.sync_player(session);
        self.world.fire(id, m.event, &[]);
        self.apply(session);
    }

    /// Level NPC `index` arrived after the level's scripts started (a
    /// server NPC online): it runs `created`, then `playerenters`.
    pub fn add_npc(&mut self, session: &mut OfflineSession, index: usize) {
        let Some(n) = session.current_level.as_ref().and_then(|l| l.npcs.get(index)) else { return };
        let mut spec = ObjSpec::npc(&n.image, n.x as f64, n.y as f64, &n.script).with_tag(index);
        spec.kind = ObjKind::Npc;
        let visible = n.visible;
        let id = self.world.add(spec);
        if let Some(o) = self.world.obj_mut(id) {
            o.visible = visible;
        }
        self.sync_player(session);
        self.world.fire(id, "created", &[]);
        self.world.fire(id, "playerenters", &[]);
        self.apply(session);
    }

    /// The server moved level NPC `index` (another player pushed it): the
    /// script sees the new `x` and `y`. No event runs.
    pub fn set_npc_pos(&mut self, index: usize, x: f32, y: f32) {
        let id = self.world.ids().into_iter().find(|&id| self.world.obj(id).is_some_and(|o| o.kind == ObjKind::Npc && o.tag == index));
        if let Some(o) = id.and_then(|id| self.world.obj_mut(id)) {
            o.x = f64::from(x);
            o.y = f64::from(y);
        }
    }

    /// Advance script time by `ms` and run what falls due. Also notices
    /// the end of sign reading (`playerendsreading`).
    pub fn tick(&mut self, session: &mut OfflineSession, ms: u64) {
        self.sync_player(session);
        let reading = session.reading.is_some();
        if self.was_reading && !reading {
            self.world.broadcast("playerendsreading");
        }
        self.was_reading = reading;
        self.world.tick(ms);
        self.apply(session);
    }

    /// The player's death started: `playerdies` on every object (the event
    /// exists from 2.01 on; the world drops it for older scripts).
    pub fn player_dies(&mut self, session: &mut OfflineSession) {
        self.sync_player(session);
        self.world.broadcast("playerdies");
        self.apply(session);
    }

    /// The player typed a chat line: `playerchats` runs on every object, and
    /// the text to send is returned.
    ///
    /// A script can change that text with `setplayerprop #c,...` (or
    /// `player.chat = ...`). The real client does not put the typed line on
    /// the wire until its scripts have run, so the changed text is what goes
    /// out (and what the bubble shows). This is how worlds stop "unstick me"
    /// in a jail: a client-side NPC rewrites the line and the server never
    /// sees it (lab capture, `documentation/unstick-me.md`). Without a
    /// script change the typed text comes back as it was.
    pub fn chat(&mut self, session: &mut OfflineSession, text: &str) -> String {
        // A "#c" left by an earlier, unrelated script is not an answer to
        // this line.
        session.props.remove("#c");
        self.sync_player(session);
        self.world.chat(text);
        self.apply(session);
        session.props.remove("#c").unwrap_or_else(|| text.to_string())
    }

    /// Apply the world's pending effects to the session. What the session
    /// has no place for goes to [`SessionScripts::unhandled`].
    pub fn apply(&mut self, s: &mut OfflineSession) {
        // The scripts' `dir` of each NPC (character NPCs face it; online
        // it goes to the server as SPRITE).
        if let Some(level) = s.current_level.as_mut() {
            for id in self.world.ids() {
                if let Some(o) = self.world.obj(id).filter(|o| o.kind == ObjKind::Npc) {
                    if let (Some(n), Some(d)) = (level.npcs.get_mut(o.tag), o.vars.get("dir")) {
                        n.dir = (d.num().round() as i64).rem_euclid(4) as u8;
                    }
                }
            }
        }
        for e in self.world.take_effects() {
            match e {
                Effect::Message { obj, text } => {
                    if let Some(n) = self.level_npc(s, obj) {
                        n.message = text.clone();
                    }
                    s.last_message = Some(text);
                }
                Effect::Say2 { text } => {
                    if s.reading.is_none() {
                        s.reading = Some(text);
                    }
                }
                Effect::SaySign { index } => {
                    let text = s.current_level.as_ref().and_then(|l| usize::try_from(index).ok().and_then(|i| l.signs.get(i))).map(|g| g.text.clone());
                    if let (Some(t), true) = (text, s.reading.is_none()) {
                        s.reading = Some(t);
                    }
                }
                Effect::SetVisible { obj, visible, .. } => {
                    if let Some(n) = self.level_npc(s, obj) {
                        n.visible = visible;
                    }
                }
                // `setimg`, `setgif`, `setimgpart`, `setgifpart`: the NPC's image (the
                // whole of it, or the part), and the NPC is shown. A name ending
                // in `.gani` picks that animation instead (Seihai's convention, see
                // `documentation/npc-images.md`); a plain image takes the NPC out
                // of any animation.
                Effect::SetImg { obj, file, part } => {
                    self.images_changed = true;
                    if let Some(n) = self.level_npc(s, obj) {
                        n.visible = true;
                        if file.to_ascii_lowercase().ends_with(".gani") {
                            set_ani(n, &file);
                        } else {
                            n.image = file;
                            n.image_part = part.map(|(x, y, w, h)| (x as i32, y as i32, w as i32, h as i32));
                            n.ani = None;
                        }
                    }
                }
                Effect::SetCharAni { obj, ani, .. } => {
                    if let Some(n) = self.level_npc(s, obj) {
                        set_ani(n, &ani);
                        n.ani_calls = n.ani_calls.wrapping_add(1);
                    }
                }
                // `setani name,params` is the same for an NPC that is not a character.
                Effect::Command { obj, ref name, ref args } if name == "setani" => {
                    let a = args.first().map(|v| v.text()).unwrap_or_default();
                    if let Some(n) = self.level_npc(s, obj) {
                        set_ani(n, &a);
                    }
                }
                // `showcharacter`: the NPC is drawn as a character; idle until a
                // `setcharani` says otherwise.
                Effect::Command { obj, ref name, .. } if name == "showcharacter" => {
                    if let Some(n) = self.level_npc(s, obj) {
                        if n.ani.is_none() {
                            set_ani(n, "idle");
                        }
                    }
                }
                // `showimg index,file,x,y`: image `index` at a level position. Using
                // an index again reuses that image: as in 2.05 it takes the new
                // file and place, its part is cleared and its height goes back to
                // one over the NPC (over the player for a weapon); its colour and
                // zoom stay.
                Effect::Command { obj, ref name, ref args } if name == "showimg" => {
                    self.images_changed = true;
                    let num = |i: usize| args.get(i).map_or(0.0, |v| v.num());
                    if let Some((images, owner)) = self.images_of(s, obj) {
                        let img = crate::assets::NpcImage {
                            index: num(0) as i64,
                            file: args.get(1).map(|v| v.text()).unwrap_or_default(),
                            x: num(2) as f32,
                            y: num(3) as f32,
                            part: None,
                            effect: Default::default(),
                            height: owner + 1,
                        };
                        match images.iter_mut().find(|i| i.index == img.index) {
                            Some(old) => *old = crate::assets::NpcImage { effect: old.effect, ..img },
                            None => {
                                images.push(img);
                                images.sort_by_key(|i| i.index);
                            }
                        }
                    }
                }
                // `changeimgpart index,x,y,w,h`: the part of image `index` that shows.
                Effect::Command { obj, ref name, ref args } if name == "changeimgpart" => {
                    let num = |i: usize| args.get(i).map_or(0.0, |v| v.num());
                    let index = num(0) as i64;
                    let part = (num(1).clamp(0.0, 16000.0) as i32, num(2).clamp(0.0, 16000.0) as i32, num(3).clamp(0.0, 220.0) as i32, num(4).clamp(0.0, 220.0) as i32);
                    if let Some((images, _)) = self.images_of(s, obj) {
                        if let Some(i) = images.iter_mut().find(|i| i.index == index) {
                            i.part = Some(part);
                        }
                    }
                }
                // `changeimgvis index,vis`: the image's height against its NPC
                // (`documentation/npc-draw-order.md`).
                Effect::Command { obj, ref name, ref args } if name == "changeimgvis" => {
                    let num = |i: usize| args.get(i).map_or(0.0, |v| v.num()).round() as i64;
                    if let Some((images, owner)) = self.images_of(s, obj) {
                        if let Some(h) = crate::assets::image_vis_height(num(1), owner) {
                            if let Some(i) = images.iter_mut().find(|i| i.index == num(0)) {
                                i.height = h;
                            }
                        }
                    }
                }
                // `hideimg index`: image `index` is deleted.
                Effect::Command { obj, ref name, ref args } if name == "hideimg" => {
                    self.images_changed = true;
                    let index = args.first().map_or(0.0, |v| v.num()) as i64;
                    if let Some((images, _)) = self.images_of(s, obj) {
                        images.retain(|i| i.index != index);
                    }
                }
                // The world has already moved the object; copy its position.
                Effect::SetPos { obj, .. } | Effect::Move { obj, .. } => {
                    let pos = self.world.obj(obj).map(|o| (o.x as f32, o.y as f32));
                    if let (Some((x, y)), Some(n)) = (pos, self.level_npc(s, obj)) {
                        n.x = x;
                        n.y = y;
                    }
                }
                Effect::Destroy { obj } => {
                    if let Some(n) = self.level_npc(s, obj) {
                        n.visible = false;
                    }
                }
                Effect::SetLevel { level, x, y } => {
                    let (px, py) = (x.map_or(s.player_x, |v| v as f32), y.map_or(s.player_y, |v| v as f32));
                    s.pending_link = Some((level, px, py));
                }
                Effect::Hurt { half_hearts } => {
                    // A dead player cannot be hurt.
                    if s.life.is_dead() {
                        continue;
                    }
                    let per_half = 100.0 / s.max_hearts.max(1) as f64;
                    let before = s.player_health;
                    s.player_health = (s.player_health as f64 - half_hearts * per_half).round().clamp(0.0, 100.0) as i32;
                    // A script that would kill: log it, and leave one point
                    // so movement is not locked while the cause is found.
                    if s.player_health == 0 && before > 0 {
                        eprintln!("script hurt: {half_hearts} half hearts, health {before} -> 0 (left at 1)");
                        s.player_health = 1;
                    }
                }
                Effect::SetPlayerVar { name, value } => match name.as_str() {
                    "playerx" => s.player_x = (value - gmap_origin(s).0) as f32,
                    "playery" => s.player_y = (value - gmap_origin(s).1) as f32,
                    "playerdir" => s.player_dir = value as u8 % 4,
                    "playerrupees" => s.rupees = value as i32,
                    "playerbombs" => s.bombs = value as i32,
                    "playerdarts" => s.darts = value as i32,
                    "playerswordpower" => s.sword_power = value.clamp(0.0, 255.0) as u8,
                    "playershieldpower" => s.shield_power = value.clamp(0.0, 255.0) as u8,
                    "playerhearts" | "playerhp" => {
                        s.player_health = (value * 2.0 / s.max_hearts.max(1) as f64 * 100.0).round().clamp(0.0, 100.0) as i32;
                    }
                    _ => {}
                },
                Effect::SetFlag { name, value } => {
                    let value = value.unwrap_or_default();
                    let was_set = s.flags.get(&name) == Some(&true);
                    let changed = !was_set || s.strings.get(&name).map_or("", String::as_str) != value;
                    s.flags.insert(name.clone(), true);
                    if value.is_empty() {
                        s.strings.remove(&name);
                    } else {
                        s.strings.insert(name.clone(), value.clone());
                    }
                    if changed && sent_to_server(&name) {
                        s.flags_out.push((name, Some(value)));
                    }
                }
                Effect::UnsetFlag { name } => {
                    let was_set = s.flags.get(&name) == Some(&true);
                    s.flags.insert(name.clone(), false);
                    s.strings.remove(&name);
                    if was_set && sent_to_server(&name) {
                        s.flags_out.push((name, None));
                    }
                }
                Effect::SetPlayerProp { code, value } => {
                    // The looks codes change the player as the looks commands
                    // do (gs1 reference: `#3` head, `#8` body, `#C0`..`#C4`
                    // colours by name or number, `#P1`..`#P5` attributes).
                    let v = value.trim();
                    let changed = match code.as_str() {
                        "#3" if !v.is_empty() => {
                            s.looks.head = Some(v.to_string());
                            true
                        }
                        "#8" if !v.is_empty() => {
                            s.looks.body = Some(v.to_string());
                            true
                        }
                        c if c.len() == 3 && c.starts_with("#C") => {
                            let slot = usize::from(c.as_bytes()[2].wrapping_sub(b'0'));
                            let color = crate::looks::color_value(v);
                            match (s.looks.colors.get_mut(slot), color) {
                                (Some(old), Some(c)) => {
                                    *old = c;
                                    true
                                }
                                _ => false,
                            }
                        }
                        c if c.len() == 3 && c.starts_with("#P") && (b'1'..=b'5').contains(&c.as_bytes()[2]) => {
                            s.looks.attrs[usize::from(c.as_bytes()[2] - b'1')] = Some(v.to_string());
                            true
                        }
                        _ => false,
                    };
                    s.looks_changed |= changed;
                    s.props.insert(code, value);
                }
                // `noplayerkilling`: other players' hits do nothing in this level.
                Effect::Command { ref name, .. } if name == "noplayerkilling" => s.no_pk = true,
                Effect::Command { obj, ref name, ref args } if name == "putbomb" => {
                    let n = |i: usize| args.get(i).map_or(0.0, |v| v.num());
                    let from_weapon = self.world.obj(obj).is_some_and(|o| o.kind == ObjKind::Weapon);
                    s.put_bomb(n(0).clamp(0.0, 7.0) as u8, n(1) as f32, n(2) as f32, from_weapon);
                }
                Effect::Command { obj, ref name, ref args } if name == "shootarrow" => {
                    let dir = args.first().map_or(0.0, |v| v.num()).rem_euclid(4.0) as u8;
                    match self.world.obj(obj) {
                        // A weapon shoots from the player.
                        Some(o) if o.kind == ObjKind::Weapon => {
                            let (x, y) = (s.player_x, s.player_y);
                            s.shoot_arrow(x, y, crate::arrows::SHOOTER_W, crate::arrows::SHOOTER_H, dir, true);
                        }
                        Some(o) => {
                            let (x, y, w, h) = o.rect();
                            s.shoot_arrow(x as f32, y as f32, w as f32, h as f32, dir, false);
                        }
                        None => {}
                    }
                }
                // `setsword power`: the player's sword strength.
                Effect::Command { ref name, ref args, .. } if name == "setsword" => {
                    if let Some(p) = args.get(1).map(|v| v.num()) {
                        s.sword_power = p.clamp(0.0, 255.0) as u8;
                    }
                }
                // `toweapons name`: the player gains that weapon.
                Effect::Command { ref name, ref args, .. } if name == "toweapons" => {
                    if let Some(n) = args.first().map(|v| v.text()) {
                        if !n.is_empty() && !s.weapons.iter().any(|w| w.eq_ignore_ascii_case(&n)) {
                            s.weapons.push(n);
                        }
                    }
                }
                // The player's looks (`documentation/appearance.md`). In 2.05
                // these change the player whichever object runs them.
                Effect::Command { ref name, ref args, .. } if name == "sethead" || name == "setbody" => {
                    let pic = args.first().map(|v| v.text()).unwrap_or_default();
                    let pic = pic.trim();
                    if !pic.is_empty() {
                        let slot = if name == "sethead" { &mut s.looks.head } else { &mut s.looks.body };
                        *slot = Some(pic.to_string());
                        s.looks_changed = true;
                    }
                }
                Effect::Command { ref name, ref args, .. } if crate::looks::color_slot(name).is_some() => {
                    if let (Some(slot), Some(c)) = (crate::looks::color_slot(name), args.first().map(|v| v.num())) {
                        let c = c.round();
                        if (0.0..20.0).contains(&c) {
                            s.looks.colors[slot] = c as u8;
                            s.looks_changed = true;
                        }
                    }
                }
                // Character props on an NPC: head, body, colors, shield.
                Effect::SetCharProp { obj, code, value } => {
                    if let Some(n) = self.level_npc(s, obj) {
                        match code.as_str() {
                            "#3" => n.head = Some(value),
                            "#8" => n.body = Some(value),
                            "#2" => n.shield = Some(value),
                            "#c" => {
                                n.message = value.clone();
                                n.props.insert(code, value);
                            }
                            _ => { n.props.insert(code, value); }
                        }
                    }
                }
                // `canbepushed` and `canbepulled` (and their `cannot` forms):
                // whether a push or a pull moves the NPC.
                Effect::Command { obj, ref name, .. }
                    if matches!(name.as_str(), "canbepushed" | "cannotbepushed" | "canbepulled" | "cannotbepulled") =>
                {
                    if let Some(n) = self.level_npc(s, obj) {
                        match name.as_str() {
                            "canbepushed" => n.pushable = true,
                            "cannotbepushed" => n.pushable = false,
                            "canbepulled" => n.pullable = true,
                            _ => n.pullable = false,
                        }
                    }
                }
                // `dontblock`: the NPC does not stop movement.
                Effect::Command { obj, ref name, .. } if name == "dontblock" => {
                    if let Some(n) = self.level_npc(s, obj) {
                        n.blocks = false;
                    }
                }
                // The drawing height (see `documentation/npc-draw-order.md`).
                Effect::Command { obj, ref name, .. }
                    if matches!(name.as_str(), "drawunderplayer" | "drawoverplayer" | "drawaslight") =>
                {
                    let layer = match name.as_str() {
                        "drawunderplayer" => crate::assets::DrawLayer::Under,
                        "drawoverplayer" => crate::assets::DrawLayer::Over,
                        _ => crate::assets::DrawLayer::Light,
                    };
                    if let Some(n) = self.level_npc(s, obj) {
                        n.layer = layer;
                    }
                }
                // `blockagain` and `blockagainlocal` undo `dontblock` and put the
                // NPC back with the players, whatever its drawing height was.
                Effect::Command { obj, ref name, .. } if name == "blockagain" || name == "blockagainlocal" => {
                    if let Some(n) = self.level_npc(s, obj) {
                        n.blocks = true;
                        n.layer = crate::assets::DrawLayer::Player;
                    }
                }
                // Colour and zoom effects (see `documentation/visual-effects.md`).
                Effect::Command { obj, ref name, ref args }
                    if matches!(name.as_str(), "setcoloreffect" | "setzoomeffect" | "changeimgcolors" | "changeimgzoom") =>
                {
                    let num = |i: usize| args.get(i).map_or(0.0, |v| v.num()) as f32;
                    let set = |e: &mut crate::assets::ImageEffect, from: usize, color: bool| {
                        if color {
                            e.color = [num(from), num(from + 1), num(from + 2), num(from + 3)];
                        } else {
                            e.zoom = num(from);
                        }
                    };
                    let color = name.ends_with("coloreffect") || name == "changeimgcolors";
                    if name.starts_with("set") {
                        if let Some(n) = self.level_npc(s, obj) {
                            set(&mut n.effect, 0, color);
                        }
                    } else if let Some((images, _)) = self.images_of(s, obj) {
                        let index = num(0) as i64;
                        if let Some(i) = images.iter_mut().find(|i| i.index == index) {
                            set(&mut i.effect, 1, color);
                        }
                    }
                }
                // `seteffect r,g,b,a`: the whole map is shaded with that colour.
                Effect::Command { ref name, ref args, .. } if name == "seteffect" => {
                    let num = |i: usize| args.get(i).map_or(0.0, |v| v.num()) as f32;
                    s.screen_effect = [num(0), num(1), num(2), num(3)];
                }
                // Collision shape for an NPC.
                Effect::SetShape { obj, w, h, .. } => {
                    if let Some(n) = self.level_npc(s, obj) {
                        n.shape = Some((w as f32, h as f32));
                    }
                }
                // Visual commands. Recognized so they do not fill the log;
                // drawing them is still open.
                Effect::Command { ref name, .. }
                    if matches!(name.as_str(),
                        "showtext" | "hideimgs" | "setbackpal"
                        | "replaceani" | "addtiledef" | "addtiledef2"
                        | "removetiledefs" | "setchargender" | "enabledefmovement"
                        | "timereverywhere"
                    ) => {}
                // `play`, `playlooped`, `play2`: queued with the GANI cues. 2.05
                // places the sound at the NPC's centre; a weapon has no place of
                // its own, so its sound is at the player (centre pan).
                Effect::Play { obj, file, looped, at } => {
                    let (x, y) = match at {
                        Some((x, y)) => (x as f32, y as f32),
                        None => self.sound_point(s, obj),
                    };
                    let file = if looped { format!("+{}", file.trim()) } else { file };
                    s.sound_events.push(crate::sound::SoundEvent { file, x, y });
                }
                Effect::StopSound { file } => s.sound_stops.push(file),
                Effect::StopMidi => {}
                Effect::RpgMessage { text } => self.rpg_messages.push(text),
                other => self.unhandled.push(other),
            }
        }
    }

    /// Where an object's sound comes from: an NPC's centre, else the
    /// player's listening point. The NPC's size is its image part or its
    /// shape; with neither (the image's size is not known here) a 32-pixel
    /// image is assumed. Only x sets the pan, at 150 units a tile, so a
    /// wrong guess moves it little.
    fn sound_point(&self, s: &mut OfflineSession, obj: usize) -> (f32, f32) {
        let listener = (s.player_x + crate::assets::LISTENER_DX, s.player_y + crate::assets::LISTENER_DY);
        let Some(n) = self.level_npc(s, obj) else { return listener };
        let (w, h) = match (n.image_part, n.shape) {
            (Some((_, _, w, h)), _) => (w as f32 / 16.0, h as f32 / 16.0),
            (None, Some((w, h))) => (w / 16.0, h / 16.0),
            (None, None) => (2.0, 2.0),
        };
        (n.x + w / 2.0, n.y + h / 2.0)
    }

    fn level_npc<'s>(&self, s: &'s mut OfflineSession, obj: usize) -> Option<&'s mut crate::assets::Npc> {
        let o = self.world.obj(obj)?;
        if o.kind != ObjKind::Npc {
            return None;
        }
        s.current_level.as_mut()?.npcs.get_mut(o.tag)
    }

    /// The `showimg` images of `obj` and the height of what owns them: a
    /// level NPC's own images, or for a weapon the player's, at the
    /// player's height ("weapon NPCs act as the player": Graal_analysis
    /// `npc-library/gs1-command-reference-205.md`).
    fn images_of<'s>(&self, s: &'s mut OfflineSession, obj: usize) -> Option<(&'s mut Vec<crate::assets::NpcImage>, i32)> {
        let o = self.world.obj(obj)?;
        match o.kind {
            ObjKind::Npc => s.current_level.as_mut()?.npcs.get_mut(o.tag).map(|n| (&mut n.images, n.layer.height())),
            ObjKind::Weapon => Some((&mut s.player_images, crate::assets::DrawLayer::Player.height())),
        }
    }
}

/// Put an NPC in the animation a script named (see `npcart::ani_name`).
fn set_ani(n: &mut crate::assets::Npc, raw: &str) {
    if let Some(name) = crate::npcart::ani_name(raw) {
        if n.ani.as_deref() != Some(name.as_str()) {
            n.ani = Some(name);
            n.ani_frame = 0;
            n.ani_ms = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::Level;

    fn session(level_text: &str) -> OfflineSession {
        let mut s = OfflineSession::new();
        s.current_level = Some(Level::parse(level_text).unwrap());
        s
    }

    const DOOR: &str = "GLEVNW01\n\
        NPC door.png 10 20\n\
        if (created) { setshape 1,32,32; }\n\
        if (playertouchsme) { hide; setlevel2 house.nw,32,60; }\n\
        NPCEND\n\
        NPC - 30 30\n\
        if (playerenters) { message welcome; }\n\
        if (playertouchsme) { hurt 2; playerrupees += 3; set seen; say2 Ouch!; }\n\
        NPCEND\n\
        SIGN 5 5\n\
        Read me\n\
        SIGNEND\n";

    #[test]
    fn noplayerkilling_marks_the_level_until_the_next_one_loads() {
        let mut s = session("GLEVNW01\nNPC - 10 10\nif (created) { noplayerkilling; }\nNPCEND\n");
        let mut sc = SessionScripts::new(ClientVersion::V205);
        assert!(!s.no_pk);
        sc.load_level(&mut s, "pk.nw");
        assert!(s.no_pk);
        let dir = std::env::temp_dir().join(format!("seihai-nopk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("next.nw"), "GLEVNW01\n").unwrap();
        s.load_level(dir.join("next.nw")).unwrap();
        assert!(!s.no_pk, "a new level starts without the flag");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `canbepushed` marks the level NPC; a push that moves it gives the
    /// script `waspushed` at its new place.
    #[test]
    fn canbepushed_npcs_move_and_hear_waspushed() {
        let mut s = session("GLEVNW01\nNPC - 20 10\nif (created) { canbepushed; canbepulled; cannotbepulled; }\nif (waspushed) { message moved; }\nNPCEND\n");
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "push.nw");
        let n = &s.current_level.as_ref().unwrap().npcs[0];
        assert!(n.pushable && !n.pullable);
        let m = crate::offline::NpcMoved { index: 0, x: 20.0, y: 9.0, event: "waspushed" };
        sc.npc_moved(&mut s, &m);
        assert_eq!(s.last_message.as_deref(), Some("moved"));
        let id = sc.world.npc_at(21.0, 9.5).expect("the script object moved with it");
        assert_eq!(sc.world.obj(id).map(|o| o.y), Some(9.0));
    }

    #[test]
    fn a_dead_player_cannot_be_hurt() {
        let mut s = session(DOOR);
        s.life.set_rules(Some(crate::protocol::DEATH_205));
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "town.nw");
        s.set_hearts_half(0);
        s.tick_life(0);
        assert!(s.life.is_dead());
        let before = s.player_health;
        s.player_x = 30.0 - FOOT_DX + 0.5;
        s.player_y = 30.0 - FOOT_DY + 0.5;
        sc.touch(&mut s);
        assert_eq!(s.player_health, before, "the touch script's hurt 2 did nothing");
    }

    #[test]
    fn playerdies_reaches_the_scripts() {
        let mut s = OfflineSession::new();
        s.current_level =
            Some(Level::parse("GLEVNW01\nNPC - 1 1\nif (playerdies) { message gone; }\nNPCEND\n").unwrap());
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "x.nw");
        assert!(s.last_message.is_none());
        sc.player_dies(&mut s);
        assert_eq!(s.last_message.as_deref(), Some("gone"));
    }

    #[test]
    fn level_load_runs_created_and_playerenters() {
        let mut s = session(DOOR);
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "town.nw");
        assert_eq!(s.last_message.as_deref(), Some("welcome"));
        assert_eq!(sc.world.ids().len(), 2);
        assert_eq!(sc.world.core.level, "town.nw");
    }

    #[test]
    fn touching_a_door_hides_it_and_asks_for_the_warp() {
        let mut s = session(DOOR);
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "town.nw");
        s.player_x = 10.0 - FOOT_DX + 0.5;
        s.player_y = 20.0 - FOOT_DY + 0.5;
        assert_eq!(sc.touch(&mut s), 1);
        assert!(!s.current_level.as_ref().unwrap().npcs[0].visible);
        assert_eq!(s.pending_link, Some(("house.nw".to_string(), 32.0, 60.0)));
        // Standing still does not fire again.
        assert_eq!(sc.touch(&mut s), 0);
    }

    #[test]
    fn effects_reach_the_session() {
        let mut s = session(DOOR);
        s.rupees = 4;
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "town.nw");
        s.player_x = 30.0 - FOOT_DX + 0.5;
        s.player_y = 30.0 - FOOT_DY + 0.5;
        sc.touch(&mut s);
        assert_eq!(s.rupees, 7);
        // Two half hearts of six: a third of the bar.
        assert_eq!(s.player_health, 67);
        assert_eq!(s.flags.get("seen"), Some(&true));
        assert_eq!(s.reading.as_deref(), Some("Ouch!"));
    }

    #[test]
    fn say_shows_a_level_sign_and_end_of_reading_is_an_event() {
        let mut s = session("GLEVNW01\nNPC - 1 1\nif (created) { say 0; }\nif (playerendsreading) { message closed; }\nNPCEND\nSIGN 5 5\nRead me\nSIGNEND\n");
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "a.nw");
        assert_eq!(s.reading.as_deref(), Some("Read me"));
        sc.tick(&mut s, 50);
        assert_eq!(s.last_message, None);
        s.reading = None;
        sc.tick(&mut s, 50);
        assert_eq!(s.last_message.as_deref(), Some("closed"));
    }

    #[test]
    fn timeouts_run_through_the_session_clock() {
        let mut s = session("GLEVNW01\nNPC - 1 1\nif (created) { hide; timeout = 2; }\nif (timeout) { show; }\nNPCEND\n");
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "a.nw");
        assert!(!s.current_level.as_ref().unwrap().npcs[0].visible);
        for _ in 0..39 {
            sc.tick(&mut s, 50);
        }
        assert!(!s.current_level.as_ref().unwrap().npcs[0].visible);
        sc.tick(&mut s, 50);
        assert!(s.current_level.as_ref().unwrap().npcs[0].visible);
    }

    #[test]
    fn the_builtin_bomb_puts_a_bomb_in_front_and_takes_one() {
        let mut s = session("GLEVNW01\n");
        s.player_x = 30.0;
        s.player_y = 30.0;
        s.player_dir = 2;
        s.bombs = 5;
        let mut sc = SessionScripts::new(ClientVersion::V205);
        let script = crate::bombs::builtin_script(8).unwrap();
        sc.sync_weapons(&mut s, &[WeaponScript { name: "Bomb", image: "wbomb1.png", script: &script }]);
        sc.sync_player(&s);
        sc.world.weapon_fired("Bomb");
        sc.apply(&mut s);
        assert_eq!(s.bombs, 4);
        assert_eq!(s.placed_bombs.len(), 1);
        assert_eq!((s.placed_bombs[0].x, s.placed_bombs[0].y, s.placed_bombs[0].power), (30.5, 32.5, 1));
        // a weapon's bomb: power 1 + 8, the fuse 55
        assert_eq!(s.take_bomb_sends(), vec![vec![0x24, 61 + 0x20, 65 + 0x20, 0x29, 0x57, b'\n']]);
        // no bombs, no bomb
        s.bombs = 0;
        sc.sync_player(&s);
        sc.world.weapon_fired("Bomb");
        sc.apply(&mut s);
        assert_eq!(s.placed_bombs.len(), 1);
        // the fuse burns 61 ticks, then the cross is out and the bomb gone
        s.tick_bombs(61 * 50);
        assert!(s.placed_bombs.is_empty());
        assert_eq!(s.blast.len(), 9);
        assert!(s.take_sound_events().iter().any(|e| e.file == "bomb.wav"));
    }

    #[test]
    fn the_builtin_bow_shoots_after_its_timeout_and_takes_an_arrow() {
        let mut s = session("GLEVNW01\n");
        s.player_x = 30.0;
        s.player_y = 30.0;
        s.player_dir = 3;
        s.darts = 5;
        let mut sc = SessionScripts::new(ClientVersion::V205);
        let script = crate::arrows::builtin_script(7).unwrap();
        sc.sync_weapons(&mut s, &[WeaponScript { name: "Bow", image: "wbowi1.png", script }]);
        sc.sync_player(&s);
        sc.world.weapon_fired("Bow");
        sc.apply(&mut s);
        assert!(s.arrows.is_empty());
        sc.tick(&mut s, 300);
        assert_eq!(s.darts, 4);
        assert_eq!(s.arrows.len(), 1);
        assert_eq!(s.arrows[0].sprite, 110);
        // facing right, a weapon's arrow (3 + 8), power 0
        let sent = s.take_arrow_sends();
        assert_eq!(sent.len(), 1);
        assert_eq!(&sent[0][..1], &[9 + 0x20]);
        assert_eq!(&sent[0][3..7], &[11 + 0x20, 110 + 0x20, 0x21, b'\n']);
        assert!(s.take_sound_events().iter().any(|e| e.file == "arrow.wav"));
        // no arrows, no shot
        s.darts = 0;
        sc.sync_player(&s);
        sc.world.weapon_fired("Bow");
        sc.apply(&mut s);
        sc.tick(&mut s, 300);
        assert_eq!(s.arrows.len(), 1);
    }

    #[test]
    fn a_level_change_replaces_npc_scripts_but_keeps_weapons() {
        let mut s = session("GLEVNW01\nNPC - 1 1\nif (created) { message first; }\nNPCEND\n");
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.sync_weapons(&mut s, &[WeaponScript { name: "Bow", image: "bow.png", script: "if (weaponfired) { message pew; }" }]);
        sc.load_level(&mut s, "a.nw");
        assert_eq!(sc.world.ids().len(), 2);
        s.current_level = Some(Level::parse("GLEVNW01\n").unwrap());
        sc.load_level(&mut s, "b.nw");
        assert_eq!(sc.world.ids().len(), 1);
        sc.world.weapon_fired("Bow");
        sc.apply(&mut s);
        assert_eq!(s.last_message.as_deref(), Some("pew"));
    }

    /// GServer-v2's `-gr_movement` weapon: the `gr.x`/`gr.y` flags the
    /// real 2.22 sends come from it, so they leave only when it runs, and
    /// only the axis that changed.
    #[test]
    fn the_gr_movement_weapon_sets_the_flags_that_go_to_the_server() {
        const GR: &str = "if (playerenters || timeout) {\n  if (!gr.off) {\n    if (!gr.x.off) setstring gr.x,#v(int((playerx+0.25)*2)/2);\n    if (!gr.y.off) setstring gr.y,#v(int((playery+0.25)*2)/2);\n  }\n  timeout=0.05;\n}\n";
        let mut s = session("GLEVNW01\n");
        s.player_x = 30.0;
        s.player_y = 30.5;
        let mut sc = SessionScripts::new(ClientVersion::V222);
        sc.sync_weapons(&mut s, &[WeaponScript { name: "-gr_movement", image: "", script: GR }]);
        sc.load_level(&mut s, "a.nw");
        let sent = |s: &mut OfflineSession| std::mem::take(&mut s.flags_out);
        let set = |n: &str, v: &str| (n.to_string(), Some(v.to_string()));
        assert_eq!(sent(&mut s), [set("gr.x", "30"), set("gr.y", "30.5")]);
        sc.tick(&mut s, 50);
        assert!(sent(&mut s).is_empty(), "no change, nothing sent");
        s.player_x = 30.5;
        sc.tick(&mut s, 50);
        assert_eq!(sent(&mut s), [set("gr.x", "30.5")]);
    }

    /// Lab 2026-10-30, real 2.22 in cell (2, 1) of a 3 x 3 gmap: the
    /// `-gr_movement` flags carry gmap coordinates, and a script that sets
    /// `playerx`/`playery` sets them in gmap coordinates too.
    #[test]
    fn on_a_gmap_playerx_and_playery_are_gmap_coordinates() {
        const GR: &str = "if (playerenters || timeout) {\n  if (!gr.off) {\n    if (!gr.x.off) setstring gr.x,#v(int((playerx+0.25)*2)/2);\n    if (!gr.y.off) setstring gr.y,#v(int((playery+0.25)*2)/2);\n  }\n  timeout=0.05;\n}\n";
        let grid = "GRMAP001\nWIDTH 3\nHEIGHT 3\nLEVELNAMES\n\"a.nw\",\"b.nw\",\"c.nw\",\n\"d.nw\",\"e.nw\",\"f.nw\",\n\"g.nw\",\"h.nw\",\"i.nw\",\nLEVELNAMESEND\n";
        let mut s = session("GLEVNW01\n");
        s.enter_gmap("g.gmap".into(), crate::gmap::Gmap::parse(grid).unwrap(), (2, 1));
        s.player_x = 0.5;
        s.player_y = 60.5;
        let mut sc = SessionScripts::new(ClientVersion::V222);
        sc.sync_weapons(&mut s, &[WeaponScript { name: "-gr_movement", image: "", script: GR }]);
        sc.load_level(&mut s, "f.nw");
        let set = |n: &str, v: &str| (n.to_string(), Some(v.to_string()));
        assert_eq!(std::mem::take(&mut s.flags_out), [set("gr.x", "128.5"), set("gr.y", "124.5")]);

        let mut s = session("GLEVNW01\n");
        s.enter_gmap("g.gmap".into(), crate::gmap::Gmap::parse(grid).unwrap(), (2, 1));
        let mut sc = SessionScripts::new(ClientVersion::V222);
        sc.sync_weapons(&mut s, &[WeaponScript { name: "Mover", image: "", script: "if (created) { playerx = 130; playery = 70; }" }]);
        assert_eq!((s.player_x, s.player_y), (2.0, 6.0));
    }

    #[test]
    fn local_flags_stay_in_the_client() {
        let mut s = session("GLEVNW01\nNPC - 1 1\nif (created) { set local.a; set b; unset b; setstring c,x; }\nNPCEND\n");
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "a.nw");
        let names: Vec<_> = s.flags_out.iter().map(|(n, v)| (n.as_str(), v.as_deref())).collect();
        assert_eq!(names, [("b", Some("")), ("b", None), ("c", Some("x"))]);
    }

    #[test]
    fn weapon_sync_adds_removes_and_runs_created_once() {
        let mut s = session("GLEVNW01\n");
        let mut sc = SessionScripts::new(ClientVersion::V205);
        let bow = WeaponScript { name: "Bow", image: "", script: "if (created) { set bowready; }" };
        sc.sync_weapons(&mut s, &[bow]);
        assert_eq!(s.flags.get("bowready"), Some(&true));
        s.flags.insert("bowready".into(), false);
        sc.world.core.flags.clear();
        sc.sync_weapons(&mut s, &[WeaponScript { name: "Bow", image: "", script: "if (created) { set bowready; }" }]);
        assert_eq!(s.flags.get("bowready"), Some(&false)); // not run again
        sc.sync_weapons(&mut s, &[]);
        assert!(sc.world.ids().is_empty());
    }

    #[test]
    fn effects_with_no_session_equivalent_are_handed_back() {
        let mut s = session("GLEVNW01\nNPC - 1 1\nif (created) { triggeraction 1,1,x,; sendtoserver hi; }\nNPCEND\n");
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "a.nw");
        assert!(sc.unhandled.iter().any(|e| matches!(e, Effect::TriggerAction { .. })));
        // 2.05 has no sendtoserver: it was gated out and only logged.
        assert!(!sc.unhandled.iter().any(|e| matches!(e, Effect::SendToServer { .. })));
        assert!(sc.world.take_log().iter().any(|l| l.contains("sendtoserver")));
    }

    #[test]
    fn onwall_uses_the_levels_collision_table() {
        let tiles: Vec<u16> = (0..64 * 64).map(|i| if i % 64 == 10 { 1 } else { 0 }).collect();
        let mut s = OfflineSession::new();
        s.current_level = Some(Level::from_board(&tiles));
        let mut types = crate::assets::TileTypes::from_types([0; 4096]);
        types.types[1] = 22;
        s.tiletypes = Some(types);
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "a.nw");
        let mut w = sc.world;
        let id = w.add(ObjSpec::npc("a.png", 0.0, 0.0, "if (created) { this.a = onwall(10.5, 3); this.b = onwall(11.5, 3); }"));
        w.fire(id, "created", &[]);
        assert_eq!(w.obj(id).unwrap().vars["this.a"], crate::scripting::engine::Val::Num(1.0));
        assert_eq!(w.obj(id).unwrap().vars["this.b"], crate::scripting::engine::Val::Num(0.0));
    }

    #[test]
    fn version_picks_the_vocabulary() {
        let src = "GLEVNW01\nNPC - 1 1\nif (created) { hitobjects 1,1,1; }\nNPCEND\n";
        let mut s = session(src);
        let mut old = SessionScripts::new(ClientVersion::V200);
        old.load_level(&mut s, "a.nw");
        assert!(old.unhandled.is_empty());
        let mut new = SessionScripts::new(ClientVersion::V222);
        new.load_level(&mut s, "a.nw");
        assert_eq!(new.unhandled.len(), 1);
    }

    // --- Script-initiated warps (setlevel / setlevel2) ---

    /// Run `src` as an NPC's `created` event with the player at (7, 9).
    fn warp_after(src: &str) -> OfflineSession {
        let mut s = session(&format!("GLEVNW01\nNPC - 1 1\n{src}\nNPCEND\n"));
        s.player_x = 7.0;
        s.player_y = 9.0;
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "a.nw");
        s
    }

    #[test]
    fn setlevel2_asks_for_the_level_at_the_given_position() {
        let s = warp_after("if (created) { setlevel2 castle.nw,12.5,3; }");
        assert_eq!(s.pending_link, Some(("castle.nw".to_string(), 12.5, 3.0)));
    }

    #[test]
    fn setlevel_keeps_the_players_position() {
        // No position: the player's own, whatever the script set before.
        let s = warp_after("if (created) { setlevel castle.nw; }");
        assert_eq!(s.pending_link, Some(("castle.nw".to_string(), 7.0, 9.0)));
        let s = warp_after("if (created) { playerx = 30; playery = 31.5; setlevel castle.nw; }");
        assert_eq!(s.pending_link, Some(("castle.nw".to_string(), 30.0, 31.5)));
    }

    #[test]
    fn a_warp_ends_the_run_and_the_request_is_one() {
        // The second warp never runs: setlevel2 ends the script.
        let s = warp_after("if (created) { setlevel2 one.nw,1,1; setlevel2 two.nw,2,2; }");
        assert_eq!(s.pending_link, Some(("one.nw".to_string(), 1.0, 1.0)));
    }

    /// Offline nothing changes: the request is the same `pending_link` the
    /// main loop takes and loads locally, and arriving puts the player at
    /// the given position.
    #[test]
    fn offline_warp_arrives_where_the_script_said() {
        let mut s = warp_after("if (created) { setlevel2 castle.nw,12,3; }");
        let (dest, x, y) = s.pending_link.take().unwrap();
        assert_eq!(dest, "castle.nw");
        s.arrive(x, y);
        assert_eq!((s.player_x, s.player_y), (12.0, 3.0));
        assert!(s.warp_pending.is_none() && s.pending_link.is_none());
    }

    const UNSTICK_JAIL: &str = "GLEVNW01\n\
        NPC - 20 20\n\
        //#CLIENTSIDE\n\
        if (playerchats && strequals(#c,unstick me)) { setplayerprop #c,Cant use unstick me here!; }\n\
        NPCEND\n";

    #[test]
    fn a_script_can_rewrite_the_typed_line_before_it_is_sent() {
        let mut s = session(UNSTICK_JAIL);
        let mut sc = SessionScripts::new(ClientVersion::V222);
        sc.load_level(&mut s, "jail.nw");
        assert_eq!(sc.chat(&mut s, "unstick me"), "Cant use unstick me here!");
        // The match is the script's, so other lines pass untouched.
        assert_eq!(sc.chat(&mut s, "hello there"), "hello there");
        // And the answer is not kept for the next line.
        assert!(!s.props.contains_key("#c"));
    }

    #[test]
    fn a_stale_chat_prop_is_not_taken_for_the_answer() {
        let mut s = session("GLEVNW01\n");
        let mut sc = SessionScripts::new(ClientVersion::V222);
        sc.load_level(&mut s, "x.nw");
        s.props.insert("#c".into(), "left over".into());
        assert_eq!(sc.chat(&mut s, "unstick me"), "unstick me");
    }

    #[test]
    fn without_scripts_the_line_goes_out_as_typed() {
        let mut s = session(DOOR);
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "town.nw");
        assert_eq!(sc.chat(&mut s, "unstick me"), "unstick me");
    }

    const CLOTHES: &str = "GLEVNW01\n\
        NPC - 20 20\n\
        //#CLIENTSIDE\n\
        if (created) { setbody body13.png; sethead head927.png; setskincolor 18; setcoatcolor red; setbeltcolor 25; }\n\
        NPCEND\n";

    #[test]
    fn looks_commands_change_the_player() {
        let mut s = session(CLOTHES);
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "shop.nw");
        assert_eq!(s.looks.body.as_deref(), Some("body13.png"));
        assert_eq!(s.looks.head.as_deref(), Some("head927.png"));
        // Skin 18, coat red (4); a belt past the list is left alone.
        assert_eq!(s.looks.colors, [18, 4, 18, 5, 18]);
        assert!(s.looks_changed);
        assert!(sc.unhandled.iter().all(|e| !format!("{e:?}").contains("set")), "{:?}", sc.unhandled);
    }

    const PROPS: &str = "GLEVNW01\n\
        NPC - 20 20\n\
        //#CLIENTSIDE\n\
        if (created) { setplayerprop #P1,hat3.png; setplayerprop #C1,blue; setplayerprop #C4,7; setplayerprop #8,body2.png; setplayerprop #3,head5.png; }\n\
        NPCEND\n";

    #[test]
    fn setplayerprop_looks_codes_change_the_player() {
        let mut s = session(PROPS);
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "shop.nw");
        assert_eq!(s.looks.attrs[0].as_deref(), Some("hat3.png"));
        assert_eq!(s.looks.body.as_deref(), Some("body2.png"));
        assert_eq!(s.looks.head.as_deref(), Some("head5.png"));
        assert_eq!(s.looks.colors[1], crate::looks::color_by_name("blue").unwrap());
        assert_eq!(s.looks.colors[4], 7);
        assert!(s.looks_changed);
    }

    #[test]
    fn play_queues_a_sound_at_the_npc_centre() {
        let mut s = session("GLEVNW01\nNPC - 10 20\nif (created) { play wolf.wav; }\nNPCEND\n");
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "a.nw");
        // No image size known: a 32-pixel image, so one tile in.
        assert_eq!(s.take_sound_events(), vec![crate::sound::SoundEvent { file: "wolf.wav".into(), x: 11.0, y: 21.0 }]);
        assert!(!sc.unhandled.iter().any(|e| matches!(e, Effect::Play { .. })), "{:?}", sc.unhandled);
    }

    #[test]
    fn play_uses_the_image_part_or_shape_for_the_centre() {
        let mut s = session("GLEVNW01\nNPC - 10 20\nif (created) { setimgpart x.png,0,0,64,16; play a.wav; }\nNPCEND\nNPC - 4 4\nif (created) { setshape 1,16,48; play b.wav; }\nNPCEND\n");
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "a.nw");
        let ev = s.take_sound_events();
        assert!(ev.contains(&crate::sound::SoundEvent { file: "a.wav".into(), x: 12.0, y: 20.5 }), "{ev:?}");
        assert!(ev.contains(&crate::sound::SoundEvent { file: "b.wav".into(), x: 4.5, y: 5.5 }), "{ev:?}");
    }

    #[test]
    fn playlooped_gets_the_loop_prefix_and_stopsound_is_queued() {
        let mut s = session("GLEVNW01\nNPC - 0 0\nif (created) { playlooped thunder.wav; stopsound rain.wav; }\nNPCEND\n");
        let mut sc = SessionScripts::new(ClientVersion::V205);
        sc.load_level(&mut s, "a.nw");
        let ev = s.take_sound_events();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].file, "+thunder.wav");
        assert_eq!(s.take_sound_stops(), vec!["rain.wav".to_string()]);
        assert!(s.take_sound_stops().is_empty());
    }

    #[test]
    fn play2_names_its_place_and_a_weapon_plays_at_the_player() {
        let mut s = session("GLEVNW01\nNPC - 0 0\nif (created) { play2 bell.wav,30,40,1; }\nNPCEND\n");
        let mut sc = SessionScripts::new(ClientVersion::V222);
        sc.load_level(&mut s, "a.nw");
        assert_eq!(s.take_sound_events(), vec![crate::sound::SoundEvent { file: "bell.wav".into(), x: 30.0, y: 40.0 }]);
        s.player_x = 7.0;
        s.player_y = 9.0;
        sc.sync_weapons(&mut s, &[WeaponScript { name: "Bow", image: "", script: "if (weaponfired) { play arrow.wav; }" }]);
        sc.world.weapon_fired("Bow");
        sc.apply(&mut s);
        let ev = s.take_sound_events();
        assert_eq!(ev, vec![crate::sound::SoundEvent { file: "arrow.wav".into(), x: 8.5, y: 11.0 }]);
    }
}
