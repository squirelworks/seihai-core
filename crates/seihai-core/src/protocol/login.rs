//! What a real 2.17, 2.22 or 2.31 client sends once the server's login burst
//! is in: the image checks, the process list and the login props.
//!
//! Source: the lab, 2026-10-24 and 2026-10-25, real clients on a private
//! loopback server (`documentation/login-burst.md`). All three builds send,
//! in one frame about 60 ms after the burst (the burst ends with
//! `PLO_LISTPROCESSES`, 182):
//!
//! 1. one file check per image the player wears or carries, in this order:
//!    sword image, shield image, head image, body image, then the image of
//!    each weapon in the order the server gave the weapons. A file the client
//!    has gets `PLI_UPDATEFILE` (34) with its mod time; a missing one gets
//!    `PLI_WANTFILE` (23);
//! 2. `PLI_PROCESSLIST` (44), the answer to 182;
//! 3. `PLI_PLAYERPROPS` (2) with the player's identity props.
//!
//! The status icons (`plisticon<status>.png`, one per `PLO_STATUSLIST` entry)
//! are asked for as well; they ride the same request queue (four in flight).
//!
//! What Seihai leaves out, and why, is in the document.

use super::dispatch::{latin1, Weapon};
use super::props::parse_player_props_in;
use crate::offline::server_status_base;
use super::{IdSet, SessionState};
use std::time::{Duration, Instant};

/// `PLI_PLAYERPROPS`.
pub const PLI_PLAYERPROPS: u8 = 2;
/// `PLI_PROCESSLIST`.
pub const PLI_PROCESSLIST: u8 = 44;
/// `PLO_LISTPROCESSES`, which the burst ends with.
pub const PLO_LISTPROCESSES: u8 = 182;

/// What a profile records about the post-login frame. `Some` only for the
/// builds observed doing it (2.17, 2.22, 2.31 and 3.04), and for 3.153,
/// which takes 3.04's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginInfo {
    /// The props carry `OSTYPE` (75) and `TEXTCODEPAGE` (76) with these
    /// values: 2.22 and 2.31 (`wind`, 1252), 3.04 (`linux`, 1250), 3.153
    /// (`mac`, the code page carried from 3.04). The 2.17
    /// message stops at `PLANGUAGE`.
    pub os_props: Option<(&'static str, u32)>,
    /// The build checks `idle.gani` (2.17 and 2.22; 2.31 does not ask). The
    /// real 2.22 puts it first in the post-login frame, ahead of the file
    /// checks and the props (lab, 2026-10-30, stock beta4: +250 ms, about
    /// 240 ms after the level), not at once after the login line.
    pub idle_gani: bool,
    /// The status icons are asked for after the burst.
    pub status_icons: bool,
    /// The 2.x shape: every worn image and every weapon image is checked,
    /// `PLI_PROCESSLIST` (44) answers the burst, and the props carry
    /// `HEADGIF` and `COLORS`. The real 3.04 checks only the head image and
    /// the default weapons' images, sends no process list (it has no 44),
    /// and its props carry neither.
    pub full: bool,
    /// The login line, `PLI_LANGUAGE` and a check of each of these files go
    /// in one plain zlib frame (3.04: `bigmap.txt` and `minimap.txt`, which
    /// its install ships in `maps/`). Empty: the login line goes alone.
    pub first_frame_files: &'static [&'static str],
    /// The login line, `PLI_LANGUAGE` and (when sent) `PLI_TAMPERCHECK` go
    /// in one frame, as the real 2.17.1 sends them (lab, 2026-10-30,
    /// `documentation/difftest-217.md` item 5). The messages after the login
    /// line carry the gen-3 insertion already. A build with
    /// `first_frame_files` bundles as well.
    pub bundle_login: bool,
    /// How long after the level a 2.x frame goes when no 182 came first.
    /// `None`: [`FRAME_AFTER_LEVEL`] (2.22, 2.31). The real 2.17.1 sent its
    /// frame 96 to 112 ms after the board on both lab servers, with or without
    /// 182 (2026-10-30, `documentation/difftest-217.md` item 2), so 2.17 has
    /// 96 ms, the shortest gap seen.
    pub frame_after_level: Option<Duration>,
}

pub const LOGIN_217: LoginInfo = LoginInfo {
    os_props: None,
    idle_gani: true,
    status_icons: true,
    full: true,
    first_frame_files: &[],
    bundle_login: true,
    frame_after_level: Some(Duration::from_millis(96)),
};
pub const LOGIN_222: LoginInfo =
    LoginInfo { os_props: Some(OS_WINDOWS), idle_gani: true, status_icons: true, full: true, first_frame_files: &[], bundle_login: false, frame_after_level: None };
pub const LOGIN_231: LoginInfo =
    LoginInfo { os_props: Some(OS_WINDOWS), idle_gani: false, status_icons: true, full: true, first_frame_files: &[], bundle_login: false, frame_after_level: None };
/// The real 3.04 Linux build (lab, 2026-10-30, `documentation/login-burst.md`).
pub const LOGIN_304: LoginInfo = LoginInfo {
    os_props: Some(("linux", 1250)),
    idle_gani: false,
    status_icons: true,
    full: false,
    first_frame_files: &["bigmap.txt", "minimap.txt"],
    bundle_login: true,
    frame_after_level: None,
};
/// 3.153 (Mac OS X), never run: 3.04's frame with `OSTYPE` `mac`, the value
/// its binary writes (Graal_analysis `version-profiles/3.04-3.153.toml`). The
/// code page and the rest of the frame are 3.04's; the notes have no 3.153
/// value for them.
pub const LOGIN_3153: LoginInfo = LoginInfo { os_props: Some(("mac", 1250)), ..LOGIN_304 };

/// What the Windows clients put in `OSTYPE` and `TEXTCODEPAGE`. Seihai has
/// no value of its own to report, so the Windows builds send the one
/// servers know.
const OS_WINDOWS: (&str, u32) = ("wind", 1252);
/// The language the post-login `PLI_LANGUAGE` names.
pub const LANGUAGE: &str = "English";

