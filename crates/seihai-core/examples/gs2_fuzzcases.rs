//! Generates GS2 bytecode fuzz cases for a live client lab, each with what
//! Seihai's own VM predicts the client will report.
//!
//! ```text
//! cargo run -p seihai-core --no-default-features --example gs2_fuzzcases -- \
//!     --seed 1 --count 400 --first-id 30000 > cases.tsv
//! ```
//!
//! Every case is one NPC script whose `onCreated` sets `player.chat` to
//! `FZ<id>a`, sleeps 0.2 s, computes one value and sets `player.chat` to
//! `FZ<id>:<value>`. A client that runs the script sends both chat values
//! back to the server as player props, which a lab proxy logs. The first
//! value shows the script loaded and started; the second shows how far
//! the fuzzed code got and what it computed. A lab script injects each
//! case as `PLO_NPCBYTECODE` and compares the two chat values with the
//! prediction columns.
//!
//! Families (`--only FAMILY` keeps one):
//!
//! - `expr`: a random expression, written as GS2 source and compiled by
//!   Seihai's compiler, over edge values (0, tolerance boundaries, the
//!   32-bit limits, odd strings). Checks the arithmetic, text and
//!   comparison rules each VM profile claims.
//! - `op`: hand-made instruction sequences spliced in where the value is
//!   computed, for the opcodes whose effect is not settled (45-49, 54,
//!   66, 67, 104, 121, 164, `waitfor`), for every byte no compiler emits
//!   (with and without an operand), and for the load-time fold bytes
//!   (200-242) written straight into the stream.
//! - `limit`: stack depth, loop passes, `maxlooplimit` and call depth at
//!   and around each profile's limit.
//! - `container`: byte-level damage to a good case (segments dropped,
//!   reordered, unknown or oversized; string and jump references out of
//!   range; operand markers F0-FF in odd places; trailing bytes).
//! - `numtext`: fixed texts near zero (`"0"`, `"00"`, `" 0"`, `"0abc"`,
//!   `"abc"`, `""` and others) through `+ 0`, `!`, `== 0` and `== -1`,
//!   to pin down which texts the clients read as -1 instead of 0. Comes
//!   last, so the ids of the other families do not move.
//!
//! Output is a TSV with one case per line: `id`, `family`, `what`, the
//! bytecode as hex (without the compiler's trailing newline), then one
//! prediction column per VM profile (`4.20r8`, `5.24r4`, `6.03r7`). A
//! prediction is the chat values in order, joined with ` | `, then
//! `; ` and how the run ended (`done`, `stopped: ...`, `parse: ...`).

use std::fmt::Write as _;

use seihai_core::scripting::gs2::compile::compile_gs2;
use seihai_core::scripting::gs2::exec::{Host, Machine, Outcome, Root};
use seihai_core::scripting::gs2::vm::{VmProfile, V420R8, V524R4, V6};
use seihai_core::scripting::gs2::{Instr, Op, Operand, Script};

/// The marker number the template computes; the splice replaces it.
const MARK: f64 = 777777.0;

const PROFILES: [&VmProfile; 3] = [&V420R8, &V524R4, &V6];

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

/// Numbers around every boundary the notes name: the 0.0001 tolerance,
/// the 32-bit integer limits, the number-to-text cut-offs.
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
    "0.00011",
    "-0.0001",
    "0.99995",
    "1.99999",
    "2.00001",
    "0.1",
    "0.3",
    "1.5",
    "-1.5",
    "2.5",
    "31",
    "32",
    "33",
    "63",
    "64",
    "-32",
    "255",
    "256",
    "65535",
    "65536",
    "2147483647",
    "2147483648",
    "-2147483648",
    "-2147483649",
    "4294967296",
    "1000000000",
    "123456789.123456789",
    "0.000000001",
    "0.123456789999",
    "1e10",
    "99999999999",
    "3.14159265358979",
];

const STRS: &[&str] = &[
    "\"\"",
    "\"a\"",
    "\"abc\"",
    "\"12\"",
    "\" 12\"",
    "\"12abc\"",
    "\"1e3\"",
    "\"0x10\"",
    "\"-3.5\"",
    "\"a,b,c\"",
    "\"a, b ,c\"",
    "\"\\\"q\\\"\"",
    "\"  pad  \"",
    "\"1.00000\"",
    "\"abcdefghij\"",
];

