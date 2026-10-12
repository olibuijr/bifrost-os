//! Region export: the part of a rendered song between two times, written as 32-bit float WAV. The bounds come
//! from `bifrost_app::controls::media_trim::TrimSession`, which keeps them inside the song and at least a bar apart.
use crate::render::Rendered;
use crate::wav;

/// The left and right samples from `start` to `end` seconds. Bounds are clamped to the rendered length; a range
/// that is empty or runs backwards gives no samples.
pub fn slice(rendered: &Rendered, start: f64, end: f64) -> (Vec<f32>, Vec<f32>) {
    let sr = f64::from(rendered.sample_rate);
    let len = rendered.left.len().min(rendered.right.len());
    let a = frame(start, sr).min(len);
    let b = frame(end, sr).min(len);
    if b <= a {
        return (Vec::new(), Vec::new());
    }
    (rendered.left[a..b].to_vec(), rendered.right[a..b].to_vec())
}

/// The sample index of a time in seconds. Negative and non-finite times count as the start.
fn frame(seconds: f64, sr: f64) -> usize {
    if seconds.is_finite() && seconds > 0.0 {
        (seconds * sr).round() as usize
    } else {
        0
    }
}

/// The region as a 32-bit float stereo WAV file.
pub fn region_wav(rendered: &Rendered, start: f64, end: f64) -> Vec<u8> {
    let (left, right) = slice(rendered, start, end);
    wav::encode_f32(&left, &right, rendered.sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered() -> Rendered {
        Rendered {
            sample_rate: 4,
            left: (0..8).map(|i| i as f32).collect(),
            right: (0..8).map(|i| -(i as f32)).collect(),
            bars: vec![],
            missing: vec![],
            content_end: 2.0,
        }
    }

    #[test]
    fn slices_between_the_rounded_frames() {
        let (l, r) = slice(&rendered(), 0.25, 0.75);
        assert_eq!(l, vec![1.0, 2.0]);
        assert_eq!(r, vec![-1.0, -2.0]);
    }

    #[test]
    fn clamps_to_the_rendered_length_and_ignores_empty_ranges() {
        assert_eq!(slice(&rendered(), 0.0, 99.0).0.len(), 8);
        assert!(slice(&rendered(), 1.5, 1.0).0.is_empty());
        assert!(slice(&rendered(), f64::NAN, -1.0).0.is_empty());
    }

    #[test]
    fn region_wav_is_a_riff_file_with_two_channels_of_floats() {
        let bytes = region_wav(&rendered(), 0.0, 1.0);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 4 * 8);
    }
}
