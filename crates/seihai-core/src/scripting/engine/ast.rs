//! Syntax tree for GS1, and its s-expression form used by the golden tests.

use std::fmt;

/// A variable reference: a lower-case, possibly dotted name
/// (`this.hp`, `client.flag`, `local.x`, `playerx`) with optional index
/// expressions (`this.a[3]`, `board[x,y]`).
#[derive(Debug, Clone, PartialEq)]
pub struct VarRef {
    pub name: String,
    pub index: Vec<Expr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    /// Text join: `@`, and `SPC`, `NL`, `TAB` (the same with a space, a
    /// line break or a tab between).
    Cat,
    CatSpc,
    CatNl,
    CatTab,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Pow => "^",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::Le => "<=",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::Cat => "@",
            BinOp::CatSpc => "SPC",
            BinOp::CatNl => "NL",
            BinOp::CatTab => "TAB",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    /// Text. Message codes in it are expanded when it is evaluated.
    Str(String),
    Var(VarRef),
    Unary(UnOp, Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    /// A built-in or script-defined function.
    Call { name: String, args: Vec<Expr> },
    /// `x in |lo,hi|`
    InRange { e: Box<Expr>, lo: Box<Expr>, hi: Box<Expr> },
    /// `x in {a,b,c}`
    InList { e: Box<Expr>, list: Vec<Expr> },
    /// `x in this.list`: one of the values of an array (2.1 and later).
    InArray { e: Box<Expr>, name: String },
    /// `cond ? a : b` (2.1 and later).
    Cond { cond: Box<Expr>, then: Box<Expr>, els: Box<Expr> },
    /// `{a,b,c}`, the right side of an assignment (2.1 and later): fills
    /// the variable as an array.
    List(Vec<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Add,
    Sub,
    Mul,
    Div,
}

impl AssignOp {
    pub fn symbol(self) -> &'static str {
        match self {
            AssignOp::Set => "=",
            AssignOp::Add => "+=",
            AssignOp::Sub => "-=",
            AssignOp::Mul => "*=",
            AssignOp::Div => "/=",
        }
    }
}

/// A command argument: a number expression or raw text.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Num(Expr),
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Block(Vec<Stmt>),
    If { cond: Expr, then: Box<Stmt>, els: Option<Box<Stmt>> },
    While { cond: Expr, body: Box<Stmt> },
    For { init: Option<Box<Stmt>>, cond: Option<Expr>, step: Option<Box<Stmt>>, body: Box<Stmt> },
    Function { name: String, params: Vec<String>, body: Vec<Stmt> },
    Return(Option<Expr>),
    Break,
    Continue,
    Assign { target: VarRef, op: AssignOp, value: Expr },
    /// `x++` (`delta` 1) and `x--` (`delta` -1).
    IncDec { target: VarRef, delta: f64 },
    /// A function call used as a statement.
    Call { name: String, args: Vec<Expr> },
    Command { name: String, args: Vec<Arg>, line: u32 },
    /// `with (target) body` (2.1 and later): the body runs as the player or
    /// NPC `target` names. Seihai does not switch objects, so the body is
    /// skipped and the skip logged.
    With { target: Expr, body: Box<Stmt>, line: u32 },
    /// A name that is not a command in this era, with its raw arguments.
    /// Logged and ignored when run.
    Unknown { name: String, raw: String, reason: &'static str, line: u32 },
}

/// A problem found while parsing. The script still runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    pub line: u32,
    pub msg: String,
}

impl fmt::Display for Diag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.msg)
    }
}

/// A parsed script.
#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    pub body: Vec<Stmt>,
    pub diagnostics: Vec<Diag>,
}

impl fmt::Display for Script {
    /// One statement per line, in s-expression form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for s in &self.body {
            writeln!(f, "{s}")?;
        }
        Ok(())
    }
}

impl fmt::Display for VarRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.index.is_empty() {
            return write!(f, "{}", self.name);
        }
        write!(f, "(idx {}", self.name)?;
        for i in &self.index {
            write!(f, " {i}")?;
        }
        write!(f, ")")
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Num(n) => write!(f, "{}", super::value::num_to_string(*n)),
            Expr::Str(s) => write!(f, "{s:?}"),
            Expr::Var(v) => write!(f, "{v}"),
            Expr::Unary(UnOp::Neg, e) => write!(f, "(neg {e})"),
            Expr::Unary(UnOp::Not, e) => write!(f, "(! {e})"),
            Expr::Bin(op, a, b) => write!(f, "({} {a} {b})", op.symbol()),
            Expr::Call { name, args } => {
                write!(f, "(call {name}")?;
                for a in args {
                    write!(f, " {a}")?;
                }
                write!(f, ")")
            }
            Expr::InRange { e, lo, hi } => write!(f, "(in {e} range {lo} {hi})"),
            Expr::InList { e, list } => {
                write!(f, "(in {e} list")?;
                for a in list {
                    write!(f, " {a}")?;
                }
                write!(f, ")")
            }
            Expr::Cond { cond, then, els } => write!(f, "(? {cond} {then} {els})"),
            Expr::InArray { e, name } => write!(f, "(in {e} array {name})"),
            Expr::List(list) => {
                write!(f, "(list")?;
                for a in list {
                    write!(f, " {a}")?;
                }
                write!(f, ")")
            }
        }
    }
}

impl fmt::Display for Arg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Arg::Num(e) => write!(f, "{e}"),
            Arg::Text(t) => write!(f, "{t:?}"),
        }
    }
}

impl fmt::Display for Stmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stmt::Block(v) => {
                write!(f, "(block")?;
                for s in v {
                    write!(f, " {s}")?;
                }
                write!(f, ")")
            }
            Stmt::If { cond, then, els } => match els {
                Some(e) => write!(f, "(if {cond} {then} {e})"),
                None => write!(f, "(if {cond} {then})"),
            },
            Stmt::While { cond, body } => write!(f, "(while {cond} {body})"),
            Stmt::For { init, cond, step, body } => {
                let o = |s: &Option<Box<Stmt>>| s.as_ref().map_or("_".to_string(), |s| s.to_string());
                let c = cond.as_ref().map_or("_".to_string(), |c| c.to_string());
                write!(f, "(for {} {c} {} {body})", o(init), o(step))
            }
            Stmt::Function { name, params, body } => {
                write!(f, "(fn {name} ({})", params.join(" "))?;
                for s in body {
                    write!(f, " {s}")?;
                }
                write!(f, ")")
            }
            Stmt::Return(Some(e)) => write!(f, "(return {e})"),
            Stmt::Return(None) => write!(f, "(return)"),
            Stmt::Break => write!(f, "(break)"),
            Stmt::Continue => write!(f, "(continue)"),
            Stmt::Assign { target, op, value } => write!(f, "({} {target} {value})", op.symbol()),
            Stmt::IncDec { target, delta } => write!(f, "({} {target})", if *delta > 0.0 { "++" } else { "--" }),
            Stmt::Call { name, args } => {
                write!(f, "(call {name}")?;
                for a in args {
                    write!(f, " {a}")?;
                }
                write!(f, ")")
            }
            Stmt::Command { name, args, .. } => {
                write!(f, "(cmd {name}")?;
                for a in args {
                    write!(f, " {a}")?;
                }
                write!(f, ")")
            }
            Stmt::With { target, body, .. } => write!(f, "(with {target} {body})"),
            Stmt::Unknown { name, raw, reason, .. } => write!(f, "(unknown {name} {raw:?} {reason})"),
        }
    }
}
