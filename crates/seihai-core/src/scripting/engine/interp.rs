//! Runs one event of one script object.

use super::ast::*;
use super::compile::Node;
use super::parser::parse_expr;
use super::value::Val;
use super::vocab::{self, ArgKind, Cmd, Era, Exec};
use super::world::*;

/// Passes a loop may make before the run is stopped.
pub const MAX_LOOP_PASSES: u32 = 10_000;
/// Nested script-function calls.
pub const MAX_CALL_DEPTH: usize = 64;
/// Nested expression evaluations, over all calls.
pub const MAX_EVAL_DEPTH: usize = 400;

pub(super) enum Flow {
    Done,
    Returned(Val),
}

pub struct Run<'a> {
    core: &'a mut Core,
    obj: &'a mut ScriptObject,
    event: &'a str,
    steps: u64,
    ended: bool,
    call_depth: usize,
    eval_depth: usize,
    /// Top-level node being run; a `sleep` resumes at the one after it.
    top_pc: usize,
    loop_counts: Vec<u32>,
}

/// Where a variable lives.
struct Place {
    base: String,
    key: String,
    index: Option<i64>,
    /// The second index, for `tiles[x,y]`.
    index2: Option<i64>,
}

/// The tile `tiles[x,y]` reads off the level (2.1 test 3): the board's
/// out-of-level value.
pub const TILES_OUTSIDE: u16 = 0x278;

/// The real client's float-to-int for board and tile indexes:
/// `floor(v + 0.0001)` (2.05 and 2.1 test 3).
fn board_int(v: f64) -> i64 {
    (v + 0.0001).floor() as i64
}

