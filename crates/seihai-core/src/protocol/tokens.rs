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
    /// C: only a constant in a captured connector script's table.
    ScriptConstant,
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
            Self::ScriptConstant => "C",
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
    /// An NPC Control login (the NPC socket of an RC, or the standalone tool).
    NpcControl,
    /// The NPC-server's own login to the game server.
    NpcServer,
}

impl TokenKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Game => "game",
            Self::ListserverHello => "listserver",
            Self::RemoteControl => "rc",
            Self::Graal3D => "graal3d",
            Self::ServerScript => "server-script",
            Self::NpcControl => "nc",
            Self::NpcServer => "npc-server",
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
        KnownToken::new(
            "GR3DV001",
            G::ListOnly,
            K::Graal3D,
            "Graal3D beta 1 (a label the archived server knows and refuses)",
            &[],
        ),
        KnownToken::new("GSERV007", G::Wire, K::Game, "1.21", &[V::V121]),
        KnownToken::new(
            "GSERV008",
            G::ListOnly,
            K::Game,
            "unidentified GSERV-era client the 1.25 server accepts",
            &[],
        ),
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
        KnownToken::new(
            "GSERV012",
            G::ListOnly,
            K::Game,
            "unidentified GSERV-era client in the 1.34 rev 1 server's table",
            &[],
        ),
        KnownToken::new("GSERV013", G::Wire, K::Game, "1.28 beta 2", &[V::V128B2]),
        KnownToken::new("GSERV014", G::Wire, K::Game, "1.3 test", &[V::V13T]),
        KnownToken::new(
            "GSERV015",
            G::ListOnly,
            K::Game,
            "unidentified client the December 1999 Linux server admits (not a known 1.30)",
            &[],
        ),
        KnownToken::new("GSERV016", G::Wire, K::Game, "1.31", &[V::V131]),
        KnownToken::new("GSERV018", G::Wire, K::Game, "1.32", &[V::V132]),
        KnownToken::new("GNW20020", G::Wire, K::Game, "1.33", &[V::V133]),
        KnownToken::new(
            "GNW28020",
            G::ListOnly,
            K::Game,
            "1.33 rev? (1.34 rev 1 server's table only)",
            &[],
        ),
        KnownToken::new("GNW22030", G::BinaryLab, K::Game, "1.34", &[V::V134]),
        KnownToken::new(
            "GNW27030",
            G::ListOnly,
            K::Game,
            "1.34 rev 1 (the 1.34 rev 1 server only)",
            &[],
        ),
        KnownToken::new("GNW13040", G::BinaryLab, K::Game, "1.35", &[V::V135]),
        KnownToken::new("GNW10050", G::BinaryLab, K::Game, "1.36", &[V::V136]),
        KnownToken::new("GNW21050", G::BinaryLab, K::Game, "1.37", &[V::V137]),
        KnownToken::new("GNW22060", G::BinaryLab, K::Game, "1.38", &[V::V138]),
        KnownToken::new(
            "GNW02070",
            G::ListOnly,
            K::Game,
            "1.38 rev 1 (the 1.39 server's table only)",
            &[],
        ),
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
            "G2N29070",
            G::ListOnly,
            K::Game,
            "NewWorld 1.39 (the 1.39 server tree only)",
            &[],
        ),
        KnownToken::new(
            "G2NRC004",
            G::ListOnly,
            K::RemoteControl,
            "NewWorld RemoteControl, 2000-08-19 label (server tree only)",
            &[],
        ),
        KnownToken::new(
            "G2N15041",
            G::Wire,
            K::Game,
            "NewWorld beta 1 (pre-update and final), beta 2",
            &[],
        ),
        KnownToken::new(
            "GSERV019",
            G::Binary,
            K::RemoteControl,
            "RemoteControl 2000-08-19",
            &[],
        ),
        KnownToken::new(
            "GSERV020",
            G::Binary,
            K::RemoteControl,
            "RemoteControl 2000-10-25",
            &[],
        ),
        KnownToken::new(
            "GSERV021",
            G::Binary,
            K::RemoteControl,
            "RemoteControl 2001-01-27",
            &[],
        ),
        KnownToken::new(
            "NCL15031",
            G::Binary,
            K::NpcControl,
            "standalone NPC Control 2001-03-19, 2001-05-21, 2001-07-13",
            &[],
        ),
        KnownToken::new(
            "G2NRC006",
            G::Binary,
            K::RemoteControl,
            "NewWorld RemoteControl 2001-05-21, 2001-08-20",
            &[],
        ),
        KnownToken::new(
            "GSERV022",
            G::Binary,
            K::RemoteControl,
            "RemoteControl 2001-08-20, 2002-01-04",
            &[],
        ),
        KnownToken::new(
            "GSERV023",
            G::Binary,
            K::RemoteControl,
            "RemoteControl 2002-01-11, 2002-05-02 (1.41)",
            &[],
        ),
        KnownToken::new(
            "NCL11012",
            G::Binary,
            K::NpcControl,
            "NPC socket of RC 2002-01-11 / 1.41, Delphi RC 2004 and RC 2005-01-24 Linux",
            &[],
        ),
        KnownToken::new(
            "GRNS0000",
            G::ListOnly,
            K::NpcServer,
            "NPC-server (generic label in the server library's table)",
            &[],
        ),
        KnownToken::new(
            "LNX00001",
            G::ListOnly,
            K::NpcServer,
            "NPC-server (Linux label in the server library's table)",
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
        KnownToken::new(
            "GSERV024",
            G::Binary,
            K::RemoteControl,
            "Delphi RemoteControl 2004-01-17 (and community variants), RemoteControl 2005-01-24 Linux",
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
        KnownToken::new(
            "GSERV025",
            G::Binary,
            K::RemoteControl,
            "RemoteControl 2006-12-20 (RC2 2.05), 2010-03-29 Linux, RC3",
            &[],
        ),
        KnownToken::new(
            "NCL21075",
            G::Binary,
            K::NpcControl,
            "NPC socket of RC 2006-12-20 and RC 2010-03-29 Linux",
            &[],
        ),
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
            "G3D2204D",
            G::ListOnly,
            K::ServerScript,
            "Worlds (gs2lib's CLVER_WORLDS; also in the captured connector script's legacy table)",
            &[],
        ),
        KnownToken::new(
            "G3D2304D",
            G::ScriptConstant,
            K::ServerScript,
            "captured connector script's legacy table only",
            &[],
        ),
        KnownToken::new(
            "G3D2404D",
            G::ScriptConstant,
            K::ServerScript,
            "captured connector script's legacy table only",
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

/// Game tokens with no profile of their own, and the profile a game server
/// serves them as: the nearest build Seihai has a profile for, on the same
/// encryption generation. These are stand-ins, not measurements: each one
/// should get its own profile once the build is read or captured.
///
/// With [`NOT_SERVED`] and the profiled rows, this accounts for every
/// [`TokenKind::Game`] and [`TokenKind::ServerScript`] token (a unit test
/// fails when a new one is not placed).
pub const STAND_INS: &[(&str, ClientVersion)] = {
    use ClientVersion as V;
    &[
        // 1.25 and 1.27: the gen-2 GSERV line, nearest profile 1.21.
        ("GSERV009", V::V121),
        ("GSERV011", V::V121),
        // Server-table revisions with no executable: their own release's profile.
        ("GNW28020", V::V133),
        ("GNW27030", V::V134),
        ("GNW02070", V::V138),
        // 1.391 (gs2lib only) and 1.40 / 1.40 rev1.
        ("GNW12080", V::V139),
        ("GNW23090", V::V141),
        // 2.15.1/2.15.2 as 2.15.0; 2.18 to 2.21 as 2.17.1 (no binary held).
        ("GNW20052", V::V215),
        ("GNW21033", V::V217),
        ("GNW15053", V::V217),
        ("GNW28063", V::V217),
        ("GNW01113", V::V217),
        // 3.0beta and 3.01 Linux as 3.04; 3.151 Linux as 3.153.
        ("G3D16053", V::V304),
        ("G3D27063", V::V304),
        ("G3D27015", V::V3153),
        // 4.02.11 to 4.04r2 Linux: protocol 19, as 4.04r2.
        ("G3D28095", V::V404R2),
        ("G3D11125", V::V404R2),
        ("G3D09125", V::V404R2),
        ("G3D19026", V::V404R2),
        // 4.10r2 / 4.11 Windows and Linux as the 4.10r2-era Mac build.
        ("G3D28076", V::V420R6),
        ("G3D26076", V::V420R6),
        // The real 4.20r6 Mac and the 4.20r6/r8 Linux builds as 4.20r8.
        ("G3D21126", V::V420R8),
        ("G3D22126", V::V420R8),
        // 6.00 beta 1 as 6.00 beta 2's token (5.24r4); the 2011 Mac and Linux
        // betas as 6.00r4; 6.015 Mac and 6.01r1 Linux as 6.0.1.5.
        ("G3D03048", V::V524R4),
        ("G3D27090", V::V600R4),
        ("G3D28090", V::V600R4),
        ("G3D3107A", V::V6015),
        ("G3D0108A", V::V6015),
        // 6.1.1.3 / 6.11r1: the original logs in through a connector script
        // (Graal_analysis#397); a rebuild that sends the token directly is
        // served as 6.03r7.
        ("GNP1905C", V::V603R7),
    ]
};

/// Game and connector-script tokens no game server serves, and why.
pub const NOT_SERVED: &[(&str, &str)] = &[
    (
        "GSERV008",
        "no client build is known for it; only the 1.25 server accepts it",
    ),
    (
        "GSERV012",
        "no client build is known for it; only a server table names it",
    ),
    (
        "GSERV015",
        "no client build is known for it; do not infer a 1.30",
    ),
    ("G2N15041", "NewWorld line: no profile yet"),
    (
        "G2N29070",
        "NewWorld line: no profile yet, and no executable",
    ),
    (
        "G3D04048",
        "iPhone: mobile framing and cipher unread (Graal_analysis#648)",
    ),
    (
        "G3D0208A",
        "iPhone: mobile framing and cipher unread (Graal_analysis#648)",
    ),
    (
        "G3D18010",
        "iPhone: mobile framing and cipher unread (Graal_analysis#648)",
    ),
    (
        "G3D29090",
        "iPhone: mobile framing and cipher unread (Graal_analysis#648)",
    ),
    (
        "G3D2504D",
        "iPhone: mobile framing and cipher unread (Graal_analysis#648)",
    ),
    (
        "G3D2204D",
        "Worlds (v7): logs in only through a connector script",
    ),
    ("G3D2304D", "connector-script constant with no known client"),
    ("G3D2404D", "connector-script constant with no known client"),
];

/// The profile a game server should serve `token` as: the token's own
/// profile (the oldest, when several builds share it), else its entry in
/// [`STAND_INS`]. `None` for tokens in [`NOT_SERVED`], non-game tokens and
/// unknown ones.
pub fn serve_as(token: &str) -> Option<ClientVersion> {
    known_token(token)
        .and_then(|t| t.versions.first().copied())
        .or_else(|| STAND_INS.iter().find(|(t, _)| *t == token).map(|&(_, v)| v))
}

/// The catalogue row for a token, if it is known.
pub fn known_token(token: &str) -> Option<&'static KnownToken> {
    KNOWN_TOKENS.iter().find(|t| t.token == token)
}

/// `documentation/version-tokens.tsv`: one row per token.
pub fn tokens_tsv() -> String {
    let mut out = String::from("token\tgrade\tkind\tbuilds\tseihai_profiles\tserved_as\n");
    for t in KNOWN_TOKENS {
        let profiles: Vec<&str> = t.versions.iter().map(|v| v.label()).collect();
        let profiles = if profiles.is_empty() {
            "-".to_string()
        } else {
            profiles.join(", ")
        };
        let served = match (
            serve_as(t.token),
            NOT_SERVED.iter().find(|(n, _)| *n == t.token),
        ) {
            (Some(v), _) => v.label().to_string(),
            (None, Some((_, why))) => format!("not served: {why}"),
            (None, None) => "-".to_string(),
        };
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            t.token,
            t.grade.letter(),
            t.kind.name(),
            t.builds,
            profiles,
            served
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

    /// Every game and connector-script token is accounted for exactly once:
    /// it has a profile, a stand-in, or a reason it is not served. Adding a
    /// token to [`KNOWN_TOKENS`] without placing it fails here.
    #[test]
    fn every_game_token_is_accounted_for() {
        let stand_in = |tok: &str| STAND_INS.iter().find(|(t, _)| *t == tok);
        let refused = |tok: &str| NOT_SERVED.iter().find(|(t, _)| *t == tok);
        for t in KNOWN_TOKENS {
            let placed = [
                !t.versions.is_empty(),
                stand_in(t.token).is_some(),
                refused(t.token).is_some(),
            ];
            match t.kind {
                TokenKind::Game | TokenKind::ServerScript => assert_eq!(
                    placed.iter().filter(|&&p| p).count(),
                    1,
                    "{} ({}): needs exactly one of a profile, a STAND_INS entry or a NOT_SERVED reason",
                    t.token,
                    t.builds
                ),
                _ => assert!(
                    placed == [false; 3],
                    "{} is not a game token; it takes no profile or stand-in",
                    t.token
                ),
            }
        }
        for (tok, v) in STAND_INS {
            assert!(
                known_token(tok).is_some(),
                "stand-in {tok} is not catalogued"
            );
            assert!(
                ClientVersion::all_versions().contains(v),
                "stand-in {tok}: {} has no profile",
                v.label()
            );
        }
        for (tok, why) in NOT_SERVED {
            assert!(known_token(tok).is_some(), "{tok} is not catalogued");
            assert!(!why.is_empty());
        }
    }

    #[test]
    fn serve_as_prefers_the_own_profile() {
        assert_eq!(serve_as("GNW03014"), Some(ClientVersion::V222));
        assert_eq!(serve_as("G3D26090"), Some(ClientVersion::V530R3));
        assert_eq!(serve_as("G3D27015"), Some(ClientVersion::V3153));
        assert_eq!(serve_as("GNP1905C"), Some(ClientVersion::V603R7));
        assert_eq!(serve_as("G3D0208A"), None);
        assert_eq!(serve_as("GSERV025"), None);
        assert_eq!(serve_as("ZZZZ0000"), None);
    }

    /// A stand-in is served on its profile's generation, so it must not be a
    /// token some other profile already claims.
    #[test]
    fn stand_ins_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (t, _) in STAND_INS {
            assert!(seen.insert(*t), "{t} listed twice");
        }
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
