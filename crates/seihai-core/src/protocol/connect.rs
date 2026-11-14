//! How each build finds and reaches a server (seihai-core#7, "Next 3").
//!
//! Four native ways are kept apart, because none implies another:
//!
//! * **No network**: the build has no networking (some pre-1.21 betas).
//! * **Relay lobby**: Knorr's GServ relay rooms (the Java and Delphi betas).
//! * **Direct game server**: the player types a host and port; the build
//!   has no listserver protocol (1.21 to 1.39).
//! * **Listserver**: a list protocol on its own port, classic (1.41 to 2.17)
//!   or gen-5 (2.22 to 3.153).
//! * **Login server**: no list protocol; a game connection to a server
//!   named `Login` shows the list as scripted GUI and warps the client on
//!   (4.04r2 on, [`super::lobby`]).
//!
//! Seihai's own server discovery is a separate question.
//! [`ClientVersion::list_version`] fetches the list as 2.05 for every row
//! without a listserver; that is a Seihai convenience
//! ([`Discovery::Borrowed`]) and says nothing about the original build.

use super::offline_builds::{Offline, OfflineBuild};
use super::{ClientVersion, ListKind, PortStatus};

/// How an original build reaches a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Native {
    NoNetwork,
    RelayLobby,
    /// The build logs in straight to a game server. `confirmed` is false
    /// for a build whose token no row has and no server was seen to accept.
    DirectGameServer {
        confirmed: bool,
    },
    Listserver {
        kind: ListKind,
        status: PortStatus,
    },
    LoginServer,
}

impl Native {
    pub fn label(self) -> &'static str {
        match self {
            Native::NoNetwork => "no network",
            Native::RelayLobby => "relay lobby",
            Native::DirectGameServer { confirmed: true } => "direct game server",
            Native::DirectGameServer { confirmed: false } => {
                "direct game server (unconfirmed token)"
            }
            Native::Listserver {
                kind: ListKind::Legacy,
                ..
            } => "listserver (classic)",
            Native::Listserver {
                kind: ListKind::Supported,
                ..
            } => "listserver (gen 5)",
            Native::LoginServer => "Login server",
        }
    }

    /// The build can be pointed at one game server by host and port.
    pub fn reaches_a_game_server(self) -> bool {
        !matches!(self, Native::NoNetwork | Native::RelayLobby)
    }
}

/// How Seihai finds servers for a profile row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discovery {
    /// With the build's own listserver protocol.
    Native,
    /// With another build's listserver login: a Seihai convenience only.
    Borrowed(ClientVersion),
}

impl ClientVersion {
    /// How the original build reaches a server, from its profile.
    pub fn native_connection(self) -> Native {
        let p = self.profile();
        if let Some(l) = p.protocol.listserver {
            Native::Listserver {
                kind: l.kind,
                status: l.status,
            }
        } else if p.protocol.lobby.is_some() {
            Native::LoginServer
        } else {
            Native::DirectGameServer { confirmed: true }
        }
    }

    /// How Seihai finds servers for this row.
    pub fn discovery(self) -> Discovery {
        match self.list_version() {
            v if v == self => Discovery::Native,
            v => Discovery::Borrowed(v),
        }
    }
}

impl OfflineBuild {
    /// How the original build reached a server.
    pub fn native_connection(&self) -> Native {
        match self.offline {
            Offline::NoNetwork => Native::NoNetwork,
            Offline::RelayLobby { .. } => Native::RelayLobby,
            Offline::UnknownToken(_) => Native::DirectGameServer { confirmed: false },
        }
    }
}

/// One line per profile row and offline-only build: label, native way,
/// Seihai discovery. Markdown, for `documentation/connection-capabilities.md`.
pub fn render() -> String {
    let mut out = String::from("| Build | Native connection | Seihai discovery |\n|---|---|---|\n");
    for b in super::offline_builds::OFFLINE_BUILDS {
        out.push_str(&format!(
            "| {} | {} | none (offline-only) |\n",
            b.label,
            b.native_connection().label()
        ));
    }
    for &v in ClientVersion::all_versions() {
        let d = match v.discovery() {
            Discovery::Native => "own listserver login".to_string(),
            Discovery::Borrowed(from) => format!("borrowed: lists as {}", from.label()),
        };
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            v.label(),
            v.native_connection().label(),
            d
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ClientVersion::*;

    #[test]
    fn the_ways_are_kept_apart() {
        for v in [V121, V133, V139] {
            assert_eq!(
                v.native_connection(),
                Native::DirectGameServer { confirmed: true }
            );
            // Seihai borrows 2.05's list, which is not the build's own.
            assert_eq!(v.discovery(), Discovery::Borrowed(V205));
        }
        assert!(matches!(
            V141.native_connection(),
            Native::Listserver {
                kind: ListKind::Legacy,
                ..
            }
        ));
        assert!(matches!(
            V222.native_connection(),
            Native::Listserver {
                kind: ListKind::Supported,
                ..
            }
        ));
        assert_eq!(V205.discovery(), Discovery::Native);
        for v in [V404R2, V420R8, V524R4, V603R7] {
            assert_eq!(v.native_connection(), Native::LoginServer);
            assert!(matches!(v.discovery(), Discovery::Borrowed(_)));
        }
    }

    #[test]
    fn a_borrowed_list_is_never_a_native_listserver() {
        for &v in ClientVersion::all_versions() {
            let native = matches!(v.native_connection(), Native::Listserver { .. });
            assert_eq!(native, v.discovery() == Discovery::Native, "{v:?}");
        }
    }

    #[test]
    fn offline_builds_never_reach_a_seihai_server_by_list() {
        for b in super::super::offline_builds::OFFLINE_BUILDS {
            let n = b.native_connection();
            assert!(
                !matches!(n, Native::Listserver { .. } | Native::LoginServer),
                "{}",
                b.label
            );
        }
    }

    #[test]
    fn the_documented_table_is_current() {
        let doc = include_str!("../../../../documentation/connection-capabilities.md")
            .replace("\r\n", "\n");
        assert!(
            doc.contains(&render()),
            "documentation/connection-capabilities.md is stale; paste \
             `cargo run -p seihai-core --example profile_audit -- connections` into it"
        );
    }
}
