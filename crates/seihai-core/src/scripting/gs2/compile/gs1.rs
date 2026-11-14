//! The GS1 front end.
//!
//! A GS1 script is parsed with the GS1 grammar in
//! [`crate::scripting::engine`] and lowered here to the compiler's tree:
//!
//! - Top-level event tests (`if (playerenters)`) become GS2 event functions
//!   (`onPlayerEnters`); see [`lift`].
//! - Text with message codes (`"hi #n"`) becomes [`Expr::Message`].
//! - GS1 commands become GS2 calls by name, or the GS2 statement that does
//!   the same (`setstring this.a,1;` is `this.a = 1;`).
//! - GS1 built-ins with no GS2 name become [`Expr::Gs1Fn`].
//! - GS1 forms whose GS2 encoding would read or run things in a different
//!   order are rewritten (`a[i] += v` is `a[i] = a[i] + v`) or refused.
//!
//! Array literals use GS2's reverse-push encoding. The GS1 interpreter
//! evaluates elements left to right, so a literal is taken only when that
//! order cannot matter: no element calls script or host code or `random`,
//! or exactly one does and the rest are constants.
//!
//! `x in {a, b, c}` is `x == a || x == b || x == c`, which is what the GS1
//! interpreter does, and is refused when `x` calls a function.
//!
//! Two of these differ on purpose from gs2-parser, which reads every script
//! as GS2 (issue 32). In an unmarked GS1 script, `if (playerenters) {…}`
//! runs on entry, where gs2-parser tests a variable that is never set; and
//! a bare command such as `noplayerkilling;` is called, where gs2-parser
//! pushes the name and drops it. On real 5.12 and 5.24r4 clients this is
//! visible: a level NPC that sets the arrow and bomb counts, or turns on
//! no-player-killing (the red crossed icon by the HP/AP bar), does so with
//! Seihai's bytecode and not with gs2-parser's. That is the GS1 meaning, so
//! it stays. `tests/gs1_hud_semantics.rs` covers both.

use super::ast::*;
use super::tables::*;
use super::{CompileError, CompileErrors};
use crate::scripting::engine::ast as g;
use crate::scripting::engine::{self, vocab, Era};
use std::collections::HashMap;

/// Parse and lower a whole GS1 script.
pub(super) fn program(src: &str, era: Era) -> Result<Program, CompileErrors> {
    let parsed = engine::parse(src, era);
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
    let mut flags = Vec::new();
    parsed.body.iter().for_each(|s| player_flags(s, &mut flags));
    let raw = lift(parsed.body, era)?;
    let mut lower = Lower {
        flags,
        era,
        line: 0,
        errors: Vec::new(),
        script_fns: raw
            .iter()
            .filter_map(|i| match i {
                Raw::Function { name, .. } => Some(name.to_ascii_lowercase()),
                Raw::Code(_) => None,
            })
            .collect(),
        with_depth: 0,
        temps: 0,
    };
    let items = raw
        .into_iter()
        .map(|item| match item {
            Raw::Function { name, params, body } => Item::Function(Function {
                cmd_call: Some(body.iter().any(|s| stmt_calls(s, true))),
                name,
                params,
                body: lower.stmts(&body),
            }),
            Raw::Code(body) => Item::Code(lower.stmts(&body)),
        })
        .collect();
    if !lower.errors.is_empty() {
        return Err(CompileErrors {
            errors: lower.errors,
        });
    }
    Ok(Program {
        items,
        lambdas: Vec::new(),
        later_wins: false,
    })
}

/// Lower one GS1 command written inside a GS2 script.
pub(super) fn lower_embedded(command: &g::Stmt, era: Era) -> Result<Stmt, String> {
    let mut lower = Lower {
        era,
        line: 0,
        errors: Vec::new(),
        script_fns: Vec::new(),
        with_depth: 0,
        temps: 0,
        flags: Vec::new(),
    };
    let body = lower.stmt(command);
    match lower.errors.into_iter().next() {
        Some(e) => Err(e.message),
        None => Ok(Stmt::Embedded {
            body: Box::new(body),
            calls: stmt_calls(command, false),
        }),
    }
}

/// GS1 text with message codes (`"#a"`, `"hp: #v(this.hp)"`), split into
/// its parts.
pub(super) fn message(text: &str, era: Era) -> Result<Vec<MsgPart>, String> {
    let mut lower = Lower {
        era,
        line: 0,
        errors: Vec::new(),
        script_fns: Vec::new(),
        with_depth: 0,
        temps: 0,
        flags: Vec::new(),
    };
    let parts = lower.message(text)?;
    match lower.errors.into_iter().next() {
        Some(e) => Err(e.message),
        None => Ok(parts),
    }
}

// ---- lifting events into functions ----

enum Raw {
    Function {
        name: String,
        params: Vec<String>,
        body: Vec<g::Stmt>,
    },
    Code(Vec<g::Stmt>),
}

