//! Which files a client version needs from the data folder, and whether
//! Seihai's loaders find and read them.
//!
//! The 2.22 data folder is the reference. [`expected_files`] lists, per
//! version profile, the files that build uses; [`check_folder`] resolves each
//! one with [`AssetResolver`] and runs the matching loader on it. The report
//! backs `seihai --check-assets` and the `#[ignore]` tests that point at the
//! real folder.
//!
//! What counts as "needed" is Seihai's list, built from the derived notes
//! (`client-internals/early-clients.md`, `sound-and-music.md`,
//! `client-assets.md`, `versions-133-139.md`) and from what the renderer
//! opens. A file that the notes do not name is not listed.

use std::fs;
use std::path::{Path, PathBuf};

use super::{
    read_png_rgba, AssetResolver, AssetRoot, Gani, Level, TileTypes, TileTypesFile,
};
use crate::gmap::Gmap;
use crate::protocol::{ClientVersion, VersionProfile};
use crate::sound::decode_wav;

/// What kind of file a [`Need`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeedKind {
    Image,
    /// An image looked up with a folder hint (`bodies`, `heads`).
    ImageIn(&'static str),
    Level,
    Gani,
    Sound,
    Music,
    TileTypes,
    TileDefs,
}

/// One file a version uses.
#[derive(Debug, Clone)]
pub struct Need {
    pub group: &'static str,
    pub name: String,
    pub kind: NeedKind,
    /// Missing it stops Seihai from running or drawing the basics.
    pub required: bool,
    /// Where the expectation comes from.
    pub why: &'static str,
}

/// The outcome for one [`Need`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Found and read by its loader; the text says what was read.
    Ok(String),
    /// Not found in the data folder.
    Missing,
    /// Found, but the loader failed.
    BadFile(String),
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub need: Need,
    pub path: Option<PathBuf>,
    pub status: Status,
}

impl Finding {
    pub fn is_ok(&self) -> bool {
        matches!(self.status, Status::Ok(_))
    }
}

/// Sound effects named in the 1.39 binary (`early-clients.md`).
const SOUNDS_139: [&str; 24] = [
    "arrow", "arrowon", "axe", "bomb", "chest", "compudead", "crush", "dead", "extra", "fire", "frog", "goal", "item",
    "item2", "jump", "nextpage", "put", "sign", "steps", "stonemove", "sword", "swordon", "thunder", "water",
];

/// Effects the 2.05 client knows by name (`sound-and-music.md`). The later
/// builds are taken as the same list (inferred).
const SOUNDS_205: [&str; 15] = [
    "arrow", "axe", "bomb", "crush", "extra", "fire", "horse2", "item", "lift", "lift2", "put", "steps", "sword",
    "thunder", "water",
];

fn version_index(v: ClientVersion) -> usize {
    ClientVersion::all_versions().iter().position(|&x| x == v).unwrap_or(0)
}

/// The files this version uses from a data folder.
pub fn expected_files(profile: &VersionProfile) -> Vec<Need> {
    let mut v = Vec::new();
    let mut need = |group, name: &str, kind, required, why| {
        v.push(Need { group, name: name.to_string(), kind, required, why })
    };

    need("tiles", "pics1.png", NeedKind::Image, true, "tile sheet, every build");
    need("tiles", "tile types", NeedKind::TileTypes, true, "collision table (arrays.dat or tiledefs.txt)");
    need("hud", "state.png", NeedKind::Image, true, "HUD art, every build");
    need("hud", "sprites.png", NeedKind::Image, true, "sprite sheet, every build");
    need("player", "body.png", NeedKind::ImageIn("bodies"), true, "player body art");
    need("player", "head0.png", NeedKind::ImageIn("heads"), true, "player head art");

    if profile.ui.ganis {
        need("gani", "walk.gani", NeedKind::Gani, true, "walk cues and frames");
        need("gani", "swim.gani", NeedKind::Gani, true, "swim cues and frames");
        need("gani", "idle.gani", NeedKind::Gani, false, "default animation");
        need("gani", "sword.gani", NeedKind::Gani, false, "sword swing");
    }

    let (names, why): (&[&str], &'static str) = if profile.ui.ganis {
        (&SOUNDS_205, "2.05 named effects (later builds inferred)")
    } else {
        (&SOUNDS_139, "1.39 named effects (builds without GANIs)")
    };
    for n in names {
        need("sound", &format!("{n}.wav"), NeedKind::Sound, false, why);
    }
    need("music", "intro.mid", NeedKind::Music, false, "a music file; every build plays MIDI");

    if profile.levels.nw {
        need("level", "new1.nw", NeedKind::Level, true, "sample GLEVNW01 level");
    }
    if profile.levels.gmap {
        need("level", "offlinetutorial.gmap", NeedKind::Level, false, "sample GRMAP001 map");
    }
    if version_index(profile.version) >= version_index(ClientVersion::V217) {
        // 2.22 reads tiledefsOffline.txt (offline-tutorial-205.md); 2.17 and
        // 2.31 are taken as the same (inferred). 2.05 uses its embedded
        // tutorial and no such file is claimed for it.
        need("tiledefs", "tiledefsOffline.txt", NeedKind::TileDefs, false, "offline tile sheets [2.22]");
    }
    v
}

