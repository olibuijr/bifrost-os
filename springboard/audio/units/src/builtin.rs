//! Built-in units: `Gain` (linear gain with sample-accurate ramps), `Biquad` (low-pass, high-pass or peaking
//! filter) and `SineSynth` (MIDI instrument that exercises the MIDI path). All allocate in `initialize` only.

use crate::param::{ParamSpec, ParamUnit, ParameterTree};
use crate::unit::{check_config, AudioUnit, Block, RenderConfig, UnitError, UnitInfo, UnitKind};

// ---------------------------------------------------------------------------------------------------- Gain

pub const GAIN_GAIN: u32 = 0;

/// Multiplies every channel by a linear gain (0 to 4), ramped over 64 frames when it changes.
pub struct Gain {
    info: UnitInfo,
    tree: ParameterTree,
    curve: Vec<f32>,
    ready: bool,
}

impl Gain {
    pub fn new() -> Gain {
        let specs = vec![ParamSpec::new(GAIN_GAIN, "Gain", ParamUnit::Ratio, 0.0, 4.0, 1.0).expect("static spec").with_ramp(64)];
        Gain {
            info: UnitInfo { name: "Gain".into(), kind: UnitKind::Effect, accepts_midi: false, max_channels: 32 },
            tree: ParameterTree::new(specs).expect("static tree"),
            curve: Vec::new(),
            ready: false,
        }
    }
}

impl Default for Gain {
    fn default() -> Gain {
        Gain::new()
    }
}

impl AudioUnit for Gain {
    fn info(&self) -> &UnitInfo {
        &self.info
    }
    fn tree(&self) -> &ParameterTree {
        &self.tree
    }
    fn tree_mut(&mut self) -> &mut ParameterTree {
        &mut self.tree
    }
    fn initialize(&mut self, cfg: &RenderConfig) -> Result<(), UnitError> {
        check_config(&self.info, cfg)?;
        self.curve = vec![0.0; cfg.max_frames as usize];
        self.ready = true;
        Ok(())
    }
    fn uninitialize(&mut self) {
        self.curve = Vec::new();
        self.ready = false;
    }
    fn reset(&mut self) {}
    fn render(&mut self, block: &mut Block<'_, '_>) {
        let n = block.frames;
        self.tree.sync();
        if !self.ready || n > self.curve.len() {
            block.outputs.iter_mut().for_each(|o| o[..n].iter_mut().for_each(|s| *s = 0.0));
            return;
        }
        self.tree.fill(GAIN_GAIN, &mut self.curve[..n]);
        for (out, inp) in block.outputs.iter_mut().zip(block.inputs.iter()) {
            for ((o, i), g) in out[..n].iter_mut().zip(inp[..n].iter()).zip(self.curve[..n].iter()) {
                *o = *i * *g;
            }
        }
    }
}

// --------------------------------------------------------------------------------------------------- Biquad

pub const BQ_TYPE: u32 = 0;
pub const BQ_FREQ: u32 = 1;
pub const BQ_Q: u32 = 2;
pub const BQ_GAIN_DB: u32 = 3;

/// Biquad filter types (the value of the `Type` parameter).
pub const BQ_LOW_PASS: f32 = 0.0;
pub const BQ_HIGH_PASS: f32 = 1.0;
pub const BQ_PEAKING: f32 = 2.0;

/// Normalized coefficients (`a0` = 1): y = b0 x + b1 x1 + b2 x2 - a1 y1 - a2 y2.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiquadCoefs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

