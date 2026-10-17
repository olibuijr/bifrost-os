//! Crossfade between two songs: the gains of the outgoing and the incoming song across the overlap. The curve is equal
//! power, so the sum of the squared gains stays at one through the whole overlap and the loudness does not dip in the
//! middle. Pure: nothing here reads or plays audio. The engine mixes the frames it is given with these gains.

use std::f32::consts::FRAC_PI_2;

/// The longest overlap a crossfade may use, in milliseconds.
pub const MAX_OVERLAP_MS: u32 = 12_000;

/// The gains `(outgoing, incoming)` at `t` through the overlap. `t` runs from 0 (the overlap starts) to 1 (it ends) and is
/// clamped to that range; NaN counts as 0.
pub fn gains(t: f32) -> (f32, f32) {
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    let angle = t * FRAC_PI_2;
    (angle.cos(), angle.sin())
}

/// The overlap in frames for a length in milliseconds at `rate` frames per second. The length is capped at
/// `MAX_OVERLAP_MS`, and 0 means no overlap.
pub fn overlap_frames(len_ms: u32, rate: u32) -> usize {
    let ms = len_ms.min(MAX_OVERLAP_MS) as u64;
    (ms * rate as u64 / 1000) as usize
}

/// Walks an overlap one frame at a time. The first frame is at `t = 0` and the last at `t = 1`; a one-frame overlap is
/// entirely the incoming song.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fade {
    total: usize,
    done: usize,
}

impl Fade {
    /// A fade over `frames` frames.
    pub fn new(frames: usize) -> Fade {
        Fade { total: frames, done: 0 }
    }

    /// The gains `(outgoing, incoming)` for the next frame, or `None` once the overlap is over.
    pub fn next_gains(&mut self) -> Option<(f32, f32)> {
        if self.done >= self.total {
            return None;
        }
        let t = if self.total <= 1 { 1.0 } else { self.done as f32 / (self.total - 1) as f32 };
        self.done += 1;
        Some(gains(t))
    }

    /// Frames still to be mixed.
    pub fn remaining(&self) -> usize {
        self.total - self.done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn the_curve_starts_on_the_outgoing_song_and_ends_on_the_incoming_one() {
        let (out, inc) = gains(0.0);
        assert!(close(out, 1.0) && close(inc, 0.0));
        let (out, inc) = gains(1.0);
        assert!(close(out, 0.0) && close(inc, 1.0));
    }

    #[test]
    fn the_summed_power_stays_at_one_through_the_overlap() {
        for step in 0..=20 {
            let (out, inc) = gains(step as f32 / 20.0);
            assert!(close(out * out + inc * inc, 1.0), "step {step}");
        }
    }

    #[test]
    fn out_of_range_and_nan_positions_are_clamped() {
        assert_eq!(gains(-3.0), gains(0.0));
        assert_eq!(gains(7.0), gains(1.0));
        assert_eq!(gains(f32::NAN), gains(0.0));
    }

    #[test]
    fn the_overlap_in_frames_is_capped_and_zero_means_none() {
        assert_eq!(overlap_frames(2000, 48_000), 96_000);
        assert_eq!(overlap_frames(0, 48_000), 0);
        assert_eq!(overlap_frames(60_000, 48_000), 576_000);
    }

    #[test]
    fn a_fade_yields_each_frame_once_from_start_to_end() {
        let mut fade = Fade::new(3);
        assert_eq!(fade.remaining(), 3);
        assert_eq!(fade.next_gains().map(|g| g.0), Some(1.0));
        let (_, mid) = fade.next_gains().unwrap();
        assert!(close(mid, gains(0.5).1));
        assert_eq!(fade.remaining(), 1);
        let last = fade.next_gains().unwrap();
        assert!(close(last.1, 1.0));
        assert_eq!(fade.next_gains(), None);
        assert_eq!(fade.remaining(), 0);
    }

    #[test]
    fn a_one_frame_fade_is_entirely_the_incoming_song() {
        let mut fade = Fade::new(1);
        let (out, inc) = fade.next_gains().unwrap();
        assert!(close(out, 0.0) && close(inc, 1.0));
        assert_eq!(fade.next_gains(), None);
    }
}
