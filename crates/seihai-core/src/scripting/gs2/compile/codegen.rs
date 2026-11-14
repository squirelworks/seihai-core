//! Code generation: the compiler's tree to GS2 bytecode.
//!
//! The encodings are the ones in `documentation/gs2-opcodes.md` and the
//! fixtures in `tests/fixtures/gs2`, and where those are silent, the
//! output of the Preagonal compiler (a port of gs2-parser) for small
//! hand-written probes, used as a black box. Where Seihai departs from that
//! output on purpose, the comment says so:
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
use super::gs1::name_is_call;
use super::lexer::Spellings;
use super::tables::*;
use super::{CompileError, CompileErrors};
use crate::scripting::engine::value::num_to_string;
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

pub(super) fn generate(program: &Program, spellings: Spellings) -> Result<Script, CompileErrors> {
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
        skips: Vec::new(),
        line: 0,
        spellings,
        lambdas: program
            .lambdas
            .iter()
            .map(|f| (f.name.clone(), f.clone()))
            .collect(),
    };
    let mut nth = 0;
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
                for s in body {
                    b.stmt(s);
                }
            }
            Item::Function(f) => {
                if last.get(f.name.as_str()) == Some(&nth) {
                    b.function(f);
                }
                nth += 1;
            }
        }
    }
    b.op(Op::Ret);
    let end = b.here();
    for skip in std::mem::take(&mut b.skips) {
        b.patch(skip, end);
    }
    if !b.errors.is_empty() {
        return Err(CompileErrors { errors: b.errors });
    }
    let script = Script {
        gs1_flags: 0,
        functions: b.functions,
        strings: b.strings,
        code: b.code,
    };
    Script::parse(&script.write()).map_err(|message| CompileErrors {
        errors: vec![CompileError { line: 0, message }],
    })
}

struct Builder {
    code: Vec<Instr>,
    strings: Vec<String>,
    index: HashMap<String, u32>,
    functions: Vec<FnEntry>,
    errors: Vec<CompileError>,
    loops: Vec<Loop>,
    /// Jumps over function bodies, patched to the next code outside them.
    skips: Vec<usize>,
    line: u32,
    spellings: Spellings,
    /// `function () { … }` values by function name, emitted where used.
    lambdas: HashMap<String, Function>,
}

// ---- what counts as a call ----

/// Whether a statement calls anything, for the function's `OP_CMD_CALL`.
fn stmt_calls(s: &Stmt) -> bool {
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
        Stmt::ForEach { list, body, .. } => expr_calls(list) || stmt_calls(body),
        Stmt::Switch { value, arms } => {
            expr_calls(value)
                || arms.iter().flat_map(|a| &a.values).any(expr_calls)
                || arms.iter().flat_map(|a| &a.body).any(stmt_calls)
        }
        Stmt::Return(v) => v.as_ref().is_some_and(expr_calls),
        Stmt::With { target, body, .. } => expr_calls(target) || stmt_calls(body),
        Stmt::NewBlock { name, body, .. } => expr_calls(name) || body.iter().any(stmt_calls),
        Stmt::Assign { target, value, .. } => index(target) || expr_calls(value),
        Stmt::IncDec { target, .. } => index(target),
        Stmt::Chain { targets, value } => targets.iter().any(index) || expr_calls(value),
        Stmt::SetPart { target, value } => expr_calls(target) || expr_calls(value),
        Stmt::Call { name, args } => name_is_call(name, false) || args.iter().any(expr_calls),
        Stmt::Eval(e) => expr_calls(e),
        Stmt::Embedded { calls, .. } => *calls,
        Stmt::Break | Stmt::Continue => false,
    }
}

