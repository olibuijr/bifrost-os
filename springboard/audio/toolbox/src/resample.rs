//! The high-quality sample-rate converter for files and offline work (task 13): a windowed-sinc interpolator
//! (Kaiser window) with a 512-phase coefficient table, linearly interpolated between phases. The low-latency
//! adaptive resampler for drift compensation is `varispeed` (task 11), not this.
//!
//! The kernel is `h(t) = c * sinc(c * t) * kaiser(t / H)` for `|t| < H`, where `t` is in input frames, `H` the
//! half width and `c` the cutoff as a fraction of the lower of the two Nyquist rates. Its samples at unit spacing
//! sum to 1 (Poisson summation), so DC passes at unity gain. Output frame `k` sits at input position
//! `k * from / to`, computed from integers so it never drifts. The stream is treated as silent before its first and
//! after its last frame; [`SincResampler::finish`] flushes the tail, giving `ceil(frames * to / from)` frames.

use bifrost_audio_types::AudioError;

const PHASES: usize = 512;

/// The trade between cost and stopband rejection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    /// 16 taps, about 60 dB.
    Fast,
    /// 64 taps.
    Standard,
    /// 128 taps.
    High,
}

impl Quality {
    fn params(self) -> (usize, f64, f64) {
        // (half width in input frames, Kaiser beta, cutoff)
        match self {
            Quality::Fast => (8, 6.0, 0.90),
            Quality::Standard => (32, 9.5, 0.95),
            Quality::High => (64, 12.0, 0.97),
        }
    }
}

/// Zeroth-order modified Bessel function of the first kind (power series).
fn bessel_i0(x: f64) -> f64 {
    let q = x * x / 4.0;
    let (mut sum, mut term) = (1.0f64, 1.0f64);
    for k in 1..200 {
        term *= q / (k as f64 * k as f64);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        let p = std::f64::consts::PI * x;
        p.sin() / p
    }
}

/// A streaming windowed-sinc resampler for interleaved f32.
pub struct SincResampler {
    from: u64,
    to: u64,
    channels: usize,
    half: usize,
    table: Vec<f32>,
    /// Interleaved history and not yet consumed input, starting `dropped` frames into the stream (the stream has
    /// `half` silent frames in front of its first real frame).
    buf: Vec<f32>,
    dropped: usize,
    out_count: u64,
    total_in: u64,
    coefs: Vec<f32>,
}

impl SincResampler {
    pub fn new(from_rate: u32, to_rate: u32, channels: usize, quality: Quality) -> Result<SincResampler, AudioError> {
        if from_rate == 0 || to_rate == 0 || channels == 0 {
            return Err(AudioError::BadValue);
        }
        let (half, beta, cut) = quality.params();
        let c = cut * (to_rate as f64 / from_rate as f64).min(1.0);
        let n = 2 * half * PHASES + 2;
        let i0b = bessel_i0(beta);
        let table = (0..n)
            .map(|j| {
                let t = j as f64 / PHASES as f64 - half as f64;
                let r = t / half as f64;
                let w = if r.abs() >= 1.0 { 0.0 } else { bessel_i0(beta * (1.0 - r * r).sqrt()) / i0b };
                (c * sinc(c * t) * w) as f32
            })
            .collect();
        let mut r = SincResampler {
            from: from_rate as u64,
            to: to_rate as u64,
            channels,
            half,
            table,
            buf: Vec::new(),
            dropped: 0,
            out_count: 0,
            total_in: 0,
            coefs: vec![0.0; 2 * half],
        };
        r.reset();
        Ok(r)
    }

    /// Starts a new stream.
    pub fn reset(&mut self) {
        self.buf.clear();
        self.buf.resize(self.half * self.channels, 0.0);
        self.dropped = 0;
        self.out_count = 0;
        self.total_in = 0;
    }

    /// Input frames of delay between a sample going in and its contribution being complete.
    pub fn latency(&self) -> usize {
        self.half
    }

