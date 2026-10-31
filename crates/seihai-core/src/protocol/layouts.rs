//! Server to client payload layouts for ids 49, 75, 150 to 189 and the
//! lower ids that went undecoded.
//!
//! Source: Graal_analysis `client-internals/decomp-findings-2026-10-24-b/`
//! (`plo_layouts.tsv`, read from the 2.17.1, 2.22 and 2.31 receive
//! dispatchers) and the 2026-10-24 lab captures. Which versions handle an id
//! is not decided here: [`super::profile::VersionProfile`]'s `plo` set gates
//! it before a payload reaches these parsers.
//!
//! Number forms: GCHAR is a byte minus 0x20, GSHORT two 7-bit groups, GINT3
//! three. Every parser returns `None` on a payload it cannot read and never
//! panics. Where the client's own reading is only partly known, the field
//! says so.

use super::dispatch::{delphi_comma_text, latin1, read_gint3, read_gshort};

/// A GCHAR: the byte minus 0x20 (wrapping, as the client's subtraction).
fn gchar(b: u8) -> u8 {
    b.wrapping_sub(0x20)
}

/// `PLO_PLAYERWARP2` (49), 2.1 test builds and 2.17 on:
/// `GCHAR x*2, GCHAR y*2, GCHAR z, GCHAR gmapX, GCHAR gmapY`, then the
/// level name. The client reads `z` and does not use it. `gmapX`/`gmapY`
/// matter only when the name ends in `.gmap`; the client then clamps them to
/// the gmap's grid and swaps the name for the cell's level. The lab server
/// followed a gmap warp with `PLO_LEVELNAME` (6) for that cell's level.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerWarp2 {
    /// Tiles (the wire value is half-tile steps).
    pub x: f32,
    pub y: f32,
    /// The z byte as sent, unused by the client.
    pub z_raw: u8,
    pub gmap_x: u8,
    pub gmap_y: u8,
    /// A level file name, or a `.gmap` name.
    pub level: String,
}

impl PlayerWarp2 {
    pub fn is_gmap(&self) -> bool {
        self.level.to_ascii_lowercase().ends_with(".gmap")
    }
}

pub fn parse_player_warp2(payload: &[u8]) -> Option<PlayerWarp2> {
    let [x, y, z, gx, gy, name @ ..] = payload else {
        return None;
    };
    Some(PlayerWarp2 {
        x: f32::from(gchar(*x)) / 2.0,
        y: f32::from(gchar(*y)) / 2.0,
        z_raw: gchar(*z),
        gmap_x: gchar(*gx),
        gmap_y: gchar(*gy),
        level: latin1(name),
    })
}

/// `PLO_PROFILE` (75): repeated `GCHAR n` + `n` characters. The first field
/// is the account the profile belongs to; the rest are its lines. A field
/// that runs past the end of the payload ends the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileCard {
    pub account: String,
    pub lines: Vec<String>,
}

pub fn parse_profile(payload: &[u8]) -> Option<ProfileCard> {
    let mut fields = Vec::new();
    let mut rest = payload;
    while let Some((&n, tail)) = rest.split_first() {
        let n = usize::from(gchar(n));
        let Some(field) = tail.get(..n) else { break };
        fields.push(latin1(field).trim().to_string());
        rest = &tail[n..];
    }
    let mut it = fields.into_iter();
    let account = it.next()?;
    Some(ProfileCard { account, lines: it.collect() })
}

/// `PLO_NPCDEL2` (150): `GCHAR n`, an `n`-character level name, `GINT3` npc id.
pub fn parse_npc_del2(payload: &[u8]) -> Option<(String, usize)> {
    let (&n, tail) = payload.split_first()?;
    let n = usize::from(gchar(n));
    let level = tail.get(..n)?;
    let id = read_gint3(tail.get(n..)?)?;
    Some((latin1(level), id))
}

