//! Temporary GS1 front end on a small GS2 compiler.
//!
//! Issue 371: one GS1 script is parsed with the GS1 grammar in
//! [`crate::scripting::engine`] and lowered to the GS2 bytecode described in
//! `documentation/gs2-opcodes.md` and `documentation/gs2-grammar.md`. The same
//! emitter compiles the GS2 subset those fixtures use, so a GS1 script and the
//! equivalent GS2 source can be compared op for op.
//!
//! This is not a second opcode set and not a permanent compiler. It is the
//! smallest front end that issue needs, and it is expected to be phased out.
//! GS2-to-GS1 and server-side GS2 are out of scope. A construct this front
//! end does not lower is a compile error.
//!
//! Opcode built-ins require their documented argument counts before emission;
//! `format` needs a pattern and accepts additional values. This is Seihai's
//! compile-time validation policy, not a claim about original-client errors.
//! Script and host calls are not subject to those opcode-specific checks.
//! Array literals use the documented reverse-push encoding. The GS1
//! interpreter evaluates elements left to right, so GS1 lowering takes a
//! literal only when that order cannot matter: no element calls script or
//! host code or `random`, or exactly one does and the rest are constants.
//!
//! Indexed reads and writes (`a[i]`, `a[i, j]`, `a[i] = v`, `a[i, j] = v`)
//! and `x in array` use the encodings in `tests/fixtures/gs2/lists.gs2bc`:
//! the variable, `OP_CONV_TO_OBJECT`, the indexes, then the value for a write,
//! and `OP_ARRAY` / `OP_ARRAY_MULTIDIM` / `OP_ARRAY_ASSIGN` /
//! `OP_ARRAY_MULTIDIM_ASSIGN` / `OP_IN_OBJ`. The fixture only has constant
//! indexes; a computed index gets `OP_CONV_TO_FLOAT` like every other numeric
//! operand here, which is Seihai's choice, not a fixture-checked encoding.
//! In GS1, compound assignment and `++`/`--` on an element are rewritten to
//! `a[i] = a[i] op v`, and `x in {a, b}` to `x == a || x == b`, which is what
//! the GS1 interpreter does; both are refused when the repeated expression
//! calls a function. The GS2 front keeps them errors (no fixture for
//! gs2-parser's own encoding). A field after an index (`npcs[i].x`), three or
//! more indexes and the `board`/`tiles` arrays have no fixture and stay
//! compile errors.
//!
//! The GS2 front end takes code outside functions (emitted where it stands,
//! so it runs from op 0 with the function bodies jumped over), `public
//! function`, a one-statement function body, method calls and members on any
//! value, `SPC`/`NL`/`TAB`, `?:`, `<<`/`>>`, `in |a, b|` and `in {a, b}`,
//! `switch`, `for (x : list)`, `new T(…)`, `new[n][m]`, `++x`, `@=` and
//! string escapes. Six of the eight fixtures compile to the same bytecode;
//! `control` and `calls` differ by one instruction each on purpose (a jump
//! out after the last `switch` arm, and popping a call's value when it is an
//! `if` body without braces).

use super::{Function, Instr, Op, Operand, Script};
use crate::scripting::engine::ast::*;
use crate::scripting::engine::value::num_to_string;
use crate::scripting::engine::{parse, vocab, Era};
use std::collections::HashMap;
use std::fmt;

/// One problem found while compiling. `line` is 0 when the grammar did not
/// record a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub line: u32,
    pub message: String,
}

/// Every problem from one compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileErrors {
    pub errors: Vec<CompileError>,
}

impl fmt::Display for CompileErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.errors.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            if e.line == 0 {
                write!(f, "{}", e.message)?;
            } else {
                write!(f, "line {}: {}", e.line, e.message)?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for CompileErrors {}

impl CompileErrors {
    fn one(line: u32, message: impl Into<String>) -> Self {
        CompileErrors {
            errors: vec![CompileError {
                line,
                message: message.into(),
            }],
        }
    }
}

/// GS1 command names that have no GS2 command and no GS2 opcode.
///
/// A name is kept when it is in the 3.153 command tables
/// (`documentation/gs2-shared-commands.md`), when `gs2-executor.md` names it
/// as a call the host still accepts (`set`, `unset`, `setlevel`,
/// `setlevel2`, `setplayerprop`), when `gs2-grammar.md` says the compiler
/// keeps it as a call by name (`join("class")`, so GS1 `join class;` lowers
/// to that call with the class name as a string), or when `gs2-opcodes.md` gives it an
/// opcode (`sleep`, `setarray`). The string-list commands are the case the
/// notes call out: GS2 has list methods instead, and a call by the GS1 name
/// is not a GS2 command.
pub fn gs1_commands_without_gs2() -> Vec<&'static str> {
    vocab::COMMANDS
        .iter()
        .filter(|c| !gs2_has_command(c.name))
        .map(|c| c.name)
        .collect()
}

fn gs2_has_command(name: &str) -> bool {
    GS2_CLIENT_COMMANDS.binary_search(&name).is_ok()
        || matches!(
            name,
            "set" | "unset" | "setlevel" | "setlevel2" | "setplayerprop" | "join"
        )
        || opcode_builtin(name).is_some()
}

/// Names registered by the 3.153 client, lower case, sorted.
/// From `documentation/gs2-shared-commands.md`.
const GS2_CLIENT_COMMANDS: &[&str] = &[
    "addcontrol",
    "addtiledef",
    "addtiledef2",
    "aindexof",
    "attachplayertoobj",
    "blockagain",
    "blockagainlocal",
    "callnpc",
    "callweapon",
    "canbecarried",
    "canbepulled",
    "canbepushed",
    "cannotbecarried",
    "cannotbepulled",
    "cannotbepushed",
    "carryobject",
    "changeimgcolors",
    "changeimgmode",
    "changeimgpart",
    "changeimgvis",
    "changeimgzoom",
    "commandtoserver",
    "contains",
    "cursoroff",
    "cursoron",
    "destroy",
    "detachplayer",
    "disabledefmovement",
    "disablemap",
    "disablepause",
    "disableselectweapons",
    "disableweapons",
    "dontblock",
    "dontblocklocal",
    "drawaslight",
    "drawoverplayer",
    "drawunderplayer",
    "echo",
    "enabledefmovement",
    "enablefeatures",
    "enablemap",
    "enablepause",
    "enableselectweapons",
    "enableweapons",
    "explodebomb",
    "extractfilebase",
    "extractfileext",
    "extractfilename",
    "extractfilepath",
    "findfirstfile",
    "findimg",
    "findnextfile",
    "findplayer",
    "findplayerbyid",
    "findweapon",
    "freezeplayer",
    "getascii",
    "getboxcenter",
    "getdatablocktypes",
    "getextension",
    "getimgheight",
    "getimgwidth",
    "getkeycode",
    "getmapx",
    "getmapy",
    "getobjectat",
    "getobjectatmouse",
    "getobjectbyray",
    "getstringkeys",
    "getsurfacetype",
    "gettextheight",
    "gettextwidth",
    "getz",
    "hide",
    "hideimg",
    "hideimgs",
    "hidelocal",
    "hideplayer",
    "hidesword",
    "hitnpc",
    "hitobjects",
    "hitplayer",
    "hurt",
    "isadminguild",
    "iscursoron",
    "isobject",
    "keydown",
    "keydown2",
    "keyname",
    "lay",
    "lay2",
    "lightscene",
    "loadmap",
    "mapshapeanimation",
    "matrixcreate",
    "matrixcreatefromeuler",
    "matrixmulpoint",
    "matrixmultiply",
    "matrixmulvector",
    "message",
    "move",
    "noplayerkilling",
    "onwall",
    "onwall2",
    "onwater",
    "onwater2",
    "openurl",
    "openurl2",
    "play",
    "play2",
    "playlooped",
    "popdialog",
    "pushdialog",
    "putleaps",
    "reloadfile",
    "removetiledefs",
    "replaceani",
    "requestfiledeletion",
    "requestfilerename",
    "requestfilesmove",
    "requesttext",
    "resetfocus",
    "savelog",
    "say",
    "say2",
    "screenx",
    "screeny",
    "selectfilefordownload",
    "selectfileforupload",
    "sendrpgmessage",
    "sendtext",
    "sendtorc",
    "serverwarp",
    "setani",
    "setbeltcolor",
    "setbow",
    "setcharani",
    "setchargender",
    "setcoatcolor",
    "setcoloreffect",
    "setcontentcontrol",
    "setcursor2",
    "seteffect",
    "seteffectmode",
    "setfocus",
    "setfogcolors",
    "setgender",
    "sethead",
    "setimg",
    "setimgpart",
    "setinteriorrendermode",
    "setletters",
    "setmap",
    "setminimap",
    "setmusicvolume",
    "setplayerdir",
    "setshadowdetaillevel",
    "setshape",
    "setshape2",
    "setshield",
    "setshoecolor",
    "setshootparams",
    "setskincolor",
    "setskybandcolors",
    "setskybandsizes",
    "setsleevecolor",
    "setsuncolors",
    "setsword",
    "setterrainrendermode",
    "setzoomeffect",
    "shootarrow",
    "shootball",
    "shootfireball",
    "shootfireblast",
    "shootnuke",
    "show",
    "showani",
    "showani2",
    "showcharacter",
    "showimg",
    "showimg2",
    "showlocal",
    "showpoly",
    "showpoly2",
    "showstats",
    "showtext",
    "showtext2",
    "spyfire",
    "stopmidi",
    "stopsound",
    "strequals",
    "synctimeofday",
    "take",
    "take2",
    "takehorse",
    "takeplayercarry",
    "takeplayerhorse",
    "testplayer",
    "throwcarry",
    "tiletype",
    "timereverywhere",
    "toweapons",
    "triggeraction",
    "triggerserver",
    "updateboard",
    "updateterrain",
    "vectoradd",
    "vectorcross",
    "vectordist",
    "vectordot",
    "vectorlen",
    "vectornormalize",
    "vectororthobasis",
    "vectorscale",
    "vectorsub",
    "worldx",
    "worldy",
    "wraptext",
    "wraptext2",
];

fn command_hint(name: &str) -> &'static str {
    match name {
        "setstring" | "addstring" | "insertstring" | "replacestring" | "removestring"
        | "deletestring" => " (GS2 uses list methods such as add and delete)",
        "tokenize" | "tokenize2" => " (GS2 uses the tokenize method)",
        "setgif" => " (GS2 has setimg)",
        "setgifpart" => " (GS2 has setimgpart)",
        _ => "",
    }
}

/// GS1 functions the opcode notes do not map and the 3.153 tables do not
/// register. `strlen` / `startswith` / `indexof` are lowered to the methods
/// named beside them in `gs2-opcodes.md`.
fn gs1_fn_without_gs2(name: &str) -> bool {
    matches!(
        name,
        "playersays"
            | "playersays2"
            | "ascii"
            | "strcontains"
            | "lindexof"
            | "onmapx"
            | "onmapy"
            | "strtofloat"
            | "hasweapon"
            | "getnpc"
            | "getplayer"
    )
}

fn opcode_builtin(name: &str) -> Option<Op> {
    Some(match name {
        "sleep" => Op::Sleep,
        "sin" => Op::Sin,
        "cos" => Op::Cos,
        "arctan" => Op::ArcTan,
        "abs" => Op::Abs,
        "random" => Op::Random,
        "arraylen" | "sarraylen" => Op::ObjSize,
        "setarray" => Op::SetArray,
        "int" => Op::Int,
        "exp" => Op::Exp,
        "log" => Op::Log,
        "min" => Op::Min,
        "max" => Op::Max,
        "pow" => Op::Pow,
        "char" => Op::Char,
        "getangle" => Op::GetAngle,
        "getdir" => Op::GetDir,
        "vecx" => Op::VecX,
        "vecy" => Op::VecY,
        "waitfor" => Op::WaitFor,
        "format" => Op::Format,
        "makevar" => Op::MakeVar,
        _ => return None,
    })
}

/// `true` when a statement call should drop the value it pushed.
fn call_pushes(name: &str) -> bool {
    !matches!(name, "sleep" | "setarray")
}

/// GS2 function name for a GS1 event. The spellings `onCreated`,
/// `onTimeout`, `onPlayerEnters` and `onPlayerChats` are the ones in
/// `documentation/gs2-grammar.md` and `documentation/seihaiscript.md`. The
/// other events in `vocab::EVENTS` are split into those same words.
pub fn gs2_event_function(event: &str) -> String {
    let mut rest = event;
    let mut parts = String::from("on");
    while !rest.is_empty() {
        let word = EVENT_WORDS
            .iter()
            .copied()
            .filter(|w| rest.starts_with(w))
            .max_by_key(|w| w.len());
        let word = word.unwrap_or(rest);
        let mut chars = word.chars();
        if let Some(c) = chars.next() {
            parts.push(c.to_ascii_uppercase());
            parts.push_str(chars.as_str());
        }
        if word.len() == rest.len() && EVENT_WORDS.iter().all(|w| !rest.starts_with(w)) {
            break;
        }
        rest = &rest[word.len()..];
    }
    parts
}

const EVENT_WORDS: &[&str] = &[
    "movement", "selected", "finished", "shooted", "reading", "exploded", "timeout", "created",
    "weapon", "player", "touchs", "enters", "leaves", "thrown", "pushed", "pulled", "compus",
    "action", "chats", "other", "baddy", "fired", "comps", "shot", "pelt", "hurt", "dies", "dead",
    "lays", "item", "ends", "was", "me", "by",
];

fn is_event(name: &str, era: Era) -> bool {
    vocab::event_since(name).is_some_and(|e| e <= era)
}

fn contains_event(e: &Expr, era: Era) -> bool {
    match e {
        Expr::Var(v) => is_event(&v.name, era),
        Expr::Unary(_, a) | Expr::InArray { e: a, .. } => contains_event(a, era),
        Expr::Bin(_, a, b) => contains_event(a, era) || contains_event(b, era),
        Expr::Call { args, .. } => args.iter().any(|a| contains_event(a, era)),
        Expr::InRange { e, lo, hi } => {
            contains_event(e, era) || contains_event(lo, era) || contains_event(hi, era)
        }
        Expr::InList { e, list } => {
            contains_event(e, era) || list.iter().any(|a| contains_event(a, era))
        }
        Expr::List(list) => list.iter().any(|a| contains_event(a, era)),
        Expr::Gs2(g) => g.parts().into_iter().any(|a| contains_event(a, era)),
        Expr::Cond { cond, then, els } => {
            contains_event(cond, era) || contains_event(then, era) || contains_event(els, era)
        }
        Expr::Num(_) | Expr::Str(_) => false,
    }
}

enum Arms {
    Events(Vec<(String, Option<Expr>)>),
    NotEvent,
    Complex,
}

/// Top-level `if (playerenters)` / `if (created || timeout)` / `if (event && guard)`.
fn event_arms(e: &Expr, era: Era) -> Arms {
    match e {
        Expr::Var(v) if is_event(&v.name, era) => Arms::Events(vec![(v.name.clone(), None)]),
        Expr::Bin(BinOp::Or, a, b) => match (event_arms(a, era), event_arms(b, era)) {
            (Arms::Events(mut l), Arms::Events(r)) => {
                if l.iter().any(|(_, g)| g.is_some()) || r.iter().any(|(_, g)| g.is_some()) {
                    Arms::Complex
                } else {
                    l.extend(r);
                    Arms::Events(l)
                }
            }
            _ => {
                if contains_event(e, era) {
                    Arms::Complex
                } else {
                    Arms::NotEvent
                }
            }
        },
        // `a && event && b && …`: one event among the terms, the rest a
        // guard in source order. A term before the event that calls a
        // function would run in every event in GS1, so that stays complex.
        Expr::Bin(BinOp::And, _, _) => {
            let mut terms = Vec::new();
            and_terms(e, &mut terms);
            let events: Vec<usize> = (0..terms.len())
                .filter(|&i| event_var(terms[i], era).is_some())
                .collect();
            let others_clean = terms
                .iter()
                .enumerate()
                .all(|(i, t)| events.contains(&i) || !contains_event(t, era));
            match events.as_slice() {
                [at] if others_clean && !terms[..*at].iter().any(|t| mentions_any_call(t)) => {
                    let ev = event_var(terms[*at], era).unwrap();
                    let guard = terms
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| i != at)
                        .map(|(_, t)| (*t).clone())
                        .reduce(|a, b| Expr::Bin(BinOp::And, Box::new(a), Box::new(b)));
                    Arms::Events(vec![(ev, guard)])
                }
                [] if others_clean => Arms::NotEvent,
                _ => Arms::Complex,
            }
        }
        _ => {
            if contains_event(e, era) {
                Arms::Complex
            } else {
                Arms::NotEvent
            }
        }
    }
}

