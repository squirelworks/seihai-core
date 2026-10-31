//! The version chain.
//!
//! Every client build Seihai can present as is one **step** in an ordered
//! list, oldest first. A step is a name (label, token, link date, basis) and
//! a small [`Delta`]: only what that build changed against the build before
//! it. The first step starts from [`ORIGIN`], a fully spelled-out 1.33 row.
//! Resolving the chain (a `const fn`, so it costs nothing at run time) yields
//! the [`VersionProfile`] rows in [`super::profile::PROFILES`].
//!
//! Adding a version is one [`Step`] in [`CHAIN`], placed after the build it
//! follows; no logic changes. See `documentation/version-chain.md`.
//!
//! Beside the deltas, a step carries what is **not** known about it
//! ([`Unknown`]) and where its changes come from (`source`). Where a build
//! has no binary behind it, the delta repeats the previous step's value and
//! says so in `unknown`; nothing is made up to fill the gap.
//!
//! The per-step handler-change table ([`changes`], [`render`]) is generated
//! from the resolved rows by comparing each step with the one before. It is
//! never written by hand, so it cannot disagree with the profiles. It is
//! printed by `seihai --version-chain` and checked in as
//! `documentation/version-chain.tsv` (a unit test fails when it is stale).

use super::profile::{
    Basis, BuiltIn, ChatFace, ChatText, DeathRules, Files, Health, HudArt, IdSet, Levels, ListKind, Listserver, LoginShape, PortStatus, Props, Protocol, ServerList, ShapeBasis, Ui,
    BUILT_IN_14921, BUILT_IN_14922,
    ItemDrops, Mannerisms, VersionProfile, DROPS_133, DROPS_138, DROPS_142, COMBAT_200, COMBAT_201, COMBAT_205, MANNER_139, MANNER_141, MANNER_205, MANNER_222, DEATH_205, DEATH_21, SHOWIMG_217, SHOWIMG_222, SHOWIMG_231, SHOWIMG_404, TAMPER_217, TAMPER_222,
    TAMPER_230, TAMPER_231,
};
use super::lobby::{Lobby, LOBBY_404, LOBBY_420_MAC, LOBBY_500, LOBBY_512};
use super::login::{LoginInfo, LOGIN_217, LOGIN_222, LOGIN_231, LOGIN_304, LOGIN_3153};
use super::showimg::ShowImgInfo;
use super::tampercheck::TamperInfo;
use super::version::{ClientVersion, EncryptionGen};
use crate::difftest::names;

/// A change to an id set (server ids, client ids, prop ids): the ids a step
/// takes out, then the ids it puts in. Ranges are inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdDelta {
    pub minus: &'static [u8],
    pub minus_ranges: &'static [(u8, u8)],
    pub plus: &'static [u8],
    pub plus_ranges: &'static [(u8, u8)],
}

impl IdDelta {
    pub const NONE: IdDelta = IdDelta { minus: &[], minus_ranges: &[], plus: &[], plus_ranges: &[] };

    /// Ids added.
    pub const fn plus(ids: &'static [u8]) -> IdDelta {
        IdDelta { plus: ids, ..IdDelta::NONE }
    }

    /// Ids added and ids removed.
    pub const fn plus_minus(plus: &'static [u8], minus: &'static [u8]) -> IdDelta {
        IdDelta { plus, minus, ..IdDelta::NONE }
    }

    pub const fn is_none(&self) -> bool {
        self.minus.is_empty() && self.minus_ranges.is_empty() && self.plus.is_empty() && self.plus_ranges.is_empty()
    }

    /// Apply to a set: removals first, then additions.
    pub const fn apply(&self, set: IdSet) -> IdSet {
        let mut s = set.without_all(self.minus);
        let mut i = 0;
        while i < self.minus_ranges.len() {
            let (lo, hi) = self.minus_ranges[i];
            s = s.without_range(lo, hi);
            i += 1;
        }
        s = s.with_all(self.plus);
        i = 0;
        while i < self.plus_ranges.len() {
            let (lo, hi) = self.plus_ranges[i];
            s = s.with_range(lo, hi);
            i += 1;
        }
        s
    }

    /// Every id the delta removes and adds, expanded.
    #[cfg(test)]
    fn expanded(&self) -> (Vec<u8>, Vec<u8>) {
        let expand = |ids: &[u8], ranges: &[(u8, u8)]| {
            let mut v: Vec<u8> = ids.to_vec();
            for &(lo, hi) in ranges {
                v.extend(lo..=hi);
            }
            v
        };
        (expand(self.minus, self.minus_ranges), expand(self.plus, self.plus_ranges))
    }
}

/// What one build changed against the build before it. `None` means "as the
/// build before"; `Some(x)` sets the value. Nested options (`Some(None)`) set
/// an optional field to nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Delta {
    // Login, framing and the packets a version speaks.
    pub client_type: Option<u8>,
    pub encryption: Option<EncryptionGen>,
    pub listserver: Option<Option<Listserver>>,
    /// Server→client ids the client has a handler for.
    pub plo: IdDelta,
    /// Client→server ids the build can send.
    pub pli: IdDelta,
    pub sends_language: Option<bool>,
    pub tampercheck: Option<bool>,
    pub tamper: Option<Option<TamperInfo>>,
    pub showimg: Option<Option<ShowImgInfo>>,
    pub login: Option<Option<LoginInfo>>,
    pub lobby: Option<Option<Lobby>>,
    pub gen5_round_down: Option<bool>,
    pub gen5_long_frames: Option<bool>,
    pub udp: Option<bool>,
    pub udp_off_by_default: Option<bool>,
    // Props.
    pub own_props: IdDelta,
    pub other_props: IdDelta,
    pub npc_max: Option<u8>,
    pub bow_prop10: Option<bool>,
    pub precise_pos: Option<bool>,
    // Files.
    pub modtime: Option<bool>,
    pub large_files: Option<bool>,
    pub large_file_size: Option<bool>,
    pub image_ext: Option<&'static str>,
    pub download_maps: Option<bool>,
    // Levels.
    pub graal: Option<bool>,
    pub zelda: Option<bool>,
    pub nw: Option<bool>,
    pub gmap: Option<bool>,
    pub heights: Option<bool>,
    pub cached_links: Option<bool>,
    // Health.
    pub death: Option<Option<DeathRules>>,
    pub combat: Option<Option<super::profile::CombatRules>>,
    // UI.
    pub server_list: Option<ServerList>,
    pub chat_recall: Option<bool>,
    pub hud: Option<HudArt>,
    pub chat_text: Option<ChatText>,
    pub minimap: Option<bool>,
    pub editor: Option<bool>,
    pub ganis: Option<bool>,
    pub emoticons: Option<bool>,
    pub key_config: Option<bool>,
    pub log_window: Option<bool>,
    pub ghost_mode: Option<bool>,
    pub server_in_title: Option<bool>,
    pub gs1_commands: Option<u16>,
    // How the player moves and looks.
    pub mannerisms: Option<Mannerisms>,
    // Who drops bush and pot items.
    pub drops: Option<ItemDrops>,
}

impl Delta {
    /// No change: the build is as the one before.
    pub const NONE: Delta = Delta {
        client_type: None,
        encryption: None,
        listserver: None,
        plo: IdDelta::NONE,
        pli: IdDelta::NONE,
        sends_language: None,
        tampercheck: None,
        tamper: None,
        showimg: None,
        login: None,
        lobby: None,
        gen5_round_down: None,
        gen5_long_frames: None,
        udp: None,
        udp_off_by_default: None,
        own_props: IdDelta::NONE,
        other_props: IdDelta::NONE,
        npc_max: None,
        bow_prop10: None,
        precise_pos: None,
        modtime: None,
        large_files: None,
        large_file_size: None,
        image_ext: None,
        download_maps: None,
        graal: None,
        zelda: None,
        nw: None,
        gmap: None,
        heights: None,
        cached_links: None,
        death: None,
        combat: None,
        server_list: None,
        chat_recall: None,
        hud: None,
        chat_text: None,
        minimap: None,
        editor: None,
        ganis: None,
        emoticons: None,
        key_config: None,
        log_window: None,
        ghost_mode: None,
        server_in_title: None,
        gs1_commands: None,
        mannerisms: None,
        drops: None,
    };
}

/// Something about a step the notes do not settle: the field group and what
/// is missing. The value in the resolved row is carried from the step before
/// (or taken from a template) until a binary or a capture says otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unknown {
    pub area: &'static str,
    pub what: &'static str,
}

/// One build in the chain.
#[derive(Debug, Clone, Copy)]
pub struct Step {
    pub version: ClientVersion,
    /// Display name, e.g. "2.05".
    pub label: &'static str,
    /// Eight-character token sent at login.
    pub token: &'static str,
    /// Link date of the build, where known.
    pub date: &'static str,
    pub basis: Basis,
    /// What changed against the step before.
    pub delta: Delta,
    /// Where the delta comes from: the binary read, release notes, server
    /// source. Free text, one line.
    pub source: &'static str,
    /// What is not known about this step.
    pub unknown: &'static [Unknown],
}

const fn pick<T: Copy>(cur: T, d: Option<T>) -> T {
    match d {
        Some(v) => v,
        None => cur,
    }
}

/// Apply one step to the profile of the step before it.
pub const fn apply(prev: VersionProfile, s: &Step) -> VersionProfile {
    let d = &s.delta;
    let mut p = prev;
    p.version = s.version;
    p.label = s.label;
    p.token = s.token;
    p.date = s.date;
    p.basis = s.basis;

    p.protocol.client_type = pick(p.protocol.client_type, d.client_type);
    p.protocol.encryption = pick(p.protocol.encryption, d.encryption);
    p.protocol.listserver = pick(p.protocol.listserver, d.listserver);
    p.protocol.plo = d.plo.apply(p.protocol.plo);
    p.protocol.pli = d.pli.apply(p.protocol.pli);
    p.protocol.sends_language = pick(p.protocol.sends_language, d.sends_language);
    p.protocol.tampercheck = pick(p.protocol.tampercheck, d.tampercheck);
    p.protocol.tamper = pick(p.protocol.tamper, d.tamper);
    p.protocol.showimg = pick(p.protocol.showimg, d.showimg);
    p.protocol.login = pick(p.protocol.login, d.login);
    p.protocol.lobby = pick(p.protocol.lobby, d.lobby);
    p.protocol.gen5_round_down = pick(p.protocol.gen5_round_down, d.gen5_round_down);
    p.protocol.gen5_long_frames = pick(p.protocol.gen5_long_frames, d.gen5_long_frames);
    p.protocol.udp = pick(p.protocol.udp, d.udp);
    p.protocol.udp_off_by_default = pick(p.protocol.udp_off_by_default, d.udp_off_by_default);

    p.props.own = d.own_props.apply(p.props.own);
    p.props.other = d.other_props.apply(p.props.other);
    p.props.npc_max = pick(p.props.npc_max, d.npc_max);
    p.props.bow_prop10 = pick(p.props.bow_prop10, d.bow_prop10);
    p.props.precise_pos = pick(p.props.precise_pos, d.precise_pos);

    p.files.modtime = pick(p.files.modtime, d.modtime);
    p.files.large_files = pick(p.files.large_files, d.large_files);
    p.files.large_file_size = pick(p.files.large_file_size, d.large_file_size);
    p.files.image_ext = pick(p.files.image_ext, d.image_ext);
    p.files.download_maps = pick(p.files.download_maps, d.download_maps);

    p.levels.graal = pick(p.levels.graal, d.graal);
    p.levels.zelda = pick(p.levels.zelda, d.zelda);
    p.levels.nw = pick(p.levels.nw, d.nw);
    p.levels.gmap = pick(p.levels.gmap, d.gmap);
    p.levels.heights = pick(p.levels.heights, d.heights);
    p.levels.cached_links = pick(p.levels.cached_links, d.cached_links);

    p.life.death = pick(p.life.death, d.death);
    p.life.combat = pick(p.life.combat, d.combat);

    p.ui.server_list = pick(p.ui.server_list, d.server_list);
    p.ui.chat_recall = pick(p.ui.chat_recall, d.chat_recall);
    p.ui.hud = pick(p.ui.hud, d.hud);
    p.ui.minimap = pick(p.ui.minimap, d.minimap);
    p.ui.editor = pick(p.ui.editor, d.editor);
    p.ui.ganis = pick(p.ui.ganis, d.ganis);
    p.ui.emoticons = pick(p.ui.emoticons, d.emoticons);
    p.ui.key_config = pick(p.ui.key_config, d.key_config);
    p.ui.log_window = pick(p.ui.log_window, d.log_window);
    p.ui.ghost_mode = pick(p.ui.ghost_mode, d.ghost_mode);
    p.ui.server_in_title = pick(p.ui.server_in_title, d.server_in_title);
    p.ui.chat_text = pick(p.ui.chat_text, d.chat_text);
    p.ui.gs1_commands = pick(p.ui.gs1_commands, d.gs1_commands);
    p.mannerisms = pick(p.mannerisms, d.mannerisms);
    p.drops = pick(p.drops, d.drops);
    p
}