/// `PLO_MOVE` (165), `PLO_SHOWIMGNPC` (166) and `PLO_MOVE2` (189, 2.31): a
/// `GINT3` npc id, then the rest (move data or an NPC prop list). The client
/// requires an id above 0 for 165. 189 is the same shape and the client
/// calls the move routine with an extra flag.
pub fn parse_npc_id_rest(payload: &[u8]) -> Option<(usize, &[u8])> {
    Some((read_gint3(payload)?, payload.get(3..)?))
}

/// `PLO_SHOOT` (175): `GSHORT` player id, then the shot data. The client
/// uses it only when the player is in the active level.
pub fn parse_shoot(payload: &[u8]) -> Option<(u16, &[u8])> {
    Some((read_gshort(payload)?, payload.get(2..)?))
}

/// A GCHAR flag where above 0 means on (`PLO_GHOSTMODE` 170,
/// `PLO_GHOSTICON` 174). Only the first byte is read.
pub fn parse_flag(payload: &[u8]) -> Option<bool> {
    payload.first().map(|&b| i32::from(b) - 0x20 > 0)
}

/// `PLO_UNKNOWN181` (181): `GSHORT` player id, `GCHAR` toggle (above 0 is
/// on). The client needs at least 3 bytes.
pub fn parse_player_toggle(payload: &[u8]) -> Option<(u16, bool)> {
    Some((read_gshort(payload)?, parse_flag(payload.get(2..)?)?))
}

/// Which mode `PLO_KINGDOMSMODE` (176) or `PLO_NEWWORLDMODE` (177) switches
/// the client main form to. Neither has a payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerMode {
    Kingdoms,
    NewWorld,
}

/// `PLO_SERVERWARP` (178): comma text with at least 4 fields, e.g.
/// `labtwo,labtwo,127.0.0.2,14803`. Field 0 is unused, field 1 is the
/// server's name, field 2 is the host and field 3 the port. A real 2.31
/// client closed its connection and logged in again to field 2 and 3 with a
/// new key and the same credentials, with no listserver step. Reordered
/// host and port made it fail to connect, a wrong field 0 did nothing, and
/// a 3-field payload was ignored entirely (lab, 2026-10-24).
///
/// The text is split by the clients' own comma-text reader (2.17.1, 2.22
/// and 2.31 share it; see [`delphi_comma_text`]): quotes, doubled quotes,
/// and white space that ends an unquoted field. A name with a space in it
/// must therefore be quoted, or it shifts the later fields. The three
/// clients read the fields the same way; field 1 is kept as the server's
/// name (2.31 puts it in the window caption; see
/// [`ServerWarp::display_name`]), not used for connecting. The wire text of
/// a real service is still unobserved: the open-source listserver
/// reimplementation sends `name,name,ip,port`, each quoted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerWarp {
    /// Field 1.
    pub name: String,
    /// Field 2.
    pub host: String,
    /// Field 3, as sent.
    pub port: String,
}

impl ServerWarp {
    /// `host:port` to connect to, if field 3 is a port (decimal, 1 to
    /// 65535) and field 2 is a host. The real client failed to connect when
    /// the two were swapped, so anything else is `None`.
    pub fn address(&self) -> Option<String> {
        let host = self.host.trim();
        let port = self.port.trim();
        if host.is_empty() || host.contains(char::is_whitespace) {
            return None;
        }
        let n: u16 = port.parse().ok().filter(|&n| n != 0)?;
        if host.contains(':') {
            // A bare IPv6 address needs brackets in `host:port`.
            Some(format!("[{host}]:{n}"))
        } else {
            Some(format!("{host}:{n}"))
        }
    }

    /// The name as the client keeps it: the listserver's two-character type
    /// prefix is dropped when the second character is a space (`P Name`
    /// becomes `Name`). The three clients do this to field 1 before they
    /// store it.
    pub fn display_name(&self) -> String {
        let mut it = self.name.chars();
        match (it.next(), it.next()) {
            (Some(_), Some(' ')) => it.collect(),
            _ => self.name.clone(),
        }
    }
}