fn plain_var(name: &str) -> VarRef {
    VarRef {
        name: name.to_string(),
        index: Vec::new(),
        field_after_index: false,
    }
}

/// The GS2 property a GS1 `setcharprop` code sets on the NPC itself.
fn char_prop(code: &str) -> Option<VarRef> {
    let prop = match code {
        "#c" => "this.chat",
        "#n" => "this.nick",
        "#1" => "this.swordimg",
        "#2" => "this.shieldimg",
        "#3" => "this.headimg",
        "#8" => "this.bodyimg",
        _ => {
            let (base, n, lo, hi) = match code.strip_prefix("#C") {
                Some(n) => ("this.colors", n, 0, 4),
                None => ("this.attr", code.strip_prefix("#P")?, 1, 30),
            };
            let i: u32 = n.parse().ok()?;
            if !(lo..=hi).contains(&i) {
                return None;
            }
            return Some(VarRef {
                name: base.into(),
                index: vec![Expr::Num(f64::from(i))],
                field_after_index: false,
            });
        }
    };
    Some(plain_var(prop))
}

fn no_lowering(code: char) -> String {
    format!("message code `#{code}` has no GS2 lowering in this temporary front end")
}

enum TextPart {
    Lit(String),
    Expr(Expr),
    Trim(String),
    Sub(Expr, Expr, String),
}

