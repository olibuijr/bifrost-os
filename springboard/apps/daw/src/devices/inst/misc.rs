//! Remaining instruments: Pluck (Karplus-Strong string, ported from `pluck/index.js`), FM (two-operator), the
//! procedural drum kit (`drums`), the piano (additive, procedural), and the sample-based Drum Rack and Sampler
//! (WAV files from the shared samples folder).
//!
//! Approximations: FM uses the first modulator only (the algorithm switch is not ported). The piano is additive
//! with inharmonic partials rather than the web plugin's full model. The drum kit is a set of analytic voices.

use crate::assets;
use crate::devices::{p, Ctx, Curve, Instrument, Params, P};
use crate::dsp::{mtof, Adsr, Biquad, Delay, FilterKind, Rng, Wave};
use crate::model::Instrument as Model;
use std::path::PathBuf;
use std::sync::Arc;

const LIN: Curve = Curve::Lin;
const LOG: Curve = Curve::Log;
const INT: Curve = Curve::Int;

const PLUCK: &[P] = &[
    p("damping", 0.0, 1.0, 0.4, LIN, ""),
    p("decay", 0.1, 8.0, 1.5, LOG, "s"),
    p("brightness", 0.0, 1.0, 0.7, LIN, ""),
    p("body", 0.0, 1.0, 0.3, LIN, ""),
    p("octave", -2.0, 2.0, 0.0, INT, ""),
    p("gain", 0.0, 1.0, 0.7, LIN, ""),
    p("pick", 0.0, 1.0, 0.0, LIN, ""),
    p("tension", 0.0, 1.0, 0.0, LIN, ""),
    p("mute", 0.0, 1.0, 0.0, LIN, ""),
];

const FM: &[P] = &[
    p("ratio", 0.5, 12.0, 1.0, LIN, ""),
    p("fine", -50.0, 50.0, 0.0, LIN, "ct"),
    p("index", 0.0, 12.0, 3.0, LIN, ""),
    p("idxDecay", 0.01, 4.0, 0.4, LOG, "s"),
    p("idxSus", 0.0, 1.0, 0.2, LIN, ""),
    p("feedback", 0.0, 1.0, 0.0, LIN, ""),
    p("octave", -2.0, 2.0, 0.0, INT, ""),
    p("attack", 0.001, 2.0, 0.003, LOG, "s"),
    p("decay", 0.01, 4.0, 0.8, LOG, "s"),
    p("sustain", 0.0, 1.0, 0.3, LIN, ""),
    p("release", 0.01, 4.0, 0.4, LOG, "s"),
    p("gain", 0.0, 1.0, 0.6, LIN, ""),
    p("algo", 0.0, 2.0, 0.0, INT, ""),
    p("ratio2", 0.5, 16.0, 14.0, LIN, ""),
    p("index2", 0.0, 8.0, 0.0, LIN, ""),
    p("idx2Decay", 0.005, 1.0, 0.08, LOG, "s"),
    p("velIdx", 0.0, 1.0, 0.5, LIN, ""),
    p("keyScale", 0.0, 1.0, 0.0, LIN, ""),
];

const DRUMS: &[P] = &[
    p("kickTune", 30.0, 90.0, 48.0, LIN, "Hz"),
    p("kickDecay", 0.1, 1.2, 0.45, LIN, "s"),
    p("kickPunch", 0.0, 1.0, 0.6, LIN, ""),
    p("snareTone", 120.0, 320.0, 190.0, LIN, "Hz"),
    p("snareSnap", 0.0, 1.0, 0.6, LIN, ""),
    p("hatTone", 4000.0, 12000.0, 8000.0, LOG, "Hz"),
    p("hatDecay", 0.02, 0.5, 0.06, LOG, "s"),
    p("tomTune", 60.0, 200.0, 110.0, LIN, "Hz"),
    p("gain", 0.0, 1.0, 0.8, LIN, ""),
];

const DRUMRACK: &[P] = &[
    p("tune", -24.0, 24.0, 0.0, INT, "st"),
    p("decay", 0.02, 4.0, 4.0, LOG, "s"),
    p("tone", 200.0, 20000.0, 20000.0, LOG, "Hz"),
    p("gain", 0.0, 1.0, 0.85, LIN, ""),
    p("chokeMode", 0.0, 2.0, 1.0, INT, ""),
];

const SAMPLER: &[P] = &[
    p("root", 24.0, 96.0, 60.0, INT, ""),
    p("start", 0.0, 1.0, 0.0, LIN, ""),
    p("end", 0.0, 1.0, 1.0, LIN, ""),
    p("attack", 0.001, 2.0, 0.002, LOG, "s"),
    p("release", 0.01, 4.0, 0.08, LOG, "s"),
    p("cutoff", 60.0, 20000.0, 20000.0, LOG, "Hz"),
    p("gain", 0.0, 1.0, 0.8, LIN, ""),
    p("mode", 0.0, 1.0, 1.0, INT, ""),
    p("loop", 0.0, 1.0, 0.0, INT, ""),
    p("reverse", 0.0, 1.0, 0.0, INT, ""),
];

