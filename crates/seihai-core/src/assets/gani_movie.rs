//! GraalShop's movie mode: the `GANI0002` timeline of actors and keyframes.
//!
//! The grammar is from the save and load routines of GraalShop (Graal_analysis
//! `tools/graalshop.md`, "Movie mode file grammar", and `formats/gani.txt`):
//!
//! ```text
//! GANI0002
//! SPRITE …              (the sprite table, read by `Gani::parse`)
//! FRAMES <count>
//! LOOP …                (header lines, read by `Gani::parse`)
//! MOVIE
//!   ACTOR <CHAR|SPRITE|SOUND|TEXT> <name>
//!     FRAME <n> <key>=<value>,<key>=<value>,…
//!   ACTOREND
//!
//! MOVIEEND
//! ```
//!
//! This module reads and writes the `FRAMES` line and the `MOVIE` block and
//! answers what an actor looks like at a timeline frame. The sprite table and
//! the other header lines stay with [`super::Gani`]. Values are kept as the
//! file spells them: how GraalShop writes booleans and indexed keys
//! (`param1` or `param 1`), and the unit of `dx` / `dy`, are not known
//! (no saved `GANI0002` file has been found), so nothing here rewrites them.
//! [`truthy`] reads a boolean in any of the usual spellings.

/// The four actor kinds GraalShop offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActorKind {
    /// A player character: `dir`, `ani`, `head`, `body`, colours and so on.
    Char,
    /// One sprite from the gani's sprite table (`sprite`).
    Sprite,
    /// A sound cue (`file`).
    Sound,
    /// A line of text with font, size, colour and style keys.
    Text,
}

impl ActorKind {
    /// The word the file uses.
    pub fn word(self) -> &'static str {
        match self {
            Self::Char => "CHAR",
            Self::Sprite => "SPRITE",
            Self::Sound => "SOUND",
            Self::Text => "TEXT",
        }
    }

    /// Read the word, any case.
    pub fn from_word(word: &str) -> Option<Self> {
        [Self::Char, Self::Sprite, Self::Sound, Self::Text]
            .into_iter()
            .find(|k| k.word().eq_ignore_ascii_case(word))
    }
}

/// `FRAME n key=value,…`: the keys set at timeline frame `frame`, in file
/// order. A key appears once.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Keyframe {
    pub frame: u32,
    pub values: Vec<(String, String)>,
}

impl Keyframe {
    /// The value of `key` here, if this keyframe sets it.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// One `ACTOR … ACTOREND` block. Keyframes are kept in frame order, one per
/// frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Actor {
    pub kind: ActorKind,
    pub name: String,
    pub keyframes: Vec<Keyframe>,
}

/// Keys that move in a straight line between keyframes; every other key
/// holds its last value.
pub const INTERPOLATED: [&str; 2] = ["dx", "dy"];

impl Actor {
    pub fn new(kind: ActorKind, name: &str) -> Self {
        Self {
            kind,
            name: name.to_string(),
            keyframes: Vec::new(),
        }
    }

    /// The actor at timeline frame `t`: for each key, the value of the last
    /// keyframe at or before `t` that sets it. `dx` and `dy` move linearly
    /// towards the next keyframe that sets them, when both values are
    /// numbers. Keys are in the order they were first set.
    pub fn state_at(&self, t: u32) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for kf in self.keyframes.iter().filter(|k| k.frame <= t) {
            for (k, v) in &kf.values {
                match out.iter_mut().find(|(ok, _)| ok == k) {
                    Some(slot) => slot.1 = v.clone(),
                    None => out.push((k.clone(), v.clone())),
                }
            }
        }
        for (k, v) in &mut out {
            if INTERPOLATED.contains(&k.as_str()) {
                if let Some(x) = self.interpolate(k, t) {
                    *v = number(x);
                }
            }
        }
        out
    }

    /// `key` at frame `t` as a number, between the keyframes around `t`.
    fn interpolate(&self, key: &str, t: u32) -> Option<f64> {
        let before = self
            .keyframes
            .iter()
            .rev()
            .find(|k| k.frame <= t && k.get(key).is_some())?;
        let a: f64 = before.get(key)?.trim().parse().ok()?;
        let Some(after) = self
            .keyframes
            .iter()
            .find(|k| k.frame > t && k.get(key).is_some())
        else {
            return Some(a);
        };
        let Ok(b) = after.get(key)?.trim().parse::<f64>() else {
            return Some(a);
        };
        let span = f64::from(after.frame - before.frame);
        Some(a + (b - a) * f64::from(t - before.frame) / span)
    }

    /// Set `key` at frame `frame` (making the keyframe if needed), or with
    /// `None` delete it there, as GraalShop's edit box does with a piece
    /// that has no `=value`. A keyframe left with no keys is removed.
    pub fn set(&mut self, frame: u32, key: &str, value: Option<&str>) {
        let i = match self.keyframes.iter().position(|k| k.frame >= frame) {
            Some(i) if self.keyframes[i].frame == frame => i,
            Some(i) if value.is_some() => {
                self.keyframes.insert(
                    i,
                    Keyframe {
                        frame,
                        values: Vec::new(),
                    },
                );
                i
            }
            None if value.is_some() => {
                self.keyframes.push(Keyframe {
                    frame,
                    values: Vec::new(),
                });
                self.keyframes.len() - 1
            }
            _ => return,
        };
        let kf = &mut self.keyframes[i];
        match value {
            Some(v) => match kf.values.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = v.to_string(),
                None => kf.values.push((key.to_string(), v.to_string())),
            },
            None => kf.values.retain(|(k, _)| k != key),
        }
        if kf.values.is_empty() {
            self.keyframes.remove(i);
        }
    }
}

