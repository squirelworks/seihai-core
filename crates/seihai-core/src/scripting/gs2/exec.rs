//! GS2 executor for the commands the classic client shares (Stage C).
//!
//! Runs a [`Script`] from the reader: jumps, calls into the script's own
//! functions, parameters and returns, `sleep`, the loop limit, the value
//! stack, variables, arithmetic and comparisons. A call to any other name
//! goes to the [`Host`]; [`EffectHost`] turns the commands GS1 already has
//! (`set`, `unset`, `say2`, `setlevel2`, `showimg`, `hideimg`, …) into the
//! GS1 engine's [`Effect`]s, so whatever applies GS1 effects can apply
//! these. Opcodes outside that set are logged and skipped; they are
//! Stage D. Behaviour was first taken from the 3.153 client's VM
//! (`documentation/gs2-executor.md`); what later VMs change (limits,
//! `%`, `^`, shifts, the opcodes they added) is a [`VmProfile`]
//! (`documentation/gs2-vm-versions.md`). [`Machine::new`] runs 3.153's
//! rules; [`Machine::for_version`] runs the rules of the client being
//! presented as. Nothing here runs on the classic path.
//!
//! The timer commands (`settimer`, `timeout`, `scheduleevent`,
//! `cancelevents`) are the machine's own and run on [`Machine::tick`]
//! (`documentation/gs2-commands.md`).

use super::lists::{self, Value};
use super::maths;
use super::strings::{self, client_int};
use super::vm::{self, VmProfile};
use super::{Op, Operand, Script};
use crate::protocol::ClientVersion;
use crate::scripting::engine::vocab::{self, Exec};
use crate::scripting::engine::world::ObjId;
use crate::scripting::engine::{Effect, Era, Val};
use std::collections::HashMap;

/// Every VM read (3.153 to 6.03r7) refuses a call deeper than this
/// ("callstack overrun"). The stack and loop limits are in [`VmProfile`].
const CALL_LIMIT: usize = 10_000;
/// Seihai's own guard against a script that never yields.
const STEP_LIMIT: u64 = 2_000_000;
/// One timer frame in seconds. GServer-v2 counts `timeout` and scheduled
/// events in frames of 1/20 s (`TNPC::runScriptTimer`); call
/// [`Machine::tick`] once per frame.
pub const TICK: f64 = 0.05;

/// The object roots a script can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Root {
    This,
    ThisO,
    Player,
    PlayerO,
    Level,
    /// `temp`: the running function's locals (the global object at top level).
    Temp,
    Global,
}

/// One stack entry.
#[derive(Debug, Clone, PartialEq)]
enum Slot {
    /// `OP_TYPE_ARRAY`: the start of an argument or parameter list.
    Marker,
    Num(f64),
    Str(String),
    /// An object: a root and a dotted path under it (empty for the root
    /// itself). `None` is `null`.
    Obj(Option<(Root, String)>),
    /// A variable not read yet: its root (`None` when unqualified) and name.
    Var(Option<Root>, String),
    /// A list value (a literal, `new[n]`, `tokenize`, …).
    List(Vec<Value>),
}

impl From<Value> for Slot {
    fn from(v: Value) -> Slot {
        match v {
            Value::Val(Val::Num(n)) => Slot::Num(n),
            Value::Val(Val::Str(s)) => Slot::Str(s),
            Value::List(l) => Slot::List(l),
        }
    }
}

/// What the script needs from its owner.
pub trait Host {
    /// A call by name that is not one of the script's functions. `None`
    /// means no such command: the call is logged and gives 0.
    fn command(&mut self, name: &str, args: &[Val]) -> Option<Val>;
    /// A built-in property such as `x` of `this`. `None` if it is not one.
    fn get_prop(&mut self, _root: Root, _name: &str) -> Option<Val> {
        None
    }
    /// Writes a built-in property. `false` if it is not one.
    fn set_prop(&mut self, _root: Root, _name: &str, _value: &Val) -> bool {
        false
    }
    fn log(&mut self, line: String);
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The event (or the top-level code) returned.
    Done,
    /// `sleep(t)` with `t > 0`: call [`Machine::resume`] after `t` seconds.
    Sleep(f64),
    /// The script stopped: a stack fault, the loop or call limit.
    Stopped(String),
    /// No function of that name.
    NoSuchEvent,
    /// `waitfor(object, event, timeout)` (4.20r8 on): the run waits for
    /// `event` on `object` or for `timeout` seconds. Call
    /// [`Machine::resume_waitfor`] with whether the event came.
    WaitFor {
        /// The object as the script named it (`None` for a null object).
        object: Option<String>,
        /// The event without its `on`, as the client stores it.
        event: String,
        timeout: f64,
    },
}

struct Frame {
    /// Where `OP_RET` continues; `None` ends the run.
    ret: Option<usize>,
    /// Stack height where the return value goes (set by `OP_JMP`).
    base: usize,
    locals: HashMap<String, Value>,
    /// `with` blocks open when the function was entered: a `return` from
    /// inside a block closes the ones the function opened.
    with_depth: usize,
    /// Local slots (opcodes 45 and 46), at most 1024.
    slots: Vec<Slot>,
}

/// A `scheduleevent` waiting for its frame.
struct Scheduled {
    /// Frames left.
    left: u32,
    /// The event without its `on` (`Ping` for `onPing`).
    event: String,
    args: Vec<Val>,
}

/// A loaded script and its variables.
pub struct Machine {
    script: Script,
    profile: &'static VmProfile,
    /// The loop limit in force: the profile's, or `maxlooplimit`.
    loop_limit: u32,
    /// Local slots used outside any function.
    top_slots: Vec<Slot>,
    vars: HashMap<(Root, String), Value>,
    stack: Vec<Slot>,
    frames: Vec<Frame>,
    pc: usize,
    loops: HashMap<usize, u32>,
    running: bool,
    /// `timeout` in frames; 0 is off.
    timeout: u32,
    scheduled: Vec<Scheduled>,
    /// Events whose frame came while a run was asleep, in order.
    due: Vec<(String, Vec<Val>)>,
    /// The objects of the `with` blocks the run is in, innermost last.
    with: Vec<Root>,
    /// For each open `with`: the first op of its body, the op its
    /// `OP_WITH` skips to, and the call depth it was opened at. A jump out
    /// of the body (`break`/`continue`, which the compiler emits as a
    /// plain jump past `OP_WITHEND`) closes it.
    with_span: Vec<(usize, usize, usize)>,
}

/// Seconds to whole frames, as GServer-v2 does it: `(int)(t * 20)`.
fn frames(secs: f64) -> u32 {
    if secs > 0.0 {
        (secs * 20.0) as u32
    } else {
        0
    }
}

/// An event name without a leading `on`, as the client's dispatcher
/// takes both `Ping` and `onPing`.
fn event_name(name: &str) -> &str {
    match name.get(..2) {
        Some(p) if p.eq_ignore_ascii_case("on") && name.len() > 2 => &name[2..],
        _ => name,
    }
}

impl Machine {
    /// A machine with the 3.153 VM's rules.
    pub fn new(script: Script) -> Machine {
        Machine::with_profile(script, &vm::V3153)
    }

    /// A machine with the VM rules of the client `version`.
    pub fn for_version(script: Script, version: ClientVersion) -> Machine {
        Machine::with_profile(script, VmProfile::for_version(version))
    }

    pub fn with_profile(script: Script, profile: &'static VmProfile) -> Machine {
        Machine {
            script,
            profile,
            loop_limit: profile.loop_limit,
            top_slots: Vec::new(),
            vars: HashMap::new(),
            stack: Vec::new(),
            frames: Vec::new(),
            pc: 0,
            loops: HashMap::new(),
            running: false,
            timeout: 0,
            scheduled: Vec::new(),
            due: Vec::new(),
            with: Vec::new(),
            with_span: Vec::new(),
        }
    }

    pub fn script(&self) -> &Script {
        &self.script
    }