pub fn parse_server_warp(payload: &[u8]) -> Option<ServerWarp> {
    let f = delphi_comma_text(payload);
    let [_, name, host, port, ..] = &f[..] else { return None };
    Some(ServerWarp { name: latin1(name), host: latin1(host), port: latin1(port) })
}

/// `PLO_SERVERWARP` (178) as the 5.x and 6.x lobby builds read it: the same
/// four fields, split on commas only (see
/// [`super::lobby::split_warp_fields`]), so `0,Lab Server,127.0.0.1,14900`
/// keeps the name `Lab Server`. Fewer than four fields is no warp, as for
/// the 2.x reader.
pub fn parse_server_warp_lobby(payload: &[u8]) -> Option<ServerWarp> {
    let f = super::lobby::split_warp_fields(payload);
    let [_, name, host, port, ..] = &f[..] else { return None };
    Some(ServerWarp { name: latin1(name), host: latin1(host), port: latin1(port) })
}

/// `PLO_SHOWIMG` (32): `GSHORT` sender id, then the sender's `PLI_SHOWIMGPLAYER`
/// payload unchanged. GServer-v2 forwards the payload as it came (a real
/// 2.31 client's is the one byte 9) to every other player in the level
/// without looking inside, so the bytes are kept opaque. A payload of any
/// length, empty included, is fine; fewer than the 2 id bytes is not a
/// message (lab, 2026-10-24).
pub fn parse_showimg_relay(payload: &[u8]) -> Option<(u16, Vec<u8>)> {
    let id = read_gshort(payload)?;
    Some((id, payload[2..].to_vec()))
}

/// `PLO_TAKESCREENSHOT` (184): a file name; the client accepts only the
/// extension `.png` (compared as written here) and saves under that base
/// name in its screenshots folder. Seihai never takes the screenshot.
pub fn parse_screenshot_name(payload: &[u8]) -> Option<String> {
    let name = latin1(payload);
    let name = name.trim_end_matches(['\r', '\n']);
    (name.len() > 4 && name.ends_with(".png")).then(|| name.to_string())
}

/// `PLO_BOARDHEIGHTS` (185), 2.17 on: `GCHAR mapX, mapY, blockX, blockY,
/// w-1, h-1`, then `w*h` cells of two raw bytes, row-major. A cell is
/// `(b0 - 0x20 - 50) + (b1 - 0x20) / 128` (a height with a fraction). `mapX`
/// and `mapY` pick the gmap cell and must lie inside the gmap's grid (the
/// client checks that; Seihai decodes the block and does not apply heights). The client's own length
/// check is only `w*h + 6` bytes although cells take two each; here all
/// `2*w*h` bytes must be present.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardHeights {
    pub map_x: u8,
    pub map_y: u8,
    pub block_x: u8,
    pub block_y: u8,
    pub width: usize,
    pub height: usize,
    pub cells: Vec<f32>,
}

pub fn parse_board_heights(payload: &[u8]) -> Option<BoardHeights> {
    let [mx, my, bx, by, w, h, cells @ ..] = payload else {
        return None;
    };
    let (width, height) = (usize::from(gchar(*w)) + 1, usize::from(gchar(*h)) + 1);
    let cells = cells.get(..2 * width * height)?;
    Some(BoardHeights {
        map_x: gchar(*mx),
        map_y: gchar(*my),
        block_x: gchar(*bx),
        block_y: gchar(*by),
        width,
        height,
        cells: cells
            .chunks_exact(2)
            .map(|c| (f32::from(gchar(c[0])) - 50.0) + f32::from(gchar(c[1])) / 128.0)
            .collect(),
    })
}