/// A random GS2 expression of at most `depth` levels.
fn expr(r: &mut Rng, depth: u32) -> String {
    if depth == 0 || r.chance(25) {
        return if r.chance(70) {
            r.pick(NUMS).to_string()
        } else {
            r.pick(STRS).to_string()
        };
    }
    let d = depth - 1;
    match r.below(9) {
        0..=2 => {
            let op = *r.pick(&[
                "+", "-", "*", "/", "%", "^", "<<", ">>", "&", "|", "==", "!=", "<", ">", "<=",
                ">=", "&&", "||", "@",
            ]);
            format!("({} {} {})", expr(r, d), op, expr(r, d))
        }
        3 => {
            let f = *r.pick(&["int", "abs", "sin", "cos", "exp", "log", "char"]);
            format!("{f}({})", expr(r, d))
        }
        4 => {
            let f = *r.pick(&["min", "max", "arctan", "getangle", "getdir", "vecx", "vecy"]);
            match f {
                "arctan" | "vecx" | "vecy" => format!("{f}({})", expr(r, d)),
                _ => format!("{f}({}, {})", expr(r, d), expr(r, d)),
            }
        }
        5 => {
            let s = if r.chance(60) {
                r.pick(STRS).to_string()
            } else {
                expr(r, d)
            };
            match r.below(7) {
                0 => format!("({s}).length()"),
                1 => format!("({s}).pos({})", r.pick(STRS)),
                2 => format!("({s}).substring({}, {})", expr(r, 0), expr(r, 0)),
                3 => format!("({s}).starts({})", r.pick(STRS)),
                4 => format!("({s}).ends({})", r.pick(STRS)),
                5 => format!("({s}).charat({})", expr(r, 0)),
                _ => format!("({s}).tokenize().size()"),
            }
        }
        6 => {
            let f = *r.pick(&[
                "%d", "%f", "%s", "%x", "%5.2f", "%i", "%c", "%%", "%03d", "%e",
            ]);
            format!("format(\"<{f}>\", {})", expr(r, d))
        }
        7 => format!("({} in |{}, {}|)", expr(r, d), expr(r, 0), expr(r, 0)),
        _ => {
            let u = *r.pick(&["-", "!"]);
            format!("{u}({})", expr(r, d))
        }
    }
}

/// The case template, with the value to compute in place of `MARK`.
fn template(id: u32, value: &str) -> String {
    format!(
        "function onCreated() {{ player.chat = \"FZ{id}a\"; sleep(0.2); temp.r = {value}; \
         player.chat = \"FZ{id}:\" @ temp.r; }}"
    )
}

/// Replaces the one `OP_TYPE_NUMBER MARK` in `s` with `seq`, moving every
/// jump target and function start after it.
fn splice(s: &mut Script, seq: Vec<Instr>) {
    let at = s
        .code
        .iter()
        .position(|i| i.number() == Some(MARK))
        .expect("template has the marker");
    let grow = seq.len() as i32 - 1;
    for ins in s.code.iter_mut() {
        if let (true, Operand::Int(t)) = (ins.op.jumps(), &mut ins.operand) {
            if *t > at as i32 {
                *t += grow;
            }
        }
    }
    for f in s.functions.iter_mut() {
        if f.start > at {
            f.start = (f.start as i32 + grow) as usize;
        }
    }
    s.code.splice(at..=at, seq);
}

fn ins(op: u8, operand: Operand) -> Instr {
    Instr {
        op: Op::from_u8(op),
        operand,
        offset: 0,
    }
}
fn num(n: i32) -> Instr {
    ins(20, Operand::Int(n))
}
fn bare(op: u8) -> Instr {
    ins(op, Operand::None)
}

/// A string-table index for `text`, adding it if needed.
fn string(s: &mut Script, text: &str) -> u32 {
    if let Some(i) = s.strings.iter().position(|t| t == text) {
        return i as u32;
    }
    s.strings.push(text.to_string());
    (s.strings.len() - 1) as u32
}

/// Pushes one value onto the stack, adding any string it needs.
type Push = fn(&mut Script) -> Instr;

