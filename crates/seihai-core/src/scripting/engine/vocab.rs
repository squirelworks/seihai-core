//! Script vocabulary, gated by client era.
//!
//! The tables are derived from the cleaned Graal_analysis notes: the 2.05
//! command reference, and the per-build vocabulary deltas
//! (`script_vocab_deltas`). A name is available from the era in its
//! `since` column on. Unknown or not-yet-available names are never a
//! parse failure: the parser records a diagnostic and the statement is
//! skipped at run time.

use crate::protocol::ClientVersion;

/// Script vocabulary generation, in release order.
///
/// One era per distinct GS1 command table, read from the client binaries
/// (Graal_analysis `npc-library/gs1-commands-by-version.tsv`, 61 builds).
/// Builds whose tables are identical share an era. The 2.1 to 2.16 eras
/// have no binary: they are the NewWorld beta 2 table plus the commands the
/// 2.1x release notes date (`client-internals/feature-timeline.md`), and
/// are inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Era {
    /// 1.21: 71 commands.
    V121,
    /// 1.28 beta 2: 84.
    V128B2,
    /// 1.3 test, 1.31 and 1.32: 85.
    V13T,
    /// 1.33: 88.
    V133,
    /// 1.34 and 1.35: 93. `seteffect` and `unseteffect` leave the table.
    V134,
    /// 1.36: 104.
    V136,
    /// 1.37: 106.
    V137,
    /// 1.38: 109.
    V138,
    /// 1.39: 111.
    V139,
    /// 1.41 rev 1: 126.
    V141,
    /// NewMain 1.42 (the NM24 and NM26 tables): 134. `seteffect` is back.
    V142,
    /// 2.00, all revisions: 142.
    V200,
    /// 2.01: 144.
    V201,
    /// 2.02, all revisions: 146.
    V202,
    /// 2.03: 148.
    V203,
    /// 2.04: 157.
    V204,
    /// 2.05 to 2.05 rev 2, the 2.1 tests and NewWorld beta 1: 161.
    V205,
    /// 2.1 (no binary): NewWorld beta 2's 165, with `play2` from the 2.1
    /// notes. Inferred.
    V21,
    /// 2.12 to 2.14 (no binary): adds `wraptext`, `wraptext2`. Inferred.
    V212,
    /// 2.15 (no binary): adds `showimg2`. Inferred.
    V215,
    /// 2.16 (no binary): adds `seteffectmode`. Inferred.
    V216,
    /// 2.17.1, 2.22, 2.3 and 2.31: 198 commands, the same in all three
    /// binaries.
    V217,
    /// The C++ engine as in Linux 3.04: 191. Drops `putobject`,
    /// `setbackpal`, `setbody`, `setbacktile`, `setbacktile2`,
    /// `drawovertrees`, and `savelog` (registered as `savelog2` twice).
    V304,
    /// 3.153 to 4.20 rev 8: 191. `savelog` is back, `with` is gone.
    V3153,
    /// 5.00 rev 7 and later: 193, adding `setspritesimage` and
    /// `setstatusimage`. Later builds were not scanned and use this table.
    V500,
}

impl Era {
    /// Every era, oldest first.
    pub const ALL: &'static [Era] = &[
        Era::V121,
        Era::V128B2,
        Era::V13T,
        Era::V133,
        Era::V134,
        Era::V136,
        Era::V137,
        Era::V138,
        Era::V139,
        Era::V141,
        Era::V142,
        Era::V200,
        Era::V201,
        Era::V202,
        Era::V203,
        Era::V204,
        Era::V205,
        Era::V21,
        Era::V212,
        Era::V215,
        Era::V216,
        Era::V217,
        Era::V304,
        Era::V3153,
        Era::V500,
    ];

    /// The command table a client version scripts with. Read from that
    /// build's binary, except 2.1 to 2.16 (inferred, see [`Era::V21`]).
    pub fn for_version(v: ClientVersion) -> Era {
        use ClientVersion::*;
        match v {
            V121 => Era::V121,
            V128B2 => Era::V128B2,
            V13T | V131 | V132 => Era::V13T,
            V133 => Era::V133,
            V134 | V135 => Era::V134,
            V136 => Era::V136,
            V137 => Era::V137,
            V138 => Era::V138,
            V139 => Era::V139,
            V141 => Era::V141,
            V142 => Era::V142,
            V200 => Era::V200,
            V201 => Era::V201,
            V202 | V202R2 => Era::V202,
            V203 => Era::V203,
            V204 => Era::V204,
            V205 => Era::V205,
            V210 => Era::V21,
            V212 | V213 | V214 => Era::V212,
            V215 => Era::V215,
            V216 => Era::V216,
            V217 | V222 | V230 | V231 => Era::V217,
            V304 => Era::V304,
            V3153 | V404R2 | V420R6 | V420R8 => Era::V3153,
            _ => Era::V500,
        }
    }

    /// True when the era's table was not read from a binary.
    pub fn inferred(self) -> bool {
        matches!(self, Era::V21 | Era::V212 | Era::V215 | Era::V216)
    }

    /// True for the C++ engine (3.04 and later), a separate code base.
    pub fn engine(self) -> bool {
        self >= Era::V304
    }

    pub fn label(self) -> &'static str {
        match self {
            Era::V121 => "1.21",
            Era::V128B2 => "1.28b2",
            Era::V13T => "1.3",
            Era::V133 => "1.33",
            Era::V134 => "1.34",
            Era::V136 => "1.36",
            Era::V137 => "1.37",
            Era::V138 => "1.38",
            Era::V139 => "1.39",
            Era::V141 => "1.41",
            Era::V142 => "1.42",
            Era::V200 => "2.00",
            Era::V201 => "2.01",
            Era::V202 => "2.02",
            Era::V203 => "2.03",
            Era::V204 => "2.04",
            Era::V205 => "2.05",
            Era::V21 => "2.1",
            Era::V212 => "2.12",
            Era::V215 => "2.15",
            Era::V216 => "2.16",
            Era::V217 => "2.17",
            Era::V304 => "3.04",
            Era::V3153 => "3.153",
            Era::V500 => "5.00",
        }
    }
}

