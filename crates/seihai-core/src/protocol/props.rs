//! Selected player property encodings.

use super::profile::IdSet;

/// Position scaled the classic way (pos * 2 as u8).
pub fn encode_pos_classic(pos: f32) -> u8 {
    (pos * 2.0) as u8
}

/// Higher-precision position (PLPROP_X2/Y2/Z2).
/// `abs(pos * 16) << 1`, low bit = sign.
pub fn encode_pos_precise(pos: f32) -> u16 {
    let mut val = ((pos.abs() * 16.0) as u16) << 1;
    if pos < 0.0 {
        val |= 1;
    }
    val
}

pub fn decode_pos_precise(val: u16) -> f32 {
    let sign = if val & 1 != 0 { -1.0 } else { 1.0 };
    sign * ((val >> 1) as f32) / 16.0
}

/// One decoded player property from a prop stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerProp {
    /// Property id (`PLPROP_*`).
    pub id: u8,
    /// Raw value bytes as they appear on the wire (after the id byte).
    pub raw: Vec<u8>,
}

impl PlayerProp {
    /// First value byte as a GCHAR (wire byte - 0x20).
    pub fn gchar(&self) -> Option<u8> {
        self.raw.first().map(|b| b.wrapping_sub(0x20))
    }
}

/// Byte length of a GSTRING (GCHAR length + bytes) at `data[0..]`.
fn gstring_len(data: &[u8]) -> Option<usize> {
    let n = data.first()?.wrapping_sub(0x20) as usize;
    Some(1 + n)
}

/// In the pre-2.1 bow slot (prop 10), whether the value at `data` is a GANI
/// name in the 2.1 string form, `c(len)` + name, rather than a bow power.
///
/// A server running the 2.x generation sends prop 10 as a GANI string to
/// every client, a 2.0x one included: GServer-v2 beta4 reads and writes the
/// prop as a GANI whenever its generation is not classic
/// (`PropertyGaniOrBowGif`), and the lab saw a 2.05 login get a GANI name.
/// The real 2.00 and 2.05 send their own swing as `c(10 + len)` + `sword`,
/// which such a server stores and relays as `c(5)` + `sword`. Read in the bow
/// form that is bow power 5 followed by garbage, and the swing is lost.
///
/// The two forms overlap for a length under 10. The string form is taken
/// when the name is 1 to 9 bytes of what a GANI name holds (lower-case
/// letters, digits, `_`, `-`, `.`, `[`, `]`), starting with a letter, and is
/// followed by the end of the stream or a prop id below 83. A bow power is
/// followed by the next prop's id, and a pre-2.1 prop id is never a
/// lower-case letter (those are ids 65 to 90). A name of 10 bytes or more
/// (`carrystill`) still reads as a bow image; none of the swing names is that
/// long.
pub fn bow_slot_holds_gani(data: &[u8]) -> bool {
    let Some(&first) = data.first() else { return false };
    let n = usize::from(first.wrapping_sub(0x20));
    if !(1..10).contains(&n) {
        return false;
    }
    let Some(name) = data.get(1..1 + n) else { return false };
    let gani_byte = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-.[]".contains(b);
    let next_ok = data.get(1 + n).is_none_or(|b| b.wrapping_sub(0x20) < 83);
    name[0].is_ascii_lowercase() && name.iter().all(gani_byte) && next_ok
}