/// `PLO_BOARDMODIFY2` (186), 2.22 on: `GCHAR mapX, mapY, x, y, w, h`, then
/// `w*h` GSHORT tiles row-major (the gmap cell's board is replaced from
/// `x`,`y`), then a trailer the client does not read. It needs at least
/// `6 + 2*w*h` bytes; one byte short is dropped without a disconnect and
/// nothing is applied. The trailer is unread padding: bytes after the
/// tiles are ignored, even a whole second record (lab, 2.22 and 2.31,
/// 2026-10-24; Graal_analysis #33). There is no trailer decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardModify2 {
    pub map_x: u8,
    pub map_y: u8,
    pub x: u8,
    pub y: u8,
    pub width: usize,
    pub height: usize,
    pub tiles: Vec<u16>,
    pub trailer: Vec<u8>,
}

pub fn parse_board_modify2(payload: &[u8]) -> Option<BoardModify2> {
    let [mx, my, x, y, w, h, rest @ ..] = payload else {
        return None;
    };
    let (width, height) = (usize::from(gchar(*w)), usize::from(gchar(*h)));
    let body = rest.get(..2 * width * height)?;
    Some(BoardModify2 {
        map_x: gchar(*mx),
        map_y: gchar(*my),
        x: gchar(*x),
        y: gchar(*y),
        width,
        height,
        tiles: body.chunks_exact(2).map(|c| read_gshort(c).unwrap_or(0)).collect(),
        trailer: rest[2 * width * height..].to_vec(),
    })
}

/// `PLO_BOARDMODIFY` (7): `GCHAR x, y, w, h`, then `w*h` GSHORT tiles
/// row-major, for the player's own level. The same shape as 186 without
/// the cell, so it is read into a [`BoardModify2`] with cell (0, 0).
pub fn parse_board_modify(payload: &[u8]) -> Option<BoardModify2> {
    let mut p = vec![0x20, 0x20];
    p.extend_from_slice(payload);
    parse_board_modify2(&p)
}

impl BoardModify2 {
    /// Write the block into a 64-wide board (`board.len()` a multiple of
    /// 64, row by row) at `x`,`y`. A tile past column 63 or past the last
    /// row is clipped, not rejected: the real 2.22 and 2.31 clients drew
    /// only columns 62 and 63 of a block at x 62 (lab, 2026-10-24).
    /// Tiles are masked to 12 bits like the 101 board. Returns how many
    /// tiles were written.
    ///
    /// The session calls this for the level at `mapX`/`mapY` (see
    /// `SessionState`); a cell other than the player's own was never tested
    /// against a real client.
    pub fn apply_to(&self, board: &mut [u16]) -> usize {
        let rows = board.len() / BOARD_SIDE;
        let mut written = 0;
        for row in 0..self.height {
            let by = usize::from(self.y) + row;
            if by >= rows {
                break;
            }
            for col in 0..self.width {
                let bx = usize::from(self.x) + col;
                if bx >= BOARD_SIDE {
                    break;
                }
                board[by * BOARD_SIDE + bx] = self.tiles[row * self.width + col] & 0x0FFF;
                written += 1;
            }
        }
        written
    }
}

/// Tiles per board side.
const BOARD_SIDE: usize = 64;

#[cfg(test)]
mod tests {
    use super::*;

    fn gc(v: u8) -> u8 {
        v + 0x20
    }

    fn gs(v: u16) -> [u8; 2] {
        [gc((v >> 7) as u8 & 0x7F), gc(v as u8 & 0x7F)]
    }

    #[test]
    fn player_warp2_halves_the_position() {
        let mut p = vec![gc(2 * 30 + 1), gc(2 * 12), gc(50), gc(1), gc(0)];
        p.extend_from_slice(b"gm_ne.nw");
        let w = parse_player_warp2(&p).unwrap();
        assert_eq!((w.x, w.y), (30.5, 12.0));
        assert_eq!((w.gmap_x, w.gmap_y, w.z_raw), (1, 0, 50));
        assert_eq!(w.level, "gm_ne.nw");
        assert!(!w.is_gmap());
        p.truncate(5);
        p.extend_from_slice(b"gm2x2.gmap");
        assert!(parse_player_warp2(&p).unwrap().is_gmap());
        assert_eq!(parse_player_warp2(&p[..4]), None);
        // An empty name is still a warp (the client then keeps its level).
        assert_eq!(parse_player_warp2(&p[..5]).unwrap().level, "");
    }

