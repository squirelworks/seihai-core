//! The GS2 VM rules of each client generation.
//!
//! The executor started from the 3.153 VM alone. These profiles come from
//! the VMs of every client with a readable binary: 3.153 and 4.20r6 (Mac
//! PPC), 4.20r8 and 5.00r7 (Windows engine DLLs) and 6.01r1 and 6.03r7
//! (Linux x64). The binaries and what each one says are listed in
//! `documentation/gs2-vm-versions.md`. A build with no binary takes the
//! closest earlier profile that was read, and the document says so.
//!
//! What every client does the same way (number tolerance, `int()`, the
//! number-to-text writer, the local-slot opcodes 45 to 47, `&&` and `||`
//! as values) is not a profile field. The executor does it for every
//! version.

use crate::protocol::ClientVersion;

/// Opcodes the clients run that gs2-parser's enum does not name. The
/// reader keeps them as [`super::Op::Unknown`].
pub mod op {
    /// `n`: copies the top value into local slot `n` of the running
    /// function and leaves it on the stack. Every client from 3.153.
    pub const SLOT_STORE: u8 = 45;
    /// `n`: pushes local slot `n`. Every client from 3.153.
    pub const SLOT_LOAD: u8 = 46;
    /// Resolves the variable on top (creating it), so a slot can keep it.
    /// Every client from 3.153.
    pub const RESOLVE_VAR: u8 = 47;
    /// 5.00r7 only: an object literal from `n` name/value pairs.
    pub const OBJECT_LITERAL: u8 = 48;
    /// 5.00r7 only: a list literal from `n` numbers.
    pub const NUMBER_LIST: u8 = 49;
    /// 5.00r7 on: a three-operand member store (not modelled).
    pub const MEMBER_STORE: u8 = 54;
    /// `a && b` as a value: 1 when both are non-zero. Every client.
    pub const LOGICAL_AND: u8 = 66;
    /// `a || b` as a value: 1 when either is non-zero. Every client.
    pub const LOGICAL_OR: u8 = 67;
    /// 5.00r7 on: `a + b`, or `a @ b` when either is a string.
    pub const ADD_OR_JOIN: u8 = 121;
}

/// How `a % b` finds its quotient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModRule {
    /// `a - floor(a / b) * b` (3.153, 5.00r7, 6.x).
    Floor,
    /// `a - floor(a / b + 0.0001) * b` (4.20r6, 4.20r8).
    FloorTolerant,
}

/// How `<<` and `>>` treat the count. The VMs shift 32-bit integers with
/// the CPU's own instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftRule {
    /// PowerPC `slw`/`sraw`: the count is taken mod 64, and 32 or more
    /// shifts everything out (3.153, 4.20r6).
    Ppc,
    /// x86 `shl`/`sar`: the count is taken mod 32 (4.20r8 on).
    X86,
}

/// The rules one client generation's GS2 VM runs by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmProfile {
    /// The build the rules were read from.
    pub label: &'static str,
    /// Values the stack holds before a push stops the script. `None`:
    /// the stack grows without a limit (6.x).
    pub stack_limit: Option<usize>,
    /// Passes of one loop before "Loop limit exceeded".
    pub loop_limit: u32,
    /// The script can set `maxlooplimit` on its object (100 to 1 000 000).
    pub max_loop_limit: bool,
    pub modulo: ModRule,
    pub shift: ShiftRule,
    /// `x ^ 0.5` for `x > 0` is a single-precision square root (6.x).
    pub float_sqrt: bool,
    /// `waitfor` (opcode 11) runs. Earlier VMs skip it.
    pub waitfor: bool,
    /// Opcode 104 (gs2-parser's `OP_OBJ_COMPARE`) runs: it compares the
    /// top two values and leaves -1, 0 or 1.
    pub compare_op: bool,
    /// Opcode 121 runs (5.00r7 on).
    pub add_or_join: bool,
    /// Opcodes 48 and 49 run (5.00r7 only).
    pub v5_literals: bool,
    /// Opcode 54 runs (5.00r7 on).
    pub member_store: bool,
}

/// 3.153 (Mac PPC). The VM the executor was first written from.
pub const V3153: VmProfile = VmProfile {
    label: "3.153",
    stack_limit: Some(256),
    loop_limit: 10_000,
    max_loop_limit: false,
    modulo: ModRule::Floor,
    shift: ShiftRule::Ppc,
    float_sqrt: false,
    waitfor: false,
    compare_op: false,
    add_or_join: false,
    v5_literals: false,
    member_store: false,
};

/// 4.20r6 (Mac PPC): the stack grows to 10 000, `%` adds 0.0001 before the
/// floor, and opcode 104 runs. No `waitfor` yet.
pub const V420R6: VmProfile = VmProfile {
    label: "4.20r6",
    stack_limit: Some(10_000),
    modulo: ModRule::FloorTolerant,
    compare_op: true,
    ..V3153
};

