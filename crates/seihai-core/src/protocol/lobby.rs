//! The 4.x, 5.x and 6.x "Login" lobby.
//!
//! From 4.04 on there is no listserver protocol. The client opens a normal
//! game connection to a server it names `Login` and is shown the server list
//! and account screens as scripted GUI there. While the current server's name
//! starts with "login" (any case) the client draws the level, chests and
//! image NPCs but no characters and no HUD; to play, the server sends
//! `PLO_SERVERWARP` (178) and the client logs in again to the host and port
//! it names, under the new name. Sources, all in Graal_analysis:
//! `protocol/v5x-clients.md`, `protocol/v6x-clients.md`,
//! `protocol/v5v6-windows-unpacked.md`, `protocol/v5v6-lab-play.md` and
//! `version-profiles/lab-v5-2026-10-30.md`; for 4.x, `protocol/v4x-clients.md`,
//! `client-internals/live-behaviour-404.md` and `version-profiles/4.04-4.20.toml`.
//!
//! The login line of these builds ends with an unprefixed comma list after
//! the password: the platform (`win`), three machine fields (MD5 hex of the
//! Windows product id, of the adapter MAC and of the `C:\` volume serial),
//! and from 5.12 on a fifth field, the quoted Windows version. Seihai reads
//! none of those machine values: it sends each of the three as `""`, the
//! value the real 5.00r7 sent for its MAC field under Wine (lab capture,
//! 2026-10-30). Whether a server accepts all three empty was not tested.

/// What a lobby build does at login and before its first server warp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lobby {
    /// The name the client gives the server it reaches first.
    pub first_server: &'static str,
    /// The first field of the comma list (`win` in every Windows build).
    pub platform: &'static str,
    /// The quoted Windows version the client sends as the fifth field, as
    /// it was captured (5.12 to 6.03r7 under Wine and on Windows 8 and later
    /// report `"6.2 9200 "` to an exe without a manifest). `None`: 5.00r7
    /// sends four fields.
    pub os_version: Option<&'static str>,
    /// What the build sends after its login and once the server's login
    /// burst is in ([`LobbyLogin`]). `None`: not recorded (5.x and 6.x).
    pub after_login: Option<LobbyLogin>,
}

/// 4.04r2, 4.20r8 and 5.00r7: four fields (`win` and three MD5s, read in
/// the 4.x engines; `win,<md5>,"",<md5>` in the 5.00r7 lab capture).
pub const LOBBY_404: Lobby = Lobby { first_server: "Login", platform: "win", os_version: None, after_login: Some(LOBBY_LOGIN_4X) };
/// The Mac 4.20r6: `mac`, a hash of the Ethernet MAC, a hash of
/// `stat("/etc")` and an empty fourth field (read in the binary, never run).
/// Its after-login messages are taken from the Windows 4.x engines, with
/// `OSTYPE` `mac` (the value 3.153's Mac binary writes).
pub const LOBBY_420_MAC: Lobby = Lobby { platform: "mac", after_login: Some(LobbyLogin { os_props: ("mac", 1252), ..LOBBY_LOGIN_4X }), ..LOBBY_404 };
/// 5.00r7: the same login line as 4.x. Its after-login messages were not
/// recorded.
pub const LOBBY_500: Lobby = Lobby { after_login: None, ..LOBBY_404 };
/// 5.12 to 6.03r7: the quoted Windows version as a fifth field.
pub const LOBBY_512: Lobby = Lobby { os_version: Some("\"6.2 9200 \""), ..LOBBY_500 };

/// The Linux 6.03r7: `linux`, then four machine ids (MD5 hex of the `eth0`
/// MAC, MD5 hex of the inode of `/etc`, an empty field, and the kernel
/// release and version), read in the binary (Graal_analysis
/// `protocol/v6x-clients.md`), never captured. Seihai sends all four as
/// `""`, as it does the Windows hashes; the kernel string is not invented.
pub const LOBBY_603_LINUX: Lobby = Lobby { platform: "linux", os_version: Some(EMPTY_MACHINE_FIELD), ..LOBBY_512 };

