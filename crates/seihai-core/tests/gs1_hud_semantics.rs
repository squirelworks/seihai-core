//! Issue 32: two GS1 forms that Seihai runs and gs2-parser drops.
//!
//! In the Zodiac/Era lab, real 5.12 and 5.24r4 clients drew a red "x" next
//! to the HP/AP bar and 99/99 arrows/bombs when two unmarked level NPC
//! scripts were Seihai-compiled, and not when gs2-parser compiled them. A
//! bisection on 5.24r4 traced each symptom to one script:
//!
//! - 99/99: a GS1 event guard, `if (playerenters) { player.<prop> = n; }`.
//!   gs2-parser reads the event as an ordinary variable that is never set,
//!   so the block never runs. Seihai reads the script as GS1 and lifts the
//!   block into `onPlayerEnters`, which the client runs on entry.
//! - Red "x": a bare GS1 command, `noplayerkilling;`, in `if (created)`.
//!   gs2-parser pushes the name and drops it. Seihai calls it with no
//!   arguments, which turns on the level's no-player-killing mode; the red
//!   crossed icon is that mode's indicator.
//!
//! Both are the GS1 meaning, which an unmarked GS1 script had on the GS1
//! servers, so they are kept on purpose. Strict and legacy GS2 still read
//! an event guard as a variable, as gs2-parser does: only the GS1 reading
//! lifts it. The scripts here are minimal and self-written; the lab's
//! scripts are not in the repository.

use seihai_core::scripting::engine::{Effect, Era};
use seihai_core::scripting::gs2::compile::{self, Front, Language, Mode};
use seihai_core::scripting::gs2::exec::{EffectHost, Machine, Outcome};
use seihai_core::scripting::gs2::{Op, Script};

const EVENT_GUARD: &str = "\
if (playerenters) {
  player.darts = 99;
  player.bombs = 99;
}
";

const BARE_COMMAND: &str = "\
if (created) {
  noplayerkilling;
}
";

fn detect(src: &str) -> compile::Compiled {
    compile::compile(src, Language::Detect(Era::V217), Mode::Legacy).unwrap()
}

fn ops(script: &Script, from: usize) -> Vec<String> {
    script.code[from..]
        .iter()
        .map(|i| i.op.to_string())
        .collect()
}

fn function_ops(script: &Script, name: &str) -> Vec<String> {
    let f = script
        .functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no {name}: {}", script.disassemble()));
    ops(script, f.start)
}

#[test]
fn an_unmarked_event_guard_is_read_as_gs1_and_lifted_into_its_handler() {
    let got = detect(EVENT_GUARD);
    assert_eq!(got.front, Front::Gs1);
    let names: Vec<_> = got
        .script
        .functions
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, ["onPlayerEnters"]);
    // The handler sets both counters; no top-level code tests the event.
    let body = function_ops(&got.script, "onPlayerEnters");
    let assigns = body.iter().filter(|o| *o == "OP_ASSIGN").count();
    assert_eq!(assigns, 2, "{}", got.script.disassemble());
    assert!(!got.script.strings.iter().any(|s| s == "playerenters"));
    assert!(!got.script.code.iter().any(|i| i.op == Op::If));
}

#[test]
fn the_same_guard_read_as_gs2_stays_a_variable_test() {
    // What gs2-parser does: `playerenters` is a variable, never true on a
    // client, so the block never runs.
    for mode in [Mode::Strict, Mode::Legacy] {
        let got = compile::compile(EVENT_GUARD, Language::Gs2, mode).unwrap();
        assert!(got.script.functions.is_empty());
        assert_eq!(got.script.code[0].op, Op::TypeVar);
        assert_eq!(got.script.strings[0], "playerenters");
        assert_eq!(got.script.code[2].op, Op::If);
    }
}

#[test]
fn an_unmarked_bare_command_is_a_zero_argument_call() {
    let got = detect(BARE_COMMAND);
    assert_eq!(got.front, Front::Gs1);
    let body = function_ops(&got.script, "onCreated");
    // OP_TYPE_ARRAY opens an empty argument list, then the name and the
    // call; the result is dropped.
    let at = body.iter().position(|o| o == "OP_CALL").unwrap();
    assert_eq!(
        &body[at - 2..=at + 1],
        ["OP_TYPE_ARRAY", "OP_TYPE_VAR", "OP_CALL", "OP_INDEX_DEC"],
        "{}",
        got.script.disassemble()
    );
    assert!(got.script.strings.iter().any(|s| s == "noplayerkilling"));
}

#[test]
fn the_bare_command_runs_on_created() {
    let got = detect(BARE_COMMAND);
    let mut machine = Machine::new(got.script);
    let mut host = EffectHost::new(Era::V217, 3);
    assert_eq!(machine.run_top_level(&mut host), Outcome::Done);
    assert_eq!(
        machine.call_event("onCreated", &[], &mut host),
        Outcome::Done
    );
    assert!(
        host.effects
            .iter()
            .any(|e| matches!(e, Effect::Command { name, .. } if name == "noplayerkilling")),
        "{:?} {:?}",
        host.effects,
        host.log
    );
}

#[test]
fn a_bare_command_in_gs2_is_also_called_with_a_warning() {
    // Legacy GS2 takes a GS1 command line as a call (`Quirk::Gs1Command`),
    // so a GS2 script gets the same call; only its `if (created)` guard
    // keeps it from running, as in gs2-parser.
    let got = compile::compile(BARE_COMMAND, Language::Gs2, Mode::Legacy).unwrap();
    assert!(got
        .warnings
        .iter()
        .any(|w| matches!(&w.quirk, compile::Quirk::Gs1Command(n) if n == "noplayerkilling")));
    assert!(got.script.code.iter().any(|i| i.op == Op::Call));
    assert_eq!(got.script.strings[0], "created");
}
