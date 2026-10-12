//! System sounds (task 13): short sounds (alerts, clicks) played over the sound-effects role. The player holds
//! registered sounds already converted to the output's rate and channel count and mixes the playing voices into
//! the sound-effects stream's buffer with [`SystemSounds::mix_into`], which is real-time safe (voices live in a
//! vector sized at construction). Registering and playing happen off the real-time thread; handing the player to the
//! IO callback (a try-lock or a command ring) is the caller's job. The sound-effects role's device and the ducking
//! of other streams are the policy crate's.

use crate::converter::{apply_matrix, default_matrix};
use crate::file::{self, AudioFile};
use crate::resample::{resample, Quality};
use bifrost_audio_types::AudioError;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// The role system sounds play on (the contract's third default device).
pub const ROLE: &str = "sound-effects";
/// The id of the built-in alert tone.
pub const ALERT: u32 = 0;
/// Sounds that can play at once; a further play replaces nothing and is refused.
pub const MAX_VOICES: usize = 16;

struct Voice {
    sound: Arc<[f32]>,
    cursor: usize,
    gain: f32,
}

pub struct SystemSounds {
    rate: u32,
    channels: usize,
    sounds: HashMap<u32, Arc<[f32]>>,
    voices: Vec<Voice>,
}

impl SystemSounds {
    /// A player for an output of `rate` Hz and `channels` channels, with the built-in alert registered.
    pub fn new(rate: u32, channels: u16) -> Result<SystemSounds, AudioError> {
        if rate == 0 || channels == 0 {
            return Err(AudioError::BadValue);
        }
        let mut s = SystemSounds { rate, channels: channels as usize, sounds: HashMap::new(), voices: Vec::with_capacity(MAX_VOICES) };
        let tone = alert_tone(rate);
        s.register_samples(ALERT, rate, 1, &tone)?;
        Ok(s)
    }

    /// Registers (or replaces) a sound given as interleaved floats at its own rate and channel count; it is
    /// converted to the output's rate and channels now.
    pub fn register_samples(&mut self, id: u32, rate: u32, channels: u16, samples: &[f32]) -> Result<(), AudioError> {
        let ch = channels as usize;
        if rate == 0 || ch == 0 || samples.is_empty() || samples.len() % ch != 0 {
            return Err(AudioError::BadValue);
        }
        let mut mixed = Vec::new();
        apply_matrix(&default_matrix(ch, self.channels), ch, self.channels, samples, &mut mixed);
        let out = if rate == self.rate { mixed } else { resample(&mixed, rate, self.rate, self.channels, Quality::Standard)? };
        self.sounds.insert(id, out.into());
        Ok(())
    }

    /// Registers a sound from an audio file (WAV, AIFF or CAF).
    pub fn register_file(&mut self, id: u32, path: &Path) -> Result<(), AudioError> {
        let AudioFile { rate, channels, samples, .. } = file::read(path)?;
        self.register_samples(id, rate, channels, &samples)
    }

    pub fn unregister(&mut self, id: u32) -> bool {
        self.sounds.remove(&id).is_some()
    }

    pub fn contains(&self, id: u32) -> bool {
        self.sounds.contains_key(&id)
    }

    /// Frames in a registered sound at the output rate.
    pub fn frames(&self, id: u32) -> Option<usize> {
        self.sounds.get(&id).map(|s| s.len() / self.channels)
    }

    /// Starts a sound at `gain` (0 through 1). Unknown ids give `UnknownObject`; a full voice list gives
    /// `BadValue`.
    pub fn play(&mut self, id: u32, gain: f32) -> Result<(), AudioError> {
        let sound = self.sounds.get(&id).ok_or(AudioError::UnknownObject)?.clone();
        if self.voices.len() >= MAX_VOICES {
            return Err(AudioError::BadValue);
        }
        self.voices.push(Voice { sound, cursor: 0, gain: if gain.is_nan() { 0.0 } else { gain.clamp(0.0, 1.0) } });
        Ok(())
    }

    /// Sounds playing now.
    pub fn playing(&self) -> usize {
        self.voices.len()
    }

    /// Stops every voice.
    pub fn stop_all(&mut self) {
        self.voices.clear();
    }

    /// Adds the playing sounds into `out` (interleaved, whole frames) and drops the ones that finished. Real-time
    /// safe: no allocation (dropping a voice releases one reference; the sound itself stays registered).
    pub fn mix_into(&mut self, out: &mut [f32]) {
        let ch = self.channels;
        let want = out.len() / ch * ch;
        self.voices.retain_mut(|v| {
            let left = v.sound.len() - v.cursor;
            let n = left.min(want);
            for (o, s) in out[..n].iter_mut().zip(&v.sound[v.cursor..v.cursor + n]) {
                *o += s * v.gain;
            }
            v.cursor += n;
            v.cursor < v.sound.len()
        });
    }
}