struct Case {
    id: u32,
    family: &'static str,
    what: String,
    bytes: Vec<u8>,
}

/// Records `player.chat` writes; every command call answers nothing.
#[derive(Default)]
struct ChatHost {
    chats: Vec<String>,
}

impl Host for ChatHost {
    fn command(
        &mut self,
        _name: &str,
        _args: &[seihai_core::scripting::engine::Val],
    ) -> Option<seihai_core::scripting::engine::Val> {
        None
    }
    fn set_prop(
        &mut self,
        root: Root,
        name: &str,
        value: &seihai_core::scripting::engine::Val,
    ) -> bool {
        if root == Root::Player && name.eq_ignore_ascii_case("chat") {
            self.chats.push(value.text());
            return true;
        }
        false
    }
    fn log(&mut self, _line: String) {}
}

fn predict(bytes: &[u8], profile: &'static VmProfile) -> String {
    let script = match Script::parse(bytes) {
        Ok(s) => s,
        Err(e) => return format!("; parse: {}", clean(&e)),
    };
    let mut m = Machine::with_profile(script, profile);
    let mut host = ChatHost::default();
    let mut out = m.call_event("onCreated", &[], &mut host);
    let mut guard = 0;
    loop {
        match out {
            Outcome::Sleep(_) if guard < 50 => {
                guard += 1;
                out = m.resume(&mut host);
            }
            Outcome::WaitFor { .. } if guard < 50 => {
                guard += 1;
                out = m.resume_waitfor(false, &mut host);
            }
            _ => break,
        }
    }
    let end = match out {
        Outcome::Done => "done".to_string(),
        Outcome::Stopped(why) => format!("stopped: {}", clean(&why)),
        other => clean(&format!("{other:?}")),
    };
    format!(
        "{}; {end}",
        host.chats
            .iter()
            .map(|c| clean(c))
            .collect::<Vec<_>>()
            .join(" | ")
    )
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

fn base(id: u32) -> Script {
    compile_gs2(&template(id, "777777")).expect("template compiles")
}

fn spliced(id: u32, f: impl FnOnce(&mut Script) -> Vec<Instr>) -> Vec<u8> {
    let mut s = base(id);
    let seq = f(&mut s);
    splice(&mut s, seq);
    s.write()
}

fn expr_cases(r: &mut Rng, id: &mut u32, n: usize, out: &mut Vec<Case>) {
    let mut made = 0;
    while made < n {
        let depth = 1 + r.below(3) as u32;
        let e = expr(r, depth);
        let Ok(s) = compile_gs2(&template(*id, &e)) else {
            continue;
        };
        out.push(Case {
            id: *id,
            family: "expr",
            what: e,
            bytes: s.write(),
        });
        *id += 1;
        made += 1;
    }
}

/// Bytes the 4.20r8 name table or a dispatcher knows, and the gaps.
fn op_cases(id: &mut u32, out: &mut Vec<Case>) {
    let mut push = |what: String, bytes: Vec<u8>, id: &mut u32| {
        out.push(Case {
            id: *id,
            family: "op",
            what,
            bytes,
        });
        *id += 1;
    };

    // Every byte below 0xF0 that no compiler writes, after a 7: a byte the
    // VM skips leaves 7; one that stops the script sends no second chat.
    let named: Vec<u8> = (0u8..0xF0)
        .filter(|&b| !matches!(Op::from_u8(b), Op::Unknown(_)))
        .collect();
    for b in 0u8..0xF0 {
        if named.contains(&b) && b != 0 {
            continue;
        }
        for (tag, operand) in [
            ("none", Operand::None),
            ("int0", Operand::Int(0)),
            ("int5", Operand::Int(5)),
        ] {
            let i = *id;
            push(
                format!("byte {b} operand {tag} after 7"),
                spliced(i, |_| vec![num(7), ins(b, operand)]),
                id,
            );
        }
    }

    // Local slots: 47 resolves a var, 45 stores into slot n, 46 loads it.
    for n in [0, 1, 5, 1023, 1024, 4000, -1] {
        let i = *id;
        push(
            format!("45 store 9 into slot {n}, pop, 46 load slot {n}"),
            spliced(i, |_| {
                vec![
                    num(9),
                    ins(45, Operand::Int(n)),
                    bare(32),
                    ins(46, Operand::Int(n)),
                ]
            }),
            id,
        );
    }
    {
        let i = *id;
        push(
            "46 load of a never-stored slot 3".into(),
            spliced(i, |_| vec![ins(46, Operand::Int(3))]),
            id,
        );
        let i = *id;
        push(
            "47 on temp.q then 45 slot 0, assign 4 through 46".into(),
            spliced(i, |s| {
                let q = string(s, "q");
                vec![
                    bare(189),
                    ins(22, Operand::Str(q)),
                    bare(35),
                    bare(47),
                    ins(45, Operand::Int(0)),
                    bare(32),
                    ins(46, Operand::Int(0)),
                    num(4),
                    bare(50),
                    bare(189),
                    ins(22, Operand::Str(q)),
                    bare(35),
                    bare(33),
                ]
            }),
            id,
        );
    }

    // 48 object literal and 49 number list: operand n, pairs/values on
    // the stack in both orders, read back as text and as size.
    for n in [0, 1, 2, 3] {
        let i = *id;
        push(
            format!("49 number list n={n} over 1,2,3 then size"),
            spliced(i, |s| {
                let size = string(s, "size");
                vec![
                    num(1),
                    num(2),
                    num(3),
                    ins(49, Operand::Int(n)),
                    ins(22, Operand::Str(size)),
                    bare(35),
                    bare(33),
                ]
            }),
            id,
        );
        let i = *id;
        push(
            format!("49 number list n={n} over 1,2,3 as text"),
            spliced(i, |_| {
                vec![num(1), num(2), num(3), ins(49, Operand::Int(n)), bare(34)]
            }),
            id,
        );
    }
    for (order, n) in [("name,value", 1), ("value,name", 1), ("name,value", 2)] {
        let i = *id;
        push(
            format!("48 object literal n={n} order {order}, read .k"),
            spliced(i, |s| {
                let k = string(s, "k");
                let k2 = string(s, "k2");
                let mut v = Vec::new();
                for j in 0..n {
                    let (name, val) = if j == 0 { (k, 5) } else { (k2, 6) };
                    if order == "name,value" {
                        v.push(ins(21, Operand::Str(name)));
                        v.push(num(val));
                    } else {
                        v.push(num(val));
                        v.push(ins(21, Operand::Str(name)));
                    }
                }
                v.push(ins(48, Operand::Int(n)));
                v.push(ins(22, Operand::Str(k)));
                v.push(bare(35));
                v.push(bare(33));
                v
            }),
            id,
        );
    }

    // 54 three-operand member store, every push order of (object, name,
    // value), then read this.k back.
    let orders: [[usize; 3]; 6] = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for ord in orders {
        let i = *id;
        push(
            format!("54 member store, push order {ord:?} of (this, \"k\", 8), read this.k"),
            spliced(i, |s| {
                let k = string(s, "k");
                let parts = [bare(180), ins(21, Operand::Str(k)), num(8)];
                let mut v: Vec<Instr> = ord.iter().map(|&j| parts[j].clone()).collect();
                v.push(bare(54));
                v.push(bare(180));
                v.push(ins(22, Operand::Str(k)));
                v.push(bare(35));
                v.push(bare(33));
                v
            }),
            id,
        );
    }
    {
        let i = *id;
        push(
            "54 member store with operand 1".into(),
            spliced(i, |s| {
                let k = string(s, "k");
                vec![
                    bare(180),
                    ins(21, Operand::Str(k)),
                    num(8),
                    ins(54, Operand::Int(1)),
                    bare(180),
                    ins(22, Operand::Str(k)),
                    bare(35),
                    bare(33),
                ]
            }),
            id,
        );
    }

    // 66/67 value logic, 104 compare, 121 add-or-join, over mixed types.
    let vals: [(&str, Push); 5] = [
        ("0", |_| num(0)),
        ("2", |_| num(2)),
        ("\"\"", |s| ins(21, Operand::Str(string(s, "")))),
        ("\"x\"", |s| ins(21, Operand::Str(string(s, "x")))),
        ("\"3\"", |s| ins(21, Operand::Str(string(s, "3")))),
    ];
    for op in [66u8, 67, 104, 121] {
        for (an, a) in vals.iter() {
            for (bn, b) in vals.iter() {
                let i = *id;
                push(
                    format!("{op} on {an}, {bn}"),
                    spliced(i, |s| vec![a(s), b(s), bare(op)]),
                    id,
                );
            }
        }
    }

    // 164 (Linux: continue in the next script) and 0 (Linux no-op), alone.
    for b in [0u8, 164] {
        let i = *id;
        push(
            format!("{b} between 7 and 1 + (sum 8 if skipped)"),
            spliced(i, |_| vec![num(7), ins(b, Operand::None), num(1), bare(60)]),
            id,
        );
    }

    // waitfor on this with a short timeout: the value it gives when the
    // time runs out.
    for t in ["0.1", "0", "-1"] {
        let i = *id;
        push(
            format!("waitfor(this, \"never\", {t})"),
            {
                let src = template(i, &format!("waitfor(this, \"never\", {t})"));
                compile_gs2(&src).map(|s| s.write()).unwrap_or_default()
            },
            id,
        );
    }

    // Fold bytes written straight into the stream, after 3 and 4 and with
    // a numeric operand: the fold handlers read a decoded record's fields,
    // so this shows what a raw byte there does.
    for b in 200u8..=242 {
        let i = *id;
        push(
            format!("fold byte {b} raw after 3, 4 with operand 2"),
            spliced(i, |_| vec![num(3), num(4), ins(b, Operand::Int(2))]),
            id,
        );
    }
}

fn limit_cases(id: &mut u32, out: &mut Vec<Case>) {
    for n in [254usize, 255, 256, 257, 9998, 9999, 10000, 10001, 20000] {
        let i = *id;
        // n values on the stack at once, then n - 1 pops: the value left is 1.
        let bytes = spliced(i, |_| {
            let mut v: Vec<Instr> = (0..n).map(|_| num(1)).collect();
            v.extend((1..n).map(|_| bare(32)));
            v
        });
        out.push(Case {
            id: *id,
            family: "limit",
            what: format!("{n} pushes then {} pops (stack depth)", n - 1),
            bytes,
        });
        *id += 1;
    }
    let mut push = |what: String, src: String, id: &mut u32| {
        if let Ok(s) = compile_gs2(&src) {
            out.push(Case {
                id: *id,
                family: "limit",
                what,
                bytes: s.write(),
            });
            *id += 1;
        }
    };
    for n in [9999u32, 10000, 10001] {
        let i = *id;
        push(
            format!("while loop of {n} passes"),
            format!(
                "function onCreated() {{ player.chat = \"FZ{i}a\"; sleep(0.2); temp.r = 0; \
                 while (temp.r < {n}) temp.r++; player.chat = \"FZ{i}:\" @ temp.r; }}"
            ),
            id,
        );
    }
    for (lim, n) in [
        (50u32, 120u32),
        (100, 150),
        (2000000, 20000),
        (100, 99),
        (1000000, 1000001),
    ] {
        let i = *id;
        push(
            format!("maxlooplimit {lim}, loop of {n}"),
            format!(
                "function onCreated() {{ player.chat = \"FZ{i}a\"; sleep(0.2); this.maxlooplimit = {lim}; temp.r = 0; \
                 while (temp.r < {n}) temp.r++; player.chat = \"FZ{i}:\" @ temp.r @ \",\" @ this.maxlooplimit; }}"
            ),
            id,
        );
    }
    for n in [9998u32, 9999, 10000, 10001] {
        let i = *id;
        push(
            format!("recursion depth {n}"),
            format!(
                "function onCreated() {{ player.chat = \"FZ{i}a\"; sleep(0.2); temp.r = rec(0); \
                 player.chat = \"FZ{i}:\" @ temp.r; }} \
                 function rec(n) {{ if (n >= {n}) return n; return rec(n + 1); }}"
            ),
            id,
        );
    }
}

fn container_cases(r: &mut Rng, id: &mut u32, out: &mut Vec<Case>) {
    let mut push = |what: String, bytes: Vec<u8>, id: &mut u32| {
        out.push(Case {
            id: *id,
            family: "container",
            what,
            bytes,
        });
        *id += 1;
    };
    // Segment helpers over a good case's bytes.
    fn segs(b: &[u8]) -> Vec<(u32, Vec<u8>)> {
        let mut v = Vec::new();
        let mut p = 0;
        while p + 8 <= b.len() {
            let t = u32::from_be_bytes(b[p..p + 4].try_into().unwrap());
            let l = u32::from_be_bytes(b[p + 4..p + 8].try_into().unwrap()) as usize;
            v.push((t, b[p + 8..p + 8 + l].to_vec()));
            p += 8 + l;
        }
        v
    }
    fn join(v: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut o = Vec::new();
        for (t, b) in v {
            o.extend_from_slice(&t.to_be_bytes());
            o.extend_from_slice(&(b.len() as u32).to_be_bytes());
            o.extend_from_slice(b);
        }
        o
    }
    let good = |i: u32| base(i).write();

    for drop in 1..=4u32 {
        let i = *id;
        let v: Vec<_> = segs(&good(i)).into_iter().filter(|s| s.0 != drop).collect();
        push(format!("segment {drop} left out"), join(&v), id);
    }
    {
        let i = *id;
        let mut v = segs(&good(i));
        v.reverse();
        push("segments in reverse order".into(), join(&v), id);
    }
    for extra in [0u32, 5, 6, 0x7fff_ffff, 0xffff_ffff] {
        let i = *id;
        let mut v = segs(&good(i));
        v.insert(0, (extra, b"junk\0".to_vec()));
        push(format!("unknown segment type {extra} first"), join(&v), id);
    }
    {
        let i = *id;
        let mut v = segs(&good(i));
        let code = v.iter().position(|s| s.0 == 4).unwrap();
        let dup = v[code].clone();
        v.insert(0, dup);
        push("code segment twice".into(), join(&v), id);
    }
    for flags in [1u32, 0xffff_ffff] {
        let i = *id;
        let mut v = segs(&good(i));
        if let Some(s) = v.iter_mut().find(|s| s.0 == 1) {
            s.1 = flags.to_be_bytes().to_vec();
        }
        push(format!("gs1 flags segment = {flags:#x}"), join(&v), id);
    }
    for (what, cut) in [("last byte", 1usize), ("last 5 bytes", 5), ("half", 0)] {
        let i = *id;
        let mut b = good(i);
        let n = if cut == 0 { b.len() / 2 } else { b.len() - cut };
        b.truncate(n);
        push(format!("truncated: {what}"), b, id);
    }
    {
        let i = *id;
        let mut b = good(i);
        b[7] = b[7].wrapping_add(200);
        push("first segment length too large".into(), b, id);
    }
    for tail in [
        &b"\n"[..],
        b"\0\0\0\0",
        b"\x00\x00\x00\x04\x00\x00\x00\x02\x14\xf3",
    ] {
        let i = *id;
        let mut b = good(i);
        b.extend_from_slice(tail);
        push(format!("trailing bytes {}", hex(tail)), b, id);
    }
    // String index out of range, a jump target out of range and negative,
    // a function start past the end.
    {
        let i = *id;
        push(
            "string index 200 (out of range) as the value".into(),
            spliced(i, |_| vec![ins(21, Operand::Str(200))]),
            id,
        );
        let i = *id;
        push(
            "string index 0xFFFF as the value".into(),
            spliced(i, |_| vec![ins(21, Operand::Str(0xffff))]),
            id,
        );
        let i = *id;
        push(
            "jump to op 9999 after 7".into(),
            spliced(i, |_| vec![num(7), ins(10, Operand::Int(9999))]),
            id,
        );
        let i = *id;
        push(
            "jump to op -1 after 7".into(),
            spliced(i, |_| vec![num(7), ins(10, Operand::Int(-1))]),
            id,
        );
    }
    {
        let i = *id;
        let mut s = base(i);
        if let Some(f) = s.functions.first_mut() {
            f.start = 60000;
        }
        push("onCreated start past the end".into(), s.write(), id);
    }
    // F6 decimal operand text oddities.
    for t in [
        "1e400",
        "",
        "-0",
        "nan",
        "inf",
        "0x10",
        "1,5",
        "12abc",
        "  3",
        "1e-400",
        "999999999999999999999",
    ] {
        let i = *id;
        push(
            format!("F6 decimal text {t:?}"),
            spliced(i, |_| vec![ins(20, Operand::Decimal(t.to_string()))]),
            id,
        );
    }
    // Marker bytes after an opcode, written by hand into the code segment.
    for m in 0xF0u8..=0xFF {
        let i = *id;
        let mut v = segs(&good(i));
        let code = v.iter_mut().find(|s| s.0 == 4).unwrap();
        // Replace `OP_TYPE_NUMBER F5 <777777>` with `14 F3 07 <m> 00 00 00 00 00 00 00 00`:
        // a number 7, then the marker with eight zero bytes behind it.
        let want = [20u8, 0xF5];
        if let Some(p) = code.1.windows(2).position(|w| w == want) {
            let mut repl = vec![20u8, 0xF3, 7, 20, m];
            repl.extend_from_slice(&[0u8; 8]);
            code.1.splice(p..p + 6, repl);
        }
        push(
            format!("marker {m:#x} where a second operand would be"),
            join(&v),
            id,
        );
    }
    // A few random single-byte flips in the code segment.
    for _ in 0..24 {
        let i = *id;
        let mut v = segs(&good(i));
        let code = v.iter_mut().find(|s| s.0 == 4).unwrap();
        let p = r.below(code.1.len());
        let nb = r.next() as u8;
        let old = code.1[p];
        code.1[p] = nb;
        push(
            format!("code byte {p} {old:#04x} -> {nb:#04x}"),
            join(&v),
            id,
        );
    }
}

fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        let _ = write!(s, "{x:02x}");
    }
    s
}

