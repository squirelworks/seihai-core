//! The script world: objects (NPCs and weapons), shared state, the
//! deterministic scheduler, and the effects scripts ask the host for.

use super::compile::{compile, Compiled};
use super::interp::Run;
use super::parser::parse;
use super::value::Val;
use super::vocab::{self, Era};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::rc::Rc;

/// Index of an object in a [`ScriptWorld`].
pub type ObjId = usize;

/// Length of one script tick. `timeout` and `sleep` count in these.
pub const TICK_MS: u64 = 50;

/// Pixels per tile, for `setshape`.
pub const PX_PER_TILE: f64 = 16.0;

/// Most node steps one event run may take. A runaway script is stopped
/// and logged.
pub const MAX_STEPS: u64 = 200_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjKind {
    /// A level NPC.
    Npc,
    /// A weapon the player owns.
    Weapon,
}

/// What the host gives to [`ScriptWorld::add`].
#[derive(Debug, Clone)]
pub struct ObjSpec {
    pub kind: ObjKind,
    pub name: String,
    pub image: String,
    /// Position in tiles.
    pub x: f64,
    pub y: f64,
    pub script: String,
    /// Host-defined index (the NPC's place in its level, say), echoed in
    /// [`Effect`]s through [`ScriptObject::tag`].
    pub tag: usize,
}

impl ObjSpec {
    pub fn npc(image: &str, x: f64, y: f64, script: &str) -> Self {
        ObjSpec { kind: ObjKind::Npc, name: String::new(), image: image.into(), x, y, script: script.into(), tag: 0 }
    }

    pub fn weapon(name: &str, image: &str, script: &str) -> Self {
        ObjSpec { kind: ObjKind::Weapon, name: name.into(), image: image.into(), x: 0.0, y: 0.0, script: script.into(), tag: 0 }
    }

    pub fn with_tag(mut self, tag: usize) -> Self {
        self.tag = tag;
        self
    }
}

/// A running script object.
#[derive(Debug)]
pub struct ScriptObject {
    pub id: ObjId,
    pub kind: ObjKind,
    pub name: String,
    pub image: String,
    pub x: f64,
    pub y: f64,
    /// Touch shape in pixels (`setshape`). Default 32 by 32.
    pub w_px: f64,
    pub h_px: f64,
    pub visible: bool,
    /// Text over the NPC (`message`).
    pub message: String,
    pub tag: usize,
    pub(super) script: Rc<Compiled>,
    /// `this.` variables and script-local variables, by lower-case name.
    pub vars: HashMap<String, Val>,
    /// Tick the next `timeout` event is due on.
    pub due: Option<u64>,
    /// Node a `sleep` resumes at.
    pub resume: Option<usize>,
    pub touching: bool,
    pub params: Vec<String>,
    /// `setcharprop` values by code (`#P1`), read back by `#P1(-1)`.
    pub char_props: HashMap<String, String>,
    pub destroyed: bool,
    /// Events run so far.
    pub runs: u64,
}

impl ScriptObject {
    /// The script's parse and compile diagnostics.
    pub fn diagnostics(&self) -> &[super::ast::Diag] {
        &self.script.diagnostics
    }

    /// Touch rectangle `(x, y, w, h)` in tiles.
    pub fn rect(&self) -> (f64, f64, f64, f64) {
        (self.x, self.y, self.w_px / PX_PER_TILE, self.h_px / PX_PER_TILE)
    }
}

/// The player as scripts see it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerView {
    pub x: f64,
    pub y: f64,
    /// 0 up, 1 left, 2 down, 3 right.
    pub dir: f64,
    pub hearts: f64,
    pub rupees: f64,
    pub bombs: f64,
    pub darts: f64,
    pub sword_power: f64,
    pub shield_power: f64,
    pub hp: f64,
    pub max_hp: f64,
    pub nick: String,
    pub account: String,
    /// Last chat line (`#c`).
    pub chat: String,
    /// The selected weapon's name (`#w`).
    pub weapon: String,
    /// Names of the status flags that are true (`playerswimming`, ...).
    pub status: BTreeSet<String>,
}

impl Default for PlayerView {
    fn default() -> Self {
        PlayerView {
            x: 0.0,
            y: 0.0,
            dir: 2.0,
            hearts: 3.0,
            rupees: 0.0,
            bombs: 0.0,
            darts: 0.0,
            sword_power: 1.0,
            shield_power: 1.0,
            hp: 3.0,
            max_hp: 3.0,
            nick: String::new(),
            account: String::new(),
            chat: String::new(),
            weapon: String::new(),
            status: BTreeSet::new(),
        }
    }
}