/// The machine field Seihai sends for each of the three hashes.
pub const EMPTY_MACHINE_FIELD: &str = "\"\"";

impl Lobby {
    /// The bytes that follow the password on the login line (before its
    /// `\n`): `win,"","",""` and, from 5.12, `,"6.2 9200 "`.
    pub fn login_tail(&self) -> Vec<u8> {
        let mut out = self.platform.as_bytes().to_vec();
        for _ in 0..3 {
            out.push(b',');
            out.extend_from_slice(EMPTY_MACHINE_FIELD.as_bytes());
        }
        if let Some(v) = self.os_version {
            out.push(b',');
            out.extend_from_slice(v.as_bytes());
        }
        out
    }
}

/// True while a lobby build would be in Login mode on a server of this
/// name: the name starts with "login", in any case (the draw-layer test read
/// in 4.04 and 5.x).
pub fn is_login_mode(server_name: &str) -> bool {
    server_name.as_bytes().get(..5).is_some_and(|p| p.eq_ignore_ascii_case(b"login"))
}

/// Split a `PLO_SERVERWARP` (178) text the way the lobby builds read it:
/// on commas only. The lab warp `0,Lab Server,127.0.0.1,14900` (unquoted)
/// reached 127.0.0.1:14900 on 5.00r7, 5.12, 5.24r4, 5.31r4 and 6.03r7, and
/// their window titles read "Lab Server", so the space did not split the
/// field as it does in the 2.x comma-text reader. How these builds treat
/// quotes was not seen; a quoted field keeps its quotes here.
pub fn split_warp_fields(payload: &[u8]) -> Vec<&[u8]> {
    let payload = payload.strip_suffix(b"\n").unwrap_or(payload);
    payload.split(|&b| b == b',').collect()
}

/// What a 4.x build sends after its login line and once the server's login
/// burst is in. Lab, 2026-10-31: the real 4.04r2 and 4.20r8 under Wine on
/// GServer-v2 (`documentation/v4-profiles.md`, "Live run", and the
/// follow-up run in "After-login messages"). Both builds sent, on the
/// Login server and again on the game server after the warp:
///
/// 1. with `PLI_LANGUAGE` (37), in the frame after the login line:
///    `PLI_WANTFILE` (23) `basepackage.gupd` twice, then `bigmap.txt` in the
///    first session only;
/// 2. once the burst is in (`PLO_LISTPROCESSES`, 182), in four frames a few
///    milliseconds apart:
///    * 23 `idle.gani` and `PLI_UPDATEGANI` (157) `idle`;
///    * the head image check, then `PLI_REQUESTTEXT` (152)
///      `GraalEngine,pmguilds,""` and `GraalEngine,pmservers,""` and
///      `PLI_SENDTEXT` (154) with the lister options and `verifybuddies`;
///    * `PLI_ENTERLEVEL` (46);
///    * `PLI_PROCESSLIST` (44, the client's window titles), the default
///      weapons' image checks and the own props: `NICKNAME` (0), `STATUS`
///      (18), `UDPPORT` (31, 0), `ADDITFLAGS` (33), `PLANGUAGE` (52),
///      `OSTYPE` (75) and `TEXTCODEPAGE` (76).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LobbyLogin {
    /// Files asked for in the frame of `PLI_LANGUAGE`, every session.
    pub after_language: &'static [&'static str],
    /// Files asked for after those in the first session only (the real
    /// clients asked `bigmap.txt` on the Login server and not again after
    /// the warp).
    pub first_session_files: &'static [&'static str],
    /// The id and text of each PM and lister setup message, in order.
    pub texts: &'static [(u8, &'static str)],
    /// `OSTYPE` (75) and `TEXTCODEPAGE` (76) in the own props.
    pub os_props: (&'static str, u32),
}

