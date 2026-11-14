//! Issue 371: the temporary GS1 front end and the GS2 subset it lowers into.
//!
//! `shared.gs2bc` is the open-source compiler's output for `shared.gs2`.
//! Both front ends must reproduce that bytecode. No real client is involved.

use seihai_core::scripting::engine::Era;
use seihai_core::scripting::engine::Val;
use seihai_core::scripting::gs2::compile::{
    self, gs1_commands_without_gs2, gs2_event_function, CompileErrors,
};
use seihai_core::scripting::gs2::exec::{EffectHost, Machine, Outcome, Root};
use seihai_core::scripting::gs2::lists::Value;
use seihai_core::scripting::gs2::{Op, Script};
use std::path::Path;

/// Legacy mode: these are forms live scripts use.
fn compile_gs2(src: &str) -> Result<Script, CompileErrors> {
    compile::compile_gs2_legacy(src).map(|c| c.script)
}

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
    let got = compile_gs2(&src).unwrap();
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
    let got = compile_gs2(src).unwrap();
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
    let got = compile_gs2(src).unwrap();
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
    let got = compile_gs2(src).unwrap();
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
    let got = compile_gs2(src).unwrap();
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
    // `setstring` on a plain name is a player flag: `set("a=b")`.
    let got = compile::compile_gs1("if (created) { setstring a,b; }\n", Era::V205).unwrap();
    assert!(
        got.strings.contains(&"a=b".to_string()),
        "{:?}",
        got.strings
    );
    let err = compile::compile_gs1("if (created) { addstring a,b; }\n", Era::V217).unwrap_err();
    assert!(err.to_string().contains("addstring"), "{err}");
    let err = compile::compile_gs1("if (created) { putnpc x,1,2; }\n", Era::V205).unwrap_err();
    assert!(err.to_string().contains("putnpc"), "{err}");
    let err = compile::compile_gs1("if (created) { if (playersays(hello)) hide; }\n", Era::V205)
        .unwrap_err();
    assert!(err.to_string().contains("playersays"), "{err}");
    // A command inside `with` runs against the `with` object in both.
    assert!(compile::compile_gs1("if (created) { with (player) { hide; } }\n", Era::V217).is_ok());
    let err = compile::compile_gs1(
        "function onCreated() { hide; }\nfunction onCreated() { show; }\n",
        Era::V205,
    )
    .unwrap_err();
    assert!(err.to_string().contains("duplicate"), "{err}");
    // Code outside every event test becomes GS2 code outside functions; after
    // an event test only a command or call is taken.
    compile::compile_gs1("x = 1;\nif (created) { hide; }\n", Era::V205).unwrap();
    compile::compile_gs1("if (created) { hide; }\ndontblock;\n", Era::V205).unwrap();
    let err = compile::compile_gs1("if (created) { hide; }\nx = 1;\n", Era::V205).unwrap_err();
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
            let error = compile_gs2(&source).unwrap_err().to_string();
            assert!(
                error.contains(name) && error.contains("expects"),
                "{source}: {error}"
            );
        }
        let args = vec!["1"; arity].join(", ");
        let source = format!("function onCreated() {{ {name}({args}); }}");
        compile_gs2(&source).unwrap_or_else(|error| panic!("{source}: {error}"));
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
        assert!(compile_gs2(source).is_err(), "{source}");
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
    assert!(compile_gs2("function onCreated() { format(); }").is_err());
    for args in ["\"plain\"", "\"%s %s\", 1, 2", "\"%s\", sin(1)"] {
        let source = format!("function onCreated() {{ this.text = format({args}); }}");
        compile_gs2(&source).unwrap();
    }
    // A name outside the opcode subset is still a script/host call.
    compile_gs2("function onCreated() { custom(); custom(1, 2, 3, 4); }").unwrap();
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
    let mut host = EffectHost::new(Era::V217, 7);
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
    let gs2 = compile_gs2("function onCreated() { this.a = {10, \"two\", 30}; }").unwrap();
    let gs1 =
        compile::compile_gs1("if (created) { this.a = {10, \"two\", 30}; }", Era::V217).unwrap();
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
        compile_gs2("function onCreated() { this.a = {}; this.b = {1, {2, 3,}, {},}; }").unwrap();
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
    let script = compile_gs2(
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
        Era::V217,
    )
    .unwrap();
    let gs2 =
        compile_gs2("function onCreated() { this.a = {1 + 2, 0, -4,}; this.b = {}; }").unwrap();
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
        assert!(compile_gs2(&source).is_err(), "{source}");
    }
    // Stored straight into a variable, these lower in order through
    // `temp.` variables; anywhere else they stay errors.
    for (literal, want) in [
        ("{next(), next()}", "one element calls"),
        ("{next(), this.v}", "one element calls"),
        ("{random(0, 1), random(0, 1)}", "one element calls"),
        ("{{next()}, 2}", "that are lists"),
    ] {
        let source = format!(
            "function next() {{ return 1; }}\nfunction keep(v) {{ this.k = v; }}\nif (created) {{ keep({literal}); }}"
        );
        let error = compile::compile_gs1(&source, Era::V217)
            .unwrap_err()
            .to_string();
        assert!(error.contains(want), "{source}: {error}");
    }
    assert!(compile::compile_gs1("if (created) { this.a = {1, 2}; }", Era::V205).is_err());
    assert!(compile_gs2("function onCreated() { this.a[0] = {1}; }").is_ok());
    // A list into an element lowers as GS2 writes it (`elemlist.gs2bc`).
    assert!(compile::compile_gs1("if (created) { this.a[0] = {1}; }", Era::V217).is_ok());
}

