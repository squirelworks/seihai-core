//! The Seihai 3D extension: an opt-in, versioned layer on the protocol-24
//! 3D link for Seihai's own 3D client, that a real 5.12 never sees.
//!
//! A real Graal3D client (4.20r8, 5.00r7, 5.12) cannot run anything new:
//! an unknown ghost class drops its link, and its limits (a 4096-slot ghost
//! table, one 256 by 256 terrain grid, terrain lit only at mission phase 2,
//! a terrain cache keyed by server name) are fixed in the binary. So the
//! extension is never offered, only asked for, and both ends must agree:
//!
//! 1. **Hello.** A Seihai client adds one connect-request argument after
//!    the account and the nickname: `seihai3d/<version>/<caps>`, the caps
//!    as eight hex digits ([`Hello::to_arg`]). A real 5.12 sends only the
//!    account and the nickname, so it never carries one.
//! 2. **Reply.** A server that speaks the extension answers with one
//!    guaranteed `RemoteCommandEvent` named `seihai3d`, arguments
//!    `<version>` and `<caps>` ([`Reply::to_args`]), queued **before**
//!    `MissionStartPhase1`. Classes an agreed capability needs are
//!    appended to the accept's class table after the stock ones, so the
//!    stock class ids never move.
//! 3. **Fallback.** Guaranteed events arrive in order, so a client that
//!    sees `MissionStartPhase1` without the reply is on a server without
//!    the extension and runs as a plain 5.12 ([`ClientState`]). A server
//!    that gets no hello, or one it cannot read, serves the stock protocol
//!    ([`negotiate`]) and sends nothing of the extension.
//!
//! The agreed set is the client's caps, the server's caps and the bits the
//! lower of the two versions defines, intersected. Unknown bits and later
//! versions are ignored, not refused, so a newer client still links to an
//! older server and the other way round.
//!
//! Version history: [`EXT_VERSION`] 1 (2026-11-27). The design and the
//! roadmap behind each bit are in Graal_analysis
//! `torque/seihai3d-ext-v1.md`. This module is the wire contract only: no
//! capability is implemented by it.

use std::fmt;

/// The name of the extension, in the hello argument and the reply command.
pub const EXT_NAME: &str = "seihai3d";

/// The extension version this crate speaks. Raise it when a capability
/// bit is added; a bit's meaning never changes once released.
pub const EXT_VERSION: u16 = 1;

/// The stock protocol the extension rides on. The connect request still
/// says 24 both times, so a server without the extension accepts it.
pub const BASE_PROTOCOL: u32 = 24;

/// The connect-request argument index the hello goes in (0 is the
/// account, 1 the nickname).
pub const HELLO_ARG_INDEX: usize = 2;

/// A set of extension capabilities, one bit each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Caps(pub u32);

impl Caps {
    pub const NONE: Caps = Caps(0);
    /// The client reports its frame rate, frame time and ghost count
    /// (`seihai3dStats` remote commands, about every 5 s).
    pub const FRAME_STATS: Caps = Caps(1 << 0);
    /// Chat in the 3D view: the server sends `seihai3dChat` (account,
    /// text) and the client shows a chat line and bubbles.
    pub const CHAT_3D: Caps = Caps(1 << 1);
    /// The terrain cache is keyed by world as well as by server: the
    /// server names the world (`seihai3dWorld`) before its terrain, so a
    /// world switch sends only the chunks that differ.
    pub const WORLD_TERRAIN_CACHE: Caps = Caps(1 << 2);
    /// The client relights a terrain chunk when it arrives, so the server
    /// need not send `MissionStartPhase2` again after a sliding-window swap.
    pub const CHUNK_RELIGHT: Caps = Caps(1 << 3);
    /// The client reports its draw distance (`seihai3dView`), which may be
    /// past the 5.12's 2400, and the server sizes its object scope by it.
    pub const VIEW_DISTANCE: Caps = Caps(1 << 4);
    /// The client keeps up with one server packet every 10 ms (the stock
    /// pacing is 50 ms).
    pub const FAST_PACING: Caps = Caps(1 << 5);
    /// Ghost indices are 14 bits wide (16384 slots) in place of 12. A wire
    /// change: from the first data packet after the reply.
    pub const WIDE_GHOSTS: Caps = Caps(1 << 6);
    /// Terrain chunks carry world chunk coordinates (signed 16-bit), so
    /// one world is not limited to the 5.12's one wrapped 16 by 16 grid.
    pub const WORLD_CHUNKS: Caps = Caps(1 << 7);

