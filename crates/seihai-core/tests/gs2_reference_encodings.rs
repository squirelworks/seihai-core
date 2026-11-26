//! Encodings found by differential testing against an independent GS2
//! compiler run as a black box over the Zodiac and Era corpus. Only the
//! observed bytecode shapes are recorded here, written out by hand; no code
//! from that compiler is used.

use seihai_core::scripting::gs2::compile;

/// Disassembly lines without their op index, e.g. `OP_TYPE_NUMBER 3.0`.
fn lines(src: &str) -> Vec<String> {
    let script = compile::compile_gs2(src).unwrap();
    script
        .disassemble()
        .lines()
        .filter_map(|l| {
            let t = l.trim_start();
            let (index, rest) = t.split_once(char::is_whitespace)?;
            index.parse::<usize>().ok()?;
            Some(rest.trim().to_string())
        })
        .collect()
}

fn has_run(lines: &[String], run: &[&str]) -> bool {
    lines
        .windows(run.len())
        .any(|w| w.iter().zip(run).all(|(a, b)| a == b))
}

#[test]
fn decimal_literals_are_normalised() {
    let l = lines(
        "function onCreated() {\n  temp.a = 3.0;\n  temp.b = .5;\n  temp.c = -1.50;\n  temp.d = 1.;\n  temp.e = 007;\n  temp.f = 2.25;\n}\n",
    );
    for want in [
        "OP_TYPE_NUMBER 3",
        "OP_TYPE_NUMBER 0.5",
        "OP_TYPE_NUMBER -1.5",
        "OP_TYPE_NUMBER 1",
        "OP_TYPE_NUMBER 7",
        "OP_TYPE_NUMBER 2.25",
    ] {
        assert!(l.iter().any(|x| x == want), "missing `{want}` in {l:#?}");
    }
}

#[test]
fn a_value_written_two_ways_falls_back_to_the_normalised_text() {
    let l = lines("function onCreated() {\n  temp.a = 3.0;\n  temp.b = 3;\n}\n");
    assert!(!l.iter().any(|x| x == "OP_TYPE_NUMBER 3.0"), "{l:#?}");
}

#[test]
fn membership_tests_are_already_numbers() {
    let l = lines(
        "function onCreated() {\n  temp.s = (temp.a in |1, 3|) + 1;\n  temp.t = (temp.a in temp.l) + 1;\n  temp.u = (temp.a in {1, 2}) + 1;\n}\n",
    );
    assert!(
        has_run(&l, &["OP_IN_RANGE 0", "OP_TYPE_NUMBER 1", "OP_ADD"]),
        "{l:#?}"
    );
    assert_eq!(
        l.windows(2)
            .filter(|w| w[0] == "OP_IN_OBJ" && w[1] == "OP_TYPE_NUMBER 1")
            .count(),
        2,
        "{l:#?}"
    );
    assert!(!has_run(&l, &["OP_IN_OBJ", "OP_CONV_TO_FLOAT"]), "{l:#?}");
}

#[test]
fn makevar_converts_only_what_is_not_already_text() {
    // Official bodies push a text argument as it is.
    let l = lines("function onCreated() {\n  temp.u = makevar(\"x\");\n}\n");
    assert!(
        has_run(&l, &["OP_TYPE_STRING #1 \"x\"", "OP_MAKEVAR"]),
        "{l:#?}"
    );
    let l = lines("function onCreated() {\n  temp.u = makevar(temp.n);\n}\n");
    assert!(has_run(&l, &["OP_CONV_TO_STRING", "OP_MAKEVAR"]), "{l:#?}");
}

#[test]
fn arraylen_takes_an_object() {
    let l = lines("function onCreated() {\n  temp.n = arraylen(temp.l);\n}\n");
    assert!(has_run(&l, &["OP_CONV_TO_OBJECT", "OP_OBJ_SIZE"]), "{l:#?}");
}

#[test]
fn waitfor_pushes_in_source_order_and_pow_is_a_call() {
    let l = lines(
        "function onCreated() {\n  temp.e = waitfor(this, \"ev\", 5);\n  temp.p = pow(temp.x, 2);\n}\n",
    );
    assert!(
        has_run(
            &l,
            &[
                "OP_TYPE_VAR #1 \"this\"",
                "OP_CONV_TO_STRING",
                "OP_TYPE_STRING #2 \"ev\"",
                "OP_TYPE_NUMBER 5",
                "OP_WAITFOR"
            ]
        ),
        "{l:#?}"
    );
    // Official bodies call `pow` by name; `OP_POW` is only for `^`.
    assert!(
        has_run(&l, &["OP_TYPE_ARRAY", "OP_TYPE_NUMBER 2"]),
        "{l:#?}"
    );
    assert!(l.iter().any(|x| x.ends_with("\"pow\"")), "{l:#?}");
    assert!(!l.iter().any(|x| x == "OP_POW"), "{l:#?}");
}

#[test]
fn closing_return_unless_the_last_statement_is_a_return() {
    // Official bodies add `return 0` even after an `if`/`else` whose
    // branches all return (GS2 captures).
    let both = lines("function g() {\n  if (temp.x) { return 1; } else if (temp.y) { return 2; } else { return 3; }\n}\n");
    assert!(has_run(&both, &["OP_TYPE_NUMBER 0", "OP_RET"]), "{both:#?}");
    let last = lines("function b() {\n  temp.x = 1;\n  return 2;\n}\n");
    assert!(!last.iter().any(|x| x == "OP_TYPE_NUMBER 0"), "{last:#?}");
    let one = lines("function a() {\n  if (temp.x) return 1;\n}\n");
    assert!(has_run(&one, &["OP_TYPE_NUMBER 0", "OP_RET"]), "{one:#?}");
}

