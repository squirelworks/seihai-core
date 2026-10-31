//! Client versions.
//!
//! The version switch. What each version does is in `profile.rs`.

use super::profile::{ListKind, Listserver, PortStatus, VersionProfile};

/// Encryption generation used on the game socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptionGen {
    /// No compression (through ~1.1.9).
    Gen1,
    /// zlib only (~1.21–1.39).
    Gen2,
    /// zlib + one LCG-inserted byte per client→server message (~1.41–2.05/2.17).
    Gen3,
    /// Full-frame partial XOR (2.22+).
    Gen5,
}

/// A client version Seihai can present as.
///
/// Everything that differs between versions lives in that version's row in
/// [`super::profile::PROFILES`]; the methods here read from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientVersion {
    /// Graal 1.21 (`GSERV007`). Gen 2, the oldest build with logins.
    V121,
    /// Graal 1.28 beta 2 (`GSERV013`). Gen 2.
    V128B2,
    /// Graal 1.3 test (`GSERV014`). Gen 2.
    V13T,
    /// Graal 1.31 (`GSERV016`). Gen 2.
    V131,
    /// Graal 1.32 (`GSERV018`). Gen 2.
    V132,
    /// Graal 1.33 (`GNW20020`). Gen 2.
    V133,
    /// Graal 1.34 (`GNW22030`). Gen 2.
    V134,
    /// Graal 1.35 (`GNW13040`). Gen 2.
    V135,
    /// Graal 1.36 (`GNW10050`). Gen 2.
    V136,
    /// Graal 1.37 (`GNW21050`). Gen 2.
    V137,
    /// Graal 1.38 (`GNW22060`). Gen 2.
    V138,
    /// Graal 1.39 (`GNW07080`). Gen 2.
    V139,
    /// Graal 1.41 (`GNW13110`). First gen 3.
    V141,
    /// Graal 1.42 NewMain (`GNW08011`). Gen 3.
    V142,
    /// Graal 2.00 (`GNW24011`). Gen 3.
    V200,
    /// Graal 2.01 (`GNW23021`). Gen 3.
    V201,
    /// Graal 2.02 (`GNW15041`), base build and rev 1. Gen 3.
    V202,
    /// Graal 2.02 rev 2 (`GNW20041`). Gen 3. First build that sends `PLI_LANGUAGE`.
    V202R2,
    /// Graal 2.03 (`GNW06051`). Gen 3.
    V203,
    /// Graal 2.04 (`GNW24061`). Gen 3.
    V204,
    /// Graal 2.05 rev 2 (`GNW03081`). Gen 3.
    V205,
    /// Graal 2.1 (`GNW31101`). Gen 3. No binary; inferred.
    V210,
    /// Graal 2.12 (`GNW01012`). Gen 3. No binary; inferred.
    V212,
    /// Graal 2.13 (`GNW23012`). Gen 3. No binary; inferred.
    V213,
    /// Graal 2.14 (`GNW30042`). Gen 3. No binary; inferred.
    V214,
    /// Graal 2.15 (`GNW19052`). Gen 3. No binary; inferred.
    V215,
    /// Graal 2.16 (`GNW12102`). Gen 3. No binary; inferred.
    V216,
    /// Graal 2.17 (`GNW22122`). Gen 3.
    V217,
    /// Graal 2.22 (`GNW03014`). Gen 5.
    V222,
    /// Graal 2.3 (`GNW14015`). Gen 5. No binary; inferred from 2.31.
    V230,
    /// Graal 2.31 (`GNW28015`). Gen 5.
    V231,
    /// Graal 3.04 (`G3D03014`), the Linux build. Gen 5. The 3.x twin of 2.22.
    V304,
    /// Graal 3.153 (`G3D28015`), the Mac OS X build. Gen 5. The 3.x twin of 2.31.
    V3153,
    /// Graal 4.04 rev 2 (`G3D17026`), Windows. Gen 5. The first build that
    /// logs in to a game server named `Login` and plays only after a server
    /// warp (178).
    V404R2,
    /// Graal 4.20 rev 6 (`G3D27076`), the Mac OS X build. Gen 5.
    V420R6,
    /// Graal 4.20 rev 8 (`G3D20126`), Windows. Gen 5.
    V420R8,
    /// Graal 5.00 rev 7 (`G3D22067`), Windows. Gen 5. Logs in to a game
    /// server named `Login` and plays only after a server warp (178).
    V500R7,
    /// Graal 5.12 (`G3D14097`), Windows. Gen 5, with its own round-down
    /// cipher step count on compressed frames.
    V512,
    /// Graal 5.24 rev 4 (`G3D15010`), Windows. Gen 5.
    V524R4,
    /// Graal 5.30 rev 3 (`G3D26090`), Windows. Gen 5.
    V530R3,
    /// Graal 5.31 rev 4 (`G3D26090`), Windows. Gen 5.
    V531R4,
    /// Graal 6.00 rev 2 (`G3D26090`), Windows. Gen 5.
    V600R2,
    /// Graal 6.03 rev 7 (`G3D0311C`), Windows. Gen 5.
    V603R7,
}

