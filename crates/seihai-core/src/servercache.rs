//! The per-server cache folder.
//!
//! The real clients kept what a server sent them in a cache of their own
//! (this is how copies of defunct playerworlds survived). Seihai does the
//! same: every server gets a folder under the cache root, and what the
//! server sends is saved there, never into the user's asset folder and never
//! into the built-in set. `documentation/file-cache.md` has the layout and
//! the search order.
//!
//! ```text
//! <cache root>/                       default: <settings dir>/cache
//!   play.example.org_14900~Maloria/   <host>_<port>[~<server name>]
//!     server.txt                      host, port and name, for people
//!     levels/  levels/ganis/  levels/bodies/  ...   the clients' folders
//!     sounds/  maps/
//! ```
//!
//! The folder is named by the game server's host and port, made file-safe.
//! The server's name, when it is known, is appended after a `~` so a person
//! can tell the folders apart. The name is not part of the identity: a
//! folder whose name starts with `<host>_<port>` (and then ends or has a `~`)
//! is the server's folder whatever follows, so a server that is first
//! reached without its name and later with it keeps one folder, and a
//! renamed server keeps its cache.

use std::fs;
use std::path::{Path, PathBuf};

/// The default cache root: `<settings dir>/cache`. It sits with the
/// settings, not in a system cache folder, because what it holds (old
/// copies of worlds) is not disposable.
pub fn default_cache_root() -> PathBuf {
    crate::config::config_dir().join("cache")
}

/// The Seihai folder: the folder the program file is in, or `None` when
/// that cannot be found.
pub fn seihai_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

/// The cache of the server at `addr` in the Seihai folder `dir`:
/// `<dir>/_cache/<host>_<port>`, the key only (no server name). Checked
/// before the settings' cache, and where every download is written.
pub fn game_cache_dir_for(dir: &Path, addr: &str) -> Option<PathBuf> {
    let (host, port) = split_addr(addr)?;
    Some(dir.join("_cache").join(server_key(&host, port)))
}

/// Split `host:port` (or `[v6]:port`). `None` when there is no usable port.
pub fn split_addr(addr: &str) -> Option<(String, u16)> {
    let addr = addr.trim();
    let (host, port) = addr.rsplit_once(':')?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port: u16 = port.trim().parse().ok()?;
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port))
}

/// A host made file-safe: lower case, only `a-z 0-9 . -`; every other
/// character (a colon of an IPv6 address, a slash, a space) becomes `_`; a
/// leading dot becomes `_` so the folder is never hidden or `..`.
pub fn safe_host(host: &str) -> String {
    let mut s: String = host
        .trim()
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        s.push('_');
    }
    if s.starts_with('.') {
        s.replace_range(0..1, "_");
    }
    s.chars().take(100).collect()
}

/// The part that identifies a server: `<safe host>_<port>`.
pub fn server_key(host: &str, port: u16) -> String {
    format!("{}_{}", safe_host(host), port)
}