/// Count files with an extension under a folder tree (for the notes).
fn count_ext(dir: &Path, ext: &str, depth: u32) -> usize {
    let Ok(rd) = fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| {
            let p = e.path();
            if p.is_dir() {
                if depth > 0 {
                    count_ext(&p, ext, depth - 1)
                } else {
                    0
                }
            } else {
                usize::from(p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case(ext)))
            }
        })
        .sum()
}

/// A resolved report for one version.
#[derive(Debug, Clone)]
pub struct Report {
    pub findings: Vec<Finding>,
    /// Facts about the folder that no single file shows (formats with no
    /// sample level, tile sheets of a different shape).
    pub notes: Vec<String>,
}

/// Resolve and load every expected file of `profile` in `root`.
pub fn check_folder(root: &AssetRoot, profile: &'static VersionProfile) -> Report {
    let r = AssetResolver::new(root.clone(), profile);
    let findings: Vec<Finding> = expected_files(profile).into_iter().map(|n| check_one(&r, n)).collect();
    let mut notes = Vec::new();
    if profile.levels.graal {
        let n = count_ext(&root.root, "graal", 3);
        if n == 0 {
            notes.push(
                "the folder holds no .graal level files, so the .graal loader is covered by synthetic tests only"
                    .to_string(),
            );
        }
    }
    let mut findings = findings;
    if profile.levels.zelda || profile.levels.graal {
        scan_binary_levels(root, profile, &mut findings, &mut notes);
    }
    if !profile.levels.nw {
        notes.push(".nw levels are not opened by this build (profile levels.nw = false)".to_string());
    }
    if !profile.ui.ganis {
        notes.push("this build has no GANIs (profile ui.ganis = false); none are looked up".to_string());
    }
    Report { findings, notes }
}