const PIANO: &[P] = &[
    p("tone", 0.0, 1.0, 0.55, LIN, ""),
    p("hardness", 0.0, 1.0, 0.5, LIN, ""),
    p("decay", 0.2, 3.0, 1.0, LOG, "x"),
    p("release", 0.02, 1.5, 0.12, LOG, "s"),
    p("stereo", 0.0, 1.0, 0.6, LIN, ""),
    p("body", 0.0, 1.0, 0.5, LIN, ""),
    p("detune", 0.0, 1.0, 0.35, LIN, ""),
    p("hammer", 0.0, 1.0, 0.4, LIN, ""),
    p("velCurve", 0.5, 3.0, 1.6, LIN, ""),
    p("gain", 0.0, 1.0, 0.7, LIN, ""),
];

pub fn spec(kind: &str) -> Option<&'static [P]> {
    Some(match kind {
        "pluck" => PLUCK,
        "fm" => FM,
        "drums" => DRUMS,
        "drumrack" => DRUMRACK,
        "sampler" => SAMPLER,
        "piano" => PIANO,
        _ => return None,
    })
}

pub fn new(kind: &str, params: &Params, model: &Model, ctx: &Ctx) -> Option<Box<dyn Instrument>> {
    let sr = ctx.sr;
    let p = params.clone();
    Some(match kind {
        "pluck" => Box::new(Pluck { p, sr, voices: vec![] }),
        "fm" => Box::new(Fm { p, sr, voices: vec![] }),
        "drums" => Box::new(Drums { p, sr, voices: vec![], rng: Rng::new(ctx.seed ^ 0xD5) }),
        "piano" => Box::new(Piano { p, sr, voices: vec![], rng: Rng::new(ctx.seed ^ 0x9A) }),
        "drumrack" => Box::new(DrumRack::new(p, sr, ctx, model)),
        "sampler" => Box::new(Sampler::new(p, sr, ctx, model)),
        _ => return None,
    })
}

/// Stereo output helper: a voice contributes `x` to both channels at the given gain.
#[inline]
fn add(l: &mut [f32], r: &mut [f32], i: usize, x: f32) {
    l[i] += x;
    r[i] += x;
}

// ---------- Pluck ----------

struct PluckVoice {
    start: u64,
    off: u64,
    delay: Delay,
    loop_samples: f32,
    feedback: f32,
    damp: Biquad,
    amp: f32,
    decay_coeff: f32,
    released: bool,
    done: bool,
    rng: Rng,
}

struct Pluck {
    p: Params,
    sr: f32,
    voices: Vec<PluckVoice>,
}

impl Instrument for Pluck {
    fn note_on(&mut self, pitch: f64, vel: f64, at: u64, dur: u64) {
        let f0 = mtof(pitch + 12.0 * self.p.get("octave")) as f32;
        let period = self.sr / f0.max(1.0);
        // the loop must be at least a render quantum long
        let loop_samples = period.max(128.0);
        let mut delay = Delay::new(self.sr, 1.0);
        let decay = self.p.get("decay") as f32;
        let g = (0.001f32).powf(loop_samples / self.sr / decay).min(0.995);
        let fc = (self.sr * 0.45).min(f0 * (1.2 + 20.0 * (1.0 - self.p.get("damping") as f32)));
        let damp = Biquad::new(FilterKind::Lowpass, self.sr, fc, -3.0, 0.0);
        // excite the loop with a burst of noise (one period long)
        let mut rng = Rng::new(at ^ (pitch.to_bits()));
        let burst = (period.round() as usize).max(2);
        for _ in 0..burst.min(delay.len() / 2) {
            delay.write(rng.bipolar() * vel as f32 * 0.5);
        }
        let vel = vel.clamp(0.0, 1.0) as f32;
        let _ = dur;
        self.voices.push(PluckVoice {
            start: at,
            off: at + dur.max(1),
            delay,
            loop_samples,
            feedback: g,
            damp,
            amp: self.p.get("gain") as f32 * 0.3 * vel,
            decay_coeff: (-1.0 / (decay * self.sr / 5.0)).exp(),
            released: false,
            done: false,
            rng,
        });
        if self.voices.len() > 32 {
            self.voices.remove(0);
        }
    }

    fn set(&mut self, name: &str, value: f64) {
        self.p.set(PLUCK, name, value);
    }

    fn render(&mut self, l: &mut [f32], r: &mut [f32], start: u64) {
        for i in 0..l.len() {
            let f = start + i as u64;
            let mut out = 0.0f32;
            for v in self.voices.iter_mut() {
                if v.done || f < v.start {
                    continue;
                }
                if !v.released && f >= v.off {
                    v.released = true;
                }
                let read = v.delay.read(v.loop_samples - 1.0);
                let filtered = v.damp.process(read);
                v.delay.write(filtered * v.feedback);
                if v.released {
                    v.amp *= v.decay_coeff;
                }
                out += filtered * v.amp;
                if v.released && v.amp < 1e-5 {
                    v.done = true;
                }
            }
            add(l, r, i, out);
        }
        self.voices.retain(|v| !v.done);
    }
}

// ---------- FM (fm/index.js port) ----------
//
// Carrier frequency is modulated by the modulator through the index envelope (Hz deviation). Feedback drives the
// modulator's frequency from a sine at the modulator's rate. algo 1 (stack) feeds a fast third operator into the
// modulator's frequency; algo 2 (parallel) feeds it into a second carrier at the root, mixed at 0.3. Envelopes
// follow the Web Audio setTargetAtTime shapes in closed form.

