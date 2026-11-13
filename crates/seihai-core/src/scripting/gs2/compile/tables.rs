//! Name tables: the GS2 client commands, the opcode built-ins, the GS1
//! event names and the root objects.

use super::super::Op;
use crate::scripting::engine::vocab;

/// GS1 command names that have no GS2 command and no GS2 opcode.
///
/// A name is kept when it is in the 3.153 command tables
/// (`documentation/gs2-shared-commands.md`), when `gs2-executor.md` names it
/// as a call the host still accepts (`set`, `unset`, `setlevel`,
/// `setlevel2`, `setplayerprop`), when `gs2-grammar.md` says the compiler
/// keeps it as a call by name (`join("class")`, so GS1 `join class;` lowers
/// to that call with the class name as a string), or when `gs2-opcodes.md`
/// gives it an opcode (`sleep`, `setarray`). The string-list commands are
/// the case the notes call out: GS2 has list methods instead, and a call by
/// the GS1 name is not a GS2 command.
pub fn gs1_commands_without_gs2() -> Vec<&'static str> {
    vocab::COMMANDS
        .iter()
        .filter(|c| !gs2_has_command(c.name))
        .map(|c| c.name)
        .collect()
}

/// How a GS1 command reaches GS2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gs1Lowering {
    /// A GS2 command or opcode of the same name, called with the same
    /// arguments.
    Same,
    /// The GS2 command of another name that does the same thing. The GS1
    /// engine runs the two names as one command (`setgif` is `setimg`).
    Renamed(&'static str),
    /// No GS2 command of its own: it becomes one or more GS2 statements.
    /// The text says which.
    Statements(&'static str),
    /// No GS2 counterpart. Compiling it is an error. `inert` is true when
    /// the 2.05 client compiles the command and does nothing with it, so
    /// leaving it out would be a possible lowering once a later client is
    /// checked.
    Missing { inert: bool },
}

/// How the GS1 command `name` is lowered, or `None` for a name that is not
/// a GS1 command.
pub fn gs1_command_lowering(name: &str) -> Option<Gs1Lowering> {
    let cmd = vocab::command(name)?;
    Some(if let Some(to) = renamed_command(name) {
        Gs1Lowering::Renamed(to)
    } else if let Some(how) = statement_lowering(name) {
        Gs1Lowering::Statements(how)
    } else if gs2_has_command(name) {
        Gs1Lowering::Same
    } else {
        Gs1Lowering::Missing {
            inert: matches!(cmd.exec, vocab::Exec::NoExec),
        }
    })
}

/// GS1 command names that are another GS2 command's old name. The GS1
/// engine runs each pair as one command.
pub(super) fn renamed_command(name: &str) -> Option<&'static str> {
    match name {
        "setgif" => Some("setimg"),
        "setgifpart" => Some("setimgpart"),
        _ => None,
    }
}

/// The GS1 commands the front end turns into GS2 statements (see
/// `gs1.rs`, `string_command`).
fn statement_lowering(name: &str) -> Option<&'static str> {
    Some(match name {
        "setstring" => "`this.`/`client.` names: an assignment; any other name is a player flag: `set(\"name=value\")` or `unset(\"name\")`",
        "addstring" | "insertstring" | "replacestring" | "removestring" | "deletestring" => {
            "the GS2 list method on a `this.`/`client.` list (`add`, `insert`, `replace`, `remove`, `delete`); before 2.1 the builds have no `addstring`"
        }
        "tokenize" | "tokenize2" => "`tokens = text.tokenize(seps)`",
        "setcharprop" => "an assignment to the NPC's own property (`this.nick`, `this.colors[i]`, ...)",
        _ => return None,
    })
}

pub(super) fn gs2_has_command(name: &str) -> bool {
    GS2_CLIENT_COMMANDS.binary_search(&name).is_ok()
        || matches!(
            name,
            "set" | "unset" | "setlevel" | "setlevel2" | "setplayerprop" | "join"
        )
        || opcode_builtin(name).is_some()
}

