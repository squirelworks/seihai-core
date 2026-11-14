//! MIDI music (`.mid`), rendered to PCM by a small built-in synthesizer.
//!
//! The 6.x worlds play `.mid` music (#446). The original clients hand it
//! to the system's MIDI device (2.x) or to FMOD (6.x), which plays it with
//! a General MIDI sound set. Seihai ships no sound set, so this is its own
//! approximation: every note is a simple waveform chosen by the channel's
//! General MIDI program family, with an envelope, and channel 10 is drums
//! made from noise and a low thump. The notes, the timing (tempo changes
//! included), the channel volume and expression, the pitch bend at note
//! start and the velocities are the file's; the timbre is not the
//! original's.
//!
//! What is read: a Standard MIDI File, format 0 or 1 (format 2's tracks
//! are mixed as if format 1), with ticks per quarter note or SMPTE
//! division; note on and off, program change, controllers 7 and 11, pitch
//! bend, and the tempo meta event. Everything else is skipped. The result
//! is mono, 22050 Hz, and at most [`MAX_SECONDS`] long.

use super::wav::Pcm;

/// The output rate.
pub const RATE: u32 = 22050;
/// Longer pieces are cut here.
pub const MAX_SECONDS: f64 = 600.0;
/// A note held longer than this (a missing note off) ends here.
const MAX_NOTE_SECONDS: f64 = 20.0;
/// Release after the note off, in seconds.
const RELEASE: f64 = 0.08;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    NoteOn { ch: u8, key: u8, vel: u8 },
    NoteOff { ch: u8, key: u8 },
    Program { ch: u8, program: u8 },
    Control { ch: u8, num: u8, value: u8 },
    Bend { ch: u8, value: i16 },
    Tempo(u32),
}

#[derive(Debug, Clone, Copy)]
struct Event {
    tick: u64,
    /// Track, then order in the track: ties keep the file's order.
    order: (usize, usize),
    kind: Kind,
}

/// The timbre a General MIDI program gets here.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Voice {
    /// Piano, chromatic percussion, guitar: struck, decaying.
    Struck,
    /// Organ, reed, pipe: held, steady.
    Reed,
    /// Bass: struck, low and round.
    Bass,
    /// Strings, ensemble, pads: slow attack, held.
    Bowed,
    /// Brass and leads: bright, held.
    Brass,
}