/// Every `.zelda` and `.graal` file under the folder (six levels deep) is
/// read with the loader its magic selects, whatever its extension says. A
/// Z3-V1 file must reach the end of the file with nothing odd on the way;
/// one that does not is a finding. A `GR-V1` file only has to load. A note
/// gives the counts. Only the first 20 failures are listed.
fn scan_binary_levels(root: &AssetRoot, profile: &VersionProfile, findings: &mut Vec<Finding>, notes: &mut Vec<String>) {
    use super::zelda::{is_zelda, ZeldaLevel, ZeldaMagic};
    let mut files = Vec::new();
    let mut stack = vec![(root.root.clone(), 6u32)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth > 0 {
                    stack.push((p, depth - 1));
                }
            } else if p
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("zelda") || x.eq_ignore_ascii_case("graal"))
            {
                files.push(p);
            }
        }
    }
    files.sort();
    let (mut z103, mut z104, mut zother, mut gr, mut clean, mut other) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let mut failed = 0usize;
    for p in &files {
        let Ok(data) = fs::read(p) else { continue };
        let verdict: Result<(), String> = if is_zelda(&data) {
            match ZeldaMagic::from_bytes(&data) {
                Some(ZeldaMagic::V103) => z103 += 1,
                Some(ZeldaMagic::V104) => z104 += 1,
                _ => zother += 1,
            }
            ZeldaLevel::parse(&data).and_then(|z| match z.layout.anomalies.first() {
                Some(a) => Err(a.clone()),
                None if z.layout.unread_tail > 0 => Err(format!("{} bytes follow the signs", z.layout.unread_tail)),
                None => {
                    clean += 1;
                    Ok(())
                }
            })
        } else if data.starts_with(b"GR-V1") {
            gr += 1;
            Level::parse_binary_with_layout(&data).map(|_| ())
        } else {
            other += 1;
            Err("not a Z3-V1 or GR-V1 level".to_string())
        };
        if let Err(e) = verdict {
            failed += 1;
            if failed <= 20 {
                let name = p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                findings.push(Finding {
                    need: Need { group: "level scan", name, kind: NeedKind::Level, required: true, why: "binary level read to its end" },
                    path: Some(p.clone()),
                    status: Status::BadFile(e),
                });
            }
        }
    }
    let zelda_files = z103 + z104 + zother;
    notes.push(format!(
        "level scan ({}): {} .zelda/.graal files read by magic: {} Z3-V1.03, {} Z3-V1.04, {} other Z3-V1, {} GR-V1, {} unrecognised; {} Z3-V1 files parse to the end with no anomaly, {} files failed{}",
        profile.label,
        files.len(),
        z103,
        z104,
        zother,
        gr,
        other,
        clean,
        failed,
        if failed > 20 { " (first 20 listed)" } else { "" }
    ));
    if zelda_files == 0 && profile.levels.zelda {
        notes.push("the folder holds no Z3-V1 (.zelda) level, so that loader is covered by synthetic tests only".to_string());
    }
}

fn check_one(r: &AssetResolver, need: Need) -> Finding {
    let path = match need.kind {
        NeedKind::Image => r.image(&need.name),
        NeedKind::ImageIn(dir) => r.image_in(dir, &need.name),
        NeedKind::Level => r.level(&need.name),
        NeedKind::Gani => r.gani(&need.name),
        NeedKind::Sound => r.sound(&need.name),
        NeedKind::Music => r.music(&need.name),
        NeedKind::TileTypes => r.tile_types().map(|(p, _)| p),
        NeedKind::TileDefs => r.tiledef(&need.name),
    };
    let Some(p) = path else {
        return Finding { need, path: None, status: Status::Missing };
    };
    let status = match load(r, need.kind, &p) {
        Ok(s) => Status::Ok(s),
        Err(e) => Status::BadFile(e),
    };
    Finding { need, path: Some(p), status }
}

/// Run the loader that matches the kind of file.
fn load(r: &AssetResolver, kind: NeedKind, p: &Path) -> Result<String, String> {
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match kind {
        NeedKind::Image | NeedKind::ImageIn(_) => {
            let data = fs::read(p).map_err(|e| e.to_string())?;
            if ext == "png" || ext == "gif" {
                let (w, h, _) = read_png_rgba(&data).map_err(|e| e.to_string())?;
                Ok(format!("{ext} {w}x{h}"))
            } else {
                // MNG has no decoder here.
                Ok(format!("{ext}, {} bytes, not decoded", data.len()))
            }
        }
        NeedKind::Level => {
            if ext == "gmap" {
                let text = fs::read_to_string(p).map_err(|e| e.to_string())?;
                let g = Gmap::parse(&text).ok_or("not a GRMAP001 map")?;
                Ok(format!("gmap {}x{}", g.width, g.height))
            } else {
                let l = Level::load(p)?;
                Ok(format!(
                    "{} board runs, {} npcs, {} links, {} signs, {} chests",
                    l.boards.len(),
                    l.npcs.len(),
                    l.links.len(),
                    l.signs.len(),
                    l.chests.len()
                ))
            }
        }
        NeedKind::Gani => {
            let g = Gani::load(p)?;
            Ok(format!("{} steps", g.frames.len()))
        }
        NeedKind::Sound => {
            let bytes = fs::read(p).map_err(|e| e.to_string())?;
            let pcm = decode_wav(&bytes)?;
            Ok(format!("wav, {} Hz", pcm.rate))
        }
        NeedKind::Music => {
            let bytes = fs::read(p).map_err(|e| e.to_string())?;
            if ext == "mid" && !bytes.starts_with(b"MThd") {
                return Err("no MThd header".into());
            }
            Ok(format!("{ext}, {} bytes, not decoded", bytes.len()))
        }
        NeedKind::TileTypes => match r.tile_types().map(|(_, k)| k) {
            Some(TileTypesFile::Text) => TileTypes::load(p).map(|_| "tiledefs.txt".to_string()),
            Some(TileTypesFile::ArraysDat) => TileTypes::load_arrays(p).map(|_| "arrays.dat".to_string()),
            None => Err("no table".into()),
        },
        NeedKind::TileDefs => {
            let v = super::load_tiledefs(p)?;
            if v.is_empty() {
                return Err("no entries".into());
            }
            Ok(format!("{} entries", v.len()))
        }
    }
}

