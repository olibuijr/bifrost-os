//! Subtractive synths: Pulse (`synth`), Pad, Supersaw, Bass, Reese and Wobble. One voice engine with per-device
//! mapping of the web plugins' parameters. Parameter names, ranges and defaults are the same as the web specs.
//!
//! Approximations (documented in the CHANGELOG): the web plugins build their graphs from Web Audio nodes, so the
//! PWM, glide, the 303-style accent and slide, the Reese beat LFO on detune and the wobble tempo sync are
//! reproduced by their closest native equivalents rather than sample-for-sample.

use crate::devices::{p, Ctx, Curve, Instrument, Params, P};
use crate::dsp::{mtof, Adsr, Biquad, Delay, FilterKind, Osc, Rng, Wave};

const LIN: Curve = Curve::Lin;
const LOG: Curve = Curve::Log;
const INT: Curve = Curve::Int;

const SYNTH: &[P] = &[
    p("wave", 0.0, 3.0, 2.0, INT, ""),
    p("voices", 1.0, 7.0, 1.0, INT, ""),
    p("detune", 0.0, 50.0, 8.0, LIN, "ct"),
    p("width", 0.0, 1.0, 0.0, LIN, ""),
    p("octave", -2.0, 2.0, 0.0, INT, ""),
    p("sub", 0.0, 1.0, 0.0, LIN, ""),
    p("osc2", 0.0, 4.0, 0.0, INT, ""),
    p("semi2", -24.0, 24.0, 0.0, INT, "st"),
    p("mix2", 0.0, 1.0, 0.5, LIN, ""),
    p("pwm", 0.0, 1.0, 0.0, LIN, ""),
    p("pwmRate", 0.1, 8.0, 0.8, LOG, "Hz"),
    p("noise", 0.0, 1.0, 0.0, LIN, ""),
    p("cutoff", 40.0, 16000.0, 2400.0, LOG, "Hz"),
    p("reso", 0.0, 20.0, 2.0, LIN, ""),
    p("fType", 0.0, 3.0, 0.0, INT, ""),
    p("envAmt", 0.0, 1.0, 0.4, LIN, ""),
    p("legacyEnv", 0.0, 1.0, 1.0, INT, ""),
    p("fAtk", 0.001, 2.0, 0.003, LOG, "s"),
    p("fDec", 0.01, 3.0, 0.25, LOG, "s"),
    p("fSus", 0.0, 1.0, 0.3, LIN, ""),
    p("keyTrk", 0.0, 1.0, 0.0, LIN, ""),
    p("vel2cut", 0.0, 1.0, 0.0, LIN, ""),
    p("attack", 0.001, 2.0, 0.005, LOG, "s"),
    p("decay", 0.01, 2.0, 0.25, LOG, "s"),
    p("sustain", 0.0, 1.0, 0.6, LIN, ""),
    p("release", 0.01, 4.0, 0.2, LOG, "s"),
    p("glide", 0.0, 0.5, 0.0, LIN, "s"),
    p("gain", 0.0, 1.0, 0.6, LIN, ""),
];

const PAD: &[P] = &[
    p("voices", 3.0, 7.0, 7.0, INT, ""),
    p("detune", 0.0, 60.0, 22.0, LIN, "ct"),
    p("spread", 0.0, 1.0, 0.8, LIN, ""),
    p("octave", -2.0, 2.0, 0.0, INT, ""),
    p("cutoff", 100.0, 12000.0, 2200.0, LOG, "Hz"),
    p("reso", 0.0, 12.0, 1.0, LIN, ""),
    p("lfoRate", 0.02, 4.0, 0.15, LOG, "Hz"),
    p("lfoAmt", 0.0, 1.0, 0.35, LIN, ""),
    p("attack", 0.005, 6.0, 0.9, LOG, "s"),
    p("release", 0.05, 8.0, 1.8, LOG, "s"),
    p("gain", 0.0, 1.0, 0.6, LIN, ""),
    p("decay", 0.05, 8.0, 1.0, LOG, "s"),
    p("sustain", 0.0, 1.0, 1.0, LIN, ""),
    p("fEnv", -1.0, 1.0, 0.0, LIN, ""),
    p("fAtk", 0.01, 8.0, 1.5, LOG, "s"),
    p("air", 0.0, 1.0, 0.0, LIN, ""),
    p("shape", 0.0, 2.0, 0.0, INT, ""),
];

