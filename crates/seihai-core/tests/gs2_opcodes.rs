//! The GS2 fixtures (`tests/fixtures/gs2/*.gs2bc`, output of the open-source
//! gs2-parser compiler) against the opcode table and the operand rules in
//! `documentation/gs2-opcodes.md`. The table is read from the document, so
//! the two cannot drift apart. The walk here is independent of the reader in
//! `seihai_core::scripting::gs2`, and the reader is checked against it.

use seihai_core::scripting::gs2;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operand {
    None,
    Str,
    Num,
    Jump,
}

struct Row {
    name: String,
    operand: Operand,
    emitted: bool,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The rows of the opcode table: `| dec | 0xHH | name | operand | … | emitted |`.
/// Range rows (`45–47`) are placeholders and are skipped.
fn table() -> HashMap<u8, Row> {
    let doc = std::fs::read_to_string(root().join("documentation/gs2-opcodes.md")).unwrap();
    let mut ops = HashMap::new();
    for line in doc.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 8 || !cells[2].starts_with("0x") {
            continue;
        }
        let Ok(dec) = cells[1].parse::<u8>() else { continue };
        let hex = u8::from_str_radix(&cells[2][2..], 16).unwrap();
        assert_eq!(dec, hex, "decimal and hex disagree for {}", cells[3]);
        let operand = match cells[4] {
            "—" => Operand::None,
            "str" => Operand::Str,
            "num" => Operand::Num,
            "jump" => Operand::Jump,
            other => panic!("unknown operand kind {other:?} for {}", cells[3]),
        };
        let emitted = match cells[cells.len() - 2] {
            "yes" => true,
            "no" => false,
            other => panic!("unknown emitted value {other:?} for {}", cells[3]),
        };
        assert!(ops.insert(dec, Row { name: cells[3].to_string(), operand, emitted }).is_none(), "{dec} listed twice");
    }
    ops
}

struct Script {
    functions: Vec<(u32, String)>,
    strings: Vec<String>,
    code: Vec<u8>,
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes(b[..4].try_into().unwrap())
}

fn nul_strings(b: &[u8]) -> Vec<String> {
    b.split_inclusive(|&c| c == 0).map(|s| String::from_utf8(s[..s.len() - 1].to_vec()).unwrap()).collect()
}

/// Four segments (`u32 type`, `u32 length`, body), types 1 to 4 in order,
/// then one `0x0A`.
fn load(name: &str) -> Script {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gs2").join(name);
    let bytes = std::fs::read(&path).unwrap();
    assert!(path.with_extension("gs2").exists(), "{name} has no source next to it");
    let mut segs = Vec::new();
    let mut p = 0;
    while segs.len() < 4 {
        let (ty, len) = (be32(&bytes[p..]), be32(&bytes[p + 4..]) as usize);
        segs.push((ty, bytes[p + 8..p + 8 + len].to_vec()));
        p += 8 + len;
    }
    assert_eq!(&bytes[p..], b"\n", "{name}: one 0x0A after the last segment");
    assert_eq!(segs.iter().map(|s| s.0).collect::<Vec<_>>(), [1, 2, 3, 4], "{name}: segment order");
    assert_eq!(segs[0].1, [0, 0, 0, 0], "{name}: GS1 event flags");
    let mut functions = Vec::new();
    let mut f = &segs[1].1[..];
    while !f.is_empty() {
        let end = 4 + f[4..].iter().position(|&c| c == 0).unwrap();
        functions.push((be32(f), String::from_utf8(f[4..end].to_vec()).unwrap()));
        f = &f[end + 1..];
    }
    Script { functions, strings: nul_strings(&segs[2].1), code: segs[3].1.clone() }
}

