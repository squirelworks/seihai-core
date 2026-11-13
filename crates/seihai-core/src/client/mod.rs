//! A client's game-server session, shared by the client lines that log in
//! the same way (`seihai-ato` for 3.x to 5.x, `seihai-issho` for 6.x): log
//! in to a game server as the build, read what it sends with the core
//! readers, keep the level ([`level`]), log it, and hand it to a view.
//!
//! The login is core's (`crate::net::login_gserver`): the profiles carry
//! the login token, the bundled first frame and the props the lab runs of
//! the real clients showed. Once the server's login burst is in, the
//! session sends the frame the real client answers with, as `seihai-mae`
//! does: core's `protocol::login::LoginBurst` (3.x: the file checks and
//! props once the board is in) or, for the lobby builds,
//! `protocol::lobby::LobbyBurst` (idle.gani, the PM and lister setup, 46,
//! the window list and the own props with the nickname). File replies are
//! kept by core's `FileSession`.
//!
//! A server NPC's GS2 script (`PLO_NPCBYTECODE`, 131) is kept by core's
//! session state, read with `scripting::gs2::Script::parse` when the level
//! is logged, and logged with its opcodes. It is not run.
//!
//! A server warp (`PLO_SERVERWARP`, 178) is followed as the real clients
//! do: leave this server and log in again at the host and port it names,
//! with a new key and the same account. The 4.x to 6.x builds need it:
//! they log in first to a game server named `Login` and play only after
//! its warp (`crate::protocol::lobby`).
//!
//! Under the level line a client line can log more (Ato logs the fields
//! its Torque profiles read, through [`run_with`]'s `level_lines`).
//!
//! This loop was `seihai-ato`'s `online` module; it moved here unchanged
//! so that Issho runs the same session.

pub mod level;

use crate::assets::{AssetResolver, AssetRoot, Level};
use crate::filecache::{DownloadStore, FileSession};
use crate::gmap::{apply_board_mods_to, sync_world, Crossing, Gmap, GmapRules, WorldLoader};
use crate::net::{
    link_request_frame, login_gserver_with, random_key, walk_message_for, wantfile_message,
};
use crate::offline::OfflineSession;
use crate::protocol::layouts::BoardModify2;
use crate::protocol::lobby::LobbyBurst;
use crate::protocol::login::{nick_props_message, sends_nick_props, LoginBurst};
use crate::protocol::npcprops::NpcProps;
use crate::protocol::walkcadence::WalkCadence;
use crate::protocol::{ClientVersion, SessionState};
use level::{Arrival, LevelStore};
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often the loop looks while a post-login frame is owed, so a frame
/// due 240 ms after the level goes out on time.
const BURST_TICK: Duration = Duration::from_millis(20);

/// What the start window (or the command line) hands over for a login.
/// The nickname and language are not sent yet: the real 3.04 sends them in
/// its post-login props, which Stage 2 leaves out.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Login {
    pub account: String,
    pub password: String,
    pub nick: String,
    pub language: String,
}

/// How long the server must stay quiet after a level is stored before it
/// is logged: its NPCs and the player's props can come in later frames.
const SETTLE: Duration = Duration::from_millis(300);

/// Extra lines logged under each level line: the level, its name and the
/// session state.
pub type LevelLines<'a> = &'a dyn Fn(&Level, &str, &SessionState) -> Vec<String>;

/// Log in at `addr` and log every level the server sends to `out`, until
/// the server closes the connection or, with `first_level_only`, the first
/// level is stored. `assets` is an asset folder whose levels may stand in
/// for the board at the server's mod time. `shown` gets each level as it
/// is logged (the game view draws it).
pub fn run(
    addr: &str,
    version: ClientVersion,
    login: &Login,
    assets: Option<PathBuf>,
    first_level_only: bool,
    out: &mut impl Write,
    shown: &mut dyn FnMut(&Level, &Arrival),
) -> Result<(), String> {
    run_with(
        addr,
        version,
        login,
        assets,
        first_level_only,
        out,
        shown,
        None,
        &|_, _, _| Vec::new(),
    )
}

/// Where the player stands and faces, as the game view last set it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pose {
    pub x: f32,
    pub y: f32,
    /// 0 up, 1 left, 2 down, 3 right.
    pub dir: u8,
    pub walking: bool,
    /// The SPRITE value (17) when it is more than the direction: `dir +
    /// 4 * S` during a sword swing on a build that shows the swing in the
    /// sprite (core's `OfflineSession::sprite_value`). `None` sends `dir`.
    pub sprite: Option<u8>,
    /// The animation the view names in the GANI prop (10) on a GANI build:
    /// `sword` or `spin` during a swing (core's
    /// `OfflineSession::bow_slot_anim`). `None` is `walk` or `idle` by
    /// `walking`.
    pub gani: Option<&'static str>,
}

impl Pose {
    /// The SPRITE value to send.
    pub fn sprite_value(&self) -> u8 {
        self.sprite.unwrap_or(self.dir)
    }
}

/// `PLI_PLAYERPROPS` (2) with GANI (10): the animation by name, as a GANI
/// build shows a swing to the other players.
pub fn gani_props_message(gani: &str) -> Vec<u8> {
    let name = &gani.as_bytes()[..gani.len().min(223)];
    let mut m = vec![2 + 0x20, 10 + 0x20, name.len() as u8 + 0x20];
    m.extend_from_slice(name);
    m.push(b'\n');
    m
}

/// The game view writes the pose here; the network thread reads it each
/// 50 ms tick and sends the walk props.
pub type SharedPose = Arc<Mutex<Option<Pose>>>;

/// Another player in the level, as the server last placed them, with
/// what the view needs to draw them from their GANI.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Other {
    pub id: u16,
    pub x: f32,
    pub y: f32,
    pub dir: u8,
    pub nick: Option<String>,
    /// SPRITE (17) as received (`dir + 4 * S`), once the server has said:
    /// the pose a pre-2.1 player is in.
    pub sprite: Option<u8>,
    /// STATUS (18), once the server has said (the dead and hidden bits).
    pub status: Option<u8>,
    /// The pictures, colours and GANI the server sent for the player.
    pub looks: crate::protocol::OtherLooks,
    /// The chat bubble over the player (CURCHAT, 12), while it lasts: the
    /// server's empty CURCHAT or the bubble's own timer ends it.
    pub chat: Option<String>,
}

/// The network thread writes the other players here after each frame; the
/// game view draws them.
pub type SharedOthers = Arc<Mutex<Vec<Other>>>;

/// A level link the player walked into: the level and where to arrive.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkAsk {
    pub dest: String,
    pub x: f32,
    pub y: f32,
    pub dir: u8,
}

/// What the game view and the network thread share.
#[derive(Debug, Clone, Default)]
pub struct Live {
    pub pose: SharedPose,
    pub others: SharedOthers,
    /// A link to ask the server for; the network thread takes it.
    pub link: Arc<Mutex<Option<LinkAsk>>>,
    /// Sounds the view could not find on disk; the network thread asks
    /// the server for each (WANTFILE), as 2.05 does, and the sound plays
    /// on a later cue once it has arrived.
    pub wanted_sounds: Arc<Mutex<Vec<String>>>,
    /// Sound files the server sent, by name, with their bytes: downloads
    /// are kept in memory, so the view's sound engine takes them from here
    /// (`SoundEngine::add`) rather than from its folders.
    pub arrived_sounds: SharedSounds,
    /// The gmap the server has the player on, as the network thread last
    /// saw it changed; the view takes it.
    pub world: Arc<Mutex<Option<WorldView>>>,
    /// Gmap edges the view's walk crossed, in order; the network thread
    /// takes them and sends each crossing.
    pub crossings: Arc<Mutex<Vec<Crossing>>>,
    /// Board changes the server sent (`PLO_BOARDMODIFY`, 7, and the 186
    /// blocks), keyed by the level's lower-case name, for the view to put
    /// on the levels it holds (`gmap::apply_board_mods_to`).
    pub board_mods: SharedBoardMods,
    /// The server's NPC updates (`PLO_NPCPROPS`, 3) for the player's level,
    /// in arrival order, for the view to put on the NPCs it draws.
    pub npc_changes: SharedNpcChanges,
    /// Set by the view when its window closes: the network thread logs out
    /// (closes the connection) within a walk tick, so the server drops the
    /// player instead of keeping them until the process ends.
    pub closed: Arc<std::sync::atomic::AtomicBool>,
    /// The player's own counters (hearts, gralats, bombs, arrows, AP).
    pub own: SharedOwn,
    /// Lines the player typed, in order; the network thread takes each and
    /// says it in the level (CURCHAT), where the server also runs chat
    /// commands such as `setnick` from it.
    pub say: Arc<Mutex<Vec<String>>>,
    /// The chat lines that came in, for the view to take.
    pub chat: SharedChat,
    /// Messages the view built for the server (core's `PLI_BOARDMODIFY`
    /// for a cut, bomb and arrow adds), in order; the network thread sends
    /// them after the walk of its next tick.
    pub send: Arc<Mutex<Vec<Vec<u8>>>>,
    /// The player's weapons as the server last listed them, for the
    /// view's item list (Q) and weapon key (D).
    pub weapons: SharedWeapons,
    /// Other players' bombs, bomb removals, explosions and arrows, in the
    /// order they came, in the player's own cell's coordinates (the
    /// sender's cell offset added on a gmap), for the view to take.
    pub level_objects: SharedLevelObjects,
    /// Other players who threw what they carried, for the view to take.
    pub throws: SharedThrows,
    /// Hits on the player from other players (their swings, and
    /// `PLO_HURTPLAYER` / `PLO_HITOBJECTS`), for the game to judge with
    /// `OfflineSession::receive_all` on the victim's side, as the real
    /// clients do.
    pub hits: SharedHits,
    /// Items the server put on the floor or took off it
    /// (`PLO_ITEMADD` / `PLO_ITEMDEL`: other players' drops, and the
    /// items the server rolls), in the order they came, for the game to
    /// add to or take from the level.
    pub items: SharedItems,
}