const SUPERSAW: &[P] = &[
    p("voices", 1.0, 9.0, 7.0, INT, ""),
    p("detune", 0.0, 1.0, 0.35, LIN, ""),
    p("mix", 0.0, 1.0, 0.6, LIN, ""),
    p("spread", 0.0, 1.0, 0.7, LIN, ""),
    p("shape", 0.0, 3.0, 0.0, INT, ""),
    p("octave", -2.0, 2.0, 0.0, INT, ""),
    p("cutoff", 60.0, 16000.0, 3000.0, LOG, "Hz"),
    p("reso", 0.0, 20.0, 1.5, LIN, ""),
    p("keytrack", 0.0, 1.0, 0.5, LIN, ""),
    p("vel2cut", 0.0, 1.0, 0.4, LIN, ""),
    p("fAmt", 0.0, 1.0, 0.4, LIN, ""),
    p("fAtk", 0.001, 2.0, 0.005, LOG, "s"),
    p("fDec", 0.01, 3.0, 0.4, LOG, "s"),
    p("fSus", 0.0, 1.0, 0.3, LIN, ""),
    p("attack", 0.002, 2.0, 0.005, LOG, "s"),
    p("decay", 0.01, 3.0, 0.5, LOG, "s"),
    p("sustain", 0.0, 1.0, 0.8, LIN, ""),
    p("release", 0.01, 4.0, 0.3, LOG, "s"),
    p("vibrato", 0.0, 1.0, 0.0, LIN, ""),
    p("vibRate", 2.0, 9.0, 5.5, LIN, "Hz"),
    p("vibDelay", 0.0, 2.0, 0.3, LIN, "s"),
    p("glide", 0.0, 0.5, 0.0, LIN, "s"),
    p("velSens", 0.0, 1.0, 0.6, LIN, ""),
    p("gain", 0.0, 1.0, 0.55, LIN, ""),
];

const BASS: &[P] = &[
    p("wave", 0.0, 1.0, 0.0, INT, ""),
    p("octave", -2.0, 2.0, 0.0, INT, ""),
    p("cutoff", 40.0, 8000.0, 400.0, LOG, "Hz"),
    p("reso", 0.0, 30.0, 14.0, LIN, ""),
    p("envMod", 0.0, 1.0, 0.6, LIN, ""),
    p("decay", 0.03, 2.0, 0.25, LOG, "s"),
    p("accent", 0.0, 1.0, 0.6, LIN, ""),
    p("slide", 0.01, 0.3, 0.06, LOG, "s"),
    p("drive", 0.0, 1.0, 0.2, LIN, ""),
    p("gain", 0.0, 1.0, 0.6, LIN, ""),
    p("lfoRate", 0.0, 7.0, 0.0, INT, ""),
    p("lfoBpm", 0.0, 200.0, 0.0, LIN, "bpm"),
    p("lfoAmt", 0.0, 1.0, 0.0, LIN, ""),
    p("lfoDest", 0.0, 1.0, 0.0, INT, ""),
    p("sub", 0.0, 1.0, 0.0, LIN, ""),
];

const REESE: &[P] = &[
    p("saws", 2.0, 4.0, 2.0, INT, ""),
    p("detune", 0.0, 40.0, 14.0, LIN, "ct"),
    p("beat", 0.05, 2.0, 0.3, LOG, "Hz"),
    p("motion", 0.0, 1.0, 0.5, LIN, ""),
    p("octave", -2.0, 1.0, -1.0, INT, ""),
    p("cutoff", 60.0, 6000.0, 700.0, LOG, "Hz"),
    p("reso", 0.0, 15.0, 2.0, LIN, ""),
    p("drive", 0.0, 1.0, 0.4, LIN, ""),
    p("fmt", 0.0, 1.0, 0.0, LIN, ""),
    p("width", 0.0, 1.0, 0.5, LIN, ""),
    p("sub", 0.0, 1.0, 0.6, LIN, ""),
    p("attack", 0.002, 1.0, 0.01, LOG, "s"),
    p("release", 0.01, 2.0, 0.15, LOG, "s"),
    p("glide", 0.005, 0.4, 0.05, LOG, "s"),
    p("velSens", 0.0, 1.0, 0.4, LIN, ""),
    p("gain", 0.0, 1.0, 0.6, LIN, ""),
];

