//! Version-aware asset lookup in a client data folder.
//!
//! The 2.22 data folder ("Graal Reborn 2.22.0") is the reference layout.
//! Every client version looks for the same kinds of files in it; the
//! differences are in which names and extensions a build can use, and those
//! come from the version's profile row, not from branches here.
//!
//! What the resolver does that a plain `root.join(name)` does not:
//!
//! * **Several folders per kind.** Levels sit at the root (`new1.nw`), in
//!   `maps/` (`.gmap`) or in `levels/` (where downloads are saved); sounds in
//!   `sounds/`; GANIs in `levels/ganis/` (and `levels/bomys/`); player art in `levels/bodies/` and
//!   friends; everything else image-like in the other `levels/<kind>/`
//!   folders.
//! * **Case-insensitive names.** Windows clients ignore case. A Linux file
//!   system does not, and the folder has names such as `TORERO-2.MID`.
//! * **Extension rules from the profile.** `files.image_ext` is added to a
//!   bare image name (GServer adds `.gif` for clients below 2.1); level
//!   extensions are limited to what the build opens (`levels.nw`,
//!   `levels.graal`, `levels.zelda`, `levels.gmap`); GANIs only where `ui.ganis` is set.
//! * **Safe names.** Names come from the server. `..`, absolute paths and
//!   drive letters never leave the folder.
//!
//! * **Layers.** A lookup searches the per-server cache first, then the
//!   user's asset folder, then the built-in default set
//!   (`documentation/default-assets.md`, `documentation/file-cache.md`).
//!   Downloads are written to the cache only; the built-in set is read-only.
//!
//! Lookups never cache a miss: a file that arrives from the server a moment
//! later is found on the next call.
//!
//! Not settled (see `documentation/assets.md`): the real client's own list
//! of folders per kind. The orders below are Seihai's.

use std::fs;
use std::path::{Path, PathBuf};

use super::AssetRoot;
use crate::protocol::VersionProfile;

/// Player and item art folders under `levels/` in the 2.22 data folder.
/// The same names are also tried directly under the root (older layouts).
pub const IMAGE_CATEGORIES: &[&str] = &[
    "images", "ganis", "bodies", "heads", "hats", "shields", "swords", "weapons", "bomys", "baddies", "emoticons",
    "horses", "backpals", "smilies", "letters", "snapshot", "webgifs", "tutorial",
];

/// The player-art folders a GIF build reads as `.gif` only.
pub const PLAYER_ART: &[&str] = &["heads", "bodies", "swords", "shields"];

/// `rel` with its image extension (if any) replaced by `ext`.
fn with_ext(rel: &str, ext: &str) -> String {
    match ext_of(rel) {
        Some(e) => format!("{}{ext}", &rel[..rel.len() - e.len()]),
        None => format!("{rel}{ext}"),
    }
}

/// Where a download is saved, and where a downloaded file is looked for
/// first. This is `levels/`, as `main.rs` writes files there.
const DOWNLOADS: &str = "levels";

/// Which layer of the resolver a path came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// The per-server cache: what the server sent. Searched first.
    Cache,
    /// The user's asset folder (`--assets`).
    Assets,
    /// The built-in default set. Read-only; searched last.
    Builtin,
}

/// Finds files for one version in up to four folders, in this order:
///
/// 1. the **per-server cache**, what the server sent: first the one in the
///    Seihai folder (`_cache/<host>_<port>/`), then the one with the
///    settings (`<settings dir>/cache/<host>_<port>~<name>/`). Downloads are
///    written to the first (the second only as a fallback, or as a copy when
///    the user named it) and nowhere else,
/// 2. the user's **asset folder** (`--assets`),
/// 3. the **built-in default set** (read-only, never a download target).
///
/// Inside one folder the folder order of each kind is as before (`levels/`,
/// the root, `maps/` for a level, and so on); the layers are searched one
/// after the other, so a file in a later layer is found only when no earlier
/// layer has it. `AssetResolver::new` makes a resolver of one folder (the
/// user's), which is what every lookup was before the layers existed.
#[derive(Debug, Clone)]
pub struct AssetResolver {
    /// The cache in the Seihai folder, searched before [`Self::cache`].
    game_cache: Option<AssetRoot>,
    cache: Option<AssetRoot>,
    user: Option<AssetRoot>,
    builtin: Option<AssetRoot>,
    /// Whether the built-in set's levels (`new1.nw`, `new2.nw`) can be
    /// found. Off while online: a server's `new1.nw` must come from the
    /// server, not from the starter level of the same name.
    builtin_levels: bool,
    profile: &'static VersionProfile,
}

impl AssetResolver {
    /// One folder (the user's), no cache and no built-in set.
    pub fn new(root: AssetRoot, profile: &'static VersionProfile) -> Self {
        Self { game_cache: None, cache: None, user: Some(root), builtin: None, builtin_levels: true, profile }
    }

    /// A resolver with no folder yet: add layers with the `with_*` methods.
    pub fn empty(profile: &'static VersionProfile) -> Self {
        Self { game_cache: None, cache: None, user: None, builtin: None, builtin_levels: true, profile }
    }

    /// Set (or clear) the user's asset folder.
    pub fn with_assets(mut self, root: Option<AssetRoot>) -> Self {
        self.user = root;
        self
    }