struct FmVoice {
    start: u64,
    off: f64,
    end: f64,
    f0: f64,
    fm: f64,
    depth: f64,
    idx_sus: f64,
    idx_tc: f64,
    idx_rel_tc: f64,
    attack: f64,
    decay_tc: f64,
    sustain: f64,
    rel_tc: f64,
    peak: f64,
    fb_gain: f64,
    stack: Option<(f64, f64, f64)>,
    parallel: Option<(f64, f64, f64)>,
    car_phase: f64,
    mod_phase: f64,
    fb_phase: f64,
    m2_phase: f64,
    c2_phase: f64,
    vel: f64,
    done: bool,
}

struct Fm {
    p: Params,
    sr: f32,
    voices: Vec<FmVoice>,
}

impl FmVoice {
    fn idx(&self, t: f64) -> f64 {
        if t < self.off {
            self.depth * self.idx_sus + self.depth * (1.0 - self.idx_sus) * (-t / self.idx_tc).exp()
        } else {
            let at_off = self.depth * self.idx_sus + self.depth * (1.0 - self.idx_sus) * (-self.off / self.idx_tc).exp();
            at_off * (-(t - self.off) / self.idx_rel_tc).exp()
        }
    }

    fn amp(&self, t: f64) -> f64 {
        let s = self.sustain;
        if t < self.attack {
            self.peak * t / self.attack
        } else if t < self.off {
            self.peak * s + self.peak * (1.0 - s) * (-(t - self.attack) / self.decay_tc).exp()
        } else {
            let at_off = self.peak * s + self.peak * (1.0 - s) * (-(self.off - self.attack) / self.decay_tc).exp();
            at_off * (-(t - self.off) / self.rel_tc).exp()
        }
    }
}

impl Instrument for Fm {
    fn note_on(&mut self, pitch: f64, vel: f64, at: u64, dur: u64) {
        let vel = vel.clamp(0.0, 1.0);
        let sr = self.sr as f64;
        let f0 = mtof(pitch + 12.0 * self.p.get("octave"));
        let ratio = self.p.get("ratio");
        let fm = f0 * ratio * 2f64.powf(self.p.get("fine") / 1200.0);
        let ks = 2f64.powf(-self.p.get("keyScale") * (pitch + 12.0 * self.p.get("octave") - 60.0) / 24.0);
        let vi = self.p.get("velIdx");
        let depth = self.p.get("index") * fm * (1.0 - vi + vi * vel);
        let release = self.p.get("release").max(0.01);
        let off = (dur as f64 / sr).max(0.02);
        let attack = self.p.get("attack").max(0.002);
        let feedback = self.p.get("feedback");
        let algo = self.p.get("algo").round() as i32;
        let index2 = self.p.get("index2");
        let f2 = f0 * self.p.get("ratio2");
        let d2 = index2 * f2 * (1.0 - vi + vi * vel);
        let tc2 = self.p.get("idx2Decay") * ks / 3.0;
        let (stack, parallel) = if algo > 0 && index2 > 0.0 {
            if algo == 1 {
                (Some((f2, d2, tc2)), None)
            } else {
                (None, Some((f2, d2, tc2)))
            }
        } else {
            (None, None)
        };
        self.voices.push(FmVoice {
            start: at,
            off,
            end: off + release * 1.6 + 0.05,
            f0,
            fm,
            depth,
            idx_sus: self.p.get("idxSus"),
            idx_tc: (self.p.get("idxDecay") * ks / 3.0).max(1e-4),
            idx_rel_tc: release / 3.0,
            attack,
            decay_tc: (self.p.get("decay") * ks / 3.0).max(1e-4),
            sustain: self.p.get("sustain"),
            rel_tc: release / 4.0,
            peak: self.p.get("gain") * vel * 0.3,
            fb_gain: feedback * fm * 1.5,
            stack,
            parallel,
            car_phase: 0.0,
            mod_phase: 0.0,
            fb_phase: 0.0,
            m2_phase: 0.0,
            c2_phase: 0.0,
            vel,
            done: false,
        });
        if self.voices.len() > 32 {
            self.voices.remove(0);
        }
    }

    fn set(&mut self, name: &str, value: f64) {
        self.p.set(FM, name, value);
    }

