//! Capture viewer and layout explorer (`seihai capture ...`).
//!
//! A command-line tool that reads a capture and prints the decoded messages
//! with the packet names and field layouts from the repository tables:
//!
//! * [`load`] reads a pcap (through the difftest reader and Seihai's own
//!   codecs) or a `seihai-trace v1` file (what `difftest trace` writes);
//! * [`build_view`] filters the messages, adds the layout and raw bytes where
//!   the tables and the decoder leave something undecoded;
//! * [`layout_report`] shows one packet's layout across the version chain;
//! * [`render`] prints text, tsv or json.
//!
//! Login frames are always redacted: the password is never decoded, and the
//! account is masked unless `--no-mask` is given. The raw bytes of a login
//! frame are never kept. See `documentation/capture-viewer.md`.

pub mod cli;
pub mod fields;
pub mod render;
pub mod tables;

use crate::difftest::decode::{decode_conns_ex, DecodeOptions};
use crate::difftest::names::{pli_name, plo_name};
use crate::difftest::pcap::{read_pcap, reassemble};
use crate::difftest::trace::{Dir, Event, Trace, ID_BADFRAME, ID_LOGIN};
use crate::protocol::ClientVersion;
use tables::{layout_line, ChainRow, LayoutRow, Tables};

/// What was loaded.
pub struct Loaded {
    pub trace: Trace,
    /// The account the login frame carried (pcap input only).
    pub account: Option<String>,
    pub kind: InputKind,
    /// The version used to decode, and how it was chosen.
    pub version: Option<ClientVersion>,
    pub version_note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Pcap,
    TraceTsv,
}

/// Read a capture file's bytes. A `seihai-trace v1` text file is read as a
/// trace; anything else must be a pcap.
///
/// `version` of `None` means auto-detect (pcap) or use the trace's `version`
/// header (trace file).
pub fn load(data: &[u8], version: Option<ClientVersion>, mask: bool) -> Result<Loaded, String> {
    if data.starts_with(b"#seihai-trace v1") {
        let text = std::str::from_utf8(data).map_err(|_| "trace file is not UTF-8".to_string())?;
        let trace = Trace::from_tsv(text)?;
        let (v, note) = match version {
            Some(v) => (Some(v), "from --version".to_string()),
            None => match trace.meta("version").and_then(ClientVersion::from_label) {
                Some(v) => (Some(v), "from the trace header".to_string()),
                None => (None, "not in the trace header".to_string()),
            },
        };
        return Ok(Loaded { trace, account: None, kind: InputKind::TraceTsv, version: v, version_note: note });
    }
    let packets = read_pcap(data)?;
    let conns = reassemble(&packets);
    let (v, note) = match version {
        Some(v) => (v, "from --version".to_string()),
        None => detect_version(&conns)?,
    };
    let mut o = DecodeOptions::new(v);
    o.mask = mask;
    o.keep_raw = true;
    let (trace, account) = decode_conns_ex(&conns, &o);
    Ok(Loaded { trace, account, kind: InputKind::Pcap, version: Some(v), version_note: note })
}

/// Pick the version whose login token the capture carries. Every version is
/// tried; the one whose token matches a game-server login (or whose
/// listserver token matches a listserver hello) and with the fewest bad
/// frames wins. If several tie, the first in chain order is used and the
/// others are named.
pub fn detect_version(conns: &[crate::difftest::pcap::TcpConn]) -> Result<(ClientVersion, String), String> {
    if conns.iter().all(|c| c.c2s.bytes.is_empty() && c.s2c.bytes.is_empty()) {
        return Err("cannot detect the version: the capture holds no TCP data".to_string());
    }
    let mut scored: Vec<(i64, ClientVersion)> = Vec::new();
    for &v in ClientVersion::all_versions() {
        let mut o = DecodeOptions::new(v);
        o.mask = true;
        let trace = crate::difftest::decode::decode_conns(conns, &o);
        let mut score = 0i64;
        for e in &trace.events {
            if e.dir != Dir::C2S || e.id != ID_LOGIN {
                continue;
            }
            if e.name == "LOGIN" && e.field("token") == Some(v.token()) {
                score += 100;
            }
            if e.name == "LS_HELLO" {
                if let Some(rest) = e.field("rest") {
                    if rest.starts_with(v.listserver_token()) {
                        score += 50;
                    }
                }
            }
        }
        if score == 0 {
            continue;
        }
        let bad: i64 = trace.meta("bad_frames").and_then(|b| b.parse().ok()).unwrap_or(0);
        scored.push((score - bad, v));
    }
    let best = scored.iter().map(|s| s.0).max().ok_or_else(|| {
        "cannot detect the version: no login token in the capture matches a known build; pass --version".to_string()
    })?;
    let tied: Vec<ClientVersion> = scored.iter().filter(|s| s.0 == best).map(|s| s.1).collect();
    let first = tied[0];
    let note = if tied.len() > 1 {
        let others: Vec<&str> = tied[1..].iter().map(|v| v.label()).collect();
        format!("auto-detected from the login token; also matches {}", others.join(", "))
    } else {
        "auto-detected from the login token".to_string()
    };
    Ok((first, note))
}