/// Something a script asked the host to do. The engine keeps its own
/// copy of the state it can (visibility, image, flags, the player's
/// numbers), so reads inside scripts are consistent; the host mirrors
/// these into the real session.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Message { obj: ObjId, text: String },
    /// `say n`: show sign `n` of the level.
    SaySign { index: i64 },
    /// `say2 text`: show the text itself.
    Say2 { text: String },
    SetImg { obj: ObjId, file: String, part: Option<(f64, f64, f64, f64)> },
    SetVisible { obj: ObjId, visible: bool, local: bool },
    SetShape { obj: ObjId, kind: i64, w: f64, h: f64 },
    /// An NPC's position changed (assignment to `x` / `y`, or `move`).
    SetPos { obj: ObjId, x: f64, y: f64 },
    Move { obj: ObjId, dx: f64, dy: f64, time: f64, options: i64 },
    Destroy { obj: ObjId },
    SetCharAni { obj: ObjId, ani: String, params: String },
    SetCharProp { obj: ObjId, code: String, value: String },
    SetPlayerProp { code: String, value: String },
    /// `play`, `playlooped`: an effect at the object's position. `play2`
    /// (2.1 and later) names the position itself, in `at`.
    Play { obj: ObjId, file: String, looped: bool, at: Option<(f64, f64)> },
    StopSound { file: String },
    StopMidi,
    /// `setlevel` (no position) and `setlevel2`.
    SetLevel { level: String, x: Option<f64>, y: Option<f64> },
    Hurt { half_hearts: f64 },
    /// A write to a built-in player number (`playerx = 5`).
    SetPlayerVar { name: String, value: f64 },
    SetFlag { name: String, value: Option<String> },
    UnsetFlag { name: String },
    TriggerAction { x: f64, y: f64, action: String, params: String },
    /// `sendrpgmessage text`: a line for the player's message window.
    RpgMessage { text: String },
    /// Seihai's `sendtoserver` stub.
    SendToServer { text: String },
    /// Any other command in the table, with its evaluated arguments.
    Command { obj: ObjId, name: String, args: Vec<Val> },
}

/// A call queued to run after the current event finishes.
#[derive(Debug, Clone)]
pub(super) struct Pending {
    pub target: Target,
    pub event: String,
    pub params: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) enum Target {
    Id(ObjId),
    /// `callnpc n`: the n-th NPC object.
    Npc(usize),
    /// `callweapon n`: the n-th weapon object.
    Weapon(usize),
    /// Every object whose touch rectangle holds this point (tiles).
    At(f64, f64),
}

/// State shared by all objects.
pub struct Core {
    pub era: Era,
    /// Player flags and `client.` / `local.` flags: name to value text
    /// (empty for a plain `set`).
    pub flags: HashMap<String, String>,
    pub server_vars: HashMap<String, Val>,
    pub level_vars: HashMap<String, Val>,
    pub player: PlayerView,
    pub level: String,
    pub effects: Vec<Effect>,
    pub log: Vec<String>,
    /// Ticks since the world started.
    pub tick: u64,
    pub tokens: Vec<String>,
    /// Connected to a server (not offline play).
    pub online: bool,
    /// The server announced an NPC server.
    pub npc_server: bool,
    pub(super) pending: VecDeque<Pending>,
    /// Collision of the current level: 64 by 64 tiles, row by row, true
    /// where blocked. `None` until the host provides it (then nothing is
    /// a wall).
    pub walls: Option<Vec<bool>>,
    /// The same 64 by 64 layout, true on a liquid tile (water or lava: the
    /// 2.05 water test asks the liquid list, which holds both).
    pub water: Option<Vec<bool>>,
    /// The map the current level is on (the gmap, else the big map), for
    /// `onmapx` / `onmapy`. `None` when there is none.
    pub map: Option<crate::gmap::Gmap>,
    rng: u64,
}

impl Core {
    pub fn note(&mut self, msg: String) {
        if self.log.len() < 1000 {
            self.log.push(msg);
        }
    }

