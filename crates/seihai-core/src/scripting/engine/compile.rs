//! Flattens a parsed script into a node list.
//!
//! The original client runs a flat list of statement nodes with jump
//! targets, and every event runs it from the top. Doing the same keeps
//! `sleep` simple: the script remembers the node after the `sleep` and a
//! later `timeout` event resumes there.

use super::ast::*;
use super::vocab::{self, Cmd};
use std::collections::HashMap;

/// A flat node. Jump targets are indexes into [`Compiled::nodes`].
#[derive(Debug, Clone)]
pub enum Node {
    Cmd { cmd: &'static Cmd, args: Vec<Arg>, line: u32 },
    Unknown { name: String, raw: String, reason: &'static str, line: u32 },
    Assign { target: VarRef, op: AssignOp, value: Expr },
    IncDec { target: VarRef, delta: f64 },
    /// A call used as a statement.
    Eval(Expr),
    JumpIfFalse(Expr, usize),
    Jump(usize),
    /// Start of a loop: resets its pass counter.
    LoopEnter(usize),
    /// End of a pass: counts it and jumps back, or ends the run past the
    /// pass limit.
    LoopBack { to: usize, slot: usize },
    Return(Option<Expr>),
    /// Ends the run: end of the script, or `break` / `continue` outside a loop.
    End,
}

/// A script-defined function.
#[derive(Debug, Clone)]
pub struct Func {
    pub entry: usize,
    pub params: Vec<String>,
}

/// A script ready to run.
#[derive(Debug, Clone)]
pub struct Compiled {
    pub nodes: Vec<Node>,
    pub funcs: HashMap<String, Func>,
    pub loops: usize,
    pub diagnostics: Vec<Diag>,
}

const HOLE: usize = usize::MAX;

#[derive(Default)]
struct LoopCtx {
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

struct Compiler {
    nodes: Vec<Node>,
    funcs: HashMap<String, Func>,
    loops: usize,
    stack: Vec<LoopCtx>,
}

pub fn compile(script: &Script) -> Compiled {
    let mut c = Compiler { nodes: Vec::new(), funcs: HashMap::new(), loops: 0, stack: Vec::new() };
    for s in &script.body {
        c.stmt(s);
    }
    c.nodes.push(Node::End);
    Compiled { nodes: c.nodes, funcs: c.funcs, loops: c.loops, diagnostics: script.diagnostics.clone() }
}

impl Compiler {
    fn here(&self) -> usize {
        self.nodes.len()
    }

    fn emit(&mut self, n: Node) -> usize {
        self.nodes.push(n);
        self.nodes.len() - 1
    }

    fn patch(&mut self, at: usize, to: usize) {
        match &mut self.nodes[at] {
            Node::Jump(t) | Node::JumpIfFalse(_, t) => *t = to,
            _ => {}
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Block(v) => v.iter().for_each(|s| self.stmt(s)),
            Stmt::If { cond, then, els } => {
                let jf = self.emit(Node::JumpIfFalse(cond.clone(), HOLE));
                self.stmt(then);
                if let Some(e) = els {
                    let j = self.emit(Node::Jump(HOLE));
                    let at = self.here();
                    self.patch(jf, at);
                    self.stmt(e);
                    let end = self.here();
                    self.patch(j, end);
                } else {
                    let end = self.here();
                    self.patch(jf, end);
                }
            }
            Stmt::While { cond, body } => {
                let slot = self.loops;
                self.loops += 1;
                self.emit(Node::LoopEnter(slot));
                let top = self.here();
                let jf = self.emit(Node::JumpIfFalse(cond.clone(), HOLE));
                self.stack.push(LoopCtx::default());
                self.stmt(body);
                let ctx = self.stack.pop().unwrap();
                let back = self.emit(Node::LoopBack { to: top, slot });
                let end = self.here();
                self.patch(jf, end);
                self.finish_loop(ctx, back, end);
            }
            Stmt::For { init, cond, step, body } => {
                if let Some(i) = init {
                    self.stmt(i);
                }
                let slot = self.loops;
                self.loops += 1;
                self.emit(Node::LoopEnter(slot));
                let top = self.here();
                let jf = cond.as_ref().map(|c| self.emit(Node::JumpIfFalse(c.clone(), HOLE)));
                self.stack.push(LoopCtx::default());
                self.stmt(body);
                let ctx = self.stack.pop().unwrap();
                let cont = self.here();
                if let Some(st) = step {
                    self.stmt(st);
                }
                self.emit(Node::LoopBack { to: top, slot });
                let end = self.here();
                if let Some(jf) = jf {
                    self.patch(jf, end);
                }
                self.finish_loop(ctx, cont, end);
            }
            Stmt::Function { name, params, body } => {
                let skip = self.emit(Node::Jump(HOLE));
                let entry = self.here();
                // `break` and `continue` do not cross a function boundary.
                let saved = std::mem::take(&mut self.stack);
                for s in body {
                    self.stmt(s);
                }
                self.stack = saved;
                self.emit(Node::Return(None));
                let after = self.here();
                self.patch(skip, after);
                self.funcs.insert(name.clone(), Func { entry, params: params.clone() });
            }
            Stmt::Return(e) => {
                self.emit(Node::Return(e.clone()));
            }
            Stmt::Break => match self.stack.is_empty() {
                true => {
                    self.emit(Node::End);
                }
                false => {
                    let j = self.emit(Node::Jump(HOLE));
                    self.stack.last_mut().unwrap().breaks.push(j);
                }
            },
            Stmt::Continue => match self.stack.is_empty() {
                true => {
                    self.emit(Node::End);
                }
                false => {
                    let j = self.emit(Node::Jump(HOLE));
                    self.stack.last_mut().unwrap().continues.push(j);
                }
            },
            Stmt::Assign { target, op, value } => {
                self.emit(Node::Assign { target: target.clone(), op: *op, value: value.clone() });
            }
            Stmt::IncDec { target, delta } => {
                self.emit(Node::IncDec { target: target.clone(), delta: *delta });
            }
            Stmt::Call { name, args } => {
                self.emit(Node::Eval(Expr::Call { name: name.clone(), args: args.clone() }));
            }
            Stmt::Command { name, args, line } => match vocab::command(name) {
                Some(cmd) => {
                    self.emit(Node::Cmd { cmd, args: args.clone(), line: *line });
                }
                None => {
                    self.emit(Node::Unknown { name: name.clone(), raw: String::new(), reason: "unknown command", line: *line });
                }
            },
            Stmt::Unknown { name, raw, reason, line } => {
                self.emit(Node::Unknown { name: name.clone(), raw: raw.clone(), reason, line: *line });
            }
            Stmt::With { line, .. } => {
                self.emit(Node::Unknown { name: "with".into(), raw: String::new(), reason: "body skipped: objects are not switched", line: *line });
            }
        }
    }

    fn finish_loop(&mut self, ctx: LoopCtx, cont: usize, end: usize) {
        for b in ctx.breaks {
            self.patch(b, end);
        }
        for c in ctx.continues {
            self.patch(c, cont);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripting::engine::{parser::parse, vocab::Era};

    fn c(src: &str) -> Compiled {
        compile(&parse(src, Era::V21))
    }

    #[test]
    fn every_script_ends_with_end() {
        assert!(matches!(c("").nodes.as_slice(), [Node::End]));
        assert!(matches!(c("hide;").nodes.last(), Some(Node::End)));
    }

    #[test]
    fn if_else_jumps_land_on_the_right_nodes() {
        let k = c("if (a) hide; else show;");
        // JumpIfFalse, hide, Jump, show, End
        assert_eq!(k.nodes.len(), 5);
        assert!(matches!(&k.nodes[0], Node::JumpIfFalse(_, 3)));
        assert!(matches!(&k.nodes[2], Node::Jump(4)));
    }

    #[test]
    fn while_loop_shape() {
        let k = c("while (a) hide;");
        // LoopEnter, JumpIfFalse(.., 4), hide, LoopBack{to:1}, End
        assert!(matches!(&k.nodes[0], Node::LoopEnter(0)));
        assert!(matches!(&k.nodes[1], Node::JumpIfFalse(_, 4)));
        assert!(matches!(&k.nodes[3], Node::LoopBack { to: 1, slot: 0 }));
        assert_eq!(k.loops, 1);
    }

    #[test]
    fn break_and_continue_are_patched() {
        let k = c("for (i=0;i<3;i++) { if (i==1) continue; if (i==2) break; hide; }");
        let jumps: Vec<usize> = k.nodes.iter().filter_map(|n| if let Node::Jump(t) = n { Some(*t) } else { None }).collect();
        assert!(jumps.iter().all(|t| *t != HOLE && *t <= k.nodes.len()));
    }

    #[test]
    fn break_outside_a_loop_ends_the_run() {
        assert!(matches!(c("break;").nodes.as_slice(), [Node::End, Node::End]));
        assert!(matches!(c("continue;").nodes.as_slice(), [Node::End, Node::End]));
    }

    #[test]
    fn functions_are_skipped_inline_and_registered() {
        let k = c("function f(a) { return a; }\nhide;");
        let f = &k.funcs["f"];
        assert_eq!(f.params, vec!["a".to_string()]);
        assert!(matches!(&k.nodes[0], Node::Jump(t) if *t == f.entry + 2));
        assert!(matches!(&k.nodes[f.entry], Node::Return(Some(_))));
        assert!(matches!(&k.nodes[f.entry + 1], Node::Return(None)));
    }

    #[test]
    fn break_does_not_cross_a_function_boundary() {
        let k = c("while (1) { function f() { break; } hide; }");
        assert!(k.nodes.iter().any(|n| matches!(n, Node::End)));
    }
}