/// How a command is carried out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exec {
    /// Handled inside the engine (flags, sleep, arrays, calls into other
    /// scripts).
    Engine,
    /// Becomes an [`Effect`](super::Effect) for the host to apply.
    Effect,
    /// Compiles in the 2.05 client but has no executor: does nothing.
    NoExec,
    /// A Seihai stub that is not part of the original table.
    Ext,
}

/// One command: name, argument signature, first era, how it runs.
///
/// Signature letters follow the notes: `R` number, `S` text (message and
/// string), `F` file, `Q` script file, `I` item, `B` baddy, `C` colour,
/// `D` direction, `L` carried object, `O` object name. `N` is Seihai's
/// own letter for a variable name (the first argument of `setarray`).
#[derive(Debug, Clone, Copy)]
pub struct Cmd {
    pub name: &'static str,
    pub sig: &'static str,
    pub since: Era,
    pub exec: Exec,
}

const fn c(name: &'static str, sig: &'static str, since: Era, exec: Exec) -> Cmd {
    Cmd {
        name,
        sig,
        since,
        exec,
    }
}

use Era::*;
use Exec::*;

/// The command table. Aliases (`setgif` and `setimg`) are two rows.
pub static COMMANDS: &[Cmd] = &[
    // Messages, sound, files.
    c("say", "R", V121, Effect),
    c("say2", "S", V141, Effect),
    c("message", "S", V121, Effect),
    c("play", "F", V121, Effect),
    c("playlooped", "F", V137, Effect),
    // 2.2 list: a sound at x, y with a volume.
    c("play2", "FRRR", V21, Effect),
    // 2.2 (GServer GS1): a line for the player's message window.
    c("sendrpgmessage", "S", V217, Effect),
    c("stopsound", "F", V137, Effect),
    c("stopmidi", "", V121, Effect),
    c("setmusicvolume", "RR", V200, Effect),
    c("openurl", "S", V121, Effect),
    c("openurl2", "SRR", V201, Effect),
    c("showfile", "F", V128B2, Effect),
    c("setbackpal", "F", V128B2, Effect),
    c("setletters", "F", V128B2, Effect),
    // Flags and variables.
    c("set", "S", V121, Engine),
    c("unset", "S", V121, Engine),
    c("setstring", "SS", V128B2, Engine),
    c("addstring", "SS", V21, Engine),
    // String lists (2.2 command list): comma-separated, in a flag.
    c("insertstring", "SRS", V21, Engine),
    c("replacestring", "SRS", V217, Engine),
    c("removestring", "SS", V21, Engine),
    c("deletestring", "SR", V21, Engine),
    c("setarray", "NR", V121, Engine),
    c("tokenize", "S", V202, Engine),
    c("tokenize2", "SS", V202, Engine),
    // Flow and calls.
    c("sleep", "R", V128B2, Engine),
    c("callnpc", "RS", V141, Engine),
    c("callweapon", "RS", V141, Engine),
    c("triggeraction", "RRSS", V203, Effect),
    // NPC appearance and behaviour.
    c("setgif", "F", V121, Effect),
    c("setimg", "F", V141, Effect),
    c("setgifpart", "FRRRR", V13T, Effect),
    c("setimgpart", "FRRRR", V141, Effect),
    c("show", "", V121, Effect),
    c("showlocal", "", V134, Effect),
    c("hide", "", V121, Effect),
    c("hidelocal", "", V134, Effect),
    c("dontblock", "", V121, Effect),
    c("dontblocklocal", "", V134, Effect),
    c("blockagain", "", V121, Effect),
    c("blockagainlocal", "", V134, Effect),
    c("drawoverplayer", "", V121, Effect),
    c("drawunderplayer", "", V121, Effect),
    c("drawaslight", "", V142, Effect),
    c("canbecarried", "", V121, Effect),
    c("cannotbecarried", "", V121, Effect),
    c("canbepushed", "", V121, Effect),
    c("cannotbepushed", "", V121, Effect),
    c("canbepulled", "", V121, Effect),
    c("cannotbepulled", "", V121, Effect),
    c("timershow", "", V121, Effect),
    c("timereverywhere", "", V128B2, Effect),
    c("setshape", "RRR", V142, Effect),
    c("setshape2", "RRN", V204, Effect),
    c("seteffect", "RRRR", V121, Effect),
    // From 2.17.1 (`npc-library/gs1-commands-by-version.md`): 0 light,
    // 1 transparency, 2 holes. The 3.153 GS2 client registers the same name.
    c("seteffectmode", "R", V216, Effect),
    c("setcoloreffect", "RRRR", V142, Effect),
    c("setzoomeffect", "R", V142, Effect),
    c("move", "RRRR", V203, Effect),
    c("destroy", "", V121, Effect),
    c("putnpc", "QQRR", V121, Effect),
    // Character NPCs.
    c("showcharacter", "", V128B2, Effect),
    c("setcharprop", "SS", V128B2, Effect),
    c("setcharani", "SS", V139, Effect),
    c("setchargender", "S", V200, Effect),
    c("carryobject", "L", V134, Effect),
    c("throwcarry", "", V134, Effect),
    c("takehorse", "R", V136, Effect),
    c("followplayer", "", V121, Effect),
    // The player.
    c("setlevel", "S", V121, Effect),
    c("setlevel2", "SRR", V138, Effect),
    c("sethead", "F", V121, Effect),
    c("setbody", "F", V133, Effect),
    c("setsword", "SR", V121, Effect),
    c("setshield", "SR", V121, Effect),
    c("setbow", "S", V121, Effect),
    c("setplayerdir", "D", V121, Effect),
    c("setskincolor", "C", V121, Effect),
    c("setcoatcolor", "C", V121, Effect),
    c("setsleevecolor", "C", V121, Effect),
    c("setshoecolor", "C", V121, Effect),
    c("setbeltcolor", "C", V121, Effect),
    c("setplayerprop", "SS", V128B2, Effect),
    c("setani", "SS", V139, Effect),
    c("replaceani", "SS", V204, Effect),
    c("setgender", "S", V200, Effect),
    c("hurt", "R", V121, Effect),
    c("freezeplayer", "R", V121, Effect),
    c("hideplayer", "R", V128B2, Effect),
    c("hidesword", "R", V121, Effect),
    c("takeplayercarry", "", V121, Effect),
    c("takeplayerhorse", "", V136, Effect),
    c("enableweapons", "", V121, Effect),
    c("disableweapons", "", V121, Effect),
    c("enabledefmovement", "", V200, Effect),
    c("disabledefmovement", "", V200, Effect),
    c("enableselectweapons", "", V204, Effect),
    c("disableselectweapons", "", V204, Effect),
    c("showstats", "R", V141, Effect),
    c("enablefeatures", "R", V217, Effect),
    c("attachplayertoobj", "RR", V204, Effect),
    c("detachplayer", "", V204, Effect),
    c("toinventory", "S", V121, Effect),
    c("toweapons", "S", V121, Effect),
    // Combat, items, level objects.
    c("lay", "I", V121, Effect),
    c("lay2", "IRR", V136, Effect),
    c("take", "I", V121, Effect),
    c("take2", "R", V136, Effect),
    c("removeitem", "R", V136, Effect),
    c("putbomb", "RRR", V121, Effect),
    c("putexplosion", "RRR", V121, Effect),
    c("putexplosion2", "RRRR", V141, Effect),
    c("putleaps", "RRR", V121, Effect),
    c("puthorse", "FRR", V121, Effect),
    c("removebomb", "R", V136, Effect),
    c("explodebomb", "R", V136, Effect),
    c("removearrow", "R", V136, Effect),
    c("reflectarrow", "R", V136, Effect),
    c("removeexplo", "R", V136, Effect),
    c("removehorse", "R", V136, Effect),
    c("shootarrow", "D", V121, Effect),
    c("shootfireball", "D", V121, Effect),
    c("shootfireblast", "D", V121, Effect),
    c("shootnuke", "D", V121, Effect),
    c("shootball", "", V121, Effect),
    c("spyfire", "RR", V141, Effect),
    c("hitplayer", "RRRR", V121, Effect),
    c("hitnpc", "RRRR", V138, Effect),
    c("hitobjects", "RRR", V201, Effect),
    c("putcomp", "BRR", V121, Effect),
    c("putnewcomp", "BRRSR", V121, Effect),
    c("hitcompu", "RRRR", V121, Effect),
    c("removecompus", "", V121, Effect),
    c("noplayerkilling", "", V121, Effect),
    // Board, map, images.
    c("updateboard", "RRRR", V121, Effect),
    c("putobject", "ORR", V121, Effect),
    c("setmap", "FFRR", V128B2, Effect),
    c("setminimap", "FFRR", V128B2, Effect),
    c("loadmap", "F", V205, Effect),
    c("addtiledef", "SSR", V204, Effect),
    c("addtiledef2", "SSRR", V204, Effect),
    c("removetiledefs", "S", V204, Effect),
    c("showimg", "RFRR", V121, Effect),
    c("changeimgpart", "RRRRR", V138, Effect),
    c("changeimgvis", "RR", V141, Effect),
    c("changeimgcolors", "RRRRR", V142, Effect),
    c("changeimgzoom", "RR", V142, Effect),
    c("hideimg", "R", V121, Effect),
    c("hideimgs", "RR", V217, Effect),
    c("showtext", "RRSSS", V217, Effect),
    // Parsed but ignored by the 2.05 client.
    c("seturllevel", "S", V121, NoExec),
    c("setfocus", "RR", V133, NoExec),
    c("resetfocus", "", V133, NoExec),
    c("setbacktile", "R", V134, NoExec),
    c("canwarp", "", V141, NoExec),
    c("canwarp2", "", V142, NoExec),
    c("drawovertrees", "", V141, NoExec),
    c("join", "S", V200, NoExec),
    c("freezeplayer2", "", V200, NoExec),
    c("unfreezeplayer", "", V200, NoExec),
    c("setbacktile2", "RRRRR", V205, NoExec),
    c("setz", "RRRRRRRR", V205, NoExec),
    c("updateterrain", "", V205, NoExec),
    // 1.21 to 1.33 only. The executor was not read; Seihai ignores it.
    c("unseteffect", "", V121, NoExec),
    // First in 2.17.1 (none of 2.12 to 2.16 has a binary; 2.12 and 2.15 are
    // dated by the release notes). The host draws or sends them; Seihai's
    // own engine passes them on as commands.
    c("wraptext", "RSS", V212, Effect),
    c("wraptext2", "RRSS", V212, Effect),
    c("showimg2", "RFRRR", V215, Effect),
    c("putnpc2", "RRS", V217, Effect),
    c("showtext2", "RRRRSSS", V217, Effect),
    c("showani", "RRRRSS", V217, Effect),
    c("showani2", "RRRRRSS", V217, Effect),
    c("showpoly", "RS", V217, Effect),
    c("showpoly2", "RS", V217, Effect),
    c("changeimgmode", "RR", V217, Effect),
    c("shoot", "RRRRRRSS", V217, Effect),
    c("setshootparams", "S", V217, Effect),
    c("serverwarp", "S", V217, Effect),
    c("sendpm", "S", V217, Effect),
    c("setpm", "S", V217, Effect),
    c("sendtonc", "S", V217, Effect),
    c("sendtorc", "S", V217, Effect),
    c("savelog", "S", V217, Effect),
    c("savelog2", "SS", V217, Effect),
    c("setcursor", "F", V217, Effect),
    c("setcursor2", "S", V217, Effect),
    c("enablemap", "", V217, Effect),
    c("disablemap", "", V217, Effect),
    c("enablepause", "", V217, Effect),
    c("disablepause", "", V217, Effect),
    // 5.00 rev 7.
    c("setspritesimage", "F", V500, Effect),
    c("setstatusimage", "F", V500, Effect),
    // Seihai stub: not in the 2.05 table. Records the text for the host.
    c("sendtoserver", "S", V21, Ext),
];