/// Instructions `from..=to` with string operands written out, so two scripts
/// with different string tables can be compared op for op.
fn listing(script: &Script, from: usize, to: usize) -> Vec<String> {
    use seihai_core::scripting::gs2::Operand;
    script.code[from..=to]
        .iter()
        .map(|i| match &i.operand {
            Operand::None => i.op.to_string(),
            Operand::Str(n) => format!("{} {:?}", i.op, script.strings[*n as usize]),
            Operand::Int(n) => format!("{} {n}", i.op),
            Operand::Decimal(t) => format!("{} {t}", i.op),
        })
        .collect()
}

/// The body of the only function, without its header (params, jump, the
/// call marker) and without the closing `return 0`.
fn body(script: &Script) -> Vec<String> {
    let names = ops(script);
    let start = names.iter().position(|n| n == "OP_JMP").unwrap() + 1;
    let start = if names[start] == "OP_CMD_CALL" {
        start + 1
    } else {
        start
    };
    listing(script, start, script.code.len() - 4)
}

#[test]
fn element_read_and_write_match_the_lists_fixture() {
    let reference = Script::parse(&fixture("lists.gs2bc")).unwrap();
    // lists.gs2: `this.first = temp.a[0]; temp.a[1] = 20;` is ops 23..=39.
    let want = listing(&reference, 23, 39);
    let gs2 =
        compile_gs2("function onCreated() {\n  this.first = temp.a[0];\n  temp.a[1] = 20;\n}\n")
            .unwrap();
    assert_eq!(body(&gs2), want);
    // GS1 has no `temp.`; `this.` lowers the same way on both sides.
    let gs1 = compile::compile_gs1(
        "if (created) {\n  this.first = this.a[0];\n  this.a[1] = 20;\n}\n",
        Era::V217,
    )
    .unwrap();
    let gs2 =
        compile_gs2("function onCreated() {\n  this.first = this.a[0];\n  this.a[1] = 20;\n}\n")
            .unwrap();
    assert_eq!(gs1.write(), gs2.write());
    assert!(ops(&gs1).contains(&"OP_ARRAY".to_string()));
    assert!(ops(&gs1).contains(&"OP_ARRAY_ASSIGN".to_string()));
}

#[test]
fn two_index_read_and_write_match_the_lists_fixture() {
    let reference = Script::parse(&fixture("lists.gs2bc")).unwrap();
    // `temp.g[1, 2] = 9; this.g = temp.g[1, 2];` is ops 171..=189.
    let want = listing(&reference, 171, 189);
    let gs2 =
        compile_gs2("function onCreated() {\n  temp.g[1, 2] = 9;\n  this.g = temp.g[1, 2];\n}\n")
            .unwrap();
    assert_eq!(body(&gs2), want);
    let gs1 = compile::compile_gs1(
        "if (created) {\n  this.h[1, 2] = 9;\n  this.g = this.h[1, 2];\n}\n",
        Era::V217,
    )
    .unwrap();
    let gs2 =
        compile_gs2("function onCreated() {\n  this.h[1, 2] = 9;\n  this.g = this.h[1, 2];\n}\n")
            .unwrap();
    assert_eq!(gs1.write(), gs2.write());
}

