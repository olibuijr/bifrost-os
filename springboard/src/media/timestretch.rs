//! Playback speed without a change of pitch: WSOLA (waveform-similarity overlap-add) time-stretch.
//!
//! The stretcher takes interleaved stereo S16 at the player's output rate and returns the same audio over
//! `input / rate` frames. Each output hop of [`HOP`] frames is a Hann-windowed copy of an input segment of
//! [`WINDOW`] frames. A segment starts near its nominal position (the previous start plus `HOP * rate`, searched
//! within [`SEEK`] frames) where it best continues the previous segment, so the copies line up in phase and the
//! pitch stays put. Resampling alone would change the pitch with the speed, which is why this module exists.
//!
//! Rules of this model:
//! - The rate is 0.5 to 2.0 ([`clamp_rate`]; NaN becomes 1.0). The caller bypasses the stretcher at exactly 1.0.
//! - The output lags the input by up to `WINDOW + SEEK` frames: the newest frames wait for their search range. The
//!   held-back tail is dropped by [`Stretcher::reset`] (a song change or a seek).
//! - The result does not depend on how the input is split into `push` calls.

/// Slowest playback rate the player accepts.
pub const RATE_MIN: f32 = 0.5;
/// Fastest playback rate the player accepts.
pub const RATE_MAX: f32 = 2.0;

/// Frames per analysis window (about 21 ms at 48 kHz).
const WINDOW: usize = 1024;
/// Output frames per hop: half a window, so periodic Hann windows at this hop sum to one.
const HOP: usize = WINDOW / 2;
/// How far from the nominal start a segment may move to match the previous one, in frames (about 5 ms at 48 kHz).
const SEEK: usize = 256;

/// Clamps a playback rate to 0.5 to 2.0. NaN becomes 1.0.
pub fn clamp_rate(rate: f32) -> f32 {
    if rate.is_nan() {
        1.0
    } else {
        rate.clamp(RATE_MIN, RATE_MAX)
    }
}

type Frame = [f32; 2];

/// The WSOLA state of one stream of stereo frames.
pub struct Stretcher {
    rate: f32,
    /// the window, periodic Hann, `WINDOW` long
    window: Vec<f32>,
    /// input frames still needed; `input[0]` is the absolute frame `base`
    input: Vec<Frame>,
    base: usize,
    /// overlap-add accumulator: `acc[0]` is the next output frame; `WINDOW` long
    acc: Vec<Frame>,
    /// absolute input frame where the last segment started (None before the first one)
    prev: Option<usize>,
    /// absolute input frame where the next segment should start before the search; it moves by `HOP * rate` per segment
    nominal: f64,
}

impl Stretcher {
    /// A stretcher at `rate` (clamped to the accepted range).
    pub fn new(rate: f32) -> Self {
        let window = (0..WINDOW).map(|n| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * n as f32 / WINDOW as f32).cos()).collect();
        Stretcher { rate: clamp_rate(rate), window, input: Vec::new(), base: 0, acc: vec![[0.0; 2]; WINDOW], prev: None, nominal: 0.0 }
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// Changes the rate from the next segment on. The buffered audio is kept, so the change is smooth.
    pub fn set_rate(&mut self, rate: f32) {
        self.rate = clamp_rate(rate);
    }

    /// Forgets all buffered audio and starts over (the next input is taken as the start of a new stream).
    pub fn reset(&mut self) {
        self.input.clear();
        self.base = 0;
        self.acc.iter_mut().for_each(|a| *a = [0.0; 2]);
        self.prev = None;
        self.nominal = 0.0;
    }

    /// Feeds interleaved stereo S16 and appends the stretched output (interleaved stereo S16) to `out`. Output may be
    /// empty for a call while the stretcher fills its first window.
    pub fn push(&mut self, input: &[i16], out: &mut Vec<i16>) {
        self.input.extend(input.chunks_exact(2).map(|f| [f[0] as f32, f[1] as f32]));
        while self.ready() {
            self.segment(out);
        }
    }

    /// The absolute frame just past the buffered input.
    fn end(&self) -> usize {
        self.base + self.input.len()
    }

    /// Whether the next segment and its whole search range are buffered.
    fn ready(&self) -> bool {
        self.end() >= self.nominal.round() as usize + SEEK + WINDOW
    }

    /// The absolute start of the next segment: the candidate that best continues the previous one.
    fn choose(&self) -> usize {
        let Some(prev) = self.prev else { return 0 };
        let target = self.nominal.round() as usize;
        // the natural continuation of the previous segment: the samples that its second half overlaps
        let template = prev + HOP;
        let lo = target.saturating_sub(SEEK).max(self.base);
        let hi = target + SEEK;
        let mut best = (f64::MIN, target.max(lo));
        for c in lo..=hi {
            let score = self.similarity(c, template);
            if score > best.0 {
                best = (score, c);
            }
        }
        best.1
    }

    /// Normalised cross-correlation of the mono mix of the `HOP` frames at `a` and at `b`.
    fn similarity(&self, a: usize, b: usize) -> f64 {
        let (xa, xb) = (&self.input[a - self.base..a - self.base + HOP], &self.input[b - self.base..b - self.base + HOP]);
        let (mut dot, mut ea, mut eb) = (0.0f64, 0.0f64, 0.0f64);
        for (p, q) in xa.iter().zip(xb) {
            let (x, y) = ((p[0] + p[1]) as f64, (q[0] + q[1]) as f64);
            dot += x * y;
            ea += x * x;
            eb += y * y;
        }
        dot / (ea * eb + 1e-9).sqrt()
    }