    /// Output frames a whole stream of `in_frames` frames produces: `ceil(in_frames * to / from)`.
    pub fn output_len(&self, in_frames: u64) -> u64 {
        (in_frames * self.to + self.from - 1) / self.from
    }

    /// Consumes interleaved input (whole frames) and appends the output frames that are now complete.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let ch = self.channels;
        let frames = input.len() / ch;
        self.buf.extend_from_slice(&input[..frames * ch]);
        self.total_in += frames as u64;
        self.run(out);
    }

    /// Flushes the tail with silence and starts a new stream.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        let pad = self.half * self.channels;
        self.buf.resize(self.buf.len() + pad, 0.0);
        self.run(out);
        self.reset();
    }

    fn run(&mut self, out: &mut Vec<f32>) {
        let (ch, h) = (self.channels, self.half);
        loop {
            // real input position of the next output, from integers
            let real_num = self.out_count * self.from;
            if real_num >= self.total_in * self.to {
                break;
            }
            let real = real_num as f64 / self.to as f64;
            let pos = real + h as f64 - self.dropped as f64;
            let i = pos.floor() as usize;
            if i + h >= self.buf.len() / ch {
                break;
            }
            let frac = pos - i as f64;
            for n in 0..2 * h {
                let k = n as f64 - (h as f64 - 1.0);
                let x = (k - frac + h as f64) * PHASES as f64;
                let j = x as usize;
                let f = (x - j as f64) as f32;
                self.coefs[n] = self.table[j] * (1.0 - f) + self.table[j + 1] * f;
            }
            let start = i + 1 - h;
            for c in 0..ch {
                let mut acc = 0.0f32;
                for n in 0..2 * h {
                    acc += self.buf[(start + n) * ch + c] * self.coefs[n];
                }
                out.push(acc);
            }
            self.out_count += 1;
        }
        // drop frames no later output needs
        let next = (self.out_count * self.from) as f64 / self.to as f64 + h as f64 - self.dropped as f64;
        let keep_from = (next.floor() as usize + 1).saturating_sub(h);
        let drop = keep_from.min(self.buf.len() / ch);
        self.buf.drain(..drop * ch);
        self.dropped += drop;
    }
}

/// Resamples a whole interleaved buffer.
pub fn resample(input: &[f32], from_rate: u32, to_rate: u32, channels: usize, quality: Quality) -> Result<Vec<f32>, AudioError> {
    let mut r = SincResampler::new(from_rate, to_rate, channels, quality)?;
    let mut out = Vec::with_capacity(input.len() * to_rate as usize / from_rate as usize + channels);
    r.process(input, &mut out);
    r.finish(&mut out);
    Ok(out)
}