/// 4.20r8 (Windows): 4.20r6's rules on x86, with `waitfor`.
pub const V420R8: VmProfile = VmProfile {
    label: "4.20r8",
    shift: ShiftRule::X86,
    waitfor: true,
    ..V420R6
};

/// 5.00r7 (Windows): `maxlooplimit`, the plain floor in `%` again, and
/// opcodes 48, 49, 54 and 121.
pub const V500R7: VmProfile = VmProfile {
    label: "5.00r7",
    max_loop_limit: true,
    modulo: ModRule::Floor,
    add_or_join: true,
    v5_literals: true,
    member_store: true,
    ..V420R8
};

/// 6.01r1 and 6.03r7 (Linux x64, the same handler set): no stack limit,
/// `x ^ 0.5` in single precision, and 48 and 49 gone.
pub const V6: VmProfile = VmProfile {
    label: "6.03r7",
    stack_limit: None,
    float_sqrt: true,
    v5_literals: false,
    ..V500R7
};

impl VmProfile {
    /// The VM a client version runs. Builds before 3.153 have no GS2 VM;
    /// they get 3.153's, as the executor had before.
    pub fn for_version(v: ClientVersion) -> &'static VmProfile {
        use ClientVersion as C;
        match v {
            // 4.04r2's VM was not read; 4.20r6 is the closest read build.
            C::V404R2 | C::V420R6 => &V420R6,
            C::V420R8 => &V420R8,
            // 5.12 to 5.31r4 were not read; 5.00r7 is the closest.
            C::V500R7 | C::V512 | C::V524R4 | C::V530R3 | C::V531R4 => &V500R7,
            // 6.00r2, 6.00r4, 6.015 and 6.034 were not read; they take
            // 6.01r1/6.03r7's.
            C::V600R2 | C::V600R4 | C::V6015 | C::V6034 | C::V603R7 | C::V603R7Linux => &V6,
            _ => &V3153,
        }
    }

    /// Whether this VM has a handler for an opcode outside gs2-parser's
    /// named set, or for one of the named opcodes later VMs added.
    pub fn runs(&self, opcode: u8) -> bool {
        match opcode {
            11 => self.waitfor,
            104 => self.compare_op,
            op::OBJECT_LITERAL | op::NUMBER_LIST => self.v5_literals,
            op::MEMBER_STORE => self.member_store,
            op::ADD_OR_JOIN => self.add_or_join,
            _ => true,
        }
    }

    /// `a % b`. A zero `b` gives 0 in every VM.
    pub fn modulo(&self, a: f64, b: f64) -> f64 {
        if b == 0.0 {
            return 0.0;
        }
        let q = match self.modulo {
            ModRule::Floor => a / b,
            ModRule::FloorTolerant => a / b + TOLERANCE,
        };
        a - q.floor() * b
    }

    /// `a ^ b`.
    pub fn pow(&self, a: f64, b: f64) -> f64 {
        if self.float_sqrt && b == 2.0 {
            return a * a;
        }
        if a > 0.0 && b == 0.5 && self.float_sqrt {
            return (a as f32).sqrt() as f64;
        }
        client_pow(a, b)
    }

    /// `a << b` and `a >> b` on the 32-bit integers the VM converts to.
    pub fn shift(&self, a: i32, b: i32, left: bool) -> i32 {
        match self.shift {
            ShiftRule::Ppc => {
                if left {
                    super::maths::shl(a, b)
                } else {
                    super::maths::shr(a, b)
                }
            }
            ShiftRule::X86 => {
                let n = (b & 31) as u32;
                if left {
                    a.wrapping_shl(n)
                } else {
                    a >> n
                }
            }
        }
    }
}

/// The tolerance every VM uses when it turns a number into an integer and
/// when it compares two numbers.
pub const TOLERANCE: f64 = 0.0001;

/// `a ^ b` as every VM from 3.153 writes it: a positive base goes to the C
/// library; a zero base gives 0; a negative base gives 1 for `b == 0`,
/// `±(-a)^b` for a whole `b` (the sign from `b`'s parity), `1 / a^-b` for
/// a negative `b` (0 when that is 0) and 0 for any other `b`. 4.x and 5.x
/// send `b == 0.5` to `sqrt`, which is the same value.
pub fn client_pow(a: f64, b: f64) -> f64 {
    if a > 0.0 {
        return a.powf(b);
    }
    if a == 0.0 {
        return 0.0;
    }
    if b < 0.0 {
        let p = client_pow(a, -b);
        return if p == 0.0 { 0.0 } else { 1.0 / p };
    }
    if b == 0.0 {
        return 1.0;
    }
    let k = (b + TOLERANCE).floor();
    if b != k {
        return 0.0;
    }
    let p = (-a).powf(b);
    if (k as i64).rem_euclid(2) == 1 {
        -p
    } else {
        p
    }
}

