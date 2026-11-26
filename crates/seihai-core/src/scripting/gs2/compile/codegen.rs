//! Code generation: the compiler's tree to GS2 bytecode.
//!
//! The target is the official server-side compiler's output, as described
//! in Graal_analysis `npc-library/gs2-official-emit-spec-2026-11-26.md`
//! and checked against the official captures listed there: the variable
//! cache (`cache`), `params` as `OP_PARAMS`, the smallest operand for every
//! jump, no closing `OP_RET`, jump threading and the hash-ordered function
//! table. Where that note is silent, the encodings are the ones in
//! `documentation/gs2-opcodes.md`, the gs2-parser fixtures in
//! `tests/fixtures/gs2`, and black-box probes of the open compilers. GS1
//! input keeps the older choices where no official GS1 output was seen.
//! Where Seihai departs from gs2-parser on purpose, the comment says so:
//!
//! - after the last `switch` arm Seihai jumps out, so a last arm without
//!   `break` does not run on into the case tests;
//! - a call that is a brace-less `if` body has its value popped;
//! - `a[i] op= v` stores into `a[i]` (gs2-parser stores `i op v`);
//! - `(x += 1)` as a value leaves `x` (gs2-parser leaves nothing);
//! - `obj.(@n)` pushes the name as text (gs2-parser adds it as numbers).
//!
//! Numbers are pushed as numbers: an operand that is not known to be a
//! number gets `OP_CONV_TO_FLOAT`, one that is not known to be text gets
//! `OP_CONV_TO_STRING` where text is wanted.
//!
//! Opcode built-ins require their documented argument counts; `format`
//! needs a pattern and accepts additional values. This is Seihai's
//! compile-time policy, not a claim about original-client errors. Script
//! and host calls are not checked.
//!
//! Indexed reads and writes (`a[i]`, `a[i, j]`, `a[i] = v`) and `x in arr`
//! use the encodings in `lists.gs2bc`: the variable, `OP_CONV_TO_OBJECT`,
//! the indexes, then the value for a write, and `OP_ARRAY` /
//! `OP_ARRAY_MULTIDIM` / `OP_ARRAY_ASSIGN` / `OP_ARRAY_MULTIDIM_ASSIGN` /
//! `OP_IN_OBJ`. A computed index gets `OP_CONV_TO_FLOAT` like every other
//! numeric operand, which is Seihai's choice. Three or more indexes are an
//! error.

use super::super::{Function as FnEntry, Instr, Op, Operand, Script};
use super::ast::*;
use super::cache;
use super::gs1::name_is_call;
use super::tables::*;
use super::{CompileError, CompileErrors};
use std::collections::{HashMap, HashSet};

/// What a value is known to be, to decide on conversions.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Number,
    String,
    Other,
}

struct Loop {
    continues: Vec<usize>,
    breaks: Vec<usize>,
    /// The name `break name;` and `continue name;` use, if any.
    label: Option<String>,
}

/// The functions a later one of the same name replaces.
pub(super) fn replaced_functions(program: &Program) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut replaced = Vec::new();
    for f in functions(program) {
        if !seen.insert(f.name.as_str()) {
            replaced.push(f.name.clone());
        }
    }
    replaced
}

fn functions(program: &Program) -> impl Iterator<Item = &Function> {
    program
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Function(f) => Some(f),
            Item::Code(_) => None,
        })
        .chain(&program.lambdas)
}

