//! Protocol definitions.
//!
//! Layouts are provisional where noted, and concrete where taken from
//! cleaned observations of client 2.05 and public sources (GServer-v2,
//! OpenGS1, captures). No decompile material.

mod board;
pub mod chain;
mod compress;
mod dispatch;
pub mod extras;
mod frame;
mod gen3;
mod gen5;
pub mod layouts;
pub mod levelcache;
pub mod lobby;
pub mod npcprops;
pub mod offline_builds;
mod list;
pub mod flagset;
pub mod login;
mod profile;
mod props;
pub mod showimg;
pub mod tampercheck;
pub mod walkcadence;
mod version;

pub use board::BoardModify;
pub use compress::{bz2_compress, bz2_decompress};
pub use dispatch::{
    latin1, read_gint3, read_gshort, script_text, split_messages, sword_power_from_wire, ChatKind, ChatLine, IncomingFile, LevelObject, Message, OtherLooks, OtherPlayer, ServerItem, SessionState, Splitter,
};
pub use extras::MapSpec;
pub use frame::{read_zlib_frame, write_zlib_frame, write_zlib_frame_best, zlib_compress, zlib_compress_best, zlib_decompress};
pub use gen3::Gen3;
pub use gen5::{choose_comp, read_frame, read_frame_len, read_frame_or_plain, write_frame, FrameLen, Gen5, Gen5Comp};
pub use list::{parse_list_reply, parse_svrlist, ListReply, ServerEntry};
pub use props::{
    decode_pos_precise, encode_pos_classic, encode_pos_precise, parse_player_props, parse_player_props_in, parse_player_props_with, PlayerProp,
};
pub use profile::{
    by_label, profile, Basis, BuiltIn, ChatFace, ChatText, CombatRules, DeathRules, Files, Health, HudArt, IdSet, Levels, ListKind, Listserver, LoginShape, PortStatus, Props, Protocol,
    ServerList, ShapeBasis, Ui, VersionProfile, BUILT_IN_14921, BUILT_IN_14922, COMBAT_139, COMBAT_200, COMBAT_201, COMBAT_205, PowerStore, MANNER_139, MANNER_141, MANNER_205, MANNER_222, Blink, HidePose, Mannerisms, ItemDrops, DROPS_133, DROPS_138, DROPS_142, PullRelease, SwimStill, DEATH_205, DEATH_21, DEAD_LIST_PORT, LEGACY_LIST_PORT, LISTSERVER_HOST,
    PROFILES, SILENT_LIST_WAIT, SUPPORTED_LIST_PORT,
};
pub use showimg::{
    apply_showimg_update, describe_showimg_update, parse_showimg_update, showimg_message, ImgField, ImgPart, ImgRecord,
    PlayerImage, PlayerImages, ShowImgInfo, ShowImgSender, ShowImgUpdate,
};
pub use tampercheck::{TamperImage, TamperSender, TamperWords};
pub use version::{ClientVersion, EncryptionGen};

use std::io::{self, Write};

/// Retired provisional packet ids.
///
/// These were early placeholders. The real ids are documented in
/// Graal_analysis `protocol/packet-ids.md`:
/// - login uses client type 0 (not a packet id)
/// - `PLI_LEVELWARP` is 0
/// - `PLI_PLAYERPROPS` is 2
/// - `PLO_LEVELBOARD` is 0, `PLO_LEVELNAME` is 6, etc.
///
/// Kept only so older call sites still compile. Do not use for new code.
#[deprecated(note = "provisional ids retired; use the documented PLI/PLO ids")]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketId {
    Login = 0x01,
    LoginResponse = 0x02,
    LevelRequest = 0x10,
    LevelData = 0x11,
    PlayerUpdate = 0x20,
}

/// Logical client→server type for a walk frame.
/// Wire byte is `logical + 0x20`, so 0x02 appears as `"`.
pub const WALK_LOGICAL: u8 = 0x02;
pub const WALK_WIRE: u8 = WALK_LOGICAL + 0x20;

/// Initial login / handshake.
///
/// Classic shape (v2.05 and captures):
/// ```text
/// Chr((rand & 0x7F) + 0x20) + GNW##### + length-biased account + length-biased password
/// ```
/// Length bias: `nbytes + 0x20`.
#[derive(Debug, Clone)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    /// Eight-character version token, e.g. "GNW07080".
    pub version: String,
    /// Optional fixed prefix for tests. If None, a random printable is used.
    pub prefix: Option<u8>,
}