const WOBBLE: &[P] = &[
    p("rate", 0.0, 7.0, 3.0, INT, ""),
    p("shape", 0.0, 3.0, 0.0, INT, ""),
    p("bpm", 0.0, 200.0, 0.0, LIN, "bpm"),
    p("lfoAmt", 0.0, 1.0, 0.7, LIN, ""),
    p("retrig", 0.0, 1.0, 1.0, INT, ""),
    p("octave", -2.0, 1.0, -1.0, INT, ""),
    p("cutoff", 40.0, 4000.0, 180.0, LOG, "Hz"),
    p("reso", 0.0, 20.0, 8.0, LIN, ""),
    p("drive", 0.0, 1.0, 0.5, LIN, ""),
    p("sub", 0.0, 1.0, 0.5, LIN, ""),
    p("glide", 0.005, 0.3, 0.04, LOG, "s"),
    p("gain", 0.0, 1.0, 0.6, LIN, ""),
];

/// Spec for a subtractive device type.
pub fn spec(kind: &str) -> Option<&'static [P]> {
    Some(match kind {
        "synth" => SYNTH,
        "pad" => PAD,
        "supersaw" => SUPERSAW,
        "bass" => BASS,
        "reese" => REESE,
        "wobble" => WOBBLE,
        _ => return None,
    })
}

/// Builds a subtractive instrument.
pub fn new(kind: &str, params: &Params, ctx: &Ctx) -> Option<Box<dyn Instrument>> {
    let spec = spec(kind)?;
    Some(Box::new(Sub { kind: kind.to_string(), spec, p: params.clone(), sr: ctx.sr, rng: Rng::new(ctx.seed ^ 0x5B), voices: vec![], bpm: 120.0, lfo: Osc::new(ctx.sr, 1.0), last_hz: None, bend_ev: vec![], bend_cursor: 0, bend_ratio: 1.0 }))
}

/// One oscillator in a voice.
struct Oscillator {
    osc: Osc,
    wave: Wave,
    gain: f32,
    /// Pulse (synth PWM): this oscillator's saw is minus a delayed copy of itself
    pwm: Option<Delay>,
}

struct SubVoice {
    /// oscillator frequencies at the note's base pitch (glide scales them)
    hz: Vec<f32>,
    /// (from Hz, base Hz, seconds, start frame): pitch glides from `from` to `base` over `secs`
    glide: Option<(f64, f64, f64, u64)>,
    base: f64,
    start: u64,
    /// frame of note-off; the amp release follows
    off: u64,
    vel: f32,
    oscs: Vec<Oscillator>,
    amp: Adsr,
    filt: Biquad,
    filt2: Biquad,
    noise: Rng,
    noise_amt: f32,
    done: bool,
    cut: f32,
    env_amt: f32,
    fenv: Adsr,
    filter_kind: FilterKind,
    two_pole: bool,
    gain: f32,
    lfo_amt: f32,
    lfo_cut: bool,
    /// Pulse PWM LFO and depth (pwm * 0.45 of the period)
    pwm: Option<(Osc, f32)>,
    /// bass LFO (per note, restarted on each non-slide note)
    lfo: Option<Osc>,
    /// bass LFO destination is pitch (otherwise cutoff)
    lfo_pitch: bool,
    /// resonance multiplier (bass accent)
    q_scale: f32,
}

struct Sub {
    kind: String,
    spec: &'static [P],
    p: Params,
    sr: f32,
    rng: Rng,
    voices: Vec<SubVoice>,
    bpm: f64,
    lfo: Osc,
    /// base pitch of the previous note (glide source)
    last_hz: Option<f64>,
    /// pitch-bend events (frame, semitones), sorted; `bend_ratio` is applied to every oscillator
    bend_ev: Vec<(u64, f64)>,
    bend_cursor: usize,
    bend_ratio: f64,
}