/// Splits GS1 text at its message codes, with the bounds and era gates of
/// Seihai's GS1 engine (`engine::interp`, `expand`).
fn split_codes(s: &str, era: Era) -> Result<Vec<TextPart>, String> {
    let b = s.as_bytes();
    let mut parts = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    let num = |x: &str| -> Expr {
        let x = x.trim();
        if x.contains('#') {
            Expr::Str(x.to_string())
        } else {
            crate::scripting::engine::parser::parse_expr(x, era).unwrap_or(Expr::Num(0.0))
        }
    };
    while i < b.len() {
        if b[i] != b'#' || i + 1 >= b.len() {
            let ch = s[i..].chars().next().unwrap();
            lit.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let code = b[i + 1] as char;
        let digits = crate::scripting::engine::lexer::code_digits(&b[i + 1..]);
        let open = i + 2 + digits;
        let (inner, end) = if b.get(open) == Some(&b'(') {
            let mut d = 0;
            let mut close = None;
            for (j, &c) in b.iter().enumerate().skip(open) {
                match c {
                    b'(' => d += 1,
                    b')' => {
                        d -= 1;
                        if d == 0 {
                            close = Some(j);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            match close {
                Some(c) => (Some(&s[open + 1..c]), c + 1),
                None => (None, open),
            }
        } else {
            (None, open)
        };
        let var = |n: &str, idx: Option<Expr>| {
            Expr::Var(VarRef {
                name: n.into(),
                index: idx.into_iter().collect(),
                field_after_index: false,
            })
        };
        let part = match (code, inner) {
            ('c', None) if digits == 0 => Some(TextPart::Expr(var("player.chat", None))),
            ('n', None) => Some(TextPart::Expr(var("player.nick", None))),
            ('a', None) => Some(TextPart::Expr(var("player.account", None))),
            ('v', Some(x)) => Some(TextPart::Expr(num(x))),
            ('s', Some(x)) => {
                let n = x.trim().to_ascii_lowercase();
                let readable = ["this.", "client.", "clientr.", "serverr."];
                if readable.iter().any(|p| n.starts_with(p)) && is_plain_name(&n) {
                    Some(TextPart::Expr(var(&n, None)))
                } else {
                    return Err(format!(
                        "`#s({x})`: only `this.`, `client.`, `clientr.` and `serverr.` strings are lowered (a plain GS1 string is a player flag)"
                    ));
                }
            }
            ('t', Some(x)) if era >= Era::V202 => Some(TextPart::Expr(var("tokens", Some(num(x))))),
            ('p', Some(x)) if era >= Era::V203 => Some(TextPart::Expr(var("params", Some(num(x))))),
            ('T', Some(x)) if era >= Era::V202 => Some(TextPart::Trim(x.to_string())),
            ('e', Some(x)) if era >= Era::V202 => {
                let p: Vec<&str> = x.splitn(3, ',').collect();
                if p.len() != 3 {
                    return Err(format!("`#e({x})` needs start, length and text"));
                }
                Some(TextPart::Sub(num(p[0]), num(p[1]), p[2].to_string()))
            }
            ('w' | 'L' | 'F', None) => return Err(no_lowering(code)),
            ('w' | 'W', Some(_)) if era >= Era::V204 => return Err(no_lowering(code)),
            ('I', Some(_)) if era >= Era::V21 => return Err(no_lowering(code)),
            ('P', _) if digits > 0 && era >= Era::V21 => return Err(no_lowering(code)),
            _ => None,
        };
        match part {
            Some(p) => {
                if !lit.is_empty() {
                    parts.push(TextPart::Lit(std::mem::take(&mut lit)));
                }
                parts.push(p);
                i = end;
            }
            None => {
                lit.push('#');
                i += 1;
            }
        }
    }
    if !lit.is_empty() || parts.is_empty() {
        parts.push(TextPart::Lit(lit));
    }
    Ok(parts)
}

fn and_terms<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
    match e {
        Expr::Bin(BinOp::And, a, b) => {
            and_terms(a, out);
            and_terms(b, out);
        }
        _ => out.push(e),
    }
}

/// The expression calls any function (script, host or built-in).
fn mentions_any_call(e: &Expr) -> bool {
    match e {
        Expr::Call { .. } => true,
        Expr::Num(_) | Expr::Str(_) => false,
        Expr::Var(v) => v.index.iter().any(mentions_any_call),
        Expr::Unary(_, a) | Expr::InArray { e: a, .. } => mentions_any_call(a),
        Expr::Bin(_, a, b) => mentions_any_call(a) || mentions_any_call(b),
        Expr::InRange { e, lo, hi } => {
            mentions_any_call(e) || mentions_any_call(lo) || mentions_any_call(hi)
        }
        Expr::InList { e, list } => mentions_any_call(e) || list.iter().any(mentions_any_call),
        Expr::Cond { cond, then, els } => {
            mentions_any_call(cond) || mentions_any_call(then) || mentions_any_call(els)
        }
        Expr::List(v) => v.iter().any(mentions_any_call),
        Expr::Gs2(_) => true,
    }
}

fn event_var(e: &Expr, era: Era) -> Option<String> {
    match e {
        Expr::Var(v) if is_event(&v.name, era) => Some(v.name.clone()),
        _ => None,
    }
}

/// One piece of a script in source order: a function, or (`top`) code
/// outside any function, which is emitted where it stands so it runs from
/// op 0 with the function bodies jumped over.
struct Item {
    name: String,
    params: Vec<String>,
    body: Vec<Stmt>,
    top: bool,
}

impl Item {
    fn func(name: String, params: Vec<String>, body: Vec<Stmt>) -> Item {
        Item {
            name,
            params,
            body,
            top: false,
        }
    }

    fn top(body: Vec<Stmt>) -> Item {
        Item {
            name: String::new(),
            params: Vec::new(),
            body,
            top: true,
        }
    }
}

/// Compile GS1 with the engine grammar and lower it to GS2 bytecode.
pub fn compile_gs1(src: &str, era: Era) -> Result<Script, CompileErrors> {
    let parsed = parse(src, era);
    if !parsed.diagnostics.is_empty() {
        return Err(CompileErrors {
            errors: parsed
                .diagnostics
                .into_iter()
                .map(|d| CompileError {
                    line: d.line,
                    message: d.msg,
                })
                .collect(),
        });
    }
    let items = lift_gs1(parsed.body, era)?;
    finish(emit_items(&items, true, era)?)
}

/// Compile the GS2 subset this temporary compiler accepts.
pub fn compile_gs2(src: &str) -> Result<Script, CompileErrors> {
    let items = parse_gs2(src)?;
    finish(emit_items(&items, false, Era::V21)?)
}

fn finish(built: Built) -> Result<Script, CompileErrors> {
    if !built.errors.is_empty() {
        return Err(CompileErrors {
            errors: built.errors,
        });
    }
    let script = Script {
        gs1_flags: 0,
        functions: built.functions,
        strings: built.strings,
        code: built.code,
    };
    Script::parse(&script.write()).map_err(|message| CompileErrors::one(0, message))
}

/// Lifts top-level GS1 into GS2 functions. Several `if (created)` blocks
/// (or any repeated event test) become one `onCreated`, their bodies in
/// source order: GS1 runs every block whose test is true, top to bottom, and
/// only one event is true per run. A block that can `return` is not merged,
/// since the return would skip the blocks after it.
fn lift_gs1(body: Vec<Stmt>, era: Era) -> Result<Vec<Item>, CompileErrors> {
    let mut items: Vec<Item> = Vec::new();
    let mut event_items = HashMap::<String, usize>::new();
    let mut errors = Vec::new();
    let mut seen_event = false;
    for stmt in body {
        match stmt {
            Stmt::Function { name, params, body } => items.push(Item::func(name, params, body)),
            Stmt::If { cond, then, els } => match event_arms(&cond, era) {
                Arms::NotEvent => {
                    let stmt = Stmt::If { cond, then, els };
                    top_level(&mut items, &mut errors, stmt, seen_event);
                }
                Arms::Complex => errors.push(CompileError {
                    line: 0,
                    message: "event test is too complex for this temporary front end; split it into one if per event".into(),
                }),
                Arms::Events(arms) => {
                    seen_event = true;
                    if els.is_some() {
                        errors.push(CompileError {
                            line: 0,
                            message: "an event test with else has no single GS2 function".into(),
                        });
                        continue;
                    }
                    let base = match *then {
                        Stmt::Block(v) => v,
                        other => vec![other],
                    };
                    for (event, guard) in arms {
                        let body = if let Some(guard) = guard {
                            vec![Stmt::If { cond: guard, then: Box::new(Stmt::Block(base.clone())), els: None }]
                        } else {
                            base.clone()
                        };
                        let name = gs2_event_function(&event);
                        if let Some(&at) = event_items.get(&name) {
                            if items[at].body.iter().any(can_return) {
                                errors.push(CompileError {
                                    line: 0,
                                    message: format!(
                                        "`{name}` blocks are not merged because an earlier one can return"
                                    ),
                                });
                                continue;
                            }
                            items[at].body.extend(body);
                        } else {
                            event_items.insert(name.clone(), items.len());
                            items.push(Item::func(name, Vec::new(), body));
                        }
                    }
                }
            },
            other => top_level(&mut items, &mut errors, other, seen_event),
        }
    }
    if !errors.is_empty() {
        return Err(CompileErrors { errors });
    }
    Ok(items)
}

/// A GS1 statement outside every event test. GS1 runs it in every event,
/// in source order. It becomes GS2 code outside functions, which the Linux
/// 4.04r2 experiment saw run before each event's handler (Graal_analysis
/// `client-internals/linux-404r2-events.md`). Before the first event test
/// the order is the same. After one, only commands and calls are taken
/// (`join`, `dontblock`, `setshape` and the like, which set state); an
/// assignment, `if` or loop there could see a different order, so it stays
/// an error.
fn top_level(items: &mut Vec<Item>, errors: &mut Vec<CompileError>, stmt: Stmt, seen_event: bool) {
    if seen_event && !matches!(stmt, Stmt::Command { .. } | Stmt::Call { .. }) {
        errors.push(CompileError {
            line: 0,
            message: "a top-level statement after an event test is not lowered \
                      (GS2 runs code outside functions before the handler); move it above the first event test"
                .into(),
        });
        return;
    }
    match items.last_mut() {
        Some(last) if last.top => last.body.push(stmt),
        _ => items.push(Item::top(vec![stmt])),
    }
}

fn can_return(s: &Stmt) -> bool {
    match s {
        Stmt::Return(_) => true,
        Stmt::Block(v) => v.iter().any(can_return),
        Stmt::If { then, els, .. } => can_return(then) || els.as_deref().is_some_and(can_return),
        Stmt::While { body, .. } | Stmt::With { body, .. } => can_return(body),
        Stmt::For { body, .. } => can_return(body),
        _ => false,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Number,
    String,
    Other,
}

struct Built {
    functions: Vec<Function>,
    strings: Vec<String>,
    code: Vec<Instr>,
    errors: Vec<CompileError>,
}

struct Loop {
    continues: Vec<usize>,
    breaks: Vec<usize>,
}

struct Builder {
    code: Vec<Instr>,
    strings: Vec<String>,
    index: HashMap<String, u32>,
    functions: Vec<Function>,
    errors: Vec<CompileError>,
    loops: Vec<Loop>,
    skips: Vec<usize>,
    gs1: bool,
    era: Era,
    line: u32,
    /// The script's own function names, lower case.
    script_fns: Vec<String>,
}

fn emit_items(items: &[Item], gs1: bool, era: Era) -> Result<Built, CompileErrors> {
    let mut seen = HashMap::<String, ()>::new();
    let mut errors = Vec::new();
    for item in items.iter().filter(|i| !i.top) {
        if seen.insert(item.name.clone(), ()).is_some() {
            errors.push(CompileError {
                line: 0,
                message: format!("duplicate function `{}`", item.name),
            });
        }
    }
    if !errors.is_empty() {
        return Err(CompileErrors { errors });
    }
    let mut b = Builder {
        code: Vec::new(),
        strings: Vec::new(),
        index: HashMap::new(),
        functions: Vec::new(),
        errors: Vec::new(),
        loops: Vec::new(),
        skips: Vec::new(),
        gs1,
        era,
        line: 0,
        script_fns: seen.keys().map(|k| k.to_ascii_lowercase()).collect(),
    };
    for item in items {
        if item.top {
            // The skips over earlier functions land here, so code after a
            // function still runs in source order. With no code after the
            // last function they stay aimed at the end, as in the fixtures.
            let here = b.here();
            for skip in std::mem::take(&mut b.skips) {
                b.patch(skip, here);
            }
            for s in &item.body {
                b.stmt(s);
            }
        } else {
            b.function(item);
        }
    }
    b.op(Op::Ret);
    let end = b.here();
    for skip in std::mem::take(&mut b.skips) {
        b.patch(skip, end);
    }
    Ok(Built {
        functions: b.functions,
        strings: b.strings,
        code: b.code,
        errors: b.errors,
    })
}

impl Builder {
    fn here(&self) -> usize {
        self.code.len()
    }

    fn err(&mut self, message: impl Into<String>) {
        self.errors.push(CompileError {
            line: self.line,
            message: message.into(),
        });
    }

    fn op(&mut self, op: Op) {
        self.code.push(Instr {
            op,
            operand: Operand::None,
            offset: 0,
        });
    }

    fn hole(&mut self, op: Op) -> usize {
        let at = self.here();
        self.code.push(Instr {
            op,
            operand: Operand::Int(0),
            offset: 0,
        });
        at
    }

    fn patch(&mut self, at: usize, target: usize) {
        self.code[at].operand = Operand::Int(target as i32);
    }

    fn jump(&mut self, op: Op, target: usize) {
        self.code.push(Instr {
            op,
            operand: Operand::Int(target as i32),
            offset: 0,
        });
    }

    fn intern(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.index.get(s) {
            return i;
        }
        if s.chars().any(|c| (c as u32) > 255) {
            self.err("a string is outside latin1");
        }
        let i = self.strings.len() as u32;
        self.index.insert(s.to_string(), i);
        self.strings.push(s.to_string());
        i
    }

    fn emit_var(&mut self, name: &str) {
        let n = self.intern(name);
        self.code.push(Instr {
            op: Op::TypeVar,
            operand: Operand::Str(n),
            offset: 0,
        });
    }

    fn emit_str(&mut self, text: &str) {
        let n = self.intern(text);
        self.code.push(Instr {
            op: Op::TypeString,
            operand: Operand::Str(n),
            offset: 0,
        });
    }

    fn emit_int(&mut self, n: i32) {
        self.code.push(Instr {
            op: Op::TypeNumber,
            operand: Operand::Int(n),
            offset: 0,
        });
    }

    fn emit_number(&mut self, n: f64) {
        if n.is_finite() && n.fract() == 0.0 && (i32::MIN as f64..=i32::MAX as f64).contains(&n) {
            self.emit_int(n as i32);
        } else {
            let text = num_to_string(n);
            self.code.push(Instr {
                op: Op::TypeNumber,
                operand: Operand::Decimal(text),
                offset: 0,
            });
        }
    }

    fn function(&mut self, item: &Item) {
        let skip = self.hole(Op::SetIndex);
        let start = self.here();
        self.functions.push(Function {
            name: item.name.clone(),
            start,
        });
        self.op(Op::TypeArray);
        // A `temp.x` parameter is the place `temp.x` (TEMP, x, member
        // access in the Preagonal baselines).
        for p in item.params.iter().rev() {
            if p.contains('.') {
                self.place(p);
            } else {
                self.emit_var(p);
            }
        }
        self.op(Op::FuncParamsEnd);
        self.op(Op::Jmp);
        if self.body_has_call(&item.body) {
            self.op(Op::CmdCall);
        }
        for s in &item.body {
            self.stmt(s);
        }
        if !ends_with_return(&item.body) {
            self.emit_int(0);
            self.op(Op::Ret);
        }
        self.skips.push(skip);
    }

    fn body_has_call(&self, body: &[Stmt]) -> bool {
        body.iter().any(|s| self.stmt_has_call(s))
    }

    fn stmt_has_call(&self, s: &Stmt) -> bool {
        match s {
            Stmt::Block(v) => self.body_has_call(v),
            Stmt::If { cond, then, els } => {
                self.expr_has_call(cond)
                    || self.stmt_has_call(then)
                    || els.as_ref().is_some_and(|e| self.stmt_has_call(e))
            }
            Stmt::While { cond, body } => self.expr_has_call(cond) || self.stmt_has_call(body),
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                init.as_ref().is_some_and(|s| self.stmt_has_call(s))
                    || cond.as_ref().is_some_and(|c| self.expr_has_call(c))
                    || step.as_ref().is_some_and(|s| self.stmt_has_call(s))
                    || self.stmt_has_call(body)
            }
            Stmt::Function { body, .. } => self.body_has_call(body),
            Stmt::Return(v) => v.as_ref().is_some_and(|e| self.expr_has_call(e)),
            Stmt::Assign { target, value, .. } => {
                self.index_has_call(target) || self.expr_has_call(value)
            }
            Stmt::Call { name, args } => {
                self.name_is_call(name) || args.iter().any(|a| self.expr_has_call(a))
            }
            Stmt::Command { name, args, .. } => {
                self.name_is_call(name) || args.iter().any(|a| self.arg_has_call(a))
            }
            Stmt::With { target, body, .. } => {
                self.expr_has_call(target) || self.stmt_has_call(body)
            }
            Stmt::IncDec { target, .. } => self.index_has_call(target),
            Stmt::Break | Stmt::Continue | Stmt::Unknown { .. } => false,
            Stmt::Gs2(g) => {
                g.exprs().into_iter().any(|e| self.expr_has_call(e))
                    || g.bodies().into_iter().any(|s| self.stmt_has_call(s))
            }
        }
    }

    fn index_has_call(&self, v: &VarRef) -> bool {
        v.index.iter().any(|e| self.expr_has_call(e))
    }

    fn arg_has_call(&self, a: &Arg) -> bool {
        match a {
            Arg::Num(e) => self.expr_has_call(e),
            Arg::Text(_) => false,
        }
    }

    /// Why a GS1 array literal cannot be pushed in GS2's right-to-left
    /// order, or `None` when it can. The GS1 interpreter evaluates the
    /// elements left to right, so the order only matters for an element
    /// whose evaluation can change something (a script or host call, or
    /// `random`). The literal is lowered when no element is like that, or
    /// when exactly one is and every other element is a constant.
    fn gs1_list_problem(&self, values: &[Expr]) -> Option<&'static str> {
        if values.iter().any(has_nested_list) {
            return Some("GS1 array elements that are lists are not lowered");
        }
        let effects = values.iter().filter(|v| self.has_effect(v)).count();
        let constants = values
            .iter()
            .filter(|v| matches!(v, Expr::Num(_) | Expr::Str(_)))
            .count();
        if effects == 0 || (effects == 1 && constants == values.len() - 1) {
            None
        } else {
            Some(
                "GS1 array elements with calls are lowered only when one element calls \
                 and the others are constants (GS1 reads them left to right, GS2 pushes \
                 them right to left); put the values in variables first",
            )
        }
    }

    /// The expression calls script or host code, or draws a random number.
    fn has_effect(&self, e: &Expr) -> bool {
        self.expr_has_call(e) || mentions_call(e, "random")
    }

    fn expr_has_call(&self, e: &Expr) -> bool {
        match e {
            Expr::Call { name, args } => {
                self.name_is_call(name) || args.iter().any(|a| self.expr_has_call(a))
            }
            Expr::Unary(_, a) | Expr::InArray { e: a, .. } => self.expr_has_call(a),
            Expr::Bin(_, a, b) => self.expr_has_call(a) || self.expr_has_call(b),
            Expr::InRange { e, lo, hi } => {
                self.expr_has_call(e) || self.expr_has_call(lo) || self.expr_has_call(hi)
            }
            Expr::InList { e, list } => {
                self.expr_has_call(e) || list.iter().any(|a| self.expr_has_call(a))
            }
            Expr::Cond { cond, then, els } => {
                self.expr_has_call(cond) || self.expr_has_call(then) || self.expr_has_call(els)
            }
            Expr::List(v) => v.iter().any(|a| self.expr_has_call(a)),
            Expr::Gs2(g) => g.calls() || g.parts().into_iter().any(|a| self.expr_has_call(a)),
            Expr::Var(v) => self.index_has_call(v),
            Expr::Num(_) | Expr::Str(_) => false,
        }
    }

    fn name_is_call(&self, name: &str) -> bool {
        // `objects.gs2bc`: a body whose only call is `format` still gets
        // `OP_CMD_CALL`.
        if name == "format" {
            return true;
        }
        if opcode_builtin(name).is_some() {
            return false;
        }
        if self.gs1 && matches!(name, "strlen" | "startswith" | "indexof") {
            return false;
        }
        if self.gs1 && gs1_fn_without_gs2(name) {
            return false;
        }
        true
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Block(v) => {
                for s in v {
                    self.stmt(s);
                }
            }
            Stmt::If { cond, then, els } => self.if_stmt(cond, then, els.as_deref()),
            Stmt::While { cond, body } => self.while_stmt(cond, body),
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => self.for_stmt(init.as_deref(), cond.as_ref(), step.as_deref(), body),
            Stmt::Function { .. } => {
                self.err("a nested function is not lowered by this temporary front end")
            }
            Stmt::Return(v) => {
                if let Some(v) = v {
                    self.value(v);
                } else {
                    self.emit_int(0);
                }
                self.op(Op::Ret);
            }
            Stmt::Break => {
                if self.loops.is_empty() {
                    self.err("`break` outside a loop");
                    return;
                }
                let h = self.hole(Op::SetIndex);
                self.loops.last_mut().unwrap().breaks.push(h);
            }
            Stmt::Continue => {
                if self.loops.is_empty() {
                    self.err("`continue` outside a loop");
                    return;
                }
                let h = self.hole(Op::SetIndex);
                self.loops.last_mut().unwrap().continues.push(h);
            }
            Stmt::Assign { target, op, value } => self.assign(target, *op, value),
            Stmt::IncDec { target, delta } => {
                if !target.index.is_empty() && self.gs1 {
                    // GS1 `a[i]++` is `a[i] = a[i] + 1` (the same reads and
                    // writes, the element read once and written once).
                    let op = if *delta < 0.0 {
                        AssignOp::Sub
                    } else {
                        AssignOp::Add
                    };
                    self.element_assign(target, op, &Expr::Num(1.0));
                    return;
                }
                if !target.index.is_empty() {
                    // GS2 `a[i]++` (Preagonal probe): the element read
                    // (`OP_ARRAY`), then `OP_INC` on it.
                    let Some(dims) = self.element(target) else {
                        return;
                    };
                    self.op(if dims == 1 {
                        Op::Array
                    } else {
                        Op::ArrayMultidim
                    });
                } else {
                    self.place(&target.name);
                }
                self.op(if *delta < 0.0 { Op::Dec } else { Op::Inc });
                self.op(Op::IndexDec);
            }
            Stmt::Call { name, args } => {
                let pop = self.call_value(name, args);
                if pop {
                    self.op(Op::IndexDec);
                }
            }
            Stmt::Command { name, args, line } => self.command(name, args, *line),
            Stmt::With { target, body, line } => self.with_stmt(target, body, *line),
            Stmt::Unknown {
                name, reason, line, ..
            } => {
                self.line = *line;
                self.err(format!("`{name}` cannot be lowered ({reason})"));
            }
            Stmt::Gs2(g) => self.gs2_stmt(g),
        }
    }

    fn gs2_stmt(&mut self, g: &Gs2Stmt) {
        match g {
            Gs2Stmt::Eval(e) => {
                if let Expr::Gs2(m) = e {
                    if let Gs2Expr::Method { obj, name, args } = m.as_ref() {
                        if self.method(obj, name, args) {
                            self.op(Op::IndexDec);
                        }
                        return;
                    }
                }
                self.value(e);
                self.op(Op::IndexDec);
            }
            Gs2Stmt::ForEach { var, list, body } => self.foreach_stmt(var, list, body),
            // The member or element as a place, the value, `OP_ASSIGN` /
            // `OP_ARRAY_ASSIGN`, as for a dotted name or `a[i]`. Seihai's
            // encoding, no fixture.
            Gs2Stmt::SetPart { target, value } => match target {
                Expr::Gs2(g) => match g.as_ref() {
                    Gs2Expr::Member { .. } | Gs2Expr::DynMember { .. } => {
                        self.value(target);
                        self.value(value);
                        self.op(Op::Assign);
                    }
                    Gs2Expr::Index { obj, index } if index.len() <= 2 => {
                        self.value(obj);
                        self.op(Op::ConvToObject);
                        for i in index {
                            self.value(i);
                        }
                        self.value(value);
                        self.op(if index.len() == 1 {
                            Op::ArrayAssign
                        } else {
                            Op::ArrayMultidimAssign
                        });
                    }
                    _ => self.err("that assignment target is not lowered"),
                },
                // `makevar("a.b") = v` (Preagonal probe): the name,
                // `OP_MAKEVAR`, the value, `OP_ASSIGN`.
                Expr::Call { .. } => {
                    self.value(target);
                    self.value(value);
                    self.op(Op::Assign);
                }
                _ => self.err("that assignment target is not lowered"),
            },
            Gs2Stmt::Switch { value, arms } => self.switch_stmt(value, arms),
            Gs2Stmt::NewBlock { ty, name, body } => self.new_block(ty, name, body),
            // `a = b = c = v` as in the Preagonal baselines: `a`, then each
            // later target and `OP_COPY_LAST_OP`, the value, and one
            // `OP_ASSIGN` per target (innermost first).
            Gs2Stmt::Chain { targets, value } => {
                if targets.iter().any(|t| !t.index.is_empty()) {
                    self.err("`a[i] = b = …` is not lowered");
                    return;
                }
                for (i, t) in targets.iter().enumerate() {
                    self.place(&t.name);
                    if i > 0 {
                        self.op(Op::CopyLastOp);
                    }
                }
                self.value(value);
                for _ in targets {
                    self.op(Op::Assign);
                }
            }
        }
    }

    /// `for (v : list)` as in `lists.gs2bc`: the variable, the list as an
    /// object and a counter 0 stay on the stack; `OP_FOREACH` writes the next
    /// element or jumps out, `OP_INC` steps the counter, and one
    /// `OP_INDEX_DEC` follows the loop. A `continue` goes to the `OP_INC`.
    fn foreach_stmt(&mut self, var: &VarRef, list: &Expr, body: &Stmt) {
        if !var.index.is_empty() {
            self.err("a for-each variable must be a plain name");
            return;
        }
        self.place(&var.name);
        self.value(list);
        self.op(Op::ConvToObject);
        self.emit_int(0);
        let top = self.hole(Op::ForEach);
        self.op(Op::CmdCall);
        self.loops.push(Loop {
            continues: Vec::new(),
            breaks: Vec::new(),
        });
        self.stmt(body);
        let frame = self.loops.pop().unwrap();
        let step = self.here();
        for c in frame.continues {
            self.patch(c, step);
        }
        self.op(Op::Inc);
        self.jump(Op::SetIndex, top);
        let end = self.here();
        self.patch(top, end);
        for b in frame.breaks {
            self.patch(b, end);
        }
        self.op(Op::IndexDec);
    }

    /// `switch` as in `control.gs2bc`: a jump to the tests, the arm bodies in
    /// source order (falling through), then the value, one
    /// `OP_COPY_LAST_OP`, case value, `OP_EQ`, `OP_SET_INDEX_TRUE` per case,
    /// a jump to `default` (or out), and one `OP_INDEX_DEC` that drops the
    /// value. `break` goes to that `OP_INDEX_DEC`.
    ///
    /// Seihai adds one jump out after the last body. Without it a last arm
    /// that does not `break` would run on into the tests again; the fixture's
    /// `default` arm does that.
    fn switch_stmt(&mut self, value: &Expr, arms: &[SwitchArm]) {
        let to_tests = self.hole(Op::SetIndex);
        self.loops.push(Loop {
            continues: Vec::new(),
            breaks: Vec::new(),
        });
        let mut starts = Vec::new();
        for arm in arms {
            starts.push(self.here());
            for s in &arm.body {
                self.stmt(s);
            }
        }
        let falls_off = arms
            .last()
            .is_some_and(|a| !matches!(a.body.last(), Some(Stmt::Break | Stmt::Return(_))));
        let out = falls_off.then(|| self.hole(Op::SetIndex));
        let frame = self.loops.pop().unwrap();
        self.patch(to_tests, self.here());
        self.value(value);
        for (arm, &start) in arms.iter().zip(&starts) {
            for v in &arm.values {
                self.op(Op::CopyLastOp);
                self.value(v);
                self.op(Op::Eq);
                self.jump(Op::SetIndexTrue, start);
            }
        }
        let default = arms
            .iter()
            .zip(&starts)
            .find(|(a, _)| a.default)
            .map(|(_, &s)| s);
        let to_default = self.hole(Op::SetIndex);
        let end = self.here();
        self.patch(to_default, default.unwrap_or(end));
        self.op(Op::IndexDec);
        for b in frame.breaks.into_iter().chain(out) {
            self.patch(b, end);
        }
        if !frame.continues.is_empty() {
            match self.loops.last_mut() {
                Some(outer) => outer.continues.extend(frame.continues),
                None => self.err("`continue` outside a loop"),
            }
        }
    }

    /// A method call `obj.name(args)`. The built-in methods use the opcodes
    /// and operand orders in `strings.gs2bc`, `lists.gs2bc` and
    /// `rest.gs2bc`; any other name is a call on the object
    /// (`objects.gs2bc`: the arguments reversed, the object, the name,
    /// `OP_MEMBER_ACCESS`, `OP_CALL`). Returns whether a value was pushed.
    fn method(&mut self, obj: &Expr, name: &str, args: &[Expr]) -> bool {
        let lower = name.to_ascii_lowercase();
        let n = args.len();
        let string_op = match (lower.as_str(), n) {
            ("trim", 0) => Some(Op::ObjTrim),
            ("length", 0) => Some(Op::ObjLength),
            ("pos", 1) => Some(Op::ObjPos),
            ("charat", 1) => Some(Op::ObjCharAt),
            ("substring", 1 | 2) => Some(Op::ObjSubstr),
            ("starts", 1) => Some(Op::ObjStarts),
            ("ends", 1) => Some(Op::ObjEnds),
            ("positions", 1) => Some(Op::ObjPositions),
            ("tokenize", 0 | 1) => Some(Op::ObjTokenize),
            _ => None,
        };
        if let Some(op) = string_op {
            // `lists.gs2bc` converts even a string literal.
            self.value(obj);
            self.op(Op::ConvToString);
            for a in args {
                self.value(a);
            }
            if op == Op::ObjTokenize && n == 0 {
                self.emit_str(" ,");
            }
            self.op(op);
            return true;
        }
        // List methods: the object as an object, then the operands. The
        // two-operand ones push the value before the index.
        let list_op = match (lower.as_str(), n) {
            ("size", 0) => Some((Op::ObjSize, true)),
            ("type", 0) => Some((Op::ObjType, true)),
            ("clear", 0) => Some((Op::ObjClear, false)),
            ("index", 1) => Some((Op::ObjIndex, true)),
            ("add", 1) => Some((Op::ObjAddString, false)),
            ("delete", 1) => Some((Op::ObjDeleteString, false)),
            ("remove", 1) => Some((Op::ObjRemoveString, false)),
            ("insert", 2) => Some((Op::ObjInsertString, false)),
            ("replace", 2) => Some((Op::ObjReplaceString, false)),
            ("subarray", 1 | 2) => Some((Op::ObjSubarray, true)),
            _ => None,
        };
        if let Some((op, pushes)) = list_op {
            self.value(obj);
            self.op(Op::ConvToObject);
            if matches!(op, Op::ObjInsertString | Op::ObjReplaceString) {
                self.value(&args[1]);
                self.value(&args[0]);
            } else {
                for a in args {
                    self.value(a);
                }
            }
            self.op(op);
            return pushes;
        }
        if lower == "indices" && n == 1 {
            self.value(&args[0]);
            self.value(obj);
            self.op(Op::ObjIndices);
            return true;
        }
        self.op(Op::TypeArray);
        for a in args.iter().rev() {
            self.value(a);
        }
        self.object(obj);
        self.emit_var(name);
        self.op(Op::MemberAccess);
        self.op(Op::Call);
        true
    }

    /// A value used as the object of a member or method: converted with
    /// `OP_CONV_TO_OBJECT` unless it is a root (`this`, `player`, …),
    /// as the Preagonal baselines do for dotted names, calls and members.
    fn object(&mut self, obj: &Expr) {
        self.value(obj);
        let root = matches!(obj, Expr::Var(v) if v.index.is_empty() && root_op(&v.name).is_some());
        if !root {
            self.op(Op::ConvToObject);
        }
    }

    fn gs2_value(&mut self, g: &Gs2Expr) -> Kind {
        match g {
            Gs2Expr::Method { obj, name, args } => {
                if !self.method(obj, name, args) {
                    // A list method that leaves nothing, used as a value.
                    self.op(Op::TypeNull);
                }
                Kind::Other
            }
            Gs2Expr::Member { obj, name } => {
                self.object(obj);
                self.emit_var(name);
                self.op(Op::MemberAccess);
                Kind::Other
            }
            Gs2Expr::Index { obj, index } => {
                // `a[i][j]`: no conversion between the two reads in the
                // Preagonal baselines.
                self.value(obj);
                if !matches!(obj, Expr::Var(v) if !v.index.is_empty())
                    && !matches!(obj, Expr::Gs2(g) if matches!(g.as_ref(), Gs2Expr::Index { .. }))
                {
                    self.op(Op::ConvToObject);
                }
                for i in index {
                    self.value(i);
                }
                self.op(if index.len() == 1 {
                    Op::Array
                } else {
                    Op::ArrayMultidim
                });
                Kind::Other
            }
            // `objects.gs2bc`: `new TStaticVar()` pushes the name
            // "unknown_object", the type, then `OP_NEW_OBJECT`. The type is
            // interned first. With an argument, Seihai pushes it in place of
            // the default name (no fixture).
            Gs2Expr::New { ty, args } => {
                let t = self.intern(ty);
                match args.first() {
                    Some(a) => {
                        self.value(a);
                    }
                    None => self.emit_var("unknown_object"),
                }
                self.code.push(Instr {
                    op: Op::TypeString,
                    operand: Operand::Str(t),
                    offset: 0,
                });
                self.op(Op::NewObject);
                Kind::Other
            }
            Gs2Expr::NewArray(dims) => {
                for (i, d) in dims.iter().enumerate() {
                    self.value(d);
                    self.op(if i == 0 {
                        Op::ArrayNew
                    } else {
                        Op::ArrayNewMultidim
                    });
                }
                Kind::Other
            }
            // `5 & 3` is the two numbers and `OP_BWA` (Preagonal probe).
            Gs2Expr::Bits { kind, a, b } => {
                self.numeric(a);
                self.numeric(b);
                self.op(match kind {
                    BitKind::Shl => Op::ShiftLeft,
                    BitKind::Shr => Op::ShiftRight,
                    BitKind::And => Op::BitAnd,
                    BitKind::Or => Op::BitOr,
                });
                Kind::Number
            }
            // `(@x).y` pushes x and `OP_CONV_TO_STRING` (Preagonal probe).
            Gs2Expr::Text(e) => {
                self.value(e);
                self.op(Op::ConvToString);
                Kind::Other
            }
            // `obj.(n)`: the object, the name as a string, member access.
            // Seihai's form: the Preagonal output for `obj.(@n)` adds the
            // name to the object as numbers.
            Gs2Expr::DynMember { obj, name } => {
                self.object(obj);
                self.value(name);
                self.op(Op::ConvToString);
                self.op(Op::MemberAccess);
                Kind::Other
            }
            // `y = x++` (Preagonal probe): x, `OP_COPY_LAST_OP`,
            // `OP_CONV_TO_FLOAT`, `OP_SWAP_LAST_OPS`, `OP_INC`,
            // `OP_INDEX_DEC`, leaving the old number.
            Gs2Expr::PostStep { target, delta } => {
                if !self.step_place(target) {
                    return Kind::Other;
                }
                self.op(Op::CopyLastOp);
                self.op(Op::ConvToFloat);
                self.op(Op::SwapLastOps);
                self.op(if *delta < 0.0 { Op::Dec } else { Op::Inc });
                self.op(Op::IndexDec);
                Kind::Number
            }
            // `y = (x = 2)` (Preagonal probe): x, `OP_COPY_LAST_OP`, the
            // value, `OP_ASSIGN`, leaving x. For `+=` and the others Seihai
            // copies the place once more, so x is still there after the
            // store (the Preagonal output does not, and leaves nothing).
            Gs2Expr::AssignValue { target, op, value } => {
                if !self.step_place(target) {
                    return Kind::Other;
                }
                self.op(Op::CopyLastOp);
                if *op == AssignOp::Set {
                    self.value(value);
                } else {
                    self.op(Op::CopyLastOp);
                    self.op(Op::ConvToFloat);
                    self.numeric(value);
                    self.op(match op {
                        AssignOp::Add => Op::Add,
                        AssignOp::Sub => Op::Sub,
                        AssignOp::Mul => Op::Mul,
                        AssignOp::Div => Op::Div,
                        AssignOp::Mod => Op::Mod,
                        AssignOp::Set => unreachable!(),
                    });
                }
                self.op(Op::Assign);
                Kind::Other
            }
        }
    }

    /// The place an expression names, for `x++` or `(x = v)` used as a
    /// value: a plain or dotted name, `makevar(…)`, or a member.
    fn step_place(&mut self, target: &Expr) -> bool {
        match target {
            Expr::Var(v) if v.index.is_empty() => {
                self.place(&v.name);
                true
            }
            Expr::Var(v) => {
                let Some(dims) = self.element(v) else {
                    return false;
                };
                self.op(if dims == 1 {
                    Op::Array
                } else {
                    Op::ArrayMultidim
                });
                true
            }
            Expr::Call { name, .. } if name == "makevar" => {
                self.value(target);
                true
            }
            Expr::Gs2(g)
                if matches!(
                    g.as_ref(),
                    Gs2Expr::Member { .. } | Gs2Expr::DynMember { .. }
                ) =>
            {
                self.value(target);
                true
            }
            _ => {
                self.err("that is not something `++`, `--` or `=` can change");
                false
            }
        }
    }

    fn if_stmt(&mut self, cond: &Expr, then: &Stmt, els: Option<&Stmt>) {
        let mut fails = Vec::new();
        self.cond_fail(cond, &mut fails);
        self.stmt(then);
        if let Some(els) = els {
            let jump = self.hole(Op::SetIndex);
            let else_at = self.here();
            for f in fails {
                self.patch(f, else_at);
            }
            self.stmt(els);
            self.patch(jump, self.here());
        } else {
            let end = self.here();
            for f in fails {
                self.patch(f, end);
            }
        }
    }

    fn while_stmt(&mut self, cond: &Expr, body: &Stmt) {
        let test = self.here();
        let mut fails = Vec::new();
        self.cond_fail(cond, &mut fails);
        self.op(Op::CmdCall);
        self.loops.push(Loop {
            continues: Vec::new(),
            breaks: Vec::new(),
        });
        self.stmt(body);
        let frame = self.loops.pop().unwrap();
        for c in frame.continues {
            self.patch(c, test);
        }
        self.jump(Op::SetIndex, test);
        let end = self.here();
        for f in fails {
            self.patch(f, end);
        }
        for b in frame.breaks {
            self.patch(b, end);
        }
    }

    fn for_stmt(
        &mut self,
        init: Option<&Stmt>,
        cond: Option<&Expr>,
        step: Option<&Stmt>,
        body: &Stmt,
    ) {
        if let Some(init) = init {
            self.stmt(init);
        }
        let test = self.here();
        let mut fails = Vec::new();
        if let Some(cond) = cond {
            self.cond_fail(cond, &mut fails);
        }
        self.op(Op::CmdCall);
        self.loops.push(Loop {
            continues: Vec::new(),
            breaks: Vec::new(),
        });
        self.stmt(body);
        let frame = self.loops.pop().unwrap();
        let step_at = self.here();
        for c in frame.continues {
            self.patch(c, step_at);
        }
        if let Some(step) = step {
            self.stmt(step);
        }
        self.jump(Op::SetIndex, test);
        let end = self.here();
        for f in fails {
            self.patch(f, end);
        }
        for b in frame.breaks {
            self.patch(b, end);
        }
    }

    fn assign(&mut self, target: &VarRef, op: AssignOp, value: &Expr) {
        if !target.index.is_empty() {
            self.element_assign(target, op, value);
            return;
        }
        self.place(&target.name);
        if op == AssignOp::Set {
            self.value(value);
            self.op(Op::Assign);
            return;
        }
        self.op(Op::CopyLastOp);
        self.op(Op::ConvToFloat);
        self.numeric(value);
        self.op(match op {
            AssignOp::Add => Op::Add,
            AssignOp::Sub => Op::Sub,
            AssignOp::Mul => Op::Mul,
            AssignOp::Div => Op::Div,
            AssignOp::Mod => Op::Mod,
            AssignOp::Set => unreachable!(),
        });
        self.op(Op::Assign);
    }

    /// `a[i] = v` and `a[i, j] = v`: object, indexes, value, assign op.
    fn element_assign(&mut self, target: &VarRef, op: AssignOp, value: &Expr) {
        if op != AssignOp::Set {
            // `a[i] += v` is `a[i] = a[i] + v`. For GS2 this is Seihai's
            // form: the Preagonal (gs2-parser) output copies the index, not
            // the element, and so stores `i + v`.
            // GS1 `a[i] += v` is `a[i] = a[i] + v`: the GS1 interpreter
            // reads the element, works the number out and stores it, and
            // the lowered form does the same with the fixture-checked
            // element read and write. The index is pushed twice, so it
            // must not call anything.
            if self.index_has_call(target) {
                self.err(format!(
                    "`{}` on an array element whose index calls a function is not lowered; \
                     put the index in a variable first",
                    op.symbol()
                ));
                return;
            }
            let bin = match op {
                AssignOp::Add => BinOp::Add,
                AssignOp::Sub => BinOp::Sub,
                AssignOp::Mul => BinOp::Mul,
                AssignOp::Div => BinOp::Div,
                AssignOp::Mod => BinOp::Mod,
                AssignOp::Set => unreachable!(),
            };
            let sum = Expr::Bin(
                bin,
                Box::new(Expr::Var(target.clone())),
                Box::new(value.clone()),
            );
            self.element_assign(target, AssignOp::Set, &sum);
            return;
        }
        if self.gs1 && matches!(value, Expr::List(_)) {
            self.err("a list stored into an array element is not lowered");
            return;
        }
        let Some(dims) = self.element(target) else {
            return;
        };
        self.value(value);
        self.op(if dims == 1 {
            Op::ArrayAssign
        } else {
            Op::ArrayMultidimAssign
        });
    }

    /// Pushes an indexed variable as an object followed by its indexes and
    /// returns how many indexes there are, or `None` after an error.
    fn element(&mut self, v: &VarRef) -> Option<usize> {
        if let Some(problem) = element_problem(v, self.gs1) {
            self.err(problem);
            return None;
        }
        self.place(&v.name);
        self.op(Op::ConvToObject);
        for i in &v.index {
            self.numeric(i);
        }
        Some(v.index.len())
    }

    /// `new T(name) { body }` as the Preagonal compiler's baselines encode
    /// it (behaviour only): the name, `OP_INLINE_NEW`, three
    /// `OP_COPY_LAST_OP`, the type, `OP_CONV_TO_STRING`, `OP_NEW_OBJECT`,
    /// `OP_ASSIGN` (the name now holds the object), `OP_CONV_TO_OBJECT`,
    /// `OP_WITH` past the block, the body, `OP_WITHEND`, then
    /// `addcontrol(object)` on the object outside and `OP_INDEX_DEC`.
    fn new_block(&mut self, ty: &str, name: &Expr, body: &[Stmt]) {
        if let Some(problem) = body.iter().find_map(|s| with_escape(s, false, !self.gs1)) {
            self.err(problem);
            return;
        }
        self.value(name);
        self.op(Op::InlineNew);
        for _ in 0..3 {
            self.op(Op::CopyLastOp);
        }
        let t = self.intern(ty);
        self.code.push(Instr {
            op: Op::TypeString,
            operand: Operand::Str(t),
            offset: 0,
        });
        self.op(Op::ConvToString);
        self.op(Op::NewObject);
        self.op(Op::Assign);
        self.op(Op::ConvToObject);
        let hole = self.hole(Op::With);
        let outer = self.block_loops();
        for s in body {
            self.stmt(s);
        }
        self.restore_loops(outer);
        self.op(Op::WithEnd);
        let after = self.here();
        self.patch(hole, after);
        self.op(Op::TypeArray);
        self.op(Op::SwapLastOps);
        self.emit_var("addcontrol");
        self.op(Op::Call);
        self.op(Op::IndexDec);
    }

    /// A `TYPE_VAR name`, `OP_CALL` pair from `start` on whose name is one
    /// of the script's own functions.
    fn calls_script_fn(&self, start: usize) -> bool {
        self.code[start..].windows(2).any(|w| {
            w[1].op == Op::Call
                && w[0].op == Op::TypeVar
                && matches!(&w[0].operand, Operand::Str(i)
                    if self.script_fns.contains(&self.strings[*i as usize].to_ascii_lowercase()))
        })
    }

    /// The loops a `with` body may leave. GS1 refuses leaving the block
    /// (checked before), so its body starts with none; GS2 keeps them.
    fn block_loops(&mut self) -> Option<Vec<Loop>> {
        self.gs1.then(|| std::mem::take(&mut self.loops))
    }

    fn restore_loops(&mut self, outer: Option<Vec<Loop>>) {
        if let Some(outer) = outer {
            self.loops = outer;
        }
    }

    /// `with (target) body` in the fixture's encoding (`rest.gs2bc`,
    /// `objects.gs2bc`): the target, `OP_CONV_TO_OBJECT`, `OP_WITH` to just
    /// past the block, the body, `OP_WITHEND`.
    fn with_stmt(&mut self, target: &Expr, body: &Stmt, line: u32) {
        self.line = line;
        if let Some(problem) = with_escape(body, false, !self.gs1) {
            self.err(problem);
            return;
        }
        if self.gs1 {
            if let Some(problem) = self.gs1_with_problem(body) {
                self.err(problem);
                return;
            }
        }
        self.value(target);
        self.op(Op::ConvToObject);
        let hole = self.hole(Op::With);
        let outer = self.block_loops();
        let start = self.here();
        self.stmt(body);
        self.restore_loops(outer);
        // A GS1 command inside `with` acts on the swapped player or NPC
        // (`tools/npcserver-2001.md`), and a GS2 call inside `with` runs
        // against the `with` object, so a command lowered to a call by name
        // keeps its meaning. One lowered to an explicit `player` does not:
        // GS2's `player` is not swapped.
        // The notes also say the script's own functions still run as the
        // original NPC, where GS2 would look the name up on the object
        // first, so a call to one of them is refused.
        if self.gs1 && self.calls_script_fn(start) {
            self.err(
                "a call to one of the script's own functions inside a GS1 `with` is not lowered \
                 (GS1 runs it as the original NPC, GS2 looks it up on the `with` object first)",
            );
        }
        if self.gs1
            && self.code[start..]
                .iter()
                .any(|i| matches!(i.op, Op::Player | Op::PlayerO))
        {
            self.err(
                "a GS1 command on the player inside `with` is not lowered \
                 (GS1 swaps the player there, GS2's `player` stays the same)",
            );
        }
        self.op(Op::WithEnd);
        let after = self.here();
        self.patch(hole, after);
    }

    /// Why a GS1 `with` body cannot be lowered, if it cannot. The NPC
    /// server's notes (`npcserverdoc.txt`, NW beta 1 and 2) say that inside
    /// `with` the target's `this.` variables and properties are read and
    /// written, plain script variables (`i`, `j`) keep working, and script
    /// functions still run as the original NPC. GS2 resolves a plain name on
    /// the `with` object first and runs a bare call against it, so a body
    /// whose meaning would change is refused: a write to a plain name that
    /// is not a built-in property, and the `player`, `client.` and `level.`
    /// names (GS1 switches them with the object, GS2 does not). Calls to the
    /// script's own functions and commands that reach the player are
    /// refused after the body is emitted (`with_stmt`); other commands act
    /// on the `with` object in both.
    fn gs1_with_problem(&self, body: &Stmt) -> Option<String> {
        let mut problem = None;
        let era = self.era;
        stmt_names(body, &mut |name, write| {
            if problem.is_some() {
                return;
            }
            let lower = name.to_ascii_lowercase();
            if lower.starts_with("player")
                || lower.starts_with("client.")
                || lower.starts_with("level.")
            {
                problem = Some(format!(
                    "`{name}` inside a GS1 `with` is not lowered (GS1 switches it with the object, GS2 does not)"
                ));
            } else if write && !name.contains('.') && vocab::builtin_var(name, era).is_none() {
                problem = Some(format!(
                    "writing the script variable `{name}` inside a GS1 `with` is not lowered \
                     (GS2 would store it on the `with` object); use `this.{name}` outside the block"
                ));
            }
        });
        problem
    }

    fn command(&mut self, name: &str, args: &[Arg], line: u32) {
        self.line = line;
        if !self.builtin_arity(name, args.len()) {
            return;
        }
        if name == "setarray" {
            self.setarray_command(args);
            return;
        }
        if name == "sleep" {
            if let Some(Arg::Num(e)) = args.first() {
                self.numeric(e);
            } else {
                self.emit_int(0);
            }
            self.op(Op::Sleep);
            return;
        }
        if self.gs1 && self.gs1_string_command(name, args) {
            return;
        }
        if self.gs1 && !gs2_has_command(name) {
            self.err(format!(
                "GS1 command `{name}` has no GS2 equivalent{}",
                command_hint(name)
            ));
            return;
        }
        // The first word of `setplayerprop` names the property (`#c`); it
        // is not a message to expand.
        if self.gs1 && name == "setplayerprop" {
            self.op(Op::TypeArray);
            for (i, a) in args.iter().enumerate().rev() {
                match a {
                    Arg::Text(t) if i == 0 => self.emit_str(t),
                    a => {
                        self.value(&arg_to_expr(a));
                    }
                }
            }
            self.emit_var(name);
            self.op(Op::Call);
            self.op(Op::IndexDec);
            return;
        }
        let exprs: Vec<Expr> = args.iter().map(arg_to_expr).collect();
        self.call_ops(name, &exprs);
        self.op(Op::IndexDec);
    }

    /// GS1 commands that become GS2 statements rather than a call by name.
    /// Returns whether `name` was one of them (lowered or reported).
    ///
    /// - `setstring name,value`: `this.` and `client.` names become an
    ///   assignment. Any other name is a player flag in GS1 (Seihai's GS1
    ///   engine writes it as a flag), so it becomes `set("name=value")`, or
    ///   `unset("name")` for a literal empty value.
    /// - `addstring`, `insertstring`, `replacestring`, `removestring`,
    ///   `deletestring` (2.1 lists) on `this.`/`client.` names: the GS2 list
    ///   methods. Before 2.1 `addstring` appends text.
    /// - `tokenize text` / `tokenize2 text,seps`: `tokens = text.tokenize()`,
    ///   which `#t(i)` then reads as `tokens[i]`.
    /// - `setcharprop code,value`: the NPC's own property (`#c` chat, `#n`
    ///   nick, `#1` sword, `#2` shield, `#3` head, `#8` body, `#C0`–`#C4`
    ///   colors, `#P1`–`#P30` attributes) as `this.<prop> = value`. The
    ///   GS2 property names are Seihai's reading, not fixture-checked.
    fn gs1_string_command(&mut self, name: &str, args: &[Arg]) -> bool {
        let text_name = |a: Option<&Arg>| match a {
            Some(Arg::Text(t)) => {
                let t = t.trim().to_ascii_lowercase();
                (is_plain_name(&t) && !t.is_empty()).then_some(t)
            }
            _ => None,
        };
        let value = |a: Option<&Arg>| a.map(arg_to_expr).unwrap_or(Expr::Str(String::new()));
        let local = |n: &str| n.starts_with("this.") || n.starts_with("client.");
        match name {
            "setstring" => {
                let Some(var) = text_name(args.first()) else {
                    self.err("`setstring` needs a plain string name");
                    return true;
                };
                let v = value(args.get(1));
                if local(&var) {
                    self.assign(&plain_var(&var), AssignOp::Set, &v);
                } else if matches!(&v, Expr::Str(s) if s.is_empty()) {
                    self.call_ops("unset", &[Expr::Str(var)]);
                    self.op(Op::IndexDec);
                } else {
                    let text = match &v {
                        Expr::Str(s) if !s.contains('#') => Expr::Str(format!("{var}={s}")),
                        _ => Expr::Bin(
                            BinOp::Cat,
                            Box::new(Expr::Str(format!("{var}="))),
                            Box::new(v),
                        ),
                    };
                    self.call_ops("set", &[text]);
                    self.op(Op::IndexDec);
                }
                true
            }
            "addstring" | "insertstring" | "replacestring" | "removestring" | "deletestring" => {
                let Some(var) = text_name(args.first()) else {
                    self.err(format!("`{name}` needs a plain string name"));
                    return true;
                };
                if !local(&var) {
                    self.err(format!(
                        "`{name}` on `{var}`: only `this.` and `client.` lists are lowered (a plain GS1 string is a player flag)"
                    ));
                    return true;
                }
                if name == "addstring" && self.era < Era::V21 {
                    let v = Expr::Bin(
                        BinOp::Cat,
                        Box::new(Expr::Var(plain_var(&var))),
                        Box::new(value(args.get(1))),
                    );
                    self.assign(&plain_var(&var), AssignOp::Set, &v);
                    return true;
                }
                let method = match name {
                    "addstring" => "add",
                    "insertstring" => "insert",
                    "replacestring" => "replace",
                    "removestring" => "remove",
                    _ => "delete",
                };
                let margs: Vec<Expr> = args.iter().skip(1).map(arg_to_expr).collect();
                if self.method(&Expr::Var(plain_var(&var)), method, &margs) {
                    self.op(Op::IndexDec);
                }
                true
            }
            "tokenize" | "tokenize2" => {
                let text = value(args.first());
                let margs = if name == "tokenize2" {
                    vec![Expr::Bin(
                        BinOp::Cat,
                        Box::new(Expr::Str(" ,".into())),
                        Box::new(value(args.get(1))),
                    )]
                } else {
                    Vec::new()
                };
                let call = Expr::Gs2(Box::new(Gs2Expr::Method {
                    obj: text,
                    name: "tokenize".into(),
                    args: margs,
                }));
                self.assign(&plain_var("tokens"), AssignOp::Set, &call);
                true
            }
            "setcharprop" => {
                let code = match args.first() {
                    Some(Arg::Text(t)) => t.trim().to_string(),
                    _ => String::new(),
                };
                let Some(target) = char_prop(&code) else {
                    self.err(format!("`setcharprop {code}` has no GS2 property here"));
                    return true;
                };
                self.assign(&target, AssignOp::Set, &value(args.get(1)));
                true
            }
            _ => false,
        }
    }

    /// GS1 text with message codes, as GS2 joins (`@`). Codes Seihai's GS1
    /// engine expands (`scripting::engine::interp`) are lowered where GS2
    /// has the same value; the others are errors. A `#` that starts no code
    /// stays text, as in GS1.
    fn gs1_text(&mut self, s: &str) -> Kind {
        let parts = match split_codes(s, self.era) {
            Ok(p) => p,
            Err(e) => {
                self.err(e);
                self.emit_str(s);
                return Kind::String;
            }
        };
        if let [TextPart::Lit(t)] = parts.as_slice() {
            self.emit_str(t);
            return Kind::String;
        }
        for (i, part) in parts.iter().enumerate() {
            match part {
                TextPart::Lit(t) => self.emit_str(t),
                TextPart::Expr(e) => self.stringish(e),
                TextPart::Trim(t) => {
                    self.gs1_text(t);
                    self.op(Op::ConvToString);
                    self.op(Op::ObjTrim);
                }
                TextPart::Sub(start, len, t) => {
                    self.gs1_text(t);
                    self.op(Op::ConvToString);
                    self.numeric(start);
                    self.numeric(len);
                    self.op(Op::ObjSubstr);
                }
            }
            if i > 0 {
                self.op(Op::Join);
            }
        }
        Kind::String
    }

    fn setarray_command(&mut self, args: &[Arg]) {
        let Some(Arg::Text(name)) = args.first() else {
            self.err("`setarray` needs a variable name");
            return;
        };
        if !is_plain_name(name) {
            self.err("`setarray` needs a plain variable name");
            return;
        }
        self.place(name);
        self.op(Op::ConvToObject);
        if let Some(Arg::Num(n)) = args.get(1) {
            self.numeric(n);
        } else {
            self.emit_int(0);
        }
        self.op(Op::SetArray);
    }

    /// Emits a call used as a value. Returns whether a statement should pop it.
    fn call_value(&mut self, name: &str, args: &[Expr]) -> bool {
        if self.known_builtin(name, args) {
            return call_pushes(name);
        }
        if self.gs1 && self.method_builtin(name, args) {
            return true;
        }
        // `strtofloat(s)` is the number in s; `strcontains(a, b)` is
        // `a.pos(b) >= 0`.
        if self.gs1 && name == "strtofloat" && args.len() == 1 {
            self.numeric(&args[0]);
            return true;
        }
        if self.gs1 && name == "strcontains" && args.len() == 2 {
            self.stringish(&args[0]);
            self.value(&args[1]);
            self.op(Op::ObjPos);
            self.op(Op::ConvToFloat);
            self.emit_int(0);
            self.op(Op::Gte);
            return true;
        }
        // GS1 `getplayer(account)` (2.17.1 on, used as `with (getplayer(a))`)
        // is the player with that account; the 3.153 client registers
        // `findplayer` for the same lookup.
        if self.gs1 && name == "getplayer" && args.len() == 1 {
            self.call_ops("findplayer", args);
            return true;
        }
        if self.gs1 && gs1_fn_without_gs2(name) {
            self.err(format!("GS1 function `{name}` has no GS2 equivalent"));
            return false;
        }
        self.call_ops(name, args);
        true
    }

    fn call_ops(&mut self, name: &str, args: &[Expr]) {
        self.op(Op::TypeArray);
        for a in args.iter().rev() {
            self.value(a);
        }
        self.emit_var(name);
        self.op(Op::Call);
    }

    fn known_builtin(&mut self, name: &str, args: &[Expr]) -> bool {
        if !self.builtin_arity(name, args.len()) {
            // It is a recognized but invalid built-in, not a host call.
            return true;
        }
        match name {
            "sleep" => {
                if let Some(a) = args.first() {
                    self.numeric(a);
                }
                self.op(Op::Sleep);
                true
            }
            "setarray" => {
                if let Some(a) = args.first() {
                    self.value(a);
                    self.op(Op::ConvToObject);
                }
                if let Some(n) = args.get(1) {
                    self.numeric(n);
                }
                self.op(Op::SetArray);
                true
            }
            "format" => {
                self.op(Op::TypeArray);
                for a in args.iter().rev() {
                    self.value(a);
                }
                self.op(Op::Format);
                true
            }
            "arraylen" | "sarraylen" => {
                if let Some(a) = args.first() {
                    self.numeric(a);
                }
                self.op(Op::ObjSize);
                true
            }
            "int" => {
                if let Some(a) = args.first() {
                    self.numeric(a);
                }
                self.op(Op::Int);
                true
            }
            // `rest.gs2bc`: `getangle(1, -1)` and `getdir(-3, 1)` push their
            // operands in source order; the others are reversed.
            "getangle" | "getdir" => {
                for a in args {
                    self.numeric(a);
                }
                self.op(opcode_builtin(name).unwrap());
                true
            }
            // `strings.gs2bc`: `_("text")` is the text, then `OP_TRANSLATE`.
            "_" if !self.gs1 && args.len() == 1 => {
                self.value(&args[0]);
                self.op(Op::Translate);
                true
            }
            "sin" | "cos" | "arctan" | "abs" | "random" | "exp" | "log" | "min" | "max" | "pow"
            | "char" | "vecx" | "vecy" | "waitfor" | "makevar" => {
                for a in args.iter().rev() {
                    self.numeric(a);
                }
                self.op(opcode_builtin(name).unwrap());
                true
            }
            _ => false,
        }
    }

    fn method_builtin(&mut self, name: &str, args: &[Expr]) -> bool {
        let op = match name {
            "strlen" => Op::ObjLength,
            "startswith" => Op::ObjStarts,
            "indexof" => Op::ObjPos,
            _ => return false,
        };
        let Some(obj) = args.first() else {
            self.err(format!("`{name}` needs a string"));
            return true;
        };
        self.stringish(obj);
        for a in args.iter().skip(1) {
            self.value(a);
        }
        self.op(op);
        true
    }

    /// Seihai's accepted call shapes for the documented opcode subset.
    /// Host calls and script functions keep their own argument contracts.
    fn builtin_arity(&mut self, name: &str, actual: usize) -> bool {
        let required = match name {
            "sleep" | "sin" | "cos" | "arctan" | "abs" | "arraylen" | "sarraylen" | "int"
            | "exp" | "char" | "vecx" | "vecy" | "makevar" => 1,
            "random" | "setarray" | "log" | "min" | "max" | "pow" | "getangle" | "getdir" => 2,
            "waitfor" => 3,
            "strlen" if self.gs1 => 1,
            "startswith" | "indexof" if self.gs1 => 2,
            "format" => {
                if actual > 0 {
                    return true;
                }
                self.err("`format` expects at least 1 argument, got 0");
                return false;
            }
            _ => return true,
        };
        if actual == required {
            return true;
        }
        self.err(format!(
            "`{name}` expects {required} argument(s), got {actual}"
        ));
        false
    }

    fn place(&mut self, name: &str) {
        if self.gs1 && name == "playerx" {
            self.place("player.x");
            return;
        }
        if self.gs1 && name == "playery" {
            self.place("player.y");
            return;
        }
        // A dotted name is its first part, then each member with
        // `OP_MEMBER_ACCESS`. As in gs2-parser's output, the value is
        // converted to an object before each member, except straight after
        // `this`/`temp`/`player`/… (`this.a.b` is THIS a MA CTO b MA;
        // `client.a` is VAR client CTO a MA).
        let mut parts = name.split('.');
        let first = parts.next().unwrap_or_default();
        self.root(first);
        let mut after_root = root_op(first).is_some();
        for member in parts {
            if !after_root {
                self.op(Op::ConvToObject);
            }
            after_root = false;
            self.emit_var(member);
            self.op(Op::MemberAccess);
        }
    }

    fn root(&mut self, name: &str) {
        if let Some(op) = root_op(name) {
            self.op(op);
            return;
        }
        match name {
            "true" => self.op(Op::TypeTrue),
            "false" => self.op(Op::TypeFalse),
            "null" => self.op(Op::TypeNull),
            "pi" => self.op(Op::Pi),
            _ => self.emit_var(name),
        }
    }

    fn value(&mut self, e: &Expr) -> Kind {
        match e {
            Expr::Num(n) => {
                self.emit_number(*n);
                Kind::Number
            }
            Expr::Str(s) if self.gs1 && s.contains('#') => self.gs1_text(s),
            Expr::Str(s) => {
                self.emit_str(s);
                Kind::String
            }
            Expr::Var(v) => {
                if !v.index.is_empty() {
                    let Some(dims) = self.element(v) else {
                        self.emit_int(0);
                        return Kind::Number;
                    };
                    self.op(if dims == 1 {
                        Op::Array
                    } else {
                        Op::ArrayMultidim
                    });
                    return Kind::Other;
                }
                if self.gs1 && !v.name.contains('.') && is_event(&v.name, self.era) {
                    self.err(format!(
                        "`{}` is an event flag and has no GS2 value; the handler is {}",
                        v.name,
                        gs2_event_function(&v.name)
                    ));
                    self.emit_int(0);
                    return Kind::Number;
                }
                self.place(&v.name);
                Kind::Other
            }
            Expr::Unary(UnOp::Neg, a) => {
                if let Expr::Num(n) = a.as_ref() {
                    self.emit_number(-n);
                    Kind::Number
                } else {
                    self.numeric(a);
                    self.op(Op::UnarySub);
                    Kind::Number
                }
            }
            Expr::Unary(UnOp::Not, a) => {
                if is_logic(a) {
                    self.logic(a);
                } else {
                    self.bool_leaf(a);
                }
                self.op(Op::Not);
                Kind::Other
            }
            Expr::Bin(BinOp::And | BinOp::Or, _, _) => {
                self.logic(e);
                Kind::Other
            }
            Expr::Bin(op, a, b) => self.binary(*op, a, b),
            Expr::Call { name, args } => {
                self.call_value(name, args);
                if matches!(name.as_str(), "int" | "arraylen" | "sarraylen" | "strlen") {
                    Kind::Number
                } else {
                    Kind::Other
                }
            }
            Expr::InRange { e, lo, hi } => {
                self.value(e);
                self.value(lo);
                self.value(hi);
                self.op(Op::InRange);
                Kind::Other
            }
            Expr::Cond { cond, then, els } => {
                let mut fails = Vec::new();
                self.cond_fail(cond, &mut fails);
                let kt = self.value(then);
                let jump = self.hole(Op::SetIndex);
                let else_at = self.here();
                for f in fails {
                    self.patch(f, else_at);
                }
                let ke = self.value(els);
                self.patch(jump, self.here());
                if kt == Kind::String && ke == Kind::String {
                    Kind::String
                } else {
                    Kind::Other
                }
            }
            // GS2 `x in {a, b}`: like `x in arr` in `lists.gs2bc`, with the
            // literal as the array. Seihai's encoding, no fixture.
            Expr::InList { e, list } if !self.gs1 => {
                self.value(e);
                self.op(Op::TypeArray);
                for v in list.iter().rev() {
                    self.value(v);
                }
                self.op(Op::ArrayEnd);
                self.op(Op::ConvToObject);
                self.op(Op::InObj);
                Kind::Other
            }
            Expr::InList { e, list } => {
                // GS1 `x in {a, b, c}` reads x, then each item left to right
                // until one equals it (the interpreter's `==`). With no
                // calls in x that is `x == a || x == b || x == c`.
                if self.expr_has_call(e) {
                    self.err(
                        "`x in {list}` where x calls a function is not lowered; \
                         put x in a variable first",
                    );
                    self.emit_int(0);
                    return Kind::Number;
                }
                let Some(chain) = list
                    .iter()
                    .map(|item| Expr::Bin(BinOp::Eq, e.clone(), Box::new(item.clone())))
                    .reduce(|acc, eq| Expr::Bin(BinOp::Or, Box::new(acc), Box::new(eq)))
                else {
                    self.emit_int(0);
                    return Kind::Number;
                };
                self.value(&chain)
            }
            Expr::InArray { e, name } => {
                self.value(e);
                self.place(name);
                self.op(Op::ConvToObject);
                self.op(Op::InObj);
                Kind::Other
            }
            Expr::Gs2(g) => self.gs2_value(g),
            Expr::List(values) => {
                if self.gs1 {
                    if let Some(problem) = self.gs1_list_problem(values) {
                        self.err(problem);
                        self.emit_int(0);
                        return Kind::Number;
                    }
                }
                self.op(Op::TypeArray);
                for value in values.iter().rev() {
                    self.value(value);
                }
                self.op(Op::ArrayEnd);
                Kind::Other
            }
        }
    }

    fn numeric(&mut self, e: &Expr) -> Kind {
        if self.value(e) != Kind::Number {
            self.op(Op::ConvToFloat);
        }
        Kind::Number
    }

    fn stringish(&mut self, e: &Expr) {
        if self.value(e) != Kind::String {
            self.op(Op::ConvToString);
        }
    }

    fn binary(&mut self, op: BinOp, a: &Expr, b: &Expr) -> Kind {
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => {
                self.numeric(a);
                self.numeric(b);
                self.op(match op {
                    BinOp::Add => Op::Add,
                    BinOp::Sub => Op::Sub,
                    BinOp::Mul => Op::Mul,
                    BinOp::Div => Op::Div,
                    BinOp::Mod => Op::Mod,
                    BinOp::Pow => Op::Pow,
                    _ => unreachable!(),
                });
                Kind::Number
            }
            BinOp::Eq | BinOp::Ne => {
                self.value(a);
                self.value(b);
                self.op(if op == BinOp::Eq { Op::Eq } else { Op::Neq });
                Kind::Other
            }
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                self.numeric(a);
                self.numeric(b);
                self.op(match op {
                    BinOp::Lt => Op::Lt,
                    BinOp::Gt => Op::Gt,
                    BinOp::Le => Op::Lte,
                    BinOp::Ge => Op::Gte,
                    _ => unreachable!(),
                });
                Kind::Other
            }
            BinOp::Cat | BinOp::CatSpc | BinOp::CatNl | BinOp::CatTab => {
                self.stringish(a);
                if op != BinOp::Cat {
                    let sep = match op {
                        BinOp::CatSpc => " ",
                        BinOp::CatNl => "\n",
                        BinOp::CatTab => "\t",
                        BinOp::Cat => unreachable!(),
                        _ => unreachable!(),
                    };
                    self.emit_str(sep);
                    self.op(Op::Join);
                }
                self.stringish(b);
                self.op(Op::Join);
                Kind::String
            }
            BinOp::And | BinOp::Or => unreachable!(),
        }
    }

    fn bool_leaf(&mut self, e: &Expr) {
        if is_comparison(e) || matches!(e, Expr::InRange { .. } | Expr::Unary(UnOp::Not, _)) {
            self.value(e);
        } else {
            self.numeric(e);
        }
    }

    fn logic(&mut self, e: &Expr) {
        self.logic_operand(e);
        self.op(Op::InlineConditional);
    }

    fn logic_operand(&mut self, e: &Expr) {
        match e {
            Expr::Bin(BinOp::Or, a, b) => {
                let mut holes = Vec::new();
                if is_and(a) {
                    self.and_parts(a, &mut holes);
                } else if is_or(a) {
                    self.logic_operand(a);
                } else {
                    self.bool_leaf(a);
                }
                let or_hole = self.hole(Op::Or);
                let right = self.here();
                for h in holes {
                    self.patch(h, right);
                }
                self.logic_operand(b);
                self.patch(or_hole, self.here());
            }
            Expr::Bin(BinOp::And, _, _) => {
                let mut holes = Vec::new();
                self.and_parts(e, &mut holes);
                let end = self.here();
                for h in holes {
                    self.patch(h, end);
                }
            }
            other => self.bool_leaf(other),
        }
    }

    fn and_parts(&mut self, e: &Expr, holes: &mut Vec<usize>) {
        if let Expr::Bin(BinOp::And, a, b) = e {
            self.and_parts(a, holes);
            holes.push(self.hole(Op::And));
            self.and_parts(b, holes);
        } else if is_or(e) {
            self.logic_operand(e);
        } else {
            self.bool_leaf(e);
        }
    }

    fn cond_fail(&mut self, e: &Expr, fails: &mut Vec<usize>) {
        match e {
            Expr::Bin(BinOp::And, a, b) => {
                self.cond_fail(a, fails);
                self.cond_fail(b, fails);
            }
            Expr::Bin(BinOp::Or, a, b) => {
                let mut trues = Vec::new();
                self.cond_true(a, &mut trues);
                self.cond_fail(b, fails);
                let success = self.here();
                for t in trues {
                    self.patch(t, success);
                }
            }
            Expr::Unary(UnOp::Not, x) if is_logic(x) => self.cond_true(x, fails),
            Expr::Unary(UnOp::Not, x) => {
                self.bool_leaf(x);
                self.op(Op::Not);
                fails.push(self.hole(Op::If));
            }
            other => {
                self.bool_leaf(other);
                fails.push(self.hole(Op::If));
            }
        }
    }

    fn cond_true(&mut self, e: &Expr, trues: &mut Vec<usize>) {
        match e {
            Expr::Bin(BinOp::Or, a, b) => {
                self.cond_true(a, trues);
                self.cond_true(b, trues);
            }
            Expr::Bin(BinOp::And, a, b) => {
                let mut local = Vec::new();
                self.cond_fail(a, &mut local);
                self.cond_true(b, trues);
                let after = self.here();
                for f in local {
                    self.patch(f, after);
                }
            }
            Expr::Unary(UnOp::Not, x) if is_logic(x) => self.cond_fail(x, trues),
            Expr::Unary(UnOp::Not, x) => {
                self.bool_leaf(x);
                trues.push(self.hole(Op::If));
            }
            other => {
                self.bool_leaf(other);
                trues.push(self.hole(Op::Or));
            }
        }
    }
}