impl LoginRequest {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
            version: "GNW07080".into(),
            prefix: None,
        }
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    pub fn with_prefix(mut self, prefix: u8) -> Self {
        self.prefix = Some(prefix);
        self
    }

    /// Write the classic login line (no packet id, no framing).
    pub fn write_classic<W: Write>(&self, w: &mut W) -> io::Result<()> {
        let prefix = self.prefix.unwrap_or_else(|| {
            // (rand & 0x7F) + 0x20 — printable ASCII.
            // Fixed seed-free: use a mid-range printable when not testing.
            0x41
        });
        w.write_all(&[prefix])?;

        let ver = self.version.as_bytes();
        if ver.len() != 8 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "version token must be 8 characters",
            ));
        }
        w.write_all(ver)?;

        write_length_biased(w, self.username.as_bytes())?;
        write_length_biased(w, self.password.as_bytes())?;
        Ok(())
    }

    /// Provisional length-prefixed form. Prefer `write_classic` for real servers.
    pub fn write_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        let user = self.username.as_bytes();
        let pass = self.password.as_bytes();
        let ver = self.version.as_bytes();

        w.write_all(&[PacketId::Login as u8])?;
        w.write_all(&(user.len() as u16).to_le_bytes())?;
        w.write_all(user)?;
        w.write_all(&(pass.len() as u16).to_le_bytes())?;
        w.write_all(pass)?;
        w.write_all(&(ver.len() as u16).to_le_bytes())?;
        w.write_all(ver)?;
        Ok(())
    }
}

/// Write a GSTRING: GCHAR length + bytes (at most 95 bytes).
pub fn write_gstring<W: Write>(w: &mut W, s: &str) -> io::Result<()> {
    write_length_biased(w, s.as_bytes())
}

fn write_length_biased<W: Write>(w: &mut W, bytes: &[u8]) -> io::Result<()> {
    let len = bytes.len();
    if len > 0x5F {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "length-biased field exceeds 95 bytes",
        ));
    }
    w.write_all(&[(len as u8) + 0x20])?;
    w.write_all(bytes)?;
    Ok(())
}

/// Write a login line for the given client version.
///
/// Uses the version's client type, token, and length-biased account/password.
/// The login frame itself is clean for all versions covered here.
pub fn write_login<W: Write>(
    w: &mut W,
    version: ClientVersion,
    key: u8,
    account: &str,
    password: &str,
) -> io::Result<()> {
    // Both are GCHARs: type 0 goes out as 0x20, then the key. Gen 2
    // (1.33–1.39) has no key: GServer picks gen 2 when the token follows
    // the type directly.
    w.write_all(&[version.client_type() + 0x20])?;
    if version.encryption() != EncryptionGen::Gen2 {
        w.write_all(&[(key & 0x7F) + 0x20])?;
    }
    w.write_all(version.token().as_bytes())?;
    write_length_biased(w, account.as_bytes())?;
    write_length_biased(w, password.as_bytes())?;
    // 5.00 on: the platform and machine-field list, with no length byte
    // (see `lobby`).
    if let Some(lobby) = version.profile().protocol.lobby {
        w.write_all(&lobby.login_tail())?;
    }
    Ok(())
}

/// Request a level by name.
#[derive(Debug, Clone)]
pub struct LevelRequest {
    pub name: String,
}

impl LevelRequest {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    pub fn write_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        let name = self.name.as_bytes();
        w.write_all(&[PacketId::LevelRequest as u8])?;
        w.write_all(&(name.len() as u16).to_le_bytes())?;
        w.write_all(name)?;
        Ok(())
    }
}

/// Simple player position update (provisional packet id).
/// Position fields use the documented codecs when written with
/// `write_classic` / `write_precise`. The raw `write_to` path stays
/// provisional until a client→server capture confirms the framing.
#[derive(Debug, Clone)]
pub struct PlayerUpdate {
    pub x: f32,
    pub y: f32,
    pub dir: u8,
}

impl PlayerUpdate {
    /// Provisional: raw f32 x/y + dir. Do not use against a real server.
    pub fn write_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        w.write_all(&[PacketId::PlayerUpdate as u8])?;
        w.write_all(&self.x.to_le_bytes())?;
        w.write_all(&self.y.to_le_bytes())?;
        w.write_all(&[self.dir])?;
        Ok(())
    }

    /// Classic encoding: PLPROP-style (pos * 2) as u8. Packet id still provisional.
    pub fn write_classic<W: Write>(&self, w: &mut W) -> io::Result<()> {
        w.write_all(&[PacketId::PlayerUpdate as u8])?;
        w.write_all(&[encode_pos_classic(self.x)])?;
        w.write_all(&[encode_pos_classic(self.y)])?;
        w.write_all(&[self.dir])?;
        Ok(())
    }

    /// Precise encoding: PLPROP_X2/Y2 style u16. Packet id still provisional.
    pub fn write_precise<W: Write>(&self, w: &mut W) -> io::Result<()> {
        w.write_all(&[PacketId::PlayerUpdate as u8])?;
        w.write_all(&encode_pos_precise(self.x).to_le_bytes())?;
        w.write_all(&encode_pos_precise(self.y).to_le_bytes())?;
        w.write_all(&[self.dir])?;
        Ok(())
    }

    /// Short walk frame: wire type `"` (logical 0x02), then char-biased x/y/dir, newline.
    ///
    /// Provisional field split. Matches the captured shape `"..\n`.
    /// Bias is `+ 0x20` on each field byte.
    pub fn write_walk<W: Write>(&self, w: &mut W) -> io::Result<()> {
        w.write_all(&[WALK_WIRE])?;
        w.write_all(&[encode_pos_classic(self.x).wrapping_add(0x20)])?;
        w.write_all(&[encode_pos_classic(self.y).wrapping_add(0x20)])?;
        w.write_all(&[self.dir.wrapping_add(0x20)])?;
        w.write_all(&[b'\n'])?;
        Ok(())
    }
}

