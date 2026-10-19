//! Per-player effects the audio service applies to decoded stereo PCM: stereo pan (balance), a timed volume fade and
//! a gain ramp across one block. Pure functions on interleaved S16 samples, so the real-time thread only calls them.
//!
//! * Pan runs from -1 (left only) to 1 (right only); 0 leaves both channels at full level (a balance law: the
//!   channel away from the pan is attenuated, the other is untouched), the same as a player's `pan` property.
//! * A `Fade` ramps the volume linearly from where it was to a target over a duration; the player asks it for the
//!   volume at the start and the end of each block and `ramp` interpolates the gain inside the block, so a fade has
//!   no steps however long the blocks are.

/// Clamps a pan to -1..=1. NaN becomes centre.
pub fn clamp_pan(pan: f32) -> f32 {
    if pan.is_nan() {
        0.0
    } else {
        pan.clamp(-1.0, 1.0)
    }
}

/// The (left, right) gains of a pan: the channel on the far side of the pan falls linearly to silence.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = clamp_pan(pan);
    ((1.0 - p).min(1.0), (1.0 + p).min(1.0))
}

fn mul(sample: i16, gain: f32) -> i16 {
    // 16.16 fixed point, rounded, saturating
    let g = (gain * 65536.0) as i64;
    ((sample as i64 * g + 32768) >> 16).clamp(i16::MIN as i64, i16::MAX as i64) as i16
}

/// Applies a pan to interleaved stereo samples. A no-op at centre.
pub fn apply_pan(pcm: &mut [i16], pan: f32) {
    let (l, r) = pan_gains(pan);
    if l >= 1.0 && r >= 1.0 {
        return;
    }
    for frame in pcm.chunks_exact_mut(2) {
        frame[0] = mul(frame[0], l);
        frame[1] = mul(frame[1], r);
    }
}

/// Scales interleaved stereo samples by a gain that moves linearly from `g0` at the first frame to `g1` after the
/// last, so consecutive blocks join without a step.
pub fn ramp(pcm: &mut [i16], g0: f32, g1: f32) {
    let frames = pcm.len() / 2;
    if frames == 0 {
        return;
    }
    if g0 == g1 {
        if g0 < 1.0 {
            pcm.iter_mut().for_each(|s| *s = mul(*s, g0.max(0.0)));
        }
        return;
    }
    for (i, frame) in pcm.chunks_exact_mut(2).enumerate() {
        let g = (g0 + (g1 - g0) * (i as f32 / frames as f32)).max(0.0);
        frame[0] = mul(frame[0], g);
        frame[1] = mul(frame[1], g);
    }
}

/// A linear volume ramp from `from` to `to` that starts at `start_ms` and lasts `duration_ms`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    pub from: f32,
    pub to: f32,
    pub start_ms: f64,
    pub duration_ms: f64,
}

impl Fade {
    pub fn new(from: f32, to: f32, start_ms: f64, duration_ms: f64) -> Fade {
        let c = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
        Fade { from: c(from), to: c(to), start_ms, duration_ms: duration_ms.max(0.0) }
    }

    /// The volume at `now_ms`.
    pub fn volume_at(&self, now_ms: f64) -> f32 {
        if self.duration_ms <= 0.0 || now_ms >= self.start_ms + self.duration_ms {
            return self.to;
        }
        if now_ms <= self.start_ms {
            return self.from;
        }
        let t = ((now_ms - self.start_ms) / self.duration_ms) as f32;
        self.from + (self.to - self.from) * t
    }

    /// True once `now_ms` has reached the end.
    pub fn done(&self, now_ms: f64) -> bool {
        now_ms >= self.start_ms + self.duration_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_leaves_the_near_channel_alone_and_fades_the_far_one() {
        assert_eq!(pan_gains(0.0), (1.0, 1.0));
        assert_eq!(pan_gains(-1.0), (1.0, 0.0));
        assert_eq!(pan_gains(1.0), (0.0, 1.0));
        assert_eq!(pan_gains(0.5), (0.5, 1.0));
        assert_eq!(pan_gains(f32::NAN), (1.0, 1.0));
        assert_eq!(pan_gains(9.0), (0.0, 1.0));
    }

    #[test]
    fn apply_pan_scales_the_right_samples() {
        let mut pcm = [1000i16, 1000, -2000, -2000, 30_000, 30_000];
        apply_pan(&mut pcm, 0.5);
        assert_eq!(pcm, [500, 1000, -1000, -2000, 15_000, 30_000]);
        let mut hard_left = [700i16, 700, -700, 700];
        apply_pan(&mut hard_left, -1.0);
        assert_eq!(hard_left, [700, 0, -700, 0]);
        let mut centre = [123i16, -456];
        apply_pan(&mut centre, 0.0);
        assert_eq!(centre, [123, -456]);
    }

    #[test]
    fn a_ramp_moves_linearly_across_the_block_and_joins_the_next() {
        let mut pcm = vec![10_000i16; 8];
        ramp(&mut pcm, 1.0, 0.0);
        // 4 frames: gains 1.0, 0.75, 0.5, 0.25
        assert_eq!(pcm, vec![10_000, 10_000, 7_500, 7_500, 5_000, 5_000, 2_500, 2_500]);
        let mut flat = vec![1000i16; 4];
        ramp(&mut flat, 0.5, 0.5);
        assert_eq!(flat, vec![500; 4]);
        let mut unity = vec![1000i16; 4];
        ramp(&mut unity, 1.0, 1.0);
        assert_eq!(unity, vec![1000; 4]);
        let mut empty: Vec<i16> = Vec::new();
        ramp(&mut empty, 0.0, 1.0);
        let mut loud = vec![i16::MAX; 4];
        ramp(&mut loud, 1.0, 1.0);
        assert_eq!(loud, vec![i16::MAX; 4]);
    }

    #[test]
    fn blocks_of_a_fade_chain_without_a_step() {
        // a 100 ms fade from 1 to 0 played as two 50 ms blocks: the second starts where the first ended
        let f = Fade::new(1.0, 0.0, 0.0, 100.0);
        let (a0, a1) = (f.volume_at(0.0), f.volume_at(50.0));
        let (b0, b1) = (f.volume_at(50.0), f.volume_at(100.0));
        assert_eq!((a0, a1, b0, b1), (1.0, 0.5, 0.5, 0.0));
        assert!(f.done(100.0) && !f.done(99.9));
    }

    #[test]
    fn a_fade_holds_before_it_starts_and_after_it_ends_and_clamps_its_ends() {
        let f = Fade::new(0.2, 0.8, 1_000.0, 500.0);
        assert_eq!(f.volume_at(0.0), 0.2);
        assert!((f.volume_at(1_250.0) - 0.5).abs() < 1e-6);
        assert_eq!(f.volume_at(9_000.0), 0.8);
        let z = Fade::new(0.5, 2.0, 0.0, 0.0);
        assert_eq!(z.volume_at(0.0), 1.0);
        assert!(z.done(0.0));
        assert_eq!(Fade::new(f32::NAN, -1.0, 0.0, 10.0).from, 0.0);
    }
}