impl ClientVersion {
    /// This version's profile row.
    pub fn profile(self) -> &'static VersionProfile {
        super::profile::profile(self)
    }

    /// Eight-character version token sent at login.
    pub fn token(self) -> &'static str {
        self.profile().token
    }

    pub fn client_type(self) -> u8 {
        self.profile().protocol.client_type
    }

    pub fn encryption(self) -> EncryptionGen {
        self.profile().protocol.encryption
    }

    /// `PLO_FILE` carries a modtime for clients GServer counts as 2.1+.
    pub fn file_has_modtime(self) -> bool {
        self.profile().files.modtime
    }

    pub fn sends_tampercheck(self) -> bool {
        self.profile().protocol.tampercheck
    }

    /// The build can send `PLI_ENTERLEVEL` (46): its profile's send set has
    /// the id (2.3 and 2.31 only). Read from `pli`, not from the version.
    pub fn sends_enter_level(self) -> bool {
        self.profile().protocol.pli.contains(crate::net::PLI_ENTERLEVEL)
    }

    /// The build asks for a level it already holds with `PLI_LEVELWARPMOD`
    /// (30): its profile says so and its send set has the id.
    pub fn sends_cached_links(self) -> bool {
        let p = self.profile();
        p.levels.cached_links && p.protocol.pli.contains(crate::net::PLI_LEVELWARPMOD)
    }

    /// The client-side death sequence this build runs, if its handler was
    /// traced (see [`crate::protocol::DeathRules`]).
    pub fn death_rules(self) -> Option<crate::protocol::DeathRules> {
        self.profile().life.death
    }

    /// How this build judges hits from other players, if its handler was
    /// traced (see [`crate::protocol::CombatRules`]).
    pub fn combat_rules(self) -> Option<crate::protocol::CombatRules> {
        self.profile().life.combat
    }

    /// The victim's client sends `PLI_CLAIMPKER` (14) when it dies to a
    /// known attacker: the build has the death sequence and the id is in its
    /// send set. Read from the profile, not from the version.
    pub fn sends_claim(self) -> bool {
        let p = self.profile();
        p.life.death.is_some() && p.protocol.pli.contains(crate::life::PLI_CLAIMPKER)
    }

    /// The listserver this version dials by default: its own, or 2.05's
    /// for versions without one (1.33 to 1.39).
    pub fn listserver(self) -> Listserver {
        let ls = self.list_version().profile().protocol.listserver;
        ls.unwrap_or(
            Listserver::new(ListKind::Legacy, PortStatus::Assumed, None, None).shape(super::profile::LoginShape::NewmainOnly, super::profile::ShapeBasis::Inferred),
        )
    }

    /// Default listserver port: 14923 for a profile that sends the classic
    /// login (1.41 to 2.17), 14922 for one that sends the gen-5 login (2.22
    /// and later). Never the dead 14921.
    pub fn listserver_port(self) -> u16 {
        self.listserver().port
    }

    /// Default listserver host: always `listserver.graal.in`.
    pub fn listserver_host(self) -> &'static str {
        self.listserver().host
    }

    /// Default `host:port` of this version's listserver.
    pub fn listserver_addr(self) -> String {
        self.listserver().addr()
    }

    pub fn listserver_token(self) -> &'static str {
        let p = self.profile();
        p.protocol.listserver.and_then(|l| l.token).unwrap_or(p.token)
    }

    /// A listserver address as given on the command line: `host:port` is
    /// kept exactly (an explicit port always wins), and a listserver host with
    /// no port gets this version's default port. Anything else is unchanged.
    pub fn complete_list_addr(self, addr: &str) -> String {
        if !addr.contains(':') && addr.contains("listserver") {
            format!("{addr}:{}", self.listserver_port())
        } else {
            addr.to_string()
        }
    }

    /// The version to fetch the server list as. 1.33–1.39 have no
    /// listserver, so they borrow 2.05's.
    pub fn list_version(self) -> ClientVersion {
        if self.profile().protocol.listserver.is_some() {
            self
        } else {
            Self::V205
        }
    }

    /// Display name for the GUI.
    pub fn label(self) -> &'static str {
        self.profile().label
    }

    /// The version with this GUI label.
    pub fn from_label(label: &str) -> Option<ClientVersion> {
        super::profile::by_label(label).map(|p| p.version)
    }

    /// Main versions shown by default.
    pub fn main_versions() -> &'static [ClientVersion] {
        &[Self::V139, Self::V205, Self::V217, Self::V222, Self::V231]
    }

    /// All versions with a profile, oldest first.
    pub fn all_versions() -> &'static [ClientVersion] {
        &[
            Self::V121, Self::V128B2, Self::V13T, Self::V131, Self::V132,
            Self::V133, Self::V134, Self::V135, Self::V136, Self::V137, Self::V138, Self::V139,
            Self::V141, Self::V142, Self::V200, Self::V201, Self::V202, Self::V202R2, Self::V203,
            Self::V204, Self::V205, Self::V210,
            Self::V212, Self::V213, Self::V214, Self::V215, Self::V216, Self::V217, Self::V222,
            Self::V230, Self::V231, Self::V304, Self::V3153,
            Self::V404R2, Self::V420R6, Self::V420R8,
            Self::V500R7, Self::V512, Self::V524R4, Self::V530R3, Self::V531R4, Self::V600R2, Self::V603R7,
        ]
    }
}

impl Default for ClientVersion {
    fn default() -> Self {
        Self::V205
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_eight_chars() {
        for v in [
            ClientVersion::V139,
            ClientVersion::V205,
            ClientVersion::V217,
            ClientVersion::V222,
            ClientVersion::V231,
        ] {
            assert_eq!(v.token().len(), 8);
        }
    }

    #[test]
    fn gen3_versions() {
        assert_eq!(ClientVersion::V205.encryption(), EncryptionGen::Gen3);
        assert_eq!(ClientVersion::V217.encryption(), EncryptionGen::Gen3);
        assert_eq!(ClientVersion::V222.encryption(), EncryptionGen::Gen5);
    }
}
