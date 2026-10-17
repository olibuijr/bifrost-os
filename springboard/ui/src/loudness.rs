//! Loudness of a mono buffer in LUFS: ITU-R BS.1770 K-weighting, with the gating of BS.1770 for the integrated value and
//! the EBU Tech 3341 windows for the maxima. It backs the `Loudness` analysis of `music::MusicUnderstandingSession`.
//! Pure Rust, no I/O. The DAW lab keeps its own stereo copy of this maths; this module is the mono one for apps.

use std::f64::consts::PI;

/// The loudness analysis result, in LUFS. Negative infinity means silence (or a buffer too short for the window).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoudnessResult {
    /// Integrated loudness: 400 ms blocks every 100 ms, an absolute gate at -70 LUFS and a relative gate 10 LU below
    /// the mean of the blocks that pass the absolute gate.
    pub integrated_lufs: f64,
    /// The loudest 400 ms window, stepped every 100 ms. Ungated.
    pub momentary_max_lufs: f64,
    /// The loudest 3 s window, stepped every 100 ms. Ungated.
    pub short_term_max_lufs: f64,
}

/// One biquad section with normalised coefficients (a0 = 1): `b` feeds the input, `a` the output (a1, a2).
#[derive(Clone, Copy, Debug)]
struct Section {
    b: [f64; 3],
    a: [f64; 2],
}

impl Section {
    /// RBJ high shelf with slope `q` and `gain_db` above the corner frequency.
    fn high_shelf(sample_rate: f64, corner: f64, q: f64, gain_db: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * corner / sample_rate;
        let (c, alpha) = (w0.cos(), w0.sin() / (2.0 * q));
        let root = 2.0 * a.sqrt() * alpha;
        let a0 = (a + 1.0) - (a - 1.0) * c + root;
        Self {
            b: [
                a * ((a + 1.0) + (a - 1.0) * c + root) / a0,
                -2.0 * a * ((a - 1.0) + (a + 1.0) * c) / a0,
                a * ((a + 1.0) + (a - 1.0) * c - root) / a0,
            ],
            a: [2.0 * ((a - 1.0) - (a + 1.0) * c) / a0, ((a + 1.0) - (a - 1.0) * c - root) / a0],
        }
    }

    /// RBJ high pass at `corner` with slope `q`.
    fn high_pass(sample_rate: f64, corner: f64, q: f64) -> Self {
        let w0 = 2.0 * PI * corner / sample_rate;
        let (c, alpha) = (w0.cos(), w0.sin() / (2.0 * q));
        let a0 = 1.0 + alpha;
        Self { b: [(1.0 + c) / 2.0 / a0, -(1.0 + c) / a0, (1.0 + c) / 2.0 / a0], a: [-2.0 * c / a0, (1.0 - alpha) / a0] }
    }

    fn run(&self, x: &[f64]) -> Vec<f64> {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        x.iter()
            .map(|&v| {
                let y = self.b[0] * v + self.b[1] * x1 + self.b[2] * x2 - self.a[0] * y1 - self.a[1] * y2;
                x2 = x1;
                x1 = v;
                y2 = y1;
                y1 = y;
                y
            })
            .collect()
    }
}

/// K-weighting (BS.1770): the high shelf of +4 dB at 1681.97 Hz, then the RLB high pass at 38.13 Hz.
fn k_weight(samples: &[f32], sample_rate: f64) -> Vec<f64> {
    let x: Vec<f64> = samples.iter().map(|&s| s as f64).collect();
    let shelf = Section::high_shelf(sample_rate, 1681.974450955533, 0.7071752369554196, 3.999843853973347);
    let rlb = Section::high_pass(sample_rate, 38.13547087602444, 0.5003270373238773);
    rlb.run(&shelf.run(&x))
}

/// Loudness in LUFS of one mean-square power (mono: channel weight 1).
fn to_lufs(mean_square: f64) -> f64 {
    -0.691 + 10.0 * mean_square.log10()
}