    #[test]
    fn profile_fields() {
        let mut p = Vec::new();
        for f in ["acct", "  Name: X ", "line two"] {
            p.push(gc(f.len() as u8));
            p.extend_from_slice(f.as_bytes());
        }
        let c = parse_profile(&p).unwrap();
        assert_eq!(c.account, "acct");
        assert_eq!(c.lines, vec!["Name: X", "line two"]);
        // A field cut short ends the list.
        p.truncate(p.len() - 3);
        assert_eq!(parse_profile(&p).unwrap().lines, vec!["Name: X"]);
        assert_eq!(parse_profile(b""), None);
    }

    #[test]
    fn npc_ids() {
        let mut p = vec![gc(5)];
        p.extend_from_slice(b"a.nwx");
        p.extend_from_slice(&[gc(0), gc(1), gc(5)]);
        assert_eq!(parse_npc_del2(&p), Some(("a.nwx".to_string(), 133)));
        assert_eq!(parse_npc_del2(&p[..8]), None);
        let m = [gc(0), gc(0), gc(7), b'a', b'b'];
        assert_eq!(parse_npc_id_rest(&m), Some((7, &b"ab"[..])));
        assert_eq!(parse_npc_id_rest(&m[..2]), None);
        let s = [gs(300)[0], gs(300)[1], b'z'];
        assert_eq!(parse_shoot(&s), Some((300, &b"z"[..])));
    }

    #[test]
    fn flags_and_toggles() {
        assert_eq!(parse_flag(b" "), Some(false));
        assert_eq!(parse_flag(b"!"), Some(true));
        assert_eq!(parse_flag(b"\x00"), Some(false));
        assert_eq!(parse_flag(b""), None);
        let t = [gs(9)[0], gs(9)[1], gc(1)];
        assert_eq!(parse_player_toggle(&t), Some((9, true)));
        assert_eq!(parse_player_toggle(&t[..2]), None);
    }

    /// The lab wire text, field 0 ignored, a 3-field payload ignored, and the
    /// reordered forms the real client could not connect with give no address.
    #[test]
    fn server_warp_lab_text_and_field_order() {
        let w = parse_server_warp(b"labtwo,labtwo,127.0.0.2,14803").unwrap();
        assert_eq!((w.name.as_str(), w.address().as_deref()), ("labtwo", Some("127.0.0.2:14803")));
        // Field 0 is not the host; field 1 is not needed to connect.
        let w = parse_server_warp(b"127.0.0.9,labtwo,127.0.0.2,14803").unwrap();
        assert_eq!(w.address().as_deref(), Some("127.0.0.2:14803"));
        let w = parse_server_warp(b"x,zzznoserver,127.0.0.2,14803").unwrap();
        assert_eq!(w.address().as_deref(), Some("127.0.0.2:14803"));
        // Host and port swapped, or name, host, port order: no address.
        assert_eq!(parse_server_warp(b"x,labtwo,14803,127.0.0.2").unwrap().address(), None);
        assert_eq!(parse_server_warp(b"x,127.0.0.2,14803,labtwo").unwrap().address(), None);
        // Only three fields: ignored completely.
        assert_eq!(parse_server_warp(b"x,labtwo,127.0.0.2"), None);
        // Ports out of range, empty host, trailing newline, extra fields.
        for bad in ["0", "65536", "-1", "", "14803 x"] {
            let w = ServerWarp { name: "n".into(), host: "h".into(), port: bad.into() };
            assert_eq!(w.address(), None, "{bad:?}");
        }
        let w = ServerWarp { name: "n".into(), host: "".into(), port: "1".into() };
        assert_eq!(w.address(), None);
        let w = parse_server_warp(b"x,n,host.example,14900\n,extra").unwrap();
        assert_eq!(w.address().as_deref(), Some("host.example:14900"));
        let w = parse_server_warp(b"x,n,::1,14900").unwrap();
        assert_eq!(w.address().as_deref(), Some("[::1]:14900"));
    }

