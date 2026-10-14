//! Listening lab: the port of `tools/lab/audio.mjs` metrics. Integrated loudness (ITU-R BS.1770 K-weighting with
//! absolute and relative gating), true peak (ITU-R BS.1770-4 style: 4x oversampled with a 48-tap-per-phase
//! Blackman-windowed sinc polyphase interpolator, max over both channels), clipped samples, crest factor, and
//! energy per band (sub < 120 Hz, bass 120-500 Hz, low-mid 500-2 kHz, high > 2 kHz).
use crate::dsp::{Biquad, FilterKind};
use std::f64::consts::PI;
use std::sync::OnceLock;

/// Oversampling factor of the true-peak meter.
const TP_PHASES: usize = 4;
/// Taps per polyphase branch (the interpolator spans TP_TAPS input samples).
const TP_TAPS: usize = 48;
/// Half-span in input samples; the interpolator reads from `-(TP_HALF - 1)` to `TP_HALF`.
const TP_HALF: usize = TP_TAPS / 2;
/// Interpolation branches for the fractional positions 1/4, 2/4, 3/4 between two input samples.
type TpBank = [[f32; TP_TAPS]; TP_PHASES - 1];

/// Polyphase windowed-sinc bank, built once on first use. Each branch is normalised to unity DC gain.
fn true_peak_bank() -> &'static TpBank {
    static BANK: OnceLock<TpBank> = OnceLock::new();
    BANK.get_or_init(|| {
        let mut bank: TpBank = [[0.0; TP_TAPS]; TP_PHASES - 1];
        let half = TP_HALF as f64;
        for (p, phase) in bank.iter_mut().enumerate() {
            let frac = (p + 1) as f64 / TP_PHASES as f64;
            let mut raw = [0.0f64; TP_TAPS];
            for (k, r) in raw.iter_mut().enumerate() {
                // tap k reads input sample i + m with m = k - (TP_HALF - 1); distance to the fractional point:
                let d = (k as f64 - (half - 1.0)) - frac;
                let sinc = if d.abs() < 1e-12 { 1.0 } else { (PI * d).sin() / (PI * d) };
                let x = d / half; // in (-1, 1) across the window
                let blackman = 0.42 + 0.5 * (PI * x).cos() + 0.08 * (2.0 * PI * x).cos();
                *r = sinc * blackman;
            }
            let sum: f64 = raw.iter().sum();
            for (t, r) in phase.iter_mut().zip(raw) {
                *t = (r / sum) as f32;
            }
        }
        bank
    })
}