/// Look up a command by (lower-case) name, any era.
pub fn command(name: &str) -> Option<&'static Cmd> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// Commands that leave the table after their `since` era: `(name, first
/// era without it, first era with it again)`, `None` for never. Read from
/// the per-build tables (`gs1-commands-by-version.tsv`).
pub static ABSENT: &[(&str, Era, Option<Era>)] = &[
    // Gone in 1.34 to 1.41 rev 1; back in NewMain 1.42.
    ("seteffect", V134, Some(V142)),
    // 1.21 to 1.33 only.
    ("unseteffect", V134, None),
    // The C++ engine never registers these.
    ("putobject", V304, None),
    ("setbackpal", V304, None),
    ("setbody", V304, None),
    ("setbacktile", V304, None),
    ("setbacktile2", V304, None),
    ("drawovertrees", V304, None),
    // 3.04 registers `savelog2` twice and `savelog` not at all; 3.153 has it.
    ("savelog", V304, Some(V3153)),
];

/// True when the per-build tables show `name` missing from `era` although
/// it was added earlier.
pub fn absent_in(name: &str, era: Era) -> bool {
    ABSENT
        .iter()
        .any(|(n, gone, back)| *n == name && era >= *gone && back.is_none_or(|b| era < b))
}

/// The command if it exists in `era`.
pub fn command_in(name: &str, era: Era) -> Option<&'static Cmd> {
    command(name).filter(|c| c.since <= era && !absent_in(c.name, era))
}

