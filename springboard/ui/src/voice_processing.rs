//! Voice processing for calls and recording: the chain a voice input runs its microphone through before an app reads
//! it. Echo cancellation removes what the speakers played (`echo_cancel`), a noise gate quietens the input between
//! words, and automatic gain control brings the voice to a steady level. The chain can be bypassed (the microphone
//! passes unchanged) or muted (silence), and each stage can be left out.
//!
//! This is a pure model over `f32` samples in -1.0..1.0: no device is opened, and the caller feeds the playback
//! reference (`far`) and the microphone (`near`) in time order. It does not do noise suppression that tells speech from
//! steady noise of the same loudness; the gate only compares the level with a threshold.

use crate::echo_cancel::{EchoCanceller, EchoError};

/// The largest automatic gain, in decibels.
pub const MAX_GAIN_DB: f32 = 40.0;
/// The quietest the gate makes a closed input, as a share of the input level (about -26 dB).
const GATE_FLOOR: f32 = 0.05;
/// The smallest automatic gain (about -26 dB): a very loud voice is brought down by at most this much.
const MIN_GAIN: f32 = 0.05;
/// Seconds the gate takes to open and to close, and the peak envelope's release.
const GATE_OPEN_S: f32 = 0.005;
const GATE_CLOSE_S: f32 = 0.05;
const ENVELOPE_RELEASE_S: f32 = 0.1;
/// Seconds over which the voice level is averaged, and over which the gain rises and falls.
const LEVEL_S: f32 = 0.2;
const GAIN_UP_S: f32 = 1.0;
const GAIN_DOWN_S: f32 = 0.05;

/// What the chain does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    pub sample_rate: u32,
    /// Echo canceller taps; 0 leaves echo cancellation out.
    pub echo_taps: usize,
    /// Echo canceller step size, in the open range (0, 2).
    pub echo_step: f32,
    pub noise_gate: bool,
    /// The input level, in dB relative to full scale, below which the gate closes. At most 0.
    pub gate_threshold_db: f32,
    pub agc: bool,
    /// The level automatic gain control aims for, in dB relative to full scale (RMS). At most 0.
    pub target_db: f32,
    /// The most gain automatic gain control may add, in decibels, from 0 to [`MAX_GAIN_DB`].
    pub max_gain_db: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            sample_rate: 16_000,
            echo_taps: 256,
            echo_step: 0.3,
            noise_gate: true,
            gate_threshold_db: -55.0,
            agc: true,
            target_db: -20.0,
            max_gain_db: 24.0,
        }
    }
}

/// Why a chain was refused or a block was not processed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The sample rate is zero.
    BadSampleRate,
    /// The echo canceller was refused (taps above the limit, or a step outside (0, 2)).
    BadEcho(EchoError),
    /// A level is not a finite number in its range.
    BadLevel,
    /// The playback reference is not empty and not as long as the microphone block.
    LengthMismatch { far: usize, near: usize },
}

/// The voice processing chain for one input.
#[derive(Clone, Debug)]
pub struct VoiceProcessor {
    cfg: Config,
    echo: Option<EchoCanceller>,
    bypass: bool,
    muted: bool,
    gate_threshold: f32,
    target: f32,
    max_gain: f32,
    /// peak envelope of the echo-free input, and its per-sample release
    envelope: f32,
    release: f32,
    gate_gain: f32,
    gate_open: f32,
    gate_close: f32,
    /// smoothed power of the gated input, and the gain applied to it
    level: f32,
    level_coef: f32,
    gain: f32,
    gain_up: f32,
    gain_down: f32,
}

/// The per-sample smoothing coefficient of a first-order filter with time constant `seconds`.
fn coef(seconds: f32, rate: u32) -> f32 {
    1.0 - (-1.0 / (seconds * rate as f32)).exp()
}

