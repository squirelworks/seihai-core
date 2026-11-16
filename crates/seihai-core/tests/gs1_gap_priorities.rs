//! seihai-core#93: lock the corpus-priority gap inventory and the inert
//! NoExec omit path. Aggregate command names only; no script text.

use seihai_core::scripting::engine::Era;
use seihai_core::scripting::gs2::compile::{
    self, gs1_command_lowering, gs1_commands_without_gs2, gs1_gap_priorities, Gs1Lowering,
};

#[test]
fn top_gap_priorities_match_issue_93() {
    let gaps = gs1_gap_priorities();
    let expect = [
        ("putexplosion", 424),
        ("puthorse", 249),
        ("setbackpal", 239),
        ("setstring", 188),
        ("putnpc", 158),
        ("setbody", 135),
        ("putbomb", 134),
        ("shoot", 126),
        ("putnpc2", 104),
        ("putexplosion2", 86),
        ("addstring", 80),
    ];
    assert_eq!(gaps.len(), expect.len());
    for (g, (name, hits)) in gaps.iter().zip(expect) {
        assert_eq!(g.name, name);
        assert_eq!(g.failing_units, hits);
    }
}

#[test]
fn high_hit_effect_spawn_commands_stay_missing() {
    // Spawn / combat puts with no 3.153 shared-command form. Emitting a
    // Same call would compile but fail on a real GS2 client ("Function
    // not found"). Keep them Missing until a lab-backed form exists.
    // TODO(lab): putexplosion(2), putbomb, puthorse, putnpc(2), shoot.
    for name in [
        "putexplosion",
        "puthorse",
        "putnpc",
        "putbomb",
        "shoot",
        "putnpc2",
        "putexplosion2",
        "timershow",
    ] {
        assert_eq!(
            gs1_command_lowering(name),
            Some(Gs1Lowering::Missing { inert: false }),
            "{name}"
        );
        assert!(
            gs1_commands_without_gs2().contains(&name),
            "{name} must stay on the without-gs2 inventory"
        );
        let err = compile::compile_gs1(&format!("if (created) {{ {name}; }}\n"), Era::V217)
            .unwrap_err()
            .to_string();
        assert!(err.contains(name), "{name}: {err}");
        if matches!(
            name,
            "putexplosion" | "putbomb" | "puthorse" | "putnpc" | "shoot"
        ) {
            assert!(
                err.contains("3.153") || err.contains("shootarrow"),
                "{name}: {err}"
            );
        }
    }
}

#[test]
fn setbody_lowers_to_player_bodyimg() {
    assert!(matches!(
        gs1_command_lowering("setbody"),
        Some(Gs1Lowering::Statements(_))
    ));
    let a = compile::compile_gs1("if (created) { setbody body13.png; }\n", Era::V217)
        .unwrap_or_else(|e| panic!("{e}"));
    let b = compile::compile_gs2("function onCreated() {\n  player.bodyimg = \"body13.png\";\n}\n")
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(a.disassemble(), b.disassemble());
    assert_eq!(a.write(), b.write());
}


#[test]
fn setbackpal_and_putobject_are_omitted() {
    // Removed from every C++ engine at 3.04; GS2-era clients never had them.
    for name in ["setbackpal", "putobject"] {
        assert!(matches!(
            gs1_command_lowering(name),
            Some(Gs1Lowering::Statements(_))
        ));
    }
    let a = compile::compile_gs1(
        "if (created) { setbackpal tiles.png; putobject bush,1,2; hide; }\n",
        Era::V217,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let b = compile::compile_gs2("function onCreated() {\n  hide();\n}\n")
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(a.disassemble(), b.disassemble());
}

#[test]
fn setstring_and_addstring_plain_names_lower() {
    assert!(matches!(
        gs1_command_lowering("setstring"),
        Some(Gs1Lowering::Statements(_))
    ));
    assert!(matches!(
        gs1_command_lowering("addstring"),
        Some(Gs1Lowering::Statements(_))
    ));
    compile::compile_gs1("if (created) { setstring door,open; }\n", Era::V217).unwrap();
    // Plain-name addstring uses the same client.gs1_ transport as setstring.
    let a = compile::compile_gs1("if (created) { addstring lastweapon,sword; }\n", Era::V217)
        .unwrap_or_else(|e| panic!("{e}"));
    let b = compile::compile_gs2(
        "function onCreated() {\n  client.gs1_lastweapon.add(\"sword\");\n}\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(a.disassemble(), b.disassemble());
}

#[test]
fn inert_noexec_commands_are_omitted() {
    for (src, keep) in [
        (
            "if (created) { drawovertrees; hide; }\n",
            "function onCreated() {\n  hide();\n}\n",
        ),
        (
            "if (created) { setz 1,2,3,4,5,6,7,8; show; }\n",
            "function onCreated() {\n  show();\n}\n",
        ),
        (
            "if (created) { canwarp; canwarp2; freezeplayer2; unfreezeplayer; dontblock; }\n",
            "function onCreated() {\n  dontblock();\n}\n",
        ),
    ] {
        let a = compile::compile_gs1(src, Era::V217).unwrap_or_else(|e| panic!("{src}\n{e}"));
        let b = compile::compile_gs2(keep).unwrap_or_else(|e| panic!("{keep}\n{e}"));
        assert_eq!(
            a.disassemble(),
            b.disassemble(),
            "GS1:\n{src}\nGS2:\n{keep}"
        );
        assert_eq!(a.write(), b.write());
    }
}