/// [`Live::items`].
pub type SharedItems = Arc<Mutex<Vec<crate::protocol::ServerItem>>>;

/// Move the server's floor item changes `st` holds to the game's queue.
/// Drained either way, so nothing builds up with no game.
fn take_items(st: &mut SessionState, to: Option<&SharedItems>) {
    let items = std::mem::take(&mut st.items);
    if let (Some(to), false) = (to, items.is_empty()) {
        to.lock().unwrap_or_else(|e| e.into_inner()).extend(items);
    }
}

/// [`Live::hits`].
pub type SharedHits = Arc<Mutex<IncomingHits>>;

/// What the game needs to judge the hits on the player: the hits since it
/// last took them, and the other players and the player's own cell and
/// nickname as they were when the last hit came.
#[derive(Debug, Clone, Default)]
pub struct IncomingHits {
    /// Other players whose update was a sword swing, by id.
    pub swings: Vec<u16>,
    /// `PLO_HURTPLAYER` (40) and `PLO_HITOBJECTS` (46), in the order they came.
    pub hits: Vec<crate::combat::IncomingHit>,
    pub others: Vec<crate::protocol::OtherPlayer>,
    pub own_cell: Option<(i32, i32)>,
    pub nickname: Option<String>,
}

impl IncomingHits {
    /// No hit is waiting.
    pub fn is_empty(&self) -> bool {
        self.swings.is_empty() && self.hits.is_empty()
    }
}

/// Move the hits `st` holds to the game's queue, with the players as they
/// stand now. Drained either way, so nothing builds up with no game.
fn take_hits(st: &mut SessionState, to: Option<&SharedHits>) {
    let swings = std::mem::take(&mut st.swings);
    let hits = std::mem::take(&mut st.hits);
    let Some(to) = to else { return };
    if swings.is_empty() && hits.is_empty() {
        return;
    }
    let mut slot = to.lock().unwrap_or_else(|e| e.into_inner());
    slot.swings.extend(swings);
    slot.hits.extend(hits);
    slot.others = st.others.clone();
    slot.own_cell = st.gmap.as_ref().map(|g| g.cell);
    slot.nickname = st.nickname.clone();
}

/// [`Live::level_objects`]: only the `Bomb`, `BombDel`, `Explosion` and
/// `Arrow` kinds of [`crate::protocol::LevelObject`].
pub type SharedLevelObjects = Arc<Mutex<Vec<crate::protocol::LevelObject>>>;

/// [`Live::throws`].
pub type SharedThrows = Arc<Mutex<Vec<OtherThrow>>>;

/// Another player threw what they carried (`PLO_THROWCARRIED`, 21): the
/// object, and where they stood and faced, in the player's own cell's
/// coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OtherThrow {
    pub id: u16,
    pub sprite: u16,
    pub x: f32,
    pub y: f32,
    pub dir: u8,
}

/// Move what `st` holds of other players' bombs, explosions, arrows and
/// throws to the view's queues, in the player's own cell's coordinates.
/// Drained either way, so nothing builds up with no view.
fn take_level_objects(
    st: &mut SessionState,
    objects: Option<&SharedLevelObjects>,
    throws: Option<&SharedThrows>,
) {
    use crate::protocol::LevelObject as L;
    let own_cell = st.gmap.as_ref().map(|g| g.cell);
    let shift = |st: &SessionState, id: u16| {
        st.others
            .iter()
            .find(|o| o.id == id)
            .map_or((0.0, 0.0), |o| {
                let (x, y) = o.position_from(own_cell);
                (x - o.x, y - o.y)
            })
    };
    let mut out = Vec::new();
    for obj in std::mem::take(&mut st.level_objects) {
        match obj {
            L::Bomb { sender, mut bomb } => {
                let (dx, dy) = shift(st, sender);
                (bomb.x, bomb.y) = (bomb.x + dx, bomb.y + dy);
                out.push(L::Bomb { sender, bomb });
            }
            L::Explosion {
                sender,
                mut explosion,
            } => {
                let (dx, dy) = shift(st, sender);
                (explosion.x, explosion.y) = (explosion.x + dx, explosion.y + dy);
                out.push(L::Explosion { sender, explosion });
            }
            L::Arrow { sender, mut arrow } => {
                let (dx, dy) = shift(st, sender);
                (arrow.x, arrow.y) = (arrow.x + dx, arrow.y + dy);
                out.push(L::Arrow { sender, arrow });
            }
            o @ L::BombDel { .. } => out.push(o),
            // The baddies are not the view's yet: dropped.
            _ => {}
        }
    }
    let thrown: Vec<OtherThrow> = std::mem::take(&mut st.throws)
        .into_iter()
        .filter_map(|o| {
            let sprite = o.carry?;
            let (x, y) = o.position_from(own_cell);
            Some(OtherThrow {
                id: o.id,
                sprite,
                x,
                y,
                dir: o.dir,
            })
        })
        .collect();
    if let (Some(q), false) = (objects, out.is_empty()) {
        q.lock().unwrap_or_else(|e| e.into_inner()).extend(out);
    }
    if let (Some(q), false) = (throws, thrown.is_empty()) {
        q.lock().unwrap_or_else(|e| e.into_inner()).extend(thrown);
    }
}

/// One server record for an NPC: the level's lower-case name, the NPC id,
/// and what came. `props` is a `PLO_NPCPROPS` (3) record for the player's
/// level, with only the props it carried; `None` is a `PLO_NPCDEL2` (150)
/// delete, for the level it names.
#[derive(Debug, Clone, PartialEq)]
pub struct NpcChange {
    pub level: String,
    pub id: u32,
    pub props: Option<NpcProps>,
}

/// The player's own counters as the server last set them with
/// `PLO_PLAYERPROPS` (9), for the view's status display. `None` is a prop
/// the server has not sent yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OwnStats {
    /// MAXPOWER (1): heart containers, in hearts.
    pub max_power: Option<u8>,
    /// CURPOWER (2): power in half hearts.
    pub cur_power: Option<u8>,
    /// RUPEESCOUNT (3), the gralats.
    pub rupees: Option<u32>,
    /// ARROWSCOUNT (4).
    pub arrows: Option<u8>,
    /// BOMBSCOUNT (5).
    pub bombs: Option<u8>,
    /// GLOVEPOWER (6).
    pub glove_power: Option<u8>,
    /// STATUS (18).
    pub status: Option<u8>,
    /// ALIGNMENT (32), the AP.
    pub alignment: Option<u8>,
}

impl OwnStats {
    /// The counters `st` holds.
    pub fn of(st: &SessionState) -> OwnStats {
        OwnStats {
            max_power: st.own_max_power,
            cur_power: st.own_cur_power,
            rupees: st.own_rupees,
            arrows: st.own_darts,
            bombs: st.own_bombs,
            glove_power: st.own_glove_power,
            status: st.own_status,
            alignment: st.own_alignment,
        }
    }
}

/// The player's weapons as the server listed them: the built-in ones
/// (`PLO_DEFAULTWEAPON`, 43) by id and the server's (`PLO_NPCWEAPONADD`,
/// 33, less `PLO_NPCWEAPONDEL`, 34) with their image and GS1 script.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnWeapons {
    pub defaults: Vec<u8>,
    pub named: Vec<crate::protocol::Weapon>,
}

impl OwnWeapons {
    /// The weapons `st` holds.
    pub fn of(st: &SessionState) -> OwnWeapons {
        OwnWeapons {
            defaults: st.default_weapons.clone(),
            named: st.weapons.clone(),
        }
    }

    /// The names the item list shows, in the 2.05 order: the built-in
    /// weapons first ([`default_weapon_name`]), then the server's. A
    /// hidden weapon (its name starts with `-`) is left out, and a name
    /// listed twice shows once.
    pub fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let names = self
            .defaults
            .iter()
            .filter_map(|&i| default_weapon_name(i))
            .chain(self.named.iter().map(|w| w.name.clone()));
        for n in names {
            if !n.is_empty() && !n.starts_with('-') && !out.contains(&n) {
                out.push(n);
            }
        }
        out
    }
}

/// The name a built-in weapon (`PLO_DEFAULTWEAPON` id) has in the
/// player's list: the item name with its first letter in upper case
/// (`Bow`, `Bomb`), for the seven ids that are weapons. `None` for the
/// other items.
pub fn default_weapon_name(id: u8) -> Option<String> {
    if !matches!(id, 7 | 8 | 12 | 20 | 21 | 22 | 23) {
        return None;
    }
    let item = crate::offline::ITEM_NAMES.get(usize::from(id))?;
    let mut c = item.chars();
    let first = c.next()?;
    Some(first.to_uppercase().chain(c).collect())
}

/// The network thread writes the player's weapons here when they change.
pub type SharedWeapons = Arc<Mutex<OwnWeapons>>;

/// The network thread writes the player's counters here when they change;
/// the view's status display reads them.
pub type SharedOwn = Arc<Mutex<OwnStats>>;

/// Chat lines from the network thread to the view, oldest first: the
/// bubbles said in the level, toalls, PMs and admin lines, as core's
/// session state reads them. The view takes them.
pub type SharedChat = Arc<Mutex<Vec<crate::protocol::ChatLine>>>;

/// Sound files from the network thread to the view: name and bytes.
pub type SharedSounds = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// NPC updates from the network thread to the view, in arrival order.
pub type SharedNpcChanges = Arc<Mutex<Vec<NpcChange>>>;

/// Board changes by lower-case level name, from the network thread to the
/// view.
pub type SharedBoardMods = Arc<Mutex<Vec<(String, BoardModify2)>>>;

