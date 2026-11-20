//! The 5.x and 6.x script cache: `GRESCR01` files, `PLO_LOADSCRIPT` (197)
//! and `PLI_UPDATESCRIPT` (158).
//!
//! From Graal_analysis `formats/gwebl001-cache.md`,
//! `formats/grescr-key-words-2026-11-16.md`,
//! `formats/windows-603-weapon-cache.md` and
//! `protocol/gs2-official-bytecode-6113.md`:
//!
//! - `PLO_NPCWEAPONSCRIPT` (140) sends a weapon's or class's compiled
//!   script with a header `type,name,cache flag,key`. The key is two GINT5
//!   words, which as little-endian 32-bit words are an 8-byte DES key. With
//!   the cache flag 1 the client keeps `GRESCR01` and the body, DES-ECB
//!   encrypted over whole 8-byte blocks (each key byte bit-reversed, as
//!   [`crate::graaldes`]), the last `len % 8` bytes in clear.
//! - `PLO_LOADSCRIPT` (197) is a descriptor, `type,name,1,key,crc`: use the
//!   cached copy if the CRC-32 of its stored (still encrypted) body is
//!   `crc`. A CRC of 0 never matches. On a miss the client asks with
//!   `PLI_UPDATESCRIPT` (158), the name as the rest of the message, and the
//!   server sends 140.
//!
//! The header is a CSV line as GServer writes it: a field with a comma, a
//! quote, a backslash or a byte outside 32 to 126 is quoted, its quotes and
//! backslashes doubled.
//!
//! [`ScriptCache`] keeps the stored files by kind and name, in memory and,
//! when given a folder, on disk as the 6.03r7 client lays them out
//! (`weapons/<name>.code`, `classes/<name>.code`).

use std::collections::HashMap;
use std::path::PathBuf;

use crate::assets::crc32::crc32;
use crate::graaldes::GraalDes;

/// The cache file's magic.
pub const MAGIC: &[u8; 8] = b"GRESCR01";

/// `PLI_UPDATESCRIPT`.
pub const PLI_UPDATESCRIPT: u8 = 158;

/// The 8-byte DES key from the header's two key words.
pub fn key_of(words: [u32; 2]) -> [u8; 8] {
    let mut k = [0u8; 8];
    k[..4].copy_from_slice(&words[0].to_le_bytes());
    k[4..].copy_from_slice(&words[1].to_le_bytes());
    k
}

/// The cache file for `body`: the magic, then the body encrypted over its
/// whole blocks.
pub fn seal(body: &[u8], key: &[u8; 8]) -> Vec<u8> {
    let mut stored = body.to_vec();
    GraalDes::new(key).encrypt(&mut stored);
    let mut out = MAGIC.to_vec();
    out.extend(stored);
    out
}

/// The body of cache file `file`, when it has the magic and its stored
/// body's CRC-32 is `crc` (not 0).
pub fn open(file: &[u8], key: &[u8; 8], crc: u32) -> Option<Vec<u8>> {
    let stored = file.strip_prefix(MAGIC.as_slice())?;
    if crc == 0 || crc32(stored) != crc {
        return None;
    }
    let mut body = stored.to_vec();
    GraalDes::new(key).decrypt(&mut body);
    Some(body)
}

/// The CRC a 197 for `body` under `key` carries.
pub fn stored_crc(body: &[u8], key: &[u8; 8]) -> u32 {
    crc32(&seal(body, key)[MAGIC.len()..])
}

/// Split a GServer CSV line into its fields.
pub fn csv_fields(line: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        let mut field = Vec::new();
        if line.get(i) == Some(&b'"') {
            i += 1;
            while i < line.len() {
                let c = line[i];
                if (c == b'"' || c == b'\\') && line.get(i + 1) == Some(&c) {
                    field.push(c);
                    i += 2;
                } else if c == b'"' {
                    i += 1;
                    break;
                } else {
                    field.push(c);
                    i += 1;
                }
            }
            // Anything up to the next comma after a closing quote is dropped.
            while i < line.len() && line[i] != b',' {
                i += 1;
            }
        } else {
            while i < line.len() && line[i] != b',' {
                field.push(line[i]);
                i += 1;
            }
        }
        out.push(field);
        if i >= line.len() {
            break;
        }
        i += 1; // the comma
    }
    out
}

/// A GINT5 (five GCHARs, high 7 bits first) as a 32-bit word.
fn gint5_word(b: &[u8]) -> Option<u32> {
    if b.len() != 5 {
        return None;
    }
    let v = b.iter().fold(0u64, |acc, &c| {
        (acc << 7) | (c.wrapping_sub(0x20) as u64 & 0x7F)
    });
    Some(v as u32)
}

/// The cache fields of a 140 header or a 197 descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descriptor {
    /// `weapon` or `class`, lower case.
    pub kind: String,
    pub name: String,
    /// The cache flag is 1.
    pub cache: bool,
    pub key: Option<[u8; 8]>,
    /// The stored body's CRC-32 (197 only).
    pub crc: Option<u32>,
}

