//! Every known login token, from the Graal_analysis catalogue.
//!
//! [`ClientVersion`] covers the builds Seihai has a profile for. This table
//! covers every token the Graal_analysis catalogue
//! (`client-internals/version-token-catalogue.md`) records, with or without a
//! profile: game logins, the listserver hellos, the 1.25 RemoteControl, the
//! Graal3D program and the one captured 6.11 connector-script token. It is
//! the single list the other Seihai repositories should take tokens from; the
//! generated copy is `documentation/version-tokens.tsv` (a unit test fails
//! when it is stale).

use super::ClientVersion;

/// How strongly a token is evidenced (the catalogue's grades).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenGrade {
    /// W: seen in the original client's first login frame in a lab run.
    Wire,
    /// B: read from the original executable; its login not captured.
    Binary,
    /// B+: read from the binary, and the original logged in to a lab server
    /// whose allow-list names each token, but the frame was not decoded.
    BinaryLab,
    /// L: only on the public version list or in gs2lib's table.
    ListOnly,
    /// S: the client stores no token; the server's connector script supplies it.
    ServerDefined,
}

impl TokenGrade {
    /// The catalogue's letter for this grade.
    pub fn letter(self) -> &'static str {
        match self {
            Self::Wire => "W",
            Self::Binary => "B",
            Self::BinaryLab => "B+",
            Self::ListOnly => "L",
            Self::ServerDefined => "S",
        }
    }
}

/// What a token is sent for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// A game client's login to a game server.
    Game,
    /// A client's hello to the listserver (id 7, with `newmain`); never sent
    /// to a game server.
    ListserverHello,
    /// A RemoteControl login.
    RemoteControl,
    /// Graal3D (2000), a separate program with its own server.
    Graal3D,
    /// Sent by a 6.11+ client because the server's connector script says so.
    ServerScript,
}

impl TokenKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Game => "game",
            Self::ListserverHello => "listserver",
            Self::RemoteControl => "rc",
            Self::Graal3D => "graal3d",
            Self::ServerScript => "server-script",
        }
    }
}

/// One token and the builds that send it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownToken {
    pub token: &'static str,
    pub grade: TokenGrade,
    pub kind: TokenKind,
    /// The builds that send it, as the catalogue names them.
    pub builds: &'static str,
    /// The Seihai profiles that present as this token (empty if none).
    pub versions: &'static [ClientVersion],
}

impl KnownToken {
    const fn new(
        token: &'static str,
        grade: TokenGrade,
        kind: TokenKind,
        builds: &'static str,
        versions: &'static [ClientVersion],
    ) -> Self {
        Self {
            token,
            grade,
            kind,
            builds,
            versions,
        }
    }
}