    /// Every bit, with its name, in bit order.
    pub const ALL: &'static [(Caps, &'static str)] = &[
        (Caps::FRAME_STATS, "frame_stats"),
        (Caps::CHAT_3D, "chat_3d"),
        (Caps::WORLD_TERRAIN_CACHE, "world_terrain_cache"),
        (Caps::CHUNK_RELIGHT, "chunk_relight"),
        (Caps::VIEW_DISTANCE, "view_distance"),
        (Caps::FAST_PACING, "fast_pacing"),
        (Caps::WIDE_GHOSTS, "wide_ghosts"),
        (Caps::WORLD_CHUNKS, "world_chunks"),
    ];

    /// The bits an extension version defines (0 for an unknown version 0).
    pub const fn known(version: u16) -> Caps {
        match version {
            0 => Caps(0),
            // Version 1 defines bits 0 to 7. Later versions add bits here.
            _ => Caps(0xFF),
        }
    }

    pub const fn contains(self, other: Caps) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn and(self, other: Caps) -> Caps {
        Caps(self.0 & other.0)
    }

    pub const fn or(self, other: Caps) -> Caps {
        Caps(self.0 | other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The names of the known bits that are set.
    pub fn names(self) -> Vec<&'static str> {
        Caps::ALL
            .iter()
            .filter(|(c, _)| self.contains(*c))
            .map(|(_, n)| *n)
            .collect()
    }

    fn hex(self) -> String {
        format!("{:08x}", self.0)
    }

    fn parse_hex(s: &[u8]) -> Option<Caps> {
        if s.is_empty() || s.len() > 8 || !s.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        u32::from_str_radix(std::str::from_utf8(s).ok()?, 16)
            .ok()
            .map(Caps)
    }
}

impl fmt::Display for Caps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = self.names();
        if names.is_empty() {
            write!(f, "none")
        } else {
            write!(f, "{}", names.join(","))
        }
    }
}

fn parse_version(s: &[u8]) -> Option<u16> {
    if s.is_empty() || s.len() > 5 || !s.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(s).ok()?.parse().ok()
}

/// The client's hello: the version it speaks and the caps it wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hello {
    pub version: u16,
    pub caps: Caps,
}

impl Hello {
    /// This crate's own hello for `caps`.
    pub const fn new(caps: Caps) -> Hello {
        Hello {
            version: EXT_VERSION,
            caps,
        }
    }

    /// The connect-request argument: `seihai3d/<version>/<caps hex>`.
    pub fn to_arg(&self) -> String {
        format!("{EXT_NAME}/{}/{}", self.version, self.caps.hex())
    }

    /// Reads one argument. Fields after the caps (`/…`) are reserved for
    /// later versions and skipped. Anything else that is not a hello is
    /// `None`.
    pub fn parse(arg: &[u8]) -> Option<Hello> {
        let mut parts = arg.split(|b| *b == b'/');
        if parts.next()? != EXT_NAME.as_bytes() {
            return None;
        }
        let version = parse_version(parts.next()?)?;
        let caps = Caps::parse_hex(parts.next()?)?;
        if version == 0 {
            return None;
        }
        Some(Hello { version, caps })
    }

    /// The hello in a connect request's arguments, if there is one. Only
    /// argument [`HELLO_ARG_INDEX`] and later are looked at, so an account
    /// or nickname that happens to read as one is not taken.
    pub fn find(args: &[Vec<u8>]) -> Option<Hello> {
        args.iter()
            .skip(HELLO_ARG_INDEX)
            .find_map(|a| Hello::parse(a))
    }
}

/// The server's reply: the version and caps both ends will use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reply {
    pub version: u16,
    pub caps: Caps,
}

impl Reply {
    /// The `RemoteCommandEvent` arguments, the command name first.
    pub fn to_args(&self) -> [String; 3] {
        [
            EXT_NAME.to_string(),
            self.version.to_string(),
            self.caps.hex(),
        ]
    }