#[test]
fn in_array_matches_the_lists_fixture() {
    let reference = Script::parse(&fixture("lists.gs2bc")).unwrap();
    // `this.has = 40 in temp.a;` is ops 89..=98.
    let want = listing(&reference, 89, 98);
    let gs2 = compile_gs2("function onCreated() {\n  this.has = 40 in temp.a;\n}\n").unwrap();
    assert_eq!(body(&gs2), want);
    let gs1 =
        compile::compile_gs1("if (created) {\n  this.has = 40 in this.a;\n}\n", Era::V217).unwrap();
    let gs2 = compile_gs2("function onCreated() {\n  this.has = 40 in this.a;\n}\n").unwrap();
    assert_eq!(gs1.write(), gs2.write());
}

#[test]
fn lowered_elements_run_on_the_gs2_machine() {
    let script = compile::compile_gs1(
        "if (created) {
  this.a = {10, 20, 30};
  this.i = 1;
  this.a[this.i + 1] = 7;
  this.first = this.a[0];
  this.last = this.a[2];
  this.has = 7 in this.a;
  this.not = 8 in this.a;
}
",
        Era::V217,
    )
    .unwrap();
    // A computed index is converted like any numeric operand.
    let names = ops(&script);
    let assign = names.iter().position(|n| n == "OP_ARRAY_ASSIGN").unwrap();
    assert_eq!(names[assign - 2], "OP_ADD", "{names:?}");
    let machine = run_created(script);
    assert_eq!(
        machine.list(Root::This, "a"),
        Some(
            [
                Value::from(Val::Num(10.0)),
                Value::from(Val::Num(20.0)),
                Value::from(Val::Num(7.0)),
            ]
            .as_slice()
        )
    );
    assert_eq!(machine.var(Root::This, "first"), Some(&Val::Num(10.0)));
    assert_eq!(machine.var(Root::This, "last"), Some(&Val::Num(7.0)));
    assert_eq!(machine.var(Root::This, "has"), Some(&Val::Num(1.0)));
    assert_eq!(machine.var(Root::This, "not"), Some(&Val::Num(0.0)));
}

#[test]
fn a_call_in_an_index_marks_the_function_as_calling() {
    let script = compile_gs2("function onCreated() { this.a[pick()] = 1; }").unwrap();
    assert!(ops(&script).contains(&"OP_CMD_CALL".to_string()));
    let script = compile_gs2("function onCreated() { this.x = this.a[pick()]; }").unwrap();
    assert!(ops(&script).contains(&"OP_CMD_CALL".to_string()));
}

#[test]
fn element_forms_without_a_fixture_stay_errors() {
    let cases = [
        ("this.a[0, 1, 2] = 1;", "GS2 array opcodes take one or two"),
        ("this.x = npcs[0].save;", "a field after an index"),
        ("players[0].x = 5;", "a field after an index"),
        ("this.x = board[5];", "`board[..]`"),
        ("this.x = tiles[1];", "two indexes"),
    ];
    for (stmt, want) in cases {
        let source = format!("if (created) {{ {stmt} }}");
        let error = compile::compile_gs1(&source, Era::V217)
            .unwrap_err()
            .to_string();
        assert!(error.contains(want), "{source}: {error}");
    }
    // The GS2 front compiles them: `a[i] += v` as `a[i] = a[i] + v`
    // (Seihai's form), `a[i]++` as the element read and `OP_INC`.
    let s = compile_gs2("function onCreated() { this.a[0] += 1; }").unwrap();
    assert_eq!(
        &ops(&s)[4..],
        [
            "OP_THIS",
            "OP_TYPE_VAR",
            "OP_MEMBER_ACCESS",
            "OP_CONV_TO_OBJECT",
            "OP_TYPE_NUMBER",
            "OP_THIS",
            "OP_TYPE_VAR",
            "OP_MEMBER_ACCESS",
            "OP_CONV_TO_OBJECT",
            "OP_TYPE_NUMBER",
            "OP_ARRAY",
            "OP_CONV_TO_FLOAT",
            "OP_TYPE_NUMBER",
            "OP_ADD",
            "OP_ARRAY_ASSIGN",
            "OP_TYPE_NUMBER",
            "OP_RET",
            "OP_RET",
        ]
    );
    let s = compile_gs2("function onCreated() { a[i]++; }").unwrap();
    assert_eq!(
        &ops(&s)[4..11],
        [
            "OP_TYPE_VAR",
            "OP_CONV_TO_OBJECT",
            "OP_TYPE_VAR",
            "OP_CONV_TO_FLOAT",
            "OP_ARRAY",
            "OP_INC",
            "OP_INDEX_DEC",
        ]
    );
    // A field or a second index after an index is a GS2 postfix form: the
    // element, then `OP_MEMBER_ACCESS` or another `OP_ARRAY` (no fixture).
    for stmt in ["this.x = npcs[0].x;", "this.x = this.a[0][1];"] {
        let source = format!("function onCreated() {{ {stmt} }}");
        let got = compile_gs2(&source).unwrap();
        assert!(got.disassemble().contains("OP_ARRAY"), "{source}");
    }
}