/// Why a statement cannot sit inside a `with` block, if it cannot: a
/// `return`, a `sleep` or `waitfor`, or a `break`/`continue` that would
/// leave the block. The NPC server's notes say the old object is not
/// restored then, and no fixture shows GS2 leaving a block early.
fn with_escape(s: &Stmt, in_loop: bool, gs2: bool) -> Option<&'static str> {
    // GS2 keeps a `return`, `sleep` or `waitfor` inside a block: the
    // executor closes the function's open blocks when it returns.
    let waits = |e: &Expr| !gs2 && expr_waits(e);
    let wait_name = |n: &str| !gs2 && is_wait(n);
    let blocked = "`return`, `sleep`, `waitfor`, or a `break`/`continue` that leaves a `with` block, is not lowered";
    match s {
        Stmt::Block(v) => v.iter().find_map(|s| with_escape(s, in_loop, gs2)),
        Stmt::If { cond, then, els } => (waits(cond))
            .then_some(blocked)
            .or_else(|| with_escape(then, in_loop, gs2))
            .or_else(|| els.as_deref().and_then(|e| with_escape(e, in_loop, gs2))),
        Stmt::While { cond, body } => waits(cond)
            .then_some(blocked)
            .or_else(|| with_escape(body, true, gs2)),
        Stmt::For {
            init,
            cond,
            step,
            body,
        } => {
            if cond.as_ref().is_some_and(waits) {
                return Some(blocked);
            }
            init.as_deref()
                .and_then(|s| with_escape(s, in_loop, gs2))
                .or_else(|| step.as_deref().and_then(|s| with_escape(s, true, gs2)))
                .or_else(|| with_escape(body, true, gs2))
        }
        Stmt::With { body, .. } => with_escape(body, false, gs2),
        Stmt::Return(_) => (!gs2).then_some(blocked),
        // GS2 jumps straight out (Preagonal probe: `OP_SET_INDEX` past
        // `OP_WITHEND`); the executor closes the block on the jump.
        Stmt::Break | Stmt::Continue => (!gs2 && !in_loop).then_some(blocked),
        Stmt::Command { name, args, .. } => (wait_name(name)
            || args.iter().any(|a| matches!(a, Arg::Num(e) if waits(e))))
        .then_some(blocked),
        Stmt::Call { name, args } => (wait_name(name) || args.iter().any(waits)).then_some(blocked),
        Stmt::Assign { target, value, .. } => {
            (target.index.iter().any(waits) || waits(value)).then_some(blocked)
        }
        Stmt::IncDec { target, .. } => target.index.iter().any(waits).then_some(blocked),
        Stmt::Function { .. } | Stmt::Unknown { .. } => None,
        Stmt::Gs2(g) => {
            if g.exprs().into_iter().any(waits) {
                return Some(blocked);
            }
            match g.as_ref() {
                // `break` leaves the `switch`, not the block; a `continue`
                // is refused when it is emitted (no loop inside the block).
                Gs2Stmt::Switch { .. } | Gs2Stmt::ForEach { .. } => g
                    .bodies()
                    .into_iter()
                    .find_map(|s| with_escape(s, true, gs2)),
                Gs2Stmt::NewBlock { body, .. } => {
                    body.iter().find_map(|s| with_escape(s, false, gs2))
                }
                Gs2Stmt::Eval(_) | Gs2Stmt::SetPart { .. } | Gs2Stmt::Chain { .. } => None,
            }
        }
    }
}