/// Every known token, oldest build first.
pub const KNOWN_TOKENS: &[KnownToken] = {
    use ClientVersion as V;
    use TokenGrade as G;
    use TokenKind as K;
    &[
        KnownToken::new("GR3DV002", G::Wire, K::Graal3D, "Graal3D (Feb 2000)", &[]),
        KnownToken::new(
            "GR3DV003",
            G::Binary,
            K::Graal3D,
            "Graal3D later beta (Genesis3D renderer)",
            &[],
        ),
        KnownToken::new("GSERV007", G::Wire, K::Game, "1.21", &[V::V121]),
        KnownToken::new("GSERV009", G::Binary, K::Game, "1.25", &[]),
        KnownToken::new(
            "GSERV010",
            G::Binary,
            K::RemoteControl,
            "RemoteControl shipped with 1.25",
            &[],
        ),
        KnownToken::new(
            "GSERV011",
            G::Binary,
            K::Game,
            "1.27 (its RemoteControl, later RC 1.28, and RC 1.3 send it too)",
            &[],
        ),
        KnownToken::new("GSERV013", G::Wire, K::Game, "1.28 beta 2", &[V::V128B2]),
        KnownToken::new("GSERV014", G::Wire, K::Game, "1.3 test", &[V::V13T]),
        KnownToken::new("GSERV016", G::Wire, K::Game, "1.31", &[V::V131]),
        KnownToken::new("GSERV018", G::Wire, K::Game, "1.32", &[V::V132]),
        KnownToken::new("GNW20020", G::Wire, K::Game, "1.33", &[V::V133]),
        KnownToken::new("GNW22030", G::BinaryLab, K::Game, "1.34", &[V::V134]),
        KnownToken::new("GNW13040", G::BinaryLab, K::Game, "1.35", &[V::V135]),
        KnownToken::new("GNW10050", G::BinaryLab, K::Game, "1.36", &[V::V136]),
        KnownToken::new("GNW21050", G::BinaryLab, K::Game, "1.37", &[V::V137]),
        KnownToken::new("GNW22060", G::BinaryLab, K::Game, "1.38", &[V::V138]),
        KnownToken::new(
            "GNW07080",
            G::Wire,
            K::Game,
            "1.39 (and one FileVersion 1.3.9.1 executable)",
            &[V::V139],
        ),
        KnownToken::new(
            "GNW12080",
            G::ListOnly,
            K::Game,
            "1.391 (gs2lib only; the 1.3.9.1 executables read send GNW07080 or GNW23090)",
            &[],
        ),
        KnownToken::new("GNW23090", G::Binary, K::Game, "1.40, 1.40 rev1", &[]),
        KnownToken::new("GNW13110", G::Wire, K::Game, "1.41 rev1", &[V::V141]),
        KnownToken::new("GNW08011", G::Wire, K::Game, "NewMain 24, 26", &[V::V142]),
        KnownToken::new(
            "GNW24011",
            G::Wire,
            K::Game,
            "2.00, 2.00 rev1-rev3",
            &[V::V200],
        ),
        KnownToken::new(
            "GNW23021",
            G::Wire,
            K::Game,
            "GraalX (2.01 pre-release), 2.01, 2.01 rev1",
            &[V::V201],
        ),
        KnownToken::new("GNW15041", G::Wire, K::Game, "2.02, 2.02 rev1", &[V::V202]),
        KnownToken::new("GNW20041", G::Wire, K::Game, "2.02 rev2", &[V::V202R2]),
        KnownToken::new("GNW06051", G::Wire, K::Game, "2.03", &[V::V203]),
        KnownToken::new("GNW24061", G::Wire, K::Game, "2.04", &[V::V204]),
        KnownToken::new(
            "GNW03081",
            G::Wire,
            K::Game,
            "2.05, 2.05 rev1, 2.05 rev2, 2.1 test 2 and 3, Graal 2002 ver7-ver9",
            &[V::V205],
        ),
        KnownToken::new(
            "G2N15041",
            G::Wire,
            K::Game,
            "NewWorld beta 1 (pre-update and final), beta 2",
            &[],
        ),
        KnownToken::new(
            "GNW31101",
            G::Binary,
            K::Game,
            "2.1.0.0 public release, 2.1.0.0-2.1.0.4 (Graal 2002 ver10, ver13, v22)",
            &[V::V210],
        ),
        KnownToken::new("GNW01012", G::ListOnly, K::Game, "2.12", &[V::V212]),
        KnownToken::new("GNW23012", G::ListOnly, K::Game, "2.13", &[V::V213]),
        KnownToken::new("GNW30042", G::ListOnly, K::Game, "2.14", &[V::V214]),
        KnownToken::new("GNW19052", G::ListOnly, K::Game, "2.15.0", &[V::V215]),
        KnownToken::new("GNW20052", G::ListOnly, K::Game, "2.15.1, 2.15.2", &[]),
        KnownToken::new("GNW12102", G::ListOnly, K::Game, "2.16", &[V::V216]),
        KnownToken::new("GNW22122", G::Wire, K::Game, "2.17.0, 2.17.1", &[V::V217]),
        KnownToken::new("GNW21033", G::ListOnly, K::Game, "2.18", &[]),
        KnownToken::new("GNW15053", G::ListOnly, K::Game, "2.19", &[]),
        KnownToken::new("GNW28063", G::ListOnly, K::Game, "2.2", &[]),
        KnownToken::new("GNW01113", G::ListOnly, K::Game, "2.21", &[]),
        KnownToken::new("GNW03014", G::Wire, K::Game, "2.22", &[V::V222]),
        KnownToken::new("GNW14015", G::ListOnly, K::Game, "2.3", &[V::V230]),
        KnownToken::new("GNW28015", G::Wire, K::Game, "2.31", &[V::V231]),
        KnownToken::new(
            "GNW30123",
            G::Wire,
            K::ListserverHello,
            "2.22 and 2.31 listserver hello (with newmain)",
            &[],
        ),
        KnownToken::new("G3D16053", G::Binary, K::Game, "3.0 Linux (3.0beta)", &[]),
        KnownToken::new("G3D27063", G::Binary, K::Game, "3.01 Linux", &[]),
        KnownToken::new("G3D03014", G::Wire, K::Game, "3.04 rev1 Linux", &[V::V304]),
        KnownToken::new("G3D27015", G::Binary, K::Game, "3.151 Linux", &[]),
        KnownToken::new("G3D28015", G::Wire, K::Game, "3.153 Mac (PPC)", &[V::V3153]),
        KnownToken::new(
            "G3D30123",
            G::Wire,
            K::ListserverHello,
            "3.x listserver hello (with newmain)",
            &[],
        ),
        KnownToken::new("G3D28095", G::ListOnly, K::Game, "4.02.11", &[]),
        KnownToken::new("G3D11125", G::Binary, K::Game, "4.03r1 Linux", &[]),
        KnownToken::new(
            "G3D09125",
            G::Binary,
            K::Game,
            "4.03r3 Windows (4.0.3.3; public list 4.03.4)",
            &[],
        ),
        KnownToken::new("G3D17026", G::Wire, K::Game, "4.04r2 Windows", &[V::V404R2]),
        KnownToken::new(
            "G3D18026",
            G::Wire,
            K::Game,
            "4.04r2 Mac (PPC)",
            &[V::V404R2Mac],
        ),
        KnownToken::new("G3D19026", G::Wire, K::Game, "4.04r2 Linux", &[]),
        KnownToken::new(
            "G3D28076",
            G::Binary,
            K::Game,
            "4.10r2 Linux, 4.11 Linux",
            &[],
        ),
        KnownToken::new(
            "G3D26076",
            G::Binary,
            K::Game,
            "4.11 Windows (4.1.1.0)",
            &[],
        ),
        KnownToken::new(
            "G3D27076",
            G::Wire,
            K::Game,
            "Mac PPC build of 2006-09-19 (labelled 4.20r6, really 4.10r2/4.11 era), 4.10r2 Mac",
            &[V::V420R6],
        ),
        KnownToken::new(
            "G3D21126",
            G::Binary,
            K::Game,
            "4.20r6 Mac (PPC, binary 2006-12-21)",
            &[],
        ),
        KnownToken::new("G3D22126", G::Wire, K::Game, "4.20r6 and 4.20r8 Linux", &[]),
        KnownToken::new("G3D20126", G::Wire, K::Game, "4.20r8 Windows", &[V::V420R8]),
        KnownToken::new("G3D22067", G::Wire, K::Game, "5.00r7 Windows", &[V::V500R7]),
        KnownToken::new("G3D14097", G::Wire, K::Game, "5.12 Windows", &[V::V512]),
        KnownToken::new(
            "G3D15097",
            G::Wire,
            K::Game,
            "5.12 Mac (PPC)",
            &[V::V512Mac],
        ),
        KnownToken::new("G3D03048", G::Wire, K::Game, "6.00 beta 1 Windows", &[]),
        KnownToken::new(
            "G3D15010",
            G::Wire,
            K::Game,
            "5.24r3, 5.24r4, 6.00 beta 2 Windows",
            &[V::V524R4],
        ),
        KnownToken::new(
            "G3D26090",
            G::Wire,
            K::Game,
            "5.30r3, 5.31r4, 6.00r2, 6.00r4 Windows",
            &[V::V530R3, V::V531R4, V::V600R2, V::V600R4],
        ),
        KnownToken::new(
            "G3D27090",
            G::Binary,
            K::Game,
            "6.004 Mac (June 2011 beta)",
            &[],
        ),
        KnownToken::new(
            "G3D28090",
            G::Binary,
            K::Game,
            "6.x Linux beta (x86, x64; June 2011, probably 6.00r4)",
            &[],
        ),
        KnownToken::new(
            "G3D3007A",
            G::Wire,
            K::Game,
            "6.0.0.7, 6.0.1.5 Windows",
            &[V::V6015],
        ),
        KnownToken::new("G3D3107A", G::Binary, K::Game, "6.015 Mac", &[]),
        KnownToken::new("G3D0108A", G::Wire, K::Game, "6.01r1 Linux (x86, x64)", &[]),
        KnownToken::new(
            "G3D2505C",
            G::Binary,
            K::Game,
            "6.0.3.4 Windows",
            &[V::V6034],
        ),
        KnownToken::new("G3D0311C", G::Wire, K::Game, "6.03r7 Windows", &[V::V603R7]),
        KnownToken::new(
            "G3D0511C",
            G::Wire,
            K::Game,
            "6.03r7 Linux x64",
            &[V::V603R7Linux],
        ),
        KnownToken::new(
            "G3D04048",
            G::Binary,
            K::Game,
            "iPhone 2008 engine: Belote 1.2/1.50 and 14 more builds, 2008-2010 (gs2lib iPhone 1.1)",
            &[],
        ),
        KnownToken::new(
            "G3D0208A",
            G::Binary,
            K::Game,
            "iPhone GraalOnline Classic+ 1.13 (not in gs2lib)",
            &[],
        ),
        KnownToken::new(
            "G3D18010",
            G::Binary,
            K::Game,
            "iPhone 2010 engine: Cocoto Kart Online 1.3, Cocoto Kart HD 1.1, Cocoto Magic Circus 1.0, Ace Tennis HD 1.0, Bomber Online 1.40, Dino Smash 1.4 (gs2lib iPhone 1.5)",
            &[],
        ),
        KnownToken::new(
            "G3D29090",
            G::ListOnly,
            K::Game,
            "iPhone 1.11 (gs2lib name; app not identified)",
            &[],
        ),
        KnownToken::new(
            "G3D2504D",
            G::ListOnly,
            K::Game,
            "iPhone Belote 1.5 (gs2lib name; not what Belote 1.50 sends)",
            &[],
        ),
        KnownToken::new(
            "GNP1905C",
            G::ServerDefined,
            K::ServerScript,
            "6.1.1.3 Windows and 6.11r1 Mac (from the one captured connector script)",
            &[],
        ),
    ]
};

