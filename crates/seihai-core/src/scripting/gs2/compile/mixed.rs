//! Scripts that mix the two styles of event code: GS1 event tests
//! (`if (created) …`) and GS2 event functions (`function onCreated() …`)
//! in one NPC.
//!
//! Level NPCs saved by the 2.x/3.x-era editors often hold both: the 2019
//! Delteria backup has 300 such NPCs in 100 levels, 292 of them with
//! `if (created)` next to `function onCreated()`, and none with a `//#GS2`
//! marker. The common shape is an editor stub:
//!
//! ```text
//! if (created) dontblock;
//! function onCreated() {
//!   y = y + .5;
//!   dontblock();
//! }
//! ```
//!
//! Seihai's reading, which no original client run has confirmed yet: both
//! parts are event code for the same event, and both run. The GS1 part
//! runs first (GS1 tests ran as the event began), then the GS2 function.
//! That holds whichever front end reads the script:
//!
//! - When the script is unmarked
//!   ([`Language::Detect`](super::Language::Detect)) and read as GS2, a
//!   top-level `if (<event>)` is lifted into the event's function, as the
//!   GS1 front end does; read as plain GS2 it would test a variable
//!   that is never set, once, at load ([`lift_event_tests`],
//!   [`Quirk::Gs1EventTest`]). A script marked GS2 is left as gs2-parser
//!   reads it (issue 32, `tests/gs1_hud_semantics.rs`).
//! - Both front ends then fold a lifted event function into the script's
//!   own function for that event ([`merge_event_functions`]). Without this,
//!   the script would hold two functions for one event, and only the first
//!   the client finds would run.

use super::ast::*;
use super::{Quirk, Warning};
use crate::scripting::engine::vocab;
use std::collections::HashMap;

/// Lift each top-level `if (<event>) …` of a GS2 script into the event's
/// function. Handles the forms the GS1 front end lifts: one event, events
/// joined by `||`, and `event && guard`. Blocks for the same event join in
/// source order. A test with `else`, or with an event inside a larger
/// condition, is left where it is.
///
/// Returns the item indexes of the functions made, and a warning for each
/// test lifted.
pub(super) fn lift_event_tests(program: &mut Program) -> (Vec<usize>, Vec<Warning>) {
    let mut lifted: Vec<Function> = Vec::new();
    let mut warnings = Vec::new();
    let mut items = Vec::new();
    for item in std::mem::take(&mut program.items) {
        let Item::Code(stmts) = item else {
            items.push(item);
            continue;
        };
        let mut kept = Vec::new();
        for stmt in stmts {
            let Stmt::If {
                cond,
                then,
                els: None,
            } = &stmt
            else {
                kept.push(stmt);
                continue;
            };
            let Some(arms) = event_arms(cond) else {
                kept.push(stmt);
                continue;
            };
            // GS1 runs every true block top to bottom; a block that can
            // return would skip the ones after it, so a later block for the
            // same event stays where it is.
            let blocked = arms.iter().any(|(event, _)| {
                let name = super::gs2_event_function(event);
                lifted
                    .iter()
                    .any(|f| f.name == name && f.body.iter().any(can_return))
            });
            if blocked {
                kept.push(stmt);
                continue;
            }
            let base = match &**then {
                Stmt::Block(v) => v.clone(),
                other => vec![other.clone()],
            };
            for (event, guard) in arms {
                warnings.push(Warning {
                    line: 0,
                    quirk: Quirk::Gs1EventTest(event.clone()),
                });
                let body = match guard {
                    Some(guard) => vec![Stmt::If {
                        cond: guard,
                        then: Box::new(Stmt::Block(base.clone())),
                        els: None,
                    }],
                    None => base.clone(),
                };
                let name = super::gs2_event_function(&event);
                match lifted.iter_mut().find(|f| f.name == name) {
                    Some(f) => f.body.extend(body),
                    None => lifted.push(Function {
                        name,
                        params: Vec::new(),
                        body,
                        cmd_call: None,
                    }),
                }
            }
        }
        if !kept.is_empty() {
            items.push(Item::Code(kept));
        }
    }
    let made = (0..lifted.len()).collect();
    // The lifted functions go first: their order in the bytecode does not
    // change what runs.
    program.items = lifted
        .into_iter()
        .map(Item::Function)
        .chain(items)
        .collect();
    (made, warnings)
}