/// Walks the bytecode with the table's operand rules. Returns each opcode
/// with its integer operand, if any; panics on anything the table does not
/// allow.
fn walk(name: &str, s: &Script, ops: &HashMap<u8, Row>) -> Vec<(u8, Option<i64>)> {
    let c = &s.code;
    let mut out = Vec::new();
    let mut jumps = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let at = i;
        let op = c[i];
        i += 1;
        let entry = ops.get(&op).unwrap_or_else(|| panic!("{name}: byte {at}: opcode {op} is not in the table"));
        assert!(entry.emitted, "{name}: byte {at}: {} is marked as never emitted", entry.name);
        let marker = c.get(i).copied().filter(|&m| m >= 0xF0);
        let value = match (entry.operand, marker) {
            (Operand::None, None) => None,
            (Operand::None, Some(m)) => panic!("{name}: byte {at}: {} takes no operand, found marker {m:#x}", entry.name),
            (kind, None) => panic!("{name}: byte {at}: {} needs a {kind:?} operand", entry.name),
            (kind, Some(m)) => {
                let allowed = match kind {
                    Operand::Str => (0xF0..=0xF2).contains(&m),
                    Operand::Num => (0xF3..=0xF6).contains(&m),
                    Operand::Jump => (0xF3..=0xF5).contains(&m),
                    Operand::None => unreachable!(),
                };
                assert!(allowed, "{name}: byte {at}: marker {m:#x} is not allowed for {}", entry.name);
                i += 1;
                let v = match m {
                    0xF0 => c[i] as i64,
                    0xF1 => u16::from_be_bytes([c[i], c[i + 1]]) as i64,
                    0xF2 => be32(&c[i..]) as i64,
                    0xF3 => c[i] as i8 as i64,
                    0xF4 => i16::from_be_bytes([c[i], c[i + 1]]) as i64,
                    0xF5 => be32(&c[i..]) as i32 as i64,
                    _ => {
                        let end = i + c[i..].iter().position(|&b| b == 0).unwrap();
                        let text = std::str::from_utf8(&c[i..end]).unwrap();
                        assert!(text.parse::<f64>().is_ok(), "{name}: byte {at}: {text:?} is not a number");
                        i = end + 1;
                        out.push((op, None));
                        continue;
                    }
                };
                i += match m {
                    0xF0 | 0xF3 => 1,
                    0xF1 | 0xF4 => 2,
                    _ => 4,
                };
                if kind == Operand::Str {
                    assert!((v as usize) < s.strings.len(), "{name}: byte {at}: string {v} is past the table");
                }
                if kind == Operand::Jump {
                    jumps.push((at, v));
                }
                Some(v)
            }
        };
        out.push((op, value));
    }
    assert_eq!(i, c.len(), "{name}: the walk ends inside an operand");
    for (at, target) in jumps {
        assert!(target >= 0 && (target as usize) <= out.len(), "{name}: byte {at}: jump to op {target} of {}", out.len());
    }
    for (index, fname) in &s.functions {
        assert_eq!(out[*index as usize].0, 23, "{name}: {fname} does not start with OP_TYPE_ARRAY");
    }
    out
}

const FIXTURES: &[&str] = &["shared.gs2bc", "control.gs2bc", "objects.gs2bc", "calls.gs2bc", "timers.gs2bc", "strings.gs2bc"];

#[test]
fn every_fixture_walks_cleanly_with_the_documented_table() {
    let ops = table();
    assert!(ops.len() > 100, "only {} rows read from gs2-opcodes.md", ops.len());
    for f in FIXTURES {
        let s = load(f);
        let walked = walk(f, &s, &ops);
        // The compiler ends every script with one extra OP_RET.
        assert_eq!(walked.last().unwrap().0, 7, "{f}: last opcode");
    }
}

#[test]
fn shared_fixture_has_the_expected_opcodes() {
    let ops = table();
    let s = load("shared.gs2bc");
    assert_eq!(s.functions, [(1, "onCreated".to_string())]);
    assert_eq!(s.strings, ["count", "gs2seen", "set", "y", "x", "light2.png", "showimg", "hideimg"]);
    let names: Vec<&str> = walk("shared", &s, &ops).iter().map(|(op, _)| ops[op].name.as_str()).collect();
    #[rustfmt::skip]
    let expected = [
        // jump over the function; its entry; parameters; body start
        "OP_SET_INDEX", "OP_TYPE_ARRAY", "OP_FUNC_PARAMS_END", "OP_JMP", "OP_CMD_CALL",
        // this.count = 3 + 4 * 2;
        "OP_THIS", "OP_TYPE_VAR", "OP_MEMBER_ACCESS",
        "OP_TYPE_NUMBER", "OP_TYPE_NUMBER", "OP_TYPE_NUMBER", "OP_MUL", "OP_ADD", "OP_ASSIGN",
        // if (this.count > 10)
        "OP_THIS", "OP_TYPE_VAR", "OP_MEMBER_ACCESS", "OP_CONV_TO_FLOAT", "OP_TYPE_NUMBER", "OP_GT", "OP_IF",
        // set("gs2seen");
        "OP_TYPE_ARRAY", "OP_TYPE_STRING", "OP_TYPE_VAR", "OP_CALL", "OP_INDEX_DEC",
        // showimg(1, "light2.png", x, y);  arguments in reverse
        "OP_TYPE_ARRAY", "OP_TYPE_VAR", "OP_TYPE_VAR", "OP_TYPE_STRING", "OP_TYPE_NUMBER",
        "OP_TYPE_VAR", "OP_CALL", "OP_INDEX_DEC",
        // hideimg(1);
        "OP_TYPE_ARRAY", "OP_TYPE_NUMBER", "OP_TYPE_VAR", "OP_CALL", "OP_INDEX_DEC",
        // sleep(0.5);
        "OP_TYPE_NUMBER", "OP_SLEEP",
        // implicit return 0, then the compiler's extra OP_RET
        "OP_TYPE_NUMBER", "OP_RET", "OP_RET",
    ];
    assert_eq!(names, expected);
}

