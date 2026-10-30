//! The normalized trace: one row per message, both directions, in the
//! order they were seen, with the session-specific values masked.
//!
//! A trace is what the differ reads. It is a plain tab-separated file so a
//! person can read it, `grep` it and diff it:
//!
//! ```text
//! #seihai-trace v1
//! #meta version=2.31
//! #conn	dir	t_ms	frame	id	name	len	fields
//! gs0	C>S	0	0	-1	LOGIN	38	type=0|token=GNW28015|key=<KEY>|account=<ACCT>|password=<PASS>
//! ```
//!
//! Fields are `key=value` pairs joined with `|`; `%`, `|`, `=` (in a key or
//! value), tab, CR and LF and non-printable bytes are written as `%XX`.

use std::fmt::Write as _;

/// Direction of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dir {
    /// Client to server.
    C2S,
    /// Server to client.
    S2C,
}

impl Dir {
    pub fn label(self) -> &'static str {
        match self {
            Dir::C2S => "C>S",
            Dir::S2C => "S>C",
        }
    }

    pub fn parse(s: &str) -> Option<Dir> {
        match s {
            "C>S" => Some(Dir::C2S),
            "S>C" => Some(Dir::S2C),
            _ => None,
        }
    }
}

/// Pseudo id of a login or handshake frame (not a numbered message).
pub const ID_LOGIN: i32 = -1;
/// Pseudo id of a frame that did not decode.
pub const ID_BADFRAME: i32 = -2;

/// One message (or pseudo message) of one connection.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// `ls0`, `ls1`... for listserver connections, `gs0`, `gs1`... for game
    /// server connections, in order of first packet.
    pub conn: String,
    pub dir: Dir,
    /// Milliseconds since the first packet of this connection.
    pub t_ms: i64,
    /// Index of the frame within its connection and direction.
    pub frame: u32,
    pub id: i32,
    pub name: String,
    /// Payload bytes.
    pub len: usize,
    pub fields: Vec<(String, String)>,
    /// Payload bytes, only when the decoder was asked to keep them
    /// ([`super::decode::DecodeOptions::keep_raw`]); empty otherwise and
    /// never written to a trace file.
    pub raw: Vec<u8>,
}

impl Event {
    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// A decoded capture.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Trace {
    pub meta: Vec<(String, String)>,
    pub events: Vec<Event>,
}

impl Trace {
    pub fn meta(&self, key: &str) -> Option<&str> {
        self.meta.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn set_meta(&mut self, key: &str, value: &str) {
        match self.meta.iter_mut().find(|(k, _)| k == key) {
            Some(m) => m.1 = value.to_string(),
            None => self.meta.push((key.to_string(), value.to_string())),
        }
    }

    /// Connection names in order of first use.
    pub fn conns(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for e in &self.events {
            if !v.contains(&e.conn) {
                v.push(e.conn.clone());
            }
        }
        v
    }

    /// Events of one connection and direction, in order.
    pub fn stream<'a>(&'a self, conn: &'a str, dir: Dir) -> impl Iterator<Item = &'a Event> + 'a {
        self.events.iter().filter(move |e| e.conn == conn && e.dir == dir)
    }

    pub fn to_tsv(&self) -> String {
        let mut s = String::from("#seihai-trace v1\n");
        for (k, v) in &self.meta {
            let _ = writeln!(s, "#meta {}={}", esc(k), esc(v));
        }
        s.push_str("#conn\tdir\tt_ms\tframe\tid\tname\tlen\tfields\n");
        for e in &self.events {
            let fields: Vec<String> = e.fields.iter().map(|(k, v)| format!("{}={}", esc(k), esc(v))).collect();
            let _ = writeln!(
                s,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                e.conn,
                e.dir.label(),
                e.t_ms,
                e.frame,
                e.id,
                esc(&e.name),
                e.len,
                fields.join("|")
            );
        }
        s
    }