    #[test]
    fn showimg_relay_is_an_id_and_opaque_bytes() {
        // Sender id 2, then the real client's one byte: 9.
        assert_eq!(parse_showimg_relay(&[0x20, 2 + 0x20, 9 + 0x20]), Some((2, vec![9 + 0x20])));
        // Empty payload, and a long one, are kept as sent.
        assert_eq!(parse_showimg_relay(&[0x20, 3 + 0x20]), Some((3, vec![])));
        assert_eq!(parse_showimg_relay(&[0x20, 3 + 0x20, 0, 255, b'\n', b'x']).unwrap().1.len(), 4);
        // Too short for the id.
        assert_eq!(parse_showimg_relay(&[]), None);
        assert_eq!(parse_showimg_relay(&[0x20]), None);
    }

    #[test]
    fn server_warp_uses_fields_1_to_3() {
        let w = parse_server_warp(b"x,\"Main Server\",127.0.0.1,14900").unwrap();
        assert_eq!((w.name.as_str(), w.host.as_str(), w.port.as_str()), ("Main Server", "127.0.0.1", "14900"));
        assert_eq!(parse_server_warp(b"a,b,c"), None);
        // Quoted fields may hold commas; a quote is doubled.
        let w = parse_server_warp(b"\"a,b\",\"c, \"\"d\"\"\",e,f,g").unwrap();
        assert_eq!((w.name.as_str(), w.host.as_str(), w.port.as_str()), ("c, \"d\"", "e", "f"));
    }

    /// The clients read the text with their own comma-text reader: white
    /// space ends an unquoted field, so an unquoted name with a space in it
    /// shifts the later fields (checked in the 2.17.1, 2.22 and 2.31
    /// binaries). Seihai reads it the same way.
    #[test]
    fn server_warp_splits_like_the_clients_list_reader() {
        // Unquoted "Main Server" becomes two fields: host is "Server".
        let w = parse_server_warp(b"x,Main Server,127.0.0.1,14900").unwrap();
        assert_eq!((w.name.as_str(), w.host.as_str(), w.port.as_str()), ("Main", "Server", "127.0.0.1"));
        assert_eq!(w.address(), None);
        // White space around items and commas is dropped.
        let w = parse_server_warp(b"  x ,  n  ,  h.example , 14900  \n").unwrap();
        assert_eq!((w.name.as_str(), w.host.as_str(), w.port.as_str()), ("n", "h.example", "14900"));
        // An empty item in the middle is kept; a comma at the end adds none.
        let w = parse_server_warp(b"x,,h,1,").unwrap();
        assert_eq!((w.name.as_str(), w.host.as_str(), w.port.as_str()), ("", "h", "1"));
        assert_eq!(parse_server_warp(b"x,n,h,"), None);
        // A NUL ends the text.
        assert_eq!(parse_server_warp(b"x,n,h\x00,1"), None);
        // An unclosed quote runs to the end.
        let w = parse_server_warp(b"x,n,h,\"14900").unwrap();
        assert_eq!(w.port, "14900");
    }

    /// The two-character type prefix (`P Name`) is dropped from the name.
    #[test]
    fn server_warp_display_name_drops_the_type_prefix() {
        let n = |s: &str| ServerWarp { name: s.into(), host: "h".into(), port: "1".into() }.display_name();
        assert_eq!(n("P Graal Era"), "Graal Era");
        assert_eq!(n("H x"), "x");
        assert_eq!(n("Plain"), "Plain");
        assert_eq!(n("a"), "a");
        assert_eq!(n("a "), "");
        assert_eq!(n(""), "");
        assert_eq!(n("ab c"), "ab c");
    }

    #[test]
    fn screenshot_name_needs_png() {
        assert_eq!(parse_screenshot_name(b"shot.png"), Some("shot.png".to_string()));
        assert_eq!(parse_screenshot_name(b"shot.jpg"), None);
        assert_eq!(parse_screenshot_name(b".png"), None);
        assert_eq!(parse_screenshot_name(b""), None);
    }