// -- The origin row: 1.33, spelled out in full. ----------------------------

/// Where the chain starts: 1.33, gen 2, no listserver, `.graal` levels only,
/// server ids 0–41 and 100, client ids 0–31. Every field is written out so a
/// reader sees one complete row before the deltas begin.
pub const ORIGIN: VersionProfile = VersionProfile {
    version: ClientVersion::V121,
    label: "1.21",
    token: "GSERV007",
    date: "1999-05-10",
    basis: Basis::Binary,
    protocol: Protocol {
        client_type: 0,
        encryption: EncryptionGen::Gen2,
        listserver: None,
        plo: IdSet::range(0, 29).with_range(31, 34).with(100),
        // The highest send id the binary passes as a constant; the ids below it
        // that it passes in a register are taken as sent (1.33 sends 0 to 31).
        pli: IdSet::range(0, 26),
        sends_language: false,
        tampercheck: false,
        tamper: None,
        showimg: None,
        login: None,
        lobby: None,
        gen5_round_down: false,
        gen5_long_frames: false,
        // No direct data packages before 1.28 beta 2.
        udp: false,
        udp_off_by_default: false,
    },
    props: Props {
        own: IdSet::range(0, 24),
        // The 2.05 other-player set cut to the own-prop range of 1.33, plus
        // 50 and 51 (read from the 1.33 binary).
        other: IdSet::EMPTY
            .with(0)
            .with(2)
            .with_range(8, 13)
            .with_range(15, 21)
            .with(24)
            .with_range(30, 32)
            .with_all(&[50, 51]),
        npc_max: 17,
        bow_prop10: true,
        precise_pos: false,
    },
    files: Files {
        modtime: false,
        large_files: false,
        large_file_size: false,
        image_ext: ".gif",
        download_maps: false,
    },
    levels: Levels { graal: true, zelda: true, nw: false, gmap: false, heights: false, cached_links: false },
    life: Health { death: None, combat: None },
    ui: Ui {
        server_list: ServerList::Direct { default: "www.graalonline.com:14804" },
        chat_recall: false,
        hud: HudArt::Letters,
        minimap: false,
        editor: true,
        ganis: false,
        emoticons: false,
        key_config: false,
        log_window: false,
        ghost_mode: false,
        server_in_title: false,
        chat_text: ChatText { face: ChatFace::Dialog, height: 28 },
        gs1_commands: 71,
    },
    // Read in 1.39; 1.33 to 1.38 were not run and take its row.
    mannerisms: MANNER_139,
    // The roll is in every build from the 4th alpha; 1.3 read.
    drops: DROPS_133,
};

// -- The chain. -------------------------------------------------------------

