//! Server messages that every public 2.22 gserver sends at login and that
//! earlier Seihai counted as "unhandled".
//!
//! What is decoded here is backed by two things: the layouts in gs2lib's
//! `IEnums.h` and Graal_analysis `protocol/packet-ids.md`, and the payload
//! shapes seen in the 2026-10-24 public 2.22 captures (four gservers).
//! Anything that is not backed by both is left out; see the list at the end
//! of `documentation/protocol.md`.
//!
//! Every parser returns `None` (or an empty list) on a payload it does not
//! understand and never panics.

use super::dispatch::{latin1, read_gint5, split_quoted};

/// A map picture spec from `PLO_BIGMAP` (171) or `PLO_MINIMAP` (172):
/// `txt,png,x,y` as one comma-separated line.
///
/// Seen: `bigmap2.txt,bigmap.png,0,0` and `minimap.txt,map2.png,0,0`. The
/// text file is the map's level grid and the PNG its picture; what `x` and
/// `y` position is not established (both were 0 in every capture). The
/// 2.17.1/2.22/2.31 clients read fields 2 and 3 as floating point numbers
/// and use the spec only with at least 4 fields (171 and 172 share one
/// routine).
#[derive(Debug, Clone, PartialEq)]
pub struct MapSpec {
    pub txt: String,
    pub png: String,
    pub x: f32,
    pub y: f32,
}

/// Parse `txt,png,x,y`. Needs at least four fields, the third and fourth
/// numeric (floating point); extra fields are ignored, as the client does.
pub fn parse_map_spec(payload: &[u8]) -> Option<MapSpec> {
    let text = latin1(payload);
    let parts: Vec<&str> = text.trim_end_matches(['\r', '\n']).split(',').collect();
    let [txt, png, x, y, ..] = parts[..] else {
        return None;
    };
    Some(MapSpec {
        txt: txt.trim().to_string(),
        png: png.trim().to_string(),
        x: x.trim().parse().ok()?,
        y: y.trim().parse().ok()?,
    })
}

/// `PLO_SIGNATURE` (25): one GCHAR. gs2 servers send 73 (`i`); GServer-RS
/// sent 41 (`I`). What the real client does with it is in
/// [`signature_action`]; those two values fall in the "ignored" case.
pub fn parse_signature(payload: &[u8]) -> Option<u8> {
    payload.first().map(|c| c.wrapping_sub(0x20))
}

/// What the 2.17.1, 2.22 and 2.31 clients do with a `PLO_SIGNATURE` value
/// (2.00 to 2.04 handle 25 too, and the kill-switch is in 2.00 already).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureAction {
    /// 1: the client quits.
    Quit,
    /// 0: the client quits unless the host it connected to is the string
    /// `127.0.0.1`.
    QuitUnlessLoopback,
    /// Any other value (73 and 41 included): ignored. The value is stored.
    Ignore,
}

pub fn signature_action(value: u8) -> SignatureAction {
    match value {
        1 => SignatureAction::Quit,
        0 => SignatureAction::QuitUnlessLoopback,
        _ => SignatureAction::Ignore,
    }
}

/// `PLO_STARTMESSAGE` (41): the client trims the text. If it begins with
/// the RTF header it is stored as `servermessage.rtf` and shown in the RTF
/// viewer, otherwise as `servermessage.html`. Returns the trimmed text and
/// whether it is RTF (`{\rtf` prefix).
pub fn parse_start_message(payload: &[u8]) -> (String, bool) {
    let text = latin1(payload).trim().to_string();
    let rtf = text.starts_with("{\\rtf");
    (text, rtf)
}

/// `PLO_LEVELMODTIME` (39): GINT5 Unix time of the level file. All four
/// captured values decode to dates in 2025 and 2026.
pub fn parse_level_mod_time(payload: &[u8]) -> Option<u32> {
    u32::try_from(read_gint5(payload)?).ok()
}

/// `PLO_GHOSTICON` (174): GCHAR, above 0 shows the ghost icon. The captures
/// sent `0x20` (0) from three servers and a raw `0x00` from GServer-RS; both
/// mean off.
pub fn parse_ghost_icon(payload: &[u8]) -> bool {
    super::layouts::parse_flag(payload) == Some(true)
}