    #[test]
    fn board_heights_cells() {
        // 2x1 block at (3,4) of gmap cell (1,0); cells 50 -> 0.0, 53.5 -> 3.5.
        let mut p = vec![gc(1), gc(0), gc(3), gc(4), gc(1), gc(0)];
        p.extend_from_slice(&[gc(50), gc(0), gc(53), gc(64)]);
        let h = parse_board_heights(&p).unwrap();
        assert_eq!((h.map_x, h.map_y, h.block_x, h.block_y, h.width, h.height), (1, 0, 3, 4, 2, 1));
        assert_eq!(h.cells, vec![0.0, 3.5]);
        // Cells must be present in full.
        assert_eq!(parse_board_heights(&p[..p.len() - 1]), None);
        assert_eq!(parse_board_heights(&p[..5]), None);
    }

    #[test]
    fn board_modify2_tiles_and_trailer() {
        let mut p = vec![gc(0), gc(1), gc(10), gc(11), gc(2), gc(1)];
        p.extend_from_slice(&gs(0x2a5));
        p.extend_from_slice(&gs(7));
        let m = parse_board_modify2(&p).unwrap();
        assert_eq!((m.map_x, m.map_y, m.x, m.y, m.width, m.height), (0, 1, 10, 11, 2, 1));
        assert_eq!(m.tiles, vec![0x2a5, 7]);
        assert!(m.trailer.is_empty());
        p.extend_from_slice(b"tail");
        assert_eq!(parse_board_modify2(&p).unwrap().trailer, b"tail");
        // Needs 6 + 2*w*h bytes.
        assert_eq!(parse_board_modify2(&p[..9]), None);
        // Zero-sized rectangles are fine.
        assert!(parse_board_modify2(&[gc(0); 6]).is_some());
    }

    /// No payload may panic any parser.
    #[test]
    fn parsers_never_panic() {
        let mut samples: Vec<Vec<u8>> = vec![vec![], vec![0], vec![0xff], vec![0x20; 3], vec![0x7f; 40]];
        for n in 0..14usize {
            samples.push((0..n as u8).map(|i| i.wrapping_mul(37).wrapping_add(0x1b)).collect());
            samples.push(vec![0xff; n]);
            samples.push(vec![0x20; n]);
        }
        for s in &samples {
            let _ = parse_player_warp2(s);
            let _ = parse_profile(s);
            let _ = parse_npc_del2(s);
            let _ = parse_npc_id_rest(s);
            let _ = parse_shoot(s);
            let _ = parse_flag(s);
            let _ = parse_player_toggle(s);
            let _ = parse_server_warp(s);
            let _ = parse_screenshot_name(s);
            let _ = parse_board_heights(s);
            let _ = parse_board_modify2(s);
        }
        // Largest rectangles a GCHAR can ask for with nothing behind them.
        assert_eq!(parse_board_heights(&[0xff; 6]), None);
        assert_eq!(parse_board_modify2(&[0xff; 6]), None);
    }

    /// A 186 payload with the given block and `extra` bytes after the tiles.
    fn p186(x: u8, y: u8, w: u8, h: u8, tiles: &[u16], extra: &[u8]) -> Vec<u8> {
        let mut p = vec![gc(0), gc(0), gc(x), gc(y), gc(w), gc(h)];
        for t in tiles {
            p.extend_from_slice(&gs(*t));
        }
        p.extend_from_slice(extra);
        p
    }