/// Every build, oldest first. The order is the order of
/// [`super::profile::PROFILES`] and of the version pickers.
pub const CHAIN: &[Step] = &[
    Step {
        version: ClientVersion::V121,
        label: "1.21",
        token: "GSERV007",
        date: "1999-05-10",
        basis: Basis::Binary,
        delta: Delta::NONE,
        source: "origin row (binary of 1.21, Graal_analysis client-internals/version-profiles/v121-v132.toml): login id 0 with token, account and password; zlib on every flush; no UDP; default server www.graalonline.com:14804",
        unknown: &[
            Unknown { area: "levels", what: "Z3-V1 (.zelda): every binary from 1st Graal to 2.31 compares the level magic with Z3-V1.03 and Z3-V1.04, so the column is yes for every build; whether a build also tries .zelda for a bare level name is in no binary (Seihai tries .graal, then .zelda)" },
            Unknown { area: "pli", what: "the binaries 1.21 to 1.32 show only the send ids passed as constants; the ids below the highest one are taken as sent" },
            Unknown { area: "other props", what: "1.21 to 1.32 have no separate parser for other players' props; 1.33's set is kept" },
            Unknown { area: "live", what: "not run: the 1.21 install is an InstallShield setup" },
        ],
    },
    Step {
        version: ClientVersion::V128B2,
        label: "1.28 beta 2",
        token: "GSERV013",
        date: "1999-11-23",
        basis: Basis::Binary,
        delta: Delta {
            plo: IdDelta::plus(&[30, 35, 36, 37, 38]),
            pli: IdDelta { plus_ranges: &[(27, 29)], ..IdDelta::NONE },
            udp: Some(true),
            own_props: IdDelta { plus_ranges: &[(25, 31)], ..IdDelta::NONE },
            npc_max: Some(32),
            gs1_commands: Some(84),
            ..Delta::NONE
        },
        source: "binary: PLO 30 and 35-38, send ids to 29, player props to 31, NPC props to 32, direct data packages (UDP ids 0, 10-22), language files, GS1 table 71 to 84",
        unknown: &[],
    },
    Step {
        version: ClientVersion::V13T,
        label: "1.3 test",
        token: "GSERV014",
        date: "1999-11-29",
        basis: Basis::Binary,
        delta: Delta {
            plo: IdDelta::plus(&[39]),
            pli: IdDelta::plus(&[30]),
            own_props: IdDelta::plus(&[32]),
            npc_max: Some(34),
            gs1_commands: Some(85),
            ..Delta::NONE
        },
        source: "binary: PLO 39, send id 30, player prop 32 (alignment), NPC props to 34, GS1 table 85 (setgifpart)",
        unknown: &[],
    },
    Step {
        version: ClientVersion::V131,
        label: "1.31",
        token: "GSERV016",
        date: "1999-12-07",
        basis: Basis::Binary,
        delta: Delta {
            plo: IdDelta::plus(&[40]),
            server_list: Some(ServerList::Direct { default: "www.graalonline.com:14900" }),
            ..Delta::NONE
        },
        source: "binary: PLO 40, default port 14900 (14804 before), noudp= and fixedudpport= options",
        unknown: &[],
    },
    Step {
        version: ClientVersion::V132,
        label: "1.32",
        token: "GSERV018",
        date: "2000-01-12",
        basis: Basis::Binary,
        delta: Delta { plo: IdDelta::plus(&[41]), own_props: IdDelta::plus(&[33]), ..Delta::NONE },
        source: "binary and lab: PLO 41, player prop 33 (ADDITFLAGS); the real 1.32 logged in to the gen-2 stub with the login props of 1.33 (2026-10-30)",
        unknown: &[],
    },
    Step {
        version: ClientVersion::V133,
        label: "1.33",
        token: "GNW20020",
        date: "2000-02-21",
        basis: Basis::Binary,
        delta: Delta { pli: IdDelta::plus(&[31]), gs1_commands: Some(88), ..Delta::NONE },
        source: "binary of 1.33: send id 31, GS1 table 88, GNW tokens from here on",
        unknown: &[],
    },
    Step {
        version: ClientVersion::V134,
        label: "1.34",
        token: "GNW22030",
        date: "2000-03-23",
        basis: Basis::Binary,
        delta: Delta { other_props: IdDelta::plus(&[34]), gs1_commands: Some(93), ..Delta::NONE },
        source: "binary: other-player prop 34 (ACCOUNTNAME), GS1 table 88 to 93 (cross-version-timeline)",
        unknown: &[],
    },
    Step {
        version: ClientVersion::V135,
        label: "1.35",
        token: "GNW13040",
        date: "2000-04-13",
        basis: Basis::Binary,
        delta: Delta { hud: Some(HudArt::LettersChat), ..Delta::NONE },
        source: "shipped state.png: CHAT label on the HUD sheet; GS1 table unchanged",
        unknown: &[],
    },
    Step {
        version: ClientVersion::V136,
        label: "1.36",
        token: "GNW10050",
        date: "2000-05-12",
        basis: Basis::Binary,
        delta: Delta { gs1_commands: Some(104), ..Delta::NONE },
        source: "binary: GS1 table 93 to 104",
        unknown: &[],
    },
    Step {
        version: ClientVersion::V137,
        label: "1.37",
        token: "GNW21050",
        date: "2000-05-23",
        basis: Basis::Binary,
        delta: Delta { gs1_commands: Some(106), ..Delta::NONE },
        source: "binary: GS1 table 104 to 106",
        unknown: &[],
    },
    Step {
        version: ClientVersion::V138,
        label: "1.38",
        token: "GNW22060",
        date: "2000-06-23",
        basis: Basis::Binary,
        delta: Delta {
            plo: IdDelta::plus(&[43]),
            pli: IdDelta::plus_minus(&[32, 33], &[25]),
            hud: Some(HudArt::BlankButtons),
            emoticons: Some(true),
            key_config: Some(true),
            gs1_commands: Some(109),
            drops: Some(DROPS_138),
            ..Delta::NONE
        },
        source: "binary: PLO 43, PLI ITEMTAKE/WEAPONADD (32, 33) in and 25 out, configurable keys and emotions, GS1 106 to 109; bush rolls sent with PLI_ITEMADD and pickups with ITEMTAKE (1.39 lab capture)",
        unknown: &[],
    },
    Step {
        version: ClientVersion::V139,
        label: "1.39",
        token: "GNW07080",
        date: "2000-08-07",
        basis: Basis::Binary,
        delta: Delta {
            plo: IdDelta::plus(&[75]),
            pli: IdDelta::plus(&[80, 81]),
            own_props: IdDelta::plus(&[34]),
            other_props: IdDelta::plus(&[33]),
            chat_recall: Some(true),
            chat_text: Some(ChatText { face: ChatFace::Helvetica, height: 24 }),
            gs1_commands: Some(111),
            ..Delta::NONE
        },
        source: "binary (lab-verified as gen 2): PLO 75, PLI PROFILEGET/SET (80, 81), own prop 34, other prop 33, chat recall, chat font Helvetica 24, GS1 109 to 111",
        unknown: &[],
    },
    // 1.41 is the first gen 3 build. Its row was built on the 2.05 template,
    // so everything not listed here is the 2.05 value; that includes the
    // death and combat rules, which no 1.41 handler was traced for.
    Step {
        version: ClientVersion::V141,
        label: "1.41",
        token: "GNW13110",
        date: "",
        basis: Basis::Binary,
        delta: Delta {
            encryption: Some(EncryptionGen::Gen3),
            // 14923 is the port the patched surviving copy carries, and the classic-login port.
            // Seihai's classic login got a list there on 2026-10-25 (matrix run).
            listserver: Some(Some(Listserver::new(ListKind::Legacy, PortStatus::Confirmed, Some(BuiltIn { host: "listserver.graal.in", port: 14923 }), None))),
            pli: IdDelta::plus_minus(&[], &[31]),
            own_props: IdDelta::plus(&[35, 36]),
            other_props: IdDelta::plus(&[35, 36]),
            npc_max: Some(35),
            death: Some(Some(DEATH_205)),
            combat: Some(Some(COMBAT_205)),
            server_list: Some(ServerList::Single),
            gs1_commands: Some(126),
            ..Delta::NONE
        },
        source: "binary: gen 3 (key byte + LCG insertion), server-list window, PLI 31 gone, props to 36, GS1 111 to 126",
        unknown: &[
            Unknown { area: "drops", what: "the roll and PLI_ITEMADD were read in the 1.41 binary; that its pickup and timeout go through ITEMTAKE and ITEMDEL as in 1.39 is assumed, not traced" },
            Unknown { area: "listserver", what: "original host and port; the only surviving 1.41 copy is patched for listserver.graal.in and carries 14923. Seihai's classic login got a legacy list on 14923 on 2026-10-25 (14922 stayed silent); a real 1.41 client was not captured" },
            Unknown { area: "listserver", what: "login frame inferred, not captured, and no evidence for its shape: Seihai keeps shape B (id 0 `newmain`, then id 1 with the account), what it sent before the lab captures; whether a real 1.41 sends the account to the listserver is unknown" },
            Unknown { area: "life", what: "death and combat rules come from the 2.05 template, no 1.41 handler was traced (1.42 clears them)" },
            Unknown { area: "levels", what: "the version-profile TOML lists GLEVNW01 (.nw) as readable by 1.41 and 1.42 and drops it at 2.00; the rows of 1.33 to 2.04 have .graal only" },
        ],
    },
    Step {
        version: ClientVersion::V142,
        label: "1.42",
        token: "GNW08011",
        date: "2001-01-10",
        basis: Basis::Binary,
        delta: Delta {
            // Classic login: 14923 (matrix run, 2026-10-25). The build itself carries
            // www.graalonline.com:14921 (dead).
            listserver: Some(Some(
                Listserver::new(ListKind::Legacy, PortStatus::Assumed, Some(BUILT_IN_14921), None).shape(LoginShape::NewmainOnly, ShapeBasis::Inferred),
            )),
            plo: IdDelta::plus(&[42, 44, 45, 150, 151, 153]),
            pli: IdDelta::plus(&[34]),
            death: Some(None),
            combat: Some(None),
            ganis: Some(true),
            chat_text: Some(ChatText { face: ChatFace::TempusSans, height: 24 }),
            gs1_commands: Some(134),
            mannerisms: Some(MANNER_141),
            drops: Some(DROPS_142),
            ..Delta::NONE
        },
        source: "binary (NewMain 1.42): ganis, PLO 42/44/45/150/151/153, UPDATEFILE (34) sent, chat font Tempus Sans ITC 24, GS1 126 to 134; no client drop roll (bush rupees and pot hearts are server-side, newfeatures142.txt)",
        unknown: &[
            Unknown { area: "listserver", what: "built-in port 14921 is dead; Seihai's classic login got a legacy list on 14923 on 2026-10-25 (14922 stayed silent); a real 1.42 client was not captured" },
            Unknown { area: "listserver", what: "login frame inferred, not captured: Seihai sends shape A (id 0 `newmain` only), the shape of the real 2.00 and 2.05, as a best guess; a real 1.42 frame was never captured, and no run on the public service has used shape A, so the port is assumed" },
        ],
    },
    Step {
        version: ClientVersion::V200,
        label: "2.00",
        token: "GNW24011",
        date: "2001-02-01",
        basis: Basis::Binary,
        delta: Delta {
            // 14923 is the port that answers a classic login (checked 2026-10-25, twice for
            // 2.00). The build itself carries www.graalonline.com:14921, which is dead.
            listserver: Some(Some(
                Listserver::new(ListKind::Legacy, PortStatus::Assumed, Some(BUILT_IN_14921), None).shape(LoginShape::NewmainOnly, ShapeBasis::Captured),
            )),
            plo: IdDelta::plus(&[154, 155]),
            combat: Some(Some(COMBAT_200)),
            death: Some(Some(DEATH_205)),
            gs1_commands: Some(142),
            ..Delta::NONE
        },
        source: "binary (2.00 r0 to r3, no differences): PLO 154/155, GS1 134 to 142, combat rules read in r0; death rules as 2.05 (timer 30, input after 27, dead bit from 28, respawn hearts by AP, name-group entry save), read in r0 and checked in the machine code of every 2.00 to 2.04 build; 2.00 fires no playerdies (the event starts in 2.01); list port 14923 from the public checks of 2026-10-25 (built-in www.graalonline.com:14921)",
        unknown: &[
            Unknown { area: "life", what: "combat read in 2.00 r0 only (Graal_analysis client-internals/combat-200-204.md); r1 to r3 assumed the same. Not run against a real client" },
            Unknown { area: "login", what: "props sent at login (verified for 2.05 only)" },
            Unknown { area: "life", what: "death read in the 2.00 r0 decompile; 2.00 r1 to 2.04 checked by their constants only (timer cap, input gate, dead bit, respawn AP steps). No real 2.00 death was captured yet" },
            Unknown { area: "listserver", what: "Seihai now sends the real 2.00 frame (shape A, captured in the lab: id 0 `newmain` only, no account). That frame has not been run on the public service. The 2026-10-25 matrix used Seihai's older classic login (shape B), which 14923 answered with a plain zlib id-0 list (4 rows) while 14922 stayed silent; the port is assumed for shape A until a run. A real 2.00's reply was not captured" },
            Unknown { area: "token", what: "the token is not in gs2lib, so a stock GServer-v2 does not know it" },
        ],
    },
    Step {
        version: ClientVersion::V201,
        label: "2.01",
        token: "GNW23021",
        date: "2001-03-08",
        basis: Basis::Binary,
        delta: Delta {
            // Listserver: the 2.00 login shape A as a best guess (the builds carry the same
            // built-in list as 2.00 and 2.05), so inferred, not captured; later steps carry it.
            listserver: Some(Some(
                Listserver::new(ListKind::Legacy, PortStatus::Assumed, Some(BUILT_IN_14921), None).shape(LoginShape::NewmainOnly, ShapeBasis::Inferred),
            )),
            plo: IdDelta::plus(&[46, 156]),
            pli: IdDelta::plus(&[35, 36]),
            combat: Some(Some(COMBAT_201)),
            gs1_commands: Some(144),
            ..Delta::NONE
        },
        source: "binary: ADJACENTLEVEL (35) and HITOBJECTS (36) sent, PLO 46 and 156, GS1 142 to 144; the hurt packet gains the AP 100, no-kill and once-a-second gates",
        unknown: &[Unknown { area: "listserver", what: "2.01 to 2.04 (and 2.02 rev 2): login frame inferred, not captured. Seihai sends shape A (id 0 `newmain` only), the shape of the real 2.00 and 2.05, as a best guess; no real client of these builds was captured and no run on the public service has used shape A, so the port is assumed. The 2026-10-25 matrix used Seihai's older shape B: a legacy list on 14923, nothing on 14922. 14921, the port the build carries, is dead" }],
    },
    Step {
        version: ClientVersion::V202,
        label: "2.02",
        token: "GNW15041",
        date: "2001-04-15",
        basis: Basis::Binary,
        delta: Delta {
            plo: IdDelta::plus(&[47]),
            own_props: IdDelta { plus_ranges: &[(37, 41)], ..IdDelta::NONE },
            other_props: IdDelta { plus_ranges: &[(37, 41)], ..IdDelta::NONE },
            npc_max: Some(40),
            gs1_commands: Some(146),
            ..Delta::NONE
        },
        source: "binary (base build and rev 1): PLO 47, player props 37 to 41, NPC props to 40, GS1 144 to 146",
        unknown: &[],
    },
    Step {
        version: ClientVersion::V202R2,
        label: "2.02 rev2",
        token: "GNW20041",
        date: "2001-04-20",
        basis: Basis::Binary,
        delta: Delta { pli: IdDelta::plus(&[37]), sends_language: Some(true), ..Delta::NONE },
        source: "binary: LANGUAGE (37) sent after login (no send site in rev 1)",
        unknown: &[],
    },
    Step {
        version: ClientVersion::V203,
        label: "2.03",
        token: "GNW06051",
        date: "2001-05-07",
        basis: Basis::Binary,
        delta: Delta {
            plo: IdDelta::plus(&[48, 165]),
            pli: IdDelta::plus(&[38]),
            combat: Some(Some(COMBAT_205)),
            gs1_commands: Some(148),
            ..Delta::NONE
        },
        source: "binary: TRIGGERACTION (38) sent, PLO 48 and 165, GS1 146 to 148; sword power stored limited to -20..20, as 2.05",
        unknown: &[],
    },
    Step {
        version: ClientVersion::V204,
        label: "2.04",
        token: "GNW24061",
        date: "2001-06-25",
        basis: Basis::Binary,
        delta: Delta {
            plo: IdDelta::plus_minus(&[166], &[26]),
            own_props: IdDelta::plus(&[42]),
            other_props: IdDelta::plus(&[42]),
            gs1_commands: Some(157),
            ..Delta::NONE
        },
        source: "binary: PLO 166 in and 26 out, prop 42 (ATTACHNPC), GS1 148 to 157",
        unknown: &[],
    },
    Step {
        version: ClientVersion::V205,
        label: "2.05",
        token: "GNW03081",
        date: "2001-08-16",
        basis: Basis::Binary,
        delta: Delta {
            // This profile sends the classic login, so it takes the classic-login port, 14923
            // (not checked with this login: see the unknown entry).
            listserver: Some(Some(
                Listserver::new(ListKind::Legacy, PortStatus::Assumed, Some(BUILT_IN_14921), None).shape(LoginShape::NewmainOnly, ShapeBasis::Captured),
            )),
            plo: IdDelta::plus(&[168]),
            nw: Some(true),
            gmap: Some(true),
            cached_links: Some(true),
            death: Some(Some(DEATH_205)),
            server_in_title: Some(true),
            gs1_commands: Some(161),
            mannerisms: Some(MANNER_205),
            ..Delta::NONE
        },
        source: "binary (rev 1 and rev 2) and lab: .nw and .gmap levels, PLO 168, server name in the title, GS1 157 to 161; death rules traced here (combat as 2.03, traced in both)",
        unknown: &[
            Unknown { area: "listserver", what: "this profile now sends the real 2.05 frame (shape A, captured in the lab: id 0 `newmain` only). No public run has used it: the 2026-10-25 matrix binary sent the gen-5 login for 2.05 (the saved-version override, since fixed) and both 14922 and 14923 answered that. 14923 answered the older classic login (shape B) of every other classic profile, so 14923 is assumed here" },
        ],
    },
    // 2.1 to 2.16 have no binary. Their deltas are release notes and server
    // source, and a value the notes do not change is carried from the step
    // before.
    Step {
        version: ClientVersion::V210,
        label: "2.1",
        token: "GNW31101",
        date: "",
        basis: Basis::Inferred,
        delta: Delta {
            // Classic login: 14923 (checked for 2.1 and 2.12 to 2.16 on 2026-10-25; 14922 stayed
            // silent). The 2.1 test builds carry www.graalonline.com:14922.
            listserver: Some(Some(Listserver::new(ListKind::Legacy, PortStatus::Confirmed, Some(BUILT_IN_14922), None))), // shape B, no evidence
            plo: IdDelta { plus: &[49], plus_ranges: &[(169, 174)], ..IdDelta::NONE },
            own_props: IdDelta { plus_ranges: &[(43, 45)], ..IdDelta::NONE },
            other_props: IdDelta { plus_ranges: &[(43, 45)], ..IdDelta::NONE },
            npc_max: Some(43),
            bow_prop10: Some(false),
            modtime: Some(true),
            image_ext: Some(".png"),
            download_maps: Some(true),
            death: Some(Some(DEATH_21)),
            minimap: Some(true),
            log_window: Some(true),
            ghost_mode: Some(true),
            ..Delta::NONE
        },
        source: "server source and release notes: gani prop 10, .png defaults, PLO_FILE modtime, gserver minimap, ghost mode; PLO 49 and 169 are in the two 2.1 test builds",
        unknown: &[
            Unknown { area: "binary", what: "no 2.1 release binary; every value of this step is inferred" },
            Unknown { area: "token", what: "the 2.1 test builds still send the 2.05 token, so GNW31101 is from gs2lib only" },
            Unknown { area: "protocol", what: "PLO 170 to 174 are placed here from the notes, the test builds handle only up to 169" },
            Unknown { area: "listserver", what: "2.1 to 2.16 have no client binary: login frame inferred, not captured. Seihai keeps shape B (id 0 `newmain`, then id 1), what it sent before the lab captures, with no evidence for it. The port was checked with that frame only (list on 14923, silence on 14922, 2026-10-25); no real client of these builds was captured" },
            Unknown { area: "pli", what: "client send set is carried from 2.05" },
            Unknown { area: "props", what: "player props 43 to 45 and the NPC prop limit 43 are not confirmed for a 2.1 release" },
            Unknown { area: "life", what: "death rule without the name-group test is inferred (the delimiter is not in the 2.1 test binaries)" },
            Unknown { area: "files", what: "the maps download folder rule (download_maps) is inferred from 2.17.1; 2.05 has no such rule" },
        ],
    },
    Step {
        version: ClientVersion::V212,
        label: "2.12",
        token: "GNW01012",
        date: "",
        basis: Basis::Inferred,
        delta: Delta::NONE,
        source: "release notes: multi-level editing in the editor",
        unknown: &[Unknown { area: "all", what: "no wire, prop or file change is known; the row is the step before (no 2.12 binary)" }],
    },
    Step {
        version: ClientVersion::V213,
        label: "2.13",
        token: "GNW23012",
        date: "",
        basis: Basis::Inferred,
        delta: Delta::NONE,
        source: "release notes: gani attributes to #P9",
        unknown: &[Unknown { area: "all", what: "no wire, prop or file change is known; the row is the step before (no 2.13 binary)" }],
    },
    Step {
        version: ClientVersion::V214,
        label: "2.14",
        token: "GNW30042",
        date: "",
        basis: Basis::Inferred,
        delta: Delta {
            plo: IdDelta::plus(&[68, 69, 178, 180]),
            pli: IdDelta::plus(&[41]),
            large_files: Some(true),
            server_list: Some(ServerList::Tabs(2)),
            ..Delta::NONE
        },
        source: "server source and release notes: large files (LARGEFILESTART/END), SERVERWARP, server list in two tabs; PLO 178 and 180 from the version-profile TOML",
        unknown: &[
            Unknown { area: "protocol", what: "PLO 178 and 180 and PLI 41 are inferred (the TOML entry is itself inferred); 175 to 185 are first seen in 2.17" },
        ],
    },
    Step {
        version: ClientVersion::V215,
        label: "2.15",
        token: "GNW19052",
        date: "",
        basis: Basis::Inferred,
        delta: Delta::NONE,
        source: "no 2.15 source of its own",
        unknown: &[Unknown { area: "all", what: "no change is known; the row is the step before (no 2.15 binary)" }],
    },
    Step {
        version: ClientVersion::V216,
        label: "2.16",
        token: "GNW12102",
        date: "",
        basis: Basis::Inferred,
        delta: Delta {
            own_props: IdDelta { plus_ranges: &[(46, 49), (52, 52), (54, 74)], ..IdDelta::NONE },
            other_props: IdDelta { plus: &[23], plus_ranges: &[(46, 49), (52, 76)], ..IdDelta::NONE },
            ..Delta::NONE
        },
        source: "release notes: gani attributes to #P30 (props 54 to 74); player prop sets follow the 2.22 binary",
        unknown: &[
            Unknown { area: "props", what: "the version-profile TOML keeps the 2.15 prop limit (45) although its note says 54 to 74 arrive in 2.16; the row follows the note" },
            Unknown { area: "props", what: "NPC prop limit is carried from 2.1 (the 2.17 value is 73)" },
        ],
    },
    Step {
        version: ClientVersion::V217,
        label: "2.17",
        token: "GNW22122",
        date: "",
        basis: Basis::Binary,
        delta: Delta {
            // Classic login: 14923 (checked 2026-10-25; 14922 stayed silent). The host and port
            // the client reads come from arrays2.dat, not the binary.
            listserver: Some(Some(
                Listserver::new(ListKind::Legacy, PortStatus::Confirmed, None, None).shape(LoginShape::NewmainAccount, ShapeBasis::Captured),
            )),
            plo: IdDelta { plus: &[179], plus_ranges: &[(175, 177), (181, 185)], ..IdDelta::NONE },
            pli: IdDelta::plus(&[39, 40, 43, 44, 45]),
            tampercheck: Some(true),
            tamper: Some(Some(TAMPER_217)),
            showimg: Some(Some(SHOWIMG_217)),
            login: Some(Some(LOGIN_217)),
            npc_max: Some(73),
            heights: Some(true),
            ..Delta::NONE
        },
        source: "binary (2.17.1): TAMPERCHECK (39) with its constant, SHOOT (40), MUTEPLAYER (43), process-list pair (44, 45), PLO 175 to 185, level heights, NPC props to 73, the post-login frame seen in the lab (image checks, process list, props up to the language), and id 24 about every 5.1 s when alone (lab, 2026-10-30)",
        unknown: &[
            Unknown { area: "props", what: "other-player prop parser was not read; the 2.22 set is used" },
            Unknown { area: "ui", what: "GS1 command count was not read; 161 is carried" },
            Unknown { area: "login", what: "the real 2.17.1 login to a lab listserver was captured: shape B (one plain zlib frame, id 0 `newmain` then id 1 with the account, no token), which Seihai sends. What the public service does for a real 2.17.1 was not observed; Seihai's frame got a legacy list on 14923 and no reply on 14922 on 2026-10-25 (with the default zlib header 78 9C; Seihai now sends 78 DA like the real client, not yet run)" },
            Unknown { area: "showimg", what: "the binary reading gave 2.17.1 2.22's send test (others in the level), and a solo 90 s capture on 2026-10-25 saw no id 24; the five solo 2026-10-30 runs on two lab servers saw it every 5.1 s, so Seihai follows those. Why the two disagree is not known, and the gap while walking (6.5 to 9.3 s) is not modelled" },
        ],
    },
    Step {
        version: ClientVersion::V222,
        label: "2.22",
        token: "GNW03014",
        date: "",
        basis: Basis::Binary,
        delta: Delta {
            client_type: Some(5),
            encryption: Some(EncryptionGen::Gen5),
            // Gen-5 login: 14922 (the current list, 2026-10-25); 14923 answers it too.
            listserver: Some(Some(Listserver::new(ListKind::Supported, PortStatus::Confirmed, None, Some("GNW30123")))),
            plo: IdDelta::plus(&[186]),
            pli: IdDelta::plus_minus(&[], &[36, 45, 81]),
            tamper: Some(Some(TAMPER_222)),
            showimg: Some(Some(SHOWIMG_222)),
            login: Some(Some(LOGIN_222)),
            udp_off_by_default: Some(true),
            server_list: Some(ServerList::Tabs(3)),
            mannerisms: Some(MANNER_222),
            ..Delta::NONE
        },
        source: "binary and lab and public servers: client type 5 and gen 5 framing, listserver id 7 with GNW30123, PLO 186, PLI 36, 45 and 81 not sent, id 24 about every 5.1 s, post-login frame with OS and code page props, links tested with a per-direction probe box",
        unknown: &[
            Unknown { area: "pli", what: "send sites of ids 36, 42 and 45 were not matched in the binary, so 36 and 45 are not sent" },
            Unknown { area: "framing", what: "a client-originated bz2 frame (0x1000 and over) was never seen" },
        ],
    },
    Step {
        version: ClientVersion::V230,
        label: "2.3",
        token: "GNW14015",
        date: "",
        basis: Basis::Inferred,
        delta: Delta {
            plo: IdDelta::plus_minus(&[84, 189], &[44]),
            pli: IdDelta::plus_minus(&[46], &[21]),
            // Gen-5 login like 2.31: 14922 answered it on 2026-10-25 (no 2.3 build exists).
            // No 2.3 build exists, so shape C is inferred from 2.31, not captured.
            listserver: Some(Some(
                Listserver::new(ListKind::Supported, PortStatus::Confirmed, None, Some("GNW30123")).shape(LoginShape::Gen5, ShapeBasis::Inferred),
            )),
            tamper: Some(Some(TAMPER_230)),
            showimg: Some(None),
            login: Some(None),
            own_props: IdDelta { plus_ranges: &[(78, 80)], ..IdDelta::NONE },
            other_props: IdDelta { plus_ranges: &[(78, 80)], ..IdDelta::NONE },
            npc_max: Some(77),
            precise_pos: Some(true),
            large_file_size: Some(true),
            editor: Some(false),
            ..Delta::NONE
        },
        source: "taken as 2.31 (no 2.3 binary): PLO 84 (LARGEFILESIZE) and 189 (MOVE2) in and 44 out, ENTERLEVEL (46) in and PUTNPC (21) out, X2/Y2/Z2 positions, editor moved out",
        unknown: &[
            Unknown { area: "binary", what: "no 2.3 binary; the whole step is the 2.31 values" },
            Unknown { area: "listserver", what: "the port was checked with Seihai's gen-5 login only (2026-10-25); no real 2.3 client was captured" },
            Unknown { area: "tampercheck", what: "constant of this build; the re-send rule is taken from 2.31" },
            Unknown { area: "showimg", what: "no id 24 observation, so none is sent" },
            Unknown { area: "login", what: "no post-login frame observation, so none is sent" },
            Unknown { area: "gr flags", what: "no observation of the 2.3 build; none are set (2.31 sets none either)" },
        ],
    },
    Step {
        version: ClientVersion::V231,
        label: "2.31",
        token: "GNW28015",
        date: "",
        basis: Basis::Binary,
        delta: Delta {
            listserver: Some(Some(Listserver::new(ListKind::Supported, PortStatus::Confirmed, None, Some("GNW30123")))),
            tamper: Some(Some(TAMPER_231)),
            showimg: Some(Some(SHOWIMG_231)),
            login: Some(Some(LOGIN_231)),
            ..Delta::NONE
        },
        source: "binary (same code as 2.22) and lab: its own tampercheck constant and timer re-send, id 24 every 5.13 s whether or not others are in the level, post-login frame without the idle.gani request",
        unknown: &[
            Unknown { area: "pli", what: "the TOML's pli_changes (21 out, 46 in) disagrees with its pli_sent list, which lacks 44; a real client sent 44, so the row keeps it" },
            Unknown { area: "live", what: "no game-server login on a public server; only the list reply was checked (2026-10-25)" },
        ],
    },
    // The 3.x builds (Graal_analysis `version-profiles/3.04-3.153.toml`). Their
    // tokens carry the dates of 2.22 and 2.31; the chain puts them after 2.31
    // and writes each as a change to the row before it.
    Step {
        version: ClientVersion::V304,
        label: "3.04",
        token: "G3D03014",
        date: "",
        basis: Basis::Binary,
        delta: Delta {
            // Against 2.31: 44, 101 and 102 in; 170, 177, 182, 183 and MOVE2 (189) out.
            plo: IdDelta::plus_minus(&[44, 101, 102], &[170, 177, 182, 183, 189]),
            // PUTNPC (21), 36 and 81 in; TAMPERCHECK (39), PROCESSLIST (44)
            // and ENTERLEVEL (46) out.
            pli: IdDelta::plus_minus(&[21, 36, 81], &[39, 44, 46]),
            listserver: Some(Some(
                // The address built into the binary; license/license.graal names the one used.
                Listserver::new(ListKind::Supported, PortStatus::Assumed, Some(BuiltIn { host: "listserver.graalonline.com", port: 14922 }), Some("G3D30123"))
                    .shape(LoginShape::Gen5OneFrame, ShapeBasis::Captured),
            )),
            tampercheck: Some(false),
            tamper: Some(None),
            showimg: Some(None),
            login: Some(Some(LOGIN_304)),
            udp_off_by_default: Some(false),
            // Parses its own props up to 74, without 33, 50, 51 and 53; no X2/Y2/Z2.
            own_props: IdDelta { minus: &[33], minus_ranges: &[(78, 80)], ..IdDelta::NONE },
            other_props: IdDelta { minus_ranges: &[(78, 80)], ..IdDelta::NONE },
            npc_max: Some(73),
            precise_pos: Some(false),
            ..Delta::NONE
        },
        source: "binary (Linux i386 ELF) and one lab run on GServer-v2 (Graal_analysis version-profiles/lab-304-linux-2026-10-30.md): PLO and PLI sets, props up to 76 (parsed to 74), NPC props to 73, no tampercheck, no showimg timer (UDP 15 instead), listserver id 7 with G3D30123",
        unknown: &[
            Unknown { area: "listserver", what: "port 14922 and the G3D token were checked against the lab listserver only, never the public one" },
            Unknown { area: "login", what: "UDPPORT (31) is left out of the first props, as for every build (no UDP socket); the nickname is the account, where the real client sends the start window's nickname field" },
            Unknown { area: "ui", what: "the Torque front end (start window, options, themes, chat as white text with a blue shadow) is not modelled; the 2.31 UI row is kept" },
            Unknown { area: "other props", what: "the props it parses for other players are taken as 2.31's without 78-80" },
        ],
    },
    Step {
        version: ClientVersion::V3153,
        label: "3.153",
        token: "G3D28015",
        date: "",
        basis: Basis::Binary,
        delta: Delta {
            plo: IdDelta::plus_minus(&[67, 74, 82, 104, 131, 132, 133, 134, 187, 189], &[44]),
            pli: IdDelta::plus(&[46, 50, 93, 96, 97, 98, 152, 154, 155, 156, 157, 158]),
            // Same G3D30123 login as 3.04, but this build was never run: shape D is 3.04's.
            listserver: Some(Some(
                Listserver::new(ListKind::Supported, PortStatus::Assumed, Some(BuiltIn { host: "listserver.graalonline.com", port: 14922 }), Some("G3D30123"))
                    .shape(LoginShape::Gen5OneFrame, ShapeBasis::Inferred),
            )),
            login: Some(Some(LOGIN_3153)),
            udp_off_by_default: Some(true),
            own_props: IdDelta { plus_ranges: &[(78, 80)], ..IdDelta::NONE },
            other_props: IdDelta { plus_ranges: &[(78, 80)], ..IdDelta::NONE },
            npc_max: Some(77),
            precise_pos: Some(true),
            ..Delta::NONE
        },
        source: "binary (Mac OS X PPC Mach-O), not run: OSTYPE mac; PLO 67, 74, 82, 104, 131-134, 187 and MOVE2 (189) in, 44 out; RC file browser, script upload and text request ids; X2/Y2/Z2; UDP off in the shipped prefs",
        unknown: &[
            Unknown { area: "pli", what: "2, 3, 15, 24, 29 and 80 are taken from 3.04; the PPC scan did not resolve those call sites" },
            Unknown { area: "live", what: "never run" },
            Unknown { area: "listserver", what: "shape D (id 7 and id 1 in one plain zlib frame at 78 9C) is 3.04's capture; the binary has the same G3D30123 token and gen-5 cipher" },
            Unknown { area: "login", what: "OSTYPE mac is from the binary; the code page (1250), the files in the first frame (bigmap.txt, minimap.txt) and the head and default-weapon checks are 3.04's" },
            Unknown { area: "3d", what: "the 3D mode (Torque GameConnection, Graal3D protocol 19, on UDP listed port + 1) is not modelled here; seihai-ato::torque has its profile" },
        ],
    },
    // The 4.x builds (Graal_analysis `protocol/v4x-clients.md`,
    // `client-internals/live-behaviour-404.md`, `version-profiles/4.04-4.20.toml`).
    // The first Login-lobby builds: no listserver protocol, a game server
    // named `Login` first, and a server warp (178) before anything but the
    // level is drawn. Their 3D plug-in (Torque protocol 19 in 4.04r2, 21 in
    // 4.20r6) is not modelled; they play on the flat path in seihai-mae.
    Step {
        version: ClientVersion::V404R2,
        label: "4.04r2",
        token: "G3D17026",
        date: "2006-03-03",
        basis: Basis::Binary,
        delta: Delta {
            // Against 3.153: update packages (105, 106), ghost mode (170), the
            // window list (182, 183), onServerListerConnect (190), SHOOT2 (191),
            // the cache check (193), script list clear (194) and server ganis (195).
            plo: IdDelta::plus(&[105, 106, 170, 182, 183, 190, 191, 193, 194, 195]),
            // The send ids read in the 4.x engine, with the ids 3.153 and 5.00r7
            // both send taken as sent; 40 and 43 are in neither 4.x nor 5.00r7.
            // 47 (VERIFYWANTSEND) for a cached update package, seen live
            // (2026-10-31).
            pli: IdDelta::plus_minus(&[44, 47, 79, 92, 130, 161], &[40, 43]),
            // No listserver: the game server named `Login` serves the list.
            listserver: Some(None),
            login: Some(None),
            lobby: Some(Some(LOBBY_404)),
            showimg: Some(Some(SHOWIMG_404)),
            server_list: Some(ServerList::Direct { default: "loginserver.graalonline.com:14900" }),
            chat_text: Some(ChatText { face: ChatFace::Arial, height: 24 }),
            ..Delta::NONE
        },
        source: "binary (stock engine DLL unpacked from the Thinstall installer) and one live lab run (live-behaviour-404): 98 PLO ids, login id 5 with G3D17026 and win,<3 md5> in the plain zlib frame, then 37; Login mode while the server name starts with login, until 178",
        unknown: &[
            Unknown { area: "pli", what: "the send ids were read in the 4.20r8 engine (provisional, as in 3.x); 4.04r2 is taken as that set without 162, its answer to 108, which 4.04r2 does not handle. 0, 3, 11, 20, 27, 30, 34, 38, 46, 93, 97, 98, 155 and 156 are 3.153's (11, 34, 38 and 46 seen in the lab; the 2026-10-31 live run saw 2, 4, 9, 23, 24, 34, 37, 38, 44, 46, 152, 154 and 157, all in the set)" },
            Unknown { area: "login", what: "Seihai sends the three machine fields as an empty quoted string, as for 5.00r7; the real client sends three MD5s" },
            Unknown { area: "login", what: "the after-login messages follow the real client (lab, 2026-10-31), but the nickname is Seihai's setting (default the account), 44 lists Seihai's own window title, and the client's file checks of its GUI images (plisticon, plistcat, state.png, pics1.png) are not sent" },
            Unknown { area: "walk", what: "the 50 ms X2/Y2 cadence with GANI walk/idle follows the lab; pushing, grabbing, carrying and the bow were not compared" },
            Unknown { area: "serverwarp", what: "the lab's 178 reached the game server, but the warp text it used was not recorded; split on commas only, as the 5.x and 6.x builds" },
            Unknown { area: "props", what: "own and other prop sets are 3.153's; no 4.x prop parser was read (the lab saw X2/Y2, 78 and 79)" },
            Unknown { area: "ui", what: "chat is white bold sans with a blue shadow in the lab; the face and cell height are taken from 5.00r7" },
            Unknown { area: "3d", what: "Graal3DEngine.dll (Torque protocol 19, UDP on listed port + 1) is not modelled; seihai-ato::torque has its profile" },
        ],
    },
    Step {
        version: ClientVersion::V420R6,
        label: "4.20r6",
        token: "G3D27076",
        date: "2006-09",
        basis: Basis::Binary,
        delta: Delta {
            // Layered board change (107) and 108 in; 41 has no case in this build.
            // Re-read 2026-10-31: the switch has 97 cases (0-195 without 41) and
            // 100 and 108 are taken before it, as in 4.20r8, so 99 ids.
            plo: IdDelta::plus_minus(&[107, 108], &[41]),
            pli: IdDelta::plus(&[162]),
            lobby: Some(Some(LOBBY_420_MAC)),
            ..Delta::NONE
        },
        source: "binary (Mac OS X PPC, not run): 4.20r8's PLO set without 41 and 196 (a 97-case switch plus 100 and 108 before it; 108 subtype 0 answers with 162), login id 5 with G3D27076 and mac,<MAC hash>,<stat hash>,<empty>",
        unknown: &[
            Unknown { area: "pli", what: "only 162 (the answer to 108) was confirmed in the Mac binary; the rest of 4.20r8's set is taken" },
            Unknown { area: "login", what: "the fourth field is empty in the binary; Seihai sends it, like the other two, as an empty quoted string" },
            Unknown { area: "live", what: "never run (PowerPC)" },
            Unknown { area: "3d", what: "Torque protocol 21 is not modelled here; seihai-ato::torque has its profile" },
        ],
    },
    Step {
        version: ClientVersion::V420R8,
        label: "4.20r8",
        token: "G3D20126",
        date: "2007-01",
        basis: Basis::Binary,
        delta: Delta {
            plo: IdDelta::plus(&[41, 196]),
            lobby: Some(Some(LOBBY_404)),
            ..Delta::NONE
        },
        source: "binary (a third-party packet-logger copy of the engine DLL, linked January 2007; its signature check is patched out): 101 PLO ids (41 and 196 in against the Mac 4.20r6), send ids from the connection code, login id 5 with G3D20126 and win,<3 md5>",
        unknown: &[
            Unknown { area: "pli", what: "provisional: some sends pass the id in a register, so the set is a floor; 0, 3, 11, 20, 27, 30, 34, 38, 46, 93, 97, 98, 155 and 156 are 3.153's" },
            Unknown { area: "binary", what: "only the modified engine DLL was readable; the stock 4.20r8 is in a password-protected archive" },
            Unknown { area: "plo", what: "196 (same layout as 193) is read and dropped by the client; Seihai drops it too" },
        ],
    },
    // The 5.x and 6.x Windows builds (Graal_analysis `protocol/v5x-clients.md`,
    // `v6x-clients.md`, `v5v6-windows-unpacked.md`, `v5v6-lab-play.md`,
    // `version-profiles/5.00-6.03.toml`). They follow 4.20r8 in the chain, but
    // their 3D mode is not modelled and they play on the flat path: no
    // listserver protocol, a game server named `Login` first, and a server
    // warp (178) before anything but the level is drawn. 6.036 has no binary
    // and no token, so it has no step (see `documentation/v5v6-profiles.md`).
    Step {
        version: ClientVersion::V500R7,
        label: "5.00r7",
        token: "G3D22067",
        date: "2007-07",
        basis: Basis::Binary,
        delta: Delta {
            // Against 4.20r8: 100, 108 and 196 out; 140 and 197 in.
            plo: IdDelta::plus_minus(&[140, 197], &[100, 108, 196]),
            // The binary's constant send ids plus 11 and 38 (lab), with the ids both
            // 3.153 and 6.03r7 send taken as sent. 162 answers 108, which 5.00r7
            // does not handle. The lobby, list and chat are 4.x's.
            pli: IdDelta::plus_minus(&[159], &[162]),
            // Not recorded for 5.00r7; the 4.x value is not carried over.
            showimg: Some(None),
            // The 4.x login line; the 4.x after-login messages are not carried over.
            lobby: Some(Some(LOBBY_500)),
            ..Delta::NONE
        },
        source: "binary (unpacked engine DLL) and lab (lab-v5-2026-10-30): PLO set of 100 ids, send ids from the send wrappers, login id 5 with G3D22067 and the win,<3 md5> list in the plain zlib frame, then 37 in its own gen-5 frame; Login mode until 178",
        unknown: &[
            Unknown { area: "pli", what: "only constant send ids were read (a floor); 0, 3, 11, 20, 27, 30, 38 and 46 are taken from the lab or from 3.153 and 6.03r7, which both send them" },
            Unknown { area: "login", what: "Seihai sends the three machine fields as an empty quoted string (the value 5.00r7 sent for its MAC field under Wine); stock GServer-v2 accepts all three empty (lab, 2026-10-31), other servers are untested" },
            Unknown { area: "login", what: "the base-file requests after the language (47 or 23 basepackage.gupd) are not sent" },
            Unknown { area: "props", what: "own and other prop sets are 3.153's; no 5.x parser was read" },
            Unknown { area: "ui", what: "chat is a bold sans serif in the lab; its cell height is not known, 24 is kept" },
            Unknown { area: "3d", what: "the Torque 3D mode (Graal3DEngine.dll, protocol 24) is not modelled" },
        ],
    },
    Step {
        version: ClientVersion::V512,
        label: "5.12",
        token: "G3D14097",
        date: "2007-10-19",
        basis: Basis::Binary,
        delta: Delta {
            lobby: Some(Some(LOBBY_512)),
            gen5_round_down: Some(true),
            ..Delta::NONE
        },
        source: "unpacked from MoleBox and lab: PLO set identical to 5.00r7, login token G3D14097 with the quoted Windows version as a fifth field, zlib/bz2 frames stepped len/4 rounded down, every frame sent compressed",
        unknown: &[
            Unknown { area: "pli", what: "not read; 5.00r7's set is kept" },
            Unknown { area: "framing", what: "a server must step as 5.12 does; GServer-v2 does not, so the lab runs it through a translating proxy" },
        ],
    },
    Step {
        version: ClientVersion::V524R4,
        label: "5.24r4",
        token: "G3D15010",
        date: "2010-02-01",
        basis: Basis::Binary,
        delta: Delta {
            plo: IdDelta::plus_minus(&[109, 198], &[75, 132, 133, 171, 193]),
            gen5_round_down: Some(false),
            ..Delta::NONE
        },
        source: "unpacked from MoleBox and lab: 97 PLO ids (75, 132, 133, 171 and 193 out, 109 and 198 in), one MSVC exe, short bodies sent uncompressed as 2.22, font=Arial",
        unknown: &[
            Unknown { area: "pli", what: "not read; 5.00r7's set is kept" },
            Unknown { area: "files", what: "stock gs2lib does not know G3D15010 and sends the pre-2.1 file layout; see documentation/v5v6-profiles.md" },
            Unknown { area: "plo", what: "109 (probably a file redirect) and 198 (one GINT5) are in the set but not decoded" },
        ],
    },
    Step {
        version: ClientVersion::V530R3,
        label: "5.30r3",
        token: "G3D26090",
        date: "2010-10-18",
        basis: Basis::Binary,
        delta: Delta {
            plo: IdDelta::plus_minus(&[], &[41, 181]),
            ..Delta::NONE
        },
        source: "unpacked from Themida and lab (clock set back): 95 PLO ids (41 and 181 out), bundles no files",
        unknown: &[Unknown { area: "pli", what: "not read; 5.00r7's set is kept" }],
    },
    Step {
        version: ClientVersion::V531R4,
        label: "5.31r4",
        token: "G3D26090",
        date: "2011-01-05",
        basis: Basis::Binary,
        delta: Delta::NONE,
        source: "unpacked from Themida and lab (clock set back): same PLO set and token as 5.30r3; draws no tiles before the warp",
        unknown: &[
            Unknown { area: "pli", what: "not read; 5.00r7's set is kept" },
            Unknown { area: "ui", what: "before the warp it draws no tiles (5.24 and 5.30r3 do); Seihai draws them for every lobby build" },
        ],
    },
    Step {
        version: ClientVersion::V600R2,
        label: "6.00r2",
        token: "G3D26090",
        date: "2011-06-12",
        basis: Basis::Binary,
        delta: Delta {
            plo: IdDelta::plus_minus(&[], &[165, 169]),
            // Linux 6.01r1's set (three months later; 6.00r2 is behind Themida).
            pli: IdDelta::plus_minus(&[45, 48, 162], &[21, 80, 81]),
            chat_text: Some(ChatText { face: ChatFace::TempusSans, height: 24 }),
            gen5_long_frames: Some(true),
            ..Delta::NONE
        },
        source: "unpacked from Themida, lab on Windows 11 (RunAsDate) and the 6.x thread's notes (v5v6/6x-findings.md): 93 PLO ids (165 and 169 out), token G3D26090 as 5.30r3, Tempus Sans ITC 24, 6.x long frame lengths",
        unknown: &[
            Unknown { area: "pli", what: "not read; Linux 6.01r1's set (6.03r7's without 160) is taken" },
            Unknown { area: "framing", what: "the 0xFFFF long frame length is read in Linux 6.01r1 and 6.03r7, not in this build; taken from 6.01r1" },
        ],
    },
    // 6.015 and 6.034 have no binary. Their tokens are in GServer-v2's
    // allowedversions.txt (G3D3007A for 6.00r7 and 6.01r5, G3D2505C for
    // 6.03r4) and stock gs2lib knows both (CLVER_6_015, CLVER_6_034). Their id
    // sets are the Linux reads of the same release lines (v5v6/6x-findings.md,
    // graal-decompile/v6x/FINDINGS.md); everything else is carried from 6.00r2.
    Step {
        version: ClientVersion::V6015,
        label: "6.015",
        token: "G3D3007A",
        date: "",
        basis: Basis::Inferred,
        delta: Delta {
            // Linux 6.01r1 (2011-09-18): 6.00r2's set without 175 and 184 (91 ids).
            plo: IdDelta::plus_minus(&[], &[175, 184]),
            // 6.00r2's send set is already Linux 6.01r1's, so no change.
            lobby: Some(Some(LOBBY_512)),
            ..Delta::NONE
        },
        source: "GServer-v2's allowedversions.txt (G3D3007A for 6.0.0.7 and 6.0.1.5) and stock gs2lib (CLVER_6_015); PLO and PLI from Linux 6.01r1 (91 PLO ids: 6.00r2's without 175 and 184); the rest as 6.00r2",
        unknown: &[
            Unknown { area: "binary", what: "no 6.00r7 or 6.01r5 binary; every value of this step is inferred" },
            Unknown { area: "token", what: "G3D3007A comes from GServer-v2's allowedversions.txt and gs2lib only; no 6.00r7 or 6.01r5 login was captured" },
            Unknown { area: "plo", what: "Linux 6.01r1's set; whether 6.00r7 (the earlier of the two builds) already drops 175 and 184 is not known" },
            Unknown { area: "pli", what: "Linux 6.01r1's set (6.03r7's without 160), as for 6.00r2" },
            Unknown { area: "login", what: "the 5.12 to 6.03r7 login line (win, three machine fields, the quoted Windows version) is taken; no capture with this token" },
            Unknown { area: "framing", what: "the 0xFFFF long frame length is read in Linux 6.01r1 and taken" },
            Unknown { area: "ui", what: "Tempus Sans ITC 24 is carried from 6.00r2; Login-mode tile drawing before the warp is not known" },
            Unknown { area: "date", what: "link date not known" },
        ],
    },
    Step {
        version: ClientVersion::V6034,
        label: "6.034",
        token: "G3D2505C",
        date: "",
        basis: Basis::Inferred,
        delta: Delta {
            // 6.03r7's sets (Linux 6.03r7, the same 6.03 release): 111 in, and 160 sent.
            plo: IdDelta::plus(&[111]),
            pli: IdDelta::plus(&[160]),
            lobby: Some(Some(LOBBY_512)),
            ..Delta::NONE
        },
        source: "GServer-v2's allowedversions.txt (G3D2505C for 6.0.3.4) and stock gs2lib (CLVER_6_034); PLO and PLI from Linux 6.03r7 (92 PLO ids, 60 send ids); the rest as 6.015",
        unknown: &[
            Unknown { area: "binary", what: "no 6.03r4 binary; every value of this step is inferred" },
            Unknown { area: "token", what: "G3D2505C comes from GServer-v2's allowedversions.txt and gs2lib only; no 6.03r4 login was captured" },
            Unknown { area: "plo", what: "6.03r7's set; 111 (creationtime.dat) is in 6.03r7 and not in 6.01r1, and whether 6.03r4 already has it is not known" },
            Unknown { area: "pli", what: "6.03r7's set; whether 6.03r4 already sends 160 (RC folder delete) is not known" },
            Unknown { area: "login", what: "the 5.12 to 6.03r7 login line (win, three machine fields, the quoted Windows version) is taken; no capture with this token" },
            Unknown { area: "ui", what: "Tempus Sans ITC 24 is carried; whether 6.03r4 draws tiles before the warp (6.03r7 does not) is not known" },
            Unknown { area: "date", what: "link date not known" },
        ],
    },
    Step {
        version: ClientVersion::V603R7,
        label: "6.03r7",
        token: "G3D0311C",
        date: "2012-11-16",
        basis: Basis::Binary,
        // 6.015 and 6.034 before it already carry 6.03r7's id sets.
        delta: Delta::NONE,
        source: "unpacked from Themida and lab: 92 PLO ids (against 6.00r2: 175 and 184 out, 111 in), equal to Linux 6.03r7; send ids and the long frame length from Linux 6.03r7 (same release); 111 stores its body as creationtime.dat and gets no reply, so Seihai drops it; no expiry date",
        unknown: &[
            Unknown { area: "token", what: "the Linux 6.03r7 sends G3D0511C; this row is the Windows build" },
        ],
    },
];