#[test]
fn jumps_and_numbers_decode_as_documented() {
    let ops = table();
    let s = load("shared.gs2bc");
    let w = walk("shared", &s, &ops);
    assert_eq!(w[0], (1, Some(44)), "the jump over onCreated lands one past the last opcode");
    assert_eq!(w[20], (4, Some(34)), "the if skips to hideimg");
    assert_eq!(w[18], (20, Some(10)));
    assert_eq!(w[39], (20, None), "0.5 is an F6 decimal");

    let s = load("control.gs2bc");
    let w = walk("control", &s, &ops);
    let numbers: Vec<i64> = w.iter().filter(|(op, _)| *op == 20).filter_map(|(_, v)| *v).collect();
    assert!(numbers.contains(&-1), "F3 is signed");
    assert!(numbers.contains(&300) && numbers.contains(&1000), "F4");
    assert!(numbers.contains(&100000), "F5");
    assert!(w.iter().any(|(op, _)| *op == 2), "switch uses OP_SET_INDEX_TRUE");

    let s = load("objects.gs2bc");
    assert_eq!(s.functions.iter().map(|f| f.1.as_str()).collect::<Vec<_>>(), ["public.report", "onCreated"]);
}

// Stage B: the reader in `seihai_core::scripting::gs2` against the same
// table and fixtures.


fn read(name: &str) -> gs2::Script {
    gs2::Script::parse(&std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gs2").join(name)).unwrap()).unwrap()
}

#[test]
fn reader_opcode_names_match_the_documented_table() {
    let ops = table();
    for v in 0..0xF0u8 {
        // Placeholders (`OP_UNKNOWN_54`) are `Op::Unknown` in the reader.
        let doc = ops.get(&v).map(|o| o.name.as_str()).filter(|n| !n.starts_with("OP_UNKNOWN"));
        assert_eq!(gs2::Op::from_u8(v).doc_name(), doc, "opcode {v}");
    }
}

#[test]
fn reader_agrees_with_the_table_walk_on_every_fixture() {
    let ops = table();
    for f in FIXTURES {
        let walked = walk(f, &load(f), &ops);
        let s = read(f);
        assert_eq!(s.code.len(), walked.len(), "{f}: instruction count");
        for (i, (ins, (op, value))) in s.code.iter().zip(&walked).enumerate() {
            assert_eq!(ins.op.value(), *op, "{f}: op {i}");
            let got = match ins.operand {
                gs2::Operand::Str(n) => Some(n as i64),
                gs2::Operand::Int(n) => Some(n as i64),
                gs2::Operand::None | gs2::Operand::Decimal(_) => None,
            };
            assert_eq!(got, *value, "{f}: operand of op {i}");
            if let Some(t) = ins.jump_target() {
                assert!(t <= s.code.len(), "{f}: op {i} jumps to {t}");
            }
        }
        for func in &s.functions {
            assert_eq!(s.code[func.start].op, gs2::Op::TypeArray, "{f}: {}", func.name);
        }
        assert!(s.code.iter().all(|i| !matches!(i.op, gs2::Op::Unknown(_))), "{f}: unknown opcode");
    }
}

#[test]
fn reader_disassembles_the_shared_fixture() {
    let s = read("shared.gs2bc");
    assert_eq!(s.gs1_flags, 0);
    let text = s.disassemble();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "    0  OP_SET_INDEX -> 44");
    assert_eq!(lines[1], "onCreated:");
    assert!(text.contains("   20  OP_IF -> 34\n"));
    assert!(text.contains("   22  OP_TYPE_STRING #1 \"gs2seen\"\n   23  OP_TYPE_VAR #2 \"set\"\n   24  OP_CALL\n"));
    assert!(text.contains("   39  OP_TYPE_NUMBER 0.5\n   40  OP_SLEEP\n"));
    let called: Vec<&str> = s.code.windows(2).filter(|w| w[1].op == gs2::Op::Call).filter_map(|w| s.string(&w[0])).collect();
    assert_eq!(called, ["set", "showimg", "hideimg"]);
}
