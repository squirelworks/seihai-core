//! Generates GS1 script fuzz cases for a live client lab, each with what
//! Seihai's GS1 engine predicts the client will report.
//!
//! ```text
//! cargo run -p seihai-core --no-default-features --example gs1_fuzzcases -- \
//!     --seed 1 --count 300 --first-id 40000 > gs1cases.tsv
//! ```
//!
//! GS1 is never sent as bytecode: the 1.x and 2.x clients get the script
//! as text (NPC prop 1) and build their own tree from it. So these cases
//! are script text. Each is one line (no newline to escape) of the form
//!
//! ```text
//! if (created) { setplayerprop #c,FZ<id>a; timeout = 0.3; }
//! if (timeout) { <setup>; setplayerprop #c,FZ<id>:<report>; }
//! ```
//!
//! A client that runs it sends both chat values back to the server as
//! player props; a lab proxy logs them.
//!
//! Families (`--only FAMILY` keeps one):
//!
//! - `expr`: `#v(...)` of a random numeric expression over edge values.
//! - `code`: message codes (`#s`, `#e`, `#t`, `#v`, `#L`, `#a`, `#n`, ...)
//!   with ordinary and odd arguments, nested and unterminated.
//! - `cond`: string and number tests (`strequals`, `startswith`,
//!   `strcontains`, `in`, `<`, `==` between text and numbers).
//! - `limit`: `while` and `for` loops at and around the loop limit.
//! - `text`: damaged script text (unbalanced braces, missing `;`,
//!   unknown commands, odd numbers, high bytes, comment forms).
//!
//! Output is a TSV: `id`, `family`, `what`, the script text, then one
//! prediction column per vocabulary (`2.05`, `2.17`): the chat values in
//! order, joined with ` | `, then `; ` and the parse diagnostics count.

use seihai_core::scripting::engine::world::{Effect, ObjSpec, ScriptWorld};
use seihai_core::scripting::engine::Era;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
}

const NUMS: &[&str] = &[
    "0",
    "1",
    "-1",
    "2",
    "3",
    "7",
    "10",
    "0.5",
    "-0.5",
    "0.0001",
    "0.00009",
    "0.1",
    "1.5",
    "-1.5",
    "2.5",
    "31",
    "32",
    "33",
    "255",
    "256",
    "65535",
    "65536",
    "2147483647",
    "2147483648",
    "-2147483649",
    "4294967296",
    "123456789.123456789",
    "0.000000001",
    "99999999999",
    "1e3",
    "3.14159",
];

const TEXTS: &[&str] = &[
    "", "a", "abc", "12", " 12", "12abc", "-3.5", "a b c", "ABC", "x,y",
];

const ERAS: [(&str, Era); 2] = [("2.05", Era::V205), ("2.17", Era::V217)];

fn nexpr(r: &mut Rng, depth: u32) -> String {
    if depth == 0 || r.chance(25) {
        return r.pick(NUMS).to_string();
    }
    let d = depth - 1;
    match r.below(6) {
        0..=2 => {
            let op = *r.pick(&["+", "-", "*", "/", "%", "^"]);
            format!("({} {op} {})", nexpr(r, d), nexpr(r, d))
        }
        3 => {
            let f = *r.pick(&["abs", "int", "sin", "cos", "arctan"]);
            format!("{f}({})", nexpr(r, d))
        }
        4 => {
            let f = *r.pick(&["strlen", "strtofloat", "ascii"]);
            format!("{f}({})", r.pick(TEXTS))
        }
        _ => format!("-{}", nexpr(r, d)),
    }
}

fn template(id: u32, setup: &str, report: &str) -> String {
    format!(
        "if (created) {{ setplayerprop #c,FZ{id}a; timeout = 0.3; }} \
         if (timeout) {{ {setup} setplayerprop #c,FZ{id}:{report}; }}"
    )
}