/// Names registered by the 3.153 client, lower case, sorted.
/// From `documentation/gs2-shared-commands.md`.
const GS2_CLIENT_COMMANDS: &[&str] = &[
    "addcontrol",
    "addtiledef",
    "addtiledef2",
    "aindexof",
    "attachplayertoobj",
    "blockagain",
    "blockagainlocal",
    "callnpc",
    "callweapon",
    "canbecarried",
    "canbepulled",
    "canbepushed",
    "cannotbecarried",
    "cannotbepulled",
    "cannotbepushed",
    "carryobject",
    "changeimgcolors",
    "changeimgmode",
    "changeimgpart",
    "changeimgvis",
    "changeimgzoom",
    "commandtoserver",
    "contains",
    "cursoroff",
    "cursoron",
    "destroy",
    "detachplayer",
    "disabledefmovement",
    "disablemap",
    "disablepause",
    "disableselectweapons",
    "disableweapons",
    "dontblock",
    "dontblocklocal",
    "drawaslight",
    "drawoverplayer",
    "drawunderplayer",
    "echo",
    "enabledefmovement",
    "enablefeatures",
    "enablemap",
    "enablepause",
    "enableselectweapons",
    "enableweapons",
    "explodebomb",
    "extractfilebase",
    "extractfileext",
    "extractfilename",
    "extractfilepath",
    "findfirstfile",
    "findimg",
    "findnextfile",
    "findplayer",
    "findplayerbyid",
    "findweapon",
    "freezeplayer",
    "getascii",
    "getboxcenter",
    "getdatablocktypes",
    "getextension",
    "getimgheight",
    "getimgwidth",
    "getkeycode",
    "getmapx",
    "getmapy",
    "getobjectat",
    "getobjectatmouse",
    "getobjectbyray",
    "getstringkeys",
    "getsurfacetype",
    "gettextheight",
    "gettextwidth",
    "getz",
    "hide",
    "hideimg",
    "hideimgs",
    "hidelocal",
    "hideplayer",
    "hidesword",
    "hitnpc",
    "hitobjects",
    "hitplayer",
    "hurt",
    "isadminguild",
    "iscursoron",
    "isobject",
    "keydown",
    "keydown2",
    "keyname",
    "lay",
    "lay2",
    "lightscene",
    "loadmap",
    "mapshapeanimation",
    "matrixcreate",
    "matrixcreatefromeuler",
    "matrixmulpoint",
    "matrixmultiply",
    "matrixmulvector",
    "message",
    "move",
    "noplayerkilling",
    "onwall",
    "onwall2",
    "onwater",
    "onwater2",
    "openurl",
    "openurl2",
    "play",
    "play2",
    "playlooped",
    "popdialog",
    "pushdialog",
    "putleaps",
    "reloadfile",
    "removetiledefs",
    "replaceani",
    "requestfiledeletion",
    "requestfilerename",
    "requestfilesmove",
    "requesttext",
    "resetfocus",
    "savelog",
    "say",
    "say2",
    "screenx",
    "screeny",
    "selectfilefordownload",
    "selectfileforupload",
    "sendrpgmessage",
    "sendtext",
    "sendtorc",
    "serverwarp",
    "setani",
    "setbeltcolor",
    "setbow",
    "setcharani",
    "setchargender",
    "setcoatcolor",
    "setcoloreffect",
    "setcontentcontrol",
    "setcursor2",
    "seteffect",
    "seteffectmode",
    "setfocus",
    "setfogcolors",
    "setgender",
    "sethead",
    "setimg",
    "setimgpart",
    "setinteriorrendermode",
    "setletters",
    "setmap",
    "setminimap",
    "setmusicvolume",
    "setplayerdir",
    "setshadowdetaillevel",
    "setshape",
    "setshape2",
    "setshield",
    "setshoecolor",
    "setshootparams",
    "setskincolor",
    "setskybandcolors",
    "setskybandsizes",
    "setsleevecolor",
    "setsuncolors",
    "setsword",
    "setterrainrendermode",
    "setzoomeffect",
    "shootarrow",
    "shootball",
    "shootfireball",
    "shootfireblast",
    "shootnuke",
    "show",
    "showani",
    "showani2",
    "showcharacter",
    "showimg",
    "showimg2",
    "showlocal",
    "showpoly",
    "showpoly2",
    "showstats",
    "showtext",
    "showtext2",
    "spyfire",
    "stopmidi",
    "stopsound",
    "strequals",
    "synctimeofday",
    "take",
    "take2",
    "takehorse",
    "takeplayercarry",
    "takeplayerhorse",
    "testplayer",
    "throwcarry",
    "tiletype",
    "timereverywhere",
    "toweapons",
    "triggeraction",
    "triggerserver",
    "updateboard",
    "updateterrain",
    "vectoradd",
    "vectorcross",
    "vectordist",
    "vectordot",
    "vectorlen",
    "vectornormalize",
    "vectororthobasis",
    "vectorscale",
    "vectorsub",
    "worldx",
    "worldy",
    "wraptext",
    "wraptext2",
];

pub(super) fn command_hint(name: &str) -> &'static str {
    match name {
        "setstring" | "addstring" | "insertstring" | "replacestring" | "removestring"
        | "deletestring" => " (GS2 uses list methods such as add and delete)",
        "tokenize" | "tokenize2" => " (GS2 uses the tokenize method)",
        _ => "",
    }
}

