//! Strict and legacy mode. Strict mode refuses every quirk; legacy mode
//! accepts it with a warning, and compiles it to what the script meant, so
//! a quirky script gives the same bytes as the same script written cleanly.

use seihai_core::scripting::engine::Era;
use seihai_core::scripting::gs2::compile::{
    compile, compile_gs1, compile_gs2, compile_gs2_legacy, Front, Language, Mode, Quirk,
};
use std::path::Path;

/// Each quirk, a script that has only it, and the same script written
/// cleanly (`None` where there is no clean spelling with the same bytes).
fn cases() -> Vec<(Quirk, &'static str, Option<&'static str>)> {
    vec![
        (
            Quirk::MissingSemicolon,
            "function f() { a = 1\n  b = 2; }",
            Some("function f() { a = 1;\n  b = 2; }"),
        ),
        (
            Quirk::CommaStatements,
            "function f() { if (c) a = 1, b = 2; }",
            Some("function f() { if (c) { a = 1; b = 2; } }"),
        ),
        (
            Quirk::EmptyStatement,
            "function f() { a = 1;; };",
            Some("function f() { a = 1; }"),
        ),
        (
            Quirk::UnclosedBlock,
            "function f() { a = 1;",
            Some("function f() { a = 1; }"),
        ),
        (
            Quirk::UnterminatedComment,
            "function f() { a = 1; } /* to the end",
            Some("function f() { a = 1; }"),
        ),
        (
            Quirk::NestedComment,
            "/* a /* b */ c */ function f() { a = 1; }",
            Some("function f() { a = 1; }"),
        ),
        (
            Quirk::AssignInCondition,
            "function f() { if (a = 1) b = 2; }",
            Some("function f() { if (a == 1) b = 2; }"),
        ),
        (
            Quirk::ElseIf,
            "function f() { if (a) b = 1; elseif (c) b = 2; }",
            Some("function f() { if (a) b = 1; else if (c) b = 2; }"),
        ),
        (
            Quirk::OldOperator("<>"),
            "function f() { if (a <> 1) b = 2; }",
            Some("function f() { if (a != 1) b = 2; }"),
        ),
        (
            Quirk::OldOperator("=<"),
            "function f() { if (a =< 1) b = 2; }",
            Some("function f() { if (a <= 1) b = 2; }"),
        ),
        (
            Quirk::OldOperator("=>"),
            "function f() { if (a => 1) b = 2; }",
            Some("function f() { if (a >= 1) b = 2; }"),
        ),
        (
            Quirk::OldOperator(":="),
            "function f() { a := 1; }",
            Some("function f() { a = 1; }"),
        ),
        (
            Quirk::BracelessFunctionBody,
            "function f(a) return a + 1;",
            Some("function f(a) { return a + 1; }"),
        ),
        (
            Quirk::MissingFunctionBody,
            "function f();\nfunction g() { a = 1; }",
            Some("function f() {}\nfunction g() { a = 1; }"),
        ),
        (
            Quirk::DeclarationBeforeBody,
            "function f();\n{ a = 1; }",
            Some("function f() { a = 1; }"),
        ),
        (
            Quirk::StringName,
            "function f(\"a\") { return a; }",
            Some("function f(a) { return a; }"),
        ),
        (
            Quirk::NestedFunction,
            "if (c) { function g() { a = 1; } }",
            None,
        ),
        (
            Quirk::DuplicateFunction("f".into()),
            "function f() { a = 1; }\nfunction f() { a = 2; }",
            Some("function f() { a = 2; }"),
        ),
        (
            Quirk::Gs1Command("hide".into()),
            "function f() { hide; }",
            Some("function f() { hide(); }"),
        ),
        (
            Quirk::Gs1MessageCode,
            "function f() { a = #a; }",
            Some("function f() { a = @player.account; }"),
        ),
        (
            Quirk::Gs1Range,
            "function f() { b = a in <1, 2>; }",
            Some("function f() { b = a in |1, 2|; }"),
        ),
        (
            Quirk::GlobalEnum,
            "global enum { A, B }\nfunction f() { x = B; }",
            Some("enum { A, B }\nfunction f() { x = B; }"),
        ),
        (
            Quirk::EnumText,
            "enum { A = \"x\" }\nfunction f() { y = A; }",
            Some("function f() { y = \"x\"; }"),
        ),
        (Quirk::BareExpression, "function f() { a == 1; }", None),
        (
            Quirk::RuntimeNameAssign,
            "function f() { (@\"a\" @ n) = 1; }",
            Some("function f() { makevar(@\"a\" @ n) = 1; }"),
        ),
        (
            Quirk::CallOfCallResult,
            "function f() { g(\"x\")(); }",
            Some("function f() { (g(\"x\"))(); }"),
        ),
        (
            Quirk::SlashWord("/sit".into()),
            "function f() { a = strequals(b, /sit); }",
            Some("function f() { a = strequals(b, \"/sit\"); }"),
        ),
    ]
}

