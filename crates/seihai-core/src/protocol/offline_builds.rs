//! Old builds Seihai can only play offline.
//!
//! These are the Zelda Online and Graal clients from before 1.21 (and the
//! NewWorld betas) that have no row in [`super::profile::PROFILES`]: no
//! Seihai login reaches a server for them. The launcher lists them under
//! "All versions" with "(offline-only)", and picking one starts the offline
//! game with the rules of [`OfflineBuild::plays_as`] and that build's own
//! asset folder.
//!
//! Every fact here comes from `graal-decompile/archive-survey/build-catalogue.md`,
//! Graal_analysis `client-internals/precursors-and-fan-builds.md`, and the
//! room names read from each build's strings (2026-10-31). No wire bytes
//! are made up: none of these builds sends anything from Seihai.
//! See `documentation/offline-only-builds.md`.

use super::ClientVersion;

/// Where a build came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Stefan Knorr's Java applets (Zelda Online 1 and 2, Java Graal).
    Java,
    /// The 1998 and early 1999 Delphi betas, 1st Graal to 1.1.9 (gen 1).
    DelphiBeta,
    /// The NewWorld betas of 2001 (`G2N15041`).
    NewWorld,
}

/// Why a build has no online play in Seihai.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offline {
    /// The build has no networking at all.
    NoNetwork,
    /// The build talks to Knorr's GServ relay lobby (`N<room>#<name>`,
    /// then `D` lines relayed to the room). Seihai has no relay client, and
    /// no public relay is left.
    RelayLobby {
        /// The room players meet in: one per level. `<level>` is the level
        /// file name, `<base>` the name without its extension.
        room: &'static str,
        /// The Delphi betas open a second connection to one fixed room for
        /// the whole game (`Graal3` for 5th Graal); the Java builds do not.
        lobby: Option<&'static str>,
        /// The relay port the build dials (or pre-fills).
        port: u16,
    },
    /// The build logs in with a token no Seihai row has, and no check has
    /// shown a server accepting it.
    UnknownToken(&'static str),
}

/// One offline-only build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfflineBuild {
    /// Launcher label. Never the same as a profile label.
    pub label: &'static str,
    /// The archive it came from (without `.7z`).
    pub archive: &'static str,
    /// Link date (Delphi resource stamp) or class date (Java).
    pub date: &'static str,
    pub family: Family,
    pub offline: Offline,
    /// The level formats the build reads, as the catalogue gives them.
    pub levels: &'static str,
    /// The profile whose offline rules Seihai uses for it: level formats,
    /// HUD, editor. The closest build Seihai has a row for.
    pub plays_as: ClientVersion,
    /// The level the build starts on, where a source says so.
    pub start_level: Option<&'static str>,
}

impl OfflineBuild {
    /// The launcher text: label, date and "(offline-only)".
    pub fn display(&self) -> String {
        format!("{} ({}, offline-only)", self.label, self.date)
    }

    /// One line on why it is offline-only, for the launcher.
    pub fn reason(&self) -> String {
        match self.offline {
            Offline::NoNetwork => "This build has no networking.".to_string(),
            Offline::RelayLobby { room, port, .. } => format!(
                "This build joins room `{room}` on a GServ relay (port {port}). Seihai has no relay client and no public relay is left, so it plays offline."
            ),
            Offline::UnknownToken(t) => {
                format!("This build logs in as `{t}`, which no Seihai row has and no server check has accepted, so it plays offline.")
            }
        }
    }

    /// The per-level room the build joins on the relay, if it uses one.
    pub fn room(&self) -> Option<&'static str> {
        match self.offline {
            Offline::RelayLobby { room, .. } => Some(room),
            _ => None,
        }
    }
}

const fn relay(room: &'static str, lobby: Option<&'static str>, port: u16) -> Offline {
    Offline::RelayLobby { room, lobby, port }
}

