//! One sounding note of a sampled instrument: its layers, envelope, filter, pan, glide and pedal state.

use super::layer::Layer;
use crate::dsp::Biquad;

/// A pitch slide of a held voice: the rate ratio goes from `from` to `to` over `len` frames from `start`,
/// shaped by `k` (`glidecurve`: 1 = geometric, <1 early, >1 late).
#[derive(Clone, Copy, Debug)]
pub struct Glide {
    pub from: f64,
    pub to: f64,
    pub start: u64,
    pub len: f64,
    pub k: f64,
}

pub struct Voice {
    pub id: u64,
    pub layers: Vec<Layer>,
    pub start: u64,
    /// frame of the note-off (None = one-shot, or held by the pedal)
    pub release_at: Option<u64>,
    pub released: bool,
    /// per-frame release multiplier (exp decay toward silence, time constant release/4)
    pub rel_coeff: f32,
    pub atk_frames: f32,
    pub peak: f32,
    pub env: f32,
    pub choke: Option<String>,
    pub filter: Option<[Biquad; 2]>,
    pub pan: (f32, f32),
    pub done: bool,
    /// MIDI pitch the voice sounds at now (glide target for the next legato note)
    pub pitch: f64,
    /// zone index of the voice's first layer (glide only into the same zone)
    pub zone: usize,
    /// rate multiplier applied to every layer's base rate
    pub ratio: f64,
    pub glide: Option<Glide>,
    /// sustain pedal holds the voice: no release until the pedal lifts or `pedal_cap` is reached
    pub pedal: bool,
    pub pedal_cap: u64,
    /// release multiplier used when the pedal lifts (pedalRelease / 4)
    pub pedal_coeff: f32,
    /// damper thump sounding after note-off
    pub thump: Option<Thump>,
    /// articulation and expression shape (vibrato, tremolo, marcato, decays)
    pub shape: Shape,
    /// a legato transition's old voice: fades without release tail or thump
    pub silent: bool,
    /// brass filter sweep: (full cutoff Hz, attack seconds)
    pub brass: Option<(f64, f64)>,
    /// legato crossfade: the envelope rises on an equal-power curve
    pub eq_rise: bool,
}

/// Amplitude and pitch shaping that is not in the envelope: vibrato (cents, phase per frame), tremolo, marcato
/// overshoot, and a decay from a given frame (pizzicato, one-shot kit decay).
#[derive(Clone, Copy, Debug, Default)]
pub struct Shape {
    pub vib_cents: f64,
    pub vib_phase: f64,
    pub vib_inc: f64,
    pub trem_phase: f64,
    pub trem_inc: f64,
    pub marc: bool,
    pub decay_start: u64,
    /// time constant in frames; 0 = no decay
    pub decay_tau: f32,
}

impl Shape {
    /// Amplitude multiplier at frame `f` of a voice that started at `start`.
    pub fn mult(&mut self, f: u64, start: u64, sr: f64) -> f32 {
        let mut m = 1.0f64;
        if self.trem_inc > 0.0 {
            m *= 0.7 + 0.3 * self.trem_phase.sin();
            self.trem_phase += self.trem_inc;
        }
        if self.marc {
            let t = (f - start) as f64 / sr;
            m *= if t < 0.08 { 1.6 } else { 0.8 + 0.8 * (-(t - 0.08) / 0.04).exp() };
        }
        if self.decay_tau > 0.0 && f >= self.decay_start {
            m *= (-((f - self.decay_start) as f32) / self.decay_tau).exp() as f64;
        }
        m as f32
    }
}

/// Damper thump on note-off (`releaseNoise` in multisample.js): low-passed noise with a 4 ms attack and an 18 ms
/// exponential decay, 90 ms long. The noise is a per-voice xorshift, not the shared buffer of the web engine.
#[derive(Clone, Copy, Debug)]
pub struct Thump {
    pub level: f32,
    pub t: f32,
    pub lp: f32,
    pub k: f32,
    pub seed: u32,
}

impl Thump {
    pub fn next(&mut self, sr: f32) -> f32 {
        let t = self.t;
        self.t += 1.0;
        let atk = 0.004 * sr;
        let dec_start = 0.006 * sr;
        let tau = 0.018 * sr;
        let env = if t < atk { t / atk } else { (-(t - dec_start).max(0.0) / tau).exp() };
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        let x = (self.seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
        self.lp += self.k * (x - self.lp);
        self.lp * env * self.level
    }

    pub fn done(&self, sr: f32) -> bool {
        self.t > 0.09 * sr
    }
}

impl Voice {
    /// Note-off: looped layers start their release tails, and the damper thump sounds (`noise` = manifest
    /// releaseNoise as (gain, tone Hz)).
    pub fn begin_release(&mut self, sr: f64, noise: Option<(f64, f64)>) {
        if self.silent {
            return;
        }
        let tails: Vec<Layer> = self.layers.iter_mut().filter_map(|l| l.release_tail(sr)).collect();
        self.layers.extend(tails);
        if let Some((gain, tone)) = noise {
            let level = gain * self.peak as f64 * (1.0 - (self.pitch - 21.0) / 88.0).clamp(0.0, 1.0);
            let k = 1.0 - (-2.0 * std::f64::consts::PI * tone / sr).exp();
            self.thump = Some(Thump {
                level: level as f32,
                t: 0.0,
                lp: 0.0,
                k: k as f32,
                seed: (self.id as u32).wrapping_mul(2_654_435_761) | 1,
            });
        }
    }
}