fn gstring(out: &mut Vec<u8>, text: &str) {
    let b: Vec<u8> = text.chars().map(|c| if (c as u32) < 0x100 { c as u8 } else { b'?' }).take(223).collect();
    out.push(b.len() as u8 + 0x20);
    out.extend(b);
}

/// The image names in the own player's `PLO_PLAYERPROPS` (9) payload, in
/// check order: `SWORDPOWER` (8), `SHIELDPOWER` (9), `HEADGIF` (11), then
/// `BODYIMG` (35). A prop whose value has no image gives nothing.
pub fn wear_images(own_props: &[u8], accepted: Option<&IdSet>) -> Vec<String> {
    worn_images(own_props, accepted).into_iter().flatten().collect()
}

/// The sword, shield, head and body images of [`wear_images`], each `None`
/// when its prop has no image.
fn worn_images(own_props: &[u8], accepted: Option<&IdSet>) -> [Option<String>; 4] {
    let mut sword = None;
    let mut shield = None;
    let mut head = None;
    let mut body = None;
    for p in parse_player_props_in(own_props, false, accepted) {
        let r = &p.raw;
        let name = |bytes: Option<&[u8]>| bytes.map(latin1).filter(|s| !s.is_empty());
        match p.id {
            // power byte, then a GSTRING image when the power byte says so
            8 | 9 => {
                let limit = if p.id == 8 { 4 } else { 3 };
                if r.first().map_or(false, |b| b.wrapping_sub(0x20) > limit) {
                    let n = r.get(1).map_or(0, |b| b.wrapping_sub(0x20) as usize);
                    let v = name(r.get(2..2 + n));
                    if p.id == 8 { sword = v } else { shield = v }
                }
            }
            // length + 100, then the image
            11 => {
                let v = r.first().map_or(0, |b| b.wrapping_sub(0x20) as usize);
                if v >= 100 {
                    head = name(r.get(1..1 + (v - 100)));
                }
            }
            35 => {
                let n = r.first().map_or(0, |b| b.wrapping_sub(0x20) as usize);
                body = name(r.get(1..1 + n));
            }
            _ => {}
        }
    }
    [sword, shield, head, body]
}

/// The image of each weapon, in the order the server added them.
///
/// A name with no extension gets `.gif`: the lab's server gave a weapon the
/// image `-`, and the real 2.22 and 2.31 clients asked for `-.gif`.
pub fn weapon_images(weapons: &[Weapon]) -> Vec<String> {
    weapons
        .iter()
        .filter_map(|w| w.image.clone())
        .filter(|s| !s.is_empty())
        .map(|s| if s.contains('.') { s } else { format!("{s}.gif") })
        .collect()
}

/// The weapon image the real 2.22 client asks for when the server sends a
/// default weapon (`PLO_DEFAULTWEAPON`, 43) with this id.
///
/// Lab pairs (real 2.22): with the server's ids filtered to 7 alone the
/// client asked `wbowi1.png`; ids 12, 20, 21, 22 and 23 sent after login made
/// it ask `wbomb2.png`, `wbowi2.png`, `wbowi3.png`, `wbowi4.png` and
/// `wbomb3.png`. Id 8 is the bomb: the 2.05 rev 2 item table ties it to
/// `wbomb1` (Graal_analysis `client-internals/status-display.md`); the lab
/// could not isolate it because the client asks `wbomb1.png` for its own
/// bomb as well. Nothing is invented for other ids.
pub fn default_weapon_image(id: u8) -> Option<&'static str> {
    match id {
        7 => Some("wbowi1.png"),
        8 => Some("wbomb1.png"),
        12 => Some("wbomb2.png"),
        20 => Some("wbowi2.png"),
        21 => Some("wbowi3.png"),
        22 => Some("wbowi4.png"),
        23 => Some("wbomb3.png"),
        _ => None,
    }
}

/// The bomb image the 3.04 login frame checks after the head image.
const BOMB_IMAGE: &str = "wbomb1.png";

/// The icon file for a status name: `plisticon` and the lower-case name
/// (`No PMs` gives `plisticonno pms.png`).
pub fn status_icon(status: &str) -> String {
    format!("plisticon{}.png", status.to_ascii_lowercase())
}

