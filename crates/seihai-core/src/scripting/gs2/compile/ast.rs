//! The compiler's syntax tree.
//!
//! Both front ends build it: the GS2 parser directly, and the GS1 front end
//! by lowering the GS1 engine's tree. Code generation reads only this tree,
//! so it has no GS1 or GS2 mode of its own. A few nodes exist only because
//! GS1 has a built-in with no GS2 name ([`Expr::Gs1Fn`], [`Expr::AnyOf`],
//! [`Expr::Message`], [`Expr::HostCall`]); they say what to emit, not which
//! language they came from.

/// A variable: a plain or dotted name (`x`, `this.a.b`, `$pref::v`) with
/// optional indexes (`a[i]`, `this.grid[x, y]`).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Var {
    pub name: String,
    pub index: Vec<Expr>,
}

impl Var {
    pub fn plain(name: impl Into<String>) -> Var {
        Var {
            name: name.into(),
            index: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BinOp {
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
    /// `@`, and `SPC`, `NL`, `TAB` (the same with a space, a line break or
    /// a tab between).
    Cat,
    CatSpc,
    CatNl,
    CatTab,
}

/// The bit operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BitOp {
    Shl,
    Shr,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AssignOp {
    Set,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
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
            AssignOp::BitAnd => "&=",
            AssignOp::BitOr => "|=",
        }
    }

    /// `a op= v` written out as `a op v`.
    pub fn expand(self, a: Expr, v: Expr) -> Expr {
        let bin = match self {
            AssignOp::Add => BinOp::Add,
            AssignOp::Sub => BinOp::Sub,
            AssignOp::Mul => BinOp::Mul,
            AssignOp::Div => BinOp::Div,
            AssignOp::Mod => BinOp::Mod,
            AssignOp::BitAnd => return Expr::Bits(BitOp::And, Box::new(a), Box::new(v)),
            AssignOp::BitOr => return Expr::Bits(BitOp::Or, Box::new(a), Box::new(v)),
            AssignOp::Set => unreachable!("`=` has no operator"),
        };
        Expr::Binary(bin, Box::new(a), Box::new(v))
    }
}

/// GS1 built-in functions with no GS2 name, lowered to opcodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Gs1Fn {
    /// `strlen(s)`: `s.length()`.
    StrLen,
    /// `startswith(s, t)`: `s.starts(t)`.
    StartsWith,
    /// `indexof(s, t)`: `s.pos(t)`.
    IndexOf,
    /// `strtofloat(s)`: the number in `s`.
    StrToFloat,
    /// `strcontains(s, t)`: `s.pos(t) >= 0`.
    StrContains,
}

