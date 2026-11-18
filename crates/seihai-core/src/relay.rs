//! Explicit-endpoint 1.1.9 relay client; local level files remain caller-owned.
//!
//! Wire facts: Graal_analysis `protocol/relay-1998.md` and
//! `client-internals/live-trace-early.md` (qualified original-client captures).
//! This implements the two rooms, pairing and player/chat subset, not HTTP,
//! shared combat, item or board synchronization, or every earlier dialect.
//! It also carries the registration-key check of 15th Graal to 1.1.9
//! (relay type `8`, the key read from `reg.code`; see
//! [`crate::graaldes::code_read`] and Graal_analysis `formats/graalsav.md`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// Seihai resource limits, not a claim about the original's maximum line.
const MAX_LINE: usize = 1024;
const MAX_PENDING: usize = 64 * 1024;
const MAX_PEERS: usize = 256;

fn text(s: &str) -> io::Result<()> {
    if s.len() > 512 || s.bytes().any(|b| b < 32 || b == 127) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid relay text",
        ));
    }
    Ok(())
}
fn number(b: u8) -> Option<u8> {
    (32..160).contains(&b).then(|| b - 32)
}
fn id(s: &[u8]) -> Option<u16> {
    let s = std::str::from_utf8(s.get(..8)?).ok()?;
    if !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = s.parse().ok()?;
    (n < 16384).then_some(n)
}
fn position(x: f32, y: f32, sprite: u8) -> [u8; 3] {
    [
        ((x * 2.0).clamp(0.0, 127.0) as u8) + 32,
        ((y * 2.0).clamp(0.0, 127.0) as u8) + 32,
        (sprite & 127) + 32,
    ]
}

#[derive(Debug, Clone)]
pub struct Player {
    pub nickname: String,
    pub level: String,
    pub x: f32,
    pub y: f32,
    pub sprite: u8,
    pub colors: [u8; 5],
}

#[derive(Debug, Clone)]
pub struct Peer {
    pub id: u16,
    pub nickname: String,
    pub level: String,
    pub x: f32,
    pub y: f32,
    pub sprite: u8,
    pub colors: [u8; 5],
    pub chat: String,
}
impl Peer {
    fn new(id: u16) -> Self {
        Self {
            id,
            nickname: String::new(),
            level: String::new(),
            x: 30.0,
            y: 30.0,
            sprite: 2,
            colors: [0; 5],
            chat: String::new(),
        }
    }
}