#[test]
fn old_names_key_codes_and_enum_members() {
    let l = lines(
        "enum Suit { HEARTS, SPADES = 3 };\nfunction onCreated() {\n  temp.a = playerx;\n  temp.b = VK_LEFT;\n  temp.c = Suit.SPADES;\n  temp.d = imgwidth(foo.png);\n  temp.e = sarraylen(temp.list);\n}\n",
    );
    assert!(l.iter().any(|x| x == "OP_PLAYER"), "{l:#?}");
    assert!(l.iter().any(|x| x == "OP_TYPE_NUMBER 37"), "{l:#?}");
    assert!(l.iter().any(|x| x == "OP_TYPE_NUMBER 3"), "{l:#?}");
    assert!(l.iter().any(|x| x.ends_with("\"foo.png\"")), "{l:#?}");
    assert!(l.iter().any(|x| x.ends_with("\"getimgwidth\"")), "{l:#?}");
    assert!(l.iter().any(|x| x.ends_with("\"temp.list\"")), "{l:#?}");
    assert!(l.iter().any(|x| x == "OP_OBJ_FROM_STR"), "{l:#?}");
    assert!(!l.iter().any(|x| x.ends_with("\"playerx\"")), "{l:#?}");
}

/// [`lines`] for legacy mode, which takes bare names as statements.
fn legacy_lines(src: &str) -> Vec<String> {
    let script = compile::compile_gs2_legacy(src).unwrap().script;
    script
        .disassemble()
        .lines()
        .filter_map(|l| {
            let t = l.trim_start();
            let (index, rest) = t.split_once(char::is_whitespace)?;
            index.parse::<usize>().ok()?;
            Some(rest.trim().to_string())
        })
        .collect()
}

#[test]
fn unused_values_and_bare_names_are_left_out() {
    let l = legacy_lines(
        "function f(temp.s) {\n  temp.s.trim();\n  sin(test());\n  float(temp.s);\n  custom;\n  temp.x;\n  hide;\n}\n",
    );
    assert!(
        !l.iter().any(|x| x == "OP_OBJ_TRIM" || x == "OP_SIN"),
        "{l:#?}"
    );
    assert!(!l.iter().any(|x| x.ends_with("\"custom\"")), "{l:#?}");
    // `test()` went with `sin`, so nothing is called but `hide`.
    assert!(l.iter().any(|x| x.ends_with("\"hide\"")), "{l:#?}");
    assert!(!l.iter().any(|x| x.ends_with("\"test\"")), "{l:#?}");
}

#[test]
fn lower_float_translate_and_link() {
    let l = lines(
        "function f(temp.s) {\n  temp.a = temp.s.lower();\n  temp.b = float(temp.s);\n  temp.c = _(temp.s);\n  temp.d = temp.s.link();\n}\n",
    );
    assert!(l.iter().any(|x| x.ends_with("\"lowercase\"")), "{l:#?}");
    assert!(has_run(&l, &["OP_CONV_TO_FLOAT", "OP_ASSIGN"]), "{l:#?}");
    assert!(
        has_run(&l, &["OP_CONV_TO_STRING", "OP_TRANSLATE"]),
        "{l:#?}"
    );
    assert!(has_run(&l, &["OP_CONV_TO_OBJECT", "OP_OBJ_LINK"]), "{l:#?}");
}

#[test]
fn bit_operators_and_compound_assignments() {
    let l = lines(
        "function f(temp.a, temp.b) {\n  temp.r = temp.a xor temp.b;\n  temp.r = ~temp.a;\n  temp.r ^= 2;\n  temp.r <<= 1;\n  temp.r >>= 1;\n  echo(temp.a += 5);\n}\n",
    );
    for op in [
        "OP_BWX",
        "OP_BWI",
        "OP_POW",
        "OP_BW_LEFTSHIFT",
        "OP_BW_RIGHTSHIFT",
    ] {
        assert!(l.iter().any(|x| x == op), "{op}: {l:#?}");
    }
}

#[test]
fn labeled_loops_jump_to_the_named_loop() {
    let l = lines(
        "function f() {\n  outer: for (temp.i = 0; temp.i < 3; temp.i++) {\n    for (temp.j = 0; temp.j < 3; temp.j++) {\n      if (temp.j == 1) continue outer;\n      if (temp.i == 2) break outer;\n    }\n  }\n}\n",
    );
    assert!(
        l.iter().filter(|x| x.starts_with("OP_SET_INDEX")).count() >= 4,
        "{l:#?}"
    );
}

#[test]
fn a_new_block_without_a_name_uses_unknown_object() {
    let l = lines("function f() {\n  new TThing() {\n    this.k = 1;\n  }\n}\n");
    assert!(
        l.iter()
            .any(|x| x.starts_with("OP_TYPE_VAR") && x.ends_with("\"unknown_object\"")),
        "{l:#?}"
    );
    assert!(!l.iter().any(|x| x == "OP_INLINE_NEW"), "{l:#?}");
}
