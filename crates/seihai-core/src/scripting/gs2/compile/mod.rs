//! The GS1 and GS2 compiler: script source to the GS2 bytecode described in
//! `documentation/gs2-opcodes.md` and `documentation/gs2-grammar.md`.
//!
//! Two front ends build one syntax tree ([`ast`]), and one code generator
//! ([`codegen`]) writes the bytecode:
//!
//! - GS2 source goes through [`parser`].
//! - GS1 source is parsed with the GS1 engine's grammar and lowered by
//!   [`gs1`]: event tests become event functions, message codes become
//!   joins, GS1 commands become GS2 calls or statements.
//!
//! # Strict and legacy
//!
//! [`Mode::Strict`], the default, takes the language as written and nothing
//! else. Live scripts from the 2.x and 3.x servers lean on forms the
//! original compilers let through: a missing `;`, `=` used as a comparison,
//! a GS1 command line in a GS2 script, a function defined twice, and more
//! (see [`Quirk`]). [`Mode::Legacy`] reads each of those the way the live
//! servers did and returns a [`Warning`] for it, so old levels still load.
//! A script with no quirks compiles to the same bytes in both modes.
//!
//! Only legacy mode guesses the language of a script that does not say
//! ([`Language::Detect`]).
//!
//! GS2-to-GS1 and server-side GS2 are out of scope. A construct the
//! compiler cannot lower is an error in both modes.

mod ast;
mod codegen;
mod gs1;
mod lexer;
mod parser;
mod tables;

pub use tables::{gs1_command_lowering, gs1_commands_without_gs2, gs2_event_function, Gs1Lowering};

use super::Script;
use crate::scripting::engine::{vocab, Era};
use std::fmt;

/// How strictly to read a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// The language as written: every [`Quirk`] is an error.
    #[default]
    Strict,
    /// Accept the quirks live scripts rely on, with a warning for each.
    Legacy,
}

/// The language of a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    /// GS1 of the given client version.
    Gs1(Era),
    Gs2,
    /// Not marked, as a server holding both GS1 and GS2 scripts would see
    /// it (legacy mode only). A script with no `function` that tests a GS1
    /// event (`if (playerenters)`) is GS1; any other is GS2, and falls back
    /// to GS1 when only the GS1 front end takes it. This is Seihai's
    /// reading: how the live servers chose is not recorded. The era is the
    /// one the GS1 front end reads.
    Detect(Era),
}

/// Which front end compiled a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Front {
    Gs2,
    Gs1,
}

/// A form live scripts use that the language does not have. Legacy mode
/// accepts each one with a [`Warning`]; strict mode refuses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Quirk {
    /// A statement ends at a line break, `}` or the end of the file
    /// instead of `;`.
    MissingSemicolon,
    /// Statements joined by `,` (`a.x = 1, a.y = 2;`).
    CommaStatements,
    /// A lone `;`.
    EmptyStatement,
    /// A block still open at the end of the file.
    UnclosedBlock,
    /// A string still open at the end of the file.
    UnterminatedString,
    /// A `/*` comment still open at the end of the file.
    UnterminatedComment,
    /// `/* /* */ */`: comments nested, ending where the nesting closes.
    NestedComment,
    /// `=` used as a comparison in an `if` or `while` condition.
    AssignInCondition,
    /// `elseif` for `else if`.
    ElseIf,
    /// `<>`, `=<`, `=>` or `:=` for `!=`, `<=`, `>=` or `=`.
    OldOperator(&'static str),
    /// A function body that is one statement without braces.
    BracelessFunctionBody,
    /// A function with no body.
    MissingFunctionBody,
    /// `function f();` with the body after the `;`.
    DeclarationBeforeBody,
    /// A function or parameter name written as a string.
    StringName,
    /// A function written inside a block; it becomes a function of the
    /// script.
    NestedFunction,
    /// A function defined again; the later one wins.
    DuplicateFunction(String),
    /// A GS1 command line in a GS2 script (`setimgpart a.png,0,0,8,8;`),
    /// read as GS1.
    Gs1Command(String),
    /// A GS1 message code (`#a`, `#v(x)`) used as a GS2 value.
    Gs1MessageCode,
    /// GS1's `in <a, b>` for `in |a, b|`.
    Gs1Range,
    /// `global enum`.
    GlobalEnum,
    /// An `enum` member given a text value.
    EnumText,
    /// A value used as a statement that is not a call, an assignment or a
    /// step (`x;`, `a == b;`).
    BareExpression,
    /// `(@"a" @ n) = v`, read as `makevar(@"a" @ n) = v`.
    RuntimeNameAssign,
    /// `f(x)()`: the result of a call called by name.
    CallOfCallResult,
    /// An unquoted `/word` argument, read as the text `"/word"`.
    SlashWord(String),
    /// The script did not say which language it is in; it was read as this
    /// one.
    GuessedLanguage(Front),
    /// A GS1 script for this version that uses later GS1 (`with`, `in`
    /// with a variable or `<a, b>`, `{…}` lists, newer commands); it was
    /// compiled as GS1 2.1.
    LaterGs1(Era),
}