/// A game server the session is about to log in to: the first one, and
/// each one a server warp (`PLO_SERVERWARP`, 178) names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerJoin {
    /// `host:port`.
    pub addr: String,
    /// The warp's name as sent, with its list-type prefix (`3 Lab3D`).
    /// `None` for the first server, whose name the session does not know.
    pub name: Option<String>,
}

impl ServerJoin {
    /// A warp to a Graal3D world: its name starts with `3 `. The first
    /// server is never one here; the caller knows from the server list
    /// (`--graal3d`).
    pub fn is_graal3d(&self) -> bool {
        self.name
            .as_deref()
            .is_some_and(crate::protocol::layouts::is_graal3d_name)
    }
}

/// Files a client line asks the session to fetch over TCP, and what came
/// back. The Graal3D side uses it for the shapes and Sky textures its
/// ghosts name: login step 6 waits until every one is answered, with the
/// file or with `PLO_FILESENDFAILED` (Graal_analysis
/// `torque/udp-session.md` §1).
#[derive(Debug, Clone, Default)]
pub struct FileAsks {
    /// Names to fetch; the session takes them each tick.
    pub wanted: Arc<Mutex<Vec<String>>>,
    /// One answer per name taken, in arrival order.
    pub answers: Arc<Mutex<Vec<FileAnswer>>>,
}

/// The answer for one name in [`FileAsks::wanted`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAnswer {
    /// The name as it was asked for.
    pub name: String,
    /// The file's bytes (sent now, or a copy already kept), or `None` when
    /// the server could not send it.
    pub data: Option<Vec<u8>>,
}

/// [`Hooks::on_server`].
pub type OnServer = Arc<dyn Fn(&ServerJoin) + Send + Sync>;

/// File messages to send, by the name they ask for.
type FileSends = Vec<(String, Vec<Vec<u8>>)>;

/// What a client line hooks into the session beyond [`Live`].
#[derive(Clone, Default)]
pub struct Hooks {
    /// Called on the network thread just before each login, the first one
    /// and each server warp's, so a client can open a second link (the
    /// Graal3D UDP link) at the same moment.
    pub on_server: Option<OnServer>,
    /// Files to fetch for the client line.
    pub files: Option<FileAsks>,
}

impl std::fmt::Debug for Hooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hooks")
            .field("on_server", &self.on_server.is_some())
            .field("files", &self.files)
            .finish()
    }
}

/// The session's side of [`FileAsks`]: the names asked for and not yet
/// answered.
#[derive(Debug, Default)]
struct FileAskState {
    pending: Vec<String>,
}

/// Does a reply for `got` answer the request for `asked`? Names compare
/// without case; a name asked for with no extension (the Sky's textures)
/// is also answered by that name with one.
fn answers(asked: &str, got: &str) -> bool {
    let (a, g) = (
        asked.trim().to_ascii_lowercase(),
        got.trim().to_ascii_lowercase(),
    );
    if a == g {
        return true;
    }
    !a.contains('.') && g.rsplit_once('.').is_some_and(|(stem, _)| stem == a)
}

impl FileAskState {
    /// A new connection: what was still on the wire is asked for again.
    fn new_connection(&mut self, asks: &FileAsks) {
        let again = std::mem::take(&mut self.pending);
        if !again.is_empty() {
            let mut w = asks.wanted.lock().unwrap_or_else(|e| e.into_inner());
            let later = std::mem::replace(&mut *w, again);
            w.extend(later);
        }
    }

    /// Take the wanted names: the messages to send, and the answers known
    /// at once (a copy already kept, or a name that failed before).
    fn take(&mut self, asks: &FileAsks, files: &mut FileSession) -> (FileSends, Vec<FileAnswer>) {
        let names = std::mem::take(&mut *asks.wanted.lock().unwrap_or_else(|e| e.into_inner()));
        let mut send = Vec::new();
        let mut now = Vec::new();
        for name in names {
            let name = name.trim().to_string();
            if self.pending.iter().any(|p| p.eq_ignore_ascii_case(&name)) {
                continue;
            }
            let msgs = files.need(&name);
            let on_wire = files
                .requests
                .wanted()
                .iter()
                .any(|n| n.eq_ignore_ascii_case(&name))
                || files
                    .requests
                    .update_asked()
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(&name));
            if !msgs.is_empty() || on_wire {
                self.pending.push(name.clone());
                if !msgs.is_empty() {
                    send.push((name, msgs));
                }
            } else if files.requests.has_failed(&name) {
                now.push(FileAnswer { name, data: None });
            } else {
                let data = files.store.read(&name);
                now.push(FileAnswer { name, data });
            }
        }
        (send, now)
    }

    /// The answers in what [`FileSession::process`] found.
    fn settle(
        &mut self,
        got: &crate::filecache::Processed,
        store: &DownloadStore,
    ) -> Vec<FileAnswer> {
        let mut out = Vec::new();
        let replies = got
            .written
            .iter()
            .chain(&got.up_to_date)
            .map(|n| (n, false))
            .chain(got.failed.iter().map(|n| (n, true)));
        for (reply, failed) in replies {
            let Some(i) = self.pending.iter().position(|p| answers(p, reply)) else {
                continue;
            };
            let name = self.pending.remove(i);
            let data = if failed { None } else { store.read(reply) };
            out.push(FileAnswer { name, data });
        }
        out
    }
}

/// [`WorldView::key`]: the gmap, the cell, the neighbours' cells and the
/// crossings sent.
type WorldKey = (String, (i32, i32), Vec<(i32, i32)>, u64);

/// The gmap the server has the player on, for the view: the grid, the
/// player's cell, and the neighbouring levels the network thread has (the
/// server's boards, or local files the file rules allow), from core's
/// `gmap::sync_world`.
#[derive(Debug, Clone)]
pub struct WorldView {
    pub name: String,
    pub map: Gmap,
    pub cell: (i32, i32),
    pub levels: Vec<((i32, i32), Level)>,
    /// How many of the view's crossings the network thread had sent when
    /// it took this snapshot. While the view has sent more, `cell` is
    /// behind the view's and says nothing about a server move.
    pub crossings: u64,
}

impl WorldView {
    /// The world `sync_world` keeps in `ws`, or `None` off a gmap.
    pub fn of(ws: &OfflineSession, crossings: u64) -> Option<WorldView> {
        let w = ws.gmap.as_ref()?;
        let mut levels: Vec<((i32, i32), Level)> =
            w.levels.iter().map(|(c, l)| (*c, l.clone())).collect();
        levels.sort_by_key(|(c, _)| *c);
        Some(WorldView {
            name: w.name.clone(),
            map: w.map.clone(),
            cell: w.cell,
            levels,
            crossings,
        })
    }

    /// What tells two snapshots apart without comparing every board: the
    /// gmap, the cell, which neighbours are in, and the crossings sent.
    fn key(&self) -> WorldKey {
        (
            self.name.to_ascii_lowercase(),
            self.cell,
            self.levels.iter().map(|(c, _)| *c).collect(),
            self.crossings,
        )
    }
}

/// Level and gmap files for `sync_world`: the `.gmap` from the download
/// store (a server's copy is kept in memory, a local one is read from the
/// asset folders), levels from the asset folders as `seihai-mae` does.
struct Loader<'a>(&'a DownloadStore);

impl WorldLoader for Loader<'_> {
    fn gmap_text(&self, name: &str) -> Option<String> {
        String::from_utf8(self.0.read(name)?).ok()
    }

    fn level(&self, name: &str, server_mod_time: Option<u32>) -> Option<Level> {
        Level::load(self.0.resolver().level_at_mod_time(name, server_mod_time)?).ok()
    }
}

/// The file messages for the sounds the view is missing: each name taken
/// from `wanted` and passed to `need` (core's `FileSession::need_missing`,
/// which skips a file already on disk or already asked for).
fn sound_requests(
    wanted: &Mutex<Vec<String>>,
    mut need: impl FnMut(&str) -> Vec<Vec<u8>>,
) -> Vec<(String, Vec<Vec<u8>>)> {
    let names = std::mem::take(&mut *wanted.lock().unwrap_or_else(|e| e.into_inner()));
    names
        .into_iter()
        .map(|n| {
            let m = need(&n);
            (n, m)
        })
        .filter(|(_, m)| !m.is_empty())
        .collect()
}

/// The sound files among `written` (names [`FileSession::process`] saved
/// or kept), with their bytes from `store`, for the view's sound engine.
fn arrived_sounds_of(written: &[String], store: &DownloadStore) -> Vec<(String, Vec<u8>)> {
    written
        .iter()
        .filter(|n| crate::sound::is_sound_name(n))
        .filter_map(|n| Some((n.clone(), store.read(n)?)))
        .collect()
}

/// The other players in `st` that stand in this level: those with no gmap
/// cell of their own (Ato has no gmap view yet).
fn others_of(st: &SessionState) -> Vec<Other> {
    // Off a gmap the cell props (43/44) mean nothing: GServer sends them
    // as 0, 0 with every full OTHERPLPROPS. On a gmap, only the players in
    // the player's own cell are drawn (x, y are local to their cell).
    let own = st.gmap.as_ref().map(|g| g.cell);
    st.others
        .iter()
        .filter(|o| own.is_none() || o.cell.is_none() || o.cell == own)
        .map(|o| Other {
            id: o.id,
            x: o.x,
            y: o.y,
            dir: o.dir,
            nick: o.nick.clone(),
            sprite: o.sprite,
            status: o.status,
            looks: o.looks.clone(),
            chat: o.chat.clone().filter(|c| !c.is_empty()),
        })
        .collect()
}

/// A post-login frame is still owed: the real clients send nothing else
/// before it.
fn burst_owed_now(login: &Option<LoginBurst>, lobby: &Option<LobbyBurst>) -> bool {
    login.as_ref().is_some_and(|b| b.pending()) || lobby.as_ref().is_some_and(|b| b.pending())
}

