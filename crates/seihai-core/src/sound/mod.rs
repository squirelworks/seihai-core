//! Sound effects.
//!
//! There is no sound packet: the client decides what to play. This module
//! is the optional audio path. [`SoundEngine`] takes a sound name and where
//! it happens, applies the rules the 2.05 client follows (see
//! `documentation/audio.md`), decodes the `.wav` from the asset folder, and
//! hands the result to a [`SoundOut`] backend.
//!
//! The backends here are [`NullSound`], which plays nothing, so a client
//! runs on a box with no sound card and with no sound files, and
//! [`RodioSound`], the device output, behind the `audio` feature.
//! [`default_backend`] picks the one a client starts with.

mod engine;
#[cfg(feature = "audio")]
mod rodio_out;
mod wav;

#[cfg(feature = "audio")]
pub use rodio_out::RodioSound;

pub use engine::{
    is_sound_name, pan_for, wanted_name, Outcome, SoundEngine, SoundEvent, MAX_VOICES, PAN_PER_TILE,
};
pub use wav::{decode_mp3, decode_ogg, decode_sound, decode_wav, Pcm};

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// One effect to start.
#[derive(Debug, Clone)]
pub struct PlayRequest {
    /// Lower-cased file name without the `+` loop prefix.
    pub name: String,
    pub pcm: Arc<Pcm>,
    /// DirectSound pan units: -10000 (left) to 10000 (right), 0 centre.
    pub pan: i32,
    pub looped: bool,
}

/// An audio output. It is told what to start and stop; the engine keeps
/// the voice count and the rules.
pub trait SoundOut {
    /// Short name for logs.
    fn kind(&self) -> &'static str;
    fn play(&mut self, req: &PlayRequest);
    /// Stop every voice with this name.
    fn stop(&mut self, name: &str);
    /// Stop all looped voices (leaving a level does this).
    fn stop_all_loops(&mut self);
}

/// Left and right gain for a DirectSound pan value (hundredths of a dB of
/// attenuation on the far side): 0 is centre, 10000 is silent on the left.
pub fn pan_gains(pan: i32) -> (f32, f32) {
    let atten = |hundredths_db: i32| 10f32.powf(-(hundredths_db.max(0) as f32) / 2000.0);
    (atten(pan), atten(-pan))
}

/// Plays nothing. Used with no sound card, `--no-sound`, or no sounds.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullSound;

impl SoundOut for NullSound {
    fn kind(&self) -> &'static str {
        "null"
    }
    fn play(&mut self, _req: &PlayRequest) {}
    fn stop(&mut self, _name: &str) {}
    fn stop_all_loops(&mut self) {}
}

/// What a [`RecordingSound`] was told.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Play {
        name: String,
        pan: i32,
        looped: bool,
    },
    Stop(String),
    StopAllLoops,
}

/// Records calls, for tests. Clones share one log.
#[derive(Debug, Default, Clone)]
pub struct RecordingSound {
    pub calls: Rc<RefCell<Vec<Call>>>,
}

impl RecordingSound {
    pub fn log(&self) -> Vec<Call> {
        self.calls.borrow().clone()
    }
}

impl SoundOut for RecordingSound {
    fn kind(&self) -> &'static str {
        "recording"
    }
    fn play(&mut self, req: &PlayRequest) {
        self.calls.borrow_mut().push(Call::Play {
            name: req.name.clone(),
            pan: req.pan,
            looped: req.looped,
        });
    }
    fn stop(&mut self, name: &str) {
        self.calls.borrow_mut().push(Call::Stop(name.to_string()));
    }
    fn stop_all_loops(&mut self) {
        self.calls.borrow_mut().push(Call::StopAllLoops);
    }
}

/// The best backend this build has: the device one when the `audio`
/// feature is on and a device opens, otherwise [`NullSound`]. The string
/// says which and why, for one startup log line.
pub fn default_backend() -> (Box<dyn SoundOut>, String) {
    #[cfg(feature = "audio")]
    {
        match RodioSound::open() {
            Ok(b) => (Box::new(b), "audio device opened".to_string()),
            Err(e) => (
                Box::new(NullSound),
                format!("no audio device ({e}); sound is silent"),
            ),
        }
    }
    #[cfg(not(feature = "audio"))]
    {
        (
            Box::new(NullSound),
            "built without the audio feature; sound is silent".to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_backend_without_the_feature_is_null() {
        let (b, why) = default_backend();
        if cfg!(feature = "audio") {
            assert!(!why.is_empty());
        } else {
            assert_eq!(b.kind(), "null");
            assert!(why.contains("audio feature"));
        }
    }

    #[test]
    fn centre_pan_is_full_both_sides() {
        assert_eq!(pan_gains(0), (1.0, 1.0));
    }

    #[test]
    fn pan_attenuates_the_far_side_in_hundredths_of_a_db() {
        // 600 hundredths of a dB is 6 dB, about half the amplitude.
        let (l, r) = pan_gains(600);
        assert_eq!(r, 1.0);
        assert!((l - 0.501).abs() < 0.01, "{l}");
        let (l, r) = pan_gains(-600);
        assert_eq!(l, 1.0);
        assert!((r - 0.501).abs() < 0.01, "{r}");
        // Fully panned is inaudible on the far side.
        assert!(pan_gains(10000).0 < 1e-4);
    }

    #[test]
    fn recording_backend_logs_calls() {
        let mut r = RecordingSound::default();
        let log = r.clone();
        r.stop("a.wav");
        r.stop_all_loops();
        assert_eq!(
            log.log(),
            vec![Call::Stop("a.wav".into()), Call::StopAllLoops]
        );
    }
}