    /// Emits one hop: windows the chosen segment into the accumulator and moves the start on by `HOP * rate`.
    fn segment(&mut self, out: &mut Vec<i16>) {
        let c = self.choose();
        let at = c - self.base;
        for i in 0..WINDOW {
            let (s, w) = (self.input[at + i], self.window[i]);
            self.acc[i][0] += s[0] * w;
            self.acc[i][1] += s[1] * w;
        }
        for a in &self.acc[..HOP] {
            out.push(a[0].round().clamp(i16::MIN as f32, i16::MAX as f32) as i16);
            out.push(a[1].round().clamp(i16::MIN as f32, i16::MAX as f32) as i16);
        }
        self.acc.copy_within(HOP.., 0);
        self.acc[HOP..].iter_mut().for_each(|a| *a = [0.0; 2]);
        self.prev = Some(c);
        // the nominal start advances on the clock of the output, not from the chosen start: a search that keeps
        // picking its lowest candidate must not stall the stream
        self.nominal += HOP as f64 * self.rate as f64;
        // drop what no later search or template can read: the next template starts at `c + HOP`, the next search at
        // the nominal start less `SEEK`
        let keep = (c + HOP).min((self.nominal - SEEK as f64).max(0.0) as usize);
        let drop = keep.saturating_sub(self.base).min(self.input.len());
        self.input.drain(..drop);
        self.base += drop;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE_HZ: f64 = 48_000.0;

    /// A stereo sine at `hz` for `frames` frames (both channels equal, amplitude 8000).
    fn sine(hz: f64, frames: usize) -> Vec<i16> {
        (0..frames).flat_map(|n| {
            let v = (8000.0 * (2.0 * std::f64::consts::PI * hz * n as f64 / RATE_HZ).sin()).round() as i16;
            [v, v]
        }).collect()
    }

    /// Stretches `input` in one call and returns the output.
    fn stretch(rate: f32, input: &[i16]) -> Vec<i16> {
        let mut s = Stretcher::new(rate);
        let mut out = Vec::new();
        s.push(input, &mut out);
        out
    }

    /// The magnitude of the DFT of `x` (mono) at `hz`.
    fn magnitude(x: &[f64], hz: f64) -> f64 {
        let (mut re, mut im) = (0.0, 0.0);
        for (n, v) in x.iter().enumerate() {
            let phase = 2.0 * std::f64::consts::PI * hz * n as f64 / RATE_HZ;
            re += v * phase.cos();
            im -= v * phase.sin();
        }
        (re * re + im * im).sqrt()
    }

    /// The frequency in 900..1100 Hz (1 Hz steps) that carries the most energy in `x`.
    fn dominant_hz(x: &[f64]) -> f64 {
        (900..=1100).map(|hz| (hz as f64, magnitude(x, hz as f64))).max_by(|a, b| a.1.total_cmp(&b.1)).unwrap().0
    }

    #[test]
    fn the_rate_is_clamped_and_nan_is_normal_speed() {
        assert_eq!(clamp_rate(0.1), RATE_MIN);
        assert_eq!(clamp_rate(9.0), RATE_MAX);
        assert_eq!(clamp_rate(f32::NAN), 1.0);
        assert_eq!(clamp_rate(1.25), 1.25);
        assert_eq!(Stretcher::new(4.0).rate(), RATE_MAX);
    }

    #[test]
    fn output_length_is_input_length_over_the_rate() {
        // 1 s of audio: the output is within 5 % of input / rate (the held-back window is about 27 ms)
        let input = sine(440.0, 48_000);
        for rate in [0.5f32, 0.75, 1.0, 1.5, 2.0] {
            let out = stretch(rate, &input);
            let frames = (out.len() / 2) as f64;
            let expect = 48_000.0 / rate as f64;
            assert!((frames - expect).abs() / expect < 0.05, "rate {rate}: {frames} frames, expected about {expect}");
        }
    }

    #[test]
    fn pitch_is_kept_when_the_speed_changes() {
        // a 1 kHz tone stretched to 1.5x and to 0.75x must still be 1 kHz: resampling would give 1.5 kHz or 750 Hz
        for rate in [1.5f32, 0.75] {
            let out = stretch(rate, &sine(1000.0, 48_000));
            let mono: Vec<f64> = out.chunks_exact(2).map(|f| f[0] as f64).collect();
            // a quarter second from the middle, clear of the start-up and the held-back tail
            let mid = &mono[mono.len() / 4..mono.len() / 4 + 12_000];
            let hz = dominant_hz(mid);
            assert!((hz - 1000.0).abs() <= 5.0, "rate {rate}: dominant {hz} Hz");
        }
    }

    #[test]
    fn the_output_does_not_depend_on_how_the_input_is_split() {
        let input = sine(700.0, 30_000);
        let whole = stretch(1.3, &input);
        let mut s = Stretcher::new(1.3);
        let mut parts = Vec::new();
        for chunk in input.chunks(333 * 2) {
            s.push(chunk, &mut parts);
        }
        assert_eq!(parts, whole);
    }

    #[test]
    fn a_reset_forgets_the_held_back_audio() {
        let mut s = Stretcher::new(1.0);
        let mut out = Vec::new();
        s.push(&sine(300.0, 4_000), &mut out);
        s.reset();
        out.clear();
        // after a reset the first window is taken from the new input's start, so a short block gives no output yet
        s.push(&sine(300.0, 500), &mut out);
        assert!(out.is_empty(), "{} samples", out.len());
    }

    #[test]
    fn silence_stays_silent_and_the_rate_can_change_mid_stream() {
        let mut s = Stretcher::new(2.0);
        let mut out = Vec::new();
        s.push(&vec![0i16; 8_000], &mut out);
        assert!(out.iter().all(|&v| v == 0));
        s.set_rate(0.5);
        assert_eq!(s.rate(), 0.5);
        s.push(&vec![0i16; 8_000], &mut out);
        assert!(out.iter().all(|&v| v == 0));
    }
}
