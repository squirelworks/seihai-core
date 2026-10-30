//! Output formats: text, tsv, json.

use super::tables::{layout_line, Enc};
use super::{LayoutReport, Row, View};
use crate::difftest::trace::esc;
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Tsv,
    Json,
}

impl Format {
    pub fn parse(s: &str) -> Option<Format> {
        match s {
            "text" => Some(Format::Text),
            "tsv" => Some(Format::Tsv),
            "json" => Some(Format::Json),
            _ => None,
        }
    }
}

/// Text safe for a terminal: control characters (including ESC) and
/// non-ASCII bytes become `\xNN`.
pub fn printable(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        let n = c as u32;
        if c == ' ' || (0x21..0x7f).contains(&n) {
            o.push(c);
        } else {
            let _ = write!(o, "\\x{:02x}", n & 0xff);
        }
    }
    o
}

pub fn hex_string(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// 16 bytes per line: offset, hex, ASCII.
pub fn hexdump(b: &[u8]) -> Vec<String> {
    b.chunks(16)
        .enumerate()
        .map(|(i, c)| {
            let hex: Vec<String> = c.iter().map(|x| format!("{x:02x}")).collect();
            let asc: String = c.iter().map(|&x| if (0x20..0x7f).contains(&x) { x as char } else { '.' }).collect();
            format!("{:04x}  {:<47}  |{}|", i * 16, hex.join(" "), asc)
        })
        .collect()
}

fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn header_lines(v: &View, source: &str, tables_note: &str) -> Vec<String> {
    let ver = v.version.map_or("unknown".to_string(), |x| x.label().to_string());
    vec![
        format!("capture {source}  version {ver} ({})  mask {}", v.version_note, if v.masked { "on" } else { "off" }),
        format!("tables {tables_note}"),
        format!(
            "messages shown {} of {}{}",
            v.rows.len(),
            v.total,
            if v.has_login { "  (login frames redacted: password never decoded)" } else { "" }
        ),
    ]
}

pub fn render_view(v: &View, fmt: Format, source: &str, tables_note: &str, show_layout: bool) -> String {
    match fmt {
        Format::Text => view_text(v, source, tables_note, show_layout),
        Format::Tsv => view_tsv(v),
        Format::Json => view_json(v, source, tables_note),
    }
}

fn fields_text(r: &Row) -> String {
    r.fields.iter().map(|(k, val)| format!("{}={}", printable(k), printable(val))).collect::<Vec<_>>().join("  ")
}

fn view_text(v: &View, source: &str, tables_note: &str, show_layout: bool) -> String {
    let mut s = String::new();
    for l in header_lines(v, &printable(source), tables_note) {
        let _ = writeln!(s, "# {l}");
    }
    for r in &v.rows {
        let e = &r.event;
        let _ = writeln!(
            s,
            "{} {} f{} +{}ms  {} {}  len {}",
            e.conn,
            e.dir.label(),
            e.frame,
            e.t_ms,
            e.id,
            printable(&e.name),
            e.len
        );
        if !r.fields.is_empty() {
            let _ = writeln!(s, "    {}", fields_text(r));
        }
        if show_layout {
            match &r.layout {
                Some(l) => {
                    let _ = writeln!(s, "    layout: {l}");
                }
                None if e.id >= 0 => {
                    let _ = writeln!(s, "    layout: (not in the tables for this version)");
                }
                None => {}
            }
        }
        for u in &r.unknown {
            let _ = writeln!(s, "    ? {u}");
        }
        if !r.raw.is_empty() {
            for l in hexdump(&r.raw) {
                let _ = writeln!(s, "    raw {l}");
            }
        }
    }
    s
}

fn view_tsv(v: &View) -> String {
    let mut s = String::from("#seihai-capture-view v1\n");
    if let Some(ver) = v.version {
        let _ = writeln!(s, "#meta version={}", esc(ver.label()));
    }
    let _ = writeln!(s, "#meta mask={}", if v.masked { "on" } else { "off" });
    s.push_str("#conn\tdir\tt_ms\tframe\tid\tname\tlen\tsource\tfields\tlayout\tunknown\traw_hex\n");
    for r in &v.rows {
        let e = &r.event;
        let fields: Vec<String> = r.fields.iter().map(|(k, val)| format!("{}={}", esc(k), esc(val))).collect();
        let unknown: Vec<String> = r.unknown.iter().map(|u| esc(u)).collect();
        let _ = writeln!(
            s,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            e.conn,
            e.dir.label(),
            e.t_ms,
            e.frame,
            e.id,
            esc(&e.name),
            e.len,
            r.source.label(),
            fields.join("|"),
            esc(r.layout.as_deref().unwrap_or("")),
            unknown.join("|"),
            hex_string(&r.raw)
        );
    }
    s
}

fn view_json(v: &View, source: &str, tables_note: &str) -> String {
    let mut s = String::from("{\n");
    let _ = writeln!(s, "  \"capture\": {},", json_str(source));
    let _ = writeln!(s, "  \"version\": {},", v.version.map_or("null".to_string(), |x| json_str(x.label())));
    let _ = writeln!(s, "  \"version_note\": {},", json_str(&v.version_note));
    let _ = writeln!(s, "  \"masked\": {},", v.masked);
    let _ = writeln!(s, "  \"tables\": {},", json_str(tables_note));
    let _ = writeln!(s, "  \"total_messages\": {},", v.total);
    let _ = writeln!(s, "  \"has_login\": {},", v.has_login);
    s.push_str("  \"messages\": [");
    for (i, r) in v.rows.iter().enumerate() {
        let e = &r.event;
        s.push_str(if i == 0 { "\n    " } else { ",\n    " });
        let fields: Vec<String> =
            r.fields.iter().map(|(k, val)| format!("{{\"name\": {}, \"value\": {}}}", json_str(k), json_str(val))).collect();
        let unknown: Vec<String> = r.unknown.iter().map(|u| json_str(u)).collect();
        let _ = write!(
            s,
            "{{\"conn\": {}, \"dir\": {}, \"t_ms\": {}, \"frame\": {}, \"id\": {}, \"name\": {}, \"len\": {}, \"source\": {}, \"fields\": [{}], \"layout\": {}, \"unknown\": [{}], \"raw_hex\": {}}}",
            json_str(&e.conn),
            json_str(e.dir.label()),
            e.t_ms,
            e.frame,
            e.id,
            json_str(&e.name),
            e.len,
            json_str(r.source.label()),
            fields.join(", "),
            r.layout.as_deref().map_or("null".to_string(), json_str),
            unknown.join(", "),
            json_str(&hex_string(&r.raw))
        );
    }
    s.push_str(if v.rows.is_empty() { "]\n}\n" } else { "\n  ]\n}\n" });
    s
}

// ---------------------------------------------------------------------------
// Layout reports

fn range_label(from: &str, to: &str) -> String {
    if from == to { from.to_string() } else { format!("{from} .. {to}") }
}

pub fn render_layouts(reports: &[LayoutReport], fmt: Format) -> String {
    match fmt {
        Format::Text => layouts_text(reports),
        Format::Tsv => layouts_tsv(reports),
        Format::Json => layouts_json(reports),
    }
}

fn run_state(r: &super::Run) -> &'static str {
    match (r.handled, r.rows.is_empty()) {
        (false, true) => "not handled",
        (false, false) => "not handled (the table has a layout, the profile does not list the id)",
        (true, true) => "handled, no layout in the tables (fields undecoded)",
        (true, false) => "handled, layout below",
    }
}

