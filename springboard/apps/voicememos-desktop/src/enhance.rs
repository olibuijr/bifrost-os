//! Enhance: a noise gate for voice memos. Quiet passages (room hum, hiss, the gaps between words) are turned down, so
//! speech stands out. Pure: it works on interleaved 16-bit samples and opens nothing. The gate follows the peak of
//! the whole frame (all channels), opens when the peak passes the threshold, and stays open for a short hold after
//! the last loud sample so word endings are not cut. Closed, it does not mute: it lowers the take by `DEPTH_DB`, so
//! a wrong setting sounds like a quieter room, not like a dropout.

/// Level above which the gate opens, in dBFS.
pub const THRESHOLD_DB: f32 = -42.0;
/// How far the gate lowers a closed passage, in dB (a negative number). It is a reduction, not silence.
pub const DEPTH_DB: f32 = -18.0;
/// The gate opens quickly so the start of a word is not clipped.
const ATTACK_MS: f32 = 5.0;
/// The gate closes slowly so a fading word is not chopped.
const RELEASE_MS: f32 = 120.0;
/// The gate stays open this long after the level last passed the threshold.
const HOLD_MS: f32 = 150.0;
/// How fast the peak follower falls back, as a time constant.
const PEAK_DECAY_MS: f32 = 30.0;

/// A one-pole coefficient for a time constant: how much of the distance to the target is covered each sample.
fn coefficient(ms: f32, rate: u32) -> f32 {
    let samples = (ms * 0.001 * rate as f32).max(1.0);
    1.0 - (-1.0 / samples).exp()
}

/// The gate's state across frames. One instance per take, so the envelope carries from one block to the next.
#[derive(Clone, Debug, PartialEq)]
pub struct Gate {
    threshold: f32,
    floor: f32,
    attack: f32,
    release: f32,
    peak_decay: f32,
    hold_frames: u32,
    /// peak follower of the frame magnitude, 0..=1
    peak: f32,
    /// current gain, `floor..=1`
    gain: f32,
    /// frames left before a closing gate may start to close
    hold_left: u32,
}

impl Gate {
    /// A gate for a take at `rate` Hz with the default threshold and depth.
    pub fn new(rate: u32) -> Gate {
        let rate = rate.max(1);
        Gate {
            threshold: db_to_linear(THRESHOLD_DB),
            floor: db_to_linear(DEPTH_DB),
            attack: coefficient(ATTACK_MS, rate),
            release: coefficient(RELEASE_MS, rate),
            peak_decay: coefficient(PEAK_DECAY_MS, rate),
            hold_frames: (HOLD_MS * 0.001 * rate as f32) as u32,
            peak: 0.0,
            gain: 1.0,
            hold_left: 0,
        }
    }

    /// Processes one frame (one sample per channel, interleaved order is kept) in place.
    pub fn process_frame(&mut self, frame: &mut [i16]) {
        let loudest = frame.iter().map(|s| s.unsigned_abs() as f32 / 32768.0).fold(0.0_f32, f32::max);
        self.peak = if loudest > self.peak { loudest } else { self.peak + (loudest - self.peak) * self.peak_decay };
        if self.peak >= self.threshold {
            self.hold_left = self.hold_frames;
        } else {
            self.hold_left = self.hold_left.saturating_sub(1);
        }
        let target = if self.hold_left > 0 { 1.0 } else { self.floor };
        let k = if target > self.gain { self.attack } else { self.release };
        self.gain += (target - self.gain) * k;
        for s in frame.iter_mut() {
            *s = (*s as f32 * self.gain).round().clamp(-32768.0, 32767.0) as i16;
        }
    }
}

