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
fn decimal_literals_keep_their_spelling() {
    let l = lines(
        "function onCreated() {\n  temp.a = 3.0;\n  temp.b = .5;\n  temp.c = -1.50;\n  temp.d = 1.;\n  temp.e = 007;\n  temp.f = 2.25;\n}\n",
    );
    for want in [
        "OP_TYPE_NUMBER 3.0",
        "OP_TYPE_NUMBER .5",
        "OP_TYPE_NUMBER -1.50",
        "OP_TYPE_NUMBER 1.",
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
        has_run(&l, &["OP_IN_RANGE", "OP_TYPE_NUMBER 1", "OP_ADD"]),
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
fn makevar_takes_a_string_even_from_a_string() {
    let l = lines("function onCreated() {\n  temp.u = makevar(\"x\");\n}\n");
    assert!(has_run(&l, &["OP_CONV_TO_STRING", "OP_MAKEVAR"]), "{l:#?}");
}

#[test]
fn arraylen_takes_an_object() {
    let l = lines("function onCreated() {\n  temp.n = arraylen(temp.l);\n}\n");
    assert!(has_run(&l, &["OP_CONV_TO_OBJECT", "OP_OBJ_SIZE"]), "{l:#?}");
}

#[test]
fn waitfor_and_pow_push_in_source_order() {
    let l = lines(
        "function onCreated() {\n  temp.e = waitfor(this, \"ev\", 5);\n  temp.p = pow(temp.x, 2);\n}\n",
    );
    assert!(
        has_run(
            &l,
            &[
                "OP_THIS",
                "OP_CONV_TO_STRING",
                "OP_TYPE_STRING #1 \"ev\"",
                "OP_TYPE_NUMBER 5",
                "OP_WAITFOR"
            ]
        ),
        "{l:#?}"
    );
    assert!(
        has_run(&l, &["OP_CONV_TO_FLOAT", "OP_TYPE_NUMBER 2", "OP_POW"]),
        "{l:#?}"
    );
}

#[test]
fn no_closing_return_after_an_if_else_that_always_returns() {
    let both = lines("function g() {\n  if (temp.x) { return 1; } else if (temp.y) { return 2; } else { return 3; }\n}\n");
    assert_eq!(
        both.iter().filter(|x| *x == "OP_RET").count(),
        3,
        "{both:#?}"
    );
    let one = lines("function a() {\n  if (temp.x) return 1;\n}\n");
    assert!(has_run(&one, &["OP_TYPE_NUMBER 0", "OP_RET"]), "{one:#?}");
}