/// Lifts top-level GS1 into GS2 functions. Several `if (created)` blocks
/// (or any repeated event test) become one `onCreated`, their bodies in
/// source order: GS1 runs every block whose test is true, top to bottom, and
/// only one event is true per run. A block that can `return` is not merged,
/// since the return would skip the blocks after it.
fn lift(body: Vec<g::Stmt>, era: Era) -> Result<Vec<Raw>, CompileErrors> {
    let mut items: Vec<Raw> = Vec::new();
    let mut event_items = HashMap::<String, usize>::new();
    let mut errors = Vec::new();
    let mut seen_event = false;
    let err = |message: &str| CompileError {
        line: 0,
        message: message.into(),
    };
    for stmt in body {
        match stmt {
            g::Stmt::Function { name, params, body } => {
                items.push(Raw::Function { name, params, body })
            }
            g::Stmt::If { cond, then, els } => match event_arms(&cond, era) {
                Arms::NotEvent => {
                    let stmt = g::Stmt::If { cond, then, els };
                    top_level(&mut items, &mut errors, stmt, seen_event);
                }
                Arms::Complex => errors.push(err(
                    "event test is too complex to lower; split it into one if per event",
                )),
                Arms::Events(arms) => {
                    seen_event = true;
                    if els.is_some() {
                        errors.push(err("an event test with else has no single GS2 function"));
                        continue;
                    }
                    let base = match *then {
                        g::Stmt::Block(v) => v,
                        other => vec![other],
                    };
                    for (event, guard) in arms {
                        let body = match guard {
                            Some(guard) => vec![g::Stmt::If {
                                cond: guard,
                                then: Box::new(g::Stmt::Block(base.clone())),
                                els: None,
                            }],
                            None => base.clone(),
                        };
                        let name = gs2_event_function(&event);
                        match event_items.get(&name) {
                            Some(&at) => {
                                let Raw::Function { body: merged, .. } = &mut items[at] else {
                                    unreachable!("event items are functions")
                                };
                                if merged.iter().any(can_return) {
                                    errors.push(err(&format!(
                                        "`{name}` blocks are not merged because an earlier one can return"
                                    )));
                                    continue;
                                }
                                merged.extend(body);
                            }
                            None => {
                                event_items.insert(name.clone(), items.len());
                                items.push(Raw::Function {
                                    name,
                                    params: Vec::new(),
                                    body,
                                });
                            }
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
fn top_level(
    items: &mut Vec<Raw>,
    errors: &mut Vec<CompileError>,
    stmt: g::Stmt,
    seen_event: bool,
) {
    if seen_event && !matches!(stmt, g::Stmt::Command { .. } | g::Stmt::Call { .. }) {
        errors.push(CompileError {
            line: 0,
            message: "a top-level statement after an event test is not lowered \
                      (GS2 runs code outside functions before the handler); move it above the first event test"
                .into(),
        });
        return;
    }
    match items.last_mut() {
        Some(Raw::Code(code)) => code.push(stmt),
        _ => items.push(Raw::Code(vec![stmt])),
    }
}

fn can_return(s: &g::Stmt) -> bool {
    match s {
        g::Stmt::Return(_) => true,
        g::Stmt::Block(v) => v.iter().any(can_return),
        g::Stmt::If { then, els, .. } => can_return(then) || els.as_deref().is_some_and(can_return),
        g::Stmt::While { body, .. } | g::Stmt::With { body, .. } => can_return(body),
        g::Stmt::For { body, .. } => can_return(body),
        _ => false,
    }
}

fn is_event(name: &str, era: Era) -> bool {
    vocab::event_since(name).is_some_and(|e| e <= era)
}

fn contains_event(e: &g::Expr, era: Era) -> bool {
    match e {
        g::Expr::Var(v) => is_event(&v.name, era),
        g::Expr::Unary(_, a) | g::Expr::InArray { e: a, .. } => contains_event(a, era),
        g::Expr::Bin(_, a, b) => contains_event(a, era) || contains_event(b, era),
        g::Expr::Call { args, .. } => args.iter().any(|a| contains_event(a, era)),
        g::Expr::InRange { e, lo, hi } => {
            contains_event(e, era) || contains_event(lo, era) || contains_event(hi, era)
        }
        g::Expr::InList { e, list } => {
            contains_event(e, era) || list.iter().any(|a| contains_event(a, era))
        }
        g::Expr::List(list) => list.iter().any(|a| contains_event(a, era)),
        g::Expr::Cond { cond, then, els } => {
            contains_event(cond, era) || contains_event(then, era) || contains_event(els, era)
        }
        g::Expr::Num(_) | g::Expr::Str(_) => false,
    }
}

enum Arms {
    Events(Vec<(String, Option<g::Expr>)>),
    NotEvent,
    Complex,
}

/// Top-level `if (playerenters)` / `if (created || timeout)` /
/// `if (event && guard)`.
fn event_arms(e: &g::Expr, era: Era) -> Arms {
    let other = || {
        if contains_event(e, era) {
            Arms::Complex
        } else {
            Arms::NotEvent
        }
    };
    match e {
        g::Expr::Var(v) if is_event(&v.name, era) => Arms::Events(vec![(v.name.clone(), None)]),
        g::Expr::Bin(g::BinOp::Or, a, b) => match (event_arms(a, era), event_arms(b, era)) {
            (Arms::Events(mut l), Arms::Events(r)) => {
                if l.iter().any(|(_, g)| g.is_some()) || r.iter().any(|(_, g)| g.is_some()) {
                    Arms::Complex
                } else {
                    l.extend(r);
                    Arms::Events(l)
                }
            }
            _ => other(),
        },
        // `a && event && b && …`: one event among the terms, the rest a
        // guard in source order. A term before the event that calls a
        // function would run in every event in GS1, so that stays complex.
        g::Expr::Bin(g::BinOp::And, _, _) => {
            let mut terms = Vec::new();
            and_terms(e, &mut terms);
            let event_var = |t: &g::Expr| match t {
                g::Expr::Var(v) if is_event(&v.name, era) => Some(v.name.clone()),
                _ => None,
            };
            let events: Vec<usize> = (0..terms.len())
                .filter(|&i| event_var(terms[i]).is_some())
                .collect();
            let others_clean = terms
                .iter()
                .enumerate()
                .all(|(i, t)| events.contains(&i) || !contains_event(t, era));
            match events.as_slice() {
                [at] if others_clean && !terms[..*at].iter().any(|t| mentions_any_call(t)) => {
                    let ev = event_var(terms[*at]).unwrap();
                    let guard = terms
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| i != at)
                        .map(|(_, t)| (*t).clone())
                        .reduce(|a, b| g::Expr::Bin(g::BinOp::And, Box::new(a), Box::new(b)));
                    Arms::Events(vec![(ev, guard)])
                }
                [] if others_clean => Arms::NotEvent,
                _ => Arms::Complex,
            }
        }
        _ => other(),
    }
}

fn and_terms<'a>(e: &'a g::Expr, out: &mut Vec<&'a g::Expr>) {
    match e {
        g::Expr::Bin(g::BinOp::And, a, b) => {
            and_terms(a, out);
            and_terms(b, out);
        }
        _ => out.push(e),
    }
}

/// The expression calls any function (script, host or built-in).
fn mentions_any_call(e: &g::Expr) -> bool {
    match e {
        g::Expr::Call { .. } => true,
        g::Expr::Num(_) | g::Expr::Str(_) => false,
        g::Expr::Var(v) => v.index.iter().any(mentions_any_call),
        g::Expr::Unary(_, a) | g::Expr::InArray { e: a, .. } => mentions_any_call(a),
        g::Expr::Bin(_, a, b) => mentions_any_call(a) || mentions_any_call(b),
        g::Expr::InRange { e, lo, hi } => {
            mentions_any_call(e) || mentions_any_call(lo) || mentions_any_call(hi)
        }
        g::Expr::InList { e, list } => mentions_any_call(e) || list.iter().any(mentions_any_call),
        g::Expr::Cond { cond, then, els } => {
            mentions_any_call(cond) || mentions_any_call(then) || mentions_any_call(els)
        }
        g::Expr::List(v) => v.iter().any(mentions_any_call),
    }
}

// ---- what counts as a call ----

/// Whether a call of `name` counts as a call for `OP_CMD_CALL`. Opcode
/// built-ins do not, except `format` (`objects.gs2bc`: a body whose only
/// call is `format` still gets `OP_CMD_CALL`). With `gs1`, the GS1
/// built-ins lowered to opcodes do not either.
pub(super) fn name_is_call(name: &str, gs1: bool) -> bool {
    if name == "format" {
        return true;
    }
    if opcode_builtin(name).is_some() {
        return false;
    }
    if gs1 && matches!(name, "strlen" | "startswith" | "indexof") {
        return false;
    }
    !(gs1 && gs1_fn_without_gs2(name))
}

/// Whether a GS1 statement counts as a call for `OP_CMD_CALL`.
fn stmt_calls(s: &g::Stmt, gs1: bool) -> bool {
    let e = |x: &g::Expr| expr_calls(x, gs1);
    let s_ = |x: &g::Stmt| stmt_calls(x, gs1);
    let index = |v: &g::VarRef| v.index.iter().any(e);
    match s {
        g::Stmt::Block(v) => v.iter().any(s_),
        g::Stmt::If { cond, then, els } => e(cond) || s_(then) || els.as_deref().is_some_and(s_),
        g::Stmt::While { cond, body } => e(cond) || s_(body),
        g::Stmt::For {
            init,
            cond,
            step,
            body,
        } => {
            init.as_deref().is_some_and(s_)
                || cond.as_ref().is_some_and(e)
                || step.as_deref().is_some_and(s_)
                || s_(body)
        }
        g::Stmt::Function { body, .. } => body.iter().any(s_),
        g::Stmt::Return(v) => v.as_ref().is_some_and(e),
        g::Stmt::Assign { target, value, .. } => index(target) || e(value),
        g::Stmt::Call { name, args, .. } => name_is_call(name, gs1) || args.iter().any(e),
        g::Stmt::Command { name, args, .. } => {
            command_is_call(name, gs1)
                || args
                    .iter()
                    .any(|a| matches!(a, g::Arg::Num(x) if expr_calls(x, gs1)))
        }
        g::Stmt::With { target, body, .. } => e(target) || s_(body),
        g::Stmt::IncDec { target, .. } => index(target),
        g::Stmt::Break | g::Stmt::Continue | g::Stmt::Unknown { .. } => false,
    }
}

/// Whether a GS1 command counts as a call for `OP_CMD_CALL`, judged by
/// what it is lowered to: `setstring this.a,1;` and `setcharprop` become
/// assignments, which do not; a flag `setstring` is a `set` call, and the
/// list and `tokenize` forms are method calls, which do.
fn command_is_call(name: &str, gs1: bool) -> bool {
    match name {
        // Lowered to an assignment to a `client.` (or `this.`) variable.
        "setstring" | "set" | "unset" => false,
        "setcharprop" => false,
        _ => name_is_call(name, gs1),
    }
}

/// Whether a GS1 expression counts as a call. Text with message codes does
/// not; a count (`tokenscount`) does, as it is lowered to `.size()`.
fn expr_calls(e: &g::Expr, gs1: bool) -> bool {
    let c = |x: &g::Expr| expr_calls(x, gs1);
    match e {
        g::Expr::Call { name, args } => name_is_call(name, gs1) || args.iter().any(c),
        g::Expr::Unary(_, a) | g::Expr::InArray { e: a, .. } => c(a),
        g::Expr::Bin(_, a, b) => c(a) || c(b),
        g::Expr::InRange { e, lo, hi } => c(e) || c(lo) || c(hi),
        g::Expr::InList { e, list } => c(e) || list.iter().any(c),
        g::Expr::Cond { cond, then, els } => c(cond) || c(then) || c(els),
        g::Expr::List(v) => v.iter().any(c),
        g::Expr::Var(v) => {
            (v.index.is_empty() && counted_list(&v.name).is_some()) || v.index.iter().any(c)
        }
        g::Expr::Num(_) | g::Expr::Str(_) => false,
    }
}

/// The expression calls `name` anywhere.
fn mentions_call(e: &g::Expr, name: &str) -> bool {
    let m = |x: &g::Expr| mentions_call(x, name);
    match e {
        g::Expr::Call { name: n, args } => n.eq_ignore_ascii_case(name) || args.iter().any(m),
        g::Expr::Num(_) | g::Expr::Str(_) => false,
        g::Expr::Var(v) => v.index.iter().any(m),
        g::Expr::Unary(_, a) | g::Expr::InArray { e: a, .. } => m(a),
        g::Expr::Bin(_, a, b) => m(a) || m(b),
        g::Expr::InRange { e, lo, hi } => m(e) || m(lo) || m(hi),
        g::Expr::InList { e, list } => m(e) || list.iter().any(m),
        g::Expr::Cond { cond, then, els } => m(cond) || m(then) || m(els),
        g::Expr::List(v) => v.iter().any(m),
    }
}

fn has_nested_list(e: &g::Expr) -> bool {
    match e {
        g::Expr::List(_) => true,
        g::Expr::Num(_) | g::Expr::Str(_) => false,
        g::Expr::Var(v) => v.index.iter().any(has_nested_list),
        g::Expr::Unary(_, a) | g::Expr::InArray { e: a, .. } => has_nested_list(a),
        g::Expr::Bin(_, a, b) => has_nested_list(a) || has_nested_list(b),
        g::Expr::Call { args, .. } => args.iter().any(has_nested_list),
        g::Expr::InRange { e, lo, hi } => {
            has_nested_list(e) || has_nested_list(lo) || has_nested_list(hi)
        }
        g::Expr::InList { e, list } => has_nested_list(e) || list.iter().any(has_nested_list),
        g::Expr::Cond { cond, then, els } => {
            has_nested_list(cond) || has_nested_list(then) || has_nested_list(els)
        }
    }
}

// ---- `with` ----

fn is_wait(name: &str) -> bool {
    name.eq_ignore_ascii_case("sleep") || name.eq_ignore_ascii_case("waitfor")
}

fn waits(e: &g::Expr) -> bool {
    mentions_call(e, "sleep") || mentions_call(e, "waitfor")
}

/// A `return`, a `sleep` or `waitfor`, or a `break`/`continue` that would
/// leave a GS1 `with` block. The NPC server's notes say the old object is
/// not restored then.
fn with_escape(s: &g::Stmt, in_loop: bool) -> bool {
    let index = |v: &g::VarRef| v.index.iter().any(waits);
    match s {
        g::Stmt::Block(v) => v.iter().any(|s| with_escape(s, in_loop)),
        g::Stmt::If { cond, then, els } => {
            waits(cond)
                || with_escape(then, in_loop)
                || els.as_deref().is_some_and(|e| with_escape(e, in_loop))
        }
        g::Stmt::While { cond, body } => waits(cond) || with_escape(body, true),
        g::Stmt::For {
            init,
            cond,
            step,
            body,
        } => {
            cond.as_ref().is_some_and(waits)
                || init.as_deref().is_some_and(|s| with_escape(s, in_loop))
                || step.as_deref().is_some_and(|s| with_escape(s, true))
                || with_escape(body, true)
        }
        g::Stmt::With { body, .. } => with_escape(body, false),
        g::Stmt::Return(_) => true,
        g::Stmt::Break | g::Stmt::Continue => !in_loop,
        g::Stmt::Command { name, args, .. } => {
            is_wait(name) || args.iter().any(|a| matches!(a, g::Arg::Num(e) if waits(e)))
        }
        g::Stmt::Call { name, args, .. } => is_wait(name) || args.iter().any(waits),
        g::Stmt::Assign { target, value, .. } => index(target) || waits(value),
        g::Stmt::IncDec { target, .. } => index(target),
        g::Stmt::Function { .. } | g::Stmt::Unknown { .. } => false,
    }
}

/// Every variable name in a statement, with `true` for a written one.
fn stmt_names(s: &g::Stmt, f: &mut dyn FnMut(&str, bool)) {
    match s {
        g::Stmt::Block(v) => v.iter().for_each(|s| stmt_names(s, f)),
        g::Stmt::If { cond, then, els } => {
            expr_names(cond, f);
            stmt_names(then, f);
            if let Some(e) = els {
                stmt_names(e, f);
            }
        }
        g::Stmt::While { cond, body } => {
            expr_names(cond, f);
            stmt_names(body, f);
        }
        g::Stmt::For {
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
        g::Stmt::With { target, body, .. } => {
            expr_names(target, f);
            stmt_names(body, f);
        }
        g::Stmt::Return(v) => {
            if let Some(e) = v {
                expr_names(e, f);
            }
        }
        g::Stmt::Assign { target, value, .. } => {
            f(&target.name, true);
            target.index.iter().for_each(|e| expr_names(e, f));
            expr_names(value, f);
        }
        g::Stmt::IncDec { target, .. } => {
            f(&target.name, true);
            target.index.iter().for_each(|e| expr_names(e, f));
        }
        g::Stmt::Call { args, .. } => args.iter().for_each(|e| expr_names(e, f)),
        g::Stmt::Command { args, .. } => args.iter().for_each(|a| {
            if let g::Arg::Num(e) = a {
                expr_names(e, f);
            }
        }),
        g::Stmt::Function { .. } | g::Stmt::Break | g::Stmt::Continue | g::Stmt::Unknown { .. } => {
        }
    }
}

fn expr_names(e: &g::Expr, f: &mut dyn FnMut(&str, bool)) {
    match e {
        g::Expr::Num(_) | g::Expr::Str(_) => {}
        g::Expr::Var(v) => {
            f(&v.name, false);
            v.index.iter().for_each(|e| expr_names(e, f));
        }
        g::Expr::Unary(_, a) => expr_names(a, f),
        g::Expr::InArray { e, name } => {
            expr_names(e, f);
            f(name, false);
        }
        g::Expr::Bin(_, a, b) => {
            expr_names(a, f);
            expr_names(b, f);
        }
        g::Expr::Call { args, .. } | g::Expr::List(args) => {
            args.iter().for_each(|e| expr_names(e, f))
        }
        g::Expr::InRange { e, lo, hi } => {
            expr_names(e, f);
            expr_names(lo, f);
            expr_names(hi, f);
        }
        g::Expr::InList { e, list } => {
            expr_names(e, f);
            list.iter().for_each(|e| expr_names(e, f));
        }
        g::Expr::Cond { cond, then, els } => {
            expr_names(cond, f);
            expr_names(then, f);
            expr_names(els, f);
        }
    }
}

/// The lowered body calls one of `fns` by name, or reads or writes the
/// player object (`player`, `playero` as the first part of a name).
fn lowered_reaches(s: &Stmt, fns: &[String]) -> (bool, bool) {
    let mut calls = false;
    let mut player = false;
    walk_stmt(s, &mut |name, call| {
        if call {
            if opcode_builtin(name).is_none() && fns.contains(&name.to_ascii_lowercase()) {
                calls = true;
            }
        } else if matches!(name.split('.').next(), Some("player" | "playero")) {
            player = true;
        }
    });
    (calls, player)
}

/// Calls every name in a lowered statement: `(name, true)` for a call by
/// name, `(name, false)` for a variable.
fn walk_stmt(s: &Stmt, f: &mut dyn FnMut(&str, bool)) {
    let var = |v: &Var, f: &mut dyn FnMut(&str, bool)| {
        f(&v.name, false);
        v.index.iter().for_each(|e| walk_expr(e, f));
    };
    match s {
        Stmt::Block(v) => v.iter().for_each(|s| walk_stmt(s, f)),
        Stmt::If { cond, then, els } => {
            walk_expr(cond, f);
            walk_stmt(then, f);
            if let Some(e) = els {
                walk_stmt(e, f);
            }
        }
        Stmt::While { cond, body } => {
            walk_expr(cond, f);
            walk_stmt(body, f);
        }
        Stmt::For {
            init,
            cond,
            step,
            body,
        } => {
            init.iter().for_each(|s| walk_stmt(s, f));
            cond.iter().for_each(|e| walk_expr(e, f));
            step.iter().for_each(|s| walk_stmt(s, f));
            walk_stmt(body, f);
        }
        Stmt::With { target, body, .. } => {
            walk_expr(target, f);
            walk_stmt(body, f);
        }
        Stmt::Return(v) => v.iter().for_each(|e| walk_expr(e, f)),
        Stmt::Assign { target, value, .. } => {
            var(target, f);
            walk_expr(value, f);
        }
        Stmt::IncDec { target, .. } => var(target, f),
        Stmt::Call { name, args } => {
            f(name, true);
            args.iter().for_each(|e| walk_expr(e, f));
        }
        Stmt::Eval(e) => walk_expr(e, f),
        Stmt::Embedded { body, .. } => walk_stmt(body, f),
        Stmt::Break | Stmt::Continue => {}
        // GS2-only statements; GS1 lowering does not build them.
        Stmt::ForEach { .. }
        | Stmt::Switch { .. }
        | Stmt::NewBlock { .. }
        | Stmt::Chain { .. }
        | Stmt::SetPart { .. } => {}
    }
}

fn walk_expr(e: &Expr, f: &mut dyn FnMut(&str, bool)) {
    match e {
        Expr::Var(v) => f(&v.name, false),
        Expr::InArray { name, .. } => f(name, false),
        Expr::Call { name, .. } | Expr::HostCall { name, .. } => f(name, true),
        Expr::Message(parts) => walk_parts(parts, f),
        _ => {}
    }
    for p in e.parts() {
        walk_expr(p, f);
    }
}

fn walk_parts(parts: &[MsgPart], f: &mut dyn FnMut(&str, bool)) {
    for p in parts {
        match p {
            MsgPart::Lit(_) => {}
            MsgPart::Value(e) => walk_expr(e, f),
            MsgPart::Trim(t) => walk_parts(t, f),
            MsgPart::Sub(a, b, t) => {
                walk_expr(a, f);
                walk_expr(b, f);
                walk_parts(t, f);
            }
        }
    }
}

// ---- lowering ----

struct Lower {
    era: Era,
    line: u32,
    errors: Vec<CompileError>,
    /// The script's own function names, lower case.
    script_fns: Vec<String>,
    /// How many GS1 `with` blocks the statement being lowered is in.
    with_depth: u32,
    /// `temp.` variables made so far (see [`Lower::ordered_list`]).
    temps: u32,
    /// Plain names the script sets as player flags, lower case (see
    /// [`player_flags`]).
    flags: Vec<String>,
}

fn bin_op(op: g::BinOp) -> BinOp {
    match op {
        g::BinOp::Add => BinOp::Add,
        g::BinOp::Sub => BinOp::Sub,
        g::BinOp::Mul => BinOp::Mul,
        g::BinOp::Div => BinOp::Div,
        g::BinOp::Mod => BinOp::Mod,
        g::BinOp::Pow => BinOp::Pow,
        g::BinOp::Eq => BinOp::Eq,
        g::BinOp::Ne => BinOp::Ne,
        g::BinOp::Lt => BinOp::Lt,
        g::BinOp::Gt => BinOp::Gt,
        g::BinOp::Le => BinOp::Le,
        g::BinOp::Ge => BinOp::Ge,
        g::BinOp::And => BinOp::And,
        g::BinOp::Or => BinOp::Or,
        g::BinOp::Cat => BinOp::Cat,
        g::BinOp::CatSpc => BinOp::CatSpc,
        g::BinOp::CatNl => BinOp::CatNl,
        g::BinOp::CatTab => BinOp::CatTab,
    }
}

fn assign_op(op: g::AssignOp) -> AssignOp {
    match op {
        g::AssignOp::Set => AssignOp::Set,
        g::AssignOp::Add => AssignOp::Add,
        g::AssignOp::Sub => AssignOp::Sub,
        g::AssignOp::Mul => AssignOp::Mul,
        g::AssignOp::Div => AssignOp::Div,
        g::AssignOp::Mod => AssignOp::Mod,
    }
}

/// A plain script variable: no prefix, and not a built-in or an event.
fn is_script_var(name: &str, era: Era) -> bool {
    !name.contains('.')
        && gs2_name(name) == name
        && vocab::builtin_var(name, era).is_none()
        && !is_event(name, era)
}

/// The GS2 name of a GS1 built-in variable.
///
/// - `timeout` is `this.timeout`, the spelling `timers.gs2` reads and
///   writes (fixture-checked). A bare `timeout` would be looked up on the
///   function's locals first.
/// - `playerx` and `playery` are `player.x` and `player.y`.
/// - The other `player…` numbers are the same word on the `player` object
///   (`playerdir` is `player.dir`). These names are Seihai's reading of the
///   GS2 player object, not fixture-checked.
fn gs2_name(name: &str) -> String {
    match name {
        "timeout" => "this.timeout".into(),
        "playerx" | "playery" | "playerdir" | "playerhearts" | "playerrupees" | "playerbombs"
        | "playerdarts" | "playerswordpower" | "playershieldpower" | "playerfullhearts" => {
            format!("player.{}", &name["player".len()..])
        }
        other => other.into(),
    }
}

/// GS1 built-in variables that are read-only under their GS2 names
/// (`player.hearts`, `player.fullhearts`, `player.weapons.size()`), so a
/// write to them is refused. `selectedweapon` keeps its name in GS2 and
/// may be written: on a real 5.24r4 client `selectedweapon = 1`, `= 0` and
/// `++` each read back as written, with no script error, the same for
/// Seihai's bytecode and gs2-parser's (lab write probe, 2026-11-13).
fn builtin_without_gs2(name: &str) -> bool {
    matches!(name, "playerhp" | "playermaxhp" | "weaponscount")
}

/// The list a GS1 count variable counts: `tokenscount` is
/// `tokens.size()`, as `#t(i)` reads `tokens[i]`; `paramscount` is
/// `params.size()`. `size()` is the method `lists.gs2` uses.
fn counted_list(name: &str) -> Option<&'static str> {
    match name {
        "tokenscount" => Some("tokens"),
        "paramscount" => Some("params"),
        _ => None,
    }
}

/// Plain names a statement sets or clears as player flags: `set name`,
/// `unset name`, `setstring name,value` where the name has no `this.` or
/// `client.` prefix. In GS1 a plain name reads the object's variable and
/// then the flag, so after `set gotkey;` the test `if (gotkey)` reads the
/// flag. GS2 looks a plain name up on the locals, the object, the level and
/// the global object, never the player's flags.
fn player_flags(s: &g::Stmt, out: &mut Vec<String>) {
    match s {
        g::Stmt::Command { name, args, .. }
            if matches!(name.as_str(), "set" | "unset" | "setstring") =>
        {
            if let Some(g::Arg::Text(t)) = args.first() {
                let t = t.trim().to_ascii_lowercase();
                let n = t.split('=').next().unwrap_or("").trim();
                if !n.is_empty() && !n.contains('.') && !n.contains('#') {
                    out.push(n.to_string());
                }
            }
        }
        g::Stmt::Block(v) => v.iter().for_each(|s| player_flags(s, out)),
        g::Stmt::Function { body, .. } => body.iter().for_each(|s| player_flags(s, out)),
        g::Stmt::If { then, els, .. } => {
            player_flags(then, out);
            if let Some(e) = els {
                player_flags(e, out);
            }
        }
        g::Stmt::While { body, .. } | g::Stmt::With { body, .. } => player_flags(body, out),
        g::Stmt::For { body, .. } => player_flags(body, out),
        _ => {}
    }
}

/// The prefix a GS1 player flag written by its plain name gets in GS2.
/// A GS2 client sends only `client.` flags to the server (the real 2.31
/// and 4.04r2, lab 2026-10-31; a 5.24r4 sends nothing for
/// `player.<flag> = ...`, lab 2026-11-13), so `set gotkey;` becomes
/// `set("client.gs1_gotkey")` and seihai-gserver keeps it as `gotkey` and
/// sends it straight back, where `player.gotkey` reads it (seihai-core#30,
/// seihai-gserver#129).
pub const GS1_FLAG_PREFIX: &str = "client.gs1_";

/// A GS1 value whose reading cannot depend on when it is read.
fn is_constant(e: &g::Expr) -> bool {
    match e {
        g::Expr::Num(_) => true,
        g::Expr::Str(s) => !s.contains('#'),
        _ => false,
    }
}

/// The GS2 property a GS1 `setcharprop` code sets on the NPC itself:
/// `#c` chat, `#n` nick, `#1` sword, `#2` shield, `#3` head, `#8` body,
/// `#C0`–`#C4` colors, `#P1`–`#P30` attributes. The GS2 names are Seihai's
/// reading, not fixture-checked.
fn char_prop(code: &str) -> Option<Var> {
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
            return Some(Var {
                name: base.into(),
                index: vec![Expr::Num(f64::from(i))],
            });
        }
    };
    Some(Var::plain(prop))
}

fn no_lowering(code: char) -> String {
    format!("message code `#{code}` has no GS2 lowering")
}

/// One message code as GS1 text holds it, before lowering.
enum Code {
    Value(g::Expr),
    Trim(String),
    Sub(g::Expr, g::Expr, String),
}

impl Lower {
    fn err(&mut self, message: impl Into<String>) {
        self.errors.push(CompileError {
            line: self.line,
            message: message.into(),
        });
    }

    fn stmts(&mut self, body: &[g::Stmt]) -> Vec<Stmt> {
        body.iter().map(|s| self.stmt(s)).collect()
    }

    fn boxed(&mut self, s: &g::Stmt) -> Box<Stmt> {
        Box::new(self.stmt(s))
    }

    fn stmt(&mut self, s: &g::Stmt) -> Stmt {
        match s {
            g::Stmt::Block(v) => Stmt::Block(self.stmts(v)),
            g::Stmt::If { cond, then, els } => Stmt::If {
                cond: self.expr(cond),
                then: self.boxed(then),
                els: els.as_deref().map(|e| self.boxed(e)),
            },
            g::Stmt::While { cond, body } => Stmt::While {
                cond: self.expr(cond),
                body: self.boxed(body),
            },
            g::Stmt::For {
                init,
                cond,
                step,
                body,
            } => Stmt::For {
                init: init.as_deref().map(|s| self.boxed(s)),
                cond: cond.as_ref().map(|c| self.expr(c)),
                step: step.as_deref().map(|s| self.boxed(s)),
                body: self.boxed(body),
            },
            g::Stmt::Function { .. } => {
                self.err("a function inside a block is not lowered");
                Stmt::empty()
            }
            g::Stmt::Return(v) => Stmt::Return(v.as_ref().map(|e| self.expr(e))),
            g::Stmt::Break => Stmt::Break,
            g::Stmt::Continue => Stmt::Continue,
            g::Stmt::Assign {
                target,
                op: g::AssignOp::Set,
                value: g::Expr::List(values),
                line,
            } if target.index.is_empty()
                && !values.iter().any(has_nested_list)
                && list_problem(values).is_some() =>
            {
                self.line = *line;
                self.ordered_list(target, values)
            }
            g::Stmt::Assign {
                target,
                op,
                value,
                line,
            } => {
                self.line = *line;
                self.assign(target, assign_op(*op), value)
            }
            g::Stmt::IncDec {
                target,
                delta,
                line,
            } => {
                self.line = *line;
                if target.index.is_empty() {
                    return Stmt::IncDec {
                        target: self.var(target),
                        delta: *delta,
                    };
                }
                // `a[i]++` is `a[i] = a[i] + 1` (the element read once and
                // written once).
                let op = if *delta < 0.0 {
                    AssignOp::Sub
                } else {
                    AssignOp::Add
                };
                self.assign(target, op, &g::Expr::Num(1.0))
            }
            g::Stmt::Call { name, args, line } => {
                self.line = *line;
                match self.call(name, args) {
                    Expr::Call { name, args } => Stmt::Call { name, args },
                    other => Stmt::Eval(other),
                }
            }
            g::Stmt::Command { name, args, line } => {
                self.line = *line;
                self.command(name, args)
            }
            g::Stmt::With { target, body, line } => {
                self.line = *line;
                self.with(target, body, *line)
            }
            g::Stmt::Unknown {
                name, reason, line, ..
            } => {
                self.line = *line;
                self.err(format!("`{name}` cannot be lowered ({reason})"));
                Stmt::empty()
            }
        }
    }

    /// A read of a GS1 name whose GS2 form differs from a plain variable,
    /// as a real 5.24r4 client reads them (lab probe, 2026-11-13):
    /// `playerhp` is `player.hearts`, `playermaxhp` `player.fullhearts`,
    /// `weaponscount` `player.weapons.size()`, `selectedweapon` the bare
    /// GS2 `selectedweapon`, and a player flag the script sets
    /// (`set gotkey;` then `if (gotkey)`) is `player.gotkey`. Writes to
    /// these are not lowered (see [`Lower::var`]).
    fn read_name(&mut self, v: &g::VarRef) -> Option<Expr> {
        if !v.index.is_empty() {
            return None;
        }
        let lower = v.name.to_ascii_lowercase();
        let e = match lower.as_str() {
            "playerhp" => Expr::var("player.hearts"),
            "playermaxhp" => Expr::var("player.fullhearts"),
            "weaponscount" => Expr::Method {
                obj: Box::new(Expr::var("player.weapons")),
                name: "size".into(),
                args: Vec::new(),
            },
            "selectedweapon" => Expr::var("selectedweapon"),
            _ if self.with_depth == 0
                && is_script_var(&v.name, self.era)
                && self.flags.contains(&lower) =>
            {
                Expr::var(format!("player.{}", v.name))
            }
            _ => return None,
        };
        Some(e)
    }

    /// `players[i].x`, `.y`, `.dir` and `npcs[i].x`, `.y`, `.dir` read:
    /// the element, then the member, with no conversion between
    /// (`members.gs2bc`). Other fields after an index stay errors.
    fn element_member(&mut self, v: &g::VarRef) -> Option<Expr> {
        if !v.field_after_index || v.index.len() != 1 {
            return None;
        }
        let (base, field) = v.name.split_once('.')?;
        let field = field.to_ascii_lowercase();
        if !matches!(base, "players" | "npcs")
            || !matches!(field.as_str(), "x" | "y" | "dir")
            || self.with_depth > 0
        {
            return None;
        }
        Some(Expr::Member {
            obj: Box::new(Expr::Var(Var {
                name: base.to_string(),
                index: v.index.iter().map(|e| self.expr(e)).collect(),
            })),
            name: field,
        })
    }

    /// A variable as GS2 names it. An element must have no field after its
    /// index (`npcs[i].x`), and GS1's level board is not lowered.
    fn var(&mut self, v: &g::VarRef) -> Var {
        if v.index.is_empty()
            && is_script_var(&v.name, self.era)
            && self.flags.contains(&v.name.to_ascii_lowercase())
        {
            self.err(format!(
                "`{}` is a player flag the script sets; assigning to it by its plain name, or \
                 reading it inside `with`, has no client-checked GS2 form (a 5.x client sends \
                 no flag for `player.{}` = ...); use `set` or `setstring`",
                v.name, v.name
            ));
        }
        if v.index.is_empty() && builtin_without_gs2(&v.name) {
            self.err(format!(
                "writing the GS1 variable `{}` has no client-checked GS2 form; it is not lowered",
                v.name
            ));
        } else if v.index.is_empty() && counted_list(&v.name).is_some() {
            self.err(format!("`{}` cannot be written", v.name));
        }
        if !v.index.is_empty() {
            if v.field_after_index {
                self.err(format!("a field after an index (`{v}`) is not lowered"));
            } else if v.name == "board" {
                self.err(
                    "`board[..]` has no GS2 form (GS2 has `tiles[x, y]`); it is not lowered"
                        .to_string(),
                );
            } else if v.name == "tiles" && v.index.len() != 2 {
                self.err("`tiles` takes two indexes, `tiles[x, y]`".to_string());
            }
        }
        Var {
            name: self.name(&v.name),
            index: v.index.iter().map(|e| self.expr(e)).collect(),
        }
    }

    /// A variable name as GS2 names it. Inside a GS1 `with`, a plain
    /// script variable stays the NPC's own: GS2 reaches it as `thiso.`.
    fn name(&self, name: &str) -> String {
        if self.with_depth == 1 && is_script_var(name, self.era) {
            format!("thiso.{name}")
        } else {
            gs2_name(name)
        }
    }

    /// `target op= value`. On an element, a compound assignment is
    /// `a[i] = a[i] op v`: the GS1 interpreter reads the element, works the
    /// number out and stores it. The index is pushed twice, so it must not
    /// call anything.
    fn assign(&mut self, target: &g::VarRef, op: AssignOp, value: &g::Expr) -> Stmt {
        if target.index.is_empty() {
            return Stmt::Assign {
                target: self.var(target),
                op,
                value: self.expr(value),
            };
        }
        if op != AssignOp::Set {
            if target.index.iter().any(|e| expr_calls(e, true)) {
                self.err(format!(
                    "`{}` on an array element whose index calls a function is not lowered; \
                     put the index in a variable first",
                    op.symbol()
                ));
                return Stmt::empty();
            }
            let target = self.var(target);
            let value = self.expr(value);
            return Stmt::Assign {
                value: op.expand(Expr::Var(target.clone()), value),
                target,
                op: AssignOp::Set,
            };
        }
        // `a[i] = {1, 2}` is `OP_ARRAY_ASSIGN` with the literal as the
        // value (`elemlist.gs2bc`); the literal goes through `expr`.
        Stmt::Assign {
            target: self.var(target),
            op,
            value: self.expr(value),
        }
    }

    /// `name = {a, f(), b}` where the order the elements are read in could
    /// matter. GS2 pushes a literal's elements right to left, and the GS1
    /// interpreter reads them left to right, so each element that is not a
    /// constant is first stored in a `temp.` variable, in source order,
    /// and the literal is built from those. The target has no index, so
    /// nothing else in the statement runs between the elements.
    fn ordered_list(&mut self, target: &g::VarRef, values: &[g::Expr]) -> Stmt {
        let mut body = Vec::new();
        let mut items = Vec::with_capacity(values.len());
        for v in values {
            if is_constant(v) {
                items.push(self.expr(v));
                continue;
            }
            let temp = format!("temp.__gs1list{}", self.temps);
            self.temps += 1;
            body.push(Stmt::Assign {
                target: Var::plain(temp.clone()),
                op: AssignOp::Set,
                value: self.expr(v),
            });
            items.push(Expr::var(temp));
        }
        body.push(Stmt::Assign {
            target: self.var(target),
            op: AssignOp::Set,
            value: Expr::List(items),
        });
        Stmt::Block(body)
    }

    /// `with (target) body`. The NPC server's notes (`npcserverdoc.txt`, NW
    /// beta 1 and 2) say that inside `with` the target's `this.` variables
    /// and properties are read and written, plain script variables (`i`,
    /// `j`) keep working, and script functions still run as the original
    /// NPC. GS2 resolves a plain name on the `with` object first and runs a
    /// bare call against it, so a plain script variable is lowered as
    /// `thiso.name` (the object the `with` was entered from), and a body
    /// whose meaning would still change is refused: a plain write inside
    /// nested `with` blocks, the `player`, `client.` and `level.` names
    /// (GS1 switches them with the object, GS2 does not), a call to one of
    /// the script's own functions, and a command lowered to an explicit
    /// `player`. Other commands act on the `with` object in both.
    fn with(&mut self, target: &g::Expr, body: &g::Stmt, line: u32) -> Stmt {
        if with_escape(body, false) {
            self.err(
                "`return`, `sleep`, `waitfor`, or a `break`/`continue` that leaves a `with` block, is not lowered",
            );
            return Stmt::empty();
        }
        let mut problem = None;
        let era = self.era;
        let nested = self.with_depth > 0;
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
            } else if nested && write && is_script_var(name, era) {
                problem = Some(format!(
                    "writing the script variable `{name}` inside nested GS1 `with` blocks is not lowered"
                ));
            }
        });
        if let Some(problem) = problem {
            self.err(problem);
            return Stmt::empty();
        }
        let target = self.expr(target);
        self.with_depth += 1;
        let body = self.boxed(body);
        self.with_depth -= 1;
        let fns = std::mem::take(&mut self.script_fns);
        let (calls_own, player) = lowered_reaches(&body, &fns);
        self.script_fns = fns;
        if calls_own {
            self.err(
                "a call to one of the script's own functions inside a GS1 `with` is not lowered \
                 (GS1 runs it as the original NPC, GS2 looks it up on the `with` object first)",
            );
        }
        if player {
            self.err(
                "a GS1 command on the player inside `with` is not lowered \
                 (GS1 swaps the player there, GS2's `player` stays the same)",
            );
        }
        Stmt::With { target, body, line }
    }

    fn command(&mut self, name: &str, args: &[g::Arg]) -> Stmt {
        if let Some(n) = builtin_arity(name) {
            if args.len() != n {
                self.err(format!(
                    "`{name}` expects {n} argument(s), got {}",
                    args.len()
                ));
                return Stmt::empty();
            }
        }
        match name {
            // The variable is named as text.
            "setarray" => {
                let Some(g::Arg::Text(var)) = args.first() else {
                    self.err("`setarray` needs a variable name");
                    return Stmt::empty();
                };
                if !is_plain_name(var) {
                    self.err("`setarray` needs a plain variable name");
                    return Stmt::empty();
                }
                let size = match args.get(1) {
                    Some(g::Arg::Num(n)) => self.expr(n),
                    _ => Expr::Num(0.0),
                };
                return Stmt::Call {
                    name: name.into(),
                    args: vec![Expr::var(self.name(var)), size],
                };
            }
            "sleep" => {
                let time = match args.first() {
                    Some(g::Arg::Num(e)) => self.expr(e),
                    _ => Expr::Num(0.0),
                };
                return Stmt::Call {
                    name: name.into(),
                    args: vec![time],
                };
            }
            _ => {}
        }
        if let Some(s) = self.string_command(name, args) {
            return s;
        }
        let name = renamed_command(name).unwrap_or(name);
        if !gs2_has_command(name) {
            self.err(format!(
                "GS1 command `{name}` has no GS2 equivalent{}",
                command_hint(name)
            ));
            return Stmt::empty();
        }
        // The first word of `setplayerprop` names the property (`#c`); it
        // is not a message to expand.
        let args = args
            .iter()
            .enumerate()
            .map(|(i, a)| match a {
                g::Arg::Text(t) if i == 0 && name == "setplayerprop" => Expr::Str(t.clone()),
                a => self.arg(a),
            })
            .collect();
        Stmt::Eval(Expr::HostCall {
            name: name.into(),
            args,
        })
    }

    fn arg(&mut self, a: &g::Arg) -> Expr {
        match a {
            g::Arg::Num(e) => self.expr(e),
            g::Arg::Text(t) => self.text(t),
        }
    }

    /// GS1 commands that become GS2 statements rather than a call by name.
    ///
    /// - `setstring name,value`: `this.` and `client.` names become an
    ///   assignment. Any other name is a player flag in GS1, so it becomes
    ///   `set("name=value")`, or `unset("name")` for a literal empty value.
    /// - `addstring`, `insertstring`, `replacestring`, `removestring`,
    ///   `deletestring` (2.1 lists) on `this.`/`client.` names: the GS2 list
    ///   methods. `addstring` is first in NewWorld beta 2 and 2.1.
    /// - `tokenize text` / `tokenize2 text,seps`: `tokens = text.tokenize()`,
    ///   which `#t(i)` then reads as `tokens[i]`.
    /// - `setcharprop code,value`: the NPC's own property (see
    ///   [`char_prop`]) as `this.<prop> = value`.
    fn string_command(&mut self, name: &str, args: &[g::Arg]) -> Option<Stmt> {
        let text_name = |a: Option<&g::Arg>| match a {
            Some(g::Arg::Text(t)) => {
                let t = t.trim().to_ascii_lowercase();
                is_plain_name(&t).then_some(t)
            }
            _ => None,
        };
        let raw_value = |a: Option<&g::Arg>| match a {
            Some(g::Arg::Num(e)) => e.clone(),
            Some(g::Arg::Text(t)) => g::Expr::Str(t.clone()),
            None => g::Expr::Str(String::new()),
        };
        let local = |n: &str| n.starts_with("this.") || n.starts_with("client.");
        Some(match name {
            "setstring" => {
                let Some(var) = text_name(args.first()) else {
                    self.err("`setstring` needs a plain string name");
                    return Some(Stmt::empty());
                };
                let v = raw_value(args.get(1));
                if local(&var) {
                    Stmt::Assign {
                        target: Var::plain(self.name(&var)),
                        op: AssignOp::Set,
                        value: self.expr(&v),
                    }
                } else {
                    let Some(flag) = self.sent_flag(name, &var) else {
                        return Some(Stmt::empty());
                    };
                    Stmt::Assign {
                        target: Var::plain(flag),
                        op: AssignOp::Set,
                        value: self.expr(&v),
                    }
                }
            }
            // `set x;` is `client.gs1_x = true;` and `unset x;` is
            // `client.gs1_x = "";`: a real 5.24r4 and 4.20r8 have no GS2
            // `set()`/`unset()` ("Function set not found", nothing sent), and
            // send PLI 18 or 19 for an assignment to a `client.` variable
            // (lab, 2026-11-14, #30).
            "set" | "unset" => {
                let Some(g::Arg::Text(t)) = args.first() else {
                    return None;
                };
                // `set door=open;` carries a value, as `setstring` does.
                let (t, value) = match t.trim().split_once('=') {
                    Some((n, v)) if name == "set" => (n.trim(), Some(v.trim())),
                    _ => (t.trim(), None),
                };
                let Some(flag) = self.sent_flag(name, t) else {
                    return Some(Stmt::empty());
                };
                Stmt::Assign {
                    target: Var::plain(flag),
                    op: AssignOp::Set,
                    value: match value {
                        Some(v) => self.text(v),
                        None if name == "set" => Expr::var("true"),
                        None => Expr::Str(String::new()),
                    },
                }
            }
            "addstring" | "insertstring" | "replacestring" | "removestring" | "deletestring" => {
                let Some(var) = text_name(args.first()) else {
                    self.err(format!("`{name}` needs a plain string name"));
                    return Some(Stmt::empty());
                };
                if !local(&var) {
                    self.err(format!(
                        "`{name}` on `{var}`: only `this.` and `client.` lists are lowered (a plain GS1 string is a player flag)"
                    ));
                    return Some(Stmt::empty());
                }
                let list = Expr::var(self.name(&var));
                let method = match name {
                    "addstring" => "add",
                    "insertstring" => "insert",
                    "replacestring" => "replace",
                    "removestring" => "remove",
                    _ => "delete",
                };
                Stmt::Eval(Expr::Method {
                    obj: Box::new(list),
                    name: method.into(),
                    args: args.iter().skip(1).map(|a| self.arg(a)).collect(),
                })
            }
            "tokenize" | "tokenize2" => {
                let text = self.expr(&raw_value(args.first()));
                let seps = if name == "tokenize2" {
                    let extra = self.expr(&raw_value(args.get(1)));
                    vec![Expr::bin(BinOp::Cat, Expr::Str(" ,".into()), extra)]
                } else {
                    Vec::new()
                };
                Stmt::Assign {
                    target: Var::plain("tokens"),
                    op: AssignOp::Set,
                    value: Expr::Method {
                        obj: Box::new(text),
                        name: "tokenize".into(),
                        args: seps,
                    },
                }
            }
            "setcharprop" => {
                let code = match args.first() {
                    Some(g::Arg::Text(t)) => t.trim().to_string(),
                    _ => String::new(),
                };
                let Some(target) = char_prop(&code) else {
                    self.err(format!("`setcharprop {code}` has no GS2 property here"));
                    return Some(Stmt::empty());
                };
                let value = raw_value(args.get(1));
                Stmt::Assign {
                    target,
                    op: AssignOp::Set,
                    value: self.expr(&value),
                }
            }
            _ => return None,
        })
    }

    /// A call. Built-ins keep their names (code generation knows them);
    /// GS1-only built-ins are lowered here.
    fn call(&mut self, name: &str, args: &[g::Expr]) -> Expr {
        // `this.join("c")`, `obj.f(x)`: a method call on the object, as
        // GS2 encodes it, not a call of a function named `this.join`.
        if let Some((obj, method)) = name.rsplit_once('.') {
            if !obj.is_empty() && !method.is_empty() {
                let obj = Expr::Var(Var {
                    name: self.name(obj),
                    index: Vec::new(),
                });
                return Expr::Method {
                    obj: Box::new(obj),
                    name: method.into(),
                    args: args.iter().map(|a| self.expr(a)).collect(),
                };
            }
        }
        let gs1_arity = match name {
            "strlen" => Some(1),
            "startswith" | "indexof" => Some(2),
            _ => builtin_arity(name),
        };
        if let Some(n) = gs1_arity {
            if args.len() != n {
                self.err(format!(
                    "`{name}` expects {n} argument(s), got {}",
                    args.len()
                ));
                return Expr::Num(0.0);
            }
        }
        let lowered: Vec<Expr> = args.iter().map(|a| self.expr(a)).collect();
        let gs1_fn = match (name, args.len()) {
            ("strlen", _) => Some(Gs1Fn::StrLen),
            ("startswith", _) => Some(Gs1Fn::StartsWith),
            ("indexof", _) => Some(Gs1Fn::IndexOf),
            ("strtofloat", 1) => Some(Gs1Fn::StrToFloat),
            ("strcontains", 2) => Some(Gs1Fn::StrContains),
            _ => None,
        };
        if let Some(f) = gs1_fn {
            return Expr::Gs1Fn(f, lowered);
        }
        // GS1 `getplayer(account)` (2.17.1 on, used as `with
        // (getplayer(a))`) is the player with that account; the 3.153
        // client registers `findplayer` for the same lookup.
        if name == "getplayer" && args.len() == 1 {
            return Expr::HostCall {
                name: "findplayer".into(),
                args: lowered,
            };
        }
        if gs1_fn_without_gs2(name) {
            self.err(format!("GS1 function `{name}` has no GS2 equivalent"));
            return Expr::Num(0.0);
        }
        if name == "format" && args.is_empty() {
            self.err("`format` expects at least 1 argument, got 0");
            return Expr::Num(0.0);
        }
        if opcode_builtin(name).is_some() {
            Expr::Call {
                name: name.into(),
                args: lowered,
            }
        } else {
            // A GS1 script may have a function named `_`; GS2 would read it
            // as the translation built-in.
            Expr::HostCall {
                name: name.into(),
                args: lowered,
            }
        }
    }

    /// The `client.` variable a GS1 `set`, `unset` or `setstring` on
    /// `flag` writes: a `client.` or `this.` name as written, any other
    /// plain name (dots allowed) with [`GS1_FLAG_PREFIX`]. A name made at
    /// run time and the `local.`, `clientr.`, `server.` and `serverr.`
    /// prefixes, which a client script cannot send, are errors.
    fn sent_flag(&mut self, cmd: &str, flag: &str) -> Option<String> {
        let lower = flag.to_ascii_lowercase();
        if lower.starts_with("client.") || lower.starts_with("this.") {
            return Some(self.name(flag));
        }
        if flag.is_empty() || flag.contains('#') || !is_plain_name(&lower) {
            self.err(format!(
                "`{cmd} {flag}`: only a plain flag name is lowered (GS2 has no `set()`)"
            ));
            return None;
        }
        if ["local.", "clientr.", "server.", "serverr."]
            .iter()
            .any(|p| lower.starts_with(p))
        {
            self.err(format!(
                "`{cmd} {flag}`: a GS2 client script cannot send this flag; it is not lowered"
            ));
            return None;
        }
        Some(format!("{GS1_FLAG_PREFIX}{flag}"))
    }

    fn text(&mut self, s: &str) -> Expr {
        if !s.contains('#') {
            return Expr::Str(s.into());
        }
        match self.message(s) {
            Ok(parts) => Expr::Message(parts),
            Err(e) => {
                self.err(e);
                Expr::Str(s.into())
            }
        }
    }

    fn expr(&mut self, e: &g::Expr) -> Expr {
        match e {
            g::Expr::Num(n) => Expr::Num(*n),
            g::Expr::Str(s) => self.text(s),
            g::Expr::Var(v) => {
                if v.index.is_empty() && !v.name.contains('.') && is_event(&v.name, self.era) {
                    self.err(format!(
                        "`{}` is an event flag and has no GS2 value; the handler is {}",
                        v.name,
                        gs2_event_function(&v.name)
                    ));
                    return Expr::Num(0.0);
                }
                if let Some(list) = counted_list(&v.name).filter(|_| v.index.is_empty()) {
                    return Expr::Method {
                        obj: Box::new(Expr::var(list)),
                        name: "size".into(),
                        args: Vec::new(),
                    };
                }
                if let Some(e) = self.read_name(v) {
                    return e;
                }
                if let Some(e) = self.element_member(v) {
                    return e;
                }
                Expr::Var(self.var(v))
            }
            g::Expr::Unary(g::UnOp::Neg, a) => Expr::Neg(Box::new(self.expr(a))),
            g::Expr::Unary(g::UnOp::Not, a) => Expr::Not(Box::new(self.expr(a))),
            g::Expr::Bin(op, a, b) => Expr::bin(bin_op(*op), self.expr(a), self.expr(b)),
            g::Expr::Call { name, args } => self.call(name, args),
            g::Expr::InRange { e, lo, hi } => Expr::InRange {
                e: Box::new(self.expr(e)),
                lo: Box::new(self.expr(lo)),
                hi: Box::new(self.expr(hi)),
            },
            g::Expr::Cond { cond, then, els } => Expr::Cond {
                cond: Box::new(self.expr(cond)),
                then: Box::new(self.expr(then)),
                els: Box::new(self.expr(els)),
            },
            g::Expr::InList { e, list } => {
                if expr_calls(e, true) {
                    self.err(
                        "`x in {list}` where x calls a function is not lowered; \
                         put x in a variable first",
                    );
                    return Expr::Num(0.0);
                }
                if list.is_empty() {
                    return Expr::Num(0.0);
                }
                Expr::AnyOf {
                    e: Box::new(self.expr(e)),
                    list: list.iter().map(|x| self.expr(x)).collect(),
                }
            }
            g::Expr::InArray { e, name } => Expr::InArray {
                e: Box::new(self.expr(e)),
                name: self.name(name),
            },
            g::Expr::List(values) => {
                if let Some(problem) = list_problem(values) {
                    self.err(problem);
                    return Expr::Num(0.0);
                }
                Expr::List(values.iter().map(|v| self.expr(v)).collect())
            }
        }
    }

    /// Splits GS1 text at its message codes, with the bounds and era gates
    /// of Seihai's GS1 engine (`engine::interp`, `expand`). Codes GS2 has a
    /// value for are lowered; the others are errors. A `#` that starts no
    /// code stays text, as in GS1.
    fn message(&mut self, s: &str) -> Result<Vec<MsgPart>, String> {
        let codes = split_codes(s, self.era)?;
        let mut parts = Vec::with_capacity(codes.len());
        for c in codes {
            parts.push(match c {
                Ok(lit) => MsgPart::Lit(lit),
                Err(Code::Value(e)) => MsgPart::Value(self.expr(&e)),
                Err(Code::Trim(t)) => MsgPart::Trim(self.message(&t)?),
                Err(Code::Sub(start, len, t)) => {
                    let start = self.expr(&start);
                    let len = self.expr(&len);
                    MsgPart::Sub(start, len, self.message(&t)?)
                }
            });
        }
        Ok(parts)
    }
}

