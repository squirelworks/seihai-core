//! Networking.
//!
//! Connection and high-level send helpers. Packet formats are provisional
//! where noted; login and framing follow the cleaned 2.05/2.22 observations.

mod udp;
pub mod warp_retry;

pub use udp::{
    decode_udp, encode_udp, encode_udp_move, udp_key, PeerChannel, UDP_ID_BOMBADD, UDP_ID_CHAT,
    UDP_ID_ENTERLEVEL, UDP_ID_HIT, UDP_ID_ITEMDEL, UDP_ID_LEVELNAME, UDP_ID_MOVE, UDP_ID_SHOWIMG,
    UDP_ID_TRIGGER, UDP_PORT,
};

use crate::protocol::{
    bz2_compress, choose_comp, encode_pos_classic, encode_pos_precise, read_zlib_frame, write_frame, write_gstring,
    write_login, write_zlib_frame, write_zlib_frame_best, zlib_compress, ClientVersion, EncryptionGen, FrameLen, Gen3, Gen5, LoginShape, TamperSender,
    Gen5Comp, IdSet, LevelRequest, ListReply, LoginRequest, PlayerUpdate, SessionState,
};
use crate::offline::OfflineSession;
use crate::protocol::parse_list_reply;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// `PLI_ENTERLEVEL` (46), client to server. Only the 2.3 and 2.31 profiles
/// have it in their send set.
pub const PLI_ENTERLEVEL: u8 = 46;

/// `PLI_LEVELWARPMOD` (30), client to server: the level link a client
/// sends for a level it already holds.
pub const PLI_LEVELWARPMOD: u8 = 30;

/// The `PLI_ENTERLEVEL` message: the id and one `GCHAR 1`. The payload was
/// 1 in every one of the 7 level entries a real 2.31 client made in the lab
/// (login and 6 gmap crossings); no other value has been seen.
pub fn enter_level_message() -> Vec<u8> {
    vec![PLI_ENTERLEVEL + 0x20, 1 + 0x20, b'\n']
}

/// The 2.05 "walk" props message: X (15), Y (16) and SPRITE (17).
pub fn position_message(x: f32, y: f32, dir: u8) -> Vec<u8> {
    position_message_sprite(x, y, dir % 4)
}

/// `PLI_PLAYERPROPS` (2) with prop 10 in the pre-2.1 bow form, `c(10 + len)`
/// + `name`: how the real 2.00 and 2.05 tell others their animation
/// (`sword` as a swing starts, `idle` after it). See
/// [`crate::combat::swing_in_bow_slot`].
pub fn bow_slot_message(name: &str) -> Vec<u8> {
    let mut m = vec![2 + 0x20, 10 + 0x20, (10 + name.len() as u8).wrapping_add(0x20)];
    m.extend_from_slice(name.as_bytes());
    m.push(b'\n');
    m
}

/// The same with the whole SPRITE value (`dir + 4 * S`), for a sword swing
/// or hurt mode. `position_message` is the S = 0 case.
pub fn position_message_sprite(x: f32, y: f32, sprite: u8) -> Vec<u8> {
    vec![
        2 + 0x20,
        15 + 0x20,
        encode_pos_classic(x).wrapping_add(0x20),
        16 + 0x20,
        encode_pos_classic(y).wrapping_add(0x20),
        17 + 0x20,
        sprite.wrapping_add(0x20),
        b'\n',
    ]
}

/// The walk props of the Login-lobby builds (4.x to 6.x): SPRITE (17), then
/// the precise X2 (78) and Y2 (79) as GSHORTs, in ascending id order. The
/// real 4.04r2 and 4.20r8 (lab, 2026-10-31, `documentation/v4-profiles.md`)
/// and the 5.x and 6.x builds (Graal_analysis `protocol/v5v6-lab-play.md`)
/// send X2/Y2 every 50 ms while walking and never the classic X/Y (15, 16).
pub fn position_message_precise(x: f32, y: f32, sprite: u8) -> Vec<u8> {
    let mut m = vec![2 + 0x20, 17 + 0x20, sprite.wrapping_add(0x20), 78 + 0x20];
    m.extend_from_slice(&gshort(encode_pos_precise(x)));
    m.push(79 + 0x20);
    m.extend_from_slice(&gshort(encode_pos_precise(y)));
    m.push(b'\n');
    m
}

/// The walk props for `version`: SPRITE, X2 and Y2
/// ([`position_message_precise`]) when its profile uses the precise
/// positions, X, Y and SPRITE ([`position_message_sprite`]) otherwise. The
/// real 2.31 walks with X2/Y2 only (cloud lab, 2026-10-31,
/// `documentation/lab-231-walk.md`), as the 4.x to 6.x builds do.
pub fn walk_message_for(version: ClientVersion, x: f32, y: f32, sprite: u8) -> Vec<u8> {
    if version.profile().props.precise_pos {
        position_message_precise(x, y, sprite)
    } else {
        position_message_sprite(x, y, sprite)
    }
}

/// Whether `version` sends only `client.` flags to the server. A GS1
/// probe weapon (`setstring gr.probe,1`, `setstring client.probe,2`, `set
/// probeflag` on `playerenters`) got only `client.probe=2` from the real
/// 2.31 (cloud lab, 2026-10-31, `documentation/lab-231-walk.md`) and the real 4.04r2 (lab, 2026-10-31,
/// `documentation/v4-profiles.md`), and the real 2.31 sent no `gr.x`/`gr.y`
/// with the `-gr_movement` weapon. The real 2.22 sends every flag
/// (`documentation/gr-flags.md`). 2.3 has no binary and takes 2.31's
/// values; the 3.x builds were not probed and keep every flag.
pub fn sends_client_flags_only(version: ClientVersion) -> bool {
    matches!(version, ClientVersion::V230 | ClientVersion::V231) || version.profile().protocol.lobby.is_some()
}

/// Extra messages that go in the same frame right after 46 on a level
/// entry. A real 2.31 client sends 46 and then its props (the cell prop
/// and the position) in one frame when it started the move itself: lab,
/// 2026-10-24, every level link, cached or not, and every gmap crossing. After
/// a server-side warp the props follow only if the position changed, which
/// Seihai does not track, so 46 goes out alone then. The position is X2/Y2
/// for a precise-position build: the real 2.31 sent `ENTERLEVEL` and
/// `PLAYERPROPS X2=61 Y2=33` in its warp frame (lan-231c, 2026-10-31).
pub fn enter_level_extras(version: ClientVersion, own_link: bool, at: &PlayerUpdate) -> Vec<Vec<u8>> {
    if own_link {
        vec![walk_message_for(version, at.x, at.y, at.dir % 4)]
    } else {
        Vec::new()
    }
}

/// Decides when `PLI_ENTERLEVEL` (46) goes out for one logged-in session:
/// once at login and once per later level entry, never as a one-shot. A
/// profile without 46 in its send set never sends it.
///
/// A level entry is a new level name. The first level the server names
/// after login is not one: the login send covers it, as in the real
/// client, whose first frame after login carries 46.
///
/// Where 46 sits depends on who started the move (lab, real 2.31):
/// - a level link whose level the client has cached sends the link request
///   (`PLI_LEVELWARPMOD` 30) and 46 in one frame, before the server's
///   answer ([`link_request_frame`], [`Self::cached_link`]);
/// - a level link whose level is not cached sends 0, waits for the server's
///   level, then sends one frame that starts with 46 and goes on with the
///   props;
/// - a server-side warp: the first client frame after the server's answer
///   starts with 46.
///
/// So after the server's answer, [`Self::note_level`] says a 46 is owed and
/// [`Self::send_pending`] sends it once the player's position is known,
/// followed by `then` in the same frame.
#[derive(Debug, Clone)]
pub struct EnterLevelSender {
    version: ClientVersion,
    level: Option<String>,
    /// A 46 is owed for the level just entered.
    pending: bool,
    /// How many 46 messages were sent.
    pub sent: usize,
    /// The login 46 goes in the post-login frame ([`Self::owe_login`]).
    login_owed: bool,
}

impl EnterLevelSender {
    pub fn for_version(version: ClientVersion) -> Self {
        Self { version, level: None, pending: false, sent: 0, login_owed: false }
    }

    /// Send 46 for the login. Call once, right after the post-login frame.
    pub fn at_login(&mut self, conn: &mut Connection) -> io::Result<bool> {
        let sent = conn.send_enter_level(self.version, &[])?;
        self.sent += usize::from(sent);
        Ok(sent)
    }

    /// Hold the login 46 back: a real client sends it inside the frame that
    /// answers the server's login burst (right after the file checks), not
    /// on its own at the start. [`Self::take_login`] hands it over.
    pub fn owe_login(&mut self) {
        self.login_owed = true;
    }

    /// Whether the login 46 is still owed.
    pub fn login_owed(&self) -> bool {
        self.login_owed
    }

    /// The login 46, once, for the post-login frame. Empty when none is owed
    /// or the profile has no 46.
    pub fn take_login(&mut self) -> Vec<Vec<u8>> {
        if !std::mem::take(&mut self.login_owed) || !self.version.sends_enter_level() {
            return Vec::new();
        }
        self.sent += 1;
        vec![enter_level_message()]
    }

    /// The server put the player in level `name`. True when that is a new
    /// level and not the first one since login, so a 46 is now owed (and
    /// stays owed until [`Self::send_pending`]). No I/O.
    pub fn note_level(&mut self, name: &str) -> bool {
        let first = self.level.is_none();
        // Level names are not case-sensitive (a gmap grid may spell one
        // differently from the server's LEVELNAME).
        if self.level.as_deref().is_some_and(|l| l.eq_ignore_ascii_case(name)) {
            return false;
        }
        self.level = Some(name.to_string());
        if first {
            return false;
        }
        self.pending = true;
        true
    }

    /// Whether a 46 is owed.
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// Send the owed 46 with `then` after it in the same frame. Returns
    /// `Ok(false)` and sends nothing if none is owed or the profile has no
    /// 46. The debt is cleared either way.
    pub fn send_pending(&mut self, conn: &mut Connection, then: &[Vec<u8>]) -> io::Result<bool> {
        if !std::mem::take(&mut self.pending) {
            return Ok(false);
        }
        let sent = conn.send_enter_level(self.version, then)?;
        self.sent += usize::from(sent);
        Ok(sent)
    }

    /// The player crossed a gmap edge and the crossing frame
    /// ([`crate::gmap::crossing_frame`]) already carried the 46 when the
    /// profile sends one: the server's answer naming `name` owes none. Call
    /// when the frame went out; `sent` says whether it held a 46.
    pub fn crossing(&mut self, name: &str, sent: bool) {
        self.level = Some(name.to_string());
        self.pending = false;
        self.sent += usize::from(sent);
    }

    /// A cached link went out for `name` and its frame already carried the
    /// 46 when the profile sends one ([`link_request_frame`]): the server's
    /// answer naming `name` owes none. Call when the frame went out;
    /// `sent` is [`LinkFrame::with_enter_level`].
    pub fn cached_link(&mut self, name: &str, sent: bool) {
        self.crossing(name, sent);
    }

    /// [`Self::note_level`] and, when a 46 is owed, [`Self::send_pending`]
    /// with nothing after it. Sends the owed 46 at once and alone.
    pub fn level_entered(&mut self, conn: &mut Connection, name: &str) -> io::Result<bool> {
        if !self.note_level(name) {
            return Ok(false);
        }
        self.send_pending(conn, &[])
    }
}

/// `PLI_BOARDMODIFY` (1): `GCHAR x, y, w, h` in whole tiles, then the
/// `w*h` tiles row by row as GSHORTs. The 2.05 client sends one for each
/// change it makes to the level (a lift, a cut, a pushed or pulled stone
/// block); the server writes it into the level and sends it to the others
/// as `PLO_BOARDMODIFY`.
pub fn board_modify_message(x: u8, y: u8, w: u8, h: u8, tiles: &[u16]) -> Vec<u8> {
    let mut m = vec![1 + 0x20, x + 0x20, y + 0x20, w + 0x20, h + 0x20];
    for &t in tiles {
        m.extend_from_slice(&gshort(t));
    }
    m.push(b'\n');
    m
}

/// `PLI_TRIGGERACTION` (38) with no NPC: GINT 0 (three spaces), `GCHAR x*2,
/// y*2`, then `action` and, when there is one, `,` and `param`. This is
/// what the 2.05 client sends for its `pushed` / `pulled` tries, with the
/// direction as the parameter.
pub fn trigger_action_message(x: f32, y: f32, action: &str, param: &str) -> Vec<u8> {
    let half = |v: f32| ((v * 2.0) as i32).clamp(0, 223) as u8 + 0x20;
    let mut m = vec![38 + 0x20, 0x20, 0x20, 0x20, half(x), half(y)];
    m.extend_from_slice(&to_latin1(action));
    if !param.is_empty() {
        m.push(b',');
        m.extend_from_slice(&to_latin1(param));
    }
    m.push(b'\n');
    m
}

/// The `PLI_THROWCARRIED` (11) message: the id and the newline.
pub fn throw_carried_message() -> Vec<u8> {
    vec![11 + 0x20, b'\n']
}

/// `PLI_PLAYERPROPS` (2) with CARRYSPRITE (19): what the player holds over
/// the head, in the 2.05 code ([`crate::throwing::carry_wire_code`]; -1
/// empty hands). Other clients draw the object from it.
pub fn carry_sprite_message(sprite: Option<u16>) -> Vec<u8> {
    vec![2 + 0x20, 19 + 0x20, crate::throwing::carry_wire_byte(sprite), b'\n']
}

/// `PLI_PLAYERPROPS` (2) with MAXPOWER (1): heart containers. A real client
/// does not send it. GServer-v2 built without its NPC server takes it, limits
/// it to the `heartlimit` option (3 by default), fills the hearts to it and
/// sends CURPOWER back. Only for the `--dev-repair-hearts` developer option.
pub fn max_power_message(hearts: u8) -> Vec<u8> {
    vec![2 + 0x20, 1 + 0x20, hearts.min(20) + 0x20, b'\n']
}

/// `PLI_PLAYERPROPS` (2) with the player's own hearts report, in the
/// order the real client writes it: CURPOWER (2, half hearts), GANI (10, a
/// GSTRING), X (15) and Y (16) in half tiles, and SPRITE (17), each only
/// when given. The real 2.00's hit with its first knockback step, run 2:
/// `22 24 2A 24 "hurt" 2F 5F 31 A2`. The real 2.05 client's
/// hit report, lab run 1 (2026-10-29 note), was `22 25 2A 24 "hurt" 31 A0`:
/// 5 half hearts, `hurt`, sprite 128 (hurt sprite 32, facing up).
pub fn power_props_message(cur_half: Option<u8>, gani: Option<&str>, pos: (Option<u8>, Option<u8>), sprite: Option<u8>) -> Vec<u8> {
    let mut m = vec![2 + 0x20];
    if let Some(c) = cur_half {
        m.push(2 + 0x20);
        m.push(c.min(223) + 0x20);
    }
    if let Some(g) = gani {
        m.push(10 + 0x20);
        push_gstring_latin1(&mut m, g);
    }
    for (id, v) in [(15u8, pos.0), (16, pos.1)] {
        if let Some(v) = v {
            m.push(id + 0x20);
            m.push(v.wrapping_add(0x20));
        }
    }
    if let Some(s) = sprite {
        m.push(17 + 0x20);
        m.push(s.wrapping_add(0x20));
    }
    m.push(b'\n');
    m
}

/// `PLI_PLAYERPROPS` (2) with ARROWSCOUNT (4) and BOMBSCOUNT (5), each
/// only when given. The real 2.05 client sends ARROWSCOUNT after its
/// ARROWADD and BOMBSCOUNT with its BOMBADD (lab, 2026-10-30: 19 after a
/// shot, 14 after a bomb).
pub fn count_props_message(arrows: Option<u8>, bombs: Option<u8>) -> Vec<u8> {
    let mut m = vec![2 + 0x20];
    if let Some(n) = arrows {
        m.extend([4 + 0x20, n.min(99) + 0x20]);
    }
    if let Some(n) = bombs {
        m.extend([5 + 0x20, n.min(99) + 0x20]);
    }
    m.push(b'\n');
    m
}

/// `PLI_LEVELWARP` (0): `GCHAR x*2`, `GCHAR y*2` (tiles), level name.
pub fn levelwarp_message(x: f32, y: f32, level: &str) -> Vec<u8> {
    let mut msg = vec![0x20];
    msg.push(((x * 2.0).clamp(0.0, 127.0) as u8) + 0x20);
    msg.push(((y * 2.0).clamp(0.0, 127.0) as u8) + 0x20);
    msg.extend_from_slice(&to_latin1(level));
    msg.push(b'\n');
    msg
}

/// `PLI_LEVELWARPMOD` (30): `GINT5` mod time of the cached copy, then
/// `GCHAR x*2`, `GCHAR y*2` (tiles) and the level name. The same as
/// [`levelwarp_message`] with the mod time in front (GServer-v2
/// `msgPLI_LEVELWARP` reads the five bytes first when the id is 30).
pub fn levelwarpmod_message(mod_time: u32, x: f32, y: f32, level: &str) -> Vec<u8> {
    let mut msg = vec![PLI_LEVELWARPMOD + 0x20];
    for shift in [28, 21, 14, 7, 0] {
        msg.push(((mod_time >> shift) as u8 & 0x7F) + 0x20);
    }
    msg.push(((x * 2.0).clamp(0.0, 127.0) as u8) + 0x20);
    msg.push(((y * 2.0).clamp(0.0, 127.0) as u8) + 0x20);
    msg.extend_from_slice(&to_latin1(level));
    msg.push(b'\n');
    msg
}

/// `PLI_WANTFILE` (23): the file name, nothing else.
/// `PLI_VERIFYWANTSEND` (47): the GUINT5 CRC-32 of the client's copy, then
/// the name. The server sends the file only when its copy differs. The real
/// 4.04r2 asks a cached `basepackage.gupd` this way (lab, 2026-10-31), as
/// 5.00r7 does; with no copy it sends `PLI_WANTFILE` (23).
pub fn verify_want_message(crc: u32, name: &str) -> Vec<u8> {
    let mut msg = vec![47 + 0x20];
    for shift in [28, 21, 14, 7, 0] {
        msg.push(((crc >> shift) & 0x7F) as u8 + 0x20);
    }
    msg.extend_from_slice(&to_latin1(name));
    msg.push(b'\n');
    msg
}

pub fn wantfile_message(name: &str) -> Vec<u8> {
    let mut msg = vec![23 + 0x20];
    msg.extend_from_slice(&to_latin1(name));
    msg.push(b'\n');
    msg
}

/// `PLI_UPDATEFILE` (34): `GINT5` last-write time (Unix seconds) of the
/// local copy, then the file name. Real 2.17, 2.22 and 2.31 clients, lab
/// 2026-10-24: the time is the file's own mod time, and the server answers
/// `PLO_FILEUPTODATE` (45) when it is the same.
pub fn updatefile_message(mod_time: u32, name: &str) -> Vec<u8> {
    let mut msg = vec![34 + 0x20];
    for shift in [28, 21, 14, 7, 0] {
        msg.push(((mod_time >> shift) as u8 & 0x7F) + 0x20);
    }
    msg.extend_from_slice(&to_latin1(name));
    msg.push(b'\n');
    msg
}

/// What went into a link-request frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkFrame {
    pub messages: Vec<Vec<u8>>,
    /// The tamper check (39) is in the frame.
    pub with_tamper: bool,
    /// `PLI_ENTERLEVEL` (46) is in the frame: the cached form on a build
    /// that sends 46.
    pub with_enter_level: bool,
}

/// The frame a client sends when the player walks into a level link.
///
/// `cached` is the mod time of the copy the client holds (see
/// [`SessionState::cached_mod_time`]) and is honoured only for a profile
/// whose levels have `cached_links`; otherwise the request is the plain
/// `PLI_LEVELWARP` (0) and nothing else but the tamper check.
///
/// Lab, real clients, 2026-10-24:
/// - not cached (0): the frame is `0` and, on 2.22, `39`. The 46 of a
///   2.31 client comes later, after the server's answer
///   ([`EnterLevelSender`]).
/// - cached (30): 2.31 sends `30`, `46`, then its props, in one frame,
///   before the server answers; 2.22 sends `30`, `39`. So with a cached
///   copy the 46 and the props (at the destination `x`, `y`) go in this
///   frame, for profiles with 46 only. If a profile ever had both the
///   tamper check and 46 the order here would be 30, 39, 46, props; no
///   build seen does.
pub fn link_request_frame(
    version: ClientVersion,
    cached: Option<u32>,
    x: f32,
    y: f32,
    dir: u8,
    level: &str,
    tamper: Option<&mut TamperSender>,
) -> LinkFrame {
    let cached = cached.filter(|_| version.sends_cached_links());
    let mut messages = vec![match cached {
        Some(t) => levelwarpmod_message(t, x, y, level),
        None => levelwarp_message(x, y, level),
    }];
    let mut with_tamper = false;
    if let Some(t) = tamper {
        if t.level_changed() {
            if let Some(m) = t.message() {
                messages.push(m);
                with_tamper = true;
            }
        }
    }
    let with_enter_level = cached.is_some() && version.sends_enter_level();
    if with_enter_level {
        messages.push(enter_level_message());
        messages.push(walk_message_for(version, x, y, dir % 4));
    }
    LinkFrame { messages, with_tamper, with_enter_level }
}