/// Script words that sit in the command table but are parsed as
/// statements: `(word, first era, first era without it)`. `if` and `else`
/// are in every table that has scripts.
pub static KEYWORDS: &[(&str, Era, Option<Era>)] = &[
    ("if", V121, None),
    ("else", V121, None),
    ("for", V121, None),
    ("while", V128B2, None),
    ("function", V141, None),
    ("return", V141, None),
    ("break", V141, None),
    ("continue", V141, None),
    // Registered by 3.04 but by no later engine: in 4.04r2 the only `with`
    // string belongs to the GS2 compiler.
    ("with", V217, Some(V3153)),
];

/// True when `word` is a statement keyword of `era`'s table.
pub fn keyword_in(word: &str, era: Era) -> bool {
    KEYWORDS
        .iter()
        .any(|(w, since, gone)| *w == word && *since <= era && gone.is_none_or(|g| era < g))
}

/// Commands that need an NPC server online (or play offline): they do
/// nothing when connected to a server without one.
pub fn online_gated(name: &str) -> bool {
    matches!(
        name,
        "seteffect"
            | "setcoloreffect"
            | "setzoomeffect"
            | "changeimgcolors"
            | "changeimgzoom"
            | "addtiledef"
            | "addtiledef2"
            | "removetiledefs"
            | "replaceani"
            | "attachplayertoobj"
    )
}

