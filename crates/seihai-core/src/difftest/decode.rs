//! Capture to trace: frames, messages and decoded fields, with the
//! session-specific values masked.
//!
//! Everything goes through Seihai's own codecs ([`Gen3`], [`Gen5`], the
//! frame readers, the message splitter and the layout parsers), so a message
//! Seihai cannot read shows up here as raw bytes, which is itself a finding.
//!
//! Masked by default: the login key, the account and password, player ids
//! (renumbered `<PID1>`, `<PID2>`... in order of first appearance), file
//! modification times, the UDP port, IP address and online-seconds props,
//! the account name inside text. Large binary payloads are reduced to their
//! length and CRC-32.

use super::names::{pli_name, plo_name, prop_name};
use super::pcap::{read_pcap, reassemble, Stream, TcpConn};
use super::trace::{Dir, Event, Trace, ID_BADFRAME, ID_LOGIN};
use crate::assets::crc32;
use crate::protocol::layouts::{
    parse_board_heights, parse_board_modify2, parse_player_warp2, parse_server_warp, parse_showimg_relay,
};
use crate::protocol::{
    bz2_decompress, decode_pos_precise, latin1, parse_list_reply, parse_player_props_in, read_gint3, read_gshort,
    split_messages, zlib_decompress, ClientVersion, EncryptionGen, Gen3, Gen5, Gen5Comp, Message, Splitter,
};

/// What to decode and how.
#[derive(Debug, Clone)]
pub struct DecodeOptions {
    /// The client version of the capture (decides the cipher generation and
    /// which layouts apply).
    pub version: ClientVersion,
    /// Mask session-specific values (default). Off keeps ids and times.
    pub mask: bool,
    /// Free-text labels stored in the trace header.
    pub client: String,
    pub scenario: String,
    /// Keep each message's payload bytes in [`Event::raw`] (the capture
    /// viewer needs them). Never kept for the login frame or for client
    /// messages on a listserver connection, which carry the account and
    /// password.
    pub keep_raw: bool,
}

impl DecodeOptions {
    pub fn new(version: ClientVersion) -> Self {
        Self { version, mask: true, client: String::new(), scenario: String::new(), keep_raw: false }
    }
}

/// Read a pcap file's bytes and decode it.
pub fn decode_pcap(data: &[u8], opts: &DecodeOptions) -> Result<Trace, String> {
    let packets = read_pcap(data)?;
    Ok(decode_conns(&reassemble(&packets), opts))
}

/// Decode reassembled connections. Connections with no payload (a probe, a
/// refused connect) are skipped.
pub fn decode_conns(conns: &[TcpConn], opts: &DecodeOptions) -> Trace {
    decode_conns_ex(conns, opts).0
}

/// [`decode_conns`], plus the account name the login frame carried, so a
/// caller that decodes text itself can mask it the same way.
pub fn decode_conns_ex(conns: &[TcpConn], opts: &DecodeOptions) -> (Trace, Option<String>) {
    let mut trace = Trace::default();
    trace.set_meta("version", opts.version.label());
    if !opts.client.is_empty() {
        trace.set_meta("client", &opts.client);
    }
    if !opts.scenario.is_empty() {
        trace.set_meta("scenario", &opts.scenario);
    }
    trace.set_meta("masked", if opts.mask { "yes" } else { "no" });
    let (mut n_ls, mut n_gs) = (0, 0);
    let mut masker = Masker::new(opts.mask);
    let mut bad = 0;
    for c in conns {
        if c.c2s.bytes.is_empty() && c.s2c.bytes.is_empty() {
            continue;
        }
        let kind = classify(&c.c2s);
        let name = match kind {
            Kind::Listserver { .. } => {
                n_ls += 1;
                format!("ls{}", n_ls - 1)
            }
            Kind::Gserver => {
                n_gs += 1;
                format!("gs{}", n_gs - 1)
            }
        };
        let evs = decode_conn(c, &name, kind, opts, &mut masker);
        bad += evs.iter().filter(|e| e.id == ID_BADFRAME).count();
        trace.events.extend(evs);
    }
    trace.set_meta("connections", &(n_ls + n_gs).to_string());
    trace.set_meta("bad_frames", &bad.to_string());
    (trace, masker.account.clone())
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// `gen5`: the `GCHAR 7` key handshake form; otherwise plain frames.
    Listserver { gen5: bool },
    Gserver,
}