impl fmt::Display for Quirk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Quirk::MissingSemicolon => write!(f, "missing `;`"),
            Quirk::CommaStatements => write!(f, "statements joined by `,` instead of `;`"),
            Quirk::EmptyStatement => write!(f, "stray `;`"),
            Quirk::UnclosedBlock => write!(f, "block not closed before the end of the file"),
            Quirk::UnterminatedString => write!(f, "string not closed before the end of the file"),
            Quirk::UnterminatedComment => {
                write!(f, "comment not closed before the end of the file")
            }
            Quirk::NestedComment => write!(f, "nested `/* */` comment"),
            Quirk::AssignInCondition => write!(f, "`=` used as a comparison; write `==`"),
            Quirk::ElseIf => write!(f, "`elseif`; write `else if`"),
            Quirk::OldOperator(op) => {
                let new = match *op {
                    "<>" => "!=",
                    "=<" => "<=",
                    "=>" => ">=",
                    _ => "=",
                };
                write!(f, "`{op}`; write `{new}`")
            }
            Quirk::BracelessFunctionBody => write!(f, "function body without braces"),
            Quirk::MissingFunctionBody => write!(f, "function without a body"),
            Quirk::DeclarationBeforeBody => write!(f, "`;` between a function and its body"),
            Quirk::StringName => write!(f, "function or parameter name written as a string"),
            Quirk::NestedFunction => write!(f, "function written inside a block"),
            Quirk::DuplicateFunction(name) => {
                write!(f, "function `{name}` defined again; the later one is used")
            }
            Quirk::Gs1Command(name) => write!(f, "GS1 command line `{name}` in a GS2 script"),
            Quirk::Gs1MessageCode => write!(f, "GS1 message code used as a GS2 value"),
            Quirk::Gs1Range => write!(f, "GS1 range `in <a, b>`; write `in |a, b|`"),
            Quirk::GlobalEnum => write!(f, "`global enum`"),
            Quirk::EnumText => write!(f, "`enum` member with a text value"),
            Quirk::BareExpression => write!(f, "value used as a statement"),
            Quirk::RuntimeNameAssign => {
                write!(f, "assignment to a joined name; write `makevar(…) = …`")
            }
            Quirk::CallOfCallResult => write!(f, "call of a call's result"),
            Quirk::SlashWord(word) => write!(f, "unquoted argument `{word}`"),
            Quirk::GuessedLanguage(Front::Gs1) => write!(f, "language not given; read as GS1"),
            Quirk::GuessedLanguage(Front::Gs2) => write!(f, "language not given; read as GS2"),
            Quirk::LaterGs1(era) => {
                write!(
                    f,
                    "GS1 newer than the script's version ({era:?}); read as GS1 2.1"
                )
            }
        }
    }
}

/// A quirk legacy mode accepted. `line` is 0 when it has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub line: u32,
    pub quirk: Quirk,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.quirk)
        } else {
            write!(f, "line {}: {}", self.line, self.quirk)
        }
    }
}

/// One problem found while compiling. `line` is 0 when it has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub line: u32,
    pub message: String,
}

/// Every problem from one compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileErrors {
    pub errors: Vec<CompileError>,
}

