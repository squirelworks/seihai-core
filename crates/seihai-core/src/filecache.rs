//! File-transfer caching rules.
//!
//! What the real clients keep, what they ask the server for, and where a
//! download is saved. Read from the 2.05 rev 2, 2.17.1, 2.22 and 2.31
//! clients and checked against the lab captures of 2.17, 2.22 and 2.31
//! (2026-10-24). `documentation/file-cache.md` has the table of rules and
//! what is still open.
//!
//! The pieces:
//!
//! * [`FileRequester`]: the three name lists a client keeps. It decides
//!   between `PLI_WANTFILE` (23, no local copy), `PLI_UPDATEFILE` (34, a
//!   local copy whose mod time the server may know better) and nothing, and
//!   it keeps at most four requests of each kind on the wire.
//! * [`download_dir`]: the folder a downloaded file goes to.
//! * [`DownloadStore`]: saves a `PLO_FILE` the way the clients do (in place
//!   when the client already has the file, else in the folder for its
//!   kind), sets the file's time to the server's mod time, and keeps the
//!   bytes in memory when saving to disk is off.
//! * [`FileSession`]: the three together, for the client binary.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use crate::assets::resolve::{clean_name, lookup};
use crate::assets::{AssetResolver, AssetRoot, Layer};
use crate::net::{updatefile_message, wantfile_message};
use crate::protocol::{ClientVersion, SessionState, VersionProfile};

/// Requests of one kind in flight at once. The rest wait in the list, in
/// order, and go out one by one as replies arrive. Read in 2.00, 2.05 rev
/// 2, 2.17.1, 2.22 and 2.31 (every client has the same list code), and seen
/// in the lab: 2.17, 2.22 and 2.31 send four `WANTFILE` and, in a separate
/// count, four `UPDATEFILE` before the first reply.
pub const WINDOW: usize = 4;

/// One request for the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileRequest {
    /// `PLI_WANTFILE` (23).
    Want(String),
    /// `PLI_UPDATEFILE` (34): the local copy's mod time and the name.
    Update { mod_time: u32, name: String },
}

impl FileRequest {
    /// The message as it goes into a frame.
    pub fn message(&self) -> Vec<u8> {
        match self {
            FileRequest::Want(n) => wantfile_message(n),
            FileRequest::Update { mod_time, name } => updatefile_message(*mod_time, name),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            FileRequest::Want(n) | FileRequest::Update { name: n, .. } => n,
        }
    }
}

/// The lists a client keeps about files it asked for.
///
/// The real client has three lists of names: *wanted* (A, requests made
/// with 23), *update asked* (B, with 34) and *settled* (C, every name a
/// reply came back for). Rules, all read from the list code:
///
/// * `need` with no local copy: if the name is not in A, append it, and
///   send `WANTFILE` when A now holds at most [`WINDOW`] names.
/// * `need` with a local copy: if the name is in neither B nor C, append it
///   to B and send `UPDATEFILE` when B holds at most [`WINDOW`] names. A
///   name in C (anything the server already answered this session) is never
///   asked about again, so each local file is checked once per connection.
/// * a reply (`FILE`, `FILESENDFAILED`, `FILEUPTODATE`, or the end of a
///   large file) removes the name from A and from B. If it stood among the
///   first [`WINDOW`] and names are still waiting, the one now in position
///   four goes out. Then the name is added to C.
/// * A name the server could not send is not asked for again on the same
///   connection (the lab clients ask for `steps.wav` once, however often
///   they walk).
///
/// A build that never sends `PLI_UPDATEFILE` (the profile's `pli` set has
/// no 34: 1.33 to 1.41) never revalidates a file it has.
#[derive(Debug, Clone)]
pub struct FileRequester {
    sends_update: bool,
    /// Names as they were asked for (the case the level or the server gave
    /// them), compared by [`key`]. A waiting name goes out in that case.
    wanted: Vec<String>,
    update_asked: Vec<(String, u32)>,
    settled: HashSet<String>,
    failed: HashSet<String>,
}

fn key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

impl FileRequester {
    pub fn new(profile: &VersionProfile) -> Self {
        Self {
            sends_update: profile.protocol.pli.contains(34),
            wanted: Vec::new(),
            update_asked: Vec::new(),
            settled: HashSet::new(),
            failed: HashSet::new(),
        }
    }