/// The first client frame tells a listserver from a game server: the
/// listserver handshake names `newmain`.
fn classify(c2s: &Stream) -> Kind {
    if let Some((body, _)) = plain_frame(&c2s.bytes, 0) {
        if body.first() == Some(&(7 + 0x20)) && contains(&body, b"newmain") {
            return Kind::Listserver { gen5: true };
        }
        if contains(&body, b"newmain") {
            return Kind::Listserver { gen5: false };
        }
    }
    Kind::Gserver
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// A plain zlib frame at `off`: the decompressed body and the next offset.
fn plain_frame(b: &[u8], off: usize) -> Option<(Vec<u8>, usize)> {
    let n = usize::from(u16::from_be_bytes([*b.get(off)?, *b.get(off + 1)?]));
    let body = b.get(off + 2..off + 2 + n)?;
    Some((zlib_decompress(body).ok()?, off + 2 + n))
}

struct Masker {
    on: bool,
    pids: Vec<u16>,
    account: Option<String>,
}

impl Masker {
    fn new(on: bool) -> Self {
        Self { on, pids: Vec::new(), account: None }
    }

    fn pid(&mut self, id: u16) -> String {
        if !self.on {
            return id.to_string();
        }
        let n = match self.pids.iter().position(|&p| p == id) {
            Some(i) => i,
            None => {
                self.pids.push(id);
                self.pids.len() - 1
            }
        };
        format!("<PID{}>", n + 1)
    }

    fn text(&self, s: &str) -> String {
        match (&self.account, self.on) {
            (Some(a), true) if !a.is_empty() => s.replace(a.as_str(), "<ACCT>"),
            _ => s.to_string(),
        }
    }

    fn val(&self, v: impl std::fmt::Display) -> String {
        if self.on { "<MASKED>".into() } else { v.to_string() }
    }
}

type Fields = Vec<(String, String)>;

fn f(k: &str, v: impl Into<String>) -> (String, String) {
    (k.to_string(), v.into())
}

struct ConnCtx<'a> {
    name: &'a str,
    stream_c: &'a Stream,
    stream_s: &'a Stream,
    t0: f64,
}

fn push_event(
    out: &mut Vec<Event>,
    cx: &ConnCtx<'_>,
    dir: Dir,
    ts: f64,
    frame: u32,
    id: i32,
    name: String,
    len: usize,
    fields: Fields,
) {
    let _ = (cx.stream_c, cx.stream_s);
    out.push(Event {
        conn: cx.name.to_string(),
        dir,
        t_ms: ((ts - cx.t0) * 1000.0).round() as i64,
        frame,
        id,
        name,
        len,
        fields,
        raw: Vec::new(),
    });
}