    /// Set the per-server cache folder (searched first, written by downloads).
    pub fn with_cache(mut self, dir: Option<PathBuf>) -> Self {
        self.cache = dir.map(AssetRoot::new);
        self
    }

    /// Set the cache folder in the Seihai folder (searched before the
    /// settings' cache, and the first download target).
    pub fn with_game_cache(mut self, dir: Option<PathBuf>) -> Self {
        self.game_cache = dir.map(AssetRoot::new);
        self
    }

    /// Set the folder of the built-in set (searched last, never written).
    pub fn with_builtin(mut self, dir: Option<PathBuf>) -> Self {
        self.builtin = dir.map(AssetRoot::new);
        self
    }

    /// Whether the built-in levels can be found (on offline, off online).
    pub fn with_builtin_levels(mut self, on: bool) -> Self {
        self.builtin_levels = on;
        self
    }

    /// Replace the cache folder of a resolver in use (after a server warp).
    pub fn set_cache(&mut self, dir: Option<PathBuf>) {
        self.cache = dir.map(AssetRoot::new);
    }

    /// Replace the Seihai folder's cache of a resolver in use.
    pub fn set_game_cache(&mut self, dir: Option<PathBuf>) {
        self.game_cache = dir.map(AssetRoot::new);
    }

    pub fn set_builtin_levels(&mut self, on: bool) {
        self.builtin_levels = on;
    }

    pub fn builtin_levels(&self) -> bool {
        self.builtin_levels
    }

    /// The main data folder: the user's, else the built-in set's. Used for
    /// what is not looked up by name (a level picker, a sounds folder
    /// message). It is not where downloads go: see [`download_root`](Self::download_root).
    ///
    /// # Panics
    /// If the resolver has no folder at all (no layer was added).
    pub fn root(&self) -> &AssetRoot {
        self.user.as_ref().or(self.builtin.as_ref()).or(self.game_cache.as_ref()).or(self.cache.as_ref()).expect("an AssetResolver has at least one folder")
    }

    pub fn user_root(&self) -> Option<&AssetRoot> {
        self.user.as_ref()
    }

    /// The settings' cache folder.
    pub fn cache_root(&self) -> Option<&AssetRoot> {
        self.cache.as_ref()
    }

    /// The cache folder in the Seihai folder.
    pub fn game_cache_root(&self) -> Option<&AssetRoot> {
        self.game_cache.as_ref()
    }

    /// Both cache folders, in search order.
    pub fn cache_roots(&self) -> impl Iterator<Item = &AssetRoot> {
        self.game_cache.iter().chain(self.cache.iter())
    }

    pub fn builtin_root(&self) -> Option<&AssetRoot> {
        self.builtin.as_ref()
    }

    /// Where a download is written first: the Seihai folder's cache, else
    /// the settings' cache, else the user's folder (a resolver made by
    /// [`new`](Self::new), as before). Never the built-in set; `None` when
    /// only the built-in set exists. With both caches the settings' one is
    /// written only as a fallback or a copy (see `DownloadStore::store`).
    pub fn download_root(&self) -> Option<&AssetRoot> {
        self.game_cache.as_ref().or(self.cache.as_ref()).or(self.user.as_ref())
    }

