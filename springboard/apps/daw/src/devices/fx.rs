//! The 17 effects, ported from `data/plugins/<id>/index.js`. Same parameter names, ranges and routing.
//! Differences from the Web Audio graph are noted on each device.

use super::{p, Bypass, Curve, Ctx, Effect, Params, P};
use crate::assets;
use crate::conv::Convolver;
use crate::dsp::{db_to_gain, Biquad, Compressor, Delay, FilterKind, Lfo, Osc, Rng, Wave};
use std::f32::consts::{PI, TAU};

const LIN: Curve = Curve::Lin;
const LOG: Curve = Curve::Log;
const INT: Curve = Curve::Int;

pub fn spec(kind: &str) -> Option<&'static [P]> {
    Some(match kind {
        "filter" => FILTER,
        "drive" => DRIVE,
        "eq" => EQ,
        "comp" => COMP,
        "delay" => DELAY,
        "tremolo" => TREMOLO,
        "chorus" => CHORUS,
        "phaser" => PHASER,
        "autofilter" => AUTOFILTER,
        "crush" => CRUSH,
        "stereo" => STEREO,
        "duck" => DUCK,
        "limiter" => LIMITER,
        "reverb" => REVERB,
        "space" => SPACE,
        "lofi" => LOFI,
        "tapedelay" => TAPEDELAY,
        _ => return None,
    })
}

pub fn new(kind: &str, params: &Params, ctx: &Ctx) -> Box<dyn Effect> {
    let sr = ctx.sr;
    let p = params.clone();
    match kind {
        "filter" => Box::new(Filter::new(p, sr)),
        "drive" => Box::new(Drive::new(p, sr)),
        "eq" => Box::new(Eq::new(p, sr)),
        "comp" => Box::new(Comp::new(p, sr)),
        "delay" => Box::new(FbDelay::new(p, sr)),
        "tremolo" => Box::new(Tremolo::new(p, sr)),
        "chorus" => Box::new(Chorus::new(p, sr)),
        "phaser" => Box::new(Phaser::new(p, sr)),
        "autofilter" => Box::new(AutoFilter::new(p, sr)),
        "crush" => Box::new(Crush::new(p, sr)),
        "stereo" => Box::new(Stereo::new(p, sr)),
        "duck" => Box::new(Duck::new(p, sr)),
        "limiter" => Box::new(Limiter::new(p, sr)),
        "reverb" => Box::new(Reverb::new(p, sr, ctx.seed)),
        "space" => Box::new(Space::new(p, sr, ctx)),
        "lofi" => Box::new(Lofi::new(p, sr, ctx)),
        "tapedelay" => Box::new(TapeDelay::new(p, sr)),
        _ => Box::new(Bypass),
    }
}

// ---------- helpers ----------

/// Tanh-based saturation over [-1, 1] (a WaveShaper curve in the web app).
#[inline]
fn tanh_shape(x: f32, drive: f32) -> f32 {
    (drive * x).tanh() / drive.tanh()
}

// ---------- Filter ----------

const FILTER: &[P] = &[p("type", 0.0, 2.0, 0.0, INT, ""), p("freq", 40.0, 18000.0, 18000.0, LOG, "Hz"), p("q", 0.0, 20.0, 1.0, LIN, "")];

struct Filter {
    p: Params,
    sr: f32,
    l: Biquad,
    r: Biquad,
}

impl Filter {
    fn new(p: Params, sr: f32) -> Self {
        let mut f = Filter { p, sr, l: Biquad::default(), r: Biquad::default() };
        f.apply();
        f
    }
    fn apply(&mut self) {
        let kind = match self.p.get("type").round() as i32 {
            1 => FilterKind::Highpass,
            2 => FilterKind::Bandpass,
            _ => FilterKind::Lowpass,
        };
        let (f, q) = (self.p.get("freq") as f32, self.p.get("q") as f32);
        self.l.set(kind, self.sr, f, q, 0.0);
        self.r.set(kind, self.sr, f, q, 0.0);
    }
}

impl Effect for Filter {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            l[i] = self.l.process(l[i]);
            r[i] = self.r.process(r[i]);
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(FILTER, name, v);
        self.apply();
    }
}

// ---------- Drive ----------

const DRIVE: &[P] = &[
    p("amount", 0.0, 1.0, 0.3, LIN, ""),
    p("tone", 500.0, 16000.0, 6000.0, LOG, "Hz"),
    p("mix", 0.0, 1.0, 1.0, LIN, ""),
    p("mode", 0.0, 2.0, 0.0, INT, ""),
    p("autogain", 0.0, 1.0, 1.0, INT, ""),
];

/// Waveshaper transfer function for the mode, as in the web app (normalised to peak 1).
fn drive_fn(mode: i32, amount: f32, x: f32) -> f32 {
    match mode {
        1 => {
            let u = (x * (1.0 + amount * 12.0)).clamp(-1.0, 1.0);
            u - u * u * u / 3.0
        }
        2 => {
            let u = x * (1.0 + amount * 20.0);
            if u >= 0.0 {
                u.tanh()
            } else {
                (u * 0.6).tanh() / 0.6 * 0.8
            }
        }
        _ => (1.0 + amount * 60.0).tanh().signum() * ((1.0 + amount * 60.0) * x).tanh(),
    }
}

/// Builds the table (2048 points over [-1, 1]) and the auto-gain for a -12 dBFS sine.
fn drive_table(mode: i32, amount: f32) -> (Vec<f32>, f32) {
    const N: usize = 2048;
    let mut c: Vec<f32> = (0..N).map(|i| drive_fn(mode, amount, i as f32 / ((N - 1) as f32 / 2.0) - 1.0)).collect();
    let peak = c.iter().fold(1e-9f32, |m, v| m.max(v.abs()));
    c.iter_mut().for_each(|v| *v /= peak);
    let a = 10f32.powf(-12.0 / 20.0);
    let (mut si, mut so) = (0.0f32, 0.0f32);
    let dc = (c[(N - 1) / 2] + c[N / 2]) / 2.0;
    for j in 0..256 {
        let x = a * (TAU * j as f32 / 256.0).sin();
        let pos = (x + 1.0) * (N - 1) as f32 / 2.0;
        let i0 = pos.floor() as usize;
        let fr = pos - i0 as f32;
        let y = c[i0] + (c[(i0 + 1).min(N - 1)] - c[i0]) * fr;
        si += x * x;
        so += (y - dc) * (y - dc);
    }
    let gain = (so / si).sqrt();
    (c, if gain.is_finite() && gain > 0.0 { gain } else { 1.0 })
}

