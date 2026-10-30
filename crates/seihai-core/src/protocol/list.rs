//! Listserver reply parser.
//!
//! Layout from the cleaned listserver notes (GServer-era listserver and
//! the 2.05 receive handler). The body is the decompressed frame: GCHAR
//! id + payload + `\n` per message.
//!
//! - id 0 `SVRLIST`: `c count`, then per server `c 8` (field count) and
//!   8 GSTRINGs: type+name, language, description, url, version,
//!   players, ip, port
//! - id 2 `STATUS`: text (`\r` = line break)
//! - id 3 `SITEURL`, id 5 `UPGURL`: url
//! - id 4 `ERROR`: text; the server closes the connection
//!
//! Still to be locked against a lab capture of the first reply frame.

use super::dispatch::split_messages;

/// One server entry from a listserver reply.
#[derive(Debug, Clone, Default)]
pub struct ServerEntry {
    /// Type prefix taken off the name (`3 `, `P `, `H `, `U `), empty for silver.
    pub kind: String,
    pub name: String,
    pub language: String,
    pub description: String,
    pub url: String,
    pub version: String,
    pub host: String,
    pub port: u16,
    pub players: Option<u16>,
}

impl ServerEntry {
    /// One line for a server picker or listing: index, kind letter (`-`
    /// for silver), name, `host:port`, version and player count.
    pub fn display_line(&self, index: usize) -> String {
        let kind = if self.kind.is_empty() { "-" } else { self.kind.as_str() };
        let players = self.players.map_or_else(|| "?".to_string(), |n| n.to_string());
        format!(
            "  {index}) [{kind}] {}  {}:{}  {}  players: {players}",
            self.name, self.host, self.port, self.version
        )
    }
}

/// Everything a listserver sent in one reply body.
#[derive(Debug, Clone, Default)]
pub struct ListReply {
    pub servers: Vec<ServerEntry>,
    pub status: Option<String>,
    pub site_url: Option<String>,
    pub error: Option<String>,
    pub upgrade_url: Option<String>,
}

/// Read a GSTRING (GCHAR length + bytes) at `*i`.
fn gstring(data: &[u8], i: &mut usize) -> Option<String> {
    let n = data.get(*i)?.wrapping_sub(0x20) as usize;
    let s = data.get(*i + 1..*i + 1 + n)?;
    *i += 1 + n;
    Some(String::from_utf8_lossy(s).into_owned())
}

/// Parse an `SVRLIST` payload (after the id byte).
pub fn parse_svrlist(payload: &[u8]) -> Vec<ServerEntry> {
    let mut out = Vec::new();
    let Some(&count) = payload.first() else {
        return out;
    };
    let mut i = 1;
    for _ in 0..count.wrapping_sub(0x20) {
        let Some(&nfields) = payload.get(i) else { break };
        i += 1;
        let mut fields = Vec::new();
        for _ in 0..nfields.wrapping_sub(0x20) {
            match gstring(payload, &mut i) {
                Some(f) => fields.push(f),
                None => return out,
            }
        }
        if fields.len() < 8 {
            continue;
        }
        let (kind, name) = match fields[0].as_bytes() {
            [k @ (b'3' | b'P' | b'H' | b'U'), b' ', ..] => {
                ((*k as char).to_string(), fields[0][2..].to_string())
            }
            _ => (String::new(), fields[0].clone()),
        };
        out.push(ServerEntry {
            kind,
            name,
            language: fields[1].clone(),
            description: fields[2].clone(),
            url: fields[3].clone(),
            version: fields[4].clone(),
            players: fields[5].trim().parse().ok(),
            // Kept exactly as received. A public listserver has been seen
            // sending a host with a leading dot (".163.252.231.43"); that
            // is how the server stored it, not a parse error, so it is
            // not repaired here.
            host: fields[6].clone(),
            port: fields[7].trim().parse().unwrap_or(0),
        });
    }
    out
}

