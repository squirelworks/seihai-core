//! seihai-core#6: GS1 scripts the per-event lift cannot split run whole
//! from one dispatcher, as GS1 runs them: every statement in source order,
//! once per event, with the event flags telling which event it is. The
//! bytecode is run on Seihai's GS2 machine; no original client is involved.

use seihai_core::scripting::engine::{Era, Val};
use seihai_core::scripting::gs2::compile;
use seihai_core::scripting::gs2::exec::{EffectHost, Machine, Outcome, Root};

fn machine(src: &str) -> (Machine, EffectHost) {
    let script = compile::compile_gs1(src, Era::V217).unwrap_or_else(|e| panic!("{src}\n{e}"));
    let mut m = Machine::new(script);
    let mut host = EffectHost::new(Era::V217, 3);
    assert_eq!(m.run_top_level(&mut host), Outcome::Done);
    (m, host)
}

fn fire(m: &mut Machine, host: &mut EffectHost, event: &str) {
    assert_eq!(m.call_event(event, &[], host), Outcome::Done, "{event}");
}

fn num(m: &Machine, name: &str) -> f64 {
    match m.var(Root::This, name) {
        Some(Val::Num(n)) => *n,
        None => 0.0,
        other => panic!("this.{name} = {other:?}"),
    }
}

#[test]
fn suffix_function_calls_follow_the_event_body_without_replaying_direct_calls() {
    let src = "if (created) { this.n = 10; } if (timeout) { this.n = 20; } bump(); function bump() { this.n += 1; }";
    let (mut m, mut host) = machine(src);
    assert_eq!(num(&m, "n"), 0.0, "the suffix must not run during load");
    fire(&mut m, &mut host, "onCreated");
    assert_eq!(num(&m, "n"), 11.0);
    fire(&mut m, &mut host, "onTimeout");
    assert_eq!(num(&m, "n"), 21.0);
    fire(&mut m, &mut host, "bump");
    assert_eq!(
        num(&m, "n"),
        22.0,
        "direct calls execute only their function"
    );
    let (mut control, mut control_host) = machine("if (created) { this.n = 10; } this.n += 1;");
    fire(&mut control, &mut control_host, "onCreated");
    assert_eq!(num(&control, "n"), 11.0, "suffix assignment control");
}

#[test]
fn code_between_event_tests_runs_in_source_order_on_each_event() {
    let src = "\
if (created) { this.order = 1; }
this.order = this.order * 10 + 2;
this.runs += 1;
if (playertouchsme) { this.order = this.order * 10 + 3; }
";
    let (mut m, mut host) = machine(src);
    fire(&mut m, &mut host, "onCreated");
    assert_eq!(num(&m, "order"), 12.0);
    fire(&mut m, &mut host, "onPlayerTouchsMe");
    // The middle line runs first, then the touch block: 12 -> 122 -> 1223.
    assert_eq!(num(&m, "order"), 1223.0);
    assert_eq!(num(&m, "runs"), 2.0);
}

#[test]
fn an_event_test_with_else_runs_the_else_on_other_events() {
    let src = "\
if (playerenters) { this.enter += 1; } else { this.other += 1; }
if (timeout) { this.t += 1; }
";
    let (mut m, mut host) = machine(src);
    fire(&mut m, &mut host, "onPlayerEnters");
    fire(&mut m, &mut host, "onTimeout");
    fire(&mut m, &mut host, "onTimeout");
    assert_eq!(num(&m, "enter"), 1.0);
    assert_eq!(num(&m, "other"), 2.0);
    assert_eq!(num(&m, "t"), 2.0);
}

#[test]
fn events_in_larger_conditions_and_as_values() {
    let src = "\
if ((playerenters || playertouchsme) && this.on == 0) { this.on = 1; this.hits += 1; }
this.last = timeout;
if (timeout) { this.on = 0; }
";
    let (mut m, mut host) = machine(src);
    fire(&mut m, &mut host, "onPlayerEnters");
    assert_eq!(num(&m, "hits"), 1.0);
    assert_eq!(num(&m, "last"), 0.0);
    fire(&mut m, &mut host, "onPlayerTouchsMe");
    assert_eq!(num(&m, "hits"), 1.0, "still on");
    fire(&mut m, &mut host, "onTimeout");
    assert_eq!(num(&m, "last"), 1.0);
    fire(&mut m, &mut host, "onPlayerTouchsMe");
    assert_eq!(num(&m, "hits"), 2.0);
}