struct Drive {
    p: Params,
    sr: f32,
    table: Vec<f32>,
    auto_gain: f32,
    key: (i32, i64),
    dc: Biquad,
    lp: Biquad,
    lp_r: Biquad,
    dc_r: Biquad,
    wet: f32,
    dry: f32,
}

impl Drive {
    fn new(p: Params, sr: f32) -> Self {
        let mut d = Drive {
            p,
            sr,
            table: vec![],
            auto_gain: 1.0,
            key: (-1, -1),
            dc: Biquad::default(),
            lp: Biquad::default(),
            lp_r: Biquad::default(),
            dc_r: Biquad::default(),
            wet: 0.0,
            dry: 1.0,
        };
        d.apply();
        d
    }
    fn apply(&mut self) {
        let mode = self.p.get("mode").round() as i32;
        let amount = self.p.get("amount") as f32;
        let key = (mode, (amount * 200.0).round() as i64);
        if key != self.key {
            self.key = key;
            let (t, g) = drive_table(mode, amount);
            self.table = t;
            self.auto_gain = g;
            let dcf = if mode == 2 { 20.0 } else { 5.0 };
            self.dc = Biquad::new(FilterKind::Highpass, self.sr, dcf, 0.707, 0.0);
            self.dc_r = self.dc;
        }
        let tone = self.p.get("tone") as f32;
        self.lp = Biquad::new(FilterKind::Lowpass, self.sr, tone, 1.0, 0.0);
        self.lp_r = self.lp;
        let auto = self.p.get("autogain") >= 0.5;
        let mix = self.p.get("mix") as f32;
        self.wet = if auto { mix / self.auto_gain } else { mix * (1.0 - amount * 0.4) };
        self.dry = 1.0 - mix;
    }
    fn shape(&self, x: f32) -> f32 {
        let n = self.table.len();
        let pos = (x.clamp(-1.0, 1.0) + 1.0) * (n - 1) as f32 / 2.0;
        let i0 = pos.floor() as usize;
        let fr = pos - i0 as f32;
        self.table[i0] + (self.table[(i0 + 1).min(n - 1)] - self.table[i0]) * fr
    }
}

impl Effect for Drive {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            let wl = self.lp.process(self.dc.process(self.shape(l[i])));
            let wr = self.lp_r.process(self.dc_r.process(self.shape(r[i])));
            let (dl, dr) = (l[i] * self.dry, r[i] * self.dry);
            l[i] = dl + wl * self.wet;
            r[i] = dr + wr * self.wet;
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(DRIVE, name, v);
        self.apply();
    }
}

// ---------- EQ ----------

const EQ: &[P] = &[
    p("low", -18.0, 18.0, 0.0, LIN, "dB"),
    p("mid", -18.0, 18.0, 0.0, LIN, "dB"),
    p("high", -18.0, 18.0, 0.0, LIN, "dB"),
    p("hpf", 20.0, 500.0, 20.0, LOG, "Hz"),
    p("lowFreq", 60.0, 400.0, 220.0, LOG, "Hz"),
    p("midFreq", 200.0, 5000.0, 1100.0, LOG, "Hz"),
    p("midQ", 0.3, 4.0, 0.8, LIN, ""),
    p("highFreq", 2000.0, 12000.0, 4500.0, LOG, "Hz"),
    p("lpf", 2000.0, 20000.0, 20000.0, LOG, "Hz"),
];

struct Eq {
    p: Params,
    sr: f32,
    /// [hp, lo, mid, hi, lp] per channel
    l: [Biquad; 5],
    r: [Biquad; 5],
}

impl Eq {
    fn new(p: Params, sr: f32) -> Self {
        let mut e = Eq { p, sr, l: [Biquad::default(); 5], r: [Biquad::default(); 5] };
        e.apply();
        e
    }
    fn apply(&mut self) {
        let sr = self.sr;
        let hpf = self.p.get("hpf") as f32;
        let hp_f = if hpf <= 20.5 { 5.0 } else { hpf };
        let lpf = self.p.get("lpf") as f32;
        let lp_f = if lpf >= 19900.0 { (sr / 2.0).min(24000.0) } else { lpf };
        let bands = [
            Biquad::new(FilterKind::Highpass, sr, hp_f, 0.707, 0.0),
            Biquad::new(FilterKind::Lowshelf, sr, self.p.get("lowFreq") as f32, 1.0, self.p.get("low") as f32),
            Biquad::new(
                FilterKind::Peaking,
                sr,
                self.p.get("midFreq") as f32,
                self.p.get("midQ") as f32,
                self.p.get("mid") as f32,
            ),
            Biquad::new(FilterKind::Highshelf, sr, self.p.get("highFreq") as f32, 1.0, self.p.get("high") as f32),
            Biquad::new(FilterKind::Lowpass, sr, lp_f, 0.707, 0.0),
        ];
        self.l = bands;
        self.r = bands;
    }
}

impl Effect for Eq {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            let mut x = l[i];
            for b in self.l.iter_mut() {
                x = b.process(x);
            }
            l[i] = x;
            let mut y = r[i];
            for b in self.r.iter_mut() {
                y = b.process(y);
            }
            r[i] = y;
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(EQ, name, v);
        self.apply();
    }
}

// ---------- Comp ----------

const COMP: &[P] = &[
    p("threshold", -60.0, 0.0, -18.0, LIN, "dB"),
    p("ratio", 1.0, 20.0, 4.0, LIN, ":1"),
    p("attack", 0.001, 0.3, 0.01, LOG, "s"),
    p("release", 0.02, 1.0, 0.2, LOG, "s"),
    p("makeup", 0.0, 24.0, 3.0, LIN, "dB"),
    p("mix", 0.0, 1.0, 1.0, LIN, ""),
];

struct Comp {
    p: Params,
    comp: Compressor,
    makeup: f32,
    wet: f32,
    dry: f32,
}

impl Comp {
    fn new(p: Params, sr: f32) -> Self {
        let mut c = Comp { p, comp: Compressor::new(sr), makeup: 1.0, wet: 1.0, dry: 0.0 };
        c.apply();
        c
    }
    fn apply(&mut self) {
        self.comp.threshold = self.p.get("threshold") as f32;
        self.comp.ratio = self.p.get("ratio") as f32;
        self.comp.attack = self.p.get("attack") as f32;
        self.comp.release = self.p.get("release") as f32;
        let mix = self.p.get("mix") as f32;
        self.makeup = db_to_gain(self.p.get("makeup") as f32) * mix;
        self.wet = 1.0;
        self.dry = 1.0 - mix;
    }
}

