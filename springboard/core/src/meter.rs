//! Level metering for 16-bit PCM blocks: the peak and the average power of each channel, in decibels relative to
//! full scale (dBFS). A level meter reads these per block; silence reports `FLOOR_DB`. Pure arithmetic, no device
//! access, so the recorder and any meter view can share it.

/// The lowest value reported. Silence and anything quieter than about one step of 16-bit audio read as this.
pub const FLOOR_DB: f32 = -160.0;

/// Peak and average power of one channel over a block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Levels {
    /// Largest absolute sample, in dBFS.
    pub peak_db: f32,
    /// Root mean square of the samples (the average power), in dBFS.
    pub average_db: f32,
}

/// Converts a linear amplitude (1.0 is full scale) to dBFS. Zero, negative and non-finite input gives `FLOOR_DB`.
pub fn to_db(amplitude: f32) -> f32 {
    if !amplitude.is_finite() || amplitude <= 0.0 {
        return FLOOR_DB;
    }
    (20.0 * amplitude.log10()).max(FLOOR_DB)
}

/// Measures an interleaved block (`L R L R ...`). Returns one `Levels` per channel. A trailing partial frame is
/// ignored, and zero channels gives an empty result.
pub fn measure(samples: &[i16], channels: usize) -> Vec<Levels> {
    if channels == 0 {
        return Vec::new();
    }
    let frames = samples.len() / channels;
    let mut peak = vec![0u32; channels];
    let mut square = vec![0f64; channels];
    for frame in samples.chunks_exact(channels).take(frames) {
        for (c, &s) in frame.iter().enumerate() {
            let magnitude = s.unsigned_abs() as u32;
            peak[c] = peak[c].max(magnitude);
            let x = s as f64 / 32768.0;
            square[c] += x * x;
        }
    }
    (0..channels)
        .map(|c| {
            let average = if frames == 0 { 0.0 } else { (square[c] / frames as f64).sqrt() as f32 };
            Levels { peak_db: to_db(peak[c] as f32 / 32768.0), average_db: to_db(average) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tolerance: f32) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn silence_reads_as_the_floor() {
        let levels = measure(&[0; 64], 2);
        assert_eq!(levels.len(), 2);
        for l in levels {
            assert_eq!(l.peak_db, FLOOR_DB);
            assert_eq!(l.average_db, FLOOR_DB);
        }
    }

    #[test]
    fn full_scale_peak_is_near_zero_dbfs() {
        let levels = measure(&[i16::MIN, 0, i16::MAX, 0], 1);
        assert!(close(levels[0].peak_db, 0.0, 0.01), "peak {}", levels[0].peak_db);
    }

    #[test]
    fn half_scale_constant_is_about_minus_six_db() {
        let levels = measure(&[16384; 100], 1);
        assert!(close(levels[0].peak_db, -6.02, 0.05), "peak {}", levels[0].peak_db);
        assert!(close(levels[0].average_db, -6.02, 0.05), "average {}", levels[0].average_db);
    }

    #[test]
    fn channels_are_measured_separately() {
        // left full scale, right silent, interleaved
        let block = [i16::MAX, 0, i16::MAX, 0, i16::MAX, 0];
        let levels = measure(&block, 2);
        assert!(levels[0].peak_db > -0.1);
        assert_eq!(levels[1].peak_db, FLOOR_DB);
    }

    #[test]
    fn average_is_below_peak_for_a_varying_signal() {
        let block = [20000, -20000, 0, 0, 10000, -10000, 0, 0];
        let l = measure(&block, 1)[0];
        assert!(l.average_db < l.peak_db);
        assert!(close(l.peak_db, to_db(20000.0 / 32768.0), 0.001));
    }

    #[test]
    fn trailing_partial_frame_and_zero_channels_are_handled() {
        // three samples at two channels: one whole frame, the odd sample is ignored
        let l = measure(&[16384, 16384, 32767], 2);
        assert_eq!(l.len(), 2);
        assert!(close(l[0].peak_db, -6.02, 0.05));
        assert!(measure(&[1, 2, 3], 0).is_empty());
        assert!(measure(&[], 2)[0].peak_db == FLOOR_DB);
    }

    #[test]
    fn to_db_handles_bad_input_without_panicking() {
        assert_eq!(to_db(0.0), FLOOR_DB);
        assert_eq!(to_db(-1.0), FLOOR_DB);
        assert_eq!(to_db(f32::NAN), FLOOR_DB);
        assert!(close(to_db(1.0), 0.0, 1e-6));
        assert!(close(to_db(0.1), -20.0, 1e-4));
    }
}