    pub fn from_tsv(text: &str) -> Result<Trace, String> {
        let mut t = Trace::default();
        let mut lines = text.lines();
        match lines.next() {
            Some("#seihai-trace v1") => {}
            _ => return Err("not a seihai trace (first line must be `#seihai-trace v1`)".into()),
        }
        for (n, line) in lines.enumerate() {
            if let Some(m) = line.strip_prefix("#meta ") {
                let (k, v) = m.split_once('=').ok_or_else(|| format!("line {}: bad meta", n + 2))?;
                t.meta.push((unesc(k), unesc(v)));
                continue;
            }
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let c: Vec<&str> = line.split('\t').collect();
            if c.len() != 8 {
                return Err(format!("line {}: expected 8 columns, got {}", n + 2, c.len()));
            }
            let bad = |what: &str| format!("line {}: bad {what}", n + 2);
            let mut fields = Vec::new();
            if !c[7].is_empty() {
                for f in c[7].split('|') {
                    let (k, v) = f.split_once('=').ok_or_else(|| bad("field"))?;
                    fields.push((unesc(k), unesc(v)));
                }
            }
            t.events.push(Event {
                conn: c[0].to_string(),
                dir: Dir::parse(c[1]).ok_or_else(|| bad("direction"))?,
                t_ms: c[2].parse().map_err(|_| bad("time"))?,
                frame: c[3].parse().map_err(|_| bad("frame"))?,
                id: c[4].parse().map_err(|_| bad("id"))?,
                name: unesc(c[5]),
                len: c[6].parse().map_err(|_| bad("length"))?,
                fields,
                raw: Vec::new(),
            });
        }
        Ok(t)
    }
}

/// Percent-escape what would break the TSV or the field syntax.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '%' | '|' | '=' | '\t' | '\n' | '\r' => {
                let _ = write!(o, "%{:02X}", c as u32);
            }
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => {
                let _ = write!(o, "%{:02X}", c as u32);
            }
            c if (c as u32) > 0xff => o.push('?'),
            c if (c as u32) > 0x7e => {
                let _ = write!(o, "%{:02X}", c as u32);
            }
            c => o.push(c),
        }
    }
    o
}

/// Undo [`esc`]. A byte escape above 0x7f comes back as that Latin-1 char.
pub fn unesc(s: &str) -> String {
    let b = s.as_bytes();
    let mut o = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                o.push((h * 16 + l) as u8 as char);
                i += 3;
                continue;
            }
        }
        // Copy one char (the input is valid UTF-8).
        let ch = s[i..].chars().next().unwrap();
        o.push(ch);
        i += ch.len_utf8();
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Trace {
        let mut t = Trace::default();
        t.set_meta("version", "2.31");
        t.set_meta("client", "real");
        t.events.push(Event {
            conn: "gs0".into(),
            dir: Dir::C2S,
            t_ms: 0,
            frame: 0,
            id: ID_LOGIN,
            name: "LOGIN".into(),
            len: 38,
            fields: vec![("token".into(), "GNW28015".into()), ("account".into(), "<ACCT>".into())],
            raw: Vec::new(),
        });
        t.events.push(Event {
            conn: "gs0".into(),
            dir: Dir::S2C,
            t_ms: 12,
            frame: 0,
            id: 13,
            name: "TOALL".into(),
            len: 9,
            fields: vec![("text".into(), "a|b=c%d\te\n".into())],
            raw: Vec::new(),
        });
        t
    }

    #[test]
    fn tsv_round_trip_keeps_awkward_text() {
        let t = sample();
        let text = t.to_tsv();
        assert_eq!(text.lines().count(), 6);
        assert_eq!(Trace::from_tsv(&text).unwrap(), t);
    }

    #[test]
    fn rejects_other_files_and_short_rows() {
        assert!(Trace::from_tsv("hello\n").is_err());
        assert!(Trace::from_tsv("#seihai-trace v1\ngs0\tC>S\t0\n").is_err());
        assert!(Trace::from_tsv("#seihai-trace v1\ngs0\tX>Y\t0\t0\t1\tn\t0\t\n").is_err());
    }

    #[test]
    fn escape_round_trip_for_latin1() {
        let s = "caf\u{e9} 100% |x|=y";
        assert_eq!(unesc(&esc(s)), s);
        assert!(!esc(s).contains('|'));
    }
}
