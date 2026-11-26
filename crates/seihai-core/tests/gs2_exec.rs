//! The GS2 executor (`seihai_core::scripting::gs2::exec`) on the compiler
//! fixtures in `tests/fixtures/gs2/`: the shared commands come out as the
//! same GS1 effects the classic engine makes. See
//! `documentation/gs2-executor.md`.

use seihai_core::scripting::engine::{Effect, Era, Val};
use seihai_core::scripting::gs2::exec::{EffectHost, Machine, Outcome, Root};
use seihai_core::scripting::gs2::Script;
use std::path::Path;

fn machine(name: &str) -> Machine {
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/gs2")
            .join(name),
    )
    .unwrap();
    Machine::new(Script::parse(&bytes).unwrap())
}

fn host() -> EffectHost {
    let mut h = EffectHost::new(Era::V217, 7);
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
            Effect::SetFlag {
                name: "gs2seen".into(),
                value: Some(String::new())
            },
            Effect::Command {
                obj: 7,
                name: "showimg".into(),
                args: vec![
                    Val::Num(1.0),
                    Val::from("light2.png"),
                    Val::Num(30.0),
                    Val::Num(31.0)
                ]
            },
            Effect::Command {
                obj: 7,
                name: "hideimg".into(),
                args: vec![Val::Num(1.0)]
            },
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
            Effect::SetLevel {
                level: "lab3x.nw".into(),
                x: Some(30.5),
                y: Some(31.0)
            },
            Effect::UnsetFlag {
                name: "gs2seen".into()
            },
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
        [
            (2, names("onTimeout")),
            (3, names("onTimeout")),
            (4, names("onTimeout")),
            (5, names("onPing")),
            (6, names("onKept"))
        ]
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
    let v = |n: &str| {
        m.var(Root::This, n)
            .cloned()
            .unwrap_or_else(|| panic!("{n} not set"))
    };
    let s = |t: &str| Val::from(t);
    assert_eq!(v("trimmed"), s("Hello, Graal"));
    assert_eq!(v("len"), Val::Num(12.0));
    assert_eq!((v("at"), v("none")), (Val::Num(7.0), Val::Num(-1.0)));
    assert_eq!((v("c"), v("out")), (s("e"), s("")));
    assert_eq!((v("sub"), v("clip")), (s("Gra"), s("Hello, Graal")));
    assert_eq!(
        (v("st"), v("en"), v("no")),
        (Val::Num(1.0), Val::Num(1.0), Val::Num(0.0))
    );
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
    let v = |name: &str| {
        m.var(Root::This, name)
            .cloned()
            .unwrap_or_else(|| panic!("{name} not set"))
    };
    assert_eq!((v("n"), v("first")), (n(3.0), n(10.0)));
    // {10, "two", 30}, a[1] = 20, add 40, insert 5 at 0, delete 1,
    // remove 30, replace 0 with 7.
    let joined: Vec<Val> = m
        .list(Root::This, "joined")
        .expect("a list")
        .iter()
        .map(Value::val)
        .collect();
    assert_eq!(joined, [n(7.0), n(20.0), n(40.0)]);
    assert_eq!((v("idx"), v("has")), (n(2.0), n(1.0)));
    assert_eq!((v("tn"), v("t2")), (n(3.0), Val::from("c")));
    let ps: Vec<Val> = m
        .list(Root::This, "ps")
        .expect("a list")
        .iter()
        .map(Value::val)
        .collect();
    assert_eq!(ps, [n(1.0), n(3.0)]);
    assert_eq!(v("sum"), n(67.0));
    assert_eq!((v("g"), v("gn")), (n(9.0), n(2.0)));
    assert_eq!(v("left"), n(0.0));
    assert!(h.log.is_empty(), "{:?}", h.log);
}

#[test]
fn missing_events_are_reported() {
    let mut m = machine("shared.gs2bc");
    assert_eq!(
        m.call_event("onPlayerEnters", &[], &mut host()),
        Outcome::NoSuchEvent
    );
}

#[test]
fn rest_fixture_runs_the_maths_ranges_types_and_with() {
    let mut m = machine("rest.gs2bc");
    let mut h = host();
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    let n = |v: f64| Val::Num(v);
    let v = |name: &str| {
        m.var(Root::This, name)
            .cloned()
            .unwrap_or_else(|| panic!("{name} not set"))
    };
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
    // `with (player)`: `x` and `this.inw` land on the player (`x` is the
    // host's player place, #497); the null `with` block is skipped.
    assert_eq!(
        (h.player.x, m.var(Root::Player, "inw")),
        (12.0, Some(&n(1.0)))
    );
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
    assert!(h
        .command("setstring", &[Val::from("a"), Val::from("b")])
        .is_none());
    assert_eq!(
        h.effects,
        [
            Effect::Command {
                obj: 7,
                name: "hideimg".into(),
                args: vec![Val::Num(2.0)]
            },
            Effect::Say2 { text: "hi".into() }
        ]
    );
}