/// What [`start_online_link`] sent and what the caller still owes.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkStart {
    /// The frame that went out.
    pub frame: LinkFrame,
    /// Where the player lands: the link's arrival, as requested. A
    /// `PLI_LEVELWARPMOD` gets no `PLO_PLAYERWARP`, and neither does a
    /// `PLI_LEVELWARP` to a plain level on GServer-v2 (it sends the level
    /// alone), so the client puts the player here when the level is named.
    /// The server's own position, if it sends one, wins.
    pub arrival: Option<(f32, f32)>,
    /// The mod time of the copy a cached request named.
    pub cached_mod_time: Option<u32>,
}

/// The arrival of a link the client asked for, held until the server names
/// the destination. It is kept by the destination's name, not by the warp:
/// a level the server has to load or send in full can arrive after the
/// pending warp was given up (the player may walk again by then), and the
/// server still put the player at the requested position. A different level
/// named first (the server warped the player elsewhere) drops it.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkArrival {
    pub dest: String,
    pub x: f32,
    pub y: f32,
}

impl LinkArrival {
    pub fn new(dest: &str, (x, y): (f32, f32)) -> Self {
        Self { dest: dest.to_string(), x, y }
    }

    /// The level `entered` is now the current one: the held position if it
    /// is the link's destination. Either way nothing stays held.
    pub fn take_for(held: &mut Option<Self>, entered: &str) -> Option<(f32, f32)> {
        let a = held.take()?;
        same_level_name(&a.dest, entered).then_some((a.x, a.y))
    }
}

/// Level names as the server and the links write them: no case, and a
/// name without an extension is the `.nw` one.
fn same_level_name(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        let s = s.trim().to_ascii_lowercase();
        if s.contains('.') {
            s
        } else {
            format!("{s}.nw")
        }
    };
    norm(a) == norm(b)
}

/// Ask the server for a level change the client started: a level link the
/// player walked into, or a level script's `setlevel` / `setlevel2`. Both
/// use the same request (see [`link_request_frame`]); the server's answer
/// is handled alike.
///
/// Does nothing and returns `Ok(None)` while another warp is pending, so a
/// script that fires again before the server has answered cannot stack a
/// second request. Otherwise the session is put in the warp-pending state
/// (links and walking stop until [`OfflineSession::arrive`] or
/// [`OfflineSession::cancel_warp`]), the frame goes out through `send`, and
/// a cached request is noted in `net_state`
/// ([`SessionState::expect_cached_level`]) and in `enter_level`
/// ([`EnterLevelSender::cached_link`]).
///
/// If `send` fails the warp is cancelled again and the error is returned,
/// so the caller decides what to do with the connection.
#[allow(clippy::too_many_arguments)]
pub fn start_online_link(
    send: impl FnOnce(&[Vec<u8>]) -> io::Result<()>,
    version: ClientVersion,
    session: &mut OfflineSession,
    net_state: &mut SessionState,
    enter_level: Option<&mut EnterLevelSender>,
    tamper: Option<&mut TamperSender>,
    dest: &str,
    x: f32,
    y: f32,
) -> io::Result<Option<LinkStart>> {
    if session.warp_pending.is_some() {
        return Ok(None);
    }
    session.begin_warp(dest);
    let cached = if version.sends_cached_links() { net_state.cached_mod_time(dest) } else { None };
    let dir = session.player_update().dir;
    let frame = link_request_frame(version, cached, x, y, dir, dest, tamper);
    if let Err(e) = send(&frame.messages) {
        session.cancel_warp();
        return Err(e);
    }
    if cached.is_some() {
        net_state.expect_cached_level(dest);
        if let Some(el) = enter_level {
            el.cached_link(dest, frame.with_enter_level);
        }
    }
    Ok(Some(LinkStart { frame, arrival: Some((x, y)), cached_mod_time: cached }))
}

/// A game-server session as [`login_gserver`] sets it up.
pub struct GserverSession {
    pub conn: Connection,
    /// `Some` with `send_tampercheck` on and a build constant.
    pub tamper: Option<TamperSender>,
    /// 46 at login was sent; later level entries go through this.
    pub enter_level: EnterLevelSender,
    /// `send_tampercheck` was on but the build has no constant to send.
    pub tamper_unavailable: bool,
    /// Post-login sends that failed. The login itself worked.
    pub warnings: Vec<String>,
}

/// Why [`login_gserver`] failed.
#[derive(Debug)]
pub struct LoginFailure {
    /// True when the TCP connection failed, false when the login send did.
    pub connect: bool,
    pub error: io::Error,
}

impl std::fmt::Display for LoginFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} failed: {}", if self.connect { "connect" } else { "login" }, self.error)
    }
}

/// Connect to a gserver at `addr` and log in with `key`: the login frame,
/// the post-login frame (language and, when asked, the tamper check) and
/// the login 46. The same steps serve the first login and the reconnect
/// that follows a `PLO_SERVERWARP` (178), which in a real 2.31 client is a
/// complete fresh login (new random key, same account and password, no
/// listserver step). A warp login turned away because the account is still
/// in use is retried by the caller ([`warp_retry`]). The reader is not
/// started: call [`Connection::spawn_reader`] on the result.
pub fn login_gserver(
    addr: &str,
    version: ClientVersion,
    key: u8,
    account: &str,
    password: &str,
    send_tampercheck: bool,
) -> Result<GserverSession, LoginFailure> {
    login_gserver_with(addr, version, key, account, password, send_tampercheck, true, |n| wantfile_message(n))
}

/// As [`login_gserver`]. `check` gives the file message for each of the
/// profile's first-frame files ([`crate::protocol::login::LoginInfo::first_frame_files`]):
/// `PLI_UPDATEFILE` for a file the client has, `PLI_WANTFILE` otherwise.
/// It gives the same for a 4.x build's file requests after the language
/// ([`crate::protocol::lobby::LobbyLogin`]); `first_session` is true for
/// the first login of the run (a lobby build's Login server), which alone
/// asks the first-session files (`bigmap.txt`).
#[allow(clippy::too_many_arguments)]
pub fn login_gserver_with(
    addr: &str,
    version: ClientVersion,
    key: u8,
    account: &str,
    password: &str,
    send_tampercheck: bool,
    first_session: bool,
    check: impl Fn(&str) -> Vec<u8>,
) -> Result<GserverSession, LoginFailure> {
    let mut conn =
        Connection::connect(addr).map_err(|error| LoginFailure { connect: true, error })?;
    let login = version.profile().protocol.login;
    let first_files = login.map_or(&[][..], |l| l.first_frame_files);
    let bundle = login.is_some_and(|l| l.bundle_login) || !first_files.is_empty();
    let mut warnings = Vec::new();
    let tamper = if send_tampercheck { TamperSender::for_version(version) } else { None };
    let tamper_unavailable = send_tampercheck && tamper.is_none();
    let after_login = version.profile().protocol.lobby.and_then(|l| l.after_login);
    if !bundle {
        conn.login_versioned(version, key, account, password)
            .map_err(|error| LoginFailure { connect: false, error })?;
        let mut msgs = conn.post_login_messages(version, "English", tamper.as_ref());
        // The real 4.04r2 and 4.20r8 ask their base files in the frame of
        // the language (lab, 2026-10-31).
        if let Some(a) = after_login {
            let first = if first_session { a.first_session_files } else { &[] };
            msgs.extend(a.after_language.iter().chain(first).map(|n| check(n)));
        }
        if !msgs.is_empty() {
            if let Err(e) = conn.send_messages(&msgs) {
                warnings.push(format!("post-login send failed: {e}"));
            }
        }
    } else {
        // The real 2.17.1: login line, language and tamper check in one frame.
        // The real 3.04: login line, language and the file checks in one plain frame.
        let mut then = conn.post_login_messages(version, "English", tamper.as_ref());
        then.extend(first_files.iter().map(|n| check(n)));
        conn.login_versioned_with(version, key, account, password, &then)
            .map_err(|error| LoginFailure { connect: false, error })?;
    }
    let mut enter_level = EnterLevelSender::for_version(version);
    if version.profile().protocol.login.is_some() || after_login.is_some() {
        // The post-login frame carries it (see `protocol::login`; for 4.x,
        // `protocol::lobby::LobbyBurst`).
        enter_level.owe_login();
    } else if let Err(e) = enter_level.at_login(&mut conn) {
        warnings.push(format!("enter-level send failed: {e}"));
    }
    Ok(GserverSession { conn, tamper, enter_level, tamper_unavailable, warnings })
}

/// The socket's own thread ([`Connection::spawn_reader_timed`]): write what
/// the game loop queued, read what has arrived, and hand on each whole frame
/// with the time it came in. Ends after the first error or when the game
/// loop drops its receiver; then the queue's sender sees the end too.
fn socket_thread(
    mut stream: TcpStream,
    mut cipher: Option<Gen5>,
    frame_len: FrameLen,
    out: Receiver<Vec<u8>>,
    frames: mpsc::Sender<(Instant, io::Result<Vec<u8>>)>,
) {
    let mut pending: Vec<u8> = Vec::new();
    let mut inbuf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    let mut queue_open = true;
    let fail = |frames: &mpsc::Sender<(Instant, io::Result<Vec<u8>>)>, e: io::Error| {
        let _ = frames.send((Instant::now(), Err(e)));
    };
    loop {
        let mut busy = false;
        while queue_open {
            match out.try_recv() {
                Ok(data) => pending.extend(data),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => queue_open = false,
            }
        }
        // The Connection was dropped (its sender with it) and everything it
        // queued is out: close the socket and end. Before, the thread kept
        // reading a link nobody used, and the server kept the player online
        // until its own timeout (stress test, 2026-10-31).
        // The game loop hears of it as a closed connection, if it still
        // listens.
        if !queue_open && pending.is_empty() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
            return fail(&frames, io::Error::new(io::ErrorKind::NotConnected, "closed on Seihai's side"));
        }
        while !pending.is_empty() {
            match stream.write(&pending) {
                Ok(0) => return fail(&frames, io::ErrorKind::WriteZero.into()),
                Ok(n) => {
                    pending.drain(..n);
                    busy = true;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return fail(&frames, e),
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) => return fail(&frames, io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                inbuf.extend_from_slice(&chunk[..n]);
                busy = true;
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => {}
            Err(e) => return fail(&frames, e),
        }
        let at = Instant::now();
        // Every whole frame: a u16 length (6.x: or `FF FF` and a u32),
        // then that many bytes.
        while let Some((head, len)) = frame_len.header(&inbuf) {
            if len > FrameLen::MAX_LONG {
                let e = io::Error::new(io::ErrorKind::InvalidData, format!("gen-5 frame length {len} is over {}", FrameLen::MAX_LONG));
                return fail(&frames, e);
            }
            if inbuf.len() < head + len {
                break;
            }
            let mut raw: &[u8] = &inbuf[..head + len];
            let frame = match cipher.as_mut() {
                Some(g) => crate::protocol::read_frame_len(&mut raw, g, frame_len),
                None => read_zlib_frame(&mut raw),
            };
            inbuf.drain(..head + len);
            let stop = frame.is_err();
            if frames.send((at, frame)).is_err() || stop {
                return;
            }
        }
        if !busy {
            std::thread::park_timeout(IO_IDLE);
        }
    }
}

/// How long one address gets to accept a connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// After the first server list arrives, how long to wait for more frames:
/// the status text, site URL and upgrade URL that some listservers send in
/// a later frame, and (on the public 14923 port) a second id 0 list. On the
/// public service the second list and the status came about 350 ms after
/// the first list, so 500 ms left little margin on a slower link.
const LIST_TAIL_WAIT: Duration = Duration::from_millis(1500);

/// A read that timed out before any reply came in is reported as what it
/// is. The raw error on Linux is "Resource temporarily unavailable (os
/// error 11)", which reads like a system fault. The public listserver on
/// 14922 stays silent (and keeps the connection open) when it gets a classic
/// plain-frame login.
fn list_read_error(e: io::Error) -> io::Error {
    match e.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => io::Error::new(
            io::ErrorKind::TimedOut,
            "the listserver sent no reply within 5 s (it accepted the connection but stayed silent)",
        ),
        _ => e,
    }
}

/// The text for a failed list fetch. A refused connection on port 14921
/// gets a pointer to the port that answered: the public listserver
/// refused 14921 on 2026-10-25 and answered a classic (2.00-style) login on
/// 14923.
pub fn list_failure_text(addr: &str, e: &io::Error) -> String {
    let mut s = format!("listserver failed: {e}");
    if e.kind() == io::ErrorKind::ConnectionRefused && addr.ends_with(":14921") {
        s.push_str(" (the public listserver refuses port 14921; it answered a classic login on 14923)");
    }
    s
}

/// Try each address in order and return the first stream that connects.
/// Fails with the last error, or `NotFound` if `addrs` is empty.
fn connect_any(addrs: &[SocketAddr], timeout: Duration) -> io::Result<TcpStream> {
    let mut last = None;
    for a in addrs {
        match TcpStream::connect_timeout(a, timeout) {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "host name resolved to no addresses")
    }))
}

/// Check that `m` is one whole client→server message: an id byte (0x20 or
/// more), then a payload with no line feed, then one closing `\n`. The
/// servers split a frame body into messages at each `\n` (GServer-v2
/// `readString("\n")`), so a line feed inside a payload, or a missing one
/// at the end, turns one message into two or two into one. On gen 3 each
/// message carries its own insertion step, so that shifts the server's
/// iterator for the rest of the session.
pub fn check_wire_message(m: &[u8]) -> Result<(), &'static str> {
    match m {
        [] => Err("empty"),
        [id, ..] if *id < 0x20 => Err("id byte below 0x20"),
        [.., last] if *last != b'\n' => Err("no closing line feed"),
        [_] => Err("no id"),
        _ if m[..m.len() - 1].contains(&b'\n') => Err("line feed inside the payload"),
        _ => Ok(()),
    }
}

/// `m` for a log line: the id, the length and the bytes, printable ones as
/// text and the rest as `\xNN`.
pub fn describe_message(m: &[u8]) -> String {
    let id = m.first().map_or(-1, |b| i32::from(*b) - 0x20);
    let mut s = format!("id {id} len {} ", m.len());
    for &b in m.iter().skip(1).take(160) {
        if (0x20..0x7f).contains(&b) && b != b'\\' {
            s.push(b as char);
        } else {
            s.push_str(&format!("\\x{b:02x}"));
        }
    }
    if m.len() > 161 {
        s.push_str("...");
    }
    s
}

/// TCP connection to a listserver or gserver.
pub struct Connection {
    stream: TcpStream,
    /// Gen-3 insertion state for client→server messages, set by a
    /// versioned 2.05-style login.
    gen3: Option<Gen3>,
    /// Gen-5 XOR state for client→server frames, set by a 2.22 login.
    gen5: Option<Gen5>,
    /// Gen-5 XOR state for server→client frames. Each direction has its
    /// own iterator, both seeded at login.
    gen5_in: Option<Gen5>,
    /// How server frames give their length: 6.x builds read a long length.
    frame_len: FrameLen,
    /// Client→server ids the logged-in version can send, set by
    /// [`Self::login_versioned`]. Others are refused.
    pli: Option<IdSet>,
    /// Plain zlib frames go out at the best level (`78 DA`): set by a gen-2
    /// login. The real 1.28 beta 2 to 1.39 clients compressed every frame
    /// that way (lab, 2026-10-30).
    zlib_best: bool,
    /// Flag messages a script queued ([`Self::queue_flag`]): they ride
    /// ahead of the next walk update, or go out on their own.
    flags_out: Vec<Vec<u8>>,
    /// When the oldest queued flag was queued.
    flags_since: Option<Instant>,
    /// File requests that ride in the frame of the next walk update
    /// ([`Self::queue_files`]).
    files_out: Vec<Vec<u8>>,
    /// When the oldest queued file request was queued.
    files_since: Option<Instant>,
    /// The last walk update sent ([`Self::send_walk_sprite`]): the same
    /// X, Y and SPRITE again is not re-sent (the real client sends props
    /// only when they change).
    last_walk: Option<Vec<u8>>,
    /// Walk updates carry X2/Y2 ([`position_message_precise`]) instead of
    /// X/Y: set at login for the builds with precise positions (2.3, 2.31,
    /// 3.153 and the Login-lobby builds; see [`walk_message_for`]).
    precise_walk: bool,
    /// Only `client.` flags go to the server: set at login for the
    /// Login-lobby builds and 2.3/2.31 ([`sends_client_flags_only`], [`Self::queue_flag`]).
    client_flags_only: bool,
    /// Where each message sent is written, before its cipher
    /// ([`Self::set_send_log`]); the debug `packets.log`.
    send_log: Option<Box<dyn std::io::Write + Send>>,
    /// Set once the reader runs ([`Self::spawn_reader`]): the reader's
    /// thread owns the socket from then on, and frames to send go to it.
    io_thread: Option<IoThread>,
}

/// The frames handed to the socket's own thread, and the thread to wake.
struct IoThread {
    out: mpsc::Sender<Vec<u8>>,
    thread: std::thread::Thread,
}

/// How long the socket thread sleeps when there was nothing to read or
/// write. A frame to send wakes it at once; a frame from the server waits
/// at most this long.
const IO_IDLE: Duration = Duration::from_millis(2);

impl Connection {
    /// Connect to `host:port`.
    ///
    /// The name is resolved with the system resolver and every address it
    /// returns is tried in turn, each with [`CONNECT_TIMEOUT`], so a dead
    /// first address (an unreachable IPv6 one, say) does not hide a
    /// working second one. The last error is returned if none connects.
    pub fn connect(addr: &str) -> io::Result<Self> {
        let addrs: Vec<SocketAddr> = addr.to_socket_addrs()?.collect();
        let stream = connect_any(&addrs, CONNECT_TIMEOUT)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        Ok(Self { stream, gen3: None, gen5: None, gen5_in: None, frame_len: FrameLen::Short, pli: None, zlib_best: false, flags_out: Vec::new(), flags_since: None, files_out: Vec::new(), files_since: None, last_walk: None, precise_walk: false, client_flags_only: false, send_log: None, io_thread: None })
    }

    /// Close the connection in both directions. A reader thread from
    /// [`Self::spawn_reader`] then ends with an error.
    pub fn shutdown(&self) -> io::Result<()> {
        self.stream.shutdown(std::net::Shutdown::Both)
    }

    /// True if this connection's client version has message `id` in its
    /// send set (always true before the login fixed the version).
    pub fn can_send(&self, id: u8) -> bool {
        self.pli.as_ref().map_or(true, |p| p.contains(id))
    }