/// The resolved rows, one per step.
pub const RESOLVED: [VersionProfile; CHAIN.len()] = resolve_chain();

/// Walk the chain from [`ORIGIN`], applying each step to the row before it.
const fn resolve_chain() -> [VersionProfile; CHAIN.len()] {
    let mut out = [ORIGIN; CHAIN.len()];
    let mut cur = ORIGIN;
    let mut i = 0;
    while i < CHAIN.len() {
        cur = apply(cur, &CHAIN[i]);
        out[i] = cur;
        i += 1;
    }
    out
}

/// The step for a version.
pub fn step(v: ClientVersion) -> &'static Step {
    CHAIN.iter().find(|s| s.version == v).expect("every ClientVersion has a step")
}

/// The version a step follows, if any.
pub fn previous(v: ClientVersion) -> Option<ClientVersion> {
    let i = CHAIN.iter().position(|s| s.version == v)?;
    i.checked_sub(1).map(|j| CHAIN[j].version)
}

// -- The generated handler-change table. ------------------------------------

/// One difference between a step and the step before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Field group: `protocol`, `props`, `files`, `levels`, `life`, `ui`,
    /// `identity`.
    pub area: &'static str,
    pub field: &'static str,
    pub detail: String,
}

/// `+49 NAME, +169 NAME`: ids with a known name one by one, runs of ids
/// with no name as `+54-74`.
fn ids_text(ids: &[u8], name: fn(u32) -> Option<&'static str>, sign: char) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < ids.len() {
        let id = ids[i];
        if let Some(n) = name(id as u32) {
            parts.push(format!("{sign}{id} {n}"));
            i += 1;
            continue;
        }
        let mut j = i;
        while j + 1 < ids.len() && ids[j + 1] == ids[j] + 1 && name(ids[j + 1] as u32).is_none() {
            j += 1;
        }
        if j == i {
            parts.push(format!("{sign}{id}"));
        } else {
            parts.push(format!("{sign}{id}-{}", ids[j]));
        }
        i = j + 1;
    }
    parts.join(", ")
}

