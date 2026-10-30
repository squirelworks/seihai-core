//! Minimal pcap reader and TCP stream reassembly (no external crates).
//!
//! Reads a classic libpcap file (either byte order, micro- or nanosecond
//! stamps; Ethernet, Linux cooked, raw IP and BSD loopback link types),
//! keeps IPv4 TCP segments, and rebuilds the two byte streams of each
//! connection in order. Every byte keeps the time of the packet that
//! delivered it, so a decoder can stamp each frame.
//!
//! Capture files hold login frames with the placeholder password: keep them
//! in a private directory, never in a repository.

use std::collections::BTreeMap;

/// One TCP segment.
#[derive(Debug, Clone)]
pub struct Packet {
    /// Seconds since the epoch.
    pub ts: f64,
    pub src: ([u8; 4], u16),
    pub dst: ([u8; 4], u16),
    pub seq: u32,
    pub syn: bool,
    pub ack: bool,
    pub payload: Vec<u8>,
}

fn rd16(b: &[u8], le: bool) -> u16 {
    let a = [b[0], b[1]];
    if le { u16::from_le_bytes(a) } else { u16::from_be_bytes(a) }
}

fn rd32(b: &[u8], le: bool) -> u32 {
    let a = [b[0], b[1], b[2], b[3]];
    if le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) }
}

/// Parse a pcap file into its IPv4 TCP segments, in file order.
pub fn read_pcap(data: &[u8]) -> Result<Vec<Packet>, String> {
    if data.len() < 24 {
        return Err("pcap: file shorter than the global header".into());
    }
    let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let (le, nanos) = match magic {
        0xa1b2_c3d4 => (true, false),
        0xd4c3_b2a1 => (false, false),
        0xa1b2_3c4d => (true, true),
        0x4d3c_b2a1 => (false, true),
        _ => return Err("pcap: bad magic (not a classic pcap file)".into()),
    };
    let link = rd32(&data[20..24], le);
    let mut out = Vec::new();
    let mut i = 24;
    while i + 16 <= data.len() {
        let sec = rd32(&data[i..], le);
        let frac = rd32(&data[i + 4..], le);
        let caplen = rd32(&data[i + 8..], le) as usize;
        i += 16;
        if i + caplen > data.len() {
            break; // truncated last record
        }
        let rec = &data[i..i + caplen];
        i += caplen;
        let ts = f64::from(sec) + f64::from(frac) / if nanos { 1e9 } else { 1e6 };
        let ip = match link {
            1 => ethernet_payload(rec),
            113 => rec.get(16..).filter(|_| rec.len() > 15 && rd16(&rec[14..], false) == 0x0800),
            // Linux cooked v2 (`tcpdump -i any` on current libpcap): 20-byte
            // header, protocol type in the first two bytes.
            276 => rec.get(20..).filter(|_| rec.len() > 19 && rd16(rec, false) == 0x0800),
            101 | 12 => Some(rec),
            0 => rec.get(4..),
            _ => None,
        };
        if let Some(p) = ip.and_then(|ip| parse_ipv4_tcp(ip, ts)) {
            out.push(p);
        }
    }
    Ok(out)
}

fn ethernet_payload(rec: &[u8]) -> Option<&[u8]> {
    let mut off = 12;
    let mut ty = rd16(rec.get(off..off + 2)?, false);
    off += 2;
    while ty == 0x8100 || ty == 0x88a8 {
        off += 2; // VLAN tag
        ty = rd16(rec.get(off..off + 2)?, false);
        off += 2;
    }
    (ty == 0x0800).then(|| rec.get(off..)).flatten()
}