    /// The VM rules this machine runs by.
    pub fn profile(&self) -> &'static VmProfile {
        self.profile
    }

    /// A script variable (not a host property). `This` and `ThisO` share one
    /// set of variables, as they are the same object for a script's owner.
    pub fn var(&self, root: Root, name: &str) -> Option<&Val> {
        match self.vars.get(&(Self::store(root), name.to_string()))? {
            Value::Val(v) => Some(v),
            Value::List(_) => None,
        }
    }

    /// A script variable that holds a list.
    pub fn list(&self, root: Root, name: &str) -> Option<&[Value]> {
        match self.vars.get(&(Self::store(root), name.to_string()))? {
            Value::List(l) => Some(l),
            Value::Val(_) => None,
        }
    }

    /// Runs the code outside any function from op 0 (the compiler jumps over
    /// each function body), as when the script is loaded.
    pub fn run_top_level(&mut self, host: &mut dyn Host) -> Outcome {
        self.stack.clear();
        self.frames.clear();
        self.loops.clear();
        self.with.clear();
        self.with_span.clear();
        self.top_slots.clear();
        self.pc = 0;
        self.running = true;
        self.exec(host)
    }

    /// Calls `event` (for example `onCreated`) with `args`, as a call from
    /// outside would: `public.` and plain names both match.
    pub fn call_event(&mut self, event: &str, args: &[Val], host: &mut dyn Host) -> Outcome {
        let Some(start) = self.find_function(event) else {
            return Outcome::NoSuchEvent;
        };
        self.stack.clear();
        self.frames.clear();
        self.loops.clear();
        self.with.clear();
        self.with_span.clear();
        self.top_slots.clear();
        self.stack.push(Slot::Marker);
        for a in args.iter().rev() {
            self.stack.push(match a {
                Val::Num(n) => Slot::Num(*n),
                Val::Str(s) => Slot::Str(s.clone()),
            });
        }
        self.frames.push(Frame {
            with_depth: self.with.len(),
            ret: None,
            base: 0,
            locals: HashMap::new(),
            slots: Vec::new(),
        });
        self.pc = start;
        self.running = true;
        self.exec(host)
    }

    /// The `timeout` left, in seconds (0 when off).
    pub fn timeout(&self) -> f64 {
        self.timeout as f64 * TICK
    }

    /// Advances the timers by one frame ([`TICK`]) and runs what is due:
    /// `onTimeout` when `timeout` reaches 0, then each scheduled event
    /// whose frame came, in the order they were scheduled. While a run is
    /// asleep nothing new starts (starting an event would end it); what
    /// came due waits for a later frame. Returns each event run and how
    /// it ended.
    pub fn tick(&mut self, host: &mut dyn Host) -> Vec<(String, Outcome)> {
        if self.timeout > 0 {
            self.timeout -= 1;
            if self.timeout == 0 {
                self.due.push(("Timeout".into(), Vec::new()));
            }
        }
        let mut i = 0;
        while i < self.scheduled.len() {
            let e = &mut self.scheduled[i];
            e.left = e.left.saturating_sub(1);
            if e.left == 0 {
                let e = self.scheduled.remove(i);
                self.due.push((e.event, e.args));
            } else {
                i += 1;
            }
        }
        let mut ran = Vec::new();
        while !self.running && !self.due.is_empty() {
            let (event, args) = self.due.remove(0);
            let name = format!("on{event}");
            let out = self.call_event(&name, &args, host);
            ran.push((name, out));
        }
        ran
    }

    /// The timer commands. `None` if `name` is not one of them.
    fn timer_command(&mut self, name: &str, args: &[Val]) -> Option<Val> {
        let lower = name.to_ascii_lowercase();
        let cmd = lower
            .strip_prefix("this.")
            .or_else(|| lower.strip_prefix("thiso."))
            .unwrap_or(&lower);
        match cmd {
            "settimer" => self.timeout = frames(args.first().map_or(0.0, Val::num)),
            "scheduleevent" => {
                let event = event_name(&args.get(1).map(Val::text).unwrap_or_default()).to_string();
                // GServer-v2 never fires a 0-frame event (it counts down
                // past 0); Seihai runs it on the next frame.
                let left = frames(args.first().map_or(0.0, Val::num)).max(1);
                self.scheduled.push(Scheduled {
                    left,
                    event,
                    args: args.get(2..).unwrap_or_default().to_vec(),
                });
            }
            "cancelevents" => {
                let event = args.first().map(Val::text).unwrap_or_default();
                let event = event_name(&event);
                self.scheduled
                    .retain(|e| !e.event.eq_ignore_ascii_case(event));
                self.due.retain(|e| !e.0.eq_ignore_ascii_case(event));
            }
            _ => return None,
        }
        Some(Val::Num(0.0))
    }

    /// Continues after `Outcome::Sleep`.
    pub fn resume(&mut self, host: &mut dyn Host) -> Outcome {
        if !self.running {
            return Outcome::Done;
        }
        self.exec(host)
    }

    /// Continues after `Outcome::WaitFor`: `waitfor` gives 1 when the event
    /// came and 0 when the time ran out. The clients (read in 5.12 and
    /// 6.00r2) store 1 as the result when the wait starts, and their timer
    /// sets it to 0 before it resumes a wait whose time has passed. A wait
    /// times out once the clock plus 0.0001 is past its end.
    pub fn resume_waitfor(&mut self, caught: bool, host: &mut dyn Host) -> Outcome {
        if !self.running {
            return Outcome::Done;
        }
        self.stack.push(Slot::Num(if caught { 1.0 } else { 0.0 }));
        self.exec(host)
    }

    /// The local slots of the running function (or of the top level).
    fn slots(&mut self) -> &mut Vec<Slot> {
        match self.frames.last_mut() {
            Some(f) => &mut f.slots,
            None => &mut self.top_slots,
        }
    }

    /// The opcodes gs2-parser does not name that the client VMs run
    /// (`documentation/gs2-vm-versions.md`). `Some` ends the run.
    fn run_extra(
        &mut self,
        n: u8,
        operand: &Operand,
        at: usize,
        host: &mut dyn Host,
    ) -> Option<Outcome> {
        let fault = |m: &mut Machine, host: &mut dyn Host| {
            Some(m.stop(format!("stack fault at opcode {n} (op {at})"), host))
        };
        let index = operand_num(operand).map(client_int);
        match n {
            // Copies the top into a local slot without popping it.
            vm::op::SLOT_STORE => {
                let Some(top) = self.stack.last().cloned() else {
                    return fault(self, host);
                };
                if let Some(i) = index.filter(|&i| i >= 0 && (i as usize) < SLOT_LIMIT) {
                    let slots = self.slots();
                    let i = i as usize;
                    if slots.len() <= i {
                        slots.resize(i + 1, Slot::Num(0.0));
                    }
                    slots[i] = top;
                }
            }
            // Pushes a copy of a local slot.
            vm::op::SLOT_LOAD => {
                let slot = index
                    .filter(|&i| i >= 0)
                    .and_then(|i| self.slots().get(i as usize).cloned())
                    .unwrap_or(Slot::Num(0.0));
                if !self.push(slot) {
                    return fault(self, host);
                }
            }
            // Turns the top into a variable reference for the slots. A
            // variable is read when it is used, so it stays as it is.
            vm::op::RESOLVE_VAR => {
                if self.stack.is_empty() {
                    return fault(self, host);
                }
            }
            // `a && b` and `a || b` as values (1 or 0), both sides run.
            vm::op::LOGICAL_AND | vm::op::LOGICAL_OR => {
                let (Some(b), Some(a)) = (self.pop_num(host), self.pop_num(host)) else {
                    return fault(self, host);
                };
                let t = if n == vm::op::LOGICAL_AND {
                    a != 0.0 && b != 0.0
                } else {
                    a != 0.0 || b != 0.0
                };
                self.push(Slot::Num(t as i32 as f64));
            }
            // `+` that joins when either side is a string (5.00r7 on).
            vm::op::ADD_OR_JOIN => {
                let (Some(b), Some(a)) = (self.pop(), self.pop()) else {
                    return fault(self, host);
                };
                let join = matches!(a, Slot::Str(_)) || matches!(b, Slot::Str(_));
                let (a, b) = (self.value(a, host), self.value(b, host));
                self.push(if join {
                    Slot::Str(val_text(&a) + &val_text(&b))
                } else {
                    Slot::Num(a.num() + b.num())
                });
            }
            // A list of numbers from the values on the stack (5.00r7).
            vm::op::NUMBER_LIST => {
                let count = index.unwrap_or(0).clamp(0, 10_000) as usize;
                if self.stack.len() < count {
                    return fault(self, host);
                }
                let at = self.stack.len() - count;
                let items: Vec<Slot> = self.stack.drain(at..).collect();
                let list = items
                    .into_iter()
                    .map(|s| Value::Val(Val::Num(self.value(s, host).num())))
                    .collect();
                self.push(Slot::List(list));
            }
            // `obj.(name) = value` in one opcode (5.00r7 on): the object
            // is converted as `OP_CONV_TO_OBJECT` does, the name taken as
            // `OP_MEMBER_ACCESS` takes it, and the value stored as
            // `OP_ASSIGN` stores it. All three are popped.
            vm::op::MEMBER_STORE => {
                let (Some(value), Some(name), Some(obj)) = (self.pop(), self.pop(), self.pop())
                else {
                    return fault(self, host);
                };
                let name = match name {
                    Slot::Var(_, n) | Slot::Str(n) => n,
                    _ => String::new(),
                };
                let target = match obj {
                    Slot::Var(root, path) => Some((root.unwrap_or(Root::Global), path)),
                    Slot::Obj(o) => o,
                    _ => None,
                };
                let value = self.value_full(value, host);
                if let Some((root, path)) = target {
                    if !name.is_empty() {
                        let full = if path.is_empty() {
                            name
                        } else {
                            format!("{path}.{name}")
                        };
                        self.write_var(Some(root), &full, value, host);
                    }
                }
            }
            // An object literal from name/value pairs (5.00r7). Seihai has
            // no object values: the pairs are dropped and the result is null.
            vm::op::OBJECT_LITERAL => {
                let pairs = index.unwrap_or(0).clamp(0, 10_000) as usize;
                if self.stack.len() < pairs * 2 {
                    return fault(self, host);
                }
                let keep = self.stack.len() - pairs * 2;
                self.stack.truncate(keep);
                host.log(format!(
                    "gs2: object literal at op {at}: Seihai has no object values, gives null"
                ));
                self.push(Slot::Obj(None));
            }
            _ => host.log(format!(
                "gs2: opcode {n} at op {at} is not run yet, skipped"
            )),
        }
        None
    }

    fn find_function(&self, name: &str) -> Option<usize> {
        let public = format!("public.{name}");
        self.script
            .functions
            .iter()
            .find(|f| f.name == name || f.name == public)
            .map(|f| f.start)
    }

    /// What `this` is: the innermost `with` object, else the script's own.
    fn this_root(&self) -> Root {
        self.with.last().copied().unwrap_or(Root::This)
    }

    fn store(root: Root) -> Root {
        match root {
            Root::ThisO => Root::This,
            Root::PlayerO => Root::Player,
            r => r,
        }
    }

    fn stop(&mut self, why: String, host: &mut dyn Host) -> Outcome {
        host.log(format!("gs2: {why}"));
        self.running = false;
        self.stack.clear();
        self.frames.clear();
        self.with.clear();
        self.with_span.clear();
        Outcome::Stopped(why)
    }

    // --- variables -------------------------------------------------------

    fn read_var(&mut self, root: Option<Root>, name: &str, host: &mut dyn Host) -> Val {
        self.read_value(root, name, host).val()
    }

    fn read_value(&mut self, root: Option<Root>, name: &str, host: &mut dyn Host) -> Value {
        match root {
            Some(Root::Temp) => match self.frames.last() {
                Some(f) => f.locals.get(name).cloned().unwrap_or_default(),
                None => self
                    .vars
                    .get(&(Root::Global, name.to_string()))
                    .cloned()
                    .unwrap_or_default(),
            },
            Some(Root::This | Root::ThisO) if name.eq_ignore_ascii_case("timeout") => {
                Val::Num(self.timeout()).into()
            }
            Some(Root::This | Root::ThisO)
                if self.profile.max_loop_limit && name.eq_ignore_ascii_case("maxlooplimit") =>
            {
                Val::Num(self.loop_limit as f64).into()
            }
            Some(r) => {
                if let Some(v) = host.get_prop(r, name) {
                    return v.into();
                }
                self.vars
                    .get(&(Self::store(r), name.to_string()))
                    .cloned()
                    .unwrap_or_default()
            }
            // Unqualified: the function's locals, then the object, then the
            // level, then the global object (the 3.153 resolver's order).
            None => {
                if let Some(v) = self.frames.last().and_then(|f| f.locals.get(name)) {
                    return v.clone();
                }
                let this = self.this_root();
                if this == Root::This && name.eq_ignore_ascii_case("timeout") {
                    return Val::Num(self.timeout()).into();
                }
                if this == Root::This
                    && self.profile.max_loop_limit
                    && name.eq_ignore_ascii_case("maxlooplimit")
                {
                    return Val::Num(self.loop_limit as f64).into();
                }
                if let Some(v) = host.get_prop(this, name) {
                    return v.into();
                }
                for r in [Self::store(this), Root::Level, Root::Global] {
                    if let Some(v) = self.vars.get(&(r, name.to_string())) {
                        return v.clone();
                    }
                }
                Value::default()
            }
        }
    }

    fn write_var(&mut self, root: Option<Root>, name: &str, value: Value, host: &mut dyn Host) {
        let target = match root {
            Some(Root::Temp) => {
                match self.frames.last_mut() {
                    Some(f) => {
                        f.locals.insert(name.to_string(), value);
                    }
                    None => {
                        self.vars.insert((Root::Global, name.to_string()), value);
                    }
                }
                return;
            }
            Some(Root::This | Root::ThisO) if name.eq_ignore_ascii_case("timeout") => {
                self.timeout = frames(value.num());
                return;
            }
            // `TGraalVar::setMaxLoopLimit` (5.00r7): 100 to 1 000 000.
            Some(Root::This | Root::ThisO)
                if self.profile.max_loop_limit && name.eq_ignore_ascii_case("maxlooplimit") =>
            {
                self.loop_limit = (client_int(value.num()).clamp(100, 1_000_000)) as u32;
                return;
            }
            Some(r) => {
                if host.set_prop(r, name, &value.val()) {
                    return;
                }
                Self::store(r)
            }
            None => {
                if let Some(f) = self
                    .frames
                    .last_mut()
                    .filter(|f| f.locals.contains_key(name))
                {
                    f.locals.insert(name.to_string(), value);
                    return;
                }
                let this = self.this_root();
                if this == Root::This && name.eq_ignore_ascii_case("timeout") {
                    self.timeout = frames(value.num());
                    return;
                }
                if this == Root::This
                    && self.profile.max_loop_limit
                    && name.eq_ignore_ascii_case("maxlooplimit")
                {
                    self.loop_limit = (client_int(value.num()).clamp(100, 1_000_000)) as u32;
                    return;
                }
                if host.set_prop(this, name, &value.val()) {
                    return;
                }
                // A new name goes to the global object, or inside `with`
                // to the `with` object.
                let new = if self.with.is_empty() {
                    Root::Global
                } else {
                    Self::store(this)
                };
                [Self::store(this), Root::Level]
                    .into_iter()
                    .find(|r| self.vars.contains_key(&(*r, name.to_string())))
                    .unwrap_or(new)
            }
        };
        self.vars.insert((target, name.to_string()), value);
    }

    /// A slot as a full value: a variable that holds a list stays a list.
    fn value_full(&mut self, slot: Slot, host: &mut dyn Host) -> Value {
        match slot {
            Slot::Var(root, name) => self.read_value(root, &name, host),
            // A variable the compiler converted to an object.
            Slot::Obj(Some((root, path))) if !path.is_empty() => {
                self.read_value(Some(root), &path, host)
            }
            Slot::List(l) => Value::List(l),
            s => self.value(s, host).into(),
        }
    }

    /// The variable a list opcode works on: `a` in `a.add(x)` (the
    /// compiler converts it to an object first).
    fn list_target(slot: &Slot) -> Option<(Option<Root>, String)> {
        match slot {
            Slot::Obj(Some((root, path))) if !path.is_empty() => Some((Some(*root), path.clone())),
            Slot::Var(root, name) if !name.is_empty() => Some((*root, name.clone())),
            _ => None,
        }
    }

    /// The list in a slot. Anything that is not a list is an empty one.
    fn list_of(&mut self, slot: &Slot, host: &mut dyn Host) -> Vec<Value> {
        match self.value_full(slot.clone(), host) {
            Value::List(l) => l,
            Value::Val(_) => Vec::new(),
        }
    }

    /// Changes the list in the variable a slot names, if it names one.
    fn edit_list(&mut self, slot: &Slot, host: &mut dyn Host, f: impl FnOnce(&mut Vec<Value>)) {
        let Some((root, name)) = Self::list_target(slot) else {
            return;
        };
        let mut l = match self.read_value(root, &name, host) {
            Value::List(l) => l,
            Value::Val(_) => Vec::new(),
        };
        f(&mut l);
        self.write_var(root, &name, Value::List(l), host);
    }

    fn value(&mut self, slot: Slot, host: &mut dyn Host) -> Val {
        match slot {
            Slot::Num(n) => Val::Num(n),
            Slot::Str(s) => Val::Str(s),
            Slot::Var(root, name) => self.read_var(root, &name, host),
            Slot::Obj(Some((_, path))) => Val::Str(path),
            Slot::List(l) => Value::List(l).val(),
            Slot::Obj(None) | Slot::Marker => Val::Num(0.0),
        }
    }

    // --- stack -----------------------------------------------------------

    fn pop(&mut self) -> Option<Slot> {
        self.stack.pop()
    }

    fn pop_val(&mut self, host: &mut dyn Host) -> Option<Val> {
        let s = self.stack.pop()?;
        Some(self.value(s, host))
    }

    fn pop_num(&mut self, host: &mut dyn Host) -> Option<f64> {
        self.pop_val(host).map(|v| v.num())
    }

    fn push(&mut self, s: Slot) -> bool {
        if self
            .profile
            .stack_limit
            .is_some_and(|limit| self.stack.len() >= limit)
        {
            return false;
        }
        self.stack.push(s);
        true
    }

    fn last_marker(&self) -> Option<usize> {
        self.stack.iter().rposition(|s| *s == Slot::Marker)
    }

    // --- the loop --------------------------------------------------------

    fn exec(&mut self, host: &mut dyn Host) -> Outcome {
        let mut steps = 0u64;
        loop {
            steps += 1;
            if steps > STEP_LIMIT {
                return self.stop(
                    format!("more than {STEP_LIMIT} steps without a sleep"),
                    host,
                );
            }
            let Some(ins) = self.script.code.get(self.pc).cloned() else {
                self.running = false;
                return Outcome::Done;
            };
            let at = self.pc;
            self.pc += 1;
            let int = match ins.operand {
                Operand::Int(v) => Some(v),
                _ => None,
            };
            let fault = |m: &mut Machine, host: &mut dyn Host| {
                m.stop(format!("stack fault at {} (op {at})", ins.op), host)
            };
            if !self.profile.runs(ins.op.value()) {
                host.log(format!(
                    "gs2: opcode {} at op {at} is not in the {} VM, skipped",
                    ins.op.value(),
                    self.profile.label
                ));
                continue;
            }
            match ins.op {
                Op::NoOp => {}
                // Jumps. Targets are op indices.
                Op::SetIndex => {
                    self.pc = int.unwrap_or(0).max(0) as usize;
                    while let Some(&(start, end, depth)) = self.with_span.last() {
                        if depth != self.frames.len() || (start..end).contains(&self.pc) {
                            break;
                        }
                        self.with_span.pop();
                        self.with.pop();
                    }
                }
                Op::If | Op::SetIndexTrue => {
                    let Some(v) = self.pop_num(host) else {
                        return fault(self, host);
                    };
                    if (v != 0.0) == (ins.op == Op::SetIndexTrue) {
                        self.pc = int.unwrap_or(0).max(0) as usize;
                    }
                }
                // `&&` / `||` in a value: jump keeping the deciding value,
                // or drop it and go on to the right side.
                Op::And | Op::Or => {
                    let Some(top) = self.stack.last().cloned() else {
                        return fault(self, host);
                    };
                    let v = self.value(top, host).num();
                    if (v != 0.0) == (ins.op == Op::Or) {
                        self.pc = int.unwrap_or(0).max(0) as usize;
                    } else {
                        self.pop();
                    }
                }
                Op::InlineConditional => {
                    let Some(top) = self.pop_val(host) else {
                        return fault(self, host);
                    };
                    let n = top.num();
                    self.push(Slot::Num(if n != 0.0 { 1.0 } else { 0.0 }));
                }
                Op::CmdCall => {
                    let n = self.loops.entry(at).or_insert(0);
                    *n += 1;
                    if *n > self.loop_limit {
                        return self.stop("Script exceeded loop limit".into(), host);
                    }
                }
                Op::Jmp => {
                    let h = self.stack.len();
                    if let Some(f) = self.frames.last_mut() {
                        f.base = h;
                    }
                }
                Op::Call => {
                    if let Some(o) = self.call(host) {
                        return o;
                    }
                }
                Op::FuncParamsEnd => self.bind_params(host),
                Op::Ret => {
                    if self.frames.is_empty() {
                        self.running = false;
                        self.stack.clear();
                        return Outcome::Done;
                    }
                    // The value is read while the function's own variables
                    // are still there (`return a;`, `return temp.x;`).
                    let v = match self.pop() {
                        Some(s) => self.value_full(s, host),
                        None => Value::default(),
                    };
                    let frame = self.frames.pop().expect("checked above");
                    self.stack.truncate(frame.base);
                    self.with.truncate(frame.with_depth);
                    self.with_span.truncate(frame.with_depth);
                    match frame.ret {
                        Some(pc) => {
                            self.pc = pc;
                            self.stack.push(v.into());
                        }
                        None => {
                            self.running = false;
                            self.stack.clear();
                            return Outcome::Done;
                        }
                    }
                }
                Op::Sleep => {
                    let Some(t) = self.pop_num(host) else {
                        return fault(self, host);
                    };
                    if t > 0.0 {
                        return Outcome::Sleep(t);
                    }
                }

                // Pushes.
                Op::TypeNumber
                | Op::TypeString
                | Op::TypeVar
                | Op::TypeArray
                | Op::TypeTrue
                | Op::TypeFalse
                | Op::TypeNull
                | Op::Pi
                | Op::This
                | Op::ThisO
                | Op::Player
                | Op::PlayerO
                | Op::Level
                | Op::Temp => {
                    let text = self.script.string(&ins).unwrap_or("").to_string();
                    let slot = match ins.op {
                        Op::TypeNumber => Slot::Num(ins.number().unwrap_or(0.0)),
                        Op::TypeString => Slot::Str(text),
                        Op::TypeVar => Slot::Var(None, text),
                        Op::TypeArray => Slot::Marker,
                        Op::TypeTrue => Slot::Num(1.0),
                        Op::TypeFalse => Slot::Num(0.0),
                        Op::TypeNull => Slot::Obj(None),
                        Op::Pi => Slot::Num(std::f64::consts::PI),
                        Op::This => Slot::Obj(Some((self.this_root(), String::new()))),
                        Op::ThisO => Slot::Obj(Some((Root::ThisO, String::new()))),
                        Op::Player => Slot::Obj(Some((Root::Player, String::new()))),
                        Op::PlayerO => Slot::Obj(Some((Root::PlayerO, String::new()))),
                        Op::Level => Slot::Obj(Some((Root::Level, String::new()))),
                        _ => Slot::Obj(Some((Root::Temp, String::new()))),
                    };
                    if !self.push(slot) {
                        return fault(self, host);
                    }
                }
                Op::CopyLastOp => {
                    let Some(top) = self.stack.last().cloned() else {
                        return fault(self, host);
                    };
                    if !self.push(top) {
                        return fault(self, host);
                    }
                }
                Op::SwapLastOps => {
                    let n = self.stack.len();
                    if n < 2 {
                        return fault(self, host);
                    }
                    self.stack.swap(n - 1, n - 2);
                }
                Op::IndexDec => {
                    if self.pop().is_none() {
                        return fault(self, host);
                    }
                }
                Op::ConvToFloat | Op::ConvToString => {
                    let Some(v) = self.pop_val(host) else {
                        return fault(self, host);
                    };
                    self.push(if ins.op == Op::ConvToFloat {
                        Slot::Num(v.num())
                    } else {
                        Slot::Str(val_text(&v))
                    });
                }
                Op::ConvToObject => {
                    let Some(top) = self.pop() else {
                        return fault(self, host);
                    };
                    self.push(match top {
                        Slot::Var(root, name) => {
                            Slot::Obj(Some((root.unwrap_or(Root::Global), name)))
                        }
                        Slot::Obj(o) => Slot::Obj(o),
                        _ => Slot::Obj(None),
                    });
                }
                // `obj.name`: a variable under the object. A null object
                // gives a variable with no name, which reads as 0.
                Op::MemberAccess => {
                    let (Some(name), Some(obj)) = (self.pop(), self.pop()) else {
                        return fault(self, host);
                    };
                    let name = match name {
                        Slot::Var(_, n) | Slot::Str(n) => n,
                        _ => String::new(),
                    };
                    self.push(match obj {
                        Slot::Obj(Some((root, path))) if path.is_empty() => {
                            Slot::Var(Some(root), name)
                        }
                        Slot::Obj(Some((root, path))) => {
                            Slot::Var(Some(root), format!("{path}.{name}"))
                        }
                        _ => Slot::Var(Some(Root::Global), String::new()),
                    });
                }
                Op::Assign => {
                    let (Some(value), Some(target)) = (self.pop(), self.pop()) else {
                        return fault(self, host);
                    };
                    let value = self.value_full(value, host);
                    if let Slot::Var(root, name) = target {
                        if !name.is_empty() {
                            self.write_var(root, &name, value, host);
                        }
                    }
                }
                Op::Inc | Op::Dec => {
                    let Some(top) = self.stack.last().cloned() else {
                        return fault(self, host);
                    };
                    let d = if ins.op == Op::Inc { 1.0 } else { -1.0 };
                    if let Slot::Var(root, name) = &top {
                        let n = self.read_var(*root, name, host).num() + d;
                        self.write_var(*root, name, Val::Num(n).into(), host);
                    } else if let Slot::Num(n) = top {
                        *self.stack.last_mut().unwrap() = Slot::Num(n + d);
                    }
                }

                // Arithmetic on numbers; the compiler converts the operands.
                Op::Add
                | Op::Sub
                | Op::Mul
                | Op::Div
                | Op::Mod
                | Op::Pow
                | Op::BitOr
                | Op::BitAnd
                | Op::BitXor
                | Op::Min
                | Op::Max => {
                    let (Some(b), Some(a)) = (self.pop_num(host), self.pop_num(host)) else {
                        return fault(self, host);
                    };
                    let (ia, ib) = (client_int(a), client_int(b));
                    self.push(Slot::Num(match ins.op {
                        Op::Add => a + b,
                        Op::Sub => a - b,
                        Op::Mul => a * b,
                        // Division by zero gives 0 in every VM.
                        Op::Div if b == 0.0 => 0.0,
                        Op::Div => a / b,
                        Op::Mod => self.profile.modulo(a, b),
                        Op::Pow => self.profile.pow(a, b),
                        Op::BitOr => (ia | ib) as f64,
                        Op::BitAnd => (ia & ib) as f64,
                        Op::BitXor => (ia ^ ib) as f64,
                        Op::Min => a.min(b),
                        _ => a.max(b),
                    }));
                }
                Op::Random => {
                    let (Some(b), Some(a)) = (self.pop_num(host), self.pop_num(host)) else {
                        return fault(self, host);
                    };
                    let r = rand_unit();
                    self.push(Slot::Num(a + r * (b - a)));
                }
                Op::Not
                | Op::UnarySub
                | Op::BitInvert
                | Op::Int
                | Op::Abs
                | Op::Sin
                | Op::Cos
                | Op::ArcTan => {
                    let Some(a) = self.pop_num(host) else {
                        return fault(self, host);
                    };
                    self.push(Slot::Num(match ins.op {
                        Op::Not => (a == 0.0) as i32 as f64,
                        Op::UnarySub => -a,
                        Op::BitInvert => !client_int(a) as f64,
                        Op::Int => client_int(a) as f64,
                        Op::Abs => a.abs(),
                        Op::Sin => a.sin(),
                        Op::Cos => a.cos(),
                        _ => a.atan(),
                    }));
                }
                Op::Eq | Op::Neq | Op::Lt | Op::Gt | Op::Lte | Op::Gte => {
                    let (Some(b), Some(a)) = (self.pop_val(host), self.pop_val(host)) else {
                        return fault(self, host);
                    };
                    let c = compare(&a, &b);
                    let t = match ins.op {
                        Op::Eq => c == 0,
                        Op::Neq => c != 0,
                        Op::Lt => c < 0,
                        Op::Gt => c > 0,
                        Op::Lte => c <= 0,
                        _ => c >= 0,
                    };
                    self.push(Slot::Num(t as i32 as f64));
                }
                Op::Join => {
                    let (Some(b), Some(a)) = (self.pop_val(host), self.pop_val(host)) else {
                        return fault(self, host);
                    };
                    self.push(Slot::Str(val_text(&a) + &val_text(&b)));
                }

                // Strings (Stage D). The compiler converts the object to a
                // string first.
                Op::Char | Op::ObjTrim | Op::ObjLength => {
                    let Some(v) = self.pop_val(host) else {
                        return fault(self, host);
                    };
                    self.push(match ins.op {
                        Op::Char => Slot::Str(strings::char_of(v.num())),
                        Op::ObjTrim => Slot::Str(strings::trim(&val_text(&v))),
                        _ => Slot::Num(val_text(&v).chars().count() as f64),
                    });
                }
                Op::ObjPos | Op::ObjCharAt | Op::ObjStarts | Op::ObjEnds => {
                    let (Some(b), Some(a)) = (self.pop_val(host), self.pop_val(host)) else {
                        return fault(self, host);
                    };
                    let (s, t) = (val_text(&a), val_text(&b));
                    self.push(match ins.op {
                        Op::ObjPos => Slot::Num(strings::pos(&s, &t) as f64),
                        Op::ObjCharAt => Slot::Str(strings::charat(&s, b.num())),
                        Op::ObjStarts => Slot::Num(s.starts_with(t.as_str()) as i32 as f64),
                        _ => Slot::Num(s.ends_with(t.as_str()) as i32 as f64),
                    });
                }
                // Always the string, the start and the length.
                Op::ObjSubstr => {
                    let (Some(len), Some(start), Some(s)) =
                        (self.pop_num(host), self.pop_num(host), self.pop_val(host))
                    else {
                        return fault(self, host);
                    };
                    self.push(Slot::Str(strings::substring(&val_text(&s), start, len)));
                }
                // `_(s)`: the client leaves the value as it is.
                Op::Translate => {
                    if self.stack.is_empty() {
                        return fault(self, host);
                    }
                }
                // The pattern is on top, its arguments below it back to the
                // marker, the first nearest; all of it becomes the result.
                Op::Format => {
                    let Some(m) = self.last_marker() else {
                        return fault(self, host);
                    };
                    let mut vals = Vec::new();
                    while self.stack.len() > m + 1 {
                        let s = self.pop().unwrap();
                        vals.push(self.value(s, host));
                    }
                    self.stack.truncate(m);
                    let text = match vals.split_first() {
                        Some((fmt, args)) => strings::format(&val_text(fmt), args),
                        None => String::new(),
                    };
                    self.push(Slot::Str(text));
                }

                // Lists (Stage D). A list operand is the variable the
                // compiler converted to an object, or a list value.
                Op::ArrayEnd => {
                    let Some(m) = self.last_marker() else {
                        return fault(self, host);
                    };
                    let mut l = Vec::new();
                    while self.stack.len() > m + 1 {
                        let s = self.pop().unwrap();
                        l.push(self.value_full(s, host));
                    }
                    self.stack.truncate(m);
                    self.push(Slot::List(l));
                }
                Op::ArrayNew => {
                    let Some(n) = self.pop_num(host) else {
                        return fault(self, host);
                    };
                    self.push(Slot::List(lists::zeros(client_int(n))));
                }
                Op::ArrayNewMultidim => {
                    let Some(n) = self.pop_num(host) else {
                        return fault(self, host);
                    };
                    if let Some(Slot::List(l)) = self.stack.last_mut() {
                        fill_leaves(l, client_int(n));
                    }
                }
                Op::ObjSize => {
                    let Some(o) = self.pop() else {
                        return fault(self, host);
                    };
                    let n = match self.value_full(o, host) {
                        Value::List(l) => l.len() as f64,
                        Value::Val(_) => 0.0,
                    };
                    self.push(Slot::Num(n));
                }
                Op::Array | Op::ArrayMultidim => {
                    let j = if ins.op == Op::ArrayMultidim {
                        self.pop_num(host)
                    } else {
                        Some(0.0)
                    };
                    let (Some(j), Some(i), Some(o)) = (j, self.pop_num(host), self.pop()) else {
                        return fault(self, host);
                    };
                    let mut v = self.value_full(o, host);
                    let path = if ins.op == Op::ArrayMultidim {
                        vec![i, j]
                    } else {
                        vec![i]
                    };
                    for k in path {
                        v = match v {
                            Value::List(l) => usize::try_from(client_int(k))
                                .ok()
                                .and_then(|k| l.into_iter().nth(k))
                                .unwrap_or_default(),
                            // A string variable indexed gives the string.
                            other => other,
                        };
                    }
                    self.push(v.into());
                }
                Op::ArrayAssign | Op::ArrayMultidimAssign => {
                    let Some(v) = self.pop() else {
                        return fault(self, host);
                    };
                    let v = self.value_full(v, host);
                    let j = if ins.op == Op::ArrayMultidimAssign {
                        self.pop_num(host)
                    } else {
                        Some(-1.0)
                    };
                    let (Some(j), Some(i), Some(o)) = (j, self.pop_num(host), self.pop()) else {
                        return fault(self, host);
                    };
                    let (i, j) = (client_int(i), client_int(j));
                    self.edit_list(&o, host, |l| {
                        let Some(e) = usize::try_from(i).ok().and_then(|i| l.get_mut(i)) else {
                            return;
                        };
                        if ins.op == Op::ArrayAssign {
                            *e = v;
                        } else if let (Value::List(inner), Ok(j)) = (e, usize::try_from(j)) {
                            if let Some(e) = inner.get_mut(j) {
                                *e = v;
                            }
                        }
                    });
                }
                Op::ObjAddString | Op::ObjRemoveString => {
                    let (Some(v), Some(o)) = (self.pop(), self.pop()) else {
                        return fault(self, host);
                    };
                    let v = self.value_full(v, host);
                    let add = ins.op == Op::ObjAddString;
                    self.edit_list(&o, host, |l| {
                        if add {
                            l.push(v);
                        } else if let Some(k) = l.iter().position(|e| e.same(&v, compare)) {
                            l.remove(k);
                        }
                    });
                }
                Op::ObjDeleteString => {
                    let (Some(i), Some(o)) = (self.pop_num(host), self.pop()) else {
                        return fault(self, host);
                    };
                    let i = client_int(i);
                    self.edit_list(&o, host, |l| {
                        if let Some(i) = usize::try_from(i).ok().filter(|&i| i < l.len()) {
                            l.remove(i);
                        }
                    });
                }
                // The index is on top, the value under it.
                Op::ObjReplaceString | Op::ObjInsertString => {
                    let (Some(i), Some(v), Some(o)) = (self.pop_num(host), self.pop(), self.pop())
                    else {
                        return fault(self, host);
                    };
                    let v = self.value_full(v, host);
                    let i = client_int(i);
                    let replace = ins.op == Op::ObjReplaceString;
                    self.edit_list(&o, host, |l| {
                        let at = usize::try_from(i).ok().filter(|&i| i < l.len());
                        match (replace, at) {
                            (true, Some(i)) => l[i] = v,
                            // Out of range, `replace` adds at the end.
                            (true, None) => l.push(v),
                            (false, _) => {
                                let i = i.clamp(0, l.len() as i32) as usize;
                                l.insert(i, v);
                            }
                        }
                    });
                }
                Op::ObjClear => {
                    let Some(o) = self.pop() else {
                        return fault(self, host);
                    };
                    self.edit_list(&o, host, |l| l.clear());
                }
                // The length on top, then the start, then the list.
                Op::ObjSubarray => {
                    let (Some(n), Some(start), Some(o)) =
                        (self.pop_num(host), self.pop_num(host), self.pop())
                    else {
                        return fault(self, host);
                    };
                    let l = self.list_of(&o, host);
                    self.push(Slot::List(lists::subarray(
                        &l,
                        client_int(start),
                        client_int(n),
                    )));
                }
                Op::ObjIndex => {
                    let (Some(v), Some(o)) = (self.pop(), self.pop()) else {
                        return fault(self, host);
                    };
                    let v = self.value_full(v, host);
                    let l = self.list_of(&o, host);
                    let k = l
                        .iter()
                        .position(|e| e.same(&v, compare))
                        .map_or(-1.0, |k| k as f64);
                    self.push(Slot::Num(k));
                }
                // `x in list`; a list `x` is in when each of its elements is.
                Op::InObj => {
                    let (Some(o), Some(v)) = (self.pop(), self.pop()) else {
                        return fault(self, host);
                    };
                    let v = self.value_full(v, host);
                    let l = self.list_of(&o, host);
                    let has = |x: &Value| l.iter().any(|e| e.same(x, compare));
                    let yes = match &v {
                        Value::List(xs) if !xs.is_empty() => xs.iter().all(has),
                        x => has(x),
                    };
                    self.push(Slot::Num(yes as i32 as f64));
                }
                Op::ObjTokenize | Op::ObjPositions => {
                    let (Some(b), Some(a)) = (self.pop_val(host), self.pop_val(host)) else {
                        return fault(self, host);
                    };
                    self.push(if ins.op == Op::ObjPositions {
                        Slot::List(lists::positions(&val_text(&a), &val_text(&b)))
                    } else {
                        let t = lists::tokenize(&val_text(&a), &val_text(&b));
                        // No tokens gives no list at all.
                        if t.is_empty() {
                            Slot::Obj(None)
                        } else {
                            Slot::List(t.into_iter().map(|s| Value::Val(Val::Str(s))).collect())
                        }
                    });
                }
                // The loop variable, the list and the index stay on the
                // stack; past the end the loop jumps out and leaves them
                // (the compiler pops only the index).
                Op::ForEach => {
                    let n = self.stack.len();
                    if n < 3 {
                        return fault(self, host);
                    }
                    let i = match &self.stack[n - 1] {
                        Slot::Num(i) => client_int(*i),
                        _ => -1,
                    };
                    let (target, o) = (self.stack[n - 3].clone(), self.stack[n - 2].clone());
                    let l = self.list_of(&o, host);
                    match (usize::try_from(i).ok().and_then(|i| l.get(i)), target) {
                        (Some(e), Slot::Var(root, name)) if !name.is_empty() => {
                            let e = e.clone();
                            self.write_var(root, &name, e, host);
                        }
                        _ => self.pc = int.unwrap_or(0).max(0) as usize,
                    }
                }

                // Below the top is S1, the top S0; see `gs2-commands.md`.
                Op::Exp => {
                    let Some(a) = self.pop_num(host) else {
                        return fault(self, host);
                    };
                    self.push(Slot::Num(a.exp()));
                }
                // log(S0) / log(S1): S1 is the base. A base of 0 or less gives 0.
                Op::Log => {
                    let (Some(x), Some(base)) = (self.pop_num(host), self.pop_num(host)) else {
                        return fault(self, host);
                    };
                    self.push(Slot::Num(if base > 0.0 { x.ln() / base.ln() } else { 0.0 }));
                }
                Op::GetAngle | Op::GetDir => {
                    let (Some(dy), Some(dx)) = (self.pop_num(host), self.pop_num(host)) else {
                        return fault(self, host);
                    };
                    self.push(Slot::Num(if ins.op == Op::GetAngle {
                        maths::angle(dx, dy)
                    } else {
                        maths::dir(dx, dy)
                    }));
                }
                Op::VecX | Op::VecY => {
                    let Some(d) = self.pop_num(host) else {
                        return fault(self, host);
                    };
                    let (x, y) = maths::vec(d);
                    self.push(Slot::Num(if ins.op == Op::VecX { x } else { y }));
                }
                Op::ShiftLeft | Op::ShiftRight => {
                    let (Some(b), Some(a)) = (self.pop_num(host), self.pop_num(host)) else {
                        return fault(self, host);
                    };
                    let f =
                        self.profile
                            .shift(client_int(a), client_int(b), ins.op == Op::ShiftLeft);
                    self.push(Slot::Num(f as f64));
                }
                // `x in |a, b|`; a list `x` is in when each element is.
                Op::InRange => {
                    let (Some(b), Some(a), Some(x)) =
                        (self.pop_num(host), self.pop_num(host), self.pop())
                    else {
                        return fault(self, host);
                    };
                    let inside = |v: f64| v >= a && v <= b;
                    let yes = match self.value_full(x, host) {
                        Value::List(l) if !l.is_empty() => l.iter().all(|e| inside(e.num())),
                        v => inside(v.num()),
                    };
                    self.push(Slot::Num(yes as i32 as f64));
                }
                // 0 number, 1 string, 2 object, 3 a list with elements, -1
                // anything else (`0x53`).
                Op::ObjType => {
                    let Some(o) = self.pop() else {
                        return fault(self, host);
                    };
                    let t = match o {
                        Slot::Obj(None) | Slot::Marker => -1.0,
                        Slot::Obj(Some((_, ref path))) if path.is_empty() => 2.0,
                        o => match self.value_full(o, host) {
                            Value::List(l) if !l.is_empty() => 3.0,
                            Value::List(_) => -1.0,
                            Value::Val(Val::Num(_)) => 0.0,
                            Value::Val(Val::Str(_)) => 1.0,
                        },
                    };
                    self.push(Slot::Num(t));
                }
                // The list below, the value on top: the client's order, which
                // is not the order gs2-parser pushes.
                Op::ObjIndices => {
                    let (Some(v), Some(o)) = (self.pop(), self.pop()) else {
                        return fault(self, host);
                    };
                    let v = self.value_full(v, host);
                    let l = self.list_of(&o, host);
                    let found = l
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| e.same(&v, compare))
                        .map(|(i, _)| Value::Val(Val::Num(i as f64)))
                        .collect();
                    self.push(Slot::List(found));
                }
                // `setarray(a, n)`: pads with 0 or cuts to `n` (0..=10 000).
                Op::SetArray => {
                    let (Some(n), Some(o)) = (self.pop_num(host), self.pop()) else {
                        return fault(self, host);
                    };
                    let n = client_int(n).clamp(0, 10_000) as usize;
                    self.edit_list(&o, host, |l| l.resize(n, Value::Val(Val::Num(0.0))));
                }
                // A null object (or one Seihai cannot name) skips the block.
                Op::With => {
                    let Some(o) = self.pop() else {
                        return fault(self, host);
                    };
                    match o {
                        Slot::Obj(Some((root, path))) if path.is_empty() => {
                            self.with.push(root);
                            let end = int.unwrap_or(0).max(0) as usize;
                            self.with_span.push((self.pc, end, self.frames.len()));
                        }
                        other => {
                            if let Slot::Obj(Some((_, path))) = other {
                                host.log(format!("gs2: with ({path}) at op {at}: Seihai has no object references, block skipped"));
                            }
                            self.pc = int.unwrap_or(0).max(0) as usize;
                        }
                    }
                }
                Op::WithEnd => {
                    self.with.pop();
                    self.with_span.pop();
                }

                // `waitfor(object, event, timeout)` (4.20r8 on).
                Op::WaitFor => {
                    let (Some(t), Some(e), Some(o)) =
                        (self.pop_num(host), self.pop_val(host), self.pop())
                    else {
                        return fault(self, host);
                    };
                    let event = event_name(&val_text(&e)).to_string();
                    if !event.is_empty() && t > 0.0 {
                        let object = match o {
                            Slot::Obj(None) => None,
                            Slot::Obj(Some((root, path))) if path.is_empty() => {
                                Some(root_name(root).to_string())
                            }
                            Slot::Obj(Some((_, path))) => Some(path),
                            other => Some(val_text(&self.value(other, host))),
                        };
                        return Outcome::WaitFor {
                            object,
                            event,
                            timeout: t,
                        };
                    }
                    self.push(Slot::Num(0.0));
                }
                // `OP_OBJ_COMPARE` (4.20r6 on): -1, 0 or 1.
                Op::ObjCompare => {
                    let (Some(b), Some(a)) = (self.pop_val(host), self.pop_val(host)) else {
                        return fault(self, host);
                    };
                    self.push(Slot::Num(compare(&a, &b).signum() as f64));
                }
                Op::Unknown(n) => {
                    if let Some(o) = self.run_extra(n, &ins.operand, at, host) {
                        return o;
                    }
                }
                other => host.log(format!(
                    "gs2: {other} at op {at} is not run yet (Stage D), skipped"
                )),
            }
        }
    }

    /// `OP_CALL`: the name is on top, the arguments below it back to the
    /// nearest marker, the first argument next to the name.
    fn call(&mut self, host: &mut dyn Host) -> Option<Outcome> {
        let Some(m) = self.last_marker() else {
            return Some(self.stop("call without an argument list".into(), host));
        };
        let name = match self.pop() {
            Some(Slot::Var(root, n)) => (root, n),
            Some(Slot::Str(n)) => (None, n),
            _ => (None, String::new()),
        };
        // The script's own functions: unqualified, or on `this`.
        if matches!(name.0, None | Some(Root::This) | Some(Root::ThisO)) {
            if let Some(start) = self.find_function(&name.1) {
                if self.frames.len() >= CALL_LIMIT {
                    return Some(self.stop("callstack overrun".into(), host));
                }
                self.frames.push(Frame {
                    with_depth: self.with.len(),
                    ret: Some(self.pc),
                    base: m,
                    locals: HashMap::new(),
                    slots: Vec::new(),
                });
                self.pc = start;
                return None;
            }
        }
        let mut args = Vec::new();
        while self.stack.len() > m + 1 {
            let s = self.pop().unwrap();
            args.push(self.value(s, host));
        }
        // The first argument sits next to the name, so it came off first.
        self.stack.truncate(m);
        let full = match name.0 {
            None => name.1.clone(),
            Some(r) => format!("{}.{}", root_name(r), name.1),
        };
        let result = match self
            .timer_command(&full, &args)
            .or_else(|| host.command(&full, &args))
        {
            Some(v) => v,
            None => {
                host.log(format!("gs2: function {full} not found"));
                Val::Num(0.0)
            }
        };
        self.stack.push(match result {
            Val::Num(n) => Slot::Num(n),
            Val::Str(s) => Slot::Str(s),
        });
        None
    }

    /// `OP_FUNC_PARAMS_END`: the parameter names are above the inner marker
    /// (the first parameter on top), the arguments between the call's
    /// marker and it (the first argument nearest). Missing arguments are 0.
    /// Everything back to and including the call's marker is dropped.
    fn bind_params(&mut self, host: &mut dyn Host) {
        let Some(inner) = self.last_marker() else {
            return;
        };
        let Some(outer) = self.stack[..inner].iter().rposition(|s| *s == Slot::Marker) else {
            self.stack.truncate(inner);
            return;
        };
        let names: Vec<String> = self.stack[inner + 1..]
            .iter()
            .rev()
            .map(|s| match s {
                Slot::Var(_, n) | Slot::Str(n) => n.clone(),
                _ => String::new(),
            })
            .collect();
        let arg_slots: Vec<Slot> = self.stack[outer + 1..inner].iter().rev().cloned().collect();
        let mut args = Vec::new();
        for s in arg_slots {
            args.push(self.value_full(s, host));
        }
        self.stack.truncate(outer);
        if let Some(f) = self.frames.last_mut() {
            for (i, n) in names.into_iter().enumerate() {
                if !n.is_empty() {
                    f.locals.insert(n, args.get(i).cloned().unwrap_or_default());
                }
            }
        }
    }
}