impl Effect for Comp {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            let (dl, dr) = (l[i], r[i]);
            let g = self.comp.gain(dl, dr) * self.makeup;
            l[i] = dl * self.dry + dl * g * self.wet;
            r[i] = dr * self.dry + dr * g * self.wet;
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(COMP, name, v);
        self.apply();
    }
}

// ---------- Delay (tempo-synced feedback) ----------

const DELAY: &[P] = &[
    p("time", 1.0, 8.0, 3.0, INT, "/16"),
    p("feedback", 0.0, 0.9, 0.35, LIN, ""),
    p("tone", 500.0, 16000.0, 4000.0, LOG, "Hz"),
    p("mix", 0.0, 1.0, 0.25, LIN, ""),
];

struct FbDelay {
    p: Params,
    sr: f32,
    bpm: f64,
    dl: Delay,
    dr: Delay,
    lp: Biquad,
    lp_r: Biquad,
}

impl FbDelay {
    fn new(p: Params, sr: f32) -> Self {
        let mut d = FbDelay {
            p,
            sr,
            bpm: 120.0,
            dl: Delay::new(sr, 4.0),
            dr: Delay::new(sr, 4.0),
            lp: Biquad::default(),
            lp_r: Biquad::default(),
        };
        d.apply();
        d
    }
    fn apply(&mut self) {
        self.lp = Biquad::new(FilterKind::Lowpass, self.sr, self.p.get("tone") as f32, 1.0, 0.0);
        self.lp_r = self.lp;
    }
    fn delay_samples(&self) -> f32 {
        let secs = self.p.get("time") * 60.0 / self.bpm / 4.0;
        (secs as f32 * self.sr).clamp(0.0, 3.99 * self.sr)
    }
}

impl Effect for FbDelay {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        let d = self.delay_samples();
        let fb = self.p.get("feedback") as f32;
        let mix = self.p.get("mix") as f32;
        for i in 0..l.len() {
            let (inl, inr) = (l[i], r[i]);
            let ol = self.dl.read(d);
            let or = self.dr.read(d);
            let wl = self.lp.process(ol);
            let wr = self.lp_r.process(or);
            self.dl.write(inl + wl * fb);
            self.dr.write(inr + wr * fb);
            l[i] = inl + wl * mix;
            r[i] = inr + wr * mix;
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(DELAY, name, v);
        self.apply();
    }
    fn tempo(&mut self, bpm: f64, _origin: u64) {
        self.bpm = bpm;
    }
}

// ---------- Tremolo ----------

const TREMOLO: &[P] = &[
    p("rate", 1.0, 16.0, 2.0, INT, "/16"),
    p("depth", 0.0, 1.0, 0.6, LIN, ""),
    p("shape", 0.0, 2.0, 0.0, INT, ""),
    p("pan", 0.0, 1.0, 0.0, INT, ""),
];

struct Tremolo {
    p: Params,
    sr: f32,
    bpm: f64,
    lfo: Osc,
    shape: Wave,
}

impl Tremolo {
    fn new(p: Params, sr: f32) -> Self {
        Tremolo { p, sr, bpm: 120.0, lfo: Osc::new(sr, 1.0), shape: Wave::Sine }
    }
    fn retune(&mut self) {
        let hz = 1.0 / (self.p.get("rate") * 60.0 / self.bpm / 4.0);
        self.lfo.set_freq(hz as f32);
        self.shape = match self.p.get("shape").round() as i32 {
            1 => Wave::Square,
            2 => Wave::Triangle,
            _ => Wave::Sine,
        };
    }
}

impl Effect for Tremolo {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        let depth = self.p.get("depth") as f32;
        let pan_mode = self.p.get("pan") >= 0.5;
        for i in 0..l.len() {
            let m = self.lfo.tick(self.shape);
            if pan_mode {
                let (gl, gr) = crate::dsp::pan_gains((depth * m).clamp(-1.0, 1.0));
                l[i] *= gl * std::f32::consts::SQRT_2;
                r[i] *= gr * std::f32::consts::SQRT_2;
            } else {
                let trem = depth / 2.0;
                let g = (1.0 - trem) + trem * m;
                l[i] *= g;
                r[i] *= g;
            }
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(TREMOLO, name, v);
        self.retune();
    }
    fn tempo(&mut self, bpm: f64, _origin: u64) {
        self.bpm = bpm;
        self.retune();
    }
}

// ---------- Chorus ----------

const CHORUS: &[P] = &[
    p("rate", 0.05, 8.0, 0.8, LOG, "Hz"),
    p("depth", 0.0, 1.0, 0.5, LIN, ""),
    p("width", 0.0, 1.0, 0.8, LIN, ""),
    p("mix", 0.0, 1.0, 0.4, LIN, ""),
];

struct Chorus {
    p: Params,
    sr: f32,
    lfo: Osc,
    lines: [Delay; 2],
}

impl Chorus {
    fn new(p: Params, sr: f32) -> Self {
        let rate = p.get("rate") as f32;
        Chorus { p, sr, lfo: Osc::new(sr, rate), lines: [Delay::new(sr, 0.1), Delay::new(sr, 0.1)] }
    }
}

impl Effect for Chorus {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        let depth = self.p.get("depth") as f32;
        let width = self.p.get("width") as f32;
        let mix = self.p.get("mix") as f32;
        let base = [0.014f32, 0.019];
        let sign = [1.0f32, -1.0];
        let (dry, wet) = (1.0 - mix * 0.5, mix * 0.7);
        for i in 0..l.len() {
            let m = self.lfo.tick(Wave::Sine);
            let (inl, inr) = (l[i], r[i]);
            // both lines read the same mono input (the web graph sums the input into each line)
            let mono = (inl + inr) * 0.5;
            let mut out_l = 0.0f32;
            let mut out_r = 0.0f32;
            for k in 0..2 {
                let secs = base[k] + m * sign[k] * depth * 0.006;
                let delay = (secs * self.sr).max(1.0);
                let y = self.lines[k].read(delay);
                self.lines[k].write(mono);
                let (gl, gr) = crate::dsp::pan_gains((-sign[k] * width).clamp(-1.0, 1.0));
                out_l += y * gl * std::f32::consts::SQRT_2;
                out_r += y * gr * std::f32::consts::SQRT_2;
            }
            l[i] = inl * dry + out_l * wet;
            r[i] = inr * dry + out_r * wet;
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(CHORUS, name, v);
        self.lfo.set_freq(self.p.get("rate") as f32);
    }
}