/// Mean-square power of each block of `block_secs`, a block starting every `hop_secs`. Empty when the buffer is shorter
/// than one block. Prefix sums keep long buffers linear.
fn block_powers(weighted: &[f64], sample_rate: f64, block_secs: f64, hop_secs: f64) -> Vec<f64> {
    let block = (block_secs * sample_rate).round() as usize;
    let hop = ((hop_secs * sample_rate).round() as usize).max(1);
    if block == 0 || weighted.len() < block {
        return Vec::new();
    }
    let mut prefix = Vec::with_capacity(weighted.len() + 1);
    prefix.push(0.0f64);
    for &y in weighted {
        prefix.push(prefix[prefix.len() - 1] + y * y);
    }
    (0..=weighted.len() - block).step_by(hop).map(|start| (prefix[start + block] - prefix[start]) / block as f64).collect()
}

/// The loudest window of a sliding measurement, in LUFS. Negative infinity when no window fits or all are silent.
fn loudest(powers: &[f64]) -> f64 {
    powers.iter().copied().map(to_lufs).fold(f64::NEG_INFINITY, f64::max)
}

/// Measures `samples` (mono, -1.0..=1.0). `None` only when there are no samples or the rate is zero.
pub fn measure(samples: &[f32], sample_rate: u32) -> Option<LoudnessResult> {
    if samples.is_empty() || sample_rate == 0 {
        return None;
    }
    let sr = sample_rate as f64;
    let weighted = k_weight(samples, sr);
    let momentary = block_powers(&weighted, sr, 0.4, 0.1);
    let gated: Vec<f64> = momentary.iter().copied().filter(|&m| to_lufs(m) > -70.0).collect();
    let integrated_lufs = if gated.is_empty() {
        f64::NEG_INFINITY
    } else {
        let mean = gated.iter().sum::<f64>() / gated.len() as f64;
        let relative = to_lufs(mean) - 10.0;
        let kept: Vec<f64> = gated.into_iter().filter(|&m| to_lufs(m) > relative).collect();
        if kept.is_empty() {
            f64::NEG_INFINITY
        } else {
            to_lufs(kept.iter().sum::<f64>() / kept.len() as f64)
        }
    };
    Some(LoudnessResult {
        integrated_lufs,
        momentary_max_lufs: loudest(&momentary),
        short_term_max_lufs: loudest(&block_powers(&weighted, sr, 3.0, 0.1)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sine of `freq` Hz at `amplitude`, mono.
    fn tone(freq: f64, amplitude: f64, sample_rate: u32, seconds: f64) -> Vec<f32> {
        let n = (seconds * sample_rate as f64) as usize;
        (0..n).map(|i| (amplitude * (2.0 * PI * freq * i as f64 / sample_rate as f64).sin()) as f32).collect()
    }

    #[test]
    fn silence_has_no_loudness() {
        let r = measure(&vec![0.0f32; 48000 * 5], 48000).unwrap();
        assert_eq!(r.integrated_lufs, f64::NEG_INFINITY);
        assert_eq!(r.momentary_max_lufs, f64::NEG_INFINITY);
        assert_eq!(r.short_term_max_lufs, f64::NEG_INFINITY);
    }

    #[test]
    fn doubling_the_amplitude_adds_about_six_decibels() {
        let quiet = measure(&tone(1000.0, 0.25, 48000, 5.0), 48000).unwrap();
        let loud = measure(&tone(1000.0, 0.5, 48000, 5.0), 48000).unwrap();
        assert!((loud.integrated_lufs - quiet.integrated_lufs - 6.0206).abs() < 0.01);
        assert!(loud.integrated_lufs.is_finite() && loud.integrated_lufs < 0.0);
    }

    #[test]
    fn a_steady_tone_reads_the_same_integrated_and_momentary() {
        let r = measure(&tone(1000.0, 0.5, 48000, 5.0), 48000).unwrap();
        assert!((r.integrated_lufs - r.momentary_max_lufs).abs() < 0.1, "{r:?}");
        assert!(r.short_term_max_lufs.is_finite());
    }

    #[test]
    fn a_buffer_shorter_than_one_block_has_no_windows() {
        let r = measure(&tone(1000.0, 0.5, 48000, 0.2), 48000).unwrap();
        assert_eq!(r.integrated_lufs, f64::NEG_INFINITY);
        assert_eq!(r.momentary_max_lufs, f64::NEG_INFINITY);
        assert_eq!(r.short_term_max_lufs, f64::NEG_INFINITY);
    }

    #[test]
    fn empty_input_or_zero_rate_is_refused() {
        assert_eq!(measure(&[], 48000), None);
        assert_eq!(measure(&[0.5], 0), None);
    }
}