/// Commands that end the current script run once executed.
pub fn ends_run(name: &str) -> bool {
    matches!(name, "setlevel" | "setlevel2" | "destroy")
}

/// One argument kind, a letter of a signature.
pub fn is_text_letter(l: char) -> bool {
    matches!(l, 'S' | 'F' | 'Q' | 'O' | 'N')
}

/// Name tables used by `I`, `B`, `C`, `D` and `L` arguments.
pub const ITEMS: &[&str] = &[
    "greenrupee",
    "bluerupee",
    "redrupee",
    "bombs",
    "darts",
    "heart",
    "glove1",
    "bow",
    "bomb",
    "shield",
    "sword",
    "fullheart",
    "superbomb",
    "battleaxe",
    "goldensword",
    "mirrorshield",
    "glove2",
    "lizardshield",
    "lizardsword",
    "goldrupee",
    "fireball",
    "fireblast",
    "nukeshot",
    "joltbomb",
    "spinattack",
];
pub const BADDIES: &[&str] = &[
    "graysoldier",
    "bluesoldier",
    "redsoldier",
    "shootingsoldier",
    "swampsoldier",
    "frog",
    "octopus",
    "goldenwarrior",
    "lizardon",
    "dragon",
];
pub const COLORS: &[&str] = &[
    "white",
    "yellow",
    "orange",
    "pink",
    "red",
    "darkred",
    "lightgreen",
    "green",
    "darkgreen",
    "lightblue",
    "blue",
    "darkblue",
    "brown",
    "cynober",
    "purple",
    "darkpurple",
    "lightgray",
    "gray",
    "black",
    "transparent",
];
pub const CARRY: &[&str] = &[
    "bush",
    "sign",
    "vase",
    "stone",
    "blackstone",
    "bomb",
    "hotbomb",
    "superbomb",
    "joltbomb",
    "hotjoltbomb",
    "none",
];
pub const DIRS: &[&str] = &["up", "left", "down", "right"];

/// Number for a name in the table a signature letter selects.
pub fn named_constant(letter: char, name: &str) -> Option<f64> {
    let table = match letter {
        'I' => ITEMS,
        'B' => BADDIES,
        'C' => COLORS,
        'L' => CARRY,
        'D' => DIRS,
        _ => return None,
    };
    table.iter().position(|n| *n == name).map(|i| i as f64)
}

