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
//! Array literals use the documented reverse-push encoding. GS1 lowering is
//! limited to scalar elements without calls: the GS1 interpreter evaluates
//! elements left to right, so calls need a separate order-preserving lowering.
//!
//! Indexed reads and writes (`a[i]`, `a[i, j]`, `a[i] = v`, `a[i, j] = v`)
//! and `x in array` use the encodings in `tests/fixtures/gs2/lists.gs2bc`:
//! the variable, `OP_CONV_TO_OBJECT`, the indexes, then the value for a write,
//! and `OP_ARRAY` / `OP_ARRAY_MULTIDIM` / `OP_ARRAY_ASSIGN` /
//! `OP_ARRAY_MULTIDIM_ASSIGN` / `OP_IN_OBJ`. The fixture only has constant
//! indexes; a computed index gets `OP_CONV_TO_FLOAT` like every other numeric
//! operand here, which is Seihai's choice, not a fixture-checked encoding.
//! Compound assignment and `++`/`--` on an element, a field after an index
//! (`npcs[i].x`), three or more indexes and the `board`/`tiles` arrays have no
//! fixture and stay compile errors.

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
/// `setlevel2`, `setplayerprop`), or when `gs2-opcodes.md` gives it an
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
            "set" | "unset" | "setlevel" | "setlevel2" | "setplayerprop"
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
        Expr::Bin(BinOp::And, a, b) => match (event_var(a, era), event_var(b, era)) {
            (Some(ev), None) => Arms::Events(vec![(ev, Some((**b).clone()))]),
            (None, Some(ev)) => Arms::Events(vec![(ev, Some((**a).clone()))]),
            _ => {
                if contains_event(e, era) {
                    Arms::Complex
                } else {
                    Arms::NotEvent
                }
            }
        },
        _ => {
            if contains_event(e, era) {
                Arms::Complex
            } else {
                Arms::NotEvent
            }
        }
    }
}

fn event_var(e: &Expr, era: Era) -> Option<String> {
    match e {
        Expr::Var(v) if is_event(&v.name, era) => Some(v.name.clone()),
        _ => None,
    }
}

struct Item {
    name: String,
    params: Vec<String>,
    body: Vec<Stmt>,
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

fn lift_gs1(body: Vec<Stmt>, era: Era) -> Result<Vec<Item>, CompileErrors> {
    let mut items = Vec::new();
    let mut errors = Vec::new();
    for stmt in body {
        match stmt {
            Stmt::Function { name, params, body } => items.push(Item { name, params, body }),
            Stmt::If { cond, then, els } => match event_arms(&cond, era) {
                Arms::NotEvent => errors.push(CompileError {
                    line: 0,
                    message: "top-level statement is not an event test or a function".into(),
                }),
                Arms::Complex => errors.push(CompileError {
                    line: 0,
                    message: "event test is too complex for this temporary front end; split it into one if per event".into(),
                }),
                Arms::Events(arms) => {
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
                        items.push(Item { name: gs2_event_function(&event), params: Vec::new(), body });
                    }
                }
            },
            _ => errors.push(CompileError {
                line: 0,
                message: "top-level statement is not an event test or a function".into(),
            }),
        }
    }
    if !errors.is_empty() {
        return Err(CompileErrors { errors });
    }
    Ok(items)
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
}

