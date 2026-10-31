//! Parser for GS1 source.
//!
//! Never fails: a bad statement becomes a diagnostic and is skipped, and
//! a name that is not a command in the chosen era becomes
//! [`Stmt::Unknown`], which does nothing when run.
//!
//! Text arguments (`message`, `setimg`, `setplayerprop #c,hi`) are taken
//! as raw text up to the next comma or `;`. Number arguments are
//! expressions. Text arguments of built-in functions are raw text too,
//! except that a plain scope-prefixed variable (`this.name`) is read as
//! that variable, so `strequals(this.mode,fight)` works.

use super::ast::*;
use super::lexer::{Lexer, Spanned, Tok};
use super::vocab::{self, ArgKind, Cmd, Era};

/// Deepest nesting of blocks and expressions the parser follows.
const MAX_DEPTH: usize = 200;

/// Parse `src` with the vocabulary of `era`.
pub fn parse(src: &str, era: Era) -> Script {
    let mut p = Parser::new(src, era);
    let mut body = Vec::new();
    loop {
        match p.peek().tok.clone() {
            Tok::Eof => break,
            Tok::Op("}") => {
                p.diag("unmatched `}`");
                p.next();
            }
            _ => {
                if let Some(s) = p.stmt() {
                    body.push(s);
                }
            }
        }
    }
    Script { body, diagnostics: p.diags }
}

/// Parse one expression (the inside of `#v(...)`). `None` if it is not
/// exactly one well-formed expression.
pub fn parse_expr(src: &str, era: Era) -> Option<Expr> {
    let mut p = Parser::new(src, era);
    let e = p.expr();
    if p.diags.is_empty() && matches!(p.peek().tok, Tok::Eof) {
        Some(e)
    } else {
        None
    }
}

