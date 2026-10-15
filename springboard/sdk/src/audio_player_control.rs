//! Playback controls that `AVAudioPlayer` adds on top of play, pause, stop and volume: stereo pan, playback rate and a
//! timed volume fade (`setVolume(_:fadeDuration:)`).
//!
//! Rules of this model:
//! - Volume is 0 to 1 (NaN becomes 0, which is silence). Pan is -1 (left) to 1 (right) (NaN becomes centre).
//!   Rate is 0.5 to 2.0 (NaN becomes 1.0).
//! - A fade ramps linearly from the volume in effect when it starts to the target. Time is passed in as milliseconds
//!   by the caller; a zero-length fade is immediate.
//! - `set_volume` cancels a running fade.
//!
//! This is the pure model. audiod does not apply these values yet, so the matrix row that uses it is partial.

/// Lowest playback rate `AVAudioPlayer` accepts.
pub const RATE_MIN: f32 = 0.5;
/// Highest playback rate `AVAudioPlayer` accepts.
pub const RATE_MAX: f32 = 2.0;

/// Clamps a volume to 0 to 1. NaN becomes 0.
pub fn clamp_volume(volume: f32) -> f32 {
    if volume.is_nan() {
        0.0
    } else {
        volume.clamp(0.0, 1.0)
    }
}

/// Clamps a stereo pan to -1 to 1. NaN becomes centre (0).
pub fn clamp_pan(pan: f32) -> f32 {
    if pan.is_nan() {
        0.0
    } else {
        pan.clamp(-1.0, 1.0)
    }
}

/// Clamps a playback rate to 0.5 to 2.0. NaN becomes normal speed (1.0).
pub fn clamp_rate(rate: f32) -> f32 {
    if rate.is_nan() {
        1.0
    } else {
        rate.clamp(RATE_MIN, RATE_MAX)
    }
}

/// A linear volume ramp that starts at `start_ms` and lasts `duration_ms`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeFade {
    from: f32,
    to: f32,
    start_ms: u64,
    duration_ms: u64,
}

impl VolumeFade {
    pub fn new(from: f32, to: f32, start_ms: u64, duration_ms: u64) -> Self {
        Self { from: clamp_volume(from), to: clamp_volume(to), start_ms, duration_ms }
    }

    /// The volume of the ramp at `now_ms`.
    pub fn volume_at(&self, now_ms: u64) -> f32 {
        let end_ms = self.start_ms.saturating_add(self.duration_ms);
        if self.duration_ms == 0 || now_ms >= end_ms {
            return self.to;
        }
        if now_ms <= self.start_ms {
            return self.from;
        }
        let t = (now_ms - self.start_ms) as f32 / self.duration_ms as f32;
        self.from + (self.to - self.from) * t
    }

    /// The volume the ramp ends at.
    pub fn target(&self) -> f32 {
        self.to
    }
}

/// The playback controls of one `AVAudioPlayer`-style player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerControl {
    volume: f32,
    pan: f32,
    rate: f32,
    fade: Option<VolumeFade>,
}

impl PlayerControl {
    pub fn new() -> Self {
        Self { volume: 1.0, pan: 0.0, rate: 1.0, fade: None }
    }

    /// Sets the volume at once and cancels any fade.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = clamp_volume(volume);
        self.fade = None;
    }

    /// Starts a fade from the volume in effect at `now_ms` to `target`, lasting `duration_ms`.
    pub fn fade_volume(&mut self, target: f32, duration_ms: u64, now_ms: u64) {
        let from = self.volume_at(now_ms);
        self.fade = Some(VolumeFade::new(from, target, now_ms, duration_ms));
    }

    /// The volume in effect at `now_ms`, including a running fade.
    pub fn volume_at(&self, now_ms: u64) -> f32 {
        match self.fade {
            Some(fade) => fade.volume_at(now_ms),
            None => self.volume,
        }
    }

    /// Whether a fade is still ramping at `now_ms`.
    pub fn is_fading(&self, now_ms: u64) -> bool {
        match self.fade {
            Some(fade) => now_ms < fade.start_ms.saturating_add(fade.duration_ms),
            None => false,
        }
    }

    pub fn set_pan(&mut self, pan: f32) {
        self.pan = clamp_pan(pan);
    }

    pub fn pan(&self) -> f32 {
        self.pan
    }

    pub fn set_rate(&mut self, rate: f32) {
        self.rate = clamp_rate(rate);
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }
}

impl Default for PlayerControl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn pan_and_rate_are_clamped_and_nan_safe() {
        assert_eq!(clamp_pan(-3.0), -1.0);
        assert_eq!(clamp_pan(0.25), 0.25);
        assert_eq!(clamp_pan(f32::NAN), 0.0);
        assert_eq!(clamp_rate(0.1), RATE_MIN);
        assert_eq!(clamp_rate(9.0), RATE_MAX);
        assert_eq!(clamp_rate(f32::NAN), 1.0);
        assert_eq!(clamp_volume(1.5), 1.0);
        assert_eq!(clamp_volume(f32::NAN), 0.0);
    }

    #[test]
    fn player_defaults_and_setters_clamp() {
        let mut p = PlayerControl::new();
        assert_eq!((p.pan(), p.rate(), p.volume_at(0)), (0.0, 1.0, 1.0));
        p.set_pan(-2.0);
        p.set_rate(4.0);
        assert_eq!((p.pan(), p.rate()), (-1.0, RATE_MAX));
    }

    #[test]
    fn fade_ramps_linearly_and_settles_on_the_target() {
        let mut p = PlayerControl::new();
        p.set_volume(1.0);
        p.fade_volume(0.0, 1000, 500);
        assert!(close(p.volume_at(500), 1.0));
        assert!(close(p.volume_at(1000), 0.5));
        assert!(close(p.volume_at(1500), 0.0));
        assert!(close(p.volume_at(9000), 0.0));
        assert!(p.is_fading(1000));
        assert!(!p.is_fading(1500));
    }

    #[test]
    fn a_zero_length_fade_is_immediate() {
        let mut p = PlayerControl::new();
        p.fade_volume(0.2, 0, 10);
        assert!(close(p.volume_at(10), 0.2));
        assert!(!p.is_fading(10));
    }

    #[test]
    fn a_new_fade_starts_from_the_volume_in_effect() {
        let mut p = PlayerControl::new();
        p.fade_volume(0.0, 1000, 0);
        p.fade_volume(1.0, 1000, 500);
        assert!(close(p.volume_at(500), 0.5));
        assert!(close(p.volume_at(1500), 1.0));
    }

    #[test]
    fn set_volume_cancels_a_fade() {
        let mut p = PlayerControl::new();
        p.fade_volume(0.0, 1000, 0);
        p.set_volume(0.8);
        assert!(close(p.volume_at(500), 0.8));
        assert!(!p.is_fading(500));
    }

    #[test]
    fn fade_arithmetic_does_not_overflow_at_the_end_of_time() {
        let fade = VolumeFade::new(1.0, 0.0, u64::MAX - 10, 10);
        assert!(close(fade.volume_at(u64::MAX), 0.0));
        assert!(close(fade.volume_at(u64::MAX - 5), 0.5));
        assert_eq!(fade.target(), 0.0);
    }
}