impl fmt::Display for CompileErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.errors.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            if e.line == 0 {
                write!(f, "{}", e.message)?;
            } else {
                write!(f, "line {}: {}", e.line, e.message)?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for CompileErrors {}

impl From<CompileError> for CompileErrors {
    fn from(e: CompileError) -> Self {
        CompileErrors { errors: vec![e] }
    }
}

/// A compiled script.
#[derive(Debug, Clone)]
pub struct Compiled {
    pub script: Script,
    pub front: Front,
    /// The quirks legacy mode accepted, in source order. Always empty in
    /// strict mode.
    pub warnings: Vec<Warning>,
}

/// Compile a script.
pub fn compile(src: &str, language: Language, mode: Mode) -> Result<Compiled, CompileErrors> {
    match language {
        Language::Gs1(era) => gs1_script(src, era, mode),
        Language::Gs2 => gs2_script(src, mode),
        Language::Detect(era) => detect(src, era, mode),
    }
}

/// Compile strict GS1.
pub fn compile_gs1(src: &str, era: Era) -> Result<Script, CompileErrors> {
    compile(src, Language::Gs1(era), Mode::Strict).map(|c| c.script)
}

/// Compile strict GS2.
pub fn compile_gs2(src: &str) -> Result<Script, CompileErrors> {
    compile(src, Language::Gs2, Mode::Strict).map(|c| c.script)
}

/// Compile GS2 in legacy mode.
pub fn compile_gs2_legacy(src: &str) -> Result<Compiled, CompileErrors> {
    compile(src, Language::Gs2, Mode::Legacy)
}

/// The GS1 grammar is the GS1 engine's, and a script it reports a problem
/// in is refused in both modes. The one GS1 quirk is a script that uses
/// GS1 newer than its version: legacy mode reads it as GS1 2.1.
fn gs1_script(src: &str, era: Era, mode: Mode) -> Result<Compiled, CompileErrors> {
    let compiled = |era| -> Result<Compiled, CompileErrors> {
        let program = gs1::program(src, era)?;
        Ok(Compiled {
            script: codegen::generate(&program, lexer::Spellings::default())?,
            front: Front::Gs1,
            warnings: Vec::new(),
        })
    };
    match compiled(era) {
        Err(e) if mode == Mode::Legacy && era < Era::V217 => {
            let mut c = compiled(Era::V217).map_err(|_| e)?;
            c.warnings.push(Warning {
                line: 0,
                quirk: Quirk::LaterGs1(era),
            });
            Ok(c)
        }
        r => r,
    }
}

fn gs2_script(src: &str, mode: Mode) -> Result<Compiled, CompileErrors> {
    let parser::Parsed {
        mut program,
        spellings,
        mut quirks,
    } = parser::parse(src)?;
    quirks.extend(
        codegen::replaced_functions(&program)
            .into_iter()
            .map(|name| (0, Quirk::DuplicateFunction(name))),
    );
    let warnings: Vec<Warning> = quirks
        .into_iter()
        .map(|(line, quirk)| Warning { line, quirk })
        .collect();
    if mode == Mode::Strict {
        if !warnings.is_empty() {
            return Err(CompileErrors {
                errors: warnings
                    .into_iter()
                    .map(|w| CompileError {
                        line: w.line,
                        message: format!("{} (accepted only in legacy mode)", w.quirk),
                    })
                    .collect(),
            });
        }
        program.later_wins = false;
    }
    Ok(Compiled {
        script: codegen::generate(&program, spellings)?,
        front: Front::Gs2,
        warnings,
    })
}

fn detect(src: &str, era: Era, mode: Mode) -> Result<Compiled, CompileErrors> {
    if mode == Mode::Strict {
        return Err(CompileError {
            line: 0,
            message: "strict mode needs the script's language (GS1 or GS2)".into(),
        }
        .into());
    }
    let guessed = |mut c: Compiled| {
        c.warnings.insert(
            0,
            Warning {
                line: 0,
                quirk: Quirk::GuessedLanguage(c.front),
            },
        );
        c
    };
    // GS1 at the given version first, then GS2; GS1 newer than the
    // version only once GS2 has failed too.
    if looks_gs1(src) {
        if let Ok(c) = gs1_script(src, era, Mode::Strict) {
            return Ok(guessed(c));
        }
    }
    match gs2_script(src, Mode::Legacy) {
        Ok(c) => Ok(guessed(c)),
        Err(e) => gs1_script(src, era, mode).map(guessed).map_err(|_| e),
    }
}

/// No `function` keyword, and an `if (` whose condition starts with a GS1
/// event name.
fn looks_gs1(src: &str) -> bool {
    let lower = src.to_ascii_lowercase();
    let has_function = lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|w| w == "function");
    if has_function {
        return false;
    }
    lower.match_indices("if").any(|(i, _)| {
        let rest = lower[i + 2..].trim_start();
        let Some(rest) = rest.strip_prefix('(') else {
            return false;
        };
        let word: String = rest
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        vocab::EVENTS.iter().any(|(e, _)| *e == word)
    })
}