// ---------------------------------------------------------------------------
// Selecting messages

/// A packet chosen by id or by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sel {
    /// An id, in one direction if it was written `PLI_n` or `PLO_n`.
    Id(i32, Option<Dir>),
    /// Upper-case name without a `PLI_`/`PLO_` prefix (which sets the direction).
    Name(String, Option<Dir>),
}

impl Sel {
    pub fn parse(s: &str) -> Sel {
        let u = s.trim().to_ascii_uppercase();
        let (rest, dir) = if let Some(r) = u.strip_prefix("PLI_") {
            (r.to_string(), Some(Dir::C2S))
        } else if let Some(r) = u.strip_prefix("PLO_") {
            (r.to_string(), Some(Dir::S2C))
        } else {
            (u.clone(), None)
        };
        match rest.parse::<i32>() {
            Ok(n) => Sel::Id(n, dir),
            Err(_) => Sel::Name(rest, dir),
        }
    }

    fn matches(&self, e: &Event) -> bool {
        match self {
            Sel::Id(n, d) => e.id == *n && d.is_none_or(|d| d == e.dir),
            Sel::Name(n, d) => e.name.eq_ignore_ascii_case(n) && d.is_none_or(|d| d == e.dir),
        }
    }
}

/// An inclusive range with open ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Range {
    pub lo: Option<i64>,
    pub hi: Option<i64>,
}

impl Range {
    /// `N`, `A..B`, `A..` or `..B`.
    pub fn parse(s: &str) -> Result<Range, String> {
        let num = |t: &str| t.trim().parse::<i64>().map_err(|_| format!("bad number `{t}` in range `{s}`"));
        match s.split_once("..") {
            None => {
                let n = num(s)?;
                Ok(Range { lo: Some(n), hi: Some(n) })
            }
            Some((a, b)) => Ok(Range {
                lo: if a.trim().is_empty() { None } else { Some(num(a)?) },
                hi: if b.trim().is_empty() { None } else { Some(num(b)?) },
            }),
        }
    }

    pub fn contains(&self, v: i64) -> bool {
        self.lo.is_none_or(|l| v >= l) && self.hi.is_none_or(|h| v <= h)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub dir: Option<Dir>,
    pub sel: Vec<Sel>,
    pub conn: Option<String>,
    /// Milliseconds since the connection's first packet.
    pub time: Range,
    /// Frame index within the connection and direction.
    pub frame: Range,
    pub limit: Option<usize>,
}

impl Filter {
    pub fn accepts(&self, e: &Event) -> bool {
        self.dir.is_none_or(|d| d == e.dir)
            && (self.sel.is_empty() || self.sel.iter().any(|s| s.matches(e)))
            && self.conn.as_ref().is_none_or(|c| *c == e.conn)
            && self.time.contains(e.t_ms)
            && self.frame.contains(i64::from(e.frame))
    }
}

// ---------------------------------------------------------------------------
// The view

/// Where a row's fields came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The trace decoder's named fields.
    Decoder,
    /// A layout from the tables.
    Table,
    /// Neither: only raw bytes (or nothing) to show.
    Raw,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Decoder => "decoder",
            Source::Table => "table",
            Source::Raw => "raw",
        }
    }
}

/// One message ready to print.
#[derive(Debug, Clone)]
pub struct Row {
    pub event: Event,
    pub fields: Vec<(String, String)>,
    pub source: Source,
    /// One-line layout from the tables for this version, if there is one.
    pub layout: Option<String>,
    /// What the layout or the decoder leaves unsettled.
    pub unknown: Vec<String>,
    /// Payload bytes to show as hex (never for a login frame).
    pub raw: Vec<u8>,
}

pub struct View {
    pub version: Option<ClientVersion>,
    pub version_note: String,
    pub masked: bool,
    pub meta: Vec<(String, String)>,
    pub total: usize,
    /// The capture holds a login (game-server login, listserver hello or
    /// listserver request).
    pub has_login: bool,
    pub rows: Vec<Row>,
}

