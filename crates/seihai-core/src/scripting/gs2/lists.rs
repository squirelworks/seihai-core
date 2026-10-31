//! GS2 lists (Stage D, lists): the value a list variable holds and the
//! helpers the executor's list opcodes use. The 3.153 client keeps a list
//! as a variable object whose methods are reached through a table that is
//! not in the decompile; what is read from the VM and what is Seihai's own
//! is in `documentation/gs2-commands.md`.

use super::strings::trim;
use crate::scripting::engine::Val;

/// A value as the GS2 machine keeps it: a GS1 value or a list.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Val(Val),
    List(Vec<Value>),
}

impl Default for Value {
    fn default() -> Value {
        Value::Val(Val::default())
    }
}

impl From<Val> for Value {
    fn from(v: Val) -> Value {
        Value::Val(v)
    }
}

impl Value {
    /// As a GS1 value. A list is its elements' text joined with commas,
    /// the form of a GS1 string list. **[seihai]**
    pub fn val(&self) -> Val {
        match self {
            Value::Val(v) => v.clone(),
            Value::List(l) => Val::Str(l.iter().map(|e| e.val().text()).collect::<Vec<_>>().join(",")),
        }
    }

    pub fn num(&self) -> f64 {
        self.val().num()
    }

    /// Equal as the list search compares: lists element by element, other
    /// values through the executor's comparison.
    pub fn same(&self, other: &Value, cmp: fn(&Val, &Val) -> i32) -> bool {
        match (self, other) {
            (Value::List(a), Value::List(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same(y, cmp)),
            (Value::Val(a), Value::Val(b)) => cmp(a, b) == 0,
            _ => false,
        }
    }
}

/// A list of `n` zeros, `n` kept to 0..=10 000 as `new[n]` does.
pub fn zeros(n: i32) -> Vec<Value> {
    vec![Value::Val(Val::Num(0.0)); n.clamp(0, 10_000) as usize]
}

/// `s.tokenize(seps)` as the client's `0x3c1884` splits: a token runs to
/// the next separator byte; one that starts with `"` runs to the next lone
/// `"`, and `""` inside it is one `"`. Tokens are trimmed (`trim`); an
/// empty token is dropped unless it was quoted.
pub fn tokenize(s: &str, seps: &str) -> Vec<String> {
    let mut rest: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    while !rest.is_empty() {
        let quoted = rest[0] == '"';
        if quoted {
            rest.remove(0);
        }
        // `stop` is the index of the byte that ends the token, or the length.
        let mut stop = 0;
        while stop < rest.len() {
            let c = rest[stop];
            if !quoted {
                if seps.contains(c) {
                    break;
                }
            } else if c == '"' {
                if rest.get(stop + 1) != Some(&'"') {
                    break;
                }
                rest.remove(stop);
            }
            stop += 1;
        }
        if stop > 0 || quoted {
            out.push(trim(&rest[..stop].iter().collect::<String>()));
        }
        let next: String = rest.get(stop + 1..).unwrap_or_default().iter().collect();
        rest = trim(&next).chars().collect();
    }
    out
}

/// `s.positions(t)`: every index where `t` starts, overlapping ones too.
pub fn positions(s: &str, t: &str) -> Vec<Value> {
    let (s, t): (Vec<char>, Vec<char>) = (s.chars().collect(), t.chars().collect());
    if t.is_empty() || t.len() > s.len() {
        return Vec::new();
    }
    (0..=s.len() - t.len()).filter(|&i| s[i..i + t.len()] == t[..]).map(|i| Value::Val(Val::Num(i as f64))).collect()
}

/// `a.subarray(start, len)`: `start` kept to the list, a `len` below 0 runs
/// to the end (`0x3da47c`).
pub fn subarray(list: &[Value], start: i32, len: i32) -> Vec<Value> {
    let n = list.len() as i64;
    let start = (start as i64).clamp(0, n);
    let len = if len < 0 { n } else { len as i64 };
    let end = (start + len).min(n);
    list[start as usize..end as usize].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_splits_on_the_given_bytes_and_keeps_quoted_tokens() {
        assert_eq!(tokenize("a b,c", " ,"), ["a", "b", "c"]);
        assert_eq!(tokenize(" a ,, b ", ","), ["a", "b"]);
        assert_eq!(tokenize("\"x, y\",\"say \"\"hi\"\"\",\"\"", ","), ["x, y", "say \"hi\"", ""]);
        assert!(tokenize("", " ,").is_empty());
    }

    #[test]
    fn positions_overlap() {
        let n = |v: f64| Value::Val(Val::Num(v));
        assert_eq!(positions("aaa", "aa"), [n(0.0), n(1.0)]);
        assert!(positions("ab", "").is_empty());
    }

    #[test]
    fn subarray_clamps() {
        let l: Vec<Value> = (0..5).map(|i| Value::Val(Val::Num(i as f64))).collect();
        assert_eq!(subarray(&l, 3, 9).len(), 2);
        assert_eq!(subarray(&l, -2, 2), l[..2]);
        assert_eq!(subarray(&l, 1, -1), l[1..]);
    }
}