fn set_change(
    out: &mut Vec<Change>,
    area: &'static str,
    field: &'static str,
    a: IdSet,
    b: IdSet,
    name: fn(u32) -> Option<&'static str>,
) {
    let added: Vec<u8> = b.iter().filter(|&i| !a.contains(i)).collect();
    let removed: Vec<u8> = a.iter().filter(|&i| !b.contains(i)).collect();
    if added.is_empty() && removed.is_empty() {
        return;
    }
    let mut parts = Vec::new();
    if !added.is_empty() {
        parts.push(ids_text(&added, name, '+'));
    }
    if !removed.is_empty() {
        parts.push(ids_text(&removed, name, '-'));
    }
    out.push(Change { area, field, detail: format!("{} ({} -> {} ids)", parts.join("; "), a.len(), b.len()) });
}

fn value_change<T: PartialEq>(
    out: &mut Vec<Change>,
    area: &'static str,
    field: &'static str,
    a: &T,
    b: &T,
    show: impl Fn(&T) -> String,
) {
    if a != b {
        out.push(Change { area, field, detail: format!("{} -> {}", show(a), show(b)) });
    }
}

fn dbg<T: std::fmt::Debug>(v: &T) -> String {
    format!("{v:?}")
}

fn yes_no(v: &bool) -> String {
    if *v { "yes" } else { "no" }.to_string()
}