#[test]
fn strict_mode_refuses_each_quirk_and_legacy_mode_warns() {
    for (quirk, src, _) in cases() {
        if quirk.tolerated() {
            continue;
        }
        let err = compile_gs2(src).unwrap_err().to_string();
        assert!(err.contains("legacy mode"), "{src}: {err}");
        let got = compile_gs2_legacy(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert!(!got.warnings.is_empty(), "{src}");
        for w in got.warnings {
            assert_eq!(w.quirk, quirk, "{src}");
        }
    }
}

#[test]
fn a_quirk_compiles_to_what_the_script_meant() {
    for (_, src, clean) in cases() {
        let Some(clean) = clean else { continue };
        let want = compile_gs2(clean).unwrap_or_else(|e| panic!("{clean}: {e}"));
        let got = compile_gs2_legacy(src).unwrap().script;
        assert_eq!(got.write(), want.write(), "{src}");
    }
}

#[test]
fn an_unterminated_string_runs_to_the_end_of_the_file() {
    let got = compile_gs2_legacy("function f() { a = \"open").unwrap();
    let quirks: Vec<Quirk> = got.warnings.into_iter().map(|w| w.quirk).collect();
    assert!(quirks.contains(&Quirk::UnterminatedString), "{quirks:?}");
    assert!(compile_gs2("function f() { a = \"open").is_err());
}

#[test]
fn warnings_carry_their_lines() {
    let got =
        compile_gs2_legacy("function f() {\n  a = 1\n  b = 2;\n  if (a = 1) c = 3;\n}").unwrap();
    let lines: Vec<String> = got.warnings.iter().map(|w| w.to_string()).collect();
    assert_eq!(
        lines,
        [
            "line 3: missing `;`",
            "line 4: `=` used as a comparison; write `==`"
        ]
    );
}

#[test]
fn the_reference_fixtures_are_strict_gs2() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gs2");
    for name in [
        "shared", "control", "calls", "timers", "strings", "lists", "rest", "objects",
    ] {
        let src = std::fs::read_to_string(dir.join(format!("{name}.gs2"))).unwrap();
        let strict = compile_gs2(&src).unwrap_or_else(|e| panic!("{name}: {e}"));
        let legacy = compile_gs2_legacy(&src).unwrap();
        assert!(legacy.warnings.is_empty(), "{name}: {:?}", legacy.warnings);
        assert_eq!(strict.write(), legacy.script.write(), "{name}");
    }
}

#[test]
fn only_legacy_mode_guesses_the_language() {
    let gs1 = "if (playerenters) { message hi; }";
    assert!(compile(gs1, Language::Detect(Era::V21), Mode::Strict).is_err());
    let got = compile(gs1, Language::Detect(Era::V21), Mode::Legacy).unwrap();
    assert_eq!(got.front, Front::Gs1);
    assert_eq!(got.warnings[0].quirk, Quirk::GuessedLanguage(Front::Gs1));
    // Named as GS1, the same script needs no guess and has no warning.
    let named = compile(gs1, Language::Gs1(Era::V21), Mode::Strict).unwrap();
    assert!(named.warnings.is_empty());
    assert_eq!(named.script.write(), got.script.write());
    assert_eq!(
        compile_gs1(gs1, Era::V21).unwrap().write(),
        got.script.write()
    );
}

#[test]
fn a_new_block_must_stand_alone() {
    // `new T(name) { … }` makes the object, runs the body with it and adds
    // it as a control; used as a value it has no encoding.
    for src in [
        "function f() { x = new T(\"a\") { y = 1; }; }",
        "function f() { g(new T(\"a\") { y = 1; }); }",
    ] {
        assert!(compile_gs2_legacy(src).is_err(), "{src}");
    }
    compile_gs2("function f() { new T(\"a\") { y = 1; } }").unwrap();
}