/// `event`, `a || b || …` of events, or `event && guard`: the events, each
/// with its guard. `None` when the condition is not an event test.
fn event_arms(e: &Expr) -> Option<Vec<(String, Option<Expr>)>> {
    match e {
        Expr::Var(v) => event(v).map(|ev| vec![(ev, None)]),
        Expr::Binary(BinOp::Or, a, b) => {
            let mut l = event_arms(a)?;
            let r = event_arms(b)?;
            if l.iter().chain(&r).any(|(_, g)| g.is_some()) {
                return None;
            }
            l.extend(r);
            Some(l)
        }
        Expr::Binary(BinOp::And, a, b) => match &**a {
            Expr::Var(v) => event(v).map(|ev| vec![(ev, Some((**b).clone()))]),
            _ => None,
        },
        _ => None,
    }
}

/// The GS1 event a plain variable names.
fn event(v: &Var) -> Option<String> {
    if !v.index.is_empty() || v.name.contains('.') {
        return None;
    }
    let lower = v.name.to_ascii_lowercase();
    vocab::event_since(&lower).map(|_| lower)
}

/// The function name without a `public.` prefix.
fn bare(name: &str) -> &str {
    name.strip_prefix("public.").unwrap_or(name)
}

/// Fold each lifted event function (the items at the indexes in `lifted`)
/// into the script's
/// own function for that event, matched without regard to case or a
/// `public.` prefix (the client matches handlers that way). The lifted
/// code goes first. When it can `return`, which would skip the function's
/// own code, it stays a function of its own under a helper name and the
/// handler calls it first. With no own function for the event, the lifted
/// one stays as it is. When a name is defined twice, the later definition
/// (the one legacy mode keeps) takes the lifted code.
pub(super) fn merge_event_functions(program: &mut Program, lifted: &[usize]) {
    // For each lifted function, the index of the own function it folds
    // into.
    let mut target = HashMap::new();
    for (i, item) in program.items.iter().enumerate() {
        let Item::Function(l) = item else { continue };
        if !lifted.contains(&i) {
            continue;
        }
        let own = program
            .items
            .iter()
            .enumerate()
            .rev()
            .find_map(|(j, it)| match it {
                Item::Function(f)
                    if !lifted.contains(&j)
                        && bare(&f.name).eq_ignore_ascii_case(bare(&l.name)) =>
                {
                    Some(j)
                }
                _ => None,
            });
        if let Some(j) = own {
            target.insert(i, j);
        }
    }
    if target.is_empty() {
        return;
    }
    let mut prefix: HashMap<usize, Vec<Stmt>> = HashMap::new();
    let mut calls: HashMap<usize, bool> = HashMap::new();
    let mut helpers = Vec::new();
    for (&i, &j) in &target {
        let Item::Function(l) = &program.items[i] else {
            unreachable!("lifted items are functions")
        };
        let body = if l.body.iter().any(can_return) {
            let name = format!("__gs1{}", bare(&l.name));
            helpers.push((
                i,
                Function {
                    name: name.clone(),
                    params: Vec::new(),
                    body: l.body.clone(),
                    cmd_call: l.cmd_call,
                },
            ));
            calls.insert(j, true);
            vec![Stmt::Call {
                name,
                args: Vec::new(),
            }]
        } else {
            if let Some(c) = l.cmd_call {
                *calls.entry(j).or_default() |= c;
            }
            l.body.clone()
        };
        prefix.entry(j).or_default().extend(body);
    }
    let mut items = Vec::new();
    for (k, item) in std::mem::take(&mut program.items).into_iter().enumerate() {
        if target.contains_key(&k) {
            if let Some((_, h)) = helpers.iter().find(|(i, _)| *i == k) {
                items.push(Item::Function(h.clone()));
            }
            continue;
        }
        match item {
            Item::Function(mut f) if prefix.contains_key(&k) => {
                let mut body = prefix.remove(&k).unwrap_or_default();
                // A lowered GS1 function knows whether it calls; a GS2 one
                // (`None`) is decided from the joined body at codegen.
                if let (Some(own), Some(&lifted)) = (f.cmd_call, calls.get(&k)) {
                    f.cmd_call = Some(own || lifted);
                }
                body.append(&mut f.body);
                f.body = body;
                items.push(Item::Function(f));
            }
            other => items.push(other),
        }
    }
    program.items = items;
}

/// Whether a statement can `return` (so code after it in the same body
/// may not run).
fn can_return(s: &Stmt) -> bool {
    match s {
        Stmt::Return(_) => true,
        Stmt::Block(v) => v.iter().any(can_return),
        Stmt::NewBlock { body, .. } => body.iter().any(can_return),
        Stmt::If { then, els, .. } => can_return(then) || els.as_deref().is_some_and(can_return),
        Stmt::While { body, .. }
        | Stmt::For { body, .. }
        | Stmt::ForEach { body, .. }
        | Stmt::With { body, .. }
        | Stmt::Embedded { body, .. }
        | Stmt::Labeled { body, .. } => can_return(body),
        Stmt::Switch { arms, .. } => arms.iter().flat_map(|a| &a.body).any(can_return),
        _ => false,
    }
}