    fn render(&mut self, l: &mut [f32], r: &mut [f32], start: u64) {
        let sr = self.sr as f64;
        for i in 0..l.len() {
            let f = start + i as u64;
            let mut out = 0.0f64;
            for v in self.voices.iter_mut() {
                if v.done || f < v.start {
                    continue;
                }
                let t = (f - v.start) as f64 / sr;
                if t > v.end {
                    v.done = true;
                    continue;
                }
                let idx = v.idx(t);
                let fb_sample = (std::f64::consts::TAU * v.fb_phase).sin();
                let m2 = (std::f64::consts::TAU * v.m2_phase).sin();
                let (stack_dev, par) = match (v.stack, v.parallel) {
                    (Some((f2, d2, tc)), _) => {
                        v.m2_phase = (v.m2_phase + f2 / sr).fract();
                        (d2 * (-t / tc.max(1e-4)).exp() * m2, 0.0)
                    }
                    (_, Some((f2, d2, tc))) => {
                        v.m2_phase = (v.m2_phase + f2 / sr).fract();
                        let dev = d2 * (-t / tc.max(1e-4)).exp() * m2;
                        v.c2_phase = (v.c2_phase + (v.f0 + dev) / sr).fract();
                        (0.0, 0.3 * (std::f64::consts::TAU * v.c2_phase).sin())
                    }
                    _ => (0.0, 0.0),
                };
                let mod_sample = (std::f64::consts::TAU * v.mod_phase).sin();
                v.mod_phase = (v.mod_phase + (v.fm + v.fb_gain * fb_sample + stack_dev) / sr).fract();
                v.fb_phase = (v.fb_phase + v.fm / sr).fract();
                v.car_phase = (v.car_phase + (v.f0 + idx * mod_sample) / sr).fract();
                let carrier = (std::f64::consts::TAU * v.car_phase).sin();
                let a = v.amp(t);
                out += (carrier + par) * a * self.p.get("gain") * 0.5 * v.vel;
            }
            l[i] += out as f32;
            r[i] += out as f32;
        }
        self.voices.retain(|v| !v.done);
    }
}

// ---------- Drums (procedural kit) ----------

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Kick,
    Snare,
    Clap,
    Rim,
    Hat,
    OpenHat,
    TomLow,
    TomHigh,
}

struct DrumVoice {
    start: u64,
    hit: Hit,
    vel: f32,
    rng: Rng,
    phase: f32,
    hp_z: f32,
    done: bool,
}

struct Drums {
    p: Params,
    sr: f32,
    voices: Vec<DrumVoice>,
    rng: Rng,
}

fn hit_of(pitch: f64) -> Option<Hit> {
    Some(match pitch.round() as i32 {
        36 => Hit::Kick,
        38 => Hit::Snare,
        39 => Hit::Clap,
        37 => Hit::Rim,
        42 | 44 => Hit::Hat,
        46 => Hit::OpenHat,
        45 => Hit::TomLow,
        50 => Hit::TomHigh,
        _ => return None,
    })
}

impl Instrument for Drums {
    fn note_on(&mut self, pitch: f64, vel: f64, at: u64, _dur: u64) {
        let Some(hit) = hit_of(pitch) else { return };
        // hats choke the open hat
        if matches!(hit, Hit::Hat) {
            for v in self.voices.iter_mut() {
                if v.hit == Hit::OpenHat {
                    v.done = true;
                }
            }
        }
        let rng = Rng::new(self.rng.next_u64());
        self.voices.push(DrumVoice { start: at, hit, vel: vel.clamp(0.0, 1.0) as f32, rng, phase: 0.0, hp_z: 0.0, done: false });
        if self.voices.len() > 48 {
            self.voices.remove(0);
        }
    }

    fn set(&mut self, name: &str, value: f64) {
        self.p.set(DRUMS, name, value);
    }

    fn render(&mut self, l: &mut [f32], r: &mut [f32], start: u64) {
        let sr = self.sr;
        let gain = self.p.get("gain") as f32;
        for i in 0..l.len() {
            let f = start + i as u64;
            let mut out = 0.0f32;
            for v in self.voices.iter_mut() {
                if v.done || f < v.start {
                    continue;
                }
                let t = (f - v.start) as f32 / sr;
                let noise = v.rng.bipolar();
                let y = match v.hit {
                    Hit::Kick => {
                        let tune = self.p.get("kickTune") as f32;
                        let dec = self.p.get("kickDecay") as f32;
                        let f_inst = tune + (tune * 3.0) * (-t / 0.02).exp();
                        v.phase += f_inst / sr;
                        let body = (std::f32::consts::TAU * v.phase).sin() * (-t / dec).exp();
                        let click = noise * (-t / 0.004).exp() * self.p.get("kickPunch") as f32 * 0.4;
                        body + click
                    }
                    Hit::Snare => {
                        let tone = self.p.get("snareTone") as f32;
                        v.phase += tone / sr;
                        let t_tone = (std::f32::consts::TAU * v.phase).sin() * (-t / 0.12).exp() * 0.6;
                        let snap = noise * (-t / 0.18).exp() * self.p.get("snareSnap") as f32;
                        t_tone + snap
                    }
                    Hit::Clap => {
                        let burst = if t < 0.03 { 1.0 } else { 0.0 } + if (0.035..0.07).contains(&t) { 1.0 } else { 0.0 } + if (0.075..0.12).contains(&t) { 1.0 } else { 0.0 };
                        noise * (burst * 0.5 + (-(t - 0.12).max(0.0) / 0.15).exp() * 0.8)
                    }
                    Hit::Rim => {
                        v.phase += 1700.0 / sr;
                        (std::f32::consts::TAU * v.phase).sin() * (-t / 0.02).exp() * 0.7
                    }
                    Hit::Hat | Hit::OpenHat => {
                        let (dec, tone) = if v.hit == Hit::Hat {
                            (self.p.get("hatDecay") as f32, self.p.get("hatTone") as f32)
                        } else {
                            (0.3, self.p.get("hatTone") as f32)
                        };
                        // one-pole highpass on white noise
                        let a = (-std::f32::consts::TAU * tone / sr).exp();
                        v.hp_z = a * v.hp_z + (1.0 - a) * noise;
                        (noise - v.hp_z) * (-t / dec).exp() * 0.6
                    }
                    Hit::TomLow | Hit::TomHigh => {
                        let base = self.p.get("tomTune") as f32 * if v.hit == Hit::TomHigh { 1.5 } else { 1.0 };
                        v.phase += (base + base * 0.5 * (-t / 0.05).exp()) / sr;
                        (std::f32::consts::TAU * v.phase).sin() * (-t / 0.25).exp() * 0.9
                    }
                };
                out += y * v.vel * gain * 0.6;
                if t > 1.5 {
                    v.done = true;
                }
            }
            add(l, r, i, out);
        }
        self.voices.retain(|v| !v.done);
    }
}