struct Parser<'a> {
    lx: Lexer<'a>,
    peeked: Option<Spanned>,
    era: Era,
    diags: Vec<Diag>,
    depth: usize,
    len: usize,
    /// Names the script defines with `function`, so `move();` calls the
    /// script's own `move` rather than the command.
    funcs: Vec<String>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str, era: Era) -> Self {
        let mut funcs = Vec::new();
        let mut lx = Lexer::new(src);
        let mut after_function = false;
        loop {
            match lx.next().tok {
                Tok::Eof => break,
                Tok::Ident(w) if after_function => {
                    funcs.push(w);
                    after_function = false;
                }
                Tok::Ident(w) => after_function = w == "function",
                _ => after_function = false,
            }
        }
        Parser { lx: Lexer::new(src), peeked: None, era, diags: Vec::new(), depth: 0, len: src.len(), funcs }
    }

    fn peek(&mut self) -> &Spanned {
        if self.peeked.is_none() {
            self.peeked = Some(self.lx.next());
        }
        self.peeked.as_ref().unwrap()
    }

    fn next(&mut self) -> Spanned {
        match self.peeked.take() {
            Some(t) => t,
            None => self.lx.next(),
        }
    }

    /// Give a peeked token back to the lexer so raw text can be read.
    fn unpeek(&mut self) {
        if let Some(t) = self.peeked.take() {
            self.lx.pos = t.start;
            self.lx.line = t.line;
        }
    }

    /// Whether the token after the peeked one is `op`. Reads nothing.
    fn second_is_op(&mut self, op: &str) -> bool {
        self.peek();
        let saved = (self.lx.clone(), self.peeked.clone());
        self.next();
        let r = self.is_op(op);
        self.lx = saved.0;
        self.peeked = saved.1;
        r
    }

    fn line(&mut self) -> u32 {
        self.peek().line
    }

    fn diag(&mut self, msg: &str) {
        let line = self.line();
        self.diags.push(Diag { line, msg: msg.to_string() });
    }

    fn is_op(&mut self, op: &str) -> bool {
        matches!(&self.peek().tok, Tok::Op(o) if *o == op)
    }

    fn eat_op(&mut self, op: &str) -> bool {
        if self.is_op(op) {
            self.next();
            true
        } else {
            false
        }
    }

    fn expect_op(&mut self, op: &str) -> bool {
        if self.eat_op(op) {
            true
        } else {
            self.diag(&format!("expected `{op}`"));
            false
        }
    }

    fn at_stmt_end(&mut self) -> bool {
        matches!(self.peek().tok, Tok::Op(";") | Tok::Op("}") | Tok::Eof)
    }

    /// Skip to the end of the current statement: past the next `;`, or up
    /// to the enclosing block's `}` or the end of input. A `{` met on the
    /// way opens a block that is skipped whole, and the statement ends with
    /// it: `*if (timeout) { ... }` (a block "commented out" with a `*`, as
    /// 2gta's scripts have it) does nothing, and a bad `*bad{x;}` inside an
    /// `if` does not close the `if`. Cases from pyReborn's (MIT)
    /// `test_gs1_parser_recovery.py`, taken from scripts 2gta serves.
    fn recover(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.peek().tok.clone() {
                Tok::Eof => return,
                Tok::Op("}") if depth == 0 => return,
                Tok::Op("}") => {
                    self.next();
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                Tok::Op("{") => {
                    self.next();
                    depth += 1;
                }
                Tok::Op(";") if depth == 0 => {
                    self.next();
                    return;
                }
                _ => {
                    self.next();
                }
            }
        }
    }

    /// `;`, or nothing before `}` and the end of input.
    fn end_stmt(&mut self) {
        if self.eat_op(";") || self.at_stmt_end() {
            return;
        }
        self.diag("expected `;`");
        self.recover();
    }

    fn enter(&mut self) -> bool {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.diag("nesting too deep; the rest of the script is dropped");
            self.peeked = None;
            self.lx.pos = self.len;
            return false;
        }
        true
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    // ---- statements ----

    fn stmt(&mut self) -> Option<Stmt> {
        if !self.enter() {
            self.leave();
            return None;
        }
        let r = self.stmt_inner();
        self.leave();
        r
    }

    fn stmt_inner(&mut self) -> Option<Stmt> {
        let t = self.peek().clone();
        match t.tok {
            Tok::Eof => None,
            Tok::Op("{") => Some(Stmt::Block(self.block())),
            Tok::Op(";") => {
                self.next();
                Some(Stmt::Block(Vec::new()))
            }
            Tok::Ident(w) => {
                self.next();
                self.ident_stmt(w, t.line, t.start)
            }
            _ => {
                self.diag("unexpected token");
                self.recover();
                Some(Stmt::Block(Vec::new()))
            }
        }
    }

    fn block(&mut self) -> Vec<Stmt> {
        self.next(); // `{`
        let mut v = Vec::new();
        loop {
            match self.peek().tok.clone() {
                Tok::Op("}") => {
                    self.next();
                    break;
                }
                Tok::Eof => {
                    self.diag("missing `}`");
                    break;
                }
                _ => {
                    if let Some(s) = self.stmt() {
                        v.push(s);
                    }
                }
            }
        }
        v
    }

    fn ident_stmt(&mut self, w: String, line: u32, start: usize) -> Option<Stmt> {
        match w.as_str() {
            "if" => {
                if !self.expect_op("(") {
                    self.recover();
                    return Some(Stmt::Block(Vec::new()));
                }
                let cond = self.expr();
                self.expect_op(")");
                let then = Box::new(self.stmt().unwrap_or(Stmt::Block(Vec::new())));
                self.skip_semis_before_else();
                let els = if matches!(&self.peek().tok, Tok::Ident(e) if e == "else" || e == "elseif") {
                    let is_elseif = matches!(&self.peek().tok, Tok::Ident(e) if e == "elseif");
                    self.next();
                    if is_elseif {
                        // `elseif` is `else if`: the client matches command
                        // names by prefix, as with `hurt1`.
                        let t = self.peek().clone();
                        Some(Box::new(self.ident_stmt("if".into(), t.line, t.start).unwrap_or(Stmt::Block(Vec::new()))))
                    } else {
                        Some(Box::new(self.stmt().unwrap_or(Stmt::Block(Vec::new()))))
                    }
                } else {
                    None
                };
                Some(Stmt::If { cond, then, els })
            }
            "while" => {
                if !self.expect_op("(") {
                    self.recover();
                    return Some(Stmt::Block(Vec::new()));
                }
                let cond = self.expr();
                self.expect_op(")");
                let body = Box::new(self.stmt().unwrap_or(Stmt::Block(Vec::new())));
                Some(Stmt::While { cond, body })
            }
            "for" => {
                if !self.expect_op("(") {
                    self.recover();
                    return Some(Stmt::Block(Vec::new()));
                }
                let init = self.simple_stmt().map(Box::new);
                self.expect_op(";");
                let cond = if self.is_op(";") { None } else { Some(self.expr()) };
                self.expect_op(";");
                let step = self.simple_stmt().map(Box::new);
                // `for (i=0;i<3;i++;)`: a `;` after the step is let through.
                self.eat_op(";");
                self.expect_op(")");
                let body = Box::new(self.stmt().unwrap_or(Stmt::Block(Vec::new())));
                Some(Stmt::For { init, cond, step, body })
            }
            "function" => {
                let name = match self.peek().tok.clone() {
                    Tok::Ident(n) => {
                        self.next();
                        n
                    }
                    _ => {
                        self.diag("function needs a name");
                        self.recover();
                        return Some(Stmt::Block(Vec::new()));
                    }
                };
                let mut params = Vec::new();
                if self.eat_op("(") {
                    while let Tok::Ident(p) = self.peek().tok.clone() {
                        self.next();
                        params.push(p);
                        if !self.eat_op(",") {
                            break;
                        }
                    }
                    self.expect_op(")");
                }
                if !self.is_op("{") {
                    self.diag("function needs a `{` body");
                    self.recover();
                    return Some(Stmt::Block(Vec::new()));
                }
                let body = self.block();
                Some(Stmt::Function { name, params, body })
            }
            "return" => {
                let v = if self.at_stmt_end() { None } else { Some(self.expr()) };
                self.end_stmt();
                Some(Stmt::Return(v))
            }
            "break" => {
                self.end_stmt();
                Some(Stmt::Break)
            }
            "continue" => {
                self.end_stmt();
                Some(Stmt::Continue)
            }
            "elseif" => {
                // `elseif` at statement start is an `if`.
                if !self.expect_op("(") {
                    self.recover();
                    return Some(Stmt::Block(Vec::new()));
                }
                let cond = self.expr();
                self.expect_op(")");
                let then = Box::new(self.stmt().unwrap_or(Stmt::Block(Vec::new())));
                Some(Stmt::If { cond, then, els: None })
            }
            // `with (target) body`: from the 2.1 clients (the word first
            // appears in 2.17) and the NPC server.
            "with" if self.era >= Era::V21 && self.is_op("(") => {
                self.next();
                let target = self.expr();
                self.expect_op(")");
                let body = Box::new(self.stmt().unwrap_or(Stmt::Block(Vec::new())));
                Some(Stmt::With { target, body, line })
            }
            "else" => {
                self.diag("`else` without `if`");
                self.recover();
                Some(Stmt::Block(Vec::new()))
            }
            _ => self.name_stmt(w, line, start),
        }
    }

    /// A statement that starts with a name that is not a keyword: a
    /// command, an assignment, a call, or something unknown.
    fn name_stmt(&mut self, w: String, line: u32, start: usize) -> Option<Stmt> {
        let nxt = self.peek().tok.clone();
        let assigns = matches!(nxt, Tok::Op("=" | ":=" | "+=" | "-=" | "*=" | "/=" | "++" | "--" | "["));
        let paren = matches!(nxt, Tok::Op("("));
        // `name(...)` calls the script's own function of that name, even
        // one named like a command (`move();`, `hurt();` in baddy scripts).
        let own = paren && self.funcs.contains(&w);
        if !assigns && !own {
            if let Some(cmd) = vocab::command_in(&w, self.era) {
                // `hideimgs();`: a command with empty parentheses and no
                // function of that name is the command with no arguments.
                if paren && self.second_is_op(")") {
                    self.next();
                    self.next();
                    self.end_stmt();
                    return Some(Stmt::Command { name: cmd.name.to_string(), args: Vec::new(), line });
                }
                return Some(self.command(cmd, line));
            }
        }
        if assigns {
            let target = self.var_tail(w);
            let s = self.assign_tail(target);
            // `this.a = {1,2,3}` with no `;` after the `}` (the hookshot
            // and Q-menu weapons): the list's `}` ends the statement.
            let list = matches!(&s, Stmt::Assign { value: Expr::List(_), .. });
            if !(list && !self.is_op(";")) {
                self.end_stmt();
            }
            return Some(s);
        }
        if paren {
            let saved = (self.lx.clone(), self.peeked.clone(), self.diags.len());
            let args = self.call_args(&w);
            if !(self.diags.len() == saved.2 && self.is_op(",")) {
                self.end_stmt();
                return Some(Stmt::Call { name: w, args });
            }
            // `name (...),`: not a call but an unknown command whose text
            // starts with `(` (`addguildmember (guildname),#t(1),#t(2);`).
            // Kept as text below.
            self.lx = saved.0;
            self.peeked = saved.1;
            self.diags.truncate(saved.2);
        }
        // Scripts taken from level files often run the first argument into
        // the command name (`hurt1;`, `shootarrowdown;`). Accept that when
        // a command name is followed at once by a number or a name from
        // its first argument's table.
        if let Some(cmd) = self.glued_command(&w) {
            self.peeked = None;
            self.lx.pos = start + cmd.name.len();
            self.lx.line = line;
            return Some(self.command(cmd, line));
        }
        // Not a command here. Keep its text and move on.
        self.unpeek();
        let raw = self.lx.raw_arg_braced(b";}");
        self.end_stmt();
        let reason = if vocab::command(&w).is_some() { "not available in this version" } else { "unknown command" };
        Some(Stmt::Unknown { name: w, raw, reason, line })
    }

    /// The longest command in this era that `w` starts with, if what
    /// follows reads as its first argument.
    fn glued_command(&self, w: &str) -> Option<&'static Cmd> {
        vocab::COMMANDS
            .iter()
            .filter(|c| c.since <= self.era && w.len() > c.name.len() && w.starts_with(c.name))
            .filter(|c| {
                let rest = &w[c.name.len()..];
                let first = c.sig.chars().next();
                rest.starts_with(|ch: char| ch.is_ascii_digit() || ch == '-')
                    && first.is_some_and(|l| !vocab::is_text_letter(l))
                    || first.is_some_and(|l| vocab::named_constant(l, rest).is_some())
            })
            .max_by_key(|c| c.name.len())
    }

    fn command(&mut self, cmd: &Cmd, line: u32) -> Stmt {
        let sig: Vec<char> = cmd.sig.chars().collect();
        let mut args = Vec::new();
        let mut i = 0;
        while !self.at_stmt_end() {
            let letter = sig.get(i).copied().unwrap_or('S');
            if vocab::is_text_letter(letter) {
                self.unpeek();
                // The last argument takes the rest of the statement, commas included.
                let last = i + 1 >= sig.len();
                let stops: &[u8] = if last { b";}" } else { b",;}" };
                args.push(Arg::Text(self.lx.raw_arg_braced(stops)));
            } else if self.is_op(",") {
                // An empty number argument (`seteffect ,,,;`) is 0.
                args.push(Arg::Num(Expr::Num(0.0)));
            } else {
                args.push(Arg::Num(self.arg_expr(letter)));
            }
            i += 1;
            if !self.eat_op(",") {
                break;
            }
        }
        self.end_stmt();
        Stmt::Command { name: cmd.name.to_string(), args, line }
    }

    /// A number argument. For `D C I B L` a bare name from the matching
    /// table (`left`, `red`, `bomb`, ...) becomes its number.
    fn arg_expr(&mut self, letter: char) -> Expr {
        if let Tok::Ident(n) = self.peek().tok.clone() {
            if let Some(v) = vocab::named_constant(letter, &n) {
                self.next();
                return Expr::Num(v);
            }
        }
        self.expr()
    }

    /// Variable name already read; read its optional `[index, ...]`
    /// and any chained `.field` or `.field[index]`.
    fn var_tail(&mut self, mut name: String) -> VarRef {
        let mut index = Vec::new();
        loop {
            if self.eat_op("[") {
                loop {
                    index.push(self.expr());
                    if !self.eat_op(",") {
                        break;
                    }
                }
                self.expect_op("]");
            }
            // `npcs[i].save` or `npcs[i].save[5]`: fold the field into the name.
            if self.eat_op(".") {
                if let Tok::Ident(f) = self.peek().tok.clone() {
                    self.next();
                    name = format!("{name}.{f}");
                    continue;
                }
            }
            break;
        }
        VarRef { name, index }
    }

    fn assign_tail(&mut self, target: VarRef) -> Stmt {
        let op = match self.peek().tok.clone() {
            Tok::Op("=" | ":=") => AssignOp::Set,
            Tok::Op("+=") => AssignOp::Add,
            Tok::Op("-=") => AssignOp::Sub,
            Tok::Op("*=") => AssignOp::Mul,
            Tok::Op("/=") => AssignOp::Div,
            Tok::Op("++") => {
                self.next();
                return Stmt::IncDec { target, delta: 1.0 };
            }
            Tok::Op("--") => {
                self.next();
                return Stmt::IncDec { target, delta: -1.0 };
            }
            _ => {
                self.diag("expected an assignment");
                self.recover();
                return Stmt::Block(Vec::new());
            }
        };
        self.next();
        let value = self.expr();
        Stmt::Assign { target, op, value }
    }

    /// The init and step parts of `for`: an assignment or `x++`, or nothing.
    fn simple_stmt(&mut self) -> Option<Stmt> {
        match self.peek().tok.clone() {
            Tok::Ident(n) => {
                self.next();
                let target = self.var_tail(n);
                Some(self.assign_tail(target))
            }
            _ => None,
        }
    }

    // ---- expressions ----

    fn expr(&mut self) -> Expr {
        if !self.enter() {
            self.leave();
            return Expr::Num(0.0);
        }
        let mut e = self.binary(1);
        // `cond ? a : b` (2.1 and later), the loosest operator.
        if self.era >= Era::V21 && self.eat_op("?") {
            let then = self.expr();
            self.expect_op(":");
            let els = self.expr();
            e = Expr::Cond { cond: Box::new(e), then: Box::new(then), els: Box::new(els) };
        }
        self.leave();
        e
    }

    fn binary(&mut self, min: u8) -> Expr {
        let mut lhs = self.unary();
        loop {
            let t = self.peek().tok.clone();
            let (op, prec, right) = match &t {
                Tok::Op("||") => (BinOp::Or, 1, false),
                Tok::Op("&&") => (BinOp::And, 2, false),
                Tok::Ident(w) if w == "in" => {
                    if 3 < min {
                        break;
                    }
                    self.next();
                    lhs = self.in_tail(lhs);
                    continue;
                }
                // `a,b in |lo,hi|`: both values in the range (GS1, as in
                // `mapdx,mapdy in |-1,1|`). Anything else after a comma is
                // left to the caller (an argument list).
                Tok::Op(",") if min <= 3 => match self.pair_in(&lhs) {
                    Some(e) => {
                        lhs = e;
                        continue;
                    }
                    None => break,
                },
                // In a condition a single `=` compares, as it does in level scripts.
                // Text join, below comparison and above `&&` (the order of the
                // GS2 grammar, which has the same four operators).
                Tok::Op("@") => (BinOp::Cat, 3, false),
                Tok::Ident(w) if w == "spc" => (BinOp::CatSpc, 3, false),
                Tok::Ident(w) if w == "nl" => (BinOp::CatNl, 3, false),
                Tok::Ident(w) if w == "tab" => (BinOp::CatTab, 3, false),
                Tok::Op("==") | Tok::Op("=") => (BinOp::Eq, 4, false),
                Tok::Op("!=") | Tok::Op("<>") => (BinOp::Ne, 4, false),
                Tok::Op("<") => (BinOp::Lt, 5, false),
                Tok::Op(">") => (BinOp::Gt, 5, false),
                Tok::Op("<=") | Tok::Op("=<") => (BinOp::Le, 5, false),
                Tok::Op(">=") | Tok::Op("=>") => (BinOp::Ge, 5, false),
                Tok::Op("+") => (BinOp::Add, 6, false),
                Tok::Op("-") => (BinOp::Sub, 6, false),
                Tok::Op("*") => (BinOp::Mul, 7, false),
                Tok::Op("/") => (BinOp::Div, 7, false),
                Tok::Op("%") => (BinOp::Mod, 7, false),
                Tok::Op("^") => (BinOp::Pow, 8, true),
                // `300++this.i` (a balloon game's showimg index): `+` and a
                // unary `+`. Statements read `x++` before this point.
                Tok::Op("++") => (BinOp::Add, 6, false),
                Tok::Op("--") => (BinOp::Sub, 6, false),
                _ => break,
            };
            if prec < min {
                break;
            }
            self.next();
            let rhs = self.binary(if right { prec } else { prec + 1 });
            let rhs = match t {
                Tok::Op("--") => Expr::Unary(UnOp::Neg, Box::new(rhs)),
                _ => rhs,
            };
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        lhs
    }

    /// At a `,` after `first`: if `, second in |lo,hi|` follows, both in
    /// the range. Otherwise nothing is consumed.
    /// Eat stray `;` between an if's statement and its `else`, as in
    /// `if (a) b=1;; else b=2;` (the Quiver weapon). Nothing is eaten when
    /// no `else` follows.
    fn skip_semis_before_else(&mut self) {
        if !self.is_op(";") {
            return;
        }
        let saved = (self.lx.clone(), self.peeked.clone());
        while self.eat_op(";") {}
        if !matches!(&self.peek().tok, Tok::Ident(e) if e == "else" || e == "elseif") {
            self.lx = saved.0;
            self.peeked = saved.1;
        }
    }

    fn pair_in(&mut self, first: &Expr) -> Option<Expr> {
        let saved = (self.lx.clone(), self.peeked.clone(), self.diags.len());
        self.next();
        let second = self.binary(4);
        if matches!(&self.peek().tok, Tok::Ident(w) if w == "in") {
            self.next();
            if self.is_op("|") {
                if let Expr::InRange { e, lo, hi } = self.in_tail(second) {
                    let a = Expr::InRange { e: Box::new(first.clone()), lo: lo.clone(), hi: hi.clone() };
                    return Some(Expr::Bin(BinOp::And, Box::new(a), Box::new(Expr::InRange { e, lo, hi })));
                }
            }
        }
        self.lx = saved.0;
        self.peeked = saved.1;
        self.diags.truncate(saved.2);
        None
    }

    /// After `in`: `|lo,hi|`, `|lo,hi>`, or `{a,b,c}`.
    fn in_tail(&mut self, e: Expr) -> Expr {
        let e = Box::new(e);
        if self.eat_op("|") {
            let lo = Box::new(self.binary(4));
            self.expect_op(",");
            // Above the comparisons, so the `>` of `|lo,hi>` closes the range.
            let hi = Box::new(self.binary(6));
            // `|lo,hi|`, `|lo,hi>`, or `|lo,hi`.
            if !self.eat_op("|") && !self.eat_op(">") {
                // Unclosed range: stop at the next token that is not part of it.
            }
            Expr::InRange { e, lo, hi }
        } else if self.era >= Era::V21 && self.eat_op("<") {
            // `in <lo,hi>` (2.1 and later): the GS2 grammar of the public
            // GServer reads it as the same range as `|lo,hi|`.
            let lo = Box::new(self.binary(4));
            self.expect_op(",");
            let hi = Box::new(self.binary(6));
            self.expect_op(">");
            Expr::InRange { e, lo, hi }
        } else if self.eat_op("{") {
            let mut list = Vec::new();
            while !self.is_op("}") && !matches!(self.peek().tok, Tok::Eof) {
                list.push(self.binary(4));
                if !self.eat_op(",") {
                    break;
                }
            }
            self.expect_op("}");
            Expr::InList { e, list }
        } else if let (true, Tok::Ident(name)) = (self.era >= Era::V21, self.peek().tok.clone()) {
            // `x in this.list`: the values of an array (2.1 and later).
            self.next();
            Expr::InArray { e, name }
        } else {
            self.diag("`in` needs |lo,hi| or {list}");
            Expr::InList { e, list: Vec::new() }
        }
    }

    fn unary(&mut self) -> Expr {
        match self.peek().tok.clone() {
            Tok::Op("-") => {
                self.next();
                match self.unary() {
                    Expr::Num(n) => Expr::Num(-n),
                    e => Expr::Unary(UnOp::Neg, Box::new(e)),
                }
            }
            Tok::Op("!") => {
                self.next();
                Expr::Unary(UnOp::Not, Box::new(self.unary()))
            }
            Tok::Op("+") => {
                self.next();
                self.unary()
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Expr {
        let t = self.peek().clone();
        match t.tok {
            Tok::Num(n) => {
                self.next();
                Expr::Num(n)
            }
            Tok::Str(s) => {
                self.next();
                Expr::Str(s)
            }
            Tok::Op("(") => {
                self.next();
                let e = self.expr();
                self.expect_op(")");
                e
            }
            // `{1,2,3}`: an array value (2.1 and later), as in
            // `this.a = {1,2,3}` and `showpoly 300,{0,0,32,0,...}`.
            Tok::Op("{") if self.era >= Era::V21 => {
                self.next();
                let mut list = Vec::new();
                while !self.is_op("}") && !matches!(self.peek().tok, Tok::Eof) {
                    // `{0x2, ,0x2A5}`: an empty entry is 0 and keeps its place.
                    list.push(if self.is_op(",") { Expr::Num(0.0) } else { self.expr() });
                    if !self.eat_op(",") {
                        break;
                    }
                }
                self.expect_op("}");
                Expr::List(list)
            }
            Tok::Ident(w) => {
                self.next();
                if w == "true" {
                    return Expr::Num(1.0);
                }
                if w == "false" {
                    return Expr::Num(0.0);
                }
                if self.is_op("(") {
                    let args = self.call_args(&w);
                    return Expr::Call { name: w, args };
                }
                Expr::Var(self.var_tail(w))
            }
            _ => {
                self.diag("expected an expression");
                Expr::Num(0.0)
            }
        }
    }

    /// `(a, b, ...)` after a function name. Built-ins read their text
    /// arguments raw.
    fn call_args(&mut self, name: &str) -> Vec<Expr> {
        self.next(); // `(`
        let kinds: &[ArgKind] = vocab::BUILTINS.iter().find(|b| b.name == name).map_or(&[], |b| b.args);
        let mut args = Vec::new();
        if self.eat_op(")") {
            return args;
        }
        loop {
            if kinds.get(args.len()) == Some(&ArgKind::Text) {
                self.unpeek();
                // The last text argument runs to the `)`, commas included,
                // as a command's last text argument does
                // (`playersays(0,buy tank1)`). [inferred]
                let last = args.len() + 1 == kinds.len();
                let raw = self.lx.raw_arg(if last { b")" } else { b",)" });
                let quoted = self.lx.last_quoted;
                args.push(text_arg(&raw, quoted, self.era));
            } else if self.is_op(",") || self.is_op(")") {
                // An empty argument (`textwidth(1,arial,,text)`) is 0, as an
                // empty number argument of a command is.
                args.push(Expr::Num(0.0));
            } else {
                args.push(self.expr());
            }
            if !self.eat_op(",") {
                break;
            }
        }
        self.expect_op(")");
        args
    }
}

const SCOPES: &[&str] = &["this.", "client.", "server.", "level.", "local."];

/// A text argument of a built-in: a plain scope-prefixed variable is read
/// as that variable; anything else is text.
fn text_arg(raw: &str, quoted: bool, era: Era) -> Expr {
    let plain_name = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    let lower = raw.to_ascii_lowercase();
    if !quoted && SCOPES.iter().any(|s| lower.starts_with(s)) && plain_name(&lower) {
        return Expr::Var(VarRef { name: lower, index: Vec::new() });
    }
    // `this.a[3]` indexed form.
    if !quoted && SCOPES.iter().any(|s| lower.starts_with(s)) && lower.ends_with(']') && lower.contains('[') {
        let mut p = Parser::new(&lower, era);
        let e = p.primary();
        if p.diags.is_empty() {
            return e;
        }
    }
    Expr::Str(raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(src: &str) -> String {
        parse(src, Era::V21).to_string()
    }

    #[test]
    fn the_quiver_condition_parses() {
        // 2gta's *Quiver weapon, line 180.
        let s = parse("if ((onmapx(#L)>=0 || strequals(#t(0),#L)) && mapdx,mapdy in |-1,1|) { this.a=1; } else { this.a=2; }", Era::V205);
        assert!(s.diagnostics.is_empty(), "{:?}", s.diagnostics);
    }

    #[test]
    fn a_double_semicolon_before_else_keeps_the_else() {
        // 2gta's *Quiver weapon, line 322.
        let s = g("if (a>4) b=a-4;;\nelse b=0;");
        assert_eq!(s, g("if (a>4) b=a-4; else b=0;"));
        assert!(parse("if (a) b=1;; else b=2;", Era::V205).diagnostics.is_empty());
    }

    #[test]
    fn event_blocks_and_commands() {
        let s = g("if (created) { setimg door.png; setshape 1,32,32; }\nif (playertouchsme) { hide; sleep 5; show; }");
        assert_eq!(
            s,
            "(if created (block (cmd setimg \"door.png\") (cmd setshape 1 32 32)))\n\
             (if playertouchsme (block (cmd hide) (cmd sleep 5) (cmd show)))\n"
        );
    }

    #[test]
    fn text_arguments_are_raw_and_split_on_commas() {
        assert_eq!(g("message Hello, world :);"), "(cmd message \"Hello, world :)\")\n");
        assert_eq!(g("setplayerprop #c,Hi #v(this.n) there;"), "(cmd setplayerprop \"#c\" \"Hi #v(this.n) there\")\n");
        assert_eq!(g("message \"quoted, text\";"), "(cmd message \"quoted, text\")\n");
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(g("x = 1 + 2 * 3;"), "(= x (+ 1 (* 2 3)))\n");
        // Chained index and no-space if.
        assert_eq!(g("if(npcs[this.i].save[5]==38) x=1;"), "(if (== (idx npcs.save this.i 5) 38) (= x 1))\n");
        assert_eq!(g("if (this.i>0&&players[this.i].hearts>0) x=1;"), "(if (&& (> this.i 0) (> (idx players.hearts this.i) 0)) (= x 1))\n");
        assert_eq!(g("x = (1 + 2) * 3;"), "(= x (* (+ 1 2) 3))\n");
        assert_eq!(g("x = 2 ^ 3 ^ 2;"), "(= x (^ 2 (^ 3 2)))\n");
        assert_eq!(g("x = a < b && c || d;"), "(= x (|| (&& (< a b) c) d))\n");
        assert_eq!(g("x = -y + !z;"), "(= x (+ (neg y) (! z)))\n");
        assert_eq!(g("x = 10 - 4 - 3;"), "(= x (- (- 10 4) 3))\n");
    }

    #[test]
    fn assignment_forms() {
        assert_eq!(
            g("this.hp = 3; this.hp += 2; x -= 1; y *= 2; z /= 4; i++; j--;"),
            "(= this.hp 3)\n(+= this.hp 2)\n(-= x 1)\n(*= y 2)\n(/= z 4)\n(++ i)\n(-- j)\n"
        );
    }

    #[test]
    fn arrays_and_indices() {
        assert_eq!(g("this.a[3] = 7; board[x,y] = 1; setarray this.list,5;"), "(= (idx this.a 3) 7)\n(= (idx board x y) 1)\n(cmd setarray \"this.list\" 5)\n");
        assert_eq!(g("x = this.a[i+1];"), "(= x (idx this.a (+ i 1)))\n");
    }

    #[test]
    fn control_flow() {
        assert_eq!(g("if (a) hide; else show;"), "(if a (cmd hide) (cmd show))\n");
        assert_eq!(g("if (a) { x=1; } else if (b) { x=2; } else { x=3; }"), "(if a (block (= x 1)) (if b (block (= x 2)) (block (= x 3))))\n");
        assert_eq!(g("while (i < 3) i++;"), "(while (< i 3) (++ i))\n");
        assert_eq!(g("for (i = 0; i < 3; i++) { x += i; }"), "(for (= i 0) (< i 3) (++ i) (block (+= x i)))\n");
        assert_eq!(g("for (;;) break;"), "(for _ _ _ (break))\n");
    }

    #[test]
    fn functions_and_calls() {
        assert_eq!(
            g("function bump(n) { return n + 1; }\nx = bump(2);\nbump(3);"),
            "(fn bump (n) (return (+ n 1)))\n(= x (call bump 2))\n(call bump 3)\n"
        );
        assert_eq!(g("function f() { }"), "(fn f ())\n");
    }

    #[test]
    fn in_ranges_and_lists() {
        assert_eq!(g("if (x in |0,64|) hide;"), "(if (in x range 0 64) (cmd hide))\n");
        assert_eq!(g("if (2 in {1,2,3}) hide;"), "(if (in 2 list 1 2 3) (cmd hide))\n");
        // Closed with `>`.
        assert_eq!(g("if (x in |1,5>) hide;"), "(if (in x range 1 5) (cmd hide))\n");
        assert_eq!(g("if (x in |a,b+1> && y) hide;"), "(if (&& (in x range a (+ b 1)) y) (cmd hide))\n");
        // A list inside a larger condition.
        assert_eq!(g("if (a==1 && x in {1,2}) hide;"), "(if (&& (== a 1) (in x list 1 2)) (cmd hide))\n");
    }

    #[test]
    fn forms_from_2_1() {
        assert_eq!(g("this.a = {1, x+1};"), "(= this.a (list 1 (+ x 1)))\n");
        assert_eq!(g("x = (a>1)?2:3;"), "(= x (? (> a 1) 2 3))\n");
        assert_eq!(g("changeimgvis 300+i,(i==0)?4:5;"), "(cmd changeimgvis (+ 300 i) (? (== i 0) 4 5))\n");
        assert_eq!(g("with (getnpc(db)) { x = 1; }"), "(with (call getnpc \"db\") (block (= x 1)))\n");
        // An empty number argument is 0, in every version.
        assert_eq!(g("seteffect ,,,;"), "(cmd seteffect 0 0 0)\n");
        assert!(parse("seteffect ,,,;", Era::V205).diagnostics.is_empty());
        // A code with digits keeps its argument whole inside a call.
        assert_eq!(g("x = strtofloat(#P1(-1));"), "(= x (call strtofloat \"#P1(-1)\"))\n");
    }

    #[test]
    fn laundry_forms_from_the_npc_corpus() {
        // No space after `if`, no spaces round `&&`.
        assert_eq!(g("if(a&&b) hide;"), "(if (&& a b) (cmd hide))\n");
        // `elseif` as one word, after a block with or without a space.
        let want = "(if a (block (= x 1)) (if b (block (= x 2)) (block (= x 3))))\n";
        assert_eq!(g("if (a) {x=1;} elseif (b) {x=2;} else {x=3;}"), want);
        assert_eq!(g("if (a) {x=1;}elseif(b){x=2;}else{x=3;}"), want);
        // A field after an index, in a statement and in an expression.
        assert_eq!(g("d = npcs[i].x - x;"), "(= d (- (idx npcs.x i) x))\n");
        assert_eq!(g("players[i].x = 3;"), "(= (idx players.x i) 3)\n");
        // The 2.05 comparison table's other spellings.
        assert_eq!(g("if (a <> b) hide;"), "(if (!= a b) (cmd hide))\n");
        assert_eq!(g("if (a => b && c =< d) hide;"), "(if (&& (>= a b) (<= c d)) (cmd hide))\n");
    }

    #[test]
    fn stress_set_forms_of_2026_10_30() {
        // A command with empty parentheses is the command; a script's own
        // function of that name is called instead.
        assert_eq!(g("hideimgs();"), "(cmd hideimgs)\n");
        assert_eq!(g("function move() { x = 1; }\nmove();"), "(fn move () (= x 1))\n(call move)\n");
        assert_eq!(g("move();\nfunction move() { }"), "(call move)\n(fn move ())\n");
        // `in <lo,hi>` (2.1 and later), the same range as `|lo,hi|`.
        assert_eq!(g("if (x in <1,a+2> && y) hide;"), "(if (&& (in x range 1 (+ a 2)) y) (cmd hide))\n");
        assert!(!parse("if (x in <1,2>) hide;", Era::V205).diagnostics.is_empty());
        // Empty entries of a list keep their place; a list's `}` ends its
        // statement.
        assert_eq!(g("a = {1, ,2};"), "(= a (list 1 0 2))\n");
        assert_eq!(g("a = {1,2}\nfunction f() { }"), "(= a (list 1 2))\n(fn f ())\n");
        // An empty argument of a call is 0; a last text argument keeps its commas.
        assert_eq!(g("w = textwidth(1,arial,,x);"), "(= w (call textwidth 1 arial 0 x))\n");
        assert_eq!(g("if (playersays(0,buy tank1)) hide;"), "(if (call playersays \"0,buy tank1\") (cmd hide))\n");
        // `with` targets read their name as text.
        assert_eq!(g("with (getnpc(House-Control#v(h))) { }"), "(with (call getnpc \"House-Control#v(h)\") (block))\n");
        // `a ++ b` between two values.
        assert_eq!(g("x = 300++i;"), "(= x (+ 300 i))\n");
        // A comma inside plain parentheses stays in the text.
        assert_eq!(g("showimg 5,@a@#s(l) (#v(x), #v(y)),1,2;"), "(cmd showimg 5 \"@a@#s(l) (#v(x), #v(y))\" 1 2)\n");
        // An unknown command whose text starts with `(`.
        assert!(parse("addguildmember (g),#t(1),#t(2);", Era::V21).diagnostics.is_empty());
    }

    #[test]
    fn builtin_text_arguments() {
        assert_eq!(g("if (strequals(#c,hello)) hide;"), "(if (call strequals \"#c\" \"hello\") (cmd hide))\n");
        assert_eq!(g("if (strequals(this.mode,fight)) hide;"), "(if (call strequals this.mode \"fight\") (cmd hide))\n");
        assert_eq!(g("if (strequals(\"this.mode\",x)) hide;"), "(if (call strequals \"this.mode\" \"x\") (cmd hide))\n");
        assert_eq!(g("x = strlen(#v(this.a));"), "(= x (call strlen \"#v(this.a)\"))\n");
        assert_eq!(g("x = random(1, 6) + abs(-2);"), "(= x (+ (call random 1 6) (call abs -2)))\n");
    }

    #[test]
    fn named_constants_in_typed_arguments() {
        assert_eq!(g("setplayerdir left;"), "(cmd setplayerdir 1)\n");
        assert_eq!(g("setplayerdir dir + 1;"), "(cmd setplayerdir (+ dir 1))\n");
        assert_eq!(g("setskincolor red;"), "(cmd setskincolor 4)\n");
        assert_eq!(g("lay bomb;"), "(cmd lay 8)\n");
        assert_eq!(g("putcomp frog,10,12;"), "(cmd putcomp 5 10 12)\n");
        assert_eq!(g("carryobject vase;"), "(cmd carryobject 2)\n");
    }

    #[test]
    fn expression_only_parsing() {
        assert_eq!(parse_expr("this.a + 1", Era::V21).unwrap().to_string(), "(+ this.a 1)");
        assert!(parse_expr("1 +", Era::V21).is_none());
        assert!(parse_expr("1 2", Era::V21).is_none());
    }

    #[test]
    fn commands_glued_to_their_first_argument() {
        assert_eq!(g("hurt1;"), "(cmd hurt 1)\n");
        assert_eq!(g("putexplosion10,38,22;"), "(cmd putexplosion 10 38 22)\n");
        assert_eq!(g("shootarrowdown;"), "(cmd shootarrow 2)\n");
        assert_eq!(g("setplayerdirleft;"), "(cmd setplayerdir 1)\n");
        // A text-first command is not split, and neither is a plain unknown.
        assert!(g("message1;").starts_with("(unknown message1"));
        assert!(g("hurtful;").starts_with("(unknown hurtful"));
        // Longest command wins: `setlevel2` over `setlevel`.
        assert_eq!(g("setlevel2 a.nw,1,2;"), "(cmd setlevel2 \"a.nw\" 1 2)\n");
    }

    #[test]
    fn a_single_equals_in_a_condition_compares() {
        assert_eq!(g("if (a && this.d=0) hide;"), "(if (&& a (== this.d 0)) (cmd hide))\n");
        assert_eq!(g("x = 1;"), "(= x 1)\n");
    }

    #[test]
    fn comments_and_case() {
        assert_eq!(g("// c\nIF (Created) { /* x */ HIDE; }"), "(if created (block (cmd hide)))\n");
    }

    #[test]
    fn unknown_and_gated_commands_are_kept_not_fatal() {
        assert_eq!(g("frobnicate 1,2;"), "(unknown frobnicate \"1,2\" unknown command)\n");
        let s = parse("hitobjects 1,2,3; tokenize2 a,b;", Era::V200);
        assert_eq!(
            s.to_string(),
            "(unknown hitobjects \"1,2,3\" not available in this version)\n(unknown tokenize2 \"a,b\" not available in this version)\n"
        );
        assert!(s.diagnostics.is_empty());
        assert_eq!(parse("hitobjects 1,2,3;", Era::V201).to_string(), "(cmd hitobjects 1 2 3)\n");
    }

    #[test]
    fn missing_semicolons_before_a_brace_are_accepted() {
        assert_eq!(g("if (a) { hide }"), "(if a (block (cmd hide)))\n");
    }

    #[test]
    fn errors_are_diagnostics_and_parsing_continues() {
        let s = parse("if created) { hide; }\nshow;\n} hide;", Era::V21);
        assert!(!s.diagnostics.is_empty());
        assert!(s.to_string().contains("(cmd show)"));
        assert!(s.to_string().contains("(cmd hide)"));
        let s = parse("if (a) { hide;", Era::V21);
        assert!(s.diagnostics.iter().any(|d| d.msg.contains("missing `}`")));
        assert!(s.to_string().contains("(cmd hide)"));
    }

    #[test]
    fn deep_nesting_is_cut_off_without_overflowing() {
        let src = "{".repeat(5000) + "hide;";
        let s = parse(&src, Era::V21);
        assert!(s.diagnostics.iter().any(|d| d.msg.contains("too deep")));
        let src = format!("x = {}1{};", "(".repeat(5000), ")".repeat(5000));
        let s = parse(&src, Era::V21);
        assert!(s.diagnostics.iter().any(|d| d.msg.contains("too deep")));
    }

    #[test]
    fn diagnostics_carry_line_numbers() {
        let s = parse("hide;\nhide;\nx = ;\n", Era::V21);
        assert_eq!(s.diagnostics[0].line, 3);
    }

    #[test]
    fn garbage_never_panics() {
        for src in ["", ";;;", "{", "}", "(((", "if", "if (", "for (", "function", "x = ", "x[", "message", "a b c d", "\"unterminated", "#", "@@@", "x in |", "x = a in {1,"] {
            let _ = parse(src, Era::V21);
            let _ = parse(src, Era::V200);
        }
    }

    #[test]
    fn sample_level_npc_script() {
        let src = "// Door NPC\nif (created) {\n  setimg door.png;\n  setshape 1,32,32;\n}\nif (playertouchsme) {\n  if (!client.keyfound) {\n    say2 The door is locked.;\n  } else {\n    hide;\n    setlevel2 house.nw,32,60;\n  }\n}\n";
        assert_eq!(
            g(src),
            "(if created (block (cmd setimg \"door.png\") (cmd setshape 1 32 32)))\n\
             (if playertouchsme (block (if (! client.keyfound) (block (cmd say2 \"The door is locked.\")) (block (cmd hide) (cmd setlevel2 \"house.nw\" 32 60)))))\n"
        );
    }
}
/// GS1 parser-recovery cases from pyReborn's `test_gs1_parser_recovery.py`
/// (MIT), taken from scripts 2gta serves.
#[cfg(test)]
mod twogta_recovery {
    use super::*;

    fn g(src: &str) -> String {
        parse(src, Era::V21).to_string()
    }

    /// A block "commented out" with a `*` does nothing; before, its body ran
    /// at the top level.
    #[test]
    fn a_star_block_is_skipped_whole() {
        let src = "*if(playerenters||timeout){\ntimereverywhere;\nshowimg 300,light2.png,x-2.8,y-1.6;\ntimeout = 0.05;\n}\nthis.after = 1;";
        assert_eq!(g(src), "(block)\n(= this.after 1)\n");
    }

    /// A bad statement with a block inside an `if` does not close the `if`.
    #[test]
    fn a_bad_block_inside_an_if_keeps_the_if() {
        assert_eq!(g("if (created) { *bad{x;} this.a = 1; }\nthis.b = 2;"), "(if created (block (block) (= this.a 1)))\n(= this.b 2)\n");
    }

    #[test]
    fn colon_equals_assigns() {
        let s = parse("this.a := 3; this.b := this.a + 1;", Era::V21);
        assert!(s.diagnostics.is_empty(), "{:?}", s.diagnostics);
        assert_eq!(s.to_string(), "(= this.a 3)\n(= this.b (+ this.a 1))\n");
    }

    #[test]
    fn a_stray_top_level_brace_is_skipped() {
        assert_eq!(g("this.x=1;\n}\nthis.y=2;"), "(= this.x 1)\n(= this.y 2)\n");
    }

    #[test]
    fn a_junk_character_loses_only_its_statement() {
        assert!(g("this.a=1;\nboxy = {1\\0,10};\nthis.b=2;").ends_with("(= this.b 2)\n"));
    }
}
