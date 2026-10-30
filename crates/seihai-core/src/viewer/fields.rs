//! Reading a payload with a layout from the tables.
//!
//! This is the viewer's own reader, driven by `packet-layouts.tsv`. It runs
//! only where the trace decoder has no named fields for a message, so a
//! table (including one given with `--tables`) can add layouts without a
//! code change. Whatever the layout does not cover stays visible: bytes left
//! over after the last field become `?tail`, and a payload that ends inside
//! a field is reported, not padded.

use super::tables::{Enc, LayoutRow};
use crate::assets::crc32;
use crate::protocol::{latin1, read_gint3, read_gshort};

/// What reading a payload with a layout gave.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Read {
    pub fields: Vec<(String, String)>,
    /// Things the layout could not settle: unknown fields, a short payload,
    /// leftover bytes.
    pub unknown: Vec<String>,
}

/// Hex of at most 32 bytes, then `...(+N)`.
pub fn short_hex(b: &[u8]) -> String {
    let mut s: String = b.iter().take(32).map(|x| format!("{x:02x}")).collect();
    if b.len() > 32 {
        s.push_str(&format!("...(+{})", b.len() - 32));
    }
    s
}

fn gchar(b: u8) -> u8 {
    b.wrapping_sub(0x20)
}

fn gint(b: &[u8]) -> u64 {
    b.iter().fold(0u64, |a, &x| (a << 7) | u64::from(gchar(x) & 0x7f))
}

fn tiles(v: u8) -> String {
    let h = f32::from(v) / 2.0;
    if (h - h.round()).abs() < 1e-4 { format!("{}", h.round() as i64) } else { format!("{h}") }
}

/// Text with the account name masked (when masking is on and the account
/// is known).
fn text(b: &[u8], mask: bool, account: Option<&str>) -> String {
    let s = latin1(b);
    match account {
        Some(a) if mask && !a.is_empty() => s.replace(a, "<ACCT>"),
        _ => s,
    }
}

