//! Compare a real-client trace with a Seihai trace of the same scenario.
//!
//! The two sessions never match byte for byte: walking is timed by the
//! clock, keys and ids are random, periodic messages drift. The differ is
//! therefore tolerant where that is expected and strict where it is not:
//!
//! * Per connection and direction the messages are collapsed into runs of
//!   the same id and aligned with an LCS, so a different number of walk
//!   updates does not count as a difference but a missing or reordered
//!   message does.
//! * Messages on a timer (ids 24 and 39 from the client) are left out of
//!   the alignment and compared by presence and by their median interval,
//!   with a jitter allowance.
//! * Known non-deterministic fields are not compared (masked values,
//!   position of later walk updates); positions elsewhere get a tolerance.
//! * Known harness noise (an `xdotool` click makes the real client send a
//!   trigger action, id 38) is counted but not ranked.
//!
//! Every divergence has a severity and a category; the report ranks them.

use super::trace::{Dir, Event, Trace};
use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    High,
    Medium,
    Low,
    Info,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
            Severity::Info => "info",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    MissingConn,
    ExtraConn,
    MissingId,
    ExtraId,
    Order,
    Bundling,
    Field,
    Count,
    Timing,
    Noise,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::MissingConn => "missing-connection",
            Category::ExtraConn => "extra-connection",
            Category::MissingId => "missing-id",
            Category::ExtraId => "extra-id",
            Category::Order => "ordering",
            Category::Bundling => "frame-bundling",
            Category::Field => "field-mismatch",
            Category::Count => "count",
            Category::Timing => "timing",
            Category::Noise => "harness-noise",
        }
    }
}

/// One difference between the two traces.
#[derive(Debug, Clone, PartialEq)]
pub struct Divergence {
    pub severity: Severity,
    pub category: Category,
    pub conn: String,
    pub dir: Option<Dir>,
    pub id: Option<i32>,
    pub name: String,
    /// Index of the message in the real trace's stream (0-based), if any.
    pub real_pos: Option<usize>,
    pub seihai_pos: Option<usize>,
    pub detail: String,
}

/// Knobs; the defaults suit the lab scenarios.
#[derive(Debug, Clone)]
pub struct DiffOptions {
    /// Timing allowance in milliseconds: an absolute floor under the 20 %
    /// relative allowance.
    pub jitter_ms: i64,
    /// Position tolerance in tiles.
    pub pos_tol: f32,
    /// Ids that are harness artifacts, not client behaviour.
    pub noise: Vec<(Dir, i32)>,
    /// Ids sent on a timer: left out of the alignment, compared by interval.
    pub periodic: Vec<(Dir, i32)>,
    /// Ids whose count follows the clock (walk updates): counts not compared.
    pub variable_count: Vec<(Dir, i32)>,
    /// Field names never compared.
    pub ignore_fields: Vec<String>,
    /// `(direction, id, field)` compared as a set over the whole stream, not
    /// message by message (which file is asked for depends on the cache).
    pub set_fields: Vec<(Dir, i32, String)>,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            jitter_ms: 5,
            pos_tol: 0.6,
            // The lab clicks the real client's window to give it focus; the
            // client answers with PLI_TRIGGERACTION (38).
            noise: vec![(Dir::C2S, 38)],
            // PLI_SHOWIMGPLAYER (24) and PLI_TAMPERCHECK (39).
            periodic: vec![(Dir::C2S, 24), (Dir::C2S, 39)],
            variable_count: vec![(Dir::C2S, 2), (Dir::S2C, 8), (Dir::S2C, 9)],
            ignore_fields: vec![],
            set_fields: vec![
                (Dir::C2S, 23, "file".into()),
                (Dir::C2S, 34, "file".into()),
                // which props each walk update carries (X2 or X,Y,SPRITE, with or
                // without the cell prop) is a property of the client, not the clock
                (Dir::C2S, 2, "props".into()),
            ],
        }
    }
}

/// Messages per stream, for the report's overview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamCount {
    pub conn: String,
    pub dir: Dir,
    pub real: usize,
    pub seihai: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub real_meta: Vec<(String, String)>,
    pub seihai_meta: Vec<(String, String)>,
    pub streams: Vec<StreamCount>,
    /// Ranked, worst first.
    pub divergences: Vec<Divergence>,
}