    /// The client needs `name`. `local` is the mod time of the copy it has,
    /// if any. Returns what to send now.
    pub fn need(&mut self, name: &str, local: Option<u32>) -> Vec<FileRequest> {
        let k = key(name);
        if k.is_empty() {
            return Vec::new();
        }
        match local {
            None => {
                if self.failed.contains(&k) || self.wanted.iter().any(|n| key(n) == k) {
                    return Vec::new();
                }
                self.wanted.push(name.trim().to_string());
                if self.wanted.len() <= WINDOW {
                    vec![FileRequest::Want(name.trim().to_string())]
                } else {
                    Vec::new()
                }
            }
            Some(mod_time) => {
                if !self.sends_update
                    || self.settled.contains(&k)
                    || self.update_asked.iter().any(|(n, _)| key(n) == k)
                {
                    return Vec::new();
                }
                self.update_asked.push((name.trim().to_string(), mod_time));
                if self.update_asked.len() <= WINDOW {
                    vec![FileRequest::Update { mod_time, name: name.trim().to_string() }]
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// The server answered for `name` (it sent the file, said it is up to
    /// date, or could not send it). Returns the next waiting requests.
    pub fn done(&mut self, name: &str) -> Vec<FileRequest> {
        let k = key(name);
        let mut next = Vec::new();
        if let Some(i) = self.wanted.iter().position(|n| key(n) == k) {
            self.wanted.remove(i);
            if i < WINDOW && self.wanted.len() >= WINDOW {
                next.push(FileRequest::Want(self.wanted[WINDOW - 1].clone()));
            }
        }
        if let Some(i) = self.update_asked.iter().position(|(n, _)| key(n) == k) {
            self.update_asked.remove(i);
            if i < WINDOW && self.update_asked.len() >= WINDOW {
                let (n, t) = self.update_asked[WINDOW - 1].clone();
                next.push(FileRequest::Update { mod_time: t, name: n });
            }
        }
        self.settled.insert(k);
        next
    }

    /// `PLO_FILESENDFAILED` (30): as [`done`](Self::done), and the name is
    /// remembered so it is not asked for again.
    pub fn failed(&mut self, name: &str) -> Vec<FileRequest> {
        let next = self.done(name);
        self.failed.insert(key(name));
        next
    }

    /// Whether the server said it could not send `name` (`FILESENDFAILED`)
    /// on this connection.
    pub fn has_failed(&self, name: &str) -> bool {
        self.failed.contains(&key(name))
    }

    /// Names waiting for a reply to `WANTFILE`, in order. The first
    /// [`WINDOW`] are on the wire.
    pub fn wanted(&self) -> &[String] {
        &self.wanted
    }

    /// Names waiting for a reply to `UPDATEFILE`, in order.
    pub fn update_asked(&self) -> Vec<&str> {
        self.update_asked.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// Has the server answered for this name on this connection?
    pub fn is_settled(&self, name: &str) -> bool {
        self.settled.contains(&key(name))
    }
}

fn is_level_name(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "nw" | "graal" | "zelda" | "gmap"))
}

/// How a build's save routine lays out downloads, read in the client
/// binaries (every build named below was disassembled; none was run).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveLayout {
    /// 1.33 to 1.41: one folder, `levels/webgifs`, for everything saved. The
    /// 1.39 routine saves only names ending `.gif`, `.png` or `.mng` (so no
    /// sound is saved at all), deletes an existing file first and sets no
    /// modification time; [`download_dir`] gives the folder only.
    Flat,
    /// 2.00 and 2.01: `sounds` for `.wav`, and under `levels` the folders
    /// `bomys`, `ganis`, `emoticons`, `heads`, `bodies`, `swords`, `shields`,
    /// else `webgifs`. There is no `hats` folder (a `hat0.png` goes to
    /// `webgifs`) and no `maps`.
    NoHats,
    /// 2.02 onward (read in 2.02, 2.03, 2.04, 2.05, both 2.1 test builds,
    /// 2.17.1, 2.22, 2.31): the list above plus `hats`.
    Full,
}

impl SaveLayout {
    /// The layout of a version. 1.42 (the NewMain builds differ from each
    /// other) is left on [`SaveLayout::Full`], unread.
    pub fn of(v: ClientVersion) -> SaveLayout {
        use ClientVersion::*;
        match v {
            V133 | V134 | V135 | V136 | V137 | V138 | V139 | V141 => SaveLayout::Flat,
            V200 | V201 => SaveLayout::NoHats,
            _ => SaveLayout::Full,
        }
    }
}

/// The folder (under the game folder) a downloaded file is saved in, as
/// path components. `name` is the file name the server sent.
///
/// Read from the save routine of 2.05 rev 2, 2.17.1, 2.22 and 2.31; names
/// compare without regard to case:
///
/// 1. `.wav` goes to `sounds`.
/// 2. With [`Files::download_maps`](crate::protocol::profile::Files): `.gmap`
///    goes to `maps`, and so does a `.txt`, `.gif`, `.png` or `.mng` whose
///    name contains `map`.
/// 3. Everything else goes under `levels`, in a folder by name: `bomys` for
///    a name that starts with `bomy` or `b_`; `ganis` for `.gani`;
///    `emoticons` for `emoticon…`; `hats` for `hat` followed by a digit
///    (the fourth character); then by a part of the name: `heads`
///    (`head`), `bodies` (`body`), `swords` (`sword`), `shields` (`shield`);
///    else `webgifs`.
///
/// The older builds differ ([`SaveLayout`]): 1.33 to 1.41 put everything in
/// `levels/webgifs`; 2.00 and 2.01 have the folder list without `hats`.
///
/// Seihai's one departure: a level file (`.nw`, `.graal`) goes straight in
/// `levels`. The real clients take levels from the board packet and their
/// own level cache, and would put a downloaded `.nw` in `levels/webgifs`;
/// Seihai asks for level files by name, and its resolver finds them in
/// `levels`.
pub fn download_dir(profile: &VersionProfile, name: &str) -> Vec<&'static str> {
    let lname = name.trim().to_ascii_lowercase();
    let base = lname.rsplit('/').next().unwrap_or(&lname);
    let ext = base.rfind('.').filter(|&i| i > 0).map(|i| &base[i..]).unwrap_or("");
    let layout = SaveLayout::of(profile.version);
    if layout == SaveLayout::Flat {
        return if matches!(ext, ".nw" | ".graal" | ".zelda") { vec!["levels"] } else { vec!["levels", "webgifs"] };
    }
    if ext == ".wav" {
        return vec!["sounds"];
    }
    if profile.files.download_maps {
        if ext == ".gmap" {
            return vec!["maps"];
        }
        if lname.contains("map") && matches!(ext, ".txt" | ".gif" | ".png" | ".mng") {
            return vec!["maps"];
        }
    }
    if matches!(ext, ".nw" | ".graal" | ".zelda") {
        return vec!["levels"];
    }
    // The real 4.04r2 keeps update packages in `updatepackages/` (lab,
    // 2026-10-31).
    if ext == ".gupd" {
        return vec!["updatepackages"];
    }
    let sub = if base.starts_with("bomy") || base.starts_with("b_") {
        "bomys"
    } else if ext == ".gani" {
        "ganis"
    } else if base.starts_with("emoticon") {
        "emoticons"
    } else if layout == SaveLayout::Full && base.starts_with("hat") && base.as_bytes().get(3).is_some_and(u8::is_ascii_digit) {
        "hats"
    } else if base.contains("head") {
        "heads"
    } else if base.contains("body") {
        "bodies"
    } else if base.contains("sword") {
        "swords"
    } else if base.contains("shield") {
        "shields"
    } else {
        "webgifs"
    };
    vec!["levels", sub]
}

/// The CRC-32 of a cached copy of `name` under the first of `roots` that
/// has one, in the folder [`download_dir`] gives it (or at the root). The
/// 4.x builds check a cached update package with `PLI_VERIFYWANTSEND` (47)
/// and this value ([`crate::net::verify_want_message`]).
pub fn cached_crc32(roots: &[&Path], profile: &VersionProfile, name: &str) -> Option<u32> {
    let rel = clean_name(name)?;
    roots.iter().find_map(|r| {
        let mut p = r.to_path_buf();
        for c in download_dir(profile, &rel) {
            p.push(c);
        }
        p.push(&rel);
        std::fs::read(&p).or_else(|_| std::fs::read(r.join(&rel))).ok()
    })
    .map(|d| crate::assets::crc32::crc32(&d))
}

/// Is this the name of an image the client draws (`.png`, `.gif`, `.mng`,
/// in any case)?
pub fn is_image_name(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    n.ends_with(".png") || n.ends_with(".gif") || n.ends_with(".mng")
}

/// The file's last-write time in Unix seconds.
pub fn mod_time_of(path: &Path) -> Option<u32> {
    let t = fs::metadata(path).ok()?.modified().ok()?;
    let secs = t.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(secs.min(u64::from(u32::MAX)) as u32)
}

/// Set a file's last-write time.
pub fn set_mod_time(path: &Path, secs: u32) -> io::Result<()> {
    let f = fs::OpenOptions::new().write(true).open(path)?;
    f.set_modified(UNIX_EPOCH + Duration::from_secs(u64::from(secs)))
}

/// A file the client holds in memory after a download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryFile {
    pub data: Vec<u8>,
    pub mod_time: Option<u32>,
}

