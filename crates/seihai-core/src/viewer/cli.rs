//! `seihai capture ...`: argument handling for the capture viewer.

use super::render::{render_layouts, render_view, Format};
use super::tables::{lint, Tables};
use super::{
    build_view, check_output_mode, layout_report, load, resolve_layout_target, write_private, Filter, Range, Sel, ViewOptions,
};
use crate::difftest::trace::Dir;
use crate::protocol::ClientVersion;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
usage: seihai capture show FILE [options]     print the decoded messages of a capture
       seihai capture layout ID|NAME [options] print one packet's field layout across versions
       seihai capture tables [--tables DIR]    list the tables in use and check them

FILE is a pcap (as the lab writes it) or a `seihai-trace v1` file (`difftest trace`).

show options:
  --version V        client version (default: detected from the login token;
                     a trace file uses its header)
  --tables DIR       read packet-layouts.tsv (and version-chain.tsv) from DIR
                     instead of the built-in tables
  --dir c2s|s2c      one direction only
  --id X[,Y..]       only these packets, by id or name (repeatable)
  --name NAME        same as --id with a name
  --conn NAME        one connection (gs0, ls0, ...)
  --time A..B        milliseconds since the connection start (A.. , ..B, or one value)
  --frame A..B       frame index within the connection and direction
  --limit N          stop after N messages
  --layouts          show the table layout under every message
  --raw              hex dump of every payload, not only undecoded ones
  --format F         text (default), tsv or json
  --out FILE         write to FILE (mode 600; refused if an existing FILE is
                     looser than 600 and the capture contains a login)
  --no-mask          keep key, player ids and account (the password is never shown)

layout options: --dir c2s|s2c, --version V (only that version), --tables DIR, --format F
Login frames are redacted by default; their raw bytes are never printed.
";

fn fail(msg: &str) -> i32 {
    eprintln!("seihai capture: {msg}");
    2
}

struct Args<'a> {
    rest: &'a [String],
    i: usize,
}

impl<'a> Args<'a> {
    fn next(&mut self) -> Option<&'a str> {
        let a = self.rest.get(self.i)?;
        self.i += 1;
        Some(a.as_str())
    }

    fn value(&mut self, flag: &str) -> Result<&'a str, String> {
        self.next().ok_or_else(|| format!("{flag} needs a value"))
    }
}

fn parse_dir(s: &str) -> Result<Dir, String> {
    match s.to_ascii_lowercase().as_str() {
        "c2s" | "c>s" => Ok(Dir::C2S),
        "s2c" | "s>c" => Ok(Dir::S2C),
        _ => Err(format!("--dir takes c2s or s2c, not `{s}`")),
    }
}

fn parse_version(s: &str) -> Result<ClientVersion, String> {
    ClientVersion::from_label(s).ok_or_else(|| format!("unknown version `{s}`"))
}

fn load_tables(dir: &Option<PathBuf>) -> Result<Tables, String> {
    match dir {
        Some(d) => Tables::load_dir(d),
        None => Ok(Tables::builtin()),
    }
}

fn tables_note(t: &Tables) -> String {
    t.sources.iter().map(|(n, s)| format!("{n}: {s}")).collect::<Vec<_>>().join(", ")
}

/// Entry point. `args` are the words after `capture`. Returns the exit code.
pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("show") => run_show(&args[1..]),
        Some("layout") => run_layout(&args[1..]),
        Some("tables") => run_tables(&args[1..]),
        Some("help") | Some("--help") | Some("-h") => {
            print!("{USAGE}");
            0
        }
        _ => {
            eprint!("{USAGE}");
            2
        }
    }
}