/// Compare `real` with `seihai`.
pub fn diff(real: &Trace, seihai: &Trace, opts: &DiffOptions) -> Report {
    let mut rep = Report {
        real_meta: real.meta.clone(),
        seihai_meta: seihai.meta.clone(),
        ..Default::default()
    };
    let rc = real.conns();
    let sc = seihai.conns();
    let mut all: Vec<String> = rc.clone();
    for c in &sc {
        if !all.contains(c) {
            all.push(c.clone());
        }
    }
    for conn in &all {
        match (rc.contains(conn), sc.contains(conn)) {
            (true, false) => rep.divergences.push(Divergence {
                severity: Severity::High,
                category: Category::MissingConn,
                conn: conn.clone(),
                dir: None,
                id: None,
                name: String::new(),
                real_pos: None,
                seihai_pos: None,
                detail: format!("the real client opened connection {conn}; Seihai did not"),
            }),
            (false, true) => rep.divergences.push(Divergence {
                severity: Severity::High,
                category: Category::ExtraConn,
                conn: conn.clone(),
                dir: None,
                id: None,
                name: String::new(),
                real_pos: None,
                seihai_pos: None,
                detail: format!("Seihai opened connection {conn}; the real client did not"),
            }),
            _ => {
                for dir in [Dir::C2S, Dir::S2C] {
                    let a: Vec<&Event> = real.stream(conn, dir).collect();
                    let b: Vec<&Event> = seihai.stream(conn, dir).collect();
                    rep.streams.push(StreamCount { conn: conn.clone(), dir, real: a.len(), seihai: b.len() });
                    compare_stream(conn, dir, &a, &b, opts, &mut rep.divergences);
                }
            }
        }
    }
    rep.divergences.sort_by(|x, y| {
        (x.severity, x.category, &x.conn, x.dir.map(|d| d as u8), x.real_pos.or(x.seihai_pos))
            .cmp(&(y.severity, y.category, &y.conn, y.dir.map(|d| d as u8), y.real_pos.or(y.seihai_pos)))
    });
    rep
}

struct Run<'a> {
    id: i32,
    /// Position of the first message in the filtered stream.
    pos: usize,
    count: usize,
    first: &'a Event,
    /// Every message of the run.
    events: Vec<&'a Event>,
    /// The first message starts a new frame.
    frame_new: bool,
}

fn has(list: &[(Dir, i32)], dir: Dir, id: i32) -> bool {
    list.iter().any(|&(d, i)| d == dir && i == id)
}

fn runs<'a>(evs: &[&'a Event], dir: Dir, opts: &DiffOptions) -> Vec<Run<'a>> {
    let mut out: Vec<Run<'a>> = Vec::new();
    let mut prev_frame: Option<u32> = None;
    for (pos, e) in evs.iter().enumerate() {
        let new = prev_frame != Some(e.frame);
        prev_frame = Some(e.frame);
        if has(&opts.periodic, dir, e.id) {
            continue;
        }
        match out.last_mut() {
            Some(r) if r.id == e.id => {
                r.count += 1;
                r.events.push(e);
            }
            _ => out.push(Run { id: e.id, pos, count: 1, first: e, events: vec![e], frame_new: new }),
        }
    }
    out
}

fn label(e: &Event) -> String {
    format!("{} ({})", e.name, e.id)
}

fn median(v: &mut Vec<i64>) -> i64 {
    v.sort_unstable();
    v[v.len() / 2]
}

fn intervals(evs: &[&Event], id: i32) -> Vec<i64> {
    let t: Vec<i64> = evs.iter().filter(|e| e.id == id).map(|e| e.t_ms).collect();
    t.windows(2).map(|w| w[1] - w[0]).collect()
}