/// Events (the `if (created)` flags). `action*` names are open ended and
/// handled by [`event_since`].
pub static EVENTS: &[(&str, Era)] = &[
    ("created", V200),
    ("playerenters", V200),
    ("playertouchsme", V200),
    ("playertouchsother", V200),
    ("playerendsreading", V200),
    ("playerchats", V200),
    ("playerlaysitem", V200),
    ("timeout", V200),
    ("wasshot", V200),
    ("wasshooted", V200),
    ("washit", V200),
    ("wasthrown", V200),
    ("waspelt", V200),
    // Moved by a push or a pull (`canbepushed`, `canbepulled`).
    ("waspushed", V200),
    ("waspulled", V200),
    ("exploded", V200),
    ("compusdied", V200),
    ("compsdead", V200),
    ("shotbyplayer", V200),
    ("shotbybaddy", V200),
    ("weaponfired", V200),
    ("weaponselected", V200),
    ("playerdies", V201),
    ("playerhurt", V201),
    ("movementfinished", V203),
    // Server-side: the NPC-server (2001, 2.x scripts) runs it when a player
    // leaves the NPC's level, not on disconnect (Graal_analysis
    // `tools/npcserver-2001.md`). It is absent from the 2.05, 2.17.1 and
    // 2.22 clients that were searched, so no client fires it; a client
    // script that tests it reads false, as an unknown flag does there.
    ("playerleaves", V21),
    // SEIHAI EXTENSION, not original Graal GS1 behaviour. For
    // seihai-npcserver's level-less Control-NPC: a
    // player finished logging in to, or left, the game server. No GS1
    // source names these. The only sample is a later GS2 Control-NPC
    // (`onPlayerLogin(player)` / `onPlayerLogout(player)`, Graal_analysis
    // `npc-library/reference/npcControl-NPC.txt`), and the 2001 NPC-server
    // notes do not list them. Server-side only; no client fires them.
    ("playerlogin", V21),
    ("playerlogout", V21),
];

/// First era an event name exists in, or `None` if it is not an event.
/// Any name starting with `action` is a `triggeraction` event from 2.03.
pub fn event_since(name: &str) -> Option<Era> {
    if let Some((_, e)) = EVENTS.iter().find(|(n, _)| *n == name) {
        return Some(*e);
    }
    if name.starts_with("action") && name.len() > 6 {
        return Some(V203);
    }
    None
}

/// Read-only status flags: true or false, supplied by the host view.
/// `(name, first era)`.
pub static STATUS_FLAGS: &[(&str, Era)] = &[
    ("visible", V200),
    ("carrying", V200),
    ("playerismale", V200),
    ("playerisfemale", V200),
    ("playeronhorse", V200),
    ("playerswimming", V200),
    ("playerreading", V201),
    ("isleader", V200),
    ("isweapon", V200),
    ("weaponsenabled", V200),
    ("followsplayer", V200),
    ("issparringzone", V202),
    ("nopkzone", V202),
    ("isonmap", V200),
    ("lighteffectsenabled", V203),
    ("playerattached", V204),
];

/// Built-in numeric variables: `(name, first era, writable)`.
pub static BUILTIN_VARS: &[(&str, Era, bool)] = &[
    ("x", V200, true),
    ("y", V200, true),
    ("timeout", V200, true),
    ("playerx", V200, true),
    ("playery", V200, true),
    ("playerdir", V200, true),
    ("playerhearts", V200, true),
    ("playerrupees", V200, true),
    ("playerbombs", V200, true),
    ("playerdarts", V200, true),
    ("playerswordpower", V200, true),
    ("playershieldpower", V200, true),
    ("playerfullhearts", V21, false),
    ("playerhp", V21, true),
    ("playermaxhp", V21, false),
    ("vecx", V203, false),
    ("vecy", V203, false),
    ("tokenscount", V202, false),
    ("paramscount", V203, false),
    ("weaponscount", V204, false),
    // The selected weapon's index in the whole list; a script can set it
    // (2.04 `newfeatures2001.txt`).
    ("selectedweapon", V204, true),
    // GS2-style names, used by scripts on servers that run NPCs (GServer GS2
    // reference); `chat` is the object's bubble text, like `message`.
    ("chat", V21, true),
    ("player.chat", V21, true),
    ("player.account", V21, false),
    ("player.nick", V21, false),
];

pub fn builtin_var(name: &str, era: Era) -> Option<(&'static str, bool)> {
    BUILTIN_VARS
        .iter()
        .find(|(n, e, _)| *n == name && *e <= era)
        .map(|(n, _, w)| (*n, *w))
}

pub fn status_flag(name: &str, era: Era) -> bool {
    STATUS_FLAGS.iter().any(|(n, e)| *n == name && *e <= era)
}

/// Kind of a built-in function argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgKind {
    Num,
    /// Raw text up to the next comma or closing parenthesis.
    Text,
}

/// A built-in function: name, argument kinds, first era.
pub struct Builtin {
    pub name: &'static str,
    pub args: &'static [ArgKind],
    pub since: Era,
}

use ArgKind::{Num as N, Text as T};