fn voice_for(program: u8) -> Voice {
    match program {
        0..=15 | 24..=31 | 104..=111 => Voice::Struck,
        16..=23 | 64..=79 => Voice::Reed,
        32..=39 => Voice::Bass,
        40..=55 | 88..=103 => Voice::Bowed,
        _ => Voice::Brass,
    }
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8, String> {
        let v = *self.b.get(self.i).ok_or("MIDI data ends early")?;
        self.i += 1;
        Ok(v)
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self
            .b
            .get(self.i..self.i + n)
            .ok_or("MIDI data ends early")?;
        self.i += n;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, String> {
        let s = self.bytes(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        let s = self.bytes(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    /// A variable-length quantity: at most four bytes.
    fn vlq(&mut self) -> Result<u32, String> {
        let mut v = 0u32;
        for _ in 0..4 {
            let c = self.byte()?;
            v = (v << 7) | u32::from(c & 0x7F);
            if c & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err("bad MIDI length".into())
    }
}

/// The events of one `MTrk` chunk.
fn read_track(data: &[u8], track: usize, out: &mut Vec<Event>) -> Result<(), String> {
    let mut r = Reader { b: data, i: 0 };
    let mut tick = 0u64;
    let mut status = 0u8;
    let mut n = 0;
    while r.i < data.len() {
        tick += u64::from(r.vlq()?);
        let mut first = r.byte()?;
        if first >= 0x80 {
            status = first;
            if status < 0xF0 {
                first = r.byte()?;
            }
        } else if status == 0 || status >= 0xF0 {
            return Err("MIDI running status with no status".into());
        }
        let ch = status & 0x0F;
        let kind = match status & 0xF0 {
            0x80 => {
                r.byte()?;
                Some(Kind::NoteOff { ch, key: first })
            }
            0x90 => {
                let vel = r.byte()?;
                Some(if vel == 0 {
                    Kind::NoteOff { ch, key: first }
                } else {
                    Kind::NoteOn {
                        ch,
                        key: first,
                        vel,
                    }
                })
            }
            0xA0 => {
                r.byte()?;
                None
            }
            0xB0 => Some(Kind::Control {
                ch,
                num: first,
                value: r.byte()?,
            }),
            0xC0 => Some(Kind::Program { ch, program: first }),
            0xD0 => None,
            0xE0 => {
                let hi = r.byte()?;
                let v = (i16::from(hi & 0x7F) << 7 | i16::from(first & 0x7F)) - 8192;
                Some(Kind::Bend { ch, value: v })
            }
            _ => match status {
                0xFF => {
                    let ty = r.byte()?;
                    let len = r.vlq()? as usize;
                    let body = r.bytes(len)?;
                    // A meta event resets running status.
                    status = 0;
                    match (ty, body) {
                        (0x2F, _) => break,
                        (0x51, [a, b, c]) => Some(Kind::Tempo(
                            u32::from(*a) << 16 | u32::from(*b) << 8 | u32::from(*c),
                        )),
                        _ => None,
                    }
                }
                0xF0 | 0xF7 => {
                    let len = r.vlq()? as usize;
                    r.bytes(len)?;
                    status = 0;
                    None
                }
                _ => return Err(format!("unknown MIDI status {status:#04x}")),
            },
        };
        if let Some(kind) = kind {
            out.push(Event {
                tick,
                order: (track, n),
                kind,
            });
            n += 1;
        }
    }
    Ok(())
}

/// One note to render.
#[derive(Debug, Clone, Copy)]
struct Note {
    start: f64,
    end: f64,
    key: u8,
    /// 0..1: velocity times the channel's volume and expression.
    gain: f64,
    /// Semitones of pitch bend at the note's start.
    bend: f64,
    voice: Option<Voice>,
}

/// The notes of a Standard MIDI File, with their times in seconds.
fn notes(bytes: &[u8]) -> Result<Vec<Note>, String> {
    let mut r = Reader { b: bytes, i: 0 };
    if r.bytes(4)? != b"MThd" {
        return Err("not a MIDI file (no MThd)".into());
    }
    let hlen = r.u32()? as usize;
    if hlen < 6 {
        return Err("short MIDI header".into());
    }
    let _format = r.u16()?;
    let ntrks = r.u16()? as usize;
    let division = r.u16()?;
    r.bytes(hlen - 6)?;
    // Seconds per tick at the default 120 bpm, and how tempo changes it.
    let (ticks_per_quarter, smpte_tick) = if division & 0x8000 != 0 {
        let fps = f64::from(-((division >> 8) as i8));
        let per_frame = f64::from(division & 0xFF).max(1.0);
        (None, 1.0 / (fps.max(1.0) * per_frame))
    } else {
        (Some(f64::from(division.max(1))), 0.0)
    };
    let mut events = Vec::new();
    for t in 0..ntrks {
        if r.i + 8 > bytes.len() {
            break;
        }
        let id = r.bytes(4)?;
        let len = r.u32()? as usize;
        let body = r.bytes(len.min(bytes.len() - r.i))?;
        if id == b"MTrk" {
            read_track(body, t, &mut events)?;
        }
    }
    events.sort_by_key(|e| (e.tick, e.order));

    let mut out: Vec<Note> = Vec::new();
    let mut tempo = 500_000.0f64;
    let mut last_tick = 0u64;
    let mut now = 0.0f64;
    let mut program = [0u8; 16];
    let mut volume = [100u8; 16];
    let mut expression = [127u8; 16];
    let mut bend = [0i16; 16];
    // Sounding notes: (channel, key) to the index in `out`.
    let mut on: std::collections::HashMap<(u8, u8), Vec<usize>> = Default::default();
    for e in &events {
        let dt = (e.tick - last_tick) as f64;
        now += match ticks_per_quarter {
            Some(q) => dt * tempo / 1e6 / q,
            None => dt * smpte_tick,
        };
        last_tick = e.tick;
        if now > MAX_SECONDS {
            break;
        }
        match e.kind {
            Kind::Tempo(t) if t > 0 => tempo = f64::from(t),
            Kind::Tempo(_) => {}
            Kind::Program { ch, program: p } => program[usize::from(ch)] = p,
            Kind::Control { ch, num: 7, value } => volume[usize::from(ch)] = value,
            Kind::Control { ch, num: 11, value } => expression[usize::from(ch)] = value,
            // All notes off and all sound off.
            Kind::Control {
                ch, num: 120 | 123, ..
            } => {
                for ((c, _), idx) in on.iter_mut() {
                    if *c == ch {
                        for i in idx.drain(..) {
                            out[i].end = now;
                        }
                    }
                }
            }
            Kind::Control { .. } => {}
            Kind::Bend { ch, value } => bend[usize::from(ch)] = value,
            Kind::NoteOn { ch, key, vel } => {
                let c = usize::from(ch);
                let gain = f64::from(vel) / 127.0
                    * (f64::from(volume[c]) / 127.0)
                    * (f64::from(expression[c]) / 127.0);
                on.entry((ch, key)).or_default().push(out.len());
                out.push(Note {
                    start: now,
                    end: now + MAX_NOTE_SECONDS,
                    key,
                    gain,
                    bend: f64::from(bend[c]) / 8192.0 * 2.0,
                    voice: (ch != 9).then(|| voice_for(program[c])),
                });
            }
            Kind::NoteOff { ch, key } => {
                if let Some(idx) = on.get_mut(&(ch, key)) {
                    if !idx.is_empty() {
                        let i = idx.remove(0);
                        out[i].end = now;
                    }
                }
            }
        }
    }
    for n in &mut out {
        n.end = n.end.min(MAX_SECONDS).max(n.start);
    }
    Ok(out)
}

/// A small fast noise source for the drums.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        f64::from(self.0) / f64::from(u32::MAX) * 2.0 - 1.0
    }
}

/// Render a Standard MIDI File to mono PCM at [`RATE`].
pub fn render_midi(bytes: &[u8]) -> Result<Pcm, String> {
    let notes = notes(bytes)?;
    let rate = f64::from(RATE);
    let end = notes
        .iter()
        .map(|n| n.end + RELEASE)
        .fold(0.0f64, f64::max)
        .min(MAX_SECONDS + RELEASE);
    let len = (end * rate).ceil() as usize + 1;
    let mut mix = vec![0f32; len];
    let mut noise = Noise(0x2545_F491);
    for n in &notes {
        let s0 = (n.start * rate) as usize;
        let held = ((n.end - n.start) * rate) as usize;
        let total = held + (RELEASE * rate) as usize;
        let amp = 0.18 * n.gain;
        match n.voice {
            Some(voice) => {
                let freq = 440.0 * 2f64.powf((f64::from(n.key) - 69.0 + n.bend) / 12.0);
                let step = freq / rate;
                let mut phase = 0.0f64;
                for k in 0..total {
                    let Some(out) = mix.get_mut(s0 + k) else {
                        break;
                    };
                    let t = k as f64 / rate;
                    let wave = match voice {
                        // Triangle with a touch of its octave.
                        Voice::Struck => tri(phase) * 0.8 + tri((phase * 2.0).fract()) * 0.2,
                        Voice::Reed => square(phase) * 0.5,
                        Voice::Bass => (phase * std::f64::consts::TAU).sin(),
                        Voice::Bowed => saw(phase) * 0.55,
                        Voice::Brass => saw(phase) * 0.45 + square(phase) * 0.2,
                    };
                    let attack = match voice {
                        Voice::Bowed => (t / 0.08).min(1.0),
                        _ => (t / 0.005).min(1.0),
                    };
                    let body = match voice {
                        Voice::Struck => (-t * 2.5).exp(),
                        Voice::Bass => (-t * 1.5).exp(),
                        _ => 1.0,
                    };
                    let release = if k < held {
                        1.0
                    } else {
                        1.0 - (k - held) as f64 / (RELEASE * rate)
                    };
                    *out += (wave * amp * attack * body * release.max(0.0)) as f32;
                    phase = (phase + step).fract();
                }
            }
            None => {
                // Drums: a kick (35, 36) is a falling sine; the rest are
                // noise, longer for cymbals (49..=59), short for hats.
                let (secs, decay) = match n.key {
                    35 | 36 => (0.25, 14.0),
                    42 | 44 => (0.06, 60.0),
                    46 | 49..=59 => (0.6, 6.0),
                    _ => (0.18, 20.0),
                };
                let total = (secs * rate) as usize;
                let mut phase = 0.0f64;
                for k in 0..total {
                    let Some(out) = mix.get_mut(s0 + k) else {
                        break;
                    };
                    let t = k as f64 / rate;
                    let env = (-t * decay).exp();
                    let v = if matches!(n.key, 35 | 36) {
                        let f = 50.0 + 90.0 * (-t * 30.0).exp();
                        phase = (phase + f / rate).fract();
                        (phase * std::f64::consts::TAU).sin() * 1.6
                    } else {
                        noise.next() * 0.7
                    };
                    *out += (v * amp * env) as f32;
                }
            }
        }
    }
    let samples = mix
        .into_iter()
        .map(|v| (soft_clip(v) * 32767.0) as i16)
        .collect();
    Ok(Pcm {
        channels: 1,
        rate: RATE,
        samples,
    })
}

fn tri(p: f64) -> f64 {
    4.0 * (p - (p + 0.5).floor()).abs() - 1.0
}

fn square(p: f64) -> f64 {
    if p < 0.5 {
        1.0
    } else {
        -1.0
    }
}

fn saw(p: f64) -> f64 {
    2.0 * p - 1.0
}

/// Keeps a loud chord from wrapping around.
fn soft_clip(v: f32) -> f32 {
    v.clamp(-3.0, 3.0) * (27.0 + v * v) / (27.0 + 9.0 * v * v)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A format-0 file: 96 ticks per quarter, tempo `tempo` (µs per
    /// quarter), then `events` (delta ticks and the bytes after it).
    pub(crate) fn smf(tempo: u32, events: &[(u32, &[u8])]) -> Vec<u8> {
        let mut trk = Vec::new();
        let t = tempo.to_be_bytes();
        trk.extend([0x00, 0xFF, 0x51, 0x03, t[1], t[2], t[3]]);
        for (dt, b) in events {
            let mut v = Vec::new();
            let mut d = *dt;
            v.push((d & 0x7F) as u8);
            d >>= 7;
            while d > 0 {
                v.push((d & 0x7F) as u8 | 0x80);
                d >>= 7;
            }
            v.reverse();
            trk.extend(v);
            trk.extend_from_slice(b);
        }
        trk.extend([0x00, 0xFF, 0x2F, 0x00]);
        let mut f = b"MThd\0\0\0\x06\0\0\0\x01\0\x60".to_vec();
        f.extend(b"MTrk");
        f.extend((trk.len() as u32).to_be_bytes());
        f.extend(trk);
        f
    }

    #[test]
    fn notes_follow_the_tempo_and_running_status() {
        // 96 ticks a quarter at 1 s a quarter: a note of 48 ticks is 0.5 s.
        // The second note on uses running status; velocity 0 ends it.
        let f = smf(
            1_000_000,
            &[
                (0, &[0xC0, 40]),
                (0, &[0x90, 60, 100]),
                (48, &[0x80, 60, 0]),
                (0, &[0x90, 64, 127]),
                (96, &[64, 0]),
                (0, &[0x99, 36, 100]),
                (0, &[0x89, 36, 0]),
            ],
        );
        let n = notes(&f).unwrap();
        assert_eq!(n.len(), 3);
        assert!((n[0].start, n[0].end) == (0.0, 0.5), "{:?}", n[0]);
        assert_eq!(n[0].voice, Some(Voice::Bowed));
        assert!((n[1].start - 0.5).abs() < 1e-9 && (n[1].end - 1.5).abs() < 1e-9);
        assert!(n[1].gain > n[0].gain);
        assert_eq!(n[2].voice, None, "channel 10 is drums");
    }

    #[test]
    fn a_tempo_change_halfway_changes_the_rest() {
        let f = smf(
            500_000,
            &[
                (0, &[0x90, 60, 100]),
                (96, &[0xFF, 0x51, 0x03, 0x0F, 0x42, 0x40]),
                (96, &[0x80, 60, 0]),
            ],
        );
        let n = notes(&f).unwrap();
        // 96 ticks at 0.5 s, then 96 at 1 s.
        assert!((n[0].end - 1.5).abs() < 1e-9, "{:?}", n[0]);
    }

    #[test]
    fn renders_sound_of_the_right_length() {
        let f = smf(500_000, &[(0, &[0x90, 69, 100]), (96, &[0x80, 69, 0])]);
        let pcm = render_midi(&f).unwrap();
        assert_eq!((pcm.channels, pcm.rate), (1, RATE));
        // 0.5 s and the release.
        assert!(
            (560..=600).contains(&pcm.duration_ms()),
            "{}",
            pcm.duration_ms()
        );
        let peak = pcm.samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(peak > 1000, "{peak}");
    }

    #[test]
    fn junk_is_an_error() {
        assert!(render_midi(b"MThd").is_err());
        assert!(render_midi(b"RIFF....").is_err());
        let mut f = smf(500_000, &[(0, &[0x90, 60, 100])]);
        // A track that claims more than the file has is read to the end.
        let at = 14 + 4;
        f[at..at + 4].copy_from_slice(&1000u32.to_be_bytes());
        assert!(render_midi(&f).is_ok());
    }
}