#[test]
fn joins_conditionals_ranges_and_player_position_lower_and_run() {
    let script = compile::compile_gs1(
        "if (created) {
  this.s = \"a\" @ \"b\";
  this.t = \"a\" SPC \"b\";
  this.c = 1 > 0 ? 5 : 6;
  this.r = 3 in |1, 5|;
  this.o = 9 in |1, 5|;
}
",
        Era::V217,
    )
    .unwrap();
    let machine = run_created(script);
    assert_eq!(machine.var(Root::This, "s"), Some(&Val::from("ab")));
    assert_eq!(machine.var(Root::This, "t"), Some(&Val::from("a b")));
    assert_eq!(machine.var(Root::This, "c"), Some(&Val::Num(5.0)));
    assert_eq!(machine.var(Root::This, "r"), Some(&Val::Num(1.0)));
    assert_eq!(machine.var(Root::This, "o"), Some(&Val::Num(0.0)));
    let gs1 = compile::compile_gs1("if (created) { this.x = playerx; }", Era::V205).unwrap();
    let gs2 = compile_gs2("function onCreated() { this.x = player.x; }").unwrap();
    assert_eq!(gs1.write(), gs2.write());
}

/// GS1 `a[i] op= v`, `a[i]++` and `a[i]--` lower to the element write of
/// `a[i] = a[i] op v`, op for op (the element read and write are the
/// `lists` fixture's), and run on the GS2 machine.
#[test]
fn compound_element_assignment_lowers_to_read_and_write() {
    let pairs = [
        ("this.a[0] += 5;", "this.a[0] = this.a[0] + 5;"),
        ("this.a[1] -= 2;", "this.a[1] = this.a[1] - 2;"),
        ("this.a[0] *= 3;", "this.a[0] = this.a[0] * 3;"),
        ("this.a[1] /= 4;", "this.a[1] = this.a[1] / 4;"),
        ("this.a[this.i]++;", "this.a[this.i] = this.a[this.i] + 1;"),
        ("this.a[1]--;", "this.a[1] = this.a[1] - 1;"),
        ("this.m[1, 2] += 1;", "this.m[1, 2] = this.m[1, 2] + 1;"),
    ];
    for (short, long) in pairs {
        let a = compile::compile_gs1(&format!("if (created) {{ {short} }}"), Era::V217).unwrap();
        let b = compile::compile_gs1(&format!("if (created) {{ {long} }}"), Era::V217).unwrap();
        assert_eq!(a.write(), b.write(), "{short}");
    }
    let script = compile::compile_gs1(
        "if (created) {
  this.a = {10, 20};
  this.a[0] += 5;
  this.a[1]--;
  this.a[1] *= 2;
  this.p = this.a[0];
  this.q = this.a[1];
}
",
        Era::V217,
    )
    .unwrap();
    let machine = run_created(script);
    assert_eq!(machine.var(Root::This, "p"), Some(&Val::Num(15.0)));
    assert_eq!(machine.var(Root::This, "q"), Some(&Val::Num(38.0)));
    // The index is pushed twice: one that calls is refused.
    let error = compile::compile_gs1(
        "function pick() { return 0; }\nif (created) { this.a[pick()] += 1; }",
        Era::V217,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("index calls a function"), "{error}");
}

/// GS1 `x in {a, b, c}` lowers to `x == a || x == b || x == c` (the GS1
/// interpreter's equality, item by item, stopping at the first match) and
/// runs on the GS2 machine; an empty list is 0.
#[test]
fn in_a_literal_list_lowers_to_equality_tests() {
    let a =
        compile::compile_gs1("if (created) { this.r = this.n in {1, 2, 3}; }", Era::V217).unwrap();
    let b = compile::compile_gs1(
        "if (created) { this.r = this.n == 1 || this.n == 2 || this.n == 3; }",
        Era::V217,
    )
    .unwrap();
    assert_eq!(a.write(), b.write());
    let script = compile::compile_gs1(
        "if (created) {
  this.n = 2;
  if (this.n in {1, 2, 3}) this.hit = 1;
  if (this.n in {4, 5}) this.miss = 1;
}
",
        Era::V217,
    )
    .unwrap();
    let machine = run_created(script);
    assert_eq!(machine.var(Root::This, "hit"), Some(&Val::Num(1.0)));
    assert_eq!(machine.var(Root::This, "miss"), None);
    assert!(compile::compile_gs1(
        "function pick() { return 0; }\nif (created) { this.r = pick() in {1, 2}; }",
        Era::V217,
    )
    .is_err());
}