fn predict(src: &str, era: Era) -> String {
    let mut w = ScriptWorld::new(era);
    let id = w.add(ObjSpec::npc("blank.png", 30.0, 30.0, src));
    let diags = w.obj(id).map_or(0, |o| o.diagnostics().len());
    w.fire(id, "created", &[]);
    let mut chats = Vec::new();
    let mut take = |w: &mut ScriptWorld| {
        for e in w.take_effects() {
            if let Effect::SetPlayerProp { code, value } = e {
                if code.trim() == "#c" {
                    chats.push(clean(&value));
                }
            }
        }
    };
    take(&mut w);
    for _ in 0..20 {
        w.tick(50);
        take(&mut w);
    }
    format!("{}; diags {diags}", chats.join(" | "))
}

fn clean(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c == '\t' || c == '\n' || c == '\r' {
                ' '
            } else {
                c
            }
        })
        .collect()
}

struct Case {
    family: &'static str,
    what: String,
    src: String,
}

fn main() {
    let mut seed = 1u64;
    let mut count = 200usize;
    let mut first = 40000u32;
    let mut only: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().expect("value after flag");
        match a.as_str() {
            "--seed" => seed = val().parse().unwrap(),
            "--count" => count = val().parse().unwrap(),
            "--first-id" => first = val().parse().unwrap(),
            "--only" => only = Some(val()),
            other => panic!("unknown flag {other}"),
        }
    }
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let want = |f: &str| only.as_deref().is_none_or(|o| o == f);
    let mut id = first;
    let mut cases: Vec<Case> = Vec::new();
    let mut add = |family: &'static str, what: String, src: String, id: &mut u32| {
        cases.push(Case { family, what, src });
        *id += 1;
    };

    if want("code") {
        let codes: Vec<String> = vec![
            "#v(1/3)".into(),
            "#v(-0.00001)".into(),
            "#v(2147483648)".into(),
            "#v(1e3)".into(),
            "#v()".into(),
            "#v(".into(),
            "#v(1".into(),
            "#v(#v(2))".into(),
            "#v(abc)".into(),
            "#s(fzs)".into(),
            "#s()".into(),
            "#s(".into(),
            "#s(nosuch)".into(),
            "#e(0,2,abcdef)".into(),
            "#e(2,-1,abcdef)".into(),
            "#e(-1,2,abcdef)".into(),
            "#e(4,9,abcdef)".into(),
            "#e(0,0,)".into(),
            "#e(1,2)".into(),
            "#e(".into(),
            "#t(0)".into(),
            "#t(1)".into(),
            "#t(-1)".into(),
            "#t(99)".into(),
            "#L".into(),
            "#a".into(),
            "#n".into(),
            "#c".into(),
            "#g".into(),
            "#f".into(),
            "#m".into(),
            "#w".into(),
            "#1".into(),
            "#2".into(),
            "#3".into(),
            "#8".into(),
            "#C0".into(),
            "#C1".into(),
            "#C2".into(),
            "#P1".into(),
            "#P1(-1)".into(),
            "#K(0)".into(),
            "#K(17)".into(),
            "#K(-1)".into(),
            "#k(0)".into(),
            "#W".into(),
            "#R".into(),
            "#S".into(),
            "#Z".into(),
            "#".into(),
            "##".into(),
            "#%".into(),
            "#v(1)#v(2)".into(),
            "#e(0,1,#s(fzs))".into(),
            "#I(fzs,0)".into(),
            "#T(  x  )".into(),
            "#D".into(),
            "#p(0)".into(),
            "#p(9)".into(),
        ];
        for c in codes {
            let i = id;
            add(
                "code",
                c.clone(),
                template(i, "setstring fzs,hello world; tokenize a b,c;", &c),
                &mut id,
            );
        }
    }

    if want("cond") {
        let tests = [
            "strequals(abc,ABC)",
            "strequals(#s(fzs),hello)",
            "strequals(,)",
            "startswith(he,#s(fzs))",
            "startswith(,abc)",
            "strcontains(#s(fzs),ll)",
            "strcontains(abc,)",
            "indexof(l,#s(fzs))",
            "indexof(,abc)",
            "lindexof(b,a,b,c)",
            "1 == 1.00001",
            "1 == 1.0001",
            "1 == 1.001",
            "0.1+0.2 == 0.3",
            "1 < 1.00001",
            "2 > 10",
            "#s(fzs) == hello",
            "12 == 12.0",
            "-0 == 0",
            "1 in |0,1|",
            "1 in |1,0|",
            "1.5 in |1,2|",
            "3 in <1,3>",
            "!(0)",
            "!(1) || 1",
            "1 && 0 || 1",
            "(1",
        ];
        for t in tests {
            let i = id;
            add(
                "cond",
                t.to_string(),
                template(
                    i,
                    &format!("setstring fzs,hello; if ({t}) fzr = 1; else fzr = 0;"),
                    "#v(fzr)",
                ),
                &mut id,
            );
        }
    }

    if want("limit") {
        for n in [9999, 10000, 10001, 20000] {
            let i = id;
            add(
                "limit",
                format!("while to {n}"),
                template(i, &format!("fzr = 0; while (fzr < {n}) fzr++;"), "#v(fzr)"),
                &mut id,
            );
            let i = id;
            add(
                "limit",
                format!("for to {n}"),
                template(
                    i,
                    &format!("for (fzr = 0; fzr < {n}; fzr++) {{}}"),
                    "#v(fzr)",
                ),
                &mut id,
            );
        }
        for n in [100, 101, 1000] {
            let i = id;
            add(
                "limit",
                format!("nested 100 x {n}"),
                template(
                    i,
                    &format!("fzr = 0; for (fzi = 0; fzi < 100; fzi++) {{ for (fzj = 0; fzj < {n}; fzj++) fzr++; }}"),
                    "#v(fzr)",
                ),
                &mut id,
            );
        }
    }

    if want("text") {
        let bodies = [
            ("missing semicolon", "fzr = 5 setplayerprop #c,x;"),
            ("extra close brace", "fzr = 5; }"),
            ("unknown command", "nosuchcommand 1,2; fzr = 5;"),
            ("command with no args", "setplayerprop; fzr = 5;"),
            ("number 1.2.3", "fzr = 1.2.3;"),
            ("number with letters", "fzr = 12abc;"),
            ("double operator", "fzr = 2 ++ 3;"),
            ("empty parens", "fzr = ();"),
            ("line comment", "fzr = 5; // fzr = 6;"),
            ("block comment", "fzr = 5; /* fzr = 6; */"),
            ("high bytes in a string", "setstring fzs,\u{e4}\u{f6}\u{fc}\u{a7}; fzr = strlen(#s(fzs));"),
            ("tab inside", "fzr\t=\t5;"),
            ("assignment to a number", "5 = fzr; fzr = 7;"),
            ("this.var", "this.fzr = 4; fzr = this.fzr;"),
            ("string var in #v", "setstring fzr,9; fzr = #v(#s(fzr));"),
            ("very long identifier", "fzrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrr = 3; fzr = 3;"),
            ("if without braces else if", "if (0) fzr = 1; else if (1) fzr = 2; else fzr = 3;"),
            ("with no target", "with () { fzr = 4; }"),
            ("unterminated string arg", "setstring fzs,\"abc; fzr = 5;"),
            ("semicolon storm", ";;;;; fzr = 5;;;;"),
        ];
        for (what, body) in bodies {
            let i = id;
            add(
                "text",
                what.to_string(),
                template(i, body, "#v(fzr)"),
                &mut id,
            );
        }
    }

    if want("expr") {
        for _ in 0..count {
            let depth = 1 + r.below(3) as u32;
            let e = nexpr(&mut r, depth);
            let i = id;
            add(
                "expr",
                e.clone(),
                template(i, "", &format!("#v({e})")),
                &mut id,
            );
        }
    }

    println!(
        "id\tfamily\twhat\tscript\t{}",
        ERAS.iter().map(|e| e.0).collect::<Vec<_>>().join("\t")
    );
    for (n, c) in cases.iter().enumerate() {
        let preds: Vec<String> = ERAS.iter().map(|(_, e)| predict(&c.src, *e)).collect();
        println!(
            "{}\t{}\t{}\t{}\t{}",
            first + n as u32,
            c.family,
            clean(&c.what),
            clean(&c.src),
            preds.join("\t")
        );
    }
}
