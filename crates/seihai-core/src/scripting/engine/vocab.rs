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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Era {
    /// 2.00 and earlier builds.
    V200,
    V201,
    /// 2.02, both revisions.
    V202,
    V203,
    V204,
    V205,
    /// 2.1 test builds and everything after, including 2.22 and 2.31.
    V21,
}

impl Era {
    /// The vocabulary a client version scripts with. Builds before 2.00
    /// use the 2.00 set, which is the earliest the notes cover.
    pub fn for_version(v: ClientVersion) -> Era {
        use ClientVersion::*;
        match v {
            V133 | V134 | V135 | V136 | V137 | V138 | V139 | V141 | V142 | V200 => Era::V200,
            V201 => Era::V201,
            V202 | V202R2 => Era::V202,
            V203 => Era::V203,
            V204 => Era::V204,
            V205 => Era::V205,
            _ => Era::V21,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Era::V200 => "2.00",
            Era::V201 => "2.01",
            Era::V202 => "2.02",
            Era::V203 => "2.03",
            Era::V204 => "2.04",
            Era::V205 => "2.05",
            Era::V21 => "2.1+",
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
    Cmd { name, sig, since, exec }
}

use Era::*;
use Exec::*;

/// The command table. Aliases (`setgif` and `setimg`) are two rows.
pub static COMMANDS: &[Cmd] = &[
    // Messages, sound, files.
    c("say", "R", V200, Effect),
    c("say2", "S", V200, Effect),
    c("message", "S", V200, Effect),
    c("play", "F", V200, Effect),
    c("playlooped", "F", V200, Effect),
    // 2.2 list: a sound at x, y with a volume.
    c("play2", "FRRR", V21, Effect),
    // 2.2 (GServer GS1): a line for the player's message window.
    c("sendrpgmessage", "S", V21, Effect),
    c("stopsound", "F", V200, Effect),
    c("stopmidi", "", V200, Effect),
    c("setmusicvolume", "RR", V200, Effect),
    c("openurl", "S", V200, Effect),
    c("openurl2", "SRR", V200, Effect),
    c("showfile", "F", V200, Effect),
    c("setbackpal", "F", V200, Effect),
    c("setletters", "F", V200, Effect),
    // Flags and variables.
    c("set", "S", V200, Engine),
    c("unset", "S", V200, Engine),
    c("setstring", "SS", V200, Engine),
    c("addstring", "SS", V200, Engine),
    // String lists (2.2 command list): comma-separated, in a flag.
    c("insertstring", "SRS", V21, Engine),
    c("replacestring", "SRS", V21, Engine),
    c("removestring", "SS", V21, Engine),
    c("deletestring", "SR", V21, Engine),
    c("setarray", "NR", V200, Engine),
    c("tokenize", "S", V202, Engine),
    c("tokenize2", "SS", V202, Engine),
    // Flow and calls.
    c("sleep", "R", V200, Engine),
    c("callnpc", "RS", V200, Engine),
    c("callweapon", "RS", V200, Engine),
    c("triggeraction", "RRSS", V203, Effect),
    // NPC appearance and behaviour.
    c("setgif", "F", V200, Effect),
    c("setimg", "F", V200, Effect),
    c("setgifpart", "FRRRR", V200, Effect),
    c("setimgpart", "FRRRR", V200, Effect),
    c("show", "", V200, Effect),
    c("showlocal", "", V200, Effect),
    c("hide", "", V200, Effect),
    c("hidelocal", "", V200, Effect),
    c("dontblock", "", V200, Effect),
    c("dontblocklocal", "", V200, Effect),
    c("blockagain", "", V200, Effect),
    c("blockagainlocal", "", V200, Effect),
    c("drawoverplayer", "", V200, Effect),
    c("drawunderplayer", "", V200, Effect),
    c("drawaslight", "", V200, Effect),
    c("canbecarried", "", V200, Effect),
    c("cannotbecarried", "", V200, Effect),
    c("canbepushed", "", V200, Effect),
    c("cannotbepushed", "", V200, Effect),
    c("canbepulled", "", V200, Effect),
    c("cannotbepulled", "", V200, Effect),
    c("timershow", "", V200, Effect),
    c("timereverywhere", "", V200, Effect),
    c("setshape", "RRR", V200, Effect),
    c("setshape2", "RRN", V204, Effect),
    c("seteffect", "RRRR", V200, Effect),
    c("setcoloreffect", "RRRR", V200, Effect),
    c("setzoomeffect", "R", V200, Effect),
    c("move", "RRRR", V200, Effect),
    c("destroy", "", V200, Effect),
    c("putnpc", "QQRR", V200, Effect),
    // Character NPCs.
    c("showcharacter", "", V200, Effect),
    c("setcharprop", "SS", V200, Effect),
    c("setcharani", "SS", V200, Effect),
    c("setchargender", "S", V200, Effect),
    c("carryobject", "L", V200, Effect),
    c("throwcarry", "", V200, Effect),
    c("takehorse", "R", V200, Effect),
    c("followplayer", "", V200, Effect),
    // The player.
    c("setlevel", "S", V200, Effect),
    c("setlevel2", "SRR", V200, Effect),
    c("sethead", "F", V200, Effect),
    c("setbody", "F", V200, Effect),
    c("setsword", "SR", V200, Effect),
    c("setshield", "SR", V200, Effect),
    c("setbow", "S", V200, Effect),
    c("setplayerdir", "D", V200, Effect),
    c("setskincolor", "C", V200, Effect),
    c("setcoatcolor", "C", V200, Effect),
    c("setsleevecolor", "C", V200, Effect),
    c("setshoecolor", "C", V200, Effect),
    c("setbeltcolor", "C", V200, Effect),
    c("setplayerprop", "SS", V200, Effect),
    c("setani", "SS", V200, Effect),
    c("replaceani", "SS", V204, Effect),
    c("setgender", "S", V200, Effect),
    c("hurt", "R", V200, Effect),
    c("freezeplayer", "R", V200, Effect),
    c("hideplayer", "R", V200, Effect),
    c("hidesword", "R", V200, Effect),
    c("takeplayercarry", "", V200, Effect),
    c("takeplayerhorse", "", V200, Effect),
    c("enableweapons", "", V200, Effect),
    c("disableweapons", "", V200, Effect),
    c("enabledefmovement", "", V200, Effect),
    c("disabledefmovement", "", V200, Effect),
    c("enableselectweapons", "", V204, Effect),
    c("disableselectweapons", "", V204, Effect),
    c("showstats", "R", V200, Effect),
    c("attachplayertoobj", "RR", V204, Effect),
    c("detachplayer", "", V204, Effect),
    c("toinventory", "S", V200, Effect),
    c("toweapons", "S", V200, Effect),
    // Combat, items, level objects.
    c("lay", "I", V200, Effect),
    c("lay2", "IRR", V200, Effect),
    c("take", "I", V200, Effect),
    c("take2", "R", V200, Effect),
    c("removeitem", "R", V200, Effect),
    c("putbomb", "RRR", V200, Effect),
    c("putexplosion", "RRR", V200, Effect),
    c("putexplosion2", "RRRR", V200, Effect),
    c("putleaps", "RRR", V200, Effect),
    c("puthorse", "FRR", V200, Effect),
    c("removebomb", "R", V200, Effect),
    c("explodebomb", "R", V200, Effect),
    c("removearrow", "R", V200, Effect),
    c("reflectarrow", "R", V200, Effect),
    c("removeexplo", "R", V200, Effect),
    c("removehorse", "R", V200, Effect),
    c("shootarrow", "D", V200, Effect),
    c("shootfireball", "D", V200, Effect),
    c("shootfireblast", "D", V200, Effect),
    c("shootnuke", "D", V200, Effect),
    c("shootball", "", V200, Effect),
    c("spyfire", "RR", V200, Effect),
    c("hitplayer", "RRRR", V200, Effect),
    c("hitnpc", "RRRR", V200, Effect),
    c("hitobjects", "RRR", V201, Effect),
    c("putcomp", "BRR", V200, Effect),
    c("putnewcomp", "BRRSR", V200, Effect),
    c("hitcompu", "RRRR", V200, Effect),
    c("removecompus", "", V200, Effect),
    c("noplayerkilling", "", V200, Effect),
    // Board, map, images.
    c("updateboard", "RRRR", V200, Effect),
    c("putobject", "ORR", V200, Effect),
    c("setmap", "FFRR", V200, Effect),
    c("setminimap", "FFRR", V200, Effect),
    c("loadmap", "F", V205, Effect),
    c("addtiledef", "SSR", V204, Effect),
    c("addtiledef2", "SSRR", V204, Effect),
    c("removetiledefs", "S", V204, Effect),
    c("showimg", "RFRR", V200, Effect),
    c("changeimgpart", "RRRRR", V200, Effect),
    c("changeimgvis", "RR", V200, Effect),
    c("changeimgcolors", "RRRRR", V200, Effect),
    c("changeimgzoom", "RR", V200, Effect),
    c("hideimg", "R", V200, Effect),
    c("hideimgs", "RR", V200, Effect),
    c("showtext", "RRSSS", V200, Effect),
    // Parsed but ignored by the 2.05 client.
    c("seturllevel", "S", V200, NoExec),
    c("setfocus", "RR", V200, NoExec),
    c("resetfocus", "", V200, NoExec),
    c("setbacktile", "R", V200, NoExec),
    c("canwarp", "", V200, NoExec),
    c("canwarp2", "", V200, NoExec),
    c("drawovertrees", "", V200, NoExec),
    c("join", "S", V200, NoExec),
    c("freezeplayer2", "", V200, NoExec),
    c("unfreezeplayer", "", V200, NoExec),
    c("setbacktile2", "RRRRR", V205, NoExec),
    c("setz", "RRRRRRRR", V200, NoExec),
    c("updateterrain", "", V205, NoExec),
    // Seihai stub: not in the 2.05 table. Records the text for the host.
    c("sendtoserver", "S", V21, Ext),
];

/// Look up a command by (lower-case) name, any era.
pub fn command(name: &str) -> Option<&'static Cmd> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// The command if it exists in `era`.
pub fn command_in(name: &str, era: Era) -> Option<&'static Cmd> {
    command(name).filter(|c| c.since <= era)
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
    "greenrupee", "bluerupee", "redrupee", "bombs", "darts", "heart", "glove1", "bow", "bomb", "shield", "sword", "fullheart",
    "superbomb", "battleaxe", "goldensword", "mirrorshield", "glove2", "lizardshield", "lizardsword", "goldrupee", "fireball",
    "fireblast", "nukeshot", "joltbomb", "spinattack",
];
pub const BADDIES: &[&str] = &[
    "graysoldier", "bluesoldier", "redsoldier", "shootingsoldier", "swampsoldier", "frog", "octopus", "goldenwarrior", "lizardon",
    "dragon",
];
pub const COLORS: &[&str] = &[
    "white", "yellow", "orange", "pink", "red", "darkred", "lightgreen", "green", "darkgreen", "lightblue", "blue", "darkblue",
    "brown", "cynober", "purple", "darkpurple", "lightgray", "gray", "black", "transparent",
];
pub const CARRY: &[&str] = &["bush", "sign", "vase", "stone", "blackstone", "bomb", "hotbomb", "superbomb", "joltbomb", "hotjoltbomb", "none"];
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
    BUILTIN_VARS.iter().find(|(n, e, _)| *n == name && *e <= era).map(|(n, _, w)| (*n, *w))
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
    Builtin { name: "abs", args: &[N], since: V200 },
    Builtin { name: "int", args: &[N], since: V200 },
    Builtin { name: "sin", args: &[N], since: V200 },
    Builtin { name: "cos", args: &[N], since: V200 },
    Builtin { name: "arctan", args: &[N], since: V200 },
    Builtin { name: "random", args: &[N, N], since: V200 },
    Builtin { name: "strequals", args: &[T, T], since: V200 },
    Builtin { name: "strlen", args: &[T], since: V202 },
    Builtin { name: "startswith", args: &[T, T], since: V202 },
    Builtin { name: "indexof", args: &[T, T], since: V202 },
    Builtin { name: "ascii", args: &[T], since: V21 },
    // 2.2 command list.
    Builtin { name: "strcontains", args: &[T, T], since: V21 },
    Builtin { name: "lindexof", args: &[T, T], since: V21 },
    Builtin { name: "onmapx", args: &[T], since: V21 },
    Builtin { name: "onmapy", args: &[T], since: V21 },
    Builtin { name: "arraylen", args: &[T], since: V200 },
    // Level collision at a point in tiles; true off the board.
    Builtin { name: "onwall", args: &[N, N], since: V200 },
    // Liquid (water or lava) at a point in tiles; false off the board.
    Builtin { name: "onwater", args: &[N, N], since: V200 },
    // -1 on the default tileset, as in 2.05 (see documentation/tile-types.md).
    Builtin { name: "tiletype", args: &[N, N], since: V200 },
    // Chat tests, true only during `playerchats`. [inferred: whole line / contained text]
    Builtin { name: "playersays", args: &[T], since: V200 },
    Builtin { name: "playersays2", args: &[T], since: V200 },
    // Stubs: return a value so scripts do not log every call.
    Builtin { name: "strtofloat", args: &[T], since: V200 },
    Builtin { name: "hasweapon", args: &[T], since: V200 },
    Builtin { name: "keydown", args: &[N], since: V200 },
    Builtin { name: "keydown2", args: &[N], since: V200 },
    Builtin { name: "sarraylen", args: &[T], since: V200 },
    // `with (getnpc(name))`, `with (getplayer(account))`: the targets of
    // `with` (2.1 and later). Seihai does not switch objects; they read 0.
    Builtin { name: "getnpc", args: &[T], since: V21 },
    Builtin { name: "getplayer", args: &[T], since: V21 },
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
        assert!(Era::V200 < Era::V202 && Era::V205 < Era::V21);
        assert_eq!(Era::for_version(ClientVersion::V202R2), Era::V202);
        assert_eq!(Era::for_version(ClientVersion::V222), Era::V21);
        assert_eq!(Era::for_version(ClientVersion::V133), Era::V200);
    }

    #[test]
    fn command_names_are_unique() {
        for (i, a) in COMMANDS.iter().enumerate() {
            assert!(COMMANDS[i + 1..].iter().all(|b| b.name != a.name), "{}", a.name);
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
        assert_eq!(event_since("action"), None);
        assert!(builtin_var("vecx", Era::V202).is_none());
        assert!(builtin_var("vecx", Era::V203).is_some());
        assert!(!status_flag("playerattached", Era::V203));
        assert!(status_flag("playerattached", Era::V204));
    }

    #[test]
    fn signatures_name_known_letters() {
        for c in COMMANDS {
            assert!(c.sig.chars().all(|l| "RSFQIBCDLON".contains(l)), "{} {}", c.name, c.sig);
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