/// One piece of GS1 text with message codes (`"hi #n"`).
#[derive(Debug, Clone, PartialEq)]
pub(super) enum MsgPart {
    Lit(String),
    /// `#n`, `#v(x)`, `#t(i)`, …: a value, as text.
    Value(Expr),
    /// `#T(text)`: the text, trimmed.
    Trim(Vec<MsgPart>),
    /// `#e(start, len, text)`: part of the text.
    Sub(Expr, Expr, Vec<MsgPart>),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Expr {
    Num(f64),
    Str(String),
    Var(Var),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Bits(BitOp, Box<Expr>, Box<Expr>),
    /// `cond ? then : els`
    Cond {
        cond: Box<Expr>,
        then: Box<Expr>,
        els: Box<Expr>,
    },
    /// `x in |lo, hi|`
    InRange {
        e: Box<Expr>,
        lo: Box<Expr>,
        hi: Box<Expr>,
    },
    /// `x in {a, b}`: membership of an array literal.
    InList {
        e: Box<Expr>,
        list: Vec<Expr>,
    },
    /// GS1 `x in {a, b}`: `x == a || x == b`, as one value.
    AnyOf {
        e: Box<Expr>,
        list: Vec<Expr>,
    },
    /// `x in name`
    InArray {
        e: Box<Expr>,
        name: String,
    },
    /// `x in value` for any other value (`x in f()`).
    InValue {
        e: Box<Expr>,
        list: Box<Expr>,
    },
    /// `{a, b, c}`
    List(Vec<Expr>),
    /// `name(args)`: a built-in, a script function or a host call.
    Call {
        name: String,
        args: Vec<Expr>,
    },
    /// A call by name that skips the built-ins (a lowered GS1 command).
    HostCall {
        name: String,
        args: Vec<Expr>,
    },
    /// `obj.name(args)`
    Method {
        obj: Box<Expr>,
        name: String,
        args: Vec<Expr>,
    },
    /// `obj.name` where `obj` is not a plain dotted name.
    Member {
        obj: Box<Expr>,
        name: String,
    },
    /// `obj[i]` where `obj` is not a plain dotted name.
    Index {
        obj: Box<Expr>,
        index: Vec<Expr>,
    },
    /// `obj.(name)`
    DynMember {
        obj: Box<Expr>,
        name: Box<Expr>,
    },
    /// `obj.(name)(args)`
    DynMethod {
        obj: Box<Expr>,
        name: Box<Expr>,
        args: Vec<Expr>,
    },
    /// `(name)(args)`
    DynCall {
        name: Box<Expr>,
        args: Vec<Expr>,
    },
    /// `new Type(args)`
    New {
        ty: String,
        args: Vec<Expr>,
    },
    /// `new[n]`, `new[n][m]`
    NewArray(Vec<Expr>),
    /// `@x`: the value as text.
    Text(Box<Expr>),
    /// `function (…) { … }` as a value: the name of the function made for it.
    Lambda(String),
    /// `x++` / `x--` as a value: the old value, then the step.
    PostStep {
        target: Box<Expr>,
        delta: f64,
    },
    /// `++x` / `--x` as a value: the step, then the place.
    PreStep {
        target: Box<Expr>,
        delta: f64,
    },
    /// `(x = v)`, `(x += v)` as a value: the place after the store.
    AssignValue {
        target: Box<Expr>,
        op: AssignOp,
        value: Box<Expr>,
    },
    /// GS1 text with message codes.
    Message(Vec<MsgPart>),
    /// A GS1 built-in with no GS2 name.
    Gs1Fn(Gs1Fn, Vec<Expr>),
}

impl Expr {
    pub fn var(name: impl Into<String>) -> Expr {
        Expr::Var(Var::plain(name))
    }

    pub fn bin(op: BinOp, a: Expr, b: Expr) -> Expr {
        Expr::Binary(op, Box::new(a), Box::new(b))
    }

    /// A value that can be stored into: a member, an element of a value,
    /// a run-time member, or `makevar(…)`.
    pub fn is_part_place(&self) -> bool {
        match self {
            Expr::Member { .. } | Expr::Index { .. } | Expr::DynMember { .. } => true,
            Expr::Call { name, .. } => name == "makevar",
            _ => false,
        }
    }

    /// A call of any kind.
    pub fn is_call(&self) -> bool {
        matches!(
            self,
            Expr::Call { .. } | Expr::Method { .. } | Expr::DynMethod { .. } | Expr::DynCall { .. }
        )
    }

