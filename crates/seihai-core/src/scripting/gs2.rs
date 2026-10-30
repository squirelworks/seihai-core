//! GS2 bytecode reader (Stage B of the GS2 plan).
//!
//! Reads the files the open-source gs2-parser compiler writes into segments,
//! a string table and a list of instructions (`exec` runs them). The layout,
//! the operand markers and the opcode numbers are in
//! `documentation/gs2-opcodes.md`; `Op::doc_name` gives the name used there.
//! An opcode value the table does not name is kept as `Op::Unknown`, so a
//! later executor can log it and skip it.

use std::fmt::{self, Write as _};

pub mod exec;

macro_rules! ops {
    ($($v:literal $name:ident $doc:literal,)*) => {
        /// A GS2 opcode. The numbers are the compiler's.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Op {
            $($name,)*
            /// A value the opcode table does not name, or a placeholder in it.
            Unknown(u8),
        }

        impl Op {
            pub fn from_u8(v: u8) -> Op {
                match v {
                    $($v => Op::$name,)*
                    other => Op::Unknown(other),
                }
            }

            pub fn value(self) -> u8 {
                match self {
                    $(Op::$name => $v,)*
                    Op::Unknown(v) => v,
                }
            }

            /// The name in `documentation/gs2-opcodes.md` (the compiler's own).
            pub fn doc_name(self) -> Option<&'static str> {
                match self {
                    $(Op::$name => Some($doc),)*
                    Op::Unknown(_) => None,
                }
            }
        }
    };
}

