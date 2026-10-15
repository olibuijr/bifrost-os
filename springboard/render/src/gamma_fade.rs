//! Gamma fade ramp for a screen saver's start and stop (the gradual fade behind `performGammaFade`). The model is pure
//! arithmetic: the caller samples its clock and applies the returned brightness level to the display gamma. Nothing
//! here touches a display, and the fade is linear in time.

/// Length of one fade in seconds when the caller has no preference.
pub const DEFAULT_FADE_SECS: f32 = 0.5;

/// Which way the brightness moves: `In` ramps from black to full, `Out` ramps from full to black.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FadeDirection {
    In,
    Out,
}

/// One gamma fade of a fixed length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GammaFade {
    secs: f32,
}

impl GammaFade {
    /// A fade of `secs` seconds. A value that is not finite or is negative makes a fade with no length, which jumps
    /// straight to its end level.
    pub fn new(secs: f32) -> Self {
        GammaFade { secs: if secs.is_finite() && secs > 0.0 { secs } else { 0.0 } }
    }

    /// The fade length in seconds.
    pub fn duration(&self) -> f32 {
        self.secs
    }

    /// The brightness level in 0..=1 after `elapsed` seconds. `In` goes 0 to 1 and `Out` goes 1 to 0, both linearly and
    /// clamped at the ends. A NaN elapsed time counts as the start, and a fade with no length is already at its end.
    pub fn level(&self, dir: FadeDirection, elapsed: f32) -> f32 {
        let t = if self.secs <= 0.0 {
            1.0
        } else if elapsed.is_nan() {
            0.0
        } else {
            (elapsed / self.secs).clamp(0.0, 1.0)
        };
        match dir {
            FadeDirection::In => t,
            FadeDirection::Out => 1.0 - t,
        }
    }

    /// `steps` evenly spaced levels across the whole fade, the first at the start and the last at the end. Zero steps
    /// give no levels and one step gives the end level alone.
    pub fn ramp(&self, dir: FadeDirection, steps: usize) -> Vec<f32> {
        match steps {
            0 => Vec::new(),
            1 => vec![self.level(dir, self.secs)],
            n => (0..n)
                .map(|i| self.level(dir, self.secs * i as f32 / (n - 1) as f32))
                .collect(),
        }
    }
}

/// The brightness of a saver's start and stop as one timeline. `start` begins a fade in at that time, and `stop` begins a
/// fade out from that time. A stop starts the fade out from full brightness, and a start after a stop begins a new fade in.
/// With the fade switched off the level is always 1, because the display gamma is never changed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FadeCycle {
    fade: GammaFade,
    enabled: bool,
    started_at: Option<f32>,
    stopped_at: Option<f32>,
}

impl FadeCycle {
    /// A cycle with fades of `secs` seconds. `enabled` is the saver's gamma fade request.
    pub fn new(secs: f32, enabled: bool) -> Self {
        FadeCycle { fade: GammaFade::new(secs), enabled, started_at: None, stopped_at: None }
    }

    /// Whether the cycle changes brightness at all.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The saver starts at `now` (seconds): a fade in begins.
    pub fn start(&mut self, now: f32) {
        self.started_at = Some(now);
        self.stopped_at = None;
    }

    /// The saver stops at `now` (seconds): a fade out begins. Stopping a saver that has not started does nothing.
    pub fn stop(&mut self, now: f32) {
        if self.started_at.is_some() {
            self.stopped_at = Some(now);
        }
    }

    /// The brightness level in 0..=1 at `now`. Before the first start it is 0; during a fade in it rises; while the saver
    /// runs it is 1; during a fade out it falls; after the fade out it is 0 again.
    pub fn level(&self, now: f32) -> f32 {
        if !self.enabled {
            return 1.0;
        }
        match (self.started_at, self.stopped_at) {
            (None, _) => 0.0,
            (Some(start), None) => self.fade.level(FadeDirection::In, now - start),
            (Some(_), Some(stop)) => self.fade.level(FadeDirection::Out, now - stop),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_and_out_are_mirror_images() {
        let f = GammaFade::new(0.5);
        assert_eq!(f.level(FadeDirection::In, 0.25), 0.5);
        assert_eq!(f.level(FadeDirection::Out, 0.25), 0.5);
        assert_eq!(f.level(FadeDirection::In, 0.125), 0.25);
        assert_eq!(f.level(FadeDirection::Out, 0.125), 0.75);
    }

    #[test]
    fn time_is_clamped_at_both_ends() {
        let f = GammaFade::new(0.5);
        assert_eq!(f.level(FadeDirection::In, -1.0), 0.0);
        assert_eq!(f.level(FadeDirection::In, 0.0), 0.0);
        assert_eq!(f.level(FadeDirection::In, 2.0), 1.0);
        assert_eq!(f.level(FadeDirection::Out, 2.0), 0.0);
        assert_eq!(f.level(FadeDirection::In, f32::NAN), 0.0);
    }

    #[test]
    fn invalid_length_jumps_to_the_end() {
        let zero = GammaFade::new(0.0);
        assert_eq!(zero.duration(), 0.0);
        assert_eq!(zero.level(FadeDirection::In, 0.0), 1.0);
        assert_eq!(zero.level(FadeDirection::Out, 0.0), 0.0);
        assert_eq!(GammaFade::new(-3.0).duration(), 0.0);
        assert_eq!(GammaFade::new(f32::NAN).duration(), 0.0);
        assert_eq!(GammaFade::new(DEFAULT_FADE_SECS).duration(), 0.5);
    }

    #[test]
    fn ramp_samples_start_to_end() {
        let f = GammaFade::new(0.5);
        assert_eq!(f.ramp(FadeDirection::In, 5), vec![0.0, 0.25, 0.5, 0.75, 1.0]);
        assert_eq!(f.ramp(FadeDirection::Out, 3), vec![1.0, 0.5, 0.0]);
        assert_eq!(f.ramp(FadeDirection::In, 1), vec![1.0]);
        assert!(f.ramp(FadeDirection::In, 0).is_empty());
    }
}
