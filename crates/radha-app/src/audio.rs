//! Typing sounds (P06): key, error, success, complete and timer, from the
//! bundled WAVs in `assets/sounds/` (embedded, about 38 KB).
//!
//! Playback never blocks typing: a request is a channel send to one audio
//! thread, which hands the in-memory WAV to the Windows `PlaySoundW`
//! (`SND_ASYNC | SND_MEMORY`, winmm, part of every Windows install). A new
//! sound cuts the previous one, which is what a key click wants. Muting is a
//! flag check before the send. Other platforms are silent.

use std::cell::Cell;
use std::sync::mpsc::{self, Sender};

/// One of the bundled sounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    Key,
    Error,
    Success,
    Complete,
    Timer,
}

impl Sound {
    /// The embedded WAV.
    #[must_use]
    pub fn bytes(self) -> &'static [u8] {
        match self {
            Sound::Key => include_bytes!("../../../assets/sounds/key.wav"),
            Sound::Error => include_bytes!("../../../assets/sounds/error.wav"),
            Sound::Success => include_bytes!("../../../assets/sounds/success.wav"),
            Sound::Complete => include_bytes!("../../../assets/sounds/complete.wav"),
            Sound::Timer => include_bytes!("../../../assets/sounds/timer.wav"),
        }
    }
}

/// The app's sound player.
pub struct Audio {
    tx: Option<Sender<Sound>>,
    enabled: Cell<bool>,
}

impl std::fmt::Debug for Audio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audio")
            .field("enabled", &self.enabled.get())
            .field("thread", &self.tx.is_some())
            .finish()
    }
}

impl Audio {
    /// Start the audio thread. If it cannot start, the app is silent.
    #[must_use]
    pub fn new(enabled: bool) -> Self {
        let (tx, rx) = mpsc::channel::<Sound>();
        let spawned = std::thread::Builder::new()
            .name("radha-audio".into())
            .spawn(move || {
                while let Ok(sound) = rx.recv() {
                    sys::play(sound.bytes());
                }
            });
        let tx = match spawned {
            Ok(_) => Some(tx),
            Err(e) => {
                log::warn!("Audio thread unavailable, sounds are off: {e}");
                None
            }
        };
        Audio {
            tx,
            enabled: Cell::new(enabled),
        }
    }

    /// Whether sounds play.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled.get()
    }

    /// Mute or unmute.
    pub fn set_enabled(&self, on: bool) {
        self.enabled.set(on);
    }

    /// Queue a sound (no-op when muted). Never blocks.
    pub fn play(&self, sound: Sound) {
        if !self.enabled.get() {
            return;
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(sound);
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;

    const SND_ASYNC: u32 = 0x0001;
    const SND_NODEFAULT: u32 = 0x0002;
    const SND_MEMORY: u32 = 0x0004;

    #[link(name = "winmm")]
    extern "system" {
        fn PlaySoundW(sound: *const u16, module: *mut c_void, flags: u32) -> i32;
    }

    /// Play an in-memory WAV asynchronously. The bytes are `'static`, so
    /// they outlive the asynchronous playback.
    pub fn play(wav: &'static [u8]) {
        // SAFETY: with SND_MEMORY the first argument points to a complete
        // WAV image in memory; `wav` is a 'static embedded file, so it stays
        // valid for the whole asynchronous playback. No module handle is
        // used.
        let ok = unsafe {
            PlaySoundW(
                wav.as_ptr().cast::<u16>(),
                std::ptr::null_mut(),
                SND_ASYNC | SND_NODEFAULT | SND_MEMORY,
            )
        };
        if ok == 0 {
            log::debug!("PlaySoundW could not play a sound");
        }
    }
}

#[cfg(not(windows))]
mod sys {
    pub fn play(_wav: &'static [u8]) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_is_an_embedded_wav() {
        for s in [
            Sound::Key,
            Sound::Error,
            Sound::Success,
            Sound::Complete,
            Sound::Timer,
        ] {
            let b = s.bytes();
            assert!(b.len() > 44, "{s:?}");
            assert_eq!(&b[..4], b"RIFF");
            assert_eq!(&b[8..12], b"WAVE");
        }
    }

    #[test]
    fn muting_is_a_flag_and_never_blocks() {
        let a = Audio::new(false);
        assert!(!a.enabled());
        a.play(Sound::Key); // muted: nothing queued
        a.set_enabled(true);
        assert!(a.enabled());
    }
}
