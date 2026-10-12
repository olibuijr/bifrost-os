//! The one playback path for system sounds (keyboard clicks): the clicks are registered once with the toolbox's
//! `SystemSounds` at the output's rate, and a click is mixed into a buffer and written to the sound-effects
//! stream (`SpeakerOut::system_sounds`). The shell binary calls [`KeyClicks::click`] for `Effect::KeyClick`.

use super::audio_out::AudioOut;
use crate::shell::kb_clicks::{self, Click};
use bifrost_audio_toolbox::system_sound::SystemSounds;
use std::io;

/// The output rate (stereo S16, like every `AudioOut`).
pub const RATE: u32 = 48_000;

/// Sound id of a click (0 is the toolbox's alert).
fn sound_id(c: Click) -> u32 {
    1 + match c {
        Click::Key => 0,
        Click::Delete => 1,
        Click::Modifier => 2,
    }
}

pub struct KeyClicks {
    out: Box<dyn AudioOut>,
    sounds: SystemSounds,
}

impl KeyClicks {
    /// Clicks played on `out`.
    pub fn new(out: Box<dyn AudioOut>) -> io::Result<KeyClicks> {
        let other = |e| io::Error::other(format!("system sounds: {e:?}"));
        let mut sounds = SystemSounds::new(RATE, 2).map_err(other)?;
        for c in [Click::Key, Click::Delete, Click::Modifier] {
            let mono: Vec<f32> = kb_clicks::pcm(c, 1.0).iter().map(|&s| s as f32 / 32768.0).collect();
            sounds.register_samples(sound_id(c), kb_clicks::RATE, 1, &mono).map_err(other)?;
        }
        Ok(KeyClicks { out, sounds })
    }

    /// Clicks on the sound-effects device role of the audio server.
    #[cfg(target_os = "linux")]
    pub fn on_sound_effects() -> io::Result<KeyClicks> {
        Self::new(Box::new(super::speaker::SpeakerOut::system_sounds(RATE, 2)))
    }

    /// Plays one click at `gain` (0 through 1): mixed through the toolbox player and queued on the stream.
    pub fn click(&mut self, c: Click, gain: f32) -> io::Result<()> {
        let id = sound_id(c);
        self.sounds.play(id, gain).map_err(|e| io::Error::other(format!("system sounds: {e:?}")))?;
        let frames = self.sounds.frames(id).unwrap_or(0);
        let mut mix = vec![0f32; frames * 2];
        self.sounds.mix_into(&mut mix);
        let pcm: Vec<i16> = mix.iter().map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).collect();
        self.out.write(&pcm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Capture(Arc<Mutex<Vec<i16>>>);

    impl AudioOut for Capture {
        fn write(&mut self, frames: &[i16]) -> io::Result<()> {
            self.0.lock().unwrap().extend_from_slice(frames);
            Ok(())
        }
        fn played(&mut self) -> u64 {
            0
        }
        fn pause(&mut self, _on: bool) {}
        fn discard(&mut self) {}
    }

    #[test]
    fn a_key_click_writes_non_zero_frames() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let mut k = KeyClicks::new(Box::new(Capture(got.clone()))).unwrap();
        k.click(Click::Key, 1.0).unwrap();
        let pcm = got.lock().unwrap();
        // 14 ms at 48 kHz, stereo
        assert_eq!(pcm.len(), 672 * 2);
        assert!(pcm.iter().any(|&s| s != 0));
    }

    #[test]
    fn zero_gain_is_silent() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let mut k = KeyClicks::new(Box::new(Capture(got.clone()))).unwrap();
        k.click(Click::Delete, 0.0).unwrap();
        assert!(got.lock().unwrap().iter().all(|&s| s == 0));
    }
}
