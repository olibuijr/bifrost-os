//! Echo cancellation for voice capture: an adaptive filter learns how the playback signal (the far end) comes back
//! into the microphone (the near end) and subtracts that estimate, so a voice chat or a recording made while the
//! speakers play does not hear its own output. The filter is a normalised least-mean-squares (NLMS) model over a
//! ring of past far-end samples. While the near end is talking over the far end, adaptation is frozen so the speech
//! is not learned as echo (a Geigel-style double-talk test). This is a pure model: it does not open a device, and the
//! caller feeds the playback reference and the microphone samples in time order.

/// The most taps a canceller may have. At 48 kHz this covers about 85 ms of echo path.
pub const MAX_TAPS: usize = 4096;

/// Adaptation is frozen on a sample whose magnitude exceeds this share of the largest far-end magnitude in the
/// filter's window. An echo path louder than this share would never adapt, so a caller with a hot playback path
/// should lower the reference level first.
pub const DOUBLE_TALK_RATIO: f32 = 0.5;

/// Added to the input power in the normalisation, so silence does not divide by zero.
const POWER_FLOOR: f32 = 1e-6;

/// Why an echo canceller was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EchoError {
    /// The tap count is zero or above `MAX_TAPS`.
    BadTaps,
    /// The step size is not a finite number in the open range (0, 2). NLMS is stable only in that range.
    BadStep,
}

/// An NLMS echo canceller with a fixed number of taps.
#[derive(Clone, Debug)]
pub struct EchoCanceller {
    weights: Vec<f32>,
    /// Ring of the most recent far-end samples. `head` is the slot the next sample is written to.
    far: Vec<f32>,
    head: usize,
    step: f32,
}

impl EchoCanceller {
    /// A canceller with `taps` filter taps and adaptation step `step`. A step near 0.1 to 0.5 suits speech.
    pub fn new(taps: usize, step: f32) -> Result<Self, EchoError> {
        if taps == 0 || taps > MAX_TAPS {
            return Err(EchoError::BadTaps);
        }
        if !(step > 0.0 && step < 2.0) {
            return Err(EchoError::BadStep);
        }
        Ok(EchoCanceller { weights: vec![0.0; taps], far: vec![0.0; taps], head: 0, step })
    }

    /// The number of filter taps.
    pub fn taps(&self) -> usize {
        self.weights.len()
    }

    /// Takes one far-end (playback reference) sample and one near-end (microphone) sample, both taken at the same
    /// time, and returns the near-end sample with the estimated echo removed. A non-finite sample counts as silence.
    pub fn process(&mut self, far: f32, near: f32) -> f32 {
        let far = finite(far);
        let near = finite(near);
        let taps = self.weights.len();
        self.far[self.head] = far;
        self.head = (self.head + 1) % taps;

        // Tap k reads the far-end sample k steps back; tap 0 is the newest.
        let mut estimate = 0.0f32;
        let mut power = 0.0f32;
        let mut peak = 0.0f32;
        for k in 0..taps {
            let x = self.far[self.slot(k)];
            estimate += self.weights[k] * x;
            power += x * x;
            peak = peak.max(x.abs());
        }
        let error = near - estimate;

        // Near-end speech over the far end freezes adaptation. With no far-end signal at all, nothing can be learnt.
        let double_talk = near.abs() > DOUBLE_TALK_RATIO * peak;
        if !double_talk {
            let gain = self.step * error / (power + POWER_FLOOR);
            for k in 0..taps {
                let x = self.far[self.slot(k)];
                self.weights[k] += gain * x;
            }
        }
        error
    }

    /// Forgets the learnt echo path and the far-end history.
    pub fn reset(&mut self) {
        self.weights.iter_mut().for_each(|w| *w = 0.0);
        self.far.iter_mut().for_each(|x| *x = 0.0);
        self.head = 0;
    }

    /// The ring index of the far-end sample `k` steps back (0 is the newest). `head + taps - 1 - k` cannot underflow
    /// because `k < taps`.
    fn slot(&self, k: usize) -> usize {
        let taps = self.weights.len();
        (self.head + taps - 1 - k) % taps
    }
}

fn finite(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_refuses_bad_taps_and_steps() {
        assert_eq!(EchoCanceller::new(0, 0.5).err(), Some(EchoError::BadTaps));
        assert_eq!(EchoCanceller::new(MAX_TAPS + 1, 0.5).err(), Some(EchoError::BadTaps));
        assert_eq!(EchoCanceller::new(4, 0.0).err(), Some(EchoError::BadStep));
        assert_eq!(EchoCanceller::new(4, 2.0).err(), Some(EchoError::BadStep));
        assert_eq!(EchoCanceller::new(4, f32::NAN).err(), Some(EchoError::BadStep));
        assert_eq!(EchoCanceller::new(MAX_TAPS, 1.9).unwrap().taps(), MAX_TAPS);
    }

    #[test]
    fn without_far_end_signal_the_near_end_passes_through() {
        let mut c = EchoCanceller::new(4, 0.5).unwrap();
        for near in [0.3f32, -0.2, 0.0, 0.7] {
            assert_eq!(c.process(0.0, near), near);
        }
    }

    #[test]
    fn one_tap_echo_is_learnt_in_one_step() {
        // Echo gain 0.4 with a step of 1.0. First sample: weight becomes 0.4 / (1 + 1e-6), about 0.3999996.
        // Second sample: the estimate is that weight times 1, so the residual is about 4e-7.
        let mut c = EchoCanceller::new(1, 1.0).unwrap();
        let first = c.process(1.0, 0.4);
        assert!((first - 0.4).abs() < 1e-6, "first {first}");
        let second = c.process(1.0, 0.4);
        assert!(second.abs() < 1e-5, "second {second}");
    }

    #[test]
    fn delayed_echo_uses_the_older_tap() {
        // Two taps. The far end sends a pulse at t0 and the microphone hears 0.4 of it at t1, so the learnt weight
        // sits on tap 1. At t3 the far end pulse at t2 comes back at 0.4 and is removed.
        let mut c = EchoCanceller::new(2, 1.0).unwrap();
        assert_eq!(c.process(1.0, 0.0), 0.0);
        let learn = c.process(0.0, 0.4);
        assert!((learn - 0.4).abs() < 1e-6, "learn {learn}");
        assert!(c.process(1.0, 0.0).abs() < 1e-6);
        let cancelled = c.process(0.0, 0.4);
        assert!(cancelled.abs() < 1e-5, "cancelled {cancelled}");
    }

    #[test]
    fn near_end_speech_freezes_adaptation() {
        // Near 0.9 is above 0.5 times the far peak of 1.0, so the sample is double talk: the output is the raw
        // near value and the weight stays at 0. The next sample therefore sees no learnt echo.
        let mut c = EchoCanceller::new(1, 1.0).unwrap();
        assert_eq!(c.process(1.0, 0.9), 0.9);
        assert_eq!(c.process(1.0, 0.4), 0.4);
    }

    #[test]
    fn reset_forgets_the_learnt_path() {
        let mut c = EchoCanceller::new(1, 1.0).unwrap();
        c.process(1.0, 0.4);
        assert!(c.process(1.0, 0.4).abs() < 1e-5);
        c.reset();
        assert_eq!(c.process(1.0, 0.4), 0.4);
    }

    #[test]
    fn non_finite_samples_count_as_silence() {
        let mut c = EchoCanceller::new(2, 0.5).unwrap();
        assert_eq!(c.process(f32::NAN, 0.25), 0.25);
        assert_eq!(c.process(0.0, f32::INFINITY), 0.0);
    }
}
