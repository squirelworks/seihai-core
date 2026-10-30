//! GANI parser (SPRITE definitions + ANI frames).

use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct GaniSprite {
    pub id: u32,
    pub sheet: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub desc: String,
}

/// One placed sprite inside an animation frame.
#[derive(Debug, Clone)]
pub struct GaniPlacement {
    pub sprite_id: u32,
    pub x: i32,
    pub y: i32,
}

/// A `PLAYSOUND name x y` cue. `x` and `y` are tiles from the player's
/// position, and the usual `1.5 2` is the player's own listening point
/// (pan 0). A cue with no coordinates counts as that point.
#[derive(Debug, Clone, PartialEq)]
pub struct GaniSound {
    pub file: String,
    pub x: f32,
    pub y: f32,
}

/// The player's own listening point, as tiles from its position.
pub const LISTENER_DX: f32 = 1.5;
pub const LISTENER_DY: f32 = 2.0;

/// One animation step. A step holds four direction rows (up, left, down,
/// right), or one row when the file has `SINGLEDIRECTION`; each row is a
/// list of placements. Sounds are the `PLAYSOUND` cues that follow it.
#[derive(Debug, Clone, Default)]
pub struct GaniFrame {
    pub rows: Vec<Vec<GaniPlacement>>,
    pub sounds: Vec<GaniSound>,
    /// `WAIT n` after the step: it is shown for `1 + n` 50 ms ticks. The
    /// real 2.05 client honours it (Graal_analysis
    /// `player-mannerisms-live.md`: push 4 ticks a frame, hurt 31, lift 6).
    pub wait: u32,
}

#[derive(Debug, Clone)]
pub struct Gani {
    pub sprites: Vec<GaniSprite>,
    pub frames: Vec<GaniFrame>,
    pub default_attr1: Option<String>,
    /// `DEFAULTATTR1` … `DEFAULTATTR5`: the picture an `ATTR1` … `ATTR5`
    /// sprite uses while the player has none of their own (index 0 is
    /// ATTR1, the hat in the stock files).
    pub default_attrs: [Option<String>; 5],
    pub default_head: Option<String>,
    pub default_body: Option<String>,
    pub single_direction: bool,
    /// `SETBACKTO name`: the animation the player returns to when this one
    /// has played once (`idle` for the sword swing).
    pub setback_to: Option<String>,
    /// `LOOP`: the animation repeats until another one is chosen.
    pub looped: bool,
    pub raw: String,
}

/// Parse `PLAYSOUND name x y` (the text after the keyword).
fn parse_playsound(rest: &str) -> Option<GaniSound> {
    let toks: Vec<&str> = rest.split_whitespace().collect();
    let (name, x, y) = match toks.as_slice() {
        [] => return None,
        [n] => (n.to_string(), LISTENER_DX, LISTENER_DY),
        [.., xs, ys] if toks.len() >= 3 && xs.parse::<f32>().is_ok() && ys.parse::<f32>().is_ok() => (
            toks[..toks.len() - 2].join(" "),
            xs.parse().unwrap_or(LISTENER_DX),
            ys.parse().unwrap_or(LISTENER_DY),
        ),
        _ => (toks.join(" "), LISTENER_DX, LISTENER_DY),
    };
    Some(GaniSound { file: name, x, y })
}

/// Parse one direction row: `id x y, id x y, ...`.
fn parse_row(line: &str) -> Vec<GaniPlacement> {
    let mut row = Vec::new();
    for chunk in line.split(',') {
        let parts: Vec<&str> = chunk.split_whitespace().collect();
        if parts.len() >= 3 {
            row.push(GaniPlacement {
                sprite_id: parts[0].parse().unwrap_or(0),
                x: parts[1].parse().unwrap_or(0),
                y: parts[2].parse().unwrap_or(0),
            });
        }
    }
    row
}