// ---------- Phaser ----------

const PHASER: &[P] = &[
    p("rate", 0.05, 8.0, 0.5, LOG, "Hz"),
    p("depth", 0.0, 1.0, 0.7, LIN, ""),
    p("freq", 200.0, 4000.0, 800.0, LOG, "Hz"),
    p("feedback", 0.0, 0.9, 0.5, LIN, ""),
    p("mix", 0.0, 1.0, 0.5, LIN, ""),
];

struct Phaser {
    p: Params,
    sr: f32,
    lfo: Lfo,
    stages_l: [Biquad; 5],
    stages_r: [Biquad; 5],
    last_l: f32,
    last_r: f32,
}

impl Phaser {
    fn new(p: Params, sr: f32) -> Self {
        let mut f = Phaser {
            p,
            sr,
            lfo: Lfo::new(sr, 0.5),
            stages_l: [Biquad::default(); 5],
            stages_r: [Biquad::default(); 5],
            last_l: 0.0,
            last_r: 0.0,
        };
        f.lfo.set_rate(f.p.get("rate") as f32);
        f
    }
    fn retune(&mut self, m: f32) {
        let depth = self.p.get("depth") as f32;
        let base = self.p.get("freq") as f32;
        for i in 0..5 {
            let f = base * (1.0 + i as f32 * 0.35);
            // detune in cents: the web LFO drives each stage's detune by depth*2400 cents
            let f = f * 2f32.powf(m * depth * 2400.0 / 1200.0);
            self.stages_l[i] = Biquad::new(FilterKind::Allpass, self.sr, f, 0.7, 0.0);
            self.stages_r[i] = self.stages_l[i];
        }
    }
}

impl Effect for Phaser {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        let fb = self.p.get("feedback") as f32;
        let mix = self.p.get("mix") as f32;
        let (dry, wet) = (1.0 - mix * 0.5, mix * 0.5);
        for i in 0..l.len() {
            let m = self.lfo.tick();
            if i % 32 == 0 {
                self.retune(m);
            }
            let inl = l[i];
            let inr = r[i];
            let mut x = inl + self.last_l * fb;
            for s in self.stages_l.iter_mut() {
                x = s.process(x);
            }
            let mut y = inr + self.last_r * fb;
            for s in self.stages_r.iter_mut() {
                y = s.process(y);
            }
            self.last_l = x;
            self.last_r = y;
            l[i] = inl * dry + x * wet;
            r[i] = inr * dry + y * wet;
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(PHASER, name, v);
        self.lfo.set_rate(self.p.get("rate") as f32);
    }
}

// ---------- Auto Filter ----------

const AUTOFILTER: &[P] = &[
    p("rate", 1.0, 32.0, 8.0, INT, "/16"),
    p("depth", 0.0, 1.0, 0.6, LIN, ""),
    p("freq", 60.0, 8000.0, 700.0, LOG, "Hz"),
    p("reso", 0.5, 18.0, 5.0, LOG, "Q"),
    p("shape", 0.0, 3.0, 0.0, INT, ""),
    p("mix", 0.0, 1.0, 1.0, LIN, ""),
];

struct AutoFilter {
    p: Params,
    sr: f32,
    bpm: f64,
    lfo: Osc,
    shape: Wave,
    f1: [Biquad; 2],
    f2: [Biquad; 2],
}

impl AutoFilter {
    fn new(p: Params, sr: f32) -> Self {
        let mut a = AutoFilter {
            p,
            sr,
            bpm: 120.0,
            lfo: Osc::new(sr, 1.0),
            shape: Wave::Sine,
            f1: [Biquad::default(); 2],
            f2: [Biquad::default(); 2],
        };
        a.retune();
        a
    }
    fn retune(&mut self) {
        self.lfo.set_freq((1.0 / (self.p.get("rate") * 60.0 / self.bpm / 4.0)) as f32);
        self.shape = match self.p.get("shape").round() as i32 {
            1 => Wave::Triangle,
            2 => Wave::Saw,
            3 => Wave::Square,
            _ => Wave::Sine,
        };
    }
}

impl Effect for AutoFilter {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        let depth = self.p.get("depth") as f32;
        let freq = self.p.get("freq") as f32;
        let reso = self.p.get("reso") as f32;
        let mix = self.p.get("mix") as f32;
        let wet_g = mix * (1.0 / (1.0 + reso * 0.08));
        let dry_g = 1.0 - mix;
        for i in 0..l.len() {
            let m = self.lfo.tick(self.shape);
            let cents = depth * 2400.0 * m;
            let f = (freq * 2f32.powf(cents / 1200.0)).clamp(20.0, self.sr * 0.45);
            for c in 0..2 {
                self.f1[c] = Biquad::new(FilterKind::Lowpass, self.sr, f, reso, 0.0);
                self.f2[c] = Biquad::new(FilterKind::Lowpass, self.sr, f, 0.7, 0.0);
            }
            let inl = l[i];
            let inr = r[i];
            let wl = self.f2[0].process(self.f1[0].process(inl));
            let wr = self.f2[1].process(self.f1[1].process(inr));
            l[i] = inl * dry_g + wl * wet_g;
            r[i] = inr * dry_g + wr * wet_g;
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(AUTOFILTER, name, v);
        self.retune();
    }
    fn tempo(&mut self, bpm: f64, _origin: u64) {
        self.bpm = bpm;
        self.retune();
    }
}

// ---------- Crush ----------

const CRUSH: &[P] = &[p("bits", 1.0, 16.0, 6.0, INT, "bit"), p("rate", 500.0, 22000.0, 8000.0, LOG, "Hz"), p("mix", 0.0, 1.0, 1.0, LIN, "")];

struct Crush {
    p: Params,
    sr: f32,
    lp: Biquad,
    lp_r: Biquad,
}