fn compare_stream(
    conn: &str,
    dir: Dir,
    a_all: &[&Event],
    b_all: &[&Event],
    opts: &DiffOptions,
    out: &mut Vec<Divergence>,
) {
    let mk = |sev, cat, id: i32, name: &str, rp, sp, detail: String| Divergence {
        severity: sev,
        category: cat,
        conn: conn.to_string(),
        dir: Some(dir),
        id: Some(id),
        name: name.to_string(),
        real_pos: rp,
        seihai_pos: sp,
        detail,
    };
    // Known harness noise: counted, not ranked.
    for (list, who) in [(a_all, "real"), (b_all, "seihai")] {
        let mut n: BTreeMap<i32, (usize, String)> = BTreeMap::new();
        for e in list.iter().filter(|e| has(&opts.noise, dir, e.id)) {
            n.entry(e.id).or_insert((0, e.name.clone())).0 += 1;
        }
        for (id, (c, nm)) in n {
            out.push(mk(Severity::Info, Category::Noise, id, &nm, None, None, format!("{who} trace has {c} x {nm}, left out (harness artifact)")));
        }
    }
    let a: Vec<&Event> = a_all.iter().copied().filter(|e| !has(&opts.noise, dir, e.id)).collect();
    let b: Vec<&Event> = b_all.iter().copied().filter(|e| !has(&opts.noise, dir, e.id)).collect();
    let ra = runs(&a, dir, opts);
    let rb = runs(&b, dir, opts);
    let ids_a: BTreeMap<i32, usize> = a.iter().fold(BTreeMap::new(), |mut m, e| {
        *m.entry(e.id).or_default() += 1;
        m
    });
    let ids_b: BTreeMap<i32, usize> = b.iter().fold(BTreeMap::new(), |mut m, e| {
        *m.entry(e.id).or_default() += 1;
        m
    });
    // Server replies follow from what the client sent: one step lower.
    let sev_dir = |hi: Severity| match (dir, hi) {
        (Dir::C2S, h) => h,
        (Dir::S2C, Severity::High) => Severity::Medium,
        (Dir::S2C, _) => Severity::Low,
    };
    // Presence (every id, periodic or not).
    for (&id, &n) in &ids_a {
        if !ids_b.contains_key(&id) {
            let e = a.iter().find(|e| e.id == id).unwrap();
            let pos = a.iter().position(|e| e.id == id);
            let sev = if id < 0 { Severity::High } else { sev_dir(Severity::High) };
            out.push(mk(sev, Category::MissingId, id, &e.name, pos, None, format!("the real client has {n} x {} here; Seihai never sends it", label(e))));
        }
    }
    for (&id, &n) in &ids_b {
        if !ids_a.contains_key(&id) {
            let e = b.iter().find(|e| e.id == id).unwrap();
            let pos = b.iter().position(|e| e.id == id);
            let sev = if id < 0 { Severity::High } else { sev_dir(Severity::High) };
            out.push(mk(sev, Category::ExtraId, id, &e.name, None, pos, format!("Seihai sends {n} x {} the real client never sent", label(e))));
        }
    }
    // Fields compared as sets over the whole stream.
    for (d, id, key) in &opts.set_fields {
        if *d != dir {
            continue;
        }
        let vals = |evs: &[&Event]| -> std::collections::BTreeSet<String> {
            evs.iter().filter(|e| e.id == *id).filter_map(|e| e.field(key)).map(str::to_string).collect()
        };
        let (sa, sb) = (vals(&a), vals(&b));
        if sa == sb {
            continue;
        }
        let only_a: Vec<&String> = sa.difference(&sb).collect();
        let only_b: Vec<&String> = sb.difference(&sa).collect();
        let nm = a.iter().chain(b.iter()).find(|e| e.id == *id).map_or(String::new(), |e| e.name.clone());
        let list = |v: &[&String]| if v.is_empty() { "-".to_string() } else { v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ") };
        out.push(mk(
            sev_dir(Severity::Medium),
            Category::Field,
            *id,
            &nm,
            a.iter().position(|e| e.id == *id),
            b.iter().position(|e| e.id == *id),
            format!("{nm} ({id}) {key} values differ: only the real client: [{}]; only Seihai: [{}]", list(&only_a), list(&only_b)),
        ));
    }
    // Alignment of the collapsed runs.
    let (n, m) = (ra.len(), rb.len());
    let mut t = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i][j] = if ra[i].id == rb[j].id { t[i + 1][j + 1] + 1 } else { t[i + 1][j].max(t[i][j + 1]) };
        }
    }
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    let (mut un_a, mut un_b): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
    while i < n && j < m {
        if ra[i].id == rb[j].id {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if t[i + 1][j] >= t[i][j + 1] {
            un_a.push(i);
            i += 1;
        } else {
            un_b.push(j);
            j += 1;
        }
    }
    un_a.extend(i..n);
    un_b.extend(j..m);
    // Unmatched runs of ids both sides have: ordering. Aggregate per id.
    let mut order: BTreeMap<i32, (Vec<usize>, Vec<usize>, String)> = BTreeMap::new();
    for &i in &un_a {
        let r = &ra[i];
        if ids_b.contains_key(&r.id) {
            let e = order.entry(r.id).or_insert((vec![], vec![], r.first.name.clone()));
            e.0.push(r.pos);
        }
    }
    for &j in &un_b {
        let r = &rb[j];
        if ids_a.contains_key(&r.id) {
            let e = order.entry(r.id).or_insert((vec![], vec![], r.first.name.clone()));
            e.1.push(r.pos);
        }
    }
    for (id, (pa, pb, nm)) in order {
        let ctx = |runs: &[Run<'_>], pos: usize| -> String {
            let k = runs.iter().position(|r| r.pos == pos).unwrap_or(0);
            let before = k.checked_sub(1).map(|p| format!("{}", runs[p].first.name)).unwrap_or_else(|| "(start)".into());
            let after = runs.get(k + 1).map(|r| r.first.name.clone()).unwrap_or_else(|| "(end)".into());
            format!("between {before} and {after}")
        };
        let mut d = format!("{nm} ({id}) is placed differently: ");
        if let Some(&p) = pa.first() {
            let _ = write!(d, "real has it {} (message {p}) where Seihai has other messages; ", ctx(&ra, p));
        }
        if let Some(&p) = pb.first() {
            let _ = write!(d, "Seihai has it {} (message {p}) where the real client has other messages; ", ctx(&rb, p));
        }
        let _ = write!(d, "{} unmatched run(s) real, {} Seihai", pa.len(), pb.len());
        out.push(mk(sev_dir(Severity::Medium), Category::Order, id, &nm, pa.first().copied(), pb.first().copied(), d));
    }
    // Matched runs: fields and frame bundling.
    let mut seen_first_props = false;
    let mut bundling: BTreeMap<i32, (usize, usize, usize, String)> = BTreeMap::new(); // id -> (mismatches, total, first pos real, name)
    let mut field_reported: BTreeMap<(i32, String), usize> = BTreeMap::new();
    for &(i, j) in &pairs {
        let (x, y) = (&ra[i], &rb[j]);
        let first_props = dir == Dir::C2S && x.id == 2 && !seen_first_props;
        if dir == Dir::C2S && x.id == 2 {
            seen_first_props = true;
        }
        let entry = bundling.entry(x.id).or_insert((0, 0, x.pos, x.first.name.clone()));
        entry.1 += 1;
        if i > 0 && j > 0 && x.frame_new != y.frame_new {
            entry.0 += 1;
        }
        // Same-length runs of a message that is not a clock-driven walk update
        // are compared message by message; otherwise the first of each run.
        let pairwise = x.count == y.count && x.count <= 16 && !has(&opts.variable_count, dir, x.id);
        let n = if pairwise { x.count } else { 1 };
        let mut diffs: Vec<(String, Option<String>, Option<String>, usize)> = Vec::new();
        for m in 0..n {
            let fp = first_props && m == 0;
            for (k, va, vb) in field_diffs(x.events[m], y.events[m], dir, fp, opts) {
                diffs.push((k, va, vb, m));
            }
        }
        // A different prop set explains the per-prop lines that are absent on one side.
        let props_differ = diffs.iter().any(|d| d.0 == "props");
        for (k, va, vb, m) in diffs {
            if props_differ && k != "props" && (va.is_none() || vb.is_none()) {
                continue;
            }
            let cnt = field_reported.entry((x.id, k.clone())).or_default();
            *cnt += 1;
            if *cnt > 1 {
                continue; // report each field of each id once, with the first example
            }
            let sev = if x.id < 0 { Severity::High } else if dir == Dir::C2S { Severity::Medium } else { Severity::Low };
            let detail = if k == "props" {
                let set = |v: &Option<String>| -> Vec<String> {
                    v.as_deref().unwrap_or("").split(',').filter(|s| !s.is_empty()).map(str::to_string).collect()
                };
                let (sa, sb) = (set(&va), set(&vb));
                let oa: Vec<&String> = sa.iter().filter(|p| !sb.contains(p)).collect();
                let ob: Vec<&String> = sb.iter().filter(|p| !sa.contains(p)).collect();
                let j = |v: &[&String]| if v.is_empty() { "-".to_string() } else { v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(",") };
                if oa.is_empty() && ob.is_empty() {
                    format!("{} property order: real {} | Seihai {}", label(x.first), show(&va), show(&vb))
                } else {
                    format!("{} property set: only the real client [{}]; only Seihai [{}]", label(x.first), j(&oa), j(&ob))
                }
            } else {
                let at = if pairwise && x.count > 1 { format!(" (message {} of the run)", m + 1) } else { String::new() };
                format!("{} field {k}{at}: real {} | Seihai {}", label(x.first), show(&va), show(&vb))
            };
            out.push(mk(sev, Category::Field, x.id, &x.first.name, Some(x.pos), Some(y.pos), detail));
        }
    }
    for (id, (bad, total, pos, nm)) in bundling {
        if bad > 0 {
            out.push(mk(
                sev_dir(Severity::Medium),
                Category::Bundling,
                id,
                &nm,
                Some(pos),
                None,
                format!("{nm} ({id}) is bundled into frames differently from the real client in {bad} of {total} matched runs"),
            ));
        }
    }
    // Counts for ids both sides send, unless the count follows the clock.
    for (&id, &na) in &ids_a {
        let Some(&nb) = ids_b.get(&id) else { continue };
        if has(&opts.variable_count, dir, id) || has(&opts.periodic, dir, id) || na == nb {
            continue;
        }
        let e = a.iter().find(|e| e.id == id).unwrap();
        out.push(mk(Severity::Low, Category::Count, id, &e.name, a.iter().position(|e| e.id == id), b.iter().position(|e| e.id == id), format!("{} sent {na} x by the real client, {nb} x by Seihai", label(e))));
    }
    // Timing of steady cadences.
    for (&id, &na) in &ids_a {
        let Some(&nb) = ids_b.get(&id) else { continue };
        if na < 4 || nb < 4 {
            continue;
        }
        let (mut ia, mut ib) = (intervals(&a, id), intervals(&b, id));
        if has(&opts.variable_count, dir, id) {
            // Clock-driven bursts (walking): compare the cadence inside a
            // burst, not the idle gaps between bursts.
            ia.retain(|&v| v < 1000);
            ib.retain(|&v| v < 1000);
        }
        if ia.len() < 3 || ib.len() < 3 {
            continue;
        }
        let (ma, mb) = (median(&mut ia), median(&mut ib));
        let mean = ia.iter().sum::<i64>() as f64 / ia.len() as f64;
        let var = ia.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / ia.len() as f64;
        let steady = mean > 0.0 && var.sqrt() / mean < 0.25;
        let allow = opts.jitter_ms.max(ma / 5);
        if (steady || has(&opts.periodic, dir, id)) && (ma - mb).abs() > allow {
            let e = a.iter().find(|e| e.id == id).unwrap();
            out.push(mk(Severity::Low, Category::Timing, id, &e.name, None, None, format!("{} interval: real median {ma} ms, Seihai {mb} ms (allowed {allow} ms)", label(e))));
        }
    }
}