fn listserver_text(l: &Option<Listserver>) -> String {
    match l {
        None => "none".into(),
        Some(l) => {
            let mut t = format!(
                "{}:{} ({}, {})",
                l.host,
                l.port,
                match l.kind {
                    ListKind::Legacy => "legacy",
                    ListKind::Supported => "supported",
                },
                match l.status {
                    PortStatus::Confirmed => "confirmed",
                    PortStatus::Assumed => "assumed",
                }
            );
            if let Some(b) = l.built_in {
                t.push_str(&format!(", built in {}:{}", b.host, b.port));
            }
            if let Some(tok) = l.token {
                t.push_str(&format!(", token {tok}"));
            }
            t.push_str(&format!(", login shape {} ({})", l.shape.letter(), l.shape_basis.text()));
            t
        }
    }
}

fn tamper_text(t: &Option<TamperInfo>) -> String {
    match t {
        None => "none".into(),
        Some(t) => format!(
            "{} words, re-send {:?}",
            if t.words.is_some() { "known" } else { "unknown" },
            t.resend
        ),
    }
}

fn showimg_text(s: &Option<ShowImgInfo>) -> String {
    match s {
        None => "none".into(),
        Some(s) => format!("every {} ms{}", s.period.as_millis(), if s.needs_others { ", only with others in the level" } else { "" }),
    }
}