fn parse_ipv4_tcp(ip: &[u8], ts: f64) -> Option<Packet> {
    if ip.len() < 20 || ip[0] >> 4 != 4 || ip[9] != 6 {
        return None;
    }
    let ihl = usize::from(ip[0] & 0x0f) * 4;
    let total = usize::from(rd16(&ip[2..], false));
    let end = total.min(ip.len());
    let tcp = ip.get(ihl..end)?;
    if tcp.len() < 20 {
        return None;
    }
    let off = usize::from(tcp[12] >> 4) * 4;
    let flags = tcp[13];
    Some(Packet {
        ts,
        src: ([ip[12], ip[13], ip[14], ip[15]], rd16(tcp, false)),
        dst: ([ip[16], ip[17], ip[18], ip[19]], rd16(&tcp[2..], false)),
        seq: rd32(&tcp[4..], false),
        syn: flags & 0x02 != 0,
        ack: flags & 0x10 != 0,
        payload: tcp.get(off..)?.to_vec(),
    })
}

/// One direction of a TCP connection: the in-order bytes, and where each
/// delivering packet started.
#[derive(Debug, Clone, Default)]
pub struct Stream {
    pub bytes: Vec<u8>,
    /// `(offset of the packet's first new byte, packet time)`.
    pub marks: Vec<(usize, f64)>,
}

impl Stream {
    /// Time of the packet that delivered the byte at `offset`.
    pub fn ts_at(&self, offset: usize) -> f64 {
        match self.marks.binary_search_by(|(o, _)| o.cmp(&offset)) {
            Ok(i) => self.marks[i].1,
            Err(0) => self.marks.first().map_or(0.0, |m| m.1),
            Err(i) => self.marks[i - 1].1,
        }
    }
}

/// One TCP connection, client side first.
#[derive(Debug, Clone)]
pub struct TcpConn {
    pub client: ([u8; 4], u16),
    pub server: ([u8; 4], u16),
    /// Time of the first packet.
    pub t0: f64,
    pub c2s: Stream,
    pub s2c: Stream,
}

#[derive(Default)]
struct Half {
    isn: Option<u32>,
    /// Segments by relative offset.
    segs: BTreeMap<u64, (f64, Vec<u8>)>,
    last_abs: u64,
}

fn rel_offset(h: &mut Half, seq: u32) -> u64 {
    // Unwrap 32-bit sequence numbers relative to the first byte.
    let isn = h.isn.unwrap_or(seq);
    let d = i64::from(seq.wrapping_sub(isn) as i32);
    let mut abs = d.max(0) as u64;
    if d < 0 {
        abs = 0;
    }
    if abs + (1 << 31) < h.last_abs {
        abs += 1 << 32;
    }
    h.last_abs = h.last_abs.max(abs);
    abs
}

/// Rebuild every TCP connection in `packets`. Retransmitted bytes are
/// dropped; a gap ends a stream at the hole. Connections are returned in
/// order of their first packet.
pub fn reassemble(packets: &[Packet]) -> Vec<TcpConn> {
    type Key = (([u8; 4], u16), ([u8; 4], u16));
    struct Acc {
        client: ([u8; 4], u16),
        server: ([u8; 4], u16),
        t0: f64,
        order: usize,
        c: Half,
        s: Half,
    }
    let mut conns: BTreeMap<Key, Acc> = BTreeMap::new();
    for (n, p) in packets.iter().enumerate() {
        let k: Key = if p.src <= p.dst { (p.src, p.dst) } else { (p.dst, p.src) };
        let acc = conns.entry(k).or_insert_with(|| {
            // The first packet decides the client unless a SYN says otherwise.
            Acc { client: p.src, server: p.dst, t0: p.ts, order: n, c: Half::default(), s: Half::default() }
        });
        if p.syn && !p.ack && acc.client != p.src {
            // A pure SYN comes from the client.
            std::mem::swap(&mut acc.client, &mut acc.server);
            std::mem::swap(&mut acc.c, &mut acc.s);
        }
        let from_client = p.src == acc.client;
        let half = if from_client { &mut acc.c } else { &mut acc.s };
        if p.syn {
            half.isn = Some(p.seq.wrapping_add(1));
            continue;
        }
        if half.isn.is_none() {
            half.isn = Some(p.seq);
        }
        if p.payload.is_empty() {
            continue;
        }
        let off = rel_offset(half, p.seq);
        half.segs.entry(off).or_insert((p.ts, p.payload.clone()));
    }
    let mut list: Vec<Acc> = conns.into_values().collect();
    list.sort_by_key(|a| a.order);
    list.into_iter()
        .map(|a| TcpConn {
            client: a.client,
            server: a.server,
            t0: a.t0,
            c2s: build(&a.c),
            s2c: build(&a.s),
        })
        .collect()
}