    /// Send a raw byte buffer.
    ///
    /// Once the reader runs, the bytes go to its thread, which writes them
    /// between its reads; an error comes back only when that thread has
    /// ended (the connection is gone).
    pub fn send_raw(&mut self, data: &[u8]) -> io::Result<()> {
        match &self.io_thread {
            Some(t) => {
                t.out.send(data.to_vec()).map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "connection closed"))?;
                t.thread.unpark();
                Ok(())
            }
            None => self.stream.write_all(data),
        }
    }

    /// Write every message sent from now on to `log`, one line each, as
    /// [`describe_message`] gives it (before the gen-3 byte or gen-5 XOR),
    /// with the reason when a malformed one is dropped.
    pub fn set_send_log(&mut self, log: Box<dyn std::io::Write + Send>) {
        self.send_log = Some(log);
    }

    fn log_send(&mut self, m: &[u8], dropped: Option<&str>) {
        if let Some(f) = self.send_log.as_mut() {
            let _ = match dropped {
                Some(why) => writeln!(f, "send DROPPED ({why}) {}", describe_message(m)),
                None => writeln!(f, "send {}", describe_message(m)),
            };
            let _ = f.flush();
        }
    }

    /// Send a version-aware game-server login.
    ///
    /// The login frame is plain zlib for every generation: no gen-3 byte,
    /// no gen-5 compression byte or XOR, since the server needs the key
    /// inside it. Every later client→server frame is ciphered. For gen 5
    /// the iterator is still at the seed for the first frame after the
    /// login (lab 2.22 capture). The `GNW30123` id 7 handshake belongs to
    /// the listserver only; see [`Self::login_listserver`].
    pub fn login_versioned(
        &mut self,
        version: ClientVersion,
        key: u8,
        account: &str,
        password: &str,
    ) -> io::Result<()> {
        self.login_versioned_with(version, key, account, password, &[])
    }

    /// As [`Self::login_versioned`], with `then` in the same frame after the
    /// login line (the real 3.04 sends its language and two file checks
    /// there; the real 2.17.1 its language and tamper check). On a gen-3 link
    /// each of `then` gets its insertion, from the first iterator step: the
    /// real 2.17.1 frame decodes that way (lab, 2026-10-30).
    pub fn login_versioned_with(
        &mut self,
        version: ClientVersion,
        key: u8,
        account: &str,
        password: &str,
        then: &[Vec<u8>],
    ) -> io::Result<()> {
        let mut line = Vec::new();
        write_login(&mut line, version, key, account, password)?;
        line.push(b'\n');
        let mut gen3 = (version.encryption() == EncryptionGen::Gen3).then(|| Gen3::new(key));
        for m in then {
            if let Err(why) = check_wire_message(m) {
                self.log_send(m, Some(why));
                eprintln!("not sent: malformed message ({why}): {}", describe_message(m));
                continue;
            }
            self.log_send(m, None);
            match gen3.as_mut() {
                Some(g) => line.extend(g.encode(m)),
                None => line.extend_from_slice(m),
            }
        }
        self.zlib_best = version.encryption() == EncryptionGen::Gen2;
        let frame = if self.zlib_best { write_zlib_frame_best(&line)? } else { write_zlib_frame(&line)? };
        self.send_raw(&frame)?;
        self.pli = Some(version.profile().protocol.pli);
        self.precise_walk = version.profile().props.precise_pos;
        self.client_flags_only = sends_client_flags_only(version);
        self.take_flags();
        match version.encryption() {
            EncryptionGen::Gen3 => self.gen3 = gen3,
            EncryptionGen::Gen5 => {
                let g = if version.profile().protocol.gen5_round_down { Gen5::new(key).round_down() } else { Gen5::new(key) };
                self.frame_len = if version.profile().protocol.gen5_long_frames { FrameLen::Long } else { FrameLen::Short };
                self.gen5 = Some(g.clone());
                self.gen5_in = Some(g);
            }
            _ => {}
        }
        self.stream.flush()
    }

    /// Log in to a listserver and ask for the server list. The shape is the
    /// profile's [`crate::protocol::LoginShape`]:
    ///
    /// - C (2.22, 2.3, 2.31): a plain zlib frame with id 7 = `GCHAR key` +
    ///   `GNW30123` + `newmain`, then a gen-5 frame with id 1 = account and
    ///   password. Replies are gen-5 frames keyed with the same key.
    /// - D (3.04): C's two messages in one plain zlib frame, at the default
    ///   level (`78 9C`) as the real 3.04 sent it. Replies as for C.
    /// - S (the real 1.41 rev 1): nothing; the listserver sends the list
    ///   first. [`Self::fetch_server_list`] sends shape B when nothing comes
    ///   within [`crate::protocol::SILENT_LIST_WAIT`].
    /// - B (2.17 and the builds with no capture from 2.1 to 2.16):
    ///   one plain zlib frame with id 0 `newmain` and id 1. There is no
    ///   cipher on this link.
    /// - A (the real 2.00 and 2.05, and 1.42 and 2.01 to 2.04 as a best
    ///   guess): one plain zlib frame of id 0 + `newmain` + LF and nothing
    ///   else: no account, password, token or key (lab captures). The reply
    ///   is read as for the others; no public run has used this frame yet.
    ///
    /// The plain frames are compressed at the best zlib level (`78 DA`),
    /// like the real clients'.
    pub fn login_listserver(
        &mut self,
        version: ClientVersion,
        key: u8,
        account: &str,
        password: &str,
    ) -> io::Result<()> {
        let shape = version.listserver().shape;
        if shape == LoginShape::Silent {
            // The real 1.41 sends nothing; the listserver speaks first.
            return Ok(());
        }
        if shape == LoginShape::NewmainOnly {
            self.send_raw(&write_zlib_frame_best(b" newmain\n")?)?;
            return self.stream.flush();
        }
        let mut serverlist = vec![1 + 0x20];
        write_gstring(&mut serverlist, account)?;
        write_gstring(&mut serverlist, password)?;
        serverlist.push(b'\n');
        if matches!(shape, LoginShape::Gen5 | LoginShape::Gen5OneFrame) {
            let mut hs = vec![7 + 0x20, (key & 0x7F) + 0x20];
            hs.extend_from_slice(version.listserver_token().as_bytes());
            hs.extend_from_slice(b"newmain\n");
            self.gen5 = Some(Gen5::new(key));
            self.gen5_in = Some(Gen5::new(key));
            if shape == LoginShape::Gen5OneFrame {
                hs.extend_from_slice(&serverlist);
                self.send_raw(&write_zlib_frame(&hs)?)?;
                return self.stream.flush();
            }
            self.send_raw(&write_zlib_frame_best(&hs)?)?;
            self.send_messages(&[serverlist])
        } else {
            self.send_classic_list_login(account, password)
        }
    }

    /// Shape B: one plain zlib frame (`78 DA`) with id 0 `newmain` and id 1
    /// (account, password).
    fn send_classic_list_login(&mut self, account: &str, password: &str) -> io::Result<()> {
        let mut body = b" newmain\n".to_vec();
        body.push(1 + 0x20);
        write_gstring(&mut body, account)?;
        write_gstring(&mut body, password)?;
        body.push(b'\n');
        self.send_raw(&write_zlib_frame_best(&body)?)?;
        self.stream.flush()
    }

    /// Log in to a listserver and collect its reply.
    ///
    /// Reads frames until an error text arrives, the server closes, or
    /// five frames have gone by. Real listservers send the server list in
    /// one frame and the status text, site URL and upgrade URL in a
    /// second. Once servers are in, this keeps reading for up to
    /// [`LIST_TAIL_WAIT`] in total, stops as soon as the status is in, and
    /// treats a timeout or any read error at that point as the end of the
    /// reply, so an open connection never makes it hang.
    /// 1.33–1.39 have no listserver, so they fetch the list as 2.05.
    pub fn fetch_server_list(
        addr: &str,
        version: ClientVersion,
        account: &str,
        password: &str,
    ) -> io::Result<ListReply> {
        let version = version.list_version();
        let mut conn = Self::connect(addr)?;
        conn.login_listserver(version, random_key(), account, password)?;
        // Shape S (1.41): wait for the listserver to speak; if it does not,
        // log in with shape B, which a listserver that waits for a request
        // answers. The real 1.41 has no such fallback.
        let mut first: Option<Vec<u8>> = None;
        if version.listserver().shape == LoginShape::Silent {
            conn.stream.set_read_timeout(Some(crate::protocol::SILENT_LIST_WAIT))?;
            match conn.read_list_body() {
                Ok(b) => first = Some(b),
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                    eprintln!("listserver: nothing within {:?} of a silent 1.41 login; sending the classic login", crate::protocol::SILENT_LIST_WAIT);
                    conn.send_classic_list_login(account, password)?;
                }
                Err(e) => return Err(list_read_error(e)),
            }
            conn.stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        }
        let mut reply = ListReply::default();
        let mut tail_deadline: Option<Instant> = None;
        for _ in 0..5 {
            if let Some(deadline) = tail_deadline {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                let _ = conn.stream.set_read_timeout(Some(left));
            }
            let read = match first.take() {
                Some(b) => Ok(b),
                None => conn.read_list_body(),
            };
            let body = match read {
                Ok(b) => b,
                // Past the server list (or the status), a timeout, close or
                // bad frame just ends the reply.
                Err(_) if reply.status.is_some() || !reply.servers.is_empty() => break,
                Err(e) => return Err(list_read_error(e)),
            };
            let part = parse_list_reply(&body);
            // Each id 0 list is the whole list. The public 14923 port sends
            // an old one and then the current one; showing both listed most
            // servers twice. A later list replaces the earlier one.
            if !part.servers.is_empty() {
                reply.servers = part.servers;
            }
            reply.status = part.status.or(reply.status);
            reply.site_url = part.site_url.or(reply.site_url);
            reply.error = part.error.or(reply.error);
            reply.upgrade_url = part.upgrade_url.or(reply.upgrade_url);
            if reply.error.is_some() || (!reply.servers.is_empty() && reply.status.is_some()) {
                break;
            }
            if !reply.servers.is_empty() && tail_deadline.is_none() {
                tail_deadline = Some(Instant::now() + LIST_TAIL_WAIT);
            }
        }
        Ok(reply)
    }

    /// Read one server frame and return its decompressed body: gen-5
    /// framing after a gen-5 login, plain zlib otherwise.
    pub fn read_body(&mut self) -> io::Result<Vec<u8>> {
        match self.gen5_in.as_mut() {
            Some(g) => crate::protocol::read_frame_len(&mut self.stream, g, self.frame_len),
            None => read_zlib_frame(&mut self.stream),
        }
    }

    /// As [`Self::read_body`] for a listserver reply: on a gen-5 link a plain
    /// zlib frame (first byte `0x78`) is accepted too.
    fn read_list_body(&mut self) -> io::Result<Vec<u8>> {
        match self.gen5_in.as_mut() {
            Some(g) => crate::protocol::read_frame_or_plain(&mut self.stream, g),
            None => read_zlib_frame(&mut self.stream),
        }
    }

    /// Send logical messages in one frame.
    ///
    /// Gen-3: zlib + insertion. Gen-5: ctype + XOR + compress (bz2 if ≥ 0x1000).
    pub fn send_messages(&mut self, msgs: &[Vec<u8>]) -> io::Result<()> {
        if let Some(pli) = &self.pli {
            if let Some(id) = msgs.iter().filter_map(|m| m.first()).map(|b| b.wrapping_sub(0x20)).find(|&id| !pli.contains(id)) {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("this client version cannot send message {id}"),
                ));
            }
        }
        let mut body = Vec::new();
        for m in msgs {
            // A message that is not one whole line would be split or merged
            // by the server's line reader. On a gen-3 link that also moves
            // the server's insertion iterator off ours, and from then on
            // every message reads as garbage ("Disconnected for sending
            // invalid packets"). Such a message is dropped, not sent.
            if let Err(why) = check_wire_message(m) {
                self.log_send(m, Some(why));
                eprintln!("not sent: malformed message ({why}): {}", describe_message(m));
                continue;
            }
            self.log_send(m, None);
            match self.gen3.as_mut() {
                Some(g) => body.extend(g.encode(m)),
                None => body.extend_from_slice(m),
            }
        }
        if let Some(g5) = self.gen5.as_mut() {
            // 5.12 sends even short bodies compressed.
            let comp = match choose_comp(body.len()) {
                Gen5Comp::None if g5.is_round_down() => Gen5Comp::Zlib,
                c => c,
            };
            let compressed = match comp {
                Gen5Comp::Bz2 => bz2_compress(&body)?,
                Gen5Comp::Zlib => zlib_compress(&body)?,
                Gen5Comp::None => body,
            };
            let encoded = g5.encode(compressed, comp);
            let frame = write_frame(comp, &encoded);
            self.send_raw(&frame)?;
        } else {
            let frame = if self.zlib_best { write_zlib_frame_best(&body)? } else { write_zlib_frame(&body)? };
            self.send_raw(&frame)?;
        }
        self.stream.flush()
    }

    /// Send what `version` sends straight after its login line:
    /// `PLI_LANGUAGE` (37) for 2.05 and later. The tamper check (39) that
    /// 2.17+ follow it with is not sent; see [`Self::send_post_login_with`].
    pub fn send_post_login(&mut self, version: ClientVersion, language: &str) -> io::Result<()> {
        self.send_post_login_with(version, language, None)
    }

    /// As [`Self::send_post_login`], and with `tamper` the build's
    /// `PLI_TAMPERCHECK` (39) goes out as the second message of the same
    /// frame, as the real 2.17/2.22/2.31 clients send it. Opt-in: pass
    /// `None` to send the language only.
    pub fn send_post_login_with(
        &mut self,
        version: ClientVersion,
        language: &str,
        tamper: Option<&TamperSender>,
    ) -> io::Result<()> {
        let msgs = self.post_login_messages(version, language, tamper);
        if msgs.is_empty() {
            return Ok(());
        }
        self.send_messages(&msgs)
    }

    /// The messages of [`Self::send_post_login_with`], not sent.
    fn post_login_messages(&self, version: ClientVersion, language: &str, tamper: Option<&TamperSender>) -> Vec<Vec<u8>> {
        let mut msgs = Vec::new();
        if version.profile().protocol.sends_language {
            let mut msg = vec![37 + 0x20];
            msg.extend_from_slice(&to_latin1(language));
            msg.push(b'\n');
            msgs.push(msg);
        }
        if let Some(m) = tamper.and_then(|t| t.message()) {
            msgs.push(m);
        }
        msgs
    }

    /// Send `PLI_ENTERLEVEL` (46) for `version`, if its profile's send set
    /// has the id. `then` are messages that go in the same frame right
    /// after it: a real 2.31 client sends 46 immediately before the props
    /// message that carries the cell prop (43/44) and the wrapped
    /// coordinate ([`crate::gmap::crossing_frame`] builds that frame for a
    /// crossing). Elsewhere callers pass nothing and the next walk update is
    /// the props frame that follows.
    ///
    /// Returns `Ok(false)` and sends nothing for a profile without 46.
    pub fn send_enter_level(&mut self, version: ClientVersion, then: &[Vec<u8>]) -> io::Result<bool> {
        if !version.sends_enter_level() {
            return Ok(false);
        }
        let mut msgs = vec![enter_level_message()];
        msgs.extend(then.iter().cloned());
        self.send_messages(&msgs)?;
        Ok(true)
    }

    /// Send the frame a client sends when it walks across a gmap edge:
    /// [`crate::gmap::crossing_frame`] (46 for 2.3 and 2.31, the tamper check
    /// for 2.22 when `tamper` is given, then the props with the cell prop and
    /// the wrapped coordinate). Returns whether a 46 went out. Sends nothing
    /// for a profile without gmaps.
    pub fn send_gmap_crossing(
        &mut self,
        version: ClientVersion,
        c: &crate::gmap::Crossing,
        tamper: Option<&mut TamperSender>,
    ) -> io::Result<bool> {
        let mut msgs = crate::gmap::crossing_frame(version, c, tamper);
        if msgs.is_empty() {
            return Ok(false);
        }
        // The flags ride ahead of the props, which come last in the frame.
        let props = msgs.pop();
        msgs.extend(self.take_files());
        msgs.extend(self.take_flags());
        msgs.extend(props);
        let with_46 = msgs.iter().any(|m| m.first() == Some(&(PLI_ENTERLEVEL + 0x20)));
        self.send_messages(&msgs)?;
        Ok(with_46)
    }

    /// Send `PLI_TAMPERCHECK` (39) again (a re-send the build makes: see
    /// [`TamperSender::advance`] and [`TamperSender::level_changed`]).
    /// Refused like any id outside the logged-in version's send set.
    pub fn send_tampercheck(&mut self, tamper: &TamperSender) -> io::Result<()> {
        match tamper.message() {
            Some(m) => self.send_messages(&[m]),
            None => Ok(()),
        }
    }

    /// Queue a player flag a script set (`Some(value)`, empty for a plain
    /// `set`) or cleared (`None`): `PLI_FLAGSET` (18) or `PLI_FLAGDEL` (19).
    /// Real 2.x clients send a flag when a script changes it; the `gr.x` and
    /// `gr.y` flags in the lab captures come from GServer-v2's `-gr_movement`
    /// weapon, and a real 2.22 without that weapon sends none
    /// (`documentation/gr-flags.md`).
    /// A version that cannot send the id drops the flag here, so it never
    /// holds up the walk update it would ride with.
    pub fn queue_flag(&mut self, name: &str, value: Option<&str>) {
        // The real 4.04r2 runs a GS1 weapon's `setstring gr.x,..`,
        // `setstring client.probe,2` and `set probeflag`, and sends only
        // `client.probe=2` (lab, 2026-10-31, `documentation/v4-profiles.md`);
        // the real 2.31 does the same (cloud lab, 2026-10-31).
        if self.client_flags_only && !name.starts_with("client.") {
            return;
        }
        let m = crate::protocol::flagset::flag_message(name, value);
        if self.pli.as_ref().is_some_and(|pli| !pli.contains(m[0] - 0x20)) {
            return;
        }
        self.flags_since.get_or_insert_with(Instant::now);
        // A newer value of a flag still waiting for its frame replaces the
        // older one: the real 2.22 sends one `gr.x` per frame (lab,
        // 2026-10-30), never a stale value ahead of the new one.
        if value.is_some() {
            let mut key = vec![m[0]];
            key.extend(name.as_bytes());
            self.flags_out.retain(|q| !(q.starts_with(&key) && matches!(q.get(key.len()), Some(b'=' | b'\n'))));
        }
        self.flags_out.push(m);
    }

    /// The queued flag messages, for a frame that carries them (the
    /// post-login frame).
    pub fn take_flags(&mut self) -> Vec<Vec<u8>> {
        self.flags_since = None;
        std::mem::take(&mut self.flags_out)
    }

    /// Queue file requests (`WANTFILE`) for the frame of the next walk
    /// update: the real 2.22 and 2.31 clients put the first-step requests
    /// in the frame that carries the step's props (lab, 2026-10-30,
    /// `documentation/difftest-leftovers.md` section 7).
    pub fn queue_files(&mut self, msgs: Vec<Vec<u8>>) {
        if msgs.is_empty() {
            return;
        }
        self.files_since.get_or_insert_with(Instant::now);
        self.files_out.extend(msgs);
    }

    /// The queued file requests, for a frame that carries them.
    fn take_files(&mut self) -> Vec<Vec<u8>> {
        self.files_since = None;
        std::mem::take(&mut self.files_out)
    }

    /// Send the queued file requests on their own once the oldest has
    /// waited `max_age` for a walk update to carry it. Nothing otherwise.
    pub fn send_stale_files(&mut self, max_age: Duration) -> io::Result<()> {
        if !self.files_since.is_some_and(|t| t.elapsed() >= max_age) {
            return Ok(());
        }
        let msgs = self.take_files();
        self.send_messages(&msgs)
    }

    /// Send the queued flags on their own once the oldest has waited
    /// `max_age` for a walk update to carry it. Nothing otherwise.
    pub fn send_stale_flags(&mut self, max_age: Duration) -> io::Result<()> {
        if !self.flags_since.is_some_and(|t| t.elapsed() >= max_age) {
            return Ok(());
        }
        let msgs = self.take_flags();
        self.send_messages(&msgs)
    }

    /// Send `PLI_PLAYERPROPS` (2) with X (15), Y (16) and SPRITE (17).
    ///
    /// This is the 2.05 "walk" message: X/Y in half-tiles, SPRITE with
    /// the direction in its low two bits.
    pub fn send_position(&mut self, x: f32, y: f32, dir: u8) -> io::Result<()> {
        self.last_walk = None;
        let m = self.walk_message(x, y, dir % 4);
        self.send_messages(&[m])
    }


    /// Read server frames on a background thread.
    ///
    /// After a gen-5 login the reader uses gen-5 framing (ctype + XOR +
    /// decompress) and takes over the server→client iterator. Otherwise
    /// it reads plain zlib frames.
    pub fn spawn_reader(&mut self) -> io::Result<Receiver<io::Result<Vec<u8>>>> {
        let timed = self.spawn_reader_timed()?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for (_, frame) in timed {
                if tx.send(frame).is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }

    /// As [`Self::spawn_reader`], with the time each frame was read off the
    /// socket, so a timer that counts from a server message (the post-login
    /// frame, 240 ms after the level) does not start late when the game loop
    /// is slow to take it.
    ///
    /// One thread does all the socket's reads and writes from here on, with
    /// the socket non-blocking: the game loop's sends ([`Self::send_raw`])
    /// are queued to it. A blocking read on one handle and a write on a
    /// clone of it from another thread froze Seihai on Windows: the 2.22
    /// login went out, then not one byte more, and the process could not be
    /// ended (lab, 2026-10-31, `documentation/cross-laptop-2026-10-31.md`).
    pub fn spawn_reader_timed(&mut self) -> io::Result<Receiver<(Instant, io::Result<Vec<u8>>)>> {
        let stream = self.stream.try_clone()?;
        stream.set_read_timeout(None)?;
        stream.set_nonblocking(true)?;
        let cipher = self.gen5_in.take();
        let frame_len = self.frame_len;
        let (tx, rx) = mpsc::channel();
        let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
        let handle = std::thread::Builder::new()
            .name("seihai-net".into())
            .spawn(move || socket_thread(stream, cipher, frame_len, out_rx, tx))?;
        self.io_thread = Some(IoThread { out: out_tx, thread: handle.thread().clone() });
        Ok(rx)
    }

    /// Read one server frame and apply its messages to `state`.
    ///
    /// Uses [`Self::read_body`], then `SessionState::apply_body`. Returns
    /// the decompressed body.
    pub fn read_frame_into(&mut self, state: &mut SessionState) -> io::Result<Vec<u8>> {
        let body = self.read_body()?;
        state.apply_body(&body);
        Ok(body)
    }

    /// Send the locked classic login line.
    pub fn login_classic(
        &mut self,
        username: &str,
        password: &str,
        version: &str,
    ) -> io::Result<()> {
        let pkt = LoginRequest::new(username, password).with_version(version);
        let mut buf = Vec::new();
        pkt.write_classic(&mut buf)?;
        self.send_raw(&buf)?;
        self.stream.write_all(b"\n")?;
        self.stream.flush()
    }

    /// Read lines from the server until the connection closes or times out.
    /// Returns the collected text. No decryption.
    pub fn read_lines(&mut self) -> io::Result<String> {
        let mut reader = BufReader::new(&self.stream);
        let mut out = String::new();
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => out.push_str(&line),
                Err(e) if e.kind() == io::ErrorKind::TimedOut => break,
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// Connect to a listserver, send the classic login, return the reply text.
    pub fn listserver_session(
        addr: &str,
        username: &str,
        password: &str,
        version: &str,
    ) -> io::Result<String> {
        let mut conn = Self::connect(addr)?;
        conn.login_classic(username, password, version)?;
        conn.read_lines()
    }

    /// Send a login request (provisional encoding).
    pub fn login(&mut self, username: &str, password: &str) -> io::Result<()> {
        let pkt = LoginRequest::new(username, password);
        let mut buf = Vec::new();
        pkt.write_to(&mut buf)?;
        self.send_raw(&buf)
    }

    /// Request a level by name.
    pub fn request_level(&mut self, name: &str) -> io::Result<()> {
        let pkt = LevelRequest::new(name);
        let mut buf = Vec::new();
        pkt.write_to(&mut buf)?;
        self.send_raw(&buf)
    }

    /// Send a player position update.
    pub fn send_player_update(&mut self, x: f32, y: f32, dir: u8) -> io::Result<()> {
        let pkt = PlayerUpdate { x, y, dir };
        let mut buf = Vec::new();
        pkt.write_to(&mut buf)?;
        self.send_raw(&buf)
    }

    /// One tick's frame for a build with a [`WalkCadence`]: `lead` (a gmap
    /// crossing's tamper check, [`crate::gmap::crossing_lead`]), the queued
    /// flags, the tile changes, then the props message, if any. Nothing when
    /// all are empty.
    ///
    /// [`WalkCadence`]: crate::protocol::walkcadence::WalkCadence
    pub fn send_tick_frame(&mut self, lead: Vec<Vec<u8>>, boards: Vec<Vec<u8>>, props: Option<Vec<u8>>) -> io::Result<()> {
        // Queued file requests go in the next tick's frame, with or without
        // props. (2.22's later-step requests do not wait for a tick: the
        // main loop sends them at once.)
        let mut msgs = lead;
        msgs.extend(self.take_files());
        msgs.extend(self.take_flags());
        msgs.extend(boards);
        msgs.extend(props);
        if msgs.is_empty() {
            return Ok(());
        }
        self.send_messages(&msgs)
    }

    /// The walk props for this login: X/Y/SPRITE, or SPRITE/X2/Y2 for a
    /// precise-position build.
    fn walk_message(&self, x: f32, y: f32, sprite: u8) -> Vec<u8> {
        if self.precise_walk {
            position_message_precise(x, y, sprite)
        } else {
            position_message_sprite(x, y, sprite)
        }
    }

    /// Send a walk update for the given position (see [`Self::send_position`]).
    pub fn send_walk(&mut self, upd: &PlayerUpdate) -> io::Result<()> {
        self.send_position(upd.x, upd.y, upd.dir)
    }

    /// A walk update with an explicit SPRITE value (`dir + 4 * S`): the
    /// sword swing and hurt mode ([`crate::combat::CombatState::sprite`]).
    pub fn send_walk_sprite(&mut self, upd: &PlayerUpdate, sprite: u8) -> io::Result<()> {
        let walk = self.walk_message(upd.x, upd.y, sprite);
        let changed = self.last_walk.as_ref() != Some(&walk);
        // Queued file requests ride with a walk update only.
        let mut msgs = if changed { self.take_files() } else { Vec::new() };
        msgs.extend(self.take_flags());
        if changed {
            self.last_walk = Some(walk.clone());
            msgs.push(walk);
        }
        if msgs.is_empty() {
            return Ok(());
        }
        self.send_messages(&msgs)
    }

    /// Say `text` in the level: `PLI_PLAYERPROPS` (2) with CURCHAT (12)
    /// as a GSTRING. The server shows it as the chat bubble and runs chat
    /// commands such as `setnick` from it. An empty text clears the bubble.
    pub fn send_chat(&mut self, text: &str) -> io::Result<()> {
        let mut msg = vec![2 + 0x20, 12 + 0x20];
        push_gstring_latin1(&mut msg, text);
        msg.push(b'\n');
        self.send_messages(&[msg])
    }

    /// Tell the server the player's looks changed: `PLI_PLAYERPROPS` (2)
    /// with HEADGIF (11, length + 100 then the name), BODYIMG (35, a
    /// GSTRING), COLORS (13, five GCHARs) and the GANI attributes that are
    /// set (GATTRIB1..5, 37..41, GSTRINGs) when `own` (the build's player
    /// props) has them: a build before 2.02 sends no attributes. The
    /// defaults go as their names (`head0.png`, `body.png`).
    pub fn send_looks(&mut self, looks: &crate::looks::Looks, own: &IdSet) -> io::Result<()> {
        let mut msg = vec![2 + 0x20, 11 + 0x20];
        let head = looks.head_name().as_bytes();
        let head = &head[..head.len().min(123)];
        msg.push((head.len() + 100) as u8 + 0x20);
        msg.extend_from_slice(head);
        msg.push(35 + 0x20);
        push_gstring_latin1(&mut msg, looks.body_name());
        msg.push(13 + 0x20);
        msg.extend(looks.colors.map(|c| c + 0x20));
        // GATTRIB1..5 (37..41), the ones set.
        for (i, a) in looks.attrs.iter().enumerate() {
            if let Some(a) = a.as_ref().filter(|_| own.contains(37 + i as u8)) {
                msg.push(37 + i as u8 + 0x20);
                push_gstring_latin1(&mut msg, a);
            }
        }
        msg.push(b'\n');
        self.send_messages(&[msg])
    }

    /// Send a toall: `PLI_TOALL` (6) with a GSTRING message.
    pub fn send_toall(&mut self, text: &str) -> io::Result<()> {
        let mut msg = vec![6 + 0x20];
        push_gstring_latin1(&mut msg, text);
        msg.push(b'\n');
        self.send_messages(&[msg])
    }

    /// Send a private message: `PLI_PRIVATEMESSAGE` (28) with a GSHORT
    /// count, a GSHORT per player id, then the message gtokenized.
    pub fn send_pm(&mut self, ids: &[u16], text: &str) -> io::Result<()> {
        let mut msg = vec![28 + 0x20];
        msg.extend_from_slice(&gshort(ids.len() as u16));
        for &id in ids {
            msg.extend_from_slice(&gshort(id));
        }
        msg.extend(to_latin1(&gtokenize(text)));
        msg.push(b'\n');
        self.send_messages(&[msg])
    }

    /// Mute or unmute a player: `PLI_MUTEPLAYER` (43), `GSHORT id`, `GCHAR 1/0`.
    /// A build whose profile does not send 43 refuses it (`Unsupported`).
    pub fn send_mute(&mut self, player_id: u16, mute: bool) -> io::Result<()> {
        self.send_messages(&[crate::chatlists::mute_message(player_id, mute)])
    }

    /// Open a chest: `PLI_OPENCHEST` (20), `c x`, `c y` in whole tiles.
    pub fn send_open_chest(&mut self, x: i32, y: i32) -> io::Result<()> {
        let msg = vec![20 + 0x20, (x as u8) + 0x20, (y as u8) + 0x20, b'\n'];
        self.send_messages(&[msg])
    }

    /// Tell the server about a change to the level's tiles
    /// ([`board_modify_message`]).
    pub fn send_board_modify(&mut self, x: u8, y: u8, w: u8, h: u8, tiles: &[u16]) -> io::Result<()> {
        self.send_messages(&[board_modify_message(x, y, w, h, tiles)])
    }

    /// Fire a trigger action at `(x, y)` ([`trigger_action_message`]).
    ///
    /// A build without `PLI_TRIGGERACTION` (38) in its send set (1.39 and
    /// 1.41 among them) sends nothing and this is `Ok`: the try still runs
    /// locally. It used to be an `Unsupported` error, and the game loop
    /// dropped the connection on it, so a 1.39 player who pushed a wall went
    /// silently offline while the socket stayed open (stress test,
    /// 2026-10-31).
    pub fn send_trigger_action(&mut self, x: f32, y: f32, action: &str, param: &str) -> io::Result<()> {
        if !self.can_send(38) {
            return Ok(());
        }
        self.send_messages(&[trigger_action_message(x, y, action, param)])
    }

    /// Throw the carried object: `PLI_THROWCARRIED` (11), no payload (the
    /// 2.05 client sends none; GServer forwards whatever follows the id to
    /// the level as `PLO_THROWCARRIED` with the sender's id).
    pub fn send_throw_carried(&mut self) -> io::Result<()> {
        self.send_messages(&[throw_carried_message()])
    }

    /// Tell the others what the player now holds ([`carry_sprite_message`]).
    pub fn send_carry_sprite(&mut self, sprite: Option<u16>) -> io::Result<()> {
        self.send_messages(&[carry_sprite_message(sprite)])
    }

    /// Warp to `level` at `(x, y)` in tiles (`PLI_LEVELWARP`, id 0):
    /// `GCHAR x*2`, `GCHAR y*2`, level name. The server answers with the
    /// level, as at login.
    pub fn send_levelwarp(&mut self, x: f32, y: f32, level: &str) -> io::Result<()> {
        self.send_messages(&[levelwarp_message(x, y, level)])
    }

    /// A client-initiated level link: `PLI_LEVELWARP` (0), and with
    /// `tamper` for a build that re-sends it on level changes (its profile's
    /// rule, 2.22) the tamper check in the same frame right after it. A real
    /// 2.22 client sends `0` then `39` in one frame on every link request
    /// and does not send 39 after a server-side warp; 2.31 does not send it
    /// on links at all (lab, 2026-10-24). Returns whether 39 went along.
    ///
    /// This is the form for a level the client does not hold. With a copy,
    /// use [`Self::send_cached_link_request`].
    pub fn send_link_request(
        &mut self,
        x: f32,
        y: f32,
        level: &str,
        tamper: Option<&mut TamperSender>,
    ) -> io::Result<bool> {
        let mut msgs = vec![levelwarp_message(x, y, level)];
        if let Some(t) = tamper {
            if t.level_changed() {
                if let Some(m) = t.message() {
                    msgs.push(m);
                }
            }
        }
        let with_tamper = msgs.len() > 1;
        self.send_messages(&msgs)?;
        Ok(with_tamper)
    }

    /// A client-initiated level link for `version`, cached or not: the frame
    /// [`link_request_frame`] builds. `cached` is the copy's mod time; the
    /// player's `dir` goes into the props that follow 46. Returns what the
    /// frame held; `with_enter_level` means the caller owes no 46 after the
    /// server's answer ([`EnterLevelSender::cached_link`]).
    pub fn send_link(
        &mut self,
        version: ClientVersion,
        cached: Option<u32>,
        x: f32,
        y: f32,
        dir: u8,
        level: &str,
        tamper: Option<&mut TamperSender>,
    ) -> io::Result<LinkFrame> {
        let frame = link_request_frame(version, cached, x, y, dir, level, tamper);
        self.send_messages(&frame.messages)?;
        Ok(frame)
    }

    /// Request a file from the server (`PLI_WANTFILE`, id 23).
    pub fn send_wantfile(&mut self, name: &str) -> io::Result<()> {
        self.send_messages(&[wantfile_message(name)])
    }

}

/// A GSHORT: two GCHARs, high 7 bits first.
pub(crate) fn gshort(v: u16) -> [u8; 2] {
    [((v >> 7) & 0x7F) as u8 + 0x20, (v & 0x7F) as u8 + 0x20]
}

/// Graal text is single-byte: Latin-1 as is, anything else as `?`.
fn to_latin1(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| if (c as u32) < 0x100 { c as u8 } else { b'?' })
        .collect()
}

