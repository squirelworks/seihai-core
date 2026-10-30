//! The sound rules: what plays, how loud, which voices fit.
//!
//! Everything here comes from the 2.05 client (`client-internals/
//! sound-and-music.md` in Graal_analysis) and is limited to what that note
//! documents:
//!
//! - A leading `+` on the name means looped, and is stripped.
//! - `.wav` is an effect. Any other extension is music, which this client
//!   does not play: it is logged once and skipped.
//! - At most 8 effects play at once. A ninth is dropped.
//! - Pan is `(sound x - listener x) * 150`, clamped to +/-10000, and
//!   `reversestereo` negates it.
//! - Decoded effects are cached by file name.
//! - A missing file is logged once. If it arrives later it plays on the
//!   next trigger.
//!
//! The volume falloff with distance is not documented beyond "grows with
//! distance on both axes", so every effect plays at full volume.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::wav::{decode_wav, Pcm};
use super::{PlayRequest, SoundOut};

/// Effects that can play at once.
pub const MAX_VOICES: usize = 8;
/// DirectSound pan units per tile of horizontal distance.
pub const PAN_PER_TILE: f32 = 150.0;
/// Rescan the sound folder for a missing file at most this often.
const RESCAN_MS: u64 = 1000;

/// A sound to play: the name as written and where it happens, in level
/// tiles.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundEvent {
    pub file: String,
    pub x: f32,
    pub y: f32,
}

/// What [`SoundEngine::play`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Handed to the backend.
    Played,
    /// Sound is switched off (`--no-sound` or `sound=0`).
    Off,
    /// No such file in the sound folder.
    Missing,
    /// The file is there but is not a WAV this decoder reads.
    Undecodable,
    /// Not a `.wav`: music, which is not played.
    NotAnEffect,
    /// Eight voices are busy.
    VoiceCap,
}

/// The name to ask the server for when a sound is missing: the file as the
/// script or GANI wrote it, without the `+` loop prefix. 2.05 asks by this
/// raw name, the path included.
pub fn wanted_name(file: &str) -> String {
    let file = file.trim();
    file.strip_prefix('+').unwrap_or(file).trim().to_string()
}

/// Pan for a sound at `sound_x` heard from `listener_x`.
pub fn pan_for(sound_x: f32, listener_x: f32, reverse_stereo: bool) -> i32 {
    let pan = ((sound_x - listener_x) * PAN_PER_TILE).round().clamp(-10000.0, 10000.0) as i32;
    if reverse_stereo {
        -pan
    } else {
        pan
    }
}

struct Voice {
    name: String,
    /// When a one-shot ends. `None` for a loop, which runs until stopped.
    end_ms: Option<u64>,
}

pub struct SoundEngine {
    out: Box<dyn SoundOut>,
    /// Folders to look in, first match wins (cache, user folder, built-in).
    dirs: Vec<PathBuf>,
    enabled: bool,
    pub reverse_stereo: bool,
    /// Lower-cased file name to path, from the last folder scan.
    index: HashMap<String, PathBuf>,
    last_scan_ms: Option<u64>,
    cache: HashMap<String, Option<Arc<Pcm>>>,
    voices: Vec<Voice>,
    /// Every distinct warning, in order. Each is printed once.
    pub warnings: Vec<String>,
    warned: HashSet<String>,
}

impl SoundEngine {
    pub fn new(out: Box<dyn SoundOut>, dir: Option<PathBuf>, enabled: bool) -> Self {
        Self::with_dirs(out, dir.into_iter().collect(), enabled)
    }

    /// An engine that looks for a sound in each folder in turn: the first
    /// folder that has the file wins.
    pub fn with_dirs(out: Box<dyn SoundOut>, dirs: Vec<PathBuf>, enabled: bool) -> Self {
        Self {
            out,
            dirs,
            enabled,
            reverse_stereo: false,
            index: HashMap::new(),
            last_scan_ms: None,
            cache: HashMap::new(),
            voices: Vec::new(),
            warnings: Vec::new(),
            warned: HashSet::new(),
        }
    }

    /// Use other folders (another server's cache after a server warp). Every
    /// decoded sound and the folder index are dropped.
    pub fn set_dirs(&mut self, dirs: Vec<PathBuf>) {
        self.dirs = dirs;
        self.index.clear();
        self.last_scan_ms = None;
        self.cache.clear();
    }