#[test]
fn gs2_top_level_code_runs_from_op_zero_around_functions() {
    use seihai_core::scripting::gs2::compile::compile_gs2;
    let src = "this.a = 1;\nfunction onCreated() { this.b = this.a + 1; }\nthis.a += 10;\nfunction onTimeout() { this.c = 3; }\n";
    let mut m = Machine::new(compile_gs2(src).unwrap());
    let mut h = host();
    assert_eq!(m.run_top_level(&mut h), Outcome::Done);
    assert_eq!(m.var(Root::This, "a"), Some(&Val::Num(11.0)));
    assert_eq!(m.var(Root::This, "b"), None);
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    assert_eq!(m.var(Root::This, "b"), Some(&Val::Num(12.0)));
    assert_eq!(m.var(Root::This, "c"), None);
}

#[test]
fn leaving_a_with_block_by_break_continue_or_return_closes_it() {
    use seihai_core::scripting::gs2::compile::compile_gs2;
    let src = "function g() { with (player) { return 4; } }\n\
               function onCreated() {\n\
                 for (i = 0; i < 3; i++) { with (player) { a = 1; break; } }\n\
                 b = 1;\n\
                 for (i = 0; i < 2; i++) { with (player) { c = 1; continue; } }\n\
                 d = 1;\n\
                 this.r = g();\n\
                 e = 1;\n\
               }\n";
    let mut m = Machine::new(compile_gs2(src).unwrap());
    let mut h = host();
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    let one = Some(&Val::Num(1.0));
    assert_eq!(
        (m.var(Root::Player, "a"), m.var(Root::Player, "c")),
        (one, one)
    );
    for name in ["b", "d", "e"] {
        assert_eq!(
            m.var(Root::Player, name),
            None,
            "{name} landed on the player"
        );
    }
    assert_eq!(m.var(Root::This, "r"), Some(&Val::Num(4.0)));
}

#[test]
fn v6_shapes_run_as_written() {
    use seihai_core::scripting::gs2::compile::{compile_gs2_legacy, Quirk};
    let src = "/* outer /* inner */ still a comment */\n\
               if (1 == 1) {\n\
                 function g(a, b) return a &= b;\n\
               }\n\
               function onCreated() {\n\
                 if (this.no) this.p = 1;\n\
                 else this.p = 2, this.q = 3;\n\
                 this.m = 6; this.m |= 9; this.m &= 12;\n\
                 this.b = g(6, 3);\n\
                 this.s = strequals(\"/Sit\", /sit);\n\
                 seteffectmode 2;\n\
               }\n";
    let compiled = compile_gs2_legacy(src).unwrap();
    let quirks: Vec<Quirk> = compiled.warnings.into_iter().map(|w| w.quirk).collect();
    assert_eq!(
        quirks,
        [
            Quirk::NestedComment,
            Quirk::NestedFunction,
            Quirk::BracelessFunctionBody,
            Quirk::CommaStatements,
            Quirk::SlashWord("/sit".into()),
            Quirk::Gs1Command("seteffectmode".into()),
        ]
    );
    let mut m = Machine::new(compiled.script);
    let mut h = EffectHost::new(Era::V205, 7);
    assert_eq!(m.run_top_level(&mut h), Outcome::Done);
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    let n = |v: f64| Some(Val::Num(v));
    let get = |k: &str| m.var(Root::This, k).cloned();
    assert_eq!((get("p"), get("q")), (n(2.0), n(3.0)));
    // 6 | 9 = 15, 15 & 12 = 12; 6 & 3 = 2.
    assert_eq!((get("m"), get("b")), (n(12.0), n(2.0)));
    assert_eq!(get("s"), n(1.0));
    assert!(h.log.is_empty(), "{:?}", h.log);
    assert_eq!(
        h.effects,
        [Effect::Command {
            obj: 7,
            name: "seteffectmode".into(),
            args: vec![Val::Num(2.0)]
        }]
    );
}

