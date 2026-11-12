//! Syntax tree for GS1, and its s-expression form used by the golden tests.

use std::fmt;

/// A variable reference: a lower-case, possibly dotted name
/// (`this.hp`, `client.flag`, `local.x`, `playerx`) with optional index
/// expressions (`this.a[3]`, `board[x,y]`).
#[derive(Debug, Clone, PartialEq)]
pub struct VarRef {
    pub name: String,
    pub index: Vec<Expr>,
    /// `true` when a `.field` follows the index (`npcs[i].x`): the field is
    /// folded into `name`, so the index applies to the part before it.
    pub field_after_index: bool,
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
    Call {
        name: String,
        args: Vec<Expr>,
    },
    /// `x in |lo,hi|`
    InRange {
        e: Box<Expr>,
        lo: Box<Expr>,
        hi: Box<Expr>,
    },
    /// `x in {a,b,c}`
    InList {
        e: Box<Expr>,
        list: Vec<Expr>,
    },
    /// `x in this.list`: one of the values of an array (2.1 and later).
    InArray {
        e: Box<Expr>,
        name: String,
    },
    /// `cond ? a : b` (2.1 and later).
    Cond {
        cond: Box<Expr>,
        then: Box<Expr>,
        els: Box<Expr>,
    },
    /// `{a,b,c}`, the right side of an assignment (2.1 and later): fills
    /// the variable as an array.
    List(Vec<Expr>),
    /// A GS2-only form. The GS1 grammar never builds one.
    Gs2(Box<Gs2Expr>),
}

/// Expression forms only the GS2 front end builds
/// (`scripting::gs2::compile`).
#[derive(Debug, Clone, PartialEq)]
pub enum Gs2Expr {
    /// `obj.name(args)`: a built-in method or a call on an object.
    Method {
        obj: Expr,
        name: String,
        args: Vec<Expr>,
    },
    /// `obj.name` where `obj` is not a plain dotted name (a call, an index).
    Member { obj: Expr, name: String },
    /// `obj[i]` or `obj[i, j]` where `obj` is not a plain dotted name.
    Index { obj: Expr, index: Vec<Expr> },
    /// `new Type(args)`.
    New { ty: String, args: Vec<Expr> },
    /// `new[n]`, `new[n][m]`.
    NewArray(Vec<Expr>),
    /// `a << b`, `a >> b`, `a & b`, `a | b`.
    Bits { kind: BitKind, a: Expr, b: Expr },
    /// `@x`: the value as a string.
    Text(Expr),
    /// `obj.(expr)`: the member whose name is the value of `expr`.
    DynMember { obj: Expr, name: Expr },
    /// `obj.(expr)(args)`: a call of that member.
    DynMethod {
        obj: Expr,
        name: Expr,
        args: Vec<Expr>,
    },
    /// A GS1 message code (`#a`, `#v(x)`) used as a GS2 value.
    Gs1Text(String),
    /// `x in list` where the list is any value (`x in f(…)`).
    InValue { e: Expr, list: Expr },
    /// `function (…) { … }` as a value; the name of the function made for it.
    Lambda(String),
    /// `x++` / `x--` used as a value: the old value, then the step.
    PostStep { target: Expr, delta: f64 },
    /// `++x` / `--x` used as a value: the step, then the new value.
    PreStep { target: Expr, delta: f64 },
    /// `(x = v)`, `(x += v)` used as a value: the place after the store.
    AssignValue {
        target: Expr,
        op: AssignOp,
        value: Expr,
    },
}

/// The bit operators of [`Gs2Expr::Bits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitKind {
    Shl,
    Shr,
    And,
    Or,
}

impl Gs2Expr {
    /// The expressions directly inside this one, in source order.
    pub fn parts(&self) -> Vec<&Expr> {
        match self {
            Gs2Expr::Method { obj, args, .. } => std::iter::once(obj).chain(args).collect(),
            Gs2Expr::Member { obj, .. } => vec![obj],
            Gs2Expr::Index { obj, index } => std::iter::once(obj).chain(index).collect(),
            Gs2Expr::New { args, .. } => args.iter().collect(),
            Gs2Expr::NewArray(dims) => dims.iter().collect(),
            Gs2Expr::Bits { a, b, .. } => vec![a, b],
            Gs2Expr::Text(e) => vec![e],
            Gs2Expr::DynMember { obj, name } => vec![obj, name],
            Gs2Expr::DynMethod { obj, name, args } => [obj, name].into_iter().chain(args).collect(),
            Gs2Expr::Lambda(_) | Gs2Expr::Gs1Text(_) => Vec::new(),
            Gs2Expr::InValue { e, list } => vec![e, list],
            Gs2Expr::PostStep { target, .. } | Gs2Expr::PreStep { target, .. } => vec![target],
            Gs2Expr::AssignValue { target, value, .. } => vec![target, value],
        }
    }

    /// The form may run script or host code (a method or `new`).
    pub fn calls(&self) -> bool {
        matches!(
            self,
            Gs2Expr::Method { .. } | Gs2Expr::DynMethod { .. } | Gs2Expr::New { .. }
        )
    }
}

