//! Script values. A GS1 variable holds a number or a string; the two
//! convert freely, the way the original scripts expect.

/// A script value.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Num(f64),
    Str(String),
}

impl Default for Val {
    fn default() -> Self {
        Val::Num(0.0)
    }
}

impl From<f64> for Val {
    fn from(n: f64) -> Self {
        Val::Num(n)
    }
}

impl From<bool> for Val {
    fn from(b: bool) -> Self {
        Val::Num(if b { 1.0 } else { 0.0 })
    }
}

impl From<&str> for Val {
    fn from(s: &str) -> Self {
        Val::Str(s.to_string())
    }
}

impl From<String> for Val {
    fn from(s: String) -> Self {
        Val::Str(s)
    }
}

impl Val {
    /// Numeric value. A string reads as the number at its start, or 0.
    pub fn num(&self) -> f64 {
        match self {
            Val::Num(n) => *n,
            Val::Str(s) => parse_leading_number(s),
        }
    }

    /// Truth value: non-zero numbers and non-empty strings that do not
    /// read as 0 are true.
    pub fn truthy(&self) -> bool {
        match self {
            Val::Num(n) => *n != 0.0,
            Val::Str(s) => {
                let t = s.trim();
                let numeric_start = t.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '.' || c == '+');
                !t.is_empty() && (!numeric_start || parse_leading_number(t) != 0.0)
            }
        }
    }

    /// Text form. Whole numbers print without a decimal point.
    pub fn text(&self) -> String {
        match self {
            Val::Num(n) => num_to_string(*n),
            Val::Str(s) => s.clone(),
        }
    }

    pub fn is_str(&self) -> bool {
        matches!(self, Val::Str(_))
    }
}

/// Format a number: whole values as integers, others with at most six
/// decimals and no trailing zeros.
pub fn num_to_string(n: f64) -> String {
    if !n.is_finite() {
        return "0".into();
    }
    if n.fract() == 0.0 && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let s = format!("{n:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The number at the start of `s` (after blanks), or 0.
pub fn parse_leading_number(s: &str) -> f64 {
    let t = s.trim_start();
    let b = t.as_bytes();
    let mut end = 0;
    if end < b.len() && (b[end] == b'-' || b[end] == b'+') {
        end += 1;
    }
    let digits_from = end;
    while end < b.len() && b[end].is_ascii_digit() {
        end += 1;
    }
    if end < b.len() && b[end] == b'.' {
        let dot = end;
        end += 1;
        while end < b.len() && b[end].is_ascii_digit() {
            end += 1;
        }
        if end == dot + 1 && dot == digits_from {
            return 0.0;
        }
    }
    if end == digits_from {
        return 0.0;
    }
    t[..end].parse().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_whole_when_whole() {
        assert_eq!(Val::Num(3.0).text(), "3");
        assert_eq!(Val::Num(-2.5).text(), "-2.5");
        assert_eq!(Val::Num(0.1 + 0.2).text(), "0.3");
        assert_eq!(Val::Num(f64::NAN).text(), "0");
    }

    #[test]
    fn strings_read_as_leading_numbers() {
        assert_eq!(Val::from("12abc").num(), 12.0);
        assert_eq!(Val::from("  -3.5x").num(), -3.5);
        assert_eq!(Val::from("abc").num(), 0.0);
        assert_eq!(Val::from(".").num(), 0.0);
    }

    #[test]
    fn truth() {
        assert!(Val::Num(1.0).truthy());
        assert!(!Val::Num(0.0).truthy());
        assert!(!Val::from("").truthy());
        assert!(!Val::from("0").truthy());
        assert!(Val::from("hello").truthy());
        assert!(Val::from("2").truthy());
    }
}