fn show(v: &Option<String>) -> String {
    match v {
        Some(s) if s.len() > 60 => format!("\"{}...\"", &s[..60]),
        Some(s) => format!("\"{s}\""),
        None => "(absent)".to_string(),
    }
}

fn is_pos_key(k: &str) -> bool {
    matches!(k, "X" | "Y" | "x" | "y")
}

/// Fields that differ between two matched messages, after the tolerances.
fn field_diffs(
    a: &Event,
    b: &Event,
    dir: Dir,
    first_props: bool,
    opts: &DiffOptions,
) -> Vec<(String, Option<String>, Option<String>)> {
    let mut keys: Vec<&str> = a.fields.iter().map(|(k, _)| k.as_str()).collect();
    for (k, _) in &b.fields {
        if !keys.contains(&k.as_str()) {
            keys.push(k);
        }
    }
    let walk = dir == Dir::C2S && a.id == 2;
    let props_as_set = opts.set_fields.iter().any(|(d, i, f)| *d == dir && *i == a.id && f == "props");
    let mut out = Vec::new();
    for k in keys {
        if props_as_set && a.field(k).is_none() != b.field(k).is_none() {
            continue; // a different prop set is reported once, as a set
        }
        if opts.ignore_fields.iter().any(|i| i == k) || opts.set_fields.iter().any(|(d, i, f)| *d == dir && *i == a.id && f == k) {
            continue;
        }
        let (va, vb) = (a.field(k), b.field(k));
        if va == vb {
            continue;
        }
        if is_pos_key(k) {
            // Later walk updates depend on the clock; the first one and every
            // other position get a tolerance.
            if walk && !first_props {
                continue;
            }
            if let (Some(x), Some(y)) = (va.and_then(|s| s.parse::<f32>().ok()), vb.and_then(|s| s.parse::<f32>().ok())) {
                if (x - y).abs() <= opts.pos_tol {
                    continue;
                }
            }
        }
        // Later walk updates carry whatever props changed; only the first
        // props message is a fair comparison of the prop set.
        if walk && !first_props && (k == "props" || k == "SPRITE") {
            continue;
        }
        out.push((k.to_string(), va.map(str::to_string), vb.map(str::to_string)));
    }
    out
}

