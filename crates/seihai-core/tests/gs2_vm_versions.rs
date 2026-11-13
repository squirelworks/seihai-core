//! The GS2 executor follows the VM of the client it presents as
//! (`documentation/gs2-vm-versions.md`): the opcodes each VM runs, its
//! limits, and its maths.

use seihai_core::protocol::ClientVersion;
use seihai_core::scripting::engine::{Era, Val};
use seihai_core::scripting::gs2::exec::{EffectHost, Machine, Outcome, Root};
use seihai_core::scripting::gs2::{Instr, Op, Operand, Script};

/// One instruction: an opcode number and its operand.
enum I {
    Num(f64),
    Str(&'static str),
    Op(u8),
    OpInt(u8, i32),
}

/// A top-level script that sets `this.r` to what `value` leaves on the stack.
fn script(value: &[I]) -> Script {
    let mut strings: Vec<String> = Vec::new();
    let mut idx = |s: &str| -> u32 {
        if let Some(i) = strings.iter().position(|x| x == s) {
            return i as u32;
        }
        strings.push(s.to_string());
        (strings.len() - 1) as u32
    };
    let mut code = Vec::new();
    let mut put = |op: u8, operand: Operand| {
        code.push(Instr {
            op: Op::from_u8(op),
            operand,
            offset: 0,
        })
    };
    put(180, Operand::None);
    put(22, Operand::Str(idx("r")));
    put(35, Operand::None);
    for i in value {
        match i {
            I::Num(n) => put(20, Operand::Decimal(n.to_string())),
            I::Str(s) => put(21, Operand::Str(idx(s))),
            I::Op(o) => put(*o, Operand::None),
            I::OpInt(o, n) => put(*o, Operand::Int(*n)),
        }
    }
    put(50, Operand::None);
    Script {
        gs1_flags: 0,
        functions: Vec::new(),
        strings,
        code,
    }
}

fn run(version: ClientVersion, value: &[I]) -> (Option<Val>, Vec<String>) {
    let mut m = Machine::for_version(script(value), version);
    let mut h = EffectHost::new(Era::V21, 1);
    assert_eq!(m.run_top_level(&mut h), Outcome::Done, "{:?}", h.log);
    (m.var(Root::This, "r").cloned(), h.log)
}

fn r(version: ClientVersion, value: &[I]) -> Val {
    run(version, value).0.unwrap_or(Val::Num(f64::NAN))
}

#[test]
fn each_version_names_the_vm_it_follows() {
    let label = |v| Machine::for_version(script(&[]), v).profile().label;
    assert_eq!(label(ClientVersion::V231), "3.153");
    assert_eq!(label(ClientVersion::V420R6), "4.20r6");
    assert_eq!(label(ClientVersion::V420R8), "4.20r8");
    assert_eq!(label(ClientVersion::V531R4), "5.00r7");
    assert_eq!(label(ClientVersion::V603R7), "6.03r7");
}

#[test]
fn modulo_adds_the_tolerance_only_in_4x() {
    // 0.3 % 0.1: 0.3/0.1 is 2.9999999999999996.
    let v = [I::Num(0.3), I::Num(0.1), I::Op(64)];
    let n = |ver| r(ver, &v).num();
    assert!((n(ClientVersion::V3153) - 0.1).abs() < 1e-9);
    assert!(n(ClientVersion::V420R8).abs() < 1e-9);
    assert!((n(ClientVersion::V500R7) - 0.1).abs() < 1e-9);
    assert!((n(ClientVersion::V603R7) - 0.1).abs() < 1e-9);
}

#[test]
fn square_root_is_single_precision_in_6x() {
    let v = [I::Num(2.0), I::Num(0.5), I::Op(65)];
    assert_eq!(r(ClientVersion::V500R7, &v), Val::Num(2f64.sqrt()));
    assert_eq!(r(ClientVersion::V603R7, &v), Val::Num((2f32).sqrt() as f64));
    // 3.153 has no 0.5 case: it is pow.
    assert_eq!(r(ClientVersion::V3153, &v), Val::Num(2f64.powf(0.5)));
}

#[test]
fn shifts_wrap_at_32_on_x86_builds_only() {
    let v = [I::Num(1.0), I::Num(33.0), I::Op(101)];
    assert_eq!(r(ClientVersion::V3153, &v), Val::Num(0.0));
    assert_eq!(r(ClientVersion::V420R8, &v), Val::Num(2.0));
}

#[test]
fn int_adds_the_tolerance_before_rounding_down() {
    let v = [I::Num(0.99995), I::Op(85)];
    assert_eq!(r(ClientVersion::V3153, &v), Val::Num(1.0));
    let v = [I::Num(-1.5), I::Op(85)];
    assert_eq!(r(ClientVersion::V603R7, &v), Val::Num(-2.0));
}

#[test]
fn numbers_within_the_tolerance_are_equal() {
    let v = [I::Num(1.0), I::Num(1.00005), I::Op(70)];
    assert_eq!(r(ClientVersion::V3153, &v), Val::Num(1.0));
}

#[test]
fn numbers_join_as_the_client_writes_them() {
    let v = [I::Str("x="), I::Num(0.1), I::Op(113)];
    assert_eq!(r(ClientVersion::V3153, &v), Val::from("x=0.1"));
    let v = [I::Str("x="), I::Num(2.5), I::Op(113)];
    assert_eq!(r(ClientVersion::V603R7, &v), Val::from("x=2.5"));
}

#[test]
fn local_slots_cache_a_value() {
    // 45 copies the top into slot 3 without popping; 46 pushes it back.
    let v = [
        I::Num(4.0),
        I::OpInt(45, 3),
        I::Op(32),
        I::OpInt(46, 3),
        I::OpInt(46, 3),
        I::Op(60),
    ];
    assert_eq!(r(ClientVersion::V3153, &v), Val::Num(8.0));
}

#[test]
fn logical_value_ops_give_one_or_zero() {
    let and = [I::Num(2.0), I::Num(0.0), I::Op(66)];
    let or = [I::Num(2.0), I::Num(0.0), I::Op(67)];
    assert_eq!(r(ClientVersion::V3153, &and), Val::Num(0.0));
    assert_eq!(r(ClientVersion::V3153, &or), Val::Num(1.0));
}

#[test]
fn add_or_join_runs_from_5x() {
    let v = [I::Str("a"), I::Num(1.0), I::Op(121)];
    assert_eq!(r(ClientVersion::V500R7, &v), Val::from("a1"));
    let v = [I::Num(2.0), I::Num(1.0), I::Op(121)];
    assert_eq!(r(ClientVersion::V603R7, &v), Val::Num(3.0));
    // 4.20r8 has no handler for 121: skipped, so the add never happens.
    let (_, log) = run(
        ClientVersion::V420R8,
        &[I::Num(2.0), I::Num(1.0), I::Op(121), I::Op(32)],
    );
    assert!(
        log.iter().any(|l| l.contains("not in the 4.20r8 VM")),
        "{log:?}"
    );
}

#[test]
fn compare_op_runs_from_4x() {
    let v = [I::Num(1.0), I::Num(2.0), I::Op(104)];
    assert_eq!(r(ClientVersion::V420R6, &v), Val::Num(-1.0));
    let (_, log) = run(
        ClientVersion::V3153,
        &[I::Num(1.0), I::Num(2.0), I::Op(104)],
    );
    assert!(
        log.iter().any(|l| l.contains("not in the 3.153 VM")),
        "{log:?}"
    );
}

#[test]
fn number_lists_run_in_5_00r7() {
    let v = [I::Num(1.0), I::Num(2.0), I::Num(3.0), I::OpInt(49, 3)];
    let mut m = Machine::for_version(script(&v), ClientVersion::V500R7);
    let mut h = EffectHost::new(Era::V21, 1);
    assert_eq!(m.run_top_level(&mut h), Outcome::Done);
    assert_eq!(m.list(Root::This, "r").map(<[_]>::len), Some(3));
}

#[test]
fn waitfor_waits_from_4_20r8_and_resumes_with_the_result() {
    let v = [I::Op(180), I::Str("onPing"), I::Num(2.0), I::Op(11)];
    let mut m = Machine::for_version(script(&v), ClientVersion::V420R8);
    let mut h = EffectHost::new(Era::V21, 1);
    assert_eq!(
        m.run_top_level(&mut h),
        Outcome::WaitFor {
            object: Some("this".into()),
            event: "Ping".into(),
            timeout: 2.0
        }
    );
    assert_eq!(m.resume_waitfor(true, &mut h), Outcome::Done);
    assert_eq!(m.var(Root::This, "r"), Some(&Val::Num(1.0)));

    // A zero timeout gives 0 at once.
    let v = [I::Op(180), I::Str("onPing"), I::Num(0.0), I::Op(11)];
    assert_eq!(r(ClientVersion::V603R7, &v), Val::Num(0.0));
}

#[test]
fn stack_limit_follows_the_version() {
    // 300 pushes: past 3.153's 256, inside 4.x's 10000.
    let mut v: Vec<I> = (0..300).map(|_| I::Num(1.0)).collect();
    v.extend((0..299).map(|_| I::Op(60)));
    let mut m = Machine::for_version(script(&v), ClientVersion::V3153);
    let mut h = EffectHost::new(Era::V21, 1);
    assert!(matches!(m.run_top_level(&mut h), Outcome::Stopped(_)));
    assert_eq!(r(ClientVersion::V420R8, &v), Val::Num(300.0));
    assert_eq!(r(ClientVersion::V603R7, &v), Val::Num(300.0));
}