/// `new[n][m]`: every element that is not a list becomes `m` zeros.
fn fill_leaves(l: &mut [Value], m: i32) {
    for e in l {
        match e {
            Value::List(inner) => fill_leaves(inner, m),
            _ => *e = Value::List(lists::zeros(m)),
        }
    }
}

fn root_name(r: Root) -> &'static str {
    match r {
        Root::This => "this",
        Root::ThisO => "thiso",
        Root::Player => "player",
        Root::PlayerO => "playero",
        Root::Level => "level",
        Root::Temp => "temp",
        Root::Global => "global",
    }
}

/// Two numbers compare as numbers (equal within [`vm::TOLERANCE`], as in
/// every VM read), two strings as text, and a string against a number as
/// the number it reads as.
fn compare(a: &Val, b: &Val) -> i32 {
    match (a, b) {
        (Val::Str(x), Val::Str(y)) => x.cmp(y) as i32,
        _ => vm::num_cmp(a.num(), b.num()),
    }
}

/// A value as text, numbers written as the client writes them
/// ([`vm::num_text`]).
fn val_text(v: &Val) -> String {
    match v {
        Val::Num(n) => vm::num_text(*n),
        Val::Str(s) => s.clone(),
    }
}

/// The number after an opcode, written as an integer or a decimal.
fn operand_num(o: &Operand) -> Option<f64> {
    match o {
        Operand::Int(v) => Some(*v as f64),
        Operand::Decimal(d) => d.trim().parse().ok(),
        _ => None,
    }
}