const fn beta(
    label: &'static str,
    archive: &'static str,
    date: &'static str,
    room: &'static str,
    lobby: &'static str,
    levels: &'static str,
) -> OfflineBuild {
    OfflineBuild {
        label,
        archive,
        date,
        family: Family::DelphiBeta,
        // Every Delphi beta pre-fills 127.0.0.1:14801 (build-catalogue.md).
        offline: relay(room, Some(lobby), 14801),
        levels,
        plays_as: ClientVersion::V121,
        start_level: None,
    }
}

const Z3: &str = "Z3-V1.03/1.04";
const GR01: &str = "Z3-V1.03/1.04, GR-V1.00–1.01";
const GR02: &str = "Z3-V1.03/1.04, GR-V1.00–1.02";
const GR03: &str = "Z3-V1.03/1.04, GR-V1.00–1.03";

/// Every offline-only build, oldest first (by link date, not archive name).
pub const OFFLINE_BUILDS: &[OfflineBuild] = &[
    OfflineBuild {
        label: "Zelda Online 1",
        archive: "zeldaonline1",
        date: "1998-05",
        family: Family::Java,
        offline: Offline::NoNetwork,
        // Seihai's Z3-V1 reader takes 1.03 and 1.04 only; 1.02 levels do not open yet.
        levels: "Z3-V1.02/1.03",
        plays_as: ClientVersion::V121,
        start_level: Some("level1.zelda"),
    },
    OfflineBuild {
        label: "Zelda Online 2",
        archive: "zeldaonline2",
        date: "1998-07",
        family: Family::Java,
        offline: relay("Z<base>1.6", None, 14801),
        levels: "Z3-V1.03",
        plays_as: ClientVersion::V121,
        start_level: None,
    },
    beta("1st Graal", "1stgraal-1", "1998-09-06", "Z<base>", "Zelda2", Z3),
    beta("2nd Graal", "2ndgraal", "1998-09-16", "Z<base>", "Zelda2", GR01),
    beta("3rd Graal", "3rdgraal", "1998-09-30", "G<level>", "Graal", GR01),
    beta("4th Graal", "4thgraal", "1998-10-05", "G<level>", "Graal", GR01),
    beta("4th Graal alpha", "4thgraalalpha", "1998-10-12", "G2<level>", "Graal2", GR01),
    beta("5th Graal", "5thgraal", "1998-10-19", "G3<level>", "Graal3", GR02),
    beta("5th Graal alpha", "5thgraalalpha", "1998-10-29", "G3<level>", "Graal3", GR02),
    beta("6th Graal (beta 5)", "6thgraal", "1998-11-09", "G4<level>", "Graal4", GR02),
    beta("7th Graal (beta 7)", "7thgraal", "1998-12-04", "G5<level>", "Graal5", GR03),
    beta("8th Graal (beta 8)", "8thgraal", "1998-12-08", "G5<level>", "Graal5", GR03),
    beta("9th Graal alpha", "9thgraalalpha", "1998-12-16", "G5<level>", "Graal5", GR03),
    beta("9th Graal (beta 9)", "9thgraal", "1998-12-18", "G5<level>", "Graal5", GR03),
    beta("10th Graal (1.0)", "10thgraal", "1998-12-22", "G6<level>", "Graal6", GR03),
    beta("11th Graal (1.0)", "11thgraal", "1998-12-23", "G6<level>", "Graal6", GR03),
    beta("12th Graal (1.0)", "12thgraal", "1999-01-05", "G6<level>", "Graal6", GR03),
    beta("13th Graal (1.1 beta 1)", "13thgraal", "1999-01-12", "G7<level>", "Graal7", GR03),
    beta("14th Graal (1.1)", "14thgraal", "1999-01-15", "G8<level>", "Graal8", GR03),
    beta("15th Graal (1.1)", "15thgraal", "1999-01-25", "G8<level>", "Graal8", GR03),
    beta("1.1.9 Gold Edition", "graal1.1.9", "1999-02-11", "G8<level>", "Graal8", GR03),
    beta("1.1", "graal1.1", "1999-02-14", "G8<level>", "Graal8", GR03),
    OfflineBuild {
        label: "Java Graal",
        archive: "javagraal",
        date: "2000-07",
        family: Family::Java,
        // Only its own codebase host on 14802 (the Cyberjoueur relay).
        offline: relay("G2<level>", None, 14802),
        levels: GR03,
        plays_as: ClientVersion::V121,
        start_level: Some("level13.graal"),
    },
    OfflineBuild {
        label: "NewWorld beta 1",
        archive: "graalnwb1",
        date: "2001-08-06",
        family: Family::NewWorld,
        offline: Offline::UnknownToken("G2N15041"),
        levels: "as 2.05 (GR-V, GLEVNW01, GRMAP001)",
        plays_as: ClientVersion::V205,
        start_level: None,
    },
    OfflineBuild {
        label: "NewWorld beta 2",
        archive: "graalnwb2",
        date: "2001-10-03",
        family: Family::NewWorld,
        offline: Offline::UnknownToken("G2N15041"),
        levels: "as 2.05 (GR-V, GLEVNW01, GRMAP001)",
        plays_as: ClientVersion::V205,
        start_level: None,
    },
];