    /// Forget the decoded copy of a sound (a new file of that name was
    /// saved), so the next play reads the file again.
    pub fn forget(&mut self, name: &str) {
        let (name, _) = Self::normalize(name);
        self.cache.remove(&name);
    }

    /// An engine that plays nothing.
    pub fn off() -> Self {
        Self::new(Box::new(super::NullSound), None, false)
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn backend(&self) -> &'static str {
        self.out.kind()
    }

    /// Voices still sounding at `now_ms`.
    pub fn active_voices(&mut self, now_ms: u64) -> usize {
        self.prune(now_ms);
        self.voices.len()
    }

    fn prune(&mut self, now_ms: u64) {
        self.voices.retain(|v| v.end_ms.map_or(true, |e| e > now_ms));
    }

    fn warn_once(&mut self, msg: String) {
        if self.warned.insert(msg.clone()) {
            eprintln!("sound: {msg}");
            self.warnings.push(msg);
        }
    }

    /// Strip the loop prefix and the path, and lower-case: the cache key.
    fn normalize(file: &str) -> (String, bool) {
        let file = file.trim();
        let (looped, file) = match file.strip_prefix('+') {
            Some(rest) => (true, rest),
            None => (false, file),
        };
        let base = file.rsplit(['/', '\\']).next().unwrap_or(file);
        (base.to_ascii_lowercase(), looped)
    }

    /// Play `ev` as heard from `listener` (level tiles) at `now_ms`.
    pub fn play(&mut self, ev: &SoundEvent, listener: (f32, f32), now_ms: u64) -> Outcome {
        if !self.enabled {
            return Outcome::Off;
        }
        let (name, looped) = Self::normalize(&ev.file);
        if name.is_empty() {
            return Outcome::Missing;
        }
        if !name.ends_with(".wav") {
            self.warn_once(format!("music skipped: {name}"));
            return Outcome::NotAnEffect;
        }
        self.prune(now_ms);
        if self.voices.len() >= MAX_VOICES {
            return Outcome::VoiceCap;
        }
        let pcm = match self.load(&name, now_ms) {
            Ok(p) => p,
            Err(o) => return o,
        };
        let req = PlayRequest {
            name: name.clone(),
            pcm: pcm.clone(),
            pan: pan_for(ev.x, listener.0, self.reverse_stereo),
            looped,
        };
        self.out.play(&req);
        self.voices.push(Voice { name, end_ms: if looped { None } else { Some(now_ms + pcm.duration_ms().max(1)) } });
        Outcome::Played
    }

    /// `stopsound`: stop a sound by file name (the path is ignored).
    pub fn stop(&mut self, file: &str) {
        let (name, _) = Self::normalize(file);
        self.out.stop(&name);
        self.voices.retain(|v| v.name != name);
    }

    /// Leaving a level stops every looped effect.
    pub fn stop_all_loops(&mut self) {
        self.out.stop_all_loops();
        self.voices.retain(|v| v.end_ms.is_some());
    }

    fn load(&mut self, name: &str, now_ms: u64) -> Result<Arc<Pcm>, Outcome> {
        if let Some(hit) = self.cache.get(name) {
            return hit.clone().ok_or(Outcome::Undecodable);
        }
        let Some(path) = self.find(name, now_ms) else {
            self.warn_once(format!("missing sound {name}"));
            return Err(Outcome::Missing);
        };
        let decoded = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| decode_wav(&b));
        match decoded {
            Ok(pcm) => {
                let pcm = Arc::new(pcm);
                self.cache.insert(name.to_string(), Some(pcm.clone()));
                Ok(pcm)
            }
            Err(e) => {
                self.warn_once(format!("cannot read sound {name}: {e}"));
                self.cache.insert(name.to_string(), None);
                Err(Outcome::Undecodable)
            }
        }
    }

    /// Find a sound by lower-cased name, ignoring case in the folder. The
    /// folders are tried in order and the first that has it wins.
    fn find(&mut self, name: &str, now_ms: u64) -> Option<PathBuf> {
        if self.dirs.is_empty() {
            return None;
        }
        for d in &self.dirs {
            let direct = d.join(name);
            if direct.is_file() {
                return Some(direct);
            }
        }
        if let Some(p) = self.index.get(name) {
            if p.is_file() {
                return Some(p.clone());
            }
        }
        // A file that arrived since the last scan, or one whose case
        // differs on a case-sensitive file system.
        let due = self.last_scan_ms.map_or(true, |t| now_ms >= t + RESCAN_MS);
        if due {
            self.last_scan_ms = Some(now_ms);
            let mut index = HashMap::new();
            // Later folders first, so an earlier folder's file replaces theirs.
            for d in self.dirs.iter().rev() {
                index.extend(scan(d));
            }
            self.index = index;
        }
        self.index.get(name).cloned()
    }
}