/// Where a received file went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stored {
    /// Written to this path.
    Disk(PathBuf),
    /// Kept in memory only (saving downloads is off).
    Memory,
    /// A zero-length stream: the real save routine (read in 1.39 to 2.31,
    /// twelve builds' worth of the same guard) returns at once when the
    /// stream is missing or empty, so nothing is written, nothing existing is
    /// deleted and nothing is remembered.
    Ignored,
}

/// Saves downloads by the real clients' rules.
///
/// Where it writes: the resolver's [`download_root`](AssetResolver::download_root),
/// which is the per-server cache folder when there is one. It never writes
/// to the user's asset folder while a cache exists, and never to the
/// built-in set (a target under it is refused).
#[derive(Debug)]
pub struct DownloadStore {
    resolver: AssetResolver,
    save_to_disk: bool,
    memory: HashMap<String, MemoryFile>,
    /// The text of `server.txt`, written once into a new cache folder.
    cache_meta: Option<String>,
    /// With the Seihai folder's cache too: also write each download to the
    /// settings' cache (the user named one with `--cache-dir` / `cache_dir=`).
    settings_copy: bool,
}

impl DownloadStore {
    /// `save_to_disk` is the real clients' `saveimages` setting (on unless
    /// a user turns it off).
    pub fn new(resolver: AssetResolver, save_to_disk: bool) -> Self {
        Self { resolver, save_to_disk, memory: HashMap::new(), cache_meta: None, settings_copy: false }
    }

    pub fn resolver(&self) -> &AssetResolver {
        &self.resolver
    }

    /// Point the store at another server's cache folder (after a server
    /// warp). The in-memory copies of the old server are dropped: they are
    /// that server's files.
    pub fn set_cache(&mut self, dir: Option<PathBuf>, meta: Option<String>) {
        self.set_caches(None, dir, meta);
    }

    /// As [`set_cache`](Self::set_cache), with the Seihai folder's cache too.
    pub fn set_caches(&mut self, game_dir: Option<PathBuf>, dir: Option<PathBuf>, meta: Option<String>) {
        self.resolver.set_game_cache(game_dir);
        self.resolver.set_cache(dir);
        self.cache_meta = meta;
        self.memory.clear();
    }

    pub fn set_cache_meta(&mut self, meta: Option<String>) {
        self.cache_meta = meta;
    }

    /// Keep a copy of every download in the settings' cache as well as in
    /// the Seihai folder's (off unless the user named a cache folder). The
    /// settings' cache is searched either way.
    pub fn set_settings_copy(&mut self, on: bool) {
        self.settings_copy = on;
    }

    /// The file the client already has under this name, and the layer it is
    /// in: per layer (cache, the user's folder, the built-in set), first the
    /// name as a path from the game folder, then the folders its kind lives
    /// in. Only a file with the same name counts (a `.png` is not the `.gif`
    /// the server was asked for).
    pub fn local_copy(&self, name: &str) -> Option<(PathBuf, Layer)> {
        let rel = clean_name(name)?;
        let r = &self.resolver;
        for (layer, root) in [
            (Layer::Cache, r.game_cache_root()),
            (Layer::Cache, r.cache_root()),
            (Layer::Assets, r.user_root()),
            (Layer::Builtin, r.builtin_root()),
        ] {
            let Some(root) = root else { continue };
            if let Some(p) = find_in_root(root, r, &rel, name).filter(|p| layer != Layer::Builtin || r.builtin_levels() || !is_level_name(p)) {
                return Some((p, layer));
            }
        }
        None
    }

    /// As [`local_copy`](Self::local_copy), the path only.
    pub fn local_path(&self, name: &str) -> Option<PathBuf> {
        self.local_copy(name).map(|(p, _)| p)
    }

    /// Mod time of the copy the client holds *of its own* (the cache or the
    /// user's folder), in Unix seconds. A built-in file has no mod time to
    /// offer a server: it is not a copy of the server's file.
    pub fn local_mod_time(&self, name: &str) -> Option<u32> {
        match self.local_copy(name)? {
            (_, Layer::Builtin) => None,
            (p, _) => Some(mod_time_of(&p).unwrap_or(0)),
        }
    }