impl Crush {
    fn new(p: Params, sr: f32) -> Self {
        let mut c = Crush { p, sr, lp: Biquad::default(), lp_r: Biquad::default() };
        c.apply();
        c
    }
    fn apply(&mut self) {
        let rate = self.p.get("rate") as f32;
        self.lp = Biquad::new(FilterKind::Lowpass, self.sr, rate / 2.0, 4.0, 0.0);
        self.lp_r = self.lp;
    }
    fn alias(&self, x: f32) -> f32 {
        let amt = (1.0 - self.p.get("rate") as f32 / 22000.0).max(0.0) * 0.5;
        (x + amt * (x * PI * 6.0).sin() / 6.0) / (1.0 + amt / 6.0)
    }
    fn quant(&self, x: f32) -> f32 {
        let steps = 2f32.powf(self.p.get("bits") as f32 - 1.0);
        (x * steps).round() / steps
    }
}

impl Effect for Crush {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        let mix = self.p.get("mix") as f32;
        for i in 0..l.len() {
            let (inl, inr) = (l[i], r[i]);
            let wl = self.lp.process(self.quant(self.alias(inl.clamp(-1.0, 1.0))));
            let wr = self.lp_r.process(self.quant(self.alias(inr.clamp(-1.0, 1.0))));
            l[i] = inl * (1.0 - mix) + wl * mix;
            r[i] = inr * (1.0 - mix) + wr * mix;
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(CRUSH, name, v);
        self.apply();
    }
}

// ---------- Stereo ----------

const STEREO: &[P] = &[
    p("width", 0.0, 2.0, 1.0, LIN, ""),
    p("monobass", 20.0, 400.0, 120.0, LOG, "Hz"),
    p("bassmono", 0.0, 1.0, 1.0, INT, ""),
    p("pan", -1.0, 1.0, 0.0, LIN, ""),
    p("gain", -12.0, 12.0, 0.0, LIN, "dB"),
];

struct Stereo {
    p: Params,
    sr: f32,
    lp: [Biquad; 2],
    lp2: [Biquad; 2],
    hp: [Biquad; 2],
    hp2: [Biquad; 2],
}

impl Stereo {
    fn new(p: Params, sr: f32) -> Self {
        let mut s = Stereo {
            p,
            sr,
            lp: [Biquad::default(); 2],
            lp2: [Biquad::default(); 2],
            hp: [Biquad::default(); 2],
            hp2: [Biquad::default(); 2],
        };
        s.apply();
        s
    }
    fn apply(&mut self) {
        // Linkwitz-Riley-like pair: two Butterworth (Q = -3.01 dB) biquads per band, as in the web graph
        let f = self.p.get("monobass") as f32;
        for c in 0..2 {
            self.lp[c] = Biquad::new(FilterKind::Lowpass, self.sr, f, -3.01, 0.0);
            self.lp2[c] = self.lp[c];
            self.hp[c] = Biquad::new(FilterKind::Highpass, self.sr, f, -3.01, 0.0);
            self.hp2[c] = self.hp[c];
        }
    }
}

impl Effect for Stereo {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        let bass_mono = self.p.get("bassmono") >= 0.5;
        let w = self.p.get("width") as f32;
        let a = (1.0 + w) / 2.0;
        let b = (1.0 - w) / 2.0;
        let g = db_to_gain(self.p.get("gain") as f32);
        let th = (self.p.get("pan") as f32 + 1.0) * PI / 4.0;
        let (gl, gr) = (g * th.cos() * std::f32::consts::SQRT_2, g * th.sin() * std::f32::consts::SQRT_2);
        for i in 0..l.len() {
            let (inl, inr) = (l[i], r[i]);
            let low_l = self.lp2[0].process(self.lp[0].process(inl));
            let low_r = self.lp2[1].process(self.lp[1].process(inr));
            let (hl, hr) = if bass_mono {
                (self.hp2[0].process(self.hp[0].process(inl)), self.hp2[1].process(self.hp[1].process(inr)))
            } else {
                (inl, inr)
            };
            // width matrix on the high (or full) band
            let (wl, wr) = (a * hl + b * hr, a * hr + b * hl);
            let mono = if bass_mono { (low_l + low_r) * 0.5 } else { 0.0 };
            let (ol, or) = (wl + mono, wr + mono);
            l[i] = ol * gl;
            r[i] = or * gr;
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(STEREO, name, v);
        self.apply();
    }
}

// ---------- Pump (sidechain-style ducking, the `duck` device) ----------

const DUCK: &[P] = &[
    p("amount", 0.0, 1.0, 0.7, LIN, ""),
    p("release", 0.05, 1.0, 0.5, LIN, ""),
    p("curve", 0.5, 4.0, 1.5, LIN, ""),
    p("rate", 1.0, 16.0, 4.0, INT, "/16"),
    p("offset", -0.1, 0.1, 0.0, LIN, "beat"),
];

struct Duck {
    p: Params,
    sr: f32,
    bpm: f64,
    origin: u64,
    env: Vec<f32>,
    key: (u64, u64, u64),
}

impl Duck {
    fn new(p: Params, sr: f32) -> Self {
        let mut d = Duck { p, sr, bpm: 120.0, origin: 0, env: vec![], key: (0, 0, 0) };
        d.rebuild();
        d
    }
    fn rebuild(&mut self) {
        let beat = 60.0 / self.bpm;
        let period = self.p.get("rate") * beat / 4.0;
        let n = ((period * self.sr as f64).round() as usize).max(2);
        let release = self.p.get("release");
        let curve = self.p.get("curve") as f32;
        let key = (period.to_bits(), release.to_bits(), curve.to_bits() as u64);
        if key == self.key && self.env.len() == n {
            return;
        }
        self.key = key;
        let rel = ((n as f64 * release).round() as usize).max(1);
        let atk = (n >> 4).min(64);
        self.env = (0..n)
            .map(|i| {
                if i < atk {
                    1.0 - i as f32 / atk as f32
                } else if i < rel {
                    let t = (i - atk) as f32 / (rel - atk).max(1) as f32;
                    t.max(0.0).powf(1.0 / curve)
                } else {
                    1.0
                }
            })
            .collect();
    }
}

