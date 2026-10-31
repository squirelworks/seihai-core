//! Viewer tests. Every capture here is synthetic (built in memory); no real
//! capture is read or committed.

use super::cli;
use super::render::{printable, render_layouts, render_view, Format};
use super::tables::{lint, parse_chain, parse_layouts, Enc, Tables};
use super::*;
use crate::difftest::pcap::tests::synth_pcap;
use crate::protocol::{write_frame, write_zlib_frame, Gen5, Gen5Comp};

fn frame5(cipher: &mut Gen5, msgs: &[&[u8]]) -> Vec<u8> {
    let mut body = Vec::new();
    for m in msgs {
        body.extend_from_slice(m);
    }
    let comp = crate::protocol::choose_comp(body.len());
    let c = match comp {
        Gen5Comp::Zlib => crate::protocol::zlib_compress(&body).unwrap(),
        Gen5Comp::Bz2 => crate::protocol::bz2_compress(&body).unwrap(),
        Gen5Comp::None => body,
    };
    write_frame(comp, &cipher.encode(c, comp))
}

fn login_line(key: u8, acct: &str) -> Vec<u8> {
    let mut l = vec![0x20, key + 0x20];
    l.extend_from_slice(b"GNW28015");
    l.push(acct.len() as u8 + 0x20);
    l.extend_from_slice(acct.as_bytes());
    l.push(10 + 0x20);
    l.extend_from_slice(b"s3cretword");
    l.push(b'\n');
    l
}

/// A 2.31 session: login, PLI 46 + props + PLI 24, then server messages that
/// the decoder names (6, 8), that only a table can read (170), that nothing
/// reads (120) and one with a stray byte (170 again with a tail).
fn session(key: u8) -> Vec<u8> {
    let login = write_zlib_frame(&login_line(key, "dtacct")).unwrap();
    let mut cc = Gen5::new(key);
    let mut sc = Gen5::new(key);
    let mut props = vec![0x22, 0x2f, 0x20 + 61, 0x30, 0x20 + 24, 0x31, 0x22, 0x2c, 0x20 + 9];
    props.extend_from_slice(b"hi dtacct");
    props.push(b'\n');
    let c1 = frame5(&mut cc, &[&[46 + 0x20, 1 + 0x20, b'\n'], &props, &[24 + 0x20, 9 + 0x20, b'\n']]);
    let c2 = frame5(&mut cc, &[&[0x20, 0x20 + 20, 0x20 + 8, b'l', b'v', b'_', b'b', b'.', b'n', b'w', b'\n']]);
    let mut m170 = vec![170 + 0x20, 0x21, b'\n'];
    let mut m170b = vec![170 + 0x20, 0x21, 0x41, 0x42, b'\n'];
    let m120: Vec<u8> = {
        let mut v = vec![120 + 0x20];
        v.extend_from_slice(b"\x01\x02abc");
        v.push(b'\n');
        v
    };
    let s1 = frame5(&mut sc, &[b"&lv_a.nw\n", &[0x28, 0x20, 0x20 + 77, 0x2f, 0x20 + 20, b'\n'], &mut m170, &m120]);
    let s2 = frame5(&mut sc, &[&mut m170b]);
    synth_pcap(
        41000,
        14802,
        &[
            (10.0, true, 0x02, 1, b""),
            (10.001, false, 0x12, 1, b""),
            (10.002, true, 0x18, 2, &login),
            (10.050, true, 0x18, 2 + login.len() as u32, &c1),
            (10.060, false, 0x18, 2, &s1),
            (10.400, true, 0x18, 2 + (login.len() + c1.len()) as u32, &c2),
            (10.500, false, 0x18, 2 + s1.len() as u32, &s2),
        ],
    )
}

fn view_of(data: &[u8], version: Option<ClientVersion>, f: Filter, layouts: bool, raw: bool) -> View {
    let loaded = load(data, version, true).unwrap();
    let opts = ViewOptions { filter: f, show_layout: layouts, raw_all: raw, mask: true };
    build_view(&loaded, &Tables::builtin(), &opts)
}