/// Byte length of one player prop value for `id`, or `None` if the id
/// is unknown or the value runs past the end.
///
/// Encodings follow the cleaned PLPROP table (2.05 forms where they
/// differ from later clients).
fn prop_len(id: u8, data: &[u8], bow_prop10: bool) -> Option<usize> {
    let c = |i: usize| data.get(i).map(|b| b.wrapping_sub(0x20) as usize);
    let len = match id {
        // Pre-2.1 bow slot: c(10 + len) + image, or c(power) under 10.
        10 if bow_prop10 && bow_slot_holds_gani(data) => gstring_len(data)?,
        10 if bow_prop10 => {
            let v = c(0)?;
            if v >= 10 {
                1 + (v - 10)
            } else {
                1
            }
        }
        // str
        // 10 is listed as the pre-2.1 bow slot for 2.05, but GServer-v2
        // sends a plain gani name ("idle") to a 2.05 login.
        0 | 10 | 12 | 20 | 21 | 34 | 35 | 37..=41 | 46..=49 | 52 | 54..=75 | 82 => gstring_len(data)?,
        // c
        1 | 2 | 4..=7 | 15..=19 | 22 | 23 | 26 | 32 | 33 | 43..=45 | 50 | 53 | 81 => 1,
        // i3
        3 | 24 | 27..=29 | 31 | 36 | 76 => 3,
        // s
        14 | 25 | 78..=80 => 2,
        // i5
        30 => 5,
        // sword: c(power + 30) + image, or c(power) with no image
        8 => {
            let v = c(0)?;
            if v > 4 {
                1 + gstring_len(data.get(1..)?)?
            } else {
                1
            }
        }
        // shield: c(power + 10) + image, or c(power) with no image
        9 => {
            let v = c(0)?;
            if v > 3 {
                1 + gstring_len(data.get(1..)?)?
            } else {
                1
            }
        }
        // head: c(n) for n < 100, else c(len + 100) + image
        11 => {
            let v = c(0)?;
            if v >= 100 {
                1 + (v - 100)
            } else {
                1
            }
        }
        // 5 color indexes
        13 => 5,
        // attach type + npc id
        42 => 4,
        // pconnected: no payload
        51 => 0,
        _ => return None,
    };
    if len > data.len() {
        return None;
    }
    Some(len)
}

/// Walk a `(GCHAR id, value)*` player prop stream.
///
/// Stops at the first unknown id or truncated value, like the 2.05
/// client does; everything decoded before that point is returned.
pub fn parse_player_props(payload: &[u8]) -> Vec<PlayerProp> {
    parse_player_props_with(payload, false)
}

/// [`parse_player_props`] with prop 10 read as the pre-2.1 bow slot.
///
/// GServer sends that form to clients it counts as older than 2.1: in
/// the lab, a 1.39 login got it and a 2.05 login got a gani name.
pub fn parse_player_props_with(payload: &[u8], bow_prop10: bool) -> Vec<PlayerProp> {
    parse_player_props_in(payload, bow_prop10, None)
}

