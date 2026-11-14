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
