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
}

/// 4.04r2, 4.20r8 and 5.00r7: four fields (`win` and three MD5s, read in
/// the 4.x engines; `win,<md5>,"",<md5>` in the 5.00r7 lab capture).
pub const LOBBY_404: Lobby = Lobby { first_server: "Login", platform: "win", os_version: None };
/// The Mac 4.20r6: `mac`, a hash of the Ethernet MAC, a hash of
/// `stat("/etc")` and an empty fourth field (read in the binary, never run).
pub const LOBBY_420_MAC: Lobby = Lobby { platform: "mac", ..LOBBY_404 };
/// 5.00r7: the same as 4.x.
pub const LOBBY_500: Lobby = LOBBY_404;
/// 5.12 to 6.03r7: the quoted Windows version as a fifth field.
pub const LOBBY_512: Lobby = Lobby { os_version: Some("\"6.2 9200 \""), ..LOBBY_500 };

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tails() {
        assert_eq!(LOBBY_404.login_tail(), b"win,\"\",\"\",\"\"");
        assert_eq!(LOBBY_420_MAC.login_tail(), b"mac,\"\",\"\",\"\"");
        assert_eq!(LOBBY_500.login_tail(), b"win,\"\",\"\",\"\"");
        assert_eq!(LOBBY_512.login_tail(), b"win,\"\",\"\",\"\",\"6.2 9200 \"");
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
}