impl Sub {
    fn wave_of(i: i32) -> Wave {
        match i {
            0 => Wave::Sine,
            1 => Wave::Triangle,
            2 => Wave::Saw,
            _ => Wave::Square,
        }
    }

    /// Filter cutoff for the note (key/velocity tracked where the device does that).
    fn base_cutoff(&self, pitch: f64, vel: f32) -> f32 {
        let cut = self.p.get("cutoff") as f32;
        let kt = self.p.get("keyTrk") as f32 + self.p.get("keytrack") as f32;
        let v2c = (self.p.get("vel2cut") as f32).max(self.p.get("velSens") as f32 * 0.5);
        let key = 2f32.powf(kt * (pitch as f32 - 60.0) / 12.0);
        (cut * key * (1.0 + v2c * (vel - 0.5))).clamp(20.0, self.sr * 0.45)
    }

    fn lfo_rate(&self) -> f32 {
        match self.kind.as_str() {
            "wobble" => {
                let bpm = if self.p.get("bpm") > 0.0 { self.p.get("bpm") } else { self.bpm };
                let beats = [1.0, 0.5, 0.25, 0.125, 0.125 * 2.0 / 3.0, 0.0625, 0.0625 * 2.0 / 3.0, 0.03125];
                let idx = (self.p.get("rate").round() as usize).min(beats.len() - 1);
                (bpm / 60.0 / beats[idx] / 4.0) as f32
            }
            "pad" => self.p.get("lfoRate") as f32,
            "reese" => self.p.get("beat") as f32,
            _ => 0.0,
        }
    }

    fn lfo_depth(&self) -> f32 {
        match self.kind.as_str() {
            "wobble" => self.p.get("lfoAmt") as f32,
            "pad" => self.p.get("lfoAmt") as f32,
            "bass" => self.p.get("lfoAmt") as f32,
            _ => 0.0,
        }
    }

    /// Synth PWM is on: pulse = saw minus a delayed saw (JS: pwm > 0 and wave is saw or square).
    fn pwm_on(&self) -> bool {
        self.kind == "synth" && self.p.get("pwm") > 0.0 && self.p.get("wave") >= 2.0
    }

    /// Bass accent: velocity above 0.9 accents the note by the `accent` amount.
    fn accent_of(&self, vel: f32) -> f32 {
        if self.kind == "bass" && vel > 0.9 {
            self.p.get("accent") as f32
        } else {
            0.0
        }
    }

    /// Bass LFO rate: sync division (JS DIV table, beats per cycle) at lfoBpm, or the song tempo if lfoBpm < 20.
    fn bass_lfo_hz(&self) -> f32 {
        const DIV: [f64; 8] = [0.0, 4.0, 2.0, 1.0, 0.5, 1.0 / 3.0, 0.25, 0.125];
        let bpm = if self.p.get("lfoBpm") >= 20.0 { self.p.get("lfoBpm") } else { self.bpm };
        let idx = (self.p.get("lfoRate").round().max(0.0) as usize).min(DIV.len() - 1);
        (bpm / 60.0 / DIV[idx].max(1e-6)) as f32
    }