    /// Bytes of a file: the in-memory copy of a download, else the file
    /// (from any layer, the built-in set included).
    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        if let Some(m) = self.memory.get(&key(name)) {
            return Some(m.data.clone());
        }
        fs::read(self.local_path(name)?).ok()
    }

    pub fn memory_file(&self, name: &str) -> Option<&MemoryFile> {
        self.memory.get(&key(name))
    }

    /// Does the client have this file in any form: kept in memory, in the
    /// cache, in the user's folder or in the built-in set?
    pub fn has_any_copy(&self, name: &str) -> bool {
        self.memory.contains_key(&key(name)) || self.local_copy(name).is_some()
    }

    /// Save one received file. A file the client already holds *in the
    /// download folder* (the cache) is replaced where it is; a new one goes
    /// to [`download_dir`] under the cache. A copy in the user's folder or
    /// in the built-in set is left alone: the server's file lands in the
    /// cache, which is searched first, so it wins on this server. With a mod
    /// time from the server (2.1+) the file gets it as its time, so the next
    /// `UPDATEFILE` carries the time the server knows; before 2.1 the
    /// file's time is the time of the download.
    ///
    /// When saving is off, everything except a level file stays in memory.
    /// Level files always go to disk: Seihai loads levels from files.
    pub fn store(&mut self, name: &str, data: &[u8], mod_time: Option<u32>) -> io::Result<Stored> {
        let rel = clean_name(name).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe file name"))?;
        if data.is_empty() {
            return Ok(Stored::Ignored);
        }
        let ext = rel.rfind('.').map(|i| rel[i..].to_ascii_lowercase()).unwrap_or_default();
        let is_level = matches!(ext.as_str(), ".nw" | ".graal" | ".zelda" | ".gmap");
        self.memory.insert(key(name), MemoryFile { data: data.to_vec(), mod_time });
        if !self.save_to_disk && !is_level {
            return Ok(Stored::Memory);
        }
        let Some(dl) = self.resolver.download_root().cloned() else {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "no download folder (only the read-only built-in set)"));
        };
        // With both caches, the download goes to the Seihai folder's. The
        // settings' cache is written only when the user named it (a copy),
        // or when the Seihai folder cannot be written (instead).
        let second = self.resolver.cache_root().filter(|c| c.root != dl.root).cloned();
        let first = self.write_into(&dl, &rel, name, data, mod_time);
        let Some(second) = second else { return first.map(Stored::Disk) };
        match first {
            Ok(t) => {
                if self.settings_copy {
                    if let Err(e) = self.write_into(&second, &rel, name, data, mod_time) {
                        eprintln!("settings cache copy of {name} not written: {e}");
                    }
                }
                Ok(Stored::Disk(t))
            }
            Err(e) => match self.write_into(&second, &rel, name, data, mod_time) {
                Ok(t) => {
                    eprintln!("Seihai folder cache copy of {name} not written ({e}); saved in the settings cache");
                    Ok(Stored::Disk(t))
                }
                Err(_) => Err(e),
            },
        }
    }

    /// Write one download into the cache (or download) folder `dl`: in place
    /// if that folder already has the file, else by [`download_dir`].
    fn write_into(&self, dl: &AssetRoot, rel: &str, name: &str, data: &[u8], mod_time: Option<u32>) -> io::Result<PathBuf> {
        let in_place = find_in_root(dl, &self.resolver, rel, name);
        let target = match in_place {
            Some(p) => p,
            None => {
                let mut p = dl.root.clone();
                for c in download_dir(self.resolver.profile(), rel) {
                    p.push(c);
                }
                p.push(rel);
                p
            }
        };
        // The built-in set is never a target, whatever the name.
        if let Some(b) = self.resolver.builtin_root() {
            if target.starts_with(&b.root) {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the built-in assets are read-only"));
            }
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if let (Some(meta), true) = (&self.cache_meta, self.resolver.cache_roots().any(|c| c.root == dl.root)) {
            let m = dl.root.join("server.txt");
            if !m.exists() {
                let _ = fs::write(m, meta);
            }
        }
        if target.exists() {
            fs::remove_file(&target)?;
        }
        fs::write(&target, data)?;
        if let Some(t) = mod_time.filter(|&t| t != 0) {
            set_mod_time(&target, t)?;
        }
        Ok(target)
    }
}

/// The copy of `rel` in one folder: the name as a path from the folder, then
/// the folders its kind lives in (as the resolver searches them, restricted
/// to this one folder).
fn find_in_root(root: &AssetRoot, resolver: &AssetResolver, rel: &str, name: &str) -> Option<PathBuf> {
    if let Some(p) = lookup(&root.root, rel) {
        return Some(p);
    }
    let last = rel.rsplit('/').next().unwrap_or(rel).to_ascii_lowercase();
    let ext = last.rfind('.').map(|i| &last[i..]).unwrap_or("");
    let one = AssetResolver::new(root.clone(), resolver.profile());
    let found = match ext {
        ".png" | ".gif" | ".mng" => one.image(name),
        ".gani" => one.gani(name),
        ".wav" => one.sound(name),
        ".nw" | ".graal" | ".zelda" | ".gmap" => one.level(name),
        _ => None,
    }?;
    let same = found.file_name().and_then(|f| f.to_str()).is_some_and(|f| f.eq_ignore_ascii_case(&last));
    same.then_some(found)
}

/// What [`FileSession::process`] found in the server's traffic.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Processed {
    /// Messages to send: the requests that were waiting for a free slot.
    pub send: Vec<Vec<u8>>,
    /// Names of files saved (or kept in memory).
    pub written: Vec<String>,
    /// Names the server said are up to date.
    pub up_to_date: Vec<String>,
    /// Names the server could not send.
    pub failed: Vec<String>,
}

/// A request list and a download store, for one connection.
#[derive(Debug)]
pub struct FileSession {
    pub requests: FileRequester,
    pub store: DownloadStore,
    /// Steps taken on this connection, for [`on_step`](Self::on_step).
    steps: u32,
}