impl Gani {
    /// Read a GANI file. The bytes need not be UTF-8 (old files carry
    /// Latin-1 text and CRLF line ends).
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        Self::parse(&String::from_utf8_lossy(&bytes))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut sprites = Vec::new();
        let mut frames: Vec<GaniFrame> = Vec::new();
        let mut default_attr1 = None;
        let mut default_attrs: [Option<String>; 5] = Default::default();
        let mut default_head = None;
        let mut default_body = None;
        let mut single_direction = false;
        let mut setback_to = None;
        let mut looped = false;

        let mut in_ani = false;
        // The step being filled, and sounds that came before its first row.
        let mut current = GaniFrame::default();
        let mut early_sounds: Vec<GaniSound> = Vec::new();

        let flush = |current: &mut GaniFrame, frames: &mut Vec<GaniFrame>| {
            if !current.rows.is_empty() {
                frames.push(std::mem::take(current));
            }
        };

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                // A blank line closes a step that is not full yet.
                if in_ani {
                    flush(&mut current, &mut frames);
                }
                continue;
            }
            if line.starts_with("//") {
                continue;
            }

            if line == "ANI" {
                in_ani = true;
                continue;
            }
            if line == "ANIEND" {
                flush(&mut current, &mut frames);
                in_ani = false;
                continue;
            }
            if line == "LOOP" {
                looped = true;
                continue;
            }
            if line == "SINGLEDIRECTION" {
                single_direction = true;
                continue;
            }

            if let Some(rest) = line.strip_prefix("SETBACKTO ") {
                let name = rest.trim();
                if !name.is_empty() {
                    setback_to = Some(name.to_string());
                }
                continue;
            }

            if line.starts_with("SPRITE ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 7 {
                    sprites.push(GaniSprite {
                        id: parts[1].parse().unwrap_or(0),
                        sheet: parts[2].to_string(),
                        x: parts[3].parse().unwrap_or(0),
                        y: parts[4].parse().unwrap_or(0),
                        w: parts[5].parse().unwrap_or(0),
                        h: parts[6].parse().unwrap_or(0),
                        desc: parts.get(7..).map(|p| p.join(" ")).unwrap_or_default(),
                    });
                }
                continue;
            }

            if let Some(n) = line.strip_prefix("DEFAULTATTR").and_then(|r| r.chars().next()).and_then(|c| c.to_digit(10)).filter(|n| (1..=5).contains(n)) {
                if line.as_bytes().get(12) == Some(&b' ') {
                    let v = line[13..].trim().to_string();
                    if n == 1 {
                        default_attr1 = Some(v.clone());
                    }
                    default_attrs[n as usize - 1] = Some(v);
                    continue;
                }
            }
            if line.starts_with("DEFAULTHEAD ") {
                default_head = Some(line[12..].trim().to_string());
                continue;
            }
            if line.starts_with("DEFAULTBODY ") {
                default_body = Some(line[12..].trim().to_string());
                continue;
            }

            if in_ani {
                // `WAIT n` (the stock `hidden.gani` writes `WAIT4`) belongs to
                // the step whose rows came just before it.
                if let Some(n) = line.strip_prefix("WAIT").and_then(|r| r.trim().parse::<u32>().ok()) {
                    if !current.rows.is_empty() {
                        current.wait = n;
                    } else if let Some(last) = frames.last_mut() {
                        last.wait = n;
                    }
                    continue;
                }
                if let Some(rest) = line.strip_prefix("PLAYSOUND ") {
                    // The cue belongs to the step whose rows came just
                    // before it (every shipped file puts it there).
                    if let Some(snd) = parse_playsound(rest) {
                        if !current.rows.is_empty() {
                            current.sounds.push(snd);
                        } else if let Some(last) = frames.last_mut() {
                            last.sounds.push(snd);
                        } else {
                            early_sounds.push(snd);
                        }
                    }
                    continue;
                }

                // Direction row: "id x y, id x y, ..."
                if line.contains(',') || line.split_whitespace().count() >= 3 {
                    if current.rows.is_empty() {
                        current.sounds.append(&mut early_sounds);
                    }
                    current.rows.push(parse_row(line));
                    let full = if single_direction { 1 } else { 4 };
                    if current.rows.len() >= full {
                        flush(&mut current, &mut frames);
                    }
                }
            }
        }
        flush(&mut current, &mut frames);

        Ok(Gani {
            sprites,
            frames,
            default_attr1,
            default_attrs,
            default_head,
            default_body,
            single_direction,
            setback_to,
            looped,
            raw: text.to_string(),
        })
    }

    /// Number of animation frames.
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Frame at `index`, wrapping. None if there are no frames.
    pub fn frame(&self, index: usize) -> Option<&GaniFrame> {
        if self.frames.is_empty() {
            None
        } else {
            self.frames.get(index % self.frames.len())
        }
    }

    /// Ticks step `index` is shown for (`1 + WAIT`).
    pub fn frame_ticks(&self, index: usize) -> usize {
        self.frames.get(index).map_or(1, |f| 1 + f.wait as usize)
    }

    /// Ticks one pass of the animation takes.
    pub fn total_ticks(&self) -> usize {
        (0..self.frames.len()).map(|i| self.frame_ticks(i)).sum()
    }

    /// The step shown `tick` ticks after the animation started, each step
    /// held for its `WAIT`: wrapping round when `wrap` is set, else held on
    /// the last step.
    pub fn frame_at_tick(&self, tick: usize, wrap: bool) -> usize {
        let total = self.total_ticks();
        if total == 0 {
            return 0;
        }
        let mut t = if wrap { tick % total } else { tick.min(total - 1) };
        for i in 0..self.frames.len() {
            let n = self.frame_ticks(i);
            if t < n {
                return i;
            }
            t -= n;
        }
        self.frames.len() - 1
    }

    /// Advance a frame index by one step, wrapping.
    pub fn step(&self, index: usize) -> usize {
        if self.frames.is_empty() {
            0
        } else {
            (index + 1) % self.frames.len()
        }
    }
}