    /// The expressions directly inside this one, in source order.
    pub fn parts(&self) -> Vec<&Expr> {
        match self {
            Expr::Num(_) | Expr::Str(_) | Expr::Lambda(_) | Expr::Message(_) => Vec::new(),
            Expr::Var(v) => v.index.iter().collect(),
            Expr::Neg(a) | Expr::Not(a) | Expr::Text(a) => vec![a],
            Expr::InArray { e, .. } => vec![e],
            Expr::Binary(_, a, b) | Expr::Bits(_, a, b) => vec![a, b],
            Expr::Cond { cond, then, els } => vec![cond, then, els],
            Expr::InRange { e, lo, hi } => vec![e, lo, hi],
            Expr::InList { e, list } | Expr::AnyOf { e, list } => {
                std::iter::once(&**e).chain(list).collect()
            }
            Expr::InValue { e, list } => vec![e, list],
            Expr::List(v) | Expr::NewArray(v) | Expr::Gs1Fn(_, v) => v.iter().collect(),
            Expr::Call { args, .. } | Expr::HostCall { args, .. } | Expr::New { args, .. } => {
                args.iter().collect()
            }
            Expr::Method { obj, args, .. } => std::iter::once(&**obj).chain(args).collect(),
            Expr::Member { obj, .. } => vec![obj],
            Expr::Index { obj, index } => std::iter::once(&**obj).chain(index).collect(),
            Expr::DynMember { obj, name } => vec![obj, name],
            Expr::DynMethod { obj, name, args } => {
                [&**obj, &**name].into_iter().chain(args).collect()
            }
            Expr::DynCall { name, args } => std::iter::once(&**name).chain(args).collect(),
            Expr::PostStep { target, .. } | Expr::PreStep { target, .. } => vec![target],
            Expr::AssignValue { target, value, .. } => vec![target, value],
        }
    }
}

/// One arm of a `switch`: `case` values (empty for `default`) and the
/// statements up to the next arm.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Arm {
    pub values: Vec<Expr>,
    pub default: bool,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Stmt {
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
    /// `for (var : list) body`
    ForEach {
        var: String,
        list: Expr,
        body: Box<Stmt>,
    },
    Switch {
        value: Expr,
        arms: Vec<Arm>,
    },
    Return(Option<Expr>),
    Break,
    Continue,
    /// `with (target) body`
    With {
        target: Expr,
        body: Box<Stmt>,
        line: u32,
    },
    /// `new Type(name) { body }`: make the object, run the body with it as
    /// the current object, then add it as a control of the object outside.
    NewBlock {
        ty: String,
        name: Expr,
        body: Vec<Stmt>,
    },
    Assign {
        target: Var,
        op: AssignOp,
        value: Expr,
    },
    /// `x++` (`delta` 1) and `x--` (`delta` -1) as a statement.
    IncDec {
        target: Var,
        delta: f64,
    },
    /// `a = b = … = value`
    Chain {
        targets: Vec<Var>,
        value: Expr,
    },
    /// `target = value` where the target is a member or element of a value
    /// that is not a plain dotted name (`findplayer(n).chat = …`).
    SetPart {
        target: Expr,
        value: Expr,
    },
    /// A call used as a statement; its value is dropped.
    Call {
        name: String,
        args: Vec<Expr>,
    },
    /// Any other expression used as a statement; its value is dropped.
    Eval(Expr),
    /// A GS1 command written inside GS2, already lowered. `calls` is
    /// whether the command counts as a call for the function's
    /// `OP_CMD_CALL` (decided on the command as written).
    Embedded {
        body: Box<Stmt>,
        calls: bool,
    },
}

impl Stmt {
    pub fn empty() -> Stmt {
        Stmt::Block(Vec::new())
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Stmt::Block(v) if v.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Function {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    /// Whether the function starts with `OP_CMD_CALL`. `None`: decided
    /// from the body.
    pub cmd_call: Option<bool>,
}

/// One piece of a script in source order.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Item {
    Function(Function),
    /// Code outside any function, emitted where it stands, so it runs from
    /// op 0 with the function bodies jumped over.
    Code(Vec<Stmt>),
}

/// A parsed script, ready for code generation.
#[derive(Debug, Default)]
pub(super) struct Program {
    pub items: Vec<Item>,
    /// `function (…) { … }` values, emitted where they are used.
    pub lambdas: Vec<Function>,
    /// Whether a later function replaces an earlier one of the same name
    /// (GS2 legacy) instead of being an error.
    pub later_wins: bool,
}

/// The body always leaves through `return`, so the function needs no
/// closing `return 0`. An `if` counts when it has an `else` and every
/// branch returns, which is where the reference compiler also leaves it out.
pub(super) fn ends_with_return(body: &[Stmt]) -> bool {
    body.last().is_some_and(always_returns)
}

fn always_returns(s: &Stmt) -> bool {
    match s {
        Stmt::Return(_) => true,
        Stmt::Block(v) => ends_with_return(v),
        Stmt::If {
            then,
            els: Some(els),
            ..
        } => always_returns(then) && always_returns(els),
        _ => false,
    }
}