/// Decode a short walk frame.
///
/// Expected: `"` + x_char + y_char + dir_char + `\n`.
/// Returns `(x, y, dir)` in the classic scale (unbiased, not yet /2).
/// Provisional until a side-by-side with a known position confirms the split.
pub fn decode_walk(frame: &[u8]) -> Option<(u8, u8, u8)> {
    if frame.len() < 5 {
        return None;
    }
    if frame[0] != WALK_WIRE {
        return None;
    }
    if *frame.last()? != b'\n' {
        return None;
    }
    let x = frame[1].wrapping_sub(0x20);
    let y = frame[2].wrapping_sub(0x20);
    let dir = frame[3].wrapping_sub(0x20);
    Some((x, y, dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_classic_shape() {
        let req = LoginRequest::new("user", "pass")
            .with_version("GNW07080")
            .with_prefix(0x41);
        let mut buf = Vec::new();
        req.write_classic(&mut buf).unwrap();

        // prefix + GNW07080 + len('user')+0x20 + "user" + len('pass')+0x20 + "pass"
        assert_eq!(buf[0], 0x41);
        assert_eq!(&buf[1..9], b"GNW07080");
        assert_eq!(buf[9], 4 + 0x20);
        assert_eq!(&buf[10..14], b"user");
        assert_eq!(buf[14], 4 + 0x20);
        assert_eq!(&buf[15..19], b"pass");
    }

    #[test]
    fn login_rejects_bad_token() {
        let req = LoginRequest::new("u", "p").with_version("GNW");
        let mut buf = Vec::new();
        assert!(req.write_classic(&mut buf).is_err());
    }

    #[test]
    fn walk_roundtrip() {
        let upd = PlayerUpdate {
            x: 30.0,
            y: 30.0,
            dir: 2,
        };
        let mut buf = Vec::new();
        upd.write_walk(&mut buf).unwrap();
        assert_eq!(buf[0], WALK_WIRE);
        assert_eq!(*buf.last().unwrap(), b'\n');

        let (x, y, dir) = decode_walk(&buf).unwrap();
        assert_eq!(x, encode_pos_classic(30.0));
        assert_eq!(y, encode_pos_classic(30.0));
        assert_eq!(dir, 2);
    }

    #[test]
    fn login_versioned_gchar_header() {
        let mut buf = Vec::new();
        write_login(&mut buf, ClientVersion::V205, 102, "ab", "cd").unwrap();
        assert_eq!(&buf[..2], &[0x20, 102 + 0x20]);
        assert_eq!(&buf[2..10], b"GNW03081");
        assert_eq!(&buf[10..], b"\"ab\"cd");
    }

    /// The 5.00r7 and 5.12 login lines as captured (lab, 2026-10-30), up to
    /// the machine fields, which Seihai sends empty. 5.00r7 (key 119):
    /// `%\x97G3D22067%guest(7Y3bdzjfwin,<md5>,"",<md5>`; 5.12 (key 0x4B):
    /// `%kG3D14097%guest(7Y3bdzjfwin,<md5>,<md5>,<md5>,"6.2 9200 "`.
    #[test]
    fn lobby_login_lines_follow_the_captures() {
        let line = |v, key| {
            let mut buf = Vec::new();
            write_login(&mut buf, v, key, "guest", "7Y3bdzjf").unwrap();
            buf
        };
        let a = line(ClientVersion::V500R7, 119);
        assert!(a.starts_with(b"%\x97G3D22067%guest(7Y3bdzjfwin,"), "{a:?}");
        assert_eq!(&a[25..], b"win,\"\",\"\",\"\"");
        let b = line(ClientVersion::V512, 0x4B);
        assert!(b.starts_with(b"%kG3D14097%guest(7Y3bdzjfwin,"), "{b:?}");
        assert!(b.ends_with(b",\"6.2 9200 \""), "{b:?}");
        assert!(line(ClientVersion::V603R7, 0x79).starts_with(b"%\x99G3D0311C%guest(7Y3bdzjfwin,"));
        // The Linux 6.03r7: its own token, `linux` and four empty machine ids.
        let l = line(ClientVersion::V603R7Linux, 0x79);
        assert!(l.starts_with(b"%\x99G3D0511C%guest(7Y3bdzjflinux,"), "{l:?}");
        assert!(l.ends_with(b"linux,\"\",\"\",\"\",\"\""), "{l:?}");
        // Builds before 5.00 end at the password.
        assert!(line(ClientVersion::V3153, 1).ends_with(b"(7Y3bdzjf"));
    }

    #[test]
    fn walk_rejects_non_walk() {
        assert!(decode_walk(b"1-\n").is_none());
        assert!(decode_walk(b"\"11-\n").is_some());
    }
}