/// The most local slots a frame has (opcodes 45 and 46).
const SLOT_LIMIT: usize = 1024;

/// A number in [0, 1). The client's generator is not reproduced; this is
/// a small xorshift seeded from the clock.
fn rand_unit() -> f64 {
    use std::cell::Cell;
    thread_local!(static STATE: Cell<u64> = const { Cell::new(0) });
    STATE.with(|s| {
        let mut x = s.get();
        if x == 0 {
            x = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(1)
                | 1;
        }
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        (x >> 11) as f64 / (1u64 << 53) as f64
    })
}

/// A [`Host`] that turns the commands GS1 already has into GS1 effects for
/// one object, and keeps the flags.
pub struct EffectHost {
    pub era: Era,
    pub obj: ObjId,
    pub x: f64,
    pub y: f64,
    pub flags: HashMap<String, String>,
    pub effects: Vec<Effect>,
    pub log: Vec<String>,
    /// `resolveHost(host)` is a builtin (6.00r4 only:
    /// `VersionProfile::scripts.resolve_host`). Off by default.
    pub resolve_host: bool,
}

impl EffectHost {
    pub fn new(era: Era, obj: ObjId) -> EffectHost {
        EffectHost {
            era,
            obj,
            x: 0.0,
            y: 0.0,
            flags: HashMap::new(),
            effects: Vec::new(),
            log: Vec::new(),
            resolve_host: false,
        }
    }
}