/// What the real client asks for when it takes its first steps, because
/// its clean folder has none of it (2.31 `gmap_cross` capture: `steps.wav`
/// and `walk.gani` in the frame of the first step, `steps2.wav` with the
/// second). `(step number, names)`.
pub const FIRST_STEP_REQUESTS: &[(u32, &[&str])] = &[(1, &["steps.wav", "walk.gani"]), (2, &["steps2.wav"])];

impl FileSession {
    pub fn new(resolver: AssetResolver, save_to_disk: bool) -> Self {
        let requests = FileRequester::new(resolver.profile());
        Self { requests, store: DownloadStore::new(resolver, save_to_disk), steps: 0 }
    }

    /// A new connection: the name lists and the failure memory start empty
    /// (the real client keeps them per connection). Downloads already kept
    /// stay.
    pub fn new_connection(&mut self) {
        self.requests = FileRequester::new(self.store.resolver().profile());
        self.steps = 0;
    }

    /// The client needs this file. Returns the messages to send now: a
    /// `WANTFILE` if the file is missing, an `UPDATEFILE` with the local
    /// mod time if it is there and the build checks, else nothing.
    ///
    /// A file that only the **built-in set** has counts as missing here: it
    /// is Seihai's stand-in, not a copy of the server's file, so it is asked
    /// for with `WANTFILE` (no mod time is ever sent for it) and the
    /// server's copy, if it has one, is saved in the cache and found before
    /// the built-in one. If the server has none the reply is
    /// `FILESENDFAILED` and the built-in file stays in use.
    pub fn need(&mut self, name: &str) -> Vec<Vec<u8>> {
        let Some(clean) = clean_name(name) else { return Vec::new() };
        let local = self.store.local_mod_time(&clean);
        let local = local.or_else(|| self.store.memory_file(&clean).map(|m| m.mod_time.unwrap_or(0)));
        let msgs = self.requests.need(name, local).iter().map(FileRequest::message).collect();
        self.with_gani_updates(msgs)
    }

    /// A 4.x build follows each `WANTFILE` for a GANI with `PLI_UPDATEGANI`
    /// (157, [`crate::protocol::lobby::with_gani_updates`]); other builds
    /// send the messages as they are.
    fn with_gani_updates(&self, msgs: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
        if self.store.resolver().profile().protocol.lobby.is_some_and(|l| l.after_login.is_some()) {
            crate::protocol::lobby::with_gani_updates(msgs)
        } else {
            msgs
        }
    }

    /// The player took a step. On the first two steps of a connection the
    /// real client asks for `steps.wav`, `walk.gani` and `steps2.wav`
    /// ([`FIRST_STEP_REQUESTS`]); Seihai asks for one **only if it has no
    /// copy at all** ([`need_missing`](Self::need_missing)). The built-in set
    /// has all three, so in practice a normal run asks for none of them; the
    /// request is for a client run without its built-in layer
    /// (`--no-builtin`). Builds without GANIs, and builds that do not send
    /// `WANTFILE`, ask for none (not captured for them).
    pub fn on_step(&mut self) -> Vec<Vec<u8>> {
        self.steps = self.steps.saturating_add(1);
        // The step sounds are cues of walk.gani: no GANIs, none of the three.
        if !self.store.resolver().profile().ui.ganis {
            return Vec::new();
        }
        let Some((_, names)) = FIRST_STEP_REQUESTS.iter().find(|(n, _)| *n == self.steps) else {
            return Vec::new();
        };
        names.iter().flat_map(|name| self.need_missing(name)).collect()
    }

    /// A GANI or a sound the client went to use and has **no copy of at
    /// all**: a copy in the cache, in the user's folder, in the built-in set
    /// or already received this session counts as having it. Returns the
    /// `WANTFILE` to send, as the real GANI and sound loaders do for a name
    /// they cannot find (`file-cache.md`, "Who asks for what?"). Gated by the
    /// profile: no request from a build that does not send `WANTFILE`
    /// (`pli` 23), and none for a GANI from a build without GANIs. Unlike
    /// [`need`](Self::need), a built-in copy counts as present, so this never
    /// asks the server to replace one.
    pub fn need_missing(&mut self, name: &str) -> Vec<Vec<u8>> {
        let profile = self.store.resolver().profile();
        if !profile.protocol.pli.contains(23) {
            return Vec::new();
        }
        let lower = name.trim().to_ascii_lowercase();
        if lower.ends_with(".gani") && !profile.ui.ganis {
            return Vec::new();
        }
        if self.store.has_any_copy(name) {
            return Vec::new();
        }
        self.need(name)
    }

