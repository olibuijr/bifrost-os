//! Shared DSP primitives for the DAW devices: filters, delay lines, oscillators, envelopes, PRNG, smoothing.
//! Written against the same semantics as the Web Audio nodes the mobile DAW used (BiquadFilter RBJ cookbook
//! coefficients, DelayNode, linear interpolation), so the ported devices keep their behaviour.

use std::f32::consts::TAU;

pub fn mtof(midi: f64) -> f64 {
    440.0 * 2f64.powf((midi - 69.0) / 12.0)
}

pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn gain_to_db(g: f32) -> f32 {
    if g <= 0.0 {
        -200.0
    } else {
        20.0 * g.log10()
    }
}

/// Equal-power pan law (WebAudio StereoPannerNode): pan -1..1 -> (left, right) gains.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let x = (pan.clamp(-1.0, 1.0) + 1.0) * 0.25 * std::f32::consts::PI;
    (x.cos(), x.sin())
}

/// Deterministic xorshift PRNG (same seed, same sequence).
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform 0..1.
    pub fn unit(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32) / (1u64 << 24) as f32
    }
    /// Uniform -1..1.
    pub fn bipolar(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

/// One-pole smoother, the Rust counterpart of `param.setTargetAtTime(v, t, tc)` used by the web `ramp()`.
#[derive(Clone, Copy)]
pub struct Smooth {
    value: f32,
    target: f32,
    coeff: f32,
}

impl Smooth {
    pub fn new(sr: f32, init: f32, time_constant_s: f32) -> Self {
        Smooth { value: init, target: init, coeff: 1.0 - (-1.0 / (time_constant_s * sr)).exp() }
    }
    pub fn set(&mut self, target: f32) {
        self.target = target;
    }
    pub fn snap(&mut self, value: f32) {
        self.value = value;
        self.target = value;
    }
    pub fn tick(&mut self) -> f32 {
        self.value += (self.target - self.value) * self.coeff;
        self.value
    }
    pub fn value(&self) -> f32 {
        self.value
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FilterKind {
    Lowpass,
    Highpass,
    Bandpass,
    Notch,
    Allpass,
    Peaking,
    Lowshelf,
    Highshelf,
}

/// Direct-form-I biquad with RBJ cookbook coefficients.
///
/// WebAudio's lowpass/highpass `Q` is a resonance in dB (linear Q = 10^(Q/20)); `set` does that conversion for
/// those two kinds. Peaking and shelving use `gain_db`. The ported Karplus-Strong loop depends on Q <= 0 dB here,
/// so keep that in mind when using it in feedback.
#[derive(Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Default for Biquad {
    fn default() -> Self {
        Biquad { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }
}

impl Biquad {
    pub fn new(kind: FilterKind, sr: f32, freq: f32, q: f32, gain_db: f32) -> Self {
        let mut b = Biquad::default();
        b.set(kind, sr, freq, q, gain_db);
        b
    }

    pub fn set(&mut self, kind: FilterKind, sr: f32, freq: f32, q: f32, gain_db: f32) {
        let nyquist = sr * 0.5;
        let f = freq.clamp(1.0, nyquist * 0.999);
        let q_lin = match kind {
            FilterKind::Lowpass | FilterKind::Highpass => 10f32.powf(q / 20.0).max(0.0001),
            _ => q.max(0.0001),
        };
        let w0 = TAU * f / sr;
        let (sin, cos) = (w0.sin(), w0.cos());
        let alpha = sin / (2.0 * q_lin);
        let a = 10f32.powf(gain_db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            FilterKind::Lowpass => {
                ((1.0 - cos) * 0.5, 1.0 - cos, (1.0 - cos) * 0.5, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            FilterKind::Highpass => {
                ((1.0 + cos) * 0.5, -(1.0 + cos), (1.0 + cos) * 0.5, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            FilterKind::Bandpass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterKind::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterKind::Allpass => (1.0 - alpha, -2.0 * cos, 1.0 + alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterKind::Peaking => (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            FilterKind::Lowshelf => {
                let sq = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + sq),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - sq),
                    (a + 1.0) + (a - 1.0) * cos + sq,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - sq,
                )
            }
            FilterKind::Highshelf => {
                let sq = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + sq),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - sq),
                    (a + 1.0) - (a - 1.0) * cos + sq,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - sq,
                )
            }
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

/// One-pole lowpass (exponential smoothing).
#[derive(Clone, Copy, Default)]
pub struct OnePole {
    a: f32,
    z: f32,
}

impl OnePole {
    pub fn lowpass(sr: f32, freq: f32) -> Self {
        let a = (-TAU * freq.min(sr * 0.45) / sr).exp();
        OnePole { a, z: 0.0 }
    }
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.z = x * (1.0 - self.a) + self.z * self.a;
        self.z
    }
}

/// Fractional delay line with linear interpolation (DelayNode).
#[derive(Clone)]
pub struct Delay {
    buf: Vec<f32>,
    w: usize,
}

impl Delay {
    /// `max_seconds` bounds the delay that can be read.
    pub fn new(sr: f32, max_seconds: f32) -> Self {
        let n = ((sr * max_seconds).ceil() as usize + 4).max(8);
        Delay { buf: vec![0.0; n], w: 0 }
    }
    #[inline]
    pub fn write(&mut self, x: f32) {
        self.buf[self.w] = x;
        self.w = (self.w + 1) % self.buf.len();
    }
    /// Read `delay` samples behind the write head (0 = the sample written last).
    #[inline]
    pub fn read(&self, delay: f32) -> f32 {
        let n = self.buf.len() as f32;
        let d = delay.clamp(0.0, n - 2.0);
        let pos = self.w as f32 - 1.0 - d;
        let pos = if pos < 0.0 { pos + n } else { pos };
        let i = pos.floor() as usize % self.buf.len();
        let j = (i + 1) % self.buf.len();
        let frac = pos - pos.floor();
        self.buf[i] * (1.0 - frac) + self.buf[j] * frac
    }
    pub fn clear(&mut self) {
        self.buf.iter_mut().for_each(|x| *x = 0.0);
    }
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wave {
    Sine,
    Triangle,
    Saw,
    Square,
}

/// Band-limited (PolyBLEP) oscillator at a given frequency.
#[derive(Clone, Copy)]
pub struct Osc {
    phase: f32,
    inc: f32,
    sr: f32,
}

impl Osc {
    pub fn new(sr: f32, freq: f32) -> Self {
        Osc { phase: 0.0, inc: freq / sr, sr }
    }
    pub fn set_freq(&mut self, freq: f32) {
        self.inc = freq / self.sr;
    }
    pub fn freq(&self) -> f32 {
        self.inc * self.sr
    }
    pub fn set_phase(&mut self, phase: f32) {
        self.phase = phase.rem_euclid(1.0);
    }
    fn blep(t: f32, dt: f32) -> f32 {
        if t < dt {
            let t = t / dt;
            2.0 * t - t * t - 1.0
        } else if t > 1.0 - dt {
            let t = (t - 1.0) / dt;
            t * t + 2.0 * t + 1.0
        } else {
            0.0
        }
    }
    #[inline]
    pub fn tick(&mut self, wave: Wave) -> f32 {
        let t = self.phase;
        let dt = self.inc.abs().max(1e-9);
        let out = match wave {
            Wave::Sine => (t * TAU).sin(),
            Wave::Triangle => {
                if t < 0.5 {
                    4.0 * t - 1.0
                } else {
                    3.0 - 4.0 * t
                }
            }
            Wave::Saw => (2.0 * t - 1.0) - Self::blep(t, dt),
            Wave::Square => {
                let sq = if t < 0.5 { 1.0 } else { -1.0 };
                sq + Self::blep(t, dt) - Self::blep((t + 0.5).rem_euclid(1.0), dt)
            }
        };
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        } else if self.phase < 0.0 {
            self.phase += 1.0;
        }
        out
    }
}

/// Sine LFO, returns -1..1.
#[derive(Clone, Copy)]
pub struct Lfo {
    osc: Osc,
}

impl Lfo {
    pub fn new(sr: f32, rate_hz: f32) -> Self {
        Lfo { osc: Osc::new(sr, rate_hz) }
    }
    pub fn set_rate(&mut self, rate_hz: f32) {
        self.osc.set_freq(rate_hz);
    }
    #[inline]
    pub fn tick(&mut self) -> f32 {
        self.osc.tick(Wave::Sine)
    }
}

/// Linear attack, linear decay to sustain, exponential release (time constant `release/5`, ~-43 dB at the end).
#[derive(Clone, Copy, Debug)]
pub struct Adsr {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl Adsr {
    /// Envelope level `t` seconds after note-on, with the note held for `hold` seconds (None = still held).
    pub fn level_at(&self, t: f32, hold: Option<f32>) -> f32 {
        match hold {
            Some(h) if t >= h => {
                let r = self.release.max(1e-4);
                self.level_held(h) * (-5.0 * (t - h) / r).exp()
            }
            _ => self.level_held(t),
        }
    }

    fn level_held(&self, t: f32) -> f32 {
        let attack = self.attack.max(1e-4);
        let decay = self.decay.max(1e-4);
        if t < attack {
            t / attack
        } else if t < attack + decay {
            1.0 + (self.sustain - 1.0) * ((t - attack) / decay)
        } else {
            self.sustain
        }
    }
}

/// Stereo-linked level detector in dB (max of the two channels, like WebAudio's compressor).
#[derive(Clone, Copy)]
pub struct Compressor {
    pub threshold: f32,
    pub ratio: f32,
    pub knee: f32,
    pub attack: f32,
    pub release: f32,
    sr: f32,
    reduction_db: f32,
}

impl Compressor {
    pub fn new(sr: f32) -> Self {
        Compressor { threshold: -24.0, ratio: 12.0, knee: 30.0, attack: 0.003, release: 0.25, sr, reduction_db: 0.0 }
    }

    /// Static curve: output level in dB for an input level in dB.
    pub fn curve(&self, x: f32) -> f32 {
        let (t, r, k) = (self.threshold, self.ratio.max(1.0), self.knee.max(0.0));
        if k > 0.0 && x > t - k / 2.0 && x < t + k / 2.0 {
            let d = x - t + k / 2.0;
            x + (1.0 / r - 1.0) * d * d / (2.0 * k)
        } else if x <= t {
            x
        } else {
            t + (x - t) / r
        }
    }

    /// Returns the linear gain to apply for the stereo sample.
    pub fn gain(&mut self, l: f32, r: f32) -> f32 {
        let level = l.abs().max(r.abs()).max(1e-9);
        let x = 20.0 * level.log10();
        let target = (self.curve(x) - x).min(0.0);
        let coeff = if target < self.reduction_db { self.attack } else { self.release };
        let a = (-1.0 / (coeff.max(1e-4) * self.sr)).exp();
        self.reduction_db = target + (self.reduction_db - target) * a;
        db_to_gain(self.reduction_db)
    }
}

/// Soft clip with a tanh knee (the master safety curve in the web app: linear to 0.7, tanh into 0.966).
pub fn safety_clip(x: f32) -> f32 {
    let t = 0.7f32;
    let top = 0.966f32;
    let a = x.abs();
    if a <= t {
        x
    } else {
        x.signum() * (t + (top - t) * ((a - t) / (top - t)).tanh())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mtof_a4_is_440() {
        assert!((mtof(69.0) - 440.0).abs() < 1e-9);
    }

    #[test]
    fn lowpass_passes_dc() {
        let mut f = Biquad::new(FilterKind::Lowpass, 48_000.0, 1_000.0, 0.0, 0.0);
        let mut y = 0.0;
        for _ in 0..2000 {
            y = f.process(1.0);
        }
        assert!((y - 1.0).abs() < 1e-3, "dc gain {y}");
    }

    #[test]
    fn delay_reads_back_what_was_written() {
        let mut d = Delay::new(48_000.0, 0.1);
        for i in 0..10 {
            d.write(i as f32);
        }
        assert!((d.read(0.0) - 9.0).abs() < 1e-6);
        assert!((d.read(3.0) - 6.0).abs() < 1e-6);
    }

    #[test]
    fn rng_is_deterministic() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn safety_clip_is_unity_below_knee() {
        assert_eq!(safety_clip(0.5), 0.5);
        assert!(safety_clip(2.0) < 0.966 + 1e-6);
    }
}
