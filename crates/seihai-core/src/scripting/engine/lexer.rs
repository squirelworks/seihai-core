//! Lexer for GS1 source.
//!
//! GS1 mixes two surfaces. Expressions are ordinary tokens. Command
//! arguments that take text (`message`, `setimg`, `setplayerprop #c,hi`)
//! are raw text up to a comma or the end of the statement, so the parser
//! asks the lexer for those with [`Lexer::raw_arg`] instead of tokens.
//!
//! Identifiers are folded to lower case: GS1 is case-insensitive.

/// A token.
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// Name, possibly dotted: `this.hp`, `client.flag`.
    Ident(String),
    Num(f64),
    /// Quoted text, or a bare message code such as `#c`. Message codes
    /// inside are expanded when the value is used.
    Str(String),
    Op(&'static str),
    Eof,
}

/// A token with its place in the source.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned {
    pub tok: Tok,
    pub start: usize,
    pub end: usize,
    pub line: u32,
}

// `=<`, `=>` and `<>` are in the 2.05 client's comparison table beside
// `<=`, `>=` and `!=`. `.` is the field after an index (`npcs[i].x`).
const OPS2: &[&str] = &["==", "!=", "<=", ">=", "=<", "=>", "<>", "&&", "||", "+=", "-=", "*=", "/=", "++", "--"];
const OPS1: &[&str] = &[
    "+", "-", "*", "/", "%", "^", "=", "<", ">", "!", "(", ")", "{", "}", "[", "]", ",", ";", "|", ":", "?", "@", ".",
];

/// The digits that belong to a message code whose letter starts `code`:
/// `#P1` to `#P30` (gani attributes) and `#C0` to `#C4` (colours), as in
/// the 2.2 command list. Other codes are one character.
pub fn code_digits(code: &[u8]) -> usize {
    match code.first() {
        Some(b'P' | b'C') => code[1..].iter().take(2).take_while(|d| d.is_ascii_digit()).count(),
        _ => 0,
    }
}