fn is_wait(name: &str) -> bool {
    name.eq_ignore_ascii_case("sleep") || name.eq_ignore_ascii_case("waitfor")
}

fn expr_waits(e: &Expr) -> bool {
    mentions_call(e, "sleep") || mentions_call(e, "waitfor")
}

/// Every variable name in a statement, with `true` for a written one.
fn stmt_names(s: &Stmt, f: &mut dyn FnMut(&str, bool)) {
    match s {
        Stmt::Block(v) => v.iter().for_each(|s| stmt_names(s, f)),
        Stmt::If { cond, then, els } => {
            expr_names(cond, f);
            stmt_names(then, f);
            if let Some(e) = els {
                stmt_names(e, f);
            }
        }
        Stmt::While { cond, body } => {
            expr_names(cond, f);
            stmt_names(body, f);
        }
        Stmt::For {
            init,
            cond,
            step,
            body,
        } => {
            if let Some(s) = init {
                stmt_names(s, f);
            }
            if let Some(c) = cond {
                expr_names(c, f);
            }
            if let Some(s) = step {
                stmt_names(s, f);
            }
            stmt_names(body, f);
        }
        Stmt::With { target, body, .. } => {
            expr_names(target, f);
            stmt_names(body, f);
        }
        Stmt::Return(v) => {
            if let Some(e) = v {
                expr_names(e, f);
            }
        }
        Stmt::Assign { target, value, .. } => {
            f(&target.name, true);
            target.index.iter().for_each(|e| expr_names(e, f));
            expr_names(value, f);
        }
        Stmt::IncDec { target, .. } => {
            f(&target.name, true);
            target.index.iter().for_each(|e| expr_names(e, f));
        }
        Stmt::Call { args, .. } => args.iter().for_each(|e| expr_names(e, f)),
        Stmt::Command { args, .. } => args.iter().for_each(|a| {
            if let Arg::Num(e) = a {
                expr_names(e, f);
            }
        }),
        Stmt::Function { .. } | Stmt::Break | Stmt::Continue | Stmt::Unknown { .. } => {}
        Stmt::Gs2(g) => {
            g.exprs().into_iter().for_each(|e| expr_names(e, f));
            g.bodies().into_iter().for_each(|s| stmt_names(s, f));
        }
    }
}