fn names(v: &View) -> Vec<(String, i32, String)> {
    v.rows.iter().map(|r| (r.event.dir.label().to_string(), r.event.id, r.event.name.clone())).collect()
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("seihai-viewer-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

use std::path::PathBuf;

#[test]
fn version_is_detected_from_the_login_token() {
    let l = load(&session(0x2b), None, true).unwrap();
    assert_eq!(l.version, Some(ClientVersion::V231));
    assert!(l.version_note.contains("auto-detected"));
    // The same bytes with an explicit version.
    let l = load(&session(0x2b), Some(ClientVersion::V231), true).unwrap();
    assert_eq!(l.version_note, "from --version");
}

#[test]
fn detect_fails_cleanly_when_nothing_matches() {
    let pcap = synth_pcap(41001, 14802, &[(1.0, true, 0x02, 1, b""), (1.1, false, 0x12, 1, b"")]);
    let e = load(&pcap, None, true).err().unwrap();
    assert!(e.contains("no TCP data"), "{e}");
    // Data that matches no build.
    let junk = synth_pcap(41001, 14802, &[(1.0, true, 0x02, 1, b""), (1.1, true, 0x18, 2, b"\x00\x02hi")]);
    let e = load(&junk, None, true).err().unwrap();
    assert!(e.contains("pass --version"), "{e}");
}

#[test]
fn show_prints_every_message_with_names_and_decoded_fields() {
    let v = view_of(&session(0x2b), None, Filter::default(), false, false);
    let n = names(&v);
    assert_eq!(n[0], ("C>S".into(), ID_LOGIN, "LOGIN".into()));
    assert!(n.contains(&("C>S".into(), 46, "ENTERLEVEL".into())));
    assert!(n.contains(&("S>C".into(), 6, "LEVELNAME".into())));
    assert!(n.contains(&("C>S".into(), 0, "LEVELWARP".into())));
    let warp = v.rows.iter().find(|r| r.event.id == 0 && r.event.dir == Dir::C2S).unwrap();
    assert_eq!(warp.source, Source::Decoder);
    assert_eq!(warp.event.field("x"), Some("10"));
    assert_eq!(warp.event.field("y"), Some("4"));
    assert_eq!(warp.event.field("level"), Some("lv_b.nw"));
    assert_eq!(v.version, Some(ClientVersion::V231));
}

#[test]
fn login_is_redacted_and_never_carries_raw_bytes() {
    let v = view_of(&session(0x2b), None, Filter::default(), true, true);
    assert!(v.has_login);
    for fmt in [Format::Text, Format::Tsv, Format::Json] {
        let out = render_view(&v, fmt, "x.pcap", "built-in", true);
        assert!(!out.contains("s3cretword"), "{fmt:?} leaked the password");
        assert!(!out.contains("dtacct"), "{fmt:?} leaked the account");
        assert!(out.contains("<PASS>"));
        assert!(out.contains("<ACCT>"));
    }
    let login = v.rows.iter().find(|r| r.event.id == ID_LOGIN).unwrap();
    assert!(login.raw.is_empty());
    assert!(login.event.raw.is_empty());
    // Hex of the password bytes must not appear either.
    let out = render_view(&v, Format::Text, "x.pcap", "built-in", true);
    assert!(!out.contains("73336372657477"), "hex of the password");
}

#[test]
fn no_mask_keeps_the_account_but_still_hides_the_password() {
    let loaded = load(&session(0x2b), None, false).unwrap();
    let opts = ViewOptions { filter: Filter::default(), show_layout: true, raw_all: true, mask: false };
    let v = build_view(&loaded, &Tables::builtin(), &opts);
    let out = render_view(&v, Format::Text, "x.pcap", "built-in", true);
    assert!(out.contains("dtacct"));
    assert!(!out.contains("s3cretword"));
    assert!(out.contains("password=<PASS>"));
}

#[test]
fn filters_by_direction_id_name_time_frame_conn_and_limit() {
    let data = session(0x2b);
    let all = view_of(&data, None, Filter::default(), false, false);
    let c2s = view_of(&data, None, Filter { dir: Some(Dir::C2S), ..Filter::default() }, false, false);
    assert!(c2s.rows.iter().all(|r| r.event.dir == Dir::C2S));
    assert!(c2s.rows.len() < all.rows.len());
    let ids = view_of(&data, None, Filter { sel: vec![Sel::parse("46"), Sel::parse("plo_6")], ..Filter::default() }, false, false);
    assert_eq!(names(&ids).len(), 2);
    let by_name = view_of(&data, None, Filter { sel: vec![Sel::parse("levelwarp")], ..Filter::default() }, false, false);
    assert_eq!(names(&by_name), vec![("C>S".to_string(), 0, "LEVELWARP".to_string())]);
    let early = view_of(&data, None, Filter { time: Range::parse("..100").unwrap(), ..Filter::default() }, false, false);
    assert!(early.rows.iter().all(|r| r.event.t_ms <= 100));
    assert!(early.rows.len() < all.rows.len());
    let late = view_of(&data, None, Filter { time: Range::parse("300..").unwrap(), ..Filter::default() }, false, false);
    assert!(late.rows.iter().all(|r| r.event.t_ms >= 300) && !late.rows.is_empty());
    let f1 = view_of(&data, None, Filter { frame: Range::parse("1").unwrap(), dir: Some(Dir::C2S), ..Filter::default() }, false, false);
    assert!(f1.rows.iter().all(|r| r.event.frame == 1));
    assert!(f1.rows.len() >= 3);
    let none = view_of(&data, None, Filter { conn: Some("gs9".into()), ..Filter::default() }, false, false);
    assert!(none.rows.is_empty());
    let lim = view_of(&data, None, Filter { limit: Some(2), ..Filter::default() }, false, false);
    assert_eq!(lim.rows.len(), 2);
    assert_eq!(lim.total, all.total);
}

#[test]
fn ranges_parse_and_reject_junk() {
    assert_eq!(Range::parse("5").unwrap(), Range { lo: Some(5), hi: Some(5) });
    assert_eq!(Range::parse("2..").unwrap(), Range { lo: Some(2), hi: None });
    assert_eq!(Range::parse("..9").unwrap(), Range { lo: None, hi: Some(9) });
    assert!(Range::parse("a..b").is_err());
    assert!(Range::parse("1..2").unwrap().contains(2));
    assert!(!Range::parse("1..2").unwrap().contains(3));
}

#[test]
fn a_table_reads_what_the_decoder_leaves_raw_and_marks_the_tail() {
    let v = view_of(&session(0x2b), None, Filter { sel: vec![Sel::parse("170")], ..Filter::default() }, true, false);
    assert_eq!(v.rows.len(), 2);
    let a = &v.rows[0];
    assert_eq!(a.source, Source::Table);
    assert_eq!(a.fields, vec![("on".to_string(), "1".to_string())]);
    assert_eq!(a.layout.as_deref(), Some("on:gchar"));
    assert!(a.unknown.is_empty());
    assert!(a.raw.is_empty(), "a fully read message needs no hex");
    let b = &v.rows[1];
    assert_eq!(b.fields[1], ("?tail".to_string(), "4142".to_string()));
    assert!(!b.unknown.is_empty());
    assert_eq!(b.raw, vec![0x21, 0x41, 0x42], "a message with leftover bytes shows its hex");
}

/// Id 24's byte 9 is the full-list marker with no images; 46 is a literal 1.
#[test]
fn id_24_shows_the_marker_and_46_is_settled() {
    let v = view_of(&session(0x2b), None, Filter { sel: vec![Sel::parse("24")], dir: Some(Dir::C2S), ..Filter::default() }, true, false);
    let r = &v.rows[0];
    assert_eq!(r.source, Source::Decoder);
    assert_eq!(r.fields[0], ("full".to_string(), "yes".to_string()));
    assert_eq!(r.fields[1], ("images".to_string(), "(none)".to_string()));
    assert!(r.unknown.is_empty(), "{:?}", r.unknown);
    assert_eq!(r.layout.as_deref(), Some("images:showimg"));
    let v = view_of(&session(0x2b), None, Filter { sel: vec![Sel::parse("46")], dir: Some(Dir::C2S), ..Filter::default() }, true, false);
    assert!(v.rows[0].unknown.is_empty(), "{:?}", v.rows[0].unknown);
}

#[test]
fn an_id_with_no_layout_shows_raw_hex_and_says_so() {
    let v = view_of(&session(0x2b), None, Filter { sel: vec![Sel::parse("120")], ..Filter::default() }, true, false);
    let r = &v.rows[0];
    assert_eq!(r.source, Source::Raw);
    assert_eq!(r.layout, None);
    assert_eq!(r.raw, b"\x01\x02abc".to_vec());
    assert!(r.unknown.iter().any(|u| u.contains("no layout")));
    let text = render_view(&v, Format::Text, "x.pcap", "built-in", true);
    assert!(text.contains("raw 0000  01 02 61 62 63"), "{text}");
    assert!(text.contains("|..abc|"));
    assert!(text.contains("layout: (not in the tables for this version)"));
}

/// The built-in tables with id 24 replaced by a one-field row whose status
/// is unknown (id 24 itself is settled now; the test needs an unknown field
/// on an id the trace decoder names).
fn table_with_unknown_24() -> Tables {
    let mut t = Tables::builtin();
    t.rows.retain(|r| !(r.dir == Dir::C2S && r.id == 24));
    t.rows.extend(
        parse_layouts("dir\tid\tname\tidx\tfield\tencoding\tfrom\tto\tstatus\tnote\nC>S\t24\tSHOWIMGPLAYER\t1\tvalue\tgchar\t-\t-\tunknown\tn\n")
            .unwrap(),
    );
    t
}

#[test]
fn unknown_fields_in_the_table_are_marked_even_when_the_decoder_named_them() {
    let loaded = load(&session(0x2b), None, true).unwrap();
    let opts = ViewOptions {
        filter: Filter { sel: vec![Sel::parse("24")], dir: Some(Dir::C2S), ..Filter::default() },
        show_layout: true,
        raw_all: false,
        mask: true,
    };
    let v = build_view(&loaded, &table_with_unknown_24(), &opts);
    let r = &v.rows[0];
    assert_eq!(r.source, Source::Decoder);
    assert!(r.unknown.iter().any(|u| u.contains("`value` is unknown")));
    assert_eq!(r.layout.as_deref(), Some("?value:gchar"));
}

#[test]
fn raw_flag_dumps_every_payload_except_logins() {
    let v = view_of(&session(0x2b), None, Filter::default(), false, true);
    let with_raw = v.rows.iter().filter(|r| !r.raw.is_empty()).count();
    let numbered_with_payload = v.rows.iter().filter(|r| r.event.id >= 0 && r.event.len > 0).count();
    assert_eq!(with_raw, numbered_with_payload);
    assert!(v.rows.iter().filter(|r| r.event.id < 0).all(|r| r.raw.is_empty()));
}

#[test]
fn json_is_valid_and_tsv_has_a_fixed_column_count() {
    let v = view_of(&session(0x2b), None, Filter::default(), true, true);
    let j = render_view(&v, Format::Json, "x.pcap", "built-in", true);
    let doc: serde_json::Value = serde_json::from_str(&j).unwrap_or_else(|e| panic!("{e}: {j}"));
    assert_eq!(doc["version"], "2.31");
    assert_eq!(doc["messages"].as_array().unwrap().len(), v.rows.len());
    assert_eq!(doc["has_login"], true);
    let m = &doc["messages"][0];
    assert_eq!(m["name"], "LOGIN");
    assert_eq!(m["raw_hex"], "");
    let t = render_view(&v, Format::Tsv, "x.pcap", "built-in", true);
    let rows: Vec<&str> = t.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(rows.len(), v.rows.len());
    assert!(rows.iter().all(|l| l.split('\t').count() == 12), "{t}");
    // Empty result is still valid json.
    let e = view_of(&session(0x2b), None, Filter { conn: Some("nope".into()), ..Filter::default() }, false, false);
    let j = render_view(&e, Format::Json, "x", "b", false);
    assert!(serde_json::from_str::<serde_json::Value>(&j).is_ok());
}

#[test]
fn control_characters_cannot_reach_the_terminal() {
    assert_eq!(printable("a\x1b[31mb\n"), "a\\x1b[31mb\\x0a");
    assert_eq!(printable("caf\u{e9}"), "caf\\xe9");
}

#[test]
fn a_trace_file_is_read_with_its_header_version() {
    let loaded = load(&session(0x2b), None, true).unwrap();
    let text = loaded.trace.to_tsv();
    let l2 = load(text.as_bytes(), None, true).unwrap();
    assert_eq!(l2.kind, InputKind::TraceTsv);
    assert_eq!(l2.version, Some(ClientVersion::V231));
    let opts = ViewOptions { filter: Filter { sel: vec![Sel::parse("6")], ..Filter::default() }, show_layout: true, raw_all: true, mask: true };
    let v = build_view(&l2, &Tables::builtin(), &opts);
    assert_eq!(v.rows.len(), 1);
    assert!(v.rows[0].raw.is_empty(), "a trace holds no payload bytes");
    assert_eq!(v.rows[0].layout.as_deref(), Some("level:text"));
}

#[test]
fn a_trace_without_a_version_needs_one() {
    let mut t = load(&session(0x2b), None, true).unwrap().trace;
    t.meta.retain(|(k, _)| k != "version");
    let l = load(t.to_tsv().as_bytes(), None, true).unwrap();
    assert_eq!(l.version, None);
    let l = load(t.to_tsv().as_bytes(), Some(ClientVersion::V222), true).unwrap();
    assert_eq!(l.version, Some(ClientVersion::V222));
}

// ---- tables ---------------------------------------------------------------

#[test]
fn built_in_tables_parse_and_lint_clean() {
    let t = Tables::builtin();
    assert!(t.rows.len() > 80);
    let problems = lint(&t);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn table_rows_only_cover_versions_whose_profile_has_the_id() {
    // A layout for a build that does not handle the id would be a table error.
    let t = Tables::builtin();
    let mut bad = Vec::new();
    for r in &t.rows {
        for &v in ClientVersion::all_versions() {
            if r.applies_to(v) && !handled(v, r.dir, r.id) {
                bad.push(format!("{} {} {} in {}", r.dir.label(), r.id, r.name, v.label()));
            }
        }
    }
    bad.dedup();
    // Rows without a version limit (`-`) may run past the builds that carry
    // the id; only limited rows are held to the profiles.
    let limited_bad: Vec<&String> = bad
        .iter()
        .filter(|b| t.rows.iter().any(|r| format!("{} {} {}", r.dir.label(), r.id, r.name) == b.rsplit_once(" in ").unwrap().0 && (r.from.is_some() || r.to.is_some())))
        .collect();
    assert!(limited_bad.is_empty(), "{limited_bad:#?}");
}

#[test]
fn file_modtime_row_follows_the_profile_flag() {
    let t = Tables::builtin();
    for &v in ClientVersion::all_versions() {
        if !handled(v, Dir::S2C, 102) {
            continue;
        }
        let has = t.rows_for(Dir::S2C, 102, Some(v)).iter().any(|r| r.field == "modtime");
        assert_eq!(has, v.file_has_modtime(), "PLO_FILE modtime row for {}", v.label());
    }
}

#[test]
fn a_bad_row_is_an_error_with_a_line_number() {
    let head = "dir\tid\tname\tidx\tfield\tencoding\tfrom\tto\tstatus\tnote\n";
    let e = parse_layouts(&format!("{head}S>C\t1\tA\t1\tf\tgchar\t-\t-\tknown\t\nS>C\tx\tA\t1\tf\tgchar\t-\t-\tknown\t\n")).unwrap_err();
    assert!(e.starts_with("line 3:"), "{e}");
    assert!(parse_layouts(&format!("{head}S>C\t1\tA\t1\tf\twhat\t-\t-\tknown\t\n")).unwrap_err().contains("encoding"));
    assert!(parse_layouts(&format!("{head}S>C\t1\tA\t1\tf\tgchar\t9.99\t-\tknown\t\n")).unwrap_err().contains("unknown from version"));
    assert!(parse_layouts(&format!("{head}S>C\t1\tA\t1\tf\tgchar\t2.31\t2.17\tknown\t\n")).unwrap_err().contains("after"));
    assert!(parse_layouts("S>C\t1\n").is_err());
    assert!(parse_layouts("").is_err());
}

#[test]
fn tables_dir_replaces_the_layouts_and_adds_the_chain() {
    let d = tmp("tables");
    std::fs::write(
        d.join("packet-layouts.tsv"),
        "dir\tid\tname\tidx\tfield\tencoding\tfrom\tto\tstatus\tnote\nS>C\t120\tMYPACKET\t1\tcount\tgchar\t-\t-\tknown\t\nS>C\t120\tMYPACKET\t2\tlabel\ttext\t-\t-\tpartial\t\n",
    )
    .unwrap();
    std::fs::write(
        d.join("version-chain.tsv"),
        "step\tlabel\ttoken\tbasis\tkind\tarea\tfield\tdetail\n8\t1.42\tGNW08011\tbinary\tchanged\tprotocol\tplo (server ids handled)\t+42 NEWWORLDTIME, +44 HASNPCSERVER (45 -> 51 ids)\n",
    )
    .unwrap();
    let t = Tables::load_dir(&d).unwrap();
    assert_eq!(t.rows.len(), 2, "the file replaces the built-in layouts");
    assert_eq!(t.chain.len(), 1);
    let loaded = load(&session(0x2b), None, true).unwrap();
    let opts = ViewOptions { filter: Filter { sel: vec![Sel::parse("120")], ..Filter::default() }, show_layout: true, raw_all: false, mask: true };
    let v = build_view(&loaded, &t, &opts);
    assert_eq!(v.rows[0].source, Source::Table);
    assert_eq!(v.rows[0].fields[0], ("count".to_string(), "225".to_string()));
    assert_eq!(v.rows[0].fields[1].0, "label");
    let rep = layout_report(&t, Dir::S2C, 44);
    assert_eq!(rep.chain.len(), 1);
    assert!(Tables::load_dir(&d.join("missing")).is_err());
    // A folder with neither file keeps the built-ins.
    let e = tmp("empty");
    assert_eq!(Tables::load_dir(&e).unwrap().rows.len(), Tables::builtin().rows.len());
    assert!(!Tables::load_dir(&e).unwrap().chain.is_empty(), "the built-in chain stays");
    let _ = (std::fs::remove_dir_all(&d), std::fs::remove_dir_all(&e));
}

#[test]
fn chain_details_name_ids() {
    assert!(detail_names_id("+43 DEFAULTWEAPON, -25 UNKNOWN25 (32 -> 33 ids)", 25));
    assert!(detail_names_id("+43 DEFAULTWEAPON, -25 UNKNOWN25 (32 -> 33 ids)", 43));
    assert!(!detail_names_id("+43 DEFAULTWEAPON, -25 UNKNOWN25 (32 -> 33 ids)", 33));
    assert!(!detail_names_id("(32 -> 33 ids)", 32));
    assert_eq!(parse_chain("step\tlabel\ttoken\tbasis\tkind\tarea\tfield\tdetail\n0\t1.33\tT\tbinary\torigin\t-\t-\tx\n").unwrap().len(), 1);
    assert!(parse_chain("nope\n").is_err());
}

// ---- layout explorer --------------------------------------------------------

#[test]
fn layout_report_follows_the_chain_and_marks_versions_without_a_layout() {
    let t = Tables::builtin();
    let rep = layout_report(&t, Dir::S2C, 49);
    assert_eq!(rep.name, "PLAYERWARP2");
    // The 3.x to 6.x builds handle 49, but their field layouts are not
    // written up yet.
    let last = rep.runs.last().unwrap();
    assert_eq!((last.from, last.to), ("3.04", "6.03r7"));
    assert!(last.handled && last.rows.is_empty());
    let gen2 = &rep.runs[rep.runs.len() - 2];
    assert_eq!(gen2.to, "2.31");
    assert!(gen2.handled);
    assert_eq!(gen2.rows.len(), 6);
    assert_eq!(rep.runs[0].from, "1.21");
    assert!(!rep.runs[0].handled);
    assert!(rep.changes.iter().any(|c| c.starts_with("added in")), "{:?}", rep.changes);
    // The built-in chain agrees: one of its protocol rows names +49 at the step the profiles add it.
    assert!(!rep.chain.is_empty());
    let added = rep.changes.iter().find_map(|c| c.strip_prefix("added in ")).unwrap();
    assert!(rep.chain.iter().any(|c| c.label == added && c.detail.contains("+49")), "{:?}", rep.chain);
    // Consecutive versions with the same state are one run.
    assert!(rep.runs.len() < ClientVersion::all_versions().len());
    let text = render_layouts(&[rep], Format::Text);
    assert!(text.contains("S>C 49  PLAYERWARP2"), "{text}");
    assert!(text.contains("halftile"));
    assert!(text.contains("not handled"));
    assert!(text.contains("one line:"));
}

#[test]
fn layout_differs_between_versions_where_the_table_says_so() {
    let t = Tables::builtin();
    let rep = layout_report(&t, Dir::S2C, 102);
    let with: Vec<&Run> = rep.runs.iter().filter(|r| r.rows.iter().any(|f| f.field == "modtime")).collect();
    let without: Vec<&Run> = rep.runs.iter().filter(|r| r.handled && !r.rows.iter().any(|f| f.field == "modtime")).collect();
    assert!(!with.is_empty() && !without.is_empty());
    assert_eq!(with[0].from, "2.1");
    // 2.05 has a different LEVELMODTIME layout from 2.17.
    let rep = layout_report(&t, Dir::S2C, 39);
    let a = rep.runs.iter().find(|r| r.from == "2.05").unwrap();
    assert_eq!(a.rows[0].enc, Enc::Gint5);
    let b = rep.runs.iter().find(|r| r.to == "2.31").unwrap();
    assert_eq!(b.rows[0].enc, Enc::Raw);
}

#[test]
fn layout_targets_resolve_by_id_and_by_name() {
    let t = Tables::builtin();
    assert_eq!(resolve_layout_target(&t, "49", None), vec![(Dir::S2C, 49)]);
    let toall = resolve_layout_target(&t, "TOALL", None);
    assert!(toall.contains(&(Dir::C2S, 6)) && toall.contains(&(Dir::S2C, 13)));
    assert_eq!(resolve_layout_target(&t, "plo_playerwarp2", None), vec![(Dir::S2C, 49)]);
    assert_eq!(resolve_layout_target(&t, "6", Some(Dir::C2S)), vec![(Dir::C2S, 6)]);
    assert!(resolve_layout_target(&t, "NOSUCHPACKET", None).is_empty());
    assert!(resolve_layout_target(&t, "254", None).is_empty());
}

#[test]
fn layout_formats_are_valid() {
    let t = table_with_unknown_24();
    let reps: Vec<_> = resolve_layout_target(&t, "24", None).into_iter().map(|(d, i)| layout_report(&t, d, i)).collect();
    assert!(!reps.is_empty());
    let j = render_layouts(&reps, Format::Json);
    let doc: serde_json::Value = serde_json::from_str(&j).unwrap_or_else(|e| panic!("{e}: {j}"));
    assert!(doc.as_array().unwrap().len() >= 1);
    let tsv = render_layouts(&reps, Format::Tsv);
    assert!(tsv.lines().all(|l| l.split('\t').count() == 11), "{tsv}");
    // Packet 24 in the client direction has the unknown `value` field (in
    // this test's table).
    let c = reps.iter().find(|r| r.dir == Dir::C2S).unwrap();
    assert!(render_layouts(&[c.clone()], Format::Text).contains("? 1  value"), "{}", render_layouts(&[c.clone()], Format::Text));
}

// ---- output file safety -----------------------------------------------------

#[cfg(unix)]
#[test]
fn output_files_are_private_and_loose_ones_are_refused_when_there_is_a_login() {
    use std::os::unix::fs::PermissionsExt;
    let d = tmp("out");
    let p = d.join("o.txt");
    write_private(&p, "x").unwrap();
    assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(check_output_mode(&p, true).is_ok());
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    let e = check_output_mode(&p, true).unwrap_err();
    assert!(e.contains("644") && e.contains("looser than 600"), "{e}");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert!(check_output_mode(&p, true).is_err());
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o400)).unwrap();
    assert!(check_output_mode(&p, true).is_ok());
    // No login in the capture: the mode is not checked.
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(check_output_mode(&p, false).is_ok());
    // A file that does not exist yet is fine.
    assert!(check_output_mode(&d.join("new"), true).is_ok());
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(unix)]
#[test]
fn cli_show_writes_a_private_file_and_refuses_a_loose_one() {
    use std::os::unix::fs::PermissionsExt;
    let d = tmp("cli");
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();
    let cap = d.join("s.pcap");
    std::fs::write(&cap, session(0x2b)).unwrap();
    let out = d.join("out.txt");
    let args = |extra: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = vec!["show".into(), cap.display().to_string(), "--out".into(), out.display().to_string()];
        v.extend(extra.iter().map(|s| s.to_string()));
        v
    };
    assert_eq!(cli::run(&args(&["--layouts"])), 0);
    assert_eq!(std::fs::metadata(&out).unwrap().permissions().mode() & 0o777, 0o600);
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains("LEVELWARP") && !text.contains("s3cretword"));
    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::write(&out, "keep").unwrap();
    assert_eq!(cli::run(&args(&[])), 1);
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "keep", "a refused write leaves the file alone");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn cli_exit_codes() {
    let s = |v: &[&str]| -> Vec<String> { v.iter().map(|x| x.to_string()).collect() };
    assert_eq!(cli::run(&s(&[])), 2);
    assert_eq!(cli::run(&s(&["show"])), 2);
    assert_eq!(cli::run(&s(&["show", "a", "b"])), 2);
    assert_eq!(cli::run(&s(&["show", "x", "--format", "xml"])), 2);
    assert_eq!(cli::run(&s(&["show", "x", "--dir", "up"])), 2);
    assert_eq!(cli::run(&s(&["show", "x", "--time"])), 2);
    assert_eq!(cli::run(&s(&["show", "/nonexistent/none.pcap"])), 1);
    assert_eq!(cli::run(&s(&["layout"])), 2);
    assert_eq!(cli::run(&s(&["layout", "49", "--version", "9.9"])), 2);
    assert_eq!(cli::run(&s(&["layout", "NOSUCHPACKET"])), 1);
    assert_eq!(cli::run(&s(&["layout", "49", "--version", "2.31"])), 0);
    assert_eq!(cli::run(&s(&["tables"])), 0);
    assert_eq!(cli::run(&s(&["help"])), 0);
}