impl Gs2Stmt {
    /// The statements directly inside this one.
    pub fn bodies(&self) -> Vec<&Stmt> {
        match self {
            Gs2Stmt::Switch { arms, .. } => arms.iter().flat_map(|a| a.body.iter()).collect(),
            Gs2Stmt::ForEach { body, .. } => vec![body],
            Gs2Stmt::Eval(_) | Gs2Stmt::SetPart { .. } => Vec::new(),
            Gs2Stmt::NewBlock { body, .. } => body.iter().collect(),
            Gs2Stmt::Chain { .. } => Vec::new(),
            Gs2Stmt::Gs1(s) => vec![s],
        }
    }

    /// The expressions directly inside this one (not in its bodies).
    pub fn exprs(&self) -> Vec<&Expr> {
        match self {
            Gs2Stmt::Switch { value, arms } => std::iter::once(value)
                .chain(arms.iter().flat_map(|a| a.values.iter()))
                .collect(),
            Gs2Stmt::ForEach { list, .. } => vec![list],
            Gs2Stmt::Eval(e) => vec![e],
            Gs2Stmt::SetPart { target, value } => vec![target, value],
            Gs2Stmt::NewBlock { name, .. } => vec![name],
            Gs2Stmt::Gs1(_) => Vec::new(),
            Gs2Stmt::Chain { targets, value } => targets
                .iter()
                .flat_map(|t| t.index.iter())
                .chain(std::iter::once(value))
                .collect(),
        }
    }
}

/// One arm of a GS2 `switch`: `case` values (empty for `default`) and the
/// statements up to the next arm.
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchArm {
    pub values: Vec<Expr>,
    pub default: bool,
    pub body: Vec<Stmt>,
}

/// Statement forms only the GS2 front end builds.
#[derive(Debug, Clone, PartialEq)]
pub enum Gs2Stmt {
    /// `switch (value) { case …: … default: … }`.
    Switch { value: Expr, arms: Vec<SwitchArm> },
    /// `for (var : list) body`.
    ForEach { var: VarRef, list: Expr, body: Stmt },
    /// An expression used as a statement (`obj.method(…);`).
    Eval(Expr),
    /// `target = value` where the target is a member or element of a value
    /// that is not a plain dotted name (`findplayer(n).chat = …`).
    SetPart { target: Expr, value: Expr },
    /// `new Type(name) { body }`: make the object, run the body with it as
    /// the current object, then add it as a control of the object outside.
    NewBlock {
        ty: String,
        name: Expr,
        body: Vec<Stmt>,
    },
    /// `a = b = … = value`: every target set to the value.
    Chain { targets: Vec<VarRef>, value: Expr },
    /// A GS1 command written GS1-style inside GS2 (`setimgpart a.png,0,0,8,8;`),
    /// parsed and lowered as GS1.
    Gs1(Stmt),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

impl AssignOp {
    pub fn symbol(self) -> &'static str {
        match self {
            AssignOp::Set => "=",
            AssignOp::Add => "+=",
            AssignOp::Sub => "-=",
            AssignOp::Mul => "*=",
            AssignOp::Div => "/=",
            AssignOp::Mod => "%=",
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
    If {
        cond: Expr,
        then: Box<Stmt>,
        els: Option<Box<Stmt>>,
    },
    While {
        cond: Expr,
        body: Box<Stmt>,
    },
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        step: Option<Box<Stmt>>,
        body: Box<Stmt>,
    },
    Function {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
    },
    Return(Option<Expr>),
    Break,
    Continue,
    Assign {
        target: VarRef,
        op: AssignOp,
        value: Expr,
    },
    /// `x++` (`delta` 1) and `x--` (`delta` -1).
    IncDec {
        target: VarRef,
        delta: f64,
    },
    /// A function call used as a statement.
    Call {
        name: String,
        args: Vec<Expr>,
    },
    Command {
        name: String,
        args: Vec<Arg>,
        line: u32,
    },
    /// `with (target) body` (2.1 and later): the body runs as the player or
    /// NPC `target` names. Seihai does not switch objects, so the body is
    /// skipped and the skip logged.
    With {
        target: Expr,
        body: Box<Stmt>,
        line: u32,
    },
    /// A name that is not a command in this era, with its raw arguments.
    /// Logged and ignored when run.
    Unknown {
        name: String,
        raw: String,
        reason: &'static str,
        line: u32,
    },
    /// A GS2-only statement. The GS1 grammar never builds one.
    Gs2(Box<Gs2Stmt>),
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
            Expr::Gs2(g) => write!(f, "(gs2 {g:?})"),
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
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                let o =
                    |s: &Option<Box<Stmt>>| s.as_ref().map_or("_".to_string(), |s| s.to_string());
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
            Stmt::IncDec { target, delta } => {
                write!(f, "({} {target})", if *delta > 0.0 { "++" } else { "--" })
            }
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
            Stmt::Unknown {
                name, raw, reason, ..
            } => write!(f, "(unknown {name} {raw:?} {reason})"),
            Stmt::Gs2(g) => write!(f, "(gs2 {g:?})"),
        }
    }
}