// ---------- Piano (Grand, port of piano/index.js) ----------
//
// Per note: two detuned strings, each a register-dependent wavetable (rich bass, few treble partials, strike
// position comb), a per-note lowpass whose brightness envelope falls as the note rings, a noise hammer through
// that lowpass, the prompt/aftersound amplitude envelope with damper release, and a body EQ (peaking 220 Hz,
// peaking 2.6 kHz, high shelf 7 kHz). Pan follows pitch. Approximation: the web plugin shares one body filter
// across notes and smooths its changes; here each voice carries its own body filter (no smoothing), and the
// hammer noise uses a per-voice PRNG instead of the shared 0.5 s noise buffer.

const PIANO_TABLE: usize = 2048;

type PianoTable = std::sync::Arc<[f32]>;

/// Wavetables per (sample rate, register), built once and shared between notes.
static PIANO_TABLES: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<(u32, usize), PianoTable>>> =
    std::sync::OnceLock::new();

fn piano_table(sr: f32, reg: usize) -> PianoTable {
    let map = PIANO_TABLES.get_or_init(Default::default);
    let mut map = map.lock().unwrap_or_else(|e| e.into_inner());
    map.entry((sr.to_bits(), reg)).or_insert_with(|| build_piano_table(sr, reg)).clone()
}

/// One period of the string spectrum for register `reg` (0..=29, MIDI 21 + 3*reg), peak-normalised.
fn build_piano_table(sr: f32, reg: usize) -> PianoTable {
    let f0 = mtof(21.0 + reg as f64 * 3.0) as f32;
    let nyq = (sr * 0.5).min(14000.0);
    let n = ((nyq / f0).floor() as usize).clamp(3, 48);
    let x = reg as f32 / 29.0;
    let slope = 0.75 + x * 0.9; // rolloff steeper in treble
    let mut amp = vec![0f32; n + 1];
    for (k, a) in amp.iter_mut().enumerate().skip(1) {
        let kf = k as f32;
        // hammer strike position (~1/8 of string length) notches every 8th partial
        let comb = 0.25 + 0.75 * (std::f32::consts::PI * kf / 8.3).sin().abs();
        // bass: fundamental weaker than the 2nd/3rd partials
        let fund = if k == 1 && x < 0.3 { 0.55 + x * 1.5 } else { 1.0 };
        *a = fund * comb / kf.powf(slope) * if k % 2 == 1 { 1.0 } else { 0.9 };
    }
    let mut t = vec![0f32; PIANO_TABLE];
    for (i, s) in t.iter_mut().enumerate() {
        let ph = std::f32::consts::TAU * i as f32 / PIANO_TABLE as f32;
        *s = (1..=n).map(|k| amp[k] * (k as f32 * ph).sin()).sum();
    }
    let peak = t.iter().fold(0f32, |m, v| m.max(v.abs()));
    if peak > 0.0 {
        for s in t.iter_mut() {
            *s /= peak;
        }
    }
    t.into()
}

/// Linear-interpolated read of one wavetable period at phase `ph` in 0..1.
#[inline]
fn read_piano_table(t: &[f32], ph: f32) -> f32 {
    let x = ph * PIANO_TABLE as f32;
    let i = (x as usize) & (PIANO_TABLE - 1);
    let j = (i + 1) & (PIANO_TABLE - 1);
    t[i] + (t[j] - t[i]) * x.fract()
}

struct PianoVoice {
    start: u64,
    /// Oscillators stop here (seconds after start).
    end_t: f32,
    /// Damper release begins here (seconds after start), INFINITY when the note rings out.
    off_t: f32,
    rel_tau: f32,
    rel_level: f32,
    /// Full-scale amplitude of the envelope (peak / 2 strings).
    amp: f32,
    aft: f32,
    tau1: f32,
    tau2: f32,
    /// End of the prompt sound (0.003 + 2.2 * tau1) and the envelope level there.
    t2: f32,
    v_t2: f32,
    cut: Biquad,
    cut_c0: f32,
    cut_c1: f32,
    cut_tc: f32,
    phase: [f32; 2],
    inc: [f32; 2],
    table: PianoTable,
    hammer_amp: f32,
    hammer_tc: f32,
    rng: Rng,
    pan: (f32, f32),
    body: [[Biquad; 3]; 2],
    done: bool,
}