/// Move the chat lines `st` has gathered into `to`, so the state does not
/// keep them and the view sees each once.
fn take_chat(st: &mut SessionState, to: &Mutex<Vec<crate::protocol::ChatLine>>) {
    if st.chat_log.is_empty() {
        return;
    }
    let lines = std::mem::take(&mut st.chat_log);
    to.lock().unwrap_or_else(|e| e.into_inner()).extend(lines);
}

/// The lines the view asked to say, taken from `say`. Blank lines are
/// dropped: an empty CURCHAT clears the bubble instead of saying anything.
fn take_say(say: &Mutex<Vec<String>>) -> Vec<String> {
    std::mem::take(&mut *say.lock().unwrap_or_else(|e| e.into_inner()))
        .into_iter()
        .filter(|t| !t.trim().is_empty())
        .collect()
}

/// The NPC updates and deletes `st` has gathered since the last call, in
/// the order they came, each with the level it is for. An update with no
/// level known is dropped: there is no level for the view to put it on.
fn npc_changes_of(st: &mut SessionState) -> Vec<NpcChange> {
    let changes = std::mem::take(&mut st.npc_changes);
    let deletes = std::mem::take(&mut st.npc_deletes);
    let level = st.current_level_name().map(|l| l.to_ascii_lowercase());
    let mut out = Vec::with_capacity(changes.len() + deletes.len());
    let mut deletes = deletes.into_iter().peekable();
    for (i, (id, props)) in changes.into_iter().enumerate() {
        while let Some(d) = deletes.next_if(|d| d.after <= i) {
            out.push(NpcChange {
                level: d.level.to_ascii_lowercase(),
                id: d.id,
                props: None,
            });
        }
        if let Some(level) = &level {
            out.push(NpcChange {
                level: level.clone(),
                id,
                props: Some(props),
            });
        }
    }
    out.extend(deletes.map(|d| NpcChange {
        level: d.level.to_ascii_lowercase(),
        id: d.id,
        props: None,
    }));
    out
}

/// After a link request the player stands where the link said: the
/// client moves itself there, and GServer does not send X/Y again with the
/// new level. Without this the login's X/Y stayed in the state, and the
/// new level drew the player at the login start (lab, 2026-11-11). X/Y the
/// server does send after this still win.
fn arrive_by_link(st: &mut SessionState, a: &LinkAsk) {
    st.player_x = Some(a.x);
    st.player_y = Some(a.y);
}

/// One walk tick, as the real clients run it.
const WALK_TICK: Duration = Duration::from_millis(50);

/// Sends the player's walk props on the build's own terms: core's
/// [`WalkCadence`] for the builds whose walk was captured (3.04 and the 4.x
/// builds with recorded after-login messages: changed props only, GANI
/// `walk`/`idle`), otherwise the whole walk message
/// ([`walk_message_for`]: X2/Y2 with precise positions, X/Y otherwise) at
/// most once a tick when the pose changed, as `seihai-mae` does for those
/// builds.
struct WalkSender {
    version: ClientVersion,
    cadence: Option<WalkCadence>,
    last: Option<Pose>,
    ganis: bool,
    /// A crossing's lead (46, where the build sends it), held for the next
    /// tick's frame.
    held: Vec<Vec<u8>>,
}

impl WalkSender {
    fn new(version: ClientVersion) -> Self {
        WalkSender {
            version,
            cadence: WalkCadence::for_version(version),
            last: None,
            ganis: version.profile().ui.ganis,
            held: Vec::new(),
        }
    }

    /// The view's walk crossed a gmap edge. With a walk cadence (3.04, the
    /// captured 4.x builds) the crossing's lead is held for the next tick's
    /// frame and the cell props ride the cadence's next props message, as
    /// `seihai-mae` does; the result is whether the lead has a 46. `None`
    /// means the build has no cadence and the caller sends core's whole
    /// crossing frame.
    fn cross(&mut self, c: &Crossing) -> Option<bool> {
        // The next pose is in the new cell: not a step to report as such.
        self.last = None;
        let wc = self.cadence.as_mut()?;
        wc.cross(c);
        let lead = crate::gmap::crossing_lead(self.version, None);
        let sent46 = lead
            .iter()
            .any(|m| m.first() == Some(&(crate::net::PLI_ENTERLEVEL + 0x20)));
        self.held.extend(lead);
        Some(sent46)
    }

    /// The frame for this tick: a held crossing lead, then the walk
    /// message, if any. Empty when there is nothing to send.
    fn tick(&mut self, pose: Pose) -> Vec<Vec<u8>> {
        let mut frame = std::mem::take(&mut self.held);
        frame.extend(self.walk(pose));
        frame
    }

    fn walk(&mut self, pose: Pose) -> Vec<Vec<u8>> {
        let moving = if pose.walking { "walk" } else { "idle" };
        if let Some(wc) = self.cadence.as_mut() {
            let gani = self.ganis.then_some(pose.gani.unwrap_or(moving));
            let st = wc.state(pose.x, pose.y, pose.sprite_value(), gani);
            return wc.tick(st).into_iter().collect();
        }
        let last = self.last;
        self.last = Some(pose);
        // The first pose is where the server put the player: nothing to say.
        let Some(last) = last else {
            return Vec::new();
        };
        let mut out = Vec::new();
        // A GANI build with no captured cadence names only the view's own
        // animations (a swing) and the return from one; its walk stays the
        // walk message, as before.
        if self.ganis && pose.gani != last.gani {
            out.push(gani_props_message(pose.gani.unwrap_or(moving)));
        }
        let place = |p: &Pose| (p.x, p.y, p.sprite_value());
        if place(&pose) != place(&last) {
            out.push(walk_message_for(
                self.version,
                pose.x,
                pose.y,
                pose.sprite_value(),
            ));
        }
        out
    }
}

/// As [`run`], with the game view's [`Live`] state (the walk is sent to
/// the server and the other players come back) and the client line's
/// `level_lines` under each level line.
#[allow(clippy::too_many_arguments)]
pub fn run_with(
    addr: &str,
    version: ClientVersion,
    login: &Login,
    assets: Option<PathBuf>,
    first_level_only: bool,
    out: &mut impl Write,
    shown: &mut dyn FnMut(&Level, &Arrival),
    live: Option<Live>,
    level_lines: LevelLines,
) -> Result<(), String> {
    run_hooked(
        addr,
        version,
        login,
        assets,
        first_level_only,
        out,
        shown,
        live,
        level_lines,
        &Hooks::default(),
    )
}

