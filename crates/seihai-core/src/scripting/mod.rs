//! Scripting support.
//!
//! `bytecode` is the OpenGS1-style reimplementation path.
//! `gs1` is the cleaned 2.05 command table (parse-tree interpreter,
//! opcode = 600 + index).
//! `gs2` reads the bytecode of the GS2 compiler; it runs nothing yet.

pub mod engine;
mod bytecode;
mod gs1;
pub mod gs2;

pub use bytecode::{Bytecode, Commands, Opcode, PackedValue, Value};
pub use gs1::{
    has_no_executor, name_for_opcode, opcode_for_name, DIR_DOWN, DIR_LEFT, DIR_RIGHT, DIR_UP,
    GS1_NO_EXECUTOR, GS1_OPCODE_BASE,
};
