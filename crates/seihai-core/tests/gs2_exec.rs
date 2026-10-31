//! The GS2 executor (`seihai_core::scripting::gs2::exec`) on the compiler
//! fixtures in `tests/fixtures/gs2/`: the shared commands come out as the
//! same GS1 effects the classic engine makes. See
//! `documentation/gs2-executor.md`.

use seihai_core::scripting::engine::{Effect, Era, Val};
use seihai_core::scripting::gs2::exec::{EffectHost, Machine, Outcome, Root};
use seihai_core::scripting::gs2::Script;
use std::path::Path;

fn machine(name: &str) -> Machine {
    let bytes = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gs2").join(name)).unwrap();
    Machine::new(Script::parse(&bytes).unwrap())
}

fn host() -> EffectHost {
    let mut h = EffectHost::new(Era::V21, 7);
    h.x = 30.0;
    h.y = 31.0;
    h
}

#[test]
fn shared_fixture_sets_a_flag_shows_and_hides_an_image_then_sleeps() {
    let mut m = machine("shared.gs2bc");
    let mut h = host();
    assert_eq!(m.run_top_level(&mut h), Outcome::Done);
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Sleep(0.5));
    assert_eq!(m.var(Root::This, "count"), Some(&Val::Num(11.0)));
    assert_eq!(
        h.effects,
        [
            Effect::SetFlag { name: "gs2seen".into(), value: Some(String::new()) },
            Effect::Command { obj: 7, name: "showimg".into(), args: vec![Val::Num(1.0), Val::from("light2.png"), Val::Num(30.0), Val::Num(31.0)] },
            Effect::Command { obj: 7, name: "hideimg".into(), args: vec![Val::Num(1.0)] },
        ]
    );
    assert_eq!(m.resume(&mut h), Outcome::Done);
    assert!(h.log.is_empty(), "{:?}", h.log);
}

#[test]
fn calls_fixture_runs_a_function_with_parameters_and_a_return_value() {
    let mut m = machine("calls.gs2bc");
    let mut h = host();
    h.flags.insert("gs2seen".into(), String::new());
    let out = m.call_event("onCreated", &[], &mut h);
    assert_eq!(m.var(Root::This, "sum"), Some(&Val::Num(5.0)));
    assert_eq!(m.var(Root::This, "text"), Some(&Val::from("n=5")));
    assert_eq!(
        h.effects,
        [
            Effect::Say2 { text: "n=5".into() },
            Effect::SetLevel { level: "lab3x.nw".into(), x: Some(30.5), y: Some(31.0) },
            Effect::UnsetFlag { name: "gs2seen".into() },
        ]
    );
    // `while (true)` stops at the loop limit, as in the client.
    assert_eq!(out, Outcome::Stopped("Script exceeded loop limit".into()));
    assert_eq!(m.var(Root::This, "k"), Some(&Val::Num(10_000.0)));
}

#[test]
fn control_fixture_runs_loops_and_switch() {
    let mut m = machine("control.gs2bc");
    let mut h = host();
    m.call_event("onTimeout", &[], &mut h);
    // n = 1 + 3 + 4 = 8, counted down to 1 by the while loop: case 1.
    assert_eq!(h.effects, [Effect::Say2 { text: "one".into() }]);
    // `in |a, b|` and `setTimer` are the machine's own now (Stage D), so
    // nothing is skipped.
    assert!(h.log.is_empty(), "{:?}", h.log);
    assert_eq!(m.timeout(), 0.1);
}

#[test]
fn timers_fixture_runs_the_timeout_and_scheduled_events_frame_by_frame() {
    let mut m = machine("timers.gs2bc");
    let mut h = host();
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    // setTimer(0.1) is two frames of 1/20 s.
    assert_eq!(m.timeout(), 0.1);
    let mut ran = Vec::new();
    for frame in 1..=8 {
        for (event, out) in m.tick(&mut h) {
            assert_eq!(out, Outcome::Done, "{event}");
            ran.push((frame, event));
        }
    }
    // onTimeout re-arms itself with `this.timeout = 0.05` (one frame) until
    // it has run three times; Ping comes at 0.25 s (frame 5), Kept at 0.3 s
    // (frame 6). Dropped was cancelled, by its name without the `on`.
    let names = |n: &str| n.to_string();
    assert_eq!(
        ran,
        [(2, names("onTimeout")), (3, names("onTimeout")), (4, names("onTimeout")), (5, names("onPing")), (6, names("onKept"))]
    );
    assert_eq!(m.var(Root::This, "ticks"), Some(&Val::Num(3.0)));
    // Inside onTimeout the timer has already run out.
    assert_eq!(m.var(Root::This, "left"), Some(&Val::Num(0.0)));
    assert_eq!(m.var(Root::This, "ping"), Some(&Val::Num(7.0)));
    assert_eq!(m.var(Root::This, "kept"), Some(&Val::Num(2.0)));
    assert_eq!(m.var(Root::This, "dropped"), None);
    assert_eq!(m.timeout(), 0.0);
    assert!(h.log.is_empty(), "{:?}", h.log);
}

#[test]
fn an_event_that_comes_due_during_a_sleep_waits_for_the_run_to_end() {
    let mut m = machine("timers.gs2bc");
    let mut h = host();
    // onSleepy sets a one-frame timer, then sleeps.
    assert_eq!(m.call_event("onSleepy", &[], &mut h), Outcome::Sleep(0.1));
    // The timer runs out while the run is asleep: nothing starts.
    assert!(m.tick(&mut h).is_empty());
    assert_eq!(m.resume(&mut h), Outcome::Done);
    assert_eq!(m.var(Root::This, "woke"), Some(&Val::Num(1.0)));
    // The next frame runs the timeout that came due.
    let ran: Vec<String> = m.tick(&mut h).into_iter().map(|(e, _)| e).collect();
    assert_eq!(ran, ["onTimeout"]);
    assert_eq!(m.var(Root::This, "ticks"), Some(&Val::Num(1.0)));
}