/// Whether an expression calls anything. Opcode built-ins other than
/// `format` do not; a method, `new`, or a call by a run-time name does.
/// GS1 text with message codes does not.
fn expr_calls(e: &Expr) -> bool {
    match e {
        Expr::Call { name, args } => name_is_call(name, false) || args.iter().any(expr_calls),
        Expr::HostCall { .. }
        | Expr::Method { .. }
        | Expr::DynMethod { .. }
        | Expr::DynCall { .. }
        | Expr::New { .. } => true,
        Expr::Message(_) | Expr::Lambda(_) => false,
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
    fn emit_number(&mut self, n: f64) {
        if let Some(text) = self.spellings.get(n) {
            self.push(Op::TypeNumber, Operand::Decimal(text));
        } else if n.is_finite()
            && n.fract() == 0.0
            && (i32::MIN as f64..=i32::MAX as f64).contains(&n)
        {
            self.emit_int(n as i32);
        } else {
            self.push(Op::TypeNumber, Operand::Decimal(num_to_string(n)));
        }
    }

    fn new_loop(&mut self) {
        self.loops.push(Loop {
            continues: Vec::new(),
            breaks: Vec::new(),
        });
    }

    // ---- functions ----

    /// A function where it stands: a jump over it, the parameter list,
    /// `OP_CMD_CALL` when the body calls anything, the body, and `return 0`
    /// unless the body always returns.
    fn function(&mut self, f: &Function) {
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
            if p.contains('.') {
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
        for s in &f.body {
            self.stmt(s);
        }
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
            Stmt::With { target, body, line } => self.with_stmt(target, body, *line),
            Stmt::NewBlock { ty, name, body } => self.new_block(ty, name, body),
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
        if let Some(cond) = cond {
            self.cond_fail(cond, &mut fails);
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
        self.op(Op::IndexDec);
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
    fn new_block(&mut self, ty: &str, name: &Expr, body: &[Stmt]) {
        self.value(name);
        self.op(Op::InlineNew);
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
                self.op(Op::ConvToFloat);
                self.numeric(value);
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
        }
    }

    // ---- places ----

    /// A dotted name: its first part, then each member with
    /// `OP_MEMBER_ACCESS`. As in gs2-parser's output, the value is
    /// converted to an object before each member, except straight after
    /// `this`/`temp`/`player`/… (`this.a.b` is THIS a MA CTO b MA;
    /// `client.a` is VAR client CTO a MA).
    fn place(&mut self, name: &str) {
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
        self.value(obj);
        let root = matches!(obj, Expr::Var(v) if v.index.is_empty() && root_op(&v.name).is_some());
        let element = matches!(obj, Expr::Var(v) if !v.index.is_empty());
        if !root && !element {
            self.op(Op::ConvToObject);
        }
    }

    // ---- calls ----

    /// A call by name. Returns whether it pushed a value a statement
    /// should drop.
    fn call(&mut self, name: &str, args: &[Expr]) -> bool {
        if self.builtin(name, args) {
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
        self.emit_var(name);
        self.op(Op::Call);
    }

    /// An opcode built-in. Returns `false` when `name` is not one.
    fn builtin(&mut self, name: &str, args: &[Expr]) -> bool {
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
            "_" if args.len() == 1 => {
                self.value(&args[0]);
                self.op(Op::Translate);
            }
            // `makevar` takes its name as a string, converted even when it
            // already is one, as the reference compiler does.
            "makevar" => {
                self.value(&args[0]);
                self.op(Op::ConvToString);
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
            "sin" | "cos" | "arctan" | "abs" | "random" | "exp" | "log" | "min" | "max"
            | "char" | "vecx" | "vecy" => {
                for a in args.iter().rev() {
                    self.numeric(a);
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
            Expr::Neg(a) => {
                if let Expr::Num(n) = a.as_ref() {
                    self.emit_number(-n);
                } else {
                    self.numeric(a);
                    self.op(Op::UnarySub);
                }
                Kind::Number
            }
            Expr::Not(a) => {
                if is_logic(a) {
                    self.logic(a);
                } else {
                    self.bool_leaf(a);
                }
                self.op(Op::Not);
                Kind::Other
            }
            Expr::Binary(BinOp::And | BinOp::Or, _, _) => {
                self.logic(e);
                Kind::Other
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
                });
                Kind::Number
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
            // The reference compiler uses the result of `OP_IN_RANGE` and
            // `OP_IN_OBJ` as a number with no `OP_CONV_TO_FLOAT`.
            Expr::InRange { e, lo, hi } => {
                self.value(e);
                self.value(lo);
                self.value(hi);
                self.op(Op::InRange);
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
                self.call(name, args);
                if matches!(name.as_str(), "int" | "arraylen" | "sarraylen" | "strlen") {
                    Kind::Number
                } else {
                    Kind::Other
                }
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
                self.object(obj);
                self.emit_var(name);
                self.op(Op::MemberAccess);
                Kind::Other
            }
            // `a[i][j]`: no conversion between the two reads in the
            // Preagonal baselines.
            Expr::Index { obj, index } => {
                self.value(obj);
                let element = matches!(&**obj, Expr::Var(v) if !v.index.is_empty())
                    || matches!(&**obj, Expr::Index { .. });
                if !element {
                    self.op(Op::ConvToObject);
                }
                for i in index {
                    self.value(i);
                }
                self.op(element_op(index.len()));
                Kind::Other
            }
            // `obj.(n)`: the object, the name as a string, member access.
            // Seihai's form: the Preagonal output for `obj.(@n)` adds the
            // name to the object as numbers.
            Expr::DynMember { obj, name } => {
                self.object(obj);
                self.value(name);
                self.op(Op::ConvToString);
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
                self.object(obj);
                self.value(name);
                self.op(Op::ConvToString);
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
            // `objects.gs2bc`: `new TStaticVar()` pushes the name
            // "unknown_object", the type, then `OP_NEW_OBJECT`. The type is
            // interned first. With an argument, Seihai pushes it in place
            // of the default name (no fixture).
            Expr::New { ty, args } => {
                let t = self.intern(ty);
                match args.first() {
                    Some(a) => {
                        self.value(a);
                    }
                    None => self.emit_var("unknown_object"),
                }
                self.push(Op::TypeString, Operand::Str(t));
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
            Expr::Text(e) => {
                self.value(e);
                self.op(Op::ConvToString);
                Kind::Other
            }
            // `function (…) { … }` as a value (Preagonal probe): the
            // function emitted where it stands and jumped over, then
            // `this.<name>` as an object.
            Expr::Lambda(name) => {
                let Some(f) = self.lambdas.remove(&format!("public.{name}")) else {
                    self.err("a `function () { … }` value is used twice");
                    return Kind::Other;
                };
                let loops = std::mem::take(&mut self.loops);
                self.function(&f);
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
                Kind::Number
            }
            // `++x` used as a value: the place and `OP_INC`, which leaves
            // the changed variable (Seihai's form, no probe).
            Expr::PreStep { target, delta } => {
                if !self.step_place(target) {
                    return Kind::Other;
                }
                self.op(if *delta < 0.0 { Op::Dec } else { Op::Inc });
                Kind::Other
            }
            // `y = (x = 2)` (Preagonal probe): x, `OP_COPY_LAST_OP`, the
            // value, `OP_ASSIGN`, leaving x. For `+=` and the others Seihai
            // copies the place once more, so x is still there after the
            // store (the Preagonal output does not, and leaves nothing).
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
            return Kind::Other;
        }
        match op {
            BinOp::Eq | BinOp::Ne => {
                self.value(a);
                self.value(b);
                self.op(if op == BinOp::Eq { Op::Eq } else { Op::Neq });
                Kind::Other
            }
            BinOp::Cat | BinOp::CatSpc | BinOp::CatNl | BinOp::CatTab => {
                self.stringish(a);
                let sep = match op {
                    BinOp::CatSpc => Some(" "),
                    BinOp::CatNl => Some("\n"),
                    BinOp::CatTab => Some("\t"),
                    _ => None,
                };
                if let Some(sep) = sep {
                    self.emit_str(sep);
                    self.op(Op::Join);
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

    fn logic_operand(&mut self, e: &Expr) {
        match e {
            Expr::Binary(BinOp::Or, a, b) => {
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
            Expr::Binary(BinOp::And, _, _) => {
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
        if let Expr::Binary(BinOp::And, a, b) = e {
            self.and_parts(a, holes);
            holes.push(self.hole(Op::And));
            self.and_parts(b, holes);
        } else if is_or(e) {
            self.logic_operand(e);
        } else {
            self.bool_leaf(e);
        }
    }

    /// A condition that jumps to the holes in `fails` when false.
    fn cond_fail(&mut self, e: &Expr, fails: &mut Vec<usize>) {
        match e {
            Expr::Binary(BinOp::And, a, b) => {
                self.cond_fail(a, fails);
                self.cond_fail(b, fails);
            }
            Expr::Binary(BinOp::Or, a, b) => {
                let mut trues = Vec::new();
                self.cond_true(a, &mut trues);
                self.cond_fail(b, fails);
                let success = self.here();
                for t in trues {
                    self.patch(t, success);
                }
            }
            Expr::Not(x) if is_logic(x) => self.cond_true(x, fails),
            Expr::Not(x) => {
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

    /// A condition that jumps to the holes in `trues` when true.
    fn cond_true(&mut self, e: &Expr, trues: &mut Vec<usize>) {
        match e {
            Expr::Binary(BinOp::Or, a, b) => {
                self.cond_true(a, trues);
                self.cond_true(b, trues);
            }
            Expr::Binary(BinOp::And, a, b) => {
                let mut local = Vec::new();
                self.cond_fail(a, &mut local);
                self.cond_true(b, trues);
                let after = self.here();
                for f in local {
                    self.patch(f, after);
                }
            }
            Expr::Not(x) if is_logic(x) => self.cond_fail(x, trues),
            Expr::Not(x) => {
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