#[test]
fn break_outside_a_loop_ends_the_run() {
    let src = "\
if (playertouchsme) {
  this.a += 1;
  if (this.a > 1) break;
  this.b += 1;
}
this.c += 1;
";
    let (mut m, mut host) = machine(src);
    fire(&mut m, &mut host, "onPlayerTouchsMe");
    fire(&mut m, &mut host, "onPlayerTouchsMe");
    assert_eq!(num(&m, "a"), 2.0);
    assert_eq!(num(&m, "b"), 1.0);
    // The second run ended at the break, before the line after the block.
    assert_eq!(num(&m, "c"), 1.0);
}

#[test]
fn a_return_before_a_repeated_event_block_still_skips_it() {
    let src = "\
if (created) { this.p += 1; if (this.p > 1) return; }
if (created) { this.q += 1; }
";
    let (mut m, mut host) = machine(src);
    fire(&mut m, &mut host, "onCreated");
    assert_eq!((num(&m, "p"), num(&m, "q")), (1.0, 1.0));
    fire(&mut m, &mut host, "onCreated");
    assert_eq!((num(&m, "p"), num(&m, "q")), (2.0, 1.0));
}

#[test]
fn hash_i_reads_a_string_list_entry_and_hash_l_the_level() {
    let got = compile::compile_gs1(
        "if (created) { addstring this.l,a; addstring this.l,b; message #I(this.l,1) #I(this.l,#v(this.i)) #L; }\n",
        Era::V217,
    )
    .unwrap();
    let dis = got.disassemble();
    assert!(dis.contains("OP_LEVEL"), "{dis}");
    assert!(dis.contains("OP_ARRAY"), "{dis}");
    // A plain flag list is still refused.
    let err =
        compile::compile_gs1("if (created) { message #I(list,0); }\n", Era::V217).unwrap_err();
    assert!(err.to_string().contains("#I(list,0)"), "{err}");
}

#[test]
fn a_string_name_built_at_run_time_is_makevar() {
    let got = compile::compile_gs1(
        "if (created) { message #s(this.weapon#v(this.i)); }\n",
        Era::V217,
    )
    .unwrap();
    assert!(
        got.disassemble().contains("OP_MAKEVAR"),
        "{}",
        got.disassemble()
    );
    // A plain flag name built at run time is still a player flag: refused.
    assert!(compile::compile_gs1("if (created) { message #s(w#v(1)); }\n", Era::V217).is_err());
}

#[test]
fn unconditional_prefix_repeats_before_each_named_event() {
    let (mut m, mut host) = machine(
        "this.runs += 1; if (created) { this.created = this.runs; } if (timeout) { this.timed = this.runs; }",
    );
    assert_eq!(num(&m, "runs"), 0.0, "loading must not run the prefix");
    fire(&mut m, &mut host, "onCreated");
    assert_eq!((num(&m, "runs"), num(&m, "created")), (1.0, 1.0));
    fire(&mut m, &mut host, "onTimeout");
    assert_eq!((num(&m, "runs"), num(&m, "timed")), (2.0, 2.0));
}

#[test]
fn sleeping_dispatch_resumes_without_replaying_prefix_or_skipping_suffix() {
    let (mut m, mut host) = machine(
        "function bump() { this.order += 1; } this.runs += 1; if (created) { this.order = 10; sleep 0.1; this.order *= 10; } bump();",
    );
    assert_eq!(m.call_event("onCreated", &[], &mut host), Outcome::Sleep(0.1));
    assert_eq!((num(&m, "runs"), num(&m, "order")), (1.0, 10.0));
    assert_eq!(m.resume(&mut host), Outcome::Done);
    assert_eq!((num(&m, "runs"), num(&m, "order")), (1.0, 101.0));
}

#[test]
fn eventless_scripts_keep_top_level_execution() {
    let (m, _) = machine("this.loaded += 1;");
    assert_eq!(num(&m, "loaded"), 1.0);
}

#[test]
fn direct_gs2_public_calls_do_not_replay_top_level_code() {
    let script = compile::compile_gs2(
        "this.loads += 1; public function bump() { this.calls += 1; }",
    )
    .unwrap();
    let mut m = Machine::new(script);
    let mut host = EffectHost::new(Era::V217, 3);
    assert_eq!(m.run_top_level(&mut host), Outcome::Done);
    fire(&mut m, &mut host, "bump");
    fire(&mut m, &mut host, "bump");
    assert_eq!((num(&m, "loads"), num(&m, "calls")), (1.0, 2.0));
}