pub static BUILTINS: &[Builtin] = &[
    Builtin {
        name: "abs",
        args: &[N],
        since: V200,
    },
    Builtin {
        name: "int",
        args: &[N],
        since: V200,
    },
    Builtin {
        name: "sin",
        args: &[N],
        since: V200,
    },
    Builtin {
        name: "cos",
        args: &[N],
        since: V200,
    },
    Builtin {
        name: "arctan",
        args: &[N],
        since: V200,
    },
    Builtin {
        name: "random",
        args: &[N, N],
        since: V200,
    },
    Builtin {
        name: "strequals",
        args: &[T, T],
        since: V200,
    },
    Builtin {
        name: "strlen",
        args: &[T],
        since: V202,
    },
    Builtin {
        name: "startswith",
        args: &[T, T],
        since: V202,
    },
    Builtin {
        name: "indexof",
        args: &[T, T],
        since: V202,
    },
    Builtin {
        name: "ascii",
        args: &[T],
        since: V21,
    },
    // 2.2 command list. `lindexof` is in NewWorld beta 2; `onmapx` and
    // `onmapy` are in 2.03's string table (`script_vocab_deltas.tsv`).
    Builtin {
        name: "strcontains",
        args: &[T, T],
        since: V21,
    },
    Builtin {
        name: "lindexof",
        args: &[T, T],
        since: V21,
    },
    Builtin {
        name: "onmapx",
        args: &[T],
        since: V203,
    },
    Builtin {
        name: "onmapy",
        args: &[T],
        since: V203,
    },
    Builtin {
        name: "arraylen",
        args: &[T],
        since: V200,
    },
    // Level collision at a point in tiles; true off the board.
    Builtin {
        name: "onwall",
        args: &[N, N],
        since: V200,
    },
    // Liquid (water or lava) at a point in tiles; false off the board.
    Builtin {
        name: "onwater",
        args: &[N, N],
        since: V200,
    },
    // -1 on the default tileset, as in 2.05 (see documentation/tile-types.md).
    Builtin {
        name: "tiletype",
        args: &[N, N],
        since: V200,
    },
    // Chat tests, true only during `playerchats`. [inferred: whole line / contained text]
    Builtin {
        name: "playersays",
        args: &[T],
        since: V200,
    },
    Builtin {
        name: "playersays2",
        args: &[T],
        since: V200,
    },
    // Stubs: return a value so scripts do not log every call.
    Builtin {
        name: "strtofloat",
        args: &[T],
        since: V200,
    },
    Builtin {
        name: "hasweapon",
        args: &[T],
        since: V200,
    },
    Builtin {
        name: "keydown",
        args: &[N],
        since: V200,
    },
    Builtin {
        name: "keydown2",
        args: &[N],
        since: V200,
    },
    Builtin {
        name: "sarraylen",
        args: &[T],
        since: V200,
    },
    // `with (getnpc(name))`, `with (getplayer(account))`: the targets of
    // `with` (2.17 and later). Seihai does not switch objects; they read 0.
    Builtin {
        name: "getnpc",
        args: &[T],
        since: V217,
    },
    Builtin {
        name: "getplayer",
        args: &[T],
        since: V217,
    },
];

pub fn builtin_fn(name: &str, era: Era) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name && b.since <= era)
}