#[test]
fn a_function_returns_the_value_of_its_own_variable() {
    use seihai_core::scripting::gs2::compile::compile_gs2_legacy;
    let src = "function g(a) { temp.t = a + 1; return temp.t; }\n\
               function h(a) return a += 2;\n\
               function onCreated() { this.b = g(6); this.c = h(6); }";
    let mut m = Machine::new(compile_gs2_legacy(src).unwrap().script);
    let mut h = host();
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    assert_eq!(m.var(Root::This, "b"), Some(&Val::Num(7.0)));
    assert_eq!(m.var(Root::This, "c"), Some(&Val::Num(8.0)));
}

/// `resolveHost` is a builtin only where the profile registers it (6.00r4):
/// off, the name falls through as an unknown command; on, it gives the
/// address as text, and "" for an empty name.
#[test]
fn resolve_host_is_a_builtin_only_when_the_profile_has_it() {
    use seihai_core::scripting::gs2::exec::Host;
    let mut h = host();
    assert_ne!(
        h.command("resolveHost", &[Val::Str("127.0.0.1".into())]),
        Some(Val::Str("127.0.0.1".into()))
    );
    h.resolve_host = true;
    assert_eq!(
        h.command("resolveHost", &[Val::Str("127.0.0.1".into())]),
        Some(Val::Str("127.0.0.1".into()))
    );
    assert_eq!(
        h.command("resolvehost", &[Val::Str(String::new())]),
        Some(Val::Str(String::new()))
    );
}

/// #446: a GS2 script's `play`, `playlooped` and `play2` are the GS1
/// engine's sound effects, so they reach the session's sound queue;
/// `stopsound` and `stopmidi` too. Era-free: a 6.x host plays them.
#[test]
fn play_and_its_kin_are_sound_effects() {
    use seihai_core::scripting::gs2::compile::compile_gs2;
    let src = "function onCreated() {\n\
                 play(\"chest.wav\");\n\
                 this.playlooped(\"+rain.wav\");\n\
                 play2(\"swish.wav\", 12.5, 20);\n\
                 stopsound(\"rain.wav\");\n\
                 stopmidi();\n\
               }\n";
    let mut m = Machine::new(compile_gs2(src).unwrap());
    let mut h = EffectHost::new(
        Era::for_version(seihai_core::protocol::ClientVersion::V6034),
        7,
    );
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    assert!(h.log.is_empty(), "{:?}", h.log);
    assert_eq!(
        h.effects,
        [
            Effect::Play {
                obj: 7,
                file: "chest.wav".into(),
                looped: false,
                at: None
            },
            Effect::Play {
                obj: 7,
                file: "+rain.wav".into(),
                looped: true,
                at: None
            },
            Effect::Play {
                obj: 7,
                file: "swish.wav".into(),
                looped: false,
                at: Some((12.5, 20.0))
            },
            Effect::StopSound {
                file: "rain.wav".into()
            },
            Effect::StopMidi,
        ]
    );
}

/// #497: `player.chat` and the player's place and counters read from the
/// host's snapshot; writing `player.chat` says it (GS1's `setplayerprop
/// #c`), and writing a place or counter is the GS1 player variable.
#[test]
fn player_props_read_the_snapshot_and_writes_are_player_effects() {
    use seihai_core::scripting::gs2::compile::compile_gs2;
    let src = "function onPlayerChats() {\n\
                 if (player.chat == \"hello\") player.chat = \"hi \" @ player.nick;\n\
                 this.at = player.x + player.y;\n\
                 player.rupees += 5;\n\
                 player.dir = 6;\n\
               }\n";
    let mut m = Machine::new(compile_gs2(src).unwrap());
    let mut h = EffectHost::new(
        Era::for_version(seihai_core::protocol::ClientVersion::V600R2),
        7,
    );
    h.player.chat = "hello".into();
    h.player.nick = "Squirrel".into();
    h.player.x = 30.0;
    h.player.y = 2.5;
    h.player.rupees = 10.0;
    assert_eq!(m.call_event("onPlayerChats", &[], &mut h), Outcome::Done);
    assert!(h.log.is_empty(), "{:?}", h.log);
    assert_eq!(m.var(Root::This, "at"), Some(&Val::Num(32.5)));
    assert_eq!(h.player.chat, "hi Squirrel");
    assert_eq!(
        h.effects,
        [
            Effect::SetPlayerProp {
                code: "#c".into(),
                value: "hi Squirrel".into()
            },
            Effect::SetPlayerVar {
                name: "playerrupees".into(),
                value: 15.0
            },
            Effect::SetPlayerVar {
                name: "playerdir".into(),
                value: 2.0
            },
        ]
    );
}