/// A `GANI0002` movie: the timeline length and its actors in file order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GaniMovie {
    /// `FRAMES n`: how many frames the timeline runs.
    pub frames: u32,
    pub actors: Vec<Actor>,
}

/// Does the text start with the movie-mode magic `GANI0002`?
pub fn is_movie(text: &str) -> bool {
    text.trim_start_matches('\u{feff}').starts_with("GANI0002")
}

/// A boolean value as a file may spell it: `1`, `true`, `yes`, `on` (any
/// case) are true; anything else is false.
pub fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn number(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        let s = format!("{x:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

impl GaniMovie {
    /// Read the `FRAMES` line and the `MOVIE` block of a gani's text.
    /// `Ok(None)` when the text has no `MOVIE` block. A `MOVIE` block with
    /// `SINGLEDIRECTION`, which GraalShop never writes in movie mode, is an
    /// error, and so is an unknown actor kind or a block left open.
    pub fn parse(text: &str) -> Result<Option<Self>, String> {
        let mut movie = GaniMovie::default();
        let (mut in_movie, mut seen, mut single) = (false, false, false);
        let mut actor: Option<Actor> = None;
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            let at = |m: &str| format!("line {}: {m}", n + 1);
            let mut words = line.split_whitespace();
            let first = words.next().unwrap_or("");
            if !in_movie {
                match first {
                    "FRAMES" => {
                        let v = words.next().unwrap_or("");
                        movie.frames = v.parse().map_err(|_| at("FRAMES needs a count"))?;
                    }
                    "SINGLEDIRECTION" => single = true,
                    "MOVIE" => {
                        in_movie = true;
                        seen = true;
                    }
                    _ => {}
                }
                continue;
            }
            match first {
                "" => {}
                "MOVIEEND" => {
                    if actor.is_some() {
                        return Err(at("MOVIEEND inside an ACTOR"));
                    }
                    in_movie = false;
                }
                "ACTOR" => {
                    if actor.is_some() {
                        return Err(at("ACTOR inside an ACTOR"));
                    }
                    let kind = words.next().unwrap_or("");
                    let kind = ActorKind::from_word(kind)
                        .ok_or_else(|| at(&format!("unknown actor kind {kind:?}")))?;
                    let name = words.collect::<Vec<_>>().join(" ");
                    actor = Some(Actor::new(kind, &name));
                }
                "ACTOREND" => {
                    let a = actor.take().ok_or_else(|| at("ACTOREND with no ACTOR"))?;
                    movie.actors.push(a);
                }
                "FRAME" => {
                    let a = actor.as_mut().ok_or_else(|| at("FRAME outside an ACTOR"))?;
                    let rest = line["FRAME".len()..].trim_start();
                    let (num, pieces) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                    let frame: u32 = num.parse().map_err(|_| at("FRAME needs a number"))?;
                    // Each piece is key=value; a bare key deletes it.
                    for piece in pieces.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                        match piece.split_once('=') {
                            Some((k, v)) => a.set(frame, k.trim(), Some(v.trim())),
                            None => a.set(frame, piece, None),
                        }
                    }
                }
                _ if first.starts_with("//") => {}
                _ => return Err(at(&format!("unexpected {first:?} in MOVIE"))),
            }
        }
        if in_movie {
            return Err("MOVIE has no MOVIEEND".into());
        }
        if !seen {
            return Ok(None);
        }
        if single {
            return Err("SINGLEDIRECTION is not allowed in movie mode".into());
        }
        Ok(Some(movie))
    }

    /// The `FRAMES` line, with its line end.
    pub fn frames_line(&self) -> String {
        format!("FRAMES {}\n", self.frames)
    }

    /// The `MOVIE` block as GraalShop writes it: two-space indent for
    /// `ACTOR`, four for `FRAME`, a blank line after each actor.
    pub fn write_block(&self) -> String {
        let mut out = String::from("MOVIE\n");
        for a in &self.actors {
            out.push_str(&format!("  ACTOR {} {}\n", a.kind.word(), a.name));
            for kf in &a.keyframes {
                let pieces: Vec<String> =
                    kf.values.iter().map(|(k, v)| format!("{k}={v}")).collect();
                out.push_str(&format!("    FRAME {} {}\n", kf.frame, pieces.join(",")));
            }
            out.push_str("  ACTOREND\n\n");
        }
        out.push_str("MOVIEEND\n");
        out
    }

    /// The timeline frame shown `step` frames after the start: wrapped when
    /// the gani loops, else held on the last frame.
    pub fn frame_at(&self, step: u32, looped: bool) -> u32 {
        match self.frames {
            0 => 0,
            n if looped => step % n,
            n => step.min(n - 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "GANI0002
SPRITE 0 SPRITES 0 0 32 32 rock
FRAMES 20
LOOP
MOVIE
  ACTOR CHAR Bob
    FRAME 0 dx=0,dy=2,dir=2,ani=walk
    FRAME 10 dx=5,chat=Hi there
  ACTOREND

  ACTOR SPRITE stone
    FRAME 4 sprite=0,dx=1, ,visible=1
  ACTOREND

MOVIEEND
";

    #[test]
    fn reads_actors_and_keyframes() {
        let m = GaniMovie::parse(FILE).unwrap().unwrap();
        assert_eq!(m.frames, 20);
        assert_eq!(m.actors.len(), 2);
        let bob = &m.actors[0];
        assert_eq!((bob.kind, bob.name.as_str()), (ActorKind::Char, "Bob"));
        assert_eq!(bob.keyframes[1].get("chat"), Some("Hi there"));
        let stone = &m.actors[1];
        assert_eq!(stone.keyframes[0].values.len(), 3, "empty piece dropped");
        assert!(truthy(stone.keyframes[0].get("visible").unwrap()));
        assert_eq!(GaniMovie::parse("GANI0001\nANI\nANIEND\n").unwrap(), None);
        // The sprite table and header of the same file stay with Gani.
        let g = super::super::Gani::parse(FILE).unwrap();
        assert_eq!((g.sprites.len(), g.frames.len(), g.looped), (1, 0, true));
    }

    #[test]
    fn holds_values_and_moves_dx_dy() {
        let m = GaniMovie::parse(FILE).unwrap().unwrap();
        let bob = &m.actors[0];
        let at = |t| bob.state_at(t);
        let get = |s: &[(String, String)], k: &str| {
            s.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone())
        };
        let s = at(4);
        assert_eq!(get(&s, "dx").as_deref(), Some("2"));
        assert_eq!(get(&s, "dy").as_deref(), Some("2"), "no later dy: held");
        assert_eq!(get(&s, "ani").as_deref(), Some("walk"));
        assert_eq!(get(&s, "chat"), None);
        let s = at(15);
        assert_eq!(get(&s, "dx").as_deref(), Some("5"));
        assert_eq!(get(&s, "chat").as_deref(), Some("Hi there"));
        assert_eq!(get(&at(3), "dx").as_deref(), Some("1.5"));
        assert!(m.actors[1].state_at(3).is_empty(), "before its first key");
        assert_eq!(m.frame_at(25, true), 5);
        assert_eq!(m.frame_at(25, false), 19);
    }

    #[test]
    fn edits_delete_keys_and_empty_keyframes() {
        let mut a = Actor::new(ActorKind::Text, "title");
        a.set(5, "text", Some("Hello"));
        a.set(2, "color", Some("#ff0000"));
        a.set(5, "bold", Some("1"));
        assert_eq!(
            a.keyframes.iter().map(|k| k.frame).collect::<Vec<_>>(),
            [2, 5]
        );
        a.set(2, "color", None);
        assert_eq!(a.keyframes.len(), 1, "empty keyframe removed");
        a.set(9, "text", None);
        assert_eq!(a.keyframes.len(), 1, "deleting nothing adds nothing");
        // A bare key in the file deletes it from that keyframe.
        let m = GaniMovie::parse(
            "MOVIE\nACTOR SOUND s\nFRAME 1 file=a.wav,x=1\nFRAME 1 x\nACTOREND\nMOVIEEND\n",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            m.actors[0].keyframes[0].values,
            [("file".into(), "a.wav".into())]
        );
    }

    #[test]
    fn writes_what_it_reads() {
        let m = GaniMovie::parse(FILE).unwrap().unwrap();
        let block = m.write_block();
        assert!(
            block.starts_with("MOVIE\n  ACTOR CHAR Bob\n    FRAME 0 dx=0,dy=2,dir=2,ani=walk\n")
        );
        assert!(block.ends_with("  ACTOREND\n\nMOVIEEND\n"));
        let again = GaniMovie::parse(&format!("{}{block}", m.frames_line()))
            .unwrap()
            .unwrap();
        assert_eq!(again, m);
    }

    #[test]
    fn refuses_what_graalshop_never_writes() {
        assert!(GaniMovie::parse("SINGLEDIRECTION\nMOVIE\nMOVIEEND\n").is_err());
        assert!(GaniMovie::parse("MOVIE\nACTOR DOG rex\nACTOREND\nMOVIEEND\n").is_err());
        assert!(GaniMovie::parse("MOVIE\nACTOR CHAR a\n").is_err());
        assert!(GaniMovie::parse("MOVIE\nFRAME 1 dx=1\nMOVIEEND\n").is_err());
        assert!(is_movie("GANI0002\n") && !is_movie("GANI0001\n"));
    }
}