/// A GSTRING of `text` in Latin-1, cut to the 223 bytes a GCHAR length
/// can hold.
fn push_gstring_latin1(out: &mut Vec<u8>, text: &str) {
    let mut b = to_latin1(text);
    b.truncate(223);
    out.push(b.len() as u8 + 0x20);
    out.extend(b);
}

/// GServer's gtokenize: one quoted token per line, `"` doubled, joined
/// with commas.
pub fn gtokenize(text: &str) -> String {
    text.lines()
        .map(|l| format!("\"{}\"", l.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(",")
}

/// A login key like the real client's `rand & 0x7F`.
///
/// A fixed even key would always put the gen-3 byte at index 0 of short
/// lines.
pub fn random_key() -> u8 {
    (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(1)
        & 0x7F) as u8
}

/// Extract server-list-like lines from a listserver reply.
///
/// Provisional: returns non-empty lines that do not start with the
/// version token. Real list framing is still to be locked.
pub fn parse_server_list(reply: &str) -> Vec<String> {
    reply
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with("GNW"))
        .map(|l| l.to_string())
        .collect()
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::filecache::FileRequest;
    use crate::protocol::{parse_player_props, zlib_decompress};
    use std::io::Read;
    use std::net::TcpListener;

    #[test]
    fn count_props_carry_arrows_then_bombs() {
        // the real 2.05 after a shot: ARROWSCOUNT 19; after a bomb: BOMBSCOUNT 14
        assert_eq!(count_props_message(Some(19), None), vec![0x22, 0x24, 19 + 0x20, b'\n']);
        assert_eq!(count_props_message(None, Some(14)), vec![0x22, 0x25, 14 + 0x20, b'\n']);
        assert_eq!(count_props_message(Some(0), Some(3)), vec![0x22, 0x24, 0x20, 0x25, 0x23, b'\n']);
    }

    #[test]
    fn precise_walk_matches_the_real_404() {
        // The real 4.04r2's first step right from x 30 (lab, 2026-10-31):
        // `*$walk1#n'p`, that is GANI walk, SPRITE 3, X2 976 (30.5 tiles).
        // Then the next step: `n'\x80` (X2 992, 31 tiles).
        let m = position_message_precise(30.5, 30.5, 3);
        assert_eq!(&m[..6], &[2 + 0x20, b'1', b'#', b'n', b'\'', b'p']);
        assert_eq!(&m[6..9], &[b'o', b'\'', b'p']);
        assert_eq!(*m.last().unwrap(), b'\n');
        assert_eq!(&position_message_precise(31.0, 0.0, 0)[4..6], b"'\x80");
    }

    #[test]
    fn walk_matches_the_real_231() {
        // Real 2.31 in the cloud lab (2026-10-31): the first step right from
        // x 30 is `GANI walk, SPRITE 3, X2 30.5`, then X2 only, never X/Y.
        // Seihai sends SPRITE, X2 and Y2 together for 2.31 now; the
        // classic X/Y stays for 2.22 and 3.04, which have no X2/Y2.
        for (v, precise) in [(ClientVersion::V222, false), (ClientVersion::V230, true), (ClientVersion::V231, true), (ClientVersion::V304, false), (ClientVersion::V3153, true)] {
            assert_eq!(walk_message_for(v, 30.5, 30.0, 3)[1], if precise { 17 } else { 15 } + 0x20, "{}", v.label());
        }
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V231, 5, "acct", "pw").unwrap();
        let _ = read_frame(&mut srv);
        assert!(conn.precise_walk && conn.client_flags_only);
        conn.queue_flag("gr.x", Some("30"));
        conn.queue_flag("probeflag", None);
        conn.queue_flag("client.probe", Some("2"));
        assert_eq!(conn.take_flags(), vec![b"\x32client.probe=2\n".to_vec()]);
        assert_eq!(conn.walk_message(30.5, 30.0, 3), position_message_precise(30.5, 30.0, 3));
        // X2 30.5 tiles is 976 = GSHORT `n'p`, as in the 4.04r2 test.
        assert_eq!(&position_message_precise(30.5, 0.0, 3)[3..6], &[78 + 0x20, b'\'', b'p']);
    }

    #[test]
    fn lobby_builds_walk_with_x2_y2_and_send_client_flags_only() {
        for (v, precise) in [(ClientVersion::V222, false), (ClientVersion::V404R2, true), (ClientVersion::V420R8, true)] {
            let (mut conn, mut srv) = pair();
            conn.login_versioned(v, 5, "acct", "pw").unwrap();
            let _ = read_frame(&mut srv);
            assert_eq!(conn.precise_walk, precise, "{}", v.label());
            assert_eq!(conn.client_flags_only, precise, "{}", v.label());
            assert_eq!(conn.walk_message(30.0, 30.5, 2)[1], if precise { 17 } else { 15 } + 0x20, "{}", v.label());
            conn.queue_flag("gr.x", Some("30"));
            conn.queue_flag("probeflag", None);
            conn.queue_flag("client.probe", Some("2"));
            let sent: Vec<Vec<u8>> = conn.take_flags();
            let want: Vec<&[u8]> = if precise { vec![b"\x32client.probe=2\n"] } else { vec![b"\x32gr.x=30\n", b"\x33probeflag\n", b"\x32client.probe=2\n"] };
            assert_eq!(sent, want, "{}", v.label());
        }
    }

    fn read_frame(s: &mut TcpStream) -> Vec<u8> {
        let mut len = [0u8; 2];
        s.read_exact(&mut len).unwrap();
        let mut body = vec![0u8; u16::from_be_bytes(len) as usize];
        s.read_exact(&mut body).unwrap();
        zlib_decompress(&body).unwrap()
    }

    /// A link's arrival is applied when its destination is named, however
    /// late, and in any case or without the extension; another level drops it.
    #[test]
    fn link_arrival_is_held_by_the_destination_name() {
        let mut held = Some(LinkArrival::new("westocean_n-17.nw", (30.0, 2.0)));
        assert_eq!(LinkArrival::take_for(&mut held, "WestOcean_n-17"), Some((30.0, 2.0)));
        assert_eq!(held, None, "applied once");
        let mut held = Some(LinkArrival::new("house.nw", (12.0, 3.0)));
        assert_eq!(LinkArrival::take_for(&mut held, "onlinestartlocal.nw"), None);
        assert_eq!(held, None, "a warp elsewhere drops it");
        assert_eq!(LinkArrival::take_for(&mut None, "house.nw"), None);
    }

    #[test]
    fn max_power_message_is_prop_1() {
        assert_eq!(max_power_message(3), vec![0x22, 0x21, 0x23, b'\n']);
        assert_eq!(max_power_message(99), vec![0x22, 0x21, 0x34, b'\n'], "20 at most");
    }

    /// A flag a script set rides ahead of the walk props in the same frame.
    /// Two sets of one flag before its frame: only the last goes, in its
    /// own place; a delete and other flags keep theirs.
    #[test]
    fn a_flag_set_twice_before_its_frame_goes_once() {
        let (mut conn, _srv) = pair();
        conn.queue_flag("gr.x", Some("128"));
        conn.queue_flag("door", Some(""));
        conn.queue_flag("gr.x", Some("127.5"));
        conn.queue_flag("door", None);
        assert_eq!(conn.take_flags(), [b"\x32door\n".to_vec(), b"\x32gr.x=127.5\n".to_vec(), b"\x33door\n".to_vec()]);
    }

    #[test]
    fn script_flags_go_ahead_of_the_walk_props() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let mut conn = Connection::connect(&addr).unwrap();
        let (mut srv, _) = listener.accept().unwrap();
        conn.login_versioned(ClientVersion::V205, 0x42, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);

        conn.send_walk_sprite(&PlayerUpdate { x: 30.0, y: 30.5, dir: 1 }, 1).unwrap();
        conn.queue_flag("gr.x", Some("29.5"));
        conn.queue_flag("door", None);
        conn.send_walk_sprite(&PlayerUpdate { x: 29.5, y: 30.5, dir: 1 }, 1).unwrap();
        let mut g = Gen3::new(0x42);
        let mut decode = |srv: &mut TcpStream| {
            let body = read_frame(srv);
            body.split_inclusive(|b| *b == b'\n').map(|l| g.decode(l).unwrap()).collect::<Vec<_>>()
        };
        assert_eq!(decode(&mut srv).len(), 1, "no flags queued");
        let second = decode(&mut srv);
        assert_eq!(second.len(), 3);
        assert_eq!(second[0], b"\x32gr.x=29.5\n");
        assert_eq!(second[1], b"\x33door\n");
        assert_eq!(second[2][0], 2 + 0x20);
        assert!(conn.take_flags().is_empty());
    }

    /// Queued file requests go first in the frame of the next walk update
    /// that changes something, ahead of the flags and the props, as the real
    /// 2.22 and 2.31 put the first-step requests; a tick frame takes them
    /// with or without props.
    #[test]
    fn queued_file_requests_lead_the_next_walk_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let mut conn = Connection::connect(&addr).unwrap();
        let (mut srv, _) = listener.accept().unwrap();
        conn.login_versioned(ClientVersion::V205, 0x42, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let mut g = Gen3::new(0x42);
        let mut decode = |srv: &mut TcpStream| {
            let body = read_frame(srv);
            body.split_inclusive(|b| *b == b'\n').map(|l| g.decode(l).unwrap()).collect::<Vec<_>>()
        };
        let want = |n: &str| FileRequest::Want(n.into()).message();

        let here = PlayerUpdate { x: 30.0, y: 30.5, dir: 3 };
        conn.send_walk_sprite(&here, 3).unwrap();
        assert_eq!(decode(&mut srv).len(), 1);
        conn.queue_files(vec![want("steps.wav"), want("walk.gani")]);
        // An unchanged update sends nothing and keeps the requests.
        conn.send_walk_sprite(&here, 3).unwrap();
        conn.queue_flag("gr.x", Some("30.5"));
        conn.send_walk_sprite(&PlayerUpdate { x: 30.5, ..here }, 3).unwrap();
        let f = decode(&mut srv);
        assert_eq!(f.len(), 4);
        assert_eq!(f[0], want("steps.wav"));
        assert_eq!(f[1], want("walk.gani"));
        assert_eq!(f[2], b"\x32gr.x=30.5\n");
        assert_eq!(f[3][0], 2 + 0x20);

        conn.queue_files(vec![want("steps2.wav")]);
        conn.send_tick_frame(Vec::new(), Vec::new(), None).unwrap();
        assert_eq!(decode(&mut srv), vec![want("steps2.wav")], "a tick without props");
        assert!(conn.take_files().is_empty());
    }

    /// The same X, Y and SPRITE again is not re-sent; a change is.
    #[test]
    fn an_unchanged_walk_update_is_not_sent_again() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let mut conn = Connection::connect(&addr).unwrap();
        let (mut srv, _) = listener.accept().unwrap();
        conn.login_versioned(ClientVersion::V205, 0x42, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let here = PlayerUpdate { x: 30.0, y: 30.5, dir: 1 };
        for _ in 0..5 {
            conn.send_walk_sprite(&here, 47).unwrap();
        }
        conn.send_walk_sprite(&here, 51).unwrap();
        let mut g = Gen3::new(0x42);
        let mut decode = |srv: &mut TcpStream| {
            let body = read_frame(srv);
            body.split_inclusive(|b| *b == b'\n').map(|l| g.decode(l).unwrap()).collect::<Vec<_>>()
        };
        assert_eq!(decode(&mut srv).len(), 1, "the first one");
        // The next frame on the wire is the SPRITE change, not a repeat.
        let next = decode(&mut srv);
        assert_eq!(next, vec![position_message_sprite(30.0, 30.5, 51)]);
    }

    #[test]
    fn walk_after_login_is_gen3_framed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let mut conn = Connection::connect(&addr).unwrap();
        let (mut srv, _) = listener.accept().unwrap();

        conn.login_versioned(ClientVersion::V205, 0x42, "ab", "cd").unwrap();
        conn.send_position(30.5, 12.0, 2).unwrap();

        let login = read_frame(&mut srv);
        assert_eq!(&login[..2], &[0x20, 0x42 + 0x20]);

        // Server side: undo the gen-3 byte, as GServer does.
        let body = read_frame(&mut srv);
        let line = body.strip_suffix(b"\n").unwrap();
        let mut g = Gen3::new(0x42);
        let mut enc = line.to_vec();
        enc.push(b'\n');
        let msg = g.decode(&enc).unwrap();
        assert_eq!(msg[0], 2 + 0x20);
        let props = parse_player_props(&msg[1..msg.len() - 1]);
        let vals: Vec<(u8, Option<u8>)> = props.iter().map(|p| (p.id, p.gchar())).collect();
        assert_eq!(vals, vec![(15, Some(61)), (16, Some(24)), (17, Some(2))]);
    }

    #[test]
    fn wire_messages_are_one_whole_line() {
        assert!(check_wire_message(b"2/\x7f0t1 \n").is_ok());
        assert!(check_wire_message(b"7\n").is_ok());
        assert_eq!(check_wire_message(b""), Err("empty"));
        assert_eq!(check_wire_message(b"\n"), Err("id byte below 0x20"));
        assert_eq!(check_wire_message(b"2/\x7f"), Err("no closing line feed"));
        assert_eq!(check_wire_message(b"2gr.x=1\ngr.y=2\n"), Err("line feed inside the payload"));
        assert_eq!(describe_message(b"7a\\b\x01\n"), "id 23 len 6 a\\x5cb\\x01\\x0a");
    }

    /// A message with a line feed inside would read as two messages on the
    /// server, which would step its gen-3 iterator once more than Seihai's
    /// for the rest of the session. It is dropped, and the iterator does not
    /// step for it: the next message still decodes.
    #[test]
    fn a_malformed_message_is_dropped_and_gen3_stays_in_step() {
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V205, 0x31, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let good = crate::protocol::flagset::flag_message("a", Some("1"));
        let bad = b"\x32b=1\nc=2\n".to_vec();
        conn.send_messages(&[bad, good.clone()]).unwrap();
        conn.send_messages(&[good.clone()]).unwrap();
        assert_eq!(gen3_messages(0x31, &read_frame(&mut srv)), vec![good.clone()]);
        let mut g = Gen3::new(0x31);
        let _ = g.next_insertion(good.len());
        let body = read_frame(&mut srv);
        assert_eq!(g.decode(&body).unwrap(), good);
    }

    fn read_raw(s: &mut TcpStream) -> Vec<u8> {
        let mut len = [0u8; 2];
        s.read_exact(&mut len).unwrap();
        let mut body = vec![0u8; u16::from_be_bytes(len) as usize];
        s.read_exact(&mut body).unwrap();
        body
    }

    fn pair() -> (Connection, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let conn = Connection::connect(&addr).unwrap();
        let (srv, _) = listener.accept().unwrap();
        (conn, srv)
    }

    /// GServer's view of a 5.12 login: the plain zlib login line ends with
    /// the machine-field list, and the language that follows goes out
    /// zlib-compressed (5.12 compresses even short bodies) with 5.12's
    /// round-down step count. 5.24r4 sends the same line with its own
    /// token, the language raw and stepped as every other build.
    /// The real 4.04r2's check of its cached `basepackage.gupd` (`GRPKG001`
    /// and a newline, CRC-32 0x15a8ea07), lab 2026-10-31.
    #[test]
    fn verify_want_matches_the_real_404() {
        assert_eq!(crate::assets::crc32::crc32(b"GRPKG001\n"), 0x15a8ea07);
        assert_eq!(verify_want_message(0x15a8ea07, "basepackage.gupd"), b"O!MCt'basepackage.gupd\n");
    }

    #[test]
    fn lobby_logins_on_the_wire() {
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V512, 0x4B, "guest", "7Y3bdzjf").unwrap();
        conn.send_post_login(ClientVersion::V512, "English").unwrap();
        assert_eq!(read_frame(&mut srv), b"%kG3D14097%guest(7Y3bdzjfwin,\"\",\"\",\"\",\"6.2 9200 \"\n");
        let raw = read_raw(&mut srv);
        assert_eq!(raw[0], Gen5Comp::Zlib as u8);
        let mut body = raw[1..].to_vec();
        let mut g = Gen5::new(0x4B).round_down();
        g.xor_body(&mut body, Gen5Comp::Zlib);
        assert_eq!(zlib_decompress(&body).unwrap(), b"EEnglish\n");

        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V524R4, 0x4B, "guest", "7Y3bdzjf").unwrap();
        conn.send_post_login(ClientVersion::V524R4, "English").unwrap();
        assert!(read_frame(&mut srv).starts_with(b"%kG3D15010%guest(7Y3bdzjfwin,"));
        assert_eq!(gen5_body(&mut srv, &mut Gen5::new(0x4B)), b"EEnglish\n");
    }

    /// 1.39 has no `PLI_TRIGGERACTION` (38): a push try sends nothing and is
    /// not an error, so the game loop keeps the connection (stress test,
    /// 2026-10-31: the error used to drop it).
    #[test]
    fn trigger_action_is_skipped_where_the_build_has_none() {
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V139, 0x21, "guest", "pw").unwrap();
        let _ = read_frame(&mut srv);
        assert!(!conn.can_send(38));
        conn.send_trigger_action(30.0, 30.0, "pushed", "0").unwrap();
        srv.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
        let mut b = [0u8; 64];
        assert!(srv.read(&mut b).is_err(), "nothing went out");

        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V205, 0x21, "guest", "pw").unwrap();
        let _ = read_frame(&mut srv);
        assert!(conn.can_send(38));
        conn.send_trigger_action(30.0, 30.0, "pushed", "0").unwrap();
    }

    /// Dropping a `Connection` whose reader thread runs closes the socket
    /// (the server sees EOF at once) and tells the receiver it is closed.
    #[test]
    fn dropping_the_connection_closes_the_socket() {
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V222, 0x21, "guest", "pw").unwrap();
        let _ = read_frame(&mut srv);
        let rx = conn.spawn_reader_timed().unwrap();
        drop(conn);
        srv.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut b = [0u8; 64];
        assert_eq!(srv.read(&mut b).unwrap(), 0, "the server sees EOF");
        let (_, got) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(got.unwrap_err().kind(), io::ErrorKind::NotConnected);
    }

    /// A 6.x build reads a server frame of 0xFFFF bytes or more as `FF FF`
    /// and a `u32be` length, on the reader thread and in `read_body`; 5.31r4
    /// has no such form.
    #[test]
    fn six_x_reads_long_frames() {
        let body = [b"8".as_slice(), &vec![b'a'; 0x10000], b"\n"].concat();
        let long = |key: u8| {
            let mut enc = body.clone();
            Gen5::new(key).xor_body(&mut enc, Gen5Comp::None);
            [&[0xFF, 0xFF][..], &((enc.len() + 1) as u32).to_be_bytes(), &[Gen5Comp::None as u8], &enc].concat()
        };
        assert_eq!(ClientVersion::V603R7.profile().protocol.gen5_long_frames, true);
        assert_eq!(ClientVersion::V600R2.profile().protocol.gen5_long_frames, true);
        assert_eq!(ClientVersion::V531R4.profile().protocol.gen5_long_frames, false);

        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V603R7, 0x19, "guest", "7Y3bdzjf").unwrap();
        srv.write_all(&long(0x19)).unwrap();
        assert_eq!(conn.read_body().unwrap(), body);

        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V600R2, 0x19, "guest", "7Y3bdzjf").unwrap();
        let rx = conn.spawn_reader_timed().unwrap();
        srv.write_all(&long(0x19)).unwrap();
        let (_, got) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(got.unwrap(), body);

        // 5.31r4 takes `FF FF` as the length itself: the next byte is not a
        // compression type.
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V531R4, 0x19, "guest", "7Y3bdzjf").unwrap();
        srv.write_all(&long(0x19)).unwrap();
        assert!(conn.read_body().is_err());
    }

    /// GServer's view of a 2.22 login: the first frame is plain zlib and
    /// is the login line itself (type 5, key, token), and the next frame
    /// is gen-5 with the iterator at the seed.
    #[test]
    fn gen5_login_is_plain_then_seeded() {
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V222, 0x2B, "ab", "cd").unwrap();
        conn.send_messages(&[b"EEnglish\n".to_vec()]).unwrap();

        let login = read_frame(&mut srv);
        assert_eq!(&login[..10], b"%KGNW03014");
        assert_eq!(&login[10..], b"\"ab\"cd\n");

        let raw = read_raw(&mut srv);
        let mut g = Gen5::new(0x2B);
        let body = crate::protocol::read_frame(
            &mut [&(raw.len() as u16).to_be_bytes()[..], &raw].concat().as_slice(),
            &mut g,
        )
        .unwrap();
        assert_eq!(body, b"EEnglish\n");
    }

    /// 2.22 listserver login: plain zlib id 7 handshake, then a gen-5
    /// frame with id 1, then a gen-5 reply read back with the same key.
    #[test]
    fn gen5_listserver_login() {
        let (mut conn, mut srv) = pair();
        conn.login_listserver(ClientVersion::V222, 0x66, "ab", "cd").unwrap();
        let first = wire(&mut srv);
        assert_eq!(&first[2..4], &[0x78, 0xDA], "the id 7 frame is compressed at the best level");
        assert_eq!(crate::protocol::zlib_decompress(&first[2..]).unwrap(), b"'\x86GNW30123newmain\n");
        let raw = read_raw(&mut srv);
        let mut g = Gen5::new(0x66);
        assert_eq!(raw[0], Gen5Comp::None as u8);
        let mut body = raw[1..].to_vec();
        g.xor_body(&mut body, Gen5Comp::None);
        assert_eq!(body, b"!\"ab\"cd\n");

        // Reply: id 2 status, gen-5 zlib, server iterator from the seed.
        let mut out = Gen5::new(0x66);
        let z = zlib_compress(b"\"Lab listserver\n").unwrap();
        let frame = write_frame(Gen5Comp::Zlib, &out.encode(z, Gen5Comp::Zlib));
        srv.write_all(&frame).unwrap();
        let reply = parse_list_reply(&conn.read_body().unwrap());
        assert_eq!(reply.status.as_deref(), Some("Lab listserver"));
    }

    /// 3.04 listserver login: id 7 and id 1 in one plain zlib frame at the
    /// default level, as the real 3.04 sent it in the lab (2026-10-30); the
    /// reply is gen 5.
    #[test]
    fn one_frame_listserver_login_304() {
        let (mut conn, mut srv) = pair();
        conn.login_listserver(ClientVersion::V304, 0x2b, "ab", "cd").unwrap();
        let first = wire(&mut srv);
        assert_eq!(&first[2..4], &[0x78, 0x9C], "the real 3.04 frame is at the default level");
        assert_eq!(crate::protocol::zlib_decompress(&first[2..]).unwrap(), b"'KG3D30123newmain\n!\"ab\"cd\n");
        let mut out = Gen5::new(0x2b);
        let z = zlib_compress(b"\"Lab listserver\n").unwrap();
        srv.write_all(&write_frame(Gen5Comp::Zlib, &out.encode(z, Gen5Comp::Zlib))).unwrap();
        assert_eq!(parse_list_reply(&conn.read_body().unwrap()).status.as_deref(), Some("Lab listserver"));
    }

    /// The raw bytes of the next frame on the wire, length prefix and all.
    fn wire(srv: &mut TcpStream) -> Vec<u8> {
        let mut len = [0u8; 2];
        srv.read_exact(&mut len).unwrap();
        let mut body = vec![0u8; u16::from_be_bytes(len) as usize];
        srv.read_exact(&mut body).unwrap();
        [&len[..], &body].concat()
    }

    /// Shape A, the real 2.00 and 2.05: one plain zlib frame, id 0 `newmain`
    /// and LF, 17 bytes on the wire, header 78 DA, and nothing after it.
    /// 1.42 and 2.01 to 2.04 send it as an inference.
    #[test]
    fn shape_a_listserver_login() {
        for v in [ClientVersion::V142, ClientVersion::V200, ClientVersion::V201, ClientVersion::V204, ClientVersion::V205] {
            assert_eq!(v.listserver().shape, LoginShape::NewmainOnly, "{}", v.label());
            let (mut conn, mut srv) = pair();
            conn.login_listserver(v, 0x66, "ab", "cd").unwrap();
            let raw = wire(&mut srv);
            assert_eq!(raw.len(), 2 + 17, "{}: 17-byte zlib stream, as the real client", v.label());
            assert_eq!(&raw[2..4], &[0x78, 0xDA], "{}", v.label());
            assert_eq!(crate::protocol::zlib_decompress(&raw[2..]).unwrap(), b" newmain\n", "{}", v.label());
            // No account or password in it, and nothing follows.
            drop(conn);
            let mut rest = Vec::new();
            srv.read_to_end(&mut rest).unwrap();
            assert!(rest.is_empty(), "{}: nothing after the frame", v.label());
        }
    }

    /// Shape B, the real 2.17.1: one plain zlib frame, id 0 then id 1, 78 DA.
    /// The builds with no capture (2.1 to 2.16) keep it.
    #[test]
    fn shape_b_listserver_login() {
        for v in [ClientVersion::V210, ClientVersion::V216, ClientVersion::V217] {
            assert_eq!(v.listserver().shape, LoginShape::NewmainAccount, "{}", v.label());
            let (mut conn, mut srv) = pair();
            conn.login_listserver(v, 0x66, "ab", "cd").unwrap();
            let raw = wire(&mut srv);
            assert_eq!(&raw[2..4], &[0x78, 0xDA], "{}", v.label());
            assert_eq!(crate::protocol::zlib_decompress(&raw[2..]).unwrap(), b" newmain\n!\"ab\"cd\n", "{}", v.label());
        }
    }

    /// GServer's gen-2 test is "type 0, then a known token": 1.39 sends
    /// no key.
    #[test]
    fn gen2_login_has_no_key() {
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V139, 0x42, "ab", "cd").unwrap();
        assert_eq!(read_frame(&mut srv), b" GNW07080\"ab\"cd\n");
    }

    /// Gen-2 frames, the login and every later one, are compressed at
    /// zlib's best level (`78 DA`), as the real 1.28 beta 2 to 1.39 sent
    /// them in the lab (1.33 and 1.39 tested here); gen 3 keeps the default level.
    #[test]
    fn gen2_frames_use_the_best_zlib_level() {
        for v in [ClientVersion::V133, ClientVersion::V139] {
            let (mut conn, mut srv) = pair();
            conn.login_versioned(v, 0, "ab", "cd").unwrap();
            conn.send_messages(&[b"\x22 \n".to_vec()]).unwrap();
            for _ in 0..2 {
                let raw = wire(&mut srv);
                assert_eq!(&raw[2..4], &[0x78, 0xDA], "{}", v.label());
            }
        }
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V205, 0x21, "ab", "cd").unwrap();
        assert_eq!(&wire(&mut srv)[2..4], &[0x78, 0x9C]);
    }

    /// With the reader running, the game loop's sends still go out, in
    /// order, while the server's frames come in, including one the server
    /// wrote in two pieces. (The socket's own thread does both; the Windows
    /// freeze of 2026-10-31 was a send stuck behind the reader's read.)
    #[test]
    fn sends_go_out_while_the_reader_waits() {
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V222, 0x21, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let rx = conn.spawn_reader_timed().unwrap();
        // Nothing from the server yet: the reader is waiting, and sends
        // must not wait with it.
        let mut g = Gen5::new(0x21);
        srv.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        for i in 0..3u8 {
            conn.send_messages(&[vec![2 + 0x20, 15 + 0x20, 0x20 + i, b'\n']]).unwrap();
            assert_eq!(gen5_body(&mut srv, &mut g), [2 + 0x20, 15 + 0x20, 0x20 + i, b'\n']);
        }
        // A server frame in two writes arrives whole.
        let mut out = Gen5::new(0x21);
        let frame = write_frame(Gen5Comp::None, &out.encode(b"&new1.nw\n".to_vec(), Gen5Comp::None));
        srv.write_all(&frame[..3]).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        conn.send_messages(&[vec![2 + 0x20, 16 + 0x20, 0x21, b'\n']]).unwrap();
        assert_eq!(gen5_body(&mut srv, &mut g), [2 + 0x20, 16 + 0x20, 0x21, b'\n']);
        srv.write_all(&frame[3..]).unwrap();
        let (_, body) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(body.unwrap(), b"&new1.nw\n");
        // The server goes away: the reader says so, and later sends fail.
        drop(srv);
        let (_, end) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(end.is_err());
        let until = Instant::now() + Duration::from_secs(5);
        while conn.send_messages(&[vec![2 + 0x20, 15 + 0x20, 0x20, b'\n']]).is_ok() {
            assert!(Instant::now() < until, "sends still accepted after the close");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// A gen-5 server that refuses the login: the reader hands over the
    /// disconnect text before the close.
    #[test]
    fn gen5_disconnect_message_reaches_state() {
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V222, 0x11, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let mut out = Gen5::new(0x11);
        let msg = b"0Your client version is not allowed on this server.\n".to_vec();
        let frame = write_frame(Gen5Comp::None, &out.encode(msg, Gen5Comp::None));
        srv.write_all(&frame).unwrap();
        drop(srv);

        let rx = conn.spawn_reader().unwrap();
        let mut state = SessionState::default();
        let mut closed = false;
        while let Ok(item) = rx.recv() {
            match item {
                Ok(body) => state.apply_body(&body),
                Err(_) => {
                    closed = true;
                    break;
                }
            }
        }
        assert!(closed);
        assert_eq!(
            state.disconnect_message.as_deref(),
            Some("Your client version is not allowed on this server.")
        );
    }

    #[test]
    fn chat_toall_and_pm_wire() {
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V139, 0, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        conn.send_chat("hi there").unwrap();
        conn.send_toall("all").unwrap();
        conn.send_pm(&[200, 3], "a \"b\"").unwrap();
        assert_eq!(read_frame(&mut srv), b"\",(hi there\n");
        assert_eq!(read_frame(&mut srv), b"&#all\n");
        // 28, count 2, ids 200 and 3 as GSHORTs, then gtokenized text.
        assert_eq!(read_frame(&mut srv), b"< \"!h #\"a \"\"b\"\"\"\n");
    }

    /// 1.39 cannot send PLI_LANGUAGE (37); 2.05 sends it after login.
    #[test]
    fn sends_follow_the_version() {
        let (mut conn, mut server) = pair();
        conn.login_versioned(ClientVersion::V139, 0, "ab", "cd").unwrap();
        let _ = read_frame(&mut server);
        conn.send_post_login(ClientVersion::V139, "English").unwrap();
        let mut lang = vec![37 + 0x20];
        lang.extend_from_slice(b"English\n");
        assert!(conn.send_messages(&[lang]).is_err());

        let (mut conn, mut server) = pair();
        conn.login_versioned(ClientVersion::V205, 0x42, "ab", "cd").unwrap();
        let _ = read_frame(&mut server);
        conn.send_post_login(ClientVersion::V205, "English").unwrap();
        let body = read_frame(&mut server);
        assert_eq!(body[0], 37 + 0x20);
    }

    /// Split a gen-3 frame body into messages and undo the insertion, as
    /// GServer does.
    fn gen3_messages(key: u8, body: &[u8]) -> Vec<Vec<u8>> {
        let mut g = Gen3::new(key);
        body.split_inclusive(|&b| b == b'\n').map(|m| g.decode(m).unwrap()).collect()
    }

    fn lang_msg() -> Vec<u8> {
        let mut m = vec![37 + 0x20];
        m.extend_from_slice(b"English\n");
        m
    }

    /// Tamper check off (the default): the first frame is the language only.
    /// On: PLI 39 follows it in the same frame, with the build's 10 bytes.
    #[test]
    fn tampercheck_is_opt_in_and_second_in_the_frame() {
        // 2.17, gen 3.
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V217, 0x31, "ab", "cd").unwrap();
        conn.send_post_login(ClientVersion::V217, "English").unwrap();
        let _ = read_frame(&mut srv);
        assert_eq!(gen3_messages(0x31, &read_frame(&mut srv)), vec![lang_msg()]);

        let (mut conn, mut srv) = pair();
        let t = TamperSender::for_version(ClientVersion::V217).unwrap();
        conn.login_versioned(ClientVersion::V217, 0x31, "ab", "cd").unwrap();
        conn.send_post_login_with(ClientVersion::V217, "English", Some(&t)).unwrap();
        let _ = read_frame(&mut srv);
        let msgs = gen3_messages(0x31, &read_frame(&mut srv));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0], lang_msg());
        assert_eq!(msgs[1][0], 39 + 0x20);
        assert_eq!(&msgs[1][1..11], &[0x2a, 0x7b, 0xad, 0x19, 0x5e, 0x23, 0x12, 0x43, 0xd8, 0x2c]);
        assert_eq!(msgs[1][11], b'\n');

        // 2.22, gen 5: language then the 10 raw bytes.
        let (mut conn, mut srv) = pair();
        let t = TamperSender::for_version(ClientVersion::V222).unwrap();
        conn.login_versioned(ClientVersion::V222, 0x2B, "ab", "cd").unwrap();
        conn.send_post_login_with(ClientVersion::V222, "English", Some(&t)).unwrap();
        let _ = read_frame(&mut srv);
        let raw = read_raw(&mut srv);
        let mut g = Gen5::new(0x2B);
        let body = crate::protocol::read_frame(
            &mut [&(raw.len() as u16).to_be_bytes()[..], &raw].concat().as_slice(),
            &mut g,
        )
        .unwrap();
        let mut want = lang_msg();
        want.extend_from_slice(&[b'G', 0x25, 0x0e, 0x4d, 0x0f, 0x96, 0x2b, 0x03, 0xed, 0xe6, 0x31, b'\n']);
        assert_eq!(body, want);
    }

    /// A re-send goes out as one message of its own, and only for a version
    /// whose send set has id 39.
    #[test]
    fn tampercheck_resend_and_gate() {
        let (mut conn, mut srv) = pair();
        let t = TamperSender::for_version(ClientVersion::V231).unwrap();
        conn.login_versioned(ClientVersion::V231, 0x10, "ab", "cd").unwrap();
        conn.send_tampercheck(&t).unwrap();
        let _ = read_frame(&mut srv);
        let raw = read_raw(&mut srv);
        let mut g = Gen5::new(0x10);
        let body = crate::protocol::read_frame(
            &mut [&(raw.len() as u16).to_be_bytes()[..], &raw].concat().as_slice(),
            &mut g,
        )
        .unwrap();
        assert_eq!(&body[..1], b"G");
        assert_eq!(body.len(), 12);

        // 2.05 has no id 39: refused, nothing on the wire.
        let (mut conn, _srv) = pair();
        conn.login_versioned(ClientVersion::V205, 0x10, "ab", "cd").unwrap();
        let t = TamperSender::for_version(ClientVersion::V222).unwrap();
        assert!(conn.send_tampercheck(&t).is_err());
        // A version that cannot send 39 gets language only from post-login.
        assert!(conn.send_post_login_with(ClientVersion::V205, "English", Some(&t)).is_err());
    }

    /// 2.00: type 0, the key byte, `GNW24011`, then plain account and
    /// password (length-biased). Gen 3 after that, and no PLI 37/39.
    #[test]
    fn v200_login_line() {
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V200, 0x55, "acct", "pw").unwrap();
        let login = read_frame(&mut srv);
        let mut want = vec![0x20, 0x55 + 0x20];
        want.extend_from_slice(b"GNW24011");
        want.extend_from_slice(&[4 + 0x20]);
        want.extend_from_slice(b"acct");
        want.extend_from_slice(&[2 + 0x20]);
        want.extend_from_slice(b"pw\n");
        assert_eq!(login, want);
        // The key byte is `key & 0x7F` as a GCHAR.
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V200, 0xD5, "a", "b").unwrap();
        assert_eq!(read_frame(&mut srv)[1], 0x55 + 0x20);
        // 2.00 sends no language, so post-login sends nothing; 37 and 39 are refused.
        conn.send_post_login(ClientVersion::V200, "English").unwrap();
        assert!(conn.send_messages(&[lang_msg()]).is_err());
        let mut t39 = vec![39 + 0x20];
        t39.extend_from_slice(&[0x20; 10]);
        t39.push(b'\n');
        assert!(conn.send_messages(&[t39]).is_err());
        // Ids it can send go through (UPDATEFILE 34 is the newest).
        assert!(conn.send_messages(&[vec![34 + 0x20, b'\n']]).is_ok());
        assert!(conn.send_messages(&[vec![35 + 0x20, b'\n']]).is_err());
    }

    // ---- listserver reply and address fallback ----

    fn gs(s: &str) -> Vec<u8> {
        let mut v = vec![s.len() as u8 + 0x20];
        v.extend_from_slice(s.as_bytes());
        v
    }

    /// Body of an id 0 server list with one server, as the real listserver
    /// sends it: id, count, field count, then 8 gstrings.
    fn svrlist_body() -> Vec<u8> {
        let mut b = vec![0x20, 1 + 0x20, 8 + 0x20];
        for f in ["P Mock", "English", "desc", "http://x", "Version: 1", "3", "127.0.0.1", "14802"] {
            b.extend(gs(f));
        }
        b.push(b'\n');
        b
    }

    /// Body with id 2 status, id 3 site URL and id 5 upgrade URL.
    fn tail_body() -> Vec<u8> {
        b"\"Welcome, ab.\r2 online\n#https://site.example\n%https://upgrade.example\n".to_vec()
    }

    fn send_gen5(srv: &mut TcpStream, out: &mut Gen5, body: &[u8]) {
        let z = zlib_compress(body).unwrap();
        srv.write_all(&write_frame(Gen5Comp::Zlib, &out.encode(z, Gen5Comp::Zlib))).unwrap();
    }

    /// Accept a 2.22 listserver login, return the connection and the key
    /// the client picked (second byte of the plain id 7 frame).
    fn accept_listserver(listener: &TcpListener) -> (TcpStream, u8) {
        let (mut srv, _) = listener.accept().unwrap();
        let hs = read_frame(&mut srv);
        assert_eq!(hs[0], 7 + 0x20);
        let key = hs[1] - 0x20;
        let _login = read_raw(&mut srv);
        (srv, key)
    }

    /// Server list in frame one, status and URLs in frame two, and the
    /// connection stays open afterwards. The fetch must return all of it
    /// and must not wait for a close.
    #[test]
    fn fetch_server_list_reads_second_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut srv, key) = accept_listserver(&listener);
            let mut out = Gen5::new(key);
            send_gen5(&mut srv, &mut out, &svrlist_body());
            std::thread::sleep(Duration::from_millis(100));
            send_gen5(&mut srv, &mut out, &tail_body());
            // Keep the socket open until the client is done.
            let _ = done_rx.recv();
        });

        let t = Instant::now();
        let reply =
            Connection::fetch_server_list(&addr, ClientVersion::V222, "ab", "cd").unwrap();
        assert!(t.elapsed() < Duration::from_secs(3));
        assert_eq!(reply.servers.len(), 1);
        assert_eq!(reply.servers[0].name, "Mock");
        assert_eq!(reply.servers[0].port, 14802);
        assert_eq!(reply.status.as_deref(), Some("Welcome, ab.\n2 online"));
        assert_eq!(reply.site_url.as_deref(), Some("https://site.example"));
        assert_eq!(reply.upgrade_url.as_deref(), Some("https://upgrade.example"));
        assert!(reply.error.is_none());
        done_tx.send(()).unwrap();
        server.join().unwrap();
    }

    /// Only the server list ever arrives and the connection stays open:
    /// the fetch returns the servers after the short wait, with no status.
    #[test]
    fn fetch_server_list_does_not_hang_without_second_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut srv, key) = accept_listserver(&listener);
            let mut out = Gen5::new(key);
            send_gen5(&mut srv, &mut out, &svrlist_body());
            let _ = done_rx.recv();
        });

        let t = Instant::now();
        let reply =
            Connection::fetch_server_list(&addr, ClientVersion::V222, "ab", "cd").unwrap();
        let took = t.elapsed();
        assert!(took >= Duration::from_millis(1400), "returned too early: {took:?}");
        assert!(took < Duration::from_secs(4), "waited too long: {took:?}");
        assert_eq!(reply.servers.len(), 1);
        assert!(reply.status.is_none() && reply.site_url.is_none() && reply.upgrade_url.is_none());
        done_tx.send(()).unwrap();
        server.join().unwrap();
    }

    /// One frame with servers and status (the lab listserver) returns at
    /// once, without the tail wait.
    #[test]
    fn fetch_server_list_single_frame_returns_at_once() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut srv, key) = accept_listserver(&listener);
            let mut out = Gen5::new(key);
            let mut body = svrlist_body();
            body.extend(tail_body());
            send_gen5(&mut srv, &mut out, &body);
            let _ = done_rx.recv();
        });

        let t = Instant::now();
        let reply =
            Connection::fetch_server_list(&addr, ClientVersion::V222, "ab", "cd").unwrap();
        assert!(t.elapsed() < Duration::from_millis(400));
        assert_eq!(reply.servers.len(), 1);
        assert_eq!(reply.upgrade_url.as_deref(), Some("https://upgrade.example"));
        done_tx.send(()).unwrap();
        server.join().unwrap();
    }

    /// Plain zlib frame to the client, as the 2.05 to 2.17 listservers send.
    fn send_plain(srv: &mut TcpStream, body: &[u8]) {
        srv.write_all(&write_zlib_frame(body).unwrap()).unwrap();
    }

    /// Accept a pre-2.22 listserver login: ONE plain zlib frame holding id 0
    /// `newmain` and id 1 account and password. Returns the frame body.
    fn accept_plain_listserver(listener: &TcpListener) -> (TcpStream, Vec<u8>) {
        let (mut srv, _) = listener.accept().unwrap();
        let login = read_frame(&mut srv);
        (srv, login)
    }

    /// Shape S, the real 1.41 rev 1: the login sends nothing at all.
    #[test]
    fn shape_s_listserver_login_sends_nothing() {
        assert_eq!(ClientVersion::V141.listserver().shape, LoginShape::Silent);
        let (mut conn, mut srv) = pair();
        conn.login_listserver(ClientVersion::V141, 0x66, "ab", "cd").unwrap();
        drop(conn);
        let mut rest = Vec::new();
        srv.read_to_end(&mut rest).unwrap();
        assert!(rest.is_empty(), "1.41 sends nothing to the listserver");
    }

    /// 1.41 against a listserver that speaks first (the lab's): the list
    /// comes with nothing sent, and nothing is sent after it.
    #[test]
    fn fetch_server_list_141_listserver_speaks_first() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut srv, _) = listener.accept().unwrap();
            send_plain(&mut srv, &svrlist_body());
            send_plain(&mut srv, &tail_body());
            let _ = done_rx.recv();
            srv.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
            let mut b = [0u8; 1];
            assert!(srv.read(&mut b).map_or(true, |n| n == 0), "1.41 sent something");
        });
        let t = Instant::now();
        let reply = Connection::fetch_server_list(&addr, ClientVersion::V141, "ab", "cd").unwrap();
        assert!(t.elapsed() < Duration::from_secs(2));
        assert_eq!(reply.servers.len(), 1);
        done_tx.send(()).unwrap();
        server.join().unwrap();
    }

    /// 1.41 against a listserver that waits for a request: after
    /// `SILENT_LIST_WAIT` Seihai sends shape B and gets the list.
    #[test]
    fn fetch_server_list_141_falls_back_to_shape_b() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut srv, login) = accept_plain_listserver(&listener);
            assert_eq!(login, b" newmain\n!\"ab\"cd\n");
            send_plain(&mut srv, &svrlist_body());
            send_plain(&mut srv, &tail_body());
            let _ = done_rx.recv();
        });
        let t = Instant::now();
        let reply = Connection::fetch_server_list(&addr, ClientVersion::V141, "ab", "cd").unwrap();
        let took = t.elapsed();
        assert!(took >= crate::protocol::SILENT_LIST_WAIT && took < crate::protocol::SILENT_LIST_WAIT + Duration::from_secs(2), "{took:?}");
        assert_eq!(reply.servers.len(), 1);
        done_tx.send(()).unwrap();
        server.join().unwrap();
    }

    /// 2.17 lab flow: the login is one plain frame with id 0 and id 1, the
    /// reply is two plain frames (list, then ids 2, 3 and 5), and the
    /// connection stays open afterwards.
    #[test]
    fn fetch_server_list_217_two_plain_frames() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut srv, login) = accept_plain_listserver(&listener);
            // Id 0 `newmain`, then id 1 with the account and password,
            // both gstrings, in the same frame. No id 7, no key.
            assert_eq!(login, b" newmain\n!\"ab\"cd\n");
            send_plain(&mut srv, &svrlist_body());
            std::thread::sleep(Duration::from_millis(100));
            send_plain(&mut srv, &tail_body());
            let _ = done_rx.recv();
        });

        let t = Instant::now();
        let reply =
            Connection::fetch_server_list(&addr, ClientVersion::V217, "ab", "cd").unwrap();
        assert!(t.elapsed() < Duration::from_secs(3));
        assert_eq!(reply.servers.len(), 1);
        assert_eq!(reply.servers[0].name, "Mock");
        assert_eq!(reply.status.as_deref(), Some("Welcome, ab.\n2 online"));
        assert_eq!(reply.site_url.as_deref(), Some("https://site.example"));
        assert_eq!(reply.upgrade_url.as_deref(), Some("https://upgrade.example"));
        assert!(reply.error.is_none());
        done_tx.send(()).unwrap();
        server.join().unwrap();
    }

    /// The same plain flow when only the list frame arrives: the tail wait
    /// bound applies to plain frames too.
    #[test]
    fn fetch_server_list_217_without_second_frame_is_bounded() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut srv, _) = accept_plain_listserver(&listener);
            send_plain(&mut srv, &svrlist_body());
            let _ = done_rx.recv();
        });

        let t = Instant::now();
        let reply =
            Connection::fetch_server_list(&addr, ClientVersion::V217, "ab", "cd").unwrap();
        let took = t.elapsed();
        assert!(took >= Duration::from_millis(1400), "returned too early: {took:?}");
        assert!(took < Duration::from_secs(4), "waited too long: {took:?}");
        assert_eq!(reply.servers.len(), 1);
        assert!(reply.status.is_none());
        done_tx.send(()).unwrap();
        server.join().unwrap();
    }

    /// Body of an id 0 list with the given names, as the public service
    /// sends it (count byte, then per row a field count of 8 and 8 gstrings).
    fn named_list_body(names: &[&str]) -> Vec<u8> {
        let mut b = vec![0x20, names.len() as u8 + 0x20];
        for n in names {
            b.push(8 + 0x20);
            for f in [n, "English", "d", "http://x", "Version: 1", "0", "127.0.0.1", "14802"] {
                b.extend(gs(f));
            }
        }
        b.push(b'\n');
        b
    }

    /// The public 14923 shape seen on 2026-10-25 (item 14), 2.22-style gen-5
    /// login: one id 0 list with the old rows right after the handshake, a
    /// second id 0 list with the current rows about 350 ms later, then the
    /// status frame. The second list replaces the first (keeping both showed
    /// most servers twice in the launcher, 2026-10-31), and the status ends
    /// the wait.
    #[test]
    fn fetch_server_list_two_lists_then_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut srv, key) = accept_listserver(&listener);
            let mut out = Gen5::new(key);
            send_gen5(&mut srv, &mut out, &named_list_body(&["Old A", "Old B"]));
            std::thread::sleep(Duration::from_millis(350));
            send_gen5(&mut srv, &mut out, &named_list_body(&["P New A", "U New B", "H New C"]));
            send_gen5(&mut srv, &mut out, &tail_body());
            let _ = done_rx.recv();
        });
        let t = Instant::now();
        let reply =
            Connection::fetch_server_list(&addr, ClientVersion::V222, "ab", "cd").unwrap();
        assert!(t.elapsed() < Duration::from_millis(1200), "status should end the wait");
        let names: Vec<_> = reply.servers.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["New A", "New B", "New C"]);
        assert_eq!(reply.servers[0].kind, "P");
        assert!(reply.status.is_some());
        done_tx.send(()).unwrap();
        server.join().unwrap();
    }

    /// A plain zlib frame inside a gen-5 listserver reply (first byte 0x78,
    /// never a gen-5 comp byte) is read as plain and does not advance the
    /// cipher, so the gen-5 frames after it still decode. One 2.05 run
    /// against the public 14923 port failed here with "unknown gen-5
    /// compression byte 0x78"; the cause was not captured.
    #[test]
    fn fetch_server_list_accepts_a_plain_frame_on_a_gen5_link() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut srv, key) = accept_listserver(&listener);
            let mut out = Gen5::new(key);
            send_plain(&mut srv, &named_list_body(&["Old A"]));
            send_gen5(&mut srv, &mut out, &named_list_body(&["P New A"]));
            send_gen5(&mut srv, &mut out, &tail_body());
            let _ = done_rx.recv();
        });
        let reply =
            Connection::fetch_server_list(&addr, ClientVersion::V222, "ab", "cd").unwrap();
        let names: Vec<_> = reply.servers.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["New A"], "the later list replaces the earlier one");
        assert!(reply.status.is_some());
        done_tx.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn refused_14921_points_at_14923() {
        let refused = io::Error::from(io::ErrorKind::ConnectionRefused);
        assert!(list_failure_text("example.org:14921", &refused).contains("14923"));
        assert!(!list_failure_text("example.org:14922", &refused).contains("14923"));
        let other = io::Error::from(io::ErrorKind::TimedOut);
        assert!(!list_failure_text("example.org:14921", &other).contains("14923"));
    }

    /// The strict reader still refuses a plain frame on a gen-5 link (game
    /// servers never send one).
    #[test]
    fn strict_gen5_reader_refuses_plain_frame() {
        let frame = write_zlib_frame(b"x\n").unwrap();
        let err = crate::protocol::read_frame(&mut frame.as_slice(), &mut Gen5::new(1)).unwrap_err();
        assert!(err.to_string().contains("0x78"));
    }

    /// A read timeout before any reply reads as "no reply", not os error 11.
    #[test]
    fn silent_listserver_error_says_so() {
        let e = list_read_error(io::Error::from(io::ErrorKind::WouldBlock));
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        assert!(e.to_string().contains("no reply"), "{e}");
        let other = list_read_error(io::Error::new(io::ErrorKind::InvalidData, "bad"));
        assert_eq!(other.to_string(), "bad");
    }

    /// 2.05 uses the same plain flow, and a listserver error text ends the
    /// reply without waiting for a second frame.
    #[test]
    fn fetch_server_list_205_error_ends_reply() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut srv, login) = accept_plain_listserver(&listener);
            assert_eq!(login, b" newmain\n", "2.05 sends shape A: id 0 only");
            send_plain(&mut srv, b"$Bad account or password.\n");
            let _ = done_rx.recv();
        });

        let t = Instant::now();
        let reply =
            Connection::fetch_server_list(&addr, ClientVersion::V205, "ab", "cd").unwrap();
        assert!(t.elapsed() < Duration::from_millis(400));
        assert!(reply.servers.is_empty());
        assert_eq!(reply.error.as_deref(), Some("Bad account or password."));
        done_tx.send(()).unwrap();
        server.join().unwrap();
    }

    /// The list is fetched on the port of the login the profile sends:
    /// 14923 for the classic login (1.41 to 2.17), 14922 for the gen-5 login
    /// (2.22 and later), and never the dead 14921. Versions with no
    /// listserver (1.33 to 1.39) borrow 2.05's classic login.
    #[test]
    fn listserver_port_follows_the_profile() {
        assert_eq!(ClientVersion::V205.listserver_port(), 14923);
        assert_eq!(ClientVersion::V217.listserver_port(), 14923);
        assert_eq!(ClientVersion::V222.listserver_port(), 14922);
        assert_eq!(ClientVersion::V231.listserver_port(), 14922);
        assert_eq!(ClientVersion::V141.listserver_port(), 14923);
        assert_eq!(ClientVersion::V139.listserver_port(), 14923);
    }

    /// The builds from 1.41 to 2.05 default to the classic-login port,
    /// whatever port the binary carries.
    #[test]
    fn legacy_versions_default_to_the_legacy_port() {
        for v in [
            ClientVersion::V141,
            ClientVersion::V142,
            ClientVersion::V200,
            ClientVersion::V201,
            ClientVersion::V202,
            ClientVersion::V203,
            ClientVersion::V204,
            ClientVersion::V205,
            ClientVersion::V210,
            ClientVersion::V217,
        ] {
            assert_eq!(v.listserver_port(), 14923, "{v:?}");
        }
    }

    /// A dead first address must not stop a working second one.
    #[test]
    fn connect_any_falls_back_to_next_address() {
        let live = TcpListener::bind("127.0.0.1:0").unwrap();
        // Bind and drop to get a local port with nothing listening.
        let dead = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        let addrs = [dead, live.local_addr().unwrap()];
        let s = connect_any(&addrs, Duration::from_secs(2)).unwrap();
        assert_eq!(s.peer_addr().unwrap(), live.local_addr().unwrap());
        let (_srv, _) = live.accept().unwrap();

        assert!(connect_any(&[dead], Duration::from_secs(2)).is_err());
        assert_eq!(
            connect_any(&[], Duration::from_secs(1)).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    /// Read the next gen-5 frame a 2.31 login's client sent.
    fn gen5_body(srv: &mut TcpStream, g: &mut Gen5) -> Vec<u8> {
        let raw = read_raw(srv);
        crate::protocol::read_frame(&mut [&(raw.len() as u16).to_be_bytes()[..], &raw].concat().as_slice(), g).unwrap()
    }

    const ENTER: [u8; 3] = [46 + 0x20, 1 + 0x20, b'\n'];

    /// The send sets of 2.3, 2.31, 3.153 and the 4.x to 6.x builds have 46; no
    /// other profile does.
    #[test]
    fn enter_level_follows_the_profile_send_set() {
        assert_eq!(enter_level_message(), ENTER);
        for p in crate::protocol::PROFILES {
            assert_eq!(p.version.sends_enter_level(), p.protocol.pli.contains(46), "{}", p.label);
        }
        let with: Vec<&str> = crate::protocol::PROFILES
            .iter()
            .filter(|p| p.version.sends_enter_level())
            .map(|p| p.label)
            .collect();
        assert_eq!(with, ["2.3", "2.31", "3.153", "4.04r2", "4.20r6", "4.20r8", "5.00r7", "5.12", "5.24r4", "5.30r3", "5.31r4", "6.00r2", "6.015", "6.034", "6.03r7", "6.03r7 Linux"]);
    }

    /// 46 goes first in the frame, then the props message that follows it.
    #[test]
    fn enter_level_comes_before_the_props_frame() {
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V231, 0x10, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let mut g = Gen5::new(0x10);
        // A props message with a cell prop (43) and a wrapped x2, as a gmap
        // crossing sends; Seihai has none, so the test only fixes the order.
        let props = vec![2 + 0x20, 43 + 0x20, 1 + 0x20, 78 + 0x20, 0x20, 0x20, b'\n'];
        assert!(conn.send_enter_level(ClientVersion::V231, &[props.clone()]).unwrap());
        let body = gen5_body(&mut srv, &mut g);
        assert_eq!([&ENTER[..], &props[..]].concat(), body);
        assert_eq!(body[0], 46 + 0x20);
        // Alone, 46 is its own 3-byte frame and a walk update is a later
        // frame, so 46 is before the props frame in the byte stream too.
        assert!(conn.send_enter_level(ClientVersion::V231, &[]).unwrap());
        conn.send_position(10.0, 20.0, 1).unwrap();
        assert_eq!(gen5_body(&mut srv, &mut g), ENTER);
        assert_eq!(gen5_body(&mut srv, &mut g)[0], 2 + 0x20);
    }

    /// Login sends 46; the first level after it does not; each later level
    /// change does; the same level again does not.
    #[test]
    fn enter_level_once_per_level_entry() {
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V231, 0x22, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let mut g = Gen5::new(0x22);
        let mut el = EnterLevelSender::for_version(ClientVersion::V231);
        assert!(el.at_login(&mut conn).unwrap());
        assert_eq!(gen5_body(&mut srv, &mut g), ENTER);
        assert!(!el.level_entered(&mut conn, "a.nw").unwrap());
        assert!(!el.level_entered(&mut conn, "a.nw").unwrap());
        assert_eq!(el.sent, 1);
        for (i, name) in ["b.nw", "a.nw", "b.nw"].iter().enumerate() {
            assert!(el.level_entered(&mut conn, name).unwrap());
            assert!(!el.level_entered(&mut conn, name).unwrap(), "repeat of {name}");
            assert_eq!(gen5_body(&mut srv, &mut g), ENTER);
            assert_eq!(el.sent, 2 + i);
        }
        // The tamper check does not ride along: 2.31 does not re-send it
        // on a level change.
        let mut t = TamperSender::for_version(ClientVersion::V231).unwrap();
        assert!(!t.level_changed());
    }

    /// The login 46 can be held back for the post-login frame: it is handed
    /// over once, and a profile without 46 hands over nothing.
    #[test]
    fn the_login_46_is_owed_until_the_post_login_frame_takes_it() {
        let mut el = EnterLevelSender::for_version(ClientVersion::V231);
        assert!(el.take_login().is_empty(), "nothing owed yet");
        el.owe_login();
        assert!(el.login_owed());
        assert_eq!(el.take_login(), vec![enter_level_message()]);
        assert!(!el.login_owed());
        assert!(el.take_login().is_empty(), "once");
        assert_eq!(el.sent, 1);
        let mut old = EnterLevelSender::for_version(ClientVersion::V222);
        old.owe_login();
        assert!(old.take_login().is_empty(), "2.22 has no 46");
        assert_eq!(old.sent, 0);
    }

    /// A profile without 46 sends nothing at login or on a level change,
    /// and the connection's own gate refuses a hand-built 46.
    #[test]
    fn enter_level_not_sent_without_46() {
        for v in [ClientVersion::V205, ClientVersion::V217, ClientVersion::V222] {
            let (mut conn, mut srv) = pair();
            conn.login_versioned(v, 0x31, "ab", "cd").unwrap();
            let _ = read_frame(&mut srv);
            let mut el = EnterLevelSender::for_version(v);
            assert!(!el.at_login(&mut conn).unwrap(), "{}", v.label());
            assert!(!el.level_entered(&mut conn, "a.nw").unwrap());
            assert!(!el.level_entered(&mut conn, "b.nw").unwrap());
            assert!(!conn.send_enter_level(v, &[position_message(1.0, 2.0, 0)]).unwrap());
            assert_eq!(el.sent, 0);
            assert!(conn.send_messages(&[enter_level_message()]).is_err());
            // Nothing was written: the next frame is the walk we send now.
            conn.send_position(3.0, 4.0, 2).unwrap();
            srv.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let body = if v == ClientVersion::V217 {
                gen3_messages(0x31, &read_frame(&mut srv)).remove(0)
            } else if v == ClientVersion::V205 {
                gen3_messages(0x31, &read_frame(&mut srv)).remove(0)
            } else {
                let mut g = Gen5::new(0x31);
                gen5_body(&mut srv, &mut g)
            };
            assert_eq!(body[0], 2 + 0x20, "{}", v.label());
        }
    }

    // --- gmap crossings ---

    fn east_crossing() -> crate::gmap::Crossing {
        crate::gmap::Crossing { from: (0, 0), to: (1, 0), local_x: 0.0, local_y: 30.5 }
    }

    /// The frame a 2.31 client sends at a crossing is one gen-5 frame: 46,
    /// then the props with the cell prop and the wrapped X2. The server's
    /// answer to the new level then owes no second 46.
    #[test]
    fn crossing_frame_on_the_wire_231() {
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V231, 0x44, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let mut g = Gen5::new(0x44);
        let mut el = EnterLevelSender::for_version(ClientVersion::V231);
        assert!(el.at_login(&mut conn).unwrap());
        assert_eq!(gen5_body(&mut srv, &mut g), ENTER);
        el.note_level("gm_nw.nw");
        let mut t = TamperSender::for_version(ClientVersion::V231);
        let sent = conn.send_gmap_crossing(ClientVersion::V231, &east_crossing(), t.as_mut()).unwrap();
        assert!(sent);
        el.crossing("gm_ne.nw", sent);
        let body = gen5_body(&mut srv, &mut g);
        let props = vec![2 + 0x20, 43 + 0x20, 1 + 0x20, 78 + 0x20, 0x20, 0x20, b'\n'];
        assert_eq!(body, [&ENTER[..], &props[..]].concat());
        // The server's answer names the new level: nothing more is owed.
        assert!(!el.note_level("gm_ne.nw"));
        assert!(!el.is_pending());
        assert_eq!(el.sent, 2);
    }

    /// 2.22 has no 46; with the tamper check opted in it goes first, and
    /// without it the frame is the props alone.
    #[test]
    fn crossing_frame_on_the_wire_222() {
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V222, 0x45, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let mut g = Gen5::new(0x45);
        let mut t = TamperSender::for_version(ClientVersion::V222);
        assert!(!conn.send_gmap_crossing(ClientVersion::V222, &east_crossing(), t.as_mut()).unwrap());
        let body = gen5_body(&mut srv, &mut g);
        assert_eq!(body[0], 39 + 0x20);
        assert_eq!(body[12], 2 + 0x20, "props follow the 12-byte tamper message");
        assert_eq!(&body[13..16], &[15 + 0x20, 0x20, 43 + 0x20], "X first, then the cell (real 2.22)");
        assert!(!conn.send_gmap_crossing(ClientVersion::V222, &east_crossing(), None).unwrap());
        let body = gen5_body(&mut srv, &mut g);
        assert_eq!(body, [&[2 + 0x20, 15 + 0x20, 0x20, 43 + 0x20, 1 + 0x20][..], b"\n"].concat());
    }

    /// 2.22 on its walk cadence, the lab's east crossing between two send
    /// slots: the crossing step's tick frame carries the 39 and the `gr.x`
    /// flag; the next slot's frame the props `X=0.5, GMAPLEVELX=1`.
    #[test]
    fn crossing_on_the_222_cadence_splits_the_39_from_the_cell_prop() {
        use crate::protocol::walkcadence::{WalkCadence, WalkState};
        let v = ClientVersion::V222;
        let (mut conn, mut srv) = pair();
        conn.login_versioned(v, 0x47, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let mut g = Gen5::new(0x47);
        let mut t = TamperSender::for_version(v);
        let mut wc = WalkCadence::for_version(v).unwrap();
        let st = |x: f32, gani| WalkState::new(x, 30.0, 3, Some(gani));
        for _ in 0..3 {
            assert!(wc.tick(st(63.0, "idle")).is_none());
        }
        // A slot on 63.5 ...
        conn.send_tick_frame(Vec::new(), Vec::new(), wc.tick(st(63.5, "walk"))).unwrap();
        let _ = gen5_body(&mut srv, &mut g);
        // ... then the crossing step, between two slots.
        let lead = crate::gmap::crossing_lead(v, t.as_mut());
        wc.cross(&east_crossing());
        conn.queue_flag("gr.x", Some("64"));
        conn.send_tick_frame(lead, Vec::new(), wc.tick(st(0.0, "walk"))).unwrap();
        let body = gen5_body(&mut srv, &mut g);
        assert_eq!(body[0], 39 + 0x20, "the 39 opens the crossing step's frame");
        assert_eq!(&body[12..], b"\x32gr.x=64\n", "then the flag; no props yet");
        // The next slot: X = 0.5 (half tiles: 1), then the cell.
        conn.send_tick_frame(Vec::new(), Vec::new(), wc.tick(st(0.5, "walk"))).unwrap();
        assert_eq!(gen5_body(&mut srv, &mut g), [2 + 0x20, 15 + 0x20, 1 + 0x20, 43 + 0x20, 1 + 0x20, b'\n']);
    }

    /// A profile without gmaps writes nothing at a crossing.
    #[test]
    fn crossing_frame_is_not_sent_without_gmaps() {
        let (mut conn, _srv) = pair();
        conn.login_versioned(ClientVersion::V205, 0x46, "ab", "cd").unwrap();
        assert!(!conn.send_gmap_crossing(ClientVersion::V205, &east_crossing(), None).unwrap());
    }

    /// The whole path for a 2.31 client on a 2x2 gmap, in process: login
    /// and the server's gmap messages, the world sync, a walk east across
    /// the edge and back, and the frames that went out. The server's answers
    /// are the lab's (name, mod time, board, gmap name).
    #[test]
    fn gmap_walk_east_and_back_231_frames() {
        use crate::assets::Level;
        use crate::gmap::{sync_world, GmapRules, WorldLoader};
        use crate::offline::OfflineSession;
        use std::collections::{HashMap, HashSet};

        struct Files(HashMap<String, Level>);
        impl WorldLoader for Files {
            fn gmap_text(&self, _: &str) -> Option<String> {
                Some("GRMAP001\nWIDTH 2\nHEIGHT 2\nLEVELNAMES\n\"gm_nw.nw\",\"gm_ne.nw\",\n\"gm_sw.nw\",\"gm_se.nw\",\nLEVELNAMESEND\n".into())
            }
            fn level(&self, name: &str, _: Option<u32>) -> Option<Level> {
                self.0.get(name).cloned()
            }
        }
        let v = ClientVersion::V231;
        let (mut conn, mut srv) = pair();
        conn.login_versioned(v, 0x61, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let mut g = Gen5::new(0x61);
        let mut el = EnterLevelSender::for_version(v);
        assert!(el.at_login(&mut conn).unwrap());
        assert_eq!(gen5_body(&mut srv, &mut g), ENTER);

        // The server: 49 (cell 0,0), the level, then the gmap name.
        let msg = |id: u8, p: &[u8]| [&[id + 0x20][..], p, b"\n"].concat();
        let warp = [&[0x20 + 116, 0x20 + 61, 0x20 + 50, 0x20, 0x20][..], b"gm2x2.gmap"].concat();
        let mut net = SessionState::for_version(v);
        net.apply_body(&[msg(49, &warp), msg(6, b"gm_nw.nw"), msg(6, b"gm2x2.gmap")].concat());
        assert!(!el.note_level("gm_nw.nw"), "the first level is covered by the login 46");

        let open = |mark: u16| Level::from_board(&vec![mark; 64 * 64]);
        let files = Files(
            ["gm_nw.nw", "gm_ne.nw", "gm_sw.nw", "gm_se.nw"].iter().map(|n| (n.to_string(), open(0))).collect(),
        );
        let mut session = OfflineSession::new();
        session.current_level = files.level("gm_nw.nw", None);
        session.player_x = 60.0;
        session.player_y = 10.0;
        let rules = GmapRules::for_version(v);
        let mut asked = HashSet::new();
        let req = sync_world(&mut session, &mut net, &rules, &files, &mut asked);
        // The three neighbours are asked for in one frame, as 35.
        assert_eq!(req.messages.len(), 3);
        conn.send_messages(&req.messages).unwrap();
        let ask = gen5_body(&mut srv, &mut g);
        assert_eq!(ask.iter().filter(|&&b| b == 35 + 0x20).count(), 3);

        // East: eight half-steps from 60.0; the last one crosses.
        let mut crossed = None;
        for _ in 0..8 {
            session.try_move(0.5, 0.0).unwrap();
            if let Some(c) = session.take_gmap_crossing() {
                net.set_gmap_cell(c.to);
                let sent = conn.send_gmap_crossing(v, &c, None).unwrap();
                el.crossing(&net.current_level_name().unwrap(), sent);
                crossed = Some(c);
            }
        }
        assert_eq!(crossed.map(|c| c.to), Some((1, 0)));
        let east = gen5_body(&mut srv, &mut g);
        assert_eq!(east, [&ENTER[..], &[2 + 0x20, 43 + 0x20, 1 + 0x20, 78 + 0x20, 0x20, 0x20, b'\n'][..]].concat());
        // The server answers: new level, mod time, empty board, gmap name.
        net.apply_body(&[msg(6, b"gm_ne.nw"), msg(39, &[0x20; 5]), msg(0, b""), msg(6, b"gm2x2.gmap")].concat());
        assert_eq!(net.level_name.as_deref(), Some("gm_ne.nw"));
        assert!(!el.note_level("gm_ne.nw") && !el.is_pending(), "no second 46 for the answer");
        sync_world(&mut session, &mut net, &rules, &files, &mut asked);
        assert_eq!(session.gmap.as_ref().unwrap().cell, (1, 0));

        // And back west: one half-step from local 0.0.
        let c = {
            session.try_move(-0.5, 0.0).unwrap();
            session.take_gmap_crossing().unwrap()
        };
        net.set_gmap_cell(c.to);
        let sent = conn.send_gmap_crossing(v, &c, None).unwrap();
        el.crossing(&net.current_level_name().unwrap(), sent);
        let west = gen5_body(&mut srv, &mut g);
        // Local x is 63.5: X2 = (63.5 * 16) << 1 = 2032 = GSHORT 0x2f 0x90.
        assert_eq!(west, [&ENTER[..], &[2 + 0x20, 43 + 0x20, 0x20, 78 + 0x20, 0x2f, 0x90, b'\n'][..]].concat());
        assert_eq!(el.sent, 3, "login plus two crossings");
        assert_eq!(net.current_level_name().as_deref(), Some("gm_nw.nw"));
    }

    // --- Lab batch c wiring (2026-10-24) ---

    /// 10 tiles is 20 half-tiles, 20 tiles is 40, then the level name.
    fn warp_msg() -> Vec<u8> {
        [&[0x20, 20 + 0x20, 40 + 0x20][..], b"lv_c.nw\n"].concat()
    }

    #[test]
    fn board_modify_is_pli_1_with_gchar_header_and_gshort_tiles() {
        // The lab capture's cut bush: x 39, y 14, 2 x 2, tiles 1834 1835 1850 1851.
        let m = board_modify_message(39, 14, 2, 2, &[1834, 1835, 1850, 1851]);
        let mut want = vec![1 + 0x20, 39 + 0x20, 14 + 0x20, 2 + 0x20, 2 + 0x20];
        for t in [1834u16, 1835, 1850, 1851] {
            want.extend_from_slice(&gshort(t));
        }
        want.push(b'\n');
        assert_eq!(m, want);
    }

    #[test]
    fn trigger_action_is_pli_38_with_no_npc_and_half_tiles() {
        assert_eq!(trigger_action_message(21.5, 9.0, "pushed", "0"), [&[38 + 0x20, 0x20, 0x20, 0x20, 43 + 0x20, 18 + 0x20][..], b"pushed,0\n"].concat());
        assert_eq!(trigger_action_message(1.0, 1.0, "x", ""), [&[38 + 0x20, 0x20, 0x20, 0x20, 2 + 0x20, 2 + 0x20][..], b"x\n"].concat());
    }

    #[test]
    fn throw_carried_is_pli_11_with_no_payload() {
        assert_eq!(throw_carried_message(), vec![11 + 0x20, b'\n']);
    }

    #[test]
    fn levelwarp_message_layout() {
        assert_eq!(levelwarp_message(10.0, 20.0, "lv_c.nw"), warp_msg());
        // Clamped to a GCHAR: 63.5 tiles is the last half-tile that fits 127.
        assert_eq!(levelwarp_message(99.0, 0.0, "a")[1], 127 + 0x20);
    }

    /// 2.22: the tamper check rides in the frame of every client-initiated
    /// link request, right after it. 2.31 and 2.17 do not send it on a link,
    /// and nothing is added without a sender (the default, off).
    #[test]
    fn link_request_carries_tamper_check_by_profile() {
        // 2.22: 0 then 39, one frame.
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V222, 0x15, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let mut g = Gen5::new(0x15);
        let mut t = TamperSender::for_version(ClientVersion::V222).unwrap();
        assert!(conn.send_link_request(10.0, 20.0, "lv_c.nw", Some(&mut t)).unwrap());
        let body = gen5_body(&mut srv, &mut g);
        assert_eq!(body, [warp_msg(), t.message().unwrap()].concat());
        // Off: the warp alone.
        assert!(!conn.send_link_request(10.0, 20.0, "lv_c.nw", None).unwrap());
        assert_eq!(gen5_body(&mut srv, &mut g), warp_msg());

        // 2.31: its 39 is periodic, never on a link.
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V231, 0x16, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let mut g = Gen5::new(0x16);
        let mut t = TamperSender::for_version(ClientVersion::V231).unwrap();
        assert!(!conn.send_link_request(10.0, 20.0, "lv_c.nw", Some(&mut t)).unwrap());
        assert_eq!(gen5_body(&mut srv, &mut g), warp_msg());

        // 2.17: sent at login only.
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V217, 0x17, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let mut t = TamperSender::for_version(ClientVersion::V217).unwrap();
        assert!(!conn.send_link_request(10.0, 20.0, "lv_c.nw", Some(&mut t)).unwrap());
        assert_eq!(gen3_messages(0x17, &read_frame(&mut srv)), vec![warp_msg()]);
    }

    /// `PLI_LEVELWARPMOD`: the `PLI_LEVELWARP` layout with a GINT5 mod time
    /// between the id and the position.
    #[test]
    fn levelwarpmod_message_layout() {
        // 1_781_284_436 = 0x6A_2A_... as five 7-bit groups, high first.
        let t = 1_781_284_436u32;
        let m = levelwarpmod_message(t, 10.0, 20.0, "lv_c.nw");
        assert_eq!(m[0], 30 + 0x20);
        let groups: Vec<u8> = m[1..6].iter().map(|b| b - 0x20).collect();
        let back = groups.iter().fold(0u64, |a, g| (a << 7) | u64::from(*g));
        assert_eq!(back, u64::from(t));
        assert_eq!(&m[6..], &warp_msg()[1..]);
        assert_eq!(levelwarpmod_message(0, 10.0, 20.0, "lv_c.nw")[1..6], [0x20; 5]);
    }

    /// The frame of a level link, per profile, level cached or not.
    /// Lab (real clients, 2026-10-24): 2.31 sends 30, 46, props before the
    /// server answers; 2.22 sends 30, 39 and never 46; the uncached form
    /// of both is 0 (and 39 on 2.22) with no 46 in the frame.
    #[test]
    fn link_frame_by_profile_cached_and_not() {
        let t = 1_781_284_436u32;
        // 2.31 has the precise positions: its props here are X2/Y2.
        let props = position_message_precise(10.0, 20.0, 2);
        let cached_msg = levelwarpmod_message(t, 10.0, 20.0, "lv_c.nw");

        // 2.31: uncached = 0 alone; cached = 30, 46, props. No tamper check.
        let mut tamper = TamperSender::for_version(ClientVersion::V231).unwrap();
        let f = link_request_frame(ClientVersion::V231, None, 10.0, 20.0, 2, "lv_c.nw", Some(&mut tamper));
        assert_eq!(f.messages, vec![warp_msg()]);
        assert!(!f.with_enter_level && !f.with_tamper);
        let f = link_request_frame(ClientVersion::V231, Some(t), 10.0, 20.0, 2, "lv_c.nw", Some(&mut tamper));
        assert_eq!(f.messages, vec![cached_msg.clone(), ENTER.to_vec(), props.clone()]);
        assert!(f.with_enter_level && !f.with_tamper);

        // 2.22: uncached = 0, 39; cached = 30, 39. Never 46, no props.
        let mut tamper = TamperSender::for_version(ClientVersion::V222).unwrap();
        let t39 = tamper.message().unwrap();
        let f = link_request_frame(ClientVersion::V222, None, 10.0, 20.0, 2, "lv_c.nw", Some(&mut tamper));
        assert_eq!(f.messages, vec![warp_msg(), t39.clone()]);
        let f = link_request_frame(ClientVersion::V222, Some(t), 10.0, 20.0, 2, "lv_c.nw", Some(&mut tamper));
        assert_eq!(f.messages, vec![cached_msg.clone(), t39.clone()]);
        assert!(f.with_tamper && !f.with_enter_level);
        // Tamper check off (the default): the request alone.
        let f = link_request_frame(ClientVersion::V222, Some(t), 10.0, 20.0, 2, "lv_c.nw", None);
        assert_eq!(f.messages, vec![cached_msg.clone()]);

        // 2.05: cached form, nothing else.
        let f = link_request_frame(ClientVersion::V205, Some(t), 10.0, 20.0, 2, "lv_c.nw", None);
        assert_eq!(f.messages, vec![cached_msg.clone()]);

        // 2.3 follows 2.31.
        let f = link_request_frame(ClientVersion::V230, Some(t), 10.0, 20.0, 2, "lv_c.nw", None);
        assert_eq!(f.messages, vec![cached_msg.clone(), ENTER.to_vec(), props.clone()]);

        // Builds without cached links ask with 0 whatever the cache holds.
        for v in [ClientVersion::V133, ClientVersion::V139, ClientVersion::V200, ClientVersion::V204] {
            assert!(!v.sends_cached_links());
            let f = link_request_frame(v, Some(t), 10.0, 20.0, 2, "lv_c.nw", None);
            assert_eq!(f.messages, vec![warp_msg()], "{}", v.label());
            assert!(!f.with_enter_level);
        }
        // Every profile: the first message is 0 or 30 by profile and cache,
        // and a 46 is in the frame only for the cached form with 46 in the
        // send set.
        for p in crate::protocol::PROFILES {
            for cached in [None, Some(t)] {
                let f = link_request_frame(p.version, cached, 10.0, 20.0, 2, "lv_c.nw", None);
                let want_mod = cached.is_some() && p.levels.cached_links;
                assert_eq!(f.messages[0][0], if want_mod { 30 } else { 0 } + 0x20, "{}", p.label);
                assert_eq!(f.with_enter_level, want_mod && p.protocol.pli.contains(46), "{}", p.label);
                assert_eq!(f.messages.iter().any(|m| m == &ENTER), f.with_enter_level, "{}", p.label);
            }
        }
    }

    /// Over the wire: one frame, in order, 30 then 46 then the props for 2.31.
    /// The 46 is then not owed again when the server names the level.
    #[test]
    fn cached_link_frame_goes_out_in_one_frame_and_owes_no_second_46() {
        let t = 1_781_284_436u32;
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V231, 0x41, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let mut g = Gen5::new(0x41);
        let mut el = EnterLevelSender::for_version(ClientVersion::V231);
        assert!(!el.note_level("a.nw"));
        let f = conn.send_link(ClientVersion::V231, Some(t), 10.0, 20.0, 2, "lv_c.nw", None).unwrap();
        assert!(f.with_enter_level);
        el.cached_link("lv_c.nw", f.with_enter_level);
        let body = gen5_body(&mut srv, &mut g);
        assert_eq!(
            body,
            [levelwarpmod_message(t, 10.0, 20.0, "lv_c.nw"), ENTER.to_vec(), position_message_precise(10.0, 20.0, 2)].concat()
        );
        // The server's answer names the level: no 46 is owed.
        assert!(!el.note_level("lv_c.nw") && !el.is_pending());
        assert_eq!(el.sent, 1);
        // Uncached link: 0 alone, and the 46 is owed after the answer.
        let f = conn.send_link(ClientVersion::V231, None, 10.0, 20.0, 2, "lv_b.nw", None).unwrap();
        assert!(!f.with_enter_level);
        assert_eq!(gen5_body(&mut srv, &mut g), levelwarp_message(10.0, 20.0, "lv_b.nw"));
        assert!(el.note_level("lv_b.nw") && el.is_pending());
    }

    /// Own link: 46 then the props (position) in one frame, 46 first. A
    /// server-side warp: 46 alone. The level entry is owed once.
    #[test]
    fn enter_level_ordering_after_the_servers_answer() {
        let at = PlayerUpdate { x: 10.5, y: 3.0, dir: 2 };
        assert_eq!(enter_level_extras(ClientVersion::V231, true, &at), vec![position_message_precise(10.5, 3.0, 2)]);
        assert_eq!(enter_level_extras(ClientVersion::V222, true, &at), vec![position_message(10.5, 3.0, 2)]);
        assert!(enter_level_extras(ClientVersion::V231, false, &at).is_empty());

        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V231, 0x23, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let mut g = Gen5::new(0x23);
        let mut el = EnterLevelSender::for_version(ClientVersion::V231);
        // Nothing owed for the first level after login, nor for a repeat.
        assert!(!el.note_level("a.nw"));
        assert!(!el.is_pending());
        assert!(!el.send_pending(&mut conn, &[]).unwrap());
        assert!(!el.note_level("a.nw"));
        // Own link to b: 46 + props, one frame, 46 first.
        assert!(el.note_level("b.nw"));
        assert!(el.is_pending());
        assert!(el.send_pending(&mut conn, &enter_level_extras(ClientVersion::V231, true, &at)).unwrap());
        let body = gen5_body(&mut srv, &mut g);
        assert_eq!(body, [&ENTER[..], &position_message_precise(10.5, 3.0, 2)[..]].concat());
        // Owed once: a second flush sends nothing.
        assert!(!el.is_pending());
        assert!(!el.send_pending(&mut conn, &[]).unwrap());
        // Server warp to a: 46 alone.
        assert!(el.note_level("a.nw"));
        assert!(el.send_pending(&mut conn, &enter_level_extras(ClientVersion::V231, false, &at)).unwrap());
        assert_eq!(gen5_body(&mut srv, &mut g), ENTER);
        assert_eq!(el.sent, 2);
    }

    /// A profile without 46: the debt is cleared and nothing is written.
    #[test]
    fn enter_level_pending_is_cleared_without_46() {
        let (mut conn, mut srv) = pair();
        conn.login_versioned(ClientVersion::V222, 0x24, "ab", "cd").unwrap();
        let _ = read_frame(&mut srv);
        let mut el = EnterLevelSender::for_version(ClientVersion::V222);
        assert!(!el.note_level("a.nw"));
        assert!(el.note_level("b.nw"));
        assert!(!el.send_pending(&mut conn, &[position_message(1.0, 2.0, 0)]).unwrap());
        assert!(!el.is_pending());
        assert_eq!(el.sent, 0);
    }

    /// What a gserver sees from a login: the plain login frame, then the
    /// gen-5 post-login frame; the 46 (2.31) is owed to the post-login frame.
    #[test]
    fn login_gserver_sends_login_post_login_and_46() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let s = login_gserver(&addr, ClientVersion::V231, 0x31, "ab", "cd", true).unwrap();
        assert!(s.warnings.is_empty() && !s.tamper_unavailable);
        assert!(s.tamper.is_some());
        let (mut srv, _) = listener.accept().unwrap();
        let login = read_frame(&mut srv);
        assert_eq!(&login[..2], &[0x20 + 5, 0x31 + 0x20]);
        assert_eq!(&login[2..10], b"GNW28015");
        assert_eq!(&login[10..], b"\"ab\"cd\n");
        let mut g = Gen5::new(0x31);
        let post = gen5_body(&mut srv, &mut g);
        assert_eq!(post[0], 37 + 0x20);
        assert!(post.len() > lang_msg().len(), "tamper check follows the language");
        assert!(s.enter_level.login_owed(), "the 46 waits for the post-login frame (2.31)");

        // Without the flag: language only, and no tamper sender.
        let s = login_gserver(&addr, ClientVersion::V231, 0x32, "ab", "cd", false).unwrap();
        assert!(s.tamper.is_none() && !s.tamper_unavailable);
        let (mut srv, _) = listener.accept().unwrap();
        let _ = read_frame(&mut srv);
        let mut g = Gen5::new(0x32);
        assert_eq!(gen5_body(&mut srv, &mut g), lang_msg());
        assert!(s.enter_level.login_owed());

        // A build with no tamper constant (2.3) says so.
        let s = login_gserver(&addr, ClientVersion::V230, 0x33, "ab", "cd", true).unwrap();
        assert!(s.tamper.is_none() && s.tamper_unavailable);
    }

    /// The real 4.04r2 (lab, 2026-10-31): the language and the update-package
    /// requests in the frame after the login line, `bigmap.txt` in the first
    /// session only; the login 46 is held for the after-burst frames.
    #[test]
    fn login_gserver_404_asks_its_base_files() {
        for (first, want) in [
            (true, &b"EEnglish\n7basepackage.gupd\n7basepackage.gupd\n7bigmap.txt\n"[..]),
            (false, &b"EEnglish\n7basepackage.gupd\n7basepackage.gupd\n"[..]),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap().to_string();
            let s = login_gserver_with(&addr, ClientVersion::V404R2, 0x31, "ab", "cd", false, first, wantfile_message).unwrap();
            assert!(s.warnings.is_empty());
            assert!(s.enter_level.login_owed());
            let (mut srv, _) = listener.accept().unwrap();
            assert!(read_frame(&mut srv).starts_with(b"%QG3D17026"));
            assert_eq!(gen5_body(&mut srv, &mut Gen5::new(0x31)), want);
        }
    }

    /// The real 2.17.1 (lab, 2026-10-30): the login line, the language and
    /// the tamper check in one frame, the last two with their gen-3
    /// insertion from the first iterator step; the next frame carries on
    /// from the third step.
    #[test]
    fn login_gserver_217_bundles_language_and_tampercheck_with_the_login() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let mut s = login_gserver(&addr, ClientVersion::V217, 0x31, "ab", "cd", true).unwrap();
        assert!(s.warnings.is_empty() && s.tamper.is_some());
        let (mut srv, _) = listener.accept().unwrap();
        let frame = read_frame(&mut srv);
        let at = frame.iter().position(|&b| b == b'\n').unwrap() + 1;
        assert_eq!(&frame[..2], &[0x20, 0x31 + 0x20]);
        assert_eq!(&frame[2..10], b"GNW22122");
        let mut g = Gen3::new(0x31);
        let msgs: Vec<Vec<u8>> = frame[at..].split_inclusive(|&b| b == b'\n').map(|m| g.decode(m).unwrap()).collect();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0], lang_msg());
        assert_eq!(&msgs[1][..11], &[39 + 0x20, 0x2a, 0x7b, 0xad, 0x19, 0x5e, 0x23, 0x12, 0x43, 0xd8, 0x2c]);
        s.conn.send_messages(&[lang_msg()]).unwrap();
        let next = read_frame(&mut srv);
        assert_eq!(g.decode(&next).unwrap(), lang_msg());

        // Without the tamper check: the login line and the language.
        let _s = login_gserver(&addr, ClientVersion::V217, 0x32, "ab", "cd", false).unwrap();
        let (mut srv, _) = listener.accept().unwrap();
        let frame = read_frame(&mut srv);
        let at = frame.iter().position(|&b| b == b'\n').unwrap() + 1;
        assert_eq!(Gen3::new(0x32).decode(&frame[at..]).unwrap(), lang_msg());
    }

    /// A connection that cannot be made is a connect failure.
    #[test]
    fn login_gserver_reports_connect_failure() {
        let addr = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().to_string()
        };
        let Err(e) = login_gserver(&addr, ClientVersion::V231, 1, "ab", "cd", false) else {
            panic!("nothing listens there")
        };
        assert!(e.connect);
        assert!(e.to_string().starts_with("connect failed:"), "{e}");
    }

    /// Following a server warp: the old connection is closed, and the new
    /// one is a complete fresh login to the warp's host and port with a new
    /// key, the same credentials, and the login 46 again. Nothing is sent
    /// to a listserver.
    #[test]
    fn server_warp_reconnects_with_a_fresh_login() {
        use crate::protocol::layouts::parse_server_warp;
        let one = TcpListener::bind("127.0.0.1:0").unwrap();
        let two = TcpListener::bind("127.0.0.1:0").unwrap();
        let (a1, a2) = (one.local_addr().unwrap().to_string(), two.local_addr().unwrap());
        let s1 = login_gserver(&a1, ClientVersion::V231, 0x41, "ab", "cd", false).unwrap();
        let (mut srv1, _) = one.accept().unwrap();
        let first = read_frame(&mut srv1);

        // The server sends 178 as in the lab: name, name, host, port.
        let text = format!("two,two,{},{}", a2.ip(), a2.port());
        let w = parse_server_warp(text.as_bytes()).unwrap();
        let target = w.address().unwrap();
        assert_eq!(target, a2.to_string());
        s1.conn.shutdown().unwrap();
        drop(s1);
        let s2 = login_gserver(&target, ClientVersion::V231, 0x42, "ab", "cd", false).unwrap();

        // The old connection is closed (FIN).
        let mut rest = Vec::new();
        srv1.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut sink = [0u8; 64];
        loop {
            match srv1.read(&mut sink) {
                Ok(0) => break,
                Ok(n) => rest.extend_from_slice(&sink[..n]),
                Err(e) => panic!("old connection not closed: {e}"),
            }
        }
        // The new one logs in again: same account and password, new key.
        let (mut srv2, _) = two.accept().unwrap();
        let second = read_frame(&mut srv2);
        assert_eq!(&first[2..], &second[2..]);
        assert_ne!(first[1], second[1]);
        assert_eq!(second[1], 0x42 + 0x20);
        let mut g = Gen5::new(0x42);
        assert_eq!(gen5_body(&mut srv2, &mut g), lang_msg());
        assert!(s2.enter_level.login_owed(), "the fresh login owes its 46 again");
        // A fresh sender: the first level on the new server is not a change.
        let mut el = s2.enter_level;
        assert!(!el.note_level("start.nw"));
    }
}
