//! One sounding sample layer: playback position, rate, loop points and equal-power weight.

use super::manifest::{Manifest, Zone};
use crate::assets::Audio;
use std::sync::Arc;

pub struct Layer {
    pub audio: Arc<Audio>,
    pub pos: f64,
    /// current playback rate (`base` times the voice's glide ratio)
    pub rate: f64,
    /// playback rate for the note's pitch, without glide
    pub base: f64,
    /// equal-power weight times the zone gain
    pub weight: f32,
    /// loop in frames (start, end), when the zone loops
    pub lp: Option<(f64, f64)>,
    /// fade gain, moves one `step` per frame toward `target` (release-tail crossfades)
    pub gain: f32,
    pub target: f32,
    pub step: f32,
}

impl Layer {
    pub fn new(audio: Arc<Audio>, rate: f64, weight: f32, lp: Option<(f64, f64)>) -> Self {
        Layer { audio, pos: 0.0, rate, base: rate, weight, lp, gain: 1.0, target: 1.0, step: 0.0 }
    }

    /// Moves the fade gain one step toward its target.
    pub fn tick_gain(&mut self) {
        if self.gain < self.target {
            self.gain = (self.gain + self.step).min(self.target);
        } else if self.gain > self.target {
            self.gain = (self.gain - self.step).max(self.target);
        }
    }

    /// On note-off, a looped layer whose recorded decay after the loop end is longer than 0.15 s fades out, and the
    /// returned layer plays that decay from the loop end over 30 ms (`releaseTail` in multisample.js).
    pub fn release_tail(&mut self, sr: f64) -> Option<Layer> {
        let (_, b) = self.lp?;
        if self.len() - b <= 0.15 * sr {
            return None;
        }
        let step = (1.0 / (0.03 * sr)) as f32;
        self.target = 0.0;
        self.step = step;
        Some(Layer { audio: self.audio.clone(), pos: b, rate: self.rate, base: self.base, weight: self.weight, lp: None, gain: 0.0, target: 1.0, step })
    }
    pub fn len(&self) -> f64 {
        self.audio.channels[0].len() as f64
    }

    /// Linear-interpolated sample of channel `c` at the current position.
    pub fn sample(&self, c: usize) -> f32 {
        let ch = &self.audio.channels[c.min(self.audio.channels.len() - 1)];
        let i0 = self.pos.floor() as usize;
        if i0 >= ch.len() {
            return 0.0;
        }
        let a = ch[i0];
        let b = ch.get(i0 + 1).copied().unwrap_or(a);
        a + (b - a) * (self.pos - i0 as f64) as f32
    }

    pub fn advance(&mut self) {
        self.pos += self.rate;
        if let Some((a, b)) = self.lp {
            if self.pos >= b {
                self.pos = a + (self.pos - b) % (b - a).max(1.0);
            }
        }
    }

    pub fn finished(&self) -> bool {
        self.lp.is_none() && self.pos >= self.len()
    }
}

/// Loop points in frames (`loopOf` in multisample.js). `sr` is the engine sample rate.
pub fn loop_frames(man: &Manifest, sr: f64, z: &Zone, audio: &Audio) -> Option<(f64, f64)> {
    let (mut a, mut b) = (z.loop_start?, z.loop_end?);
    let dur = audio.channels[0].len() as f64 / sr;
    if !(b > a) {
        return None;
    }
    if man.loop_units_samples || b > dur + 1e-3 {
        a /= man.manifest_sr;
        b /= man.manifest_sr;
    }
    if !(b <= dur + 1e-3) || b - a < 0.05 {
        return None;
    }
    Some((a * sr, b.min(dur) * sr))
}

/// Equal-power blend of the audio before the loop start into the loop end (`makeXf` in multisample.js), so the
/// loop seam does not click. `a`, `b` are loop points in frames. None when the blend does not fit.
pub fn seam(audio: &Audio, a: f64, b: f64, sr: f64) -> Option<Vec<Vec<f32>>> {
    let len = audio.channels[0].len();
    let l = (b - a).round() as usize;
    let e = (b.round().max(0.0) as usize).min(len);
    let n = (0.1 * sr).min((b - a) / 3.0).min(a).floor().max(0.0) as usize;
    if n < 32 || e < n + l {
        return None;
    }
    let mut out = audio.channels.clone();
    for ch in out.iter_mut() {
        for i in 0..n {
            let j = e - n + i;
            let t = std::f64::consts::FRAC_PI_2 * (i + 1) as f64 / n as f64;
            let (c, s) = (t.cos() as f32, t.sin() as f32);
            ch[j] = ch[j] * c + ch[j - l] * s;
        }
    }
    Some(out)
}