/// A GS1 array literal whose element evaluation order cannot matter is
/// lowered even with calls: one calling element among constants, or calls
/// only to opcode built-ins that change nothing (`int`, `abs`, ...). The
/// bytes are the GS2 front end's for the same literal.
#[test]
fn array_literals_whose_order_cannot_matter_lower_with_calls() {
    for literal in [
        "{next(), 1}",
        "{1 + next(), 2, \"x\"}",
        "{int(this.v), abs(this.w), 3}",
        "{random(0, 1), 2}",
    ] {
        let gs1 = compile::compile_gs1(
            &format!("function next() {{ return 4; }}\nif (created) {{ this.a = {literal}; }}"),
            Era::V217,
        )
        .unwrap();
        let gs2 = compile_gs2(&format!(
            "function next() {{ return 4; }}\nfunction onCreated() {{ this.a = {literal}; }}"
        ))
        .unwrap();
        assert_eq!(gs1.write(), gs2.write(), "{literal}");
    }
    let script = compile::compile_gs1(
        "function next() { return 4; }\nif (created) { this.v = 2.5; this.a = {int(this.v), 9}; this.b = {next(), 7}; }",
        Era::V217,
    )
    .unwrap();
    let machine = run_created(script);
    assert_eq!(
        machine.list(Root::This, "a"),
        Some([Value::from(Val::Num(2.0)), Value::from(Val::Num(9.0))].as_slice())
    );
    assert_eq!(
        machine.list(Root::This, "b"),
        Some([Value::from(Val::Num(4.0)), Value::from(Val::Num(7.0))].as_slice())
    );
    // A call next to a variable read, stored straight into a variable, is
    // read left to right through `temp.` variables.
    let script = compile::compile_gs1(
        "function next() { this.v = 9; return 4; }\nif (created) { this.v = 1; this.a = {this.v, next(), this.v, 5}; }",
        Era::V217,
    )
    .unwrap();
    assert!(
        script.strings.iter().any(|s| s == "__gs1list0"),
        "{:?}",
        script.strings
    );
    let machine = run_created(script);
    assert_eq!(
        machine.list(Root::This, "a"),
        Some(
            [1.0, 4.0, 9.0, 5.0]
                .map(|n| Value::from(Val::Num(n)))
                .as_slice()
        )
    );
    // In a call's argument the order is still refused.
    assert!(compile::compile_gs1(
        "function next() { return 4; }\nfunction keep(v) { this.k = v; }\nif (created) { keep({next(), int(this.v)}); }",
        Era::V217,
    )
    .is_err());
}

/// Ops `from..=to` with every `OP_WITH` jump made relative to `from`, so a
/// block can be compared wherever it sits in a script.
fn with_block(script: &Script, from: usize, to: usize) -> Vec<String> {
    use seihai_core::scripting::gs2::Operand;
    let mut out = listing(script, from, to);
    for (k, i) in script.code[from..=to].iter().enumerate() {
        if i.op == Op::With {
            if let Operand::Int(n) = i.operand {
                out[k] = format!("OP_WITH +{}", n as usize - from);
            }
        }
    }
    out
}

/// `with` is lowered in the encoding of `rest.gs2bc` (`with (player)` and
/// `with (null)`, ops 141..=161), by both front ends.
#[test]
fn with_matches_the_rest_fixture_in_both_front_ends() {
    let reference = Script::parse(&fixture("rest.gs2bc")).unwrap();
    let want = with_block(&reference, 141, 161);
    assert_eq!(want[0], "OP_PLAYER");
    assert_eq!(want[20], "OP_WITHEND");
    let gs2 = compile_gs2(
        "function onCreated() {\n  with (player) {\n    x = 12;\n    this.inw = 1;\n  }\n  with (null) {\n    this.never = 1;\n  }\n}\n",
    )
    .unwrap();
    let gs1 = compile::compile_gs1(
        "if (created) {\n  with (player) {\n    x = 12;\n    this.inw = 1;\n  }\n  with (null) {\n    this.never = 1;\n  }\n}\n",
        Era::V217,
    )
    .unwrap();
    for got in [&gs2, &gs1] {
        let start = ops(got).iter().position(|n| n == "OP_PLAYER").unwrap();
        assert_eq!(with_block(got, start, start + 20), want);
    }
    assert_eq!(gs1.write(), gs2.write());
}