impl Effect for Duck {
    fn process(&mut self, start: u64, l: &mut [f32], r: &mut [f32]) {
        let amount = self.p.get("amount") as f32;
        let beat = 60.0 / self.bpm;
        let off = self.p.get("offset") * beat * self.sr as f64;
        let origin = self.origin as f64 + off;
        for i in 0..l.len() {
            let n = self.env.len() as f64;
            let pos = (start as f64 + i as f64 - origin).rem_euclid(n);
            let e = self.env[pos as usize];
            let g = (1.0 - amount) + amount * e;
            l[i] *= g;
            r[i] *= g;
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(DUCK, name, v);
        self.rebuild();
    }
    fn tempo(&mut self, bpm: f64, origin: u64) {
        self.bpm = bpm;
        self.origin = origin;
        self.rebuild();
    }
}

// ---------- Limiter ----------

const LIMITER: &[P] = &[
    p("gain", 0.0, 24.0, 0.0, LIN, "dB"),
    p("ceiling", -12.0, 0.0, -0.3, LIN, "dB"),
    p("release", 10.0, 1000.0, 120.0, LOG, "ms"),
    p("soft", 0.0, 1.0, 0.3, LIN, ""),
    p("output", -12.0, 6.0, 0.0, LIN, "dB"),
];

struct Limiter {
    p: Params,
    comp: Compressor,
    drive: f32,
    ceil: f32,
    knee: f32,
    trim: f32,
}

impl Limiter {
    fn new(p: Params, sr: f32) -> Self {
        let mut l = Limiter { p, comp: Compressor::new(sr), drive: 1.0, ceil: 1.0, knee: 0.1, trim: 1.0 };
        l.apply();
        l
    }
    fn apply(&mut self) {
        self.drive = db_to_gain(self.p.get("gain") as f32);
        self.ceil = db_to_gain(self.p.get("ceiling") as f32);
        self.comp.threshold = self.p.get("ceiling") as f32 - 1.0;
        self.comp.knee = self.p.get("soft") as f32 * 6.0;
        self.comp.ratio = 20.0;
        self.comp.attack = 0.0005;
        self.comp.release = self.p.get("release") as f32 / 1000.0;
        self.knee = 0.1 + self.p.get("soft") as f32 * 0.4;
        self.trim = db_to_gain(self.p.get("output") as f32);
    }
    /// Soft knee into the ceiling: unity below `t`, saturating smoothly to 1.
    fn sat(&self, x: f32) -> f32 {
        let t = 1.0 - self.knee;
        let a = x.abs();
        let y = if a <= t { a } else { t + (1.0 - t) * ((a - t) / (1.0 - t).max(1e-3)).tanh() };
        x.signum() * y.min(1.0)
    }
}

impl Effect for Limiter {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            let dl = l[i] * self.drive;
            let dr = r[i] * self.drive;
            let g = self.comp.gain(dl, dr);
            let (cl, cr) = (dl * g, dr * g);
            l[i] = self.sat(cl / self.ceil) * self.ceil * self.trim;
            r[i] = self.sat(cr / self.ceil) * self.ceil * self.trim;
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(LIMITER, name, v);
        self.apply();
    }
}

// ---------- Reverb (generated, damped noise impulse) ----------

const REVERB: &[P] = &[
    p("size", 0.2, 6.0, 2.0, LIN, "s"),
    p("damp", 1000.0, 16000.0, 6000.0, LOG, "Hz"),
    p("mix", 0.0, 1.0, 0.25, LIN, ""),
];

struct Reverb {
    p: Params,
    sr: f32,
    seed: u64,
    conv: [Convolver; 2],
    key: (i64, i64),
    wet: f32,
    dry: f32,
}

impl Reverb {
    fn new(p: Params, sr: f32, seed: u64) -> Self {
        let ir = impulse(sr, 0.5, 6000.0, seed);
        let mut r = Reverb {
            p,
            sr,
            seed,
            conv: [Convolver::new(&ir[0]), Convolver::new(&ir[1])],
            key: (-1, -1),
            wet: 0.0,
            dry: 1.0,
        };
        r.apply();
        r
    }
    fn apply(&mut self) {
        let size = self.p.get("size") as f32;
        let damp = self.p.get("damp") as f32;
        let key = ((size * 10.0).round() as i64, (damp / 500.0).round() as i64);
        if key != self.key {
            self.key = key;
            let ir = impulse(self.sr, size, damp, self.seed);
            self.conv = [Convolver::new(&ir[0]), Convolver::new(&ir[1])];
        }
        let mix = self.p.get("mix") as f32;
        self.wet = mix * 1.4;
        self.dry = 1.0 - mix * 0.5;
    }
}

/// Stereo impulse: exponentially decaying one-pole-filtered noise (the web version's `impulse()`).
fn impulse(sr: f32, secs: f32, damp: f32, seed: u64) -> Vec<Vec<f32>> {
    let len = (sr * secs).floor() as usize;
    let a = (-2.0 * PI * damp / sr).exp();
    let mut rng = Rng::new(seed);
    let mut out = vec![vec![0.0f32; len], vec![0.0f32; len]];
    for ch in out.iter_mut() {
        let mut lp = 0.0f32;
        for i in 0..len {
            let n = rng.bipolar() * (1.0 - i as f32 / len as f32).powi(3);
            lp = lp * a + n * (1.0 - a);
            ch[i] = lp * (1.0 + damp / 8000.0);
        }
    }
    out
}

impl Effect for Reverb {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            let (inl, inr) = (l[i], r[i]);
            let wl = self.conv[0].process(inl);
            let wr = self.conv[1].process(inr);
            l[i] = inl * self.dry + wl * self.wet;
            r[i] = inr * self.dry + wr * self.wet;
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(REVERB, name, v);
        self.apply();
    }
}

// ---------- Space (measured impulse responses) ----------

const SPACE: &[P] = &[
    p("space", 0.0, 5.0, 0.0, INT, ""),
    p("mix", 0.0, 1.0, 0.25, LIN, ""),
    p("predelay", 0.0, 200.0, 10.0, LIN, "ms"),
    p("decay", 0.2, 1.0, 1.0, LIN, "x"),
    p("tone", 1000.0, 18000.0, 9000.0, LOG, "Hz"),
    p("width", 0.0, 1.5, 1.0, LIN, ""),
];

const SPACE_IRS: [&str; 6] = ["hall", "cathedral", "church", "chamber", "room", "studio"];
const SPACE_TRIM: [f32; 6] = [0.6, 1.0, 1.0, 1.0, 1.0, 1.0];

struct Space {
    p: Params,
    sr: f32,
    plugins: std::path::PathBuf,
    pre: Delay,
    conv: [Convolver; 2],
    lp: Biquad,
    lp_r: Biquad,
    key: (usize, i64),
    trim: f32,
    wet: f32,
    dry: f32,
}

