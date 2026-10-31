//! The GS2 maths built-ins the 3.153 client's VM runs in place
//! (Stage D, the rest): `getangle`, `getdir`, `vecx`, `vecy` and the
//! shifts. Constants the decompile names but does not show are marked
//! **[inferred]**; see `documentation/gs2-commands.md`.

use std::f64::consts::PI;

/// `getangle(dx, dy)` (`0x3dc038`): `atan(-dy / dx)`, plus π when `dx` is
/// below 0, plus 2π when that is still below 0, so 0..2π with y down.
/// `dx == 0` gives 3π/2 when `dy > 0`, else π/2 **[inferred]**.
pub fn angle(dx: f64, dy: f64) -> f64 {
    if dx == 0.0 {
        return if dy > 0.0 { 3.0 * PI / 2.0 } else { PI / 2.0 };
    }
    let mut a = (-dy / dx).atan();
    if dx < 0.0 {
        a += PI;
    }
    if a < 0.0 {
        a += 2.0 * PI;
    }
    a
}

/// `getdir(dx, dy)` (case `0x60`): the axis that is longer, ties to the
/// vertical. Up 0, left 1, down 2, right 3 **[inferred]**.
pub fn dir(dx: f64, dy: f64) -> f64 {
    if dx.abs() <= dy.abs() {
        if dy < 0.0 {
            0.0
        } else {
            2.0
        }
    } else if dx < 0.0 {
        1.0
    } else {
        3.0
    }
}

/// `(vecx(d), vecy(d))`: the client reads a four-row table for `d` 0..3
/// and gives 0 for any other `d`. The rows are the usual direction steps
/// **[inferred]**: the table's bytes were not read.
pub fn vec(d: f64) -> (f64, f64) {
    match super::strings::client_int(d) {
        0 => (0.0, -1.0),
        1 => (-1.0, 0.0),
        2 => (0.0, 1.0),
        3 => (1.0, 0.0),
        _ => (0.0, 0.0),
    }
}

/// `a << b` on 32-bit integers, the count taken as `b & 63` as the client
/// masks it. A count of 32 or more gives 0 (PowerPC `slw`).
pub fn shl(a: i32, b: i32) -> i32 {
    let n = (b & 63) as u32;
    if n >= 32 {
        0
    } else {
        a.wrapping_shl(n)
    }
}

/// `a >> b`, keeping the sign; a count of 32 or more gives 0 or -1
/// (PowerPC `sraw`).
pub fn shr(a: i32, b: i32) -> i32 {
    let n = (b & 63) as u32;
    a >> n.min(31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angle_runs_counter_clockwise_with_y_down() {
        assert!((angle(1.0, 0.0)).abs() < 1e-12);
        assert!((angle(1.0, -1.0) - PI / 4.0).abs() < 1e-12);
        assert!((angle(-1.0, 0.0) - PI).abs() < 1e-12);
        assert!((angle(1.0, 1.0) - 7.0 * PI / 4.0).abs() < 1e-12);
        assert_eq!(angle(0.0, 2.0), 3.0 * PI / 2.0);
    }

    #[test]
    fn dir_picks_the_longer_axis() {
        assert_eq!([dir(0.0, -2.0), dir(-3.0, 1.0), dir(1.0, 1.0), dir(5.0, 0.0)], [0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn shifts_mask_the_count() {
        assert_eq!((shl(1, 4), shl(1, 32), shl(1, 68), shr(-256, 2), shr(-1, 40), shr(256, 40)), (16, 0, 16, -64, -1, 0));
    }
}