    /// Lab cases A and B: the block is applied; a trailer is ignored and
    /// kept only as raw bytes.
    #[test]
    fn board_modify2_applies_block_and_ignores_the_trailer() {
        let tiles: Vec<u16> = (1..=8).collect();
        let m = parse_board_modify2(&p186(44, 26, 4, 2, &tiles, &[])).unwrap();
        assert_eq!((m.x, m.y, m.width, m.height), (44, 26, 4, 2));
        let mut board = vec![0u16; 4096];
        assert_eq!(m.apply_to(&mut board), 8);
        assert_eq!(&board[26 * 64 + 44..26 * 64 + 48], &[1, 2, 3, 4]);
        assert_eq!(&board[27 * 64 + 44..27 * 64 + 48], &[5, 6, 7, 8]);
        assert_eq!(board.iter().filter(|&&t| t != 0).count(), 8);

        // Five arbitrary trailer bytes: same block, nothing else changes.
        let mut board2 = vec![0u16; 4096];
        let t = parse_board_modify2(&p186(44, 26, 4, 2, &tiles, &[1, 2, 3, 4, 5])).unwrap();
        assert_eq!(t.tiles, m.tiles);
        t.apply_to(&mut board2);
        assert_eq!(board, board2);
    }

    /// Lab case C: a complete second record in the trailer is not applied.
    #[test]
    fn board_modify2_second_record_in_the_trailer_is_not_applied() {
        let first: Vec<u16> = vec![1; 8];
        let second = p186(50, 36, 4, 2, &[9; 8], &[]);
        let m = parse_board_modify2(&p186(44, 32, 4, 2, &first, &second)).unwrap();
        let mut board = vec![0u16; 4096];
        assert_eq!(m.apply_to(&mut board), 8);
        assert_eq!(board.iter().filter(|&&t| t != 0).count(), 8);
        assert_eq!(board[36 * 64 + 50], 0);
        assert_eq!(m.trailer, second);
    }

    /// Lab cases D and E: short by any amount, down to one byte, is dropped.
    #[test]
    fn board_modify2_one_byte_short_is_dropped() {
        let full = p186(40, 36, 4, 2, &[3; 8], &[]);
        assert!(parse_board_modify2(&full).is_some());
        assert_eq!(parse_board_modify2(&full[..full.len() - 1]), None);
        assert_eq!(parse_board_modify2(&full[..full.len() - 2]), None);
        // Declared 4x2 with only 4 tiles behind it.
        assert_eq!(parse_board_modify2(&p186(40, 36, 4, 2, &[3; 4], &[])), None);
        // Through the session: a dropped 186 leaves the earlier one alone
        // and counts as handled (no unhandled entry, no state change).
        let mut st = crate::protocol::SessionState::default();
        let msg = |payload: &[u8]| [&[186 + 0x20][..], payload, b"\n"].concat();
        st.apply_body(&msg(&p186(1, 2, 1, 1, &[7], &[])));
        let before = st.board_modify2.clone();
        st.apply_body(&msg(&full[..full.len() - 1]));
        assert_eq!(st.board_modify2, before);
        assert!(st.unhandled.is_empty());
    }

    /// Lab case F: a block past column 63 is clipped, not rejected.
    #[test]
    fn board_modify2_clips_at_column_63_and_the_last_row() {
        let m = parse_board_modify2(&p186(62, 26, 4, 2, &[1, 2, 3, 4, 5, 6, 7, 8], &[])).unwrap();
        let mut board = vec![0u16; 4096];
        assert_eq!(m.apply_to(&mut board), 4);
        assert_eq!(&board[26 * 64 + 62..26 * 64 + 64], &[1, 2]);
        assert_eq!(&board[27 * 64 + 62..27 * 64 + 64], &[5, 6]);
        // Nothing wrapped onto the next row's start.
        assert_eq!((board[27 * 64], board[28 * 64]), (0, 0));
        // Past the last row too.
        let m = parse_board_modify2(&p186(0, 63, 2, 2, &[1, 2, 3, 4], &[])).unwrap();
        let mut board = vec![0u16; 4096];
        assert_eq!(m.apply_to(&mut board), 2);
        assert_eq!(&board[63 * 64..63 * 64 + 2], &[1, 2]);
        // Start column 64 or more: nothing written.
        let m = parse_board_modify2(&p186(70, 0, 2, 1, &[1, 2], &[])).unwrap();
        assert_eq!(m.apply_to(&mut vec![0u16; 4096]), 0);
    }
}