#[derive(Clone)]
pub struct Lexer<'a> {
    src: &'a str,
    b: &'a [u8],
    pub pos: usize,
    pub line: u32,
    /// Whether the last [`Lexer::raw_arg`] result had its quotes removed.
    pub last_quoted: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer { src, b: src.as_bytes(), pos: 0, line: 1, last_quoted: false }
    }

    pub fn at_end(&mut self) -> bool {
        self.skip_blank();
        self.pos >= self.b.len()
    }

    /// Skip white space and comments.
    pub fn skip_blank(&mut self) {
        loop {
            match self.b.get(self.pos) {
                Some(b'\n') => {
                    self.line += 1;
                    self.pos += 1;
                }
                Some(c) if c.is_ascii_whitespace() => self.pos += 1,
                Some(b'/') if self.b.get(self.pos + 1) == Some(&b'/') => {
                    while self.pos < self.b.len() && self.b[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                }
                Some(b'/') if self.b.get(self.pos + 1) == Some(&b'*') => {
                    self.pos += 2;
                    while self.pos < self.b.len() && !(self.b[self.pos] == b'*' && self.b.get(self.pos + 1) == Some(&b'/')) {
                        if self.b[self.pos] == b'\n' {
                            self.line += 1;
                        }
                        self.pos += 1;
                    }
                    self.pos = (self.pos + 2).min(self.b.len());
                }
                _ => break,
            }
        }
    }

    /// Next token. Characters that start no token are skipped as `Op`
    /// would be wrong for them; they come back as a one-character `Str`
    /// so the parser can report them.
    pub fn next(&mut self) -> Spanned {
        self.skip_blank();
        let start = self.pos;
        let line = self.line;
        let mk = |tok, end| Spanned { tok, start, end, line };
        let Some(&c) = self.b.get(self.pos) else {
            return mk(Tok::Eof, start);
        };
        // Quoted string. No escapes: it ends at the next quote.
        if c == b'"' {
            let mut i = self.pos + 1;
            while i < self.b.len() && self.b[i] != b'"' {
                if self.b[i] == b'\n' {
                    self.line += 1;
                }
                i += 1;
            }
            let text = self.src[self.pos + 1..i.min(self.b.len())].to_string();
            self.pos = (i + 1).min(self.b.len());
            return mk(Tok::Str(text), self.pos);
        }
        // Bare message code: `#c`, `#v(this.a)`.
        if c == b'#' {
            let mut i = self.pos + 1;
            if i < self.b.len() && self.b[i].is_ascii_alphanumeric() {
                i += 1;
                i += code_digits(&self.b[i - 1..]);
                if self.b.get(i) == Some(&b'(') {
                    let mut depth = 0;
                    while i < self.b.len() {
                        match self.b[i] {
                            b'(' => depth += 1,
                            b')' => {
                                depth -= 1;
                                if depth == 0 {
                                    i += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                }
            }
            let text = self.src[self.pos..i.min(self.b.len())].to_string();
            self.pos = i.min(self.b.len());
            return mk(Tok::Str(text), self.pos);
        }
        if c.is_ascii_digit() || (c == b'.' && self.b.get(self.pos + 1).is_some_and(|d| d.is_ascii_digit())) {
            let mut i = self.pos;
            while i < self.b.len() && self.b[i].is_ascii_digit() {
                i += 1;
            }
            if self.b.get(i) == Some(&b'.') && self.b.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
                i += 1;
                while i < self.b.len() && self.b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            // Flag names may start with a digit (`2wallgizone`): a number
            // run straight into letters is a name.
            if self.b.get(i).is_some_and(|d| d.is_ascii_alphabetic() || *d == b'_') {
                while i < self.b.len() && (self.b[i].is_ascii_alphanumeric() || self.b[i] == b'_') {
                    i += 1;
                }
                let text = self.src[self.pos..i].to_ascii_lowercase();
                self.pos = i;
                return mk(Tok::Ident(text), i);
            }
            let n = self.src[self.pos..i].parse().unwrap_or(0.0);
            self.pos = i;
            return mk(Tok::Num(n), i);
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let mut i = self.pos;
            while i < self.b.len()
                && (self.b[i].is_ascii_alphanumeric()
                    || self.b[i] == b'_'
                    || (self.b[i] == b'.' && self.b.get(i + 1).is_some_and(|d| d.is_ascii_alphanumeric() || *d == b'_')))
            {
                i += 1;
            }
            let text = self.src[self.pos..i].to_ascii_lowercase();
            self.pos = i;
            return mk(Tok::Ident(text), i);
        }
        let rest = &self.src[self.pos..];
        for op in OPS2 {
            if rest.starts_with(op) {
                self.pos += 2;
                return mk(Tok::Op(op), self.pos);
            }
        }
        for op in OPS1 {
            if rest.starts_with(op) {
                self.pos += 1;
                return mk(Tok::Op(op), self.pos);
            }
        }
        // Anything else (a stray byte): hand it back as text.
        let ch = rest.chars().next().unwrap_or('?');
        self.pos += ch.len_utf8();
        mk(Tok::Str(ch.to_string()), self.pos)
    }

    /// Raw argument text from the current position up to a top-level
    /// delimiter (one of `stops`, never consumed) or the end of input.
    /// Parentheses are counted only inside message codes (`#v(...)`), so a
    /// text such as `Hello :)` is safe. A pair of surrounding double
    /// quotes is removed.
    pub fn raw_arg(&mut self, stops: &[u8]) -> String {
        self.raw_arg_in(stops, false)
    }

    /// [`Self::raw_arg`], where `{...}` pairs are kept whole (`showpoly
    /// 300,{0,0,32,0}` of an unknown command), so their `}` ends nothing.
    pub fn raw_arg_braced(&mut self, stops: &[u8]) -> String {
        self.raw_arg_in(stops, true)
    }

    fn raw_arg_in(&mut self, stops: &[u8], braces: bool) -> String {
        self.skip_blank();
        let mut brace = 0i32;
        let start = self.pos;
        let mut depth = 0i32;
        // Plain parentheses in text: a comma inside them does not end the
        // argument (`@font@#s(a) (#v(x), #v(y))`). [inferred]
        let mut paren = 0i32;
        let mut i = self.pos;
        // A whole argument in quotes may hold commas and semicolons.
        if self.b.get(i) == Some(&b'"') {
            if let Some(close) = self.b[i + 1..].iter().position(|&c| c == b'"') {
                let mut j = i + 1 + close + 1;
                while j < self.b.len() && matches!(self.b[j], b' ' | b'\t') {
                    j += 1;
                }
                if j >= self.b.len() || stops.contains(&self.b[j]) {
                    self.line += self.b[i..j].iter().filter(|&&c| c == b'\n').count() as u32;
                    self.pos = j;
                    self.last_quoted = true;
                    return self.src[i + 1..i + 1 + close].to_string();
                }
            }
        }
        while i < self.b.len() {
            let c = self.b[i];
            if braces && c == b'{' {
                brace += 1;
            } else if braces && c == b'}' && brace > 0 {
                brace -= 1;
                i += 1;
                continue;
            }
            if depth == 0 && brace == 0 && stops.contains(&c) && !(paren > 0 && matches!(c, b',' | b')')) {
                break;
            }
            if depth == 0 && c == b'(' {
                paren += 1;
            } else if depth == 0 && c == b')' && paren > 0 {
                paren -= 1;
            }
            if c == b'\n' {
                self.line += 1;
            }
            if c == b'#' && self.b.get(i + 1).is_some_and(|d| d.is_ascii_alphanumeric()) {
                let open = i + 2 + code_digits(&self.b[i + 1..]);
                if self.b.get(open) == Some(&b'(') {
                    depth += 1;
                    i = open + 1;
                    continue;
                }
            }
            if depth > 0 {
                if c == b'(' {
                    depth += 1;
                } else if c == b')' {
                    depth -= 1;
                }
            }
            i += 1;
        }
        self.pos = i;
        let text = self.src[start..i].trim();
        self.last_quoted = false;
        if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') && !text[1..text.len() - 1].contains('"') {
            self.last_quoted = true;
            text[1..text.len() - 1].to_string()
        } else {
            text.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        let mut l = Lexer::new(src);
        let mut v = Vec::new();
        loop {
            let t = l.next().tok;
            if t == Tok::Eof {
                return v;
            }
            v.push(t);
        }
    }

    #[test]
    fn identifiers_are_lowercase_and_dotted() {
        assert_eq!(toks("This.HP"), vec![Tok::Ident("this.hp".into())]);
        assert_eq!(toks("a.b.c1"), vec![Tok::Ident("a.b.c1".into())]);
    }

    #[test]
    fn numbers_strings_and_codes() {
        assert_eq!(toks("12 3.5 .25"), vec![Tok::Num(12.0), Tok::Num(3.5), Tok::Num(0.25)]);
        assert_eq!(toks("\"hi there\""), vec![Tok::Str("hi there".into())]);
        assert_eq!(toks("#c"), vec![Tok::Str("#c".into())]);
        assert_eq!(toks("#v(this.a+1)"), vec![Tok::Str("#v(this.a+1)".into())]);
    }

    #[test]
    fn names_may_start_with_a_digit() {
        assert_eq!(toks("2wallgizone 12 3x"), vec![Tok::Ident("2wallgizone".into()), Tok::Num(12.0), Tok::Ident("3x".into())]);
    }

    #[test]
    fn operators_prefer_two_characters() {
        assert_eq!(
            toks("a<=b && c!=d ++ += ="),
            vec![
                Tok::Ident("a".into()),
                Tok::Op("<="),
                Tok::Ident("b".into()),
                Tok::Op("&&"),
                Tok::Ident("c".into()),
                Tok::Op("!="),
                Tok::Ident("d".into()),
                Tok::Op("++"),
                Tok::Op("+="),
                Tok::Op("="),
            ]
        );
    }

    #[test]
    fn comments_are_skipped_and_lines_counted() {
        let mut l = Lexer::new("// one\n/* two\nthree */ x");
        let t = l.next();
        assert_eq!(t.tok, Tok::Ident("x".into()));
        assert_eq!(t.line, 3);
    }

    #[test]
    fn raw_args_stop_at_delimiters_and_keep_codes_whole() {
        let mut l = Lexer::new("  Hello :), #v(max(1,2)) there ;rest");
        assert_eq!(l.raw_arg(b",;"), "Hello :)");
        l.pos += 1;
        assert_eq!(l.raw_arg(b",;"), "#v(max(1,2)) there");
        assert_eq!(l.raw_arg(b",;"), "");
    }

    #[test]
    fn raw_args_drop_surrounding_quotes() {
        let mut l = Lexer::new("\"quoted text\";");
        assert_eq!(l.raw_arg(b",;"), "quoted text");
    }

    #[test]
    fn stray_bytes_do_not_stop_the_lexer() {
        assert_eq!(toks("~ x"), vec![Tok::Str("~".into()), Tok::Ident("x".into())]);
    }
}