#[test]
fn strings_fixture_runs_the_string_built_ins() {
    let mut m = machine("strings.gs2bc");
    let mut h = host();
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    let v = |n: &str| m.var(Root::This, n).cloned().unwrap_or_else(|| panic!("{n} not set"));
    let s = |t: &str| Val::from(t);
    assert_eq!(v("trimmed"), s("Hello, Graal"));
    assert_eq!(v("len"), Val::Num(12.0));
    assert_eq!((v("at"), v("none")), (Val::Num(7.0), Val::Num(-1.0)));
    assert_eq!((v("c"), v("out")), (s("e"), s("")));
    assert_eq!((v("sub"), v("clip")), (s("Gra"), s("Hello, Graal")));
    assert_eq!((v("st"), v("en"), v("no")), (Val::Num(1.0), Val::Num(1.0), Val::Num(0.0)));
    assert_eq!((v("ch"), v("tr")), (s("A"), s("as is")));
    assert_eq!(v("f1"), s("42|    7|7    |+3|005"));
    assert_eq!(v("f2"), s("ab|    cd|ef|%|1.5"));
    // 9.7 rounds down; the missing argument is "".
    assert_eq!(v("f3"), s("9 and "));
    assert_eq!(v("f4"), s("plain"));
    assert!(h.log.is_empty(), "{:?}", h.log);
}

#[test]
fn lists_fixture_builds_changes_and_walks_lists() {
    use seihai_core::scripting::gs2::lists::Value;
    let mut m = machine("lists.gs2bc");
    let mut h = host();
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    let n = |v: f64| Val::Num(v);
    let v = |name: &str| m.var(Root::This, name).cloned().unwrap_or_else(|| panic!("{name} not set"));
    assert_eq!((v("n"), v("first")), (n(3.0), n(10.0)));
    // {10, "two", 30}, a[1] = 20, add 40, insert 5 at 0, delete 1,
    // remove 30, replace 0 with 7.
    let joined: Vec<Val> = m.list(Root::This, "joined").expect("a list").iter().map(Value::val).collect();
    assert_eq!(joined, [n(7.0), n(20.0), n(40.0)]);
    assert_eq!((v("idx"), v("has")), (n(2.0), n(1.0)));
    assert_eq!((v("tn"), v("t2")), (n(3.0), Val::from("c")));
    let ps: Vec<Val> = m.list(Root::This, "ps").expect("a list").iter().map(Value::val).collect();
    assert_eq!(ps, [n(1.0), n(3.0)]);
    assert_eq!(v("sum"), n(67.0));
    assert_eq!((v("g"), v("gn")), (n(9.0), n(2.0)));
    assert_eq!(v("left"), n(0.0));
    assert!(h.log.is_empty(), "{:?}", h.log);
}

#[test]
fn missing_events_are_reported() {
    let mut m = machine("shared.gs2bc");
    assert_eq!(m.call_event("onPlayerEnters", &[], &mut host()), Outcome::NoSuchEvent);
}

#[test]
fn rest_fixture_runs_the_maths_ranges_types_and_with() {
    let mut m = machine("rest.gs2bc");
    let mut h = host();
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    let n = |v: f64| Val::Num(v);
    let v = |name: &str| m.var(Root::This, name).cloned().unwrap_or_else(|| panic!("{name} not set"));
    assert_eq!(v("e"), n(1.0));
    // gs2-parser pushes log(2, 8) as 8 then 2, and the client takes the
    // lower one as the base: log base 8 of 2.
    assert!((v("l").num() - 1.0 / 3.0).abs() < 1e-12);
    assert!((v("ga").num() - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
    assert_eq!((v("gd"), v("vx"), v("vy")), (n(1.0), n(1.0), n(1.0)));
    assert_eq!((v("sh"), v("sr")), (n(16.0), n(64.0)));
    assert_eq!((v("inr"), v("outr")), (n(1.0), n(0.0)));
    // gs2-parser pushes the value under the list; the client wants the
    // list under the value, so this finds nothing.
    assert_eq!(v("ixn"), n(0.0));
    assert_eq!((v("tl"), v("tn")), (n(3.0), n(0.0)));
    assert_eq!(m.list(Root::This, "arr").map(<[_]>::len), Some(3));
    assert_eq!(v("an"), n(3.0));
    // `with (player)`: `x` and `this.inw` land on the player; the null
    // `with` block is skipped.
    assert_eq!((m.var(Root::Player, "x"), m.var(Root::Player, "inw")), (Some(&n(12.0)), Some(&n(1.0))));
    assert_eq!((v("px"), h.x), (n(12.0), 30.0));
    assert_eq!(m.var(Root::This, "never"), None);
    assert!(h.log.is_empty(), "{:?}", h.log);
}

#[test]
fn shared_commands_match_in_any_case_and_under_this() {
    use seihai_core::scripting::gs2::exec::Host;
    let mut h = host();
    assert!(h.command("this.hideImg", &[Val::Num(2.0)]).is_some());
    assert!(h.command("Say2", &[Val::from("hi")]).is_some());
    // GS2 has its own lists; the GS1 string-list commands are not there.
    assert!(h.command("setstring", &[Val::from("a"), Val::from("b")]).is_none());
    assert_eq!(
        h.effects,
        [Effect::Command { obj: 7, name: "hideimg".into(), args: vec![Val::Num(2.0)] }, Effect::Say2 { text: "hi".into() }]
    );
}