ops! {
    0 NoOp "OP_NONE",
    1 SetIndex "OP_SET_INDEX",
    2 SetIndexTrue "OP_SET_INDEX_TRUE",
    3 Or "OP_OR",
    4 If "OP_IF",
    5 And "OP_AND",
    6 Call "OP_CALL",
    7 Ret "OP_RET",
    8 Sleep "OP_SLEEP",
    9 CmdCall "OP_CMD_CALL",
    10 Jmp "OP_JMP",
    11 WaitFor "OP_WAITFOR",
    20 TypeNumber "OP_TYPE_NUMBER",
    21 TypeString "OP_TYPE_STRING",
    22 TypeVar "OP_TYPE_VAR",
    23 TypeArray "OP_TYPE_ARRAY",
    24 TypeTrue "OP_TYPE_TRUE",
    25 TypeFalse "OP_TYPE_FALSE",
    26 TypeNull "OP_TYPE_NULL",
    27 Pi "OP_PI",
    30 CopyLastOp "OP_COPY_LAST_OP",
    31 SwapLastOps "OP_SWAP_LAST_OPS",
    32 IndexDec "OP_INDEX_DEC",
    33 ConvToFloat "OP_CONV_TO_FLOAT",
    34 ConvToString "OP_CONV_TO_STRING",
    35 MemberAccess "OP_MEMBER_ACCESS",
    36 ConvToObject "OP_CONV_TO_OBJECT",
    37 ArrayEnd "OP_ARRAY_END",
    38 ArrayNew "OP_ARRAY_NEW",
    39 SetArray "OP_SETARRAY",
    40 InlineNew "OP_INLINE_NEW",
    41 MakeVar "OP_MAKEVAR",
    42 NewObject "OP_NEW_OBJECT",
    43 ObjFromStr "OP_OBJ_FROM_STR",
    44 InlineConditional "OP_INLINE_CONDITIONAL",
    50 Assign "OP_ASSIGN",
    51 FuncParamsEnd "OP_FUNC_PARAMS_END",
    52 Inc "OP_INC",
    53 Dec "OP_DEC",
    60 Add "OP_ADD",
    61 Sub "OP_SUB",
    62 Mul "OP_MUL",
    63 Div "OP_DIV",
    64 Mod "OP_MOD",
    65 Pow "OP_POW",
    68 Not "OP_NOT",
    69 UnarySub "OP_UNARYSUB",
    70 Eq "OP_EQ",
    71 Neq "OP_NEQ",
    72 Lt "OP_LT",
    73 Gt "OP_GT",
    74 Lte "OP_LTE",
    75 Gte "OP_GTE",
    76 BitOr "OP_BWO",
    77 BitAnd "OP_BWA",
    78 BitXor "OP_BWX",
    79 BitInvert "OP_BWI",
    80 InRange "OP_IN_RANGE",
    81 InObj "OP_IN_OBJ",
    82 ObjIndex "OP_OBJ_INDEX",
    83 ObjType "OP_OBJ_TYPE",
    84 Format "OP_FORMAT",
    85 Int "OP_INT",
    86 Abs "OP_ABS",
    87 Random "OP_RANDOM",
    88 Sin "OP_SIN",
    89 Cos "OP_COS",
    90 ArcTan "OP_ARCTAN",
    91 Exp "OP_EXP",
    92 Log "OP_LOG",
    93 Min "OP_MIN",
    94 Max "OP_MAX",
    95 GetAngle "OP_GETANGLE",
    96 GetDir "OP_GETDIR",
    97 VecX "OP_VECX",
    98 VecY "OP_VECY",
    99 ObjIndices "OP_OBJ_INDICES",
    100 ObjLink "OP_OBJ_LINK",
    101 ShiftLeft "OP_BW_LEFTSHIFT",
    102 ShiftRight "OP_BW_RIGHTSHIFT",
    103 Char "OP_CHAR",
    104 ObjCompare "OP_OBJ_COMPARE",
    110 ObjTrim "OP_OBJ_TRIM",
    111 ObjLength "OP_OBJ_LENGTH",
    112 ObjPos "OP_OBJ_POS",
    113 Join "OP_JOIN",
    114 ObjCharAt "OP_OBJ_CHARAT",
    115 ObjSubstr "OP_OBJ_SUBSTR",
    116 ObjStarts "OP_OBJ_STARTS",
    117 ObjEnds "OP_OBJ_ENDS",
    118 ObjTokenize "OP_OBJ_TOKENIZE",
    119 Translate "OP_TRANSLATE",
    120 ObjPositions "OP_OBJ_POSITIONS",
    130 ObjSize "OP_OBJ_SIZE",
    131 Array "OP_ARRAY",
    132 ArrayAssign "OP_ARRAY_ASSIGN",
    133 ArrayMultidim "OP_ARRAY_MULTIDIM",
    134 ArrayMultidimAssign "OP_ARRAY_MULTIDIM_ASSIGN",
    135 ObjSubarray "OP_OBJ_SUBARRAY",
    136 ObjAddString "OP_OBJ_ADDSTRING",
    137 ObjDeleteString "OP_OBJ_DELETESTRING",
    138 ObjRemoveString "OP_OBJ_REMOVESTRING",
    139 ObjReplaceString "OP_OBJ_REPLACESTRING",
    140 ObjInsertString "OP_OBJ_INSERTSTRING",
    141 ObjClear "OP_OBJ_CLEAR",
    142 ArrayNewMultidim "OP_ARRAY_NEW_MULTIDIM",
    150 With "OP_WITH",
    151 WithEnd "OP_WITHEND",
    163 ForEach "OP_FOREACH",
    180 This "OP_THIS",
    181 ThisO "OP_THISO",
    182 Player "OP_PLAYER",
    183 PlayerO "OP_PLAYERO",
    184 Level "OP_LEVEL",
    189 Temp "OP_TEMP",
    190 Params "OP_PARAMS",
}

impl Op {
    /// The opcodes whose operand is an op index to jump to.
    pub fn jumps(self) -> bool {
        matches!(self, Op::SetIndex | Op::SetIndexTrue | Op::Or | Op::If | Op::And | Op::With | Op::ForEach)
    }
}

/// The value after an opcode, from its marker byte (`F0`–`F6`).
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    None,
    /// `F0`–`F2`: an index into the string table.
    Str(u32),
    /// `F3`–`F5`: a signed integer, or the op index of a jump target.
    Int(i32),
    /// `F6`: a decimal written as text. The text is kept as written.
    Decimal(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Instr {
    pub op: Op,
    pub operand: Operand,
    /// Offset of the opcode byte in the bytecode segment.
    pub offset: usize,
}

impl Instr {
    /// The op index this instruction jumps to, for the jumping opcodes.
    pub fn jump_target(&self) -> Option<usize> {
        match (self.op.jumps(), &self.operand) {
            (true, Operand::Int(t)) if *t >= 0 => Some(*t as usize),
            _ => None,
        }
    }

    /// The number this instruction pushes, for `OP_TYPE_NUMBER`.
    pub fn number(&self) -> Option<f64> {
        match (self.op, &self.operand) {
            (Op::TypeNumber, Operand::Int(v)) => Some(*v as f64),
            (Op::TypeNumber, Operand::Decimal(t)) => t.parse().ok(),
            _ => None,
        }
    }
}

/// An entry of the function table: the name as written (`public.report`,
/// `obj.f`) and the op index where it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub start: usize,
}