fn emit_items(items: &[Item], gs1: bool, era: Era) -> Result<Built, CompileErrors> {
    let mut seen = HashMap::<String, ()>::new();
    let mut errors = Vec::new();
    for item in items {
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
    };
    for item in items {
        b.function(item);
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
        for p in item.params.iter().rev() {
            self.emit_var(p);
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
            Expr::Var(v) => self.index_has_call(v),
            Expr::Num(_) | Expr::Str(_) => false,
        }
    }

    fn name_is_call(&self, name: &str) -> bool {
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
                if !target.index.is_empty() {
                    self.err(format!(
                        "`{}` on an array element has no fixture-checked GS2 form; write `a[i] = a[i] {} 1`",
                        if *delta < 0.0 { "--" } else { "++" },
                        if *delta < 0.0 { "-" } else { "+" },
                    ));
                    return;
                }
                self.place(&target.name);
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
            Stmt::With { line, .. } => {
                self.line = *line;
                self.err("`with` is not lowered by this temporary front end");
            }
            Stmt::Unknown {
                name, reason, line, ..
            } => {
                self.line = *line;
                self.err(format!("`{name}` cannot be lowered ({reason})"));
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
            AssignOp::Set => unreachable!(),
        });
        self.op(Op::Assign);
    }

    /// `a[i] = v` and `a[i, j] = v`: object, indexes, value, assign op.
    fn element_assign(&mut self, target: &VarRef, op: AssignOp, value: &Expr) {
        if op != AssignOp::Set {
            self.err(format!(
                "`{}` on an array element has no fixture-checked GS2 form; write `a[i] = a[i] {} v`",
                op.symbol(),
                op.symbol().trim_end_matches('='),
            ));
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
        if let Some(problem) = element_problem(v) {
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
        if self.gs1 && !gs2_has_command(name) {
            self.err(format!(
                "GS1 command `{name}` has no GS2 equivalent{}",
                command_hint(name)
            ));
            return;
        }
        let exprs: Vec<Expr> = args.iter().map(arg_to_expr).collect();
        self.call_ops(name, &exprs);
        self.op(Op::IndexDec);
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
            "sin" | "cos" | "arctan" | "abs" | "random" | "exp" | "log" | "min" | "max" | "pow"
            | "char" | "getangle" | "getdir" | "vecx" | "vecy" | "waitfor" | "makevar" => {
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
        if let Some((root, member)) = name.split_once('.') {
            self.root(root);
            self.emit_var(member);
            self.op(Op::MemberAccess);
        } else {
            self.root(name);
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
            Expr::InList { .. } => {
                self.err("`in {list}` is not lowered by this temporary front end");
                self.emit_int(0);
                Kind::Number
            }
            Expr::InArray { e, name } => {
                self.value(e);
                self.place(name);
                self.op(Op::ConvToObject);
                self.op(Op::InObj);
                Kind::Other
            }
            Expr::List(values) => {
                if self.gs1 && !values.iter().all(gs1_array_element) {
                    self.err("GS1 array elements with calls or nested lists are not lowered yet");
                    self.emit_int(0);
                    return Kind::Number;
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

/// Why an indexed variable is not lowered, if it is not.
fn element_problem(v: &VarRef) -> Option<String> {
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
    if matches!(v.name.as_str(), "board" | "tiles") {
        return Some(format!(
            "`{}[..]` has no fixture-checked GS2 encoding; it is not lowered",
            v.name
        ));
    }
    None
}

/// Scalar expressions whose evaluation cannot call script or host code.
fn gs1_array_element(expr: &Expr) -> bool {
    match expr {
        Expr::Num(_) | Expr::Str(_) => true,
        Expr::Var(v) => v.index.iter().all(gs1_array_element),
        Expr::Unary(_, value) => gs1_array_element(value),
        Expr::Bin(_, left, right) => gs1_array_element(left) && gs1_array_element(right),
        Expr::InRange { e, lo, hi } => {
            gs1_array_element(e) && gs1_array_element(lo) && gs1_array_element(hi)
        }
        Expr::Cond { cond, then, els } => {
            gs1_array_element(cond) && gs1_array_element(then) && gs1_array_element(els)
        }
        Expr::Call { .. } | Expr::List(_) | Expr::InList { .. } | Expr::InArray { .. } => false,
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
        if !p.eat_id("function") {
            return Err(CompileErrors::one(
                p.line,
                "only functions are accepted by this temporary GS2 compiler",
            ));
        }
        items.push(p.function()?);
    }
    Ok(items)
}

struct Gs2<'a> {
    b: &'a [u8],
    i: usize,
    line: u32,
    peek: Option<GTok>,
    bad: Option<String>,
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
        }
    }

    fn eof(&mut self) -> bool {
        matches!(self.peek_tok(), GTok::Eof)
    }

    fn function(&mut self) -> Result<Item, CompileErrors> {
        let name = self.ident()?;
        self.expect("(")?;
        let mut params = Vec::new();
        if !self.eat(")") {
            loop {
                params.push(self.ident()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
        }
        self.expect("{")?;
        let body = self.block_body()?;
        if let Some(message) = self.bad.take() {
            return Err(CompileErrors::one(self.line, message));
        }
        Ok(Item { name, params, body })
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
        if self.eat_id("switch")
            || self.eat_id("with")
            || self.eat_id("new")
            || self.eat_id("public")
        {
            return Err(CompileErrors::one(
                self.line,
                "that statement is not in this temporary GS2 compiler",
            ));
        }
        let e = self.expr()?;
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
                let value = self.expr()?;
                self.expect(";")?;
                return Ok(Stmt::Assign {
                    target: v.clone(),
                    op,
                    value,
                });
            }
        }
        self.expect(";")?;
        match e {
            Expr::Call { name, args } => Ok(Stmt::Call { name, args }),
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
        if self.sees(":") {
            return Err(CompileErrors::one(
                self.line,
                "for-each is not in this temporary GS2 compiler",
            ));
        }
        let init = if self.eat(";") {
            None
        } else {
            Some(Box::new(self.simple()?))
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

    fn expr(&mut self) -> Result<Expr, CompileErrors> {
        self.binary(1)
    }

    fn binary(&mut self, min: u8) -> Result<Expr, CompileErrors> {
        let mut lhs = self.unary()?;
        loop {
            // `x in arr`, at the equality level. Only a variable on the right.
            if min <= 6 && self.eat_id("in") {
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
            let op = match text {
                "||" => BinOp::Or,
                "&&" => BinOp::And,
                "@" => BinOp::Cat,
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
        let GTok::Op(text) = tok else { return None };
        let (prec, op) = match text {
            "||" => (1, BinOp::Or),
            "&&" => (2, BinOp::And),
            "@" => (3, BinOp::Cat),
            "<" | "<=" => (4, if text == "<" { BinOp::Lt } else { BinOp::Le }),
            ">" | ">=" => (5, if text == ">" { BinOp::Gt } else { BinOp::Ge }),
            "==" | "!=" => (6, if text == "==" { BinOp::Eq } else { BinOp::Ne }),
            "+" | "-" => (7, if text == "+" { BinOp::Add } else { BinOp::Sub }),
            "*" | "/" | "%" | "^" => (
                8,
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
        match self.bump() {
            GTok::Num(n) => Ok(Expr::Num(n)),
            GTok::Str(s) => Ok(Expr::Str(s)),
            GTok::Id(mut name) => {
                if self.eat("(") {
                    let args = self.arg_list()?;
                    return Ok(Expr::Call { name, args });
                }
                while self.eat(".") {
                    name.push('.');
                    name.push_str(&self.ident()?);
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
                let field_after_index = !index.is_empty() && self.sees(".");
                if field_after_index || self.sees("[") {
                    return Err(CompileErrors::one(
                        self.line,
                        "only `a[i]` and `a[i, j]` are in this temporary GS2 compiler",
                    ));
                }
                Ok(Expr::Var(VarRef {
                    name,
                    index,
                    field_after_index,
                }))
            }
            GTok::Op("(") => {
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
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
        if c == b'"' {
            let start = self.i + 1;
            self.i = start;
            while self.i < self.b.len() && self.b[self.i] != b'"' {
                if self.b[self.i] == b'\\' {
                    self.bad = Some("string escapes are not in this temporary compiler".into());
                }
                if self.b[self.i] == b'\n' {
                    self.line += 1;
                }
                self.i += 1;
            }
            let text = String::from_utf8_lossy(&self.b[start..self.i]).into_owned();
            if self.i < self.b.len() {
                self.i += 1;
            }
            return GTok::Str(text);
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
            while self.i < self.b.len()
                && (self.b[self.i].is_ascii_alphanumeric()
                    || self.b[self.i] == b'_'
                    || self.b[self.i] == b'$')
            {
                self.i += 1;
            }
            let text = String::from_utf8_lossy(&self.b[start..self.i]).into_owned();
            return GTok::Id(text);
        }
        const OPS: &[&str] = &[
            "||", "&&", "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=", ":=", "++", "--", "+", "-",
            "*", "/", "%", "^", "=", "<", ">", "!", "(", ")", "{", "}", "[", "]", ",", ";", ".",
            "@", "|", ":", "?",
        ];
        let rest = &self.b[self.i..];
        for op in OPS {
            if rest.starts_with(op.as_bytes()) {
                self.i += op.len();
                return GTok::Op(op);
            }
        }
        self.i += 1;
        GTok::Op("?")
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
    }

    #[test]
    fn client_command_list_is_sorted() {
        let mut sorted = GS2_CLIENT_COMMANDS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, GS2_CLIENT_COMMANDS);
    }
}
