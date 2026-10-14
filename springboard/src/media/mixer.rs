//! Mixing and layering: several sources of interleaved samples are summed into one output, each through its own strip
//! (a gain and a mute). The sum saturates at the sample range and counts the samples it had to clip, so a layer that
//! is too loud shows up in a number. A source that is shorter than the others is silent past its end. Pure arithmetic:
//! no device, no clock.

/// One layer's level: a linear gain and a mute.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strip {
    pub gain: f32,
    pub muted: bool,
}

impl Default for Strip {
    fn default() -> Self {
        Strip { gain: 1.0, muted: false }
    }
}

impl Strip {
    /// A strip at `db` decibels relative to unity. Negative infinity is silence.
    pub fn from_db(db: f32) -> Self {
        Strip { gain: db_to_linear(db), muted: false }
    }
}

/// Converts decibels to a linear amplitude factor: 0 dB is 1.0, -6 dB is about 0.5, and negative infinity is 0.0.
pub fn db_to_linear(db: f32) -> f32 {
    if db == f32::NEG_INFINITY {
        return 0.0;
    }
    10f32.powf(db / 20.0)
}

/// The summed output and the number of samples the sum had to clip to the sample range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mixed {
    pub samples: Vec<i16>,
    pub clipped: usize,
}

/// Sums the layers. The output is as long as the longest layer. A muted layer, or a layer with a gain of zero,
/// contributes nothing. The order of the layers does not change the result.
pub fn mix(layers: &[(&[i16], Strip)]) -> Mixed {
    let length = layers.iter().map(|(samples, _)| samples.len()).max().unwrap_or(0);
    let mut sums = vec![0i32; length];
    for (samples, strip) in layers {
        if strip.muted || strip.gain == 0.0 {
            continue;
        }
        for (sum, &sample) in sums.iter_mut().zip(samples.iter()) {
            *sum += (f32::from(sample) * strip.gain).round() as i32;
        }
    }
    let mut clipped = 0;
    let samples = sums
        .into_iter()
        .map(|sum| {
            let clamped = sum.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
            if clamped != sum {
                clipped += 1;
            }
            clamped as i16
        })
        .collect();
    Mixed { samples, clipped }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decibels_map_to_linear_factors() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_linear(-6.0) - 0.501_187).abs() < 1e-4);
        assert_eq!(db_to_linear(f32::NEG_INFINITY), 0.0);
    }

    #[test]
    fn a_single_unity_layer_comes_out_unchanged() {
        let out = mix(&[(&[1, -2, 3], Strip::default())]);
        assert_eq!(out, Mixed { samples: vec![1, -2, 3], clipped: 0 });
    }

    #[test]
    fn two_layers_are_summed_sample_by_sample() {
        let out = mix(&[(&[100, 200], Strip::default()), (&[1, 2], Strip::default())]);
        assert_eq!(out.samples, vec![101, 202]);
    }

    #[test]
    fn a_layer_at_minus_six_db_is_about_half_its_amplitude() {
        let out = mix(&[(&[1000], Strip::from_db(-6.0))]);
        assert_eq!(out.samples, vec![501]);
    }

    #[test]
    fn a_muted_layer_adds_nothing() {
        let quiet = Strip { gain: 1.0, muted: true };
        let out = mix(&[(&[500, 500], quiet), (&[1, 1], Strip::default())]);
        assert_eq!(out.samples, vec![1, 1]);
    }

    #[test]
    fn a_sum_past_the_sample_range_saturates_and_is_counted() {
        let loud = [30_000i16, -30_000, 10];
        let out = mix(&[(&loud, Strip::default()), (&loud, Strip::default())]);
        assert_eq!(out.samples, vec![i16::MAX, i16::MIN, 20]);
        assert_eq!(out.clipped, 2);
    }

    #[test]
    fn a_shorter_layer_is_silent_past_its_end() {
        let out = mix(&[(&[5, 5, 5], Strip::default()), (&[1], Strip::default())]);
        assert_eq!(out.samples, vec![6, 5, 5]);
    }

    #[test]
    fn no_layers_make_no_output() {
        assert_eq!(mix(&[]), Mixed { samples: Vec::new(), clipped: 0 });
    }

    #[test]
    fn the_order_of_the_layers_does_not_change_the_result() {
        let a: &[i16] = &[10, -20, 30];
        let b: &[i16] = &[7, 7];
        let forward = mix(&[(a, Strip::default()), (b, Strip::from_db(-3.0))]);
        let backward = mix(&[(b, Strip::from_db(-3.0)), (a, Strip::default())]);
        assert_eq!(forward, backward);
    }
}