fn run_show(args: &[String]) -> i32 {
    let mut a = Args { rest: args, i: 0 };
    let mut file: Option<String> = None;
    let mut version: Option<ClientVersion> = None;
    let mut tdir: Option<PathBuf> = None;
    let mut filter = Filter::default();
    let mut fmt = Format::Text;
    let mut out: Option<PathBuf> = None;
    let (mut show_layout, mut raw_all, mut mask) = (false, false, true);
    while let Some(w) = a.next() {
        let r: Result<(), String> = (|| {
            match w {
                "--version" => version = Some(parse_version(a.value(w)?)?),
                "--tables" => tdir = Some(PathBuf::from(a.value(w)?)),
                "--dir" => filter.dir = Some(parse_dir(a.value(w)?)?),
                "--id" | "--name" => {
                    for part in a.value(w)?.split(',').filter(|p| !p.trim().is_empty()) {
                        filter.sel.push(Sel::parse(part));
                    }
                }
                "--conn" => filter.conn = Some(a.value(w)?.to_string()),
                "--time" => filter.time = Range::parse(a.value(w)?)?,
                "--frame" => filter.frame = Range::parse(a.value(w)?)?,
                "--limit" => {
                    let v = a.value(w)?;
                    filter.limit = Some(v.parse().map_err(|_| format!("--limit takes a number, not `{v}`"))?);
                }
                "--layouts" => show_layout = true,
                "--raw" => raw_all = true,
                "--format" => {
                    let v = a.value(w)?;
                    fmt = Format::parse(v).ok_or_else(|| format!("--format takes text, tsv or json, not `{v}`"))?;
                }
                "--out" => out = Some(PathBuf::from(a.value(w)?)),
                "--no-mask" => mask = false,
                f if f.starts_with("--") => return Err(format!("unknown option {f}")),
                f => {
                    if file.replace(f.to_string()).is_some() {
                        return Err("more than one capture file given".into());
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = r {
            return fail(&e);
        }
    }
    let Some(file) = file else {
        eprint!("{USAGE}");
        return 2;
    };
    let tables = match load_tables(&tdir) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    let data = match std::fs::read(&file) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("seihai capture: cannot read {file}: {e}");
            return 1;
        }
    };
    let loaded = match load(&data, version, mask) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("seihai capture: {file}: {e}");
            return 1;
        }
    };
    if loaded.version.is_none() {
        eprintln!("seihai capture: {file}: the trace header has no version; pass --version");
        return 1;
    }
    let opts = ViewOptions { filter, show_layout, raw_all, mask };
    let view = build_view(&loaded, &tables, &opts);
    if let Some(p) = &out {
        if let Err(e) = check_output_mode(p, view.has_login) {
            eprintln!("seihai capture: {e}");
            return 1;
        }
    }
    let name = Path::new(&file).file_name().map_or(file.clone(), |n| n.to_string_lossy().into_owned());
    let text = render_view(&view, fmt, &name, &tables_note(&tables), show_layout);
    match out {
        Some(p) => {
            if let Err(e) = write_private(&p, &text) {
                eprintln!("seihai capture: {e}");
                return 1;
            }
            eprintln!("wrote {} message(s) to {}", view.rows.len(), p.display());
        }
        None => print!("{text}"),
    }
    0
}

fn run_layout(args: &[String]) -> i32 {
    let mut a = Args { rest: args, i: 0 };
    let mut what: Option<String> = None;
    let mut dir: Option<Dir> = None;
    let mut version: Option<ClientVersion> = None;
    let mut tdir: Option<PathBuf> = None;
    let mut fmt = Format::Text;
    while let Some(w) = a.next() {
        let r: Result<(), String> = (|| {
            match w {
                "--dir" => dir = Some(parse_dir(a.value(w)?)?),
                "--version" => version = Some(parse_version(a.value(w)?)?),
                "--tables" => tdir = Some(PathBuf::from(a.value(w)?)),
                "--format" => {
                    let v = a.value(w)?;
                    fmt = Format::parse(v).ok_or_else(|| format!("--format takes text, tsv or json, not `{v}`"))?;
                }
                f if f.starts_with("--") => return Err(format!("unknown option {f}")),
                f => {
                    if what.replace(f.to_string()).is_some() {
                        return Err("more than one packet given".into());
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = r {
            return fail(&e);
        }
    }
    let Some(what) = what else {
        eprint!("{USAGE}");
        return 2;
    };
    let tables = match load_tables(&tdir) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    let mut reports: Vec<_> =
        resolve_layout_target(&tables, &what, dir).into_iter().map(|(d, id)| layout_report(&tables, d, id)).collect();
    if let Some(v) = version {
        let r = super::tables::rank(v);
        for rep in &mut reports {
            rep.runs.retain(|run| {
                let (f, t) = (ClientVersion::from_label(run.from), ClientVersion::from_label(run.to));
                matches!((f, t), (Some(f), Some(t)) if super::tables::rank(f) <= r && r <= super::tables::rank(t))
            });
        }
    }
    print!("{}", render_layouts(&reports, fmt));
    if reports.is_empty() { 1 } else { 0 }
}

fn run_tables(args: &[String]) -> i32 {
    let mut a = Args { rest: args, i: 0 };
    let mut tdir: Option<PathBuf> = None;
    while let Some(w) = a.next() {
        match w {
            "--tables" => match a.value(w) {
                Ok(v) => tdir = Some(PathBuf::from(v)),
                Err(e) => return fail(&e),
            },
            f => return fail(&format!("unknown argument {f}")),
        }
    }
    let t = match load_tables(&tdir) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    println!("tables in use: {}", tables_note(&t));
    let ids = t.ids();
    let c2s = ids.iter().filter(|(d, _)| *d == Dir::C2S).count();
    println!("packet-layouts.tsv: {} field rows, {} packets (C>S {}, S>C {})", t.rows.len(), ids.len(), c2s, ids.len() - c2s);
    for st in [super::tables::Status::Known, super::tables::Status::Partial, super::tables::Status::Unknown] {
        println!("  {:<8} {}", st.label(), t.rows.iter().filter(|r| r.status == st).count());
    }
    if !t.chain.is_empty() {
        println!("version-chain.tsv: {} rows", t.chain.len());
    }
    let problems = lint(&t);
    for p in &problems {
        println!("warning: {p}");
    }
    if problems.is_empty() {
        println!("no problems found");
        0
    } else {
        1
    }
}
