//! GS2 forms the corpus surveys found in real 2007–2008 scripts, compiled to
//! the op sequences that the Preagonal compiler (a port of gs2-parser) gives
//! for small hand-written probes. That compiler is used as a black box only:
//! no code is taken from it. Where Seihai departs from its output on purpose,
//! the test says so.

use seihai_core::scripting::gs2::compile::{self, CompileErrors};
use seihai_core::scripting::gs2::Script;

/// Legacy mode: these are forms live scripts use.
fn compile_gs2(src: &str) -> Result<Script, CompileErrors> {
    compile::compile_gs2_legacy(src).map(|c| c.script)
}

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

#[test]
fn gs1_style_commands_inside_gs2() {
    let got = f("setimgpart door.png,0,0,32,32;\n  hide;\n  x = 1;");
    assert_eq!(
        got[..9],
        names(&[
            "TYPE_ARRAY",
            "TYPE_NUMBER",
            "TYPE_NUMBER",
            "TYPE_NUMBER",
            "TYPE_NUMBER",
            "TYPE_STRING",
            "TYPE_VAR",
            "CALL",
            "INDEX_DEC",
        ])
    );
    let s = compile_gs2("function onCreated() { setimgpart door.png,0,0,32,32; }").unwrap();
    assert!(s.strings.iter().any(|t| t == "door.png"));
    // GS1 message codes in the arguments are lowered as in GS1.
    let s = compile_gs2("function onCreated() { setcharprop #c,hi #a; }").unwrap();
    assert!(ops(&s).contains(&"OP_PLAYER".to_string()));
    // A GS1 name used GS2-style stays GS2.
    assert_eq!(
        f("hide();"),
        names(&["TYPE_ARRAY", "TYPE_VAR", "CALL", "INDEX_DEC"])
    );
}

#[test]
fn leniency_the_live_servers_had() {
    // A missing `;` at the end of a line or before `}`.
    assert_eq!(f("x = 1\n  y = 2\n"), f("x = 1; y = 2;"));
    assert_eq!(
        f("echo(1) }\nfunction g() {"),
        f("echo(1); }\nfunction g() {")
    );
    assert!(compile_gs2("function f() { x = 1 y = 2; }").is_err());
    // `=` compares inside an `if` condition.
    assert_eq!(f("if (a = \"m\") x = 1;"), f("if (a == \"m\") x = 1;"));
    // A later function of the same name replaces the earlier one.
    let s = compile_gs2("function f() { x = 1; }\nfunction f() { y = 2; }").unwrap();
    assert_eq!(s.functions.len(), 1);
    assert!(s.strings.iter().any(|t| t == "y") && !s.strings.iter().any(|t| t == "x"));
    // Declarations and functions without a body.
    assert!(compile_gs2("function a();\nfunction b()\nfunction c() { return 2; }").is_ok());
    // Text beyond latin1.
    assert!(compile_gs2("function f() { x = \"\u{263A}\"; }").is_ok());
}

#[test]
fn lambdas_steps_and_dynamic_calls() {
    let s = compile_gs2("function onCreated() { temp.f = function() { return 1; }; }").unwrap();
    assert_eq!(
        ops(&s)[4..18],
        names(&[
            "TEMP",
            "TYPE_VAR",
            "MEMBER_ACCESS",
            "SET_INDEX",
            "TYPE_ARRAY",
            "FUNC_PARAMS_END",
            "JMP",
            "TYPE_NUMBER",
            "RET",
            "THIS",
            "TYPE_VAR",
            "MEMBER_ACCESS",
            "CONV_TO_OBJECT",
            "ASSIGN",
        ])
    );
    assert!(s
        .functions
        .iter()
        .any(|f| f.name.starts_with("public.function_")));
    assert_eq!(
        f("a[1][2]--;"),
        names(&[
            "TYPE_VAR",
            "CONV_TO_OBJECT",
            "TYPE_NUMBER",
            "ARRAY",
            "TYPE_NUMBER",
            "ARRAY",
            "DEC",
            "INDEX_DEC",
        ])
    );
    assert!(compile_gs2("function f() { for (i = 0; i < 4; ++i) showpoly(++this.n, x); }").is_ok());
    assert!(compile_gs2("function f() { obj.(@\"add\" @ n)(1, 2); a.b.(n)(); }").is_ok());
    assert!(
        compile_gs2("function f() { this.n = makevar(\"a\" @ i) += 2; a[i][j] -= 1; }").is_ok()
    );
    // Integers past 32 bits stay exact.
    assert!(compile_gs2("function f() { x = 98765432109876; }")
        .unwrap()
        .disassemble()
        .contains("98765432109876"));
}

