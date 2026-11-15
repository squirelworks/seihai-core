//! Native GS1 command-ID names for the documented client 2.05 rev 2.
//!
//! This client parses source into a tree; these IDs are 600 + registration
//! index, not serialized GS1 bytecode or Seihai's own instruction numbers.
//! The complete table has 161 entries (600..=760), ending in show/hide.
//! Parser membership by era and execution live separately in `engine`.
//! Do not apply this native ordering to other client builds.
//!
//! ID/name facts: Graal_analysis by squirelworks, CC BY 4.0:
//! https://github.com/squirelworks/Graal_analysis/blob/07f55908c1eb888d7f44c41fcf18213b2d955113/npc-library/gs1-reference-205.md
//! https://creativecommons.org/licenses/by/4.0/
//! Cleaned ID/name pairs only; source argument columns and prose omitted.
//! No original addresses, binary data or decompiler implementation.
//!
//! Script-facing directions: 0 up, 1 left, 2 down, 3 right.

/// Opcode base for GS1 commands in 2.05.
pub const GS1_OPCODE_BASE: u16 = 600;

/// Commands that compile but have no executor case in 2.05.
pub const GS1_NO_EXECUTOR: &[&str] = &[
    "seturllevel",
    "setfocus",
    "resetfocus",
    "setbacktile",
    "setbacktile2",
    "canwarp",
    "canwarp2",
    "drawovertrees",
    "join",
    "freezeplayer2",
    "unfreezeplayer",
    "setz",
    "updateterrain",
];

/// Script-facing direction constants (2.05).
pub const DIR_UP: u8 = 0;
pub const DIR_LEFT: u8 = 1;
pub const DIR_DOWN: u8 = 2;
pub const DIR_RIGHT: u8 = 3;

/// Look up a command name by its 2.05 opcode (600..=760).
pub fn name_for_opcode(op: u16) -> Option<&'static str> {
    let idx = op.checked_sub(GS1_OPCODE_BASE)? as usize;
    NAMES.get(idx).copied()
}

/// Look up the 2.05 opcode for a command name.
pub fn opcode_for_name(name: &str) -> Option<u16> {
    NAMES
        .iter()
        .position(|&n| n == name)
        .map(|i| GS1_OPCODE_BASE + i as u16)
}

/// True if the command compiles but has no executor case in 2.05.
pub fn has_no_executor(name: &str) -> bool {
    GS1_NO_EXECUTOR.contains(&name)
}

/// Cleaned 2.05 command names in opcode order (600..).
/// Count: 161. `show` = 759, `hide` = 760.
const NAMES: [&str; 161] = [
    "say",
    "lay",
    "take",
    "hurt",
    "set",
    "if",
    "play",
    "unset",
    "setgif",
    "removecompus",
    "setlevel",
    "setskincolor",
    "setcoatcolor",
    "setsleevecolor",
    "setshoecolor",
    "setbeltcolor",
    "setplayerdir",
    "putcomp",
    "takeplayercarry",
    "dontblocklocal",
    "drawoverplayer",
    "enableweapons",
    "disableweapons",
    "shootarrow",
    "shootfireball",
    "shootfireblast",
    "shootnuke",
    "putbomb",
    "putexplosion",
    "putleaps",
    "shootball",
    "timershow",
    "toinventory",
    "followplayer",
    "else",
    "openurl",
    "seturllevel",
    "stopmidi",
    "blockagainlocal",
    "drawunderplayer",
    "puthorse",
    "canbecarried",
    "cannotbecarried",
    "noplayerkilling",
    "canbepushed",
    "cannotbepushed",
    "canbepulled",
    "cannotbepulled",
    "message",
    "setsword",
    "setshield",
    "putnewcomp",
    "putnpc",
    "destroy",
    "freezeplayer",
    "sethead",
    "toweapons",
    "showimg",
    "hideimg",
    "hidesword",
    "for",
    "hitcompu",
    "hitplayer",
    "setbow",
    "updateboard",
    "putobject",
    "setarray",
    "sleep",
    "while",
    "hideplayer",
    "timereverywhere",
    "setminimap",
    "setmap",
    "showcharacter",
    "setplayerprop",
    "setcharprop",
    "showfile",
    "setstring",
    "setbackpal",
    "setletters",
    "setgifpart",
    "setfocus",
    "resetfocus",
    "setbody",
    "carryobject",
    "throwcarry",
    "showlocal",
    "hidelocal",
    "dontblock",
    "blockagain",
    "setbacktile",
    "takeplayerhorse",
    "removebomb",
    "removearrow",
    "removeitem",
    "removeexplo",
    "removehorse",
    "explodebomb",
    "reflectarrow",
    "lay2",
    "take2",
    "takehorse",
    "playlooped",
    "stopsound",
    "changeimgpart",
    "setlevel2",
    "hitnpc",
    "canwarp2",
    "setani",
    "setcharani",
    "callnpc",
    "say2",
    "break",
    "continue",
    "callweapon",
    "spyfire",
    "function",
    "return",
    "changeimgvis",
    "setimg",
    "setimgpart",
    "drawovertrees",
    "putexplosion2",
    "showstats",
    "canwarp",
    "setshape",
    "seteffect",
    "setcoloreffect",
    "setzoomeffect",
    "drawaslight",
    "changeimgcolors",
    "changeimgzoom",
    "setgender",
    "setchargender",
    "join",
    "freezeplayer2",
    "unfreezeplayer",
    "enabledefmovement",
    "disabledefmovement",
    "setmusicvolume",
    "openurl2",
    "hitobjects",
    "tokenize",
    "tokenize2",
    "triggeraction",
    "move",
    "addtiledef",
    "addtiledef2",
    "removetiledefs",
    "replaceani",
    "attachplayertoobj",
    "setshape2",
    "detachplayer",
    "enableselectweapons",
    "disableselectweapons",
    "setbacktile2",
    "setz",
    "updateterrain",
    "loadmap",
    "show",
    "hide",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_native_slots() {
        assert_eq!(name_for_opcode(600), Some("say"));
        assert_eq!(opcode_for_name("message"), Some(648));
        assert_eq!(opcode_for_name("show"), Some(759));
        assert_eq!(name_for_opcode(760), Some("hide"));
    }

    #[test]
    fn no_executor_set() {
        assert!(has_no_executor("setz"));
        assert!(has_no_executor("join"));
        assert!(!has_no_executor("message"));
    }
}