/// [`parse_player_props_with`] that also stops at the first id outside
/// `accepted`, the way a client whose prop switch lacks that id does.
pub fn parse_player_props_in(
    payload: &[u8],
    bow_prop10: bool,
    accepted: Option<&IdSet>,
) -> Vec<PlayerProp> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < payload.len() {
        let id = payload[i].wrapping_sub(0x20);
        if accepted.is_some_and(|s| !s.contains(id)) {
            break;
        }
        let rest = &payload[i + 1..];
        let Some(n) = prop_len(id, rest, bow_prop10) else { break };
        out.push(PlayerProp {
            id,
            raw: rest[..n].to_vec(),
        });
        i += 1 + n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bow_slot_for_old_clients() {
        // Prop 10 as bow power 0, then X = 30, SPRITE dir 2.
        let p = [0x2a, 0x20, 0x2f, 0x20 + 60, 0x31, 0x22];
        let ids: Vec<u8> = parse_player_props_with(&p, true).iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![10, 15, 17]);
        // Bow image "b.gif": c(10 + 5) + name.
        let p = [&[0x2a, 0x20 + 15][..], b"b.gif", &[0x31, 0x22]].concat();
        let props = parse_player_props_with(&p, true);
        assert_eq!(props[1].id, 17);
    }

    #[test]
    fn bow_slot_takes_a_gani_string_from_a_2x_server() {
        // GANI `sword` as a 2.x server relays it, then SPRITE dir 1.
        let p = [&[0x2a, 0x20 + 5][..], b"sword", &[0x31, 0x21]].concat();
        let props = parse_player_props_with(&p, true);
        assert_eq!(props.iter().map(|p| p.id).collect::<Vec<_>>(), vec![10, 17]);
        assert_eq!(props[0].raw, [&[0x25][..], b"sword"].concat());
        // Alone at the end of the stream.
        assert!(bow_slot_holds_gani(&[&[0x24][..], b"idle"].concat()));
        // A bow power followed by a prop: X = 30, SPRITE.
        assert!(!bow_slot_holds_gani(&[0x23, 0x2f, 0x20 + 60, 0x31, 0x22]));
        // Power 0, and a bow image in the bow form, stay the bow slot.
        assert!(!bow_slot_holds_gani(&[0x20, 0x2f, 0x20 + 60]));
        assert!(!bow_slot_holds_gani(&[&[0x20 + 15][..], b"b.gif"].concat()));
        // Upper case is no GANI name.
        assert!(!bow_slot_holds_gani(&[&[0x25][..], b"Sword"].concat()));
    }

    #[test]
    fn prop_stream_position_and_sprite() {
        // NICKNAME "ab", X = 30.5 (61), Y = 12 (24), SPRITE = dir 2 + 4*1, ALIGNMENT 53
        let mut p = vec![0x20, 0x22, b'a', b'b'];
        p.extend_from_slice(&[0x2f, 0x20 + 61, 0x30, 0x20 + 24, 0x31, 0x20 + 6, 0x40, 0x55]);
        let props = parse_player_props(&p);
        let ids: Vec<u8> = props.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![0, 15, 16, 17, 32]);
        assert_eq!(props[1].gchar(), Some(61));
        assert_eq!(props[3].gchar().map(|s| s % 4), Some(2));
    }

    /// First PLO_PLAYERPROPS sent by the lab GServer to a 2.05 login,
    /// cut before the account prop.
    #[test]
    fn lab_login_burst() {
        let hex = "2123222623202020242a25252621283f2a73776f7264312e706e67292b2b736869656c64312e706e672a2469646c652b8d68656164302e706e672d22202a24323122323435203620372039205d3a";
        let p: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let props = parse_player_props(&p);
        let ids: Vec<u8> = props.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5, 6, 8, 9, 10, 11, 13, 17, 18, 21, 22, 23, 25]);
        assert_eq!(&props[8].raw[1..], b"idle");
        assert_eq!(props[11].gchar(), Some(2));
        // APCOUNTER is a GSHORT: 61 - 1 = 60 s
        assert_eq!(props[16].raw, vec![0x20, 0x5d]);
    }

    /// The same burst read as each main version: 2.05 stops at 43, 1.39
    /// at 35, and 2.22 reads on.
    #[test]
    fn prop_sets_per_version() {
        use crate::protocol::ClientVersion;
        // X = 30, prop 34 ACCOUNTNAME "a", prop 43 GMAPLEVELX 1, Y = 12.
        let p = [0x2f, 0x20 + 60, 0x20 + 34, 0x21, b'a', 0x20 + 43, 0x21, 0x30, 0x20 + 24];
        let ids = |v: ClientVersion| -> Vec<u8> {
            let pr = v.profile().props;
            parse_player_props_in(&p, false, Some(&pr.own)).iter().map(|p| p.id).collect()
        };
        assert_eq!(ids(ClientVersion::V138), vec![15]);
        assert_eq!(ids(ClientVersion::V139), vec![15, 34]);
        assert_eq!(ids(ClientVersion::V205), vec![15, 34]);
        assert_eq!(ids(ClientVersion::V222), vec![15, 34, 43, 16]);
    }

    /// 2.05 drops another player's stream at MAXPOWER (1); 2.22 too.
    #[test]
    fn other_player_stops_at_maxpower() {
        use crate::protocol::ClientVersion;
        let p = [0x2f, 0x20 + 60, 0x21, 0x23, 0x30, 0x20 + 24];
        for v in [ClientVersion::V205, ClientVersion::V222, ClientVersion::V231] {
            let other = v.profile().props.other;
            let ids: Vec<u8> = parse_player_props_in(&p, false, Some(&other)).iter().map(|p| p.id).collect();
            assert_eq!(ids, vec![15], "{v:?}");
        }
    }

    #[test]
    fn prop_stream_stops_at_unknown() {
        // prop 77 has no known encoding: stop, keep what came before.
        let p = [0x2f, 0x20 + 10, 0x20 + 77, 0x21];
        let props = parse_player_props(&p);
        assert_eq!(props.len(), 1);
    }

    #[test]
    fn classic_roundtrip() {
        assert_eq!(encode_pos_classic(30.0), 60);
        assert_eq!(encode_pos_classic(0.5), 1);
    }

    #[test]
    fn precise_roundtrip() {
        let e = encode_pos_precise(30.0);
        assert!((decode_pos_precise(e) - 30.0).abs() < 0.1);
        let n = encode_pos_precise(-5.0);
        assert!(decode_pos_precise(n) < 0.0);
    }
}