/// Parse a decompressed listserver reply body.
pub fn parse_list_reply(body: &[u8]) -> ListReply {
    let mut reply = ListReply::default();
    for msg in split_messages(body) {
        let text = || String::from_utf8_lossy(&msg.payload).into_owned();
        match msg.id {
            0 => reply.servers.extend(parse_svrlist(&msg.payload)),
            2 => reply.status = Some(text().replace('\r', "\n")),
            3 => reply.site_url = Some(text()),
            4 => reply.error = Some(text()),
            5 => reply.upgrade_url = Some(text()),
            _ => {}
        }
    }
    reply
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gs(s: &str) -> Vec<u8> {
        let mut v = vec![s.len() as u8 + 0x20];
        v.extend_from_slice(s.as_bytes());
        v
    }

    #[test]
    fn parse_svrlist_one_server() {
        let mut body = vec![0x20, 0x21, 0x28];
        for f in ["P Classic", "English", "desc", "http://x", "2.05", "3", "127.0.0.1", "14802"] {
            body.extend(gs(f));
        }
        body.push(b'\n');
        body.extend_from_slice(b"\"hello\rworld\n");
        let reply = parse_list_reply(&body);
        assert_eq!(reply.servers.len(), 1);
        let s = &reply.servers[0];
        assert_eq!(s.kind, "P");
        assert_eq!(s.name, "Classic");
        assert_eq!(s.port, 14802);
        assert_eq!(s.players, Some(3));
        assert_eq!(reply.status.as_deref(), Some("hello\nworld"));
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn check_lab_reply(reply: &ListReply) {
        assert_eq!(reply.servers.len(), 1);
        let s = &reply.servers[0];
        assert_eq!(s.kind, "");
        assert_eq!(s.name, "Lab205");
        assert_eq!(s.language, "English");
        assert_eq!(s.players, Some(0));
        assert_eq!(s.host, "127.0.0.1");
        assert_eq!(s.port, 14802);
        assert!(reply.status.is_some());
    }

    /// First server frame from the lab listserver stub, 2.05 login on
    /// 14921: plain zlib, no cipher.
    #[test]
    fn lab_reply_205() {
        let body = crate::protocol::zlib_decompress(&unhex("789c5350d450f3494c3232305577cd4bcfc92cced0f4ad54084e2d2a4b2d32c9282929b0d2d72f2f2fd74b2f4a4cccd1cbccd3b735d633d0b3d44d4a2d4954d030323032d33534d03532543034b33230d45434d034343207aa30d033543534b13030e252021aaf0034b8a4186c2817005a691f8b")).unwrap();
        check_lab_reply(&parse_list_reply(&body));
    }

    /// Same reply to a 2.22 login on 14922: gen-5 frame, key 102, zlib.
    #[test]
    fn lab_reply_222() {
        use crate::protocol::{Gen5, Gen5Comp};
        let frame = unhex("040684f41a6d786486a22eb866af40715864c8e5eeac55038e6e3af9042984b6508a24c1e0db1b74fb879f0f5ab5bbfb41e8b8bfade332a7493f3ef00cf11d6237e5fc72d69652708cde2f38aff67a40c28b1942f6a084a4374c4618b877c4ba25cd72aa07216fb93b93349bca438573faa10e18fea592c6e60b362723bf");
        assert_eq!(frame[0], Gen5Comp::Zlib as u8);
        let raw = Gen5::new(102).decode(frame[1..].to_vec(), Gen5Comp::Zlib);
        let body = crate::protocol::zlib_decompress(&raw).unwrap();
        check_lab_reply(&parse_list_reply(&body));
    }

    /// One row in the layout the public listserver sent on 2026-10-25
    /// (item 14): field count 8, then type+name, language, description, url,
    /// version, players, host, port.
    fn public_row(name: &str, host: &str, port: &str) -> Vec<u8> {
        let mut r = vec![8 + 0x20];
        for f in [name, "English", "desc", "https://x", "Beta Version: 3.0.9", "0", host, port] {
            r.extend(gs(f));
        }
        r
    }

    fn public_list(rows: &[Vec<u8>]) -> Vec<u8> {
        let mut b = vec![0x20, rows.len() as u8 + 0x20];
        for r in rows {
            b.extend_from_slice(r);
        }
        b.push(b'\n');
        b
    }

    /// The public list text: eight rows come back as eight rows, a row
    /// without a type prefix is silver (`-`), a host with a leading dot and
    /// a name with `?` bytes are kept exactly as sent (the pcap shows the
    /// server sent both), and the two NUL bytes that follow the last
    /// newline in the real frames add nothing.
    #[test]
    fn public_list_layout_eight_rows() {
        let mut rows = vec![
            public_row("P Alpha", "10.0.0.1", "14801"),
            public_row("U Beta", "login.example", "14911"),
            public_row("H Jojo???s", "10.0.0.2", "30903"),
            public_row("Gamma", ".163.252.231.43", "14802"),
        ];
        for i in 0..4 {
            rows.push(public_row(&format!("H Row{i}"), "h.example", "14842"));
        }
        let mut body = public_list(&rows);
        body.extend_from_slice(&[0, 0]);
        let reply = parse_list_reply(&body);
        assert_eq!(reply.servers.len(), 8);
        assert_eq!(reply.servers[2].name, "Jojo???s");
        assert_eq!(reply.servers[2].kind, "H");
        assert_eq!(reply.servers[3].kind, "");
        assert_eq!(reply.servers[3].host, ".163.252.231.43");
        assert_eq!(reply.servers[3].port, 14802);
        assert_eq!(reply.servers[7].name, "Row3");
        assert!(reply.status.is_none() && reply.error.is_none());
    }

    /// The status frame of the public service: id 2 status text (two
    /// lines, `\r` between), id 3 site URL, id 5 upgrade URL, then two NULs.
    /// The "N server(s) online" number is the server's own text and is not a
    /// row count (it said 9 over a list of 8, and over 12 on port 14923).
    #[test]
    fn public_status_frame_layout() {
        let mut body = b"\"Welcome to X, a.\rThere are 9 server(s) online.\n#https://s.example\n%https://u.example\n".to_vec();
        body.extend_from_slice(&[0, 0]);
        let r = parse_list_reply(&body);
        assert_eq!(r.status.as_deref(), Some("Welcome to X, a.\nThere are 9 server(s) online."));
        assert_eq!(r.site_url.as_deref(), Some("https://s.example"));
        assert_eq!(r.upgrade_url.as_deref(), Some("https://u.example"));
        assert!(r.servers.is_empty());
    }

    /// A row with fewer than 8 fields is skipped without disturbing the
    /// rows after it (not seen on the public service; kept as the rule).
    #[test]
    fn short_row_is_skipped_and_the_next_row_still_parses() {
        let mut short = vec![3 + 0x20];
        for f in ["P Short", "English", "d"] {
            short.extend(gs(f));
        }
        let rows = [public_row("P A", "h", "1"), short, public_row("P B", "h", "2")];
        let reply = parse_list_reply(&public_list(&rows));
        let names: Vec<_> = reply.servers.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["A", "B"]);
    }

    #[test]
    fn display_line_shows_all_columns() {
        let e = ServerEntry {
            kind: "P".into(),
            name: "Mock".into(),
            version: "Version: 1".into(),
            host: "127.0.0.1".into(),
            port: 14802,
            players: Some(3),
            ..Default::default()
        };
        assert_eq!(e.display_line(2), "  2) [P] Mock  127.0.0.1:14802  Version: 1  players: 3");
        let silver = ServerEntry { players: None, ..Default::default() };
        assert!(silver.display_line(0).starts_with("  0) [-] "));
        assert!(silver.display_line(0).ends_with("players: ?"));
    }
}