    /// Next number from the world's generator, in `[0, 1)`.
    pub fn random(&mut self) -> f64 {
        // xorshift64*: deterministic for a given seed.
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Convert seconds to ticks, rounding up, at least one.
pub fn secs_to_ticks(secs: f64) -> u64 {
    ((secs * 1000.0 / TICK_MS as f64) - 1e-9).ceil().max(1.0) as u64
}

/// All objects and their shared state.
pub struct ScriptWorld {
    pub core: Core,
    objects: Vec<Option<ScriptObject>>,
    acc_ms: u64,
}

impl ScriptWorld {
    pub fn new(era: Era) -> Self {
        Self::with_seed(era, 0x5EED_5EED)
    }

    pub fn with_seed(era: Era, seed: u64) -> Self {
        ScriptWorld {
            core: Core {
                era,
                flags: HashMap::new(),
                server_vars: HashMap::new(),
                level_vars: HashMap::new(),
                player: PlayerView::default(),
                level: String::new(),
                effects: Vec::new(),
                log: Vec::new(),
                tick: 0,
                tokens: Vec::new(),
                online: false,
                npc_server: false,
                pending: VecDeque::new(),
                walls: None,
                water: None,
                map: None,
                rng: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
            },
            objects: Vec::new(),
            acc_ms: 0,
        }
    }

    pub fn era(&self) -> Era {
        self.core.era
    }

    pub fn set_online(&mut self, online: bool, npc_server: bool) {
        self.core.online = online;
        self.core.npc_server = npc_server;
    }

    /// Parse and add an object. Does not run any event. Problems in the
    /// script are logged and the object still exists.
    pub fn add(&mut self, spec: ObjSpec) -> ObjId {
        let id = self.objects.len();
        let parsed = parse(&spec.script, self.core.era);
        let compiled = compile(&parsed);
        for d in &compiled.diagnostics {
            // Say what the object is and show the line, so a log line can
            // be matched to a script without the level at hand.
            let who = match spec.kind {
                ObjKind::Weapon => format!("weapon {}", spec.name),
                ObjKind::Npc => format!("npc {} at {},{}", if spec.image.is_empty() { "-" } else { &spec.image }, spec.x, spec.y),
            };
            let src: String = spec
                .script
                .lines()
                .nth((d.line as usize).saturating_sub(1))
                .map(|l| l.trim().chars().take(80).collect())
                .unwrap_or_default();
            self.core.note(format!("object {id} ({who}): {d} [{src}]"));
        }
        self.objects.push(Some(ScriptObject {
            id,
            kind: spec.kind,
            name: spec.name,
            image: spec.image,
            x: spec.x,
            y: spec.y,
            w_px: 32.0,
            h_px: 32.0,
            visible: true,
            message: String::new(),
            tag: spec.tag,
            script: Rc::new(compiled),
            vars: HashMap::new(),
            due: None,
            resume: None,
            touching: false,
            params: Vec::new(),
            char_props: HashMap::new(),
            destroyed: false,
            runs: 0,
        }));
        id
    }

    pub fn obj(&self, id: ObjId) -> Option<&ScriptObject> {
        self.objects.get(id).and_then(|o| o.as_ref())
    }

    pub fn obj_mut(&mut self, id: ObjId) -> Option<&mut ScriptObject> {
        self.objects.get_mut(id).and_then(|o| o.as_mut())
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Ids of the live objects, in creation order.
    pub fn ids(&self) -> Vec<ObjId> {
        self.objects.iter().flatten().filter(|o| !o.destroyed).map(|o| o.id).collect()
    }

    /// Drop every level NPC and the `level.` variables (a level change).
    /// Weapons stay.
    pub fn clear_level(&mut self) {
        for o in self.objects.iter_mut() {
            if o.as_ref().is_some_and(|o| o.kind == ObjKind::Npc) {
                *o = None;
            }
        }
        self.core.level_vars.clear();
        self.core.pending.retain(|p| !matches!(p.target, Target::Npc(_)));
    }

    /// Remove the weapon with this name, if present.
    pub fn remove_weapon(&mut self, name: &str) {
        for o in self.objects.iter_mut() {
            if o.as_ref().is_some_and(|o| o.kind == ObjKind::Weapon && o.name == name) {
                *o = None;
            }
        }
    }

    pub fn flag(&self, name: &str) -> Option<&str> {
        self.core.flags.get(&name.to_ascii_lowercase()).map(|s| s.as_str())
    }

    pub fn set_flag(&mut self, name: &str, value: &str) {
        self.core.flags.insert(name.to_ascii_lowercase(), value.to_string());
    }

    /// Effects produced since the last call.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.core.effects)
    }

    /// Log lines (diagnostics, ignored commands) since the last call.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.core.log)
    }

    // ---- events ----

    /// Run one event on one object, then everything it queued.
    pub fn fire(&mut self, id: ObjId, event: &str, params: &[String]) {
        self.fire_one(id, event, params);
        self.drain();
    }

    /// Run an event on every object, in creation order.
    pub fn broadcast(&mut self, event: &str) {
        for id in self.ids() {
            self.fire_one(id, event, &[]);
        }
        self.drain();
    }

    /// `created` on every object that has not run anything yet.
    pub fn start_all(&mut self) {
        for id in self.ids() {
            if self.obj(id).is_some_and(|o| o.runs == 0) {
                self.fire_one(id, "created", &[]);
            }
        }
        self.drain();
    }

    /// The player said something: `playerchats` everywhere.
    pub fn chat(&mut self, text: &str) {
        self.core.player.chat = text.to_string();
        self.broadcast("playerchats");
    }

    /// The player pressed the weapon button for the named weapon.
    pub fn weapon_fired(&mut self, name: &str) {
        let ids: Vec<ObjId> =
            self.ids().into_iter().filter(|&i| self.obj(i).is_some_and(|o| o.kind == ObjKind::Weapon && o.name == name)).collect();
        for id in ids {
            self.fire_one(id, "weaponfired", &[]);
        }
        self.drain();
    }

    /// The player's foot point is at `(fx, fy)` tiles. Fires
    /// `playertouchsme` on each visible NPC whose touch rectangle it has
    /// just entered. Returns how many fired.
    pub fn touch(&mut self, fx: f64, fy: f64) -> usize {
        let mut hits = Vec::new();
        for id in self.ids() {
            let o = self.obj_mut(id).unwrap();
            if o.kind != ObjKind::Npc || !o.visible {
                o.touching = false;
                continue;
            }
            let (x, y, w, h) = o.rect();
            let inside = fx >= x && fx < x + w && fy >= y && fy < y + h;
            if inside && !o.touching {
                hits.push(id);
            }
            o.touching = inside;
        }
        for &id in &hits {
            self.fire_one(id, "playertouchsme", &[]);
        }
        self.drain();
        hits.len()
    }

    /// The first visible NPC, in creation order, whose rectangle holds the
    /// point `(fx, fy)` in tiles: what a thrown object at that point hits.
    pub fn npc_at(&self, fx: f64, fy: f64) -> Option<ObjId> {
        self.ids().into_iter().find(|&id| {
            self.obj(id).is_some_and(|o| {
                let (x, y, w, h) = o.rect();
                o.kind == ObjKind::Npc && o.visible && !o.destroyed && fx >= x && fx < x + w && fy >= y && fy < y + h
            })
        })
    }

    /// Advance the clock by `ms`. Timeouts that fall due run in tick
    /// order, and within a tick in creation order, so a given sequence of
    /// `tick` calls always produces the same events.
    pub fn tick(&mut self, ms: u64) {
        self.acc_ms += ms;
        while self.acc_ms >= TICK_MS {
            self.acc_ms -= TICK_MS;
            self.core.tick += 1;
            let now = self.core.tick;
            for id in self.ids() {
                let due = self.obj(id).and_then(|o| o.due).is_some_and(|d| d <= now);
                if due {
                    self.obj_mut(id).unwrap().due = None;
                    self.fire_one(id, "timeout", &[]);
                }
            }
            self.drain();
        }
    }

    /// Time the world has been advanced to, in milliseconds.
    pub fn now_ms(&self) -> u64 {
        self.core.tick * TICK_MS + self.acc_ms
    }

    fn drain(&mut self) {
        let mut budget = 1000;
        while let Some(p) = self.core.pending.pop_front() {
            if budget == 0 {
                self.core.note("queued script calls cut off after 1000".into());
                self.core.pending.clear();
                break;
            }
            budget -= 1;
            let id = match p.target {
                Target::At(x, y) => {
                    let hits: Vec<ObjId> = self
                        .ids()
                        .into_iter()
                        .filter(|&i| {
                            self.obj(i).is_some_and(|o| {
                                let (ox, oy, w, h) = o.rect();
                                o.kind == ObjKind::Npc && x >= ox && x < ox + w && y >= oy && y < oy + h
                            })
                        })
                        .collect();
                    for i in hits {
                        self.fire_one(i, &p.event, &p.params);
                    }
                    continue;
                }
                Target::Id(i) => Some(i),
                Target::Npc(n) => self.ids().into_iter().filter(|&i| self.obj(i).is_some_and(|o| o.kind == ObjKind::Npc)).nth(n),
                Target::Weapon(n) => self.ids().into_iter().filter(|&i| self.obj(i).is_some_and(|o| o.kind == ObjKind::Weapon)).nth(n),
            };
            match id {
                Some(id) => self.fire_one(id, &p.event, &p.params),
                None => self.core.note(format!("call to a missing object ({})", p.event)),
            }
        }
    }

    fn fire_one(&mut self, id: ObjId, event: &str, params: &[String]) {
        let event = event.to_ascii_lowercase();
        match vocab::event_since(&event) {
            Some(e) if e <= self.core.era => {}
            Some(_) => {
                self.core.note(format!("event `{event}` does not exist in {}", self.core.era.label()));
                return;
            }
            None => {
                self.core.note(format!("`{event}` is not an event"));
                return;
            }
        }
        let Some(mut obj) = self.objects.get_mut(id).and_then(|o| o.take()) else {
            return;
        };
        if obj.destroyed {
            self.objects[id] = Some(obj);
            return;
        }
        obj.params = params.to_vec();
        obj.runs += 1;
        let start = if event == "timeout" { obj.resume.take().unwrap_or(0) } else { 0 };
        {
            let mut run = Run::new(&mut self.core, &mut obj, &event);
            run.exec(start);
        }
        self.objects[id] = Some(obj);
    }
}