/// The offline-only build with this launcher label.
pub fn by_label(label: &str) -> Option<&'static OfflineBuild> {
    OFFLINE_BUILDS.iter().find(|b| b.label == label)
}

/// Whether two offline-only builds would meet on the same GServ relay: both
/// join the same per-level room name for a level (seen live 2026-10-31 for
/// Java Graal and 4th Graal alpha, both `G2<level>`).
pub fn share_a_room(a: &OfflineBuild, b: &OfflineBuild) -> bool {
    match (a.room(), b.room()) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_unique_and_never_a_profile_label() {
        for (i, b) in OFFLINE_BUILDS.iter().enumerate() {
            assert!(OFFLINE_BUILDS[..i].iter().all(|o| o.label != b.label), "{} twice", b.label);
            assert!(ClientVersion::from_label(b.label).is_none(), "{} is also a profile", b.label);
            assert_eq!(by_label(b.label), Some(b));
        }
    }

    #[test]
    fn oldest_first() {
        for w in OFFLINE_BUILDS.windows(2) {
            assert!(w[0].date <= w[1].date, "{} before {}", w[0].label, w[1].label);
        }
    }

    #[test]
    fn display_says_offline_only() {
        for b in OFFLINE_BUILDS {
            assert!(b.display().ends_with("offline-only)"));
            assert!(!b.reason().is_empty());
        }
    }

    /// Java Graal joins `G2<level>`, 5th Graal `G3<level>`. GServ links only
    /// players in the same room, so the two never meet on a stock relay (a
    /// bridge that renames `G2` to `G3` makes them meet: seen live
    /// 2026-10-31). 4th Graal alpha joins `G2<level>` too and meets Java
    /// Graal with no bridge (also seen live).
    #[test]
    fn java_graal_meets_4th_graal_alpha_not_5th_graal() {
        let java = by_label("Java Graal").unwrap();
        let fifth = by_label("5th Graal").unwrap();
        assert!(!share_a_room(java, fifth));
        assert!(share_a_room(java, by_label("4th Graal alpha").unwrap()));
        let g2: Vec<_> = OFFLINE_BUILDS.iter().filter(|b| share_a_room(java, b) && b.label != java.label).map(|b| b.label).collect();
        assert_eq!(g2, ["4th Graal alpha"]);
        assert!(share_a_room(fifth, by_label("5th Graal alpha").unwrap()));
        assert!(share_a_room(by_label("3rd Graal").unwrap(), by_label("4th Graal").unwrap()));
    }

    #[test]
    fn plays_as_a_profile_that_reads_their_levels() {
        for b in OFFLINE_BUILDS {
            let l = &b.plays_as.profile().levels;
            assert!(l.zelda, "{}", b.label);
            if b.levels.contains("GR-V") {
                assert!(l.graal, "{}", b.label);
            }
        }
    }
}