fn login_text(l: &Option<LoginInfo>) -> String {
    match l {
        None => "none".into(),
        Some(l) => format!(
            "{}{}{}{}{}{}",
            if l.full { "files, process list, props" } else { "head and default-weapon files, props without head or colours" },
            match l.os_props {
                Some((os, cp)) => format!(" with OS {os} and code page {cp}"),
                None => String::new(),
            },
            if l.idle_gani { ", idle.gani at once" } else { "" },
            if l.status_icons { ", status icons" } else { "" },
            if l.bundle_login && l.first_frame_files.is_empty() { ", language and tamper check in the login frame" } else { "" },
            match l.frame_after_level {
                Some(d) => format!(", {} ms after the level without 182", d.as_millis()),
                None => String::new(),
            },
        ),
    }
}

fn lobby_text(l: &Option<Lobby>) -> String {
    match l {
        None => "none".into(),
        Some(l) => format!(
            "first server {:?}, nothing drawn until 178, login line ends {}{}",
            l.first_server,
            String::from_utf8_lossy(&l.login_tail()),
            match l.after_login {
                Some(a) => format!(
                    "; after the login {} (first session also {}), after the burst idle.gani, 152/154 PM and lister setup, 46, 44, props with OS {} and code page {}",
                    a.after_language.join(", "),
                    a.first_session_files.join(", "),
                    a.os_props.0,
                    a.os_props.1
                ),
                None => String::new(),
            }
        ),
    }
}

fn death_text(d: &Option<DeathRules>) -> String {
    match d {
        None => "none".into(),
        Some(d) if *d == DEATH_205 => "2.05 rules (name-group test)".into(),
        Some(d) if *d == DEATH_21 => "2.1 rules (no name-group test)".into(),
        Some(_) => "custom".into(),
    }
}

fn combat_text(c: &Option<super::profile::CombatRules>) -> String {
    match c {
        None => "none".into(),
        Some(c) if *c == COMBAT_205 => "2.05 rules".into(),
        Some(c) if *c == COMBAT_201 => "2.01 rules (raw sword power)".into(),
        Some(c) if *c == COMBAT_200 => "2.00 rules (raw sword power, fewer packet gates)".into(),
        Some(_) => "custom".into(),
    }
}

fn list_text(l: &ServerList) -> String {
    match l {
        ServerList::Direct { default } => format!("direct ({default})"),
        ServerList::Single => "one list".into(),
        ServerList::Tabs(n) => format!("{n} tabs"),
    }
}

/// Everything that differs between two resolved rows, grouped by field.
/// Identity (label, token, date, basis) is reported on its own by [`render`].
pub fn diff(a: &VersionProfile, b: &VersionProfile) -> Vec<Change> {
    let mut o = Vec::new();
    let (pa, pb) = (&a.protocol, &b.protocol);
    value_change(&mut o, "protocol", "client_type", &pa.client_type, &pb.client_type, dbg);
    value_change(&mut o, "protocol", "encryption", &pa.encryption, &pb.encryption, dbg);
    value_change(&mut o, "protocol", "listserver", &pa.listserver, &pb.listserver, listserver_text);
    set_change(&mut o, "protocol", "plo (server ids handled)", pa.plo, pb.plo, names::plo_name);
    set_change(&mut o, "protocol", "pli (client ids sent)", pa.pli, pb.pli, names::pli_name);
    value_change(&mut o, "protocol", "sends_language", &pa.sends_language, &pb.sends_language, yes_no);
    value_change(&mut o, "protocol", "tampercheck", &pa.tampercheck, &pb.tampercheck, yes_no);
    value_change(&mut o, "protocol", "tamper", &pa.tamper, &pb.tamper, tamper_text);
    value_change(&mut o, "protocol", "showimg (id 24)", &pa.showimg, &pb.showimg, showimg_text);
    value_change(&mut o, "protocol", "login frame", &pa.login, &pb.login, login_text);
    value_change(&mut o, "protocol", "login lobby", &pa.lobby, &pb.lobby, lobby_text);
    value_change(&mut o, "protocol", "gen5_round_down", &pa.gen5_round_down, &pb.gen5_round_down, yes_no);
    value_change(&mut o, "protocol", "gen5_long_frames", &pa.gen5_long_frames, &pb.gen5_long_frames, yes_no);
    value_change(&mut o, "protocol", "udp", &pa.udp, &pb.udp, yes_no);
    value_change(&mut o, "protocol", "udp_off_by_default", &pa.udp_off_by_default, &pb.udp_off_by_default, yes_no);
    let (ra, rb) = (&a.props, &b.props);
    set_change(&mut o, "props", "own-player props", ra.own, rb.own, names::prop_name);
    set_change(&mut o, "props", "other-player props", ra.other, rb.other, names::prop_name);
    value_change(&mut o, "props", "npc_max", &ra.npc_max, &rb.npc_max, dbg);
    value_change(&mut o, "props", "bow_prop10", &ra.bow_prop10, &rb.bow_prop10, yes_no);
    value_change(&mut o, "props", "precise_pos", &ra.precise_pos, &rb.precise_pos, yes_no);
    let (fa, fb) = (&a.files, &b.files);
    value_change(&mut o, "files", "modtime", &fa.modtime, &fb.modtime, yes_no);
    value_change(&mut o, "files", "large_files", &fa.large_files, &fb.large_files, yes_no);
    value_change(&mut o, "files", "large_file_size", &fa.large_file_size, &fb.large_file_size, yes_no);
    value_change(&mut o, "files", "image_ext", &fa.image_ext, &fb.image_ext, dbg);
    value_change(&mut o, "files", "download_maps", &fa.download_maps, &fb.download_maps, yes_no);
    let (la, lb) = (&a.levels, &b.levels);
    value_change(&mut o, "levels", "graal", &la.graal, &lb.graal, yes_no);
    value_change(&mut o, "levels", "zelda", &la.zelda, &lb.zelda, yes_no);
    value_change(&mut o, "levels", "nw", &la.nw, &lb.nw, yes_no);
    value_change(&mut o, "levels", "gmap", &la.gmap, &lb.gmap, yes_no);
    value_change(&mut o, "levels", "heights", &la.heights, &lb.heights, yes_no);
    value_change(&mut o, "levels", "cached_links", &la.cached_links, &lb.cached_links, yes_no);
    value_change(&mut o, "life", "death", &a.life.death, &b.life.death, death_text);
    value_change(&mut o, "life", "combat", &a.life.combat, &b.life.combat, combat_text);
    let (ua, ub) = (&a.ui, &b.ui);
    value_change(&mut o, "ui", "server_list", &ua.server_list, &ub.server_list, list_text);
    value_change(&mut o, "ui", "chat_recall", &ua.chat_recall, &ub.chat_recall, yes_no);
    value_change(&mut o, "ui", "hud", &ua.hud, &ub.hud, dbg);
    value_change(&mut o, "ui", "minimap", &ua.minimap, &ub.minimap, yes_no);
    value_change(&mut o, "ui", "editor", &ua.editor, &ub.editor, yes_no);
    value_change(&mut o, "ui", "ganis", &ua.ganis, &ub.ganis, yes_no);
    value_change(&mut o, "ui", "emoticons", &ua.emoticons, &ub.emoticons, yes_no);
    value_change(&mut o, "ui", "key_config", &ua.key_config, &ub.key_config, yes_no);
    value_change(&mut o, "ui", "log_window", &ua.log_window, &ub.log_window, yes_no);
    value_change(&mut o, "ui", "ghost_mode", &ua.ghost_mode, &ub.ghost_mode, yes_no);
    value_change(&mut o, "ui", "server_in_title", &ua.server_in_title, &ub.server_in_title, yes_no);
    value_change(&mut o, "ui", "chat_text", &ua.chat_text, &ub.chat_text, dbg);
    value_change(&mut o, "ui", "gs1_commands", &ua.gs1_commands, &ub.gs1_commands, dbg);
    value_change(&mut o, "mannerisms", "mannerisms", &a.mannerisms, &b.mannerisms, manner_text);
    value_change(&mut o, "drops", "drops", &a.drops, &b.drops, drops_text);
    o
}

fn drops_text(d: &ItemDrops) -> String {
    match *d {
        DROPS_133 => "client rolls bush and pot drops, applies pickups itself".into(),
        DROPS_138 => "client rolls, tells the server (PLI_ITEMADD), pickups go through PLI_ITEMTAKE".into(),
        DROPS_142 => "server drops (PLO_ITEMADD), pickups go through PLI_ITEMTAKE".into(),
        _ => format!("{d:?}"),
    }
}

fn manner_text(m: &Mannerisms) -> String {
    match *m {
        MANNER_139 => "1.39 (blink after the hurt pose, hide keeps the carry poses, 4-tile knockback)".into(),
        MANNER_141 => "NewMain 1.41 (pull lets go to grab, swim freezes when still, hide keeps the carry poses)".into(),
        MANNER_205 => "2.05 (hidden ganis, pull stays, swim runs on)".into(),
        MANNER_222 => "2.22 (as 2.05, swim restarts at the stop)".into(),
        _ => format!("{m:?}"),
    }
}

/// The changes of step `i` against step `i - 1`; empty for the origin.
pub fn changes(i: usize) -> Vec<Change> {
    if i == 0 {
        return Vec::new();
    }
    diff(&RESOLVED[i - 1], &RESOLVED[i])
}

/// Output flavours of [`render`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Markdown,
    Tsv,
}

fn basis_text(b: Basis) -> &'static str {
    match b {
        Basis::Binary => "binary",
        Basis::Inferred => "inferred",
    }
}