fn decode_conn(c: &TcpConn, name: &str, kind: Kind, opts: &DecodeOptions, mask: &mut Masker) -> Vec<Event> {
    let cx = ConnCtx { name, stream_c: &c.c2s, stream_s: &c.s2c, t0: c.t0 };
    let mut out = Vec::new();
    let ver = opts.version;
    let listserver = matches!(kind, Kind::Listserver { .. });
    let (gen5, gen3) = match kind {
        Kind::Listserver { gen5 } => (gen5, false),
        Kind::Gserver => (ver.encryption() == EncryptionGen::Gen5, ver.encryption() == EncryptionGen::Gen3),
    };
    // ---- client to server ---------------------------------------------
    let mut key: Option<u8> = None;
    let mut c_cipher5: Option<Gen5> = None;
    let mut c_cipher3: Option<Gen3> = None;
    let mut off = 0;
    let mut frame_no = 0u32;
    let bytes = &c.c2s.bytes;
    let mut first = true;
    while off + 2 <= bytes.len() {
        let n = usize::from(u16::from_be_bytes([bytes[off], bytes[off + 1]]));
        let ts = c.c2s.ts_at(off);
        let Some(body) = bytes.get(off + 2..off + 2 + n) else {
            push_event(&mut out, &cx, Dir::C2S, ts, frame_no, ID_BADFRAME, "BADFRAME".into(), bytes.len() - off, vec![f("reason", "truncated frame")]);
            break;
        };
        off += 2 + n;
        let plain: Result<Vec<u8>, String> = if first && (gen5 || gen3 || listserver) {
            // The login or handshake frame: plain zlib, no cipher.
            zlib_decompress(body).map_err(|e| format!("login frame: {e}"))
        } else if gen5 {
            decode_gen5_body(body, c_cipher5.as_mut())
        } else {
            zlib_decompress(body).map_err(|e| format!("zlib: {e}"))
        };
        let plain = match plain {
            Ok(p) => p,
            Err(why) => {
                push_event(&mut out, &cx, Dir::C2S, ts, frame_no, ID_BADFRAME, "BADFRAME".into(), body.len(), vec![f("reason", why)]);
                first = false;
                frame_no += 1;
                continue;
            }
        };
        let mut lines: Vec<&[u8]> = plain.split(|&b| b == b'\n').collect();
        if lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        let mut li = 0;
        if first {
            first = false;
            let line = lines.first().copied().unwrap_or(&[]);
            li = 1;
            let (fields, k) = decode_login(line, kind, ver, mask);
            key = k;
            if let Some(k) = k {
                if gen5 {
                    c_cipher5 = Some(gen5_for(ver, listserver, k));
                }
                if gen3 {
                    c_cipher3 = Some(Gen3::new(k));
                }
            }
            let nm = if listserver { "LS_HELLO" } else { "LOGIN" };
            push_event(&mut out, &cx, Dir::C2S, ts, frame_no, ID_LOGIN, nm.into(), line.len(), fields);
        }
        for line in &lines[li.min(lines.len())..] {
            let mut msg = line.to_vec();
            if let Some(g) = c_cipher3.as_mut() {
                let mut enc = msg.clone();
                enc.push(b'\n');
                match g.decode(&enc) {
                    Some(mut d) => {
                        d.pop();
                        msg = d;
                    }
                    None => {
                        push_event(&mut out, &cx, Dir::C2S, ts, frame_no, ID_BADFRAME, "BADMSG".into(), msg.len(), vec![f("reason", "gen-3 junk byte did not match")]);
                        continue;
                    }
                }
            }
            let Some((&idb, payload)) = msg.split_first() else { continue };
            let id = i32::from(idb.wrapping_sub(0x20));
            let (nm, fields) = if listserver {
                describe_ls_c2s(id, payload, mask)
            } else {
                describe_c2s(ver, id, payload, mask)
            };
            push_event(&mut out, &cx, Dir::C2S, ts, frame_no, id, nm, payload.len(), fields);
            if opts.keep_raw && !listserver {
                if let Some(e) = out.last_mut() {
                    e.raw = payload.to_vec();
                }
            }
        }
        frame_no += 1;
    }
    // ---- server to client ---------------------------------------------
    let mut s_cipher5: Option<Gen5> = if gen5 { key.map(|k| gen5_for(ver, listserver, k)) } else { None };
    // 6.x game servers send `FF FF` + u32 lengths for long frames.
    let long = !listserver && ver.profile().protocol.gen5_long_frames;
    let mut splitter = Splitter::default();
    let bytes = &c.s2c.bytes;
    let mut off = 0;
    let mut frame_no = 0u32;
    while off + 2 <= bytes.len() {
        let fl = if long { crate::protocol::FrameLen::Long } else { crate::protocol::FrameLen::Short };
        let Some((head, n)) = fl.header(&bytes[off..]) else { break };
        let ts = c.s2c.ts_at(off);
        let Some(body) = bytes.get(off + head..off + head + n) else {
            push_event(&mut out, &cx, Dir::S2C, ts, frame_no, ID_BADFRAME, "BADFRAME".into(), bytes.len() - off, vec![f("reason", "truncated frame")]);
            break;
        };
        off += head + n;
        let plain = if gen5 {
            decode_gen5_body(body, s_cipher5.as_mut())
        } else {
            zlib_decompress(body).map_err(|e| format!("zlib: {e}"))
        };
        let plain = match plain {
            Ok(p) => p,
            Err(why) => {
                push_event(&mut out, &cx, Dir::S2C, ts, frame_no, ID_BADFRAME, "BADFRAME".into(), body.len(), vec![f("reason", why)]);
                frame_no += 1;
                continue;
            }
        };
        let msgs: Vec<Message> = splitter.push(&plain);
        if listserver {
            // One extra row for the whole reply, so a server list shows up
            // as a list and not as raw bytes.
            let reply = parse_list_reply(&plain);
            if !reply.servers.is_empty() {
                let names: Vec<String> = reply.servers.iter().map(|s| format!("{}@{}", s.name, s.port)).collect();
                push_event(
                    &mut out,
                    &cx,
                    Dir::S2C,
                    ts,
                    frame_no,
                    1000,
                    "LS_SERVERLIST".into(),
                    plain.len(),
                    vec![f("servers", reply.servers.len().to_string()), f("list", names.join(","))],
                );
                frame_no += 1;
                continue;
            }
        }
        for m in msgs {
            let (nm, fields) = if listserver {
                (format!("LS_{}", m.id), generic(&m.payload))
            } else {
                describe_s2c(ver, i32::from(m.id), &m.payload, mask)
            };
            push_event(&mut out, &cx, Dir::S2C, ts, frame_no, i32::from(m.id), nm, m.payload.len(), fields);
            if opts.keep_raw {
                if let Some(e) = out.last_mut() {
                    e.raw = m.payload.clone();
                }
            }
        }
        frame_no += 1;
    }
    // Messages are stamped per frame; keep the two directions interleaved by time.
    out.sort_by(|a, b| a.t_ms.cmp(&b.t_ms).then((a.dir as u8).cmp(&(b.dir as u8))));
    let _ = split_messages;
    out
}