    /// Reads a `RemoteCommandEvent`'s arguments (command name first).
    /// `None` when it is not the reply or cannot be read.
    pub fn parse(args: &[Vec<u8>]) -> Option<Reply> {
        let [name, version, caps, ..] = args else {
            return None;
        };
        if name.as_slice() != EXT_NAME.as_bytes() {
            return None;
        }
        Some(Reply {
            version: parse_version(version)?,
            caps: Caps::parse_hex(caps)?,
        })
    }
}

/// What a server serves one 3D link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The stock protocol, exactly as for a real 5.12. Nothing of the
    /// extension is sent.
    Stock,
    /// The extension with the agreed version and caps. `caps` may be
    /// empty: the reply still goes out, and the client knows the server
    /// speaks the extension.
    Extended(Reply),
}

impl Mode {
    pub fn caps(self) -> Caps {
        match self {
            Mode::Stock => Caps::NONE,
            Mode::Extended(r) => r.caps,
        }
    }

    pub fn has(self, c: Caps) -> bool {
        self.caps().contains(c)
    }
}

/// The server's choice for a link, from the connect request's arguments,
/// the caps the server enables, and the newest version it speaks.
///
/// No hello, or one that does not read, is [`Mode::Stock`]. Otherwise the
/// version is the lower of the two, and the caps are the client's, the
/// server's and those that version defines.
pub fn negotiate(args: &[Vec<u8>], server_caps: Caps, server_version: u16) -> Mode {
    let Some(hello) = Hello::find(args) else {
        return Mode::Stock;
    };
    if server_version == 0 {
        return Mode::Stock;
    }
    let version = hello.version.min(server_version);
    let caps = hello.caps.and(server_caps).and(Caps::known(version));
    Mode::Extended(Reply { version, caps })
}

/// What a Seihai client is running, as the link's guaranteed events
/// arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientState {
    /// Hello sent; neither the reply nor the mission start seen yet.
    Waiting,
    /// The server replied: these caps are on.
    Extended(Reply),
    /// The mission started with no reply: a server without the
    /// extension. Run as a plain 5.12.
    Stock,
}

impl ClientState {
    /// The state after one guaranteed `RemoteCommandEvent`, command name
    /// first. `hello` is what the client sent; a reply that claims a bit
    /// the client did not ask for has that bit dropped.
    pub fn on_command(self, hello: Hello, args: &[Vec<u8>]) -> ClientState {
        if self != ClientState::Waiting {
            return self;
        }
        if let Some(r) = Reply::parse(args) {
            let version = r.version.min(hello.version);
            let caps = r.caps.and(hello.caps).and(Caps::known(version));
            return ClientState::Extended(Reply { version, caps });
        }
        let is_phase1 = args
            .first()
            .is_some_and(|n| n.as_slice() == b"MissionStartPhase1");
        if is_phase1 {
            ClientState::Stock
        } else {
            self
        }
    }

