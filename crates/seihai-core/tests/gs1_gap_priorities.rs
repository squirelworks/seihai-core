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
fn high_hit_effect_commands_stay_missing_until_a_gs2_form_exists() {
    // These names are not in the 3.153 shared-command table. Emitting a
    // Same call would compile but fail on a real GS2 client ("Function
    // not found"). Keep them Missing until a lab-backed form exists.
    for name in [
        "putexplosion",
        "puthorse",
        "setbackpal",
        "putnpc",
        "setbody",
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
        // Hints name a concrete reason for the top combat/spawn gaps.
        if matches!(
            name,
            "putexplosion" | "putbomb" | "puthorse" | "putnpc" | "setbody" | "shoot"
        ) {
            assert!(
                err.contains("3.153") || err.contains("sethead") || err.contains("shootarrow"),
                "{name}: {err}"
            );
        }
    }
}

#[test]
fn setstring_and_addstring_are_partial_statement_gaps() {
    assert!(matches!(
        gs1_command_lowering("setstring"),
        Some(Gs1Lowering::Statements(_))
    ));
    assert!(matches!(
        gs1_command_lowering("addstring"),
        Some(Gs1Lowering::Statements(_))
    ));
    // Plain-name setstring still lowers (player flag transport).
    compile::compile_gs1("if (created) { setstring door,open; }\n", Era::V217).unwrap();
    // Plain-name addstring still refuses (only this./client. lists).
    let err = compile::compile_gs1("if (created) { addstring a,b; }\n", Era::V217)
        .unwrap_err()
        .to_string();
    assert!(err.contains("addstring"), "{err}");
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
        assert_eq!(a.disassemble(), b.disassemble(), "GS1:\n{src}\nGS2:\n{keep}");
        assert_eq!(a.write(), b.write());
    }
}
