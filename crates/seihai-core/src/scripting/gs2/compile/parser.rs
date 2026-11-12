//! The GS2 parser.
//!
//! One grammar for both modes. Wherever live scripts lean on a form the
//! language does not have (a missing `;`, `=` used as a comparison, a GS1
//! command line, …), the parser reads it the way the live servers did and
//! records a [`Quirk`]. The mode decides afterwards what a quirk is: an
//! error in strict mode, a warning in legacy mode. A script with no quirks
//! gives the same tree in both.

use super::ast::*;
use super::gs1;
use super::lexer::{Lexer, Tok};
use super::{CompileError, Quirk};
use crate::scripting::engine::{self, vocab, Era};
use std::collections::HashMap;

type PResult<T> = Result<T, CompileError>;

/// What the parser read: the program, and the quirks it met on the way.
pub(super) struct Parsed {
    pub program: Program,
    pub spellings: super::lexer::Spellings,
    pub quirks: Vec<(u32, Quirk)>,
}

pub(super) fn parse(src: &str) -> PResult<Parsed> {
    let mut p = Parser {
        lex: Lexer::new(src),
        consts: HashMap::new(),
        lambdas: Vec::new(),
        nested: Vec::new(),
        new_body: None,
        comma: false,
        quirks: Vec::new(),
    };
    let mut items: Vec<Item> = Vec::new();
    loop {
        p.lex.skip();
        if p.eof() {
            break;
        }
        if p.eat(";") {
            p.quirk(Quirk::EmptyStatement);
            continue;
        }
        let public = p.eat_id("public");
        if p.eat_id("function") {
            let mut f = p.function()?;
            if public {
                f.name = format!("public.{}", f.name);
            }
            items.push(Item::Function(f));
            continue;
        }
        if public {
            return Err(p.error("expected `function` after `public`"));
        }
        let s = p.stmt()?;
        if s.is_empty() {
            continue;
        }
        match items.last_mut() {
            Some(Item::Code(code)) => code.push(s),
            _ => items.push(Item::Code(vec![s])),
        }
    }
    // Functions written inside a block are functions of the script, after
    // the others.
    items.extend(p.nested.into_iter().map(Item::Function));
    let mut quirks = p.lex.quirks;
    quirks.append(&mut p.quirks);
    quirks.sort_by_key(|(line, _)| *line);
    Ok(Parsed {
        program: Program {
            items,
            lambdas: p.lambdas,
            later_wins: true,
        },
        spellings: p.lex.spellings,
        quirks,
    })
}

struct Parser<'a> {
    lex: Lexer<'a>,
    /// `const` and `enum` names (lower case), folded where they are read.
    consts: HashMap<String, Expr>,
    /// `function (…) { … }` values met so far.
    lambdas: Vec<Function>,
    /// Functions written inside a block.
    nested: Vec<Function>,
    /// The body of a `new T(name) { … }` the expression parser just read,
    /// for the statement parser to pick up.
    new_body: Option<Vec<Stmt>>,
    /// The last statement ended at a `,`: the next one belongs with it.
    comma: bool,
    quirks: Vec<(u32, Quirk)>,
}

impl<'a> Parser<'a> {
    fn quirk(&mut self, q: Quirk) {
        self.quirks.push((self.lex.line, q));
    }

    fn error(&self, message: impl Into<String>) -> CompileError {
        CompileError {
            line: self.lex.line,
            message: message.into(),
        }
    }

    // ---- tokens ----

    fn eof(&mut self) -> bool {
        matches!(self.lex.peek(), Tok::Eof)
    }

    fn sees(&mut self, op: &str) -> bool {
        matches!(self.lex.peek(), Tok::Op(s) if s == op)
    }

    fn eat(&mut self, op: &str) -> bool {
        if self.sees(op) {
            self.lex.bump();
            true
        } else {
            false
        }
    }

    fn sees_id(&mut self, name: &str) -> bool {
        matches!(self.lex.peek(), Tok::Id(ref s) if s == name)
    }