/// The built-in alert: 880 Hz, 150 ms, a 10 ms linear fade in and a 40 ms fade out, peak 0.25.
fn alert_tone(rate: u32) -> Vec<f32> {
    let n = (rate as u64 * 150 / 1000) as usize;
    let fade_in = (rate as u64 * 10 / 1000) as usize;
    let fade_out = (rate as u64 * 40 / 1000) as usize;
    (0..n)
        .map(|i| {
            let env_in = if i < fade_in { i as f32 / fade_in as f32 } else { 1.0 };
            let env_out = if n - i <= fade_out { (n - i) as f32 / fade_out as f32 } else { 1.0 };
            let t = i as f64 / rate as f64;
            0.25 * env_in * env_out * (2.0 * std::f64::consts::PI * 880.0 * t).sin() as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_alert_is_built_in() {
        let s = SystemSounds::new(48000, 2).unwrap();
        // 150 ms at 48 kHz
        assert_eq!(s.frames(ALERT), Some(7200));
        assert!(s.contains(ALERT));
        assert_eq!(ROLE, "sound-effects");
    }

    #[test]
    fn alert_tone_shape() {
        let t = alert_tone(48000);
        assert_eq!(t.len(), 7200);
        assert_eq!(t[0], 0.0);
        assert!(t.iter().all(|v| v.abs() <= 0.25 + 1e-6));
        // the last frame is nearly silent (fade out reaches 1/1920 of the peak)
        assert!(t[7199].abs() < 0.001);
        // loud in the middle: the peak of a 880 Hz cycle is reached somewhere
        assert!(t[1000..2000].iter().any(|v| v.abs() > 0.24));
    }

    #[test]
    fn sounds_mix_additively_and_finish() {
        let mut s = SystemSounds::new(48000, 2).unwrap();
        s.register_samples(1, 48000, 2, &[0.1, 0.1, 0.2, 0.2, 0.3, 0.3]).unwrap();
        s.play(1, 1.0).unwrap();
        s.play(1, 0.5).unwrap();
        assert_eq!(s.playing(), 2);
        let mut out = [1.0; 4]; // two frames over an existing signal of 1.0
        s.mix_into(&mut out);
        // 1 + 0.1 + 0.05, 1 + 0.2 + 0.1 (in f32 arithmetic)
        let want = [1.0 + 0.1 + 0.05, 1.0 + 0.1 + 0.05, 1.0 + 0.2 + 0.1, 1.0 + 0.2 + 0.1];
        for (a, b) in out.iter().zip(want) {
            assert!((a - b).abs() < 1e-6, "{a} vs {b}");
        }
        assert_eq!(s.playing(), 2);
        let mut out = [0.0; 4]; // one frame left, then silence
        s.mix_into(&mut out);
        assert!((out[0] - 0.45).abs() < 1e-6 && out[2] == 0.0);
        assert_eq!(s.playing(), 0);
    }

    #[test]
    fn mono_sounds_are_widened_and_rates_converted() {
        let mut s = SystemSounds::new(48000, 2).unwrap();
        s.register_samples(2, 48000, 1, &[0.5, -0.5]).unwrap();
        s.play(2, 1.0).unwrap();
        let mut out = [0.0; 4];
        s.mix_into(&mut out);
        assert_eq!(out, [0.5, 0.5, -0.5, -0.5]);
        // 441 frames at 44.1 kHz are 480 at 48 kHz
        s.register_samples(3, 44100, 1, &vec![0.1; 441]).unwrap();
        assert_eq!(s.frames(3), Some(480));
    }

    #[test]
    fn errors() {
        let mut s = SystemSounds::new(48000, 2).unwrap();
        assert_eq!(s.play(99, 1.0), Err(AudioError::UnknownObject));
        assert_eq!(s.register_samples(5, 48000, 2, &[0.0; 3]), Err(AudioError::BadValue));
        assert_eq!(s.register_samples(5, 0, 2, &[0.0; 2]), Err(AudioError::BadValue));
        for _ in 0..MAX_VOICES {
            s.play(ALERT, 1.0).unwrap();
        }
        assert_eq!(s.play(ALERT, 1.0), Err(AudioError::BadValue));
        s.stop_all();
        assert_eq!(s.playing(), 0);
        assert!(s.unregister(ALERT));
        assert!(!s.unregister(ALERT));
        assert!(SystemSounds::new(0, 2).is_err());
    }

    #[test]
    fn files_register_from_disk() {
        let dir = std::env::temp_dir().join(format!("bifrost-sysound-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("click.wav");
        let f = AudioFile { container: file::Container::Wav, rate: 48000, channels: 1, sample: crate::converter::SampleFormat::S16, samples: vec![0.5; 10] };
        file::write(&p, &f).unwrap();
        let mut s = SystemSounds::new(48000, 2).unwrap();
        s.register_file(7, &p).unwrap();
        assert_eq!(s.frames(7), Some(10));
        assert_eq!(s.register_file(8, &dir.join("none.wav")), Err(AudioError::Io));
        std::fs::remove_dir_all(&dir).ok();
    }
}