/// As [`run_with`], with the client line's [`Hooks`]: told of each server
/// before its login, and fetching the files it asks for.
#[allow(clippy::too_many_arguments)]
pub fn run_hooked(
    addr: &str,
    version: ClientVersion,
    login: &Login,
    assets: Option<PathBuf>,
    first_level_only: bool,
    out: &mut impl Write,
    shown: &mut dyn FnMut(&Level, &Arrival),
    live: Option<Live>,
    level_lines: LevelLines,
    hooks: &Hooks,
) -> Result<(), String> {
    let mut asked = FileAskState::default();
    // The name the next server was warped to under, as sent.
    let mut warp_name: Option<String> = None;
    let pose = live.as_ref().map(|l| l.pose.clone());
    let others = live.as_ref().map(|l| l.others.clone());
    let link = live.as_ref().map(|l| l.link.clone());
    let wanted_sounds = live.as_ref().map(|l| l.wanted_sounds.clone());
    let arrived_sounds = live.as_ref().map(|l| l.arrived_sounds.clone());
    let world = live.as_ref().map(|l| l.world.clone());
    let crossings = live.as_ref().map(|l| l.crossings.clone());
    let board_mods = live.as_ref().map(|l| l.board_mods.clone());
    let npc_changes = live.as_ref().map(|l| l.npc_changes.clone());
    let closed = live.as_ref().map(|l| l.closed.clone());
    let own = live.as_ref().map(|l| l.own.clone());
    let weapons = live.as_ref().map(|l| l.weapons.clone());
    let level_objects = live.as_ref().map(|l| l.level_objects.clone());
    let throws = live.as_ref().map(|l| l.throws.clone());
    let hits = live.as_ref().map(|l| l.hits.clone());
    let items = live.as_ref().map(|l| l.items.clone());
    let say = live.as_ref().map(|l| l.say.clone());
    let chat = live.as_ref().map(|l| l.chat.clone());
    let outbox = live.as_ref().map(|l| l.send.clone());
    // The bubbles' timers run on the loop's own clock.
    let mut bubble_clock = Instant::now();
    let view_closed = || {
        closed
            .as_ref()
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::SeqCst))
    };
    // Crossings sent, over every connection: the view counts the same.
    let mut crossings_sent: u64 = 0;
    let gmap_rules = GmapRules::for_version(version);
    let resolver = assets.map(|d| AssetResolver::new(AssetRoot::new(d), version.profile()));
    let local = |file: &str, mod_time: Option<u32>| -> Option<Level> {
        Level::load(resolver.as_ref()?.level_at_mod_time(file, mod_time)?).ok()
    };
    let file_resolver = resolver
        .clone()
        .unwrap_or_else(|| AssetResolver::empty(version.profile()));
    let mut files = FileSession::new(file_resolver, false);
    let mut addr = addr.to_string();
    let mut first_session = true;
    let mut server_name: Option<String> = None;
    'servers: loop {
        if let Some(f) = hooks.on_server.as_ref() {
            f(&ServerJoin {
                addr: addr.clone(),
                name: warp_name.take(),
            });
        }
        let mut s = login_gserver_with(
            &addr,
            version,
            random_key(),
            &login.account,
            &login.password,
            false,
            first_session,
            wantfile_message,
        )
        .map_err(|e| e.to_string())?;
        first_session = false;
        for w in &s.warnings {
            let _ = writeln!(out, "warning: {w}");
        }
        let _ = writeln!(
            out,
            "logged in at {addr} as {} ({})",
            login.account,
            version.label()
        );
        let frames = s.conn.spawn_reader().map_err(|e| e.to_string())?;
        files.new_connection();
        if let Some(a) = hooks.files.as_ref() {
            asked.new_connection(a);
        }
        let mut login_burst = LoginBurst::for_version(version);
        let mut lobby_burst = LobbyBurst::for_version(version);
        // 6.x: the nickname, once the first level is in (no recorded frame
        // carries it).
        let mut nick_owed = sends_nick_props(version);
        let title = version.profile().window_title(server_name.as_deref());
        let mut st = SessionState::for_version(version);
        let mut walker = WalkSender::new(version);
        // The gmap side of core's walk, kept here where the server's state
        // is: `sync_world` fills its world, and the view gets a copy.
        let mut ws = OfflineSession::default();
        let mut gmap_asked = std::collections::HashSet::new();
        let mut world_key = None;
        // The last link asked for: where the player arrives in its level.
        let mut last_link: Option<LinkAsk> = None;
        let mut walk_clock = Instant::now();
        let mut store = LevelStore::default();
        // A level was stored and has not been logged yet.
        let mut owed = false;
        let mut owed_at: Option<Instant> = None;
        let mut quit = false;
        loop {
            if view_closed() {
                let _ = s.conn.shutdown();
                let _ = writeln!(out, "the window closed: logged out");
                return Ok(());
            }
            let burst_owed = login_burst.as_ref().is_some_and(|b| b.pending())
                || lobby_burst.as_ref().is_some_and(|b| b.pending());
            let mut wait = match (owed, burst_owed) {
                (_, true) => BURST_TICK,
                (true, false) => SETTLE,
                (false, false) => Duration::from_secs(3600),
            };
            // A view polls for its window closing each walk tick.
            // So does a client line's file list.
            if pose.is_some() || closed.is_some() || hooks.files.is_some() {
                wait = wait.min(WALK_TICK);
            }
            // The time `owed` started, so the settle wait is not cut short
            // by the burst tick.
            let settle_until = owed_at.map(|t| t + SETTLE);
            match frames.recv_timeout(wait) {
                Ok(Ok(body)) => {
                    st.apply_body(&body);
                    if let Some(o) = others.as_ref() {
                        let now = others_of(&st);
                        let mut slot = o.lock().unwrap_or_else(|e| e.into_inner());
                        if *slot != now {
                            *slot = now;
                        }
                    }
                    if let Some(o) = own.as_ref() {
                        let now = OwnStats::of(&st);
                        let mut slot = o.lock().unwrap_or_else(|e| e.into_inner());
                        if *slot != now {
                            *slot = now;
                        }
                    }
                    if let Some(w) = weapons.as_ref() {
                        let mut slot = w.lock().unwrap_or_else(|e| e.into_inner());
                        if slot.defaults != st.default_weapons || slot.named != st.weapons {
                            *slot = OwnWeapons::of(&st);
                        }
                    }
                    if let Some(c) = chat.as_ref() {
                        take_chat(&mut st, c);
                    }
                    take_level_objects(&mut st, level_objects.as_ref(), throws.as_ref());
                    take_hits(&mut st, hits.as_ref());
                    take_items(&mut st, items.as_ref());
                    if let Some(w) = st.server_warp.take() {
                        if owed {
                            log_level(level_lines, &store, &st, out, shown);
                        }
                        let _ = s.conn.shutdown();
                        let name = w.display_name();
                        let Some(to) = w.address() else {
                            return Err(format!(
                                "server warp to {name:?} has no usable host and port"
                            ));
                        };
                        if let Some(o) = others.as_ref() {
                            o.lock().unwrap_or_else(|e| e.into_inner()).clear();
                        }
                        let _ = writeln!(out, "server warp to {name:?}: {to}");
                        addr = to;
                        warp_name = Some(w.name.clone());
                        server_name = Some(name);
                        continue 'servers;
                    }
                    let before = st.current_level_name();
                    // A new level ends the one owed: log it as it stood.
                    if owed
                        && store.level_name().is_some_and(|n| {
                            before.as_deref().is_none_or(|b| !b.eq_ignore_ascii_case(n))
                        })
                    {
                        log_level(level_lines, &store, &st, out, shown);
                        owed = false;
                    }
                    if store.update(&mut st, local) {
                        owed = true;
                        owed_at = Some(Instant::now());
                    }
                    if st.current_level_name().is_some() {
                        let now = Instant::now();
                        if let Some(b) = login_burst.as_mut() {
                            b.note_level(now);
                        }
                        if let Some(b) = lobby_burst.as_mut() {
                            b.note_level(now);
                        }
                    }
                    let got = files.process(&mut st);
                    if let Some(a) = hooks.files.as_ref() {
                        let done = asked.settle(&got, &files.store);
                        if !done.is_empty() {
                            a.answers
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .extend(done);
                        }
                    }
                    if !got.send.is_empty() {
                        let _ = s.conn.send_messages(&got.send);
                    }
                    if let Some(a) = arrived_sounds.as_ref() {
                        let sounds = arrived_sounds_of(&got.written, &files.store);
                        if !sounds.is_empty() {
                            a.lock().unwrap_or_else(|e| e.into_inner()).extend(sounds);
                        }
                    }
                    if nick_owed && st.current_level_name().is_some() {
                        nick_owed = false;
                        let nick = if login.nick.is_empty() {
                            &login.account
                        } else {
                            &login.nick
                        };
                        if s.conn.send_messages(&[nick_props_message(nick)]).is_ok() {
                            let _ = writeln!(out, "sent the nickname {nick:?}");
                        }
                    }
                    // Board changes: onto this side's gmap levels, and on to
                    // the view.
                    let mods = st.take_board_mods();
                    if !mods.is_empty() {
                        let here = st.current_level_name();
                        apply_board_mods_to(&mut ws, here.as_deref(), &mods);
                        if let Some(b) = board_mods.as_ref() {
                            b.lock().unwrap_or_else(|e| e.into_inner()).extend(mods);
                        }
                    }
                    // NPC updates for the player's level: on to the view,
                    // which has the level's NPCs. Taken either way, so they
                    // do not pile up in the state.
                    let changes = npc_changes_of(&mut st);
                    if let (Some(n), false) = (npc_changes.as_ref(), changes.is_empty()) {
                        n.lock().unwrap_or_else(|e| e.into_inner()).extend(changes);
                    }
                    // The gmap: the grid, the cell, the neighbours, and a
                    // request for what the server still has to send.
                    if world.is_some() {
                        let req = sync_world(
                            &mut ws,
                            &mut st,
                            &gmap_rules,
                            &Loader(&files.store),
                            &mut gmap_asked,
                        );
                        if let Some(f) = req.gmap_file.as_ref().or(req.gmap_local.as_ref()) {
                            let msgs = files.need(f);
                            if !msgs.is_empty() && s.conn.send_messages(&msgs).is_ok() {
                                let _ = writeln!(out, "requesting {f}");
                            }
                        }
                        if !req.messages.is_empty() {
                            let _ = s.conn.send_messages(&req.messages);
                        }
                    }
                    if let Some(w) = world.as_ref() {
                        let view = WorldView::of(&ws, crossings_sent);
                        let key = view.as_ref().map(WorldView::key);
                        if key != world_key {
                            world_key = key;
                            *w.lock().unwrap_or_else(|e| e.into_inner()) = view;
                        }
                    }
                }
                Ok(Err(e)) => {
                    if owed {
                        log_level(level_lines, &store, &st, out, shown);
                    }
                    let why = st
                        .disconnect_message
                        .take()
                        .unwrap_or_else(|| e.to_string());
                    let _ = writeln!(out, "connection closed: {why}");
                    return Ok(());
                }
                Err(RecvTimeoutError::Timeout) => {
                    if owed && settle_until.is_some_and(|t| Instant::now() >= t) {
                        log_level(level_lines, &store, &st, out, shown);
                        owed = false;
                        owed_at = None;
                        // Hang up once the post-login frames due by now
                        // have gone (below), as a client would have sent
                        // them before the level settled.
                        quit = first_level_only;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
            // The other players' bubbles run out on their own timers.
            let dt = bubble_clock.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
            bubble_clock = Instant::now();
            if st.tick_bubbles(dt) {
                if let Some(o) = others.as_ref() {
                    *o.lock().unwrap_or_else(|e| e.into_inner()) = others_of(&st);
                }
            }
            // What the view built for the server, once the post-login frame
            // has gone.
            if let Some(q) = outbox
                .as_ref()
                .filter(|_| !burst_owed_now(&login_burst, &lobby_burst))
            {
                let msgs = std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()));
                if !msgs.is_empty() {
                    let _ = s.conn.send_messages(&msgs);
                }
            }
            // What the player typed: said in the level.
            if let Some(q) = say.as_ref() {
                for text in take_say(q) {
                    if s.conn.send_chat(&text).is_ok() {
                        let _ = writeln!(out, "said {text:?}");
                    }
                }
            }
            // The walk, once a tick, after the post-login frame has gone
            // (the real clients send nothing before it).
            let burst_owed = login_burst.as_ref().is_some_and(|b| b.pending())
                || lobby_burst.as_ref().is_some_and(|b| b.pending());
            if let Some(p) = pose.as_ref() {
                if !burst_owed && walk_clock.elapsed() >= WALK_TICK {
                    walk_clock = Instant::now();
                    let current = *p.lock().unwrap_or_else(|e| e.into_inner());
                    let frame = current.map(|c| walker.tick(c)).unwrap_or_default();
                    if !frame.is_empty() {
                        let _ = s.conn.send_messages(&frame);
                    }
                }
            }
            // A new level after login owes a 46 on the builds that send it
            // (core's EnterLevelSender), with the arrival props after it in
            // the same frame, as the real 2.31 and 4.x clients do.
            if let Some(name) = st.current_level_name() {
                s.enter_level.note_level(&name);
            }
            if s.enter_level.is_pending() {
                let then: Vec<Vec<u8>> = last_link
                    .as_ref()
                    .filter(|a| {
                        st.current_level_name()
                            .is_some_and(|n| n.eq_ignore_ascii_case(&a.dest))
                    })
                    .map(|a| vec![walk_message_for(version, a.x, a.y, a.dir)])
                    .unwrap_or_default();
                let _ = s.enter_level.send_pending(&mut s.conn, &then);
            }
            // A gmap edge the view's walk crossed: the player is in the next
            // cell already. The frame carries the cell prop and the wrapped
            // position, and the server follows with the new cell's level.
            let crossed = crossings
                .as_ref()
                .map(|c| std::mem::take(&mut *c.lock().unwrap_or_else(|e| e.into_inner())))
                .unwrap_or_default();
            for c in crossed {
                crossings_sent += 1;
                st.set_gmap_cell(c.to);
                let sent46 = match walker.cross(&c) {
                    Some(sent46) => Ok(sent46),
                    None => s.conn.send_gmap_crossing(version, &c, None),
                };
                if let (Ok(sent46), Some(n)) = (sent46, st.current_level_name()) {
                    s.enter_level.crossing(&n, sent46);
                }
                let _ = writeln!(
                    out,
                    "gmap crossing {:?} -> {:?} at {} {}",
                    c.from, c.to, c.local_x, c.local_y
                );
            }
            // A level link the player walked into: the request the build
            // sends (core's `link_request_frame`: `PLI_LEVELWARP`, or the
            // cached form with its mod time where the build has one). The
            // server answers with the level, which is logged and shown as
            // at login.
            let ask = link
                .as_ref()
                .and_then(|l| l.lock().unwrap_or_else(|e| e.into_inner()).take());
            if let Some(a) = ask {
                let cached = if version.sends_cached_links() {
                    st.cached_mod_time(&a.dest)
                } else {
                    None
                };
                let frame = link_request_frame(version, cached, a.x, a.y, a.dir, &a.dest, None);
                last_link = Some(a.clone());
                if s.conn.send_messages(&frame.messages).is_ok() {
                    if cached.is_some() {
                        st.expect_cached_level(&a.dest);
                        s.enter_level.cached_link(&a.dest, frame.with_enter_level);
                    }
                    arrive_by_link(&mut st, &a);
                    let _ = writeln!(out, "link to {} at {} {}", a.dest, a.x, a.y);
                }
            }
            // Files the client line asked for: ask the server for them.
            if let Some(a) = hooks.files.as_ref() {
                let (send, now) = asked.take(a, &mut files);
                for (name, msgs) in send {
                    if s.conn.send_messages(&msgs).is_ok() {
                        let _ = writeln!(out, "requesting {name}");
                    }
                }
                if !now.is_empty() {
                    a.answers
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .extend(now);
                }
            }
            // Sounds the view is missing: ask the server for them.
            if let Some(w) = wanted_sounds.as_ref() {
                for (name, msgs) in sound_requests(w, |n| files.need_missing(n)) {
                    if s.conn.send_messages(&msgs).is_ok() {
                        let _ = writeln!(out, "requesting {name}");
                    }
                }
            }
            // The post-login frame, once it is due (core decides when).
            let now = Instant::now();
            if let Some(b) = login_burst.as_mut() {
                let el = &mut s.enter_level;
                let conn = &mut s.conn;
                let msgs = b.take(
                    &st,
                    &login.account,
                    now,
                    |n| files.need(n),
                    || el.take_login(),
                    Vec::new,
                );
                if !msgs.is_empty() {
                    let _ = conn.send_messages(&msgs);
                    let _ = writeln!(out, "sent the post-login frame ({} messages)", msgs.len());
                }
            }
            if let Some(b) = lobby_burst.as_mut() {
                let el = &mut s.enter_level;
                let frames = b.take(
                    &st,
                    if login.nick.is_empty() {
                        &login.account
                    } else {
                        &login.nick
                    },
                    now,
                    |n| files.need(n),
                    || el.take_login(),
                    &[title.as_str()],
                );
                if !frames.is_empty() {
                    for f in &frames {
                        let _ = s.conn.send_messages(f);
                    }
                    let _ = writeln!(out, "sent the post-login frames ({})", frames.len());
                }
            }
            if quit {
                let _ = s.conn.shutdown();
                return Ok(());
            }
        }
    }
}