/// A server name made file-safe, or `None` when nothing is left: ASCII
/// letters and digits kept, runs of anything else become one `-`, at most 32
/// characters.
pub fn safe_name(name: &str) -> Option<String> {
    let mut out = String::new();
    let mut gap = false;
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
        if out.len() >= 32 {
            break;
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The folder name for a new cache: the key, plus `~name` when the name is
/// known.
pub fn folder_name(host: &str, port: u16, name: Option<&str>) -> String {
    let key = server_key(host, port);
    match name.and_then(safe_name) {
        Some(n) => format!("{key}~{n}"),
        None => key,
    }
}

/// Does the folder name `dir` belong to the server with this key?
pub fn belongs_to(dir: &str, key: &str) -> bool {
    let (d, k) = (dir.to_ascii_lowercase(), key.to_ascii_lowercase());
    d == k || d.strip_prefix(&k).is_some_and(|rest| rest.starts_with('~'))
}

/// The cache folder of the server at `addr` (`host:port`) under `root`: the
/// existing folder of that server if there is one (the first by name, if
/// there are several), else the path a new one would have. Nothing is
/// created. `None` when `addr` has no host and port.
pub fn cache_dir_for(root: &Path, addr: &str, name: Option<&str>) -> Option<PathBuf> {
    let (host, port) = split_addr(addr)?;
    let key = server_key(&host, port);
    let mut found: Vec<String> = fs::read_dir(root)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| belongs_to(n, &key))
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    Some(match found.first() {
        Some(n) => root.join(n),
        None => root.join(folder_name(&host, port, name)),
    })
}

/// The text of `server.txt`.
pub fn meta_text(addr: &str, name: Option<&str>) -> String {
    let (host, port) = split_addr(addr).unwrap_or_else(|| (addr.to_string(), 0));
    let mut s = format!("host={host}\nport={port}\n");
    if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty() && !n.contains(['\n', '\r'])) {
        s.push_str(&format!("name={n}\n"));
    }
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn game_cache_is_named_by_host_and_port() {
        let d = std::path::Path::new("/s");
        assert_eq!(super::game_cache_dir_for(d, "Play.Example.ORG:14900"), Some(d.join("_cache/play.example.org_14900")));
        assert_eq!(super::game_cache_dir_for(d, "[::1]:14802"), Some(d.join("_cache/__1_14802")));
        assert_eq!(super::game_cache_dir_for(d, "nohost"), None);
    }

    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("seihai-servercache-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn addresses_split_into_host_and_port() {
        assert_eq!(split_addr("play.example.org:14900"), Some(("play.example.org".into(), 14900)));
        assert_eq!(split_addr("[::1]:14900"), Some(("::1".into(), 14900)));
        assert_eq!(split_addr(" 10.0.0.2:1 "), Some(("10.0.0.2".into(), 1)));
        for bad in ["", "host", "host:", "host:99999", ":14900", "host:abc"] {
            assert_eq!(split_addr(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn keys_are_file_safe() {
        assert_eq!(server_key("Play.Example.ORG", 14900), "play.example.org_14900");
        assert_eq!(server_key("::1", 14900), "__1_14900");
        assert_eq!(server_key("a/b c", 1), "a_b_c_1");
        assert_eq!(server_key("..", 1), "_._1");
        assert_eq!(server_key(".hidden", 1), "_hidden_1");
        assert!(!server_key("../../etc", 1).contains('/'));
        assert_eq!(server_key("", 1), "__1");
    }

    #[test]
    fn names_are_appended_when_known() {
        assert_eq!(folder_name("h.org", 14900, None), "h.org_14900");
        assert_eq!(folder_name("h.org", 14900, Some("Maloria")), "h.org_14900~Maloria");
        assert_eq!(folder_name("h.org", 14900, Some("  P  Elven Lands: 2.0 / test ")), "h.org_14900~P-Elven-Lands-2-0-test");
        assert_eq!(folder_name("h.org", 14900, Some("???")), "h.org_14900");
        assert_eq!(folder_name("h.org", 14900, Some("")), "h.org_14900");
        let long = "x".repeat(80);
        assert!(safe_name(&long).unwrap().len() <= 32);
    }

    #[test]
    fn a_folder_belongs_to_its_key_not_to_a_longer_one() {
        assert!(belongs_to("h.org_14900", "h.org_14900"));
        assert!(belongs_to("h.org_14900~Maloria", "h.org_14900"));
        assert!(belongs_to("H.ORG_14900~x", "h.org_14900"));
        assert!(!belongs_to("h.org_149001", "h.org_14900"));
        assert!(!belongs_to("h.org_1490", "h.org_14900"));
        assert!(!belongs_to("xh.org_14900", "h.org_14900"));
    }

    #[test]
    fn the_folder_of_a_server_is_found_again_whatever_its_name_is_now() {
        let root = tmp("find");
        // Nothing yet: the path a new folder would have; nothing created.
        let p = cache_dir_for(&root, "h.org:14900", Some("Maloria")).unwrap();
        assert_eq!(p, root.join("h.org_14900~Maloria"));
        assert!(!p.exists());
        fs::create_dir_all(&p).unwrap();
        // Same server later, name unknown or changed: same folder.
        assert_eq!(cache_dir_for(&root, "h.org:14900", None).unwrap(), p);
        assert_eq!(cache_dir_for(&root, "H.org:14900", Some("Renamed")).unwrap(), p);
        // Another port or host: another folder.
        assert_eq!(cache_dir_for(&root, "h.org:14901", None).unwrap(), root.join("h.org_14901"));
        assert_eq!(cache_dir_for(&root, "other.org:14900", None).unwrap(), root.join("other.org_14900"));
        assert_eq!(cache_dir_for(&root, "no-port", None), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn server_txt_holds_host_port_and_name() {
        assert_eq!(meta_text("h.org:14900", Some("Maloria")), "host=h.org\nport=14900\nname=Maloria\n");
        assert_eq!(meta_text("h.org:14900", None), "host=h.org\nport=14900\n");
        assert_eq!(meta_text("h.org:14900", Some("a\nb")), "host=h.org\nport=14900\n");
    }
}
