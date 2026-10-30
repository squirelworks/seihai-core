//! Player flags a client script changes, sent to the server.
//!
//! A real 2.x client sends `PLI_FLAGSET` (18) with `name` or `name=value`
//! when a script sets a player flag, and `PLI_FLAGDEL` (19) with the name
//! when one clears it; `local.` flags are never sent (Graal_analysis
//! `protocol/packet-layouts.md`). The `gr.x` and `gr.y` flags seen in the
//! lab captures are this: GServer-v2's `-gr_movement` weapon sets them every
//! 50 ms from `playerx`/`playery`, and a real 2.22 on an account without that
//! weapon sends no flag at all (lab, 2026-10-30; `documentation/gr-flags.md`).

/// `PLI_FLAGSET`.
pub const PLI_FLAGSET: u8 = 18;
/// `PLI_FLAGDEL`.
pub const PLI_FLAGDEL: u8 = 19;

/// The message for one flag: `PLI_FLAGSET` with `name=value` (`name` alone
/// for an empty value), or `PLI_FLAGDEL` with the name when `value` is `None`.
pub fn flag_message(name: &str, value: Option<&str>) -> Vec<u8> {
    let id = if value.is_some() { PLI_FLAGSET } else { PLI_FLAGDEL };
    let mut m = vec![id + 0x20];
    m.extend(name.bytes());
    if let Some(v) = value.filter(|v| !v.is_empty()) {
        m.push(b'=');
        m.extend(v.bytes());
    }
    m.push(b'\n');
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_is_flagset_with_name_equals_value() {
        // The bytes of the real 2.22 client's `gr.x=30`.
        assert_eq!(flag_message("gr.x", Some("30")), [18 + 0x20, 0x67, 0x72, 0x2e, 0x78, 0x3d, 0x33, 0x30, b'\n']);
        assert_eq!(flag_message("door", Some("")), b"\x32door\n");
        assert_eq!(flag_message("door", None), b"\x33door\n");
    }
}