fn log_level(
    level_lines: LevelLines,
    store: &LevelStore,
    st: &SessionState,
    out: &mut impl Write,
    shown: &mut dyn FnMut(&Level, &Arrival),
) {
    if let (Some(a), Some(level)) = (store.arrival(st), store.level.as_ref()) {
        let _ = writeln!(out, "{}", a.line());
        for l in level_lines(level, &a.name, st) {
            let _ = writeln!(out, "{l}");
        }
        for l in a.script_lines() {
            let _ = writeln!(out, "{l}");
        }
        shown(level, &a);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(x: f32, dir: u8, walking: bool) -> Pose {
        Pose {
            x,
            y: 30.0,
            dir,
            walking,
            ..Default::default()
        }
    }

    /// 3.153 has X2/Y2 and no captured cadence: the whole walk message,
    /// once the pose changes from where the server put the player.
    #[test]
    fn a_build_without_a_cadence_sends_its_walk_message_on_change() {
        let mut w = WalkSender::new(ClientVersion::V3153);
        assert!(w.cadence.is_none());
        assert!(w.tick(pose(30.0, 2, false)).is_empty());
        assert!(w.tick(pose(30.0, 2, false)).is_empty());
        let m = w.tick(pose(30.5, 3, true));
        assert_eq!(
            m,
            vec![walk_message_for(ClientVersion::V3153, 30.5, 30.0, 3)]
        );
        assert!(w.tick(pose(30.5, 3, true)).is_empty());
    }

    /// A 6.x swing: the GANI prop names `sword` when the view starts it and
    /// `idle` when it ends; standing still, no walk message goes with it.
    #[test]
    fn a_gani_build_names_the_swing_and_its_end() {
        let v = ClientVersion::V603R7;
        let mut w = WalkSender::new(v);
        assert!(w.cadence.is_none() && w.ganis);
        assert!(w.tick(pose(30.0, 2, false)).is_empty());
        let mut swing = pose(30.0, 2, false);
        swing.gani = Some("sword");
        assert_eq!(w.tick(swing), vec![gani_props_message("sword")]);
        assert!(w.tick(swing).is_empty(), "once");
        assert_eq!(
            w.tick(pose(30.0, 2, false)),
            vec![gani_props_message("idle")]
        );
        assert_eq!(gani_props_message("sword"), b"\"*%sword\n");
    }

    /// A sprite build shows the swing in SPRITE: the walk message carries
    /// `dir + 4 * S`.
    #[test]
    fn a_sprite_build_sends_the_swing_sprite() {
        let v = ClientVersion::V3153;
        let mut w = WalkSender::new(v);
        assert!(w.tick(pose(30.0, 2, false)).is_empty());
        let mut swing = pose(30.0, 2, false);
        swing.sprite = Some(2 + 4 * 6);
        let got = w.tick(swing);
        assert!(
            got.contains(&walk_message_for(v, 30.0, 30.0, 26)),
            "{got:?}"
        );
    }

    /// 3.04 and 4.04r2 walk on core's cadence: nothing on the first tick,
    /// then the changed props.
    #[test]
    fn the_captured_builds_walk_on_cores_cadence() {
        for v in [ClientVersion::V304, ClientVersion::V404R2] {
            let mut w = WalkSender::new(v);
            assert!(w.cadence.is_some(), "{}", v.label());
            assert!(w.tick(pose(30.0, 2, false)).is_empty(), "{}", v.label());
            let mut sent = 0;
            for i in 1..=4 {
                sent += !w.tick(pose(30.0 + 0.5 * i as f32, 3, true)).is_empty() as u32;
            }
            assert!(sent >= 1, "{}", v.label());
        }
    }

    /// A `.gmap` the server sent is kept in memory (and on disk where there
    /// is a download folder, which this test has not); the loader finds it, and `WorldView` carries the world `sync_world`
    /// built, neighbours in cell order.
    #[test]
    fn the_server_gmap_is_read_from_memory_and_shown_to_the_view() {
        let v = ClientVersion::V304;
        let mut store = DownloadStore::new(AssetResolver::empty(v.profile()), false);
        let text = "GRMAP001\nWIDTH 2\nHEIGHT 1\nLEVELNAMES\n\"a.nw\",\"b.nw\"\nLEVELNAMESEND\n";
        // No download folder: the disk write fails, the memory copy stays.
        let _ = store.store("w.gmap", text.as_bytes(), Some(1));
        let loader = Loader(&store);
        assert_eq!(loader.gmap_text("w.gmap").as_deref(), Some(text));
        assert!(loader.level("a.nw", None).is_none());
        let mut ws = OfflineSession::default();
        assert!(WorldView::of(&ws, 0).is_none());
        ws.enter_gmap("w.gmap".into(), Gmap::parse(text).unwrap(), (1, 0));
        let w = ws.gmap.as_mut().unwrap();
        w.levels.insert((0, 0), Level::from_board(&[0; 4096]));
        let view = WorldView::of(&ws, 3).unwrap();
        assert_eq!((view.cell, view.crossings), ((1, 0), 3));
        assert_eq!(view.levels.len(), 1);
        assert_eq!(view.key().2, vec![(0, 0)]);
    }

    fn crossing() -> Crossing {
        Crossing {
            from: (0, 0),
            to: (1, 0),
            local_x: 0.5,
            local_y: 30.0,
        }
    }

    /// A build without a cadence leaves the crossing frame to the caller,
    /// and the next pose (in the new cell) is not reported as a step.
    #[test]
    fn a_crossing_without_a_cadence_is_the_callers() {
        let mut w = WalkSender::new(ClientVersion::V3153);
        w.tick(pose(30.0, 2, false));
        assert_eq!(w.cross(&crossing()), None);
        assert!(w.tick(pose(0.5, 3, true)).is_empty());
    }

    /// With a cadence the crossing's lead opens the next tick's frame, once.
    #[test]
    fn a_crossing_lead_rides_the_next_tick() {
        for v in [ClientVersion::V304, ClientVersion::V404R2] {
            let mut w = WalkSender::new(v);
            w.tick(pose(30.0, 2, false));
            let lead = crate::gmap::crossing_lead(v, None);
            let sent46 = w.cross(&crossing()).expect("cadence");
            assert_eq!(sent46, v.sends_enter_level(), "{}", v.label());
            let frame = w.tick(pose(0.5, 3, true));
            assert!(frame.starts_with(&lead), "{}", v.label());
            let again = w.tick(pose(0.5, 3, true));
            assert!(
                lead.is_empty() || !again.starts_with(&lead),
                "{}",
                v.label()
            );
        }
    }
}

#[cfg(test)]
mod others_tests {
    use super::*;

    /// `PLO_OTHERPLPROPS` (8) for player `id` at half-tile `x`, `y`.
    fn other(id: u16, x: u8, y: u8) -> Vec<u8> {
        let mut m = vec![8 + 0x20];
        m.extend([((id >> 7) & 0x7F) as u8 + 0x20, (id & 0x7F) as u8 + 0x20]);
        m.extend([15 + 0x20, x + 0x20, 16 + 0x20, y + 0x20, b'\n']);
        m
    }

    #[test]
    fn the_players_the_server_places_are_shared() {
        let mut st = SessionState::for_version(ClientVersion::V3153);
        st.apply_body(&[other(2, 20, 24), other(3, 40, 10)].concat());
        let o = others_of(&st);
        let got: Vec<_> = o.iter().map(|o| (o.id, o.x, o.y)).collect();
        assert_eq!(got, vec![(2, 10.0, 12.0), (3, 20.0, 5.0)]);
    }

    /// The view gets what it draws a player with: the pictures, the
    /// colours, the GANI and the sprite.
    #[test]
    fn the_looks_and_pose_go_with_the_player() {
        let mut st = SessionState::for_version(ClientVersion::V3153);
        st.apply_body(&other(2, 20, 24));
        st.others[0].looks.head = Some("head7.png".into());
        st.others[0].looks.colors = Some([1, 2, 3, 4, 5]);
        st.others[0].looks.gani = Some("walk".into());
        st.others[0].sprite = Some(2 + 4);
        st.others[0].status = Some(0);
        let o = &others_of(&st)[0];
        assert_eq!(o.looks.head.as_deref(), Some("head7.png"));
        assert_eq!(o.looks.colors, Some([1, 2, 3, 4, 5]));
        assert_eq!(o.looks.gani.as_deref(), Some("walk"));
        assert_eq!((o.sprite, o.status), (Some(6), Some(0)));
    }

    /// A player's chat bubble goes to the view while it lasts. An empty
    /// CURCHAT lets it end once its minimum time is up (core's 2.05 rule),
    /// and the loop's bubble clock takes it down then.
    #[test]
    fn the_bubble_goes_with_the_player() {
        let mut st = SessionState::for_version(ClientVersion::V3153);
        st.apply_body(&other(2, 20, 24));
        // OTHERPLPROPS (8) for player 2: CURCHAT (12) "hi".
        let say = |text: &str| {
            let mut m = vec![8 + 0x20, 0x20, 2 + 0x20, 12 + 0x20, text.len() as u8 + 0x20];
            m.extend_from_slice(text.as_bytes());
            m.push(b'\n');
            m
        };
        st.apply_body(&say("hi"));
        assert_eq!(others_of(&st)[0].chat.as_deref(), Some("hi"));
        st.apply_body(&say(""));
        assert!(st.tick_bubbles(60_000), "the bubble ends");
        assert_eq!(others_of(&st)[0].chat, None);
    }

    /// Other players' swings and the hit packets go to the game with the
    /// players as they stand, once each; the state keeps none, and with no
    /// game they are dropped.
    #[test]
    fn the_hits_go_to_the_game_with_the_players() {
        let mut st = SessionState::for_version(ClientVersion::V205);
        st.apply_body(&other(2, 20, 24));
        st.nickname = Some("me".into());
        st.swings.push(2);
        let hurt = crate::combat::HurtPlayer {
            attacker: 2,
            dx: 1.0,
            dy: 0.0,
            half: 2,
            npc: None,
        };
        st.hits.push(crate::combat::IncomingHit::Hurt(hurt));
        let to: SharedHits = Arc::default();
        take_hits(&mut st, Some(&to));
        assert!(st.swings.is_empty() && st.hits.is_empty());
        let got = std::mem::take(&mut *to.lock().unwrap());
        assert_eq!(got.swings, vec![2]);
        assert_eq!(got.hits.len(), 1);
        assert_eq!(got.others.len(), 1);
        assert_eq!(got.nickname.as_deref(), Some("me"));
        assert!(!got.is_empty());
        take_hits(&mut st, Some(&to));
        assert!(to.lock().unwrap().is_empty());
        st.swings.push(2);
        take_hits(&mut st, None);
        assert!(st.swings.is_empty());
    }

    /// The server's floor item changes go to the game once each, in
    /// order; with no game they are dropped.
    #[test]
    fn the_server_items_go_to_the_game() {
        use crate::protocol::ServerItem;
        let mut st = SessionState::for_version(ClientVersion::V205);
        st.items.push(ServerItem::Del(10.0, 12.0));
        st.items.push(ServerItem::Del(11.0, 12.0));
        let to: SharedItems = Arc::default();
        take_items(&mut st, Some(&to));
        assert!(st.items.is_empty());
        let got = std::mem::take(&mut *to.lock().unwrap());
        assert!(
            matches!(got[..], [ServerItem::Del(x, _), ServerItem::Del(y, _)] if x == 10.0 && y == 11.0)
        );
        st.items.push(ServerItem::Del(1.0, 1.0));
        take_items(&mut st, None);
        assert!(st.items.is_empty());
    }

    /// The chat lines move to the view once each; the state keeps none.
    #[test]
    fn chat_lines_go_to_the_view_once() {
        use crate::protocol::ChatKind;
        let mut st = SessionState::for_version(ClientVersion::V3153);
        let to = Mutex::new(Vec::new());
        st.push_chat(ChatKind::Toall, "Ann".into(), "hello".into());
        take_chat(&mut st, &to);
        take_chat(&mut st, &to);
        let got = to.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(
            (got[0].from.as_str(), got[0].text.as_str()),
            ("Ann", "hello")
        );
        assert!(st.chat_log.is_empty());
    }

    /// What the player typed is taken in order; blank lines are not said.
    #[test]
    fn typed_lines_are_taken_in_order_without_blanks() {
        let q = Mutex::new(vec!["one".to_string(), "  ".into(), "two".into()]);
        assert_eq!(take_say(&q), vec!["one".to_string(), "two".into()]);
        assert!(take_say(&q).is_empty());
    }

    /// Other players' bombs and arrows go to the view's queue; the
    /// baddies do not, and the session's list is drained.
    #[test]
    fn other_players_bombs_and_arrows_go_to_the_view() {
        use crate::protocol::LevelObject as L;
        let mut st = SessionState::for_version(ClientVersion::V205);
        let bomb = crate::bombs::BombAdd {
            x: 10.0,
            y: 12.0,
            power: 1,
            fuse: 55,
            owned: true,
        };
        st.level_objects.push(L::BaddiesClear);
        st.level_objects.push(L::Bomb { sender: 3, bomb });
        st.level_objects.push(L::BombDel { x: 1.0, y: 2.0 });
        let q = SharedLevelObjects::default();
        let t = SharedThrows::default();
        take_level_objects(&mut st, Some(&q), Some(&t));
        assert!(st.level_objects.is_empty());
        assert_eq!(
            *q.lock().unwrap(),
            vec![L::Bomb { sender: 3, bomb }, L::BombDel { x: 1.0, y: 2.0 }]
        );
        assert!(t.lock().unwrap().is_empty());
        // With no view, the list is still drained.
        st.level_objects.push(L::Bomb { sender: 3, bomb });
        take_level_objects(&mut st, None, None);
        assert!(st.level_objects.is_empty());
    }

    /// The item list's names: built-in weapons first, by their item
    /// names, then the server's; hidden and repeated names left out.
    #[test]
    fn own_weapons_list_built_ins_then_the_servers() {
        use crate::protocol::Weapon;
        let w = |n: &str| Weapon {
            name: n.into(),
            image: None,
            script: None,
        };
        let o = OwnWeapons {
            defaults: vec![8, 3, 7],
            named: vec![w("Lamp"), w("-hidden"), w("Bow"), w("")],
        };
        assert_eq!(o.names(), vec!["Bomb", "Bow", "Lamp"]);
        assert_eq!(default_weapon_name(23).as_deref(), Some("Joltbomb"));
        assert_eq!(default_weapon_name(5), None);
    }

    /// The counters the status display draws come from the player props.
    #[test]
    fn own_counters_from_the_player_props() {
        let mut st = SessionState::for_version(ClientVersion::V3153);
        // PLAYERPROPS (9): MAXPOWER 3, CURPOWER 5, ARROWSCOUNT 7,
        // BOMBSCOUNT 15, ALIGNMENT 50.
        st.apply_body(&[
            9 + 0x20,
            1 + 0x20,
            3 + 0x20,
            2 + 0x20,
            5 + 0x20,
            4 + 0x20,
            7 + 0x20,
            5 + 0x20,
            15 + 0x20,
            32 + 0x20,
            50 + 0x20,
            b'\n',
        ]);
        let o = OwnStats::of(&st);
        assert_eq!(
            (o.max_power, o.cur_power, o.arrows, o.bombs, o.alignment),
            (Some(3), Some(5), Some(7), Some(15), Some(50))
        );
        assert_eq!(o.rupees, None);
    }

    /// An NPC update after the level is handed on with the level it is
    /// for, and only once.
    #[test]
    fn npc_updates_go_to_the_view_with_their_level() {
        use crate::protocol::npcprops::{npc_props_message, PropValue, NPCPROP_X};
        let mut st = SessionState::for_version(ClientVersion::V3153);
        st.apply_body(&npc_props_message(5, &[(NPCPROP_X, PropValue::Byte(60))]));
        assert!(npc_changes_of(&mut st).is_empty(), "no level yet");
        st.level_name = Some("Start.nw".into());
        st.apply_body(&npc_props_message(5, &[(NPCPROP_X, PropValue::Byte(60))]));
        let got = npc_changes_of(&mut st);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].level.as_str(), got[0].id), ("start.nw", 5));
        assert_eq!(got[0].props.as_ref().and_then(|p| p.x), Some(30.0));
        assert!(npc_changes_of(&mut st).is_empty());
    }

    /// `PLO_NPCDEL2`: GCHAR length, the level, GINT3 id.
    fn npc_del2(level: &str, id: u32) -> Vec<u8> {
        let mut m = vec![150 + 0x20, level.len() as u8 + 0x20];
        m.extend(level.as_bytes());
        m.extend([
            ((id >> 14) & 0x7f) as u8 + 0x20,
            ((id >> 7) & 0x7f) as u8 + 0x20,
            (id & 0x7f) as u8 + 0x20,
        ]);
        m.push(b'\n');
        m
    }

    /// A delete goes to the view in its place among the updates, for the
    /// level it names, and the server's record of the NPC is forgotten.
    #[test]
    fn npc_deletes_go_to_the_view_in_order() {
        use crate::protocol::npcprops::{npc_props_message, PropValue, NPCPROP_X};
        let mut st = SessionState::for_version(ClientVersion::V3153);
        st.level_name = Some("start.nw".into());
        let x = |id| npc_props_message(id, &[(NPCPROP_X, PropValue::Byte(60))]);
        st.apply_body(&[x(5), npc_del2("Start.nw", 5), x(5), npc_del2("other.nw", 9)].concat());
        let got: Vec<_> = npc_changes_of(&mut st)
            .into_iter()
            .map(|c| (c.level, c.id, c.props.is_some()))
            .collect();
        assert_eq!(
            got,
            [
                ("start.nw".to_string(), 5, true),
                ("start.nw".to_string(), 5, false),
                ("start.nw".to_string(), 5, true),
                ("other.nw".to_string(), 9, false),
            ]
        );
        // Re-added after the delete: the record is the new one.
        assert!(st.server_npcs.contains_key(&5));
        st.apply_body(&npc_del2("start.nw", 5));
        assert!(!st.server_npcs.contains_key(&5));
        assert_eq!(st.npc_dels, 3);
    }

    /// GServer's full OTHERPLPROPS carries GMAPLEVELX/Y (43/44) = 0 on a
    /// plain level too (lab, 2026-11-11: two Ato clients never saw each
    /// other). Off a gmap that cell is ignored.
    #[test]
    fn a_zero_cell_off_a_gmap_still_draws_the_player() {
        let mut st = SessionState::for_version(ClientVersion::V3153);
        let mut m = other(2, 20, 24);
        m.pop();
        m.extend([43 + 0x20, 0x20, 44 + 0x20, 0x20, b'\n']);
        st.apply_body(&m);
        assert_eq!(st.others[0].cell, Some((0, 0)));
        assert!(st.gmap.is_none());
        let got: Vec<_> = others_of(&st).iter().map(|o| o.id).collect();
        assert_eq!(got, vec![2]);
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;

    /// The login put the player at (23.5, 30.5); the link says (54.5, 19).
    /// The next level's start is the link's, not the login's.
    #[test]
    fn a_link_moves_the_start_to_its_destination() {
        let mut st = SessionState::for_version(ClientVersion::V3153);
        st.player_x = Some(23.5);
        st.player_y = Some(30.5);
        let a = LinkAsk {
            dest: "egk-10.graal".into(),
            x: 54.5,
            y: 19.0,
            dir: 2,
        };
        arrive_by_link(&mut st, &a);
        assert_eq!(st.player_x.zip(st.player_y), Some((54.5, 19.0)));
    }
}

