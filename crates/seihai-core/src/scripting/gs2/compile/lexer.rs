//! GS2 tokens.
//!
//! The lexer reads one token ahead. Its cursor and line count can run past
//! a token that is already peeked (the parser skips blank space between
//! statements before it looks at the next one), and the line number of
//! that cursor is what names a `function () { … }` value, so the cursor is
//! kept as it is rather than tokenizing the whole file up front.

use super::Quirk;
use crate::scripting::engine::value::num_to_string;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Tok {
    Id(String),
    Num(f64),
    Str(String),
    /// A GS1 message code (`#a`, `#v(x)`) where a GS2 value is expected.
    Code(String),
    Op(&'static str),
    /// A byte that starts no token.
    Unknown,
    Eof,
}

const OPS: &[&str] = &[
    "<<", ">>", "||", "&&", "==", "!=", "<>", "<=", ">=", "=<", "=>", "+=", "-=", "*=", "/=", "%=",
    "&=", "|=", "@=", ":=", "++", "--", "+", "-", "*", "/", "%", "^", "=", "<", ">", "!", "(", ")",
    "{", "}", "[", "]", ",", ";", ".", "@", "|", "&", ":", "?",
];

/// The source text of decimal literals, keyed by the bits of their
/// absolute value. The reference compiler writes a decimal literal's text
/// as it stands (`3.0`, `.5`, `1.50`), not a normalised number. A value
/// written two different ways in one script falls back to the normalised
/// text.
#[derive(Debug, Default)]
pub(crate) struct Spellings {
    text: HashMap<u64, String>,
    mixed: HashSet<u64>,
}

impl Spellings {
    fn record(&mut self, value: f64, text: &str) {
        let key = value.to_bits();
        if self.mixed.contains(&key) {
            return;
        }
        match self.text.get(&key) {
            Some(seen) if seen != text => {
                self.text.remove(&key);
                self.mixed.insert(key);
            }
            Some(_) => {}
            None => {
                self.text.insert(key, text.to_string());
            }
        }
    }

    /// The written text for a literal whose spelling differs from the
    /// normalised form, or `None` to use the normalised form.
    pub fn get(&self, value: f64) -> Option<String> {
        let t = self.text.get(&value.abs().to_bits())?;
        if *t == num_to_string(value.abs()) {
            return None;
        }
        Some(if value < 0.0 {
            format!("-{t}")
        } else {
            t.clone()
        })
    }
}

pub(super) struct Lexer<'a> {
    pub src: &'a [u8],
    /// The cursor: just past the last token read (or past the blank space
    /// after it, once skipped).
    pub i: usize,
    /// The line of the cursor.
    pub line: u32,
    peek: Option<Tok>,
    /// The line the peeked token starts on.
    pub peek_line: u32,
    /// The line the last token taken started on.
    pub last_line: u32,
    pub spellings: Spellings,
    /// Non-standard spellings met while lexing, with their lines.
    pub quirks: Vec<(u32, Quirk)>,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer {
            src: src.as_bytes(),
            i: 0,
            line: 1,
            peek: None,
            peek_line: 1,
            last_line: 1,
            spellings: Spellings::default(),
            quirks: Vec::new(),
        }
    }

    pub fn peek(&mut self) -> Tok {
        if self.peek.is_none() {
            self.skip();
            self.peek_line = self.line;
            self.peek = Some(self.lex());
        }
        self.peek.clone().unwrap()
    }

    pub fn bump(&mut self) -> Tok {
        self.peek();
        self.last_line = self.peek_line;
        self.peek.take().unwrap()
    }

    /// Forget the peeked token and move the cursor to `at`, `lines` lines
    /// further on (after reading raw text past it).
    pub fn reset(&mut self, at: usize, lines: u32) {
        self.peek = None;
        self.i = at;
        self.line += lines;
    }

    /// Skips blank space and comments at the cursor.
    pub fn skip(&mut self) {
        loop {
            match self.src.get(self.i) {
                Some(b'\n') => {
                    self.line += 1;
                    self.i += 1;
                }
                Some(c) if c.is_ascii_whitespace() => self.i += 1,
                Some(b'/') if self.src.get(self.i + 1) == Some(&b'/') => {
                    while self.i < self.src.len() && self.src[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                Some(b'/') if self.src.get(self.i + 1) == Some(&b'*') => {
                    let end = self.block_comment_end(self.i);
                    self.line += self.src[self.i..end]
                        .iter()
                        .filter(|&&c| c == b'\n')
                        .count() as u32;
                    self.i = end;
                }
                _ => break,
            }
        }
    }

    /// The end of the `/* … */` comment at `start`. A C comment ends at its
    /// first `*/`. Live scripts nest them (`/* a /* b */ c */`), so where
    /// the nesting closes, the comment ends there instead (a quirk); one
    /// whose nesting never closes ends at its first `*/`.
    fn block_comment_end(&mut self, start: usize) -> usize {
        let b = self.src;
        let first = b[start + 2..]
            .windows(2)
            .position(|w| w == b"*/")
            .map(|p| start + 2 + p + 2);
        let (mut i, mut depth) = (start + 2, 1);
        while i + 1 < b.len() {
            match &b[i..i + 2] {
                b"/*" => {
                    depth += 1;
                    i += 2;
                }
                b"*/" => {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        if Some(i) != first {
                            self.quirks.push((self.line, Quirk::NestedComment));
                        }
                        return i;
                    }
                }
                _ => i += 1,
            }
        }
        match first {
            Some(end) => end,
            None => {
                self.quirks.push((self.line, Quirk::UnterminatedComment));
                b.len()
            }
        }
    }

    fn lex(&mut self) -> Tok {
        self.skip();
        let b = self.src;
        let Some(&c) = b.get(self.i) else {
            return Tok::Eof;
        };
        if c == b'"' || c == b'\'' {
            // `\n`, `\t` and `\r` are unescaped; any other escaped byte
            // stands for itself.
            let line = self.line;
            self.i += 1;
            let mut text = Vec::new();
            while self.i < b.len() && b[self.i] != c {
                let mut ch = b[self.i];
                if ch == b'\\' && self.i + 1 < b.len() {
                    self.i += 1;
                    ch = match b[self.i] {
                        b'n' => b'\n',
                        b't' => b'\t',
                        b'r' => b'\r',
                        other => other,
                    };
                }
                if ch == b'\n' {
                    self.line += 1;
                }
                text.push(ch);
                self.i += 1;
            }
            if self.i < b.len() {
                self.i += 1;
            } else {
                self.quirks.push((line, Quirk::UnterminatedString));
            }
            return Tok::Str(String::from_utf8_lossy(&text).into_owned());
        }
        // `#a`, `#c`, `#v(this.x)`, `#P1`: a GS1 message code.
        if c == b'#' && b.get(self.i + 1).is_some_and(|d| d.is_ascii_alphabetic()) {
            let start = self.i;
            self.i += 2;
            while b.get(self.i).is_some_and(|d| d.is_ascii_digit()) {
                self.i += 1;
            }
            if b.get(self.i) == Some(&b'(') {
                let mut depth = 0;
                while let Some(&d) = b.get(self.i) {
                    self.i += 1;
                    match d {
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            return Tok::Code(String::from_utf8_lossy(&b[start..self.i]).into_owned());
        }
        if c == b'0' && matches!(b.get(self.i + 1), Some(b'x' | b'X')) {
            self.i += 2;
            let start = self.i;
            while self.i < b.len() && b[self.i].is_ascii_hexdigit() {
                self.i += 1;
            }
            let digits = std::str::from_utf8(&b[start..self.i]).unwrap_or("0");
            return Tok::Num(u32::from_str_radix(digits, 16).unwrap_or(0) as f64);
        }
        if c.is_ascii_digit()
            || (c == b'.' && b.get(self.i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let start = self.i;
            while self.i < b.len() && b[self.i].is_ascii_digit() {
                self.i += 1;
            }
            if b.get(self.i) == Some(&b'.') {
                self.i += 1;
                while self.i < b.len() && b[self.i].is_ascii_digit() {
                    self.i += 1;
                }
            }
            let text = std::str::from_utf8(&b[start..self.i]).unwrap_or("0");
            let value = text.parse().unwrap_or(0.0);
            if text.contains('.') {
                self.spellings.record(value, text);
            } else {
                // Integer literals are written normalised (`007` is `7`);
                // they only mark the value as also written that way.
                self.spellings.record(value, &num_to_string(value));
            }
            return Tok::Num(value);
        }
        if c.is_ascii_alphabetic() || c == b'_' || c == b'$' {
            let start = self.i;
            self.i += 1;
            // `$pref::Video::x` is one name.
            loop {
                match b.get(self.i) {
                    Some(c) if c.is_ascii_alphanumeric() || *c == b'_' || *c == b'$' => self.i += 1,
                    Some(b':')
                        if b.get(self.i + 1) == Some(&b':')
                            && b.get(self.i + 2)
                                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') =>
                    {
                        self.i += 2
                    }
                    _ => break,
                }
            }
            return Tok::Id(String::from_utf8_lossy(&b[start..self.i]).into_owned());
        }
        let rest = &b[self.i..];
        for op in OPS {
            if rest.starts_with(op.as_bytes()) {
                self.i += op.len();
                return Tok::Op(op);
            }
        }
        self.i += 1;
        Tok::Unknown
    }
}