    /// Oscillator stack for a note.
    fn build_oscs(&mut self, pitch: f64, vel: f32) -> Vec<Oscillator> {
        let base = mtof(pitch + 12.0 * self.p.get("octave")) as f32;
        let mut out = vec![];
        let rnd = |rng: &mut Rng| rng.bipolar();
        match self.kind.as_str() {
            "synth" => {
                let n = (self.p.get("voices").round() as usize).clamp(1, 7);
                let pwm = self.pwm_on();
                let wave = if pwm { Wave::Saw } else { Self::wave_of(self.p.get("wave").round() as i32) };
                let detune = self.p.get("detune") as f32;
                let gain = (self.p.get("gain") as f32 * 0.32 * vel) / (n as f32).sqrt();
                for i in 0..n {
                    let pos = if n > 1 { i as f32 / (n - 1) as f32 * 2.0 - 1.0 } else { 0.0 };
                    let cents = pos * detune;
                    // the delay line spans one period (+ margin) so the lowest note is never clamped
                    let pwm_line = if pwm { Some(Delay::new(self.sr, 1.0 / base + 0.01)) } else { None };
                    out.push(Oscillator { osc: Osc::new(self.sr, base * 2f32.powf(cents / 1200.0)), wave, gain, pwm: pwm_line });
                }
                if self.p.get("osc2") > 0.0 {
                    let w2 = Self::wave_of(self.p.get("osc2").round() as i32 - 1);
                    let r = 2f32.powf(self.p.get("semi2") as f32 / 12.0);
                    out.push(Oscillator { osc: Osc::new(self.sr, base * r), wave: w2, gain: self.p.get("mix2") as f32 * gain, pwm: None });
                }
                if self.p.get("sub") > 0.0 {
                    out.push(Oscillator { osc: Osc::new(self.sr, base * 0.5), wave: Wave::Sine, gain: self.p.get("sub") as f32 * gain, pwm: None });
                }
            }
            "pad" | "supersaw" => {
                let (n, det, mix) = if self.kind == "pad" {
                    (self.p.get("voices").round() as usize, self.p.get("detune") as f32, 1.0f32)
                } else {
                    (self.p.get("voices").round() as usize, self.p.get("detune") as f32 * 60.0, self.p.get("mix") as f32)
                };
                let n = n.clamp(1, 9);
                let shape = self.p.get("shape").round() as i32;
                let gain = self.p.get("gain") as f32 * 0.4 / (n as f32).sqrt() * mix.max(0.2);
                for i in 0..n {
                    let pos = if n > 1 { i as f32 / (n - 1) as f32 * 2.0 - 1.0 } else { 0.0 };
                    let wave = if shape == 2 || (shape == 3 && i % 2 == 1) { Wave::Square } else { Wave::Saw };
                    out.push(Oscillator { osc: Osc::new(self.sr, base * 2f32.powf(pos * det / 1200.0)), wave, gain, pwm: None });
                }
            }
            "bass" => {
                let wave = if self.p.get("wave") >= 0.5 { Wave::Square } else { Wave::Saw };
                let acc = self.accent_of(vel);
                let gain = self.p.get("gain") as f32 * 0.5 * vel * (1.0 + acc * 0.5);
                out.push(Oscillator { osc: Osc::new(self.sr, base), wave, gain, pwm: None });
                if self.p.get("sub") > 0.0 {
                    out.push(Oscillator { osc: Osc::new(self.sr, base * 0.5), wave: Wave::Sine, gain: self.p.get("sub") as f32 * gain, pwm: None });
                }
            }
            "reese" | "wobble" => {
                let saws = if self.kind == "reese" { self.p.get("saws").round() as usize } else { 2 };
                let det = if self.kind == "reese" { self.p.get("detune") as f32 } else { 8.0 };
                let gain = self.p.get("gain") as f32 * 0.35 / (saws as f32).sqrt();
                for i in 0..saws.max(1) {
                    let pos = if saws > 1 { i as f32 / (saws - 1) as f32 * 2.0 - 1.0 } else { 0.0 };
                    out.push(Oscillator { osc: Osc::new(self.sr, base * 2f32.powf(pos * det / 1200.0)), wave: Wave::Saw, gain, pwm: None });
                }
                let sub = self.p.get("sub") as f32;
                if sub > 0.0 {
                    out.push(Oscillator { osc: Osc::new(self.sr, base * 0.5), wave: Wave::Sine, gain: sub * gain, pwm: None });
                }
            }
            _ => {}
        }
        for o in out.iter_mut() {
            // a small random start phase per oscillator, so stacked saws do not line up on every note
            o.osc.set_phase(rnd(&mut self.rng).abs() * 0.5);
        }
        out
    }