/// Handler lookup folds ASCII case: a script that spells `onTimeOut` still
/// receives the timer fire that looks up `onTimeout` (#92).
#[test]
fn timeout_dispatch_matches_ontimeout_spelling() {
    use seihai_core::scripting::gs2::compile;
    let script = compile::compile_gs2(
        "function onCreated() { setTimer(0.05); }\n\
         function onTimeOut() { this.fired += 1; }\n",
    )
    .unwrap();
    let mut m = Machine::new(script);
    let mut h = host();
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    assert_eq!(m.timeout(), 0.05);
    let ran: Vec<String> = m.tick(&mut h).into_iter().map(|(e, _)| e).collect();
    assert_eq!(ran, ["onTimeout"]);
    assert_eq!(m.var(Root::This, "fired"), Some(&Val::Num(1.0)));
    // Direct dispatch with the canonical casing also finds the handler.
    assert_eq!(m.call_event("onTimeout", &[], &mut h), Outcome::Done);
    assert_eq!(m.var(Root::This, "fired"), Some(&Val::Num(2.0)));
}

/// Scheduled dispatch and cancel fold ASCII case the same way: a queued
/// `pInG` finds `onPing`, and `cancelEvents("drop")` drops `Drop`.
#[test]
fn scheduled_event_name_folds_case() {
    use seihai_core::scripting::gs2::compile;
    let script = compile::compile_gs2(
        "function onCreated() {\n\
           scheduleEvent(0.05, \"pInG\");\n\
           scheduleEvent(0.10, \"Drop\");\n\
           cancelEvents(\"drop\");\n\
         }\n\
         function onPing() { this.ping += 1; }\n\
         function onDrop() { this.drop += 1; }\n",
    )
    .unwrap();
    let mut m = Machine::new(script);
    let mut h = host();
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    let mut ran = Vec::new();
    for _ in 1..=3 {
        for (event, out) in m.tick(&mut h) {
            assert_eq!(out, Outcome::Done, "{event}");
            ran.push(event);
        }
    }
    // Tick reports the lookup spelling (`on` + queued name); the handler ran.
    assert_eq!(ran, ["onpInG"]);
    assert_eq!(m.var(Root::This, "ping"), Some(&Val::Num(1.0)));
    assert_eq!(m.var(Root::This, "drop"), None);
}

#[test]
fn an_events_arguments_are_also_params() {
    use seihai_core::scripting::gs2::compile::compile_gs2;
    let src = "function onKeyPressed() { this.k = params[0]; this.t = params[1]; }\n\
               function onNamed(a) { this.a = a; this.p = params[0]; }\n";
    let mut m = Machine::new(compile_gs2(src).unwrap());
    let mut h = host();
    let args = [Val::Num(65.0), Val::Str("a".into())];
    assert_eq!(m.call_event("onKeyPressed", &args, &mut h), Outcome::Done);
    assert_eq!(m.var(Root::This, "k"), Some(&Val::Num(65.0)));
    assert_eq!(m.var(Root::This, "t"), Some(&Val::Str("a".into())));
    assert_eq!(
        m.call_event("onNamed", &[Val::Num(3.0)], &mut h),
        Outcome::Done
    );
    assert_eq!(m.var(Root::This, "a"), Some(&Val::Num(3.0)));
    assert_eq!(m.var(Root::This, "p"), Some(&Val::Num(3.0)));
}

/// An object kept in a variable or passed as a parameter is still that
/// object: `this.me = player; this.me.chat = …` writes the player, and an
/// object compares unequal to null (Seihai #458: a real 5.12 walks its 3D
/// player through a variable that holds `player`).
#[test]
fn an_object_in_a_variable_or_parameter_is_still_the_object() {
    use seihai_core::scripting::gs2::compile::compile_gs2;
    let src = "function onCreated() {\n\
                 this.me = NULL;\n\
                 this.wasnull = (this.me == NULL);\n\
                 this.me = player;\n\
                 this.isnull = (this.me == NULL);\n\
                 this.same = (this.me == player);\n\
                 this.me.chat = \"via variable\";\n\
                 this.got = this.me.nick;\n\
                 tell(player);\n\
               }\n\
               function tell(o) {\n\
                 this.param = o.nick;\n\
               }\n";
    let mut m = Machine::new(compile_gs2(src).unwrap());
    let mut h = EffectHost::new(
        Era::for_version(seihai_core::protocol::ClientVersion::V600R2),
        7,
    );
    h.player.nick = "Squirrel".into();
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    assert!(h.log.is_empty(), "{:?}", h.log);
    assert_eq!(m.var(Root::This, "wasnull"), Some(&Val::Num(1.0)));
    assert_eq!(m.var(Root::This, "isnull"), Some(&Val::Num(0.0)));
    assert_eq!(m.var(Root::This, "same"), Some(&Val::Num(1.0)));
    assert_eq!(h.player.chat, "via variable");
    assert_eq!(m.var(Root::This, "got"), Some(&Val::from("Squirrel")));
    assert_eq!(m.var(Root::This, "param"), Some(&Val::from("Squirrel")));
}