/// `PLI_REQUESTTEXT`.
pub const PLI_REQUESTTEXT: u8 = 152;
/// `PLI_SENDTEXT`.
pub const PLI_SENDTEXT: u8 = 154;
/// `PLI_UPDATEGANI`: a GUINT5 checksum of the GANI the client has (0 for
/// none), then the name without `.gani`. GServer-v2 answers with the
/// GANI when the checksum differs, then 195 (`SETBACKTO`).
pub const PLI_UPDATEGANI: u8 = 157;

/// The real 4.04r2 and 4.20r8 (lab, 2026-10-31).
pub const LOBBY_LOGIN_4X: LobbyLogin = LobbyLogin {
    after_language: &["basepackage.gupd", "basepackage.gupd"],
    first_session_files: &["bigmap.txt"],
    texts: &[
        (PLI_REQUESTTEXT, "GraalEngine,pmguilds,\"\""),
        (PLI_REQUESTTEXT, "GraalEngine,pmservers,\"\""),
        (PLI_SENDTEXT, "GraalEngine,lister,options,globalpms=true,buddytracking=true,showbuddies=true"),
        (PLI_SENDTEXT, "GraalEngine,lister,verifybuddies,1,0"),
    ],
    os_props: ("win", 1252),
};

fn latin1(text: &str) -> impl Iterator<Item = u8> + '_ {
    text.chars().map(|c| if (c as u32) < 0x100 { c as u8 } else { b'?' })
}

fn gstring(out: &mut Vec<u8>, text: &str) {
    let b: Vec<u8> = latin1(text).take(223).collect();
    out.push(b.len() as u8 + 0x20);
    out.extend(b);
}

/// One text message (152 or 154): the id, then the text as it is.
pub fn text_message(id: u8, text: &str) -> Vec<u8> {
    let mut m = vec![id.wrapping_add(0x20)];
    m.extend(latin1(text));
    m.push(b'\n');
    m
}

/// `PLI_UPDATEGANI` (157) for a GANI the client has no copy of: checksum
/// 0 (five `GCHAR 0`), then the name without `.gani`, as the real 4.x
/// clients send it next to their `WANTFILE` for the GANI.
pub fn update_gani_message(gani: &str) -> Vec<u8> {
    let name = gani.strip_suffix(".gani").unwrap_or(gani);
    let mut m = vec![PLI_UPDATEGANI + 0x20, 0x20, 0x20, 0x20, 0x20, 0x20];
    m.extend(latin1(name));
    m.push(b'\n');
    m
}

/// After each `PLI_WANTFILE` (23) for a `.gani` in `msgs`, the matching
/// `PLI_UPDATEGANI` (157): the real 4.x clients send the pair whenever they
/// ask for a GANI (`idle` after the login, `walk` on the first step,
/// `sword` and `shoot` on their first use).
pub fn with_gani_updates(msgs: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let mut out = Vec::with_capacity(msgs.len());
    for m in msgs {
        let gani = (m.first() == Some(&(23 + 0x20)))
            .then(|| m[1..].strip_suffix(b"\n").unwrap_or(&m[1..]).to_vec())
            .filter(|n| n.to_ascii_lowercase().ends_with(b".gani"));
        out.push(m);
        if let Some(n) = gani {
            let name: String = n.iter().map(|&b| b as char).collect();
            out.push(update_gani_message(&name));
        }
    }
    out
}

/// `PLI_PROCESSLIST` (44) as the 4.x clients send it: the titles of the
/// client's windows, comma separated, unquoted (`Message,Open URL,...`).
/// A title with a comma or a quote is quoted, as gtokenize does.
pub fn window_list_message(titles: &[&str]) -> Vec<u8> {
    let mut m = vec![44 + 0x20];
    for (i, t) in titles.iter().enumerate() {
        if i > 0 {
            m.push(b',');
        }
        if t.is_empty() || t.contains([',', '"']) {
            m.push(b'"');
            m.extend(latin1(&t.replace('"', "\"\"")));
            m.push(b'"');
        } else {
            m.extend(latin1(t));
        }
    }
    m.push(b'\n');
    m
}