/// True if `name` is a built-in function in any era.
pub fn is_builtin_fn_any(name: &str) -> bool {
    BUILTINS.iter().any(|b| b.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eras_follow_release_order() {
        assert!(Era::ALL.windows(2).all(|w| w[0] < w[1]));
        assert!(Era::V200 < Era::V202 && Era::V205 < Era::V21 && Era::V21 < Era::V217);
        assert_eq!(Era::for_version(ClientVersion::V202R2), Era::V202);
        assert_eq!(Era::for_version(ClientVersion::V222), Era::V217);
        assert_eq!(Era::for_version(ClientVersion::V133), Era::V133);
        assert_eq!(Era::for_version(ClientVersion::V135), Era::V134);
        assert_eq!(Era::for_version(ClientVersion::V304), Era::V304);
        assert_eq!(Era::for_version(ClientVersion::V420R8), Era::V3153);
        assert_eq!(Era::for_version(ClientVersion::V603R7), Era::V500);
        assert!(Era::V215.inferred() && !Era::V217.inferred());
        for v in ClientVersion::all_versions() {
            let _ = Era::for_version(*v);
        }
    }

    /// Command counts per era, as `gs1-commands-by-version.md` gives them for
    /// each build's table. Seihai's `sendtoserver` stub is not counted, and
    /// the statement words are counted from [`KEYWORDS`].
    #[test]
    fn command_counts_match_the_build_tables() {
        let count = |era: Era| {
            COMMANDS
                .iter()
                .filter(|c| c.exec != Exec::Ext && command_in(c.name, era).is_some())
                .count()
                + KEYWORDS
                    .iter()
                    .filter(|(w, _, _)| keyword_in(w, era))
                    .count()
        };
        for (era, n) in [
            (Era::V121, 71),
            (Era::V128B2, 84),
            (Era::V13T, 85),
            (Era::V133, 88),
            (Era::V134, 93),
            (Era::V136, 104),
            (Era::V137, 106),
            (Era::V138, 109),
            (Era::V139, 111),
            (Era::V141, 126),
            (Era::V142, 134),
            (Era::V200, 142),
            (Era::V201, 144),
            (Era::V202, 146),
            (Era::V203, 148),
            (Era::V204, 157),
            (Era::V205, 161),
            (Era::V217, 198),
            (Era::V304, 191),
            (Era::V3153, 191),
            (Era::V500, 193),
        ] {
            assert_eq!(count(era), n, "{}", era.label());
        }
    }

    #[test]
    fn removals_follow_the_build_tables() {
        assert!(command_in("seteffect", Era::V133).is_some());
        assert!(command_in("seteffect", Era::V141).is_none());
        assert!(command_in("seteffect", Era::V142).is_some());
        assert!(command_in("unseteffect", Era::V133).is_some());
        assert!(command_in("unseteffect", Era::V205).is_none());
        assert!(command_in("putobject", Era::V217).is_some());
        assert!(command_in("putobject", Era::V304).is_none());
        assert!(command_in("savelog", Era::V304).is_none());
        assert!(command_in("savelog", Era::V3153).is_some());
        assert!(keyword_in("with", Era::V304) && !keyword_in("with", Era::V3153));
        assert!(!keyword_in("while", Era::V121) && keyword_in("while", Era::V128B2));
        assert!(!keyword_in("function", Era::V139) && keyword_in("function", Era::V141));
        assert!(command_in("move", Era::V202).is_none());
        assert!(command_in("showtext", Era::V205).is_none());
        assert!(command_in("showtext", Era::V217).is_some());
    }

    #[test]
    fn command_names_are_unique() {
        for (i, a) in COMMANDS.iter().enumerate() {
            assert!(
                COMMANDS[i + 1..].iter().all(|b| b.name != a.name),
                "{}",
                a.name
            );
        }
    }

    #[test]
    fn gating_follows_the_vocab_deltas() {
        assert!(command_in("hitobjects", Era::V200).is_none());
        assert!(command_in("hitobjects", Era::V201).is_some());
        assert!(command_in("tokenize2", Era::V201).is_none());
        assert!(command_in("tokenize2", Era::V202).is_some());
        assert!(command_in("triggeraction", Era::V202).is_none());
        assert!(command_in("triggeraction", Era::V203).is_some());
        assert!(command_in("setshape2", Era::V203).is_none());
        assert!(command_in("setshape2", Era::V204).is_some());
        assert!(command_in("loadmap", Era::V204).is_none());
        assert!(command_in("loadmap", Era::V205).is_some());
        assert!(builtin_fn("strlen", Era::V201).is_none());
        assert!(builtin_fn("strlen", Era::V202).is_some());
        assert!(builtin_fn("ascii", Era::V205).is_none());
        assert!(builtin_fn("ascii", Era::V21).is_some());
        assert_eq!(event_since("playerdies"), Some(Era::V201));
        assert_eq!(event_since("actionpushed"), Some(Era::V203));
        assert_eq!(event_since("playerleaves"), Some(Era::V21));
        assert_eq!(event_since("playerlogin"), Some(Era::V21));
        assert_eq!(event_since("playerlogout"), Some(Era::V21));
        assert_eq!(event_since("playerlogin"), event_since("playerleaves"));
        assert_eq!(event_since("action"), None);
        assert!(builtin_var("vecx", Era::V202).is_none());
        assert!(builtin_var("vecx", Era::V203).is_some());
        assert!(!status_flag("playerattached", Era::V203));
        assert!(status_flag("playerattached", Era::V204));
    }

    #[test]
    fn signatures_name_known_letters() {
        for c in COMMANDS {
            assert!(
                c.sig.chars().all(|l| "RSFQIBCDLON".contains(l)),
                "{} {}",
                c.name,
                c.sig
            );
        }
    }

    #[test]
    fn name_tables_match_the_notes() {
        assert_eq!(ITEMS.len(), 25);
        assert_eq!(BADDIES.len(), 10);
        assert_eq!(COLORS.len(), 20);
        assert_eq!(CARRY.len(), 11);
        assert_eq!(named_constant('D', "left"), Some(1.0));
        assert_eq!(named_constant('C', "transparent"), Some(19.0));
        assert_eq!(named_constant('I', "spinattack"), Some(24.0));
        assert_eq!(named_constant('L', "none"), Some(10.0));
        assert_eq!(named_constant('R', "left"), None);
    }

    #[test]
    fn online_gate_and_run_ending_lists() {
        assert!(online_gated("seteffect") && !online_gated("message"));
        assert!(ends_run("setlevel2") && !ends_run("hide"));
    }
}
