//! The variable cache official bodies use: inside a function, a variable
//! that is read or written often, or at all inside a loop, is resolved
//! once at the top of the function and kept in a numbered slot.
//!
//! The rule is the one in `Graal_analysis`
//! `npc-library/gs2-official-emit-spec-2026-11-26.md`, checked against
//! the official captures listed there:
//!
//! - a reference is a plain name (`x`, `player`, `level`), a member of
//!   `this`, `thiso` or `temp` (`this.x`), or the name of a script or host
//!   call; `this`, `thiso`, `temp`, `playero` and `pi` on their own are
//!   not;
//! - a name is cached when it is referenced five times or more in the
//!   function, or once inside a loop (its test, its step and its body; a
//!   `for` initialiser counts as inside);
//! - `with` bodies, `new … { }` blocks, `@` texts, opcode built-ins and the
//!   inline string and list methods are not searched;
//! - slots are numbered in the order of the engine's name hash list over
//!   every referenced name, keeping only the cached ones;
//! - lambda bodies are never cached.
//!
//! The function prologue resolves each slot (`OP_MAKEVAR`-like opcode 47,
//! then 45 to store it, then `OP_INDEX_DEC`), and every later use of that
//! name loads the slot with opcode 46.

use super::ast::*;
use super::codegen::engine_hash_order;
use super::tables::opcode_builtin;
use std::collections::HashMap;

/// Opcode 47: resolve the value on top into a variable reference.
pub(super) const RESOLVE: u8 = 47;
/// Opcode 45: store the top into slot n, leaving it on the stack.
pub(super) const STORE: u8 = 45;
/// Opcode 46: push slot n.
pub(super) const LOAD: u8 = 46;

/// The inline object methods; their operands are not searched.
pub(super) fn inline_method(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
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
            | "clear"
            | "add"
            | "delete"
            | "insert"
            | "remove"
            | "replace"
            | "lower"
            | "upper"
    )
}

/// A call whose arguments are part of the instruction (not searched, and
/// its name is no reference). `pow` is the one built-in whose arguments
/// still count.
pub(super) fn inline_call(name: &str) -> bool {
    name != "pow" && (opcode_builtin(name).is_some() || matches!(name, "_" | "float"))
}

/// The slot key for a variable written `a.b.c`, if it has one: `this.b`,
/// `thiso.b` or `temp.b` for those roots, the root name otherwise.
pub(super) fn key_of(name: &str) -> Option<String> {
    let mut parts = name.split('.');
    let first = parts.next()?;
    match first {
        "this" | "thiso" | "temp" => parts.next().map(|m| format!("{first}.{m}")),
        "" | "playero" | "pi" | "true" | "false" | "null" | "NULL" => None,
        _ => Some(first.to_string()),
    }
}

#[derive(Default)]
struct Refs {
    /// Keys in first-seen order, with their count and whether any use is
    /// inside a loop.
    order: Vec<String>,
    seen: HashMap<String, (usize, bool)>,
    /// What a bare name stands for: the first of `name` or `temp.name`
    /// seen, or the loop variable a `for (temp.name : …)` rebinds it to.
    binding: HashMap<String, String>,
    /// The first binding of each bare name.
    first: HashMap<String, String>,
}

impl Refs {
    fn add(&mut self, key: String, in_loop: bool) {
        let key = match key.split_once('.') {
            Some(("temp", n)) => {
                self.binding
                    .entry(n.to_string())
                    .or_insert_with(|| key.clone());
                self.first
                    .entry(n.to_string())
                    .or_insert_with(|| key.clone());
                key
            }
            Some(_) => key,
            None => {
                let b = self
                    .binding
                    .entry(key.clone())
                    .or_insert_with(|| key.clone())
                    .clone();
                self.first.entry(key).or_insert_with(|| b.clone());
                b
            }
        };
        let e = self.seen.entry(key.clone()).or_insert_with(|| {
            self.order.push(key);
            (0, false)
        });
        e.0 += 1;
        e.1 |= in_loop;
    }

    fn var(&mut self, v: &Var, l: bool) {
        if let Some(k) = key_of(&v.name) {
            self.add(k, l);
        }
        for i in &v.index {
            self.expr(i, l);
        }
    }