/// Playback state for a loaded GANI.
#[derive(Debug, Clone)]
pub struct GaniPlayer {
    pub gani: Gani,
    pub frame: usize,
}

impl GaniPlayer {
    pub fn new(gani: Gani) -> Self {
        Self { gani, frame: 0 }
    }

    /// Current frame placements.
    pub fn current(&self) -> Option<&GaniFrame> {
        self.gani.frame(self.frame)
    }

    /// Advance one frame. Returns the new index.
    pub fn step(&mut self) -> usize {
        self.frame = self.gani.step(self.frame);
        self.frame
    }

    /// Advance one frame and return the sound cues of the frame entered.
    /// Called once per frame entry, so each cue is returned once per pass.
    pub fn step_sounds(&mut self) -> Vec<GaniSound> {
        self.step();
        self.current().map(|f| f.sounds.clone()).unwrap_or_default()
    }

    /// Reset to the first frame.
    pub fn reset(&mut self) {
        self.frame = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaultattr_lines_fill_their_slots() {
        let g = Gani::parse("DEFAULTATTR1 hat0.png\nDEFAULTATTR3 star.png\nANI\n1 0 0\n1 0 0\n1 0 0\n1 0 0\nANIEND\n").unwrap();
        assert_eq!(g.default_attr1.as_deref(), Some("hat0.png"));
        assert_eq!(g.default_attrs[0].as_deref(), Some("hat0.png"));
        assert_eq!(g.default_attrs[1], None);
        assert_eq!(g.default_attrs[2].as_deref(), Some("star.png"));
    }

    #[test]
    fn wait_holds_a_step_for_more_ticks() {
        // The stock push.gani: five steps, each WAIT 3 (4 ticks).
        let step = "1 0 0\n1 0 0\n1 0 0\n1 0 0\nWAIT 3\n\n";
        let g = Gani::parse(&format!("ANI\n{}ANIEND\n", step.repeat(5))).unwrap();
        assert_eq!(g.total_ticks(), 20);
        let frames: Vec<_> = (0..21).map(|t| g.frame_at_tick(t, true)).collect();
        assert_eq!(&frames[..9], [0, 0, 0, 0, 1, 1, 1, 1, 2]);
        assert_eq!(frames[20], 0, "wraps after 20 ticks");
        // hurt.gani: one step, WAIT 30, held to the end when not wrapping.
        let hurt = Gani::parse("ANI\n1 0 0\n1 0 0\n1 0 0\n1 0 0\nWAIT 30\nANIEND\n").unwrap();
        assert_eq!(hurt.total_ticks(), 31);
        assert_eq!(hurt.frame_at_tick(99, false), 0);
        // `WAIT4` with no space, as in the stock hidden.gani.
        let hidden = Gani::parse("SINGLEDIRECTION\nANI\n1 0 0\nWAIT4\nANIEND\n").unwrap();
        assert_eq!(hidden.total_ticks(), 5);
    }

    #[test]
    fn setbackto_names_the_animation_to_return_to() {
        let g = Gani::parse("SETBACKTO idle\nANI\n1 0 0\n1 0 0\n1 0 0\n1 0 0\nANIEND\n").unwrap();
        assert_eq!(g.setback_to.as_deref(), Some("idle"));
        assert_eq!(g.frame_count(), 1);
        assert_eq!(Gani::parse("ANI\n1 0 0\n1 0 0\n1 0 0\n1 0 0\nANIEND\n").unwrap().setback_to, None);
    }

    #[test]
    fn stepper_wraps() {
        let text = "\
SINGLEDIRECTION
ANI
1 0 0, 2 1 1
3 0 0
ANIEND
";
        let g = Gani::parse(text).unwrap();
        assert_eq!(g.frame_count(), 2);
        let mut p = GaniPlayer::new(g);
        assert_eq!(p.frame, 0);
        p.step();
        assert_eq!(p.frame, 1);
        p.step();
        assert_eq!(p.frame, 0);
    }

    #[test]
    fn four_rows_make_one_step() {
        let text = "ANI\n1 0 0, 2 1 1\n3 0 0, 4 0 0\n5 0 0, 6 0 0\n7 0 0, 8 0 0\n\n1 0 0, 2 0 0\n1 0 0, 2 0 0\n1 0 0, 2 0 0\n1 0 0, 2 0 0\nANIEND\n";
        let g = Gani::parse(text).unwrap();
        assert_eq!(g.frame_count(), 2);
        assert_eq!(g.frames[0].rows.len(), 4);
        assert_eq!(g.frames[0].rows[0].len(), 2);
        assert!(!g.single_direction);
    }

    #[test]
    fn playsound_attaches_to_the_step_before_it() {
        let text = "\
ANI
1 0 0, 2 0 0
1 0 0, 2 0 0
1 0 0, 2 0 0
1 0 0, 2 0 0
PLAYSOUND steps2.wav 1.5 2

3 0 0, 4 0 0
3 0 0, 4 0 0
3 0 0, 4 0 0
3 0 0, 4 0 0

5 0 0, 6 0 0
5 0 0, 6 0 0
5 0 0, 6 0 0
5 0 0, 6 0 0
PLAYSOUND jump.wav 0.5 3.25
ANIEND
";
        let g = Gani::parse(text).unwrap();
        assert_eq!(g.frame_count(), 3);
        assert_eq!(g.frames[0].sounds, vec![GaniSound { file: "steps2.wav".into(), x: 1.5, y: 2.0 }]);
        assert!(g.frames[1].sounds.is_empty());
        assert_eq!(g.frames[2].sounds, vec![GaniSound { file: "jump.wav".into(), x: 0.5, y: 3.25 }]);
    }

    #[test]
    fn playsound_after_a_single_direction_row() {
        let text = "SINGLEDIRECTION\nANI\n1 0 0, 2 0 0\nPLAYSOUND +loop.wav 1.5 2\n\n3 0 0, 4 0 0\nANIEND\n";
        let g = Gani::parse(text).unwrap();
        assert_eq!(g.frame_count(), 2);
        assert_eq!(g.frames[0].sounds[0].file, "+loop.wav");
        assert!(g.frames[1].sounds.is_empty());
    }

    #[test]
    fn playsound_without_coordinates_is_at_the_listener() {
        let s = parse_playsound("bell.wav").unwrap();
        assert_eq!((s.x, s.y), (LISTENER_DX, LISTENER_DY));
        let s = parse_playsound("two words.wav 3 4").unwrap();
        assert_eq!((s.file.as_str(), s.x, s.y), ("two words.wav", 3.0, 4.0));
        assert!(parse_playsound("   ").is_none());
    }

    #[test]
    fn step_sounds_returns_the_cues_of_the_frame_entered() {
        let text = "SINGLEDIRECTION\nANI\n1 0 0, 2 0 0\nPLAYSOUND a.wav 1.5 2\n\n3 0 0, 4 0 0\n\n5 0 0, 6 0 0\nPLAYSOUND b.wav 1.5 2\nANIEND\n";
        let mut p = GaniPlayer::new(Gani::parse(text).unwrap());
        // Frame 0 is the pose before the first step; it is not entered yet.
        assert!(p.step_sounds().is_empty()); // enters 1
        assert_eq!(p.step_sounds()[0].file, "b.wav"); // enters 2
        assert_eq!(p.step_sounds()[0].file, "a.wav"); // wraps to 0
        assert!(p.step_sounds().is_empty()); // enters 1
    }

    #[test]
    fn real_walk_gani_has_two_footsteps_in_eight_steps() {
        // Same shape as the 2.22 walk.gani: 4 rows per step, blank lines
        // between steps, cues at the end of a step.
        let mut text = String::from("LOOP\nCONTINUOUS\nANI\n");
        for i in 0..8 {
            for _ in 0..4 {
                text.push_str("   0  12  36,  100   8   4\r\n");
            }
            if i == 0 || i == 4 {
                text.push_str("PLAYSOUND steps2.wav 1.5 2\r\n");
            }
            text.push_str("\r\n");
        }
        text.push_str("ANIEND\n");
        let g = Gani::parse(&text).unwrap();
        assert_eq!(g.frame_count(), 8);
        let with: Vec<usize> = (0..8).filter(|i| !g.frames[*i].sounds.is_empty()).collect();
        assert_eq!(with, vec![0, 4]);
    }

    /// Parse every GANI of a real client folder and count the cues. Run with
    /// `SEIHAI_GANI_DIR=<levels/ganis> cargo test real_ganis -- --ignored`.
    #[test]
    #[ignore]
    fn real_ganis_keep_every_cue() {
        let Ok(dir) = std::env::var("SEIHAI_GANI_DIR") else {
            return;
        };
        let (mut cues, mut files) = (0, 0);
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let path = e.path();
            let raw = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).to_string();
            let want = raw.lines().filter(|l| l.trim().starts_with("PLAYSOUND ")).count();
            let g = Gani::load(&path).unwrap();
            let got: usize = g.frames.iter().map(|f| f.sounds.len()).sum();
            assert_eq!(got, want, "{}", path.display());
            if want > 0 {
                files += 1;
            }
            cues += got;
        }
        eprintln!("{cues} cues in {files} files");
        let walk = Gani::load(std::path::Path::new(&std::env::var("SEIHAI_GANI_DIR").unwrap()).join("walk.gani")).unwrap();
        assert_eq!(walk.frame_count(), 8);
        let with: Vec<usize> = (0..8).filter(|i| !walk.frames[*i].sounds.is_empty()).collect();
        assert_eq!(with, vec![0, 4]);
    }
}
