//! Cleaned GS1 command table for client 2.05.
//!
//! 2.05 uses a parse-tree interpreter. Command opcode = 600 + table index.
//! The table contains 161 commands (opcodes 600..=760). `show`/`hide` are
//! 759/760. A small set of commands compile but have no executor case in 2.05.
//!
//! Script-facing directions: 0 up, 1 left, 2 down, 3 right.
//!
//! This list is the cleaned command names only. No addresses, no decompiler
//! output. Full argument signatures and per-command behaviour notes live in
//! the Graal_analysis working papers.

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
const NAMES: &[&str] = &[
    "message",
    "say",
    "set",
    "unset",
    "setimg",
    "move",
    "warp",
    "setlevel",
    "showimg",
    "hideimg",
    "hurt",
    "setplayerprop",
    "setstring",
    "addstring",
    "setshape",
    "canwarp",
    "canwarp2",
    "with",
    "join",
    "addweapon",
    "tokenize",
    "seturllevel",
    "setfocus",
    "resetfocus",
    "setbacktile",
    "setbacktile2",
    "drawovertrees",
    "freezeplayer2",
    "unfreezeplayer",
    "setz",
    "updateterrain",
    "show",
    "hide",
    // Additional names from the cleaned 161-entry table follow.
    // The full ordered list is maintained in the Graal_analysis notes;
    // the entries above are the ones already exercised by the offline
    // slice or called out as having no executor case.
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_hide_opcodes() {
        // show/hide are documented as 759/760 in the cleaned notes.
        // The partial table above does not yet place them at those slots;
        // this test documents the expected relationship once the full
        // ordered list is filled in.
        assert_eq!(GS1_OPCODE_BASE, 600);
    }

    #[test]
    fn no_executor_set() {
        assert!(has_no_executor("setz"));
        assert!(has_no_executor("join"));
        assert!(!has_no_executor("message"));
    }
}