/// Does the capture contain a login frame?
pub fn has_login(t: &Trace) -> bool {
    t.events.iter().any(|e| e.id == ID_LOGIN || (e.dir == Dir::C2S && e.name == "LS_SERVERLIST"))
}

fn generic_only(fields: &[(String, String)]) -> bool {
    fields.iter().all(|(k, _)| matches!(k.as_str(), "hex" | "crc" | "len" | "unparsed_tail"))
}

pub struct ViewOptions {
    pub filter: Filter,
    /// Show the layout line for every message.
    pub show_layout: bool,
    /// Show the raw bytes of every message, not only undecoded ones.
    pub raw_all: bool,
    pub mask: bool,
}

pub fn build_view(loaded: &Loaded, tables: &Tables, opts: &ViewOptions) -> View {
    let trace = &loaded.trace;
    let mut rows = Vec::new();
    for e in &trace.events {
        if !opts.filter.accepts(e) {
            continue;
        }
        if opts.filter.limit.is_some_and(|l| rows.len() >= l) {
            break;
        }
        rows.push(make_row(e, loaded, tables, opts));
    }
    View {
        version: loaded.version,
        version_note: loaded.version_note.clone(),
        masked: opts.mask,
        meta: trace.meta.clone(),
        total: trace.events.len(),
        has_login: has_login(trace),
        rows,
    }
}

fn make_row(e: &Event, loaded: &Loaded, tables: &Tables, opts: &ViewOptions) -> Row {
    let numbered = e.id >= 0;
    let layout_rows: Vec<&LayoutRow> = if numbered && !e.conn.starts_with("ls") {
        tables.rows_for(e.dir, e.id as u32, loaded.version)
    } else {
        Vec::new()
    };
    let layout = if layout_rows.is_empty() { None } else { Some(layout_line(&layout_rows)) };
    let mut fields = e.fields.clone();
    let mut unknown = Vec::new();
    let mut source = Source::Decoder;
    let undecoded = numbered && generic_only(&e.fields);
    if undecoded {
        source = Source::Raw;
        if !layout_rows.is_empty() && (!e.raw.is_empty() || layout_rows.iter().all(|r| r.enc == tables::Enc::None)) {
            if let Some(rd) = fields::read_layout(&layout_rows, &e.raw, opts.mask, loaded.account.as_deref()) {
                fields = rd.fields;
                unknown = rd.unknown;
                source = Source::Table;
            }
        }
    } else if numbered {
        // The decoder named the fields; still say which fields the tables
        // call partial or unknown.
        for r in &layout_rows {
            if r.status != tables::Status::Known {
                unknown.push(format!("`{}` is {} in the table", r.field, r.status.label()));
            }
        }
    }
    if numbered && layout_rows.is_empty() && source == Source::Raw && !e.raw.is_empty() {
        unknown.push("no layout for this id and version in the tables".into());
    }
    // Raw bytes: undecoded messages, or all with --raw. Never a login frame.
    let show_raw = numbered
        && !e.raw.is_empty()
        && (opts.raw_all || source == Source::Raw || (source == Source::Table && !unknown.is_empty()));
    let raw = if show_raw { e.raw.clone() } else { Vec::new() };
    if show_raw {
        // The dump says what `hex=` said.
        fields.retain(|(k, _)| k != "hex");
    }
    if e.id == ID_BADFRAME {
        unknown.push("frame did not decode".into());
    }
    Row { event: e.clone(), fields, source, layout, unknown, raw }
}

// ---------------------------------------------------------------------------
// Layout explorer

/// A stretch of consecutive versions that handle a packet the same way.
#[derive(Debug, Clone)]
pub struct Run {
    pub from: &'static str,
    pub to: &'static str,
    pub handled: bool,
    /// The layout rows that apply (empty when the table has none).
    pub rows: Vec<LayoutRow>,
}

#[derive(Debug, Clone)]
pub struct LayoutReport {
    pub dir: Dir,
    pub id: u32,
    pub name: String,
    pub runs: Vec<Run>,
    /// `+`/`-` steps along the chain, from the profiles.
    pub changes: Vec<String>,
    /// Rows of the version chain that name the id.
    pub chain: Vec<ChainRow>,
}

fn id_name(tables: &Tables, dir: Dir, id: u32) -> String {
    let built = match dir {
        Dir::C2S => pli_name(id),
        Dir::S2C => plo_name(id),
    };
    built.map(str::to_string).or_else(|| tables.name_of(dir, id).map(str::to_string)).unwrap_or_else(|| format!("{}_{id}", if dir == Dir::C2S { "PLI" } else { "PLO" }))
}