    fn env_for(&self, acc: f32) -> (Adsr, Adsr, f32) {
        match self.kind.as_str() {
            "synth" => {
                let legacy = self.p.get("legacyEnv") >= 0.5;
                let amp = Adsr { attack: self.p.get("attack") as f32, decay: self.p.get("decay") as f32, sustain: self.p.get("sustain") as f32, release: self.p.get("release") as f32 };
                let fenv = if legacy {
                    Adsr { attack: self.p.get("attack") as f32, decay: self.p.get("decay") as f32, sustain: self.p.get("sustain") as f32 * 0.5, release: self.p.get("release") as f32 }
                } else {
                    Adsr { attack: self.p.get("fAtk") as f32, decay: self.p.get("fDec") as f32, sustain: self.p.get("fSus") as f32, release: self.p.get("release") as f32 }
                };
                (amp, fenv, self.p.get("envAmt") as f32)
            }
            "pad" => (
                Adsr { attack: self.p.get("attack") as f32, decay: self.p.get("decay") as f32, sustain: self.p.get("sustain") as f32, release: self.p.get("release") as f32 },
                Adsr { attack: self.p.get("fAtk") as f32, decay: 0.5, sustain: 0.5, release: 1.0 },
                self.p.get("fEnv") as f32,
            ),
            "supersaw" => (
                Adsr { attack: self.p.get("attack") as f32, decay: self.p.get("decay") as f32, sustain: self.p.get("sustain") as f32, release: self.p.get("release") as f32 },
                Adsr { attack: self.p.get("fAtk") as f32, decay: self.p.get("fDec") as f32, sustain: self.p.get("fSus") as f32, release: self.p.get("release") as f32 },
                self.p.get("fAmt") as f32,
            ),
            "bass" => {
                // accent shortens the decay and raises the filter envelope amount
                let dec = self.p.get("decay") as f32 * (1.0 - acc * 0.5);
                (
                    Adsr { attack: 0.002, decay: dec, sustain: 0.0, release: 0.08 },
                    Adsr { attack: 0.002, decay: dec, sustain: 0.0, release: 0.08 },
                    self.p.get("envMod") as f32 * (1.0 + acc),
                )
            }
            "reese" => (
                Adsr { attack: self.p.get("attack") as f32, decay: 0.2, sustain: 1.0, release: self.p.get("release") as f32 },
                Adsr { attack: 0.01, decay: 0.2, sustain: 0.7, release: 0.3 },
                0.0,
            ),
            _ => (
                Adsr { attack: 0.005, decay: 0.2, sustain: 1.0, release: 0.15 },
                Adsr { attack: 0.005, decay: 0.2, sustain: 1.0, release: 0.15 },
                0.0,
            ),
        }
    }

    fn filter_of(&self) -> (FilterKind, bool, f32) {
        match self.kind.as_str() {
            "synth" => match self.p.get("fType").round() as i32 {
                1 => (FilterKind::Lowpass, true, self.p.get("reso") as f32 * 0.7),
                2 => (FilterKind::Bandpass, false, self.p.get("reso").max(0.5) as f32),
                3 => (FilterKind::Highpass, false, self.p.get("reso") as f32),
                _ => (FilterKind::Lowpass, false, self.p.get("reso") as f32),
            },
            "bass" => (FilterKind::Lowpass, true, self.p.get("reso") as f32 * 0.3 + 0.5),
            "wobble" | "reese" => (FilterKind::Lowpass, true, self.p.get("reso") as f32 * 0.3 + 0.5),
            _ => (FilterKind::Lowpass, false, self.p.get("reso") as f32),
        }
    }
}