/// A comma-separated list as GServer writes it, with optional double quotes
/// around items (`"a","b, c"` or `a,b`). Used by `PLO_STAFFGUILDS` (47),
/// `PLO_STATUSLIST` (180) and `PLO_RPGWINDOW` (179).
pub fn parse_text_list(payload: &[u8]) -> Vec<String> {
    split_quoted(&latin1(payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_spec_as_seen() {
        let m = parse_map_spec(b"bigmap2.txt,bigmap.png,0,0").unwrap();
        assert_eq!((m.txt.as_str(), m.png.as_str(), m.x, m.y), ("bigmap2.txt", "bigmap.png", 0.0, 0.0));
        // The sample in gs2lib's header has non-zero x and y.
        let m = parse_map_spec(b"zodiacminimap.txt,zodiacworldminimap3.png,10,10").unwrap();
        assert_eq!((m.x, m.y), (10.0, 10.0));
        // The client reads x and y as floats and ignores fields past the fourth.
        let m = parse_map_spec(b"a.txt,b.png,10.5,-2.25,extra").unwrap();
        assert_eq!((m.x, m.y), (10.5, -2.25));
        assert_eq!(parse_map_spec(b"a.txt,b.png,1"), None);
        assert_eq!(parse_map_spec(b"a.txt,b.png,x,y"), None);
        assert_eq!(parse_map_spec(b""), None);
    }

    #[test]
    fn signature_rule() {
        assert_eq!(signature_action(1), SignatureAction::Quit);
        assert_eq!(signature_action(0), SignatureAction::QuitUnlessLoopback);
        // Public servers send 73 (and GServer-RS 41): ignored.
        assert_eq!(signature_action(73), SignatureAction::Ignore);
        assert_eq!(signature_action(41), SignatureAction::Ignore);
        assert_eq!(signature_action(255), SignatureAction::Ignore);
    }

    #[test]
    fn start_message_is_trimmed_and_typed() {
        assert_eq!(parse_start_message(b"  <html>hi</html>\r\n"), ("<html>hi</html>".to_string(), false));
        assert_eq!(parse_start_message(b"{\\rtf1\\ansi hello}"), ("{\\rtf1\\ansi hello}".to_string(), true));
        assert_eq!(parse_start_message(b""), (String::new(), false));
    }

    #[test]
    fn signature_and_ghost_icon() {
        assert_eq!(parse_signature(b"i"), Some(73));
        assert_eq!(parse_signature(b"I"), Some(41));
        assert_eq!(parse_signature(b""), None);
        assert!(!parse_ghost_icon(b" "));
        assert!(!parse_ghost_icon(b"\x00"));
        assert!(parse_ghost_icon(b"!"));
        // Above 0 is on, not only 1.
        assert!(parse_ghost_icon(b"\""));
        assert!(!parse_ghost_icon(b""));
    }

    #[test]
    fn level_mod_time_is_gint5_unix_time() {
        // 1_781_284_436 (2026-06-12) as GINT5: 7-bit groups, high first, +0x20.
        let v: u64 = 1_781_284_436;
        let b: Vec<u8> = (0..5).rev().map(|i| (((v >> (7 * i)) & 0x7F) as u8) + 0x20).collect();
        assert_eq!(parse_level_mod_time(&b), Some(1_781_284_436));
        assert_eq!(parse_level_mod_time(&b[..4]), None);
    }

    #[test]
    fn text_lists_quoted_and_plain() {
        assert_eq!(
            parse_text_list(b"\"Welcome to X.\",\"Programmed by A, B, and C.\""),
            vec!["Welcome to X.", "Programmed by A, B, and C."]
        );
        assert_eq!(
            parse_text_list(b"Online,Away,DND,Eating,Hiding,No PMs,RPing,Sparring,PKing").len(),
            9
        );
        assert_eq!(parse_text_list(b"Server,Staff,Manager"), vec!["Server", "Staff", "Manager"]);
        assert!(parse_text_list(b"").is_empty());
    }
}
