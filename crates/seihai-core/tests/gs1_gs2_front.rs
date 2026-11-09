//! Issue 371: the temporary GS1 front end and the GS2 subset it lowers into.
//!
//! `shared.gs2bc` is the open-source compiler's output for `shared.gs2`.
//! Both front ends must reproduce that bytecode. No real client is involved.

use seihai_core::scripting::engine::Era;
use seihai_core::scripting::engine::Val;
use seihai_core::scripting::gs2::compile::{self, gs1_commands_without_gs2, gs2_event_function};
use seihai_core::scripting::gs2::exec::{EffectHost, Machine, Outcome, Root};
use seihai_core::scripting::gs2::lists::Value;
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

fn first_literal(script: &Script) -> Vec<(Op, seihai_core::scripting::gs2::Operand)> {
    let end = script
        .code
        .iter()
        .position(|i| i.op == Op::ArrayEnd)
        .unwrap();
    let start = script.code[..end]
        .iter()
        .rposition(|i| i.op == Op::TypeArray)
        .unwrap();
    script.code[start..=end]
        .iter()
        .map(|i| (i.op, i.operand.clone()))
        .collect()
}

fn run_created(script: Script) -> Machine {
    let mut machine = Machine::new(script);
    let mut host = EffectHost::new(Era::V21, 7);
    assert_eq!(machine.run_top_level(&mut host), Outcome::Done);
    assert_eq!(
        machine.call_event("onCreated", &[], &mut host),
        Outcome::Done
    );
    assert!(host.log.is_empty(), "{:?}", host.log);
    machine
}

#[test]
fn array_literal_matches_the_existing_fixture_and_both_front_ends() {
    let gs2 = compile::compile_gs2("function onCreated() { this.a = {10, \"two\", 30}; }").unwrap();
    let gs1 =
        compile::compile_gs1("if (created) { this.a = {10, \"two\", 30}; }", Era::V21).unwrap();
    assert_eq!(gs1.write(), gs2.write());
    let reference = Script::parse(&fixture("lists.gs2bc")).unwrap();
    assert_eq!(&gs2.strings[..2], &reference.strings[..2]);
    assert_eq!(first_literal(&gs2), first_literal(&reference));
    let machine = run_created(gs1);
    assert_eq!(
        machine.list(Root::This, "a"),
        Some(
            [
                Value::from(Val::Num(10.0)),
                Value::from(Val::from("two")),
                Value::from(Val::Num(30.0)),
            ]
            .as_slice()
        )
    );
}

#[test]
fn empty_nested_and_trailing_comma_literals_keep_their_structure() {
    let script =
        compile::compile_gs2("function onCreated() { this.a = {}; this.b = {1, {2, 3,}, {},}; }")
            .unwrap();
    let machine = run_created(script);
    assert_eq!(machine.list(Root::This, "a"), Some([].as_slice()));
    assert_eq!(
        machine.list(Root::This, "b"),
        Some(
            [
                Value::from(Val::Num(1.0)),
                Value::List(vec![Value::from(Val::Num(2.0)), Value::from(Val::Num(3.0))]),
                Value::List(vec![]),
            ]
            .as_slice()
        )
    );
}

#[test]
fn gs2_literal_calls_preserve_the_reverse_push_order_and_array_marker() {
    let script = compile::compile_gs2(
        "function next() { this.count += 1; return this.count; }
         function onCreated() { this.count = 0; this.a = {next(), next()}; }",
    )
    .unwrap();
    let machine = run_created(script);
    assert_eq!(machine.var(Root::This, "count"), Some(&Val::Num(2.0)));
    assert_eq!(
        machine.list(Root::This, "a"),
        Some([Value::from(Val::Num(2.0)), Value::from(Val::Num(1.0))].as_slice())
    );
}

#[test]
fn gs1_sparse_entries_and_scalar_expressions_lower_without_losing_positions() {
    let gs1 = compile::compile_gs1(
        "if (created) { this.a = {1 + 2, , -4,}; this.b = {}; }",
        Era::V21,
    )
    .unwrap();
    let gs2 =
        compile::compile_gs2("function onCreated() { this.a = {1 + 2, 0, -4,}; this.b = {}; }")
            .unwrap();
    assert_eq!(gs1.write(), gs2.write());
    let machine = run_created(gs1);
    assert_eq!(
        machine.list(Root::This, "a"),
        Some(
            [
                Value::from(Val::Num(3.0)),
                Value::from(Val::Num(0.0)),
                Value::from(Val::Num(-4.0)),
            ]
            .as_slice()
        )
    );
    assert_eq!(machine.list(Root::This, "b"), Some([].as_slice()));
}

#[test]
fn malformed_and_unsupported_array_forms_fail_clearly() {
    for literal in ["{1", "{1,,2}", "{,1}", "{1 2}"] {
        let source = format!("function onCreated() {{ this.a = {literal}; }}");
        assert!(compile::compile_gs2(&source).is_err(), "{source}");
    }
    for literal in ["{next(), 1}", "{1 + next(), 2}", "{{1}, 2}"] {
        let source = format!("if (created) {{ this.a = {literal}; }}");
        let error = compile::compile_gs1(&source, Era::V21)
            .unwrap_err()
            .to_string();
        assert!(error.contains("calls or nested lists"), "{source}: {error}");
    }
    assert!(compile::compile_gs1("if (created) { this.a = {1, 2}; }", Era::V205).is_err());
    assert!(compile::compile_gs2("function onCreated() { this.a[0] = 1; }").is_err());
}
