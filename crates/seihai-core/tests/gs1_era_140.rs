use std::collections::BTreeSet;

use seihai_core::protocol::ClientVersion;
use seihai_core::scripting::engine::ast::Stmt;
use seihai_core::scripting::engine::compile::{compile, Node};
use seihai_core::scripting::engine::parse;
use seihai_core::scripting::engine::vocab::{
    command_in, keyword_in, Era, Exec, COMMANDS, KEYWORDS,
};

fn words(era: Era) -> BTreeSet<&'static str> {
    COMMANDS
        .iter()
        .filter(|c| c.exec != Exec::Ext && command_in(c.name, era).is_some())
        .map(|c| c.name)
        .chain(
            KEYWORDS
                .iter()
                .filter(|(name, _, _)| keyword_in(name, era))
                .map(|(name, _, _)| *name),
        )
        .collect()
}

#[test]
fn exact_registration_boundary_is_111_then_125_then_126() {
    let before = words(Era::V139);
    let selected = words(Era::V140);
    let after = words(Era::V141);
    assert_eq!((before.len(), selected.len(), after.len()), (111, 125, 126));
    let expected = BTreeSet::from([
        "break",
        "callnpc",
        "callweapon",
        "canwarp",
        "changeimgvis",
        "continue",
        "drawovertrees",
        "function",
        "putexplosion2",
        "return",
        "say2",
        "setimg",
        "setimgpart",
        "spyfire",
    ]);
    assert_eq!(
        selected
            .difference(&before)
            .copied()
            .collect::<BTreeSet<_>>(),
        expected
    );
    assert!(before.is_subset(&selected));
    assert_eq!(
        after.difference(&selected).copied().collect::<Vec<_>>(),
        ["showstats"]
    );
    assert!(selected.is_subset(&after));
}

#[test]
fn production_parser_and_compiler_select_the_newly_registered_commands() {
    for (name, source) in [
        ("say2", "say2 hello;"),
        ("callnpc", "callnpc 1,entry;"),
        ("callweapon", "callweapon 1,entry;"),
        ("setimg", "setimg head0.gif;"),
        ("setimgpart", "setimgpart head0.gif,0,0,16,16;"),
        ("putexplosion2", "putexplosion2 1,2,3,4;"),
        ("spyfire", "spyfire 1,2;"),
        ("changeimgvis", "changeimgvis 1,2;"),
        ("canwarp", "canwarp;"),
        ("drawovertrees", "drawovertrees;"),
    ] {
        let old = compile(&parse(source, Era::V139));
        // Older prefix matching can select a different command (say2 ->
        // say, for example); do not turn that into a native refusal claim.
        assert!(
            !old.nodes
                .iter()
                .any(|n| matches!(n, Node::Cmd { cmd, .. } if cmd.name == name)),
            "{name}"
        );
        for era in [Era::V140, Era::V141] {
            let parsed = parse(source, era);
            assert!(
                parsed.diagnostics.is_empty(),
                "{name} {era:?}: {:?}",
                parsed.diagnostics
            );
            assert!(
                matches!(&parsed.body[0], Stmt::Command { name: actual, .. } if actual == name)
            );
            let compiled = compile(&parsed);
            assert!(
                compiled
                    .nodes
                    .iter()
                    .any(|n| matches!(n, Node::Cmd { cmd, .. } if cmd.name == name)),
                "{name} {era:?}"
            );
        }
    }
    let source = "showstats 1;";
    let old = compile(&parse(source, Era::V140));
    assert!(!old
        .nodes
        .iter()
        .any(|n| matches!(n, Node::Cmd { cmd, .. } if cmd.name == "showstats")));
    let after = compile(&parse(source, Era::V141));
    assert!(after
        .nodes
        .iter()
        .any(|n| matches!(n, Node::Cmd { cmd, .. } if cmd.name == "showstats")));
}

#[test]
fn function_and_loop_control_reach_the_existing_compiler_at_140() {
    let source = "function mark() { return; } \
                  for (i=0;i<2;i++) { if (i==0) continue; break; } \
                  mark();";
    let before = parse(source, Era::V139);
    assert!(!before.diagnostics.is_empty());
    assert!(!compile(&before).funcs.contains_key("mark"));
    for era in [Era::V140, Era::V141] {
        let parsed = parse(source, era);
        assert!(
            parsed.diagnostics.is_empty(),
            "{era:?}: {:?}",
            parsed.diagnostics
        );
        let compiled = compile(&parsed);
        assert!(compiled.funcs.contains_key("mark"));
        assert!(compiled.loops > 0);
        assert!(compiled.nodes.iter().any(|n| matches!(n, Node::Return(_))));
    }
}

#[test]
fn explicit_era_selection_does_not_create_a_network_profile() {
    assert_eq!(
        Era::ALL.iter().find(|e| e.label() == "1.40"),
        Some(&Era::V140)
    );
    assert_eq!(Era::for_version(ClientVersion::V139), Era::V139);
    assert_eq!(Era::for_version(ClientVersion::V141), Era::V141);
    assert!(ClientVersion::all_versions()
        .iter()
        .all(|v| Era::for_version(*v) != Era::V140));
}