fn expr_names(e: &Expr, f: &mut dyn FnMut(&str, bool)) {
    match e {
        Expr::Num(_) | Expr::Str(_) => {}
        Expr::Var(v) => {
            f(&v.name, false);
            v.index.iter().for_each(|e| expr_names(e, f));
        }
        Expr::Unary(_, a) => expr_names(a, f),
        Expr::InArray { e, name } => {
            expr_names(e, f);
            f(name, false);
        }
        Expr::Bin(_, a, b) => {
            expr_names(a, f);
            expr_names(b, f);
        }
        Expr::Call { args, .. } | Expr::List(args) => args.iter().for_each(|e| expr_names(e, f)),
        Expr::InRange { e, lo, hi } => {
            expr_names(e, f);
            expr_names(lo, f);
            expr_names(hi, f);
        }
        Expr::InList { e, list } => {
            expr_names(e, f);
            list.iter().for_each(|e| expr_names(e, f));
        }
        Expr::Cond { cond, then, els } => {
            expr_names(cond, f);
            expr_names(then, f);
            expr_names(els, f);
        }
        Expr::Gs2(g) => g.parts().into_iter().for_each(|e| expr_names(e, f)),
    }
}

/// Why an indexed variable is not lowered, if it is not.
fn element_problem(v: &VarRef, gs1: bool) -> Option<String> {
    if v.field_after_index {
        return Some(format!(
            "a field after an index (`{}`) is not lowered by this temporary front end",
            v
        ));
    }
    if v.index.len() > 2 {
        return Some(format!(
            "`{}` has {} indexes; GS2 array opcodes take one or two",
            v.name,
            v.index.len()
        ));
    }
    // GS2 `tiles[x, y]` and `board[i]` are ordinary element reads and
    // writes (Preagonal probe). Whether GS1's level board maps onto them is
    // not checked, so the GS1 front end still refuses them.
    if gs1 && matches!(v.name.as_str(), "board" | "tiles") {
        return Some(format!(
            "`{}[..]` has no fixture-checked GS2 encoding; it is not lowered",
            v.name
        ));
    }
    None
}

/// The expression holds a list literal anywhere.
fn has_nested_list(e: &Expr) -> bool {
    match e {
        Expr::List(_) => true,
        Expr::Num(_) | Expr::Str(_) => false,
        Expr::Var(v) => v.index.iter().any(has_nested_list),
        Expr::Unary(_, a) | Expr::InArray { e: a, .. } => has_nested_list(a),
        Expr::Bin(_, a, b) => has_nested_list(a) || has_nested_list(b),
        Expr::Call { args, .. } => args.iter().any(has_nested_list),
        Expr::InRange { e, lo, hi } => {
            has_nested_list(e) || has_nested_list(lo) || has_nested_list(hi)
        }
        Expr::InList { e, list } => has_nested_list(e) || list.iter().any(has_nested_list),
        Expr::Cond { cond, then, els } => {
            has_nested_list(cond) || has_nested_list(then) || has_nested_list(els)
        }
        Expr::Gs2(g) => g.parts().into_iter().any(has_nested_list),
    }
}