impl Space {
    fn new(p: Params, sr: f32, ctx: &Ctx) -> Self {
        let ir = space_ir(&ctx.plugins, 0, 1.0, sr);
        let mut s = Space {
            p,
            sr,
            plugins: ctx.plugins.clone(),
            pre: Delay::new(sr, 0.5),
            conv: [Convolver::new(&ir[0]), Convolver::new(&ir[1])],
            lp: Biquad::default(),
            lp_r: Biquad::default(),
            key: (usize::MAX, -1),
            trim: 1.0,
            wet: 0.0,
            dry: 1.0,
        };
        s.apply();
        s
    }
    fn apply(&mut self) {
        let idx = (self.p.get("space").round() as usize).min(SPACE_IRS.len() - 1);
        let dec = (self.p.get("decay") as f32 * 20.0).round() as i64;
        let key = (idx, dec);
        if key != self.key {
            self.key = key;
            let ir = space_ir(&self.plugins, idx, self.p.get("decay") as f32, self.sr);
            self.conv = [Convolver::new(&ir[0]), Convolver::new(&ir[1])];
            self.trim = SPACE_TRIM[idx];
        }
        self.lp = Biquad::new(FilterKind::Lowpass, self.sr, self.p.get("tone") as f32, 0.5, 0.0);
        self.lp_r = self.lp;
        self.wet = self.p.get("mix") as f32 * 1.2 * self.trim;
        self.dry = 1.0 - self.p.get("mix") as f32 * 0.5;
    }
}

/// The impulse response for `idx`, from the plugin's OGG file when it decodes, otherwise a generated tail.
/// Decay trims the tail with the web app's exponential fade.
fn space_ir(plugins: &std::path::Path, idx: usize, decay: f32, sr: f32) -> Vec<Vec<f32>> {
    let file = plugins.join("space").join(format!("{}.ogg", SPACE_IRS[idx]));
    let mut ir = match assets::load_stereo(&file, sr) {
        Some(v) => v,
        None => {
            let secs = [2.5f32, 4.0, 3.5, 1.2, 0.9, 0.7][idx];
            impulse(sr, secs, 7000.0, 0x5ACE + idx as u64)
        }
    };
    let decay = decay.clamp(0.2, 1.0);
    if decay < 1.0 {
        let k = 5.0 * (1.0 / decay - 1.0);
        let len = ((ir[0].len() as f32) * (0.25 + 0.75 * decay)) as usize;
        for ch in ir.iter_mut() {
            ch.truncate(len.max(64));
            let n = ch.len();
            for (i, x) in ch.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let fade = if i as f32 > n as f32 * 0.8 { (n - i) as f32 / (n as f32 * 0.2) } else { 1.0 };
                *x *= (-k * t).exp() * fade;
            }
        }
    }
    ir
}

impl Effect for Space {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        let pre = (self.p.get("predelay") as f32 / 1000.0 * self.sr).clamp(0.0, self.sr * 0.5 - 2.0);
        let w = self.p.get("width") as f32;
        let (a, b) = ((1.0 + w) / 2.0, (1.0 - w) / 2.0);
        for i in 0..l.len() {
            let (inl, inr) = (l[i], r[i]);
            let mono = (inl + inr) * 0.5;
            let pd = self.pre.read(pre);
            self.pre.write(mono);
            let cl = self.lp.process(self.conv[0].process(pd));
            let cr = self.lp_r.process(self.conv[1].process(pd));
            let (wl, wr) = (a * cl + b * cr, a * cr + b * cl);
            l[i] = inl * self.dry + wl * self.wet;
            r[i] = inr * self.dry + wr * self.wet;
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(SPACE, name, v);
        self.apply();
    }
}

// ---------- Lo-Fi ----------

const LOFI: &[P] = &[
    p("crackle", 0.0, 1.0, 0.3, LIN, ""),
    p("wow", 0.0, 1.0, 0.3, LIN, ""),
    p("age", 0.0, 1.0, 0.4, LIN, ""),
    p("drive", 0.0, 1.0, 0.3, LIN, ""),
    p("mix", 0.0, 1.0, 1.0, LIN, ""),
];

const LOFI_BASE: f32 = 0.008;

struct Lofi {
    p: Params,
    sr: f32,
    tape: Delay,
    dry_delay: Delay,
    wow: Osc,
    flut: Osc,
    hp: Biquad,
    lp: Biquad,
    crackle: Vec<f32>,
    crackle_pos: usize,
    rng: Rng,
    pre: f32,
    post: f32,
    wg: f32,
    fg: f32,
    crack_g: f32,
    wet: f32,
    dry: f32,
}

impl Lofi {
    fn new(p: Params, sr: f32, ctx: &Ctx) -> Self {
        let crackle = assets::load_mono(&ctx.plugins.join("lofi").join("crackle.ogg"), sr).unwrap_or_default();
        let mut l = Lofi {
            p,
            sr,
            tape: Delay::new(sr, 0.1),
            dry_delay: Delay::new(sr, 0.1),
            wow: Osc::new(sr, 0.45),
            flut: Osc::new(sr, 7.1),
            hp: Biquad::default(),
            lp: Biquad::default(),
            crackle,
            crackle_pos: 0,
            rng: Rng::new(ctx.seed ^ 0x10F1),
            pre: 0.25,
            post: 1.0,
            wg: 0.0,
            fg: 0.0,
            crack_g: 0.0,
            wet: 1.0,
            dry: 0.0,
        };
        l.apply();
        l
    }
    fn apply(&mut self) {
        let drive = self.p.get("drive") as f32;
        let d = 1.0 + drive * 7.0;
        self.pre = d / 4.0;
        self.post = 0.5 / (d * 0.5).tanh() * d.powf(-0.3);
        let age = self.p.get("age") as f32;
        self.hp = Biquad::new(FilterKind::Highpass, self.sr, 20.0 + age * 130.0, 0.6, 0.0);
        self.lp = Biquad::new(FilterKind::Lowpass, self.sr, 18000.0 * (3000.0f32 / 18000.0).powf(age), 0.5, 0.0);
        let wow = self.p.get("wow") as f32;
        self.wg = wow * 0.0035;
        self.fg = wow * 0.00035;
        let crackle = self.p.get("crackle") as f32;
        self.crack_g = crackle * crackle * 0.3;
        let mix = self.p.get("mix") as f32;
        self.wet = mix;
        self.dry = 1.0 - mix;
    }
    /// Next crackle sample: the decoded loop when available, otherwise sparse clicks.
    fn next_crackle(&mut self) -> f32 {
        if !self.crackle.is_empty() {
            let v = self.crackle[self.crackle_pos];
            self.crackle_pos = (self.crackle_pos + 1) % self.crackle.len();
            return v;
        }
        if self.rng.unit() < 0.0005 {
            self.rng.bipolar() * 0.5
        } else {
            0.0
        }
    }
}