impl Descriptor {
    /// Read `type,name,flag,key[,crc]`. `None` without a type or a name.
    pub fn parse(line: &[u8]) -> Option<Descriptor> {
        let f = csv_fields(line);
        let text = |b: &[u8]| b.iter().map(|&c| c as char).collect::<String>();
        let kind = text(f.first()?).trim().to_ascii_lowercase();
        let name = text(f.get(1)?);
        if kind.is_empty() || name.is_empty() {
            return None;
        }
        let cache = f.get(2).is_some_and(|c| c.as_slice() == b"1");
        let key = f.get(3).and_then(|k| {
            (k.len() == 10)
                .then(|| Some(key_of([gint5_word(&k[..5])?, gint5_word(&k[5..])?])))
                .flatten()
        });
        let crc = f.get(4).and_then(|c| gint5_word(c));
        Some(Descriptor {
            kind,
            name,
            cache,
            key,
            crc,
        })
    }
}

/// `PLI_UPDATESCRIPT` (158) for `name`: ask the server for the script.
pub fn update_script_message(name: &str) -> Vec<u8> {
    let mut m = vec![PLI_UPDATESCRIPT + 0x20];
    m.extend(name.chars().map(|c| c as u32 as u8));
    m
}

/// What a 197 came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Load {
    /// The cached copy matched: its body.
    Hit(Vec<u8>),
    /// No copy, or not this one: ask with 158.
    Miss,
}

/// The scripts a client keeps, by kind and lower-case name.
#[derive(Debug, Default, Clone)]
pub struct ScriptCache {
    files: HashMap<(String, String), Vec<u8>>,
    /// Where the files are also kept on disk (the server's cache folder).
    dir: Option<PathBuf>,
}

impl ScriptCache {
    /// Keep the files under `dir` too (`weapons/<name>.code`,
    /// `classes/<name>.code`), and read from there what memory lacks.
    pub fn set_dir(&mut self, dir: Option<PathBuf>) {
        self.dir = dir;
    }

    fn path(&self, kind: &str, name: &str) -> Option<PathBuf> {
        let folder = match kind {
            "weapon" => "weapons",
            "class" => "classes",
            _ => return None,
        };
        // One file name, no folders: the name is the server's.
        let safe: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "-_. ".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if safe.trim_matches('.').is_empty() {
            return None;
        }
        Some(self.dir.as_ref()?.join(folder).join(format!("{safe}.code")))
    }