    pub fn profile(&self) -> &'static VersionProfile {
        self.profile
    }

    /// The folders to search for `kind`, in order, each with its layer.
    fn roots(&self, levels: bool) -> Vec<(Layer, &AssetRoot)> {
        let mut v = Vec::with_capacity(4);
        for r in self.cache_roots() {
            v.push((Layer::Cache, r));
        }
        if let Some(r) = &self.user {
            v.push((Layer::Assets, r));
        }
        if let Some(r) = &self.builtin {
            if !levels || self.builtin_levels {
                v.push((Layer::Builtin, r));
            }
        }
        v
    }

    /// Which layer a path found by this resolver is in.
    pub fn layer_of(&self, path: &Path) -> Option<Layer> {
        let mut best: Option<(Layer, usize)> = None;
        for (layer, r) in [(Layer::Cache, &self.game_cache), (Layer::Cache, &self.cache), (Layer::Assets, &self.user), (Layer::Builtin, &self.builtin)] {
            if let Some(r) = r {
                if path.starts_with(&r.root) {
                    let depth = r.root.components().count();
                    if best.map_or(true, |(_, d)| depth > d) {
                        best = Some((layer, depth));
                    }
                }
            }
        }
        best.map(|(l, _)| l)
    }

    // ---- levels ----------------------------------------------------------

    /// Find a level, gmap, `.graal` or `.zelda` file. A name with an extension must be
    /// a format this version opens; a bare name gets the version's level
    /// extensions in turn.
    pub fn level(&self, name: &str) -> Option<PathBuf> {
        self.level_in(name, false)
    }

    /// A level in the server cache folders only (what a server sent), with
    /// no mod-time check: the copy the F5 window offers to open offline.
    pub fn cached_level(&self, name: &str) -> Option<PathBuf> {
        self.level_in(name, true)
    }

    /// [`Self::level`], in the per-server cache only (`cache_only`) or in
    /// every layer.
    fn level_in(&self, name: &str, cache_only: bool) -> Option<PathBuf> {
        let rel = clean_name(name)?;
        let candidates: Vec<String> = match level_ext(&rel) {
            Some(ext) => {
                if !self.opens(ext) {
                    return None;
                }
                vec![rel]
            }
            None => self.profile.level_extensions().iter().map(|e| format!("{rel}{e}")).collect(),
        };
        for (_, r) in self.roots(true).into_iter().filter(|(l, _)| !cache_only || *l == Layer::Cache) {
            let dirs = [r.root.join(DOWNLOADS), r.root.clone(), r.root.join("maps")];
            if let Some(p) = first_in(&dirs, &candidates) {
                return Some(p);
            }
        }
        None
    }

    /// Online, the local copy of a level that may stand in for the
    /// server's: one in this server's cache folder only, and only when its
    /// last-write time is the mod time the server gave for it
    /// (`PLO_LEVELMODTIME`, 39). The real clients draw the server's board
    /// and keep levels only in their web-level cache, one folder per server
    /// (`weblevels/<host>/<level>-<port>.code`) keyed by that mod time
    /// (Graal_analysis `formats/gwebl001-cache.md`). A same-named file of a
    /// client install (its `new1.nw`) is therefore never the server's
    /// level. A download is saved there with the server's time
    /// (`documentation/file-cache.md`), so it matches.
    pub fn level_at_mod_time(&self, name: &str, server_mod_time: Option<u32>) -> Option<PathBuf> {
        let want = server_mod_time?;
        self.level_in(name, true).filter(|p| crate::filecache::mod_time_of(p) == Some(want))
    }

    /// Does this version open files with this level extension (".nw")?
    pub fn opens(&self, ext: &str) -> bool {
        let l = &self.profile.levels;
        match ext.to_ascii_lowercase().as_str() {
            ".nw" => l.nw,
            ".graal" => l.graal,
            ".zelda" => l.zelda,
            ".gmap" => l.gmap,
            _ => false,
        }
    }

    /// The path of a level for loading or saving: the file if it exists,
    /// else where a download of that name would be saved (in the cache
    /// folder, or the user's folder without one). A name that
    /// would leave the folder falls back to the downloads folder joined
    /// with its last component.
    pub fn level_path(&self, name: &str) -> PathBuf {
        self.level(name).unwrap_or_else(|| {
            let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
            match self.download_root().or(self.builtin.as_ref()) {
                Some(r) => r.level(last),
                None => PathBuf::from(DOWNLOADS).join(last),
            }
        })
    }

    // ---- GANIs -----------------------------------------------------------

    /// Find a GANI. Builds without GANIs (`ui.ganis` false) find none. The
    /// `.gani` extension is added when the name has none.
    pub fn gani(&self, name: &str) -> Option<PathBuf> {
        if !self.profile.ui.ganis {
            return None;
        }
        self.gani_in_layers(name, |_| true)
    }

    /// The animation a player pose or a character NPC is drawn with. A
    /// build with GANIs finds it as [`AssetResolver::gani`]. A build without
    /// them (1.33 to 1.39) has its player animations built into the
    /// executable, and the first GANI files (NewMain 1.41) are those
    /// tables written out, so the same stock files stand in for them: only
    /// the names in [`crate::pose::BUILTIN_ANIMS`], and only from the
    /// user's asset folder or Seihai's built-in set, never from what a
    /// server sent (the cache), since such a build downloads no GANIs.
    pub fn pose_gani(&self, name: &str) -> Option<PathBuf> {
        if self.profile.ui.ganis {
            return self.gani(name);
        }
        let stem = name.trim().to_ascii_lowercase();
        let stem = stem.strip_suffix(".gani").unwrap_or(&stem);
        if !crate::pose::BUILTIN_ANIMS.contains(&stem) {
            return None;
        }
        self.gani_in_layers(name, |l| l != Layer::Cache)
    }

    fn gani_in_layers(&self, name: &str, keep: impl Fn(Layer) -> bool) -> Option<PathBuf> {
        let rel = clean_name(name)?;
        let file = if has_ext(&rel) { rel } else { format!("{rel}.gani") };
        for (_, r) in self.roots(false).into_iter().filter(|(l, _)| keep(*l)) {
            let dirs = [
                r.gani_dir(),
                r.root.join(DOWNLOADS).join("ganis"),
                // The bomy (pet) animations sit in their own folder.
                r.root.join(DOWNLOADS).join("bomys"),
                r.root.join(DOWNLOADS),
                r.root.clone(),
            ];
            if let Some(p) = first_in(&dirs, std::slice::from_ref(&file)) {
                return Some(p);
            }
        }
        None
    }

    // ---- images ----------------------------------------------------------

    /// Find an image (tile sheet, NPC image, HUD art, player art, ...).
    pub fn image(&self, name: &str) -> Option<PathBuf> {
        self.image_in("", name)
    }

    /// As [`image`](Self::image), preferring the folder `category`
    /// (`"bodies"`, `"heads"`, ...). An empty category has no preference.
    ///
    /// Name rules: a bare name gets the profile's `image_ext` first, then
    /// `.png`, `.gif`, `.mng`. A `.gif` or `.png` name that is not found
    /// is tried with the other extension: the 2.22 folder ships PNG only,
    /// and a pre-2.1 server names images `.gif`. (Seihai's reading; the
    /// real clients' fallback is not captured.)
    ///
    /// **Player art in a GIF build.** A build whose `image_ext` is `.gif`
    /// (up to 2.05) looks for heads, bodies, swords and shields as `.gif`
    /// only, in every layer, whatever extension the name has. The real
    /// 2.05 rev 2 does the same: given the 2.22 folder (PNG only) it draws
    /// no head, body or shadow, yet the name tag and chat still show
    /// (Graal_analysis `player-mannerisms-live.md`, "Asset gotcha"; lab
    /// check in `documentation/assets.md`). Seihai matches that: there is
    /// no PNG fallback for these.
    pub fn image_in(&self, category: &str, name: &str) -> Option<PathBuf> {
        let rel = clean_name(name)?;
        let gif_only = self.gif_only_player_art(category);
        let candidates = if gif_only { vec![with_ext(&rel, ".gif")] } else { self.image_names(&rel) };
        let cat = if category.is_empty() { None } else { Some(clean_name(category)?) };
        // The built-in .gif player art stands in for a 2.05 install's own
        // pictures. When the assets folder has player art of its own (a
        // `bodies` or `heads` folder, at its root or under `levels/`), a GIF
        // build uses only that folder's: a PNG-only folder shows no figure,
        // as with the real client. An empty or missing folder still gets
        // the built-in art.
        let skip_builtin = gif_only && cat.as_deref().is_some_and(|c| self.user_has_folder(c));
        self.image_pass(cat.as_deref(), &candidates, skip_builtin)
    }

    fn image_pass(&self, cat: Option<&str>, candidates: &[String], skip_builtin: bool) -> Option<PathBuf> {
        for (layer, root) in self.roots(false) {
            if skip_builtin && layer == Layer::Builtin {
                continue;
            }
            let r = &root.root;
            let mut dirs: Vec<PathBuf> = Vec::new();
            if let Some(cat) = cat {
                dirs.push(r.join(cat));
                dirs.push(r.join(DOWNLOADS).join(cat));
            }
            // Root before levels/: the real pics1.png lives at the
            // install root. A levels/ copy (a download, or the built-in
            // sheet copied in) must not hide it.
            dirs.push(r.clone());
            dirs.push(r.join(DOWNLOADS));
            for c in IMAGE_CATEGORIES {
                dirs.push(r.join(DOWNLOADS).join(c));
            }
            // Map pictures named by a `.gmap` (`MAPIMG`).
            dirs.push(r.join("maps"));
            for c in IMAGE_CATEGORIES {
                dirs.push(r.join(c));
            }
            if let Some(p) = first_in(&dirs, candidates) {
                return Some(p);
            }
        }
        None
    }

    /// The assets folder has a `category` folder, at its root or under `levels/`.
    fn user_has_folder(&self, category: &str) -> bool {
        self.user.as_ref().is_some_and(|u| u.root.join(category).is_dir() || u.root.join(DOWNLOADS).join(category).is_dir())
    }

    /// See [`image_in`](Self::image_in): player art of a GIF build.
    fn gif_only_player_art(&self, category: &str) -> bool {
        self.profile.files.image_ext.eq_ignore_ascii_case(".gif") && PLAYER_ART.iter().any(|c| c.eq_ignore_ascii_case(category))
    }

    fn image_names(&self, rel: &str) -> Vec<String> {
        let mut v = Vec::new();
        match ext_of(rel) {
            None => {
                v.push(format!("{rel}{}", self.profile.files.image_ext));
                for e in [".png", ".gif", ".mng"] {
                    let s = format!("{rel}{e}");
                    if !v.contains(&s) {
                        v.push(s);
                    }
                }
            }
            Some(e) => {
                v.push(rel.to_string());
                let stem = &rel[..rel.len() - e.len()];
                match e.to_ascii_lowercase().as_str() {
                    ".gif" => v.push(format!("{stem}.png")),
                    ".png" => v.push(format!("{stem}.gif")),
                    _ => {}
                }
            }
        }
        v
    }

    // ---- sounds and music ------------------------------------------------

    /// The folders sound effects are looked up in, in search order: per layer,
    /// `sounds/` (else `levels/sounds/`) when it exists. The cache's `sounds/`
    /// is always listed (it is made by the first download), so a sound that
    /// arrives later is found.
    pub fn sound_dirs(&self) -> Vec<PathBuf> {
        let mut v = Vec::new();
        for (layer, r) in self.roots(false) {
            match r.sounds_dir() {
                Some(d) => v.push(d),
                None if layer == Layer::Cache => v.push(r.root.join("sounds")),
                None => {}
            }
        }
        v
    }

    /// The first folder of [`sound_dirs`](Self::sound_dirs).
    pub fn sounds_dir(&self) -> Option<PathBuf> {
        self.sound_dirs().into_iter().next()
    }

    /// Find a sound effect. `.wav` is added to a bare name.
    pub fn sound(&self, name: &str) -> Option<PathBuf> {
        let rel = clean_name(name.trim_start_matches('+'))?;
        let file = if has_ext(&rel) { rel } else { format!("{rel}.wav") };
        for (_, root) in self.roots(false) {
            let r = &root.root;
            let dirs = [r.join("sounds"), r.join(DOWNLOADS).join("sounds"), r.join(DOWNLOADS), r.clone()];
            if let Some(p) = first_in(&dirs, std::slice::from_ref(&file)) {
                return Some(p);
            }
        }
        None
    }

    /// Find a music file. `.mid` is added to a bare name (the common case).
    pub fn music(&self, name: &str) -> Option<PathBuf> {
        let rel = clean_name(name)?;
        let file = if has_ext(&rel) { rel } else { format!("{rel}.mid") };
        for (_, root) in self.roots(false) {
            let r = &root.root;
            let dirs = [
                r.join(DOWNLOADS).join("midis"),
                r.join("midis"),
                r.join(DOWNLOADS),
                r.join("sounds"),
                r.clone(),
            ];
            if let Some(p) = first_in(&dirs, std::slice::from_ref(&file)) {
                return Some(p);
            }
        }
        None
    }

    // ---- tile tables -----------------------------------------------------

    /// The tile type table: `tiledefs.txt` (text) or `arrays.dat`.
    ///
    /// The built-in table uses the standard Graal tile layout, the same as
    /// the built-in sheet, so it also serves a user's own `pics1.png` when
    /// their folder has no table (`documentation/default-assets.md`).
    pub fn tile_types(&self) -> Option<(PathBuf, TileTypesFile)> {
        for (_, root) in self.roots(false) {
            let r = &root.root;
            if let Some(p) = first_in(&[r.clone(), r.join(DOWNLOADS)], &["tiledefs.txt".to_string()]) {
                return Some((p, TileTypesFile::Text));
            }
            if let Some(p) = first_in(&[r.clone(), r.join(DOWNLOADS)], &["arrays.dat".to_string()]) {
                return Some((p, TileTypesFile::ArraysDat));
            }
        }
        None
    }

    /// Find one tiledefs file by name in `levels/tiledefs/` (or `tiledefs/`).
    pub fn tiledef(&self, name: &str) -> Option<PathBuf> {
        let rel = clean_name(name)?;
        for (_, root) in self.roots(false) {
            let r = &root.root;
            if let Some(p) = first_in(&[r.join(DOWNLOADS).join("tiledefs"), r.join("tiledefs")], std::slice::from_ref(&rel)) {
                return Some(p);
            }
        }
        None
    }

    /// The `tiledefs*.txt` files in `levels/tiledefs/` (the offline and
    /// online tile-set lists), sorted by name. The first layer that has any
    /// is used, so one set is never a mix of two layers.
    pub fn tiledef_files(&self) -> Vec<PathBuf> {
        for (_, root) in self.roots(false) {
            let dir = root.root.join(DOWNLOADS).join("tiledefs");
            let mut v: Vec<PathBuf> = fs::read_dir(&dir)
                .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect())
                .unwrap_or_default();
            if !v.is_empty() {
                v.sort();
                return v;
            }
        }
        Vec::new()
    }
}