const SEGMENT_GS1_FLAGS: u32 = 1;
const SEGMENT_FUNCTIONS: u32 = 2;
const SEGMENT_STRINGS: u32 = 3;
const SEGMENT_CODE: u32 = 4;

/// One compiled GS2 script. `code` is indexed by op index, which is what
/// jump targets and `Function::start` count.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Script {
    pub gs1_flags: u32,
    pub functions: Vec<Function>,
    pub strings: Vec<String>,
    pub code: Vec<Instr>,
}

fn be32(b: &[u8], at: usize) -> Result<u32, String> {
    b.get(at..at + 4).map(|s| u32::from_be_bytes(s.try_into().unwrap())).ok_or_else(|| format!("truncated at byte {at}"))
}

/// Strings are bytes; each byte is one character, as in the level reader.
fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

fn nul_terminated(b: &[u8], at: usize) -> Result<(String, usize), String> {
    let len = b.get(at..).and_then(|s| s.iter().position(|&c| c == 0)).ok_or_else(|| format!("unterminated string at byte {at}"))?;
    Ok((latin1(&b[at..at + len]), at + len + 1))
}

impl Script {
    /// Reads a whole file. Segments may come in any order. A segment type
    /// other than 1 to 4 is skipped. Bytes after the last whole segment
    /// (the compiler writes one `0x0A`) are ignored.
    pub fn parse(data: &[u8]) -> Result<Script, String> {
        let mut script = Script::default();
        let mut code = None;
        let mut p = 0;
        while p + 8 <= data.len() {
            let ty = be32(data, p)?;
            let len = be32(data, p + 4)? as usize;
            let body = data.get(p + 8..p + 8 + len).ok_or_else(|| format!("segment {ty} at byte {p} runs past the end"))?;
            match ty {
                SEGMENT_GS1_FLAGS => script.gs1_flags = be32(body, 0)?,
                SEGMENT_FUNCTIONS => script.functions = read_functions(body)?,
                SEGMENT_STRINGS => {
                    let mut at = 0;
                    while at < body.len() {
                        let (s, next) = nul_terminated(body, at)?;
                        script.strings.push(s);
                        at = next;
                    }
                }
                SEGMENT_CODE => code = Some(body),
                _ => {}
            }
            p += 8 + len;
        }
        script.code = read_code(code.ok_or("no bytecode segment")?)?;
        Ok(script)
    }

    /// The string-table entry an instruction refers to.
    pub fn string(&self, instr: &Instr) -> Option<&str> {
        match instr.operand {
            Operand::Str(i) => self.strings.get(i as usize).map(String::as_str),
            _ => None,
        }
    }

    /// The function that starts at `op_index`, if any.
    pub fn function_at(&self, op_index: usize) -> Option<&Function> {
        self.functions.iter().find(|f| f.start == op_index)
    }

    /// One line per instruction: op index, name, operand. Strings are shown
    /// with their table text. This is the form the logs use.
    pub fn disassemble(&self) -> String {
        let mut out = String::new();
        for (i, ins) in self.code.iter().enumerate() {
            if let Some(f) = self.function_at(i) {
                let _ = writeln!(out, "{}:", f.name);
            }
            let _ = write!(out, "{i:5}  {}", ins.op);
            match &ins.operand {
                Operand::None => {}
                Operand::Str(n) => {
                    let _ = write!(out, " #{n} {:?}", self.strings.get(*n as usize).map(String::as_str).unwrap_or("?"));
                }
                Operand::Int(v) if ins.op.jumps() => {
                    let _ = write!(out, " -> {v}");
                }
                Operand::Int(v) => {
                    let _ = write!(out, " {v}");
                }
                Operand::Decimal(t) => {
                    let _ = write!(out, " {t}");
                }
            }
            out.push('\n');
        }
        out
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.doc_name() {
            Some(n) => f.write_str(n),
            None => write!(f, "OP_{}", self.value()),
        }
    }
}