/// A number as text, as every VM writes it: 0 for anything smaller than
/// 0.0001; otherwise the sign, the whole part (at most 2 147 483 647) and
/// up to nine decimals, cut (not rounded) and without trailing zeros, with
/// one exception: a ninth-place run of 999 is carried up, so 0.3 is
/// `0.3`, not `0.299999999`.
pub fn num_text(n: f64) -> String {
    if n.is_nan() || n.abs() < TOLERANCE {
        return "0".into();
    }
    let mut out = String::new();
    let mut x = n;
    if x < 0.0 {
        out.push('-');
        x = -x;
    }
    let (mut whole, mut frac) = if x <= 2_147_483_647.0 {
        let w = x as u32 as u64;
        let f = ((x - w as f64) * 1e9) as i64;
        (w, f)
    } else {
        (2_147_483_647, 0)
    };
    if frac % 1000 == 999 {
        frac += 1;
        if frac == 1_000_000_000 {
            whole += 1;
            frac = 0;
        }
    }
    out.push_str(&whole.to_string());
    if frac != 0 {
        let mut digits = 9;
        while frac % 10 == 0 {
            frac /= 10;
            digits -= 1;
        }
        out.push('.');
        out.push_str(&format!("{frac:0digits$}"));
    }
    out
}

/// Compares two numbers as every VM does: equal when they are within
/// 0.0001 of each other.
pub fn num_cmp(a: f64, b: f64) -> i32 {
    if a + TOLERANCE < b {
        -1
    } else if b + TOLERANCE < a {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_version_gets_the_vm_read_for_it_or_the_closest_earlier_one() {
        use ClientVersion as C;
        let label = |v| VmProfile::for_version(v).label;
        assert_eq!(label(C::V3153), "3.153");
        assert_eq!(label(C::V231), "3.153");
        assert_eq!(label(C::V404R2), "4.20r6");
        assert_eq!(label(C::V420R8), "4.20r8");
        assert_eq!(label(C::V524R4), "5.00r7");
        assert_eq!(label(C::V600R2), "6.03r7");
        assert_eq!(label(C::V603R7Linux), "6.03r7");
    }

    #[test]
    fn modulo_floors_and_4x_adds_the_tolerance() {
        assert_eq!(V3153.modulo(-7.0, 3.0), 2.0);
        assert_eq!(V3153.modulo(7.0, -3.0), -2.0);
        assert_eq!(V3153.modulo(5.0, 0.0), 0.0);
        // 0.3 / 0.1 is 2.9999999999999996: the plain floor gives 2.
        assert!((V6.modulo(0.3, 0.1) - 0.1).abs() < 1e-12);
        assert!(V420R8.modulo(0.3, 0.1).abs() < 1e-12);
    }

    #[test]
    fn pow_follows_the_client_rules() {
        assert_eq!(client_pow(-2.0, 3.0), -8.0);
        assert_eq!(client_pow(-2.0, 2.0), 4.0);
        assert_eq!(client_pow(-2.0, 0.5), 0.0);
        assert_eq!(client_pow(-2.0, -2.0), 0.25);
        assert_eq!(client_pow(0.0, 0.0), 0.0);
        assert_eq!(client_pow(-3.0, 0.0), 1.0);
        assert_eq!(V6.pow(2.0, 0.5), (2.0f32).sqrt() as f64);
        assert_eq!(V3153.pow(2.0, 0.5), 2.0f64.sqrt());
        assert_eq!(V6.pow(-3.0, 2.0), 9.0);
    }

    #[test]
    fn shifts_follow_the_cpu() {
        assert_eq!(V3153.shift(1, 32, true), 0);
        assert_eq!(V420R8.shift(1, 32, true), 1);
        assert_eq!(V420R8.shift(-256, 34, false), -64);
        assert_eq!(V3153.shift(-1, 40, false), -1);
    }

    #[test]
    fn numbers_print_with_nine_cut_decimals() {
        assert_eq!(num_text(1.0), "1");
        assert_eq!(num_text(-2.5), "-2.5");
        assert_eq!(num_text(0.1), "0.1");
        assert_eq!(num_text(0.3), "0.3");
        assert_eq!(num_text(1.0 / 3.0), "0.333333333");
        assert_eq!(num_text(2.0 / 3.0), "0.666666666");
        assert_eq!(num_text(0.00005), "0");
        assert_eq!(num_text(-0.0), "0");
        assert_eq!(num_text(3e10), "2147483647");
    }

    #[test]
    fn numbers_within_the_tolerance_are_equal() {
        assert_eq!(num_cmp(0.1 + 0.2, 0.3), 0);
        assert_eq!(num_cmp(1.0, 1.00005), 0);
        assert_eq!(num_cmp(1.0, 1.001), -1);
        assert_eq!(num_cmp(2.0, 1.0), 1);
    }
}