    fn eat_id(&mut self, name: &str) -> bool {
        self.lex.skip();
        if self.sees_id(name) {
            self.lex.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, op: &str) -> PResult<()> {
        if self.eat(op) {
            Ok(())
        } else {
            Err(self.error(format!("expected `{op}`")))
        }
    }

    fn ident(&mut self) -> PResult<String> {
        match self.lex.bump() {
            Tok::Id(s) => Ok(s),
            _ => Err(self.error("expected a name")),
        }
    }

    /// The `;` that ends a statement.
    fn end_stmt(&mut self) -> PResult<()> {
        if self.eat(",") {
            self.quirk(Quirk::CommaStatements);
            self.comma = true;
            return Ok(());
        }
        if self.eat(";") {
            return Ok(());
        }
        let next = self.lex.peek();
        if self.lex.peek_line > self.lex.last_line || matches!(next, Tok::Op("}") | Tok::Eof) {
            self.quirk(Quirk::MissingSemicolon);
            return Ok(());
        }
        Err(self.error("expected `;`"))
    }

    // ---- functions ----

    /// A function or parameter name: dotted (`Obj.onAction`, `temp.x`), or
    /// a string literal (a quirk).
    fn dotted_ident(&mut self) -> PResult<String> {
        if let Tok::Str(text) = self.lex.peek() {
            self.lex.bump();
            self.quirk(Quirk::StringName);
            return Ok(text);
        }
        let mut name = self.ident()?;
        while self.eat(".") {
            name.push('.');
            name.push_str(&self.ident()?);
        }
        Ok(name)
    }

    fn params(&mut self) -> PResult<Vec<String>> {
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
        Ok(params)
    }

    /// `name(params) { body }`, after `function`.
    fn function(&mut self) -> PResult<Function> {
        let name = self.dotted_ident()?;
        self.expect("(")?;
        let params = self.params()?;
        let body = if self.eat("{") {
            self.block_body()?
        } else if self.eat(";") {
            if self.eat("{") {
                self.quirk(Quirk::DeclarationBeforeBody);
                self.block_body()?
            } else {
                self.quirk(Quirk::MissingFunctionBody);
                Vec::new()
            }
        } else if self.sees_id("function") || self.sees_id("public") || self.eof() {
            self.quirk(Quirk::MissingFunctionBody);
            Vec::new()
        } else {
            self.quirk(Quirk::BracelessFunctionBody);
            vec![self.stmt()?]
        };
        Ok(Function {
            name,
            params,
            body,
            cmd_call: None,
        })
    }

    /// The statements up to the `}` that closes a block.
    fn block_body(&mut self) -> PResult<Vec<Stmt>> {
        let mut body = Vec::new();
        loop {
            self.lex.skip();
            if self.eat("}") {
                break;
            }
            if self.eof() {
                self.quirk(Quirk::UnclosedBlock);
                break;
            }
            if self.eat(";") {
                self.quirk(Quirk::EmptyStatement);
                continue;
            }
            body.push(self.stmt()?);
        }
        Ok(body)
    }

    /// The next token is `function` and a name follows it (not a
    /// `function () { … }` value).
    fn sees_function_header(&mut self) -> bool {
        if !self.sees_id("function") {
            return false;
        }
        let rest = &self.lex.src[self.lex.i..];
        let start = rest.iter().position(|c| !c.is_ascii_whitespace());
        start.is_some_and(|k| rest[k].is_ascii_alphabetic() || rest[k] == b'_')
    }

    // ---- statements ----

    /// One statement. Statements joined by commas (`a.x = 1, b.x = 2;`, a
    /// quirk) are one statement, so a brace-less body keeps every part.
    fn stmt(&mut self) -> PResult<Stmt> {
        if self.eat(";") {
            self.quirk(Quirk::EmptyStatement);
            return Ok(Stmt::empty());
        }
        self.comma = false;
        let first = self.one_stmt()?;
        if !self.comma {
            return Ok(first);
        }
        let mut all = vec![first];
        while std::mem::take(&mut self.comma) {
            all.push(self.one_stmt()?);
        }
        Ok(Stmt::Block(all))
    }

    fn one_stmt(&mut self) -> PResult<Stmt> {
        let s = self.one_stmt_inner()?;
        if self.new_body.take().is_some() {
            return Err(self.error("`new T(name) { … }` must stand alone as a statement"));
        }
        Ok(s)
    }

    fn one_stmt_inner(&mut self) -> PResult<Stmt> {
        if let Some(s) = self.gs1_command()? {
            return Ok(s);
        }
        if self.eat_id("if") {
            return self.if_stmt();
        }
        if self.eat_id("while") {
            self.expect("(")?;
            let cond = self.cond()?;
            self.expect(")")?;
            let body = Box::new(self.stmt()?);
            return Ok(Stmt::While { cond, body });
        }
        if self.eat_id("for") {
            return self.for_stmt();
        }
        if self.eat_id("return") {
            let v = if self.sees(";") || self.sees("}") {
                None
            } else {
                Some(self.assigned_value()?)
            };
            self.end_stmt()?;
            return Ok(Stmt::Return(v));
        }
        if self.eat_id("break") {
            self.end_stmt()?;
            return Ok(Stmt::Break);
        }
        if self.eat_id("continue") {
            self.end_stmt()?;
            return Ok(Stmt::Continue);
        }
        if self.eat("{") {
            return Ok(Stmt::Block(self.block_body()?));
        }
        if self.eat_id("with") {
            let line = self.lex.line;
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
            // `const NAME = value;`: folded where it is read, no code.
            loop {
                let name = self.ident()?;
                self.expect("=")?;
                let value = self.const_value()?;
                self.consts.insert(name.to_ascii_lowercase(), value);
                if !self.eat(",") {
                    break;
                }
            }
            self.end_stmt()?;
            return Ok(Stmt::empty());
        }
        if self.eat_id("global") {
            if !self.sees_id("enum") {
                return Err(self.error("expected `enum` after `global`"));
            }
            self.quirk(Quirk::GlobalEnum);
        }
        if self.eat_id("enum") {
            self.enum_decl()?;
            return Ok(Stmt::empty());
        }
        if self.sees_id("public") || self.sees_function_header() {
            let public = self.eat_id("public");
            if !self.eat_id("function") {
                return Err(self.error("expected `function` after `public`"));
            }
            self.quirk(Quirk::NestedFunction);
            let mut f = self.function()?;
            if public {
                f.name = format!("public.{}", f.name);
            }
            self.nested.push(f);
            return Ok(Stmt::empty());
        }
        if self.sees("++") || self.sees("--") {
            let delta = if self.eat("++") {
                1.0
            } else {
                self.lex.bump();
                -1.0
            };
            let Expr::Var(target) = self.unary()? else {
                return Err(self.error("`++`/`--` needs a variable"));
            };
            self.end_stmt()?;
            return Ok(Stmt::IncDec { target, delta });
        }
        let e = self.expr()?;
        self.expr_stmt(e)
    }

    /// A statement that starts with an expression: an assignment, a step,
    /// a call, or (a quirk) any other value.
    fn expr_stmt(&mut self, mut e: Expr) -> PResult<Stmt> {
        match e {
            Expr::PostStep { target, delta } | Expr::PreStep { target, delta } => {
                self.end_stmt()?;
                return Ok(match *target {
                    Expr::Var(target) => Stmt::IncDec { target, delta },
                    // `a[i][j]++;`: the place, the step, the drop.
                    target => Stmt::Eval(Expr::PreStep {
                        target: Box::new(target),
                        delta,
                    }),
                });
            }
            _ => {}
        }
        if let Some(body) = self.new_body.take() {
            return match e {
                Expr::New { ty, mut args } if args.len() == 1 => {
                    self.eat(";");
                    Ok(Stmt::NewBlock {
                        ty,
                        name: args.remove(0),
                        body,
                    })
                }
                Expr::New { .. } => Err(self.error("`new T(name) { … }` needs one name")),
                _ => Err(self.error("`new T(name) { … }` must stand alone as a statement")),
            };
        }
        if let Expr::Var(v) = &e {
            if let Some(op) = self.assign_op() {
                let mut value = self.expr()?;
                if op == AssignOp::Set && matches!(value, Expr::Var(_)) && self.sees("=") {
                    // `a = b = c = v`
                    let mut targets = vec![v.clone()];
                    while let Expr::Var(t) = &value {
                        if !self.eat("=") {
                            break;
                        }
                        targets.push(t.clone());
                        value = self.expr()?;
                    }
                    self.end_stmt()?;
                    return Ok(Stmt::Chain { targets, value });
                }
                // `a = makevar(…) += v`: the inner store is a value.
                if let Some(inner) = self.assign_op() {
                    let v2 = self.expr()?;
                    value = Expr::AssignValue {
                        target: Box::new(value),
                        op: inner,
                        value: Box::new(v2),
                    };
                }
                self.end_stmt()?;
                return Ok(Stmt::Assign {
                    target: v.clone(),
                    op,
                    value,
                });
            }
            // `a @= b` is `a = a @ b`.
            if self.eat("@=") {
                let value = self.expr()?;
                self.end_stmt()?;
                return Ok(Stmt::Assign {
                    target: v.clone(),
                    op: AssignOp::Set,
                    value: Expr::bin(BinOp::Cat, e.clone(), value),
                });
            }
        }
        // `(@"a" @ n) = v`: a variable named at run time, read as
        // `makevar(@"a" @ n) = v`.
        let joined = matches!(&e, Expr::Binary(BinOp::Cat, ..) | Expr::Text(_));
        if joined && matches!(self.lex.peek(), Tok::Op("=")) {
            self.quirk(Quirk::RuntimeNameAssign);
            e = Expr::Call {
                name: "makevar".into(),
                args: vec![e],
            };
        }
        if e.is_part_place() {
            if self.eat("@=") {
                let value = self.expr()?;
                self.end_stmt()?;
                let value = Expr::bin(BinOp::Cat, e.clone(), value);
                return Ok(Stmt::SetPart { target: e, value });
            }
            if let Some(op) = self.assign_op() {
                let mut value = self.assigned_value()?;
                // `obj.x += v` is `obj.x = obj.x + v`.
                if op != AssignOp::Set {
                    value = op.expand(e.clone(), value);
                }
                self.end_stmt()?;
                return Ok(Stmt::SetPart { target: e, value });
            }
        }
        self.end_stmt()?;
        Ok(match e {
            Expr::Call { name, args } => Stmt::Call { name, args },
            e @ (Expr::Method { .. }
            | Expr::DynMethod { .. }
            | Expr::DynCall { .. }
            | Expr::New { .. }
            | Expr::AssignValue { .. }) => Stmt::Eval(e),
            other => {
                self.quirk(Quirk::BareExpression);
                Stmt::Eval(other)
            }
        })
    }

    /// A GS1 command written GS1-style in a GS2 script (a quirk): a GS1
    /// command name followed by its arguments without brackets, or by `;`
    /// alone (`setimgpart a.png,0,0,8,8;`, `hide;`). The live servers ran
    /// such scripts, so the text up to `;` goes through the GS1 parser and
    /// is lowered as GS1.
    fn gs1_command(&mut self) -> PResult<Option<Stmt>> {
        let Tok::Id(name) = self.lex.peek() else {
            return Ok(None);
        };
        if vocab::command(&name.to_ascii_lowercase()).is_none() {
            return Ok(None);
        }
        let b = self.lex.src;
        // The name was just lexed, so it ends at the cursor.
        let start = self.lex.i - name.len();
        let mut j = self.lex.i;
        while matches!(b.get(j), Some(b' ' | b'\t')) {
            j += 1;
        }
        match b.get(j) {
            Some(b';') => {}
            Some(c)
                if c.is_ascii_alphanumeric() || matches!(c, b'#' | b'"' | b'-' | b'_' | b'.') =>
            {
                // `name.x`, `name--` are GS2.
                if *c == b'.' || *c == b'-' && b.get(j + 1) == Some(&b'-') {
                    return Ok(None);
                }
            }
            _ => return Ok(None),
        }
        let mut end = j;
        let mut quote = None;
        let mut lines = 0;
        while let Some(&c) = b.get(end) {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (None, b'"') => quote = Some(c),
                (None, b';') => break,
                (None, b'{' | b'}') => return Ok(None),
                (_, b'\n') => lines += 1,
                _ => {}
            }
            end += 1;
        }
        if b.get(end) != Some(&b';') {
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&b[start..=end]).into_owned();
        let parsed = engine::parse(&text, Era::V21);
        if !parsed.diagnostics.is_empty() {
            return Ok(None);
        }
        let mut body = parsed.body;
        if body.len() != 1 || !matches!(body[0], engine::ast::Stmt::Command { .. }) {
            return Ok(None);
        }
        let mut command = body.remove(0);
        let line = self.lex.line;
        if let engine::ast::Stmt::Command { line: l, .. } = &mut command {
            *l = line;
        }
        self.quirk(Quirk::Gs1Command(name.to_ascii_lowercase()));
        let lowered = gs1::lower_embedded(&command, Era::V21)
            .map_err(|message| CompileError { line, message })?;
        self.lex.reset(end + 1, lines);
        Ok(Some(lowered))
    }