/// The standard audio-EQ-cookbook design for the three types.
pub fn design(kind: f32, rate: f64, freq: f64, q: f64, gain_db: f64) -> BiquadCoefs {
    let w0 = 2.0 * std::f64::consts::PI * (freq / rate).clamp(1e-5, 0.499);
    let (s, c) = w0.sin_cos();
    let alpha = s / (2.0 * q.max(0.01));
    let (b0, b1, b2, a0, a1, a2);
    if kind < 0.5 {
        b0 = (1.0 - c) / 2.0;
        b1 = 1.0 - c;
        b2 = (1.0 - c) / 2.0;
        a0 = 1.0 + alpha;
        a1 = -2.0 * c;
        a2 = 1.0 - alpha;
    } else if kind < 1.5 {
        b0 = (1.0 + c) / 2.0;
        b1 = -(1.0 + c);
        b2 = (1.0 + c) / 2.0;
        a0 = 1.0 + alpha;
        a1 = -2.0 * c;
        a2 = 1.0 - alpha;
    } else {
        let a = 10f64.powf(gain_db / 40.0);
        b0 = 1.0 + alpha * a;
        b1 = -2.0 * c;
        b2 = 1.0 - alpha * a;
        a0 = 1.0 + alpha / a;
        a1 = -2.0 * c;
        a2 = 1.0 - alpha / a;
    }
    BiquadCoefs { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
}

#[derive(Clone, Copy, Default)]
struct BqState {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

/// A filter whose coefficients follow the parameters once per block.
pub struct Biquad {
    info: UnitInfo,
    tree: ParameterTree,
    rate: f64,
    state: Vec<BqState>,
}

impl Biquad {
    pub fn new() -> Biquad {
        let specs = vec![
            ParamSpec::new(BQ_TYPE, "Type", ParamUnit::Indexed, 0.0, 2.0, BQ_PEAKING)
                .expect("static spec")
                .with_value_strings(&["Low Pass", "High Pass", "Peaking"]),
            ParamSpec::new(BQ_FREQ, "Frequency", ParamUnit::Hertz, 20.0, 20_000.0, 1_000.0).expect("static spec"),
            ParamSpec::new(BQ_Q, "Q", ParamUnit::Generic, 0.1, 18.0, 0.707).expect("static spec"),
            ParamSpec::new(BQ_GAIN_DB, "Gain", ParamUnit::Decibels, -24.0, 24.0, 0.0).expect("static spec"),
        ];
        Biquad {
            info: UnitInfo { name: "Biquad Filter".into(), kind: UnitKind::Effect, accepts_midi: false, max_channels: 32 },
            tree: ParameterTree::new(specs).expect("static tree"),
            rate: 48_000.0,
            state: Vec::new(),
        }
    }
}

impl Default for Biquad {
    fn default() -> Biquad {
        Biquad::new()
    }
}

impl AudioUnit for Biquad {
    fn info(&self) -> &UnitInfo {
        &self.info
    }
    fn tree(&self) -> &ParameterTree {
        &self.tree
    }
    fn tree_mut(&mut self) -> &mut ParameterTree {
        &mut self.tree
    }
    fn initialize(&mut self, cfg: &RenderConfig) -> Result<(), UnitError> {
        check_config(&self.info, cfg)?;
        self.rate = cfg.sample_rate;
        self.state = vec![BqState::default(); cfg.output_channels as usize];
        Ok(())
    }
    fn uninitialize(&mut self) {
        self.state = Vec::new();
    }
    fn reset(&mut self) {
        self.state.iter_mut().for_each(|s| *s = BqState::default());
    }
    /// A rough ring-out estimate: 50 ms.
    fn tail_frames(&self) -> u32 {
        (self.rate * 0.05) as u32
    }
    fn render(&mut self, block: &mut Block<'_, '_>) {
        let n = block.frames;
        self.tree.sync();
        let kind = self.tree.advance(BQ_TYPE, n);
        let f = self.tree.advance(BQ_FREQ, n) as f64;
        let q = self.tree.advance(BQ_Q, n) as f64;
        let g = self.tree.advance(BQ_GAIN_DB, n) as f64;
        let c = design(kind, self.rate, f, q, g);
        for ((out, inp), st) in block.outputs.iter_mut().zip(block.inputs.iter()).zip(self.state.iter_mut()) {
            for (o, i) in out[..n].iter_mut().zip(inp[..n].iter()) {
                let x = *i as f64;
                let y = c.b0 * x + c.b1 * st.x1 + c.b2 * st.x2 - c.a1 * st.y1 - c.a2 * st.y2;
                st.x2 = st.x1;
                st.x1 = x;
                st.y2 = st.y1;
                st.y1 = y;
                *o = y as f32;
            }
        }
    }
}

// ----------------------------------------------------------------------------------------------- SineSynth

pub const SYNTH_LEVEL: u32 = 0;
const VOICES: usize = 8;

#[derive(Clone, Copy, Default)]
struct Voice {
    note: u8,
    active: bool,
    phase: f64,
    step: f64,
    amp: f32,
}

/// An 8-voice sine instrument; velocity sets the amplitude (velocity / 127 times the Level parameter).
pub struct SineSynth {
    info: UnitInfo,
    tree: ParameterTree,
    rate: f64,
    voices: [Voice; VOICES],
}

impl SineSynth {
    pub fn new() -> SineSynth {
        let specs = vec![ParamSpec::new(SYNTH_LEVEL, "Level", ParamUnit::Ratio, 0.0, 1.0, 1.0).expect("static spec")];
        SineSynth {
            info: UnitInfo { name: "Sine Synth".into(), kind: UnitKind::Instrument, accepts_midi: true, max_channels: 2 },
            tree: ParameterTree::new(specs).expect("static tree"),
            rate: 48_000.0,
            voices: [Voice::default(); VOICES],
        }
    }

    fn handle_event(&mut self, e: &crate::midi::MidiEvent) {
        if e.is_note_on() {
            let slot = self.voices.iter().position(|v| !v.active).unwrap_or(0);
            let hz = crate::midi::note_to_hz(e.data1);
            self.voices[slot] = Voice { note: e.data1, active: true, phase: 0.0, step: hz / self.rate, amp: e.data2 as f32 / 127.0 };
        } else if e.is_note_off() {
            for v in self.voices.iter_mut().filter(|v| v.active && v.note == e.data1) {
                v.active = false;
            }
        } else if e.status & 0xF0 == 0xB0 && e.data1 == 123 {
            self.voices.iter_mut().for_each(|v| v.active = false);
        }
    }
}

impl Default for SineSynth {
    fn default() -> SineSynth {
        SineSynth::new()
    }
}

impl AudioUnit for SineSynth {
    fn info(&self) -> &UnitInfo {
        &self.info
    }
    fn tree(&self) -> &ParameterTree {
        &self.tree
    }
    fn tree_mut(&mut self) -> &mut ParameterTree {
        &mut self.tree
    }
    fn initialize(&mut self, cfg: &RenderConfig) -> Result<(), UnitError> {
        check_config(&self.info, cfg)?;
        self.rate = cfg.sample_rate;
        self.reset();
        Ok(())
    }
    fn reset(&mut self) {
        self.voices = [Voice::default(); VOICES];
    }
    fn render(&mut self, block: &mut Block<'_, '_>) {
        let n = block.frames;
        self.tree.sync();
        let level = self.tree.advance(SYNTH_LEVEL, n);
        let mut next = 0usize;
        for i in 0..n {
            while next < block.midi.len() && block.midi[next].offset as usize <= i {
                let e = block.midi[next];
                self.handle_event(&e);
                next += 1;
            }
            let mut s = 0.0f32;
            for v in self.voices.iter_mut().filter(|v| v.active) {
                s += (v.phase * std::f64::consts::TAU).sin() as f32 * v.amp;
                v.phase += v.step;
                if v.phase >= 1.0 {
                    v.phase -= 1.0;
                }
            }
            for out in block.outputs.iter_mut() {
                out[i] = s * level;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::UnitHost;
    use crate::midi::MidiEvent;

    fn host(unit: Box<dyn AudioUnit>, inp: u32, out: u32, frames: u32) -> UnitHost {
        let mut h = UnitHost::new(unit);
        h.initialize(RenderConfig { sample_rate: 48_000.0, max_frames: frames, input_channels: inp, output_channels: out }).unwrap();
        h
    }

    #[test]
    fn gain_scales_exactly() {
        let g = Gain::new();
        g.params().set_ramped(GAIN_GAIN, 0.5, 0).unwrap();
        let mut h = host(Box::new(g), 2, 2, 4);
        let l = [1.0f32, 2.0, -4.0, 0.5];
        let r = [0.0f32, 8.0, 1.0, -1.0];
        let (mut ol, mut or) = ([0.0f32; 4], [0.0f32; 4]);
        h.render(&[&l, &r], &mut [&mut ol, &mut or], 4, 0.0, &[]).unwrap();
        assert_eq!(ol, [0.5, 1.0, -2.0, 0.25]);
        assert_eq!(or, [0.0, 4.0, 0.5, -0.5]);
    }

    #[test]
    fn gain_ramp_is_sample_accurate() {
        let g = Gain::new();
        g.params().set_ramped(GAIN_GAIN, 0.0, 4).unwrap();
        let mut h = host(Box::new(g), 1, 1, 8);
        let i = [1.0f32; 6];
        let mut o = [0.0f32; 6];
        h.render(&[&i], &mut [&mut o], 6, 0.0, &[]).unwrap();
        // 1.0 falling by 0.25 per frame, then 0
        assert_eq!(o, [1.0, 0.75, 0.5, 0.25, 0.0, 0.0]);
    }

    #[test]
    fn gain_survives_block_splits_identically() {
        let run = |splits: &[usize]| {
            let g = Gain::new();
            g.params().set_ramped(GAIN_GAIN, 2.0, 8).unwrap();
            let mut h = host(Box::new(g), 1, 1, 16);
            let i = [1.0f32; 16];
            let mut out = vec![];
            let mut t = 0usize;
            for &s in splits {
                let mut o = [0.0f32; 16];
                h.render(&[&i[..s]], &mut [&mut o[..s]], s, t as f64, &[]).unwrap();
                out.extend_from_slice(&o[..s]);
                t += s;
            }
            out
        };
        assert_eq!(run(&[16]), run(&[3, 5, 8]));
        assert_eq!(run(&[16])[..3], [1.0, 1.125, 1.25]);
    }

    #[test]
    fn peaking_at_zero_db_is_transparent() {
        let mut h = host(Box::new(Biquad::new()), 1, 1, 8);
        let i = [1.0f32, -0.5, 0.25, 0.75, -1.0, 0.0, 0.5, 0.125];
        let mut o = [0.0f32; 8];
        h.render(&[&i], &mut [&mut o], 8, 0.0, &[]).unwrap();
        for (a, b) in i.iter().zip(o.iter()) {
            assert!((a - b).abs() < 1e-6, "{a} vs {b}");
        }
    }

    #[test]
    fn low_pass_passes_dc_and_high_pass_blocks_it() {
        let dc = vec![1.0f32; 2048];
        let run = |kind: f32| {
            let b = Biquad::new();
            b.params().set_ramped(BQ_TYPE, kind, 0).unwrap();
            let mut h = host(Box::new(b), 1, 1, 2048);
            let mut o = vec![0.0f32; 2048];
            h.render(&[&dc], &mut [&mut o], 2048, 0.0, &[]).unwrap();
            o[2047]
        };
        assert!((run(BQ_LOW_PASS) - 1.0).abs() < 1e-4);
        assert!(run(BQ_HIGH_PASS).abs() < 1e-4);
    }

    #[test]
    fn cookbook_low_pass_at_quarter_rate_has_known_coefficients() {
        // f0 = rate / 4: w0 = pi/2, cos = 0, sin = 1; Q = 1/sqrt(2) gives alpha = 1/(2Q) = 0.7071...
        let q = std::f64::consts::FRAC_1_SQRT_2;
        let c = design(BQ_LOW_PASS, 48_000.0, 12_000.0, q, 0.0);
        let alpha = 1.0 / (2.0 * q);
        let a0 = 1.0 + alpha;
        assert!((c.b0 - 0.5 / a0).abs() < 1e-12);
        assert!((c.b1 - 1.0 / a0).abs() < 1e-12);
        assert!((c.a1 - 0.0).abs() < 1e-12);
        assert!((c.a2 - (1.0 - alpha) / a0).abs() < 1e-12);
    }

    #[test]
    fn synth_is_silent_until_the_note_offset_then_sounds() {
        let mut h = host(Box::new(SineSynth::new()), 0, 1, 64);
        let mut o = [9.0f32; 32];
        let ev = [MidiEvent::note_on(10, 0, 69, 127)];
        h.render(&[], &mut [&mut o], 32, 0.0, &ev).unwrap();
        assert!(o[..10].iter().all(|s| *s == 0.0));
        assert_eq!(o[10], 0.0); // phase 0
        // 440 Hz at 48 kHz: a quarter period is 27.27 frames, so the first frames after the onset rise.
        assert!(o[11] > 0.0 && o[12] > o[11]);
    }

    #[test]
    fn synth_note_off_silences_and_events_past_the_block_are_dropped() {
        let mut h = host(Box::new(SineSynth::new()), 0, 1, 16);
        let mut o = [0.0f32; 16];
        let ev = [MidiEvent::note_on(0, 0, 69, 127), MidiEvent::note_off(8, 0, 69), MidiEvent::note_on(99, 0, 60, 100)];
        h.render(&[], &mut [&mut o], 16, 0.0, &ev).unwrap();
        assert!(o[1] > 0.0);
        assert!(o[8..].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn synth_level_scales_by_velocity() {
        let mut h = host(Box::new(SineSynth::new()), 0, 1, 64);
        let mut o = [0.0f32; 64];
        h.render(&[], &mut [&mut o], 64, 0.0, &[MidiEvent::note_on(0, 0, 69, 64)]).unwrap();
        let peak = o.iter().cloned().fold(0.0f32, f32::max);
        assert!(peak <= 64.0 / 127.0 + 1e-6 && peak > 0.4);
    }

    #[test]
    fn sample_time_jump_resets_state() {
        let mut h = host(Box::new(SineSynth::new()), 0, 1, 16);
        let mut o = [0.0f32; 16];
        h.render(&[], &mut [&mut o], 16, 0.0, &[MidiEvent::note_on(0, 0, 69, 127)]).unwrap();
        h.render(&[], &mut [&mut o], 16, 16.0, &[]).unwrap();
        assert!(o.iter().any(|s| *s != 0.0));
        h.render(&[], &mut [&mut o], 16, 1000.0, &[]).unwrap(); // discontinuity: voices cleared
        assert!(o.iter().all(|s| *s == 0.0));
    }
}