/// Undo the gen-5 framing of one body (compression byte, XOR, inflate).
/// The gen-5 cipher a game connection of `ver` uses: 5.12 steps its
/// compressed bodies with the round-down count (`gen5_round_down`). Without
/// it every 5.12 client frame read as BADFRAME (stress test, 2026-10-31).
fn gen5_for(ver: ClientVersion, listserver: bool, key: u8) -> Gen5 {
    if !listserver && ver.profile().protocol.gen5_round_down {
        Gen5::new(key).round_down()
    } else {
        Gen5::new(key)
    }
}

fn decode_gen5_body(body: &[u8], cipher: Option<&mut Gen5>) -> Result<Vec<u8>, String> {
    let (&ct, rest) = body.split_first().ok_or("empty gen-5 frame")?;
    let comp = match ct {
        0x02 => Gen5Comp::None,
        0x04 => Gen5Comp::Zlib,
        0x06 => Gen5Comp::Bz2,
        o => return Err(format!("unknown gen-5 compression byte {o:#04x}")),
    };
    let Some(cipher) = cipher else { return Err("no key: the login frame is missing".into()) };
    let raw = cipher.decode(rest.to_vec(), comp);
    match comp {
        Gen5Comp::None => Ok(raw),
        Gen5Comp::Zlib => zlib_decompress(&raw).map_err(|e| format!("zlib: {e}")),
        Gen5Comp::Bz2 => bz2_decompress(&raw).map_err(|e| format!("bz2: {e}")),
    }
}

fn gstring(b: &[u8], i: &mut usize) -> Option<String> {
    let n = usize::from(b.get(*i)?.wrapping_sub(0x20));
    let s = b.get(*i + 1..*i + 1 + n)?;
    *i += 1 + n;
    Some(latin1(s))
}