    /// `enum Name { A, B = 5, C }`, after `enum`: numbers from 0, or from a
    /// given value, folded where they are read like `const`.
    fn enum_decl(&mut self) -> PResult<()> {
        let prefix = if self.sees("{") {
            None
        } else {
            Some(self.ident()?.to_ascii_lowercase())
        };
        self.expect("{")?;
        let mut next = 0.0;
        while !self.eat("}") {
            let name = self.ident()?;
            // `A = "x"` makes the member that text (a quirk); numbering
            // carries on from the last number.
            let mut value = None;
            if self.eat("=") {
                match self.const_value()? {
                    Expr::Num(n) => next = n,
                    v => {
                        self.quirk(Quirk::EnumText);
                        value = Some(v)
                    }
                }
            }
            let value = value.unwrap_or_else(|| {
                next += 1.0;
                Expr::Num(next - 1.0)
            });
            let lower = name.to_ascii_lowercase();
            if let Some(p) = &prefix {
                self.consts.insert(format!("{p}::{lower}"), value.clone());
            }
            self.consts.insert(lower, value);
            if !self.eat(",") {
                self.expect("}")?;
                break;
            }
        }
        self.eat(";");
        Ok(())
    }

    /// An `if`/`while` condition. A single `=` there compares, as in GS1
    /// (`if (key = "m")`, a quirk).
    fn cond(&mut self) -> PResult<Expr> {
        let e = self.expr()?;
        if self.eat("=") {
            self.quirk(Quirk::AssignInCondition);
            let rhs = self.expr()?;
            return Ok(Expr::bin(BinOp::Eq, e, rhs));
        }
        Ok(e)
    }

