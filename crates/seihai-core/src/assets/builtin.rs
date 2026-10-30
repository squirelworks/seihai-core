//! The built-in default asset set.
//!
//! `assets/default/` (the original art, sounds and starter levels of
//! `documentation/default-assets.md`, 55 KB) is compiled into the binary, so
//! a fresh download opens a level and walks without any data folder.
//!
//! The rest of Seihai finds files by path (`AssetResolver`, the sound
//! engine, the readers), so the set is *materialised*: written once to a
//! folder of its own, `<settings dir>/builtin/<id>/`, and that folder is the
//! lowest layer of the resolver. The layer is read-only in every sense the
//! client controls:
//!
//! * the files are written 0444, and rewritten only from the bytes in the
//!   binary (a changed or missing file is repaired at start-up);
//! * the download store never targets it (`filecache.rs` refuses a target
//!   under it, and a download goes to the per-server cache instead);
//! * `<id>` is a CRC-32 of the whole set, so a new build with new art uses a
//!   new folder and an old build's folder is never touched.
//!
//! If the settings folder cannot be written the set goes to a folder in the
//! system temp directory instead; if that fails too, the client runs without
//! a built-in layer and says so.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::crc32::crc32_update;

/// Every file of `assets/default/`, by path relative to it, sorted. A test
/// walks the folder and fails when this list and the folder differ.
pub const FILES: &[(&str, &[u8])] = &[
    ("bodies/body.gif", include_bytes!("../../../../assets/default/bodies/body.gif")),
    ("bodies/body.png", include_bytes!("../../../../assets/default/bodies/body.png")),
    ("gani/carry.gani", include_bytes!("../../../../assets/default/gani/carry.gani")),
    ("gani/carrystill.gani", include_bytes!("../../../../assets/default/gani/carrystill.gani")),
    ("gani/dead.gani", include_bytes!("../../../../assets/default/gani/dead.gani")),
    ("gani/hurt.gani", include_bytes!("../../../../assets/default/gani/hurt.gani")),
    ("gani/idle.gani", include_bytes!("../../../../assets/default/gani/idle.gani")),
    ("gani/sit.gani", include_bytes!("../../../../assets/default/gani/sit.gani")),
    ("gani/sleep.gani", include_bytes!("../../../../assets/default/gani/sleep.gani")),
    ("gani/swim.gani", include_bytes!("../../../../assets/default/gani/swim.gani")),
    ("gani/sword.gani", include_bytes!("../../../../assets/default/gani/sword.gani")),
    ("gani/walk.gani", include_bytes!("../../../../assets/default/gani/walk.gani")),
    ("heads/head0.gif", include_bytes!("../../../../assets/default/heads/head0.gif")),
    ("heads/head0.png", include_bytes!("../../../../assets/default/heads/head0.png")),
    ("levels/new1.nw", include_bytes!("../../../../assets/default/levels/new1.nw")),
    ("levels/new2.nw", include_bytes!("../../../../assets/default/levels/new2.nw")),
    ("levels/tiledefs/tiledefsOffline.txt", include_bytes!("../../../../assets/default/levels/tiledefs/tiledefsOffline.txt")),
    ("pics1.png", include_bytes!("../../../../assets/default/pics1.png")),
    ("shields/shield1.gif", include_bytes!("../../../../assets/default/shields/shield1.gif")),
    ("shields/shield1.png", include_bytes!("../../../../assets/default/shields/shield1.png")),
    ("sounds/steps.wav", include_bytes!("../../../../assets/default/sounds/steps.wav")),
    ("sounds/steps2.wav", include_bytes!("../../../../assets/default/sounds/steps2.wav")),
    ("sounds/sword.wav", include_bytes!("../../../../assets/default/sounds/sword.wav")),
    ("sounds/water.wav", include_bytes!("../../../../assets/default/sounds/water.wav")),
    ("sprites.png", include_bytes!("../../../../assets/default/sprites.png")),
    ("state.png", include_bytes!("../../../../assets/default/state.png")),
    ("swords/sword1.gif", include_bytes!("../../../../assets/default/swords/sword1.gif")),
    ("swords/sword1.png", include_bytes!("../../../../assets/default/swords/sword1.png")),
    ("tiledefs.txt", include_bytes!("../../../../assets/default/tiledefs.txt")),
];

/// An identifier of the whole set: CRC-32 over each name and its bytes.
pub fn set_id() -> u32 {
    let mut reg = 0xFFFF_FFFFu32;
    for (name, data) in FILES {
        reg = crc32_update(reg, name.as_bytes());
        reg = crc32_update(reg, &(data.len() as u32).to_le_bytes());
        reg = crc32_update(reg, data);
    }
    !reg
}

