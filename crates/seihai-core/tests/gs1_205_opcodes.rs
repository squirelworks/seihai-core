use std::collections::BTreeSet;

use seihai_core::scripting::{name_for_opcode, opcode_for_name, GS1_OPCODE_BASE};

#[test]
fn every_native_205_id_matches_the_pinned_research_fixture() {
    let rows = include_str!("fixtures/gs1-205-opcodes.tsv")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .skip(1)
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 161);
    let mut unique = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        let (op, name) = row.split_once('\t').expect("opcode/name pair");
        let op = op.parse::<u16>().expect("numeric native ID");
        assert_eq!(op, GS1_OPCODE_BASE + index as u16);
        assert!(unique.insert(name), "duplicate native name {name}");
        assert_eq!(name_for_opcode(op), Some(name), "native ID {op}");
        assert_eq!(opcode_for_name(name), Some(op), "native name {name}");
    }
}

#[test]
fn documented_anchors_cover_the_old_wrong_and_missing_slots() {
    for (op, name) in [
        (600, "say"),
        (601, "lay"),
        (602, "take"),
        (605, "if"),
        (617, "putcomp"),
        (648, "message"),
        (674, "setplayerprop"),
        (716, "function"),
        (719, "setimg"),
        (745, "move"),
        (759, "show"),
        (760, "hide"),
    ] {
        assert_eq!(name_for_opcode(op), Some(name));
        assert_eq!(opcode_for_name(name), Some(op));
    }
}

#[test]
fn numeric_domain_and_version_binding_are_explicit() {
    for op in [0, 599, 761, u16::MAX] {
        assert_eq!(name_for_opcode(op), None);
    }
    // These old placeholder entries are not native 2.05 table names.
    // Parser support in some other era does not assign a native 2.05 ID.
    for name in ["warp", "addstring", "with", "addweapon", "unknown", "SHOW"] {
        assert_eq!(opcode_for_name(name), None, "{name}");
    }
}
