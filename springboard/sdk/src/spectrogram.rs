//! Spectrogram for apps: the short-time Fourier magnitude of a signal, frame by frame.
//!
//! The reference vector library has no spectrogram of its own; an app builds one from windowed FFT frames. This module
//! does that with the SDK FFT ([`crate::fft`]): each frame of `size` samples is multiplied by a periodic Hann window,
//! transformed, and its magnitudes for the bins `0..=size/2` are kept. Frames start every `hop` samples and only where
//! a whole frame fits, so a signal shorter than `size` gives no frames. `size` must be a power of two of at least 2.
//!
//! Pure computation: no device, no clock, no output. The magnitudes are linear (not decibels); an app converts them
//! with [`magnitude_to_db`] when it draws them.

use crate::fft::{fft, Complex};

/// Why a spectrogram cannot be computed from the arguments given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpectrogramError {
    /// The frame size is not a power of two, or is smaller than 2.
    BadFrameSize,
    /// The hop between frames is zero.
    ZeroHop,
}

/// The periodic Hann window of `size` points: `0.5 - 0.5 cos(2 pi n / size)`. Empty for a size of zero.
pub fn hann(size: usize) -> Vec<f64> {
    (0..size)
        .map(|n| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / size as f64).cos())
        .collect()
}

/// The magnitude spectrum of each windowed frame. Every inner vector has `size / 2 + 1` bins, from DC up to the Nyquist
/// bin, and there is one inner vector per frame.
pub fn spectrogram(samples: &[f64], size: usize, hop: usize) -> Result<Vec<Vec<f64>>, SpectrogramError> {
    if size < 2 || !size.is_power_of_two() {
        return Err(SpectrogramError::BadFrameSize);
    }
    if hop == 0 {
        return Err(SpectrogramError::ZeroHop);
    }
    let window = hann(size);
    let mut frames = Vec::new();
    let mut start = 0;
    while start + size <= samples.len() {
        let mut buffer: Vec<Complex> = samples[start..start + size]
            .iter()
            .zip(window.iter())
            .map(|(x, w)| Complex::real(x * w))
            .collect();
        // The length is a power of two by the check above, so the transform cannot refuse it.
        fft(&mut buffer).map_err(|_| SpectrogramError::BadFrameSize)?;
        frames.push(buffer[..=size / 2].iter().map(|c| c.magnitude()).collect());
        start += hop;
    }
    Ok(frames)
}

/// `20 log10(magnitude)`, clamped at `floor_db` so a silent bin does not become negative infinity.
pub fn magnitude_to_db(magnitude: f64, floor_db: f64) -> f64 {
    if magnitude <= 0.0 {
        return floor_db;
    }
    (20.0 * magnitude.log10()).max(floor_db)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(bin: usize, size: usize, len: usize) -> Vec<f64> {
        (0..len)
            .map(|n| (2.0 * std::f64::consts::PI * bin as f64 * n as f64 / size as f64).cos())
            .collect()
    }

    #[test]
    fn the_hann_window_is_zero_at_the_start_and_one_at_the_centre() {
        let w = hann(8);
        assert!(w[0].abs() < 1e-12);
        assert!((w[4] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_tone_peaks_in_its_bin_in_every_frame() {
        let size = 16;
        let frames = spectrogram(&tone(4, size, 64), size, 8).unwrap();
        assert!(!frames.is_empty());
        for frame in &frames {
            assert_eq!(frame.len(), size / 2 + 1);
            let peak = frame
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .map(|(i, _)| i)
                .unwrap();
            assert_eq!(peak, 4);
        }
    }

    #[test]
    fn frames_advance_by_the_hop_and_only_whole_frames_count() {
        let size = 16;
        // Starts at 0, 8, 16 and 24: (40 - 16) / 8 + 1 frames.
        assert_eq!(spectrogram(&vec![0.0; 40], size, 8).unwrap().len(), 4);
        // A signal shorter than one frame gives no frames.
        assert!(spectrogram(&vec![0.0; 10], size, 8).unwrap().is_empty());
    }

    #[test]
    fn silence_has_zero_magnitude_everywhere() {
        let frames = spectrogram(&vec![0.0; 32], 8, 4).unwrap();
        assert!(frames.iter().flatten().all(|m| *m == 0.0));
    }

    #[test]
    fn bad_sizes_and_hops_are_refused() {
        assert_eq!(spectrogram(&[0.0; 32], 12, 4), Err(SpectrogramError::BadFrameSize));
        assert_eq!(spectrogram(&[0.0; 32], 1, 4), Err(SpectrogramError::BadFrameSize));
        assert_eq!(spectrogram(&[0.0; 32], 8, 0), Err(SpectrogramError::ZeroHop));
    }

    #[test]
    fn magnitudes_convert_to_decibels_with_a_floor() {
        assert!((magnitude_to_db(1.0, -120.0)).abs() < 1e-12);
        assert!((magnitude_to_db(10.0, -120.0) - 20.0).abs() < 1e-9);
        assert_eq!(magnitude_to_db(0.0, -120.0), -120.0);
        assert_eq!(magnitude_to_db(1e-12, -60.0), -60.0);
    }
}