/// The login line (gserver) or handshake (listserver). Returns the fields
/// and the cipher key, if the line carries one.
fn decode_login(line: &[u8], kind: Kind, ver: ClientVersion, mask: &mut Masker) -> (Fields, Option<u8>) {
    let mut out = Vec::new();
    let Some(&t) = line.first() else { return (out, None) };
    let typ = t.wrapping_sub(0x20);
    out.push(f("type", typ.to_string()));
    let mut i = 1;
    let mut key = None;
    let has_key = match kind {
        Kind::Listserver { gen5 } => gen5,
        Kind::Gserver => ver.encryption() != EncryptionGen::Gen2,
    };
    if has_key {
        if let Some(&k) = line.get(1) {
            key = Some(k.wrapping_sub(0x20));
            out.push(f("key", mask.val(k.wrapping_sub(0x20))));
            i = 2;
        }
    }
    match kind {
        Kind::Listserver { .. } => {
            // gen-5: token then `newmain`. Plain form: ` newmain` and nothing else.
            out.push(f("rest", mask.text(&latin1(line.get(i..).unwrap_or(&[])))));
        }
        Kind::Gserver => {
            if let Some(tok) = line.get(i..i + 8) {
                out.push(f("token", latin1(tok)));
                i += 8;
                if let Some(a) = gstring(line, &mut i) {
                    if mask.account.is_none() {
                        mask.account = Some(a.clone());
                    }
                    out.push(f("account", if mask.on { "<ACCT>".into() } else { a }));
                }
                if gstring(line, &mut i).is_some() {
                    out.push(f("password", "<PASS>"));
                }
            }
        }
    }
    (out, key)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Raw payload: short ones as hex, long ones as length and CRC-32.
fn generic(p: &[u8]) -> Fields {
    if p.len() <= 32 {
        if p.is_empty() { vec![] } else { vec![f("hex", hex(p))] }
    } else {
        vec![f("crc", format!("{:08x}", crc32(p)))]
    }
}

fn gchar(b: u8) -> u8 {
    b.wrapping_sub(0x20)
}

fn describe_ls_c2s(id: i32, payload: &[u8], _mask: &Masker) -> (String, Fields) {
    match id {
        0 => ("LS_NEWMAIN".into(), vec![f("text", latin1(payload))]),
        1 => {
            // account and password: never shown
            ("LS_SERVERLIST".into(), vec![f("account", "<ACCT>"), f("password", "<PASS>")])
        }
        _ => (format!("LS_{id}"), generic(payload)),
    }
}

fn gint5(b: &[u8]) -> Option<u64> {
    let c = b.get(..5)?;
    Some(c.iter().fold(0u64, |a, &x| (a << 7) | u64::from(gchar(x) & 0x7f)))
}

fn describe_c2s(ver: ClientVersion, id: i32, p: &[u8], mask: &mut Masker) -> (String, Fields) {
    let name = pli_name(id as u32).map_or_else(|| format!("PLI_{id}"), |n| n.to_string());
    let fields: Fields = match id {
        0 => match p {
            [x, y, lvl @ ..] => vec![
                f("x", fnum(f32::from(gchar(*x)) / 2.0)),
                f("y", fnum(f32::from(gchar(*y)) / 2.0)),
                f("level", mask.text(&latin1(lvl))),
            ],
            _ => generic(p),
        },
        30 => match (gint5(p), p.get(5..)) {
            (Some(m), Some([x, y, lvl @ ..])) => vec![
                f("modtime", mask.val(m)),
                f("x", fnum(f32::from(gchar(*x)) / 2.0)),
                f("y", fnum(f32::from(gchar(*y)) / 2.0)),
                f("level", mask.text(&latin1(lvl))),
            ],
            _ => generic(p),
        },
        35 => match (gint5(p), p.get(5..)) {
            (Some(m), Some(lvl)) => vec![f("modtime", mask.val(m)), f("level", latin1(lvl))],
            _ => generic(p),
        },
        2 => describe_props(ver, p, mask),
        6 => vec![f("text", mask.text(&latin1(p)))],
        23 => vec![f("file", latin1(p))],
        // PLI_UPDATEFILE: GINT5 mod time, then the file name.
        34 => match (gint5(p), p.get(5..)) {
            (Some(m), Some(name)) => vec![f("modtime", mask.val(m)), f("file", latin1(name))],
            _ => generic(p),
        },
        // PLI_SHOWIMGPLAYER: the sender's weapon-script image list, as
        // changes (or, from the marker on, the full list).
        24 => showimg_fields(p),
        46 => vec![f("value", p.first().map_or("-".into(), |&b| gchar(b).to_string())), f("len", p.len().to_string())],
        37 => vec![f("language", latin1(p))],
        39 => vec![f("len", p.len().to_string()), f("crc", format!("{:08x}", crc32(p)))],
        // PLI_PROCESSLIST: the client's process list, different every run
        44 => vec![f("len", p.len().to_string())],
        41 => vec![f("server", latin1(p))],
        _ => generic(p),
    };
    (name, fields)
}

/// The id 24 / 32 text as `full` and an `images` summary; the bytes stay
/// visible as `hex` when they are short.
fn showimg_fields(text: &[u8]) -> Fields {
    let up = crate::protocol::parse_showimg_update(text);
    let mut v = vec![f("full", if up.full { "yes" } else { "no" })];
    v.push(f("images", crate::protocol::describe_showimg_update(&up)));
    if !text.is_empty() && text.len() <= 32 {
        v.push(f("hex", hex(text)));
    }
    v
}

fn fnum(v: f32) -> String {
    if (v - v.round()).abs() < 1e-4 { format!("{}", v.round() as i64) } else { format!("{}", (v * 100.0).round() / 100.0) }
}

/// A prop stream as named fields, plus `props=` with the id order.
fn describe_props(ver: ClientVersion, p: &[u8], mask: &mut Masker) -> Fields {
    let props = parse_player_props_in(p, false, None);
    let mut out = Vec::new();
    let mut order = Vec::new();
    let mut used = 0;
    for pr in &props {
        used += 1 + pr.raw.len();
        let nm = prop_name(u32::from(pr.id)).map_or_else(|| format!("P{}", pr.id), |n| n.to_string());
        order.push(nm.clone());
        let raw = &pr.raw;
        let text = |r: &[u8]| mask.text(&latin1(r.get(1..).unwrap_or(&[])));
        let v = match pr.id {
            0 | 12 | 20 | 21 | 34 | 35 | 52 | 75 | 82 => text(raw),
            10 if !raw.is_empty() => text(raw),
            8 | 9 => format!("power={},image={}", raw.first().map_or(0, |&b| gchar(b)), text(raw)),
            11 => format!("image={}", text(raw)),
            15 | 16 => fnum(f32::from(gchar(*raw.first().unwrap_or(&0x20))) / 2.0),
            17 => {
                let s = gchar(*raw.first().unwrap_or(&0x20));
                format!("{s} (dir {})", s % 4)
            }
            13 => raw.iter().map(|&b| gchar(b).to_string()).collect::<Vec<_>>().join(","),
            14 | 29 | 30 | 31 => mask.val(hex(raw)),
            78 | 79 => read_gshort(raw).map_or("?".into(), |v| fnum(decode_pos_precise(v))),
            _ if raw.len() == 1 => gchar(raw[0]).to_string(),
            _ => hex(raw),
        };
        out.push((nm, v));
    }
    let _ = ver;
    out.insert(0, f("props", order.join(",")));
    if used < p.len() {
        out.push(f("unparsed_tail", hex(&p[used..p.len().min(used + 16)])));
    }
    out
}

fn describe_s2c(ver: ClientVersion, id: i32, p: &[u8], mask: &mut Masker) -> (String, Fields) {
    let name = plo_name(id as u32).map_or_else(|| format!("PLO_{id}"), |n| n.to_string());
    let fields: Fields = match id {
        6 => vec![f("level", mask.text(&latin1(p)))],
        13 => match read_gshort(p) {
            Some(pid) => vec![f("from", mask.pid(pid)), f("text", mask.text(&latin1(&p[2..])))],
            None => generic(p),
        },
        14 => match p {
            [x, y, lvl @ ..] => vec![
                f("x", fnum(f32::from(gchar(*x)) / 2.0)),
                f("y", fnum(f32::from(gchar(*y)) / 2.0)),
                f("level", latin1(lvl)),
            ],
            _ => generic(p),
        },
        49 => match parse_player_warp2(p) {
            Some(w) => vec![
                f("x", fnum(w.x)),
                f("y", fnum(w.y)),
                f("cell", format!("{},{}", w.gmap_x, w.gmap_y)),
                f("level", w.level),
            ],
            None => generic(p),
        },
        8 => match read_gshort(p) {
            Some(pid) => {
                let mut v = vec![f("player", mask.pid(pid))];
                v.extend(describe_props(ver, &p[2..], mask));
                v
            }
            None => generic(p),
        },
        9 => describe_props(ver, p, mask),
        32 => match parse_showimg_relay(p) {
            Some((pid, rest)) => {
                let mut v = vec![f("from", mask.pid(pid))];
                v.extend(showimg_fields(&rest));
                v
            }
            None => generic(p),
        },
        178 => match parse_server_warp(p) {
            Some(w) => vec![f("name", w.name), f("host", w.host), f("port", w.port)],
            None => generic(p),
        },
        185 => match parse_board_heights(p) {
            Some(h) => vec![
                f("cell", format!("{},{}", h.map_x, h.map_y)),
                f("block", format!("{},{} {}x{}", h.block_x, h.block_y, h.width, h.height)),
            ],
            None => generic(p),
        },
        186 => match parse_board_modify2(p) {
            Some(b) => {
                let tiles: Vec<u8> = b.tiles.iter().flat_map(|t| t.to_le_bytes()).collect();
                vec![
                    f("cell", format!("{},{}", b.map_x, b.map_y)),
                    f("block", format!("{},{} {}x{}", b.x, b.y, b.width, b.height)),
                    f("tiles_crc", format!("{:08x}", crc32(&tiles))),
                    f("trailer", b.trailer.len().to_string()),
                ]
            }
            None => generic(p),
        },
        // PLO_FILE (102) after a RAWDATA header: [modtime] name_len name data
        102 => {
            let (m, rest) = if ver.file_has_modtime() {
                (gint5(p), p.get(5..).unwrap_or(&[]))
            } else {
                (None, p)
            };
            match rest.split_first() {
                Some((&n, tail)) if tail.len() >= usize::from(gchar(n)) => {
                    let n = usize::from(gchar(n));
                    let data = &tail[n..];
                    let mut v = vec![f("file", latin1(&tail[..n])), f("size", data.len().to_string()), f("crc", format!("{:08x}", crc32(data)))];
                    if let Some(m) = m {
                        v.push(f("modtime", mask.val(m)));
                    }
                    v
                }
                _ => generic(p),
            }
        }
        0 | 3 | 1 | 100 | 101 => {
            // level board (raw), npc props, links: length and CRC only
            vec![f("len", p.len().to_string()), f("crc", format!("{:08x}", crc32(p)))]
        }
        // PLO_NEWWORLDTIME: the server clock
        42 => vec![f("time", mask.val(hex(p)))],
        // file name replies and weapon names
        30 | 34 | 45 | 156 => vec![f("text", latin1(p))],
        44 | 45 => generic(p),
        _ => {
            if p.len() > 32 { vec![f("len", p.len().to_string()), f("crc", format!("{:08x}", crc32(p)))] } else { generic(p) }
        }
    };
    let _ = read_gint3;
    (name, fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 5.12 game links use the round-down step count in both directions;
    /// other builds and every listserver link do not.
    #[test]
    fn gen5_cipher_follows_the_profile() {
        assert!(gen5_for(ClientVersion::V512, false, 0x21).is_round_down());
        assert!(!gen5_for(ClientVersion::V512, true, 0x21).is_round_down());
        assert!(!gen5_for(ClientVersion::V222, false, 0x21).is_round_down());
        assert!(!gen5_for(ClientVersion::V603R7, false, 0x21).is_round_down());
    }
    use crate::difftest::pcap::tests::synth_pcap;
    use crate::protocol::{write_zlib_frame, Gen5};

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
        crate::protocol::write_frame(comp, &cipher.encode(c, comp))
    }

    fn login_line(key: u8, acct: &str) -> Vec<u8> {
        let mut l = vec![0x20, key + 0x20];
        l.extend_from_slice(b"GNW28015");
        l.push(acct.len() as u8 + 0x20);
        l.extend_from_slice(acct.as_bytes());
        l.push(4 + 0x20);
        l.extend_from_slice(b"pass");
        l.push(b'\n');
        l
    }

    /// A gen-5 gserver session: login, a props frame, a 46, and server frames.
    fn session_pcap(key: u8) -> Vec<u8> {
        let login = write_zlib_frame(&login_line(key, "dtacct")).unwrap();
        let mut cc = Gen5::new(key);
        let mut sc = Gen5::new(key);
        // props: X = 30.5 (61), Y = 12 (24), SPRITE 2, CURCHAT "hi dtacct"
        let mut props = vec![0x22, 0x2f, 0x20 + 61, 0x30, 0x20 + 24, 0x31, 0x22, 0x2c, 0x20 + 9];
        props.extend_from_slice(b"hi dtacct");
        props.push(b'\n');
        let c1 = frame5(&mut cc, &[&[46 + 0x20, 1 + 0x20, b'\n'], &props]);
        // server: PLO_LEVELNAME 6 "lv_a.nw", PLO_OTHERPLPROPS 8 pid 77 X 10
        let s1 = frame5(&mut sc, &[b"&lv_a.nw\n", &[0x28, 0x20, 0x20 + 77, 0x2f, 0x20 + 20, b'\n']]);
        synth_pcap(
            41000,
            14802,
            &[
                (10.0, true, 0x02, 1, b""),
                (10.001, false, 0x12, 1, b""),
                (10.002, true, 0x18, 2, &login),
                (10.050, true, 0x18, 2 + login.len() as u32, &c1),
                (10.060, false, 0x18, 2, &s1),
            ],
        )
    }

    #[test]
    fn gen5_session_decodes_with_masks() {
        let t = decode_pcap(&session_pcap(0x2b), &DecodeOptions::new(ClientVersion::V231)).unwrap();
        let rows: Vec<(String, i32, String)> = t.events.iter().map(|e| (e.dir.label().into(), e.id, e.name.clone())).collect();
        assert_eq!(
            rows,
            vec![
                ("C>S".into(), ID_LOGIN, "LOGIN".into()),
                ("C>S".into(), 46, "ENTERLEVEL".into()),
                ("C>S".into(), 2, "PLAYERPROPS".into()),
                ("S>C".into(), 6, "LEVELNAME".into()),
                ("S>C".into(), 8, "OTHERPLPROPS".into()),
            ]
        );
        let login = &t.events[0];
        assert_eq!(login.field("token"), Some("GNW28015"));
        assert_eq!(login.field("key"), Some("<MASKED>"));
        assert_eq!(login.field("account"), Some("<ACCT>"));
        assert_eq!(login.field("password"), Some("<PASS>"));
        let props = &t.events[2];
        assert_eq!(props.field("X"), Some("30.5"));
        assert_eq!(props.field("Y"), Some("12"));
        assert_eq!(props.field("CURCHAT"), Some("hi <ACCT>"));
        assert_eq!(props.field("props"), Some("X,Y,SPRITE,CURCHAT"));
        assert_eq!(t.events[1].field("value"), Some("1"));
        let other = &t.events[4];
        assert_eq!(other.field("player"), Some("<PID1>"));
        assert_eq!(other.field("X"), Some("10"));
        // Frames: both client messages 46 and props share frame 1; login is frame 0.
        assert_eq!(t.events[0].frame, 0);
        assert_eq!(t.events[1].frame, t.events[2].frame);
        assert_eq!(t.events[1].frame, 1);
        // Time is relative to the connection's first packet.
        assert_eq!(t.events[0].t_ms, 2);
    }

    #[test]
    fn no_mask_keeps_the_key_and_ids() {
        let mut o = DecodeOptions::new(ClientVersion::V231);
        o.mask = false;
        let t = decode_pcap(&session_pcap(0x2b), &o).unwrap();
        assert_eq!(t.events[0].field("key"), Some("43"));
        assert_eq!(t.events[4].field("player"), Some("77"));
        // The password is never written, masked or not.
        assert_eq!(t.events[0].field("password"), Some("<PASS>"));
    }

    #[test]
    fn different_keys_give_identical_traces() {
        let a = decode_pcap(&session_pcap(0x11), &DecodeOptions::new(ClientVersion::V231)).unwrap();
        let b = decode_pcap(&session_pcap(0x6e), &DecodeOptions::new(ClientVersion::V231)).unwrap();
        assert_eq!(a.to_tsv(), b.to_tsv());
    }

    #[test]
    fn broken_frame_is_reported_not_fatal() {
        let key = 0x2b;
        let login = write_zlib_frame(&login_line(key, "dtacct")).unwrap();
        let bad = vec![0, 4, 9, 9, 9, 9]; // unknown compression byte 9
        let pcap = synth_pcap(
            41001,
            14802,
            &[(1.0, true, 0x02, 1, b""), (1.1, true, 0x18, 2, &login), (1.2, true, 0x18, 2 + login.len() as u32, &bad)],
        );
        let t = decode_pcap(&pcap, &DecodeOptions::new(ClientVersion::V231)).unwrap();
        assert_eq!(t.events.len(), 2);
        assert_eq!(t.events[1].id, ID_BADFRAME);
        assert_eq!(t.meta("bad_frames"), Some("1"));
    }

    #[test]
    fn listserver_connection_is_recognised() {
        let mut hs = vec![7 + 0x20, 0x2b + 0x20];
        hs.extend_from_slice(b"GNW30123newmain\n");
        let f1 = write_zlib_frame(&hs).unwrap();
        let pcap = synth_pcap(41002, 14922, &[(1.0, true, 0x02, 1, b""), (1.1, true, 0x18, 2, &f1)]);
        let t = decode_pcap(&pcap, &DecodeOptions::new(ClientVersion::V222)).unwrap();
        assert_eq!(t.events[0].conn, "ls0");
        assert_eq!(t.events[0].name, "LS_HELLO");
        assert_eq!(t.events[0].field("key"), Some("<MASKED>"));
    }
}