fn text(args: &[Val], i: usize) -> String {
    args.get(i).map(Val::text).unwrap_or_default()
}

fn num(args: &[Val], i: usize) -> f64 {
    args.get(i).map(Val::num).unwrap_or(0.0)
}

impl Host for EffectHost {
    fn command(&mut self, name: &str, args: &[Val]) -> Option<Val> {
        let obj = self.obj;
        // Built-in names match in any case, and `this.showimg(…)` is the
        // script's own `showimg`, as for the timer commands.
        let lower = name.to_ascii_lowercase();
        let name = lower
            .strip_prefix("this.")
            .or_else(|| lower.strip_prefix("thiso."))
            .unwrap_or(&lower);
        match name {
            // `set "name"` or `set "name=value"`, as GS1 reads it.
            "set" => {
                let t = text(args, 0);
                let (k, v) = t.split_once('=').unwrap_or((t.as_str(), ""));
                let k = k.trim().to_ascii_lowercase();
                if !k.is_empty() && self.flags.get(&k).map(String::as_str) != Some(v.trim()) {
                    self.flags.insert(k.clone(), v.trim().to_string());
                    if !k.starts_with("local.") {
                        self.effects.push(Effect::SetFlag {
                            name: k,
                            value: Some(v.trim().to_string()),
                        });
                    }
                }
            }
            "unset" => {
                let k = text(args, 0).trim().to_ascii_lowercase();
                if self.flags.remove(&k).is_some() && !k.starts_with("local.") {
                    self.effects.push(Effect::UnsetFlag { name: k });
                }
            }
            "message" => self.effects.push(Effect::Message {
                obj,
                text: text(args, 0),
            }),
            "say" => self.effects.push(Effect::SaySign {
                index: num(args, 0) as i64,
            }),
            "say2" => self.effects.push(Effect::Say2 {
                text: text(args, 0),
            }),
            "setlevel" => self.effects.push(Effect::SetLevel {
                level: text(args, 0),
                x: None,
                y: None,
            }),
            "setlevel2" => self.effects.push(Effect::SetLevel {
                level: text(args, 0),
                x: Some(num(args, 1)),
                y: Some(num(args, 2)),
            }),
            "setplayerprop" => self.effects.push(Effect::SetPlayerProp {
                code: text(args, 0),
                value: text(args, 1),
            }),
            // GS1's `strequals(a, b)`, which GS1 scripts in GS2 files call:
            // the two texts equal, letter case aside (the GS1 engine's rule).
            "strequals" => {
                let same = text(args, 0).to_lowercase() == text(args, 1).to_lowercase();
                return Some(Val::Num(if same { 1.0 } else { 0.0 }));
            }
            // 6.00r4's `resolveHost(host)`: a DNS lookup, giving the address
            // as text, or "" when the name does not resolve.
            "resolvehost" if self.resolve_host => {
                let addr = crate::net::resolve_host(&text(args, 0)).unwrap_or_default();
                return Some(Val::Str(addr));
            }
            // How `seteffect` mixes. The GS1 vocabulary dates it to 2.1;
            // a GS2 host takes it whatever era it was made for, and sends it
            // as GS1 does.
            "seteffectmode" => self.effects.push(Effect::Command {
                obj,
                name: name.to_string(),
                args: args.to_vec(),
            }),
            _ => {
                let cmd = vocab::command_in(name, self.era)?;
                match cmd.exec {
                    // The rest of the shared set goes out as GS1 does it
                    // (`showimg`, `hideimg`, `play`, …).
                    Exec::Effect | Exec::Ext => self.effects.push(Effect::Command {
                        obj,
                        name: name.to_string(),
                        args: args.to_vec(),
                    }),
                    Exec::NoExec => {}
                    // Flags, arrays and calls inside the GS1 engine; GS2 has
                    // its own forms of these (Stage D).
                    Exec::Engine => return None,
                }
            }
        }
        Some(Val::Num(0.0))
    }

    fn get_prop(&mut self, root: Root, name: &str) -> Option<Val> {
        match (root, name) {
            (Root::This | Root::ThisO, "x") => Some(Val::Num(self.x)),
            (Root::This | Root::ThisO, "y") => Some(Val::Num(self.y)),
            _ => None,
        }
    }

    fn set_prop(&mut self, root: Root, name: &str, value: &Val) -> bool {
        match (root, name) {
            (Root::This | Root::ThisO, "x" | "y") => {
                if name == "x" {
                    self.x = value.num();
                } else {
                    self.y = value.num();
                }
                self.effects.push(Effect::SetPos {
                    obj: self.obj,
                    x: self.x,
                    y: self.y,
                });
                true
            }
            _ => false,
        }
    }

    fn log(&mut self, line: String) {
        self.log.push(line);
    }
}