    /// A script came with 140: keep it if the server said to.
    pub fn delivered(&mut self, d: &Descriptor, body: &[u8]) {
        let (true, Some(key)) = (d.cache, d.key) else {
            return;
        };
        let file = seal(body, &key);
        if let Some(p) = self.path(&d.kind, &d.name) {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&p, &file);
        }
        self.files
            .insert((d.kind.clone(), d.name.to_ascii_lowercase()), file);
    }

    /// The server sent 197: the cached body, or a miss.
    pub fn load(&mut self, d: &Descriptor) -> Load {
        let (Some(key), Some(crc)) = (d.key, d.crc) else {
            return Load::Miss;
        };
        let k = (d.kind.clone(), d.name.to_ascii_lowercase());
        if !self.files.contains_key(&k) {
            if let Some(file) = self
                .path(&d.kind, &d.name)
                .and_then(|p| std::fs::read(p).ok())
            {
                self.files.insert(k.clone(), file);
            }
        }
        match self.files.get(&k).and_then(|f| open(f, &key, crc)) {
            Some(body) => Load::Hit(body),
            None => Load::Miss,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gint5(v: u32) -> Vec<u8> {
        (0..5)
            .rev()
            .map(|i| ((v as u64 >> (7 * i)) & 0x7F) as u8 + 0x20)
            .collect()
    }

    /// The header as GServer writes it (`string::toCSV`).
    fn to_csv(fields: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for f in fields {
            let complex = f
                .iter()
                .any(|&c| !(32..=126).contains(&c) || b"\",\\".contains(&c));
            if complex {
                out.push(b'"');
                for &c in f {
                    out.push(c);
                    if c == b'"' || c == b'\\' {
                        out.push(c);
                    }
                }
                out.push(b'"');
            } else {
                out.extend(f);
            }
            out.push(b',');
        }
        out.pop();
        out
    }

    fn header(kind: &str, name: &str, words: [u32; 2], crc: Option<u32>) -> Vec<u8> {
        let mut key = gint5(words[0]);
        key.extend(gint5(words[1]));
        let mut f = vec![
            kind.as_bytes().to_vec(),
            name.as_bytes().to_vec(),
            b"1".to_vec(),
            key,
        ];
        if let Some(c) = crc {
            f.push(gint5(c));
        }
        to_csv(&f)
    }

    #[test]
    fn a_sealed_body_opens_with_its_key_and_stored_crc() {
        let key = key_of([0x0102_0304, 0xA0B0_C0D0]);
        assert_eq!(key, [4, 3, 2, 1, 0xD0, 0xC0, 0xB0, 0xA0]);
        let body: Vec<u8> = (0..21u8).collect();
        let file = seal(&body, &key);
        assert_eq!(&file[..8], MAGIC);
        // Whole blocks are encrypted; the last 21 % 8 = 5 bytes stay clear.
        assert_ne!(&file[8..24], &body[..16]);
        assert_eq!(&file[24..], &body[16..]);
        let crc = crc32(&file[8..]);
        assert_eq!(stored_crc(&body, &key), crc);
        assert_eq!(open(&file, &key, crc), Some(body.clone()));
        // The CRC is of the stored bytes, not the plain body; 0 never matches.
        assert_eq!(open(&file, &key, crc32(&body)), None);
        assert_eq!(open(&file, &key, 0), None);
        // The magic alone is no script.
        assert_eq!(open(MAGIC, &key, crc32(&[])), None);
        assert_eq!(open(b"GRESCR02", &key, crc), None);
    }

    #[test]
    fn csv_fields_read_quoted_and_doubled_bytes() {
        assert_eq!(
            csv_fields(b"weapon,-Bomb,1,\"a,b\"\"c\\\\d\",x"),
            [
                b"weapon".to_vec(),
                b"-Bomb".to_vec(),
                b"1".to_vec(),
                b"a,b\"c\\d".to_vec(),
                b"x".to_vec()
            ]
        );
        assert_eq!(csv_fields(b""), [Vec::<u8>::new()]);
        assert_eq!(csv_fields(b"a,"), [b"a".to_vec(), Vec::new()]);
    }

    #[test]
    fn descriptors_read_their_key_and_crc() {
        // Words whose GINT5 bytes include a comma and a quote.
        let words = [0x0C_00_0B_8C, 0xFFFF_FFFF];
        let h = header("weapon", "Bomb,2", words, Some(0x1234_5678));
        let d = Descriptor::parse(&h).unwrap();
        assert_eq!(d.kind, "weapon");
        assert_eq!(d.name, "Bomb,2");
        assert!(d.cache);
        assert_eq!(d.key, Some(key_of(words)));
        assert_eq!(d.crc, Some(0x1234_5678));
        let d = Descriptor::parse(&header("Class", "shop", [1, 2], None)).unwrap();
        assert_eq!((d.kind.as_str(), d.crc), ("class", None));
        assert_eq!(Descriptor::parse(b"weapon"), None);
        assert_eq!(Descriptor::parse(b",x"), None);
    }

    #[test]
    fn the_cache_hits_after_a_delivery_and_misses_otherwise() {
        let words = [77, 99];
        let key = key_of(words);
        let body = b"compiled gs2 bytes".to_vec();
        let crc = stored_crc(&body, &key);
        let mut c = ScriptCache::default();
        let load = Descriptor::parse(&header("weapon", "Bomb", words, Some(crc))).unwrap();
        assert_eq!(c.load(&load), Load::Miss);
        let sent = Descriptor::parse(&header("weapon", "Bomb", words, None)).unwrap();
        c.delivered(&sent, &body);
        assert_eq!(c.load(&load), Load::Hit(body.clone()));
        // Another CRC (the script changed), or a class of the same name: miss.
        let changed = Descriptor::parse(&header("weapon", "bomb", words, Some(crc ^ 1))).unwrap();
        assert_eq!(c.load(&changed), Load::Miss);
        let class = Descriptor::parse(&header("class", "Bomb", words, Some(crc))).unwrap();
        assert_eq!(c.load(&class), Load::Miss);
        // A delivery with the cache flag 0 is not kept.
        let mut c = ScriptCache::default();
        let mut no = sent.clone();
        no.cache = false;
        c.delivered(&no, &body);
        assert_eq!(c.load(&load), Load::Miss);
    }

    #[test]
    fn the_cache_folder_keeps_files_across_sessions() {
        let dir = std::env::temp_dir().join(format!("seihai-scriptcache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let words = [5, 6];
        let body = b"class body".to_vec();
        let crc = stored_crc(&body, &key_of(words));
        let mut c = ScriptCache::default();
        c.set_dir(Some(dir.clone()));
        c.delivered(
            &Descriptor::parse(&header("class", "shop/../x", words, None)).unwrap(),
            &body,
        );
        assert!(dir.join("classes").join("shop_.._x.code").is_file());
        let mut fresh = ScriptCache::default();
        fresh.set_dir(Some(dir.clone()));
        let d = Descriptor::parse(&header("class", "shop/../x", words, Some(crc))).unwrap();
        assert_eq!(fresh.load(&d), Load::Hit(body));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_script_asks_by_name() {
        assert_eq!(update_script_message("-Bomb"), b"\xbe-Bomb".to_vec());
    }
}