    fn if_stmt(&mut self) -> PResult<Stmt> {
        self.expect("(")?;
        let cond = self.cond()?;
        self.expect(")")?;
        let then = Box::new(self.stmt()?);
        let els = if self.eat_id("else") {
            Some(Box::new(self.stmt()?))
        } else if self.eat_id("elseif") {
            self.quirk(Quirk::ElseIf);
            Some(Box::new(self.if_stmt()?))
        } else {
            None
        };
        Ok(Stmt::If { cond, then, els })
    }

    fn for_stmt(&mut self) -> PResult<Stmt> {
        self.expect("(")?;
        let init = if self.eat(";") {
            None
        } else {
            let first = self.expr()?;
            if self.eat(":") {
                let Expr::Var(var) = first else {
                    return Err(self.error("a for-each variable must be a name"));
                };
                if !var.index.is_empty() {
                    return Err(self.error("a for-each variable must be a plain name"));
                }
                let list = self.expr()?;
                self.expect(")")?;
                let body = Box::new(self.stmt()?);
                return Ok(Stmt::ForEach {
                    var: var.name,
                    list,
                    body,
                });
            }
            let init = self.simple_from(first)?;
            self.expect(";")?;
            Some(Box::new(init))
        };
        let cond = if self.eat(";") {
            None
        } else {
            let c = self.expr()?;
            self.expect(";")?;
            Some(c)
        };
        let step = if self.eat(")") {
            None
        } else {
            let e = self.expr()?;
            let s = self.simple_from(e)?;
            self.expect(")")?;
            Some(Box::new(s))
        };
        let body = Box::new(self.stmt()?);
        Ok(Stmt::For {
            init,
            cond,
            step,
            body,
        })
    }