/// The bytes of one built-in file (exact, `/`-separated relative name).
pub fn bytes(rel: &str) -> Option<&'static [u8]> {
    FILES.iter().find(|(n, _)| *n == rel).map(|(_, d)| *d)
}

/// Total size of the set in bytes.
pub fn total_size() -> usize {
    FILES.iter().map(|(_, d)| d.len()).sum()
}

/// Where the set is written under `base`: `<base>/<id>`.
pub fn dir_under(base: &Path) -> PathBuf {
    base.join(format!("{:08x}", set_id()))
}

/// Make sure `<base>/<id>/` holds exactly the bytes of the set and return
/// that folder. A file that is missing or differs is rewritten; the files are
/// left read-only. Nothing else in `<base>` is touched.
pub fn ensure_in(base: &Path) -> io::Result<PathBuf> {
    let dir = dir_under(base);
    for (rel, data) in FILES {
        let target = dir.join(rel);
        if fs::read(&target).map(|d| d == *data).unwrap_or(false) {
            set_read_only(&target);
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if target.exists() {
            // A read-only file cannot be removed on every system.
            if let Ok(meta) = fs::metadata(&target) {
                let mut perm = meta.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                perm.set_readonly(false);
                let _ = fs::set_permissions(&target, perm);
            }
            let _ = fs::remove_file(&target);
        }
        fs::write(&target, data)?;
        set_read_only(&target);
    }
    Ok(dir)
}

fn set_read_only(path: &Path) {
    if let Ok(meta) = fs::metadata(path) {
        let mut perm = meta.permissions();
        if !perm.readonly() {
            perm.set_readonly(true);
            let _ = fs::set_permissions(path, perm);
        }
    }
}

/// The folder of the built-in set for this run: under the settings folder
/// (`<settings dir>/builtin/`), else under the system temp directory.
pub fn default_dir() -> Result<PathBuf, String> {
    let primary = crate::config::config_dir().join("builtin");
    match ensure_in(&primary) {
        Ok(d) => Ok(d),
        Err(e1) => {
            let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "user".into());
            let user: String = user.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            let fallback = std::env::temp_dir().join(format!("seihai-builtin-{user}"));
            ensure_in(&fallback).map_err(|e2| {
                format!("cannot write the built-in assets to {} ({e1}) or {} ({e2})", primary.display(), fallback.display())
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("seihai-builtin-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let r = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            if e.path().is_dir() {
                walk(&e.path(), &r, out);
            } else {
                out.push(r);
            }
        }
    }

    #[test]
    fn the_embedded_list_is_exactly_the_default_folder() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets").join("default");
        let mut on_disk = Vec::new();
        walk(&root, "", &mut on_disk);
        on_disk.sort();
        let listed: Vec<&str> = FILES.iter().map(|(n, _)| *n).collect();
        assert_eq!(on_disk, listed, "assets/default and builtin::FILES differ: add or remove the entry");
        for (n, d) in FILES {
            assert_eq!(&fs::read(root.join(n)).unwrap(), d, "{n}");
        }
        // 128 KB since the sheet holds art for all 4096 tile ids of the Graal layout.
        assert!(total_size() < 128 * 1024, "the set stays small: {} bytes", total_size());
    }

    #[test]
    fn ensure_writes_the_set_read_only_and_repairs_it() {
        let base = tmp("ensure");
        let dir = ensure_in(&base).unwrap();
        assert_eq!(dir, dir_under(&base));
        for (n, d) in FILES {
            let p = dir.join(n);
            assert_eq!(&fs::read(&p).unwrap(), d, "{n}");
            assert!(fs::metadata(&p).unwrap().permissions().readonly(), "{n} is read-only");
        }
        // A tampered file and a deleted one are put back from the binary.
        let (n0, d0) = FILES[0];
        let (n1, d1) = FILES[1];
        let mut perm = fs::metadata(dir.join(n0)).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        fs::set_permissions(dir.join(n0), perm).unwrap();
        fs::write(dir.join(n0), b"changed").unwrap();
        fs::remove_file(dir.join(n1)).unwrap();
        ensure_in(&base).unwrap();
        assert_eq!(&fs::read(dir.join(n0)).unwrap(), d0);
        assert_eq!(&fs::read(dir.join(n1)).unwrap(), d1);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn the_id_depends_on_the_bytes() {
        assert_eq!(set_id(), set_id());
        assert_ne!(set_id(), 0);
        assert!(bytes("pics1.png").is_some() && bytes("nope.png").is_none());
    }
}
