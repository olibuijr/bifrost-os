//! The one playback path for system sounds: keyboard clicks and alert tones. Each sound is registered once with the
//! toolbox's `SystemSounds` at the output's rate. A play is mixed into a buffer and written to the sound-effects
//! stream (`SpeakerOut::system_sounds`), which the audio server (bifrost-audiod) plays. The shell binary calls
//! [`SoundPlayer::click`] for `Effect::KeyClick` and [`SoundPlayer::alert`] when a timer or alarm fires.

use super::audio_out::AudioOut;
use crate::shell::alert_sound;
use crate::shell::earcon;
use crate::shell::kb_clicks::{self, Click};
use crate::shell::options::ALERT_SOUNDS;
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

/// Sound id of alert tone `index` (the click ids are 1 through 3).
fn alert_id(index: usize) -> u32 {
    10 + index as u32
}

/// Sound id of earcon `index` (alert ids run 10 up to 10 + the alert count, so earcons start at 40).
fn earcon_id(index: usize) -> u32 {
    40 + index as u32
}

pub struct SoundPlayer {
    out: Box<dyn AudioOut>,
    sounds: SystemSounds,
}

impl SoundPlayer {
    /// Clicks and alert tones played on `out`.
    pub fn new(out: Box<dyn AudioOut>) -> io::Result<SoundPlayer> {
        let other = |e| io::Error::other(format!("system sounds: {e:?}"));
        let mut sounds = SystemSounds::new(RATE, 2).map_err(other)?;
        for c in [Click::Key, Click::Delete, Click::Modifier] {
            let mono: Vec<f32> = kb_clicks::pcm(c, 1.0).iter().map(|&s| s as f32 / 32768.0).collect();
            sounds.register_samples(sound_id(c), kb_clicks::RATE, 1, &mono).map_err(other)?;
        }
        for index in 0..ALERT_SOUNDS.len() {
            let mono: Vec<f32> = alert_sound::pcm(index, 1.0).iter().map(|&s| s as f32 / 32768.0).collect();
            sounds.register_samples(alert_id(index), kb_clicks::RATE, 1, &mono).map_err(other)?;
        }
        for index in 0..earcon::count() {
            let mono: Vec<f32> = earcon::pcm(index, 1.0).iter().map(|&s| s as f32 / 32768.0).collect();
            sounds.register_samples(earcon_id(index), kb_clicks::RATE, 1, &mono).map_err(other)?;
        }
        Ok(SoundPlayer { out, sounds })
    }

    /// Sounds on the sound-effects device role of the audio server.
    #[cfg(target_os = "linux")]
    pub fn on_sound_effects() -> io::Result<SoundPlayer> {
        Self::new(Box::new(super::speaker::SpeakerOut::system_sounds(RATE, 2)))
    }

    /// Plays one click at `gain` (0 through 1).
    pub fn click(&mut self, c: Click, gain: f32) -> io::Result<()> {
        self.play(sound_id(c), gain)
    }

    /// Plays alert tone `index` (an index into `options::ALERT_SOUNDS`) at `gain` (0 through 1).
    pub fn alert(&mut self, index: usize, gain: f32) -> io::Result<()> {
        if index >= ALERT_SOUNDS.len() {
            return Err(io::Error::other(format!("no alert sound {index}")));
        }
        self.play(alert_id(index), gain)
    }

    /// Plays earcon `index` (an index into `shell::earcon::EARCONS`) at `gain` (0 through 1).
    pub fn earcon(&mut self, index: usize, gain: f32) -> io::Result<()> {
        if index >= earcon::count() {
            return Err(io::Error::other(format!("no earcon {index}")));
        }
        self.play(earcon_id(index), gain)
    }

    /// Mixes sound `id` through the toolbox player and queues the result on the stream.
    fn play(&mut self, id: u32, gain: f32) -> io::Result<()> {
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
            self.0.lock().unwrap_or_else(|p| p.into_inner()).extend_from_slice(frames);
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
        let mut k = SoundPlayer::new(Box::new(Capture(got.clone()))).unwrap();
        k.click(Click::Key, 1.0).unwrap();
        let pcm = got.lock().unwrap_or_else(|p| p.into_inner());
        // 14 ms at 48 kHz, stereo
        assert_eq!(pcm.len(), 672 * 2);
        assert!(pcm.iter().any(|&s| s != 0));
    }

    #[test]
    fn zero_gain_is_silent() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let mut k = SoundPlayer::new(Box::new(Capture(got.clone()))).unwrap();
        k.click(Click::Delete, 0.0).unwrap();
        assert!(got.lock().unwrap_or_else(|p| p.into_inner()).iter().all(|&s| s == 0));
    }

    #[test]
    fn an_alert_plays_its_tone_through_the_stream() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let mut k = SoundPlayer::new(Box::new(Capture(got.clone()))).unwrap();
        k.alert(1, 1.0).unwrap();
        let pcm = got.lock().unwrap_or_else(|p| p.into_inner());
        // Chime is 900 ms: 43200 frames of stereo
        assert_eq!(pcm.len(), 43_200 * 2);
        assert!(pcm.iter().any(|&s| s != 0));
    }

    #[test]
    fn an_earcon_plays_through_the_stream() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let mut k = SoundPlayer::new(Box::new(Capture(got.clone()))).unwrap();
        k.earcon(0, 1.0).unwrap();
        assert!(got.lock().unwrap_or_else(|p| p.into_inner()).iter().any(|&s| s != 0));
        assert!(k.earcon(earcon::count(), 1.0).is_err());
    }

    #[test]
    fn an_unknown_alert_is_an_error_and_writes_nothing() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let mut k = SoundPlayer::new(Box::new(Capture(got.clone()))).unwrap();
        assert!(k.alert(ALERT_SOUNDS.len(), 1.0).is_err());
        assert!(got.lock().unwrap_or_else(|p| p.into_inner()).is_empty());
    }
}