/// Which loader reads the tile type table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileTypesFile {
    Text,
    ArraysDat,
}

// ---- name helpers --------------------------------------------------------

/// Turn a server-supplied name into a safe relative path with `/`
/// separators, or `None` when it would escape the folder or is empty.
pub(crate) fn clean_name(name: &str) -> Option<String> {
    let name = name.trim().trim_matches('"');
    if name.is_empty() || name.contains('\0') {
        return None;
    }
    let norm = name.replace('\\', "/");
    if norm.starts_with('/') {
        return None;
    }
    let mut parts = Vec::new();
    for p in norm.split('/') {
        match p {
            "" | "." => {}
            ".." => return None,
            // A drive letter ("c:") or any other colon.
            p if p.contains(':') => return None,
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// `".nw"`, `".png"`: the extension of the last component, with the dot.
fn ext_of(rel: &str) -> Option<&str> {
    let last = rel.rsplit('/').next().unwrap_or(rel);
    last.rfind('.').filter(|&i| i > 0 && i + 1 < last.len()).map(|i| &last[i..])
}

fn has_ext(rel: &str) -> bool {
    ext_of(rel).is_some()
}

/// A level extension if the name has one of the four Seihai knows.
fn level_ext(rel: &str) -> Option<&str> {
    ext_of(rel).filter(|e| matches!(e.to_ascii_lowercase().as_str(), ".nw" | ".graal" | ".zelda" | ".gmap"))
}

/// The first existing file, trying each directory in order and, inside
/// it, each candidate name in order.
fn first_in(dirs: &[PathBuf], names: &[String]) -> Option<PathBuf> {
    for d in dirs {
        for n in names {
            if let Some(p) = lookup(d, n) {
                return Some(p);
            }
        }
    }
    None
}

/// Find `rel` under `dir`, matching every component ignoring ASCII case.
pub(crate) fn lookup(dir: &Path, rel: &str) -> Option<PathBuf> {
    // Exact case first: one stat, no directory read.
    let direct = dir.join(rel);
    if direct.is_file() {
        return Some(direct);
    }
    let mut cur = dir.to_path_buf();
    let mut comps = rel.split('/').peekable();
    while let Some(c) = comps.next() {
        let last = comps.peek().is_none();
        let found = fs::read_dir(&cur).ok()?.flatten().find(|e| {
            e.file_name().to_str().is_some_and(|n| n.eq_ignore_ascii_case(c))
                && e.path().is_dir() != last
        })?;
        cur = found.path();
    }
    cur.is_file().then_some(cur)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ClientVersion;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("seihai-resolve-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(root: &Path, rel: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"x").unwrap();
    }

    fn resolver(root: &Path, v: ClientVersion) -> AssetResolver {
        AssetResolver::new(AssetRoot::new(root), v.profile())
    }

    #[test]
    fn names_stay_inside_the_folder() {
        assert_eq!(clean_name("a/b.png").as_deref(), Some("a/b.png"));
        assert_eq!(clean_name("a\\b.png").as_deref(), Some("a/b.png"));
        assert_eq!(clean_name("./a.png").as_deref(), Some("a.png"));
        assert_eq!(clean_name("\"quoted.png\"").as_deref(), Some("quoted.png"));
        for bad in ["", "..", "../x", "a/../../x", "/etc/passwd", "c:/x", "c:\\x", "a\0b", "///"] {
            assert_eq!(clean_name(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn online_a_local_level_counts_only_at_the_server_mod_time() {
        let d = tmp("level-mod-time");
        touch(&d, "new1.nw");
        crate::filecache::set_mod_time(&d.join("new1.nw"), 1_000).unwrap();
        let r = AssetResolver::empty(ClientVersion::V222.profile()).with_cache(Some(d.clone()));
        // The server's time for the level: the file is its copy.
        assert_eq!(r.level_at_mod_time("new1.nw", Some(1_000)), Some(d.join("new1.nw")));
        assert_eq!(r.level_at_mod_time("NEW1", Some(1_000)), Some(d.join("new1.nw")));
        // Another time, or none sent yet: the server's board wins.
        assert_eq!(r.level_at_mod_time("new1.nw", Some(1_001)), None);
        assert_eq!(r.level_at_mod_time("new1.nw", None), None);
        // No file at all.
        assert_eq!(r.level_at_mod_time("other.nw", Some(1_000)), None);
    }

    /// The install's own `new1.nw` is not this server's level, even at the
    /// same time: only the server's cache folder counts.
    #[test]
    fn online_levels_come_only_from_the_server_cache() {
        let install = tmp("level-install");
        let cache = tmp("level-cache");
        touch(&install, "new1.nw");
        crate::filecache::set_mod_time(&install.join("new1.nw"), 1_000).unwrap();
        let r = AssetResolver::empty(ClientVersion::V222.profile())
            .with_assets(Some(AssetRoot::new(&install)))
            .with_cache(Some(cache.clone()));
        assert_eq!(r.level("new1.nw"), Some(install.join("new1.nw")), "offline lookups still see it");
        assert_eq!(r.level_at_mod_time("new1.nw", Some(1_000)), None);
        touch(&cache, "levels/new1.nw");
        crate::filecache::set_mod_time(&cache.join("levels/new1.nw"), 1_000).unwrap();
        assert_eq!(r.level_at_mod_time("new1.nw", Some(1_000)), Some(cache.join("levels/new1.nw")));
    }

    /// The F5 window's "open offline" copy: the server cache's file at
    /// any time, never the install's.
    #[test]
    fn cached_level_is_the_server_cache_copy_only() {
        let install = tmp("cached-install");
        let cache = tmp("cached-cache");
        touch(&install, "levels/onlinestartlocal.nw");
        let r = AssetResolver::empty(ClientVersion::V222.profile())
            .with_assets(Some(AssetRoot::new(&install)))
            .with_cache(Some(cache.clone()));
        assert_eq!(r.cached_level("onlinestartlocal.nw"), None);
        touch(&cache, "levels/onlinestartlocal.nw");
        assert_eq!(r.cached_level("onlinestartlocal.nw"), Some(cache.join("levels/onlinestartlocal.nw")));
    }

    #[test]
    fn levels_come_from_root_levels_or_maps_and_ignore_case() {
        let d = tmp("levels");
        touch(&d, "new1.nw");
        touch(&d, "maps/Offline.gmap");
        touch(&d, "levels/saved.nw");
        let r = resolver(&d, ClientVersion::V222);
        assert_eq!(r.level("new1.nw"), Some(d.join("new1.nw")));
        assert_eq!(r.level("NEW1.NW"), Some(d.join("new1.nw")));
        assert_eq!(r.level("offline.gmap"), Some(d.join("maps/Offline.gmap")));
        assert_eq!(r.level("saved.nw"), Some(d.join("levels/saved.nw")));
        // A bare name takes the version's extensions in turn.
        assert_eq!(r.level("new1"), Some(d.join("new1.nw")));
        assert_eq!(r.level("missing.nw"), None);
        // The downloads folder wins over the root.
        touch(&d, "levels/new1.nw");
        assert_eq!(r.level("new1.nw"), Some(d.join("levels/new1.nw")));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn level_formats_follow_the_profile() {
        let d = tmp("levelgate");
        touch(&d, "a.nw");
        touch(&d, "b.graal");
        touch(&d, "c.gmap");
        // 1.39: `.graal` only.
        let old = resolver(&d, ClientVersion::V139);
        assert_eq!(old.level("a.nw"), None);
        assert!(old.level("b.graal").is_some());
        assert_eq!(old.level("c.gmap"), None);
        assert_eq!(old.level("b"), Some(d.join("b.graal")));
        // 2.05 and later: all three.
        for v in [ClientVersion::V205, ClientVersion::V217, ClientVersion::V222, ClientVersion::V231] {
            let r = resolver(&d, v);
            assert!(r.level("a.nw").is_some() && r.level("b.graal").is_some() && r.level("c.gmap").is_some(), "{v:?}");
        }
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn zelda_levels_open_in_every_build_and_bare_names_try_graal_first() {
        let d = tmp("zelda");
        touch(&d, "levels/Level11.Zelda");
        touch(&d, "house1.graal");
        touch(&d, "house1.zelda");
        touch(&d, "only.zelda");
        for p in crate::protocol::PROFILES.iter() {
            let r = AssetResolver::new(AssetRoot::new(&d), p);
            // An explicit name is found whatever case the disk has.
            assert_eq!(r.level("level11.zelda"), Some(d.join("levels/Level11.Zelda")), "{}", p.label);
            assert!(r.opens(".zelda") && r.opens(".ZELDA"), "{}", p.label);
            // A bare name: the build's own order, `.zelda` after `.graal`.
            assert_eq!(r.level("only"), Some(d.join("only.zelda")), "{}", p.label);
            assert_eq!(r.level("house1"), Some(d.join("house1.graal")), "{}", p.label);
            // No swap for a named extension that is missing.
            assert_eq!(r.level("house2.zelda"), None);
        }
        // A build whose profile does not read Z3-V1 would not open it.
        let mut p = ClientVersion::V139.profile().clone();
        p.levels.zelda = false;
        let p: &'static VersionProfile = Box::leak(Box::new(p));
        let r = AssetResolver::new(AssetRoot::new(&d), p);
        assert_eq!(r.level("only.zelda"), None);
        assert_eq!(r.level("only"), None);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn ganis_only_where_the_build_has_them() {
        let d = tmp("gani");
        touch(&d, "levels/ganis/Walk.gani");
        touch(&d, "gani/idle.gani");
        let old = resolver(&d, ClientVersion::V139);
        assert_eq!(old.gani("walk.gani"), None);
        let new = resolver(&d, ClientVersion::V222);
        assert_eq!(new.gani("walk.gani"), Some(d.join("levels/ganis/Walk.gani")));
        assert_eq!(new.gani("walk"), Some(d.join("levels/ganis/Walk.gani")));
        assert_eq!(new.gani("idle.gani"), Some(d.join("gani/idle.gani")));
        assert_eq!(new.gani("../walk.gani"), None);
        let _ = fs::remove_dir_all(&d);
    }

    /// A build without GANIs still draws its built-in player animations
    /// from the stock files of the user's folder, but never from the cache
    /// and never for a name that is not one of its built-in poses.
    #[test]
    fn builds_without_ganis_take_their_built_in_poses_from_local_files() {
        let d = tmp("pose-gani");
        let cache = tmp("pose-gani-cache");
        touch(&d, "levels/ganis/walk.gani");
        touch(&d, "levels/ganis/sword.gani");
        touch(&d, "levels/ganis/lift.gani");
        touch(&d, "levels/ganis/mywalk.gani");
        touch(&cache, "levels/ganis/sword.gani");
        touch(&cache, "levels/ganis/push.gani");
        let r = AssetResolver::new(AssetRoot::new(&d), ClientVersion::V139.profile()).with_cache(Some(cache.clone()));
        assert_eq!(r.gani("walk.gani"), None, "no GANI files as such");
        assert_eq!(r.pose_gani("walk.gani"), Some(d.join("levels/ganis/walk.gani")));
        assert_eq!(r.pose_gani("walk"), Some(d.join("levels/ganis/walk.gani")));
        assert_eq!(r.pose_gani("sword.gani"), Some(d.join("levels/ganis/sword.gani")), "the user's copy, not the server's");
        assert_eq!(r.pose_gani("push.gani"), None, "a server's file is never read");
        assert_eq!(r.pose_gani("lift.gani"), None, "1.39 has no lift pose");
        assert_eq!(r.pose_gani("mywalk.gani"), None, "a server's own animation is not built in");
        // A GANI build reads them as GANIs, cache first.
        let n = AssetResolver::new(AssetRoot::new(&d), ClientVersion::V222.profile()).with_cache(Some(cache.clone()));
        assert_eq!(n.pose_gani("sword.gani"), Some(cache.join("levels/ganis/sword.gani")));
        let _ = fs::remove_dir_all(&d);
        let _ = fs::remove_dir_all(&cache);
    }

    #[test]
    fn images_search_the_category_folders_and_the_extension_rules() {
        let d = tmp("image");
        touch(&d, "pics1.png");
        touch(&d, "levels/bodies/body.png");
        touch(&d, "levels/heads/head0.png");
        touch(&d, "levels/images/door.png");
        touch(&d, "levels/webgifs/Alienpics.png");
        touch(&d, "swords/old.png");
        touch(&d, "levels/ganis/gralats.png");
        touch(&d, "levels/bomys/bomy_idle.gani");
        touch(&d, "maps/map.png");
        let r = resolver(&d, ClientVersion::V222);
        assert_eq!(r.image("pics1.png"), Some(d.join("pics1.png")));
        assert_eq!(r.image_in("bodies", "body.png"), Some(d.join("levels/bodies/body.png")));
        // No category hint: still found by name.
        assert_eq!(r.image("head0.png"), Some(d.join("levels/heads/head0.png")));
        assert_eq!(r.image("door.png"), Some(d.join("levels/images/door.png")));
        assert_eq!(r.image("alienpics.png"), Some(d.join("levels/webgifs/Alienpics.png")));
        // GANI sprite sheets sit beside the GANIs; map pictures in maps/.
        assert_eq!(r.image("gralats.png"), Some(d.join("levels/ganis/gralats.png")));
        assert_eq!(r.image("map.png"), Some(d.join("maps/map.png")));
        assert_eq!(r.gani("bomy_idle.gani"), Some(d.join("levels/bomys/bomy_idle.gani")));
        // Old layout: the category folder straight under the root.
        assert_eq!(r.image("old.png"), Some(d.join("swords/old.png")));
        // A bare name gets the profile's extension, then the others.
        assert_eq!(r.image("door"), Some(d.join("levels/images/door.png")));
        // `.gif` asked, `.png` shipped.
        assert_eq!(r.image("door.gif"), Some(d.join("levels/images/door.png")));
        assert_eq!(r.image("nothere.png"), None);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_gif_build_reads_player_art_as_gif_only() {
        let d = tmp("gifart");
        touch(&d, "levels/heads/head0.png");
        touch(&d, "levels/heads/head0.gif");
        touch(&d, "levels/bodies/body.png");
        touch(&d, "levels/images/door.png");
        touch(&d, "levels/images/door.gif");
        let old = resolver(&d, ClientVersion::V205);
        // 2.05 asks for head0.png and gets the GIF, as the real client reads.
        assert_eq!(old.image_in("heads", "head0.png"), Some(d.join("levels/heads/head0.gif")));
        // No GIF body: the real 2.05 draws none, and finds none here either.
        assert_eq!(old.image_in("bodies", "body.png"), None);
        // Only player art: other images keep the name's extension.
        assert_eq!(old.image_in("images", "door.png"), Some(d.join("levels/images/door.png")));
        let new = resolver(&d, ClientVersion::V222);
        assert_eq!(new.image_in("heads", "head0.png"), Some(d.join("levels/heads/head0.png")));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_gif_build_takes_built_in_player_art_only_without_its_own() {
        let d = tmp("gifbuiltin");
        let ours = d.join("builtin");
        touch(&ours, "bodies/body.gif");
        // A PNG-only folder with its own bodies: no body at all, as the real 2.05.
        touch(&d, "pngonly/levels/bodies/body.png");
        let p = AssetResolver::new(AssetRoot::new(&d.join("pngonly")), ClientVersion::V205.profile()).with_builtin(Some(ours.clone()));
        assert_eq!(p.image_in("bodies", "body.png"), None);
        // A folder without player art: the built-in GIF.
        fs::create_dir_all(d.join("empty")).unwrap();
        let e = AssetResolver::new(AssetRoot::new(&d.join("empty")), ClientVersion::V205.profile()).with_builtin(Some(ours.clone()));
        assert_eq!(e.image_in("bodies", "body.png"), Some(ours.join("bodies/body.gif")));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn bare_image_names_prefer_the_profile_extension() {
        let d = tmp("imgext");
        touch(&d, "pic.gif");
        touch(&d, "pic.png");
        let old = resolver(&d, ClientVersion::V205);
        assert_eq!(old.profile().files.image_ext, ".gif");
        assert_eq!(old.image("pic"), Some(d.join("pic.gif")));
        let new = resolver(&d, ClientVersion::V222);
        assert_eq!(new.profile().files.image_ext, ".png");
        assert_eq!(new.image("pic"), Some(d.join("pic.png")));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn sounds_and_music() {
        let d = tmp("sound");
        touch(&d, "sounds/Arrow.wav");
        touch(&d, "levels/midis/INTRO.MID");
        let r = resolver(&d, ClientVersion::V222);
        assert_eq!(r.sound("arrow.wav"), Some(d.join("sounds/Arrow.wav")));
        assert_eq!(r.sound("arrow"), Some(d.join("sounds/Arrow.wav")));
        // The loop prefix of a play request is not part of the name.
        assert_eq!(r.sound("+arrow.wav"), Some(d.join("sounds/Arrow.wav")));
        assert_eq!(r.sound("nothere.wav"), None);
        assert_eq!(r.music("intro.mid"), Some(d.join("levels/midis/INTRO.MID")));
        assert_eq!(r.music("intro"), Some(d.join("levels/midis/INTRO.MID")));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn tile_tables_and_tiledefs() {
        let d = tmp("tiles");
        touch(&d, "arrays.dat");
        touch(&d, "levels/tiledefs/b.txt");
        touch(&d, "levels/tiledefs/a.txt");
        let r = resolver(&d, ClientVersion::V222);
        assert_eq!(r.tile_types(), Some((d.join("arrays.dat"), TileTypesFile::ArraysDat)));
        touch(&d, "tiledefs.txt");
        assert_eq!(r.tile_types(), Some((d.join("tiledefs.txt"), TileTypesFile::Text)));
        let files = r.tiledef_files();
        assert_eq!(files, vec![d.join("levels/tiledefs/a.txt"), d.join("levels/tiledefs/b.txt")]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_that_arrives_later_is_found() {
        let d = tmp("late");
        let r = resolver(&d, ClientVersion::V222);
        assert_eq!(r.image("late.png"), None);
        touch(&d, "levels/late.png");
        assert_eq!(r.image("late.png"), Some(d.join("levels/late.png")));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn level_path_falls_back_to_the_downloads_folder() {
        let d = tmp("lp");
        let r = resolver(&d, ClientVersion::V222);
        assert_eq!(r.level_path("x.nw"), d.join("levels/x.nw"));
        // A name that tries to leave the folder keeps only its last part.
        assert_eq!(r.level_path("../../etc/x.nw"), d.join("levels/x.nw"));
        let _ = fs::remove_dir_all(&d);
    }
}