#[test]
fn listserver_requests_are_redacted_and_keep_no_raw() {
    // A gen-5 listserver hello, then a server-list request with account and password.
    let mut hs = vec![7 + 0x20, 0x2b + 0x20];
    hs.extend_from_slice(b"GNW30123newmain\n");
    let f1 = write_zlib_frame(&hs).unwrap();
    let mut cc = Gen5::new(0x2b);
    let mut req = vec![1 + 0x20, 6 + 0x20];
    req.extend_from_slice(b"dtacct");
    req.push(10 + 0x20);
    req.extend_from_slice(b"s3cretword");
    req.push(b'\n');
    let f2 = frame5(&mut cc, &[&req]);
    let pcap = synth_pcap(41002, 14922, &[(1.0, true, 0x02, 1, b""), (1.1, true, 0x18, 2, &f1), (1.2, true, 0x18, 2 + f1.len() as u32, &f2)]);
    let v = view_of(&pcap, Some(ClientVersion::V222), Filter::default(), true, true);
    assert!(v.has_login);
    assert!(v.rows.iter().all(|r| r.event.raw.is_empty() && r.raw.is_empty()));
    let out = render_view(&v, Format::Text, "ls.pcap", "built-in", true);
    assert!(!out.contains("s3cretword") && !out.contains("dtacct"));
}

/// Writes the synthetic session used above, so the examples in
/// `documentation/capture-viewer.md` can be re-run:
/// `SEIHAI_VIEWER_SAMPLE_OUT=/path/sample.pcap cargo test -p seihai-core --lib write_sample_pcap -- --ignored`
#[test]
#[ignore = "writes a file; run by hand"]
fn write_sample_pcap() {
    let Ok(path) = std::env::var("SEIHAI_VIEWER_SAMPLE_OUT") else { return };
    std::fs::write(path, session(0x2b)).unwrap();
}