/// The catalogue row for a token, if it is known.
pub fn known_token(token: &str) -> Option<&'static KnownToken> {
    KNOWN_TOKENS.iter().find(|t| t.token == token)
}

/// `documentation/version-tokens.tsv`: one row per token.
pub fn tokens_tsv() -> String {
    let mut out = String::from("token\tgrade\tkind\tbuilds\tseihai_profiles\n");
    for t in KNOWN_TOKENS {
        let profiles: Vec<&str> = t.versions.iter().map(|v| v.label()).collect();
        let profiles = if profiles.is_empty() {
            "-".to_string()
        } else {
            profiles.join(", ")
        };
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            t.token,
            t.grade.letter(),
            t.kind.name(),
            t.builds,
            profiles
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unique_and_eight_chars() {
        let mut seen = std::collections::HashSet::new();
        for t in KNOWN_TOKENS {
            assert_eq!(t.token.len(), 8, "{}", t.token);
            assert!(
                t.token.bytes().all(|b| b.is_ascii_alphanumeric()),
                "{}",
                t.token
            );
            assert!(seen.insert(t.token), "{} listed twice", t.token);
        }
    }

    /// Every profile's token is in the table, and the row names that profile.
    #[test]
    fn every_profile_is_catalogued() {
        for &v in ClientVersion::all_versions() {
            let row = known_token(v.token())
                .unwrap_or_else(|| panic!("{}: {} not catalogued", v.label(), v.token()));
            assert_eq!(row.kind, TokenKind::Game, "{}", v.label());
            assert!(
                row.versions.contains(&v),
                "{}: row {} does not name it",
                v.label(),
                row.token
            );
        }
        for t in KNOWN_TOKENS {
            for v in t.versions {
                assert_eq!(v.token(), t.token, "{} names {}", t.token, v.label());
            }
        }
    }

    /// The Mac build seihai labels 4.20r6 is the 4.10r2-era `G3D27076`; the
    /// real Mac 4.20r6 sends `G3D21126` and has no profile yet.
    #[test]
    fn mac_420r6_label() {
        assert_eq!(
            known_token("G3D27076").unwrap().versions,
            &[ClientVersion::V420R6]
        );
        assert!(known_token("G3D21126").unwrap().versions.is_empty());
    }

    const TSV_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../documentation/version-tokens.tsv"
    );

    #[test]
    fn version_tokens_tsv_is_current() {
        let doc = std::fs::read_to_string(TSV_PATH).unwrap_or_default();
        assert!(
            doc == tokens_tsv(),
            "documentation/version-tokens.tsv is stale; run: cargo test -p seihai-core write_version_tokens_tsv -- --ignored"
        );
    }

    /// Rewrites `documentation/version-tokens.tsv`.
    #[test]
    #[ignore]
    fn write_version_tokens_tsv() {
        std::fs::write(TSV_PATH, tokens_tsv()).unwrap();
    }
}