/// The own props of a 4.x build after the login: `NICKNAME` (0), `STATUS`
/// (18) without the paused and dead bits, `UDPPORT` (31) 0, `ADDITFLAGS`
/// (33) 0, `PLANGUAGE` (52), `OSTYPE` (75) and `TEXTCODEPAGE` (76). The
/// real clients sent `Real404` (the start screen's nickname field), STATUS
/// 20, UDPPORT 0, `English`, `win` and 1252; no head, colours or body.
pub fn lobby_props_message(info: &LobbyLogin, own_props: &[u8], nickname: &str, accepted: Option<&super::IdSet>) -> Vec<u8> {
    let props = super::props::parse_player_props_in(own_props, false, accepted);
    let mut m = vec![2 + 0x20, 0x20];
    gstring(&mut m, nickname);
    if let Some(b) = props.iter().find(|p| p.id == crate::life::PLPROP_STATUS).and_then(|p| p.raw.first()) {
        m.extend([18 + 0x20, crate::offline::server_status_base(b.saturating_sub(0x20)) + 0x20]);
    }
    m.extend([31 + 0x20, 0x20, 0x20, 0x20]);
    m.extend([33 + 0x20, 0x20]);
    m.push(52 + 0x20);
    gstring(&mut m, super::login::LANGUAGE);
    let (os, cp) = info.os_props;
    m.push(75 + 0x20);
    gstring(&mut m, os);
    m.push(76 + 0x20);
    m.extend([((cp >> 14) & 0x7F) as u8 + 0x20, ((cp >> 7) & 0x7F) as u8 + 0x20, (cp & 0x7F) as u8 + 0x20]);
    m.push(b'\n');
    m
}

/// Decides when a 4.x build's after-burst frames go out: once per session,
/// when `PLO_LISTPROCESSES` (182) has come, or
/// [`super::login::FRAME_AFTER_LEVEL`] after the first level when the
/// server sends no 182 (then without the window list, which answers 182).
#[derive(Debug, Clone)]
pub struct LobbyBurst {
    info: LobbyLogin,
    sent: bool,
    level_at: Option<std::time::Instant>,
}

impl LobbyBurst {
    /// `None` for a build without recorded after-login messages.
    pub fn for_version(version: super::ClientVersion) -> Option<Self> {
        let p = version.profile();
        let info = p.protocol.lobby?.after_login?;
        p.protocol.pli.contains(2).then_some(Self { info, sent: false, level_at: None })
    }

    pub fn info(&self) -> LobbyLogin {
        self.info
    }

    /// The frames have not gone out yet.
    pub fn pending(&self) -> bool {
        !self.sent
    }

    /// The first level came in at `at`. Later calls keep the first time.
    pub fn note_level(&mut self, at: std::time::Instant) {
        self.level_at.get_or_insert(at);
    }

    /// The frames are due at `now`.
    pub fn due_at(&self, state: &super::SessionState, now: std::time::Instant) -> bool {
        !self.sent
            && (state.process_list_requests > 0
                || self.level_at.is_some_and(|t| now.saturating_duration_since(t) >= super::login::FRAME_AFTER_LEVEL))
    }