/// The expression calls `name` anywhere.
fn mentions_call(e: &Expr, name: &str) -> bool {
    let m = |x: &Expr| mentions_call(x, name);
    match e {
        Expr::Call { name: n, args } => n.eq_ignore_ascii_case(name) || args.iter().any(m),
        Expr::Num(_) | Expr::Str(_) => false,
        Expr::Var(v) => v.index.iter().any(m),
        Expr::Unary(_, a) | Expr::InArray { e: a, .. } => m(a),
        Expr::Bin(_, a, b) => m(a) || m(b),
        Expr::InRange { e, lo, hi } => m(e) || m(lo) || m(hi),
        Expr::InList { e, list } => m(e) || list.iter().any(m),
        Expr::Cond { cond, then, els } => m(cond) || m(then) || m(els),
        Expr::List(v) => v.iter().any(m),
        Expr::Gs2(g) => g.parts().into_iter().any(m),
    }
}

fn root_op(name: &str) -> Option<Op> {
    Some(match name {
        "this" => Op::This,
        "thiso" => Op::ThisO,
        "player" => Op::Player,
        "playero" => Op::PlayerO,
        "level" => Op::Level,
        "temp" => Op::Temp,
        _ => return None,
    })
}

fn is_comparison(e: &Expr) -> bool {
    matches!(e, Expr::Bin(op, _, _) if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge))
}

fn is_and(e: &Expr) -> bool {
    matches!(e, Expr::Bin(BinOp::And, _, _))
}

fn is_or(e: &Expr) -> bool {
    matches!(e, Expr::Bin(BinOp::Or, _, _))
}

fn is_logic(e: &Expr) -> bool {
    match e {
        Expr::Bin(BinOp::And | BinOp::Or, _, _) => true,
        Expr::Unary(UnOp::Not, x) => is_logic(x),
        _ => false,
    }
}

fn ends_with_return(body: &[Stmt]) -> bool {
    match body.last() {
        Some(Stmt::Return(_)) => true,
        Some(Stmt::Block(v)) => ends_with_return(v),
        _ => false,
    }
}

fn arg_to_expr(a: &Arg) -> Expr {
    match a {
        Arg::Num(e) => e.clone(),
        Arg::Text(t) => Expr::Str(t.clone()),
    }
}

fn is_plain_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

fn parse_gs2(src: &str) -> Result<Vec<Item>, CompileErrors> {
    let mut p = Gs2::new(src);
    let mut items = Vec::new();
    loop {
        p.skip();
        if p.eof() {
            break;
        }
        let public = p.eat_id("public");
        if p.eat_id("function") {
            let mut item = p.function()?;
            if public {
                item.name = format!("public.{}", item.name);
            }
            items.push(item);
            continue;
        }
        if public {
            return Err(CompileErrors::one(
                p.line,
                "expected `function` after `public`",
            ));
        }
        let s = p.stmt()?;
        if let Some(message) = p.bad.take() {
            return Err(CompileErrors::one(p.line, message));
        }
        if matches!(&s, Stmt::Block(v) if v.is_empty()) {
            continue;
        }
        match items.last_mut() {
            Some(last) if last.top => last.body.push(s),
            _ => items.push(Item::top(vec![s])),
        }
    }
    Ok(items)
}

struct Gs2<'a> {
    b: &'a [u8],
    i: usize,
    line: u32,
    peek: Option<GTok>,
    bad: Option<String>,
    /// The body of a `new T(name) { … }` the expression parser just read,
    /// for the statement parser to pick up.
    new_body: Option<Vec<Stmt>>,
    /// `const` and `enum` names (lowercase), folded where they are read.
    consts: HashMap<String, Expr>,
}

#[derive(Clone)]
enum GTok {
    Id(String),
    Num(f64),
    Str(String),
    Op(&'static str),
    Eof,
}

impl<'a> Gs2<'a> {
    fn new(src: &'a str) -> Self {
        Gs2 {
            b: src.as_bytes(),
            i: 0,
            line: 1,
            peek: None,
            bad: None,
            new_body: None,
            consts: HashMap::new(),
        }
    }

    fn eof(&mut self) -> bool {
        matches!(self.peek_tok(), GTok::Eof)
    }

    /// A name with dots in it: `Obj.onAction` as a function name (an event
    /// of a GUI control, kept whole as the function's name as the Preagonal
    /// baselines do) or `temp.x` as a parameter.
    fn dotted_ident(&mut self) -> Result<String, CompileErrors> {
        let mut name = self.ident()?;
        while self.eat(".") {
            name.push('.');
            name.push_str(&self.ident()?);
        }
        Ok(name)
    }

