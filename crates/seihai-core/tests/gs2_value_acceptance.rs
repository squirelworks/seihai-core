//! Authored source-to-own-VM acceptance controls, independent of corpus
//! bytecode and official compiler/native-client behavior.

use seihai_core::scripting::engine::{Era, Val};
use seihai_core::scripting::gs2::compile::{compile_gs2, compile_gs2_legacy, Quirk};
use seihai_core::scripting::gs2::exec::{EffectHost, Machine, Outcome, Root};

fn in_both_modes(source: &str, check: impl Fn(&Machine)) {
    let strict = compile_gs2(source).unwrap();
    let legacy = compile_gs2_legacy(source).unwrap();
    assert!(legacy.warnings.is_empty(), "{:?}", legacy.warnings);
    for script in [strict, legacy.script] {
        let mut machine = Machine::new(script);
        let mut host = EffectHost::new(Era::V217, 7);
        assert_eq!(machine.run_top_level(&mut host), Outcome::Done);
        assert_eq!(
            machine.call_event("onCreated", &[], &mut host),
            Outcome::Done
        );
        assert!(host.log.is_empty(), "{:?}", host.log);
        assert!(host.effects.is_empty(), "{:?}", host.effects);
        check(&machine);
    }
}

#[test]
fn numeric_enums_keep_default_values_explicit_continuations_and_both_names() {
    in_both_modes(
        r#"
        enum Direction { NORTH, EAST, SOUTH = 5, WEST, RESET = 2, AFTERRESET };
        this.top = Direction::WEST;
        function onCreated() {
            this.n0 = NORTH; this.n0q = Direction::NORTH;
            this.n1 = EAST; this.n1q = Direction::EAST;
            this.n5 = SOUTH; this.n5q = Direction::SOUTH;
            this.n6 = WEST; this.n6q = Direction::WEST;
            this.n2 = RESET; this.n2q = Direction::RESET;
            this.n3 = AFTERRESET; this.n3q = Direction::AFTERRESET;
        }
        "#,
        |machine| {
            assert_eq!(machine.var(Root::This, "top"), Some(&Val::Num(6.0)));
            for (bare, qualified, expected) in [
                ("n0", "n0q", 0.0),
                ("n1", "n1q", 1.0),
                ("n5", "n5q", 5.0),
                ("n6", "n6q", 6.0),
                ("n2", "n2q", 2.0),
                ("n3", "n3q", 3.0),
            ] {
                assert_eq!(
                    machine.var(Root::This, bare),
                    Some(&Val::Num(expected)),
                    "{bare}"
                );
                assert_eq!(
                    machine.var(Root::This, qualified),
                    Some(&Val::Num(expected)),
                    "{qualified}"
                );
            }
        },
    );
}

#[test]
fn computed_dynamic_members_read_and_write_the_selected_property() {
    in_both_modes(
        r#"
        function onCreated() {
            this.slot5 = 11;
            this.slot6 = 99;
            this.before = this.("slot" @ (2 + 3));
            this.("slot" @ (1 + 4)) = this.before + 6;
            this.after = this.("slot" @ (9 - 4));
            this.staticread = this.slot5;
        }
        "#,
        |machine| {
            for (name, expected) in [
                ("before", 11.0),
                ("after", 17.0),
                ("staticread", 17.0),
                ("slot5", 17.0),
                ("slot6", 99.0),
            ] {
                assert_eq!(
                    machine.var(Root::This, name),
                    Some(&Val::Num(expected)),
                    "{name}"
                );
            }
        },
    );
}

#[test]
fn legacy_text_enum_preserves_both_names_and_numeric_continuation() {
    let source = r#"
        enum Status { BASE = 4, WORD = "leaf", NEXT, RESET = 9, AFTER };
        function onCreated() {
            this.base = Status::BASE;
            this.word = WORD; this.wordq = Status::WORD;
            this.next = NEXT; this.nextq = Status::NEXT;
            this.reset = Status::RESET;
            this.after = AFTER; this.afterq = Status::AFTER;
        }
    "#;
    let strict = compile_gs2(source).unwrap_err();
    assert_eq!(strict.errors.len(), 1, "{strict}");
    assert!(
        strict.to_string().contains(&Quirk::EnumText.to_string()),
        "{strict}"
    );
    let legacy = compile_gs2_legacy(source).unwrap();
    assert_eq!(legacy.warnings.len(), 1, "{:?}", legacy.warnings);
    assert_eq!(legacy.warnings[0].quirk, Quirk::EnumText);
    let mut machine = Machine::new(legacy.script);
    let mut host = EffectHost::new(Era::V217, 7);
    assert_eq!(machine.run_top_level(&mut host), Outcome::Done);
    assert_eq!(
        machine.call_event("onCreated", &[], &mut host),
        Outcome::Done
    );
    assert!(host.log.is_empty(), "{:?}", host.log);
    assert!(host.effects.is_empty(), "{:?}", host.effects);
    assert_eq!(machine.var(Root::This, "word"), Some(&Val::from("leaf")));
    assert_eq!(machine.var(Root::This, "wordq"), Some(&Val::from("leaf")));
    for (name, expected) in [
        ("base", 4.0),
        ("next", 5.0),
        ("nextq", 5.0),
        ("reset", 9.0),
        ("after", 10.0),
        ("afterq", 10.0),
    ] {
        assert_eq!(
            machine.var(Root::This, name),
            Some(&Val::Num(expected)),
            "{name}"
        );
    }
}

#[test]
fn computed_own_method_keeps_argument_bindings_and_evaluation_order() {
    // Seihai's generator evaluates arguments right-to-left, then the
    // computed name. This records its own contract, not an official order.
    in_both_modes(
        r#"
        function record_arg(tag, value) {
            this.trace = this.trace @ tag;
            return value;
        }
        function choose_own_name() {
            this.names += 1;
            this.trace = this.trace @ "N";
            return "com" @ "bine";
        }
        function combine(a, b) {
            this.calls += 1;
            this.trace = this.trace @ "C";
            return a * 10 + b;
        }
        function onCreated() {
            this.trace = ""; this.names = 0; this.calls = 0;
            this.named = this.combine(record_arg("L", 1), record_arg("R", 2));
            this.namedtrace = this.trace;
            this.namedcalls = this.calls; this.namednames = this.names;
            this.trace = ""; this.calls = 0;
            this.computed = this.(choose_own_name())(record_arg("L", 1), record_arg("R", 2));
            this.computedtrace = this.trace;
        }
        "#,
        |machine| {
            for (name, expected) in [
                ("named", 12.0),
                ("computed", 12.0),
                ("namedcalls", 1.0),
                ("calls", 1.0),
                ("namednames", 0.0),
                ("names", 1.0),
            ] {
                assert_eq!(
                    machine.var(Root::This, name),
                    Some(&Val::Num(expected)),
                    "{name}"
                );
            }
            assert_eq!(
                machine.var(Root::This, "namedtrace"),
                Some(&Val::from("RLC"))
            );
            assert_eq!(
                machine.var(Root::This, "computedtrace"),
                Some(&Val::from("RLNC"))
            );
        },
    );
}