    /// The init or step of a `for`: an assignment or a step.
    fn simple_from(&mut self, e: Expr) -> PResult<Stmt> {
        if let Expr::PostStep { target, delta } | Expr::PreStep { target, delta } = &e {
            if let Expr::Var(v) = target.as_ref() {
                return Ok(Stmt::IncDec {
                    target: v.clone(),
                    delta: *delta,
                });
            }
        }
        let Expr::Var(v) = e else {
            return Err(self.error("for init and step must assign a variable"));
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
            return Err(self.error("expected an assignment"));
        };
        let value = self.expr()?;
        Ok(Stmt::Assign {
            target: v,
            op,
            value,
        })
    }

    fn switch_stmt(&mut self) -> PResult<Stmt> {
        self.expect("(")?;
        let value = self.expr()?;
        self.expect(")")?;
        self.expect("{")?;
        let mut arms: Vec<Arm> = Vec::new();
        loop {
            self.lex.skip();
            if self.eat("}") {
                break;
            }
            if self.eat_id("case") {
                let v = self.expr()?;
                self.expect(":")?;
                arms.push(Arm {
                    values: vec![v],
                    default: false,
                    body: Vec::new(),
                });
                continue;
            }
            if self.eat_id("default") {
                self.expect(":")?;
                arms.push(Arm {
                    values: Vec::new(),
                    default: true,
                    body: Vec::new(),
                });
                continue;
            }
            let s = self.stmt()?;
            match arms.last_mut() {
                Some(arm) => arm.body.push(s),
                None => return Err(self.error("a statement before the first `case`")),
            }
        }
        Ok(Stmt::Switch { value, arms })
    }

    // ---- expressions ----

