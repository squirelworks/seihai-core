//! The GS2 string built-ins as the 3.153 client's VM runs them (Stage D,
//! strings): `format`, `char`, and the `trim`, `length`, `pos`, `charat`,
//! `substring`, `starts` and `ends` methods. See
//! `documentation/gs2-commands.md`. Strings are bytes, one `char` each, as
//! in the reader.

use crate::scripting::engine::value::num_to_string;
use crate::scripting::engine::Val;

/// The client's number-to-integer conversion (`0x3d3ab8`): rounds down, so
/// `int(-1.5)` is -2. The client adds a small constant first, which was not
/// read.
pub fn client_int(x: f64) -> i32 {
    x.floor() as i32
}

/// `s.trim()`: drops leading and trailing bytes whose low 7 bits are a
/// space (0x20 and 0xA0).
pub fn trim(s: &str) -> String {
    s.trim_matches(|c: char| (c as u32) & 0x7F == 0x20 && (c as u32) < 0x100).to_string()
}

fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

/// `s.pos(sub)`: the first index of `sub`, or -1. An empty `sub` is -1.
pub fn pos(s: &str, sub: &str) -> i32 {
    let (s, sub) = (chars(s), chars(sub));
    if sub.is_empty() || sub.len() > s.len() {
        return -1;
    }
    (0..=s.len() - sub.len()).find(|&i| s[i..i + sub.len()] == sub[..]).map_or(-1, |i| i as i32)
}

/// `s.charat(i)`: that byte, or "" outside the string.
pub fn charat(s: &str, i: f64) -> String {
    let i = client_int(i);
    let c = chars(s);
    if i >= 0 && (i as usize) < c.len() {
        c[i as usize].to_string()
    } else {
        String::new()
    }
}

/// `s.substring(start, len)`: `start` below 0 is 0 and past the end is the
/// end; a `len` below 0 runs to the end.
pub fn substring(s: &str, start: f64, len: f64) -> String {
    let c = chars(s);
    let n = c.len() as i64;
    let start = (client_int(start) as i64).clamp(0, n);
    let len = client_int(len) as i64;
    let len = if len < 0 { n } else { len };
    let end = (start + len).min(n);
    c[start as usize..end as usize].iter().collect()
}

/// `char(n)`: the byte `n` as a one-byte string (a C `(char)(int)` cast).
pub fn char_of(n: f64) -> String {
    ((n as i64) as u8 as char).to_string()
}

/// `format(fmt, args…)`, as the client's `0x3dbb0c` reads the pattern:
/// `%[-][+][width][.precision][l]` then `d`/`i`, `u`, `f`, `s` or `%`.
/// Width and precision stop at 120. Any other letter after `%` is dropped
/// with it. Arguments the pattern asks for and the call did not give are
/// 0 or "" (the client reads on down its stack).
pub fn format(fmt: &str, args: &[Val]) -> String {
    let mut out = String::new();
    let mut args = args.iter();
    let mut it = fmt.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(mut c) = it.next() else { break };
        let left = c == '-';
        if left {
            let Some(n) = it.next() else { break };
            c = n;
        }
        let plus = c == '+';
        if plus {
            let Some(n) = it.next() else { break };
            c = n;
        }
        let mut width = 0usize;
        while let Some(d) = c.to_digit(10) {
            width = (width * 10 + d as usize).min(120);
            let Some(n) = it.next() else { return out };
            c = n;
        }
        let mut precision = None;
        if c == '.' {
            let mut p = 0usize;
            loop {
                let Some(n) = it.next() else { return out };
                c = n;
                match c.to_digit(10) {
                    Some(d) => p = (p * 10 + d as usize).min(120),
                    None => break,
                }
            }
            precision = Some(p);
        }
        if c == 'l' {
            let Some(n) = it.next() else { break };
            c = n;
        }
        match c {
            'd' | 'i' | 'u' => {
                let v = args.next().map_or(0.0, Val::num);
                let n = client_int(v) as i64;
                let n = if c == 'u' { n as i32 as u32 as i64 } else { n };
                out.push_str(&integer(n, width, precision, plus, left));
            }
            // The client's float writer is not given the width or precision.
            'f' => out.push_str(&num_to_string(args.next().map_or(0.0, Val::num))),
            's' => {
                let s: Vec<char> = args.next().map(Val::text).unwrap_or_default().chars().collect();
                let take = precision.map_or(s.len(), |p| p.min(s.len()));
                // Padded on the left whatever the `-` flag says.
                for _ in take..width {
                    out.push(' ');
                }
                out.extend(&s[..take]);
            }
            '%' => out.push('%'),
            _ => {}
        }
    }
    out
}

/// `%d`: at least `precision` digits, a sign for negatives (and `+` with
/// the flag), then padded with spaces to `width`, on the right with `-`.
fn integer(n: i64, width: usize, precision: Option<usize>, plus: bool, left: bool) -> String {
    let mut digits = n.unsigned_abs().to_string();
    if let Some(p) = precision {
        while digits.len() < p {
            digits.insert(0, '0');
        }
    }
    if n < 0 {
        digits.insert(0, '-');
    } else if plus {
        digits.insert(0, '+');
    }
    let pad = width.saturating_sub(digits.chars().count());
    if left {
        digits + &" ".repeat(pad)
    } else {
        " ".repeat(pad) + &digits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_conversion_rounds_down() {
        assert_eq!((client_int(1.9), client_int(-1.5), client_int(-2.0)), (1, -2, -2));
    }

    #[test]
    fn format_reads_the_pattern_as_the_client_does() {
        let n = |v: f64| Val::Num(v);
        assert_eq!(format("%d|%5d|%-5d|%+d|%.3d", &[n(42.0), n(7.0), n(7.0), n(3.0), n(5.0)]), "42|    7|7    |+3|005");
        assert_eq!(format("%s|%6s|%-6s|%.2s|%%|%q|", &[Val::from("ab"), Val::from("cd"), Val::from("ef"), Val::from("ghij")]), "ab|    cd|    ef|gh|%||");
        assert_eq!(format("%d and %s", &[n(-9.5)]), "-10 and ");
        assert_eq!(format("%u", &[n(-1.0)]), "4294967295");
        assert_eq!(format("cut %", &[]), "cut ");
    }

    #[test]
    fn substring_clamps_like_the_client() {
        assert_eq!(substring("Hello", 1.0, 3.0), "ell");
        assert_eq!(substring("Hello", -4.0, 100.0), "Hello");
        assert_eq!(substring("Hello", 9.0, 2.0), "");
        assert_eq!(substring("Hello", 2.0, -1.0), "llo");
    }

    #[test]
    fn trim_drops_spaces_and_their_high_bit_twin() {
        assert_eq!(trim("\u{a0} a b  "), "a b");
        assert_eq!(trim("\ta\t"), "\ta\t");
    }
}
