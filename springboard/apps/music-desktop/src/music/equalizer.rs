//! A 10-band graphic equalizer: presets, band gains in dB, and peaking biquad filters (RBJ cookbook) that the
//! audio engine runs per channel. `Equalizer::process` filters a block in place. Pure std.
#![allow(dead_code)] // the Equalizer panel and the engine's gain stage wire this in next

use std::f32::consts::PI;

/// Centre frequencies of the ten bands, in Hz.
pub const BANDS: [f32; 10] = [32.0, 64.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
/// The widest cut or boost of a band, in dB.
pub const MAX_DB: f32 = 12.0;

pub const PRESETS: [(&str, [f32; 10]); 6] = [
    ("Flat", [0.0; 10]),
    ("Bass Booster", [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Treble Booster", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 5.0, 6.0, 6.0]),
    ("Vocal", [-2.0, -2.0, -1.0, 1.0, 3.0, 4.0, 3.0, 1.0, 0.0, -1.0]),
    ("Loudness", [5.0, 4.0, 1.0, 0.0, -1.0, -1.0, 0.0, 2.0, 4.0, 5.0]),
    ("Rock", [4.0, 3.0, 2.0, -1.0, -2.0, -1.0, 2.0, 3.0, 4.0, 4.0]),
];

/// Coefficients of one peaking filter, normalised (`a0` = 1): y = b0 x + b1 x1 + b2 x2 - a1 y1 - a2 y2.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biquad {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl Biquad {
    pub const IDENTITY: Biquad = Biquad { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    /// A peaking EQ at `freq` with `gain_db`, bandwidth `q`, for `rate` Hz audio.
    pub fn peaking(freq: f32, gain_db: f32, q: f32, rate: f32) -> Biquad {
        if gain_db.abs() < 0.01 || freq >= rate / 2.0 {
            return Biquad::IDENTITY;
        }
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / rate;
        let alpha = w0.sin() / (2.0 * q);
        let cos = w0.cos();
        let a0 = 1.0 + alpha / a;
        Biquad {
            b0: (1.0 + alpha * a) / a0,
            b1: -2.0 * cos / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }

    /// The magnitude response at `freq`, as a gain factor.
    pub fn response(&self, freq: f32, rate: f32) -> f32 {
        let w = 2.0 * PI * freq / rate;
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let (nr, ni) = (self.b0 + self.b1 * c1 + self.b2 * c2, -(self.b1 * s1 + self.b2 * s2));
        let (dr, di) = (1.0 + self.a1 * c1 + self.a2 * c2, -(self.a1 * s1 + self.a2 * s2));
        ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt()
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct State {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

/// The equalizer: ten gains and the per-channel filter state.
#[derive(Clone, Debug)]
pub struct Equalizer {
    pub gains: [f32; 10],
    pub preamp_db: f32,
    pub enabled: bool,
    rate: f32,
    filters: [Biquad; 10],
    state: Vec<[State; 10]>,
}

impl Equalizer {
    pub fn new(rate: f32, channels: usize) -> Equalizer {
        let mut e = Equalizer {
            gains: [0.0; 10],
            preamp_db: 0.0,
            enabled: true,
            rate,
            filters: [Biquad::IDENTITY; 10],
            state: vec![[State::default(); 10]; channels.max(1)],
        };
        e.rebuild();
        e
    }

    fn rebuild(&mut self) {
        for (i, f) in BANDS.iter().enumerate() {
            self.filters[i] = Biquad::peaking(*f, self.gains[i].clamp(-MAX_DB, MAX_DB), 1.41, self.rate);
        }
    }

    pub fn set_gain(&mut self, band: usize, db: f32) {
        if band < 10 {
            self.gains[band] = db.clamp(-MAX_DB, MAX_DB);
            self.rebuild();
        }
    }

    /// Loads a preset by name; false for an unknown name.
    pub fn load_preset(&mut self, name: &str) -> bool {
        match PRESETS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            Some((_, g)) => {
                self.gains = *g;
                self.rebuild();
                true
            }
            None => false,
        }
    }

    /// The preset whose gains equal the current ones, if any.
    pub fn preset_name(&self) -> Option<&'static str> {
        PRESETS.iter().find(|(_, g)| *g == self.gains).map(|(n, _)| *n)
    }

    /// Filters interleaved samples in place.
    pub fn process(&mut self, samples: &mut [f32]) {
        if !self.enabled || (self.gains.iter().all(|g| g.abs() < 0.01) && self.preamp_db.abs() < 0.01) {
            return;
        }
        let pre = 10f32.powf(self.preamp_db / 20.0);
        let channels = self.state.len();
        for (i, s) in samples.iter_mut().enumerate() {
            let st = &mut self.state[i % channels];
            let mut x = *s * pre;
            for (f, b) in self.filters.iter().zip(st.iter_mut()) {
                let y = f.b0 * x + f.b1 * b.x1 + f.b2 * b.x2 - f.a1 * b.y1 - f.a2 * b.y2;
                b.x2 = b.x1;
                b.x1 = x;
                b.y2 = b.y1;
                b.y1 = y;
                x = y;
            }
            *s = x.clamp(-1.0, 1.0);
        }
    }

    /// The combined response in dB at `freq` (for drawing the curve).
    pub fn response_db(&self, freq: f32) -> f32 {
        let g: f32 = self.filters.iter().map(|f| f.response(freq, self.rate)).product();
        20.0 * g.max(1e-6).log10() + self.preamp_db
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_gain_is_identity() {
        assert_eq!(Biquad::peaking(1000.0, 0.0, 1.0, 44100.0), Biquad::IDENTITY);
        assert_eq!(Biquad::peaking(30000.0, 6.0, 1.0, 44100.0), Biquad::IDENTITY);
    }

    #[test]
    fn peaking_boosts_at_its_centre() {
        let f = Biquad::peaking(1000.0, 6.0, 1.41, 44100.0);
        let db = 20.0 * f.response(1000.0, 44100.0).log10();
        assert!((db - 6.0).abs() < 0.2, "{db}");
        // far from the centre the response returns near 0 dB
        let far = 20.0 * f.response(60.0, 44100.0).log10();
        assert!(far.abs() < 1.0, "{far}");
    }

    #[test]
    fn cut_lowers_the_level() {
        let f = Biquad::peaking(250.0, -9.0, 1.41, 48000.0);
        assert!(f.response(250.0, 48000.0) < 0.4);
    }

    #[test]
    fn presets_and_names() {
        let mut e = Equalizer::new(44100.0, 2);
        assert_eq!(e.preset_name(), Some("Flat"));
        assert!(e.load_preset("bass booster"));
        assert_eq!(e.preset_name(), Some("Bass Booster"));
        assert!(!e.load_preset("nope"));
        e.set_gain(0, 99.0);
        assert_eq!(e.gains[0], MAX_DB);
        assert_eq!(e.preset_name(), None);
    }

    #[test]
    fn flat_leaves_samples_untouched_and_boost_raises_a_tone() {
        let mut e = Equalizer::new(44100.0, 1);
        let tone: Vec<f32> = (0..4410).map(|i| 0.2 * (2.0 * PI * 1000.0 * i as f32 / 44100.0).sin()).collect();
        let mut flat = tone.clone();
        e.process(&mut flat);
        assert_eq!(flat, tone);
        e.set_gain(5, 9.0);
        let mut boosted = tone.clone();
        e.process(&mut boosted);
        let peak = |v: &[f32]| v[2000..].iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak(&boosted) > peak(&tone) * 2.0, "{} vs {}", peak(&boosted), peak(&tone));
        assert!(e.response_db(1000.0) > 8.0);
    }

    #[test]
    fn disabled_does_nothing() {
        let mut e = Equalizer::new(44100.0, 1);
        e.set_gain(3, 12.0);
        e.enabled = false;
        let mut s = vec![0.1, -0.1, 0.3];
        e.process(&mut s);
        assert_eq!(s, vec![0.1, -0.1, 0.3]);
    }
}