/// Read `payload` with `rows` (already in field order). Returns `None` when
/// a row needs the trace decoder (`props`, `showimg`).
pub fn read_layout(rows: &[&LayoutRow], payload: &[u8], mask: bool, account: Option<&str>) -> Option<Read> {
    if rows.iter().any(|r| matches!(r.enc, Enc::Props | Enc::Showimg)) {
        return None;
    }
    let mut out = Read::default();
    let mut pos = 0usize;
    let mut rest_taken = false;
    for r in rows {
        let left = payload.get(pos..).unwrap_or(&[]);
        let short = |out: &mut Read, need: usize| {
            out.unknown.push(format!("payload ends inside `{}` (needs {need}, {} left)", r.field, left.len()));
        };
        let masked = |v: String| if mask { "<MASKED>".to_string() } else { v };
        let value = match r.enc {
            Enc::None => continue,
            Enc::Gchar => match left.first() {
                Some(&b) => {
                    pos += 1;
                    gchar(b).to_string()
                }
                None => {
                    short(&mut out, 1);
                    break;
                }
            },
            Enc::Halftile => match left.first() {
                Some(&b) => {
                    pos += 1;
                    tiles(gchar(b))
                }
                None => {
                    short(&mut out, 1);
                    break;
                }
            },
            Enc::Gshort => match read_gshort(left) {
                Some(v) => {
                    pos += 2;
                    if r.field == "player" { masked(v.to_string()) } else { v.to_string() }
                }
                None => {
                    short(&mut out, 2);
                    break;
                }
            },
            Enc::Gint3 => match read_gint3(left) {
                Some(v) => {
                    pos += 3;
                    v.to_string()
                }
                None => {
                    short(&mut out, 3);
                    break;
                }
            },
            Enc::Gint4 | Enc::Gint5 => {
                let n = if r.enc == Enc::Gint4 { 4 } else { 5 };
                match left.get(..n) {
                    Some(b) => {
                        pos += n;
                        let v = gint(b).to_string();
                        if r.field == "modtime" { masked(v) } else { v }
                    }
                    None => {
                        short(&mut out, n);
                        break;
                    }
                }
            }
            Enc::Gstring => match left.split_first() {
                Some((&n, tail)) if tail.len() >= usize::from(gchar(n)) => {
                    let n = usize::from(gchar(n));
                    pos += 1 + n;
                    text(&tail[..n], mask, account)
                }
                Some((&n, tail)) => {
                    short(&mut out, 1 + usize::from(gchar(n)));
                    let _ = tail;
                    break;
                }
                None => {
                    short(&mut out, 1);
                    break;
                }
            },
            Enc::GstringList => {
                let mut items = Vec::new();
                let mut rest = left;
                while let Some((&n, tail)) = rest.split_first() {
                    let n = usize::from(gchar(n));
                    let Some(item) = tail.get(..n) else {
                        out.unknown.push(format!("`{}` ends inside an item", r.field));
                        break;
                    };
                    items.push(text(item, mask, account));
                    rest = &tail[n..];
                }
                pos = payload.len() - rest.len();
                rest_taken = true;
                items.join(" | ")
            }
            Enc::Text | Enc::Comma => {
                pos = payload.len();
                rest_taken = true;
                text(left, mask, account)
            }
            Enc::Raw => {
                pos = payload.len();
                rest_taken = true;
                if left.len() > 32 { format!("len={} crc={:08x}", left.len(), crc32(left)) } else { short_hex(left) }
            }
            Enc::Props | Enc::Showimg => unreachable!(),
        };
        if r.status != super::tables::Status::Known {
            out.unknown.push(format!("`{}` is {} in the table", r.field, r.status.label()));
        }
        let name = if r.status == super::tables::Status::Unknown { format!("?{}", r.field) } else { r.field.clone() };
        out.fields.push((name, value));
    }
    if pos < payload.len() && !rest_taken && out.unknown.iter().all(|u| !u.starts_with("payload ends")) {
        let tail = &payload[pos..];
        out.fields.push(("?tail".into(), short_hex(tail)));
        out.unknown.push(format!("{} byte(s) after the last field in the layout", tail.len()));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::difftest::trace::Dir;
    use crate::viewer::tables::{parse_layouts, LayoutRow};

    fn rows(tsv: &str) -> Vec<LayoutRow> {
        parse_layouts(&format!("dir\tid\tname\tidx\tfield\tencoding\tfrom\tto\tstatus\tnote\n{tsv}")).unwrap()
    }

    fn r(rows: &[LayoutRow]) -> Vec<&LayoutRow> {
        rows.iter().collect()
    }

    #[test]
    fn reads_numbers_text_and_marks_a_tail() {
        let t = rows("S>C\t99\tX\t1\ta\tgchar\t-\t-\tknown\t\nS>C\t99\tX\t2\tb\tgshort\t-\t-\tknown\t\nS>C\t99\tX\t3\tc\thalftile\t-\t-\tknown\t\n");
        // a=5, b=130 (1<<7 | 2), c=61 half tiles = 30.5, then two stray bytes.
        let p = [0x25, 0x21, 0x22, 0x20 + 61, 0x41, 0x42];
        let o = read_layout(&r(&t), &p, true, None).unwrap();
        assert_eq!(o.fields[0], ("a".into(), "5".into()));
        assert_eq!(o.fields[1], ("b".into(), "130".into()));
        assert_eq!(o.fields[2], ("c".into(), "30.5".into()));
        assert_eq!(o.fields[3], ("?tail".into(), "4142".into()));
        assert_eq!(o.unknown.len(), 1);
    }

    #[test]
    fn unknown_status_field_is_marked() {
        let t = rows("C>S\t24\tX\t1\tvalue\tgchar\t-\t-\tunknown\tnote\n");
        let o = read_layout(&r(&t), &[0x29], true, None).unwrap();
        assert_eq!(o.fields, vec![("?value".to_string(), "9".to_string())]);
        assert!(o.unknown[0].contains("unknown"));
    }

    #[test]
    fn short_payload_is_reported_not_padded() {
        let t = rows("S>C\t99\tX\t1\ta\tgint3\t-\t-\tknown\t\n");
        let o = read_layout(&r(&t), &[0x21, 0x22], true, None).unwrap();
        assert!(o.fields.is_empty());
        assert!(o.unknown[0].contains("ends inside `a`"));
    }

    #[test]
    fn text_masks_the_account_and_player_ids() {
        let t = rows("S>C\t13\tTOALL\t1\tplayer\tgshort\t-\t-\tknown\t\nS>C\t13\tTOALL\t2\ttext\ttext\t-\t-\tknown\t\n");
        let mut p = vec![0x20, 0x20 + 77];
        p.extend_from_slice(b"hi dtacct");
        let o = read_layout(&r(&t), &p, true, Some("dtacct")).unwrap();
        assert_eq!(o.fields[0].1, "<MASKED>");
        assert_eq!(o.fields[1].1, "hi <ACCT>");
        let o = read_layout(&r(&t), &p, false, Some("dtacct")).unwrap();
        assert_eq!(o.fields[0].1, "77");
        assert_eq!(o.fields[1].1, "hi dtacct");
    }

    #[test]
    fn gstring_list_and_big_raw() {
        let t = rows("S>C\t75\tP\t1\tf\tgstring_list\t-\t-\tknown\t\nS>C\t76\tQ\t1\tdata\traw\t-\t-\tknown\t\n");
        let p = [0x23, b'a', b'b', b'c', 0x22, b'x', b'y'];
        let o = read_layout(&[&t[0]], &p, true, None).unwrap();
        assert_eq!(o.fields[0].1, "abc | xy");
        let o = read_layout(&[&t[1]], &[7u8; 40], true, None).unwrap();
        assert!(o.fields[0].1.starts_with("len=40 crc="));
        let _ = Dir::C2S;
    }

    #[test]
    fn props_layouts_are_left_to_the_decoder() {
        let t = rows("C>S\t2\tP\t1\tprops\tprops\t-\t-\tknown\t\n");
        assert!(read_layout(&r(&t), &[0x20], true, None).is_none());
    }
}