/// The NPC server's notes describe `with (players[i])` in a loop over the
/// players; the target is an element read and the loop variable is read
/// inside the block.
#[test]
fn gs1_with_over_an_element_lowers_and_reads_outer_variables() {
    let script = compile::compile_gs1(
        "if (created) {\n  for (i = 0; i < 3; i++) with (this.list[i]) {\n    if (this.hp < i) x = i;\n    while (this.hp > 0) { if (this.hp == 1) break; this.hp--; }\n  }\n}\n",
        Era::V217,
    )
    .unwrap();
    let names = ops(&script);
    let w = names.iter().position(|n| n == "OP_WITH").unwrap();
    assert_eq!(names[w - 1], "OP_CONV_TO_OBJECT");
    assert_eq!(names[w - 2], "OP_ARRAY");
    let end = names.iter().position(|n| n == "OP_WITHEND").unwrap();
    assert_eq!(
        listing(&script, w, w)[0],
        format!("OP_WITH {}", end + 1),
        "the jump lands just past OP_WITHEND"
    );
}

/// Bodies whose GS1 meaning GS2 would change are refused, as are ways out
/// of the block the NPC server's notes warn about. Commands and calls are
/// lowered (both run against the `with` object) unless they reach the
/// player, which GS1 swaps there and GS2 does not.
#[test]
fn gs1_with_bodies_that_change_meaning_stay_errors() {
    for (body, needle) in [
        ("message #a;", "command on the player"),
        ("this.a = next();", "script's own functions"),
        ("sleep 1;", "`sleep`"),
        ("return;", "`return`"),
        ("with (this.p) { i = 1; }", "script variable `i`"),
        ("with (this.p) { i++; }", "script variable `i`"),
        ("this.a = playerx;", "`playerx`"),
        ("player.chat = 1;", "`player.chat`"),
        ("client.a = 1;", "`client.a`"),
        ("this.a = level.b;", "`level.b`"),
    ] {
        let src = format!(
            "function next() {{ return 1; }}\nif (created) {{ with (this.o) {{ {body} }} }}\n"
        );
        let err = compile::compile_gs1(&src, Era::V217)
            .unwrap_err()
            .to_string();
        assert!(err.contains(needle), "{body}: {err}");
    }
    let err = compile::compile_gs1(
        "if (created) { while (1) { with (this.o) { break; } } }\n",
        Era::V217,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("leaves a `with` block"), "{err}");
    // GS2 jumps straight out; the executor closes the block.
    assert!(compile_gs2("function onCreated() { while (1) { with (player) { break; } } }").is_ok());
    // Before 2.1 `with` is not GS1.
    assert!(
        compile::compile_gs1("if (created) { with (this.o) { x = 1; } }\n", Era::V205).is_err()
    );
}

#[test]
fn repeated_event_tests_merge_into_one_function_in_source_order() {
    let merged = compile::compile_gs1(
        "if (created) hide;\nif (playerenters) show;\nif (created) { set a; }\n",
        Era::V205,
    )
    .unwrap();
    let want = compile_gs2(
        "function onCreated() { hide(); set(\"a\"); }\nfunction onPlayerEnters() { show(); }\n",
    )
    .unwrap();
    assert_eq!(merged.functions, want.functions);
    assert_eq!(ops(&merged), ops(&want));
    assert_eq!(merged.write(), want.write());

    let err = compile::compile_gs1(
        "if (created) { if (x) return; hide; }\nif (created) show;\n",
        Era::V217,
    )
    .unwrap_err();
    assert!(err.to_string().contains("not merged"), "{err}");
}

#[test]
fn gs1_join_lowers_to_the_gs2_join_call() {
    assert!(!gs1_commands_without_gs2().contains(&"join"));
    let got = compile::compile_gs1("if (created) { join baddyclass; }\n", Era::V217).unwrap();
    let want = compile_gs2("function onCreated() { join(\"baddyclass\"); }\n").unwrap();
    assert_eq!(ops(&got), ops(&want));
    assert_eq!(got.strings, want.strings);
    assert_eq!(got.write(), want.write());
}

fn fixture_source(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/gs2")
            .join(format!("{name}.gs2")),
    )
    .unwrap()
}

