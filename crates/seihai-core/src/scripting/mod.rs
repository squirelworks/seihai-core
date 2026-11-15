//! Scripting support.
//!
//! `bytecode` is the OpenGS1-style reimplementation path.
//! `gs1` is the cleaned 2.05 command table (parse-tree interpreter,
//! opcode = 600 + index).
//! `engine` parses and runs GS1 source using Seihai's own nodes; the native
//! ID/name table is metadata, not its dispatch table or a GS2 opcode table.
//! `gs2` reads and writes the bytecode of the GS2 compiler; `gs2::exec` runs
//! the part that GS1 shares. `gs2::compile` compiles GS1 and GS2 to it.

mod bytecode;
pub mod delivery;
pub mod engine;
mod gs1;
pub mod gs2;

pub use bytecode::{Bytecode, Commands, Opcode, PackedValue, Value};
pub use gs1::{
    has_no_executor, name_for_opcode, opcode_for_name, DIR_DOWN, DIR_LEFT, DIR_RIGHT, DIR_UP,
    GS1_NO_EXECUTOR, GS1_OPCODE_BASE,
};