struct Room {
    socket: TcpStream,
    input: Vec<u8>,
    output: VecDeque<u8>,
    own_id: Option<u16>,
    available: BTreeSet<u16>,
    party: BTreeSet<u16>,
}
impl Room {
    fn connect(addr: &[std::net::SocketAddr], room: &str) -> io::Result<Self> {
        let mut last = io::Error::new(io::ErrorKind::NotFound, "no relay address");
        let mut socket = None;
        for a in addr {
            match TcpStream::connect_timeout(a, Duration::from_secs(3)) {
                Ok(s) => {
                    socket = Some(s);
                    break;
                }
                Err(e) => last = e,
            }
        }
        let socket = socket.ok_or(last)?;
        socket.set_nonblocking(true)?;
        let mut r = Self {
            socket,
            input: Vec::new(),
            output: VecDeque::new(),
            own_id: None,
            available: BTreeSet::new(),
            party: BTreeSet::new(),
        };
        r.line(format!("N{room}#def").as_bytes())?;
        Ok(r)
    }
    fn line(&mut self, line: &[u8]) -> io::Result<()> {
        if line.len() > MAX_LINE
            || line.contains(&b'\n')
            || line.contains(&b'\r')
            || self.output.len() + line.len() + 1 > MAX_PENDING
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "relay line or queue limit",
            ));
        }
        self.output.extend(line);
        self.output.push_back(b'\n');
        Ok(())
    }
    fn data(&mut self, kind: u8, body: &[u8]) -> io::Result<()> {
        let Some(id) = self.own_id else {
            return Ok(());
        };
        let mut line = vec![b'D', (id / 128) as u8 + 32, (id % 128) as u8 + 32, kind];
        line.extend(body);
        self.line(&line)
    }
    fn flush(&mut self) -> io::Result<()> {
        while !self.output.is_empty() {
            let (a, _) = self.output.as_slices();
            match self.socket.write(a) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => {
                    self.output.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
    fn lines(&mut self) -> io::Result<Vec<Vec<u8>>> {
        self.flush()?;
        let mut lines = Vec::new();
        let mut buf = [0; 1024];
        // Bound one frame's work even when a relay keeps sending.
        for _ in 0..64 {
            match self.socket.read(&mut buf) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => {
                    self.input.extend_from_slice(&buf[..n]);
                    while let Some(end) = self.input.iter().position(|&b| b == b'\n') {
                        if end > MAX_LINE {
                            return Err(io::ErrorKind::InvalidData.into());
                        }
                        let mut line: Vec<_> = self.input.drain(..=end).collect();
                        line.pop();
                        if line.last() == Some(&b'\r') {
                            line.pop();
                        }
                        lines.push(line);
                    }
                    if self.input.len() > MAX_LINE {
                        return Err(io::ErrorKind::InvalidData.into());
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(lines)
    }
    fn push(&mut self, player: &Player, clock: u128, key: Option<&[u8]>) -> io::Result<()> {
        self.data(b'L', player.level.as_bytes())?;
        self.data(b'P', &position(player.x, player.y, player.sprite))?;
        self.data(b'N', player.nickname.as_bytes())?;
        let mut colors = vec![37];
        colors.extend(player.colors.map(|c| (c & 127) + 32));
        self.data(b'S', &colors)?;
        self.data(b'7', &[])?;
        if let Some(key) = key {
            self.data(b'8', key)?;
        }
        let time: Vec<_> = (0..10)
            .map(|i| ((clock >> (i * 7)) & 127) as u8 + 32)
            .collect();
        self.data(b'T', &time)
    }
}
impl Drop for Room {
    fn drop(&mut self) {
        let _ = self.line(b"-");
        let _ = self.line(b"C");
        let _ = self.flush();
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}

/// Two-channel 1.1.9 relay connection. Call `poll` regularly from the game loop.
/// The client requests its first available peer. A compatible relay merges
/// whole parties and confirms every resulting membership; the generated
/// three-client test covers this in both rooms, without claiming world sync.
pub struct Client {
    lobby: Room,
    level: Room,
    player: Player,
    clock: u128,
    peers: BTreeMap<u16, Peer>,
    chats: Vec<(u16, String)>,
    registration: Option<Vec<u8>>,
    duplicate_registration: bool,
}
impl Client {
    pub fn connect(endpoint: &str, player: Player) -> io::Result<Self> {
        text(&player.nickname)?;
        text(&player.level)?;
        if player.level.contains('#') || !player.x.is_finite() || !player.y.is_finite() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let addr: Vec<_> = endpoint.to_socket_addrs()?.collect();
        Ok(Self {
            lobby: Room::connect(&addr, "Graal8")?,
            level: Room::connect(&addr, &format!("G8{}", player.level))?,
            // Rebuild clock value only: the documented ten-character shape
            // does not establish the original epoch/unit. No leader election
            // is inferred from this value in this player/chat subset.
            player,
            clock: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            peers: BTreeMap::new(),
            chats: Vec::new(),
            registration: None,
            duplicate_registration: false,
        })
    }
    /// The registration key this client shows its peers (relay type `8`,
    /// sent with the player's state each time a peer joins), as 15th Graal
    /// to 1.1.9 broadcast the key read from `reg.code`. Set it before
    /// polling. A key that does not decode (the client's `###`) should not
    /// be set: when no key is set nothing is sent **[inf]**.
    pub fn set_registration_key(&mut self, key: &str) -> io::Result<()> {
        text(key)?;
        self.registration = Some(key.as_bytes().to_vec());
        Ok(())
    }
    /// True once a peer in the level room has shown the same registration
    /// key as this client. The original then says "Another player uses the
    /// same registration key like you. You will be disconnected." and
    /// leaves; the caller does that.
    pub fn duplicate_registration(&self) -> bool {
        self.duplicate_registration
    }
    pub fn peers(&self) -> impl Iterator<Item = &Peer> {
        self.peers.values()
    }
    pub fn take_chats(&mut self) -> Vec<(u16, String)> {
        std::mem::take(&mut self.chats)
    }
    /// Send chat once the level relay has assigned an identity.
    /// Returns `WouldBlock` before that handshake; poll and retry the message.
    pub fn chat(&mut self, chat: &str) -> io::Result<()> {
        text(chat)?;
        if self.level.own_id.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "relay identity pending; poll before retrying chat",
            ));
        }
        self.level.data(b'C', chat.as_bytes())?;
        self.level.flush()
    }
    pub fn update(&mut self, level: &str, x: f32, y: f32, sprite: u8) -> io::Result<()> {
        text(level)?;
        if level.contains('#') {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        if !x.is_finite() || !y.is_finite() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        if self.player.level == level
            && self.player.x == x
            && self.player.y == y
            && self.player.sprite == sprite
        {
            return Ok(());
        }
        if self.player.level != level {
            self.level.line(b"-")?;
            self.level.line(b"C")?;
            self.level.line(format!("NG8{level}#def").as_bytes())?;
            self.peers.clear();
            self.chats.clear();
            self.level.available.clear();
            self.level.party.clear();
            self.player.level = level.to_owned();
        }
        self.player.x = x;
        self.player.y = y;
        self.player.sprite = sprite;
        self.level.data(b'P', &position(x, y, sprite))?;
        self.level.flush()
    }
    pub fn poll(&mut self) -> io::Result<()> {
        for fixed in [true, false] {
            let room = if fixed {
                &mut self.lobby
            } else {
                &mut self.level
            };
            for line in room.lines()? {
                let Some((&kind, body)) = line.split_first() else {
                    continue;
                };
                match kind {
                    b'I' => room.own_id = id(body),
                    b'+' => {
                        if let Some(peer) = id(body) {
                            if !room.available.contains(&peer) && room.available.len() >= MAX_PEERS
                            {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "relay peer limit",
                                ));
                            }
                            // The client requests its first available peer;
                            // the relay joins whole parties and confirms all
                            // resulting members. Deduplicate advertisements.
                            if room.available.insert(peer) && room.available.len() == 1 {
                                room.line(format!("P{peer:08}").as_bytes())?;
                            }
                        }
                    }
                    b'P' => {
                        if let Some(peer) = id(body) {
                            if !room.party.contains(&peer) && room.party.len() >= MAX_PEERS {
                                return Err(io::ErrorKind::InvalidData.into());
                            }
                            room.party.insert(peer);
                            room.push(&self.player, self.clock, self.registration.as_deref())?;
                        }
                    }
                    b'-' => {
                        if let Some(peer) = id(body) {
                            room.available.remove(&peer);
                        }
                    }
                    b'C' => {
                        if let Some(peer) = id(body) {
                            room.party.remove(&peer);
                            if !fixed {
                                self.peers.remove(&peer);
                            }
                        }
                    }
                    b'D' if !fixed && body.len() >= 3 => {
                        let Some(a) = number(body[0]) else {
                            continue;
                        };
                        let Some(b) = number(body[1]) else {
                            continue;
                        };
                        let peer_id = u16::from(a) * 128 + u16::from(b);
                        if !room.party.contains(&peer_id) {
                            continue;
                        }
                        let peer = self
                            .peers
                            .entry(peer_id)
                            .or_insert_with(|| Peer::new(peer_id));
                        let data = &body[3..];
                        match body[2] {
                            b'L' => peer.level = String::from_utf8_lossy(data).into_owned(),
                            b'N' => peer.nickname = String::from_utf8_lossy(data).into_owned(),
                            b'8' => {
                                if self.registration.as_deref() == Some(data) {
                                    self.duplicate_registration = true;
                                }
                            }
                            b'C' => {
                                peer.chat = String::from_utf8_lossy(data).into_owned();
                                if self.chats.len() >= MAX_PEERS {
                                    self.chats.remove(0);
                                }
                                self.chats.push((peer_id, peer.chat.clone()));
                            }
                            b'P' if data.len() >= 3 => {
                                if let (Some(x), Some(y), Some(sprite)) =
                                    (number(data[0]), number(data[1]), number(data[2]))
                                {
                                    peer.x = f32::from(x) / 2.0;
                                    peer.y = f32::from(y) / 2.0;
                                    peer.sprite = sprite;
                                }
                            }
                            b'S' if data.len() >= 6 && data[0] == 37 => {
                                for (to, from) in peer.colors.iter_mut().zip(&data[1..6]) {
                                    if let Some(c) = number(*from) {
                                        *to = c;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            room.flush()?;
        }
        Ok(())
    }
}
