//! GS2 forms the corpus surveys found in real 2007–2008 scripts, compiled to
//! the op sequences that the Preagonal compiler (a port of gs2-parser) gives
//! for small hand-written probes. That compiler is used as a black box only:
//! no code is taken from it. Where Seihai departs from its output on purpose,
//! the test says so.

use seihai_core::scripting::gs2::compile::compile_gs2;
use seihai_core::scripting::gs2::Script;

/// The ops of the function body after its four-op header, without the
/// trailing `return 0` and the closing `OP_RET`.
fn body(src: &str) -> Vec<String> {
    let s = compile_gs2(src).unwrap_or_else(|e| panic!("{src}: {e}"));
    let ops = ops(&s);
    let start = ops.iter().position(|o| o == "OP_JMP").unwrap() + 1;
    let start = if ops[start] == "OP_CMD_CALL" {
        start + 1
    } else {
        start
    };
    ops[start..ops.len() - 3].to_vec()
}

fn ops(s: &Script) -> Vec<String> {
    s.code.iter().map(|i| i.op.to_string()).collect()
}

fn f(stmt: &str) -> Vec<String> {
    body(&format!("function onCreated() {{ {stmt} }}"))
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|o| format!("OP_{o}")).collect()
}

#[test]
fn new_block_makes_the_object_runs_the_body_and_adds_the_control() {
    let got = f("new GuiWindowCtrl(\"W\")\n  {\n    x = 10;\n    profile.bitmap = \"a.png\";\n  }");
    assert_eq!(
        got,
        names(&[
            "TYPE_STRING",
            "INLINE_NEW",
            "COPY_LAST_OP",
            "COPY_LAST_OP",
            "COPY_LAST_OP",
            "TYPE_STRING",
            "CONV_TO_STRING",
            "NEW_OBJECT",
            "ASSIGN",
            "CONV_TO_OBJECT",
            "WITH",
            "TYPE_VAR",
            "TYPE_NUMBER",
            "ASSIGN",
            "TYPE_VAR",
            "CONV_TO_OBJECT",
            "TYPE_VAR",
            "MEMBER_ACCESS",
            "TYPE_STRING",
            "ASSIGN",
            "WITHEND",
            "TYPE_ARRAY",
            "SWAP_LAST_OPS",
            "TYPE_VAR",
            "CALL",
            "INDEX_DEC",
        ])
    );
    // Nested blocks and a bare name compile too.
    let nested = f("new GuiWindowCtrl(W) { new GuiButtonCtrl(B) { text = \"ok\"; }; }");
    assert_eq!(nested.iter().filter(|o| *o == "OP_INLINE_NEW").count(), 2);
    assert_eq!(nested[0], "OP_TYPE_VAR");
    // `return` inside a block is refused like inside `with` in GS1, but GS2
    // keeps it: the executor closes the block.
    assert!(compile_gs2("function f() { new GuiX(\"a\") { return 1; } }").is_ok());
}

#[test]
fn dotted_function_names_and_temp_parameters() {
    let s = compile_gs2("function Bar.onAction(temp.a, b) { temp.a = b; }").unwrap();
    assert_eq!(s.functions[0].name, "Bar.onAction");
    // Parameters reversed: `b`, then the place `temp.a`.
    assert_eq!(
        ops(&s)[1..6],
        names(&[
            "TYPE_ARRAY",
            "TYPE_VAR",
            "TEMP",
            "TYPE_VAR",
            "MEMBER_ACCESS"
        ])
    );
}

#[test]
fn chained_and_value_assignments() {
    assert_eq!(
        f("temp.a = temp.b = 42;"),
        names(&[
            "TEMP",
            "TYPE_VAR",
            "MEMBER_ACCESS",
            "TEMP",
            "TYPE_VAR",
            "MEMBER_ACCESS",
            "COPY_LAST_OP",
            "TYPE_NUMBER",
            "ASSIGN",
            "ASSIGN",
        ])
    );
    assert_eq!(
        f("y = (x = 2);"),
        names(&[
            "TYPE_VAR",
            "TYPE_VAR",
            "COPY_LAST_OP",
            "TYPE_NUMBER",
            "ASSIGN",
            "ASSIGN"
        ])
    );
    // `(x += 1)` keeps x for the outer store (Seihai: one more copy than
    // the probe's output, which leaves the outer `OP_ASSIGN` short).
    assert_eq!(
        f("y = (x += 1);"),
        names(&[
            "TYPE_VAR",
            "TYPE_VAR",
            "COPY_LAST_OP",
            "COPY_LAST_OP",
            "CONV_TO_FLOAT",
            "TYPE_NUMBER",
            "ADD",
            "ASSIGN",
            "ASSIGN",
        ])
    );
    assert_eq!(
        f("y = x++;"),
        names(&[
            "TYPE_VAR",
            "TYPE_VAR",
            "COPY_LAST_OP",
            "CONV_TO_FLOAT",
            "SWAP_LAST_OPS",
            "INC",
            "INDEX_DEC",
            "ASSIGN",
        ])
    );
    assert_eq!(
        f("x %= 3;"),
        names(&[
            "TYPE_VAR",
            "COPY_LAST_OP",
            "CONV_TO_FLOAT",
            "TYPE_NUMBER",
            "MOD",
            "ASSIGN"
        ])
    );
}