fn layouts_text(reports: &[LayoutReport]) -> String {
    let mut s = String::new();
    if reports.is_empty() {
        s.push_str("nothing known about that id or name: no layout rows and no profile lists it\n");
    }
    for rep in reports {
        let _ = writeln!(s, "{} {}  {}", rep.dir.label(), rep.id, rep.name);
        for c in &rep.changes {
            let _ = writeln!(s, "  profiles: {c}");
        }
        for c in &rep.chain {
            let _ = writeln!(s, "  chain: step {} ({}) {}: {}", c.step, c.label, c.field, c.detail);
        }
        let mut layout_no = 0;
        let mut shown: Vec<(usize, Vec<super::LayoutRow>)> = Vec::new();
        for run in &rep.runs {
            let tag = if run.rows.is_empty() {
                String::new()
            } else if let Some(p) = shown.iter().find(|(_, rows)| *rows == run.rows) {
                format!("  [layout {}]", p.0)
            } else {
                layout_no += 1;
                shown.push((layout_no, run.rows.clone()));
                format!("  [layout {layout_no}]")
            };
            let _ = writeln!(s, "  {:<14} {}{}", range_label(run.from, run.to), run_state(run), tag);
        }
        for (n, rows) in &shown {
            let _ = writeln!(s, "  layout {n}:");
            if rows.iter().all(|r| r.enc == Enc::None) {
                let _ = writeln!(s, "    (no payload)");
            }
            for (pos, r) in rows.iter().filter(|r| r.enc != Enc::None).enumerate() {
                let mark = match r.status {
                    super::tables::Status::Known => " ",
                    super::tables::Status::Partial => "~",
                    super::tables::Status::Unknown => "?",
                };
                let _ = writeln!(
                    s,
                    "    {}{:>2}  {:<12} {:<12} {}{}",
                    mark,
                    pos + 1,
                    r.field,
                    r.enc.label(),
                    r.status.label(),
                    if r.note.is_empty() { String::new() } else { format!("  ({})", r.note) }
                );
            }
            let refs: Vec<&super::LayoutRow> = rows.iter().collect();
            let _ = writeln!(s, "    one line: {}", layout_line(&refs));
        }
        s.push_str("  (? = field exists, meaning unknown; ~ = bytes known, contents only partly decoded)\n");
    }
    s
}