/// GS1 functions the opcode notes do not map and the 3.153 tables do not
/// register. `strlen` / `startswith` / `indexof` are lowered to the methods
/// named beside them in `gs2-opcodes.md`.
pub(super) fn gs1_fn_without_gs2(name: &str) -> bool {
    matches!(
        name,
        "playersays"
            | "playersays2"
            | "ascii"
            | "strcontains"
            | "lindexof"
            | "onmapx"
            | "onmapy"
            | "strtofloat"
            | "hasweapon"
            | "getnpc"
            | "getplayer"
    )
}

/// The opcode a built-in function compiles to.
pub(super) fn opcode_builtin(name: &str) -> Option<Op> {
    Some(match name {
        "sleep" => Op::Sleep,
        "sin" => Op::Sin,
        "cos" => Op::Cos,
        "arctan" => Op::ArcTan,
        "abs" => Op::Abs,
        "random" => Op::Random,
        "arraylen" | "sarraylen" => Op::ObjSize,
        "setarray" => Op::SetArray,
        "int" => Op::Int,
        "exp" => Op::Exp,
        "log" => Op::Log,
        "min" => Op::Min,
        "max" => Op::Max,
        "pow" => Op::Pow,
        "char" => Op::Char,
        "getangle" => Op::GetAngle,
        "getdir" => Op::GetDir,
        "vecx" => Op::VecX,
        "vecy" => Op::VecY,
        "waitfor" => Op::WaitFor,
        "format" => Op::Format,
        "makevar" => Op::MakeVar,
        _ => return None,
    })
}

/// The argument count a built-in takes, where Seihai checks it. `format`
/// takes a pattern and any number of values.
pub(super) fn builtin_arity(name: &str) -> Option<usize> {
    Some(match name {
        "sleep" | "sin" | "cos" | "arctan" | "abs" | "arraylen" | "sarraylen" | "int" | "exp"
        | "char" | "vecx" | "vecy" | "makevar" => 1,
        "random" | "setarray" | "log" | "min" | "max" | "pow" | "getangle" | "getdir" => 2,
        "waitfor" => 3,
        _ => return None,
    })
}

/// Whether a call of `name` used as a statement leaves a value to drop.
pub(super) fn call_pushes(name: &str) -> bool {
    !matches!(name, "sleep" | "setarray")
}

/// The opcode for a root object name (`this`, `player`, …).
pub(super) fn root_op(name: &str) -> Option<Op> {
    Some(match name {
        "this" => Op::This,
        "thiso" => Op::ThisO,
        "player" => Op::Player,
        "playero" => Op::PlayerO,
        "level" => Op::Level,
        "temp" => Op::Temp,
        _ => return None,
    })
}

pub(super) fn is_plain_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

/// GS2 function name for a GS1 event. The spellings `onCreated`,
/// `onTimeout`, `onPlayerEnters` and `onPlayerChats` are the ones in
/// `documentation/gs2-grammar.md` and `documentation/seihaiscript.md`. The
/// other events in `vocab::EVENTS` are split into those same words.
pub fn gs2_event_function(event: &str) -> String {
    let mut rest = event;
    let mut parts = String::from("on");
    while !rest.is_empty() {
        let word = EVENT_WORDS
            .iter()
            .copied()
            .filter(|w| rest.starts_with(w))
            .max_by_key(|w| w.len());
        let word = word.unwrap_or(rest);
        let mut chars = word.chars();
        if let Some(c) = chars.next() {
            parts.push(c.to_ascii_uppercase());
            parts.push_str(chars.as_str());
        }
        if word.len() == rest.len() && EVENT_WORDS.iter().all(|w| !rest.starts_with(w)) {
            break;
        }
        rest = &rest[word.len()..];
    }
    parts
}

const EVENT_WORDS: &[&str] = &[
    "movement", "selected", "finished", "shooted", "reading", "exploded", "timeout", "created",
    "weapon", "player", "touchs", "enters", "leaves", "thrown", "pushed", "pulled", "compus",
    "action", "chats", "other", "baddy", "fired", "comps", "shot", "pelt", "hurt", "dies", "dead",
    "lays", "item", "ends", "was", "me", "by",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_follow_the_notes() {
        assert_eq!(gs2_event_function("created"), "onCreated");
        assert_eq!(gs2_event_function("timeout"), "onTimeout");
        assert_eq!(gs2_event_function("playerenters"), "onPlayerEnters");
        assert_eq!(gs2_event_function("playerchats"), "onPlayerChats");
        assert_eq!(gs2_event_function("playertouchsme"), "onPlayerTouchsMe");
        assert_eq!(gs2_event_function("weaponfired"), "onWeaponFired");
        assert_eq!(gs2_event_function("playerleaves"), "onPlayerLeaves");
        assert_eq!(gs2_event_function("playerlogin"), "onPlayerLogin");
        assert_eq!(gs2_event_function("playerlogout"), "onPlayerLogout");
    }

    #[test]
    fn client_command_list_is_sorted() {
        let mut sorted = GS2_CLIENT_COMMANDS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, GS2_CLIENT_COMMANDS);
    }
}