impl Instrument for Sub {
    fn note_on(&mut self, pitch: f64, vel: f64, at: u64, dur: u64) {
        let vel = vel.clamp(0.0, 1.0) as f32;
        let base = mtof(pitch + 12.0 * self.p.get("octave"));
        let acc = self.accent_of(vel);
        // bass slide: a note that starts while the previous one still sounds keeps that voice and its
        // envelope; the pitch glides over `slide` seconds
        if self.kind == "bass" {
            let slide = self.p.get("slide");
            if let Some(v) = self.voices.last_mut() {
                if !v.done && v.off >= at {
                    let k = (base / v.base) as f32;
                    v.glide = Some((v.base, base, slide, at));
                    v.base = base;
                    v.hz.iter_mut().for_each(|h| *h *= k);
                    v.off = at + dur.max(1);
                    v.env_amt = self.p.get("envMod") as f32 * (1.0 + acc);
                    v.q_scale = 1.0 + acc * 0.3;
                    self.last_hz = Some(base);
                    return;
                }
            }
        }
        let oscs = self.build_oscs(pitch, vel);
        let hz: Vec<f32> = oscs.iter().map(|o| o.osc.freq()).collect();
        let glide_secs = match self.kind.as_str() {
            "synth" | "supersaw" | "reese" | "wobble" => self.p.get("glide"),
            _ => 0.0,
        };
        let glide = match self.last_hz {
            Some(from) if glide_secs > 0.0 => Some((from, base, glide_secs, at)),
            _ => None,
        };
        self.last_hz = Some(base);
        let bass = self.kind == "bass";
        let bass_lfo = if bass && self.p.get("lfoRate") >= 1.0 && self.p.get("lfoAmt") > 0.0 {
            Some(Osc::new(self.sr, self.bass_lfo_hz()))
        } else {
            None
        };
        let (amp, fenv, env_amt) = self.env_for(acc);
        let (kind, two_pole, q) = self.filter_of();
        let cut = self.base_cutoff(pitch, vel);
        let mut filt = Biquad::new(kind, self.sr, cut, q, 0.0);
        let filt2 = filt;
        if !two_pole {
            filt.set(kind, self.sr, cut, q, 0.0);
        }
        let voice = SubVoice {
            hz,
            glide,
            base,
            start: at,
            off: at + dur.max(1),
            vel,
            oscs,
            amp,
            filt,
            filt2,
            noise: Rng::new(self.rng.next_u64()),
            noise_amt: self.p.get("noise") as f32 * 0.5,
            done: false,
            cut,
            env_amt,
            fenv,
            filter_kind: kind,
            two_pole,
            gain: 1.0,
            lfo_amt: self.lfo_depth(),
            lfo_cut: matches!(self.kind.as_str(), "wobble" | "pad"),
            pwm: if self.pwm_on() { Some((Osc::new(self.sr, self.p.get("pwmRate") as f32), self.p.get("pwm") as f32 * 0.45)) } else { None },
            lfo_pitch: bass_lfo.is_some() && self.p.get("lfoDest") >= 0.5,
            lfo: bass_lfo,
            q_scale: 1.0 + acc * 0.3,
        };
        self.voices.push(voice);
        if self.voices.len() > 48 {
            self.voices.remove(0);
        }
    }

    fn set(&mut self, name: &str, value: f64) {
        self.p.set(self.spec, name, value);
    }

    fn tempo(&mut self, bpm: f64, _origin: u64) {
        self.bpm = bpm;
    }

    fn bend(&mut self, semis: f64, at: u64) {
        let i = self.bend_ev.partition_point(|e| e.0 <= at);
        self.bend_ev.insert(i, (at, semis));
    }

