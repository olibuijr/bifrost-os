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

/// The tail of a song that has ended, fading out under the start of the next song.
#[derive(Debug)]
struct Mix {
    /// interleaved stereo S16: the last frames of the song that ended
    tail: Vec<i16>,
    /// frames of the tail already mixed
    at: usize,
    fade: Fade,
}

impl Mix {
    fn remaining(&self) -> usize {
        self.tail.len() / 2 - self.at
    }
}

/// Plays one song into the next with a crossfade, on interleaved stereo S16 blocks.
///
/// While a song plays, the last `frames` frames of it are held back, so when it ends they can still be faded: `push`
/// passes on everything older than that. When the next song starts, `begin_mix` turns the held tail into the outgoing
/// side of the fade and the next song's first frames are mixed under it with the equal-power gains, so those frames are
/// not played twice. The overlap therefore shortens the two songs' total by the tail's length. With `frames` at 0 nothing
/// is held and `push` hands every block straight on. Pure: it only moves samples between buffers.
#[derive(Debug)]
pub struct Crossfader {
    frames: usize,
    /// interleaved stereo S16: the most recent frames of the song, at most `frames` of them
    held: Vec<i16>,
    mix: Option<Mix>,
}

impl Crossfader {
    /// A crossfader that holds back `frames` frames (0 turns the crossfade off).
    pub fn new(frames: usize) -> Self {
        Crossfader { frames, held: Vec::new(), mix: None }
    }

    /// The frames held back at most.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Whether `push` would change anything: the crossfade is on, or there is still a tail to play.
    pub fn active(&self) -> bool {
        self.frames > 0 || !self.held.is_empty() || self.mix.is_some()
    }

    /// Frames of the song held back now.
    pub fn held_frames(&self) -> usize {
        self.held.len() / 2
    }

    /// Whether the start of a song is being mixed under a tail.
    pub fn mixing(&self) -> bool {
        self.mix.is_some()
    }

    /// Changes how many frames are held back. Frames held beyond the new length are released into `out`.
    pub fn set_frames(&mut self, frames: usize, out: &mut Vec<i16>) {
        self.frames = frames;
        self.release_excess(out);
    }

    fn release_excess(&mut self, out: &mut Vec<i16>) {
        let held = self.held_frames();
        if held > self.frames {
            out.extend(self.held.drain(..(held - self.frames) * 2));
        }
    }

    /// Feeds a block of the song playing and appends what is ready to play to `out`: the mixed start of a song that
    /// follows a tail, and then everything older than the held frames.
    pub fn push(&mut self, block: &[i16], out: &mut Vec<i16>) {
        let mut rest = block;
        let mut finished = false;
        if let Some(mix) = &mut self.mix {
            let n = (rest.len() / 2).min(mix.remaining());
            for f in 0..n {
                let (outgoing, incoming) = mix.fade.next_gains().unwrap_or((0.0, 1.0));
                for c in 0..2 {
                    let a = mix.tail[(mix.at + f) * 2 + c] as f32 * outgoing;
                    let b = rest[f * 2 + c] as f32 * incoming;
                    out.push((a + b).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16);
                }
            }
            mix.at += n;
            rest = &rest[n * 2..];
            finished = mix.remaining() == 0;
        }
        if finished {
            self.mix = None;
        }
        if rest.is_empty() {
            return;
        }
        if self.frames == 0 && self.held.is_empty() {
            out.extend_from_slice(rest);
            return;
        }
        self.held.extend_from_slice(rest);
        self.release_excess(out);
    }

    /// The song ended and another follows: the held tail becomes the outgoing side of a fade that the next `push`es mix
    /// the new song's first frames under. With nothing held there is nothing to fade.
    pub fn begin_mix(&mut self) {
        let tail = std::mem::take(&mut self.held);
        let frames = tail.len() / 2;
        self.mix = (frames > 0).then(|| Mix { tail, at: 0, fade: Fade::new(frames) });
    }

    /// Playback ends here (the last song, or a song that would not open): releases what is held into `out`. A tail still
    /// being mixed fades out on the rest of its curve.
    pub fn flush(&mut self, out: &mut Vec<i16>) {
        if let Some(mut mix) = self.mix.take() {
            for f in mix.at..mix.tail.len() / 2 {
                let (outgoing, _) = mix.fade.next_gains().unwrap_or((0.0, 1.0));
                for c in 0..2 {
                    out.push((mix.tail[f * 2 + c] as f32 * outgoing).round() as i16);
                }
            }
        }
        out.append(&mut self.held);
    }

