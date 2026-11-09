//! Issue 371: the temporary GS1 front end and the GS2 subset it lowers into.
//!
//! `shared.gs2bc` is the open-source compiler's output for `shared.gs2`.
//! Both front ends must reproduce that bytecode. No real client is involved.

use seihai_core::scripting::engine::Era;
use seihai_core::scripting::gs2::compile::{self, gs1_commands_without_gs2, gs2_event_function};
use seihai_core::scripting::gs2::{Op, Script};
use std::path::Path;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/gs2")
            .join(name),
    )
    .unwrap()
}

fn ops(script: &Script) -> Vec<String> {
    script.code.iter().map(|ins| ins.op.to_string()).collect()
}

#[test]
fn gs2_subset_matches_the_shared_fixture_byte_for_byte() {
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gs2/shared.gs2"),
    )
    .unwrap();
    let got = compile::compile_gs2(&src).unwrap();
    let bytes = fixture("shared.gs2bc");
    let want = Script::parse(&bytes).unwrap();
    assert_eq!(got.disassemble(), want.disassemble(), "opcodes diverged");
    assert_eq!(got.write(), bytes);
}

#[test]
fn gs1_front_end_matches_the_shared_fixture() {
    let src = "\
if (created) {
  this.count = 3 + 4 * 2;
  if (this.count > 10) {
    set gs2seen;
    showimg 1, light2.png, x, y;
  }
  hideimg 1;
  sleep 0.5;
}
";
    let got = compile::compile_gs1(src, Era::V205).unwrap();
    let want = Script::parse(&fixture("shared.gs2bc")).unwrap();
    assert_eq!(got.functions, want.functions);
    assert_eq!(got.strings, want.strings);
    assert_eq!(ops(&got), ops(&want));
    assert_eq!(got.write(), want.write());
}

#[test]
fn add_function_matches_the_calls_fixture_prefix() {
    let src = "function add(a, b) {\n  return a + b;\n}\n";
    let got = compile::compile_gs2(src).unwrap();
    let want = Script::parse(&fixture("calls.gs2bc")).unwrap();
    assert_eq!(got.strings, &want.strings[..2]);
    assert_eq!(got.functions[0].name, "add");
    assert_eq!(got.functions[0].start, 1);
    let got_ops = ops(&got);
    let want_ops = ops(&want);
    assert_eq!(&got_ops[1..12], &want_ops[1..12]);
    assert_eq!(got.code.last().map(|i| i.op), Some(Op::Ret));
}

#[test]
fn while_true_uses_the_calls_fixture_shape() {
    let src = "function onCreated() {\n  while (true) {\n    this.k++;\n  }\n}\n";
    let got = compile::compile_gs2(src).unwrap();
    let names = ops(&got);
    assert_eq!(
        names,
        [
            "OP_SET_INDEX",
            "OP_TYPE_ARRAY",
            "OP_FUNC_PARAMS_END",
            "OP_JMP",
            "OP_TYPE_TRUE",
            "OP_CONV_TO_FLOAT",
            "OP_IF",
            "OP_CMD_CALL",
            "OP_THIS",
            "OP_TYPE_VAR",
            "OP_MEMBER_ACCESS",
            "OP_INC",
            "OP_INDEX_DEC",
            "OP_SET_INDEX",
            "OP_TYPE_NUMBER",
            "OP_RET",
            "OP_RET",
        ]
    );
    assert_eq!(got.code[13].op, Op::SetIndex);
    assert_eq!(
        got.code[13].jump_target(),
        Some(4),
        "the loop jumps back to true"
    );
    assert_eq!(
        got.code[6].jump_target(),
        Some(14),
        "a failed test leaves the loop"
    );
}

