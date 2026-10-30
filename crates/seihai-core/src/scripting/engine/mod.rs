//! GS1 script engine.
//!
//! Source text goes through [`parse`] into a [`Script`], which a
//! [`ScriptWorld`] runs against NPC and weapon objects.
//! This is written from observed behaviour and the cleaned
//! Graal_analysis notes, not from any other implementation.

pub mod ast;
pub mod compile;
pub mod host;
pub mod lexer;
pub mod interp;
pub mod parser;
pub mod value;
pub mod vocab;
pub mod world;

#[cfg(test)]
mod tests;

pub use value::Val;
pub use ast::Script;
pub use parser::parse;
pub use vocab::Era;
pub use world::Effect;