fn build(h: &Half) -> Stream {
    let mut s = Stream::default();
    let mut next = 0u64;
    for (&off, (ts, data)) in &h.segs {
        if off > next {
            break; // hole
        }
        let skip = (next - off) as usize;
        if skip >= data.len() {
            continue;
        }
        s.marks.push((s.bytes.len(), *ts));
        s.bytes.extend_from_slice(&data[skip..]);
        next = off + data.len() as u64;
    }
    s
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a little-endian Ethernet pcap from `(time, from_client, flags, seq, payload)`.
    pub fn synth_pcap(client_port: u16, server_port: u16, pkts: &[(f64, bool, u8, u32, &[u8])]) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&0xa1b2_c3d4u32.to_le_bytes());
        f.extend_from_slice(&[2, 0, 4, 0]);
        f.extend_from_slice(&[0; 8]);
        f.extend_from_slice(&65535u32.to_le_bytes());
        f.extend_from_slice(&1u32.to_le_bytes());
        for &(t, c2s, flags, seq, payload) in pkts {
            let (sp, dp) = if c2s { (client_port, server_port) } else { (server_port, client_port) };
            let mut fr = vec![0u8; 12];
            fr.extend_from_slice(&[0x08, 0x00]);
            let tcp_len = 20 + payload.len();
            let mut ip = vec![0x45, 0];
            ip.extend_from_slice(&((20 + tcp_len) as u16).to_be_bytes());
            ip.extend_from_slice(&[0, 0, 0, 0, 64, 6, 0, 0, 127, 0, 0, 1, 127, 0, 0, 1]);
            let mut tcp = Vec::new();
            tcp.extend_from_slice(&sp.to_be_bytes());
            tcp.extend_from_slice(&dp.to_be_bytes());
            tcp.extend_from_slice(&seq.to_be_bytes());
            tcp.extend_from_slice(&[0; 4]);
            tcp.push(5 << 4);
            tcp.push(flags);
            tcp.extend_from_slice(&[0; 6]);
            tcp.extend_from_slice(payload);
            fr.extend_from_slice(&ip);
            fr.extend_from_slice(&tcp);
            let sec = t.floor() as u32;
            let usec = ((t - t.floor()) * 1e6).round() as u32;
            f.extend_from_slice(&sec.to_le_bytes());
            f.extend_from_slice(&usec.to_le_bytes());
            f.extend_from_slice(&(fr.len() as u32).to_le_bytes());
            f.extend_from_slice(&(fr.len() as u32).to_le_bytes());
            f.extend_from_slice(&fr);
        }
        f
    }

    #[test]
    fn reassembles_both_directions_in_order_and_drops_retransmits() {
        let pcap = synth_pcap(
            40000,
            14802,
            &[
                (1.0, true, 0x02, 1000, b""),
                (1.001, false, 0x12, 5000, b""),
                (1.002, true, 0x18, 1001, b"hello "),
                (1.003, true, 0x18, 1001, b"hello "), // retransmit
                (1.004, true, 0x18, 1007, b"world"),
                (1.005, false, 0x18, 5001, b"reply"),
            ],
        );
        let pk = read_pcap(&pcap).unwrap();
        assert_eq!(pk.len(), 6);
        let conns = reassemble(&pk);
        assert_eq!(conns.len(), 1);
        let c = &conns[0];
        assert_eq!(c.client.1, 40000);
        assert_eq!(c.server.1, 14802);
        assert_eq!(c.c2s.bytes, b"hello world");
        assert_eq!(c.s2c.bytes, b"reply");
        assert!((c.c2s.ts_at(0) - 1.002).abs() < 1e-6);
        assert!((c.c2s.ts_at(8) - 1.004).abs() < 1e-6);
    }

    #[test]
    fn out_of_order_segments_are_sorted() {
        let pcap = synth_pcap(
            40001,
            14802,
            &[
                (1.0, true, 0x02, 99, b""),
                (1.1, true, 0x18, 106, b"def"),
                (1.2, true, 0x18, 100, b"abcdef"[..3].as_ref()),
                (1.3, true, 0x18, 103, b"def"),
            ],
        );
        let c = &reassemble(&read_pcap(&pcap).unwrap())[0];
        // "def" at 106 leaves a hole at 103..106 until the later segment fills it.
        assert_eq!(c.c2s.bytes, b"abcdefdef");
    }

    #[test]
    fn client_is_the_syn_sender() {
        // The capture starts mid-handshake: the SYN-ACK comes first.
        let pcap = synth_pcap(
            40002,
            14922,
            &[(1.0, false, 0x12, 7, b""), (1.1, true, 0x02, 3, b""), (1.2, true, 0x18, 4, b"x")],
        );
        let c = &reassemble(&read_pcap(&pcap).unwrap())[0];
        assert_eq!(c.client.1, 40002);
        assert_eq!(c.c2s.bytes, b"x");
    }

    /// Rewrite an Ethernet pcap from `synth_pcap` as Linux cooked (v1, link
    /// type 113, 16-byte header) or cooked v2 (276, 20-byte header).
    fn to_cooked(eth: &[u8], link: u32) -> Vec<u8> {
        let hdr = if link == 113 { 16 } else { 20 };
        let mut out = eth[..20].to_vec();
        out.extend_from_slice(&link.to_le_bytes());
        let mut i = 24;
        while i + 16 <= eth.len() {
            let caplen = u32::from_le_bytes([eth[i + 8], eth[i + 9], eth[i + 10], eth[i + 11]]) as usize;
            let ip = &eth[i + 16 + 14..i + 16 + caplen];
            let mut fr = vec![0u8; hdr];
            if link == 113 {
                fr[14..16].copy_from_slice(&[0x08, 0x00]);
            } else {
                fr[0..2].copy_from_slice(&[0x08, 0x00]);
            }
            fr.extend_from_slice(ip);
            out.extend_from_slice(&eth[i..i + 8]);
            out.extend_from_slice(&(fr.len() as u32).to_le_bytes());
            out.extend_from_slice(&(fr.len() as u32).to_le_bytes());
            out.extend_from_slice(&fr);
            i += 16 + caplen;
        }
        out
    }

    #[test]
    fn linux_cooked_v1_and_v2_read_like_ethernet() {
        let eth = synth_pcap(
            40003,
            14801,
            &[(1.0, true, 0x02, 10, b""), (1.1, false, 0x12, 50, b""), (1.2, true, 0x18, 11, b"abc"), (1.3, false, 0x18, 51, b"xyz")],
        );
        let want = reassemble(&read_pcap(&eth).unwrap());
        for link in [113, 276] {
            let c = reassemble(&read_pcap(&to_cooked(&eth, link)).unwrap());
            assert_eq!(c.len(), 1, "link {link}");
            assert_eq!(c[0].c2s.bytes, want[0].c2s.bytes);
            assert_eq!(c[0].s2c.bytes, b"xyz");
            assert_eq!(c[0].server.1, 14801);
        }
    }

    #[test]
    fn rejects_non_pcap() {
        assert!(read_pcap(b"not a pcap file at all, really not").is_err());
        assert!(read_pcap(b"short").is_err());
    }
}