    /// Apply what the server sent since the last call: save finished
    /// files, and settle the names of every reply.
    pub fn process(&mut self, state: &mut SessionState) -> Processed {
        let mut out = Processed::default();
        let mut next = Vec::new();
        for f in state.take_files() {
            if self.store.store(&f.name, &f.data, f.mod_time).is_ok_and(|s| s != Stored::Ignored) {
                out.written.push(f.name.clone());
            }
            next.extend(self.requests.done(&f.name));
        }
        for r in std::mem::take(&mut state.file_requests) {
            if let Some(n) = r.strip_prefix("FAIL:") {
                next.extend(self.requests.failed(n));
                out.failed.push(n.to_string());
            } else if let Some(n) = r.strip_prefix("UPTODATE:") {
                next.extend(self.requests.done(n));
                out.up_to_date.push(n.to_string());
            }
        }
        out.send = self.with_gani_updates(next.iter().map(FileRequest::message).collect());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetRoot;
    use crate::protocol::ClientVersion;

    fn req(v: ClientVersion) -> FileRequester {
        FileRequester::new(v.profile())
    }

    fn names(r: &[FileRequest]) -> Vec<&str> {
        r.iter().map(FileRequest::name).collect()
    }

    #[test]
    fn image_names_in_any_case() {
        assert!(is_image_name("Block.PNG") && is_image_name("door.gif") && is_image_name("x.Mng"));
        assert!(!is_image_name("a.nw") && !is_image_name("png"));
    }

    #[test]
    fn four_wants_go_out_and_the_rest_wait() {
        let mut r = req(ClientVersion::V222);
        let mut sent = Vec::new();
        for n in ["a.png", "b.png", "c.png", "d.png", "e.png", "f.png"] {
            sent.extend(r.need(n, None));
        }
        assert_eq!(names(&sent), ["a.png", "b.png", "c.png", "d.png"]);
        assert_eq!(r.wanted().len(), 6);
        // A reply for one of the four releases the first waiting name.
        assert_eq!(names(&r.done("b.png")), ["e.png"]);
        assert_eq!(names(&r.done("a.png")), ["f.png"]);
        assert!(r.done("c.png").is_empty());
        assert_eq!(r.wanted(), ["d.png", "e.png", "f.png"]);
    }

    #[test]
    fn a_waiting_name_goes_out_as_the_server_named_it() {
        // Names past the first four wait; when a slot frees, the one that
        // goes out must keep the case the level or the server gave it (an
        // older GServer-v2 fails a name asked for in another case).
        let mut r = req(ClientVersion::V222);
        for n in ["a.png", "b.png", "c.png", "d.png", "Block.PNG", "npcs/Sign.gif"] {
            r.need(n, None);
        }
        assert_eq!(names(&r.done("a.png")), ["Block.PNG"]);
        assert_eq!(names(&r.done("B.PNG")), ["npcs/Sign.gif"]);
        assert_eq!(r.wanted(), ["c.png", "d.png", "Block.PNG", "npcs/Sign.gif"]);
        // The same for the UPDATEFILE list.
        for (i, n) in ["u1.png", "u2.png", "u3.png", "u4.png", "Door_Open.GIF"].iter().enumerate() {
            r.need(n, Some(10 + i as u32));
        }
        assert_eq!(r.done("u1.png"), vec![FileRequest::Update { mod_time: 14, name: "Door_Open.GIF".into() }]);
        assert_eq!(r.update_asked(), ["u2.png", "u3.png", "u4.png", "Door_Open.GIF"]);
        // A reply in another case still settles the name.
        assert!(r.done("door_open.gif").is_empty());
        assert!(r.is_settled("DOOR_OPEN.GIF") && r.update_asked().len() == 3);
    }

    #[test]
    fn a_want_is_not_repeated_while_it_is_pending() {
        let mut r = req(ClientVersion::V222);
        assert_eq!(r.need("A.PNG", None).len(), 1);
        assert!(r.need("a.png", None).is_empty());
    }

    #[test]
    fn update_has_its_own_window_and_one_ask_per_name() {
        let mut r = req(ClientVersion::V222);
        let mut sent = Vec::new();
        for n in ["w1.png", "w2.png", "w3.png", "w4.png", "w5.png"] {
            sent.extend(r.need(n, None));
        }
        // Four wants are out. Updates count on their own.
        for (i, n) in ["u1.png", "u2.png", "u3.png", "u4.png", "u5.png"].iter().enumerate() {
            sent.extend(r.need(n, Some(100 + i as u32)));
        }
        let want: Vec<_> = sent.iter().filter(|s| matches!(s, FileRequest::Want(_))).collect();
        let upd: Vec<_> = sent.iter().filter(|s| matches!(s, FileRequest::Update { .. })).collect();
        assert_eq!((want.len(), upd.len()), (4, 4));
        // The fifth update waits and keeps the mod time it was queued with.
        assert_eq!(r.done("u1.png"), vec![FileRequest::Update { mod_time: 104, name: "u5.png".into() }]);
        // A name that was answered is not asked about again.
        assert!(r.need("u1.png", Some(100)).is_empty());
        // Even if it is asked for again as a pending name.
        assert!(r.need("u2.png", Some(101)).is_empty());
    }

    #[test]
    fn a_file_that_arrived_is_not_revalidated_in_the_same_session() {
        let mut r = req(ClientVersion::V222);
        assert_eq!(r.need("x.png", None).len(), 1);
        r.done("x.png");
        // Now there is a local copy; the answer for it is already in.
        assert!(r.need("x.png", Some(5)).is_empty());
    }

    #[test]
    fn a_missing_file_is_asked_for_once_per_connection() {
        let mut r = req(ClientVersion::V231);
        assert_eq!(r.need("steps.wav", None).len(), 1);
        r.failed("steps.wav");
        for _ in 0..50 {
            assert!(r.need("steps.wav", None).is_empty());
        }
    }

    #[test]
    fn builds_without_updatefile_never_revalidate() {
        // 1.41 sends 23 but never 34; 1.42 and 2.00 add 34.
        let mut r141 = req(ClientVersion::V141);
        assert!(r141.need("x.gif", Some(9)).is_empty());
        assert_eq!(r141.need("y.gif", None).len(), 1);
        let mut r142 = req(ClientVersion::V142);
        assert_eq!(r142.need("x.gif", Some(9)), vec![FileRequest::Update { mod_time: 9, name: "x.gif".into() }]);
    }

    #[test]
    fn empty_names_are_ignored() {
        let mut r = req(ClientVersion::V222);
        assert!(r.need("", None).is_empty());
        assert!(r.need("  ", Some(3)).is_empty());
    }

    #[test]
    fn messages_follow_the_captures() {
        // `PLI_WANTFILE`: id 23 + 0x20, the name.
        assert_eq!(FileRequest::Want("walk.gani".into()).message(), b"7walk.gani\n");
        // `PLI_UPDATEFILE`: id 34 + 0x20, GINT5 time, the name. 1792835138 =
        // 0x6AFE_E2C2: five 7-bit groups, high first, each plus 0x20.
        let m = FileRequest::Update { mod_time: 1_792_835_138, name: "body.png".into() }.message();
        assert_eq!(m[0], 34 + 0x20);
        let t = m[1..6].iter().fold(0u64, |a, &c| (a << 7) | u64::from(c - 0x20));
        assert_eq!(t, 1_792_835_138);
        assert_eq!(&m[6..], b"body.png\n");
    }

    fn dir(v: ClientVersion, name: &str) -> String {
        download_dir(v.profile(), name).join("/")
    }

    #[test]
    fn old_builds_lay_downloads_out_by_their_own_routine() {
        use ClientVersion::*;
        // 1.33 to 1.41: one folder for everything.
        for v in [V133, V135, V139, V141] {
            assert_eq!(SaveLayout::of(v), SaveLayout::Flat);
            for n in ["a.png", "body.png", "sword1.gif", "x.mng", "walk.gani", "b.wav", "worldmap.png"] {
                assert_eq!(dir(v, n), "levels/webgifs", "{v:?} {n}");
            }
        }
        assert_eq!(dir(V139, "old.graal"), "levels", "Seihai's level departure holds");
        // 2.00 and 2.01: the folder list without hats.
        for v in [V200, V201] {
            assert_eq!(SaveLayout::of(v), SaveLayout::NoHats);
            assert_eq!(dir(v, "hat0.png"), "levels/webgifs");
            assert_eq!(dir(v, "head25.png"), "levels/heads");
            assert_eq!(dir(v, "walk.gani"), "levels/ganis");
            assert_eq!(dir(v, "b_x.png"), "levels/bomys");
            assert_eq!(dir(v, "s.wav"), "sounds");
            assert_eq!(dir(v, "gm.gmap"), "levels/webgifs", "no maps folder");
        }
        // 2.02 onward: hats exist.
        for v in [V202, V202R2, V203, V204, V205, V210, V217, V231] {
            assert_eq!(SaveLayout::of(v), SaveLayout::Full);
            assert_eq!(dir(v, "hat0.png"), "levels/hats", "{v:?}");
        }
    }

    #[test]
    fn an_empty_stream_changes_nothing() {
        let root = tmp("empty");
        let mut s = store(ClientVersion::V222, &root, true);
        assert_eq!(s.store("body.png", b"keep", Some(7)).unwrap(), Stored::Disk(root.join("levels/bodies/body.png")));
        assert_eq!(s.store("body.png", b"", Some(9)).unwrap(), Stored::Ignored);
        assert_eq!(fs::read(root.join("levels/bodies/body.png")).unwrap(), b"keep", "the old copy stays");
        assert_eq!(s.store("new.png", b"", None).unwrap(), Stored::Ignored);
        assert!(!root.join("levels/webgifs/new.png").exists(), "no empty file is created");
        assert!(!s.has_any_copy("new.png"), "and nothing is remembered in memory");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn download_folders_follow_the_clients() {
        use ClientVersion::*;
        // 2.22, 2.17 and 2.31 read the same routine.
        for v in [V217, V222, V231] {
            assert_eq!(dir(v, "steps.wav"), "sounds");
            assert_eq!(dir(v, "gm2x2.gmap"), "maps");
            assert_eq!(dir(v, "bigmap.txt"), "maps");
            assert_eq!(dir(v, "worldmap1.png"), "maps");
            assert_eq!(dir(v, "bomy_walk.gani"), "levels/bomys");
            assert_eq!(dir(v, "b_x.png"), "levels/bomys");
            assert_eq!(dir(v, "walk.gani"), "levels/ganis");
            assert_eq!(dir(v, "emoticon_smile.png"), "levels/emoticons");
            assert_eq!(dir(v, "hat0.png"), "levels/hats");
            assert_eq!(dir(v, "head25.png"), "levels/heads");
            assert_eq!(dir(v, "body.png"), "levels/bodies");
            assert_eq!(dir(v, "sword1.png"), "levels/swords");
            assert_eq!(dir(v, "shield2.png"), "levels/shields");
            assert_eq!(dir(v, "plisticononline.png"), "levels/webgifs");
            assert_eq!(dir(v, "SWORD1.PNG"), "levels/swords");
        }
        // "hat" counts only with a digit as the fourth character.
        assert_eq!(dir(V222, "hat.png"), "levels/webgifs");
        assert_eq!(dir(V222, "hatx.png"), "levels/webgifs");
        assert_eq!(dir(V222, "hatshead.png"), "levels/heads");
        // A `.png` with `map` in the name is a map file only with the rule.
        assert_eq!(dir(V205, "worldmap1.png"), "levels/webgifs");
        assert_eq!(dir(V205, "gm2x2.gmap"), "levels/webgifs");
        assert_eq!(dir(V205, "steps.wav"), "sounds");
        assert_eq!(dir(V205, "walk.gani"), "levels/ganis");
        // Seihai's departure: level files stay in levels/.
        assert_eq!(dir(V222, "lv_a.nw"), "levels");
        assert_eq!(dir(V205, "old.graal"), "levels");
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("seihai-fc-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn store(v: ClientVersion, root: &Path, save: bool) -> DownloadStore {
        DownloadStore::new(AssetResolver::new(AssetRoot::new(root), v.profile()), save)
    }

    #[test]
    fn a_new_download_goes_to_its_folder_with_the_servers_time() {
        let root = tmp("new");
        let mut s = store(ClientVersion::V222, &root, true);
        let got = s.store("sword1.png", b"PNGDATA", Some(1_700_000_000)).unwrap();
        let want = root.join("levels/swords/sword1.png");
        assert_eq!(got, Stored::Disk(want.clone()));
        assert_eq!(fs::read(&want).unwrap(), b"PNGDATA");
        assert_eq!(mod_time_of(&want), Some(1_700_000_000));
        // The resolver finds it, and an UPDATEFILE would carry that time.
        assert_eq!(s.local_mod_time("sword1.png"), Some(1_700_000_000));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_existing_file_is_replaced_where_it_is() {
        let root = tmp("inplace");
        fs::create_dir_all(root.join("levels/bodies")).unwrap();
        fs::write(root.join("levels/bodies/body.png"), b"old").unwrap();
        // A copy at the game folder's root wins, as the clients try the
        // name as a path first.
        fs::write(root.join("body.png"), b"root").unwrap();
        let mut s = store(ClientVersion::V222, &root, true);
        let got = s.store("body.png", b"new", Some(1_650_000_000)).unwrap();
        assert_eq!(got, Stored::Disk(root.join("body.png")));
        assert_eq!(fs::read(root.join("body.png")).unwrap(), b"new");
        assert_eq!(fs::read(root.join("levels/bodies/body.png")).unwrap(), b"old");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn before_2_1_the_file_time_is_the_download_time() {
        let root = tmp("205");
        let mut s = store(ClientVersion::V205, &root, true);
        let before = mod_time_of(&root).unwrap_or(0);
        let Stored::Disk(p) = s.store("walk.gani", b"GANI", None).unwrap() else { panic!() };
        assert_eq!(p, root.join("levels/ganis/walk.gani"));
        // No server time to set: the file is as new as the download.
        assert!(mod_time_of(&p).unwrap() >= before);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn saving_off_keeps_downloads_in_memory_but_levels_still_go_to_disk() {
        let root = tmp("nosave");
        let mut s = store(ClientVersion::V222, &root, false);
        assert_eq!(s.store("head0.png", b"H", Some(5)).unwrap(), Stored::Memory);
        assert!(!root.join("levels").exists());
        assert_eq!(s.read("HEAD0.png").unwrap(), b"H");
        assert_eq!(s.memory_file("head0.png").unwrap().mod_time, Some(5));
        let Stored::Disk(p) = s.store("lv.nw", b"GLEVNW01\n", None).unwrap() else { panic!("level") };
        assert_eq!(p, root.join("levels/lv.nw"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unsafe_names_are_refused() {
        let root = tmp("unsafe");
        let mut s = store(ClientVersion::V222, &root, true);
        for bad in ["../x.png", "/etc/x", "c:/x.png", ""] {
            assert!(s.store(bad, b"x", None).is_err(), "{bad}");
        }
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn only_a_file_with_the_same_name_counts_as_local() {
        let root = tmp("samename");
        fs::write(root.join("pic.png"), b"x").unwrap();
        let s = store(ClientVersion::V222, &root, true);
        assert!(s.local_path("pic.png").is_some());
        // The resolver would fall back from .gif to .png; the client's
        // name index would not.
        assert!(s.local_path("pic.gif").is_none());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn session_asks_for_missing_and_checks_present_files_once() {
        let root = tmp("session");
        fs::create_dir_all(root.join("levels/swords")).unwrap();
        fs::write(root.join("levels/swords/sword1.png"), b"S").unwrap();
        set_mod_time(&root.join("levels/swords/sword1.png"), 1_792_835_138).unwrap();
        let mut fs_ = FileSession::new(AssetResolver::new(AssetRoot::new(&root), ClientVersion::V222.profile()), true);
        // Present: UPDATEFILE with the file's time. Missing: WANTFILE.
        let m = fs_.need("sword1.png");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0][0], 34 + 0x20);
        assert!(fs_.need("sword1.png").is_empty());
        assert_eq!(fs_.need("head0.png"), vec![wantfile_message("head0.png")]);
        // The server answers: up to date for one, failed for the other.
        let mut st = SessionState::for_version(ClientVersion::V222);
        st.file_requests.push("UPTODATE:sword1.png".into());
        st.file_requests.push("FAIL:head0.png".into());
        let p = fs_.process(&mut st);
        assert_eq!(p.up_to_date, ["sword1.png"]);
        assert_eq!(p.failed, ["head0.png"]);
        assert!(st.file_requests.is_empty());
        assert!(fs_.need("head0.png").is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_new_connection_starts_the_lists_again() {
        let root = tmp("newconn");
        let mut fs_ = FileSession::new(AssetResolver::new(AssetRoot::new(&root), ClientVersion::V222.profile()), true);
        assert_eq!(fs_.need("nothere.png").len(), 1);
        assert!(fs_.need("nothere.png").is_empty());
        fs_.new_connection();
        assert_eq!(fs_.need("nothere.png").len(), 1);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn session_saves_a_file_and_frees_the_next_waiting_request() {
        let root = tmp("session2");
        let mut fs_ = FileSession::new(AssetResolver::new(AssetRoot::new(&root), ClientVersion::V222.profile()), true);
        let mut sent = 0;
        for i in 0..6 {
            sent += fs_.need(&format!("img{i}.png")).len();
        }
        assert_eq!(sent, 4);
        let mut st = SessionState::for_version(ClientVersion::V222);
        st.incoming_files.push(("img1.png".into(), b"P".to_vec()));
        st.incoming_mod_times.insert("img1.png".into(), 1_600_000_000);
        let p = fs_.process(&mut st);
        assert_eq!(p.written, ["img1.png"]);
        assert_eq!(p.send, vec![wantfile_message("img4.png")]);
        let saved = root.join("levels/webgifs/img1.png");
        assert_eq!(mod_time_of(&saved), Some(1_600_000_000));
        fs::remove_dir_all(&root).ok();
    }

    /// A 4.x build keeps update packages in `updatepackages/` and checks a
    /// cached one by its CRC-32 (lab, 2026-10-31).
    #[test]
    fn update_packages_and_their_crc() {
        let p = crate::protocol::ClientVersion::V404R2.profile();
        assert_eq!(download_dir(p, "basepackage.gupd"), ["updatepackages"]);
        let a = tmp("gupd-a");
        let b = tmp("gupd-b");
        assert_eq!(cached_crc32(&[&a, &b], p, "basepackage.gupd"), None);
        fs::create_dir_all(b.join("updatepackages")).unwrap();
        fs::write(b.join("updatepackages").join("basepackage.gupd"), b"GRPKG001\n").unwrap();
        assert_eq!(cached_crc32(&[&a, &b], p, "basepackage.gupd"), Some(0x15a8ea07));
    }
}