#[cfg(test)]
mod sound_request_tests {
    use super::*;

    /// Each wanted name is taken once; a name `need` has nothing for (on
    /// disk, or asked for already) sends nothing.
    #[test]
    fn wanted_sounds_are_asked_for_once() {
        let w = Mutex::new(vec!["steps.wav".to_string(), "have.wav".to_string()]);
        let got = sound_requests(&w, |n| {
            if n == "have.wav" {
                Vec::new()
            } else {
                vec![n.as_bytes().to_vec()]
            }
        });
        assert_eq!(
            got,
            vec![("steps.wav".to_string(), vec![b"steps.wav".to_vec()])]
        );
        assert!(w.lock().unwrap().is_empty());
        assert!(sound_requests(&w, |_| vec![vec![1]]).is_empty());
    }

    /// A sound the server sent is handed on with its bytes, from the
    /// in-memory store; other files are not (lab #441).
    #[test]
    fn arrived_sounds_come_from_the_memory_store() {
        let mut store =
            DownloadStore::new(AssetResolver::empty(ClientVersion::V603R7.profile()), false);
        store.store("steps2.wav", b"RIFF..", None).unwrap();
        store.store("pics1.png", b"png", None).unwrap();
        let written = vec!["steps2.wav".to_string(), "pics1.png".to_string()];
        assert_eq!(
            arrived_sounds_of(&written, &store),
            vec![("steps2.wav".to_string(), b"RIFF..".to_vec())]
        );
    }
}

