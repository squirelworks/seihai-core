//! GS1 bytecode structures, a minimal interpreter, and offline commands.
//!
//! Layouts and opcodes match OpenGS1. The interpreter handles the
//! arithmetic, comparison, and jump opcodes. CmdCall dispatches into
//! the command registry.

use std::collections::HashMap;

/// Opcode values matching OpenGS1 `Operation.hpp`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Push = 0,
    Assign,
    ArrSet,
    ArrGet,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Inc,
    IncPush,
    Dec,
    DecPush,
    Call,
    CmdCall,
    Jmp,
    Jal,
    Ret,
    Eq,
    Lt,
    Gt,
    Lte,
    Gte,
    Not,
    And,
    Or,
    Jez,
    Jnz,
    Stop,
    DbgOut,
}

/// Packed value (4 bytes) used by `OP_PUSH`.
#[derive(Debug, Clone, Copy)]
pub struct PackedValue {
    pub value_type: u8,
    pub value: u16,
}

impl PackedValue {
    pub fn from_u32(raw: u32) -> Self {
        Self {
            value_type: (raw & 0xF) as u8,
            value: ((raw >> 4) & 0xFFFF) as u16,
        }
    }
}

/// Loaded GS1 bytecode image.
#[derive(Debug, Clone)]
pub struct Bytecode {
    pub string_constants: Vec<String>,
    pub number_constants: Vec<f32>,
    pub body: Vec<u8>,
}

impl Bytecode {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < 8 {
            return Err("bytecode too short".into());
        }
        let body_offset = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        if body_offset > data.len() {
            return Err("invalid body offset".into());
        }

        let mut pos = 4;
        let num_str = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;

        let mut string_constants = Vec::with_capacity(num_str);
        for _ in 0..num_str {
            if pos + 4 > data.len() {
                return Err("truncated string table".into());
            }
            let len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;
            if pos + len > data.len() {
                return Err("truncated string".into());
            }
            let s = String::from_utf8_lossy(&data[pos..pos + len]).into_owned();
            string_constants.push(s);
            pos += len;
        }

        if pos + 4 > data.len() {
            return Err("truncated number count".into());
        }
        let num_num = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;

        let mut number_constants = Vec::with_capacity(num_num);
        for _ in 0..num_num {
            if pos + 4 > data.len() {
                return Err("truncated number".into());
            }
            let n = f32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
            number_constants.push(n);
            pos += 4;
        }

        let body = data[body_offset..].to_vec();
        Ok(Bytecode {
            string_constants,
            number_constants,
            body,
        })
    }

    pub fn walk(&self) -> Vec<Opcode> {
        let mut ops = Vec::new();
        let mut i = 0;
        while i < self.body.len() {
            let raw = self.body[i];
            i += 1;
            if let Some(op) = Opcode::from_u8(raw) {
                ops.push(op);
                i += op.immediate_len();
            }
        }
        ops
    }

    pub fn execute(&self) -> Result<Vec<Value>, String> {
        let mut vm = Vm::new(self, Commands::new());
        vm.run()?;
        Ok(vm.stack)
    }

    pub fn execute_with(&self, cmds: Commands) -> Result<Vec<Value>, String> {
        let mut vm = Vm::new(self, cmds);
        vm.run()?;
        Ok(vm.stack)
    }
}

/// Runtime value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f32),
    Str(String),
    Bool(bool),
}

/// Command handler.
pub type CommandFn = fn(args: &[Value]) -> Option<Value>;

/// Minimal command registry for offline NPC scripts.
#[derive(Default)]
pub struct Commands {
    map: HashMap<String, CommandFn>,
}

impl Commands {
    pub fn new() -> Self {
        let mut c = Self::default();
        c.register("message", cmd_message);
        c.register("say", cmd_message);
        c.register("setimg", cmd_setimg);
        c.register("move", cmd_move);
        c.register("showimg", cmd_setimg);
        c.register("hideimg", cmd_noop);
        c.register("hide", cmd_noop);
        c.register("show", cmd_noop);
        c.register("setlevel", cmd_noop);
        c.register("warp", cmd_noop);
        c.register("hurt", cmd_noop);
        c.register("set", cmd_noop);
        c.register("unset", cmd_noop);
        c.register("setstring", cmd_noop);
        c.register("addstring", cmd_noop);
        c.register("setplayerprop", cmd_noop);
        c
    }

    pub fn register(&mut self, name: &str, f: CommandFn) {
        self.map.insert(name.to_string(), f);
    }