fn layouts_tsv(reports: &[LayoutReport]) -> String {
    let mut s = String::from("dir\tid\tname\tfrom\tto\thandled\tidx\tfield\tencoding\tstatus\tnote\n");
    for rep in reports {
        for run in &rep.runs {
            if run.rows.is_empty() {
                let _ = writeln!(s, "{}\t{}\t{}\t{}\t{}\t{}\t-\t-\t-\tunknown\t", rep.dir.label(), rep.id, rep.name, run.from, run.to, run.handled);
            }
            for r in &run.rows {
                let _ = writeln!(
                    s,
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    rep.dir.label(),
                    rep.id,
                    rep.name,
                    run.from,
                    run.to,
                    run.handled,
                    r.idx,
                    r.field,
                    r.enc.label(),
                    r.status.label(),
                    esc(&r.note)
                );
            }
        }
    }
    s
}

fn layouts_json(reports: &[LayoutReport]) -> String {
    let mut s = String::from("[");
    for (i, rep) in reports.iter().enumerate() {
        s.push_str(if i == 0 { "\n  " } else { ",\n  " });
        let _ = write!(
            s,
            "{{\"dir\": {}, \"id\": {}, \"name\": {}, \"changes\": [{}], \"runs\": [",
            json_str(rep.dir.label()),
            rep.id,
            json_str(&rep.name),
            rep.changes.iter().map(|c| json_str(c)).collect::<Vec<_>>().join(", ")
        );
        for (j, run) in rep.runs.iter().enumerate() {
            if j > 0 {
                s.push_str(", ");
            }
            let fields: Vec<String> = run
                .rows
                .iter()
                .map(|r| {
                    format!(
                        "{{\"idx\": {}, \"field\": {}, \"encoding\": {}, \"status\": {}, \"note\": {}}}",
                        r.idx,
                        json_str(&r.field),
                        json_str(r.enc.label()),
                        json_str(r.status.label()),
                        json_str(&r.note)
                    )
                })
                .collect();
            let _ = write!(
                s,
                "{{\"from\": {}, \"to\": {}, \"handled\": {}, \"has_layout\": {}, \"fields\": [{}]}}",
                json_str(run.from),
                json_str(run.to),
                run.handled,
                !run.rows.is_empty(),
                fields.join(", ")
            );
        }
        s.push_str("]}");
    }
    s.push_str(if reports.is_empty() { "]\n" } else { "\n]\n" });
    s
}