#[cfg(test)]
mod hook_tests {
    use super::*;
    use crate::assets::AssetResolver;
    use crate::filecache::Processed;

    fn session() -> FileSession {
        FileSession::new(AssetResolver::empty(ClientVersion::V512.profile()), false)
    }

    fn ask(asks: &FileAsks, names: &[&str]) {
        asks.wanted
            .lock()
            .unwrap()
            .extend(names.iter().map(|n| n.to_string()));
    }

    #[test]
    fn a_warp_name_with_the_3_prefix_is_a_graal3d_server() {
        let j = |name: Option<&str>| ServerJoin {
            addr: "127.0.0.1:14900".into(),
            name: name.map(String::from),
        };
        assert!(j(Some("3 Lab3D")).is_graal3d());
        assert!(!j(Some("Lab Server")).is_graal3d());
        assert!(!j(Some("3D world")).is_graal3d());
        assert!(!j(None).is_graal3d());
    }

    #[test]
    fn a_texture_asked_for_without_an_extension_is_answered_by_its_file() {
        assert!(answers("labcube.3ds", "LabCube.3DS"));
        assert!(answers("sky_front", "sky_front.png"));
        assert!(!answers("sky_front.png", "sky_front"));
        assert!(!answers("labcube.3ds", "labcube.dts"));
    }

    /// Each name goes out once; the server's file, its failure and an
    /// already-kept copy each give one answer, under the name asked for.
    #[test]
    fn every_asked_name_gets_one_answer() {
        let asks = FileAsks::default();
        let mut files = session();
        let mut st = FileAskState::default();
        ask(
            &asks,
            &["labcube.3ds", "sky_front", "labcube.3ds", "missing.dts"],
        );
        let (send, now) = st.take(&asks, &mut files);
        let sent: Vec<&str> = send.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(sent, ["labcube.3ds", "sky_front", "missing.dts"]);
        assert!(now.is_empty());
        // Asked again while on the wire: nothing more goes out.
        ask(&asks, &["LABCUBE.3DS"]);
        let (send, now) = st.take(&asks, &mut files);
        assert!(send.is_empty() && now.is_empty());

        files.store.store("labcube.3ds", b"3ds", None).unwrap();
        files.store.store("sky_front.png", b"png", None).unwrap();
        let got = Processed {
            written: vec!["labcube.3ds".into(), "sky_front.png".into()],
            failed: vec!["missing.dts".into()],
            ..Default::default()
        };
        let done = st.settle(&got, &files.store);
        assert_eq!(
            done,
            vec![
                FileAnswer {
                    name: "labcube.3ds".into(),
                    data: Some(b"3ds".to_vec())
                },
                FileAnswer {
                    name: "sky_front".into(),
                    data: Some(b"png".to_vec())
                },
                FileAnswer {
                    name: "missing.dts".into(),
                    data: None
                },
            ]
        );
        assert!(st.pending.is_empty());
    }

    /// A name the session already settled is answered at once.
    #[test]
    fn a_copy_already_kept_is_answered_at_once() {
        let asks = FileAsks::default();
        let mut files = session();
        let mut st = FileAskState::default();
        files.store.store("ground.3ds", b"g", None).unwrap();
        files.requests.done("ground.3ds");
        files.requests.failed("gone.dts");
        ask(&asks, &["ground.3ds", "gone.dts"]);
        let (send, now) = st.take(&asks, &mut files);
        assert!(send.is_empty());
        assert_eq!(
            now,
            vec![
                FileAnswer {
                    name: "ground.3ds".into(),
                    data: Some(b"g".to_vec())
                },
                FileAnswer {
                    name: "gone.dts".into(),
                    data: None
                },
            ]
        );
    }

    /// After a server warp, the names still on the wire are asked again
    /// before any new ones.
    #[test]
    fn a_new_connection_asks_again_for_what_was_pending() {
        let asks = FileAsks::default();
        let mut files = session();
        let mut st = FileAskState::default();
        ask(&asks, &["a.3ds"]);
        let _ = st.take(&asks, &mut files);
        ask(&asks, &["b.3ds"]);
        st.new_connection(&asks);
        files.new_connection();
        let (send, _) = st.take(&asks, &mut files);
        let sent: Vec<&str> = send.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(sent, ["a.3ds", "b.3ds"]);
    }
}