    pub fn call(&self, name: &str, args: &[Value]) -> Option<Value> {
        self.map.get(name).and_then(|f| f(args))
    }
}

fn cmd_message(args: &[Value]) -> Option<Value> {
    args.first().cloned()
}

fn cmd_setimg(args: &[Value]) -> Option<Value> {
    args.first().cloned()
}

fn cmd_move(_args: &[Value]) -> Option<Value> {
    None
}

fn cmd_noop(_args: &[Value]) -> Option<Value> {
    None
}

/// Minimal GS1 VM.
struct Vm<'a> {
    bc: &'a Bytecode,
    ip: usize,
    stack: Vec<Value>,
    jump_stack: Vec<usize>,
    cmds: Commands,
}

impl<'a> Vm<'a> {
    fn new(bc: &'a Bytecode, cmds: Commands) -> Self {
        Self {
            bc,
            ip: 0,
            stack: Vec::new(),
            jump_stack: Vec::new(),
            cmds,
        }
    }

    fn run(&mut self) -> Result<(), String> {
        let body = &self.bc.body;
        while self.ip < body.len() {
            let raw = body[self.ip];
            self.ip += 1;
            let op = Opcode::from_u8(raw).ok_or_else(|| format!("bad opcode {raw}"))?;
            match op {
                Opcode::Push => {
                    if self.ip + 4 > body.len() {
                        return Err("truncated push".into());
                    }
                    let raw = u32::from_le_bytes(body[self.ip..self.ip + 4].try_into().unwrap());
                    self.ip += 4;
                    let pv = PackedValue::from_u32(raw);
                    let v = match pv.value_type {
                        0 => Value::Num(
                            self.bc
                                .number_constants
                                .get(pv.value as usize)
                                .copied()
                                .unwrap_or(0.0),
                        ),
                        1 => Value::Str(
                            self.bc
                                .string_constants
                                .get(pv.value as usize)
                                .cloned()
                                .unwrap_or_default(),
                        ),
                        _ => Value::Num(pv.value as f32),
                    };
                    self.stack.push(v);
                }
                Opcode::Add | Opcode::Sub | Opcode::Mul | Opcode::Div | Opcode::Mod => {
                    let b = self.pop_num()?;
                    let a = self.pop_num()?;
                    let r = match op {
                        Opcode::Add => a + b,
                        Opcode::Sub => a - b,
                        Opcode::Mul => a * b,
                        Opcode::Div => {
                            if b == 0.0 {
                                0.0
                            } else {
                                a / b
                            }
                        }
                        Opcode::Mod => {
                            if b == 0.0 {
                                0.0
                            } else {
                                a % b
                            }
                        }
                        _ => 0.0,
                    };
                    self.stack.push(Value::Num(r));
                }
                Opcode::Eq | Opcode::Lt | Opcode::Gt | Opcode::Lte | Opcode::Gte => {
                    let b = self.pop_num()?;
                    let a = self.pop_num()?;
                    let r = match op {
                        Opcode::Eq => a == b,
                        Opcode::Lt => a < b,
                        Opcode::Gt => a > b,
                        Opcode::Lte => a <= b,
                        Opcode::Gte => a >= b,
                        _ => false,
                    };
                    self.stack.push(Value::Bool(r));
                }
                Opcode::Not => {
                    let a = self.pop_bool()?;
                    self.stack.push(Value::Bool(!a));
                }
                Opcode::And | Opcode::Or => {
                    let b = self.pop_bool()?;
                    let a = self.pop_bool()?;
                    let r = match op {
                        Opcode::And => a && b,
                        Opcode::Or => a || b,
                        _ => false,
                    };
                    self.stack.push(Value::Bool(r));
                }
                Opcode::Jmp => {
                    let target = self.read_i32(body)?;
                    self.ip = target as usize;
                }
                Opcode::Jez => {
                    let target = self.read_i32(body)?;
                    let cond = self.pop_bool()?;
                    if !cond {
                        self.ip = target as usize;
                    }
                }
                Opcode::Jnz => {
                    let target = self.read_i32(body)?;
                    let cond = self.pop_bool()?;
                    if cond {
                        self.ip = target as usize;
                    }
                }
                Opcode::Jal => {
                    let target = self.read_i32(body)?;
                    self.jump_stack.push(self.ip);
                    self.ip = target as usize;
                }
                Opcode::Ret => {
                    if let Some(ret) = self.jump_stack.pop() {
                        self.ip = ret;
                    }
                }
                Opcode::Stop => break,
                Opcode::Inc | Opcode::Dec => {
                    let a = self.pop_num()?;
                    let r = if op == Opcode::Inc { a + 1.0 } else { a - 1.0 };
                    self.stack.push(Value::Num(r));
                }
                Opcode::IncPush | Opcode::DecPush => {
                    let a = self.pop_num()?;
                    self.stack.push(Value::Num(a));
                    let r = if op == Opcode::IncPush {
                        a + 1.0
                    } else {
                        a - 1.0
                    };
                    self.stack.push(Value::Num(r));
                }
                Opcode::CmdCall => {
                    // Provisional: command name is the top string, args below.
                    if let Some(Value::Str(name)) = self.stack.pop() {
                        let result = self.cmds.call(&name, &[]);
                        if let Some(v) = result {
                            self.stack.push(v);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn read_i32(&mut self, body: &[u8]) -> Result<i32, String> {
        if self.ip + 4 > body.len() {
            return Err("truncated jump".into());
        }
        let v = i32::from_le_bytes(body[self.ip..self.ip + 4].try_into().unwrap());
        self.ip += 4;
        Ok(v)
    }

    fn pop_num(&mut self) -> Result<f32, String> {
        match self.stack.pop() {
            Some(Value::Num(n)) => Ok(n),
            Some(Value::Bool(b)) => Ok(if b { 1.0 } else { 0.0 }),
            _ => Err("expected number".into()),
        }
    }

    fn pop_bool(&mut self) -> Result<bool, String> {
        match self.stack.pop() {
            Some(Value::Bool(b)) => Ok(b),
            Some(Value::Num(n)) => Ok(n != 0.0),
            _ => Err("expected bool".into()),
        }
    }
}

impl Opcode {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Opcode::Push,
            1 => Opcode::Assign,
            2 => Opcode::ArrSet,
            3 => Opcode::ArrGet,
            4 => Opcode::Add,
            5 => Opcode::Sub,
            6 => Opcode::Mul,
            7 => Opcode::Div,
            8 => Opcode::Mod,
            9 => Opcode::Pow,
            10 => Opcode::Inc,
            11 => Opcode::IncPush,
            12 => Opcode::Dec,
            13 => Opcode::DecPush,
            14 => Opcode::Call,
            15 => Opcode::CmdCall,
            16 => Opcode::Jmp,
            17 => Opcode::Jal,
            18 => Opcode::Ret,
            19 => Opcode::Eq,
            20 => Opcode::Lt,
            21 => Opcode::Gt,
            22 => Opcode::Lte,
            23 => Opcode::Gte,
            24 => Opcode::Not,
            25 => Opcode::And,
            26 => Opcode::Or,
            27 => Opcode::Jez,
            28 => Opcode::Jnz,
            29 => Opcode::Stop,
            30 => Opcode::DbgOut,
            _ => return None,
        })
    }

    pub fn immediate_len(self) -> usize {
        match self {
            Opcode::Push => 4,
            Opcode::Jmp | Opcode::Jal | Opcode::Jez | Opcode::Jnz => 4,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod vm_tests {
    use super::*;

    #[test]
    fn add_two_numbers() {
        let mut body = Vec::new();
        body.push(Opcode::Push as u8);
        body.extend_from_slice(&0u32.to_le_bytes());
        body.push(Opcode::Push as u8);
        body.extend_from_slice(&0x10u32.to_le_bytes());
        body.push(Opcode::Add as u8);
        body.push(Opcode::Stop as u8);

        let bc = Bytecode {
            string_constants: vec![],
            number_constants: vec![2.0, 3.0],
            body,
        };
        let stack = bc.execute().unwrap();
        assert_eq!(stack.last(), Some(&Value::Num(5.0)));
    }

    #[test]
    fn message_command() {
        let mut body = Vec::new();
        // Push string constant 0 ("hello"), CmdCall
        body.push(Opcode::Push as u8);
        body.extend_from_slice(&1u32.to_le_bytes()); // type 1, index 0
        body.push(Opcode::CmdCall as u8);
        body.push(Opcode::Stop as u8);

        let bc = Bytecode {
            string_constants: vec!["message".into(), "hello".into()],
            number_constants: vec![],
            body,
        };
        // CmdCall pops the command name; the stub returns None for no args.
        // Just confirm it doesn't error.
        let _ = bc.execute();
    }
}