/// `PLI_PROCESSLIST`: the processes the client runs, one per line,
/// gtokenized (GServer reads it with `guntokenize` and splits on newlines).
/// Seihai lists itself and nothing else.
pub fn process_list_message(names: &[&str]) -> Vec<u8> {
    let mut m = vec![PLI_PROCESSLIST + 0x20];
    m.extend(
        names
            .iter()
            .map(|n| format!("\"{}\"", n.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(",")
            .bytes(),
    );
    m.push(b'\n');
    m
}

/// The login `PLI_PLAYERPROPS`, in the order the real clients write it:
/// `NICKNAME` (0), `HEADGIF` (11), `COLORS` (13), `STATUS` (18),
/// `ADDITFLAGS` (33), `PLANGUAGE` (52), and with `os_props` `OSTYPE` (75) and
/// `TEXTCODEPAGE` (76). Head, colours and status are copied from the server's
/// own props as they came, except that the status goes out without the
/// paused and dead bits ([`server_status_base`]): the real 2.22 sent STATUS
/// 20 for the server's 29 (lab, 2026-10-31, `lab-deadlogin.md`). Copying 29
/// drew a full-hearts player dead on other clients. Without `full` (3.04) the props have
/// no head or colours. A real client also sends `UDPPORT` (31) between
/// `STATUS` and `ADDITFLAGS`; Seihai binds no UDP socket, so it leaves the
/// prop out.
pub fn login_props_message(info: &LoginInfo, own_props: &[u8], account: &str, accepted: Option<&IdSet>) -> Vec<u8> {
    let props = parse_player_props_in(own_props, false, accepted);
    let raw_of = |id: u8| props.iter().find(|p| p.id == id).map(|p| p.raw.clone());
    let mut m = vec![PLI_PLAYERPROPS + 0x20];
    m.push(0x20);
    gstring(&mut m, account);
    let ids: &[u8] = if info.full { &[11, 13, 18] } else { &[18] };
    for &id in ids {
        if let Some(mut raw) = raw_of(id) {
            if id == crate::life::PLPROP_STATUS {
                if let Some(b) = raw.first_mut() {
                    *b = server_status_base(b.saturating_sub(0x20)) + 0x20;
                }
            }
            m.push(id + 0x20);
            m.extend(raw);
        }
    }
    m.extend([33 + 0x20, 0x20]);
    m.push(52 + 0x20);
    gstring(&mut m, LANGUAGE);
    if let Some((os, v)) = info.os_props {
        m.push(75 + 0x20);
        gstring(&mut m, os);
        m.push(76 + 0x20);
        m.extend([((v >> 14) & 0x7F) as u8 + 0x20, ((v >> 7) & 0x7F) as u8 + 0x20, (v & 0x7F) as u8 + 0x20]);
    }
    m.push(b'\n');
    m
}

/// The first `PLI_PLAYERPROPS` of a gen-2 build (1.21 to 1.39), sent once
/// the first level has loaded: `NICKNAME` (0), `HEADGIF` (11), `COLORS`
/// (13), `STATUS` (18) from 1.31, `ADDITFLAGS` (33) from 1.32. The values
/// follow lab runs of the real 1.28 beta 2 to 1.39 against a stub that
/// varied the server's props (`documentation/login-burst.md`):
///
/// * the nickname is the server's (prop 0), not the account;
/// * colours go back as they came;
/// * the status keeps only bits 16 and 64 of the server's (the others are
///   the client's own state, all clear at login);
/// * a numbered head (`c(n)`) goes back as it came. 1.39 sends a
///   `head<N>.gif` name as `c(N)` and any other name as it came. 1.33 sends
///   `c(0)` for any name (it has no image heads).
///
/// The real clients also send `UDPPORT` (31) before `ADDITFLAGS`; as in
/// [`login_props_message`], Seihai binds no UDP socket and leaves it out.
pub fn classic_login_props_message(version: super::ClientVersion, own_props: &[u8], account: &str, accepted: Option<&IdSet>) -> Vec<u8> {
    let props = parse_player_props_in(own_props, false, accepted);
    let raw_of = |id: u8| props.iter().find(|p| p.id == id).map(|p| p.raw.as_slice());
    let mut m = vec![PLI_PLAYERPROPS + 0x20];
    m.push(0x20);
    match raw_of(0) {
        Some(nick) => m.extend(nick),
        None => gstring(&mut m, account),
    }
    if let Some(head) = raw_of(11) {
        m.push(11 + 0x20);
        m.extend(classic_head(version, head));
    }
    if let Some(colors) = raw_of(13) {
        m.push(13 + 0x20);
        m.extend(colors);
    }
    if classic_sends_status(version) {
        if let Some(&b) = raw_of(18).and_then(|r| r.first()) {
            m.extend([18 + 0x20, (b.saturating_sub(0x20) & CLASSIC_STATUS_KEPT) + 0x20]);
        }
    }
    // ADDITFLAGS came in with 1.32, the first build whose props reach 33.
    if version.profile().props.own.contains(33) {
        m.extend([33 + 0x20, 0x20]);
    }
    m.push(b'\n');
    m
}

/// Whether the gen-2 login props carry `STATUS`: from 1.31. The real 1.28
/// beta 2 and 1.3 test sent nick, head and colours only (and `UDPPORT`),
/// lab 2026-10-30; 1.21, which was not run, is taken as 1.28 beta 2.
fn classic_sends_status(version: super::ClientVersion) -> bool {
    use super::ClientVersion::*;
    !matches!(version, V121 | V128B2 | V13T)
}

/// The server's status bits a gen-2 client sends back at login (lab, 1.39:
/// 1, 2, 4, 8 and 32 came back clear, 16 and 64 as sent).
pub const CLASSIC_STATUS_KEPT: u8 = 16 | 64;

/// The head value of [`classic_login_props_message`] for the server's
/// `HEADGIF` bytes: `c(n)` for a numbered head, else `c(len + 100)` + name.
/// Builds before 1.34 send `c(0)` for a named head (lab, 2026-10-30: the
/// real 1.33 sent `c(0)` and the real 1.34 to 1.39 sent `head3.png`).
fn classic_head(version: super::ClientVersion, raw: &[u8]) -> Vec<u8> {
    let Some(&first) = raw.first() else { return vec![0x20] };
    let n = first.saturating_sub(0x20);
    if n < 100 {
        return vec![first];
    }
    let all = super::ClientVersion::all_versions();
    let at = |v| all.iter().position(|&x| x == v);
    if at(version) < at(super::ClientVersion::V134) {
        return vec![0x20];
    }
    let name = raw.get(1..).unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    let num = lower.strip_prefix(b"head").and_then(|r| r.strip_suffix(b".gif")).filter(|d| !d.is_empty() && d.iter().all(u8::is_ascii_digit));
    match num.and_then(|d| std::str::from_utf8(d).ok()?.parse::<u8>().ok()).filter(|&v| v < 100) {
        Some(v) => vec![v + 0x20],
        None => raw.to_vec(),
    }
}

/// Whether `version` sends [`classic_login_props_message`] once its first
/// level has loaded: the gen-2 builds that can send `PLI_PLAYERPROPS`.
pub fn sends_classic_login_props(version: super::ClientVersion) -> bool {
    let p = version.profile();
    p.protocol.encryption == super::EncryptionGen::Gen2 && p.protocol.pli.contains(PLI_PLAYERPROPS)
}

/// Decides when the post-login frame goes out: once, when the server's
/// burst has ended (`PLO_LISTPROCESSES` seen). A build with no
/// [`LoginInfo`], or one that cannot send `PLI_PLAYERPROPS` (2), never sends it.
#[derive(Debug, Clone)]
pub struct LoginBurst {
    info: LoginInfo,
    sent: bool,
    /// `idle.gani` is still to be checked ([`LoginInfo::idle_gani`]).
    idle_owed: bool,
    /// When the first level arrived from the server ([`Self::note_level`]).
    level_at: Option<Instant>,
}

/// How long after the level the real 2.22 and 2.31 sent their post-login
/// frame when the server sent no `PLO_LISTPROCESSES` (lab, 2026-10-30, stock
/// GServer beta4: 236 to 246 ms in four runs). A 2.x build's frame is due
/// then if no 182 came first ([`LoginBurst::due_at`]).
pub const FRAME_AFTER_LEVEL: Duration = Duration::from_millis(240);

impl LoginBurst {
    /// `None` for a profile without a recorded post-login frame.
    pub fn for_version(version: super::ClientVersion) -> Option<Self> {
        let p = version.profile();
        let info = p.protocol.login?;
        let can = p.protocol.pli.contains(PLI_PLAYERPROPS) && (!info.full || p.protocol.pli.contains(PLI_PROCESSLIST));
        can.then_some(Self { info, sent: false, idle_owed: info.idle_gani, level_at: None })
    }

    pub fn info(&self) -> LoginInfo {
        self.info
    }

    /// The burst has ended and the frame is still owed.
    /// The frame has not gone out yet.
    pub fn pending(&self) -> bool {
        !self.sent
    }

    /// The first level arrived from the server at `at` (the time the frame
    /// was read off the socket, not when the game loop got to it). Later
    /// calls keep the first time.
    pub fn note_level(&mut self, at: Instant) {
        self.level_at.get_or_insert(at);
    }

    /// The frame is due at `now` without a 182: a 2.x build whose first level
    /// has been in for [`FRAME_AFTER_LEVEL`] (or the build's own
    /// [`LoginInfo::frame_after_level`]).
    fn late_at(&self, now: Instant) -> bool {
        let after = self.info.frame_after_level.unwrap_or(FRAME_AFTER_LEVEL);
        self.info.full && self.level_at.is_some_and(|t| now.saturating_duration_since(t) >= after)
    }

    /// [`Self::due`], or the frame is late at `now` ([`FRAME_AFTER_LEVEL`]).
    pub fn due_at(&self, state: &SessionState, now: Instant) -> bool {
        self.due(state) || (!self.sent && self.late_at(now))
    }

    /// The 2.x builds answer the burst's end (`PLO_LISTPROCESSES`); 3.04,
    /// which has no handler for 182, sent its frame once the first board
    /// was in (lab, 2026-10-30).
    pub fn due(&self, state: &SessionState) -> bool {
        !self.sent && if self.info.full { state.process_list_requests > 0 } else { state.board.is_some() }
    }

    /// Build the frame's messages (once). `check` turns an image name into
    /// the file messages the request rules give (`PLI_UPDATEFILE` or
    /// `PLI_WANTFILE`, or nothing when the name is queued or settled).
    /// `enter_level` gives the login `PLI_ENTERLEVEL` (46) when one is owed
    /// (2.31): the real client sends it right after the file checks.
    /// `flags` gives the flag messages scripts have queued so far (see
    /// [`super::flagset`]), which go between the file checks and the process
    /// list, where the real 2.22 put the `-gr_movement` weapon's first
    /// `gr.x`/`gr.y`; it runs only when the frame is built.
    pub fn take(
        &mut self,
        state: &SessionState,
        account: &str,
        now: Instant,
        mut check: impl FnMut(&str) -> Vec<Vec<u8>>,
        enter_level: impl FnOnce() -> Vec<Vec<u8>>,
        flags: impl FnOnce() -> Vec<Vec<u8>>,
    ) -> Vec<Vec<u8>> {
        if !self.due_at(state, now) {
            return Vec::new();
        }
        self.sent = true;
        let mut out = Vec::new();
        if std::mem::take(&mut self.idle_owed) {
            out.extend(check("idle.gani"));
        }
        if self.info.full && state.process_list_requests == 0 {
            return self.late_frame(out, state, account, check, enter_level, flags);
        }
        let mut names = if self.info.full {
            let mut n = wear_images(&state.player_props, state.own_prop_ids.as_ref());
            n.extend(weapon_images(&state.weapons));
            n
        } else {
            // 3.04 checks the head image alone of the worn ones, then the
            // bomb image: the real 2.22 asks `wbomb1.png` whatever default
            // weapons it was sent, and the real 3.04 asked it in the same
            // place (lab, 2026-10-30).
            let [_, _, head, _] = worn_images(&state.player_props, state.own_prop_ids.as_ref());
            head.into_iter().chain([BOMB_IMAGE.to_string()]).collect()
        };
        // Default weapons the server sent, in arrival order: the real client
        // asks for their images too (2.31: ids 8 then 7 gave `wbomb1.png`
        // then `wbowi1.png`).
        names.extend(
            state
                .default_weapons
                .iter()
                .filter_map(|&id| default_weapon_image(id))
                .map(str::to_string)
                .filter(|n| !names.contains(n))
                .collect::<Vec<_>>(),
        );
        let icons: Vec<String> = if self.info.status_icons { state.status_list.iter().map(|s| status_icon(s)).collect() } else { Vec::new() };
        // 2.x asks the icons with the other files; the real 3.04 asked them
        // after the flags.
        if self.info.full {
            names.extend(icons.iter().cloned());
        }
        for n in &names {
            out.extend(check(n));
        }
        out.extend(enter_level());
        out.extend(flags());
        if !self.info.full {
            for n in &icons {
                out.extend(check(n));
            }
        }
        if self.info.full {
            out.push(process_list_message(&["seihai"]));
        }
        out.push(login_props_message(&self.info, &state.player_props, account, state.own_prop_ids.as_ref()));
        out
    }

    /// The 2.x frame with no 182 (lab, 2026-10-30, stock beta4, real 2.22 and
    /// 2.31): the status icon checks, the login 46 (2.31), the props; no
    /// process list. The worn images (sword, shield, body: the head is never
    /// checked) and the weapon images are checked after the icons, so with
    /// four icons in flight they go out in later frames as replies come;
    /// any the window lets out now follow the props.
    fn late_frame(
        &self,
        mut out: Vec<Vec<u8>>,
        state: &SessionState,
        account: &str,
        mut check: impl FnMut(&str) -> Vec<Vec<u8>>,
        enter_level: impl FnOnce() -> Vec<Vec<u8>>,
        flags: impl FnOnce() -> Vec<Vec<u8>>,
    ) -> Vec<Vec<u8>> {
        let icons: Vec<String> = if self.info.status_icons { state.status_list.iter().map(|s| status_icon(s)).collect() } else { Vec::new() };
        let [sword, shield, _head, body] = worn_images(&state.player_props, state.own_prop_ids.as_ref());
        let mut images: Vec<String> = [sword, shield, body].into_iter().flatten().collect();
        images.extend(weapon_images(&state.weapons));
        for n in state.default_weapons.iter().filter_map(|&id| default_weapon_image(id)) {
            if !images.iter().any(|i| i == n) {
                images.push(n.to_string());
            }
        }
        for n in &icons {
            out.extend(check(n));
        }
        out.extend(enter_level());
        out.extend(flags());
        out.push(login_props_message(&self.info, &state.player_props, account, state.own_prop_ids.as_ref()));
        for n in &images {
            out.extend(check(n));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2.22-style own-props stream, built by hand: power bytes, image
    /// strings, head, colours, status, body.
    fn own_props() -> Vec<u8> {
        let mut p = Vec::new();
        p.extend([8 + 0x20, 31 + 0x20]);
        gstring(&mut p, "sword1.png");
        p.extend([9 + 0x20, 11 + 0x20 + 0]);
        gstring(&mut p, "shield1.png");
        p.push(11 + 0x20);
        p.push(0x20 + 100 + 9);
        p.extend(b"head0.png");
        p.extend([13 + 0x20, 0x22, 0x20, 0x2a, 0x24, 0x32]);
        p.extend([18 + 0x20, 0x20 + 20]);
        p.push(35 + 0x20);
        gstring(&mut p, "body.png");
        p
    }

    #[test]
    fn wear_images_come_in_prop_order() {
        assert_eq!(wear_images(&own_props(), None), ["sword1.png", "shield1.png", "head0.png", "body.png"]);
    }

    #[test]
    fn a_prop_without_image_gives_none() {
        // sword power 1 (byte 0x20 + 1), shield power 1 (0x20 + 1): no images.
        let p = [8 + 0x20, 0x21, 9 + 0x20, 0x21];
        assert!(wear_images(&p, None).is_empty());
    }

    #[test]
    fn status_icon_names() {
        assert_eq!(status_icon("Online"), "plisticononline.png");
        assert_eq!(status_icon("No PMs"), "plisticonno pms.png");
    }

    #[test]
    fn process_list_is_gtokenized() {
        assert_eq!(process_list_message(&["seihai"]), b"\x4c\"seihai\"\n");
        assert_eq!(process_list_message(&["a", "b"]), b"\x4c\"a\",\"b\"\n");
    }

    #[test]
    fn login_props_222_layout() {
        let m = login_props_message(&LOGIN_222, &own_props(), "acct", None);
        // id, NICKNAME, HEADGIF, COLORS, STATUS, ADDITFLAGS, PLANGUAGE, OSTYPE, TEXTCODEPAGE
        let mut want = vec![0x22, 0x20, 0x24];
        want.extend(b"acct");
        want.extend([0x2b, 0x20 + 109]);
        want.extend(b"head0.png");
        want.extend([0x2d, 0x22, 0x20, 0x2a, 0x24, 0x32]);
        want.extend([0x32, 0x20 + 20]);
        want.extend([0x41, 0x20]);
        want.extend([0x20 + 52, 0x20 + 7]);
        want.extend(b"English");
        want.extend([0x20 + 75, 0x24]);
        want.extend(b"wind");
        want.extend([0x20 + 76, 0x20, 0x29, 0x84]);
        want.push(b'\n');
        assert_eq!(m, want);
        // TEXTCODEPAGE 1252 reads back as the capture's three bytes.
        assert_eq!(&m[m.len() - 4..m.len() - 1], &[0x20, 0x29, 0x84]);
    }

    /// An account saved dead keeps STATUS 29 with full hearts. The real 2.22
    /// sent 20 back (lab, 2026-10-31, `lab-deadlogin.md`): the dead and paused
    /// bits cleared, 0x10 and 0x04 kept.
    #[test]
    fn login_props_clear_the_dead_and_paused_bits() {
        let mut st = SessionState::for_version(super::super::ClientVersion::V222);
        st.apply_body(&[9 + 0x20, 18 + 0x20, 0x20 + 29, b'\n']);
        let m = login_props_message(&LOGIN_222, &st.player_props, "acct", st.own_prop_ids.as_ref());
        let props = parse_player_props_in(&m[1..m.len() - 1], false, None);
        let status = props.iter().find(|p| p.id == 18).map(|p| p.raw.clone());
        assert_eq!(status.as_deref(), Some(&[0x20 + 20][..]));
    }

    /// The server's props of the lab runs: nick `stubnick`, head, colours,
    /// status.
    fn classic_server_props(head: &[u8], status: u8) -> Vec<u8> {
        let mut p = vec![0x20, 0x28];
        p.extend(b"stubnick");
        p.extend([1 + 0x20, 3 + 0x20, 2 + 0x20, 6 + 0x20, 11 + 0x20]);
        p.extend(head);
        p.extend([13 + 0x20, 0x22, 0x20, 0x2a, 0x24, 0x32, 18 + 0x20, status + 0x20, 15 + 0x20, 60 + 0x20, 16 + 0x20, 60 + 0x20]);
        p
    }

    fn image_head(name: &[u8]) -> Vec<u8> {
        let mut h = vec![name.len() as u8 + 100 + 0x20];
        h.extend(name);
        h
    }

    /// Lab, 2026-10-30: the real clients' first id 2 against the stub,
    /// without the `UDPPORT` (31) prop, which Seihai leaves out. 1.34 to
    /// 1.38 were run with `head3.png` only.
    #[test]
    fn classic_login_props_match_the_real_1_33_to_1_39() {
        use crate::protocol::ClientVersion::{V133, V134, V135, V136, V137, V138, V139};
        let real = |hex: &str| {
            let mut v = vec![PLI_PLAYERPROPS + 0x20];
            v.extend((0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()));
            v.extend([33 + 0x20, 0x20, b'\n']);
            v
        };
        let cases: &[(crate::protocol::ClientVersion, Vec<u8>, u8, &str)] = &[
            (V139, image_head(b"head3.png"), 20, "2028737475626e69636b2b8d68656164332e706e672d22202a24323230"),
            (V133, image_head(b"head3.png"), 20, "2028737475626e69636b2b202d22202a24323230"),
            (V139, image_head(b"head1.gif"), 0, "2028737475626e69636b2b212d22202a24323220"),
            (V133, image_head(b"head1.gif"), 0, "2028737475626e69636b2b202d22202a24323220"),
            (V139, image_head(b"head1.gif"), 4, "2028737475626e69636b2b212d22202a24323220"),
            (V139, image_head(b"head1.gif"), 64, "2028737475626e69636b2b212d22202a24323260"),
            (V139, image_head(b"head3.png"), 0, "2028737475626e69636b2b8d68656164332e706e672d22202a24323220"),
            (V133, vec![5 + 0x20], 16, "2028737475626e69636b2b252d22202a24323230"),
            (V139, vec![5 + 0x20], 16, "2028737475626e69636b2b252d22202a24323230"),
            (V134, image_head(b"head3.png"), 20, "2028737475626e69636b2b8d68656164332e706e672d22202a24323230"),
            (V135, image_head(b"head3.png"), 20, "2028737475626e69636b2b8d68656164332e706e672d22202a24323230"),
            (V136, image_head(b"head3.png"), 20, "2028737475626e69636b2b8d68656164332e706e672d22202a24323230"),
            (V137, image_head(b"head3.png"), 20, "2028737475626e69636b2b8d68656164332e706e672d22202a24323230"),
            (V138, image_head(b"head3.png"), 20, "2028737475626e69636b2b8d68656164332e706e672d22202a24323230"),
        ];
        for (v, head, status, hex) in cases {
            let ids = v.profile().props.own.clone();
            let m = classic_login_props_message(*v, &classic_server_props(head, *status), "acct", Some(&ids));
            assert_eq!(m, real(hex), "{} status {status}", v.label());
        }
        assert!(sends_classic_login_props(V133) && sends_classic_login_props(V139));
        assert!(!sends_classic_login_props(crate::protocol::ClientVersion::V141));
    }

    /// The real 1.28 beta 2, 1.3 test, 1.31 and 1.32 against the gen-2
    /// stub (lab, 2026-10-30; server head `head3.png`, status 20): the
    /// bytes up to `UDPPORT`, then `ADDITFLAGS` 0 in 1.32 only. 1.28 beta 2
    /// and 1.3 test send no `STATUS`.
    #[test]
    fn classic_login_props_match_the_real_1_28_to_1_32() {
        use crate::protocol::ClientVersion::{V128B2, V131, V132, V13T};
        let hex = |h: &str| -> Vec<u8> { (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect() };
        let base = "2028737475626e69636b2b202d22202a2432";
        let cases = [(V128B2, base.to_string()), (V13T, base.to_string()), (V131, format!("{base}3230")), (V132, format!("{base}32304120"))];
        for (v, want) in cases {
            let ids = v.profile().props.own.clone();
            let m = classic_login_props_message(v, &classic_server_props(&image_head(b"head3.png"), 20), "acct", Some(&ids));
            let mut w = vec![PLI_PLAYERPROPS + 0x20];
            w.extend(hex(&want));
            w.push(b'\n');
            assert_eq!(m, w, "{}", v.label());
            assert!(sends_classic_login_props(v), "{}", v.label());
        }
    }

    #[test]
    fn login_props_217_stop_at_language() {
        let m = login_props_message(&LOGIN_217, &own_props(), "acct", None);
        let props = parse_player_props_in(&m[1..], false, None);
        let ids: Vec<u8> = props.iter().map(|p| p.id).collect();
        assert_eq!(ids, [0, 11, 13, 18, 33, 52]);
    }

    #[test]
    fn login_props_are_read_by_the_prop_parser() {
        let m = login_props_message(&LOGIN_231, &own_props(), "acct", None);
        let ids: Vec<u8> = parse_player_props_in(&m[1..m.len() - 1], false, None).iter().map(|p| p.id).collect();
        assert_eq!(ids, [0, 11, 13, 18, 33, 52, 75, 76]);
    }

    #[test]
    fn default_weapon_images_follow_the_lab_pairs() {
        assert_eq!(default_weapon_image(7), Some("wbowi1.png"));
        assert_eq!(default_weapon_image(20), Some("wbowi2.png"));
        assert_eq!(default_weapon_image(23), Some("wbomb3.png"));
        // Id 8 from the 2.05 rev 2 item table.
        assert_eq!(default_weapon_image(8), Some("wbomb1.png"));
        assert_eq!(default_weapon_image(0), None);
        assert_eq!(default_weapon_image(99), None);
    }

    /// The real 3.04 on the lab's GServer-v2 (2026-10-30): once the board
    /// is in, the head image, `wbomb1.png` and the default weapon 7's
    /// image, the queued flags, the status icons, then the props
    /// `NICKNAME`, `STATUS`, `ADDITFLAGS`, `PLANGUAGE`, `OSTYPE` `linux`,
    /// `TEXTCODEPAGE` 1250, with no process list.
    #[test]
    fn the_304_login_frame_follows_the_lab_run() {
        let v = super::super::ClientVersion::V304;
        let mut b = LoginBurst::for_version(v).unwrap();
        let mut st = SessionState::for_version(v);
        st.player_props = own_props();
        st.weapons = vec![Weapon { name: "-gr_movement".into(), image: Some("-".into()), script: None }];
        st.default_weapons = vec![8, 7];
        st.status_list = vec!["Eating".into(), "No PMs".into()];
        st.process_list_requests = 1;
        assert!(!b.due(&st), "3.04 does not wait for 182; it waits for the board");
        st.board = Some(vec![0; 4096]);
        let msgs = b.take(&st, "Lab304", Instant::now(), |n| vec![n.as_bytes().to_vec()], Vec::new, || vec![b"flag".to_vec()]);
        let names: Vec<&[u8]> = msgs.iter().map(|m| m.as_slice()).collect();
        assert_eq!(&names[..6], [&b"head0.png"[..], b"wbomb1.png", b"wbowi1.png", b"flag", b"plisticoneating.png", b"plisticonno pms.png"]);
        let mut want = vec![0x22, 0x20, 0x26];
        want.extend(b"Lab304");
        want.extend([0x32, 0x20 + 20, 0x41, 0x20, 0x20 + 52, 0x20 + 7]);
        want.extend(b"English");
        want.extend([0x20 + 75, 0x25]);
        want.extend(b"linux");
        // TEXTCODEPAGE: the capture's three bytes, 1250.
        want.extend([0x20 + 76, 0x20, 0x29, 0x82]);
        want.push(b'\n');
        assert_eq!(names.len(), 7);
        assert_eq!(names[6], want.as_slice());
    }

    /// 3.153 writes `OSTYPE` `mac` (binary); the rest of its frame is 3.04's.
    #[test]
    fn the_3153_login_frame_is_304s_with_ostype_mac() {
        let p = super::super::ClientVersion::V3153.profile().protocol.login.unwrap();
        assert_eq!(p.os_props, Some(("mac", 1250)));
        assert_eq!(LoginInfo { os_props: LOGIN_304.os_props, ..p }, LOGIN_304);
    }

    #[test]
    fn login_burst_asks_the_image_of_default_weapon_7() {
        let mut b = LoginBurst::for_version(super::super::ClientVersion::V222).unwrap();
        let mut st = burst_state();
        st.default_weapons = vec![8, 7];
        st.process_list_requests = 1;
        let mut asked = Vec::new();
        b.take(&st, "acct", Instant::now(), |n| {
            asked.push(n.to_string());
            vec![n.as_bytes().to_vec()]
        }, Vec::new, Vec::new);
        // wbowi1.png is already asked for by the Bow weapon: once only; a
        // server without the Bow weapon but with default id 7 gets it too.
        assert_eq!(asked.iter().filter(|n| *n == "wbowi1.png").count(), 1);
        let mut b = LoginBurst::for_version(super::super::ClientVersion::V222).unwrap();
        let mut st = burst_state();
        st.weapons.retain(|w| w.name != "Bow");
        st.default_weapons = vec![8, 7];
        st.process_list_requests = 1;
        let mut asked = Vec::new();
        b.take(&st, "acct", Instant::now(), |n| {
            asked.push(n.to_string());
            vec![n.as_bytes().to_vec()]
        }, Vec::new, Vec::new);
        assert!(asked.contains(&"wbowi1.png".to_string()));
    }

    /// The server sends default weapons 8 then 7 and no weapon of its own
    /// names their images: the real 2.31 asked `wbomb1.png` then
    /// `wbowi1.png` (difftest `gmap_cross`).
    #[test]
    fn login_burst_asks_both_default_weapon_images_in_arrival_order() {
        let mut b = LoginBurst::for_version(super::super::ClientVersion::V231).unwrap();
        let mut st = burst_state();
        st.weapons.clear();
        st.default_weapons = vec![8, 7];
        st.process_list_requests = 1;
        let mut asked = Vec::new();
        b.take(&st, "acct", Instant::now(), |n| {
            asked.push(n.to_string());
            vec![n.as_bytes().to_vec()]
        }, Vec::new, Vec::new);
        let w: Vec<&str> = asked.iter().map(String::as_str).filter(|n| n.starts_with("wbo")).collect();
        assert_eq!(w, ["wbomb1.png", "wbowi1.png"]);
    }

    fn burst_state() -> SessionState {
        let mut st = SessionState::for_version(super::super::ClientVersion::V222);
        st.player_props = own_props();
        st.weapons = vec![
            Weapon { name: "Bomb".into(), image: Some("wbomb1.png".into()), script: None },
            Weapon { name: "Bow".into(), image: Some("wbowi1.png".into()), script: None },
        ];
        st.status_list = vec!["Online".into(), "No PMs".into()];
        st
    }

    #[test]
    fn frame_waits_for_the_process_list_request_and_goes_once() {
        let mut b = LoginBurst::for_version(super::super::ClientVersion::V222).unwrap();
        let mut st = burst_state();
        assert!(b.pending());
        assert!(!b.due(&st));
        assert!(b.take(&st, "acct", Instant::now(), |_| vec![b"x".to_vec()], Vec::new, Vec::new).is_empty());
        st.process_list_requests = 1;
        assert!(b.due(&st));
        let mut asked = Vec::new();
        let out = b.take(&st, "acct", Instant::now(), |n| {
            asked.push(n.to_string());
            vec![n.as_bytes().to_vec()]
        }, || vec![b"enter".to_vec()], || vec![b"flag".to_vec()]);
        // idle.gani first (2.22), the images in order, then the icons, the login 46, the flags, the process list and the props
        assert_eq!(
            asked,
            ["idle.gani", "sword1.png", "shield1.png", "head0.png", "body.png", "wbomb1.png", "wbowi1.png", "plisticononline.png", "plisticonno pms.png"]
        );
        assert_eq!(out.len(), asked.len() + 4);
        assert_eq!(out[asked.len()], b"enter");
        assert_eq!(out[asked.len() + 1], b"flag");
        assert_eq!(out[asked.len() + 2][0], PLI_PROCESSLIST + 0x20);
        assert_eq!(out[asked.len() + 3][0], PLI_PLAYERPROPS + 0x20);
        assert!(!b.pending(), "sent");
        assert!(!b.due(&st), "once");
        assert!(b.take(&st, "acct", Instant::now(), |_| vec![b"x".to_vec()], Vec::new, Vec::new).is_empty());
    }

    /// No 182 (stock beta4): a 2.x frame goes 240 ms after the level, in the
    /// shape the real 2.22 and 2.31 sent then: `idle.gani` (2.22), the icon
    /// checks, the 46 (2.31), the props, no process list; the sword, shield,
    /// body (no head) and weapon images after, as the request window allows.
    #[test]
    fn without_a_process_list_request_the_frame_goes_240_ms_after_the_level() {
        let st = burst_state();
        let t0 = Instant::now();
        let mut b = LoginBurst::for_version(super::super::ClientVersion::V222).unwrap();
        let check = |n: &str| vec![n.as_bytes().to_vec()];
        assert!(!b.due_at(&st, t0 + Duration::from_secs(5)), "no level yet");
        b.note_level(t0);
        b.note_level(t0 + Duration::from_secs(1));
        assert!(!b.due_at(&st, t0 + Duration::from_millis(239)));
        assert!(b.take(&st, "acct", t0 + Duration::from_millis(239), check, Vec::new, Vec::new).is_empty());
        let out = b.take(&st, "acct", t0 + FRAME_AFTER_LEVEL, check, || vec![b"enter".to_vec()], Vec::new);
        let names: Vec<&[u8]> = out.iter().map(|m| m.as_slice()).collect();
        assert_eq!(&names[..4], [&b"idle.gani"[..], b"plisticononline.png", b"plisticonno pms.png", b"enter"]);
        assert_eq!(out[4][0], PLI_PLAYERPROPS + 0x20);
        assert_eq!(&names[5..], [&b"sword1.png"[..], b"shield1.png", b"body.png", b"wbomb1.png", b"wbowi1.png"]);
        assert!(!out.iter().any(|m| m.first() == Some(&(PLI_PROCESSLIST + 0x20))), "no process list");
        assert!(!out.iter().any(|m| m == b"head0.png"), "the head is not checked");
        assert!(b.take(&st, "acct", t0 + Duration::from_secs(9), check, Vec::new, Vec::new).is_empty(), "once");

        // A 182 first keeps the burst shape, with idle.gani at its head.
        let mut b = LoginBurst::for_version(super::super::ClientVersion::V222).unwrap();
        let mut st182 = burst_state();
        st182.process_list_requests = 1;
        let out = b.take(&st182, "acct", t0, check, Vec::new, Vec::new);
        assert_eq!(out[0], b"idle.gani");
        assert_eq!(out[1], b"sword1.png");

        // 2.31 does not check idle.gani.
        let mut b = LoginBurst::for_version(super::super::ClientVersion::V231).unwrap();
        b.note_level(t0);
        let out = b.take(&st, "acct", t0 + FRAME_AFTER_LEVEL, check, Vec::new, Vec::new);
        assert_eq!(out[0], b"plisticononline.png");
    }

    /// Stock beta4 (lab, 2026-10-30): the full own props, then `Z` alone at
    /// the end of the level frame. The login frame still has the head,
    /// colours and status, and the sword is checked.
    #[test]
    fn a_later_z_only_props_message_keeps_the_login_props() {
        let hex = "2123222623202020242a25252621283f2a73776f7264312e706e67292b2b736869656c64312e706e672a2469646c652b8d68656164302e706e672d22202a24323122323435203620372039205c3a20405242287a313837613232324328626f64792e706e67444e927e452046204720482049204d524e204f205020512056205720582059205a205b205c205d205e205f2060206120622063206420652066206720682069206a206e27606f277070202072287a31383761323232";
        let full: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        for v in [super::super::ClientVersion::V222, super::super::ClientVersion::V231] {
            let mut st = SessionState::for_version(v);
            let mut body = vec![9 + 0x20];
            body.extend(&full);
            body.push(b'\n');
            body.extend([9 + 0x20, 0x4d, 0x52, b'\n']);
            st.apply_body(&body);
            assert_eq!(wear_images(&st.player_props, st.own_prop_ids.as_ref()), ["sword1.png", "shield1.png", "head0.png", "body.png"], "{v:?}");
            let m = login_props_message(&LOGIN_222, &st.player_props, "acct", st.own_prop_ids.as_ref());
            let props = parse_player_props_in(&m[1..m.len() - 1], false, None);
            let raw = |id: u8| props.iter().find(|p| p.id == id).map(|p| p.raw.clone());
            assert_eq!(raw(11).as_deref(), Some(&b"\x8dhead0.png"[..]), "{v:?} HEADGIF: as the server sent it");
            assert_eq!(raw(13).as_deref(), Some(&[0x22, 0x20, 0x2a, 0x24, 0x32][..]), "{v:?} COLORS");
            assert_eq!(raw(18).as_deref(), Some(&[0x34][..]), "{v:?} STATUS");
        }
    }

    /// Real 2.17.1 on beta4 (no 182, lab 2026-10-30): the frame went 96 and
    /// 105 ms after the board, with the status icons, the flags and the
    /// props, no process list.
    #[test]
    fn the_217_frame_without_182_goes_96_ms_after_the_level() {
        let st = burst_state();
        let t0 = Instant::now();
        let mut b = LoginBurst::for_version(super::super::ClientVersion::V217).unwrap();
        let check = |n: &str| vec![n.as_bytes().to_vec()];
        b.note_level(t0);
        assert!(!b.due_at(&st, t0 + Duration::from_millis(95)));
        let out = b.take(&st, "acct", t0 + Duration::from_millis(96), check, Vec::new, || vec![b"flag".to_vec()]);
        let names: Vec<&[u8]> = out.iter().map(|m| m.as_slice()).collect();
        assert_eq!(&names[..4], [&b"idle.gani"[..], b"plisticononline.png", b"plisticonno pms.png", b"flag"]);
        assert_eq!(out[4][0], PLI_PLAYERPROPS + 0x20);
        assert!(!out.iter().any(|m| m.first() == Some(&(PLI_PROCESSLIST + 0x20))), "no process list");
    }

    #[test]
    fn a_weapon_image_without_an_extension_gets_gif() {
        let w = |n: &str, i: &str| Weapon { name: n.into(), image: Some(i.into()), script: None };
        let list = [w("a", "wbomb1.png"), w("b", "-"), w("c", "")];
        assert_eq!(weapon_images(&list), ["wbomb1.png", "-.gif"]);
    }

    #[test]
    fn builds_without_a_recorded_frame_send_nothing() {
        use super::super::ClientVersion::*;
        for v in [V205, V210, V230] {
            assert!(LoginBurst::for_version(v).is_none(), "{v:?}");
        }
        for v in [V217, V222, V231] {
            assert!(LoginBurst::for_version(v).is_some(), "{v:?}");
        }
    }

    #[test]
    fn only_the_builds_that_ask_send_idle_gani() {
        use super::super::ClientVersion::*;
        assert!(LoginBurst::for_version(V217).unwrap().info().idle_gani);
        assert!(LoginBurst::for_version(V222).unwrap().info().idle_gani);
        assert!(!LoginBurst::for_version(V231).unwrap().info().idle_gani);
    }
}