fn finite(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

impl VoiceProcessor {
    /// A chain for `config`. Refuses a zero sample rate, a bad echo canceller and levels out of range.
    pub fn new(config: Config) -> Result<Self, Error> {
        if config.sample_rate == 0 {
            return Err(Error::BadSampleRate);
        }
        let level_ok = |v: f32, lo: f32, hi: f32| v.is_finite() && v >= lo && v <= hi;
        if !level_ok(config.gate_threshold_db, -200.0, 0.0) || !level_ok(config.target_db, -200.0, 0.0) || !level_ok(config.max_gain_db, 0.0, MAX_GAIN_DB) {
            return Err(Error::BadLevel);
        }
        let echo = if config.echo_taps == 0 { None } else { Some(EchoCanceller::new(config.echo_taps, config.echo_step).map_err(Error::BadEcho)?) };
        let rate = config.sample_rate;
        let target = db_to_linear(config.target_db);
        Ok(VoiceProcessor {
            cfg: config,
            echo,
            bypass: false,
            muted: false,
            gate_threshold: db_to_linear(config.gate_threshold_db),
            target,
            max_gain: db_to_linear(config.max_gain_db),
            envelope: 0.0,
            release: (-1.0 / (ENVELOPE_RELEASE_S * rate as f32)).exp(),
            gate_gain: 1.0,
            gate_open: coef(GATE_OPEN_S, rate),
            gate_close: coef(GATE_CLOSE_S, rate),
            // start at the target so the first words are not boosted or cut before the level is known
            level: target * target,
            level_coef: coef(LEVEL_S, rate),
            gain: 1.0,
            gain_up: coef(GAIN_UP_S, rate),
            gain_down: coef(GAIN_DOWN_S, rate),
        })
    }

    pub fn config(&self) -> Config {
        self.cfg
    }

    /// While bypassed, the microphone passes unchanged and no stage learns anything.
    pub fn set_bypass(&mut self, bypass: bool) {
        self.bypass = bypass;
    }

    pub fn is_bypassed(&self) -> bool {
        self.bypass
    }

    /// While muted, the output is silence (muting wins over bypass).
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// The gain automatic gain control applies now, in decibels.
    pub fn gain_db(&self) -> f32 {
        20.0 * self.gain.log10()
    }

    /// Forgets the learnt echo path, the gate and the gain.
    pub fn reset(&mut self) {
        if let Some(e) = self.echo.as_mut() {
            e.reset();
        }
        self.envelope = 0.0;
        self.gate_gain = 1.0;
        self.level = self.target * self.target;
        self.gain = 1.0;
    }

    /// Processes one block. `far` is the playback reference, the same length as `near`, or empty when nothing plays.
    /// Returns the processed microphone block. A non-finite sample counts as silence.
    pub fn process_block(&mut self, far: &[f32], near: &[f32]) -> Result<Vec<f32>, Error> {
        if !far.is_empty() && far.len() != near.len() {
            return Err(Error::LengthMismatch { far: far.len(), near: near.len() });
        }
        let mut out = Vec::with_capacity(near.len());
        for (i, &n) in near.iter().enumerate() {
            let n = finite(n);
            if self.muted {
                out.push(0.0);
                continue;
            }
            if self.bypass {
                out.push(n);
                continue;
            }
            let reference = far.get(i).copied().unwrap_or(0.0);
            let mut x = match self.echo.as_mut() {
                Some(e) => e.process(reference, n),
                None => n,
            };
            let a = x.abs();
            self.envelope = if a > self.envelope { a } else { self.envelope * self.release };
            let speaking = self.envelope > self.gate_threshold;
            if self.cfg.noise_gate {
                let (target, rate) = if speaking { (1.0, self.gate_open) } else { (GATE_FLOOR, self.gate_close) };
                self.gate_gain += (target - self.gate_gain) * rate;
                x *= self.gate_gain;
            }
            if self.cfg.agc {
                // the voice level and the gain only move while someone is speaking, so silence is not amplified
                if speaking || !self.cfg.noise_gate {
                    self.level += (x * x - self.level) * self.level_coef;
                    let rms = self.level.sqrt().max(1e-5);
                    let want = (self.target / rms).clamp(MIN_GAIN, self.max_gain);
                    let rate = if want < self.gain { self.gain_down } else { self.gain_up };
                    self.gain += (want - self.gain) * rate;
                }
                x = (x * self.gain).clamp(-1.0, 1.0);
            }
            out.push(x);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

    fn sine(amplitude: f32, hz: f32, samples: usize) -> Vec<f32> {
        (0..samples).map(|i| amplitude * (2.0 * std::f32::consts::PI * hz * i as f32 / RATE as f32).sin()).collect()
    }

    /// Repeatable noise in -amplitude..amplitude.
    fn noise(amplitude: f32, samples: usize, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..samples)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                amplitude * ((state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0)
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn only(f: impl FnOnce(&mut Config)) -> Config {
        let mut c = Config { echo_taps: 0, noise_gate: false, agc: false, ..Config::default() };
        f(&mut c);
        c
    }

    #[test]
    fn a_chain_is_refused_for_a_bad_rate_echo_or_level() {
        assert_eq!(VoiceProcessor::new(Config { sample_rate: 0, ..Config::default() }).err(), Some(Error::BadSampleRate));
        assert_eq!(VoiceProcessor::new(Config { echo_step: 0.0, ..Config::default() }).err(), Some(Error::BadEcho(EchoError::BadStep)));
        assert_eq!(VoiceProcessor::new(Config { gate_threshold_db: 3.0, ..Config::default() }).err(), Some(Error::BadLevel));
        assert_eq!(VoiceProcessor::new(Config { target_db: f32::NAN, ..Config::default() }).err(), Some(Error::BadLevel));
        assert_eq!(VoiceProcessor::new(Config { max_gain_db: MAX_GAIN_DB + 1.0, ..Config::default() }).err(), Some(Error::BadLevel));
        assert!(VoiceProcessor::new(Config::default()).is_ok());
    }

    #[test]
    fn a_reference_of_another_length_is_refused_and_an_empty_one_means_nothing_plays() {
        let mut p = VoiceProcessor::new(Config::default()).unwrap();
        assert_eq!(p.process_block(&[0.0; 3], &[0.0; 4]), Err(Error::LengthMismatch { far: 3, near: 4 }));
        assert_eq!(p.process_block(&[], &[0.0; 4]).unwrap().len(), 4);
    }

    #[test]
    fn bypass_passes_the_microphone_and_mute_silences_it() {
        let mut p = VoiceProcessor::new(Config::default()).unwrap();
        let mic = sine(0.3, 300.0, 64);
        p.set_bypass(true);
        assert_eq!(p.process_block(&[], &mic).unwrap(), mic);
        p.set_muted(true);
        assert!(p.process_block(&[], &mic).unwrap().iter().all(|v| *v == 0.0), "muting wins over bypass");
        p.set_muted(false);
        p.set_bypass(false);
        assert_ne!(p.process_block(&[], &mic).unwrap(), mic, "the chain runs again");
        assert!(p.process_block(&[], &[f32::NAN, f32::INFINITY]).unwrap().iter().all(|v| v.is_finite()));
    }

    #[test]
    fn echo_of_the_playback_is_removed_from_the_microphone() {
        let mut p = VoiceProcessor::new(only(|c| {
            c.echo_taps = 16;
            c.echo_step = 0.5;
        }))
        .unwrap();
        let far = noise(0.5, 4000, 7);
        let near: Vec<f32> = far.iter().map(|v| 0.4 * v).collect();
        let out = p.process_block(&far, &near).unwrap();
        let (tail_out, tail_near) = (&out[3000..], &near[3000..]);
        assert!(rms(tail_out) < 0.1 * rms(tail_near), "residual {} of {}", rms(tail_out), rms(tail_near));
    }

    #[test]
    fn the_gate_quietens_the_input_between_words_and_lets_a_voice_through() {
        let mut p = VoiceProcessor::new(only(|c| c.noise_gate = true)).unwrap();
        let hiss = noise(0.001, 4000, 3);
        let quiet = p.process_block(&[], &hiss).unwrap();
        assert!(quiet[3000..].iter().all(|v| v.abs() < 0.0002), "hiss at -60 dB is cut to the floor");

        let mut p = VoiceProcessor::new(only(|c| c.noise_gate = true)).unwrap();
        let voice = sine(0.5, 200.0, 4000);
        let out = p.process_block(&[], &voice).unwrap();
        let worst = voice.iter().zip(&out).skip(400).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-3, "a voice well above the threshold passes, off by {worst}");
    }

    #[test]
    fn automatic_gain_brings_quiet_and_loud_voices_to_the_target() {
        let target = db_to_linear(-20.0);
        let mut quiet = VoiceProcessor::new(only(|c| c.agc = true)).unwrap();
        let out = quiet.process_block(&[], &sine(0.02, 200.0, 4 * RATE as usize)).unwrap();
        let last = rms(&out[3 * RATE as usize..]);
        assert!(last > target * 0.7 && last < target * 1.4, "quiet voice at {last}");
        assert!(quiet.gain_db() > 10.0);

        let mut loud = VoiceProcessor::new(only(|c| c.agc = true)).unwrap();
        let out = loud.process_block(&[], &sine(0.9, 200.0, 4 * RATE as usize)).unwrap();
        let last = rms(&out[3 * RATE as usize..]);
        assert!(last > target * 0.7 && last < target * 1.4, "loud voice at {last}");
        assert!(loud.gain_db() < -3.0);
    }

    #[test]
    fn the_gain_never_exceeds_the_limit_and_the_output_stays_in_range() {
        let mut p = VoiceProcessor::new(only(|c| {
            c.agc = true;
            c.max_gain_db = 12.0;
        }))
        .unwrap();
        let out = p.process_block(&[], &sine(0.002, 200.0, 8 * RATE as usize)).unwrap();
        assert!(p.gain_db() <= 12.01, "gain {}", p.gain_db());
        assert!(rms(&out[7 * RATE as usize..]) < 0.002 * 0.7071 * 4.0 * 1.05, "no more than 12 dB (x3.98) above the input");
        let mut hot = VoiceProcessor::new(only(|c| c.agc = true)).unwrap();
        assert!(hot.process_block(&[], &sine(1.0, 150.0, 2000)).unwrap().iter().all(|v| v.abs() <= 1.0));
    }

    #[test]
    fn silence_is_not_amplified_when_the_gate_is_on() {
        let mut p = VoiceProcessor::new(Config { echo_taps: 0, ..Config::default() }).unwrap();
        p.process_block(&[], &noise(0.0005, 4 * RATE as usize, 11)).unwrap();
        assert!(p.gain_db().abs() < 0.5, "the gain stayed at unity while only hiss arrived: {} dB", p.gain_db());
    }

    #[test]
    fn reset_forgets_the_learnt_gain_and_echo_path() {
        let mut p = VoiceProcessor::new(only(|c| c.agc = true)).unwrap();
        p.process_block(&[], &sine(0.02, 200.0, 2 * RATE as usize)).unwrap();
        assert!(p.gain_db() > 3.0);
        p.reset();
        assert!(p.gain_db().abs() < 1e-3);
    }
}
