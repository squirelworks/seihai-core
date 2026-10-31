//! The built-in tutorial: levels whose names start with
//! [`PREFIX`] are drawn with their own tile sheet (`tutorial/pics1.png` of
//! the built-in set) and played with their own tile and object tables
//! (`tutorial/arrays.dat`), whatever asset folder is in use. The client
//! swaps them in on entering such a level and puts the player's own back on
//! leaving it, the way a Graal client picks a sheet per level with
//! tiledefs. See `documentation/tutorial-levels.md`.

use super::builtin;
use super::{ObjectTables, TileTypes};

/// Level names that use the tutorial sheet start with this (any case).
pub const PREFIX: &str = "seihai_tutorial";

/// The first level of the tutorial.
pub const START: &str = "seihai_tutorial1.nw";

/// Where the tutorial starts the player in [`START`] (the sprite's top-left).
pub const START_POS: (f32, f32) = (30.0, 30.0);

/// The built-in file of the tutorial's tile sheet.
pub const SHEET_FILE: &str = "tutorial/pics1.png";
/// The built-in file of the tutorial's tile and object tables.
pub const ARRAYS_FILE: &str = "tutorial/arrays.dat";

/// True for a level of the tutorial (`seihai_tutorial1.nw`,
/// `levels/Seihai_Tutorial_house.nw`, ...).
pub fn is_tutorial_level(name: &str) -> bool {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    base.len() >= PREFIX.len() && base[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
}

/// The tutorial sheet's PNG bytes, from the binary.
pub fn sheet_png() -> Option<&'static [u8]> {
    builtin::bytes(SHEET_FILE)
}

/// The tutorial's tile table and object tables, from the binary.
pub fn tables() -> Option<(TileTypes, Option<ObjectTables>)> {
    let data = builtin::bytes(ARRAYS_FILE)?;
    let types = TileTypes::parse_arrays(data).ok()?;
    Some((types, ObjectTables::parse(data)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_by_prefix_any_case_any_folder() {
        assert!(is_tutorial_level("seihai_tutorial1.nw"));
        assert!(is_tutorial_level("SEIHAI_TUTORIAL_house.nw"));
        assert!(is_tutorial_level("levels/seihai_tutorial4.nw"));
        assert!(is_tutorial_level("C:\\x\\seihai_tutorial2.nw"));
        assert!(!is_tutorial_level("new1.nw"));
        assert!(!is_tutorial_level("seihai_tut.nw"));
        assert!(!is_tutorial_level("my_seihai_tutorial1.nw"));
    }

    #[test]
    fn the_built_in_tables_have_cut_lift_ledges_and_blocks() {
        let (types, obj) = tables().expect("tutorial/arrays.dat is built in");
        let obj = obj.expect("object tables");
        assert_eq!(obj.cut.len(), 2, "bush and tall grass");
        assert_eq!(obj.lift.len(), 5, "bush, sign, pot, stone, black stone");
        assert_eq!(obj.lift_into.len(), 5);
        assert_eq!(obj.blocks.len(), 2, "push and pull block");
        assert!(!obj.ledges[2].is_empty(), "down ledges");
        // bush 2 is a thrown-over wall, grass 0 open, deep water 322 water
        assert_eq!(types.types[0], 0);
        assert_eq!(types.types[322], crate::assets::TYPE_WATER);
        assert!(types.types[2] >= 20);
        assert!(sheet_png().is_some());
    }
}