#[test]
fn for_loop_continue_lands_on_the_step() {
    let src = "\
function onTimeout() {
  for (temp.i = 0; temp.i < 5; temp.i++) {
    if (temp.i == 2) continue;
    temp.n += temp.i;
  }
}
";
    let got = compile::compile_gs2(src).unwrap();
    let names = ops(&got);
    let inc = names.iter().position(|n| n == "OP_INC").unwrap();
    let step = inc - 3;
    assert_eq!(names[step], "OP_TEMP");
    assert_eq!(
        got.code
            .iter()
            .filter(|i| i.op == Op::SetIndex && i.jump_target() == Some(step))
            .count(),
        1,
        "continue lands on the ++ step, got {names:?}"
    );
    let test = names
        .iter()
        .enumerate()
        .position(|(i, n)| i > 4 && n == "OP_LT")
        .unwrap()
        - 5;
    assert!(
        got.code
            .iter()
            .any(|i| i.op == Op::SetIndex && i.jump_target() == Some(test)),
        "the for loop jumps back to its test, got {names:?}"
    );
}

#[test]
fn logic_chain_matches_the_control_fixture_opcodes() {
    let src = "\
function onTimeout() {
  temp.ok = (temp.n > 0 && temp.n < 300) || temp.n == -1;
}
";
    let got = compile::compile_gs2(src).unwrap();
    let names = ops(&got);
    let start = names.iter().position(|n| n == "OP_TEMP").unwrap();
    assert_eq!(
        &names[start..],
        [
            "OP_TEMP",
            "OP_TYPE_VAR",
            "OP_MEMBER_ACCESS",
            "OP_TEMP",
            "OP_TYPE_VAR",
            "OP_MEMBER_ACCESS",
            "OP_CONV_TO_FLOAT",
            "OP_TYPE_NUMBER",
            "OP_GT",
            "OP_AND",
            "OP_TEMP",
            "OP_TYPE_VAR",
            "OP_MEMBER_ACCESS",
            "OP_CONV_TO_FLOAT",
            "OP_TYPE_NUMBER",
            "OP_LT",
            "OP_OR",
            "OP_TEMP",
            "OP_TYPE_VAR",
            "OP_MEMBER_ACCESS",
            "OP_TYPE_NUMBER",
            "OP_EQ",
            "OP_INLINE_CONDITIONAL",
            "OP_ASSIGN",
            "OP_TYPE_NUMBER",
            "OP_RET",
            "OP_RET",
        ]
    );
}

#[test]
fn events_become_gs2_functions() {
    let src = "\
if (playerenters) { hide; }
if (created || timeout) { show; }
if (playerchats && client.key) { message hi; }
";
    let got = compile::compile_gs1(src, Era::V205).unwrap();
    let names: Vec<_> = got.functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        ["onPlayerEnters", "onCreated", "onTimeout", "onPlayerChats"]
    );
    assert!(got.disassemble().contains("OP_IF"));
    assert_eq!(gs2_event_function("playerenters"), "onPlayerEnters");
}

#[test]
fn commands_and_functions_without_gs2_are_errors() {
    let lacking = gs1_commands_without_gs2();
    for name in [
        "setstring",
        "addstring",
        "tokenize",
        "tokenize2",
        "setarray",
    ] {
        if name == "setarray" {
            assert!(!lacking.contains(&name), "{name} has an opcode");
        } else {
            assert!(lacking.contains(&name), "{name} should be listed");
        }
    }
    for name in [
        "showimg",
        "set",
        "unset",
        "setplayerprop",
        "sleep",
        "hide",
        "message",
    ] {
        assert!(!lacking.contains(&name), "{name} is a GS2 command");
    }
    let err = compile::compile_gs1("if (created) { setstring a,b; }\n", Era::V205).unwrap_err();
    assert!(err.to_string().contains("setstring"), "{err}");
    assert!(err.to_string().contains("no GS2 equivalent"), "{err}");
    let err = compile::compile_gs1("if (created) { putnpc x,1,2; }\n", Era::V205).unwrap_err();
    assert!(err.to_string().contains("putnpc"), "{err}");
    let err = compile::compile_gs1("if (created) { if (playersays(hello)) hide; }\n", Era::V205)
        .unwrap_err();
    assert!(err.to_string().contains("playersays"), "{err}");
    let err =
        compile::compile_gs1("if (created) { with (player) { hide; } }\n", Era::V21).unwrap_err();
    assert!(err.to_string().contains("with"), "{err}");
    let err =
        compile::compile_gs1("if (created) hide;\nif (created) show;\n", Era::V205).unwrap_err();
    assert!(err.to_string().contains("duplicate"), "{err}");
    let err = compile::compile_gs1("x = 1;\n", Era::V205).unwrap_err();
    assert!(err.to_string().contains("top-level"), "{err}");
}