#[test]
fn gs2_front_end_matches_ten_fixtures() {
    for name in [
        "shared", "timers", "strings", "lists", "rest", "objects", "members", "nested", "tiles",
        "elemlist",
    ] {
        let got = compile_gs2(&fixture_source(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let bytes = fixture(&format!("{name}.gs2bc"));
        let want = Script::parse(&bytes).unwrap();
        assert_eq!(got.disassemble(), want.disassemble(), "{name}");
        assert_eq!(got.strings, want.strings, "{name}");
        // `timers.gs2bc` lists its functions in another order; the bytes
        // match everywhere else.
        if name != "timers" {
            assert_eq!(got.write(), bytes, "{name}");
        }
    }
}

/// The two fixtures Seihai does not copy exactly: each differs by one
/// instruction, on purpose (see `switch_stmt` and the if-body call).
#[test]
fn gs2_front_end_differs_from_two_fixtures_by_one_deliberate_instruction() {
    for (name, extra) in [("control", Op::SetIndex), ("calls", Op::IndexDec)] {
        let got = compile_gs2(&fixture_source(name)).unwrap();
        let want = Script::parse(&fixture(&format!("{name}.gs2bc"))).unwrap();
        let (g, w) = (ops(&got), ops(&want));
        assert_eq!(g.len(), w.len() + 1, "{name}");
        let at = g.iter().zip(&w).position(|(a, b)| a != b).unwrap();
        assert_eq!(g[at], extra.to_string(), "{name}");
        assert_eq!(g[..at], w[..at], "{name}");
        assert_eq!(g[at + 1..], w[at..], "{name}");
        assert_eq!(got.strings, want.strings, "{name}");
    }
}

#[test]
fn gs2_switch_default_without_break_leaves_the_switch() {
    let src = "function onCreated() {\n  switch (this.n) {\n    case 1: this.a = 1; break;\n    default: this.a = 2;\n  }\n  this.done = 1;\n}\n";
    let mut m = Machine::new(compile_gs2(src).unwrap());
    let mut h = EffectHost::new(Era::V217, 1);
    assert_eq!(m.run_top_level(&mut h), Outcome::Done);
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    assert_eq!(m.var(Root::This, "a"), Some(&Val::Num(2.0)));
    assert_eq!(m.var(Root::This, "done"), Some(&Val::Num(1.0)));
}

#[test]
fn gs2_corpus_shapes_compile() {
    for src in [
        "public function f(a) { return a; }\nfunction onCreated() this.x = f(1);\n",
        "function onCreated() { temp.s = \"a\" SPC \"b\" NL \"c\" TAB \"d\"; temp.s @= \"e\"; }",
        "function onCreated() { temp.p = findplayer(\"x\").account; findplayer(\"x\").say2(\"hi\"); }",
        "function onCreated() { temp.l = {1, 2}; temp.y = temp.l[1].y; }",
        "function onCreated() { for (temp.v : {1, 2}) echo(temp.v); ++this.n; --this.n; }",
        "function onCreated() { temp.w = new GuiWindowCtrl(\"W\"); temp.k = temp.n in {1, 2}; }",
        "function onCreated() { temp.q = this.n > 0 ? \"y\" : \"n\"; temp.e = \"say \\\"hi\\\"\"; }",
        "this.loaded = 1;\nif (created) setTimer(1);\nfunction onTimeout() { this.k = 1 << 3; }\n",
    ] {
        compile_gs2(src).unwrap_or_else(|e| panic!("{src}\n{e}"));
    }
}

#[test]
fn gs1_top_level_join_and_guard_chains_lower() {
    let got = compile::compile_gs1(
        "join mover;\nif (playertouchsme && this.on == 1 && !isweapon) { hide; }\n",
        Era::V217,
    )
    .unwrap();
    let dis = got.disassemble();
    assert!(dis.starts_with("    0  OP_TYPE_ARRAY"), "{dis}");
    assert!(dis.contains("\"join\""), "{dis}");
    assert_eq!(got.functions[0].name, "onPlayerTouchsMe");
    // A call before the event would run in every GS1 event: not taken.
    let err =
        compile::compile_gs1("if (f() && playertouchsme) { hide; }\n", Era::V217).unwrap_err();
    assert!(err.to_string().contains("too complex"), "{err}");
}

#[test]
fn gs1_message_codes_and_string_commands_lower_and_run() {
    let src = "if (created) {
  this.n = 4;
  setstring this.s,n=#v(this.n+1);
  setstring client.who,#a;
  tokenize a b,c;
  setstring this.t,#t(1)/#e(1,2,hello)/#T(  x  );
  setcharprop #c,hi #s(this.s);
  this.f = strtofloat(#v(2.5));
  this.k = strcontains(#s(this.s),=5);
}
";
    let script = compile::compile_gs1(src, Era::V217).unwrap();
    let mut m = Machine::new(script);
    let mut h = EffectHost::new(Era::V217, 3);
    assert_eq!(m.run_top_level(&mut h), Outcome::Done);
    m.call_event("onCreated", &[], &mut h);
    let s = |m: &Machine, n: &str| match m.var(Root::This, n) {
        Some(Val::Str(s)) => s.clone(),
        other => format!("{other:?}"),
    };
    assert_eq!(s(&m, "s"), "n=5");
    assert_eq!(s(&m, "t"), "b/el/x");
    assert_eq!(s(&m, "chat"), "hi n=5");
    assert_eq!(m.var(Root::This, "f"), Some(&Val::Num(2.5)));
    assert_eq!(m.var(Root::This, "k"), Some(&Val::Num(1.0)));
    // Codes Seihai cannot lower are errors, not literal text.
    let err = compile::compile_gs1("if (created) { message #w; }\n", Era::V217).unwrap_err();
    assert!(err.to_string().contains("#w"), "{err}");
    // A `#` that starts no code stays text.
    compile::compile_gs1("if (created) { message #1 and # ok; }\n", Era::V217).unwrap();
}

#[test]
fn gs2_assigns_to_members_and_elements_of_values() {
    let src = "function onCreated() { temp.l = {1, 2}; temp.l[0] = 5; findplayer(\"a\").chat = \"hi\"; temp.g = new[2][2]; temp.g[1, 1] = 3; }";
    compile_gs2(src).unwrap();
    compile::compile_gs1(
        "if (created) { message #s(clientr.x) #s(serverr.y); }\n",
        Era::V217,
    )
    .unwrap();
}

/// `with (getplayer(acct))` (2.17.1 on) becomes `with (findplayer(acct))`,
/// and commands in the body stay calls by name.
#[test]
fn gs1_with_bodies_keep_commands_and_calls() {
    let s = compile::compile_gs1(
        "function next() { return 1; }\nif (created) { with (getplayer(this.acct)) { hide; setlevel2 a.nw,30,30; this.a = strlen(#s(this.b)); } }\n",
        Era::V217,
    )
    .unwrap();
    assert!(s.strings.iter().any(|t| t == "findplayer"));
    let names = ops(&s);
    let w = names.iter().position(|n| n == "OP_WITH").unwrap();
    let end = names.iter().position(|n| n == "OP_WITHEND").unwrap();
    assert_eq!(
        names[w + 1..end].iter().filter(|n| *n == "OP_CALL").count(),
        2
    );
}

/// #6: the forms the four 2026-11-14 fixtures settle lower to the same
/// bytecode as the GS2 source: `players[i].x` / `.y` / `.dir` and
/// `npcs[i].x` reads (`members.gs2bc`), lists inside literals
/// (`nested.gs2bc`), a list stored into an element (`elemlist.gs2bc`) and
/// `tiles[x, y]` read and written (`tiles.gs2bc`).
#[test]
fn gs1_element_members_nested_lists_and_tiles_match_gs2() {
    for (gs1, gs2) in [
        (
            "this.a = players[0].x; this.b = players[1].y; this.c = players[this.a].dir; \
             this.d = npcs[2].x;",
            "this.a = players[0].x; this.b = players[1].y; this.c = players[this.a].dir; \
             this.d = npcs[2].x;",
        ),
        (
            "this.l = {1, {2, 3}, 4}; this.m = {{\"a\", \"b\"}, {\"c\"}};",
            "this.l = {1, {2, 3}, 4}; this.m = {{\"a\", \"b\"}, {\"c\"}};",
        ),
        ("this.a[0] = {1, 2};", "this.a[0] = {1, 2};"),
        (
            "this.t = tiles[3, 4]; tiles[3, 4] = 5; updateboard 3, 4, 1, 1;",
            "this.t = tiles[3, 4]; tiles[3, 4] = 5; updateboard(3, 4, 1, 1);",
        ),
    ] {
        let got = compile::compile_gs1(&format!("if (created) {{ {gs1} }}"), Era::V217)
            .unwrap_or_else(|e| panic!("{gs1}: {e}"));
        let want = compile_gs2(&format!("function onCreated() {{ {gs2} }}")).unwrap();
        assert_eq!(ops(&got), ops(&want), "{gs1}");
        assert_eq!(got.strings, want.strings, "{gs1}");
    }
}