#[test]
fn legacy_mode_reads_later_gs1_as_gs1_2_1() {
    // GS1 2.05 scripts that use features 2.05 gates: `with`, `in` with a
    // variable, `in <a, b>`, `{…}` lists, and a later command.
    for src in [
        "if (created) {\n  with (getplayer(#a)) {\n    hide;\n  }\n}",
        "if (actiongrab) {\n  if (strequals(#g,w1 w2) || player.a in b) {\n    c = 1;\n  }\n}",
        "function a() {\n  if (playerx in <x+1,x+3> && playery > y) {\n    d = 1;\n  }\n}",
        "if (created) {\n  this.a = {0,1,2,3};\n}",
        "if (created) {\n  seteffectmode 2;\n}",
    ] {
        assert!(compile_gs1(src, Era::V205).is_err(), "{src}");
        let got = compile(src, Language::Gs1(Era::V205), Mode::Legacy)
            .unwrap_or_else(|e| panic!("{src}: {e}"));
        let quirks: Vec<Quirk> = got.warnings.into_iter().map(|w| w.quirk).collect();
        assert_eq!(quirks, [Quirk::LaterGs1(Era::V205)], "{src}");
        assert_eq!(
            got.script.write(),
            compile_gs1(src, Era::V21).unwrap().write(),
            "{src}"
        );
    }
}

#[test]
fn a_gs1_with_keeps_plain_variables_on_the_npc() {
    // Inside GS1 `with`, plain variables stay the NPC's own; GS2 reaches
    // them through `thiso`.
    let plain = "if (created) {\n  with (getplayer(#a)) {\n    n = 1;\n    m = n + x;\n    if (n in b) hide;\n  }\n}";
    let thiso = "if (created) {\n  with (getplayer(#a)) {\n    thiso.n = 1;\n    thiso.m = thiso.n + x;\n    if (n in thiso.b) hide;\n  }\n}";
    let got = compile_gs1(plain, Era::V21).unwrap_or_else(|e| panic!("{e}"));
    let want = compile_gs1(thiso, Era::V21).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(got.write(), want.write());
    let nested = "if (created) { with (a) { with (b) { n = 1; } } }";
    assert!(compile_gs1(nested, Era::V21).is_err());
}

#[test]
fn detect_keeps_gs1_that_needs_a_later_version_as_gs1() {
    // Looks like GS1 and fails as GS1 2.05; read as GS2, `created` would be
    // a variable, so it is read as GS1 2.1.
    let src = "if (created) {\n  this.a = {0,1,2,3};\n}";
    let got = compile(src, Language::Detect(Era::V205), Mode::Legacy).unwrap();
    assert_eq!(got.front, Front::Gs1);
    let quirks: Vec<Quirk> = got.warnings.into_iter().map(|w| w.quirk).collect();
    assert_eq!(
        quirks,
        [
            Quirk::GuessedLanguage(Front::Gs1),
            Quirk::LaterGs1(Era::V205)
        ]
    );
}

#[test]
fn a_gs1_method_call_is_a_member_call() {
    // GS2-style calls in a GS1 script call a method on the object, not a
    // function whose name has a dot in it.
    let gs1 = "function onCreated() {\n  this.join(\"c\");\n  this.setshape(1, 32, 32);\n  x = player.level.name.length();\n}";
    // GS1 folds names to lower case.
    let gs2 = "function oncreated() {\n  this.join(\"c\");\n  this.setshape(1, 32, 32);\n  x = player.level.name.length();\n}";
    assert_eq!(
        compile_gs1(gs1, Era::V205).unwrap().write(),
        compile_gs2(gs2).unwrap().write()
    );
}

#[test]
fn strict_mode_takes_what_the_engine_tolerates_and_warns() {
    // The reference compilers accept these with the clean spelling's bytes.
    let mut seen = 0;
    for (quirk, src, clean) in cases() {
        if !quirk.tolerated() {
            continue;
        }
        seen += 1;
        let got =
            compile(src, Language::Gs2, Mode::Strict).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert!(!got.warnings.is_empty(), "{src}");
        assert!(got.warnings.iter().all(|w| w.quirk == quirk), "{src}");
        if let Some(clean) = clean {
            assert_eq!(
                got.script.write(),
                compile_gs2(clean).unwrap().write(),
                "{src}"
            );
        }
    }
    assert_eq!(seen, 10);
    // Mixed with one that is not tolerated, only that one is an error.
    let err = compile_gs2("function f() { if (a <> 1) b = 2;; c = 1\n d = 2; }").unwrap_err();
    assert_eq!(err.errors.len(), 1, "{err}");
    assert!(err.to_string().contains("missing `;`"), "{err}");
}

#[test]
fn detect_reports_both_fronts_when_both_fail() {
    let err = compile(
        "if (created) { x = ; }",
        Language::Detect(Era::V205),
        Mode::Legacy,
    )
    .unwrap_err()
    .to_string();
    let gs1 = err.find("as GS1: ").unwrap_or_else(|| panic!("{err}"));
    let gs2 = err.find("as GS2: ").unwrap_or_else(|| panic!("{err}"));
    assert!(gs1 < gs2, "{err}");
}
