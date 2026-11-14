//! `gs1c`: compile GS1 client scripts with Seihai's clean-room compiler and
//! show what each client build would be sent (seihai-core#6).
//!
//! ```text
//! cargo run --example gs1c -- [--era 2.17] [--out DIR] [--packet ID] FILE.txt...
//! ```
//!
//! For each file it prints the disassembly and the per-client delivery
//! table, and writes `FILE.gs2bc` (the raw bytecode) next to the input or
//! into `--out`. With `--packet ID` it also writes `FILE.plo131.bin`: the
//! `PLO_RAWDATA` + `PLO_NPCBYTECODE` pair for NPC `ID`, the way GServer-v2 sends a level NPC's bytecode, so a lab proxy can
//! splice it into a stream unchanged.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use seihai_core::protocol::ClientVersion;
use seihai_core::scripting::delivery::{Delivery, Prepared};
use seihai_core::scripting::engine::Era;
use seihai_core::scripting::gs2::compile;

fn era_from_label(label: &str) -> Option<Era> {
    Era::ALL.iter().copied().find(|e| e.label() == label)
}

/// Graal's GINT3: three bytes, seven bits each, plus 32, high first.
fn gint3(v: u32) -> [u8; 3] {
    [
        ((v >> 14) & 0x7f) as u8 + 32,
        ((v >> 7) & 0x7f) as u8 + 32,
        (v & 0x7f) as u8 + 32,
    ]
}

fn bytecode_packet(npc: u32, code: &[u8]) -> Vec<u8> {
    let mut body = vec![131 + 32];
    body.extend(gint3(npc));
    // The bytecode already ends in the compiler's `\n`.
    body.extend(code);
    let mut out = vec![100 + 32];
    out.extend(gint3(body.len() as u32));
    out.push(b'\n');
    out.extend(body);
    out
}

fn main() -> ExitCode {
    let mut era = Era::V217;
    let mut out_dir: Option<PathBuf> = None;
    let mut packet: Option<u32> = None;
    let mut files = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--era" => {
                let l = args.next().unwrap_or_default();
                match era_from_label(&l) {
                    Some(e) => era = e,
                    None => {
                        eprintln!("unknown era {l:?}");
                        return ExitCode::from(2);
                    }
                }
            }
            "--out" => out_dir = args.next().map(PathBuf::from),
            "--packet" => packet = args.next().and_then(|s| s.parse().ok()),
            _ => files.push(PathBuf::from(a)),
        }
    }
    if files.is_empty() {
        eprintln!("usage: gs1c [--era 2.17] [--out DIR] [--packet NPCID] FILE...");
        return ExitCode::from(2);
    }
    let mut failed = false;
    for f in &files {
        let src = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: {e}", f.display());
                failed = true;
                continue;
            }
        };
        println!("== {} (GS1 era {})", f.display(), era.label());
        match compile::compile_gs1(&src, era) {
            Ok(script) => {
                println!("{}", script.disassemble());
                let code = script.write();
                let stem = f.file_stem().unwrap_or_default().to_string_lossy();
                let dir = out_dir
                    .clone()
                    .unwrap_or_else(|| f.parent().unwrap_or(Path::new(".")).to_path_buf());
                let bc = dir.join(format!("{stem}.gs2bc"));
                if let Err(e) = std::fs::write(&bc, &code) {
                    eprintln!("{}: {e}", bc.display());
                    failed = true;
                }
                println!("wrote {} ({} bytes)", bc.display(), code.len());
                if let Some(id) = packet {
                    let p = dir.join(format!("{stem}.plo131.bin"));
                    if let Err(e) = std::fs::write(&p, bytecode_packet(id, &code)) {
                        eprintln!("{}: {e}", p.display());
                        failed = true;
                    }
                    println!("wrote {}", p.display());
                }
            }
            Err(e) => {
                println!("does not compile:\n{e}");
                failed = true;
            }
        }
        let prepared = Prepared::gs1(&src, era);
        println!("delivery:");
        for v in ClientVersion::all_versions() {
            let c = prepared.for_client(*v);
            let what = match c.delivery {
                Delivery::Gs1Text(_) => "GS1 text".to_string(),
                Delivery::Gs2Bytecode(b) => format!("GS2 bytecode, {} bytes", b.len()),
                Delivery::Stub(why) => format!("stub ({why:?})"),
            };
            let mark = if c.confirmed { "" } else { " (unconfirmed)" };
            println!("  {:<14} {what}{mark}", v.label());
        }
        println!();
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn gint3_round_trip() {
        assert_eq!(super::gint3(5), [32, 32, 37]);
    }
}