#[test]
fn reading_an_event_flag_is_an_error() {
    let err = compile::compile_gs1("if (created) { this.a = timeout; }\n", Era::V205).unwrap_err();
    assert!(err.to_string().contains("event flag"), "{err}");
    let got = compile::compile_gs1("if (timeout) { timeout = 0.05; }\n", Era::V205).unwrap();
    assert_eq!(got.functions[0].name, "onTimeout");
    assert!(got.disassemble().contains("OP_ASSIGN"));
}

#[test]
fn opcode_builtins_reject_missing_and_extra_arguments() {
    let fixed = [
        ("sleep", 1),
        ("sin", 1),
        ("cos", 1),
        ("arctan", 1),
        ("abs", 1),
        ("arraylen", 1),
        ("sarraylen", 1),
        ("int", 1),
        ("exp", 1),
        ("char", 1),
        ("vecx", 1),
        ("vecy", 1),
        ("makevar", 1),
        ("random", 2),
        ("setarray", 2),
        ("log", 2),
        ("min", 2),
        ("max", 2),
        ("pow", 2),
        ("getangle", 2),
        ("getdir", 2),
        ("waitfor", 3),
    ];
    for (name, arity) in fixed {
        for count in [0, arity - 1, arity + 1] {
            let args = vec!["1"; count].join(", ");
            let source = format!("function onCreated() {{ {name}({args}); }}");
            let error = compile::compile_gs2(&source).unwrap_err().to_string();
            assert!(
                error.contains(name) && error.contains("expects"),
                "{source}: {error}"
            );
        }
        let args = vec!["1"; arity].join(", ");
        let source = format!("function onCreated() {{ {name}({args}); }}");
        compile::compile_gs2(&source).unwrap_or_else(|error| panic!("{source}: {error}"));
    }
}

#[test]
fn malformed_nested_builtins_and_gs1_calls_fail_compilation() {
    for call in [
        "sin()",
        "sin(1, 2)",
        "random(1)",
        "strlen()",
        "startswith(1)",
        "indexof(1)",
    ] {
        let source = format!("if (created) {{ this.n = {call}; }}");
        assert!(
            compile::compile_gs1(&source, Era::V205).is_err(),
            "{source}"
        );
    }
    for source in [
        "function onCreated() { this.n = 1 + sin(); }",
        "function onCreated() { hostcall(cos(1, 2)); }",
        "function onCreated() { if (random(1)) hide(); }",
    ] {
        assert!(compile::compile_gs2(source).is_err(), "{source}");
    }
    for source in [
        "if (created) { sleep; }",
        "if (created) { sleep 0.5, 1; }",
        "if (created) { setarray this.a; }",
        "if (created) { setarray this.a, 3, 4; }",
    ] {
        assert!(compile::compile_gs1(source, Era::V205).is_err(), "{source}");
    }
    // GS1's final raw text argument can itself contain commas.
    compile::compile_gs1(
        "if (created) { this.n = indexof(needle, hay,stack); }",
        Era::V205,
    )
    .unwrap();
}

#[test]
fn format_requires_a_pattern_but_keeps_variadic_arguments() {
    assert!(compile::compile_gs2("function onCreated() { format(); }").is_err());
    for args in ["\"plain\"", "\"%s %s\", 1, 2", "\"%s\", sin(1)"] {
        let source = format!("function onCreated() {{ this.text = format({args}); }}");
        compile::compile_gs2(&source).unwrap();
    }
    // A name outside the opcode subset is still a script/host call.
    compile::compile_gs2("function onCreated() { custom(); custom(1, 2, 3, 4); }").unwrap();
}