#[test]
fn more_forms_from_the_corpus_shapes() {
    for stmt in [
        "f(1).x = f(2).x = 3;",
        "f(1).x -= 2;",
        "a[i] = b = 3;",
        "temp.a[i][2] += 1;",
        "if (\"a\" in f(\"b\")) x = 1;",
        "a.x = 1, b.x = 2;",
        "this.x == \"a\";",
    ] {
        let src = format!("function g() {{ {stmt} }}");
        assert!(
            compile_gs2(&src).is_ok(),
            "{src}: {:?}",
            compile_gs2(&src).err()
        );
    }
    // `x in f(…)`: the value, `OP_CONV_TO_OBJECT`, `OP_IN_OBJ`.
    let got = f("y = \"a\" in f(\"b\");");
    assert_eq!(
        got[got.len() - 3..],
        names(&["CONV_TO_OBJECT", "IN_OBJ", "ASSIGN"])
    );
    // Enum members may be text; `global enum` too.
    let s = compile_gs2("global enum E { A = \"x\", B };\nfunction g() { y = E::A @ B; }").unwrap();
    assert!(s.strings.iter().any(|t| t == "x"));
    // A declaration followed by its body, and a block left open at the end.
    assert!(compile_gs2("function g();\n{ x = 1; }").is_ok());
    assert!(compile_gs2("function g() {\n x = 1;\n").is_ok());
}

#[test]
fn compile_script_picks_the_front_end() {
    use seihai_core::scripting::engine::Era;
    use seihai_core::scripting::gs2::compile::{compile, Front, Language, Mode};
    let compile_script = |src: &str, era: Era| {
        compile(src, Language::Detect(era), Mode::Legacy).map(|c| (c.script, c.front))
    };
    let v21 = Era::V217;
    let gs1 = "if (playerenters) { setimgpart door.png,0,0,32,32; message hi; }";
    assert_eq!(compile_script(gs1, v21).unwrap().1, Front::Gs1);
    let gs2 = "function onCreated() { setimgpart door.png,0,0,32,32; }";
    assert_eq!(compile_script(gs2, v21).unwrap().1, Front::Gs2);
    // Only the GS1 front end takes this one.
    assert_eq!(
        compile_script("if (created) { tokenize #c; }", v21)
            .unwrap()
            .1,
        Front::Gs1
    );
    // The GS1 front end reads the era it is given: `#I(…)` lowers before 2.1.
    let old = "if (playerenters) { message #I(x); }";
    assert!(compile_script(old, v21).is_err());
    assert_eq!(compile_script(old, Era::V205).unwrap().1, Front::Gs1);
}

#[test]
fn gs1_codes_and_ranges_inside_gs2() {
    // `#a` as a call argument reads the player's account, as GS1 does.
    let got = f("showtext(1, #a);");
    assert!(got.contains(&"OP_PLAYER".to_string()), "{got:?}");
    assert!(compile_gs2("function g() { x = #v(this.y) @ #c; }").is_ok());
    // GS1's `in <a, b>` is the same range as `in |a, b|`.
    assert_eq!(f("if (x in <1, 3>) y = 1;"), f("if (x in |1, 3|) y = 1;"));
}

#[test]
fn v5_corpus_shapes() {
    for stmt in [
        "(@\"go\" @ n)(1, 2);",
        "x = (@\"go\")(1);",
        "switch (x) { case 1: (@\"go\")(); break; }",
        "return x = 1;",
        "f(1).text @= \"a\";",
        "a[f(1)] += 2;",
    ] {
        let src = format!("function g() {{ {stmt} }}");
        assert!(
            compile_gs2(&src).is_ok(),
            "{src}: {:?}",
            compile_gs2(&src).err()
        );
    }
    assert!(compile_gs2(
        "function g(\"a\", b) { x = a; };\n;\nconst K = 2 * 3;\nfunction h() { y = K; }"
    )
    .is_ok());
    // `a[f()] += v` calls f once: the element read as a place, then stored.
    let got = f("a[f(1)] += 2;");
    assert_eq!(got.iter().filter(|o| *o == "OP_CALL").count(), 1);
    assert_eq!(
        got[got.len() - 6..],
        names(&[
            "ARRAY",
            "COPY_LAST_OP",
            "CONV_TO_FLOAT",
            "TYPE_NUMBER",
            "ADD",
            "ASSIGN"
        ])
    );
}

#[test]
fn v6_shapes_compile() {
    for src in [
        // An empty statement after a block inside a `case`.
        "function f() { switch (x) { case 1: { y = 1; } ; break; } }",
        // Calling what a call returned.
        "function f() { with (a) g(\"x\" @ b.c())(); }",
        // A run-time name as the target, and a member named at run time.
        "function f() { (@\"a\" @ temp.b) = new TStaticVar(); this.(@\"a\" @ temp.b) = 1; }",
        // A function inside a GS1-style event test.
        "if (created) {\n function f() { x = 1; }\n}",
        // `/word` as a call argument, next to a GS1 code.
        "function f() { if (g(#c(1), /word) || h()) x = 1; }",
    ] {
        if let Err(e) = compile_gs2(src) {
            panic!("{src}: {e:?}");
        }
    }
    // An unclosed nested comment ends at its first `*/`.
    assert!(compile_gs2("/* a /* b */\nfunction f() { x = 1; }").is_ok());
    // `&=` is the place, its value, the operand, `OP_BWA`, assign.
    assert_eq!(
        f("this.a &= 1;")[..7],
        names(&[
            "THIS",
            "TYPE_VAR",
            "MEMBER_ACCESS",
            "COPY_LAST_OP",
            "CONV_TO_FLOAT",
            "TYPE_NUMBER",
            "BWA"
        ])
    );
}