    fn expr(&mut self) -> PResult<Expr> {
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

    /// The right side of an assignment, where it may itself be an
    /// assignment (`a.x = b.x = v`, `a = f(1).y += v`): those inner stores
    /// are values, innermost first.
    fn assigned_value(&mut self) -> PResult<Expr> {
        let value = self.expr()?;
        if matches!(value, Expr::Var(_)) || value.is_part_place() {
            if let Some(op) = self.assign_op() {
                let inner = self.assigned_value()?;
                return Ok(Expr::AssignValue {
                    target: Box::new(value),
                    op,
                    value: Box::new(inner),
                });
            }
        }
        Ok(value)
    }

    /// Binary operators at or above precedence `min`, left to right.
    fn binary(&mut self, min: u8) -> PResult<Expr> {
        let mut lhs = self.unary()?;
        loop {
            // `x in …`, at the equality level.
            if min <= 6 && self.eat_id("in") {
                lhs = self.in_tail(lhs)?;
                continue;
            }
            let Some((op, prec)) = self.bin_op(min) else {
                break;
            };
            self.lex.bump();
            let rhs = self.binary(prec + 1)?;
            lhs = match op {
                Either::Bin(op) => Expr::bin(op, lhs, rhs),
                Either::Bits(op) => Expr::Bits(op, Box::new(lhs), Box::new(rhs)),
            };
        }
        Ok(lhs)
    }

    /// What follows `x in`: `|lo, hi|`, GS1's `<lo, hi>` (a quirk), `{a, b}`
    /// or any value.
    fn in_tail(&mut self, lhs: Expr) -> PResult<Expr> {
        let angle = self.sees("<");
        if self.eat("|") || self.eat("<") {
            if angle {
                self.quirk(Quirk::Gs1Range);
            }
            let lo = self.binary(if angle { 7 } else { 4 })?;
            self.expect(",")?;
            let hi = self.binary(if angle { 7 } else { 4 })?;
            self.expect(if angle { ">" } else { "|" })?;
            return Ok(Expr::InRange {
                e: Box::new(lhs),
                lo: Box::new(lo),
                hi: Box::new(hi),
            });
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
            return Ok(Expr::InList {
                e: Box::new(lhs),
                list,
            });
        }
        Ok(match self.unary()? {
            Expr::Var(v) if v.index.is_empty() => Expr::InArray {
                e: Box::new(lhs),
                name: v.name,
            },
            other => Expr::InValue {
                e: Box::new(lhs),
                list: Box::new(other),
            },
        })
    }

    /// The binary operator next, if its precedence is at least `min`.
    /// Precedence, low to high: `||`; `&&`; `@ SPC NL TAB & |`; `<`;
    /// `>`; `== != in`; `<< >>`; `+ -`; `* / % ^`.
    fn bin_op(&mut self, min: u8) -> Option<(Either, u8)> {
        let text: &'static str = match self.lex.peek() {
            Tok::Op(text) => text,
            Tok::Id(id) if id == "SPC" => "SPC",
            Tok::Id(id) if id == "NL" => "NL",
            Tok::Id(id) if id == "TAB" => "TAB",
            _ => return None,
        };
        let (prec, op) = match text {
            "||" => (1, Either::Bin(BinOp::Or)),
            "&&" => (2, Either::Bin(BinOp::And)),
            "@" => (3, Either::Bin(BinOp::Cat)),
            "SPC" => (3, Either::Bin(BinOp::CatSpc)),
            "NL" => (3, Either::Bin(BinOp::CatNl)),
            "TAB" => (3, Either::Bin(BinOp::CatTab)),
            // Below `<` so that `x in |a, b|` still closes its range.
            "&" => (3, Either::Bits(BitOp::And)),
            "|" => (3, Either::Bits(BitOp::Or)),
            "<" => (4, Either::Bin(BinOp::Lt)),
            "<=" | "=<" => (4, Either::Bin(BinOp::Le)),
            ">" => (5, Either::Bin(BinOp::Gt)),
            ">=" | "=>" => (5, Either::Bin(BinOp::Ge)),
            "==" => (6, Either::Bin(BinOp::Eq)),
            "!=" | "<>" => (6, Either::Bin(BinOp::Ne)),
            "<<" => (7, Either::Bits(BitOp::Shl)),
            ">>" => (7, Either::Bits(BitOp::Shr)),
            "+" => (8, Either::Bin(BinOp::Add)),
            "-" => (8, Either::Bin(BinOp::Sub)),
            "*" => (9, Either::Bin(BinOp::Mul)),
            "/" => (9, Either::Bin(BinOp::Div)),
            "%" => (9, Either::Bin(BinOp::Mod)),
            "^" => (9, Either::Bin(BinOp::Pow)),
            _ => return None,
        };
        if prec < min {
            return None;
        }
        if matches!(text, "=<" | "=>" | "<>") {
            self.quirk(Quirk::OldOperator(text));
        }
        Some((op, prec))
    }

