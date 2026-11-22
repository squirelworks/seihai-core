//! Authored source-to-own-VM acceptance controls, independent of corpus
//! bytecode and official compiler/native-client behavior.

use seihai_core::scripting::engine::{Era, Val};
use seihai_core::scripting::gs2::compile::{compile_gs2, compile_gs2_legacy};
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