/// Why a GS1 array literal cannot be pushed in GS2's right-to-left order,
/// or `None` when it can.
fn list_problem(values: &[g::Expr]) -> Option<&'static str> {
    // Lists inside a literal are pushed right to left at every level
    // (`nested.gs2bc`), so they are lowered when nothing in them can see
    // the order: no element anywhere calls script or host code or `random`.
    if values.iter().any(has_nested_list) {
        let effects = values
            .iter()
            .any(|v| expr_calls(v, true) || mentions_call(v, "random"));
        return effects.then_some(
            "GS1 array elements that are lists are lowered only when no element calls \
             anything; put the values in variables first",
        );
    }
    let effects = values
        .iter()
        .filter(|v| expr_calls(v, true) || mentions_call(v, "random"))
        .count();
    let constants = values
        .iter()
        .filter(|v| matches!(v, g::Expr::Num(_) | g::Expr::Str(_)))
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

/// GS1 text split at its codes: `Ok` for literal text, `Err` for a code.
fn split_codes(s: &str, era: Era) -> Result<Vec<Result<String, Code>>, String> {
    let b = s.as_bytes();
    let mut parts = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    let num = |x: &str| -> g::Expr {
        let x = x.trim();
        if x.contains('#') {
            g::Expr::Str(x.to_string())
        } else {
            engine::parser::parse_expr(x, era).unwrap_or(g::Expr::Num(0.0))
        }
    };
    let var = |n: &str, idx: Option<g::Expr>| {
        Code::Value(g::Expr::Var(g::VarRef {
            name: n.into(),
            index: idx.into_iter().collect(),
            field_after_index: false,
        }))
    };
    while i < b.len() {
        if b[i] != b'#' || i + 1 >= b.len() {
            let ch = s[i..].chars().next().unwrap();
            lit.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let code = b[i + 1] as char;
        let digits = engine::lexer::code_digits(&b[i + 1..]);
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
        let part = match (code, inner) {
            ('c', None) if digits == 0 => Some(var("player.chat", None)),
            ('n', None) => Some(var("player.nick", None)),
            ('a', None) => Some(var("player.account", None)),
            ('v', Some(x)) => Some(Code::Value(num(x))),
            ('s', Some(x)) => {
                let n = x.trim().to_ascii_lowercase();
                let readable = ["this.", "client.", "clientr.", "serverr."];
                if readable.iter().any(|p| n.starts_with(p)) && is_plain_name(&n) {
                    Some(var(&n, None))
                } else {
                    return Err(format!(
                        "`#s({x})`: only `this.`, `client.`, `clientr.` and `serverr.` strings are lowered (a plain GS1 string is a player flag)"
                    ));
                }
            }
            ('t', Some(x)) if era >= Era::V202 => Some(var("tokens", Some(num(x)))),
            ('p', Some(x)) if era >= Era::V203 => Some(var("params", Some(num(x)))),
            ('T', Some(x)) if era >= Era::V202 => Some(Code::Trim(x.to_string())),
            ('e', Some(x)) if era >= Era::V202 => {
                let p: Vec<&str> = x.splitn(3, ',').collect();
                if p.len() != 3 {
                    return Err(format!("`#e({x})` needs start, length and text"));
                }
                Some(Code::Sub(num(p[0]), num(p[1]), p[2].to_string()))
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
                    parts.push(Ok(std::mem::take(&mut lit)));
                }
                parts.push(Err(p));
                i = end;
            }
            None => {
                lit.push('#');
                i += 1;
            }
        }
    }
    if !lit.is_empty() || parts.is_empty() {
        parts.push(Ok(lit));
    }
    Ok(parts)
}