    fn unary(&mut self) -> PResult<Expr> {
        if self.sees("++") || self.sees("--") {
            let delta = if self.eat("++") {
                1.0
            } else {
                self.lex.bump();
                -1.0
            };
            let target = self.unary()?;
            return Ok(Expr::PreStep {
                target: Box::new(target),
                delta,
            });
        }
        if self.eat("!") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        if self.eat("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.eat("+") {
            return self.unary();
        }
        let e = self.atom()?;
        self.postfix(e)
    }

    /// `.name`, `.name(args)`, `.(expr)`, `[i]`, `++`/`--` and `(args)`
    /// after a value.
    fn postfix(&mut self, mut e: Expr) -> PResult<Expr> {
        loop {
            if self.eat(".") {
                if self.eat("(") {
                    let n = Box::new(self.expr()?);
                    self.expect(")")?;
                    let obj = Box::new(e);
                    e = if self.eat("(") {
                        let args = self.arg_list()?;
                        Expr::DynMethod { obj, name: n, args }
                    } else {
                        Expr::DynMember { obj, name: n }
                    };
                    continue;
                }
                let name = self.ident()?;
                let obj = Box::new(e);
                e = if self.eat("(") {
                    let args = self.arg_list()?;
                    Expr::Method { obj, name, args }
                } else {
                    Expr::Member { obj, name }
                };
            } else if self.sees("[") && !matches!(e, Expr::Num(_)) {
                self.lex.bump();
                let index = self.index_list()?;
                e = Expr::Index {
                    obj: Box::new(e),
                    index,
                };
            } else if self.sees("++") || self.sees("--") {
                let delta = if self.eat("++") {
                    1.0
                } else {
                    self.lex.bump();
                    -1.0
                };
                e = Expr::PostStep {
                    target: Box::new(e),
                    delta,
                };
            } else if self.sees("(") && e.is_call() {
                // `f("x")()`: call what the call returned, by name.
                self.quirk(Quirk::CallOfCallResult);
                self.lex.bump();
                let args = self.arg_list()?;
                e = Expr::DynCall {
                    name: Box::new(e),
                    args,
                };
            } else {
                return Ok(e);
            }
        }
    }