fn numeric_str(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// A value as a number if it is one (or a string that is entirely one).
fn as_number(v: &Val) -> Option<f64> {
    match v {
        Val::Num(n) => Some(*n),
        Val::Str(s) => numeric_str(s),
    }
}

/// A string list as its entries: comma-separated, an entry in double
/// quotes may hold commas (`""` is a quote). An empty list has no entries.
pub(super) fn split_list(s: &str) -> Vec<String> {
    if s.trim().is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' if quoted => quoted = false,
            '"' if cur.trim().is_empty() => {
                cur.clear();
                quoted = true;
            }
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// The entries as a string list, quoting those with a comma or a quote.
pub(super) fn join_list(items: &[String]) -> String {
    items
        .iter()
        .map(|i| if i.contains(',') || i.contains('"') { format!("\"{}\"", i.replace('"', "\"\"")) } else { i.clone() })
        .collect::<Vec<_>>()
        .join(",")
}

fn sv(v: &[Val], i: usize) -> String {
    v.get(i).map(Val::text).unwrap_or_default()
}

fn nv(v: &[Val], i: usize) -> f64 {
    v.get(i).map(Val::num).unwrap_or(0.0)
}

impl<'a> Run<'a> {
    pub(super) fn new(core: &'a mut Core, obj: &'a mut ScriptObject, event: &'a str) -> Self {
        let loops = obj.script.loops;
        Run { core, obj, event, steps: 0, ended: false, call_depth: 0, eval_depth: 0, top_pc: 0, loop_counts: vec![0; loops] }
    }

    fn era(&self) -> Era {
        self.core.era
    }

    fn note(&mut self, msg: String) {
        let id = self.obj.id;
        self.core.note(format!("object {id}: {msg}"));
    }

    // ---- statements ----

    /// Run nodes from `start` until the script ends or, inside a
    /// function, returns.
    pub(super) fn exec(&mut self, start: usize) -> Flow {
        let script = self.obj.script.clone();
        let mut pc = start;
        loop {
            if self.ended {
                return Flow::Done;
            }
            self.steps += 1;
            if self.steps > MAX_STEPS {
                self.note(format!("stopped after {MAX_STEPS} steps in `{}`", self.event));
                self.ended = true;
                return Flow::Done;
            }
            if self.call_depth == 0 {
                self.top_pc = pc;
            }
            let Some(node) = script.nodes.get(pc) else {
                return Flow::Done;
            };
            match node {
                Node::Cmd { cmd, args, .. } => {
                    self.command(cmd, args);
                    pc += 1;
                }
                Node::Unknown { name, raw, reason, line } => {
                    self.note(format!("line {line}: `{name} {raw}` ignored ({reason})"));
                    pc += 1;
                }
                Node::Assign { target, op: AssignOp::Set, value: Expr::List(items) } if target.index.is_empty() => {
                    self.set_list(&target.name, items);
                    pc += 1;
                }
                Node::Assign { target, op, value } => {
                    let place = self.place(target);
                    let rhs = self.eval(value);
                    let new = match op {
                        AssignOp::Set => rhs,
                        _ => {
                            let cur = self.get(&place);
                            let bop = match op {
                                AssignOp::Add => BinOp::Add,
                                AssignOp::Sub => BinOp::Sub,
                                AssignOp::Mul => BinOp::Mul,
                                _ => BinOp::Div,
                            };
                            self.binop(bop, cur, rhs)
                        }
                    };
                    self.set(&place, new);
                    pc += 1;
                }
                Node::IncDec { target, delta } => {
                    let place = self.place(target);
                    let cur = self.get(&place).num();
                    self.set(&place, Val::Num(cur + delta));
                    pc += 1;
                }
                Node::Eval(e) => {
                    self.eval(e);
                    pc += 1;
                }
                Node::JumpIfFalse(c, to) => {
                    pc = if self.eval(c).truthy() { pc + 1 } else { *to };
                }
                Node::Jump(to) => pc = *to,
                Node::LoopEnter(slot) => {
                    self.loop_counts[*slot] = 0;
                    pc += 1;
                }
                Node::LoopBack { to, slot } => {
                    self.loop_counts[*slot] += 1;
                    if self.loop_counts[*slot] >= MAX_LOOP_PASSES {
                        self.note(format!("loop stopped after {MAX_LOOP_PASSES} passes"));
                        self.ended = true;
                        return Flow::Done;
                    }
                    pc = *to;
                }
                Node::Return(e) => {
                    let v = e.as_ref().map(|e| self.eval(e)).unwrap_or_default();
                    if self.call_depth > 0 {
                        return Flow::Returned(v);
                    }
                    self.ended = true;
                    return Flow::Done;
                }
                Node::End => return Flow::Done,
            }
        }
    }

    fn call_user(&mut self, name: &str, args: Vec<Val>) -> Val {
        let script = self.obj.script.clone();
        let Some(f) = script.funcs.get(name) else {
            self.note(format!("call to unknown function `{name}`"));
            return Val::default();
        };
        if self.call_depth >= MAX_CALL_DEPTH {
            self.note(format!("calls nested deeper than {MAX_CALL_DEPTH}; run stopped"));
            self.ended = true;
            return Val::default();
        }
        // Parameters are plain variables for the length of the call.
        let saved: Vec<(String, Option<Val>)> = f.params.iter().map(|p| (p.clone(), self.obj.vars.get(p).cloned())).collect();
        for (i, p) in f.params.iter().enumerate() {
            self.obj.vars.insert(p.clone(), args.get(i).cloned().unwrap_or_default());
        }
        self.call_depth += 1;
        let r = self.exec(f.entry);
        self.call_depth -= 1;
        for (p, old) in saved {
            match old {
                Some(v) => self.obj.vars.insert(p, v),
                None => self.obj.vars.remove(&p),
            };
        }
        match r {
            Flow::Returned(v) => v,
            Flow::Done => Val::default(),
        }
    }

    // ---- variables ----

    fn place(&mut self, v: &VarRef) -> Place {
        if v.index.is_empty() {
            return Place { base: v.name.clone(), key: v.name.clone(), index: None, index2: None };
        }
        let to_int = if self.is_board(&v.name) { board_int } else { |n: f64| n as i64 };
        let idx: Vec<i64> = v.index.iter().map(|e| to_int(self.eval(e).num())).collect();
        let key = format!("{}[{}]", v.name, idx.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","));
        Place { base: v.name.clone(), key, index: Some(idx[0]), index2: idx.get(1).copied() }
    }

    fn event_flag(&self, name: &str) -> Option<bool> {
        match vocab::event_since(name) {
            Some(e) if e <= self.era() => Some(name == self.event),
            _ => None,
        }
    }

    fn get(&mut self, p: &Place) -> Val {
        if p.index.is_none() {
            // `timeout` reads as the event flag; as a variable it is write-only.
            if let Some(b) = self.event_flag(&p.base) {
                return b.into();
            }
            if let Some((n, _)) = vocab::builtin_var(&p.base, self.era()) {
                return self.builtin_get(n);
            }
            if vocab::status_flag(&p.base, self.era()) {
                return self.status_get(&p.base).into();
            }
        } else if self.is_board(&p.base) {
            return self.board_get(p).map_or(Val::default(), |t| Val::Num(f64::from(t)));
        } else if let (Some(i), Some(len)) = (p.index, self.store_get(&format!("{}#len", p.base))) {
            if i < 0 || i as f64 >= len.num() {
                return Val::default();
            }
        }
        self.store_get(&p.key).unwrap_or_default()
    }

    /// `board` is the level board in every era; `tiles` is one from 2.1 (in
    /// 2.05 it is not in the expression table, so it is a plain variable).
    fn is_board(&self, name: &str) -> bool {
        name == "board" || (name == "tiles" && self.era() >= Era::V21)
    }

    /// A `board[i]` or `tiles[x,y]` read (`documentation/board-reads.md`).
    /// `board[i]`: the tile at index `i = x + 64*y`, 0 outside 0..4095.
    /// `tiles[x,y]` (2.1 and later): the tile at `(x, y)`,
    /// [`TILES_OUTSIDE`] off the level. Both read 0 with no level.
    fn board_get(&self, p: &Place) -> Option<u16> {
        let board = self.core.board.as_ref()?;
        let i = p.index?;
        if p.base == "board" {
            return usize::try_from(i).ok().and_then(|i| board.get(i)).copied();
        }
        let j = p.index2?;
        if (0..64).contains(&i) && (0..64).contains(&j) {
            board.get((j * 64 + i) as usize).copied()
        } else {
            Some(TILES_OUTSIDE)
        }
    }

    fn set(&mut self, p: &Place, v: Val) {
        if p.index.is_none() {
            if let Some((n, writable)) = vocab::builtin_var(&p.base, self.era()) {
                if writable {
                    self.builtin_set(n, v);
                }
                return;
            }
            if vocab::status_flag(&p.base, self.era()) || self.event_flag(&p.base).is_some() {
                return;
            }
        } else if self.is_board(&p.base) {
            return;
        } else if let (Some(i), Some(len)) = (p.index, self.store_get(&format!("{}#len", p.base))) {
            if i < 0 || i as f64 >= len.num() {
                return;
            }
        }
        self.store_set(&p.key, v);
    }

    fn store_get(&self, key: &str) -> Option<Val> {
        let flag = |k: &str| {
            self.core.flags.get(k).map(|s| if s.is_empty() { Val::Num(1.0) } else { Val::Str(s.clone()) })
        };
        if key.starts_with("level.") {
            self.core.level_vars.get(key).cloned()
        } else if key.starts_with("server.") {
            self.core.server_vars.get(key).cloned()
        } else if key.starts_with("client.") || key.starts_with("local.") {
            flag(key)
        } else if !key.contains('.') {
            self.obj.vars.get(key).cloned().or_else(|| flag(key))
        } else {
            self.obj.vars.get(key).cloned()
        }
    }

    fn store_set(&mut self, key: &str, v: Val) {
        if key.starts_with("level.") {
            self.core.level_vars.insert(key.to_string(), v);
        } else if key.starts_with("server.") {
            self.core.server_vars.insert(key.to_string(), v);
        } else if key.starts_with("client.") || key.starts_with("local.") || (!key.contains('.') && self.core.flags.contains_key(key)) {
            self.write_flag(key, v.text());
        } else {
            self.obj.vars.insert(key.to_string(), v);
        }
    }

    /// Set a flag. The server is told only when the value changes: a script
    /// that sets the same value every frame (`-System3`'s `lastweapon`) must
    /// not send a `FLAGSET` every frame.
    fn write_flag(&mut self, name: &str, value: String) {
        if self.core.flags.get(name) == Some(&value) {
            return;
        }
        self.core.flags.insert(name.to_string(), value.clone());
        if !name.starts_with("local.") {
            self.core.effects.push(Effect::SetFlag { name: name.to_string(), value: Some(value) });
        }
    }

    fn status_get(&self, name: &str) -> bool {
        match name {
            "visible" => self.obj.visible,
            "isweapon" => self.obj.kind == ObjKind::Weapon,
            _ => self.core.player.status.contains(name),
        }
    }

    fn builtin_get(&self, name: &str) -> Val {
        let p = &self.core.player;
        match name {
            "chat" => return Val::Str(self.obj.message.clone()),
            "player.chat" => return Val::Str(p.chat.clone()),
            "player.account" => return Val::Str(p.account.clone()),
            "player.nick" => return Val::Str(p.nick.clone()),
            _ => {}
        }
        Val::Num(match name {
            "x" => self.obj.x,
            "y" => self.obj.y,
            "playerx" => p.x,
            "playery" => p.y,
            "playerdir" => p.dir,
            "playerhearts" => p.hearts,
            "playerrupees" => p.rupees,
            "playerbombs" => p.bombs,
            "playerdarts" => p.darts,
            "playerswordpower" => p.sword_power,
            "playershieldpower" => p.shield_power,
            "playerfullhearts" => p.max_hp.floor(),
            "playerhp" => p.hp,
            "playermaxhp" => p.max_hp,
            "tokenscount" => self.core.tokens.len() as f64,
            "paramscount" => self.obj.params.len() as f64,
            "weaponscount" => p.weapons.len() as f64,
            "selectedweapon" => p.selected_weapon.unwrap_or(0) as f64,
            _ => 0.0,
        })
    }

    fn builtin_set(&mut self, name: &str, v: Val) {
        let n = v.num();
        let id = self.obj.id;
        match name {
            // The object's chat bubble: the same as `message`.
            "chat" => {
                self.obj.message = v.text();
                self.core.effects.push(Effect::Message { obj: id, text: v.text() });
            }
            "player.chat" => {
                self.core.player.chat = v.text();
                self.core.effects.push(Effect::SetPlayerProp { code: "#c".into(), value: v.text() });
            }
            "x" => {
                self.obj.x = n;
                self.core.effects.push(Effect::SetPos { obj: id, x: n, y: self.obj.y });
            }
            "y" => {
                self.obj.y = n;
                self.core.effects.push(Effect::SetPos { obj: id, x: self.obj.x, y: n });
            }
            "timeout" => self.set_timeout(n),
            "selectedweapon" => {
                let p = &mut self.core.player;
                let len = p.weapons.len();
                if n < 0.0 || n.floor() as usize >= len {
                    return;
                }
                // The client's own select: the first weapon from `i` on,
                // wrapping, that is not hidden; `i` itself if all are.
                let i = n.floor() as usize;
                let pick = (0..len).map(|k| (i + k) % len).find(|&j| !p.weapons[j].0.is_empty() && !p.weapons[j].0.starts_with('-')).unwrap_or(i);
                p.selected_weapon = Some(pick);
                p.weapon = p.weapons[pick].0.clone();
                self.core.effects.push(Effect::SetPlayerVar { name: name.to_string(), value: pick as f64 });
            }
            _ => {
                let p = &mut self.core.player;
                match name {
                    "playerx" => p.x = n,
                    "playery" => p.y = n,
                    "playerdir" => p.dir = n.rem_euclid(4.0).floor(),
                    "playerhearts" => p.hearts = n,
                    "playerrupees" => p.rupees = n,
                    "playerbombs" => p.bombs = n,
                    "playerdarts" => p.darts = n,
                    "playerswordpower" => p.sword_power = n,
                    "playershieldpower" => p.shield_power = n,
                    "playerhp" => p.hp = n,
                    _ => return,
                }
                let value = match name {
                    "playerdir" => self.core.player.dir,
                    _ => n,
                };
                self.core.effects.push(Effect::SetPlayerVar { name: name.to_string(), value });
            }
        }
    }

    fn set_timeout(&mut self, secs: f64) {
        self.obj.resume = None;
        self.obj.due = if secs > 0.0 { Some(self.core.tick + secs_to_ticks(secs)) } else { None };
    }

    // ---- expressions ----

    fn eval(&mut self, e: &Expr) -> Val {
        if self.eval_depth >= MAX_EVAL_DEPTH {
            self.note("expression nested too deep; run stopped".into());
            self.ended = true;
            return Val::default();
        }
        self.eval_depth += 1;
        let v = self.eval_inner(e);
        self.eval_depth -= 1;
        v
    }

    fn eval_inner(&mut self, e: &Expr) -> Val {
        match e {
            Expr::Num(n) => Val::Num(*n),
            Expr::Str(s) => Val::Str(self.expand(s, 0)),
            Expr::Var(v) => {
                let p = self.place(v);
                self.get(&p)
            }
            Expr::Unary(UnOp::Neg, a) => Val::Num(-self.eval(a).num()),
            Expr::Unary(UnOp::Not, a) => (!self.eval(a).truthy()).into(),
            Expr::Bin(BinOp::And, a, b) => (self.eval(a).truthy() && self.eval(b).truthy()).into(),
            Expr::Bin(BinOp::Or, a, b) => (self.eval(a).truthy() || self.eval(b).truthy()).into(),
            Expr::Bin(op, a, b) => {
                let l = self.eval(a);
                let r = self.eval(b);
                self.binop(*op, l, r)
            }
            Expr::InRange { e, lo, hi } => {
                let (x, lo, hi) = (self.eval(e).num(), self.eval(lo).num(), self.eval(hi).num());
                (x >= lo && x <= hi).into()
            }
            Expr::InList { e, list } => {
                let x = self.eval(e);
                for item in list {
                    let y = self.eval(item);
                    if self.equals(&x, &y) {
                        return Val::Num(1.0);
                    }
                }
                Val::Num(0.0)
            }
            Expr::Call { name, args } => self.call(name, args),
            Expr::Cond { cond, then, els } => {
                if self.eval(cond).truthy() {
                    self.eval(then)
                } else {
                    self.eval(els)
                }
            }
            // A list outside an assignment has no value of its own.
            Expr::List(_) => Val::default(),
            Expr::InArray { e, name } => {
                let x = self.eval(e);
                let len = self.store_get(&format!("{name}#len")).map_or(0.0, |v| v.num()).max(0.0) as i64;
                for i in 0..len {
                    let y = self.store_get(&format!("{name}[{i}]")).unwrap_or_default();
                    if self.equals(&x, &y) {
                        return Val::Num(1.0);
                    }
                }
                Val::Num(0.0)
            }
        }
    }

    /// `name = {a,b,c}`: an array of that length, as `setarray` makes,
    /// holding the values. Only a plain or `this.` variable can be an array.
    fn set_list(&mut self, name: &str, items: &[Expr]) {
        if !(name.starts_with("this.") || !name.contains('.')) {
            return;
        }
        let vals: Vec<Val> = items.iter().map(|e| self.eval(e)).collect();
        self.obj.vars.insert(format!("{name}#len"), Val::Num(vals.len() as f64));
        for (i, v) in vals.into_iter().enumerate() {
            self.obj.vars.insert(format!("{name}[{i}]"), v);
        }
    }

    fn equals(&self, a: &Val, b: &Val) -> bool {
        match (as_number(a), as_number(b)) {
            (Some(x), Some(y)) => x == y,
            _ => a.text().to_lowercase() == b.text().to_lowercase(),
        }
    }

    fn binop(&mut self, op: BinOp, l: Val, r: Val) -> Val {
        match op {
            BinOp::Cat | BinOp::CatSpc | BinOp::CatNl | BinOp::CatTab => {
                let sep = match op {
                    BinOp::CatSpc => " ",
                    BinOp::CatNl => "\n",
                    BinOp::CatTab => "\t",
                    _ => "",
                };
                Val::Str(format!("{}{sep}{}", l.text(), r.text()))
            }
            BinOp::Add => {
                if as_number(&l).is_none() && l.is_str() || as_number(&r).is_none() && r.is_str() {
                    return Val::Str(format!("{}{}", l.text(), r.text()));
                }
                Val::Num(l.num() + r.num())
            }
            BinOp::Sub => Val::Num(l.num() - r.num()),
            BinOp::Mul => Val::Num(l.num() * r.num()),
            BinOp::Div => {
                let d = r.num();
                Val::Num(if d == 0.0 { 0.0 } else { l.num() / d })
            }
            BinOp::Mod => {
                let d = r.num();
                Val::Num(if d == 0.0 { 0.0 } else { l.num() % d })
            }
            BinOp::Pow => {
                let v = l.num().powf(r.num());
                Val::Num(if v.is_finite() { v } else { 0.0 })
            }
            BinOp::Eq => self.equals(&l, &r).into(),
            BinOp::Ne => (!self.equals(&l, &r)).into(),
            BinOp::Lt => (l.num() < r.num()).into(),
            BinOp::Gt => (l.num() > r.num()).into(),
            BinOp::Le => (l.num() <= r.num()).into(),
            BinOp::Ge => (l.num() >= r.num()).into(),
            BinOp::And => (l.truthy() && r.truthy()).into(),
            BinOp::Or => (l.truthy() || r.truthy()).into(),
        }
    }

    fn call(&mut self, name: &str, args: &[Expr]) -> Val {
        let script = self.obj.script.clone();
        if script.funcs.contains_key(name) {
            let vals: Vec<Val> = args.iter().map(|a| self.eval(a)).collect();
            return self.call_user(name, vals);
        }
        let Some(b) = vocab::builtin_fn(name, self.era()) else {
            if vocab::is_builtin_fn_any(name) {
                self.note(format!("`{name}` does not exist in {}", self.era().label()));
            } else {
                self.note(format!("call to unknown function `{name}`"));
            }
            return Val::default();
        };
        if name == "arraylen" {
            return match args.first() {
                Some(Expr::Var(v)) => self.store_get(&format!("{}#len", v.name)).unwrap_or_default(),
                Some(Expr::Str(s)) => self.store_get(&format!("{}#len", s.trim().to_ascii_lowercase())).unwrap_or_default(),
                _ => Val::default(),
            };
        }
        let vals: Vec<Val> = args.iter().map(|a| self.eval(a)).collect();
        let n = |i: usize| nv(&vals, i);
        let s = |i: usize| sv(&vals, i);
        debug_assert!(b.args.iter().all(|k| matches!(k, ArgKind::Num | ArgKind::Text)));
        match name {
            "abs" => Val::Num(n(0).abs()),
            "int" => Val::Num(n(0).trunc()),
            "sin" => Val::Num(n(0).sin()),
            "cos" => Val::Num(n(0).cos()),
            "arctan" => Val::Num(n(0).atan()),
            "random" => {
                let (lo, hi) = (n(0), n(1));
                Val::Num(lo + self.core.random() * (hi - lo))
            }
            "strequals" => (s(0).to_lowercase() == s(1).to_lowercase()).into(),
            "strlen" => Val::Num(s(0).chars().count() as f64),
            "startswith" => s(1).to_lowercase().starts_with(&s(0).to_lowercase()).into(),
            "indexof" => {
                let (part, text) = (s(0).to_lowercase(), s(1).to_lowercase());
                Val::Num(text.find(&part).map_or(-1.0, |b| text[..b].chars().count() as f64))
            }
            "ascii" => Val::Num(s(0).chars().next().map_or(0.0, |c| c as u32 as f64)),
            "strcontains" => s(0).to_lowercase().contains(&s(1).to_lowercase()).into(),
            "lindexof" => {
                let t = s(0);
                Val::Num(split_list(&s(1)).iter().position(|i| i.trim() == t.trim()).map_or(-1.0, |p| p as f64))
            }
            // The level's cell on the current map: -1 with no map, 0 for a
            // level not on it (as GServer-v2's GS1).
            "onmapx" | "onmapy" => match &self.core.map {
                None => Val::Num(-1.0),
                Some(m) => {
                    let (x, y) = m.cell_of(s(0).trim()).unwrap_or((0, 0));
                    Val::Num(if name == "onmapx" { x } else { y } as f64)
                }
            },
            "strtofloat" => Val::Num(s(0).trim().parse().unwrap_or(0.0)),
            "hasweapon" => Val::Num(0.0),
            // `keydown(i)`: the host's key bits (always clear while paused).
            "keydown" => {
                let i = n(0).trunc();
                Val::Num(if (0.0..16.0).contains(&i) && self.core.keys_down & (1 << i as u32) != 0 { 1.0 } else { 0.0 })
            }
            "keydown2" => Val::Num(0.0),
            // The argument names the list (a flag), as in `#I(list,i)`.
            "sarraylen" => {
                let list = s(0);
                let text = self.core.flags.get(&list.trim().to_ascii_lowercase()).cloned().unwrap_or(list);
                Val::Num(split_list(&text).len() as f64)
            }
            "onwall" => {
                let (x, y) = (n(0).floor(), n(1).floor());
                let blocked = match &self.core.walls {
                    _ if !(0.0..64.0).contains(&x) || !(0.0..64.0).contains(&y) => true,
                    Some(w) => w.get(y as usize * 64 + x as usize).copied().unwrap_or(false),
                    None => false,
                };
                blocked.into()
            }
            "onwater" => {
                let (x, y) = (n(0).floor(), n(1).floor());
                let wet = match &self.core.water {
                    Some(w) if (0.0..64.0).contains(&x) && (0.0..64.0).contains(&y) => {
                        w.get(y as usize * 64 + x as usize).copied().unwrap_or(false)
                    }
                    _ => false,
                };
                wet.into()
            }
            // 2.05 answers only for the tilesets of the new kind (`addtiledef`
            // type 1 or 2); the default tileset, typed by `arrays.dat`, gets -1.
            "tiletype" => Val::Num(-1.0),
            "playersays" | "playersays2" => {
                if self.event != "playerchats" {
                    return Val::Num(0.0);
                }
                let said = self.core.player.chat.trim().to_lowercase();
                let want = s(0).trim().to_lowercase();
                (if name == "playersays" { said == want } else { said.contains(&want) }).into()
            }
            _ => Val::default(),
        }
    }

    // ---- message codes ----

    /// Replace message codes in `s`: `#c` chat, `#n` nickname, `#a`
    /// account, `#w` selected weapon, `#L`/`#F` level, `#v(expr)`, `#s(name)`, and from 2.02
    /// `#t(i)`, `#e(start,len,text)`, `#T(text)`, and from 2.03 `#p(n)`.
    /// Anything else stays as written.
    fn expand(&mut self, s: &str, depth: usize) -> String {
        if !s.contains('#') || depth > 8 {
            return s.to_string();
        }
        let b = s.as_bytes();
        let mut out = String::new();
        let mut i = 0;
        let mut lit_from = 0;
        while i < b.len() {
            if b[i] != b'#' || i + 1 >= b.len() {
                i += 1;
                continue;
            }
            let code = b[i + 1] as char;
            // `#P1` to `#P30`, `#C0` to `#C4`: the digits are part of the code.
            let digits = super::lexer::code_digits(&b[i + 1..]);
            let num: Option<usize> = (digits > 0).then(|| s[i + 2..i + 2 + digits].parse().unwrap_or(0));
            let open = i + 2 + digits;
            // Argument in parentheses, if there is one.
            let (inner, end) = if b.get(open) == Some(&b'(') {
                let mut d = 0;
                let mut j = open;
                let mut close = None;
                while j < b.len() {
                    match b[j] {
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
                    j += 1;
                }
                match close {
                    Some(c) => (Some(&s[open + 1..c]), c + 1),
                    None => (None, open),
                }
            } else {
                (None, open)
            };
            let era = self.era();
            let rep: Option<String> = match (code, inner) {
                // `#P1` to `#P30`: a gani attribute, of the NPC itself with
                // no index or -1 (2.1 and later). Seihai keeps the ones a
                // script set with `setcharprop`; a player's are not known.
                ('P', x) if num.is_some() && era >= Era::V21 => {
                    let own = x.map_or(true, |x| self.expand(x, depth + 1).trim().parse::<f64>().unwrap_or(0.0) < 0.0);
                    let key = format!("#P{}", num.unwrap_or(0));
                    Some(if own { self.obj.char_props.get(&key).cloned().unwrap_or_default() } else { String::new() })
                }
                ('c', None) => Some(self.core.player.chat.clone()),
                ('n', None) => Some(self.core.player.nick.clone()),
                ('a', None) => Some(self.core.player.account.clone()),
                ('w', None) => Some(self.core.player.weapon.clone()),
                // `#w(i)` and `#W(i)`: name and image of weapon `i` of the
                // whole list (2.04).
                ('w' | 'W', Some(x)) if era >= Era::V204 => {
                    let i = self.expand(x, depth + 1).trim().parse::<f64>().unwrap_or(-1.0);
                    let w = (i >= 0.0).then(|| self.core.player.weapons.get(i as usize)).flatten();
                    Some(w.map(|(n, img)| if code == 'w' { n.clone() } else { img.clone() }).unwrap_or_default())
                }
                ('L', None) | ('F', None) => Some(self.core.level.clone()),
                ('v', Some(x)) => match parse_expr(x, era) {
                    Some(e) => Some(self.eval(&e).text()),
                    None => Some(String::new()),
                },
                ('s', Some(x)) => {
                    let p = Place { base: x.trim().to_ascii_lowercase(), key: x.trim().to_ascii_lowercase(), index: None, index2: None };
                    Some(self.get(&p).text())
                }
                ('t', Some(x)) if era >= Era::V202 => {
                    let n = self.expand(x, depth + 1);
                    let n: usize = n.trim().parse::<f64>().unwrap_or(0.0).max(0.0) as usize;
                    Some(self.core.tokens.get(n).cloned().unwrap_or_default())
                }
                ('T', Some(x)) if era >= Era::V202 => Some(self.expand(x, depth + 1).trim().to_string()),
                // `#I(list,index)`: an entry of a string list (2.2 list).
                ('I', Some(x)) if era >= Era::V21 => {
                    let (list, index) = x.rsplit_once(',').unwrap_or((x, "0"));
                    let i = self.expand(index, depth + 1).trim().parse::<f64>().unwrap_or(0.0);
                    let list = self.expand(list, depth + 1);
                    let name = list.trim().to_ascii_lowercase();
                    // A list is named like `#s(...)`: a flag or variable.
                    let text = self.core.flags.get(&name).cloned().unwrap_or(list);
                    Some(if i < 0.0 { String::new() } else { split_list(&text).get(i as usize).cloned().unwrap_or_default() })
                }
                ('e', Some(x)) if era >= Era::V202 => {
                    let parts: Vec<&str> = x.splitn(3, ',').collect();
                    if parts.len() == 3 {
                        let start = self.expand(parts[0], depth + 1).trim().parse::<f64>().unwrap_or(0.0).max(0.0) as usize;
                        let len = self.expand(parts[1], depth + 1).trim().parse::<f64>().unwrap_or(0.0).max(0.0) as usize;
                        let text = self.expand(parts[2], depth + 1);
                        Some(text.chars().skip(start).take(len).collect())
                    } else {
                        Some(String::new())
                    }
                }
                ('p', Some(x)) if era >= Era::V203 => {
                    let n = self.expand(x, depth + 1).trim().parse::<f64>().unwrap_or(0.0).max(0.0) as usize;
                    Some(self.obj.params.get(n).cloned().unwrap_or_default())
                }
                _ => None,
            };
            match rep {
                Some(r) => {
                    out.push_str(&s[lit_from..i]);
                    out.push_str(&r);
                    i = end;
                    lit_from = i;
                }
                None => i += 1,
            }
        }
        out.push_str(&s[lit_from..]);
        out
    }

    // ---- commands ----

    fn command(&mut self, cmd: &'static Cmd, args: &[Arg]) {
        let vals: Vec<Val> = args
            .iter()
            .enumerate()
            .map(|(i, a)| match a {
                Arg::Num(e) => self.eval(e),
                // The first word of `setplayerprop` names the property
                // (`#c` is the chat text, `#n` the nickname, ...). It is a
                // code, not a message to expand: `setplayerprop #c,hi` must
                // not turn into `setplayerprop <what was said>,hi`.
                Arg::Text(t) if i == 0 && matches!(cmd.name, "setplayerprop" | "setcharprop") => Val::Str(t.clone()),
                Arg::Text(t) => Val::Str(self.expand(t, 0)),
            })
            .collect();
        if self.ended {
            return;
        }
        match cmd.exec {
            // `setfocus x,y` / `resetfocus`: 2.05 has no executor for them,
            // but 2gta's title level (`splashscreen.nw`, written for 2.1+
            // clients) moves the player to y 65 and draws its title around
            // `setfocus 30,30`, so later builds must move the view. Inferred
            // from that script, not read in a 2.1+ binary.
            Exec::NoExec if self.era() >= Era::V21 && matches!(cmd.name, "setfocus" | "resetfocus") => {
                self.effect_command(cmd.name, vals);
            }
            Exec::NoExec => self.note(format!("`{}` compiles but does nothing in 2.05", cmd.name)),
            Exec::Ext => {
                if cmd.name == "sendtoserver" {
                    self.core.effects.push(Effect::SendToServer { text: sv(&vals, 0) });
                }
            }
            Exec::Engine => self.engine_command(cmd.name, &vals),
            Exec::Effect => {
                if vocab::online_gated(cmd.name) && self.core.online && !self.core.npc_server {
                    return;
                }
                self.effect_command(cmd.name, vals);
                if vocab::ends_run(cmd.name) {
                    self.ended = true;
                }
            }
        }
    }

    fn engine_command(&mut self, name: &str, v: &[Val]) {
        match name {
            "set" => {
                let text = sv(v, 0);
                let (k, val) = text.split_once('=').unwrap_or((text.as_str(), ""));
                let k = k.trim().to_ascii_lowercase();
                if !k.is_empty() {
                    self.write_flag(&k, val.trim().to_string());
                }
            }
            "unset" => {
                let k = sv(v, 0).trim().to_ascii_lowercase();
                if self.core.flags.remove(&k).is_some() && !k.starts_with("local.") {
                    self.core.effects.push(Effect::UnsetFlag { name: k });
                }
            }
            "setstring" => {
                let k = sv(v, 0).trim().to_ascii_lowercase();
                let val = sv(v, 1);
                if k.is_empty() {
                    return;
                }
                if val.is_empty() {
                    if self.core.flags.remove(&k).is_some() {
                        self.core.effects.push(Effect::UnsetFlag { name: k });
                    }
                } else {
                    self.write_flag(&k, val);
                }
            }
            "addstring" | "insertstring" | "replacestring" | "removestring" | "deletestring" => {
                let k = sv(v, 0).trim().to_ascii_lowercase();
                if k.is_empty() {
                    return;
                }
                let cur = self.core.flags.get(&k).cloned().unwrap_or_default();
                // Before 2.1 `addstring` adds text to the string.
                if name == "addstring" && self.era() < Era::V21 {
                    self.write_flag(&k, format!("{cur}{}", sv(v, 1)));
                    return;
                }
                let mut items = split_list(&cur);
                let at = |i: usize| nv(v, i).max(0.0).floor() as usize;
                match name {
                    "addstring" => items.push(sv(v, 1)),
                    "insertstring" => items.insert(at(1).min(items.len()), sv(v, 2)),
                    "replacestring" if at(1) < items.len() => items[at(1)] = sv(v, 2),
                    "removestring" => {
                        let t = sv(v, 1);
                        items.retain(|s| s.trim() != t.trim());
                    }
                    "deletestring" if at(1) < items.len() => {
                        items.remove(at(1));
                    }
                    _ => return,
                }
                self.write_flag(&k, join_list(&items));
            }
            "setarray" => {
                let k = sv(v, 0).trim().to_ascii_lowercase();
                // Only a plain or `this.` variable can be an array.
                if k.is_empty() || !(k.starts_with("this.") || !k.contains('.')) {
                    return;
                }
                let n = nv(v, 1).max(0.0).floor();
                self.obj.vars.insert(format!("{k}#len"), Val::Num(n));
            }
            "tokenize" => self.core.tokens = tokenize(&sv(v, 0), ""),
            "tokenize2" => self.core.tokens = tokenize(&sv(v, 0), &sv(v, 1)),
            "sleep" => {
                let secs = nv(v, 0);
                if secs == 0.0 {
                    return;
                }
                self.obj.due = if secs > 0.0 { Some(self.core.tick + secs_to_ticks(secs)) } else { None };
                self.obj.resume = Some(self.top_pc + 1);
                self.ended = true;
            }
            "callnpc" | "callweapon" => {
                let idx = nv(v, 0).max(0.0) as usize;
                let target = if name == "callnpc" { Target::Npc(idx) } else { Target::Weapon(idx) };
                self.core.pending.push_back(Pending { target, event: sv(v, 1).trim().to_ascii_lowercase(), params: Vec::new() });
            }
            _ => {}
        }
    }

    fn effect_command(&mut self, name: &str, v: Vec<Val>) {
        let id = self.obj.id;
        match name {
            "message" => {
                self.obj.message = sv(&v, 0);
                self.core.effects.push(Effect::Message { obj: id, text: sv(&v, 0) });
            }
            "say" => self.core.effects.push(Effect::SaySign { index: nv(&v, 0) as i64 }),
            "say2" => self.core.effects.push(Effect::Say2 { text: sv(&v, 0) }),
            "show" | "showlocal" | "hide" | "hidelocal" => {
                let visible = name.starts_with("show");
                self.obj.visible = visible;
                self.core.effects.push(Effect::SetVisible { obj: id, visible, local: name.ends_with("local") });
            }
            "setimg" | "setgif" | "setimgpart" | "setgifpart" => {
                let file = sv(&v, 0);
                let part = (v.len() >= 5).then(|| (nv(&v, 1), nv(&v, 2), nv(&v, 3), nv(&v, 4)));
                self.obj.image = file.clone();
                self.obj.visible = true;
                self.core.effects.push(Effect::SetImg { obj: id, file, part });
            }
            "setshape" => {
                let kind = nv(&v, 0) as i64;
                // Only types 0 and 1 are used.
                if kind == 0 || kind == 1 {
                    self.obj.w_px = nv(&v, 1).max(0.0);
                    self.obj.h_px = nv(&v, 2).max(0.0);
                    self.core.effects.push(Effect::SetShape { obj: id, kind, w: self.obj.w_px, h: self.obj.h_px });
                }
            }
            "move" => {
                let (dx, dy) = (nv(&v, 0), nv(&v, 1));
                // Before 2.03 `move` takes only the two offsets.
                let (time, options) = if self.era() >= Era::V203 { (nv(&v, 2), nv(&v, 3) as i64) } else { (0.0, 0) };
                self.obj.x += dx;
                self.obj.y += dy;
                self.core.effects.push(Effect::Move { obj: id, dx, dy, time, options });
                if options & 8 != 0 && self.era() >= Era::V203 {
                    self.core.pending.push_back(Pending { target: Target::Id(id), event: "movementfinished".into(), params: Vec::new() });
                }
            }
            "destroy" => {
                self.obj.destroyed = true;
                self.obj.due = None;
                self.core.effects.push(Effect::Destroy { obj: id });
            }
            "setlevel" => self.core.effects.push(Effect::SetLevel { level: sv(&v, 0), x: None, y: None }),
            "setlevel2" => self.core.effects.push(Effect::SetLevel { level: sv(&v, 0), x: Some(nv(&v, 1)), y: Some(nv(&v, 2)) }),
            "hurt" => {
                let half = nv(&v, 0);
                self.core.player.hearts = (self.core.player.hearts - half / 2.0).max(0.0);
                self.core.effects.push(Effect::Hurt { half_hearts: half });
            }
            "setcharani" => self.core.effects.push(Effect::SetCharAni { obj: id, ani: sv(&v, 0), params: sv(&v, 1) }),
            "setcharprop" => {
                self.obj.char_props.insert(sv(&v, 0).trim().to_string(), sv(&v, 1));
                self.core.effects.push(Effect::SetCharProp { obj: id, code: sv(&v, 0), value: sv(&v, 1) });
            }
            "setplayerprop" => {
                if sv(&v, 0).trim() == "#c" {
                    self.core.player.chat = sv(&v, 1);
                }
                self.core.effects.push(Effect::SetPlayerProp { code: sv(&v, 0), value: sv(&v, 1) });
            }
            "play" | "playlooped" => self.core.effects.push(Effect::Play { obj: id, file: sv(&v, 0), looped: name == "playlooped", at: None }),
            "play2" => self.core.effects.push(Effect::Play { obj: id, file: sv(&v, 0), looped: false, at: Some((nv(&v, 1), nv(&v, 2))) }),
            "sendrpgmessage" => self.core.effects.push(Effect::RpgMessage { text: sv(&v, 0) }),
            "stopsound" => self.core.effects.push(Effect::StopSound { file: sv(&v, 0) }),
            "stopmidi" => self.core.effects.push(Effect::StopMidi),
            "triggeraction" => {
                let (x, y) = (nv(&v, 0), nv(&v, 1));
                let action = sv(&v, 2).trim().to_ascii_lowercase();
                let params = sv(&v, 3);
                self.core.effects.push(Effect::TriggerAction { x, y, action: action.clone(), params: params.clone() });
                // Locally: the object under (x, y) gets `action<name>`.
                self.core.pending.push_back(Pending {
                    target: Target::At(x, y),
                    event: format!("action{action}"),
                    params: params.split(',').map(|s| s.trim().to_string()).collect(),
                });
            }
            _ => {
                self.core.effects.push(Effect::Command { obj: id, name: name.to_string(), args: v });
            }
        }
    }
}

/// `tokenize`: split on spaces and commas (and `extra`); a double-quoted
/// piece is one token.
pub fn tokenize(s: &str, extra: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut have = false;
    for c in s.chars() {
        if c == '"' {
            quoted = !quoted;
            have = true;
        } else if !quoted && (c == ' ' || c == ',' || extra.contains(c)) {
            if have {
                out.push(std::mem::take(&mut cur));
                have = false;
            }
        } else {
            cur.push(c);
            have = true;
        }
    }
    if have {
        out.push(cur);
    }
    out
}