/// A plain-text report.
pub fn format_report(label: &str, rep: &Report) -> String {
    let mut out = String::new();
    let total = rep.findings.len();
    let ok = rep.findings.iter().filter(|f| f.is_ok()).count();
    let req_missing = rep.findings.iter().filter(|f| f.need.required && !f.is_ok()).count();
    out.push_str(&format!(
        "== {label}: {ok}/{total} found and read, {req_missing} required missing ==\n"
    ));
    let mut group = "";
    for f in &rep.findings {
        if f.need.group != group {
            group = f.need.group;
            out.push_str(&format!("[{group}]\n"));
        }
        let (mark, detail) = match &f.status {
            Status::Ok(d) => ("ok     ", d.clone()),
            Status::Missing => ("MISSING", f.need.why.to_string()),
            Status::BadFile(e) => ("BAD    ", e.clone()),
        };
        let req = if f.need.required { "*" } else { " " };
        out.push_str(&format!("  {mark}{req} {:<24} {detail}\n", f.need.name));
    }
    for n in &rep.notes {
        out.push_str(&format!("  note: {n}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: ClientVersion) -> &'static VersionProfile {
        v.profile()
    }

    fn names(v: ClientVersion, group: &str) -> Vec<String> {
        expected_files(p(v)).into_iter().filter(|n| n.group == group).map(|n| n.name).collect()
    }

    #[test]
    fn expectations_follow_the_profile() {
        // 1.39: no GANIs, no .nw, no gmap, the 1.39 sound list.
        assert!(names(ClientVersion::V139, "gani").is_empty());
        assert!(names(ClientVersion::V139, "level").is_empty());
        assert_eq!(names(ClientVersion::V139, "sound").len(), 24);
        assert!(names(ClientVersion::V139, "tiledefs").is_empty());
        // 2.05: GANIs, .nw and .gmap, the 15-effect list, no tiledefs file
        // (tiledefsOffline.txt is read from 2.17 on).
        assert!(names(ClientVersion::V205, "tiledefs").is_empty());
        assert_eq!(names(ClientVersion::V205, "sound").len(), 15);
        assert_eq!(names(ClientVersion::V205, "gani"), ["walk.gani", "swim.gani", "idle.gani", "sword.gani"]);
        assert_eq!(names(ClientVersion::V205, "level"), ["new1.nw", "offlinetutorial.gmap"]);
        for v in [ClientVersion::V217, ClientVersion::V222, ClientVersion::V231] {
            assert_eq!(names(v, "tiledefs"), ["tiledefsOffline.txt"], "{v:?}");
            assert_eq!(names(v, "level"), ["new1.nw", "offlinetutorial.gmap"], "{v:?}");
        }
    }

    #[test]
    fn every_build_lists_the_core_files() {
        for &v in ClientVersion::all_versions() {
            let core: Vec<String> = expected_files(p(v)).into_iter().filter(|n| n.required).map(|n| n.name).collect();
            for want in ["pics1.png", "state.png", "sprites.png", "body.png", "head0.png", "tile types"] {
                assert!(core.iter().any(|c| c == want), "{v:?} lacks {want}");
            }
        }
    }

    /// A tiny synthetic folder in the 2.22 layout, no game assets.
    fn synthetic(tag: &str) -> PathBuf {
        use crate::assets::write_png_rgb;
        let d = std::env::temp_dir().join(format!("seihai-cov-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let put = |rel: &str, bytes: &[u8]| {
            let f = d.join(rel);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, bytes).unwrap();
        };
        let mut png = Vec::new();
        write_png_rgb(&mut png, 4, 4, &[0u8; 4 * 4 * 3]).unwrap();
        for n in ["pics1.png", "state.png", "sprites.png", "levels/bodies/body.png", "levels/heads/head0.png"] {
            put(n, &png);
        }
        put("tiledefs.txt", format!("{}\n", vec!["0"; 128].join(",")).repeat(32).as_bytes());
        put("new1.nw", b"GLEVNW01\nBOARD 0 0 4 0 AAAAAAAA\n");
        put("maps/offlinetutorial.gmap", b"GRMAP001\nWIDTH 1\nHEIGHT 1\nLEVELNAMES\n\"a.nw\"\nLEVELNAMESEND\n");
        put("levels/ganis/walk.gani", b"ANI\nSPRITE 0 body.png 0 0 1 1 x\nANIEND\n");
        put("levels/tiledefs/tiledefsOffline.txt", b"picso.png,offlinetutorial,1,0,0\n");
        put("levels/midis/intro.mid", b"MThd\0\0\0\x06");
        d
    }

    #[test]
    fn synthetic_folder_reports_found_and_missing() {
        let d = synthetic("rep");
        let rep = check_folder(&AssetRoot::new(&d), p(ClientVersion::V222));
        let get = |n: &str| rep.findings.iter().find(|f| f.need.name == n).unwrap();
        assert!(get("pics1.png").is_ok(), "{:?}", get("pics1.png").status);
        assert!(get("body.png").is_ok());
        assert!(get("new1.nw").is_ok(), "{:?}", get("new1.nw").status);
        assert!(get("offlinetutorial.gmap").is_ok(), "{:?}", get("offlinetutorial.gmap").status);
        assert!(get("walk.gani").is_ok(), "{:?}", get("walk.gani").status);
        assert!(get("tile types").is_ok(), "{:?}", get("tile types").status);
        assert!(get("tiledefsOffline.txt").is_ok());
        assert!(get("intro.mid").is_ok());
        assert_eq!(get("swim.gani").status, Status::Missing);
        assert_eq!(get("arrow.wav").status, Status::Missing);
        let text = format_report("2.22", &rep);
        assert!(text.contains("MISSING") && text.contains("[tiles]"), "{text}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_broken_file_is_reported_not_hidden() {
        let d = synthetic("bad");
        fs::write(d.join("new1.nw"), b"not a level").unwrap();
        let rep = check_folder(&AssetRoot::new(&d), p(ClientVersion::V222));
        let f = rep.findings.iter().find(|f| f.need.name == "new1.nw").unwrap();
        assert!(matches!(f.status, Status::BadFile(_)), "{:?}", f.status);
        let _ = fs::remove_dir_all(&d);
    }

    /// A GIF build's player art is decoded and its size reported, and a
    /// broken GIF is a bad file, not a pass.
    #[test]
    fn gif_player_art_is_decoded() {
        let d = synthetic("gif");
        let body = fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/default/bodies/body.gif")).unwrap();
        fs::write(d.join("levels/bodies/body.gif"), &body).unwrap();
        fs::write(d.join("levels/heads/head0.gif"), b"GIF89a broken").unwrap();
        let rep = check_folder(&AssetRoot::new(&d), p(ClientVersion::V205));
        let get = |n: &str| rep.findings.iter().find(|f| f.need.name.starts_with(n)).unwrap();
        assert_eq!(get("body").status, Status::Ok("gif 128x720".into()));
        assert!(matches!(get("head0").status, Status::BadFile(_)), "{:?}", get("head0").status);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn old_build_ignores_what_it_cannot_open() {
        let d = synthetic("old");
        let rep = check_folder(&AssetRoot::new(&d), p(ClientVersion::V139));
        // No GANI or level needs; the pics1.png/hud/player ones are found.
        assert!(rep.findings.iter().all(|f| f.need.group != "gani" && f.need.group != "level"));
        assert!(rep.notes.iter().any(|n| n.contains(".nw")));
        assert!(rep.notes.iter().any(|n| n.contains("no .graal")));
        let _ = fs::remove_dir_all(&d);
    }

    /// The real 2.22 folder: set `SEIHAI_DATA_222` to its path. Every
    /// main version must find and read its required files in it.
    #[test]
    #[ignore = "needs the 2.22 data folder; set SEIHAI_DATA_222"]
    fn real_222_folder_serves_every_main_version() {
        let Some(dir) = std::env::var_os("SEIHAI_DATA_222") else {
            panic!("set SEIHAI_DATA_222 to the 'Graal Reborn 2.22.0' folder");
        };
        let root = AssetRoot::new(dir);
        let mut summary = String::new();
        for &v in ClientVersion::main_versions() {
            let rep = check_folder(&root, v.profile());
            let text = format_report(v.label(), &rep);
            summary.push_str(&text);
            for f in &rep.findings {
                assert!(
                    !(f.need.required && !f.is_ok()),
                    "{}: required {} -> {:?}\n{text}",
                    v.label(),
                    f.need.name,
                    f.status
                );
                assert!(!matches!(f.status, Status::BadFile(_)), "{}: {} -> {:?}", v.label(), f.need.name, f.status);
            }
        }
        eprintln!("{summary}");
    }
    /// Every file of a kind in the real folder loads, and every one is found
    /// by name through the resolver. Prints the counts.
    #[test]
    #[ignore = "needs the 2.22 data folder; set SEIHAI_DATA_222"]
    fn real_222_folder_every_file_of_each_kind_loads() {
        let Some(dir) = std::env::var_os("SEIHAI_DATA_222") else {
            panic!("set SEIHAI_DATA_222 to the 'Graal Reborn 2.22.0' folder");
        };
        fn walk(d: &Path, out: &mut Vec<PathBuf>) {
            if let Ok(rd) = fs::read_dir(d) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        // Windows tools and DLL trees, not game data.
                        let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        if n != "RC2" && n != "generators" {
                            walk(&p, out);
                        }
                    } else {
                        out.push(p);
                    }
                }
            }
        }
        let root = AssetRoot::new(&dir);
        let r = AssetResolver::new(root.clone(), ClientVersion::V222.profile());
        let mut files = Vec::new();
        walk(Path::new(&dir), &mut files);
        let mut bad: Vec<String> = Vec::new();
        let (mut png, mut gani, mut wav, mut nw, mut lost) = (0, 0, 0, 0, 0);
        let mut sheets = std::collections::BTreeMap::new();
        let mut missing_snd = std::collections::BTreeMap::new();
        for f in &files {
            let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            let name = f.file_name().and_then(|n| n.to_str()).unwrap().to_string();
            let res: Result<(), String> = match ext.as_str() {
                "png" => {
                    png += 1;
                    fs::read(f).map_err(|e| e.to_string()).and_then(|b| read_png_rgba(&b).map(|_| ()).map_err(|e| e.to_string()))
                }
                "gani" => {
                    gani += 1;
                    Gani::load(f).map(|g| {
                        for sp in &g.sprites {
                            if r.image(&sp.sheet).is_none() {
                                *sheets.entry(sp.sheet.to_ascii_lowercase()).or_insert(0usize) += 1;
                            }
                        }
                        for fr in &g.frames {
                            for snd in &fr.sounds {
                                if r.sound(&snd.file).is_none() {
                                    *missing_snd.entry(snd.file.to_ascii_lowercase()).or_insert(0usize) += 1;
                                }
                            }
                        }
                    })
                }
                "wav" => {
                    wav += 1;
                    fs::read(f).map_err(|e| e.to_string()).and_then(|b| decode_wav(&b).map(|_| ()))
                }
                "nw" => {
                    nw += 1;
                    Level::load(f).map(|_| ())
                }
                _ => Ok(()),
            };
            if let Err(e) = res {
                bad.push(format!("{}: {e}", f.display()));
            }
            // Found by name, wherever it sits.
            let found = match ext.as_str() {
                "png" => r.image(&name),
                "gani" => r.gani(&name),
                "wav" => r.sound(&name),
                "nw" => r.level(&name),
                _ => continue,
            };
            if found.is_none() {
                lost += 1;
                bad.push(format!("not found by name: {name}"));
            }
        }
        eprintln!("png {png}, gani {gani}, wav {wav}, nw {nw}; not loadable or not found: {}", bad.len());
        eprintln!("GANI sprite sheets with no file: {sheets:?}");
        eprintln!("GANI sounds with no file: {missing_snd:?}");
        for b in &bad {
            eprintln!("  {b}");
        }
        assert_eq!(lost, 0, "files the resolver cannot find by name");
    }


    fn z3_bytes(computers: usize) -> Vec<u8> {
        use super::super::zelda::{Computer, ZeldaLevel, ZeldaMagic};
        ZeldaLevel {
            magic: ZeldaMagic::V104,
            board: vec![3; 4096],
            links: vec![],
            computers: (0..computers).map(|i| Computer { x: i as u8, y: 2, kind: 1, extra: b"\\\\".to_vec() }).collect(),
            signs: vec![],
            layout: Default::default(),
        }
        .to_bytes()
        .unwrap()
    }

    /// Z3-V1 files are found by magic whatever their extension, a clean one
    /// is counted, a damaged one is a finding, and the note gives the counts.
    #[test]
    fn level_scan_reads_zelda_levels_by_magic() {
        let d = std::env::temp_dir().join(format!("seihai-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("levels/sub")).unwrap();
        fs::write(d.join("levels/a.zelda"), z3_bytes(2)).unwrap();
        fs::write(d.join("levels/sub/b.graal"), z3_bytes(0)).unwrap(); // Z3-V1 in a .graal name
        let mut cut = z3_bytes(1);
        cut.truncate(cut.len() - 3); // the terminator is cut
        fs::write(d.join("levels/bad.zelda"), cut).unwrap();
        fs::write(d.join("levels/junk.graal"), b"not a level at all").unwrap();
        let rep = check_folder(&AssetRoot::new(&d), ClientVersion::V139.profile());
        let bad: Vec<_> = rep.findings.iter().filter(|f| f.need.group == "level scan").collect();
        assert_eq!(bad.len(), 2, "{bad:?}");
        assert!(bad.iter().all(|f| matches!(f.status, Status::BadFile(_)) && f.need.required));
        let note = rep.notes.iter().find(|n| n.starts_with("level scan")).expect("a scan note");
        assert!(note.contains("4 .zelda/.graal files") && note.contains("0 Z3-V1.03, 3 Z3-V1.04"), "{note}");
        assert!(note.contains("2 Z3-V1 files parse to the end with no anomaly, 2 files failed"), "{note}");
        let _ = fs::remove_dir_all(&d);
        // A folder with none says so.
        let e = std::env::temp_dir().join(format!("seihai-scan-empty-{}", std::process::id()));
        fs::create_dir_all(&e).unwrap();
        let rep = check_folder(&AssetRoot::new(&e), ClientVersion::V139.profile());
        assert!(rep.notes.iter().any(|n| n.contains("no Z3-V1 (.zelda) level")), "{:?}", rep.notes);
        let _ = fs::remove_dir_all(&e);
    }

    /// The old client folders: `SEIHAI_DRIVE_CLIENTS` is the folder holding
    /// the build folders. Every Z3-V1 file in each build that ships levels
    /// must read to its end. Prints the scan note per build.
    #[test]
    #[ignore = "needs SEIHAI_DRIVE_CLIENTS=<folder with the old client builds>"]
    fn real_drive_builds_level_scan_is_clean() {
        let Some(dir) = std::env::var_os("SEIHAI_DRIVE_CLIENTS") else {
            panic!("set SEIHAI_DRIVE_CLIENTS to the folder that holds the client builds");
        };
        let mut seen = 0;
        for e in fs::read_dir(&dir).unwrap().flatten() {
            if !e.path().is_dir() {
                continue;
            }
            let rep = check_folder(&AssetRoot::new(e.path()), ClientVersion::V139.profile());
            let bad: Vec<_> = rep.findings.iter().filter(|f| f.need.group == "level scan").collect();
            let note = rep.notes.iter().find(|n| n.starts_with("level scan")).cloned().unwrap_or_default();
            if note.contains(" 0 .zelda/.graal files") {
                continue;
            }
            seen += 1;
            eprintln!("{}: {note}", e.file_name().to_string_lossy());
            assert!(bad.is_empty(), "{}: {} files do not read clean", e.file_name().to_string_lossy(), bad.len());
        }
        assert!(seen >= 10, "builds with levels: {seen}");
    }
}