    /// `i, j]` after `[`.
    fn index_list(&mut self) -> PResult<Vec<Expr>> {
        let mut index = Vec::new();
        loop {
            index.push(self.expr()?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect("]")?;
        Ok(index)
    }

    fn atom(&mut self) -> PResult<Expr> {
        match self.lex.bump() {
            Tok::Num(n) => Ok(Expr::Num(n)),
            Tok::Str(s) => Ok(Expr::Str(s)),
            Tok::Id(id) if id == "new" => self.new_expr(),
            Tok::Id(id) if id == "function" && self.sees("(") => self.lambda(),
            Tok::Id(name) => self.name_expr(name),
            Tok::Op("(") => {
                let mut e = self.expr()?;
                // `(x = v)`, `(x += v)`: an assignment used as a value.
                if let Some(op) = self.assign_op() {
                    let value = self.expr()?;
                    e = Expr::AssignValue {
                        target: Box::new(e),
                        op,
                        value: Box::new(value),
                    };
                }
                self.expect(")")?;
                // `(@"name" @ n)(args)`: a call by a name worked out at run
                // time.
                if self.eat("(") {
                    let args = self.arg_list()?;
                    return Ok(Expr::DynCall {
                        name: Box::new(e),
                        args,
                    });
                }
                Ok(e)
            }
            Tok::Code(code) => {
                self.quirk(Quirk::Gs1MessageCode);
                match gs1::message(&code, Era::V21) {
                    Ok(parts) => Ok(Expr::Message(parts)),
                    Err(message) => Err(self.error(message)),
                }
            }
            Tok::Op("@") => Ok(Expr::Text(Box::new(self.unary()?))),
            Tok::Op("{") => {
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
            Tok::Op("|") => Err(self.error("`|` starts no value (`in` ranges are `x in |a, b|`)")),
            _ => Err(self.error("expected an expression")),
        }
    }

    /// After `new`: `new[n][m]`, or `new Type(args)` with an optional
    /// `{ body }` for the statement to pick up.
    fn new_expr(&mut self) -> PResult<Expr> {
        if self.sees("[") {
            let mut dims = Vec::new();
            while self.eat("[") {
                dims.push(self.expr()?);
                self.expect("]")?;
            }
            return Ok(Expr::NewArray(dims));
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
        Ok(Expr::New { ty, args })
    }

    /// `function (params) { … }` as a value, after `function`: a public
    /// function named `function_<line>_<n>`, emitted where it is used.
    fn lambda(&mut self) -> PResult<Expr> {
        self.lex.bump();
        let params = self.params()?;
        self.expect("{")?;
        let body = self.block_body()?;
        let name = format!(
            "function_{}_{}",
            self.lex.line,
            self.lambdas.len() + self.nested.len() + 1
        );
        self.lambdas.push(Function {
            name: format!("public.{name}"),
            params,
            body,
            cmd_call: None,
        });
        Ok(Expr::Lambda(name))
    }

    /// A name: a call `f(…)`, a folded constant, a dotted name with an
    /// optional index, or a method or run-time member on a dotted name.
    fn name_expr(&mut self, mut name: String) -> PResult<Expr> {
        if self.eat("(") {
            let args = self.arg_list()?;
            return Ok(Expr::Call { name, args });
        }
        if let Some(v) = self.consts.get(&name.to_ascii_lowercase()).cloned() {
            if !self.sees(".") && !self.sees("[") {
                return Ok(v);
            }
        }
        while self.eat(".") {
            if self.eat("(") {
                // `a.b.(expr)`: a member named by a value.
                let n = Box::new(self.expr()?);
                self.expect(")")?;
                let obj = Box::new(Expr::var(name));
                if self.eat("(") {
                    let args = self.arg_list()?;
                    return Ok(Expr::DynMethod { obj, name: n, args });
                }
                return Ok(Expr::DynMember { obj, name: n });
            }
            let part = self.ident()?;
            if self.eat("(") {
                // `a.b.f(x)`: a method on the dotted name before it.
                let args = self.arg_list()?;
                return Ok(Expr::Method {
                    obj: Box::new(Expr::var(name)),
                    name: part,
                    args,
                });
            }
            name.push('.');
            name.push_str(&part);
        }
        let index = if self.eat("[") {
            self.index_list()?
        } else {
            Vec::new()
        };
        Ok(Expr::Var(Var { name, index }))
    }

    /// A `const` or `enum` value. A negative number is folded; any other
    /// value is put in where the name is read.
    fn const_value(&mut self) -> PResult<Expr> {
        Ok(match self.expr()? {
            Expr::Neg(n) if matches!(*n, Expr::Num(_)) => match *n {
                Expr::Num(x) => Expr::Num(-x),
                _ => unreachable!(),
            },
            other => other,
        })
    }

    /// `a, b)` after `(`.
    fn arg_list(&mut self) -> PResult<Vec<Expr>> {
        let mut args = Vec::new();
        if self.eat(")") {
            return Ok(args);
        }
        loop {
            match self.slash_word() {
                Some(word) => args.push(Expr::Str(word)),
                None => args.push(self.expr()?),
            }
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        Ok(args)
    }

    /// An argument written as unquoted `/word` (`strequals(#c, /sit)`, as
    /// GS1 writes text arguments; a quirk), read as the string `"/word"`.
    fn slash_word(&mut self) -> Option<String> {
        if !self.sees("/") {
            return None;
        }
        let rest = &self.lex.src[self.lex.i..];
        let n = rest
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
            .count();
        let after = rest[n..].iter().find(|c| !c.is_ascii_whitespace());
        if n == 0 || !rest[0].is_ascii_alphabetic() || !matches!(after, Some(b',' | b')')) {
            return None;
        }
        let word = format!("/{}", String::from_utf8_lossy(&rest[..n]));
        self.lex.bump();
        self.lex.i += n;
        self.quirk(Quirk::SlashWord(word.clone()));
        Some(word)
    }

    fn assign_op(&mut self) -> Option<AssignOp> {
        let op = match self.lex.peek() {
            Tok::Op("=") => AssignOp::Set,
            Tok::Op(":=") => {
                self.quirk(Quirk::OldOperator(":="));
                AssignOp::Set
            }
            Tok::Op("+=") => AssignOp::Add,
            Tok::Op("-=") => AssignOp::Sub,
            Tok::Op("*=") => AssignOp::Mul,
            Tok::Op("/=") => AssignOp::Div,
            Tok::Op("%=") => AssignOp::Mod,
            Tok::Op("&=") => AssignOp::BitAnd,
            Tok::Op("|=") => AssignOp::BitOr,
            _ => return None,
        };
        self.lex.bump();
        Some(op)
    }
}

enum Either {
    Bin(BinOp),
    Bits(BitOp),
}
