//! Device backend on `rodio` (feature `audio`).
//!
//! The output stream lives on its own thread and is told what to do over
//! a channel. On Windows the launcher's window (winit, with drag and drop)
//! makes the main thread a single-threaded COM apartment, and a WASAPI
//! stream opened there stays silent: the device opens but nothing is heard
//! (seen on Windows, 2026-10-30).
//! A thread of our own starts with no COM apartment, so `cpal` sets up the
//! one it needs. (`OutputStream` is not `Send` either, so it has to stay on
//! the thread that made it.)

use std::sync::mpsc::{channel, Sender};

use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};

use super::{pan_gains, PlayRequest, SoundOut};

enum Cmd {
    Play(PlayRequest),
    Stop(String),
    StopAllLoops,
}

struct Playing {
    name: String,
    looped: bool,
    sink: Sink,
}

/// The audio thread's side: the stream handle and the voices.
struct Voices {
    handle: OutputStreamHandle,
    playing: Vec<Playing>,
}

impl Voices {
    fn play(&mut self, req: &PlayRequest) {
        self.playing.retain(|p| p.looped || !p.sink.empty());
        let Ok(sink) = Sink::try_new(&self.handle) else {
            return;
        };
        let (l, r) = pan_gains(req.pan);
        let src = &req.pcm.samples;
        let stereo: Vec<i16> = if req.pcm.channels == 1 {
            src.iter()
                .flat_map(|s| [(*s as f32 * l) as i16, (*s as f32 * r) as i16])
                .collect()
        } else {
            src.as_chunks::<2>()
                .0
                .iter()
                .flat_map(|f| [(f[0] as f32 * l) as i16, (f[1] as f32 * r) as i16])
                .collect()
        };
        let buf = SamplesBuffer::new(2, req.pcm.rate, stereo);
        if req.looped {
            sink.append(buf.repeat_infinite());
        } else {
            sink.append(buf);
        }
        self.playing.push(Playing {
            name: req.name.clone(),
            looped: req.looped,
            sink,
        });
    }

    fn stop_where(&mut self, f: impl Fn(&Playing) -> bool) {
        self.playing.retain(|p| {
            if f(p) {
                p.sink.stop();
                false
            } else {
                true
            }
        });
    }
}

pub struct RodioSound {
    tx: Sender<Cmd>,
}

impl RodioSound {
    /// Open the default output device on a new audio thread. Returns once
    /// the device is open, or with the reason it could not be.
    pub fn open() -> Result<Self, String> {
        let (tx, rx) = channel::<Cmd>();
        let (ready_tx, ready_rx) = channel::<Result<(), String>>();
        std::thread::Builder::new()
            .name("seihai-audio".into())
            .spawn(move || {
                // The stream must stay alive for the handle to work: it is
                // dropped when the client drops its sender and the loop ends.
                let (_stream, handle) = match OutputStream::try_default() {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.to_string()));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                let mut voices = Voices {
                    handle,
                    playing: Vec::new(),
                };
                for cmd in rx {
                    match cmd {
                        Cmd::Play(req) => voices.play(&req),
                        Cmd::Stop(name) => voices.stop_where(|p| p.name == name),
                        Cmd::StopAllLoops => voices.stop_where(|p| p.looped),
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        ready_rx
            .recv()
            .map_err(|_| "audio thread ended".to_string())??;
        Ok(Self { tx })
    }
}

impl SoundOut for RodioSound {
    fn kind(&self) -> &'static str {
        "rodio"
    }

    fn play(&mut self, req: &PlayRequest) {
        let _ = self.tx.send(Cmd::Play(req.clone()));
    }

    fn stop(&mut self, name: &str) {
        let _ = self.tx.send(Cmd::Stop(name.to_string()));
    }

    fn stop_all_loops(&mut self) {
        let _ = self.tx.send(Cmd::StopAllLoops);
    }
}