/// Maximum absolute value of the 4x-interpolated signal, including the input samples themselves.
fn oversampled_peak(x: &[f32]) -> f32 {
    let bank = true_peak_bank();
    let n = x.len();
    // zero-pad so every window is contiguous: padded[i + k] is input sample i + k - (TP_HALF - 1)
    let mut padded = vec![0.0f32; n + TP_TAPS - 1];
    padded[TP_HALF - 1..TP_HALF - 1 + n].copy_from_slice(x);
    let mut peak = 0.0f32;
    for i in 0..n {
        let win = &padded[i..i + TP_TAPS];
        for taps in bank.iter() {
            let s: f32 = win.iter().zip(taps.iter()).map(|(a, b)| a * b).sum();
            peak = peak.max(s.abs());
        }
    }
    peak.max(x.iter().fold(0.0f32, |m, v| m.max(v.abs())))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Metrics {
    pub seconds: f64,
    pub lufs: f64,
    /// Loudest 400 ms window, LUFS.
    pub momentary_max_lufs: f64,
    /// Loudest 3 s window, LUFS.
    pub short_term_max_lufs: f64,
    pub true_peak_db: f64,
    pub clipped: usize,
    pub crest_db: f64,
    /// energy share, percent: sub, bass, low-mid, high
    pub bands: [f64; 4],
}

/// K-weighting of one channel: the pre-filter (high shelf +4 dB at 1681 Hz) then the RLB high-pass at 38 Hz, as in BS.1770.
fn kweight(x: &[f32], sr: f32) -> Vec<f32> {
    let mut shelf = Biquad::new(FilterKind::Highshelf, sr, 1681.97, 0.7071752, 3.999844);
    let mut hp = Biquad::new(FilterKind::Highpass, sr, 38.13, 0.5003, 0.0);
    x.iter().map(|&s| hp.process(shelf.process(s))).collect()
}

/// Loudness of a stereo signal in LUFS for one mean-square power.
fn to_lufs(ms: f64) -> f64 {
    -0.691 + 10.0 * ms.log10()
}

/// Mean-square power of each K-weighted block of `block_secs`, a block starting every `hop_secs`, summed over both
/// channels. Empty when the signal is shorter than one block.
fn block_powers(left: &[f32], right: &[f32], sr: f32, block_secs: f64, hop_secs: f64) -> Vec<f64> {
    let n = left.len().min(right.len());
    let kl = kweight(&left[..n], sr);
    let kr = kweight(&right[..n], sr);
    let block = (block_secs * sr as f64) as usize;
    let hop = ((hop_secs * sr as f64) as usize).max(1);
    if block == 0 || n < block {
        return vec![];
    }
    (0..=n - block)
        .step_by(hop)
        .map(|start| {
            let sum: f64 = (start..start + block).map(|i| (kl[i] as f64).powi(2) + (kr[i] as f64).powi(2)).sum();
            sum / block as f64
        })
        .collect()
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

/// Integrated loudness of a stereo signal (LUFS): 400 ms blocks every 100 ms, an absolute gate at -70 LUFS and a
/// relative gate 10 LU below the mean of the blocks that pass the absolute gate.
pub fn integrated_lufs(left: &[f32], right: &[f32], sr: f32) -> f64 {
    let gated: Vec<f64> = block_powers(left, right, sr, 0.4, 0.1).into_iter().filter(|&m| to_lufs(m) > -70.0).collect();
    if gated.is_empty() {
        return f64::NEG_INFINITY;
    }
    let rel = to_lufs(mean(&gated)) - 10.0;
    let kept: Vec<f64> = gated.into_iter().filter(|&m| to_lufs(m) > rel).collect();
    if kept.is_empty() {
        return f64::NEG_INFINITY;
    }
    to_lufs(mean(&kept))
}

/// The loudest window of a sliding measurement, in LUFS. Negative infinity when no window fits or all are silent.
fn loudest(powers: &[f64]) -> f64 {
    powers.iter().copied().map(to_lufs).fold(f64::NEG_INFINITY, f64::max)
}

/// Maximum momentary loudness (LUFS): the loudest 400 ms window, stepped every 100 ms. Ungated, as EBU Tech 3341 defines it.
pub fn max_momentary_lufs(left: &[f32], right: &[f32], sr: f32) -> f64 {
    loudest(&block_powers(left, right, sr, 0.4, 0.1))
}

/// Maximum short-term loudness (LUFS): the loudest 3 s window, stepped every 100 ms. Ungated, as EBU Tech 3341 defines it.
pub fn max_short_term_lufs(left: &[f32], right: &[f32], sr: f32) -> f64 {
    loudest(&block_powers(left, right, sr, 3.0, 0.1))
}

/// Full metrics of a stereo render.
pub fn measure(left: &[f32], right: &[f32], sr: u32) -> Metrics {
    let srf = sr as f32;
    let n = left.len().min(right.len());
    let mut peak = 0.0f32;
    let mut clipped = 0usize;
    let mut sum_sq = 0.0f64;
    for i in 0..n {
        for x in [left[i], right[i]] {
            peak = peak.max(x.abs());
            if x.abs() >= 0.999 {
                clipped += 1;
            }
            sum_sq += (x as f64) * (x as f64);
        }
    }
    let tp = oversampled_peak(&left[..n]).max(oversampled_peak(&right[..n]));
    let rms = (sum_sq / (2.0 * n.max(1) as f64)).sqrt();
    let crest_db = 20.0 * (peak as f64 / rms.max(1e-12)).log10();

    // band split on the mono sum
    let mono: Vec<f32> = (0..n).map(|i| (left[i] + right[i]) * 0.5).collect();
    let mut lp120 = Biquad::new(FilterKind::Lowpass, srf, 120.0, 0.7071, 0.0);
    let mut lp500 = Biquad::new(FilterKind::Lowpass, srf, 500.0, 0.7071, 0.0);
    let mut lp2k = Biquad::new(FilterKind::Lowpass, srf, 2000.0, 0.7071, 0.0);
    let (mut e_sub, mut e_bass, mut e_low, mut e_high) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for &x in &mono {
        let a = lp120.process(x) as f64;
        let b = lp500.process(x) as f64;
        let c = lp2k.process(x) as f64;
        e_sub += a * a;
        e_bass += (b - a) * (b - a);
        e_low += (c - b) * (c - b);
        e_high += (x as f64 - c) * (x as f64 - c);
    }
    let total = (e_sub + e_bass + e_low + e_high).max(1e-18);
    let bands = [e_sub / total * 100.0, e_bass / total * 100.0, e_low / total * 100.0, e_high / total * 100.0];

    Metrics {
        seconds: n as f64 / sr as f64,
        lufs: integrated_lufs(left, right, srf),
        momentary_max_lufs: max_momentary_lufs(left, right, srf),
        short_term_max_lufs: max_short_term_lufs(left, right, srf),
        true_peak_db: 20.0 * (tp.max(peak) as f64).max(1e-12).log10(),
        clipped,
        crest_db,
        bands,
    }
}

impl Metrics {
    /// The listening targets the mobile AI composer enforces (`listen.js`): -16..-10 LUFS, true peak <= -1 dBTP,
    /// no clipping, sub + bass <= 65 %. Returns the problems found.
    pub fn check_targets(&self) -> Vec<String> {
        let mut out = vec![];
        if !(-16.0..=-10.0).contains(&self.lufs) {
            out.push(format!("loudness {:.1} LUFS outside -16..-10", self.lufs));
        }
        if self.true_peak_db > -1.0 {
            out.push(format!("true peak {:.1} dBTP above -1", self.true_peak_db));
        }
        if self.clipped > 0 {
            out.push(format!("{} clipped samples", self.clipped));
        }
        if self.bands[0] + self.bands[1] > 65.0 {
            out.push(format!("sub+bass {:.0}% above 65", self.bands[0] + self.bands[1]));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_minus_infinity_and_a_tone_has_a_sane_level() {
        let z = vec![0.0f32; 48_000];
        assert!(integrated_lufs(&z, &z, 48_000.0).is_infinite());
        let tone: Vec<f32> = (0..48_000).map(|i| 0.5 * (std::f32::consts::TAU * 1000.0 * i as f32 / 48_000.0).sin()).collect();
        let m = measure(&tone, &tone, 48_000);
        assert!(m.lufs > -20.0 && m.lufs < -5.0, "{}", m.lufs);
        assert!((m.true_peak_db - (-6.02)).abs() < 0.2, "{}", m.true_peak_db);
    }

    #[test]
    fn true_peak_finds_inter_sample_peak() {
        // fs/4 sine at 45 degrees: samples are +-0.707 but the waveform peaks at 1.0 between them
        let x: Vec<f32> = (0..4800).map(|i| (std::f32::consts::FRAC_PI_2 * i as f32 + std::f32::consts::FRAC_PI_4).sin()).collect();
        let m = measure(&x, &x, 48_000);
        assert!(m.true_peak_db.abs() < 0.2, "{}", m.true_peak_db);
        assert!(m.true_peak_db > 20.0 * 0.7071f64.log10() + 1.0, "{}", m.true_peak_db);
    }
}