/// Total harmonic distortion plus noise of a sine at `hz` in `y` (sampled at `rate`), in dB relative to the
/// sine's level: fits the best sine and cosine at `hz` by projection and measures the residual. Use a window that
/// holds a whole number of cycles.
pub fn thd_n_db(y: &[f32], rate: f64, hz: f64) -> f64 {
    let n = y.len() as f64;
    let w = 2.0 * std::f64::consts::PI * hz / rate;
    let (mut a, mut b) = (0.0f64, 0.0f64);
    for (i, &v) in y.iter().enumerate() {
        a += v as f64 * (w * i as f64).sin();
        b += v as f64 * (w * i as f64).cos();
    }
    a *= 2.0 / n;
    b *= 2.0 / n;
    let mut resid = 0.0f64;
    for (i, &v) in y.iter().enumerate() {
        let fit = a * (w * i as f64).sin() + b * (w * i as f64).cos();
        resid += (v as f64 - fit).powi(2);
    }
    let sig = (a * a + b * b) / 2.0;
    10.0 * (resid / n / sig).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: f64, hz: f64, n: usize, amp: f32) -> Vec<f32> {
        (0..n).map(|i| amp * (2.0 * std::f64::consts::PI * hz * i as f64 / rate).sin() as f32).collect()
    }

    #[test]
    fn rejects_zero_arguments() {
        assert!(SincResampler::new(0, 48000, 2, Quality::Fast).is_err());
        assert!(SincResampler::new(44100, 0, 2, Quality::Fast).is_err());
        assert!(SincResampler::new(44100, 48000, 0, Quality::Fast).is_err());
    }

    #[test]
    fn output_length_is_ceil_of_the_ratio() {
        let r = SincResampler::new(44100, 48000, 1, Quality::Fast).unwrap();
        // 441 * 48000 / 44100 = 480 exactly; 442 * 48000 / 44100 = 481.09, which rounds up to 482
        assert_eq!(r.output_len(441), 480);
        assert_eq!(r.output_len(442), 482);
        let out = resample(&vec![0.0; 4410], 44100, 48000, 1, Quality::Fast).unwrap();
        assert_eq!(out.len(), 4800);
        let down = resample(&vec![0.0; 4800], 48000, 44100, 2, Quality::Fast).unwrap();
        assert_eq!(down.len(), 4410 * 2);
    }

    #[test]
    fn dc_passes_at_unity_gain() {
        for (a, b) in [(44100, 48000), (48000, 44100), (48000, 96000)] {
            let out = resample(&vec![1.0; 2000], a, b, 1, Quality::Standard).unwrap();
            let mid = out.len() / 2;
            for v in &out[mid - 20..mid + 20] {
                assert!((v - 1.0).abs() < 2e-3, "{a}->{b}: {v}");
            }
        }
    }

    #[test]
    fn edges_fade_from_silence() {
        // the stream is silent before its first frame, so the first output is the kernel summed over t >= 0:
        // (1 + h(0)) / 2 with h(0) = cutoff = 0.95 * 44100 / 48000 = 0.873, i.e. about 0.94
        let out = resample(&vec![1.0; 500], 48000, 44100, 1, Quality::Standard).unwrap();
        assert!(out[0] > 0.88 && out[0] < 1.0, "{}", out[0]);
        // and the middle is flat
        assert!((out[200] - 1.0).abs() < 2e-3);
    }

    #[test]
    fn blocks_match_one_shot() {
        let input = sine(44100.0, 1000.0, 3000, 0.5);
        let whole = resample(&input, 44100, 48000, 1, Quality::Fast).unwrap();
        let mut r = SincResampler::new(44100, 48000, 1, Quality::Fast).unwrap();
        let mut out = Vec::new();
        for chunk in input.chunks(333) {
            r.process(chunk, &mut out);
        }
        r.finish(&mut out);
        assert_eq!(out.len(), whole.len());
        for (a, b) in out.iter().zip(&whole) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn channels_stay_independent() {
        let l = sine(44100.0, 500.0, 2000, 0.5);
        let mut inter = Vec::new();
        for &v in &l {
            inter.push(v);
            inter.push(0.0);
        }
        let out = resample(&inter, 44100, 48000, 2, Quality::Fast).unwrap();
        assert!(out.iter().skip(1).step_by(2).all(|&v| v == 0.0));
        assert!(out.iter().step_by(2).any(|&v| v.abs() > 0.4));
    }

    #[test]
    fn thd_n_44100_to_48000_is_below_minus_60_db() {
        let input = sine(44100.0, 1000.0, 8820, 0.5);
        let out = resample(&input, 44100, 48000, 1, Quality::Standard).unwrap();
        // 4800 frames at 48 kHz hold exactly 100 cycles of 1 kHz
        let db = thd_n_db(&out[240..240 + 4800], 48000.0, 1000.0);
        assert!(db < -60.0, "THD+N {db} dB");
    }

    #[test]
    fn thd_n_helper_measures_a_known_error() {
        // a sine plus a 0.01 amplitude 3 kHz tone: residual/signal = 0.01 -> -40 dB
        let mut y = sine(48000.0, 1000.0, 4800, 1.0);
        let h = sine(48000.0, 3000.0, 4800, 0.01);
        y.iter_mut().zip(&h).for_each(|(a, b)| *a += b);
        let db = thd_n_db(&y, 48000.0, 1000.0);
        assert!((db + 40.0).abs() < 0.1, "{db}");
    }
}