fn read_functions(body: &[u8]) -> Result<Vec<Function>, String> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < body.len() {
        let start = be32(body, at)? as usize;
        let (name, next) = nul_terminated(body, at + 4)?;
        out.push(Function { name, start });
        at = next;
    }
    Ok(out)
}

/// The operand rules of `gs2-opcodes.md`: any byte from `F0` up after an
/// opcode is an operand marker, since no opcode is that high.
fn read_code(c: &[u8]) -> Result<Vec<Instr>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let offset = i;
        let op = Op::from_u8(c[i]);
        if c[i] >= 0xF0 {
            return Err(format!("marker {:#x} at byte {offset} where an opcode was expected", c[i]));
        }
        i += 1;
        let short = |n: usize| format!("{op} at byte {offset}: operand needs {n} bytes");
        let operand = match c.get(i).copied() {
            Some(m @ 0xF0..=0xF6) => {
                i += 1;
                let (operand, len) = match m {
                    0xF0 => (Operand::Str(*c.get(i).ok_or_else(|| short(1))? as u32), 1),
                    0xF1 => (Operand::Str(u16::from_be_bytes(c.get(i..i + 2).ok_or_else(|| short(2))?.try_into().unwrap()) as u32), 2),
                    0xF2 => (Operand::Str(be32(c, i).map_err(|_| short(4))?), 4),
                    0xF3 => (Operand::Int(*c.get(i).ok_or_else(|| short(1))? as i8 as i32), 1),
                    0xF4 => (Operand::Int(i16::from_be_bytes(c.get(i..i + 2).ok_or_else(|| short(2))?.try_into().unwrap()) as i32), 2),
                    0xF5 => (Operand::Int(be32(c, i).map_err(|_| short(4))? as i32), 4),
                    _ => {
                        let (text, next) = nul_terminated(c, i)?;
                        (Operand::Decimal(text), next - i)
                    }
                };
                i += len;
                operand
            }
            Some(m @ 0xF7..) => return Err(format!("{op} at byte {offset}: unknown operand marker {m:#x}")),
            _ => Operand::None,
        };
        out.push(Instr { op, operand, offset });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_value_round_trips() {
        for v in 0..0xF0u8 {
            assert_eq!(Op::from_u8(v).value(), v);
        }
        assert_eq!(Op::from_u8(45), Op::Unknown(45));
    }

    #[test]
    fn operands_by_marker() {
        // OP_TYPE_NUMBER -2 (F3), 300 (F4), 100000 (F5), "0.25" (F6);
        // OP_TYPE_STRING #1 (F0), #300 (F1); OP_IF -> 7 (F4); OP_RET.
        let code = [
            20, 0xF3, 0xFE, 20, 0xF4, 0x01, 0x2C, 20, 0xF5, 0, 1, 0x86, 0xA0, 20, 0xF6, b'0', b'.', b'2', b'5', 0,
            21, 0xF0, 1, 21, 0xF1, 0x01, 0x2C, 4, 0xF4, 0, 7, 7,
        ];
        let ins = read_code(&code).unwrap();
        let nums: Vec<f64> = ins.iter().filter_map(Instr::number).collect();
        assert_eq!(nums, [-2.0, 300.0, 100000.0, 0.25]);
        assert_eq!(ins[4].operand, Operand::Str(1));
        assert_eq!(ins[5].operand, Operand::Str(300));
        assert_eq!(ins[6].jump_target(), Some(7));
        assert_eq!((ins[7].op, ins[7].offset), (Op::Ret, code.len() - 1));
    }

    #[test]
    fn unknown_opcodes_are_kept() {
        let ins = read_code(&[45, 7]).unwrap();
        assert_eq!(ins[0].op, Op::Unknown(45));
        assert_eq!(ins[0].op.to_string(), "OP_45");
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(read_code(&[20, 0xF4, 0]).is_err(), "short operand");
        assert!(read_code(&[20, 0xF6, b'1']).is_err(), "unterminated decimal");
        assert!(read_code(&[20, 0xF9]).is_err(), "unknown marker");
        assert!(read_code(&[0xF3, 1]).is_err(), "marker in opcode position");
        assert!(Script::parse(&[0, 0, 0, 4, 0, 0, 0, 9, 1]).is_err(), "segment past the end");
        assert!(Script::parse(&[0, 0, 0, 1, 0, 0, 0, 4, 0, 0, 0, 0]).is_err(), "no bytecode");
    }
}
