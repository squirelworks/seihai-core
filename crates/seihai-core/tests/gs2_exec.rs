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
fn control_fixture_runs_loops_and_switch_and_logs_what_it_skips() {
    let mut m = machine("control.gs2bc");
    let mut h = host();
    m.call_event("onTimeout", &[], &mut h);
    // n = 1 + 3 + 4 = 8, counted down to 1 by the while loop: case 1.
    assert_eq!(h.effects, [Effect::Say2 { text: "one".into() }]);
    assert!(h.log.iter().any(|l| l.contains("OP_IN_RANGE") && l.contains("Stage D")), "{:?}", h.log);
    // `setTimer` is the machine's own now (Stage D, timers).
    assert!(!h.log.iter().any(|l| l.contains("setTimer")), "{:?}", h.log);
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
fn missing_events_are_reported() {
    let mut m = machine("shared.gs2bc");
    assert_eq!(m.call_event("onPlayerEnters", &[], &mut host()), Outcome::NoSuchEvent);
}