    /// The four frames, once, when due (empty frames left out). `check`
    /// gives the file messages for a name (with the 157 of a GANI), and
    /// `enter_level` the login 46 when one is owed. `windows` are the
    /// window titles for 44.
    pub fn take(
        &mut self,
        state: &super::SessionState,
        nickname: &str,
        now: std::time::Instant,
        mut check: impl FnMut(&str) -> Vec<Vec<u8>>,
        enter_level: impl FnOnce() -> Vec<Vec<u8>>,
        windows: &[&str],
    ) -> Vec<Vec<Vec<u8>>> {
        if !self.due_at(state, now) {
            return Vec::new();
        }
        self.sent = true;
        let mut frames = vec![check("idle.gani")];
        let mut setup: Vec<Vec<u8>> = super::login::head_image(&state.player_props, state.own_prop_ids.as_ref()).map(|h| check(&h)).unwrap_or_default();
        setup.extend(self.info.texts.iter().map(|&(id, t)| text_message(id, t)));
        frames.push(setup);
        frames.push(enter_level());
        let mut last = Vec::new();
        if state.process_list_requests > 0 {
            last.push(window_list_message(windows));
        }
        let mut images: Vec<&str> = Vec::new();
        for n in state.default_weapons.iter().filter_map(|&id| super::login::default_weapon_image(id)) {
            if !images.contains(&n) {
                images.push(n);
            }
        }
        for n in images {
            last.extend(check(n));
        }
        last.push(lobby_props_message(&self.info, &state.player_props, nickname, state.own_prop_ids.as_ref()));
        frames.push(last);
        frames.retain(|f| !f.is_empty());
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tails() {
        assert_eq!(LOBBY_404.login_tail(), b"win,\"\",\"\",\"\"");
        assert_eq!(LOBBY_420_MAC.login_tail(), b"mac,\"\",\"\",\"\"");
        assert_eq!(LOBBY_500.login_tail(), b"win,\"\",\"\",\"\"");
        assert_eq!(LOBBY_512.login_tail(), b"win,\"\",\"\",\"\",\"6.2 9200 \"");
        assert_eq!(LOBBY_603_LINUX.login_tail(), b"linux,\"\",\"\",\"\",\"\"");
    }

    #[test]
    fn login_mode_by_name() {
        assert!(is_login_mode("Login"));
        assert!(is_login_mode("LOGIN server"));
        assert!(is_login_mode("loginserver"));
        assert!(!is_login_mode("Lab Server"));
        assert!(!is_login_mode("Logi"));
        assert!(!is_login_mode(""));
    }

    /// The bytes the lab proxy injected (`rules.py`, `WARP`).
    #[test]
    fn lab_warp_splits_on_commas_only() {
        let f = split_warp_fields(b"0,Lab Server,127.0.0.1,14900");
        assert_eq!(f, [&b"0"[..], b"Lab Server", b"127.0.0.1", b"14900"]);
    }

    use crate::protocol::{ClientVersion, SessionState};
    use std::time::{Duration, Instant};

    /// The real 4.04r2's own props (lab, 2026-10-31, `proxy-404-and-420login.log`):
    /// nickname `Real404`, STATUS 20, UDPPORT 0, ADDITFLAGS 0, `English`, `win`, 1252.
    #[test]
    fn props_match_the_real_404() {
        let own = [18 + 0x20, 20 + 0x20];
        let m = lobby_props_message(&LOBBY_LOGIN_4X, &own, "Real404", None);
        assert_eq!(m, b"\x22 'Real40424?   A T'Englishk#winl )\x84\n");
        // The server's paused and dead bits do not go back.
        let own = [18 + 0x20, 29 + 0x20];
        assert_eq!(lobby_props_message(&LOBBY_LOGIN_4X, &own, "Real404", None), m);
    }

    #[test]
    fn text_gani_and_window_messages() {
        assert_eq!(text_message(PLI_REQUESTTEXT, "GraalEngine,pmguilds,\"\""), b"\xb8GraalEngine,pmguilds,\"\"\n");
        assert_eq!(text_message(PLI_SENDTEXT, "GraalEngine,lister,verifybuddies,1,0"), b"\xbaGraalEngine,lister,verifybuddies,1,0\n");
        assert_eq!(update_gani_message("idle.gani"), b"\xbd     idle\n");
        assert_eq!(update_gani_message("walk"), b"\xbd     walk\n");
        let msgs = with_gani_updates(vec![b"7steps.wav\n".to_vec(), b"7walk.gani\n".to_vec()]);
        assert_eq!(msgs, [b"7steps.wav\n".to_vec(), b"7walk.gani\n".to_vec(), b"\xbd     walk\n".to_vec()]);
        assert_eq!(window_list_message(&["Message", "Open URL", "Graal 4.04r2 - Login"]), b"LMessage,Open URL,Graal 4.04r2 - Login\n");
        assert_eq!(window_list_message(&["a,b"]), b"L\"a,b\"\n");
    }

    #[test]
    fn which_builds_have_it() {
        for v in [ClientVersion::V404R2, ClientVersion::V420R6, ClientVersion::V420R8] {
            let b = LobbyBurst::for_version(v).unwrap_or_else(|| panic!("{}", v.label()));
            let pli = &v.profile().protocol.pli;
            for id in [2, 23, 24, 44, 46, 47, PLI_REQUESTTEXT, PLI_SENDTEXT, PLI_UPDATEGANI] {
                assert!(pli.contains(id), "{} {id}", v.label());
            }
            assert_eq!(b.info().os_props.0, if v == ClientVersion::V420R6 { "mac" } else { "win" });
        }
        assert!(LobbyBurst::for_version(ClientVersion::V500R7).is_none());
        assert!(LobbyBurst::for_version(ClientVersion::V603R7).is_none());
        assert!(LobbyBurst::for_version(ClientVersion::V231).is_none());
    }

    /// The four frames, in the real 4.04r2's order, once 182 is in.
    #[test]
    fn the_frames_after_the_burst() {
        let mut st = SessionState::for_version(ClientVersion::V404R2);
        let mut b = LobbyBurst::for_version(ClientVersion::V404R2).unwrap();
        let now = Instant::now();
        let none = |_: &str| -> Vec<Vec<u8>> { unreachable!() };
        assert!(b.take(&st, "n", now, none, Vec::new, &[]).is_empty());
        st.player_props = [&[11 + 0x20, 9 + 100 + 0x20][..], b"head0.png", &[18 + 0x20, 20 + 0x20]].concat();
        st.default_weapons = vec![8, 7];
        st.process_list_requests = 1;
        let check = |n: &str| with_gani_updates(vec![crate::net::wantfile_message(n)]);
        let f = b.take(&st, "SeihaiNick", now, check, || vec![b"N!\n".to_vec()], &["Graal 4.04r2 - Login"]);
        let text = |m: &Vec<u8>| String::from_utf8_lossy(m).into_owned();
        let f: Vec<Vec<String>> = f.iter().map(|fr| fr.iter().map(text).collect()).collect();
        assert_eq!(f[0], ["7idle.gani\n", "\u{fffd}     idle\n"]);
        assert_eq!(f[1][0], "7head0.png\n");
        assert_eq!(f[1].len(), 5);
        assert!(f[1][4].ends_with("GraalEngine,lister,verifybuddies,1,0\n"));
        assert_eq!(f[2], ["N!\n"]);
        assert_eq!(f[3][0], "LGraal 4.04r2 - Login\n");
        assert_eq!(f[3][1..3], ["7wbomb1.png\n", "7wbowi1.png\n"]);
        assert!(f[3][3].starts_with("\" *SeihaiNick24?   A T"));
        assert!(!b.pending());
        assert!(b.take(&st, "n", now, |_: &str| Vec::new(), Vec::new, &[]).is_empty());
    }

    /// No 182: the frames go 240 ms after the level, without the window list.
    #[test]
    fn late_without_182() {
        let st = SessionState::for_version(ClientVersion::V420R8);
        let mut b = LobbyBurst::for_version(ClientVersion::V420R8).unwrap();
        let t0 = Instant::now();
        b.note_level(t0);
        assert!(!b.due_at(&st, t0 + Duration::from_millis(100)));
        let f = b.take(&st, "x", t0 + Duration::from_millis(250), |_: &str| Vec::new(), Vec::new, &["w"]);
        assert_eq!(f.len(), 2);
        assert_eq!(f[1].len(), 1);
        assert_eq!(f[1][0][0], 2 + 0x20);
    }
}