/// The handler-change table for the chain, or for one step if `only` is set.
pub fn render(format: Format, only: Option<ClientVersion>) -> String {
    let mut out = String::new();
    if format == Format::Tsv {
        out.push_str("step\tlabel\ttoken\tbasis\tkind\tarea\tfield\tdetail\n");
    }
    for (i, s) in CHAIN.iter().enumerate() {
        if only.is_some_and(|v| v != s.version) {
            continue;
        }
        let ch = changes(i);
        let basis = basis_text(s.basis);
        let mut rows: Vec<(&str, String, String, String)> = Vec::new();
        if i == 0 {
            rows.push(("origin", "-".into(), "-".into(), "start of the chain, written out in full as ORIGIN".into()));
        } else if ch.is_empty() {
            rows.push(("same", "-".into(), "-".into(), "no change against the step before".into()));
        }
        for c in &ch {
            rows.push(("changed", c.area.into(), c.field.into(), c.detail.clone()));
        }
        for u in s.unknown {
            rows.push(("unknown", u.area.into(), "-".into(), u.what.into()));
        }
        rows.push(("source", "-".into(), "-".into(), s.source.into()));
        match format {
            Format::Tsv => {
                for (kind, area, field, detail) in &rows {
                    out.push_str(&format!("{}\t{}\t{}\t{basis}\t{kind}\t{area}\t{field}\t{detail}\n", i, s.label, s.token));
                }
            }
            Format::Text => {
                out.push_str(&format!("{:>2}  {}  {}  {basis}\n", i, s.label, s.token));
                for (kind, area, field, detail) in &rows {
                    match *kind {
                        "changed" => out.push_str(&format!("      {area}.{field}: {detail}\n")),
                        "unknown" => out.push_str(&format!("      unknown ({area}): {detail}\n")),
                        "source" => out.push_str(&format!("      source: {detail}\n")),
                        _ => out.push_str(&format!("      {detail}\n")),
                    }
                }
            }
            Format::Markdown => {
                let date = if s.date.is_empty() { String::new() } else { format!(", linked {}", s.date) };
                out.push_str(&format!("#### {} (`{}`, {basis}{date})\n\n", s.label, s.token));
                for (kind, area, field, detail) in &rows {
                    match *kind {
                        "changed" => out.push_str(&format!("- {area}: **{field}**: {detail}\n")),
                        "unknown" => out.push_str(&format!("- unknown ({area}): {detail}\n")),
                        "source" => out.push_str(&format!("- source: {detail}\n")),
                        _ => out.push_str(&format!("- {detail}\n")),
                    }
                }
                out.push('\n');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PROFILES;

    /// The rows as they were written by hand before the chain existed, one
    /// `Debug` line per build. Builds added later need no line.
    const GOLDEN: &str = include_str!("profiles_golden.txt");

    #[test]
    fn chain_resolves_to_the_profile_table() {
        assert_eq!(PROFILES.len(), CHAIN.len());
        for (p, r) in PROFILES.iter().zip(RESOLVED.iter()) {
            assert_eq!(p, r);
        }
        for (i, s) in CHAIN.iter().enumerate() {
            assert_eq!(PROFILES[i].version, s.version);
            assert_eq!(PROFILES[i].label, s.label);
            assert_eq!(PROFILES[i].token, s.token);
        }
    }

    /// `row` without the text from `from` up to (not including) `until`.
    fn strip_field(row: &str, from: &str, until: &str) -> String {
        let a = row.find(from).expect("field");
        let b = a + row[a..].find(until).expect("next field");
        format!("{}{}", &row[..a].trim_end_matches(' '), &row[b..]).replace(",, udp:", ", udp:")
    }

    /// Behaviour is unchanged: every build that existed before the chain
    /// resolves to exactly the row that was written out by hand. One row was
    /// changed on purpose afterwards: every listserver is now `Listserver {
    /// host, port, kind, status, built_in, token }` on listserver.graal.in
    /// (classic login 14923, gen-5 login 14922; the old rows had the bare port the
    /// build carries, mostly the dead 14921). 2.17 then got its id 24
    /// sender (lab, 2026-10-30, `documentation/difftest-217.md` item 3).
    /// 2.00 to 2.04 then got combat rows read in their own binaries, and
    /// every combat row two more fields (`sword_power_store`,
    /// `packet_victim_gates`; Graal_analysis `client-internals/combat-200-204.md`).
    #[test]
    fn chain_equals_the_hand_written_rows() {
        let mut seen = 0;
        for line in GOLDEN.lines() {
            let (label, want) = line.split_once('\t').expect("label<TAB>row");
            let p = super::super::profile::by_label(label).unwrap_or_else(|| panic!("no profile {label}"));
            // Fields added after the hand-written rows existed (the post-login
            // frame `login`) are not in them: compare without them.
            let got = strip_field(&format!("{p:?}"), "login: ", ", udp:");
            // So did the chat font (`chat_text`).
            let got = strip_field(&got, ", chat_text: ", ", gs1_commands");
            // And the mannerisms, the last field.
            let got = match got.find(", mannerisms: ") {
                Some(i) => format!("{} }}", &got[..i]),
                None => got,
            };
            // The login shape and its basis (`Listserver::shape`) came later too.
            let got = if got.contains(", shape: ") { strip_field(&got, ", shape: ", " }") } else { got };
            if got != want {
                let at = got.bytes().zip(want.bytes()).position(|(a, b)| a != b).unwrap_or(got.len().min(want.len()));
                let ctx = |s: &str| s[at.saturating_sub(60)..(at + 80).min(s.len())].to_string();
                panic!("{label} differs near byte {at}:\n  chain: ...{}\n  was:   ...{}", ctx(&got), ctx(want));
            }
            seen += 1;
        }
        assert_eq!(seen, 26, "the golden file holds the 26 rows that existed before the chain");
    }

    /// Every id a delta adds is new at that step and every id it removes was
    /// there, with no id on both sides: a delta says only what changed.
    #[test]
    fn deltas_are_minimal() {
        let sets = |p: &VersionProfile| [p.protocol.plo, p.protocol.pli, p.props.own, p.props.other];
        for i in 1..CHAIN.len() {
            let s = &CHAIN[i];
            let before = sets(&RESOLVED[i - 1]);
            let d = &s.delta;
            for (n, delta) in [d.plo, d.pli, d.own_props, d.other_props].iter().enumerate() {
                let (minus, plus) = delta.expanded();
                for id in &plus {
                    assert!(!before[n].contains(*id), "{}: delta {n} adds id {id} that is already there", s.label);
                    assert!(!minus.contains(id), "{}: id {id} is on both sides", s.label);
                }
                for id in &minus {
                    assert!(before[n].contains(*id), "{}: delta {n} removes id {id} that was not there", s.label);
                }
                let mut all = plus.clone();
                all.sort();
                all.dedup();
                assert_eq!(all.len(), plus.len(), "{}: duplicate ids in a plus list", s.label);
            }
        }
    }

    /// A delta that sets a field to the value it already has is noise, and a
    /// step with no delta must say why. So: the delta is empty exactly when
    /// the resolved row did not change, and an empty step has an `unknown`.
    #[test]
    fn deltas_are_effective_and_empty_steps_explain_themselves() {
        for i in 1..CHAIN.len() {
            let s = &CHAIN[i];
            let ch = changes(i);
            assert_eq!(s.delta == Delta::NONE, ch.is_empty(), "{}: delta and resolved change disagree", s.label);
            if ch.is_empty() {
                assert!(!s.unknown.is_empty(), "{}: a step that changes nothing must say what is unknown", s.label);
            }
            assert!(!s.source.is_empty(), "{}", s.label);
        }
        assert!(changes(0).is_empty());
    }

    /// Where a step has no binary, it admits it.
    #[test]
    fn inferred_steps_say_what_is_unknown() {
        for s in CHAIN {
            if s.basis == Basis::Inferred {
                assert!(!s.unknown.is_empty(), "{}", s.label);
            }
        }
        // Builds with a binary behind them that are known to be incomplete.
        assert!(step(ClientVersion::V222).unknown.iter().any(|u| u.what.contains("36, 42 and 45")));
    }

    /// Identity: one step per `ClientVersion`, in the version enum's order,
    /// with unique labels and tokens.
    #[test]
    fn steps_cover_every_version_once() {
        let all = ClientVersion::all_versions();
        assert_eq!(CHAIN.len(), all.len());
        for (s, v) in CHAIN.iter().zip(all.iter()) {
            assert_eq!(s.version, *v);
        }
        assert_eq!(previous(ClientVersion::V121), None);
        assert_eq!(previous(ClientVersion::V133), Some(ClientVersion::V132));
        assert_eq!(previous(ClientVersion::V217), Some(ClientVersion::V216));
        assert_eq!(previous(ClientVersion::V231), Some(ClientVersion::V230));
        assert_eq!(ORIGIN, RESOLVED[0]);
    }

    /// A new version is a step and nothing else: a made-up build after 2.31
    /// resolves from the last row with no new code.
    #[test]
    fn adding_a_version_is_one_step() {
        let last = RESOLVED[RESOLVED.len() - 1];
        let next = Step {
            version: ClientVersion::V231,
            label: "2.4 (test)",
            token: "GNW00000",
            date: "",
            basis: Basis::Inferred,
            delta: Delta {
                plo: IdDelta::plus(&[192]),
                pli: IdDelta::plus_minus(&[31], &[24]),
                npc_max: Some(80),
                ..Delta::NONE
            },
            source: "test",
            unknown: &[Unknown { area: "all", what: "made up" }],
        };
        let p = apply(last, &next);
        assert!(p.protocol.plo.contains(192) && !last.protocol.plo.contains(192));
        assert!(p.protocol.pli.contains(31) && !p.protocol.pli.contains(24));
        assert_eq!(p.props.npc_max, 80);
        // Everything the step does not mention is the last row.
        assert_eq!(p.ui, last.ui);
        assert_eq!(p.files, last.files);
        assert_eq!(p.protocol.encryption, last.protocol.encryption);
        let ch = diff(&last, &p);
        assert_eq!(ch.len(), 3, "{ch:?}");
    }

    /// Facts the chain was built to show, read off the generated table.
    #[test]
    fn known_step_facts() {
        let ch = |v: ClientVersion| {
            let i = CHAIN.iter().position(|s| s.version == v).unwrap();
            changes(i)
        };
        let has = |c: &[Change], field: &str, text: &str| c.iter().any(|c| c.field.starts_with(field) && c.detail.contains(text));
        // 2.00 -> 2.01: ADJACENTLEVEL and HITOBJECTS become sendable.
        assert!(has(&ch(ClientVersion::V201), "pli", "+35 ADJACENTLEVEL"));
        assert!(has(&ch(ClientVersion::V201), "pli", "+36 HITOBJECTS"));
        // 2.02 rev 2: LANGUAGE.
        assert!(has(&ch(ClientVersion::V202R2), "pli", "+37 LANGUAGE"));
        assert!(has(&ch(ClientVersion::V202R2), "sends_language", "no -> yes"));
        // 2.04 drops PLO 26.
        assert!(has(&ch(ClientVersion::V204), "plo", "-26"));
        // Server ids 53 (2.00) to 61 (2.1 tests): the 2.1 step adds 49 and 169 to 174.
        assert!(has(&ch(ClientVersion::V210), "plo", "+49"));
        // 2.17 first sends TAMPERCHECK; 2.22 changes framing; 2.31 drops PLO 44.
        assert!(has(&ch(ClientVersion::V217), "pli", "+39 TAMPERCHECK"));
        assert!(has(&ch(ClientVersion::V222), "encryption", "Gen3 -> Gen5"));
        assert!(has(&ch(ClientVersion::V230), "plo", "-44"));
        assert!(has(&ch(ClientVersion::V230), "pli", "+46 ENTERLEVEL"));
    }

    /// `documentation/version-chain.tsv` is the generated table. To refresh
    /// it: `seihai --version-chain --chain-format tsv > documentation/version-chain.tsv`.
    #[test]
    fn checked_in_table_is_current() {
        let doc = include_str!("../../../../documentation/version-chain.tsv");
        assert!(
            doc == render(Format::Tsv, None),
            "documentation/version-chain.tsv is stale; run: seihai --version-chain --chain-format tsv > documentation/version-chain.tsv"
        );
    }

    /// The markdown copy between the markers in `documentation/version-chain.md`
    /// is the generated one. Refresh with `--chain-format md`.
    #[test]
    fn checked_in_markdown_is_current() {
        let doc = include_str!("../../../../documentation/version-chain.md");
        let begin = "<!-- version-chain:begin -->\n";
        let end = "<!-- version-chain:end -->";
        let a = doc.find(begin).expect("begin marker") + begin.len();
        let b = doc.find(end).expect("end marker");
        assert!(
            doc[a..b].trim() == render(Format::Markdown, None).trim(),
            "the table in documentation/version-chain.md is stale; paste the output of seihai --version-chain --chain-format md between the markers"
        );
    }

    #[test]
    fn render_one_step() {
        let t = render(Format::Text, Some(ClientVersion::V202R2));
        assert!(t.contains("2.02 rev2") && t.contains("+37 LANGUAGE"), "{t}");
        assert!(!t.contains("2.03"));
    }
}
