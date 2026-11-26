//! Function values on the GS2 executor: a `function () { … }` kept in a
//! variable and called through it, a named function passed as `this.f`, a
//! lambda passed inline, and functions set as members of an object, where
//! `this` is that object. The call shape is the official compiler's (a
//! lambda is `this.function_<id>_<n>` converted to an object and assigned:
//! Graal_analysis `npc-library/gs2-official-emit-spec-2026-11-26.md`); that
//! `obj.f()` runs with `this` as `obj` is how real weapon scripts build
//! their own object types.

use seihai_core::scripting::engine::Val;
use seihai_core::scripting::gs2::compile::{self, Language, Mode, Options};
use seihai_core::scripting::gs2::exec::{Host, Machine, Outcome};

#[derive(Default)]
struct Echo {
    lines: Vec<String>,
    log: Vec<String>,
}

impl Host for Echo {
    fn command(&mut self, name: &str, args: &[Val]) -> Option<Val> {
        if name.eq_ignore_ascii_case("echo") {
            self.lines.push(match args.first() {
                Some(Val::Str(s)) => s.clone(),
                Some(Val::Num(n)) => n.to_string(),
                None => String::new(),
            });
            return Some(Val::Num(0.0));
        }
        None
    }
    fn log(&mut self, line: String) {
        self.log.push(line);
    }
}

fn run(src: &str) -> Echo {
    let c = compile::compile_with(
        src,
        Language::Gs2,
        Options {
            mode: Mode::Legacy,
            lambda_id: Some(1),
        },
    )
    .unwrap();
    let mut m = Machine::new(c.script);
    let mut h = Echo::default();
    assert_eq!(m.run_top_level(&mut h), Outcome::Done);
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    h
}

#[test]
fn a_lambda_in_a_temp_variable_is_called_through_it() {
    let h = run(r#"
function onCreated() {
  temp.hello = function() { return "hello"; };
  temp.sum = function(x, y) { return x + y; };
  echo("plain " @ temp.hello() @ " " @ temp.sum(5, 10));
}
"#);
    assert_eq!(h.lines, ["plain hello 15"], "{:?}", h.log);
}

#[test]
fn functions_are_passed_by_name_by_variable_and_inline() {
    let h = run(r#"
function onCreated() {
  temp.sum = function(x, y) { return x + y; };
  echo("named " @ this.apply(this.triple, 4));
  echo("passed " @ this.apply(temp.sum, 2) @ " " @ this.apply(function(n) { return n * n; }, 6));
}
function triple(v) { return v * 3; }
function apply(fn, v) { return fn(v); }
"#);
    assert_eq!(h.lines, ["named 12", "passed 2 36"], "{:?}", h.log);
}

#[test]
fn a_function_member_runs_with_this_as_its_object() {
    let h = run(r#"
function onCreated() {
  temp.box = new TStaticVar();
  temp.box.size = 7;
  temp.box.grow = function(by) {
    this.size += by;
    return this.size;
  };
  echo("method " @ temp.box.grow(3) @ " " @ temp.box.size);
  this.counter = new TStaticVar();
  this.counter.n = 0;
  this.counter.bump = function() { this.n++; };
  for (temp.i = 0; temp.i < 3; temp.i++)
    this.counter.bump();
  echo("member " @ this.counter.n @ " " @ this.n);
}
"#);
    assert_eq!(h.lines, ["method 10 10", "member 3 0"], "{:?}", h.log);
}

#[test]
fn a_parameter_passed_on_by_its_bare_name_keeps_its_value() {
    let h = run(r#"
function onCreated() {
  echo("outer " @ this.outer(4));
}
function outer(v) { return this.inner(v) + 1; }
function inner(w) { return w * 10; }
"#);
    assert_eq!(h.lines, ["outer 41"], "{:?}", h.log);
}