impl Effect for Lofi {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            let mono = (l[i] + r[i]) * 0.5;
            let wow = self.wow.tick(Wave::Sine) * self.wg + self.flut.tick(Wave::Sine) * self.fg;
            let delay = ((LOFI_BASE + wow) * self.sr).clamp(1.0, 0.099 * self.sr);
            let dry_delayed = self.dry_delay.read(LOFI_BASE * self.sr);
            self.dry_delay.write(mono);
            let tapped = self.tape.read(delay);
            self.tape.write(mono);
            let shaped = (tapped * self.pre * 4.0).tanh() * self.post;
            let crackle = self.next_crackle() * self.crack_g;
            let wet = self.lp.process(self.hp.process(shaped + crackle));
            l[i] = dry_delayed * self.dry + wet * self.wet;
            r[i] = dry_delayed * self.dry + wet * self.wet;
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(LOFI, name, v);
        self.apply();
    }
}

// ---------- Tape Delay ----------

const TAPEDELAY: &[P] = &[
    p("time", 1.0, 16.0, 3.0, INT, "/16"),
    p("feedback", 0.0, 0.95, 0.45, LIN, ""),
    p("wow", 0.0, 1.0, 0.3, LIN, ""),
    p("tone", 500.0, 12000.0, 3000.0, LOG, "Hz"),
    p("mix", 0.0, 1.0, 0.3, LIN, ""),
    p("mode", 0.0, 1.0, 1.0, INT, ""),
];

struct TapeDelay {
    p: Params,
    sr: f32,
    bpm: f64,
    dl: Delay,
    dr: Delay,
    hp: [Biquad; 2],
    lp: [Biquad; 2],
    wow: Osc,
    flut: Osc,
}

impl TapeDelay {
    fn new(p: Params, sr: f32) -> Self {
        let mut t = TapeDelay {
            p,
            sr,
            bpm: 120.0,
            dl: Delay::new(sr, 5.0),
            dr: Delay::new(sr, 5.0),
            hp: [Biquad::default(); 2],
            lp: [Biquad::default(); 2],
            wow: Osc::new(sr, 0.55),
            flut: Osc::new(sr, 6.3),
        };
        t.apply();
        t
    }
    fn apply(&mut self) {
        for c in 0..2 {
            self.hp[c] = Biquad::new(FilterKind::Highpass, self.sr, 90.0, 0.707, 0.0);
            self.lp[c] = Biquad::new(FilterKind::Lowpass, self.sr, self.p.get("tone") as f32, 0.3, 0.0);
        }
    }
    fn sat(x: f32) -> f32 {
        (x * 1.6).tanh() / 1.6f32.tanh()
    }
}

impl Effect for TapeDelay {
    fn process(&mut self, _s: u64, l: &mut [f32], r: &mut [f32]) {
        let secs = (self.p.get("time") * 60.0 / self.bpm / 4.0).min(4.9);
        let base = secs as f32 * self.sr;
        let fb = self.p.get("feedback") as f32;
        let wow = self.p.get("wow") as f32;
        let pingpong = self.p.get("mode") >= 0.5;
        let (a, b) = if pingpong { (1.0f32, 0.0f32) } else { (0.7, 0.7) };
        let mix = self.p.get("mix") as f32;
        let (dry, wet) = (1.0 - mix * 0.3, mix);
        for i in 0..l.len() {
            let mod_s = self.wow.tick(Wave::Sine) * wow * 0.0025 * self.sr + self.flut.tick(Wave::Sine) * wow * 0.0003 * self.sr;
            let delay = (base + mod_s).clamp(1.0, 4.99 * self.sr);
            let (inl, inr) = (l[i], r[i]);
            let mono = (inl + inr) * 0.5;
            let ol = self.dl.read(delay);
            let or = self.dr.read(delay);
            let lpl = self.lp[0].process(self.hp[0].process(ol));
            let lpr = self.lp[1].process(self.hp[1].process(or));
            // feedback paths: L -> R, R -> L, each through the saturator
            let fl = Self::sat(lpl) * fb;
            let fr = Self::sat(lpr) * fb;
            self.dl.write(mono + fr);
            self.dr.write(fl);
            // taps: ping-pong puts the left line on the left, mono puts both on both sides
            let wl = a * lpl + b * lpr;
            let wr = b * lpl + a * lpr;
            l[i] = inl * dry + wl * wet;
            r[i] = inr * dry + wr * wet;
        }
    }
    fn set(&mut self, name: &str, v: f64) {
        self.p.set(TAPEDELAY, name, v);
        self.apply();
    }
    fn tempo(&mut self, bpm: f64, _origin: u64) {
        self.bpm = bpm;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Extra;

    fn ctx() -> Ctx {
        Ctx { sr: 48_000.0, plugins: std::path::PathBuf::from("/nonexistent"), seed: 1 }
    }

    #[test]
    fn every_effect_has_a_spec_and_builds() {
        for kind in [
            "filter", "drive", "eq", "comp", "delay", "tremolo", "chorus", "phaser", "autofilter", "crush", "stereo",
            "duck", "limiter", "reverb", "space", "lofi", "tapedelay",
        ] {
            let spec = spec(kind).unwrap_or_else(|| panic!("{kind}"));
            assert!(!spec.is_empty(), "{kind}");
            let params = Params::new(spec, &Extra::new());
            let mut fx = new(kind, &params, &ctx());
            let mut l = vec![0.1f32; 256];
            let mut r = vec![0.1f32; 256];
            fx.tempo(120.0, 0);
            fx.process(0, &mut l, &mut r);
            assert!(l.iter().all(|x| x.is_finite()), "{kind} produced non-finite output");
        }
    }

    #[test]
    fn unknown_effect_is_a_bypass() {
        assert!(spec("nope").is_none());
        let mut fx = crate::devices::effect("nope", &Extra::new(), &ctx());
        let mut l = vec![0.5f32; 8];
        let mut r = vec![0.5f32; 8];
        fx.process(0, &mut l, &mut r);
        assert_eq!(l[0], 0.5);
    }
}