/// A host property can be a list: `temp.p = player.position; temp.p[1]`.
#[test]
fn a_host_property_can_be_a_list() {
    use seihai_core::scripting::gs2::compile::compile_gs2;
    use seihai_core::scripting::gs2::exec::Host;
    use seihai_core::scripting::gs2::lists::Value;
    struct ListHost(Vec<String>);
    impl Host for ListHost {
        fn command(&mut self, _: &str, _: &[Val]) -> Option<Val> {
            None
        }
        fn get_prop_value(&mut self, root: Root, name: &str) -> Option<Value> {
            (root == Root::Player && name == "position")
                .then(|| Value::List([0.0, -6.0, 1.0].map(|n| Value::Val(Val::Num(n))).to_vec()))
        }
        fn log(&mut self, line: String) {
            self.0.push(line);
        }
    }
    let src = "function onCreated() {\n  temp.p = player.position;\n  this.y = temp.p[1];\n  this.n = player.position.size();\n}\n";
    let mut m = Machine::new(compile_gs2(src).unwrap());
    let mut h = ListHost(Vec::new());
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    assert_eq!(m.var(Root::This, "y"), Some(&Val::Num(-6.0)));
    assert_eq!(m.var(Root::This, "n"), Some(&Val::Num(3.0)));
    assert!(h.0.is_empty(), "{:?}", h.0);
}

/// seihai-core#136: the real 6.03r7 passes only the button to
/// `onMouseDown` and leaves the click position in `mousex`/`mousey`.
#[test]
fn on_mouse_down_reads_the_click_from_mousex_and_mousey() {
    use seihai_core::scripting::gs2::compile::compile_gs2;
    let src = "function onMouseDown(mode) {\n\
                 this.mode = mode;\n\
                 this.at = mousex @ \",\" @ mousey;\n\
               }\n";
    let mut m = Machine::new(compile_gs2(src).unwrap());
    let mut h = host();
    h.mouse = (31.0, 26.5);
    assert_eq!(
        m.call_event("onMouseDown", &[Val::from("left")], &mut h),
        Outcome::Done
    );
    assert_eq!(m.var(Root::This, "mode"), Some(&Val::from("left")));
    assert_eq!(m.var(Root::This, "at"), Some(&Val::from("31,26.5")));
}

fn run_source(src: &str, event: &str) -> Machine {
    let script = seihai_core::scripting::gs2::compile::compile_gs2(src).unwrap();
    let mut m = Machine::new(Script::parse(&script.write()).unwrap());
    let mut h = host();
    m.call_event(event, &[], &mut h);
    assert!(h.log.is_empty(), "{:?}", h.log);
    m
}

#[test]
fn labeled_loops_break_and_continue_the_named_loop() {
    let m = run_source(
        "function onCreated() {\n  this.n = 0;\n  outer: for (temp.i = 0; temp.i < 5; temp.i++) {\n    for (temp.j = 0; temp.j < 5; temp.j++) {\n      if (temp.j == 2) continue outer;\n      if (temp.i == 3) break outer;\n      this.n++;\n    }\n  }\n}\n",
        "onCreated",
    );
    // Rows 0 to 2 count two each before `continue outer`; row 3 stops.
    assert_eq!(m.var(Root::This, "n"), Some(&Val::Num(6.0)));
}

#[test]
fn lower_upper_xor_and_invert_run() {
    let m = run_source(
        "function onCreated() {\n  this.a = \"AbC\".lower();\n  this.b = \"AbC\".upper();\n  this.c = 6 xor 3;\n  this.d = ~0;\n  this.e = 2;\n  this.e ^= 3;\n}\n",
        "onCreated",
    );
    assert_eq!(m.var(Root::This, "a"), Some(&Val::from("abc")));
    assert_eq!(m.var(Root::This, "b"), Some(&Val::from("ABC")));
    assert_eq!(m.var(Root::This, "c"), Some(&Val::Num(5.0)));
    assert_eq!(m.var(Root::This, "d"), Some(&Val::Num(-1.0)));
    assert_eq!(m.var(Root::This, "e"), Some(&Val::Num(8.0)));
}