/// Texts that may or may not read as zero, each through `+ 0`, `!`, `== 0`
/// and `== -1`: which texts the clients turn into −1 rather than 0.
fn numtext_cases(id: &mut u32, out: &mut Vec<Case>) {
    const TEXTS: &[&str] = &[
        "0", "0.0", "00", " 0", "0 ", "+0", "-0", ".0", "0.", "0e5", "0x0", "0abc", "000001", "1",
        "-1", "1abc", "a1", "abc", "", " ", "true", "false", "null",
    ];
    for t in TEXTS {
        let lit = format!("\"{t}\"");
        for e in [
            format!("{lit} + 0"),
            format!("!{lit}"),
            format!("({lit} == 0)"),
            format!("({lit} == -1)"),
        ] {
            let Ok(s) = compile_gs2(&template(*id, &e)) else {
                continue;
            };
            out.push(Case {
                id: *id,
                family: "numtext",
                what: e,
                bytes: s.write(),
            });
            *id += 1;
        }
    }
}

fn main() {
    let mut seed = 1u64;
    let mut count = 300usize;
    let mut first = 30000u32;
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
    let mut id = first;
    let mut cases = Vec::new();
    let want = |f: &str| only.as_deref().is_none_or(|o| o == f);
    // Two controls first: the bare template, and a sum through the compiler.
    cases.push(Case {
        id,
        family: "control",
        what: "template as compiled".into(),
        bytes: base(id).write(),
    });
    id += 1;
    cases.push(Case {
        id,
        family: "control",
        what: "1 + 2 @ \"x\"".into(),
        bytes: compile_gs2(&template(id, "1 + 2 @ \"x\"")).unwrap().write(),
    });
    id += 1;
    if want("op") {
        op_cases(&mut id, &mut cases);
    }
    if want("limit") {
        limit_cases(&mut id, &mut cases);
    }
    if want("container") {
        container_cases(&mut r, &mut id, &mut cases);
    }
    if want("expr") {
        expr_cases(&mut r, &mut id, count, &mut cases);
    }
    if want("numtext") {
        numtext_cases(&mut id, &mut cases);
    }
    println!(
        "id\tfamily\twhat\thex\t{}",
        PROFILES
            .iter()
            .map(|p| p.label)
            .collect::<Vec<_>>()
            .join("\t")
    );
    for c in cases.iter().filter(|c| !c.bytes.is_empty()) {
        let preds: Vec<String> = PROFILES.iter().map(|p| predict(&c.bytes, p)).collect();
        // The case's own id is the one baked into its chat strings.
        let cid = c.id;
        println!(
            "{cid}\t{}\t{}\t{}\t{}",
            c.family,
            clean(&c.what),
            hex(&c.bytes),
            preds.join("\t")
        );
    }
}