    fn stmt(&mut self, s: &Stmt, l: bool) {
        match s {
            Stmt::Block(v) => v.iter().for_each(|s| self.stmt(s, l)),
            Stmt::If { cond, then, els } => {
                self.expr(cond, l);
                self.stmt(then, l);
                if let Some(e) = els {
                    self.stmt(e, l);
                }
            }
            Stmt::While { cond, body } => {
                self.expr(cond, true);
                self.stmt(body, true);
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                if let Some(i) = init {
                    self.stmt(i, true);
                }
                if let Some(c) = cond {
                    self.expr(c, true);
                }
                if let Some(s) = step {
                    self.stmt(s, true);
                }
                self.stmt(body, true);
            }
            Stmt::ForEach { var, list, body } => {
                // The loop variable is a use inside the loop.
                if let Some(k) = key_of(var) {
                    self.add(k, true);
                }
                if let Some(n) = var.strip_prefix("temp.") {
                    if !n.contains('.') {
                        self.binding.insert(n.to_string(), var.clone());
                    }
                }
                self.expr(list, true);
                self.stmt(body, true);
            }
            Stmt::Switch { value, arms } => {
                self.expr(value, l);
                for a in arms {
                    for v in &a.values {
                        self.expr(v, l);
                    }
                    for s in &a.body {
                        self.stmt(s, l);
                    }
                }
            }
            Stmt::Return(v) => {
                if let Some(v) = v {
                    self.expr(v, l);
                }
            }
            Stmt::Labeled { body, .. } => self.stmt(body, l),
            Stmt::Break
            | Stmt::Continue
            | Stmt::BreakTo(_)
            | Stmt::ContinueTo(_)
            | Stmt::With { .. }
            | Stmt::NewBlock { .. } => {}
            Stmt::Assign { target, value, .. } => {
                self.var(target, l);
                self.expr(value, l);
            }
            Stmt::IncDec { target, .. } => self.var(target, l),
            Stmt::Chain { targets, value } => {
                for t in targets {
                    self.var(t, l);
                }
                self.expr(value, l);
            }
            Stmt::SetPart { target, value } => {
                self.expr(target, l);
                self.expr(value, l);
            }
            Stmt::Call { name, args } => self.call(name, args, l),
            Stmt::Eval(e) => self.expr(e, l),
            Stmt::Embedded { body, .. } => self.stmt(body, l),
        }
    }

    fn call(&mut self, name: &str, args: &[Expr], l: bool) {
        if inline_call(name) {
            return;
        }
        if name != "pow" {
            self.add(name.to_string(), l);
        }
        for a in args {
            self.expr(a, l);
        }
    }

    fn expr(&mut self, e: &Expr, l: bool) {
        match e {
            Expr::Var(v) => self.var(v, l),
            Expr::Text(_) | Expr::Lambda { .. } | Expr::New { .. } | Expr::NewArray(_) => {}
            Expr::Call { name, args } => self.call(name, args, l),
            Expr::Method { obj, name, args } => {
                if inline_method(name) {
                    return;
                }
                if let Expr::Var(v) = &**obj {
                    if v.index.is_empty() && matches!(v.name.as_str(), "this" | "thiso") {
                        self.add(format!("{}.{name}", v.name), l);
                    }
                }
                self.expr(obj, l);
                for a in args {
                    self.expr(a, l);
                }
            }
            Expr::Member { obj, name } => match &**obj {
                Expr::Var(v)
                    if v.index.is_empty()
                        && matches!(v.name.as_str(), "this" | "thiso" | "temp") =>
                {
                    self.add(format!("{}.{name}", v.name), l)
                }
                _ => self.expr(obj, l),
            },
            // A run-time member of a bare `this` uses `this` itself.
            // `x in this.list`: the list is a use too.
            Expr::InArray { e, name } => {
                self.expr(e, l);
                if let Some(k) = key_of(name) {
                    self.add(k, l);
                }
            }
            Expr::DynMember { obj, name } if is_this(obj) => {
                self.add("this".to_string(), l);
                self.expr(name, l);
            }
            Expr::DynMethod { obj, args, .. } if is_this(obj) => {
                self.add("this".to_string(), l);
                for a in args {
                    self.expr(a, l);
                }
            }
            Expr::DynMethod { obj, args, .. } => {
                self.expr(obj, l);
                for a in args {
                    self.expr(a, l);
                }
            }
            other => {
                for p in other.parts() {
                    self.expr(p, l);
                }
            }
        }
    }
}

/// The slots for one function body: key to slot number, and the keys in
/// slot order.
pub(super) fn slots(body: &[Stmt]) -> (HashMap<String, i32>, Vec<String>) {
    let mut refs = Refs::default();
    for s in body {
        refs.stmt(s, false);
    }
    let Refs {
        order, seen, first, ..
    } = refs;
    let kept: Vec<String> = engine_hash_order(order, |k| k.clone())
        .into_iter()
        .filter(|k| {
            let (n, in_loop) = seen[k];
            n >= 5 || in_loop
        })
        .collect();
    let mut map: HashMap<String, i32> = kept
        .iter()
        .enumerate()
        .map(|(i, k)| (k.clone(), i as i32))
        .collect();
    // A bare name first seen as `temp.name` uses that slot too.
    for (name, b) in first {
        if b != name {
            if let Some(&slot) = map.get(&b) {
                map.entry(name).or_insert(slot);
            }
        }
    }
    (map, kept)
}

fn is_this(e: &Expr) -> bool {
    matches!(e, Expr::Var(v) if v.index.is_empty() && v.name == "this")
}