    fn render(&mut self, l: &mut [f32], r: &mut [f32], start: u64) {
        let lfo_hz = self.lfo_rate();
        self.lfo.set_freq(lfo_hz);
        let shape = Wave::Sine;
        let bass = self.kind == "bass";
        for i in 0..l.len() {
            let f = start + i as u64;
            while self.bend_cursor < self.bend_ev.len() && self.bend_ev[self.bend_cursor].0 <= f {
                let (_, semis) = self.bend_ev[self.bend_cursor];
                self.bend_cursor += 1;
                self.bend_ratio = 2f64.powf(semis.clamp(-48.0, 48.0) / 12.0);
            }
            let bend = self.bend_ratio;
            let lfo = self.lfo.tick(shape);
            let (mut sl, mut sr) = (0.0f32, 0.0f32);
            for v in self.voices.iter_mut() {
                if v.done || f < v.start {
                    continue;
                }
                let t = (f - v.start) as f32 / self.sr;
                let hold = (v.off - v.start) as f32 / self.sr;
                let a = v.amp.level_at(t, Some(hold));
                if a < 1e-5 && t > hold {
                    v.done = true;
                    continue;
                }
                // per-voice modulators advance every frame so their phase runs continuously
                let bl = match v.lfo.as_mut() {
                    Some(o) => o.tick(Wave::Sine),
                    None => 0.0,
                };
                let (pwm_lfo, pwm_depth) = match v.pwm.as_mut() {
                    Some((o, k)) => (o.tick(Wave::Sine), *k),
                    None => (0.0, 0.0),
                };
                // PWM delay in samples: half a period, swept by the LFO
                let pwm_delay = (0.5 + pwm_depth * pwm_lfo) * (self.sr / v.base as f32);
                if i % 32 == 0 {
                    // glide ratio and bass pitch LFO (cents), applied to every oscillator
                    let mut factor = 1.0f32;
                    if let Some((from, base, secs, gs)) = v.glide {
                        let gt = f.saturating_sub(gs) as f64 / self.sr as f64;
                        let cur = if gt < secs { from * (base / from).powf(gt / secs) } else { base };
                        factor = (cur / v.base) as f32;
                    }
                    let pitch_mod = if v.lfo_pitch { 2f32.powf(bl * v.lfo_amt * 100.0 / 1200.0) } else { 1.0 };
                    // always refresh: bend can return to 1.0 and must not leave a stale pitch behind
                    factor *= pitch_mod * bend as f32;
                    for (o, hz) in v.oscs.iter_mut().zip(v.hz.iter()) {
                        o.osc.set_freq(hz * factor);
                    }
                }
                let fe = v.fenv.level_at(t, Some(hold));
                let mut sample = 0.0f32;
                for o in v.oscs.iter_mut() {
                    sample += match o.pwm.as_mut() {
                        Some(line) => {
                            let saw = o.osc.tick(o.wave);
                            line.write(saw);
                            (saw - line.read(pwm_delay)) * o.gain
                        }
                        None => o.osc.tick(o.wave) * o.gain,
                    };
                }
                if v.noise_amt > 0.0 {
                    sample += v.noise.bipolar() * v.noise_amt * v.vel;
                }
                // cutoff: filter envelope + LFO, updated every 32 frames
                if i % 32 == 0 {
                    let mut c = v.cut * 2f32.powf(v.env_amt * fe * 4.0);
                    if v.lfo_cut && v.lfo_amt > 0.0 {
                        c *= 2f32.powf(lfo * v.lfo_amt * 2.0);
                    }
                    if v.lfo.is_some() && !v.lfo_pitch {
                        // bass cutoff LFO: amplitude in Hz as in the web plugin (cutoff * amt * 5)
                        c += bl * v.cut * v.lfo_amt * 5.0;
                    }
                    let c = c.clamp(20.0, self.sr * 0.45);
                    let q = self.p.get("reso") as f32 * v.q_scale;
                    v.filt.set(v.filter_kind, self.sr, c, q.max(0.0001), 0.0);
                    v.filt2 = v.filt;
                }
                let mut y = v.filt.process(sample);
                if v.two_pole {
                    y = v.filt2.process(y);
                }
                let (pl, pr) = (1.0f32, 1.0f32);
                sl += y * a * pl * v.gain;
                sr += y * a * pr * v.gain;
            }
            if bass {
                // drive: the web plugin's tanh WaveShaper (input clamped to +-1) and its output trim
                let d = self.p.get("drive") as f32;
                let k = 1.0 + d * 8.0;
                let post = 1.0 / (1.0 + d * 1.5);
                let norm = k.tanh();
                sl = (k * sl.clamp(-1.0, 1.0)).tanh() / norm * post;
                sr = (k * sr.clamp(-1.0, 1.0)).tanh() / norm * post;
            }
            l[i] += sl;
            r[i] += sr;
        }
        self.voices.retain(|v| !v.done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_subtractive_device_builds_and_sounds() {
        let ctx = Ctx { sr: 48_000.0, plugins: std::path::PathBuf::from("/nonexistent"), seed: 3 };
        for kind in ["synth", "pad", "supersaw", "bass", "reese", "wobble"] {
            let spec = spec(kind).unwrap();
            let params = Params::new(spec, &crate::model::Extra::new());
            let mut inst = new(kind, &params, &ctx).unwrap();
            inst.note_on(60.0, 0.8, 0, 4_800);
            let mut l = vec![0.0f32; 4_800];
            let mut r = vec![0.0f32; 4_800];
            inst.render(&mut l, &mut r, 0);
            let peak = l.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak > 1e-3, "{kind} is silent");
            assert!(peak < 4.0, "{kind} peak {peak}");
        }
    }
}
