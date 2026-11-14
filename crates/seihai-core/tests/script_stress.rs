//! Parse every unique NPC script in the fixtures and report failures.
//!
//! Run with: cargo test -p seihai-core --test script_stress -- --nocapture
//! Set `STRESS_ALL=1` to list every script with diagnostics, not just the
//! first 40.

use seihai_core::scripting::engine::{parse, Era};
use std::fs;
use std::path::Path;

/// A binary level's NPC record starts `x+32, y+32, image#`, the image empty
/// or a file name. Comment lines (`//#CLIENTSIDE`) are not records.
fn record_start(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    if b.len() < 3 || line.starts_with("//") || !b[..2].iter().all(|c| (0x20..0x7f).contains(c)) {
        return None;
    }
    let hash = 2 + line[2..].find('#')?;
    let image = &line[2..hash];
    let lower = image.to_ascii_lowercase();
    let plain = image
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_-. ".contains(c));
    let pic = [".gif", ".png", ".mng", ".jpg", ".jpeg", ".bmp"]
        .iter()
        .any(|e| lower.ends_with(e));
    (image.is_empty() || plain && pic).then_some(hash + 1)
}

/// A block marked `recovered-from-binary` holds a whole binary level as
/// text, its NPC records not split and their 0xA7 line breaks turned into
/// line breaks. Take the NPC records out, one script each, as the level
/// loader (`assets::level`) reads them; a line `#` ends the NPC section.
fn split_binary(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur: Option<String> = None;
    for line in body.lines() {
        if line == "#" && cur.is_some() {
            break;
        }
        if let Some(at) = record_start(line) {
            out.extend(cur.take());
            cur = Some(format!("{}\n", &line[at..]));
        } else if let Some(c) = cur.as_mut() {
            c.push_str(line);
            c.push('\n');
        }
    }
    out.extend(cur);
    out.retain(|s| !s.trim().is_empty());
    out
}

fn scripts_in(path: &Path) -> Vec<(String, String)> {
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut name = String::new();
    let mut binary = false;
    // Inside an `NPC ...` block: a script line that starts `NPC ` (a sign's
    // text, `NPC made by ...`) is not a new block.
    let mut in_body = false;
    let push = |name: &String, body: &mut String, binary: bool, out: &mut Vec<(String, String)>| {
        if body.is_empty() {
            return;
        }
        if binary {
            for (i, s) in split_binary(body).into_iter().enumerate() {
                out.push((format!("{name}#{i}"), s));
            }
        } else {
            out.push((name.clone(), body.clone()));
        }
        body.clear();
    };
    for line in text.lines() {
        if line.starts_with("// ===== ") {
            push(&name, &mut cur, binary, &mut out);
            name = line
                .trim_start_matches("// ===== ")
                .trim_end_matches(" =====")
                .to_string();
            binary = false;
            in_body = false;
        } else if line == "// recovered-from-binary: true" {
            binary = true;
        } else if line.starts_with("NPC ") && !in_body {
            cur.clear();
            in_body = true;
        } else if line == "NPCEND" {
            push(&name, &mut cur, binary, &mut out);
            in_body = false;
        } else if binary || !line.starts_with("//") {
            // A binary level's records keep their comments: a record's
            // first line is often `//#CLIENTSIDE` after its header.
            cur.push_str(line);
            cur.push('\n');
        }
    }
    push(&name, &mut cur, binary, &mut out);
    out
}

#[test]
fn parse_all_unique_npc_scripts() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/npc-scripts");
    let mut total = 0;
    let mut failed = 0;
    // The same scripts read as a 2.1 or later client reads them.
    let mut failed_21 = 0;
    let mut diags: Vec<(String, String)> = Vec::new();
    // Records split out of binary levels repeat (the same sign NPC many times).
    let mut seen = std::collections::HashSet::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "txt") {
            continue;
        }
        for (name, body) in scripts_in(&path) {
            if name.contains(".txt#") && !seen.insert(body.clone()) {
                continue;
            }
            if std::env::var("STRESS_SHOW").is_ok_and(|n| n.split(',').any(|n| name.ends_with(n))) {
                eprintln!(
                    "==== {name}\n{}",
                    body.lines()
                        .enumerate()
                        .map(|(i, l)| format!("{:4} {l}", i + 1))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
            total += 1;
            let s21 = parse(&body, Era::V217);
            if !s21.diagnostics.is_empty() {
                failed_21 += 1;
                if std::env::var("STRESS_ALL").is_ok() {
                    let d = &s21.diagnostics[0];
                    let text = body.lines().nth(d.line as usize - 1).unwrap_or("").trim();
                    eprintln!(
                        "V21 {:32} | {:.120} [{}] +{}",
                        d.msg,
                        text,
                        name.rsplit('/').next().unwrap_or(""),
                        s21.diagnostics.len() - 1
                    );
                }
            }
            let s = parse(&body, Era::V205);
            if !s.diagnostics.is_empty() {
                failed += 1;
                if std::env::var("STRESS_ALL").is_ok() && s21.diagnostics.is_empty() {
                    let d = &s.diagnostics[0];
                    let text = body.lines().nth(d.line as usize - 1).unwrap_or("").trim();
                    eprintln!(
                        "V205 {:32} | {:.120} [{}] +{}",
                        d.msg,
                        text,
                        name.rsplit('/').next().unwrap_or(""),
                        s.diagnostics.len() - 1
                    );
                }
                if diags.len() < 40 || std::env::var("STRESS_ALL").is_ok() {
                    let msg = s
                        .diagnostics
                        .iter()
                        .map(|d| format!("L{}: {}", d.line, d.msg))
                        .collect::<Vec<_>>()
                        .join("; ");
                    diags.push((name, msg));
                }
            }
        }
    }
    eprintln!(
        "parsed {total} scripts, {failed} with diagnostics as 2.05, {failed_21} as 2.1 and later"
    );
    for (n, m) in &diags {
        eprintln!("  {n}: {m}");
    }
    // Informational: do not fail the build. The count is the point.
    assert!(total > 1000, "expected the full corpus");
}

/// seihai-core#6: how much of the corpus the GS1 to GS2 lowering takes.
/// Of the unique scripts that parse as 2.17, the share that compiles to
/// GS2 bytecode in strict mode. `STRESS_LOWER=1` lists the first error of
/// each script that does not. The floor guards against a regression; the
/// rest are GS1 commands with no GS2 counterpart in the client tables,
/// server-side commands, and dynamic flag names.
#[test]
fn lower_unique_npc_scripts_to_gs2() {
    use seihai_core::scripting::gs2::compile;
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/npc-scripts");
    let (mut total, mut ok) = (0, 0);
    let mut seen = std::collections::HashSet::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "txt") {
            continue;
        }
        for (name, body) in scripts_in(&path) {
            if !seen.insert(body.clone()) || !parse(&body, Era::V217).diagnostics.is_empty() {
                continue;
            }
            total += 1;
            match compile::compile_gs1(&body, Era::V217) {
                Ok(_) => ok += 1,
                Err(e) if std::env::var("STRESS_LOWER").is_ok() => {
                    eprintln!("{name}: {}", e.to_string().lines().next().unwrap_or(""))
                }
                Err(_) => {}
            }
        }
    }
    eprintln!("lowered {ok} of {total} scripts that parse as 2.17");
    assert!(total > 1500, "expected the full corpus");
    assert!(ok >= 1200, "lowering coverage fell to {ok} of {total}");
}