impl PianoVoice {
    /// Amplitude before any damper release: 3 ms attack ramp, prompt decay (tau1), aftersound (tau2).
    #[inline]
    fn env(&self, t: f32) -> f32 {
        if t < 0.003 {
            self.amp * t / 0.003
        } else if t < self.t2 {
            self.amp * self.aft + self.amp * (1.0 - self.aft) * (-(t - 0.003) / self.tau1).exp()
        } else {
            self.v_t2 * (-(t - self.t2) / self.tau2).exp()
        }
    }

    #[inline]
    fn gain_at(&self, t: f32) -> f32 {
        if t >= self.off_t {
            self.rel_level * (-(t - self.off_t) / self.rel_tau).exp()
        } else {
            self.env(t)
        }
    }

    #[inline]
    fn cutoff(&self, t: f32) -> f32 {
        if t < 0.005 {
            self.cut_c0
        } else {
            self.cut_c1 + (self.cut_c0 - self.cut_c1) * (-(t - 0.005) / self.cut_tc).exp()
        }
    }
}

#[inline]
fn piano_body(bq: &mut [Biquad; 3], x: f32) -> f32 {
    bq.iter_mut().fold(x, |y, b| b.process(y))
}

struct Piano {
    p: Params,
    sr: f32,
    voices: Vec<PianoVoice>,
    rng: Rng,
}

impl Instrument for Piano {
    fn note_on(&mut self, pitch: f64, vel: f64, at: u64, dur: u64) {
        let sr = self.sr;
        let tone = self.p.get("tone") as f32;
        let hardness = self.p.get("hardness") as f32;
        let decay = self.p.get("decay") as f32;
        let release = self.p.get("release") as f32;
        let stereo = self.p.get("stereo") as f32;
        let body = self.p.get("body") as f32;
        let detune = self.p.get("detune") as f32;
        let hammer = self.p.get("hammer") as f32;
        let vel_curve = self.p.get("velCurve") as f32;
        let gain = self.p.get("gain") as f32;

        let v = vel.clamp(0.0, 1.0) as f32;
        let f0 = mtof(pitch) as f32;
        let oct = ((pitch - 60.0) / 12.0) as f32;
        let reg = ((pitch - 21.0) / 87.0).clamp(0.0, 1.0) as f32; // 0 = A0, 1 = C8
        let reg_i = ((pitch - 21.0) / 3.0).round().clamp(0.0, 29.0) as usize;
        let vc = v.powf(vel_curve);
        let hard = 0.25 + 0.75 * hardness;
        let bright = 0.3 + 1.4 * tone;

        // amplitude envelope times
        let tau1 = (0.28 * decay * 2f32.powf(-oct * 0.55)).clamp(0.03, 2.0); // prompt sound
        let tau2 = (3.2 * decay * 2f32.powf(-oct * 0.75)).clamp(0.15, 14.0); // aftersound
        let aft = 0.28 + 0.1 * (1.0 - reg);
        let peak = gain * (0.01 + 0.16 * vc) * (1.1 - 0.35 * reg);
        let amp = peak / 2.0;
        let t2 = 0.003 + 2.2 * tau1;
        let nat = 0.004 + tau1 * 2.2 + tau2 * 6.5;
        let undamped = pitch >= 89.0;
        let rel_tau = (release / 4.0).max(0.008);
        let off_raw = (dur as f32 / sr).max(0.03);
        let end_t = if undamped { nat } else { nat.min(off_raw + rel_tau * 7.0) };
        let off_t = if !undamped && off_raw < nat { off_raw } else { f32::INFINITY };

        // strings: 2 detuned oscillators; stretch tuning (bass flat, treble sharp), detune spread by register
        let d = (pitch - 64.0) / 44.0;
        let base_det = (d.signum() * d * d * 22.0) as f32;
        let spread = detune * (0.6 + 2.4 * reg);
        let inc_of = |cents: f32| f0 * 2f32.powf(cents / 1200.0) / sr;
        let inc = [inc_of(base_det - spread), inc_of(base_det + spread)];

        // brightness: velocity x hardness x tone, falling as the note rings
        let c0 = (f0 * (1.6 + 26.0 * v.powf(1.4) * hard) * bright + 300.0 * v).clamp(120.0, 18000.0);
        let c1 = (f0 * (1.2 + 3.0 * v * hard) * bright + 150.0).clamp(100.0, c0);

        // hammer thump: noise through the note's own lowpass, decaying, cut at 40 ms
        let nlen = (0.012 - 0.006 * reg).clamp(0.004, 0.012);
        let hammer_amp = hammer * gain * 0.1 * v.powf(1.3) * (0.5 + 0.5 * hard);

        // body EQ, same chain as the shared web filter
        let b = body;
        let chain = [
            Biquad::new(FilterKind::Peaking, sr, 220.0, 1.1, 4.0 * b),
            Biquad::new(FilterKind::Peaking, sr, 2600.0, 1.4, 2.5 * b),
            Biquad::new(FilterKind::Highshelf, sr, 7000.0, 1.0, -3.0 * b),
        ];

        let pan_pos = ((pitch - 64.0) / 40.0).clamp(-1.0, 1.0) as f32 * stereo;
        let pan = crate::dsp::pan_gains(pan_pos);

        let rng = Rng::new(self.rng.next_u64());
        let mut voice = PianoVoice {
            start: at,
            end_t,
            off_t,
            rel_tau,
            rel_level: 0.0,
            amp,
            aft,
            tau1,
            tau2,
            t2,
            v_t2: amp * (aft + (1.0 - aft) * (-2.2f32).exp()),
            cut: Biquad::new(FilterKind::Lowpass, sr, c0, 0.6, 0.0),
            cut_c0: c0,
            cut_c1: c1,
            cut_tc: tau1 * 1.3 + 0.05,
            phase: [0.0; 2],
            inc,
            table: piano_table(sr, reg_i),
            hammer_amp: if hammer > 0.0 { hammer_amp } else { 0.0 },
            hammer_tc: nlen / 3.0,
            rng,
            pan,
            body: [chain, chain],
            done: false,
        };
        if off_t.is_finite() {
            voice.rel_level = voice.env(off_t);
        }
        self.voices.push(voice);
        if self.voices.len() > 48 {
            self.voices.remove(0);
        }
    }