fn scan(dir: &Path) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if let Some(n) = e.file_name().to_str() {
                map.insert(n.to_ascii_lowercase(), e.path());
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::super::wav::test_wav;
    use super::super::{Call, NullSound, RecordingSound};
    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("seihai-sound-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn ev(file: &str) -> SoundEvent {
        SoundEvent { file: file.into(), x: 10.0 + 1.5, y: 5.0 + 2.0 }
    }

    const L: (f32, f32) = (11.5, 7.0);

    fn engine(tag: &str, files: &[(&str, u32, usize)]) -> (SoundEngine, RecordingSound, PathBuf) {
        let dir = tempdir(tag);
        for (n, rate, len) in files {
            std::fs::write(dir.join(n), test_wav(*rate, *len)).unwrap();
        }
        let rec = RecordingSound::default();
        (SoundEngine::new(Box::new(rec.clone()), Some(dir.clone()), true), rec, dir)
    }

    #[test]
    fn plays_a_wav_once_with_centre_pan_at_the_listener() {
        let (mut e, rec, _) = engine("one", &[("steps.wav", 8000, 800)]);
        assert_eq!(e.play(&ev("steps.wav"), L, 0), Outcome::Played);
        assert_eq!(rec.log(), vec![Call::Play { name: "steps.wav".into(), pan: 0, looped: false }]);
    }

    #[test]
    fn pan_is_150_per_tile_clamped_and_reversible() {
        assert_eq!(pan_for(12.5, 11.5, false), 150);
        assert_eq!(pan_for(10.5, 11.5, false), -150);
        assert_eq!(pan_for(1000.0, 0.0, false), 10000);
        assert_eq!(pan_for(-1000.0, 0.0, false), -10000);
        assert_eq!(pan_for(12.5, 11.5, true), -150);
        assert_eq!(pan_for(11.5, 11.5, true), 0);
    }

    #[test]
    fn a_sound_to_the_right_pans_right() {
        let (mut e, rec, _) = engine("pan", &[("a.wav", 8000, 80)]);
        let right = SoundEvent { file: "a.wav".into(), x: 21.5, y: 7.0 };
        e.play(&right, L, 0);
        e.reverse_stereo = true;
        e.play(&right, L, 1000);
        assert_eq!(
            rec.log(),
            vec![
                Call::Play { name: "a.wav".into(), pan: 1500, looped: false },
                Call::Play { name: "a.wav".into(), pan: -1500, looped: false },
            ]
        );
    }

    #[test]
    fn plus_prefix_loops_and_is_stripped() {
        let (mut e, rec, _) = engine("loop", &[("rain.wav", 8000, 80)]);
        assert_eq!(e.play(&ev("+Rain.WAV"), L, 0), Outcome::Played);
        assert_eq!(rec.log(), vec![Call::Play { name: "rain.wav".into(), pan: 0, looped: true }]);
        // A loop never ends by itself.
        assert_eq!(e.active_voices(10_000_000), 1);
        e.stop_all_loops();
        assert_eq!(e.active_voices(10_000_000), 0);
        assert_eq!(rec.log().last(), Some(&Call::StopAllLoops));
    }

    #[test]
    fn stop_ignores_path_and_case() {
        let (mut e, rec, _) = engine("stop", &[("rain.wav", 8000, 80)]);
        e.play(&ev("+rain.wav"), L, 0);
        e.stop("sounds/RAIN.wav");
        assert_eq!(e.active_voices(0), 0);
        assert_eq!(rec.log().last(), Some(&Call::Stop("rain.wav".into())));
    }

    #[test]
    fn file_lookup_ignores_case_and_path() {
        let (mut e, _, _) = engine("case", &[("Steps2.WAV", 8000, 80)]);
        assert_eq!(e.play(&ev("steps2.wav"), L, 0), Outcome::Played);
        assert_eq!(e.play(&ev("levels\\sounds\\STEPS2.wav"), L, 1000), Outcome::Played);
    }

    #[test]
    fn missing_file_is_logged_once_and_plays_nothing() {
        let (mut e, rec, _) = engine("miss", &[]);
        assert_eq!(e.play(&ev("nothere.wav"), L, 0), Outcome::Missing);
        assert_eq!(e.play(&ev("nothere.wav"), L, 5), Outcome::Missing);
        assert_eq!(e.play(&ev("nothere.wav"), L, 5000), Outcome::Missing);
        assert!(rec.log().is_empty());
        assert_eq!(e.warnings, vec!["missing sound nothere.wav".to_string()]);
    }

    #[test]
    fn no_folder_at_all_is_just_missing() {
        let mut e = SoundEngine::new(Box::new(NullSound), None, true);
        assert_eq!(e.play(&ev("steps.wav"), L, 0), Outcome::Missing);
        assert_eq!(e.warnings.len(), 1);
    }

    #[test]
    fn a_file_that_arrives_later_plays_on_the_next_trigger() {
        let (mut e, rec, dir) = engine("late", &[]);
        assert_eq!(e.play(&ev("late.wav"), L, 0), Outcome::Missing);
        std::fs::write(dir.join("late.wav"), test_wav(8000, 80)).unwrap();
        assert_eq!(e.play(&ev("late.wav"), L, 10), Outcome::Played);
        assert_eq!(rec.log().len(), 1);
    }

    #[test]
    fn undecodable_file_is_logged_once() {
        let (mut e, rec, dir) = engine("bad", &[]);
        std::fs::write(dir.join("bad.wav"), b"definitely not riff").unwrap();
        assert_eq!(e.play(&ev("bad.wav"), L, 0), Outcome::Undecodable);
        assert_eq!(e.play(&ev("bad.wav"), L, 1), Outcome::Undecodable);
        assert!(rec.log().is_empty());
        assert_eq!(e.warnings.len(), 1);
        assert!(e.warnings[0].starts_with("cannot read sound bad.wav"));
    }

    #[test]
    fn music_is_skipped_with_one_note() {
        let (mut e, rec, _) = engine("music", &[]);
        assert_eq!(e.play(&ev("song.mid"), L, 0), Outcome::NotAnEffect);
        assert_eq!(e.play(&ev("song.mid"), L, 1), Outcome::NotAnEffect);
        assert!(rec.log().is_empty());
        assert_eq!(e.warnings, vec!["music skipped: song.mid".to_string()]);
    }

    #[test]
    fn a_ninth_voice_is_dropped_until_one_ends() {
        // 1 s effects.
        let (mut e, rec, _) = engine("cap", &[("s.wav", 8000, 8000)]);
        for i in 0..MAX_VOICES {
            assert_eq!(e.play(&ev("s.wav"), L, i as u64), Outcome::Played, "{i}");
        }
        assert_eq!(e.play(&ev("s.wav"), L, 100), Outcome::VoiceCap);
        assert_eq!(rec.log().len(), MAX_VOICES);
        assert_eq!(e.active_voices(100), 8);
        // The first one ended at 1000 ms.
        assert_eq!(e.play(&ev("s.wav"), L, 1000), Outcome::Played);
        assert_eq!(e.active_voices(1000), 8);
    }

    #[test]
    fn off_engine_touches_nothing() {
        let (mut e, rec, _) = engine("off", &[("s.wav", 8000, 80)]);
        e.enabled = false;
        assert_eq!(e.play(&ev("s.wav"), L, 0), Outcome::Off);
        assert!(rec.log().is_empty() && e.warnings.is_empty());
        let mut e = SoundEngine::off();
        assert!(!e.is_enabled());
        assert_eq!(e.play(&ev("steps.wav"), L, 0), Outcome::Off);
        assert_eq!(e.backend(), "null");
    }

    #[test]
    fn null_backend_accepts_everything() {
        let dir = tempdir("null");
        std::fs::write(dir.join("a.wav"), test_wav(8000, 80)).unwrap();
        let mut e = SoundEngine::new(Box::new(NullSound), Some(dir), true);
        assert_eq!(e.play(&ev("a.wav"), L, 0), Outcome::Played);
        e.stop("a.wav");
        e.stop_all_loops();
        assert_eq!(e.backend(), "null");
    }

    #[test]
    fn wanted_name_drops_only_the_loop_prefix() {
        assert_eq!(wanted_name("+thunder.wav"), "thunder.wav");
        assert_eq!(wanted_name(" Sounds/Wolf.WAV "), "Sounds/Wolf.WAV");
        assert_eq!(wanted_name("+"), "");
    }
}