pub(super) fn generate(program: &Program) -> Result<Script, CompileErrors> {
    // The index of the last function of each name: with `later_wins` it
    // replaces the earlier ones; without, two of a name are an error.
    let mut last = HashMap::<&str, usize>::new();
    let mut errors = Vec::new();
    for (i, f) in functions(program).enumerate() {
        if last.insert(f.name.as_str(), i).is_some() && !program.later_wins {
            errors.push(CompileError {
                line: 0,
                message: format!("duplicate function `{}`", f.name),
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
        label: None,
        skips: Vec::new(),
        line: 0,
        cache: HashMap::new(),
        gs1: program.gs1,
        lambdas: program
            .lambdas
            .iter()
            .map(|f| (f.name.clone(), f.clone()))
            .collect(),
    };
    let mut nth = 0;
    let mut top_level = Vec::new();
    for item in &program.items {
        match item {
            Item::Code(body) => {
                // The skips over earlier functions land here, so code after
                // a function still runs in source order. With no code after
                // the last function they stay aimed at the end, as in the
                // fixtures.
                let here = b.here();
                for skip in std::mem::take(&mut b.skips) {
                    b.patch(skip, here);
                }
                let start = b.here();
                for s in body {
                    b.stmt(s);
                }
                top_level.push(start..b.here());
            }
            Item::Function(f) => {
                // Official bodies keep every copy of a function on an
                // object (`obj.f`), each listed in the table.
                let on_object = {
                    let n = f.name.strip_prefix("public.").unwrap_or(&f.name);
                    n.contains('.')
                };
                if last.get(f.name.as_str()) == Some(&nth) || on_object {
                    b.function(f, false);
                }
                nth += 1;
            }
        }
    }
    // Official bodies end with the last instruction of the script: no
    // closing `OP_RET` (gs2-parser and older Seihai wrote one).
    let end = b.here();
    for skip in std::mem::take(&mut b.skips) {
        b.patch(skip, end);
    }
    if !b.errors.is_empty() {
        return Err(CompileErrors { errors: b.errors });
    }
    thread_jumps(&mut b.code);
    let gs1_flags = if program.gs1 {
        0
    } else {
        event_mask(&b, &top_level)
    };
    let script = Script {
        gs1_flags,
        functions: official_function_order(b.functions),
        strings: b.strings,
        code: b.code,
    };
    Script::parse(&script.write()).map_err(|message| CompileErrors {
        errors: vec![CompileError { line: 0, message }],
    })
}

/// A jump to an unconditional jump goes straight to where that one goes,
/// as in official bodies (none of their jumps lands on an `OP_SET_INDEX`).
/// `OP_WITH` and `OP_FOREACH` keep their targets.
fn thread_jumps(code: &mut [Instr]) {
    let target = |code: &[Instr], at: usize| -> Option<i32> {
        let mut t = match code.get(at)?.operand {
            Operand::Int(t) => t,
            _ => return None,
        };
        let mut seen = HashSet::new();
        while let Some(next) = usize::try_from(t).ok().and_then(|i| code.get(i)) {
            match (next.op, &next.operand) {
                (Op::SetIndex, Operand::Int(n)) if seen.insert(t) => t = *n,
                _ => break,
            }
        }
        Some(t)
    };
    for at in 0..code.len() {
        if matches!(
            code[at].op,
            Op::SetIndex | Op::SetIndexTrue | Op::Or | Op::If | Op::And
        ) {
            if let Some(t) = target(code, at) {
                code[at].operand = Operand::Int(t);
            }
        }
    }
}

/// The name hash the engine's string hash list uses: start at `0x1505`,
/// and for each byte (ASCII capitals folded to lower case, other bytes
/// read as signed) multiply by 17 and xor the byte in.
pub(super) fn engine_name_hash(name: &str) -> u32 {
    let mut h: u32 = 0x1505;
    for &b in name.as_bytes() {
        let c = if b.is_ascii_uppercase() {
            (b + 32) as u32
        } else {
            b as i8 as i32 as u32
        };
        h = h.wrapping_mul(17) ^ c;
    }
    h
}

/// Orders items the way the engine's hash list walks them: 32 buckets to
/// start, eight times as many each time the count reaches the bucket count
/// (moving the old entries bucket by bucket), each new entry put at the
/// front of its bucket, and the walk bucket by bucket from the front.
pub(super) fn engine_hash_order<T>(items: Vec<T>, name: impl Fn(&T) -> String) -> Vec<T> {
    let mut buckets: Vec<Vec<(u32, T)>> = (0..32).map(|_| Vec::new()).collect();
    for (count, item) in items.into_iter().enumerate() {
        if count == buckets.len() {
            let grown = buckets.len() * 8;
            let old = std::mem::replace(&mut buckets, (0..grown).map(|_| Vec::new()).collect());
            for (h, it) in old.into_iter().flatten() {
                buckets[h as usize & (grown - 1)].insert(0, (h, it));
            }
        }
        let h = engine_name_hash(&name(&item));
        let n = buckets.len();
        buckets[h as usize & (n - 1)].insert(0, (h, item));
    }
    buckets.into_iter().flatten().map(|(_, it)| it).collect()
}

/// Segment 2 as official bodies write it: plain functions in hash-list
/// order of their bare names (without `public.`), then the functions on an
/// object (`obj.f`, `universe` entries) in source order.
fn official_function_order(functions: Vec<FnEntry>) -> Vec<FnEntry> {
    let bare = |f: &FnEntry| -> String {
        let n = f.name.split(',').next().unwrap_or_default();
        n.strip_prefix("public.").unwrap_or(n).to_string()
    };
    // A `universe` function is listed without `public.`, and a plain
    // `onServerListerConnect` is listed a second time as
    // `universe.onServerListerConnect`, in source order with the others.
    let mut plain = Vec::new();
    let mut qualified = Vec::new();
    for mut f in functions {
        let name = bare(&f);
        if name.contains('.') {
            if name.starts_with("universe.") {
                f.name = name;
            }
            qualified.push(f);
        } else {
            if name == "onServerListerConnect" {
                qualified.push(FnEntry {
                    name: format!("universe.{name}"),
                    start: f.start,
                });
            }
            plain.push(f);
        }
    }
    let mut out = engine_hash_order(plain, |f| bare(f));
    out.extend(qualified);
    out
}

struct Builder {
    code: Vec<Instr>,
    strings: Vec<String>,
    index: HashMap<String, u32>,
    functions: Vec<FnEntry>,
    errors: Vec<CompileError>,
    loops: Vec<Loop>,
    /// The label for the next loop opened (`name: for …`).
    label: Option<String>,
    /// Jumps over function bodies, patched to the next code outside them.
    skips: Vec<usize>,
    line: u32,
    /// `function () { … }` values by function name, emitted where used.
    lambdas: HashMap<String, Function>,
    /// The running function's variable-cache slots (see `cache`).
    cache: HashMap<String, i32>,
    /// Lowered from GS1 (see `Program::gs1`).
    gs1: bool,
}

// ---- what counts as a call ----

/// Whether a statement calls anything, for the function's `OP_CMD_CALL`.
pub(super) fn stmt_calls(s: &Stmt) -> bool {
    let index = |v: &Var| v.index.iter().any(expr_calls);
    match s {
        Stmt::Block(v) => v.iter().any(stmt_calls),
        Stmt::If { cond, then, els } => {
            expr_calls(cond) || stmt_calls(then) || els.as_deref().is_some_and(stmt_calls)
        }
        Stmt::While { cond, body } => expr_calls(cond) || stmt_calls(body),
        Stmt::For {
            init,
            cond,
            step,
            body,
        } => {
            init.as_deref().is_some_and(stmt_calls)
                || cond.as_ref().is_some_and(expr_calls)
                || step.as_deref().is_some_and(stmt_calls)
                || stmt_calls(body)
        }
        // A `new … { }` block in the loop body counts as a call.
        Stmt::ForEach { list, body, .. } => {
            expr_calls(list) || stmt_calls(body) || has_new_block(body)
        }
        Stmt::Switch { value, arms } => {
            expr_calls(value)
                || arms.iter().flat_map(|a| &a.values).any(expr_calls)
                || arms.iter().flat_map(|a| &a.body).any(stmt_calls)
        }
        Stmt::Return(v) => v.as_ref().is_some_and(expr_calls),
        Stmt::With { target, body, .. } => expr_calls(target) || stmt_calls(body),
        Stmt::NewBlock { name, body, .. } => {
            name.as_ref().is_some_and(expr_calls) || body.iter().any(stmt_calls)
        }
        Stmt::Assign { target, value, .. } => index(target) || expr_calls(value),
        Stmt::IncDec { target, .. } => index(target),
        Stmt::Chain { targets, value } => targets.iter().any(index) || expr_calls(value),
        Stmt::SetPart { target, value } => expr_calls(target) || expr_calls(value),
        // A built-in whose value goes unused is left out, calls and all.
        Stmt::Call { name, .. } if unused_builtin(name) => false,
        Stmt::Call { name, args } => name_is_call(name, false) || args.iter().any(expr_calls),
        Stmt::Eval(Expr::Method { name, .. }) if value_method(name) => false,
        Stmt::Eval(e) => expr_calls(e),
        Stmt::Embedded { calls, .. } => *calls,
        Stmt::Break | Stmt::Continue | Stmt::BreakTo(_) | Stmt::ContinueTo(_) => false,
        Stmt::Labeled { body, .. } => stmt_calls(body),
    }
}

fn has_new_block(s: &Stmt) -> bool {
    match s {
        Stmt::NewBlock { .. } => true,
        Stmt::Block(v) => v.iter().any(|s| matches!(s, Stmt::NewBlock { .. })),
        _ => false,
    }
}

/// Whether an expression calls anything. Opcode built-ins other than
/// `format` do not; a method, `new`, or a call by a run-time name does.
/// GS1 text with message codes does not.
fn expr_calls(e: &Expr) -> bool {
    match e {
        // A folded constant calls nothing (`false && f()`).
        e if const_bool(e).is_some() => false,
        // Official bodies treat `format` like the other opcode built-ins.
        Expr::Call { name, args } => {
            (name != "format" && name_is_call(name, false)) || args.iter().any(expr_calls)
        }
        // The inline string and list methods are not calls.
        Expr::Method { name, .. } if cache::inline_method(name) => false,
        // `new T(args)` calls only when an argument does.
        Expr::New { args, .. } => args.iter().any(expr_calls),
        Expr::HostCall { .. }
        | Expr::Method { .. }
        | Expr::DynMethod { .. }
        | Expr::DynCall { .. } => true,
        Expr::Lambda { calls, .. } => *calls,
        Expr::Message(_) => false,
        other => other.parts().into_iter().any(expr_calls),
    }
}

fn is_comparison(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Binary(
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge,
            _,
            _
        )
    )
}

fn is_and(e: &Expr) -> bool {
    matches!(e, Expr::Binary(BinOp::And, _, _))
}

fn is_or(e: &Expr) -> bool {
    matches!(e, Expr::Binary(BinOp::Or, _, _))
}

fn is_logic(e: &Expr) -> bool {
    match e {
        Expr::Binary(BinOp::And | BinOp::Or, _, _) => true,
        Expr::Not(x) => is_logic(x),
        _ => false,
    }
}

fn arith(op: AssignOp) -> Op {
    match op {
        AssignOp::Add => Op::Add,
        AssignOp::Sub => Op::Sub,
        AssignOp::Mul => Op::Mul,
        AssignOp::Div => Op::Div,
        AssignOp::Mod => Op::Mod,
        AssignOp::BitAnd => Op::BitAnd,
        AssignOp::BitOr => Op::BitOr,
        AssignOp::Pow => Op::Pow,
        AssignOp::Shl => Op::ShiftLeft,
        AssignOp::Shr => Op::ShiftRight,
        AssignOp::Cat => Op::Join,
        AssignOp::Set => unreachable!("`=` has no operator"),
    }
}

fn element_op(dims: usize) -> Op {
    if dims == 1 {
        Op::Array
    } else {
        Op::ArrayMultidim
    }
}

impl Builder {
    // ---- emitting ----

    fn here(&self) -> usize {
        self.code.len()
    }

    fn err(&mut self, message: impl Into<String>) {
        self.errors.push(CompileError {
            line: self.line,
            message: message.into(),
        });
    }

    fn push(&mut self, op: Op, operand: Operand) {
        self.code.push(Instr {
            op,
            operand,
            offset: 0,
        });
    }

    fn op(&mut self, op: Op) {
        self.push(op, Operand::None);
    }

    /// A jump whose target is patched later.
    fn hole(&mut self, op: Op) -> usize {
        let at = self.here();
        self.push(op, Operand::Int(0));
        at
    }

    fn patch(&mut self, at: usize, target: usize) {
        self.code[at].operand = Operand::Int(target as i32);
    }

    fn jump(&mut self, op: Op, target: usize) {
        self.push(op, Operand::Int(target as i32));
    }

    /// Text beyond latin1 is written as its UTF-8 bytes, which is what a
    /// byte-oriented compiler gives for a UTF-8 source file.
    fn intern(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.index.get(s) {
            return i;
        }
        let i = self.strings.len() as u32;
        self.index.insert(s.to_string(), i);
        self.strings.push(s.to_string());
        i
    }

    fn emit_var(&mut self, name: &str) {
        let n = self.intern(name);
        self.push(Op::TypeVar, Operand::Str(n));
    }

    fn emit_str(&mut self, text: &str) {
        let n = self.intern(text);
        self.push(Op::TypeString, Operand::Str(n));
    }

    fn emit_int(&mut self, n: i32) {
        self.push(Op::TypeNumber, Operand::Int(n));
    }

    /// A number: as written, when the script wrote it differently from its
    /// normalised text; else an integer where it is one, else its text.
    /// A number as official bodies write it (literals and folded values
    /// alike): an integer when the value is within 0.0001 of one; past the
    /// `i32` range, that range's end as text; otherwise the integer part
    /// and the first nine decimals, cut rather than rounded, except that a
    /// run ending in 999 is carried up.
    fn emit_number(&mut self, n: f64) {
        let near = n.round();
        if (i32::MIN as f64..=i32::MAX as f64).contains(&near) && (n - near).abs() <= 0.0001 {
            self.emit_int(near as i32);
            return;
        }
        let sign = if n < 0.0 { "-" } else { "" };
        let mag = n.abs();
        if !mag.is_finite() || mag > i32::MAX as f64 {
            self.push(
                Op::TypeNumber,
                Operand::Decimal(format!("{sign}{}", i32::MAX)),
            );
            return;
        }
        let mut whole = mag.trunc() as i64;
        let mut frac = ((mag - mag.trunc()) * 1e9) as i64;
        if frac % 1000 == 999 {
            frac += 1;
            if frac == 1_000_000_000 {
                whole += 1;
                frac = 0;
            }
        }
        let mut text = format!("{sign}{whole}");
        if frac != 0 {
            let digits = format!("{frac:09}");
            text.push('.');
            text.push_str(digits.trim_end_matches('0'));
        }
        self.push(Op::TypeNumber, Operand::Decimal(text));
    }

    fn new_loop(&mut self) {
        self.loops.push(Loop {
            continues: Vec::new(),
            breaks: Vec::new(),
            label: self.label.take(),
        });
    }

    // ---- functions ----

    /// A function where it stands: a jump over it, the parameter list,
    /// `OP_CMD_CALL` when the body calls anything, the body, and `return 0`
    /// unless the body always returns.
    fn function(&mut self, f: &Function, lambda: bool) {
        let skip = self.hole(Op::SetIndex);
        let start = self.here();
        self.functions.push(FnEntry {
            name: f.name.clone(),
            start,
        });
        self.op(Op::TypeArray);
        // A `temp.x` parameter is the place `temp.x` (TEMP, x, member
        // access in the Preagonal baselines).
        for p in f.params.iter().rev() {
            if p == "params" {
                self.op(Op::Params);
            } else if p.contains('.') {
                self.place(p);
            } else {
                self.emit_var(p);
            }
        }
        self.op(Op::FuncParamsEnd);
        self.op(Op::Jmp);
        if f.cmd_call.unwrap_or_else(|| f.body.iter().any(stmt_calls)) {
            self.op(Op::CmdCall);
        }
        let outer = std::mem::take(&mut self.cache);
        if !lambda {
            let (slots, order) = cache::slots(&f.body);
            for (n, key) in order.iter().enumerate() {
                match key.split_once('.') {
                    _ if key == "params" => self.op(Op::Params),
                    Some((root, member)) => {
                        self.root(root);
                        self.emit_var(member);
                        self.op(Op::MemberAccess);
                    }
                    None => self.emit_var(key),
                }
                self.op(Op::Unknown(cache::RESOLVE));
                self.push(Op::Unknown(cache::STORE), Operand::Int(n as i32));
                self.op(Op::IndexDec);
            }
            self.cache = slots;
        }
        for s in &f.body {
            self.stmt(s);
        }
        self.cache = outer;
        if !ends_with_return(&f.body) {
            self.emit_int(0);
            self.op(Op::Ret);
        }
        self.skips.push(skip);
    }

    // ---- statements ----

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Block(v) => v.iter().for_each(|s| self.stmt(s)),
            Stmt::If { cond, then, els } => self.if_stmt(cond, then, els.as_deref()),
            Stmt::While { cond, body } => self.while_stmt(cond, body),
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => self.for_stmt(init.as_deref(), cond.as_ref(), step.as_deref(), body),
            Stmt::ForEach { var, list, body } => self.foreach_stmt(var, list, body),
            Stmt::Switch { value, arms } => self.switch_stmt(value, arms),
            Stmt::Return(v) => {
                match v {
                    Some(v) => {
                        self.value(v);
                    }
                    None => self.emit_int(0),
                }
                self.op(Op::Ret);
            }
            Stmt::Break | Stmt::Continue => {
                let Some(frame) = self.loops.len().checked_sub(1) else {
                    let word = if matches!(s, Stmt::Break) {
                        "break"
                    } else {
                        "continue"
                    };
                    self.err(format!("`{word}` outside a loop"));
                    return;
                };
                let h = self.hole(Op::SetIndex);
                let frame = &mut self.loops[frame];
                if matches!(s, Stmt::Break) {
                    frame.breaks.push(h);
                } else {
                    frame.continues.push(h);
                }
            }
            Stmt::Labeled { label, body } => {
                self.label = Some(label.clone());
                self.stmt(body);
                self.label = None;
            }
            Stmt::BreakTo(label) | Stmt::ContinueTo(label) => {
                let Some(frame) = self
                    .loops
                    .iter()
                    .rposition(|l| l.label.as_deref() == Some(label.as_str()))
                else {
                    self.err(format!("no loop named `{label}`"));
                    return;
                };
                let h = self.hole(Op::SetIndex);
                let frame = &mut self.loops[frame];
                if matches!(s, Stmt::BreakTo(_)) {
                    frame.breaks.push(h);
                } else {
                    frame.continues.push(h);
                }
            }
            Stmt::With { target, body, line } => self.with_stmt(target, body, *line),
            Stmt::NewBlock { ty, name, body } => self.new_block(ty, name.as_ref(), body),
            Stmt::Assign { target, op, value } => self.assign(target, *op, value),
            Stmt::IncDec { target, delta } => {
                if target.index.is_empty() {
                    self.place(&target.name);
                } else {
                    // `a[i]++` (Preagonal probe): the element read
                    // (`OP_ARRAY`), then `OP_INC` on it.
                    let Some(dims) = self.element(target) else {
                        return;
                    };
                    self.op(element_op(dims));
                }
                self.op(if *delta < 0.0 { Op::Dec } else { Op::Inc });
                self.op(Op::IndexDec);
            }
            Stmt::Chain { targets, value } => self.chain(targets, value),
            Stmt::SetPart { target, value } => self.set_part(target, value),
            // GS2 leaves out a built-in whose value is not used.
            Stmt::Call { name, args }
                if !self.gs1 && unused_builtin(name) && builtin_args_fit(name, args.len()) => {}
            Stmt::Eval(Expr::Method { name, .. }) if !self.gs1 && value_method(name) => {}
            Stmt::Call { name, args } => {
                if self.call(name, args) {
                    self.op(Op::IndexDec);
                }
            }
            Stmt::Eval(e) => {
                if let Expr::Method { obj, name, args } = e {
                    if self.method(obj, name, args) {
                        self.op(Op::IndexDec);
                    }
                    return;
                }
                self.value(e);
                self.op(Op::IndexDec);
            }
            Stmt::Embedded { body, .. } => self.stmt(body),
        }
    }

    fn if_stmt(&mut self, cond: &Expr, then: &Stmt, els: Option<&Stmt>) {
        // With nothing to run on either side, official bodies keep only the
        // test as a dropped value, and drop a constant test altogether.
        let empty = |s: &Stmt| matches!(s, Stmt::Block(v) if v.is_empty());
        if empty(then) && els.is_none_or(empty) {
            match cond {
                Expr::Num(_) | Expr::Str(_) => {}
                Expr::Var(v)
                    if v.index.is_empty() && matches!(v.name.as_str(), "true" | "false") => {}
                e if is_and(e) || is_or(e) => {
                    self.logic_operand(e);
                    self.op(Op::IndexDec);
                }
                e => {
                    self.value(e);
                    self.op(Op::IndexDec);
                }
            }
            return;
        }
        let mut fails = Vec::new();
        self.cond_fail(cond, &mut fails);
        self.stmt(then);
        let jump = els.map(|_| self.hole(Op::SetIndex));
        let else_at = self.here();
        for f in fails {
            self.patch(f, else_at);
        }
        if let (Some(els), Some(jump)) = (els, jump) {
            self.stmt(els);
            self.patch(jump, self.here());
        }
    }

    fn while_stmt(&mut self, cond: &Expr, body: &Stmt) {
        let test = self.here();
        let mut fails = Vec::new();
        self.cond_fail(cond, &mut fails);
        self.op(Op::CmdCall);
        self.new_loop();
        self.stmt(body);
        let frame = self.loops.pop().unwrap();
        for c in frame.continues {
            self.patch(c, test);
        }
        self.jump(Op::SetIndex, test);
        let end = self.here();
        for at in fails.into_iter().chain(frame.breaks) {
            self.patch(at, end);
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
        match cond {
            // GS2: a `&&`/`||` test of a `for` is the chain as a value, then
            // one `OP_IF` (official captures); `if` and `while` split it.
            Some(cond) if !self.gs1 && (is_and(cond) || is_or(cond)) => {
                self.logic(cond);
                fails.push(self.hole(Op::If));
            }
            Some(cond) => self.cond_fail(cond, &mut fails),
            None => {}
        }
        self.op(Op::CmdCall);
        self.new_loop();
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
        for at in fails.into_iter().chain(frame.breaks) {
            self.patch(at, end);
        }
    }

    /// `for (v : list)` as in `lists.gs2bc`: the variable, the list as an
    /// object and a counter 0 stay on the stack; `OP_FOREACH` writes the
    /// next element or jumps out, `OP_INC` steps the counter, and one
    /// `OP_INDEX_DEC` follows the loop. A `continue` goes to the `OP_INC`.
    fn foreach_stmt(&mut self, var: &str, list: &Expr, body: &Stmt) {
        self.place(var);
        // From here on a bare `x` means the loop's `temp.x` and its slot.
        if let Some(n) = var.strip_prefix("temp.").filter(|n| !n.contains('.')) {
            if let Some(&slot) = self.cache.get(var) {
                self.cache.insert(n.to_string(), slot);
            }
        }
        self.value(list);
        self.op(Op::ConvToObject);
        self.emit_int(0);
        let top = self.hole(Op::ForEach);
        self.op(Op::CmdCall);
        self.new_loop();
        self.stmt(body);
        let frame = self.loops.pop().unwrap();
        let step = self.here();
        for c in frame.continues {
            self.patch(c, step);
        }
        self.op(Op::Inc);
        self.jump(Op::SetIndex, top);
        let end = self.here();
        for at in std::iter::once(top).chain(frame.breaks) {
            self.patch(at, end);
        }
        // The variable, the list and the counter: official bodies drop all
        // three.
        for _ in 0..3 {
            self.op(Op::IndexDec);
        }
    }

    /// `switch` as in `control.gs2bc`: a jump to the tests, the arm bodies
    /// in source order (falling through), then the value, one
    /// `OP_COPY_LAST_OP`, case value, `OP_EQ`, `OP_SET_INDEX_TRUE` per case,
    /// a jump to `default` (or out), and one `OP_INDEX_DEC` that drops the
    /// value. `break` goes to that `OP_INDEX_DEC`.
    ///
    /// Seihai adds one jump out after the last body. Without it a last arm
    /// that does not `break` would run on into the tests again; the
    /// fixture's `default` arm does that.
    fn switch_stmt(&mut self, value: &Expr, arms: &[Arm]) {
        let to_tests = self.hole(Op::SetIndex);
        self.new_loop();
        let mut starts = Vec::new();
        for arm in arms {
            starts.push(self.here());
            for s in &arm.body {
                self.stmt(s);
            }
        }
        // Official GS2 bodies always jump out after the last arm. From GS1
        // Seihai jumps out only when the last arm would run on into the
        // tests.
        let falls_off = arms
            .last()
            .is_some_and(|a| !matches!(a.body.last(), Some(Stmt::Break | Stmt::Return(_))));
        let out = (!self.gs1 || falls_off).then(|| self.hole(Op::SetIndex));
        let frame = self.loops.pop().unwrap();
        self.patch(to_tests, self.here());
        self.value(value);
        // The tests in source order; `default` jumps where it stands.
        for (arm, &start) in arms.iter().zip(&starts) {
            for v in &arm.values {
                self.op(Op::CopyLastOp);
                self.value(v);
                self.op(Op::Eq);
                self.jump(Op::SetIndexTrue, start);
            }
            if arm.default {
                self.jump(Op::SetIndex, start);
            }
        }
        let end = self.here();
        self.op(Op::IndexDec);
        for b in frame.breaks.into_iter().chain(out) {
            self.patch(b, end);
        }
        // A `continue` in a `switch` belongs to the loop around it.
        if !frame.continues.is_empty() {
            match self.loops.last_mut() {
                Some(outer) => outer.continues.extend(frame.continues),
                None => self.err("`continue` outside a loop"),
            }
        }
    }

    /// `with (target) body` in the fixture's encoding (`rest.gs2bc`,
    /// `objects.gs2bc`): the target, `OP_CONV_TO_OBJECT`, `OP_WITH` to just
    /// past the block, the body, `OP_WITHEND`. A `return` or a jump out of
    /// the block is kept: the executor closes the function's open blocks.
    fn with_stmt(&mut self, target: &Expr, body: &Stmt, line: u32) {
        let cache = std::mem::take(&mut self.cache);
        self.with_body(target, body, line);
        self.cache = cache;
    }

    /// `with`, `new … { }`, the opcode built-ins, the inline methods and a
    /// dynamic member name are compiled without the variable cache.
    fn with_body(&mut self, target: &Expr, body: &Stmt, line: u32) {
        self.line = line;
        self.value(target);
        self.op(Op::ConvToObject);
        let hole = self.hole(Op::With);
        self.stmt(body);
        self.op(Op::WithEnd);
        let after = self.here();
        self.patch(hole, after);
    }

    /// `new T(name) { body }` as the Preagonal compiler's baselines encode
    /// it (behaviour only): the name, `OP_INLINE_NEW`, three
    /// `OP_COPY_LAST_OP`, the type, `OP_CONV_TO_STRING`, `OP_NEW_OBJECT`,
    /// `OP_ASSIGN` (the name now holds the object), `OP_CONV_TO_OBJECT`,
    /// `OP_WITH` past the block, the body, `OP_WITHEND`, then
    /// `addcontrol(object)` on the object outside and `OP_INDEX_DEC`.
    fn new_block(&mut self, ty: &str, name: Option<&Expr>, body: &[Stmt]) {
        let cache = std::mem::take(&mut self.cache);
        self.new_block_body(ty, name, body);
        self.cache = cache;
    }

    fn new_block_body(&mut self, ty: &str, name: Option<&Expr>, body: &[Stmt]) {
        // With no name, official bodies push `unknown_object` by name.
        match name {
            Some(name) => {
                self.value(name);
                self.op(Op::InlineNew);
            }
            None => self.emit_var("unknown_object"),
        }
        for _ in 0..3 {
            self.op(Op::CopyLastOp);
        }
        let t = self.intern(ty);
        self.push(Op::TypeString, Operand::Str(t));
        self.op(Op::ConvToString);
        self.op(Op::NewObject);
        self.op(Op::Assign);
        self.op(Op::ConvToObject);
        let hole = self.hole(Op::With);
        for s in body {
            self.stmt(s);
        }
        self.op(Op::WithEnd);
        let after = self.here();
        self.patch(hole, after);
        self.op(Op::TypeArray);
        self.op(Op::SwapLastOps);
        self.emit_var("addcontrol");
        self.op(Op::Call);
        self.op(Op::IndexDec);
    }

    fn assign(&mut self, target: &Var, op: AssignOp, value: &Expr) {
        if !target.index.is_empty() {
            self.element_assign(target, op, value);
            return;
        }
        self.place(&target.name);
        if op == AssignOp::Set {
            self.value(value);
        } else if op == AssignOp::Cat {
            // `a @= b`: a, `OP_COPY_LAST_OP`, as text, b as text, join.
            self.op(Op::CopyLastOp);
            self.op(Op::ConvToString);
            self.stringish(value);
            self.op(Op::Join);
        } else {
            self.op(Op::CopyLastOp);
            self.op(Op::ConvToFloat);
            self.numeric(value);
            self.op(arith(op));
        }
        self.op(Op::Assign);
    }

    /// `a[i] = v` and `a[i, j] = v`: object, indexes, value, assign op.
    ///
    /// `a[i] op= v` is `a[i] = a[i] op v` (Seihai's form: the gs2-parser
    /// output copies the index, not the element, and so stores `i op v`).
    /// That pushes the index twice, so when the index calls something the
    /// element is read once as a place (`OP_ARRAY`, as `a[i]++` does) and
    /// stored through instead.
    fn element_assign(&mut self, target: &Var, op: AssignOp, value: &Expr) {
        if op != AssignOp::Set {
            if target.index.iter().any(expr_calls) {
                let Some(dims) = self.element(target) else {
                    return;
                };
                self.op(element_op(dims));
                self.op(Op::CopyLastOp);
                if op == AssignOp::Cat {
                    self.op(Op::ConvToString);
                    self.stringish(value);
                } else {
                    self.op(Op::ConvToFloat);
                    self.numeric(value);
                }
                self.op(arith(op));
                self.op(Op::Assign);
                return;
            }
            let sum = op.expand(Expr::Var(target.clone()), value.clone());
            self.element_assign(target, AssignOp::Set, &sum);
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

    /// `a = b = c = v` as in the Preagonal baselines: `a`, then each later
    /// target and `OP_COPY_LAST_OP`, the value, and one `OP_ASSIGN` per
    /// target (innermost first). With an element among the targets, the
    /// later stores are values and the first a plain store (Seihai's form;
    /// no probe).
    fn chain(&mut self, targets: &[Var], value: &Expr) {
        if targets.iter().any(|t| !t.index.is_empty()) {
            let mut v = value.clone();
            for t in targets[1..].iter().rev() {
                v = Expr::AssignValue {
                    target: Box::new(Expr::Var(t.clone())),
                    op: AssignOp::Set,
                    value: Box::new(v),
                };
            }
            self.assign(&targets[0], AssignOp::Set, &v);
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

    /// A store into a member or element of a value: the place, the value,
    /// `OP_ASSIGN` / `OP_ARRAY_ASSIGN`, as for a dotted name or `a[i]`
    /// (Seihai's encoding, no fixture). `makevar("a.b") = v` (Preagonal
    /// probe) is the name, `OP_MAKEVAR`, the value, `OP_ASSIGN`.
    fn set_part(&mut self, target: &Expr, value: &Expr) {
        match target {
            Expr::Member { .. } | Expr::DynMember { .. } | Expr::Call { .. } => {
                self.value(target);
                self.value(value);
                self.op(Op::Assign);
            }
            Expr::Index { obj, index } if index.len() <= 2 => {
                self.value(obj);
                self.op(Op::ConvToObject);
                for i in index {
                    self.numeric(i);
                }
                self.value(value);
                self.op(if index.len() == 1 {
                    Op::ArrayAssign
                } else {
                    Op::ArrayMultidimAssign
                });
            }
            _ => self.err("that assignment target is not lowered"),
        }
    }

    // ---- places ----

    /// A dotted name: its first part, then each member with
    /// `OP_MEMBER_ACCESS`. As in gs2-parser's output, the value is
    /// converted to an object before each member, except straight after
    /// `this`/`temp`/`player`/… (`this.a.b` is THIS a MA CTO b MA;
    /// `client.a` is VAR client CTO a MA).
    fn place(&mut self, name: &str) {
        if let Some(slot) = cache::key_of(name).and_then(|k| self.cache.get(&k).copied()) {
            let skip = if matches!(name.split('.').next(), Some("this" | "thiso" | "temp")) {
                2
            } else {
                1
            };
            self.push(Op::Unknown(cache::LOAD), Operand::Int(slot));
            for member in name.split('.').skip(skip) {
                self.op(Op::ConvToObject);
                self.emit_var(member);
                self.op(Op::MemberAccess);
            }
            return;
        }
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
            "null" | "NULL" => self.op(Op::TypeNull),
            "pi" => self.op(Op::Pi),
            // Official bodies push the argument list with `OP_PARAMS`.
            "params" => self.op(Op::Params),
            _ => self.emit_var(name),
        }
    }

    /// Pushes an indexed variable as an object followed by its indexes and
    /// returns how many indexes there are, or `None` after an error.
    fn element(&mut self, v: &Var) -> Option<usize> {
        if v.index.len() > 2 {
            self.err(format!(
                "`{}` has {} indexes; GS2 array opcodes take one or two",
                v.name,
                v.index.len()
            ));
            return None;
        }
        self.place(&v.name);
        self.op(Op::ConvToObject);
        for i in &v.index {
            self.numeric(i);
        }
        Some(v.index.len())
    }

    /// The place an expression names, for `x++` or `(x = v)` used as a
    /// value: a plain or dotted name, an element, `makevar(…)`, or a member.
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
                self.op(element_op(dims));
                true
            }
            Expr::Call { name, .. } if name == "makevar" => {
                self.value(target);
                true
            }
            Expr::Member { .. } | Expr::DynMember { .. } | Expr::Index { .. } => {
                self.value(target);
                true
            }
            _ => {
                self.err("that is not something `++`, `--` or `=` can change");
                false
            }
        }
    }

    /// A value used as the object of a member or method: converted with
    /// `OP_CONV_TO_OBJECT` unless it is a root (`this`, `player`, …), as
    /// the Preagonal baselines do for dotted names, calls and members. An
    /// array element (`players[i].x`) is used as `OP_ARRAY` left it, with
    /// no conversion (`members.gs2bc`).
    fn object(&mut self, obj: &Expr) {
        match obj {
            // An owner root keeps its opcode (or its cache slot).
            Expr::Var(v) if v.index.is_empty() && root_op(&v.name).is_some() => self.place(&v.name),
            _ => {
                self.value(obj);
            }
        }
        // Official bodies convert every owner except a root object pushed
        // by its own opcode, including an element read (`a[i].x`).
        let root = matches!(obj, Expr::Var(v) if v.index.is_empty()
            && root_op(&v.name).is_some()
            && !(v.name != "this" && self.cache.contains_key(&v.name)));
        if !root {
            self.op(Op::ConvToObject);
        }
    }

    // ---- calls ----

    /// A call by name. Returns whether it pushed a value a statement
    /// should drop.
    fn call(&mut self, name: &str, args: &[Expr]) -> bool {
        let cache = if cache::inline_call(name) {
            std::mem::take(&mut self.cache)
        } else {
            HashMap::new()
        };
        let done = self.builtin(name, args);
        if !cache.is_empty() {
            self.cache = cache;
        }
        if done {
            return call_pushes(name);
        }
        self.call_by_name(name, args);
        true
    }

    /// The arguments in reverse, the name, `OP_CALL`.
    fn call_by_name(&mut self, name: &str, args: &[Expr]) {
        self.op(Op::TypeArray);
        for a in args.iter().rev() {
            self.value(a);
        }
        self.load_or_var(name);
        self.op(Op::Call);
    }

    /// The owner of `obj.(name)`: as any member owner, and `this` is
    /// converted too.
    fn dyn_owner(&mut self, obj: &Expr) {
        // A `this` owner is pushed by name and converted.
        let this = matches!(obj, Expr::Var(v) if v.index.is_empty() && v.name == "this");
        if this && !self.gs1 {
            self.value(obj);
            self.op(Op::ConvToObject);
            return;
        }
        self.object(obj);
        if this {
            self.op(Op::ConvToObject);
        }
    }

    /// The name of `obj.(name)`, compiled without the variable cache. Text
    /// is used as it is; anything else is converted to text.
    fn dyn_name(&mut self, name: &Expr) {
        let cache = std::mem::take(&mut self.cache);
        let inner = match name {
            Expr::Text(e) => &**e,
            e => e,
        };
        let kind = self.value(inner);
        self.cache = cache;
        // GS2: a plain expression is used as it is; `@x` is made text.
        if !self.gs1 && !matches!(name, Expr::Text(_)) {
            return;
        }
        if kind != Kind::String
            && !matches!(
                inner,
                Expr::Binary(
                    BinOp::Cat | BinOp::CatSpc | BinOp::CatNl | BinOp::CatTab,
                    ..
                )
            )
        {
            self.op(Op::ConvToString);
        }
    }

    /// A name: its cache slot when it has one, else `OP_TYPE_VAR`.
    fn load_or_var(&mut self, key: &str) {
        match self.cache.get(key).copied() {
            Some(slot) => self.push(Op::Unknown(cache::LOAD), Operand::Int(slot)),
            None => self.emit_var(key),
        }
    }

    /// `this.name` / `thiso.name` / `temp.name` as one cached slot, when
    /// `obj` is that root and the member has a slot.
    fn cached_member(&mut self, obj: &Expr, name: &str) -> bool {
        let Expr::Var(v) = obj else { return false };
        if !v.index.is_empty() || !matches!(v.name.as_str(), "this" | "thiso" | "temp") {
            return false;
        }
        match self.cache.get(&format!("{}.{name}", v.name)).copied() {
            Some(slot) => {
                self.push(Op::Unknown(cache::LOAD), Operand::Int(slot));
                true
            }
            None => false,
        }
    }

    /// An opcode built-in. Returns `false` when `name` is not one.
    fn builtin(&mut self, name: &str, args: &[Expr]) -> bool {
        // Official bodies compile `pow(a, b)` as a script call; `OP_POW`
        // is only for `^`.
        if name == "pow" && !self.gs1 {
            return false;
        }
        if name == "format" && args.is_empty() {
            self.err("`format` expects at least 1 argument, got 0");
            return true;
        }
        if let Some(n) = builtin_arity(name) {
            if args.len() != n {
                self.err(format!(
                    "`{name}` expects {n} argument(s), got {}",
                    args.len()
                ));
                return true;
            }
        }
        match name {
            "sleep" => {
                self.numeric(&args[0]);
                self.op(Op::Sleep);
            }
            "setarray" => {
                self.value(&args[0]);
                self.op(Op::ConvToObject);
                self.numeric(&args[1]);
                self.op(Op::SetArray);
            }
            "format" => {
                self.op(Op::TypeArray);
                for a in args.iter().rev() {
                    self.value(a);
                }
                self.op(Op::Format);
            }
            // GS2 `sarraylen(a)`: the array named by the text.
            "sarraylen" if !self.gs1 => {
                self.value(&args[0]);
                self.op(Op::ObjFromStr);
                self.op(Op::ConvToObject);
                self.op(Op::ObjSize);
            }
            // The reference compiler hands `OP_OBJ_SIZE` an object.
            "arraylen" | "sarraylen" => {
                self.value(&args[0]);
                self.op(Op::ConvToObject);
                self.op(Op::ObjSize);
            }
            "int" => {
                self.numeric(&args[0]);
                self.op(Op::Int);
            }
            // `rest.gs2bc`: `getangle(1, -1)` and `getdir(-3, 1)` push their
            // operands in source order, as does `pow`; the others are
            // reversed.
            "getangle" | "getdir" | "pow" => {
                for a in args {
                    self.numeric(a);
                }
                self.op(opcode_builtin(name).unwrap());
            }
            // `strings.gs2bc`: `_("text")` is the text, then `OP_TRANSLATE`.
            // GS2 converts anything but text first.
            "_" if args.len() == 1 => {
                if self.gs1 {
                    self.value(&args[0]);
                } else {
                    self.stringish(&args[0]);
                }
                self.op(Op::Translate);
            }
            // GS2 `float(x)`: `x` as a number, with no opcode of its own.
            "float" if args.len() == 1 && !self.gs1 => {
                self.numeric(&args[0]);
            }
            // `makevar` takes its name as a string, converted even when it
            // already is one, as the reference compiler does.
            "makevar" => {
                self.stringish(&args[0]);
                self.op(Op::MakeVar);
            }
            // `waitfor(object, event, timeout)` pushes in source order: the
            // object and event as strings, the timeout as a number.
            "waitfor" => {
                self.stringish(&args[0]);
                self.stringish(&args[1]);
                self.numeric(&args[2]);
                self.op(Op::WaitFor);
            }
            // Official bodies push these in source order from GS2 and
            // reversed from GS1.
            "sin" | "cos" | "arctan" | "abs" | "random" | "exp" | "log" | "min" | "max"
            | "char" | "vecx" | "vecy" => {
                if self.gs1 {
                    for a in args.iter().rev() {
                        self.numeric(a);
                    }
                } else {
                    for a in args {
                        self.numeric(a);
                    }
                }
                self.op(opcode_builtin(name).unwrap());
            }
            _ => return false,
        }
        true
    }

    /// A method call `obj.name(args)`. The built-in methods use the opcodes
    /// and operand orders in `strings.gs2bc`, `lists.gs2bc` and
    /// `rest.gs2bc`; any other name is a call on the object
    /// (`objects.gs2bc`: the arguments reversed, the object, the name,
    /// `OP_MEMBER_ACCESS`, `OP_CALL`). Returns whether a value was pushed.
    fn method(&mut self, obj: &Expr, name: &str, args: &[Expr]) -> bool {
        if cache::inline_method(name) && !self.cache.is_empty() {
            let cache = std::mem::take(&mut self.cache);
            let pushed = self.method(obj, name, args);
            self.cache = cache;
            return pushed;
        }
        let lower = name.to_ascii_lowercase();
        let n = args.len();
        // GS2: `s.lower()` and `s.upper()` are calls of `lowercase` and
        // `uppercase` on the text.
        if !self.gs1 && n == 0 && matches!(name, "lower" | "upper") {
            self.op(Op::TypeArray);
            self.stringish(obj);
            self.emit_var(if name == "lower" {
                "lowercase"
            } else {
                "uppercase"
            });
            self.op(Op::Call);
            return true;
        }
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
            // `"text".length()` is the byte count itself.
            if let (Op::ObjLength, Expr::Str(text), false) = (op, obj, self.gs1) {
                self.emit_int(text.len() as i32);
                return true;
            }
            // Official bodies convert the receiver unless it is text.
            if self.gs1 {
                self.value(obj);
                self.op(Op::ConvToString);
            } else {
                self.stringish(obj);
            }
            // Official bodies make positions numbers and searched-for
            // text text (GS2).
            for a in args {
                match op {
                    _ if self.gs1 => {
                        self.value(a);
                    }
                    Op::ObjSubstr | Op::ObjCharAt => self.numeric(a),
                    Op::ObjPos | Op::ObjStarts | Op::ObjEnds | Op::ObjPositions => {
                        self.stringish(a)
                    }
                    _ => {
                        self.value(a);
                    }
                }
            }
            if op == Op::ObjSubstr && n == 1 {
                self.emit_int(-1);
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
            ("link", 0) if !self.gs1 => Some((Op::ObjLink, true)),
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
            // GS2: indexes and counts are made numbers, and a missing
            // `subarray` count is -1.
            let index = |b: &mut Self, a: &Expr| {
                if b.gs1 {
                    b.value(a);
                } else {
                    b.numeric(a);
                }
            };
            match op {
                Op::ObjInsertString | Op::ObjReplaceString => {
                    self.value(&args[1]);
                    index(self, &args[0]);
                }
                Op::ObjDeleteString | Op::ObjSubarray => {
                    for a in args {
                        index(self, a);
                    }
                    if op == Op::ObjSubarray && n == 1 && !self.gs1 {
                        self.emit_int(-1);
                    }
                }
                _ => {
                    for a in args {
                        self.value(a);
                    }
                }
            }
            self.op(op);
            return pushes;
        }
        if lower == "indices" && n == 1 {
            // GS2 pushes the list as an object, then the value.
            if self.gs1 {
                self.value(&args[0]);
                self.value(obj);
            } else {
                self.value(obj);
                self.op(Op::ConvToObject);
                self.value(&args[0]);
            }
            self.op(Op::ObjIndices);
            return true;
        }
        self.op(Op::TypeArray);
        for a in args.iter().rev() {
            self.value(a);
        }
        if !matches!(obj, Expr::Var(v) if v.name != "temp" && self.cached_member(obj, name)) {
            self.object(obj);
            self.emit_var(name);
            self.op(Op::MemberAccess);
        }
        self.op(Op::Call);
        true
    }

    /// A GS1 built-in with no GS2 name.
    fn gs1_fn(&mut self, f: Gs1Fn, args: &[Expr]) -> Kind {
        match f {
            Gs1Fn::StrLen | Gs1Fn::StartsWith | Gs1Fn::IndexOf => {
                self.stringish(&args[0]);
                for a in &args[1..] {
                    self.value(a);
                }
                self.op(match f {
                    Gs1Fn::StrLen => Op::ObjLength,
                    Gs1Fn::StartsWith => Op::ObjStarts,
                    _ => Op::ObjPos,
                });
                if f == Gs1Fn::StrLen {
                    Kind::Number
                } else {
                    Kind::Other
                }
            }
            Gs1Fn::StrToFloat => {
                self.numeric(&args[0]);
                Kind::Other
            }
            Gs1Fn::StrContains => {
                self.stringish(&args[0]);
                self.value(&args[1]);
                self.op(Op::ObjPos);
                self.op(Op::ConvToFloat);
                self.emit_int(0);
                self.op(Op::Gte);
                Kind::Other
            }
        }
    }

    /// GS1 text with message codes, as GS2 joins (`@`).
    fn message(&mut self, parts: &[MsgPart]) -> Kind {
        if let [MsgPart::Lit(t)] = parts {
            self.emit_str(t);
            return Kind::String;
        }
        for (i, part) in parts.iter().enumerate() {
            match part {
                MsgPart::Lit(t) => self.emit_str(t),
                MsgPart::Value(e) => self.stringish(e),
                MsgPart::Trim(t) => {
                    self.message(t);
                    self.op(Op::ConvToString);
                    self.op(Op::ObjTrim);
                }
                MsgPart::Sub(start, len, t) => {
                    self.message(t);
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

    // ---- values ----

    fn numeric(&mut self, e: &Expr) {
        if self.value(e) != Kind::Number {
            self.op(Op::ConvToFloat);
        }
    }

    fn stringish(&mut self, e: &Expr) {
        if self.value(e) != Kind::String {
            self.op(Op::ConvToString);
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
            // GS2: `true` and `false` are numbers; `!true`, `false && x`
            // and `true || x` fold to a constant.
            e if !self.gs1 && const_bool(e).is_some() && !matches!(e, Expr::Var(_)) => {
                self.op(if const_bool(e).unwrap() {
                    Op::TypeTrue
                } else {
                    Op::TypeFalse
                });
                Kind::Number
            }
            Expr::Var(v)
                if !self.gs1
                    && v.index.is_empty()
                    && matches!(v.name.as_str(), "true" | "false") =>
            {
                self.place(&v.name);
                Kind::Number
            }
            // `pi` is a number; it folds like a literal (`pi * 1.5`).
            Expr::Var(v) if v.index.is_empty() && v.name == "pi" => {
                self.place(&v.name);
                Kind::Number
            }
            // As a value (not an owner), official bodies push these roots
            // by name; `player` and `level` keep their opcodes.
            Expr::Var(v)
                if v.index.is_empty()
                    && !self.gs1
                    && matches!(v.name.as_str(), "this" | "thiso" | "playero" | "temp") =>
            {
                self.load_or_var(&v.name);
                Kind::Other
            }
            Expr::Var(v) if v.index.is_empty() => {
                self.place(&v.name);
                Kind::Other
            }
            Expr::Var(v) => {
                let Some(dims) = self.element(v) else {
                    self.emit_int(0);
                    return Kind::Number;
                };
                self.op(element_op(dims));
                Kind::Other
            }
            Expr::Binary(..) | Expr::Bits(..) | Expr::Neg(_) if fold(e).is_some() => {
                self.emit_number(fold(e).unwrap());
                Kind::Number
            }
            Expr::Neg(a) => {
                if let Expr::Num(n) = a.as_ref() {
                    self.emit_number(-n);
                } else {
                    self.numeric(a);
                    self.op(Op::UnarySub);
                }
                Kind::Number
            }
            Expr::Invert(a) => {
                self.numeric(a);
                self.op(Op::BitInvert);
                Kind::Number
            }
            // Official bodies use `!`, `&&`/`||` and comparisons as numbers.
            Expr::Not(a) => {
                self.not_operand(a);
                self.op(Op::Not);
                Kind::Number
            }
            Expr::Binary(BinOp::And | BinOp::Or, _, _) => {
                self.logic(e);
                Kind::Number
            }
            Expr::Binary(op, a, b) => self.binary(*op, a, b),
            // `5 & 3` is the two numbers and `OP_BWA` (Preagonal probe).
            Expr::Bits(op, a, b) => {
                self.numeric(a);
                self.numeric(b);
                self.op(match op {
                    BitOp::Shl => Op::ShiftLeft,
                    BitOp::Shr => Op::ShiftRight,
                    BitOp::And => Op::BitAnd,
                    BitOp::Or => Op::BitOr,
                    BitOp::Xor => Op::BitXor,
                });
                Kind::Number
            }
            Expr::Cond { cond, then, els } => {
                // A `&&`/`||` test is the value chain and one `OP_IF`.
                let mut fails = Vec::new();
                if is_and(cond) || is_or(cond) {
                    self.logic_operand(cond);
                    fails.push(self.hole(Op::If));
                } else {
                    self.cond_fail(cond, &mut fails);
                }
                let kt = self.value(then);
                let jump = self.hole(Op::SetIndex);
                let else_at = self.here();
                for f in fails {
                    self.patch(f, else_at);
                }
                let ke = self.value(els);
                self.patch(jump, self.here());
                // GS2 converts a choice of two texts again.
                if self.gs1 && kt == Kind::String && ke == Kind::String {
                    Kind::String
                } else {
                    Kind::Other
                }
            }
            // The reference compiler uses the result of `OP_IN_RANGE` and
            // `OP_IN_OBJ` as a number with no `OP_CONV_TO_FLOAT`.
            Expr::InRange { e, lo, hi } => {
                // GS2 makes the bounds numbers; the value stays as it is.
                self.value(e);
                if self.gs1 {
                    self.value(lo);
                    self.value(hi);
                } else {
                    self.numeric(lo);
                    self.numeric(hi);
                }
                // The operand: 2 for an open lower bound, 1 for an open
                // upper one; written ranges are closed.
                self.push(Op::InRange, Operand::Int(0));
                Kind::Number
            }
            // `x in {a, b}`: like `x in arr` in `lists.gs2bc`, with the
            // literal as the array (Seihai's encoding, no fixture).
            Expr::InList { e, list } => {
                self.value(e);
                self.op(Op::TypeArray);
                for v in list.iter().rev() {
                    self.value(v);
                }
                self.op(Op::ArrayEnd);
                self.op(Op::ConvToObject);
                self.op(Op::InObj);
                Kind::Number
            }
            Expr::AnyOf { e, list } => {
                let chain = list
                    .iter()
                    .map(|item| Expr::bin(BinOp::Eq, (**e).clone(), item.clone()))
                    .reduce(|acc, eq| Expr::bin(BinOp::Or, acc, eq))
                    .unwrap_or(Expr::Num(0.0));
                self.value(&chain)
            }
            Expr::InArray { e, name } => {
                self.value(e);
                self.place(name);
                self.op(Op::ConvToObject);
                self.op(Op::InObj);
                Kind::Number
            }
            Expr::InValue { e, list } => {
                self.value(e);
                self.value(list);
                self.op(Op::ConvToObject);
                self.op(Op::InObj);
                Kind::Number
            }
            Expr::List(values) => {
                self.op(Op::TypeArray);
                for value in values.iter().rev() {
                    self.value(value);
                }
                self.op(Op::ArrayEnd);
                Kind::Other
            }
            Expr::Call { name, args } => {
                // Official bodies convert every call's result, `int(…)`
                // included.
                self.call(name, args);
                Kind::Other
            }
            Expr::HostCall { name, args } => {
                self.call_by_name(name, args);
                Kind::Other
            }
            Expr::Method { obj, name, args } => {
                if !self.method(obj, name, args) {
                    // A list method that leaves nothing, used as a value.
                    self.op(Op::TypeNull);
                }
                Kind::Other
            }
            Expr::Member { obj, name } => {
                if !self.cached_member(obj, name) {
                    self.object(obj);
                    self.emit_var(name);
                    self.op(Op::MemberAccess);
                }
                Kind::Other
            }
            // `a[i][j]`: no conversion between the two reads in the
            // Preagonal baselines.
            Expr::Index { obj, index } => {
                // `a[i][j]`: official bodies convert the first read too.
                self.value(obj);
                self.op(Op::ConvToObject);
                for i in index {
                    self.numeric(i);
                }
                self.op(element_op(index.len()));
                Kind::Other
            }
            // `obj.(n)`: the object, the name as a string, member access.
            // Seihai's form: the Preagonal output for `obj.(@n)` adds the
            // name to the object as numbers.
            Expr::DynMember { obj, name } => {
                self.dyn_owner(obj);
                self.dyn_name(name);
                self.op(Op::MemberAccess);
                Kind::Other
            }
            // `obj.(n)(args)`: a method named by a value (Seihai's form, as
            // a method call with the name pushed as a string).
            Expr::DynMethod { obj, name, args } => {
                self.op(Op::TypeArray);
                for a in args.iter().rev() {
                    self.value(a);
                }
                self.dyn_owner(obj);
                self.dyn_name(name);
                self.op(Op::MemberAccess);
                self.op(Op::Call);
                Kind::Other
            }
            Expr::DynCall { name, args } => {
                self.op(Op::TypeArray);
                for a in args.iter().rev() {
                    self.value(a);
                }
                self.value(name);
                self.op(Op::ConvToString);
                self.op(Op::Call);
                Kind::Other
            }
            // `new T()` pushes the name "unknown_object", the type, then
            // `OP_NEW_OBJECT`, with the strings stored in that order
            // (gs2-parser stores the type first). `new T(name)` pushes the
            // name and `OP_INLINE_NEW` in its place. Official bodies push
            // the default name for two or more arguments and drop them.
            Expr::New { ty, args } => {
                match args.as_slice() {
                    [a] => {
                        self.value(a);
                        self.op(Op::InlineNew);
                    }
                    _ => self.emit_var("unknown_object"),
                }
                self.emit_str(ty);
                self.op(Op::NewObject);
                Kind::Other
            }
            Expr::NewArray(dims) => {
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
            // `(@x).y` pushes x and `OP_CONV_TO_STRING` (Preagonal probe).
            // GS2: `@x` is x as text (no conversion when it already is).
            Expr::Text(e) if !self.gs1 => {
                self.stringish(e);
                Kind::String
            }
            Expr::Text(e) => {
                self.value(e);
                self.op(Op::ConvToString);
                Kind::Other
            }
            // `function (…) { … }` as a value (Preagonal probe): the
            // function emitted where it stands and jumped over, then
            // `this.<name>` as an object.
            Expr::Lambda { name, .. } => {
                let Some(f) = self.lambdas.remove(&format!("public.{name}")) else {
                    self.err("a `function () { … }` value is used twice");
                    return Kind::Other;
                };
                let loops = std::mem::take(&mut self.loops);
                self.function(&f, true);
                self.loops = loops;
                if let Some(skip) = self.skips.pop() {
                    let here = self.here();
                    self.patch(skip, here);
                }
                self.op(Op::This);
                self.emit_var(name);
                self.op(Op::MemberAccess);
                self.op(Op::ConvToObject);
                Kind::Other
            }
            // `y = x++` (Preagonal probe): x, `OP_COPY_LAST_OP`,
            // `OP_CONV_TO_FLOAT`, `OP_SWAP_LAST_OPS`, `OP_INC`,
            // `OP_INDEX_DEC`, leaving the old number.
            Expr::PostStep { target, delta } => {
                if !self.step_place(target) {
                    return Kind::Other;
                }
                self.op(Op::CopyLastOp);
                self.op(Op::ConvToFloat);
                self.op(Op::SwapLastOps);
                self.op(if *delta < 0.0 { Op::Dec } else { Op::Inc });
                self.op(Op::IndexDec);
                // GS2 converts the old value again where a number is needed.
                if self.gs1 {
                    Kind::Number
                } else {
                    Kind::Other
                }
            }
            // `++x` used as a value: the place and `OP_INC`, which leaves
            // the changed variable (Seihai's form, no probe).
            Expr::PreStep { target, delta } => {
                if !self.step_place(target) {
                    return Kind::Other;
                }
                self.op(if *delta < 0.0 { Op::Dec } else { Op::Inc });
                Kind::Number
            }
            // `y = (x = 2)` (Preagonal probe): x, `OP_COPY_LAST_OP`, the
            // value, `OP_ASSIGN`, leaving x. For `+=` and the others Seihai
            // copies the place once more, so x is still there after the
            // store (the Preagonal output does not, and leaves nothing).
            // GS2 `x = a[i] += v`: the element store, then the element read
            // again as the value.
            Expr::AssignValue { target, op, value }
                if !self.gs1
                    && *op != AssignOp::Set
                    && matches!(&**target, Expr::Var(v) if !v.index.is_empty()) =>
            {
                let Expr::Var(v) = &**target else {
                    unreachable!()
                };
                self.element_assign(v, *op, value);
                self.value(target);
                Kind::Other
            }
            // `a == ((x && b) = 2)` in a GS2 test: a value that is no place
            // is stored to as it is (official captures).
            Expr::AssignValue { target, op, value }
                if *op == AssignOp::Set && !self.gs1 && !is_place(target) =>
            {
                self.value(target);
                self.op(Op::CopyLastOp);
                self.value(value);
                self.op(Op::Assign);
                Kind::Other
            }
            Expr::AssignValue { target, op, value } => {
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
                    self.op(arith(*op));
                }
                self.op(Op::Assign);
                Kind::Other
            }
            Expr::Message(parts) => self.message(parts),
            Expr::Gs1Fn(f, args) => self.gs1_fn(*f, args),
        }
    }

    fn binary(&mut self, op: BinOp, a: &Expr, b: &Expr) -> Kind {
        let arith = match op {
            BinOp::Add => Some(Op::Add),
            BinOp::Sub => Some(Op::Sub),
            BinOp::Mul => Some(Op::Mul),
            BinOp::Div => Some(Op::Div),
            BinOp::Mod => Some(Op::Mod),
            BinOp::Pow => Some(Op::Pow),
            _ => None,
        };
        if let Some(code) = arith {
            self.numeric(a);
            self.numeric(b);
            self.op(code);
            return Kind::Number;
        }
        let compare = match op {
            BinOp::Lt => Some(Op::Lt),
            BinOp::Gt => Some(Op::Gt),
            BinOp::Le => Some(Op::Lte),
            BinOp::Ge => Some(Op::Gte),
            _ => None,
        };
        if let Some(code) = compare {
            self.numeric(a);
            self.numeric(b);
            self.op(code);
            return Kind::Number;
        }
        match op {
            BinOp::Eq | BinOp::Ne => {
                self.value(a);
                self.value(b);
                self.op(if op == BinOp::Eq { Op::Eq } else { Op::Neq });
                Kind::Number
            }
            BinOp::Cat | BinOp::CatSpc | BinOp::CatNl | BinOp::CatTab => {
                // Official bodies join `@` between text literals at compile
                // time, and the left side of `SPC`/`NL`/`TAB` with its
                // separator, but never the right side of those.
                if let Some(text) =
                    fold_text(&Expr::Binary(op, Box::new(a.clone()), Box::new(b.clone())))
                {
                    self.emit_str(&text);
                    return Kind::String;
                }
                let sep = match op {
                    BinOp::CatSpc => Some(" "),
                    BinOp::CatNl => Some("\n"),
                    BinOp::CatTab => Some("\t"),
                    _ => None,
                };
                match (sep, fold_text(a)) {
                    (Some(sep), Some(left)) => self.emit_str(&format!("{left}{sep}")),
                    _ => {
                        self.stringish(a);
                        if let Some(sep) = sep {
                            self.emit_str(sep);
                            self.op(Op::Join);
                        }
                    }
                }
                self.stringish(b);
                self.op(Op::Join);
                Kind::String
            }
            _ => unreachable!("handled above or by `logic`"),
        }
    }

    // ---- conditions ----

    fn bool_leaf(&mut self, e: &Expr) {
        if is_comparison(e) || matches!(e, Expr::InRange { .. } | Expr::Not(_)) {
            self.value(e);
        } else {
            self.numeric(e);
        }
    }

    /// `&&` and `||` as a value: short-circuit jumps, then
    /// `OP_INLINE_CONDITIONAL`.
    fn logic(&mut self, e: &Expr) {
        self.logic_operand(e);
        self.op(Op::InlineConditional);
    }

    /// The operand of `!`: a `&&`/`||` chain negates the chain itself, with
    /// no `OP_INLINE_CONDITIONAL`; a comparison or membership test as it
    /// is; anything else gets `OP_CONV_TO_FLOAT`, even a number.
    fn not_operand(&mut self, x: &Expr) {
        if is_logic(x) {
            self.logic_operand(x);
        } else if is_comparison(x)
            || matches!(
                x,
                Expr::InRange { .. }
                    | Expr::InList { .. }
                    | Expr::InArray { .. }
                    | Expr::InValue { .. }
                    | Expr::Not(_)
            )
        {
            self.value(x);
        } else {
            self.value(x);
            self.op(Op::ConvToFloat);
        }
    }

    /// A `&&`/`||` chain as a value. Each run of the same operator jumps
    /// to the end of that run: `a && b || c` is `a`, `OP_AND` to the
    /// `OP_OR`, `b`, `OP_OR` to the end, `c`.
    fn logic_operand(&mut self, e: &Expr) {
        let op = match e {
            Expr::Binary(op @ (BinOp::And | BinOp::Or), _, _) => *op,
            other => return self.bool_leaf(other),
        };
        let mut parts = Vec::new();
        flatten(e, op, &mut parts);
        let jump = if op == BinOp::And { Op::And } else { Op::Or };
        let mut holes = Vec::new();
        for (i, p) in parts.iter().enumerate() {
            self.logic_operand(p);
            if i + 1 < parts.len() {
                holes.push(self.hole(jump));
            }
        }
        let end = self.here();
        for h in holes {
            self.patch(h, end);
        }
    }

    /// A condition that jumps to the holes in `fails` when false. A
    /// top-level `&&` is one `OP_IF` per part. An `||` is its parts but the
    /// last as values, each followed by `OP_OR` to the last part's final
    /// `OP_IF` (which then sees the true value), and the last part as a
    /// condition. `!(…)` of a chain is the chain, `OP_NOT` and `OP_IF`.
    fn cond_fail(&mut self, e: &Expr, fails: &mut Vec<usize>) {
        match e {
            Expr::Binary(BinOp::And, a, b) => {
                self.cond_fail(a, fails);
                self.cond_fail(b, fails);
            }
            Expr::Binary(BinOp::Or, _, _) => {
                let mut parts = Vec::new();
                flatten(e, BinOp::Or, &mut parts);
                let (last, rest) = parts.split_last().unwrap();
                let mut trues = Vec::new();
                for p in rest {
                    self.logic_operand(p);
                    trues.push(self.hole(Op::Or));
                }
                self.cond_fail(last, fails);
                let test = self.here() - 1;
                for t in trues {
                    self.patch(t, test);
                }
            }
            Expr::Not(x) => {
                self.not_operand(x);
                self.op(Op::Not);
                fails.push(self.hole(Op::If));
            }
            other => {
                self.bool_leaf(other);
                fails.push(self.hole(Op::If));
            }
        }
    }
}

/// The operands of a run of one logical operator, left to right.
fn flatten<'e>(e: &'e Expr, op: BinOp, out: &mut Vec<&'e Expr>) {
    match e {
        Expr::Binary(o, a, b) if *o == op => {
            flatten(a, op, out);
            flatten(b, op, out);
        }
        other => out.push(other),
    }
}

/// The value of an arithmetic or bit expression over number literals, as
/// official bodies fold it. Only literals that are not negative fold, so
/// `2 + -1` stays as written; `-(2 + 3)` folds to -5.
fn fold(e: &Expr) -> Option<f64> {
    fn operand(e: &Expr) -> Option<f64> {
        match e {
            Expr::Num(n) if *n >= 0.0 => Some(*n),
            Expr::Var(v) if v.index.is_empty() && v.name == "pi" => Some(std::f64::consts::PI),
            Expr::Binary(..) | Expr::Bits(..) => fold(e),
            _ => None,
        }
    }
    let v = match e {
        Expr::Binary(op, a, b) => {
            let (a, b) = (operand(a)?, operand(b)?);
            match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a * b,
                BinOp::Div if b != 0.0 => a / b,
                BinOp::Mod if b != 0.0 => a % b,
                BinOp::Pow => a.powf(b),
                _ => return None,
            }
        }
        Expr::Bits(op, a, b) => {
            let (a, b) = (operand(a)? as i64 as i32, operand(b)? as i64 as i32);
            (match op {
                BitOp::Shl => a.wrapping_shl(b as u32),
                BitOp::Shr => a.wrapping_shr(b as u32),
                BitOp::And => a & b,
                BitOp::Or => a | b,
                BitOp::Xor => a ^ b,
            }) as f64
        }
        Expr::Neg(x) if matches!(**x, Expr::Binary(..) | Expr::Bits(..)) => -fold(x)?,
        _ => return None,
    };
    v.is_finite().then_some(v)
}

/// Text that `@` between text literals folds to.
fn fold_text(e: &Expr) -> Option<String> {
    match e {
        Expr::Str(s) => Some(s.clone()),
        Expr::Binary(BinOp::Cat, a, b) => Some(fold_text(a)? + &fold_text(b)?),
        _ => None,
    }
}

/// The engine's script events, by bit in the segment 1 mask (the client's
/// `script_event` list; Graal_analysis
/// `client-internals/gs2-vm-from-engines.md`).
const SCRIPT_EVENTS: [&str; 27] = [
    "playerenters",
    "playertouchsme",
    "playertouchsother",
    "playerchats",
    "playerhurt",
    "playerdies",
    "playerlaysitem",
    "playerendsreading",
    "compusdied",
    "emoticon",
    "mousedown",
    "mouseup",
    "mousewheel",
    "exploded",
    "wasshot",
    "waspelt",
    "keypressed",
    "actionprojectile2",
    "",
    "",
    "",
    "playerleaves",
    "washit",
    "",
    "",
    "shapetrigger",
    "playanimation",
];

/// Segment 1 as official bodies write it: a bit for each script event
/// named as a plain variable in code outside the functions (an old-style
/// `if (playerenters)` test), matched without regard to case.
fn event_mask(b: &Builder, top_level: &[std::ops::Range<usize>]) -> u32 {
    let mut mask = 0;
    for range in top_level {
        for at in range.clone() {
            let ins = &b.code[at];
            let (Op::TypeVar, Operand::Str(n)) = (ins.op, &ins.operand) else {
                continue;
            };
            let next = b.code.get(at + 1).map(|i| i.op);
            if matches!(next, Some(Op::MemberAccess | Op::Call)) {
                continue;
            }
            let name = b.strings[*n as usize].to_ascii_lowercase();
            if let Some(bit) = SCRIPT_EVENTS
                .iter()
                .position(|e| !e.is_empty() && *e == name)
            {
                mask |= 1 << bit;
            }
        }
    }
    mask
}

/// The value of a `true`/`false` test official bodies fold: the constants
/// themselves, `!` of one, `false && …` and `true || …`.
fn const_bool(e: &Expr) -> Option<bool> {
    match e {
        Expr::Var(v) if v.index.is_empty() && v.name == "true" => Some(true),
        Expr::Var(v) if v.index.is_empty() && v.name == "false" => Some(false),
        Expr::Not(x) => const_bool(x).map(|b| !b),
        Expr::Binary(BinOp::And, a, _) if const_bool(a) == Some(false) => Some(false),
        Expr::Binary(BinOp::Or, a, _) if const_bool(a) == Some(true) => Some(true),
        _ => None,
    }
}

/// A built-in that only gives a value (not `waitfor`), which GS2 leaves
/// out when it is not used.
fn unused_builtin(name: &str) -> bool {
    name != "waitfor" && call_pushes(name) && cache::inline_call(name)
}

/// Whether a built-in call has an argument count it takes, so that one
/// left out unused is still checked.
fn builtin_args_fit(name: &str, n: usize) -> bool {
    match name {
        "format" => n > 0,
        _ => builtin_arity(name).is_none_or(|want| want == n),
    }
}

/// Whether `++`, `--` and `=` can change `e` ([`Builder::step_place`]).
fn is_place(e: &Expr) -> bool {
    match e {
        Expr::Var(_) | Expr::Member { .. } | Expr::DynMember { .. } | Expr::Index { .. } => true,
        Expr::Call { name, .. } => name == "makevar",
        _ => false,
    }
}

/// The built-in methods that only give a value, which GS2 leaves out when
/// it is not used.
fn value_method(name: &str) -> bool {
    matches!(
        name,
        "size"
            | "type"
            | "indices"
            | "link"
            | "index"
            | "length"
            | "substring"
            | "pos"
            | "charat"
            | "starts"
            | "ends"
            | "trim"
            | "tokenize"
            | "positions"
            | "subarray"
            | "lower"
            | "upper"
    )
}
