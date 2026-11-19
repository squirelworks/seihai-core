//! seihai-core#93: lock the corpus-priority gap inventory and the
//! Statements / omit paths for high-hit names. Aggregate command names
//! only; no script text.

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
fn spawn_commands_without_a_gs2_client_form_stay_missing() {
    // No 3.153 shared-command form, and real GS2 clients refuse `putnpc2`
    // ("Function putnpc2 not found", 4.20r8 and 5.12, lab #104). Emitting a
    // Same call would compile but fail there.
    for name in ["putnpc", "putnpc2", "timershow"] {
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
    }
}

#[test]
fn putexplosion_and_puthorse_call_through_as_real_gs2_clients_run_them() {
    // Real 4.20r8 and 5.12 clients run these from a GS2 weapon script with
    // the GS1 argument order (lab #104, 2026-11-19).
    for (gs1, gs2) in [
        ("putexplosion 1,30,31;", "putexplosion(1, 30, 31);"),
        ("putexplosion2 1,2,30,31;", "putexplosion2(1, 2, 30, 31);"),
        (
            "puthorse ride.png,30,31;",
            "puthorse(\"ride.png\", 30, 31);",
        ),
    ] {
        let name = gs1.split(' ').next().unwrap();
        assert_eq!(
            gs1_command_lowering(name),
            Some(Gs1Lowering::Same),
            "{name}"
        );
        let a = compile::compile_gs1(&format!("if (created) {{ {gs1} }}\n"), Era::V217)
            .unwrap_or_else(|e| panic!("{gs1}: {e}"));
        let b = compile::compile_gs2(&format!("function onCreated() {{\n  {gs2}\n}}\n")).unwrap();
        assert_eq!(a.disassemble(), b.disassemble(), "{gs1}");
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
fn savelog2_lowers_to_savelog_join() {
    assert!(matches!(
        gs1_command_lowering("savelog2"),
        Some(Gs1Lowering::Statements(_))
    ));
    let a = compile::compile_gs1("if (created) { savelog2 a,b; }\n", Era::V217)
        .unwrap_or_else(|e| panic!("{e}"));
    let b = compile::compile_gs2("function onCreated() {\n  savelog(\"a\" @ \"b\");\n}\n")
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

#[test]
fn putbomb_and_shoot_call_through_by_name() {
    // Real compiled GS2 weapon scripts call both on 4.x to 6.x clients.
    for name in ["putbomb", "shoot"] {
        assert_eq!(
            gs1_command_lowering(name),
            Some(Gs1Lowering::Same),
            "{name}"
        );
        assert!(!gs1_commands_without_gs2().contains(&name), "{name}");
    }
    let a = compile::compile_gs1("if (created) { putbomb 1,x+2,y; }\n", Era::V217)
        .unwrap_or_else(|e| panic!("{e}"));
    let b = compile::compile_gs2("function onCreated() {\n  putbomb(1, x + 2, y);\n}\n")
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(a.disassemble(), b.disassemble());
    let a = compile::compile_gs1(
        "if (playerchats) { shoot x,y,0,0,0,2,arrow.gani,; }\n",
        Era::V217,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(a.disassemble().contains("shoot"), "{}", a.disassemble());
}