impl Report {
    /// Divergences that count (not info).
    pub fn ranked(&self) -> impl Iterator<Item = &Divergence> {
        self.divergences.iter().filter(|d| d.severity != Severity::Info)
    }

    pub fn count(&self, s: Severity) -> usize {
        self.divergences.iter().filter(|d| d.severity == s).count()
    }

    pub fn to_tsv(&self) -> String {
        let mut s = String::from("rank\tseverity\tcategory\tconn\tdir\tid\tname\treal_pos\tseihai_pos\tdetail\n");
        for (i, d) in self.divergences.iter().enumerate() {
            let _ = writeln!(
                s,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                i + 1,
                d.severity.label(),
                d.category.label(),
                d.conn,
                d.dir.map_or("-", Dir::label),
                d.id.map_or("-".to_string(), |i| i.to_string()),
                d.name,
                d.real_pos.map_or("-".to_string(), |p| p.to_string()),
                d.seihai_pos.map_or("-".to_string(), |p| p.to_string()),
                d.detail.replace(['\t', '\n'], " ")
            );
        }
        s
    }

    pub fn to_text(&self) -> String {
        let meta = |m: &[(String, String)], k: &str| m.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_else(|| "?".into());
        let mut s = String::new();
        let _ = writeln!(
            s,
            "Differential report: version {} scenario {}\n  real trace: client={}  seihai trace: client={}",
            meta(&self.real_meta, "version"),
            meta(&self.real_meta, "scenario"),
            meta(&self.real_meta, "client"),
            meta(&self.seihai_meta, "client")
        );
        s.push_str("\nMessages per stream (real / Seihai)\n");
        for c in &self.streams {
            let _ = writeln!(s, "  {} {}: {} / {}", c.conn, c.dir.label(), c.real, c.seihai);
        }
        let _ = writeln!(
            s,
            "\nDivergences: {} high, {} medium, {} low ({} info)\n",
            self.count(Severity::High),
            self.count(Severity::Medium),
            self.count(Severity::Low),
            self.count(Severity::Info)
        );
        for (i, d) in self.divergences.iter().enumerate() {
            let _ = writeln!(
                s,
                "{:>3}. [{}] {} {} {}: {}",
                i + 1,
                d.severity.label(),
                d.category.label(),
                d.conn,
                d.dir.map_or("-", Dir::label),
                d.detail
            );
        }
        if self.divergences.is_empty() {
            s.push_str("  none: the two traces agree within the tolerances.\n");
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(dir: Dir, id: i32, name: &str, frame: u32, t: i64, fields: &[(&str, &str)]) -> Event {
        Event {
            conn: "gs0".into(),
            dir,
            t_ms: t,
            frame,
            id,
            name: name.into(),
            len: 1,
            fields: fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            raw: Vec::new(),
        }
    }

    fn tr(events: Vec<Event>) -> Trace {
        let mut t = Trace::default();
        t.set_meta("version", "2.31");
        t.events = events;
        t
    }

    fn c(id: i32, name: &str, frame: u32, t: i64, f: &[(&str, &str)]) -> Event {
        ev(Dir::C2S, id, name, frame, t, f)
    }

    fn base() -> Vec<Event> {
        vec![
            c(-1, "LOGIN", 0, 0, &[("token", "GNW28015")]),
            c(46, "ENTERLEVEL", 1, 10, &[("value", "1")]),
            c(2, "PLAYERPROPS", 1, 10, &[("X", "30"), ("Y", "30.5"), ("props", "X,Y")]),
            c(2, "PLAYERPROPS", 2, 300, &[("X", "31"), ("Y", "30.5"), ("props", "X,Y")]),
            c(6, "TOALL", 3, 900, &[("text", "hi")]),
        ]
    }

    #[test]
    fn identical_traces_have_no_divergence() {
        let r = diff(&tr(base()), &tr(base()), &DiffOptions::default());
        assert_eq!(r.ranked().count(), 0, "{}", r.to_text());
        assert!(r.to_text().contains("none"));
    }

    #[test]
    fn walk_count_and_positions_are_tolerated() {
        let mut b = base();
        b.insert(4, c(2, "PLAYERPROPS", 2, 500, &[("X", "40"), ("Y", "30.5"), ("props", "X,Y")]));
        b[2].fields[0].1 = "30.4".into(); // first props within tolerance
        let r = diff(&tr(base()), &tr(b), &DiffOptions::default());
        assert_eq!(r.ranked().count(), 0, "{}", r.to_text());
    }

    #[test]
    fn a_prop_set_used_by_one_side_only_in_later_walk_updates_is_reported() {
        let mut b = base();
        b.insert(4, c(2, "PLAYERPROPS", 2, 500, &[("X", "40"), ("Y", "30.5"), ("props", "X,Y,SPRITE")]));
        let r = diff(&tr(base()), &tr(b), &DiffOptions::default());
        assert!(r.ranked().any(|d| d.category == Category::Field && d.detail.contains("only Seihai: [X,Y,SPRITE]")), "{}", r.to_text());
    }

    #[test]
    fn walk_cadence_is_compared_with_a_relative_allowance() {
        let walk = |step: i64, n: i64| -> Vec<Event> {
            let mut v = vec![c(-1, "LOGIN", 0, 0, &[])];
            for k in 0..n {
                v.push(c(2, "PLAYERPROPS", 1 + k as u32, 100 + k * step, &[("X", "30"), ("props", "X")]));
            }
            v
        };
        let slow = diff(&tr(walk(50, 20)), &tr(walk(64, 20)), &DiffOptions::default());
        assert!(slow.ranked().any(|d| d.category == Category::Timing), "{}", slow.to_text());
        let same = diff(&tr(walk(50, 20)), &tr(walk(52, 20)), &DiffOptions::default());
        assert!(!same.ranked().any(|d| d.category == Category::Timing), "{}", same.to_text());
    }

    #[test]
    fn missing_and_extra_ids_are_high_on_the_client_side() {
        let mut b = base();
        b.remove(1); // no 46
        b.push(c(24, "SHOWIMGPLAYER", 4, 1000, &[("value", "9")]));
        let r = diff(&tr(base()), &tr(b), &DiffOptions::default());
        let cats: Vec<(Severity, Category, Option<i32>)> = r.ranked().map(|d| (d.severity, d.category, d.id)).collect();
        assert!(cats.contains(&(Severity::High, Category::MissingId, Some(46))), "{cats:?}");
        assert!(cats.contains(&(Severity::High, Category::ExtraId, Some(24))), "{cats:?}");
    }

    #[test]
    fn reordering_is_reported_with_context() {
        let a = vec![
            c(-1, "LOGIN", 0, 0, &[]),
            c(46, "ENTERLEVEL", 1, 10, &[]),
            c(2, "PLAYERPROPS", 1, 10, &[]),
            c(6, "TOALL", 2, 50, &[]),
        ];
        let b = vec![
            c(-1, "LOGIN", 0, 0, &[]),
            c(2, "PLAYERPROPS", 1, 10, &[]),
            c(6, "TOALL", 2, 50, &[]),
            c(46, "ENTERLEVEL", 3, 90, &[]),
        ];
        let r = diff(&tr(a), &tr(b), &DiffOptions::default());
        let d: Vec<&Divergence> = r.ranked().filter(|d| d.category == Category::Order).collect();
        assert_eq!(d.len(), 1, "{}", r.to_text());
        assert_eq!(d[0].id, Some(46));
        assert!(d[0].detail.contains("ENTERLEVEL"));
    }

    #[test]
    fn frame_bundling_difference_is_found() {
        let a = base();
        let mut b = base();
        b[2].frame = 2; // props split from the 46
        b[3].frame = 3;
        b[4].frame = 4;
        let r = diff(&tr(a), &tr(b), &DiffOptions::default());
        assert!(r.ranked().any(|d| d.category == Category::Bundling && d.id == Some(2)), "{}", r.to_text());
    }

    #[test]
    fn field_mismatch_names_both_values_and_masked_fields_are_equal() {
        let mut b = base();
        b[4].fields[0].1 = "bye".into();
        let r = diff(&tr(base()), &tr(b), &DiffOptions::default());
        let d = r.ranked().find(|d| d.category == Category::Field).unwrap();
        assert_eq!(d.id, Some(6));
        assert!(d.detail.contains("\"hi\"") && d.detail.contains("\"bye\""));
    }

    #[test]
    fn position_beyond_tolerance_on_the_first_props_is_a_field_mismatch() {
        let mut b = base();
        b[2].fields[0].1 = "35".into();
        let r = diff(&tr(base()), &tr(b), &DiffOptions::default());
        assert!(r.ranked().any(|d| d.category == Category::Field && d.detail.contains("field X")));
    }

    #[test]
    fn timer_messages_compare_by_interval_with_jitter() {
        let mk = |step: i64| -> Vec<Event> {
            let mut v = base();
            for k in 0..6 {
                v.push(c(39, "TAMPERCHECK", 10 + k, 1000 + step * i64::from(k), &[("len", "10")]));
            }
            v
        };
        let same = diff(&tr(mk(3200)), &tr(mk(3300)), &DiffOptions::default());
        assert_eq!(same.ranked().count(), 0, "{}", same.to_text());
        let off = diff(&tr(mk(3200)), &tr(mk(5100)), &DiffOptions::default());
        assert!(off.ranked().any(|d| d.category == Category::Timing && d.id == Some(39)));
        // A timer message that is not there at all is missing, not a timing issue.
        let gone = diff(&tr(mk(3200)), &tr(base()), &DiffOptions::default());
        assert!(gone.ranked().any(|d| d.category == Category::MissingId && d.id == Some(39)));
    }

    #[test]
    fn harness_noise_is_info_only() {
        let mut a = base();
        a.push(c(38, "TRIGGERACTION", 5, 2000, &[]));
        let r = diff(&tr(a), &tr(base()), &DiffOptions::default());
        assert_eq!(r.ranked().count(), 0);
        assert_eq!(r.count(Severity::Info), 1);
    }

    #[test]
    fn missing_connection_is_high() {
        let a = tr(base());
        let mut b = tr(base());
        for e in &mut b.events {
            e.conn = "ls0".into();
        }
        let r = diff(&a, &b, &DiffOptions::default());
        let cats: Vec<Category> = r.ranked().map(|d| d.category).collect();
        assert!(cats.contains(&Category::MissingConn) && cats.contains(&Category::ExtraConn));
        assert_eq!(r.divergences[0].severity, Severity::High);
    }

    #[test]
    fn server_side_differences_rank_below_client_ones() {
        let mut a = base();
        a.push(ev(Dir::S2C, 6, "LEVELNAME", 0, 20, &[("level", "a.nw")]));
        a.push(ev(Dir::S2C, 14, "PLAYERWARP", 1, 40, &[]));
        let mut b = base();
        b.remove(1);
        b.push(ev(Dir::S2C, 6, "LEVELNAME", 0, 20, &[("level", "a.nw")]));
        let r = diff(&tr(a), &tr(b), &DiffOptions::default());
        let ranked: Vec<&Divergence> = r.ranked().collect();
        assert_eq!(ranked[0].dir, Some(Dir::C2S));
        assert!(ranked.iter().any(|d| d.dir == Some(Dir::S2C) && d.severity < Severity::Info));
        let tsv = r.to_tsv();
        assert!(tsv.starts_with("rank\tseverity\t"));
        assert_eq!(tsv.lines().count(), r.divergences.len() + 1);
    }

    #[test]
    fn requested_files_are_compared_as_a_set() {
        let mut a = base();
        a.push(c(23, "WANTFILE", 5, 100, &[("file", "head0.png")]));
        a.push(c(23, "WANTFILE", 5, 100, &[("file", "body.png")]));
        let mut b = base();
        b.push(c(23, "WANTFILE", 5, 100, &[("file", "body.png")]));
        b.push(c(23, "WANTFILE", 5, 100, &[("file", "lv_a.nw")]));
        let r = diff(&tr(a), &tr(b), &DiffOptions::default());
        let d = r.ranked().find(|d| d.category == Category::Field && d.id == Some(23)).unwrap();
        assert!(d.detail.contains("only the real client: [head0.png]"), "{}", d.detail);
        assert!(d.detail.contains("only Seihai: [lv_a.nw]"), "{}", d.detail);
        // Same set in another order: no divergence.
        let mut a2 = base();
        a2.push(c(23, "WANTFILE", 5, 100, &[("file", "x.png")]));
        a2.push(c(23, "WANTFILE", 5, 100, &[("file", "y.png")]));
        let mut b2 = base();
        b2.push(c(23, "WANTFILE", 5, 100, &[("file", "y.png")]));
        b2.push(c(23, "WANTFILE", 5, 100, &[("file", "x.png")]));
        assert_eq!(diff(&tr(a2), &tr(b2), &DiffOptions::default()).ranked().count(), 0);
    }

    #[test]
    fn runs_of_equal_length_are_compared_message_by_message() {
        let adj = |names: &[&str]| -> Vec<Event> {
            let mut v = base();
            for n in names {
                v.insert(2, c(35, "ADJACENTLEVEL", 1, 10, &[("level", n)]));
            }
            v
        };
        let r = diff(&tr(adj(&["a.nw", "b.nw", "c.nw"])), &tr(adj(&["c.nw", "b.nw", "a.nw"])), &DiffOptions::default());
        let d = r.ranked().find(|d| d.category == Category::Field && d.id == Some(35)).unwrap();
        assert!(d.detail.contains("message 1 of the run"), "{}", d.detail);
        // A different count is a count difference, not a field mismatch.
        let r = diff(&tr(adj(&["a.nw", "b.nw"])), &tr(adj(&["a.nw", "b.nw", "c.nw"])), &DiffOptions::default());
        assert!(r.ranked().any(|d| d.category == Category::Count && d.id == Some(35)));
    }

    #[test]
    fn a_different_prop_set_is_one_finding_not_one_per_prop() {
        let a = vec![
            c(-1, "LOGIN", 0, 0, &[]),
            c(2, "PLAYERPROPS", 1, 10, &[("props", "X2,Y2"), ("X2", "30"), ("Y2", "30.5")]),
        ];
        let b = vec![
            c(-1, "LOGIN", 0, 0, &[]),
            c(2, "PLAYERPROPS", 1, 10, &[("props", "X,Y,SPRITE"), ("X", "30"), ("Y", "30.5"), ("SPRITE", "3 (dir 3)")]),
        ];
        let r = diff(&tr(a), &tr(b), &DiffOptions::default());
        let f: Vec<&Divergence> = r.ranked().filter(|d| d.category == Category::Field).collect();
        assert_eq!(f.len(), 1, "{}", r.to_text());
        assert!(f[0].detail.contains("only the real client: [X2,Y2]") && f[0].detail.contains("only Seihai: [X,Y,SPRITE]"), "{}", f[0].detail);
    }
}