#[test]
fn operators_names_and_constants() {
    assert_eq!(
        f("y = 5 & 3;"),
        names(&["TYPE_VAR", "TYPE_NUMBER", "TYPE_NUMBER", "BWA", "ASSIGN"])
    );
    assert_eq!(
        f("y = 5 | 3;"),
        names(&["TYPE_VAR", "TYPE_NUMBER", "TYPE_NUMBER", "BWO", "ASSIGN"])
    );
    // `in |a, b|` still closes its range.
    assert!(compile_gs2("function f() { if (x in |1, 3|) y = 1; }").is_ok());
    assert_eq!(
        f("if (x => 0) y = 1;"),
        names(&[
            "TYPE_VAR",
            "CONV_TO_FLOAT",
            "TYPE_NUMBER",
            "GTE",
            "IF",
            "TYPE_VAR",
            "TYPE_NUMBER",
            "ASSIGN",
        ])
    );
    let s = compile_gs2("function f() { y = $pref::Video::style; }").unwrap();
    assert!(s.strings.iter().any(|t| t == "$pref::Video::style"));
    // `const` folds and leaves no code at top level.
    let s = compile_gs2("const C = 1; y = C;").unwrap();
    assert_eq!(
        ops(&s),
        names(&["TYPE_VAR", "TYPE_NUMBER", "ASSIGN", "RET"])
    );
    // Seihai folds `enum` members too (the probe reads them as variables).
    let s = compile_gs2("enum Dir { UP, DOWN = 5, LEFT };\ny = LEFT + Dir::DOWN;").unwrap();
    assert!(s.strings.iter().all(|t| t != "LEFT" && t != "Dir::DOWN"));
}

#[test]
fn member_forms() {
    assert_eq!(
        f("(@x).y = 1;"),
        names(&[
            "TYPE_VAR",
            "CONV_TO_STRING",
            "CONV_TO_OBJECT",
            "TYPE_VAR",
            "MEMBER_ACCESS",
            "TYPE_NUMBER",
            "ASSIGN",
        ])
    );
    // `obj.(n)`: Seihai's form, the name as a string (the probe adds it).
    assert_eq!(
        f("obj.(n) = 2;"),
        names(&[
            "TYPE_VAR",
            "CONV_TO_OBJECT",
            "TYPE_VAR",
            "CONV_TO_STRING",
            "MEMBER_ACCESS",
            "TYPE_NUMBER",
            "ASSIGN",
        ])
    );
    let got = f("makevar(\"a.b\") = 1;");
    assert_eq!(
        got[got.len() - 3..],
        names(&["MAKEVAR", "TYPE_NUMBER", "ASSIGN"])
    );
    // A method on a dotted name or a call result converts it first.
    assert_eq!(
        f("a.b.go();"),
        names(&[
            "TYPE_ARRAY",
            "TYPE_VAR",
            "CONV_TO_OBJECT",
            "TYPE_VAR",
            "MEMBER_ACCESS",
            "CONV_TO_OBJECT",
            "TYPE_VAR",
            "MEMBER_ACCESS",
            "CALL",
            "INDEX_DEC",
        ])
    );
    assert_eq!(
        f("tiles[1, 2] = 3;"),
        names(&[
            "TYPE_VAR",
            "CONV_TO_OBJECT",
            "TYPE_NUMBER",
            "TYPE_NUMBER",
            "TYPE_NUMBER",
            "ARRAY_MULTIDIM_ASSIGN",
        ])
    );
}

#[test]
fn elseif_and_with_exits() {
    assert_eq!(
        f("if (a) x = 1; elseif (b) x = 2;"),
        f("if (a) x = 1; else if (b) x = 2;")
    );
    assert_eq!(
        f("with (player) { return 1; }"),
        names(&[
            "PLAYER",
            "CONV_TO_OBJECT",
            "WITH",
            "TYPE_NUMBER",
            "RET",
            "WITHEND"
        ])
    );
    assert_eq!(
        f("for (i = 0; i < 2; i++) { with (player) { break; } }")[9..14],
        names(&["PLAYER", "CONV_TO_OBJECT", "WITH", "SET_INDEX", "WITHEND"])
    );
}