    fn set(&mut self, name: &str, value: f64) {
        self.p.set(PIANO, name, value);
    }

    fn render(&mut self, l: &mut [f32], r: &mut [f32], start: u64) {
        let sr = self.sr;
        for i in 0..l.len() {
            let f = start + i as u64;
            let (mut sum_l, mut sum_r) = (0.0f32, 0.0f32);
            for v in self.voices.iter_mut() {
                if v.done || f < v.start {
                    continue;
                }
                let n = f - v.start;
                let t = n as f32 / sr;
                if t >= v.end_t {
                    v.done = true;
                    continue;
                }
                // control-rate update of the brightness lowpass (every 32 frames)
                if n & 31 == 0 {
                    let c = v.cutoff(t);
                    v.cut.set(FilterKind::Lowpass, sr, c, 0.6, 0.0);
                }
                let mut x = 0.0f32;
                for k in 0..2 {
                    x += read_piano_table(&v.table, v.phase[k]);
                    v.phase[k] = (v.phase[k] + v.inc[k]).fract();
                }
                if v.hammer_amp > 0.0 && t < 0.04 {
                    x += v.rng.bipolar() * v.hammer_amp * (-t / v.hammer_tc).exp();
                }
                let s = v.cut.process(x) * v.gain_at(t);
                let (pl, pr) = (s * v.pan.0, s * v.pan.1);
                sum_l += piano_body(&mut v.body[0], pl);
                sum_r += piano_body(&mut v.body[1], pr);
            }
            l[i] += sum_l;
            r[i] += sum_r;
        }
        self.voices.retain(|v| !v.done);
    }
}

// ---------- Drum Rack and Sampler (WAV files from the samples folder) ----------

const DEFAULT_PADS: &[(i32, &str)] = &[
    (36, "kick.wav"),
    (38, "snare.wav"),
    (39, "clap.wav"),
    (37, "rim.wav"),
    (42, "hat.wav"),
    (46, "openhat.wav"),
    (45, "tom.wav"),
    (50, "perc.wav"),
];

fn samples_dir(ctx: &Ctx) -> PathBuf {
    ctx.plugins.parent().map(|d| d.join("samples")).unwrap_or_else(|| PathBuf::from("samples"))
}

struct PadVoice {
    audio: Arc<assets::Audio>,
    pos: f64,
    rate: f64,
    group: Option<String>,
    amp: f32,
    fade: f32,
    done: bool,
}

struct DrumRack {
    p: Params,
    sr: f32,
    pads: Vec<(i32, Arc<assets::Audio>, String)>,
    voices: Vec<PadVoice>,
}

impl DrumRack {
    fn new(p: Params, sr: f32, ctx: &Ctx, model: &Model) -> Self {
        let dir = samples_dir(ctx);
        let custom: Vec<(i32, String)> = model
            .pads
            .as_ref()
            .and_then(|v| v.as_object())
            .map(|m| m.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_str()?.to_string()))).collect())
            .unwrap_or_default();
        let mut pads = vec![];
        let names: Vec<(i32, String)> = if custom.is_empty() {
            DEFAULT_PADS.iter().map(|(n, f)| (*n, f.to_string())).collect()
        } else {
            custom
        };
        for (note, file) in names {
            if let Some(a) = assets::cached(&dir.join(&file), sr) {
                pads.push((note, a, file));
            }
        }
        DrumRack { p, sr, pads, voices: vec![] }
    }

    fn group_of(&self, note: i32) -> Option<String> {
        match self.p.get("chokeMode").round() as i32 {
            0 => None,
            1 => match note {
                42 | 44 | 46 => Some("hat".to_string()),
                _ => None,
            },
            _ => Some(format!("pad{note}")),
        }
    }
}

impl Instrument for DrumRack {
    fn note_on(&mut self, pitch: f64, vel: f64, _at: u64, _dur: u64) {
        let note = pitch.round() as i32;
        let Some((_, audio, _)) = self.pads.iter().find(|(n, _, _)| *n == note).cloned() else { return };
        let rate = 2f64.powf(self.p.get("tune") / 12.0);
        let group = self.group_of(note);
        // choke: a new hit in a group cuts the voices already sounding in that group
        if let Some(g) = &group {
            for v in self.voices.iter_mut() {
                if v.group.as_deref() == Some(g.as_str()) {
                    v.done = true;
                }
            }
        }
        self.voices.push(PadVoice {
            audio,
            pos: 0.0,
            rate,
            group,
            amp: self.p.get("gain") as f32 * vel.clamp(0.0, 1.0) as f32,
            fade: 0.0,
            done: false,
        });
    }