/// Converts decibels to a linear amplitude factor.
pub fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Applies the noise gate to interleaved 16-bit samples of a take with `channels` channels at `rate` Hz. A trailing
/// partial frame is left alone.
pub fn gate_samples(samples: &mut [i16], rate: u32, channels: usize) {
    let width = channels.max(1);
    let mut gate = Gate::new(rate);
    for frame in samples.chunks_exact_mut(width) {
        gate.process_frame(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(samples: &[i16]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        (samples.iter().map(|&s| (s as f32).powi(2)).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// `hiss` quiet samples, then `speech` loud ones, then `hiss` quiet again.
    fn hiss_then_speech(hiss: usize, speech: usize) -> Vec<i16> {
        let quiet = |i: usize| if i % 2 == 0 { 40i16 } else { -40 };
        let loud = |i: usize| (((i as f32) * 0.3).sin() * 16000.0) as i16;
        let mut v: Vec<i16> = (0..hiss).map(quiet).collect();
        v.extend((0..speech).map(loud));
        v.extend((0..hiss).map(quiet));
        v
    }

    #[test]
    fn quiet_hiss_is_turned_down_and_speech_is_kept() {
        let rate = 16_000;
        let mut samples = hiss_then_speech(16_000, 16_000);
        let before_hiss = rms(&samples[..8000]);
        let before_speech = rms(&samples[20_000..28_000]);
        gate_samples(&mut samples, rate, 1);
        let after_hiss = rms(&samples[8000..16_000]);
        let after_speech = rms(&samples[20_000..28_000]);
        assert!(after_hiss < before_hiss * 0.5, "hiss before {before_hiss} after {after_hiss}");
        assert!(after_speech > before_speech * 0.95, "speech before {before_speech} after {after_speech}");
    }

    #[test]
    fn the_gate_reduces_by_the_depth_and_never_mutes() {
        let rate = 8_000;
        let mut samples = vec![40i16; 40_000];
        gate_samples(&mut samples, rate, 1);
        let last = *samples.last().unwrap() as f32;
        let expected = 40.0 * db_to_linear(DEPTH_DB);
        assert!((last - expected).abs() < 1.5, "closed gain: {last} vs {expected}");
        assert!(last != 0.0);
    }

    #[test]
    fn a_word_ending_is_held_open_before_the_gate_closes() {
        let rate = 8_000;
        let mut samples = vec![16000i16; 800];
        samples.extend(vec![40i16; 800]);
        gate_samples(&mut samples, rate, 1);
        let just_after = samples[800..840].iter().map(|s| s.unsigned_abs() as u32).max().unwrap();
        assert!(just_after > 30, "the hold keeps the tail of the word: {just_after}");
    }

    #[test]
    fn stereo_frames_share_one_gain_so_the_image_does_not_move() {
        // both channels are below the threshold, so the closed gate scales them by the same factor
        let rate = 8_000;
        let mut samples: Vec<i16> = Vec::new();
        for _ in 0..8000 {
            samples.push(250);
            samples.push(120);
        }
        gate_samples(&mut samples, rate, 2);
        let last = &samples[samples.len() - 2..];
        let balance = last[1] as f32 / last[0] as f32;
        assert!((balance - 120.0 / 250.0).abs() < 0.08, "left {} right {}", last[0], last[1]);
    }

    #[test]
    fn silence_and_empty_input_stay_as_they_are() {
        let mut silent = vec![0i16; 100];
        gate_samples(&mut silent, 8000, 1);
        assert!(silent.iter().all(|&s| s == 0));
        let mut empty: Vec<i16> = Vec::new();
        gate_samples(&mut empty, 8000, 2);
        assert!(empty.is_empty());
    }

    #[test]
    fn a_trailing_partial_frame_is_left_alone() {
        let mut samples = vec![40i16, 40, 40];
        gate_samples(&mut samples, 8000, 2);
        assert_eq!(samples[2], 40);
    }

    #[test]
    fn the_coefficients_are_fractions_and_faster_for_shorter_times() {
        let fast = coefficient(ATTACK_MS, 16_000);
        let slow = coefficient(RELEASE_MS, 16_000);
        assert!(fast > slow && fast < 1.0 && slow > 0.0);
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
    }
}