fn handled(v: ClientVersion, dir: Dir, id: u32) -> bool {
    let p = &v.profile().protocol;
    match (u8::try_from(id), dir) {
        (Ok(i), Dir::C2S) => p.pli.contains(i),
        // The board (101) and the file (102) arrive as the raw packet after
        // `PLO_RAWDATA` (100), so a profile lists only 100.
        (Ok(i), Dir::S2C) => p.plo.contains(i) || (matches!(i, 101 | 102) && p.plo.contains(100)),
        _ => false,
    }
}

pub fn layout_report(tables: &Tables, dir: Dir, id: u32) -> LayoutReport {
    let versions = ClientVersion::all_versions();
    let mut runs: Vec<Run> = Vec::new();
    let mut changes = Vec::new();
    let mut prev: Option<bool> = None;
    for &v in versions {
        let h = handled(v, dir, id);
        let rows: Vec<LayoutRow> = tables.rows_for(dir, id, Some(v)).into_iter().cloned().collect();
        match runs.last_mut() {
            Some(last) if last.handled == h && last.rows == rows => last.to = v.label(),
            _ => runs.push(Run { from: v.label(), to: v.label(), handled: h, rows }),
        }
        match (prev, h) {
            (Some(false), true) => changes.push(format!("added in {}", v.label())),
            (Some(true), false) => changes.push(format!("dropped in {}", v.label())),
            (None, true) => changes.push(format!("handled from {} (first build in the chain)", v.label())),
            _ => {}
        }
        prev = Some(h);
    }
    let area = if dir == Dir::C2S { "pli" } else { "plo" };
    let chain = tables
        .chain
        .iter()
        .filter(|c| c.area == "protocol" && c.field.starts_with(area) && detail_names_id(&c.detail, id))
        .cloned()
        .collect();
    LayoutReport { dir, id, name: id_name(tables, dir, id), runs, changes, chain }
}

/// Does a chain detail like `+43 DEFAULTWEAPON, -25 UNKNOWN25 (32 -> 33 ids)`
/// carry a `+id` or `-id` for this id?
fn detail_names_id(detail: &str, id: u32) -> bool {
    detail.split(',').any(|t| {
        let t = t.trim();
        let Some(rest) = t.strip_prefix('+').or_else(|| t.strip_prefix('-')) else { return false };
        rest.split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse::<u32>().ok()) == Some(id)
    })
}

/// What `--layout X` means: a number is an id in each direction that has
/// something to say; a name is every packet of that name.
pub fn resolve_layout_target(tables: &Tables, what: &str, dir: Option<Dir>) -> Vec<(Dir, u32)> {
    let mut out: Vec<(Dir, u32)> = Vec::new();
    let dirs: Vec<Dir> = match dir {
        Some(d) => vec![d],
        None => vec![Dir::C2S, Dir::S2C],
    };
    match what.trim().parse::<u32>() {
        Ok(id) => {
            for d in dirs {
                let in_table = tables.rows.iter().any(|r| r.dir == d && r.id == id);
                let in_profile = ClientVersion::all_versions().iter().any(|&v| handled(v, d, id));
                if in_table || in_profile {
                    out.push((d, id));
                }
            }
        }
        Err(_) => {
            let sel = what.trim().to_ascii_uppercase();
            let sel = sel.strip_prefix("PLI_").or_else(|| sel.strip_prefix("PLO_")).unwrap_or(&sel).to_string();
            for d in dirs {
                for id in 0..=255u32 {
                    if id_name(tables, d, id).eq_ignore_ascii_case(&sel) {
                        out.push((d, id));
                    }
                }
                for (dd, id) in tables.ids() {
                    if dd == d && tables.name_of(dd, id).is_some_and(|n| n.eq_ignore_ascii_case(&sel)) && !out.contains(&(dd, id)) {
                        out.push((dd, id));
                    }
                }
            }
        }
    }
    out
}

/// Mode check for `--out`: when the capture has a login, an existing output
/// file must not be readable or writable by group or others.
pub fn check_output_mode(path: &std::path::Path, has_login: bool) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if has_login {
            if let Ok(m) = std::fs::metadata(path) {
                let mode = m.permissions().mode() & 0o777;
                if mode & 0o077 != 0 {
                    return Err(format!(
                        "refusing to write {}: it has mode {:03o}, looser than 600, and the capture contains a login (chmod 600 it, or remove it, first)",
                        path.display(),
                        mode
                    ));
                }
            }
        }
    }
    let _ = (path, has_login);
    Ok(())
}

/// Write `text` to `path`, creating it with mode 600.
pub fn write_private(path: &std::path::Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    f.write_all(text.as_bytes()).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests;