    fn set(&mut self, name: &str, value: f64) {
        self.p.set(DRUMRACK, name, value);
    }

    fn render(&mut self, l: &mut [f32], r: &mut [f32], _start: u64) {
        let len_cap = (self.p.get("decay") as f32 * self.sr) as f64;
        for i in 0..l.len() {
            let mut out_l = 0.0f32;
            let mut out_r = 0.0f32;
            for v in self.voices.iter_mut() {
                if v.done {
                    continue;
                }
                let ch = &v.audio.channels;
                let idx = v.pos.floor() as usize;
                if idx >= ch[0].len() || v.pos >= len_cap {
                    v.done = true;
                    continue;
                }
                let frac = (v.pos - idx as f64) as f32;
                let s0 = ch[0][idx];
                let s1 = ch[0].get(idx + 1).copied().unwrap_or(s0);
                let m = s0 + (s1 - s0) * frac;
                let r0 = ch.get(1).map_or(m, |c| c[idx.min(c.len() - 1)]);
                let fade = if v.pos > len_cap * 0.8 { ((len_cap - v.pos) / (len_cap * 0.2)).clamp(0.0, 1.0) as f32 } else { 1.0 };
                out_l += m * v.amp * fade;
                out_r += r0 * v.amp * fade;
                v.pos += v.rate;
                v.fade = fade;
            }
            l[i] += out_l;
            r[i] += out_r;
            // choke: a new hat cuts the open hat (group "hat") that was already sounding
        }
        self.voices.retain(|v| !v.done);
    }
}

struct Sampler {
    p: Params,
    sr: f32,
    audio: Option<Arc<assets::Audio>>,
    voices: Vec<PadVoice>,
}

impl Sampler {
    fn new(p: Params, sr: f32, ctx: &Ctx, model: &Model) -> Self {
        let name = match model.sample.as_ref() {
            Some(serde_json::Value::String(s)) => Some(s.clone()),
            Some(serde_json::Value::Object(o)) => o.get("name").and_then(|v| v.as_str()).map(String::from),
            _ => None,
        };
        let audio = name.and_then(|n| assets::cached(&samples_dir(ctx).join(n), sr));
        Sampler { p, sr, audio, voices: vec![] }
    }
}

impl Instrument for Sampler {
    fn note_on(&mut self, pitch: f64, vel: f64, _at: u64, _dur: u64) {
        let Some(audio) = self.audio.clone() else { return };
        let root = self.p.get("root");
        let rate = 2f64.powf((pitch - root) / 12.0) * if self.p.get("reverse") >= 0.5 { -1.0 } else { 1.0 };
        let len = audio.channels[0].len() as f64;
        let start = self.p.get("start") * len;
        self.voices.push(PadVoice {
            audio,
            pos: if rate < 0.0 { len - 1.0 } else { start },
            rate: rate.abs().max(1e-6),
            group: None,
            amp: self.p.get("gain") as f32 * vel.clamp(0.0, 1.0) as f32,
            fade: 0.0,
            done: false,
        });
    }

    fn set(&mut self, name: &str, value: f64) {
        self.p.set(SAMPLER, name, value);
    }

    fn render(&mut self, l: &mut [f32], r: &mut [f32], _start: u64) {
        let end = self.p.get("end");
        for i in 0..l.len() {
            let mut out_l = 0.0f32;
            let mut out_r = 0.0f32;
            for v in self.voices.iter_mut() {
                if v.done {
                    continue;
                }
                let ch = &v.audio.channels;
                let len = ch[0].len() as f64;
                let stop = len * end;
                let idx = v.pos.floor() as usize;
                if (v.pos >= stop && v.rate > 0.0) || idx >= ch[0].len() {
                    v.done = true;
                    continue;
                }
                out_l += ch[0][idx] * v.amp;
                out_r += ch.get(1).map_or(ch[0][idx], |c| c[idx]) * v.amp;
                v.pos += v.rate;
            }
            l[i] += out_l;
            r[i] += out_r;
        }
        self.voices.retain(|v| !v.done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Extra;

    #[test]
    fn procedural_devices_make_sound() {
        let ctx = Ctx { sr: 48_000.0, plugins: PathBuf::from("/nonexistent"), seed: 9 };
        for kind in ["pluck", "fm", "drums", "piano"] {
            let spec = spec(kind).unwrap();
            let params = Params::new(spec, &Extra::new());
            let model = Model { kind: kind.into(), params: Extra::new(), sample: None, pads: None, extra: Extra::new() };
            let mut inst = new(kind, &params, &model, &ctx).unwrap();
            inst.note_on(if kind == "drums" { 36.0 } else { 60.0 }, 0.9, 0, 9_600);
            let mut l = vec![0.0f32; 9_600];
            let mut r = vec![0.0f32; 9_600];
            inst.render(&mut l, &mut r, 0);
            let peak = l.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak > 1e-4, "{kind} is silent");
            assert!(peak < 8.0, "{kind} peak {peak}");
        }
    }
}
