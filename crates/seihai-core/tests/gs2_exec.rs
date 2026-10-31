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
    assert!(h.log.iter().any(|l| l == "gs2: function setTimer not found"), "{:?}", h.log);
}

#[test]
fn missing_events_are_reported() {
    let mut m = machine("shared.gs2bc");
    assert_eq!(m.call_event("onPlayerEnters", &[], &mut host()), Outcome::NoSuchEvent);
}