    /// Drops everything held and any mix in progress (a skip or a seek: the old song's audio is not wanted).
    pub fn reset(&mut self) {
        self.held.clear();
        self.mix = None;
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

    /// `n` stereo frames of the constant `v`.
    fn flat(v: i16, n: usize) -> Vec<i16> {
        vec![v; n * 2]
    }

    fn near(a: i16, b: i16) -> bool {
        (a as i32 - b as i32).abs() <= 1
    }

    #[test]
    fn with_no_overlap_every_block_passes_straight_through() {
        let mut x = Crossfader::new(0);
        let mut out = Vec::new();
        x.push(&[1, 2, 3, 4], &mut out);
        assert_eq!(out, vec![1, 2, 3, 4]);
        assert_eq!((x.held_frames(), x.active()), (0, false));
    }

    #[test]
    fn the_last_frames_of_a_song_are_held_back_in_order() {
        let mut x = Crossfader::new(2);
        let mut out = Vec::new();
        // five frames, each channel pair (n, n)
        let block: Vec<i16> = (1..=5).flat_map(|n| [n, n]).collect();
        x.push(&block, &mut out);
        assert_eq!(out, vec![1, 1, 2, 2, 3, 3], "everything older than the last two frames plays");
        assert_eq!(x.held_frames(), 2);
        let mut more = Vec::new();
        x.push(&[6, 6], &mut more);
        assert_eq!(more, vec![4, 4], "a new frame releases the oldest held one");
        let mut rest = Vec::new();
        x.flush(&mut rest);
        assert_eq!(rest, vec![5, 5, 6, 6], "the end of the queue plays out whole");
        assert_eq!((x.held_frames(), x.active()), (0, true));
    }

    #[test]
    fn the_next_song_is_mixed_under_the_held_tail_with_equal_power() {
        let mut x = Crossfader::new(3);
        let mut out = Vec::new();
        x.push(&flat(1000, 6), &mut out);
        assert_eq!(out, flat(1000, 3), "the first half of the song played alone");
        x.begin_mix();
        assert!(x.mixing() && x.held_frames() == 0);
        out.clear();
        x.push(&flat(2000, 6), &mut out);
        assert_eq!(out.len(), 6, "three mixed frames; the other three of the new song are held back");
        assert!(near(out[0], 1000) && near(out[1], 1000), "the fade starts on the song that ended");
        assert!(near(out[2], 2121) && near(out[3], 2121), "halfway the two sum to 1000 cos + 2000 sin of a quarter turn");
        assert!(near(out[4], 2000) && near(out[5], 2000), "and ends on the new song");
        assert!(!x.mixing());
        assert_eq!(x.held_frames(), 3);
        out.clear();
        x.push(&flat(2000, 3), &mut out);
        assert_eq!(out, flat(2000, 3), "after the mix the new song plays on, again held back by the overlap");
    }

    #[test]
    fn a_mix_split_over_several_blocks_gives_the_same_samples() {
        let mut whole = Crossfader::new(4);
        let mut split = Crossfader::new(4);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for x in [&mut whole, &mut split] {
            x.push(&flat(500, 8), &mut Vec::new());
            x.begin_mix();
        }
        whole.push(&flat(-900, 4), &mut a);
        for _ in 0..4 {
            split.push(&flat(-900, 1), &mut b);
        }
        assert_eq!(a, b);
        assert_eq!(a.len(), 8);
    }

    #[test]
    fn the_sum_of_two_loud_songs_is_clamped_not_wrapped() {
        let mut x = Crossfader::new(3);
        x.push(&flat(i16::MAX, 3), &mut Vec::new());
        x.begin_mix();
        let mut out = Vec::new();
        x.push(&flat(i16::MAX, 3), &mut out);
        assert!(out.iter().all(|s| *s > 0), "{out:?}");
        assert_eq!(out[2], i16::MAX, "at the middle the two add past full scale");
    }

    #[test]
    fn shrinking_the_overlap_releases_the_frames_it_no_longer_holds() {
        let mut x = Crossfader::new(4);
        x.push(&flat(7, 6), &mut Vec::new());
        assert_eq!(x.held_frames(), 4);
        let mut out = Vec::new();
        x.set_frames(2, &mut out);
        assert_eq!(out.len(), 4, "two frames released");
        assert_eq!(x.held_frames(), 2);
        out.clear();
        x.set_frames(0, &mut out);
        assert_eq!(out.len(), 4);
        assert_eq!((x.held_frames(), x.frames()), (0, 0));
    }

    #[test]
    fn a_song_shorter_than_the_overlap_fades_whole() {
        let mut x = Crossfader::new(100);
        x.push(&flat(800, 2), &mut Vec::new());
        assert_eq!(x.held_frames(), 2, "all of the short song is held");
        x.begin_mix();
        let mut out = Vec::new();
        x.push(&flat(0, 2), &mut out);
        assert_eq!(out.len(), 4);
        assert!(near(out[0], 800) && near(out[2], 0), "it fades out over two frames: {out:?}");
    }

    #[test]
    fn ending_during_a_mix_fades_the_rest_of_the_tail_out_and_a_skip_drops_everything() {
        let mut x = Crossfader::new(4);
        x.push(&flat(1000, 4), &mut Vec::new());
        x.begin_mix();
        let mut out = Vec::new();
        x.push(&flat(0, 1), &mut out);
        assert_eq!(out.len(), 2);
        out.clear();
        x.flush(&mut out);
        assert_eq!(out.len(), 6, "three tail frames remain");
        assert!(out.windows(2).step_by(2).all(|w| w[0] == w[1]));
        assert!(out[0] > out[2] && out[2] > out[4], "and they keep fading: {out:?}");
        assert!(!x.mixing());
        x.push(&flat(5, 6), &mut Vec::new());
        x.reset();
        assert_eq!((x.held_frames(), x.mixing()), (0, false));
        let mut none = Vec::new();
        x.flush(&mut none);
        assert!(none.is_empty(), "a reset leaves nothing to play");
    }
}