    pub fn caps(self) -> Caps {
        match self {
            ClientState::Extended(r) => r.caps,
            _ => Caps::NONE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<Vec<u8>> {
        list.iter().map(|s| s.as_bytes().to_vec()).collect()
    }

    #[test]
    fn hello_round_trips() {
        let h = Hello::new(Caps::FRAME_STATS.or(Caps::CHAT_3D));
        assert_eq!(h.to_arg(), "seihai3d/1/00000003");
        assert_eq!(Hello::parse(h.to_arg().as_bytes()), Some(h));
    }

    #[test]
    fn later_fields_are_skipped() {
        let h = Hello::parse(b"seihai3d/3/000001ff/lod=2/x").unwrap();
        assert_eq!((h.version, h.caps), (3, Caps(0x1ff)));
    }

    #[test]
    fn bad_hellos_do_not_read() {
        for bad in [
            &b""[..],
            b"seihai3d",
            b"seihai3d/1",
            b"seihai3d//00000001",
            b"seihai3d/0/00000001",
            b"seihai3d/1/xyz",
            b"seihai3d/1/123456789",
            b"seihai3d/99999999/1",
            b"Seihai3D/1/00000001",
            b"other/1/00000001",
        ] {
            assert_eq!(
                Hello::parse(bad),
                None,
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn a_real_512_request_is_stock() {
        // A 5.12 sends the account and the nickname only.
        let req = args(&["account", "nick"]);
        assert_eq!(negotiate(&req, Caps(0xFF), EXT_VERSION), Mode::Stock);
        // An empty nickname too.
        assert_eq!(
            negotiate(&args(&["account", ""]), Caps(0xFF), 1),
            Mode::Stock
        );
    }

    #[test]
    fn an_account_named_like_a_hello_is_not_one() {
        let req = args(&["seihai3d/1/000000ff", "seihai3d/1/000000ff"]);
        assert_eq!(negotiate(&req, Caps(0xFF), 1), Mode::Stock);
    }

    #[test]
    fn caps_are_the_intersection() {
        let hello = Hello::new(Caps::FRAME_STATS.or(Caps::WIDE_GHOSTS).or(Caps::CHAT_3D));
        let req = args(&["a", "n", &hello.to_arg()]);
        let server = Caps::FRAME_STATS.or(Caps::CHAT_3D).or(Caps::FAST_PACING);
        let m = negotiate(&req, server, 1);
        assert_eq!(
            m,
            Mode::Extended(Reply {
                version: 1,
                caps: Caps::FRAME_STATS.or(Caps::CHAT_3D)
            })
        );
        assert!(m.has(Caps::CHAT_3D));
        assert!(!m.has(Caps::WIDE_GHOSTS));
        assert!(!Mode::Stock.has(Caps::FRAME_STATS));
    }

    #[test]
    fn a_newer_client_meets_an_older_server() {
        // Version 7 asks for bit 20, which version 1 does not define.
        let req = args(&["a", "n", "seihai3d/7/00100001"]);
        assert_eq!(
            negotiate(&req, Caps(u32::MAX), 1),
            Mode::Extended(Reply {
                version: 1,
                caps: Caps::FRAME_STATS
            })
        );
    }

    #[test]
    fn a_server_with_nothing_enabled_still_replies() {
        let req = args(&["a", "n", &Hello::new(Caps(0xFF)).to_arg()]);
        let m = negotiate(&req, Caps::NONE, 1);
        assert_eq!(
            m,
            Mode::Extended(Reply {
                version: 1,
                caps: Caps::NONE
            })
        );
        assert_eq!(negotiate(&req, Caps(0xFF), 0), Mode::Stock);
    }

    #[test]
    fn reply_round_trips() {
        let r = Reply {
            version: 1,
            caps: Caps::WORLD_TERRAIN_CACHE,
        };
        let a: Vec<Vec<u8>> = r.to_args().iter().map(|s| s.as_bytes().to_vec()).collect();
        assert_eq!(a[0], b"seihai3d");
        assert_eq!(Reply::parse(&a), Some(r));
        assert_eq!(Reply::parse(&args(&["seihai3d", "1"])), None);
        assert_eq!(Reply::parse(&args(&["MissionStartPhase1", "1", "0"])), None);
    }

    #[test]
    fn client_falls_back_when_the_mission_starts_first() {
        let hello = Hello::new(Caps(0xFF));
        let s = ClientState::Waiting
            .on_command(hello, &args(&["SomethingElse"]))
            .on_command(hello, &args(&["MissionStartPhase1", "x"]));
        assert_eq!(s, ClientState::Stock);
        // A reply after the mission start is too late and is ignored.
        let s = s.on_command(hello, &args(&["seihai3d", "1", "000000ff"]));
        assert_eq!(s, ClientState::Stock);
        assert!(s.caps().is_empty());
    }

    #[test]
    fn client_drops_bits_it_did_not_ask_for() {
        let hello = Hello::new(Caps::FRAME_STATS);
        let s = ClientState::Waiting.on_command(hello, &args(&["seihai3d", "1", "000000ff"]));
        assert_eq!(s.caps(), Caps::FRAME_STATS);
        // Later commands do not change it.
        let s = s.on_command(hello, &args(&["MissionStartPhase1"]));
        assert_eq!(s.caps(), Caps::FRAME_STATS);
    }

    #[test]
    fn names_and_known_bits() {
        assert_eq!(Caps(0x05).to_string(), "frame_stats,world_terrain_cache");
        assert_eq!(Caps::NONE.to_string(), "none");
        let all = Caps::ALL.iter().fold(Caps::NONE, |a, (c, _)| a.or(*c));
        assert_eq!(all, Caps::known(EXT_VERSION));
        assert_eq!(Caps::known(0), Caps::NONE);
    }
}