    fn function(&mut self) -> Result<Item, CompileErrors> {
        let name = self.dotted_ident()?;
        self.expect("(")?;
        let mut params = Vec::new();
        if !self.eat(")") {
            loop {
                params.push(self.dotted_ident()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
        }
        // A body without braces is one statement (`function f() g();`).
        let body = if self.eat("{") {
            self.block_body()?
        } else {
            vec![self.stmt()?]
        };
        if let Some(message) = self.bad.take() {
            return Err(CompileErrors::one(self.line, message));
        }
        Ok(Item::func(name, params, body))
    }

    fn block_body(&mut self) -> Result<Vec<Stmt>, CompileErrors> {
        let mut body = Vec::new();
        loop {
            self.skip();
            if self.eat("}") {
                break;
            }
            if self.eof() {
                return Err(CompileErrors::one(self.line, "unclosed `{`"));
            }
            if self.eat(";") {
                continue;
            }
            body.push(self.stmt()?);
        }
        Ok(body)
    }

    fn stmt(&mut self) -> Result<Stmt, CompileErrors> {
        if self.eat_id("if") {
            return self.if_stmt();
        }
        if self.eat_id("while") {
            return self.while_stmt();
        }
        if self.eat_id("for") {
            return self.for_stmt();
        }
        if self.eat_id("return") {
            let v = if self.sees(";") || self.sees("}") {
                None
            } else {
                Some(self.expr()?)
            };
            self.expect(";")?;
            return Ok(Stmt::Return(v));
        }
        if self.eat_id("break") {
            self.expect(";")?;
            return Ok(Stmt::Break);
        }
        if self.eat_id("continue") {
            self.expect(";")?;
            return Ok(Stmt::Continue);
        }
        if self.eat("{") {
            return Ok(Stmt::Block(self.block_body()?));
        }
        if self.eat_id("with") {
            let line = self.line;
            self.expect("(")?;
            let target = self.expr()?;
            self.expect(")")?;
            let body = Box::new(self.stmt()?);
            return Ok(Stmt::With { target, body, line });
        }
        if self.eat_id("switch") {
            return self.switch_stmt();
        }
        if self.eat_id("const") {
            // `const NAME = value;`: folded where it is read, no code
            // (Preagonal probe: `const C = 1; y = C;` is `y = 1`).
            loop {
                let name = self.ident()?;
                self.expect("=")?;
                let value = self.const_value()?;
                self.consts.insert(name.to_ascii_lowercase(), value);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(";")?;
            return Ok(Stmt::Block(Vec::new()));
        }
        if self.eat_id("enum") {
            // `enum Name { A, B = 5, C }`: numbers from 0, or from a given
            // value. Seihai folds them like `const`; the Preagonal output
            // reads a bare member as a variable.
            let prefix = if self.sees("{") {
                None
            } else {
                Some(self.ident()?.to_ascii_lowercase())
            };
            self.expect("{")?;
            let mut next = 0.0;
            while !self.eat("}") {
                let name = self.ident()?;
                if self.eat("=") {
                    match self.const_value()? {
                        Expr::Num(n) => next = n,
                        _ => {
                            return Err(CompileErrors::one(
                                self.line,
                                "an `enum` value must be a number",
                            ))
                        }
                    }
                }
                let lower = name.to_ascii_lowercase();
                if let Some(p) = &prefix {
                    self.consts.insert(format!("{p}::{lower}"), Expr::Num(next));
                }
                self.consts.insert(lower, Expr::Num(next));
                next += 1.0;
                if !self.eat(",") {
                    self.expect("}")?;
                    break;
                }
            }
            self.eat(";");
            return Ok(Stmt::Block(Vec::new()));
        }
        if self.eat_id("public") {
            return Err(CompileErrors::one(
                self.line,
                "that statement is not in this temporary GS2 compiler",
            ));
        }
        if self.sees("++") || self.sees("--") {
            let delta = if self.eat("++") {
                1.0
            } else {
                self.bump();
                -1.0
            };
            let Expr::Var(target) = self.unary()? else {
                return Err(CompileErrors::one(self.line, "`++`/`--` needs a variable"));
            };
            self.expect(";")?;
            return Ok(Stmt::IncDec { target, delta });
        }
        let e = self.expr()?;
        if let Expr::Gs2(g) = &e {
            if let Gs2Expr::PostStep {
                target: Expr::Var(v),
                delta,
            } = g.as_ref()
            {
                self.expect(";")?;
                return Ok(Stmt::IncDec {
                    target: v.clone(),
                    delta: *delta,
                });
            }
        }
        if let Some(body) = self.new_body.take() {
            return match e {
                Expr::Gs2(g) => match *g {
                    Gs2Expr::New { ty, mut args } if args.len() == 1 => {
                        self.eat(";");
                        Ok(Stmt::Gs2(Box::new(Gs2Stmt::NewBlock {
                            ty,
                            name: args.remove(0),
                            body,
                        })))
                    }
                    _ => Err(CompileErrors::one(
                        self.line,
                        "`new T(name) { … }` needs one name and must stand alone",
                    )),
                },
                _ => Err(CompileErrors::one(
                    self.line,
                    "`new T(name) { … }` used as a value is not lowered",
                )),
            };
        }
        if let Expr::Var(v) = &e {
            if self.eat("++") {
                self.expect(";")?;
                return Ok(Stmt::IncDec {
                    target: v.clone(),
                    delta: 1.0,
                });
            }
            if self.eat("--") {
                self.expect(";")?;
                return Ok(Stmt::IncDec {
                    target: v.clone(),
                    delta: -1.0,
                });
            }
            if let Some(op) = self.assign_op() {
                let mut value = self.expr()?;
                if op == AssignOp::Set && matches!(value, Expr::Var(_)) && self.sees("=") {
                    let mut targets = vec![v.clone()];
                    while let Expr::Var(t) = &value {
                        if !self.eat("=") {
                            break;
                        }
                        targets.push(t.clone());
                        value = self.expr()?;
                    }
                    self.expect(";")?;
                    return Ok(Stmt::Gs2(Box::new(Gs2Stmt::Chain { targets, value })));
                }
                self.expect(";")?;
                return Ok(Stmt::Assign {
                    target: v.clone(),
                    op,
                    value,
                });
            }
            // `a @= b` is `a = a @ b` here (Seihai's reading, no fixture).
            if self.eat("@=") {
                let value = self.expr()?;
                self.expect(";")?;
                return Ok(Stmt::Assign {
                    target: v.clone(),
                    op: AssignOp::Set,
                    value: Expr::Bin(BinOp::Cat, Box::new(e.clone()), Box::new(value)),
                });
            }
        }
        let settable = match &e {
            Expr::Gs2(g) => matches!(
                g.as_ref(),
                Gs2Expr::Member { .. } | Gs2Expr::Index { .. } | Gs2Expr::DynMember { .. }
            ),
            Expr::Call { name, .. } => name == "makevar",
            _ => false,
        };
        if settable && self.eat("=") {
            let value = self.expr()?;
            self.expect(";")?;
            return Ok(Stmt::Gs2(Box::new(Gs2Stmt::SetPart { target: e, value })));
        }
        self.expect(";")?;
        match e {
            Expr::Call { name, args } => Ok(Stmt::Call { name, args }),
            Expr::Gs2(g)
                if matches!(
                    g.as_ref(),
                    Gs2Expr::Method { .. }
                        | Gs2Expr::New { .. }
                        | Gs2Expr::PostStep { .. }
                        | Gs2Expr::AssignValue { .. }
                ) =>
            {
                Ok(Stmt::Gs2(Box::new(Gs2Stmt::Eval(Expr::Gs2(g)))))
            }
            _ => Err(CompileErrors::one(
                self.line,
                "expression statement is not a call",
            )),
        }
    }

    fn if_stmt(&mut self) -> Result<Stmt, CompileErrors> {
        self.expect("(")?;
        let cond = self.expr()?;
        self.expect(")")?;
        let then = Box::new(self.stmt()?);
        let els = if self.eat_id("else") {
            Some(Box::new(self.stmt()?))
        } else if self.eat_id("elseif") {
            // `elseif (c) s` is `else if (c) s`.
            Some(Box::new(self.if_stmt()?))
        } else {
            None
        };
        Ok(Stmt::If { cond, then, els })
    }

    fn while_stmt(&mut self) -> Result<Stmt, CompileErrors> {
        self.expect("(")?;
        let cond = self.expr()?;
        self.expect(")")?;
        let body = Box::new(self.stmt()?);
        Ok(Stmt::While { cond, body })
    }

    fn for_stmt(&mut self) -> Result<Stmt, CompileErrors> {
        self.expect("(")?;
        let init = if self.eat(";") {
            None
        } else {
            let first = self.expr()?;
            if self.eat(":") {
                let Expr::Var(var) = first else {
                    return Err(CompileErrors::one(
                        self.line,
                        "a for-each variable must be a name",
                    ));
                };
                let list = self.expr()?;
                self.expect(")")?;
                let body = self.stmt()?;
                return Ok(Stmt::Gs2(Box::new(Gs2Stmt::ForEach { var, list, body })));
            }
            Some(Box::new(self.simple_from(first)?))
        };
        if init.is_some() {
            self.expect(";")?;
        }
        let cond = if self.eat(";") {
            None
        } else {
            Some(self.expr()?)
        };
        if cond.is_some() {
            self.expect(";")?;
        }
        let step = if self.eat(")") {
            None
        } else {
            Some(Box::new(self.simple()?))
        };
        if step.is_some() {
            self.expect(")")?;
        }
        let body = Box::new(self.stmt()?);
        Ok(Stmt::For {
            init,
            cond,
            step,
            body,
        })
    }

    fn simple(&mut self) -> Result<Stmt, CompileErrors> {
        let e = self.expr()?;
        self.simple_from(e)
    }

    fn simple_from(&mut self, e: Expr) -> Result<Stmt, CompileErrors> {
        if let Expr::Gs2(g) = &e {
            if let Gs2Expr::PostStep {
                target: Expr::Var(v),
                delta,
            } = g.as_ref()
            {
                return Ok(Stmt::IncDec {
                    target: v.clone(),
                    delta: *delta,
                });
            }
        }
        let Expr::Var(v) = e else {
            return Err(CompileErrors::one(
                self.line,
                "for init and step must assign a variable",
            ));
        };
        if self.eat("++") {
            return Ok(Stmt::IncDec {
                target: v,
                delta: 1.0,
            });
        }
        if self.eat("--") {
            return Ok(Stmt::IncDec {
                target: v,
                delta: -1.0,
            });
        }
        let Some(op) = self.assign_op() else {
            return Err(CompileErrors::one(self.line, "expected an assignment"));
        };
        let value = self.expr()?;
        Ok(Stmt::Assign {
            target: v,
            op,
            value,
        })
    }

    fn switch_stmt(&mut self) -> Result<Stmt, CompileErrors> {
        self.expect("(")?;
        let value = self.expr()?;
        self.expect(")")?;
        self.expect("{")?;
        let mut arms: Vec<SwitchArm> = Vec::new();
        loop {
            self.skip();
            if self.eat("}") {
                break;
            }
            if self.eat_id("case") {
                let v = self.expr()?;
                self.expect(":")?;
                arms.push(SwitchArm {
                    values: vec![v],
                    default: false,
                    body: Vec::new(),
                });
                continue;
            }
            if self.eat_id("default") {
                self.expect(":")?;
                arms.push(SwitchArm {
                    values: Vec::new(),
                    default: true,
                    body: Vec::new(),
                });
                continue;
            }
            let s = self.stmt()?;
            match arms.last_mut() {
                Some(arm) => arm.body.push(s),
                None => {
                    return Err(CompileErrors::one(
                        self.line,
                        "a statement before the first `case`",
                    ))
                }
            }
        }
        Ok(Stmt::Gs2(Box::new(Gs2Stmt::Switch { value, arms })))
    }

    fn expr(&mut self) -> Result<Expr, CompileErrors> {
        let cond = self.binary(1)?;
        if self.eat("?") {
            let then = self.expr()?;
            self.expect(":")?;
            let els = self.expr()?;
            return Ok(Expr::Cond {
                cond: Box::new(cond),
                then: Box::new(then),
                els: Box::new(els),
            });
        }
        Ok(cond)
    }

    fn binary(&mut self, min: u8) -> Result<Expr, CompileErrors> {
        let mut lhs = self.unary()?;
        loop {
            // `x in arr`, at the equality level. Only a variable on the right.
            if min <= 6 && self.eat_id("in") {
                if self.eat("|") {
                    let lo = self.binary(4)?;
                    self.expect(",")?;
                    let hi = self.binary(4)?;
                    self.expect("|")?;
                    lhs = Expr::InRange {
                        e: Box::new(lhs),
                        lo: Box::new(lo),
                        hi: Box::new(hi),
                    };
                    continue;
                }
                if self.eat("{") {
                    let mut list = Vec::new();
                    if !self.eat("}") {
                        loop {
                            list.push(self.expr()?);
                            if !self.eat(",") {
                                break;
                            }
                        }
                        self.expect("}")?;
                    }
                    lhs = Expr::InList {
                        e: Box::new(lhs),
                        list,
                    };
                    continue;
                }
                let mut name = self.ident()?;
                while self.eat(".") {
                    name.push('.');
                    name.push_str(&self.ident()?);
                }
                lhs = Expr::InArray {
                    e: Box::new(lhs),
                    name,
                };
                continue;
            }
            let Some((fallback, prec, text)) = self.bin_op(min) else {
                break;
            };
            self.bump();
            let rhs = self.binary(prec + 1)?;
            let bits = match text {
                "<<" => Some(BitKind::Shl),
                ">>" => Some(BitKind::Shr),
                "&" => Some(BitKind::And),
                "|" => Some(BitKind::Or),
                _ => None,
            };
            if let Some(kind) = bits {
                lhs = Expr::Gs2(Box::new(Gs2Expr::Bits {
                    kind,
                    a: lhs,
                    b: rhs,
                }));
                continue;
            }
            let op = match text {
                "||" => BinOp::Or,
                "&&" => BinOp::And,
                "@" => BinOp::Cat,
                "SPC" => BinOp::CatSpc,
                "NL" => BinOp::CatNl,
                "TAB" => BinOp::CatTab,
                "==" => BinOp::Eq,
                "!=" => BinOp::Ne,
                "<" => BinOp::Lt,
                ">" => BinOp::Gt,
                "<=" => BinOp::Le,
                ">=" => BinOp::Ge,
                "+" => BinOp::Add,
                "-" => BinOp::Sub,
                "*" => BinOp::Mul,
                "/" => BinOp::Div,
                "%" => BinOp::Mod,
                "^" => BinOp::Pow,
                _ => fallback,
            };
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn bin_op(&mut self, min: u8) -> Option<(BinOp, u8, &'static str)> {
        let tok = self.peek_tok();
        let text: &'static str = match tok {
            GTok::Op(text) => text,
            GTok::Id(id) if id == "SPC" => "SPC",
            GTok::Id(id) if id == "NL" => "NL",
            GTok::Id(id) if id == "TAB" => "TAB",
            _ => return None,
        };
        let (prec, op) = match text {
            "||" => (1, BinOp::Or),
            "&&" => (2, BinOp::And),
            "@" => (3, BinOp::Cat),
            // Below `<` so that `x in |a, b|` still closes its range.
            "&" | "|" => (3, BinOp::Mul),
            "SPC" => (3, BinOp::CatSpc),
            "NL" => (3, BinOp::CatNl),
            "TAB" => (3, BinOp::CatTab),
            "<" | "<=" | "=<" => (4, if text == "<" { BinOp::Lt } else { BinOp::Le }),
            ">" | ">=" | "=>" => (5, if text == ">" { BinOp::Gt } else { BinOp::Ge }),
            "==" | "!=" | "<>" => (6, if text == "==" { BinOp::Eq } else { BinOp::Ne }),
            "<<" | ">>" => (7, BinOp::Mul),
            "+" | "-" => (8, if text == "+" { BinOp::Add } else { BinOp::Sub }),
            "*" | "/" | "%" | "^" => (
                9,
                match text {
                    "*" => BinOp::Mul,
                    "/" => BinOp::Div,
                    "%" => BinOp::Mod,
                    _ => BinOp::Pow,
                },
            ),
            _ => return None,
        };
        if prec < min {
            return None;
        }
        Some((op, prec, text))
    }

    fn unary(&mut self) -> Result<Expr, CompileErrors> {
        if self.eat("!") {
            return Ok(Expr::Unary(UnOp::Not, Box::new(self.unary()?)));
        }
        if self.eat("-") {
            return Ok(Expr::Unary(UnOp::Neg, Box::new(self.unary()?)));
        }
        if self.eat("+") {
            return self.unary();
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, CompileErrors> {
        let e = self.atom()?;
        self.postfix(e)
    }

    /// `.name`, `.name(args)` and `[i]` after a value that is not a plain
    /// dotted name.
    fn postfix(&mut self, mut e: Expr) -> Result<Expr, CompileErrors> {
        loop {
            if self.eat(".") {
                if self.eat("(") {
                    let n = self.expr()?;
                    self.expect(")")?;
                    e = Expr::Gs2(Box::new(Gs2Expr::DynMember { obj: e, name: n }));
                    continue;
                }
                let name = self.ident()?;
                if self.eat("(") {
                    let args = self.arg_list()?;
                    e = Expr::Gs2(Box::new(Gs2Expr::Method { obj: e, name, args }));
                } else {
                    e = Expr::Gs2(Box::new(Gs2Expr::Member { obj: e, name }));
                }
            } else if self.sees("[") && !matches!(e, Expr::Num(_)) {
                self.bump();
                let mut index = Vec::new();
                loop {
                    index.push(self.expr()?);
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect("]")?;
                e = Expr::Gs2(Box::new(Gs2Expr::Index { obj: e, index }));
            } else if self.sees("++") || self.sees("--") {
                let delta = if self.eat("++") {
                    1.0
                } else {
                    self.bump();
                    -1.0
                };
                e = Expr::Gs2(Box::new(Gs2Expr::PostStep { target: e, delta }));
            } else {
                return Ok(e);
            }
        }
    }

    fn atom(&mut self) -> Result<Expr, CompileErrors> {
        match self.bump() {
            GTok::Num(n) => Ok(Expr::Num(n)),
            GTok::Str(s) => Ok(Expr::Str(s)),
            GTok::Id(id) if id == "new" => {
                if self.sees("[") {
                    let mut dims = Vec::new();
                    while self.eat("[") {
                        dims.push(self.expr()?);
                        self.expect("]")?;
                    }
                    return Ok(Expr::Gs2(Box::new(Gs2Expr::NewArray(dims))));
                }
                let ty = self.ident()?;
                let args = if self.eat("(") {
                    self.arg_list()?
                } else {
                    Vec::new()
                };
                if self.eat("{") {
                    let body = self.block_body()?;
                    self.new_body = Some(body);
                }
                Ok(Expr::Gs2(Box::new(Gs2Expr::New { ty, args })))
            }
            GTok::Id(mut name) => {
                if self.eat("(") {
                    let args = self.arg_list()?;
                    return Ok(Expr::Call { name, args });
                }
                if let Some(v) = self.consts.get(&name.to_ascii_lowercase()).cloned() {
                    if !self.sees(".") && !self.sees("[") {
                        return Ok(v);
                    }
                }
                while self.sees(".") {
                    self.bump();
                    if self.eat("(") {
                        // `a.b.(expr)`: a member named by a value.
                        let n = self.expr()?;
                        self.expect(")")?;
                        let obj = Expr::Var(VarRef {
                            name,
                            index: Vec::new(),
                            field_after_index: false,
                        });
                        return Ok(Expr::Gs2(Box::new(Gs2Expr::DynMember { obj, name: n })));
                    }
                    let part = self.ident()?;
                    if self.eat("(") {
                        // `a.b.f(x)`: a method on the dotted name before it.
                        let args = self.arg_list()?;
                        let obj = Expr::Var(VarRef {
                            name,
                            index: Vec::new(),
                            field_after_index: false,
                        });
                        return Ok(Expr::Gs2(Box::new(Gs2Expr::Method {
                            obj,
                            name: part,
                            args,
                        })));
                    }
                    name.push('.');
                    name.push_str(&part);
                }
                let mut index = Vec::new();
                if self.eat("[") {
                    loop {
                        index.push(self.expr()?);
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect("]")?;
                }
                Ok(Expr::Var(VarRef {
                    name,
                    index,
                    field_after_index: false,
                }))
            }
            GTok::Op("(") => {
                let mut e = self.expr()?;
                // `(x = v)`, `(x += v)`: an assignment used as a value.
                if let Some(op) = self.assign_op() {
                    let value = self.expr()?;
                    e = Expr::Gs2(Box::new(Gs2Expr::AssignValue {
                        target: e,
                        op,
                        value,
                    }));
                }
                self.expect(")")?;
                Ok(e)
            }
            GTok::Op("@") => {
                let e = self.unary()?;
                Ok(Expr::Gs2(Box::new(Gs2Expr::Text(e))))
            }
            GTok::Op("{") => {
                let mut values = Vec::new();
                if !self.eat("}") {
                    loop {
                        values.push(self.expr()?);
                        if !self.eat(",") || self.sees("}") {
                            break;
                        }
                    }
                    self.expect("}")?;
                }
                Ok(Expr::List(values))
            }
            GTok::Op("|") => Err(CompileErrors::one(
                self.line,
                "`in` ranges are written in GS1; this GS2 parser does not accept `|`",
            )),
            _ => Err(CompileErrors::one(self.line, "expected an expression")),
        }
    }

    /// A `const` or `enum` value: a number, a string, or a negative number.
    fn const_value(&mut self) -> Result<Expr, CompileErrors> {
        match self.expr()? {
            v @ (Expr::Num(_) | Expr::Str(_)) => Ok(v),
            Expr::Unary(UnOp::Neg, n) if matches!(*n, Expr::Num(_)) => match *n {
                Expr::Num(x) => Ok(Expr::Num(-x)),
                _ => unreachable!(),
            },
            _ => Err(CompileErrors::one(
                self.line,
                "a `const` value must be a number or a string",
            )),
        }
    }

    fn arg_list(&mut self) -> Result<Vec<Expr>, CompileErrors> {
        let mut args = Vec::new();
        if self.eat(")") {
            return Ok(args);
        }
        loop {
            args.push(self.expr()?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        Ok(args)
    }

    fn assign_op(&mut self) -> Option<AssignOp> {
        let op = match self.peek_tok() {
            GTok::Op("=") | GTok::Op(":=") => AssignOp::Set,
            GTok::Op("+=") => AssignOp::Add,
            GTok::Op("-=") => AssignOp::Sub,
            GTok::Op("*=") => AssignOp::Mul,
            GTok::Op("/=") => AssignOp::Div,
            GTok::Op("%=") => AssignOp::Mod,
            _ => return None,
        };
        self.bump();
        Some(op)
    }

    fn ident(&mut self) -> Result<String, CompileErrors> {
        match self.bump() {
            GTok::Id(s) => Ok(s),
            _ => Err(CompileErrors::one(self.line, "expected a name")),
        }
    }

    fn expect(&mut self, op: &str) -> Result<(), CompileErrors> {
        if self.eat(op) {
            Ok(())
        } else {
            Err(CompileErrors::one(self.line, format!("expected `{op}`")))
        }
    }

    fn eat_id(&mut self, name: &str) -> bool {
        self.skip();
        match self.peek_tok() {
            GTok::Id(s) if s == name => {
                self.bump();
                true
            }
            _ => false,
        }
    }

    fn sees(&mut self, op: &str) -> bool {
        matches!(self.peek_tok(), GTok::Op(s) if s == op)
    }

    fn eat(&mut self, op: &str) -> bool {
        if self.sees(op) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn peek_tok(&mut self) -> GTok {
        if self.peek.is_none() {
            self.peek = Some(self.lex());
        }
        self.peek.clone().unwrap()
    }

    fn bump(&mut self) -> GTok {
        self.peek_tok();
        self.peek.take().unwrap()
    }

    fn skip(&mut self) {
        loop {
            match self.b.get(self.i) {
                Some(b'\n') => {
                    self.line += 1;
                    self.i += 1;
                }
                Some(c) if c.is_ascii_whitespace() => self.i += 1,
                Some(b'/') if self.b.get(self.i + 1) == Some(&b'/') => {
                    while self.i < self.b.len() && self.b[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                Some(b'/') if self.b.get(self.i + 1) == Some(&b'*') => {
                    self.i += 2;
                    while self.i + 1 < self.b.len()
                        && !(self.b[self.i] == b'*' && self.b[self.i + 1] == b'/')
                    {
                        if self.b[self.i] == b'\n' {
                            self.line += 1;
                        }
                        self.i += 1;
                    }
                    self.i = (self.i + 2).min(self.b.len());
                }
                _ => break,
            }
        }
    }

    fn lex(&mut self) -> GTok {
        self.skip();
        let Some(&c) = self.b.get(self.i) else {
            return GTok::Eof;
        };
        if c == b'"' || c == b'\'' {
            // `\n`, `\t`, `\"`, `\'` and `\\` are unescaped here; any other
            // escaped byte stands for itself. Seihai's reading, no fixture.
            self.i += 1;
            let mut text = Vec::new();
            while self.i < self.b.len() && self.b[self.i] != c {
                let mut ch = self.b[self.i];
                if ch == b'\\' && self.i + 1 < self.b.len() {
                    self.i += 1;
                    ch = match self.b[self.i] {
                        b'n' => b'\n',
                        b't' => b'\t',
                        b'r' => b'\r',
                        other => other,
                    };
                }
                if ch == b'\n' {
                    self.line += 1;
                }
                text.push(ch);
                self.i += 1;
            }
            if self.i < self.b.len() {
                self.i += 1;
            }
            return GTok::Str(String::from_utf8_lossy(&text).into_owned());
        }
        if c == b'0' && matches!(self.b.get(self.i + 1), Some(b'x' | b'X')) {
            self.i += 2;
            let start = self.i;
            while self.i < self.b.len() && self.b[self.i].is_ascii_hexdigit() {
                self.i += 1;
            }
            let n = u32::from_str_radix(
                std::str::from_utf8(&self.b[start..self.i]).unwrap_or("0"),
                16,
            )
            .unwrap_or(0);
            return GTok::Num(n as f64);
        }
        if c.is_ascii_digit()
            || (c == b'.' && self.b.get(self.i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let start = self.i;
            while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
                self.i += 1;
            }
            if self.b.get(self.i) == Some(&b'.') {
                self.i += 1;
                while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
                    self.i += 1;
                }
            }
            let text = std::str::from_utf8(&self.b[start..self.i]).unwrap_or("0");
            return GTok::Num(text.parse().unwrap_or(0.0));
        }
        if c.is_ascii_alphabetic() || c == b'_' || c == b'$' {
            let start = self.i;
            self.i += 1;
            // `$pref::Video::x` is one name (Preagonal probe).
            loop {
                match self.b.get(self.i) {
                    Some(c) if c.is_ascii_alphanumeric() || *c == b'_' || *c == b'$' => self.i += 1,
                    Some(b':')
                        if self.b.get(self.i + 1) == Some(&b':')
                            && self
                                .b
                                .get(self.i + 2)
                                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') =>
                    {
                        self.i += 2
                    }
                    _ => break,
                }
            }
            let text = String::from_utf8_lossy(&self.b[start..self.i]).into_owned();
            return GTok::Id(text);
        }
        const OPS: &[&str] = &[
            "<<", ">>", "||", "&&", "==", "!=", "<>", "<=", ">=", "=<", "=>", "+=", "-=", "*=",
            "/=", "%=", "@=", ":=", "++", "--", "+", "-", "*", "/", "%", "^", "=", "<", ">", "!",
            "(", ")", "{", "}", "[", "]", ",", ";", ".", "@", "|", "&", ":", "?",
        ];
        let rest = &self.b[self.i..];
        for op in OPS {
            if rest.starts_with(op.as_bytes()) {
                self.i += op.len();
                return GTok::Op(op);
            }
        }
        self.i += 1;
        GTok::Op("unknown character")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_follow_the_notes() {
        assert_eq!(gs2_event_function("created"), "onCreated");
        assert_eq!(gs2_event_function("timeout"), "onTimeout");
        assert_eq!(gs2_event_function("playerenters"), "onPlayerEnters");
        assert_eq!(gs2_event_function("playerchats"), "onPlayerChats");
        assert_eq!(gs2_event_function("playertouchsme"), "onPlayerTouchsMe");
        assert_eq!(gs2_event_function("weaponfired"), "onWeaponFired");
        assert_eq!(gs2_event_function("playerleaves"), "onPlayerLeaves");
        assert_eq!(gs2_event_function("playerlogin"), "onPlayerLogin");
        assert_eq!(gs2_event_function("playerlogout"), "onPlayerLogout");
    }

    #[test]
    fn client_command_list_is_sorted() {
        let mut sorted = GS2_CLIENT_COMMANDS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, GS2_CLIENT_COMMANDS);
    }
}
