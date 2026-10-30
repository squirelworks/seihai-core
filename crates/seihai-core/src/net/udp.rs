//! UDP peer channel (2.05).
//!
//! Datagram = `seq` (1 byte) + GCHAR UDP id + fields, the whole datagram
//! XORed with `recipientId XOR 1`. No zlib, no length prefix, no gen-3
//! insertion. `seq` counts from 0 per sender→recipient pair and wraps.
//! Lab-verified against two- and three-player 2.05 captures.
//!
//! IP and port are exchanged via player props 30/31. The client binds
//! `fixedudpport` when set, otherwise an ephemeral port.

use std::net::UdpSocket;

/// UDP peer message ids (own numbering, not PLI/PLO).
pub const UDP_ID_MOVE: u8 = 0;
pub const UDP_ID_BOMBADD: u8 = 10;
pub const UDP_ID_SHOWIMG: u8 = 15;
pub const UDP_ID_ITEMDEL: u8 = 20;
pub const UDP_ID_HIT: u8 = 21;
pub const UDP_ID_CHAT: u8 = 22;
pub const UDP_ID_LEVELNAME: u8 = 23;
pub const UDP_ID_ENTERLEVEL: u8 = 24;
pub const UDP_ID_TRIGGER: u8 = 26;

/// Default peer port (`fixedudpport`) used in the lab.
pub const UDP_PORT: u16 = 14899;

/// XOR key for datagrams sent to player `recipient_id`.
pub fn udp_key(recipient_id: u16) -> u8 {
    (recipient_id as u8) ^ 1
}

/// Build a datagram: `seq` + GCHAR `id` + `fields`, XORed with `key`.
pub fn encode_udp(seq: u8, id: u8, fields: &[u8], key: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + fields.len());
    out.push(seq);
    out.push(id.wrapping_add(0x20));
    out.extend_from_slice(fields);
    for b in &mut out {
        *b ^= key;
    }
    out
}

/// Undo the XOR and split a datagram into `(seq, id, fields)`.
pub fn decode_udp(datagram: &[u8], key: u8) -> Option<(u8, u8, Vec<u8>)> {
    if datagram.len() < 2 {
        return None;
    }
    let plain: Vec<u8> = datagram.iter().map(|b| b ^ key).collect();
    Some((plain[0], plain[1].wrapping_sub(0x20), plain[2..].to_vec()))
}

/// Movement fields (id 0): six GCHARs.
///
/// X×2, Y×2, SPRITE, STATUS, CARRYSPRITE (-1 = none), and a byte that
/// follows the current gani (first letter of its name: `W`alk, `I`dle,
/// `S`word in the lab captures).
pub fn encode_udp_move(x2: u8, y2: u8, sprite: u8, status: u8, carry: i8, gani: u8) -> Vec<u8> {
    vec![
        x2.wrapping_add(0x20),
        y2.wrapping_add(0x20),
        sprite.wrapping_add(0x20),
        status.wrapping_add(0x20),
        (carry as u8).wrapping_add(0x20),
        gani,
    ]
}

/// A UDP peer socket.
pub struct PeerChannel {
    sock: UdpSocket,
}

impl PeerChannel {
    /// Bind to `0.0.0.0:port` (0 for an ephemeral port).
    pub fn bind(port: u16) -> std::io::Result<Self> {
        let sock = UdpSocket::bind(("0.0.0.0", port))?;
        sock.set_nonblocking(true)?;
        Ok(Self { sock })
    }

    /// Local port actually bound.
    pub fn local_port(&self) -> std::io::Result<u16> {
        Ok(self.sock.local_addr()?.port())
    }

    /// Send a raw datagram to `addr`.
    pub fn send_to(&self, data: &[u8], addr: &str) -> std::io::Result<usize> {
        self.sock.send_to(data, addr)
    }

    /// Receive one datagram, if available.
    pub fn recv(&self, buf: &mut [u8]) -> std::io::Result<(usize, std::net::SocketAddr)> {
        self.sock.recv_from(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = udp_key(4);
        assert_eq!(key, 5);
        let fields = encode_udp_move(61, 24, 2, 20, -1, b'W');
        let d = encode_udp(3, UDP_ID_MOVE, &fields, key);
        assert_eq!(d.len(), 8);
        let (seq, id, f) = decode_udp(&d, key).unwrap();
        assert_eq!((seq, id), (3, UDP_ID_MOVE));
        assert_eq!(f, fields);
    }
}
