//! Visualizer levels: how loud each band of the equalizer's ten bands is in a block of samples, and how a drawn bar
//! moves toward its level. The levels are a Goertzel filter per band, so they need no FFT. The audio service
//! publishes no spectrum yet, so no window draws these levels.

use super::equalizer::BANDS;
use std::f32::consts::PI;

/// The level of each band in `samples` (mono audio at `rate` Hz), from 0 to 1. A full-scale sine at a band's centre
/// frequency reads as its amplitude. A band at or above half the rate, or at zero, reads 0.
pub fn band_levels(samples: &[f32], rate: f32, bands: &[f32]) -> Vec<f32> {
    let n = samples.len();
    bands
        .iter()
        .map(|&freq| {
            if n == 0 || rate <= 0.0 || freq <= 0.0 || freq >= rate / 2.0 {
                return 0.0;
            }
            let coeff = 2.0 * (2.0 * PI * freq / rate).cos();
            let (mut s1, mut s2) = (0.0f32, 0.0f32);
            for &x in samples {
                let s0 = x + coeff * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            let power = (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0);
            (2.0 * power.sqrt() / n as f32).min(1.0)
        })
        .collect()
}

/// The levels of the equalizer's ten bands in `samples`.
pub fn bars(samples: &[f32], rate: f32) -> Vec<f32> {
    band_levels(samples, rate, &BANDS)
}

/// The height a bar is drawn at after it was at `shown` and the band now reads `level`. A bar rises quickly and falls
/// slowly, so a peak stays readable for a moment.
pub fn smooth(shown: f32, level: f32) -> f32 {
    const ATTACK: f32 = 0.6;
    const RELEASE: f32 = 0.15;
    let rate = if level > shown { ATTACK } else { RELEASE };
    shown + (level - shown) * rate
}
