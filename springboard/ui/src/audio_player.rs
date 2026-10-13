//! Audio player with the AVAudioPlayer shape: a player over decoded float samples with play, pause and stop, a current
//! time and a duration, a volume that can fade over a time, a stereo pan, a playback rate, and a loop count.
//!
//! The model renders interleaved stereo blocks. Pan uses the equal-power law of `panner.rs`, so the squared gains sum
//! to one across the pan range. A mono source is placed by the pan; a stereo source keeps both channels and the pan
//! scales each side (a balance). The rate resamples by linear interpolation and advances the playhead by `rate` source
//! frames per output frame, so a rate of 2 plays twice as fast and keeps the source sample rate. Durations and times
//! are in source time and do not change with the rate. Loops follow AVAudioPlayer: 0 plays once, N plays N extra
//! times, -1 loops until stopped. A player that finishes stops and rewinds to the start.
//!
//! Pure arithmetic with no device: the owner calls `render` from its own thread and feeds the blocks to an output.

/// The lowest playback rate a player accepts.
pub const MIN_RATE: f32 = 0.5;
/// The highest playback rate a player accepts.
pub const MAX_RATE: f32 = 2.0;
/// The channel count of rendered blocks.
pub const OUTPUT_CHANNELS: usize = 2;

/// Failures of the player model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerError {
    /// A value is not finite or is outside its range.
    InvalidValue,
    /// The source is empty, has no channels or more than two, or its sample rate is outside 8 kHz to 192 kHz.
    InvalidFormat,
}

/// The play state of a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    /// Not playing. A player that finished or was stopped is here, and a play starts from the current time.
    Stopped,
    /// Playing; `render` produces samples.
    Playing,
    /// Paused at the current time; `play` resumes from it.
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Fade {
    from: f32,
    to: f32,
    elapsed: u64,
    total: u64,
}

/// An audio player over interleaved float samples.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioPlayer {
    samples: Vec<f32>,
    channels: usize,
    sample_rate: u32,
    /// The playhead in source frames; fractional while the rate is not 1.
    position: f64,
    state: PlayState,
    volume: f32,
    fade: Option<Fade>,
    pan: f32,
    rate: f32,
    loops: i32,
    /// Passes still to come for the current play, None for an endless loop.
    passes_left: Option<u32>,
}

fn finite_in(v: f32, lo: f32, hi: f32) -> Result<f32, PlayerError> {
    if v.is_finite() && (lo..=hi).contains(&v) {
        Ok(v)
    } else {
        Err(PlayerError::InvalidValue)
    }
}

impl AudioPlayer {
    /// A stopped player at time zero over `samples` (interleaved, `channels` of 1 or 2, a whole number of frames).
    /// Volume is 1, pan is centre, rate is 1 and the player plays once.
    pub fn new(samples: Vec<f32>, channels: u16, sample_rate: u32) -> Result<Self, PlayerError> {
        if !(1..=2).contains(&channels) || !(8_000..=192_000).contains(&sample_rate) {
            return Err(PlayerError::InvalidFormat);
        }
        let channels = channels as usize;
        if samples.is_empty() || samples.len() % channels != 0 {
            return Err(PlayerError::InvalidFormat);
        }
        if samples.iter().any(|s| !s.is_finite()) {
            return Err(PlayerError::InvalidValue);
        }
        Ok(Self {
            samples,
            channels,
            sample_rate,
            position: 0.0,
            state: PlayState::Stopped,
            volume: 1.0,
            fade: None,
            pan: 0.0,
            rate: 1.0,
            loops: 0,
            passes_left: Some(0),
        })
    }

    /// Source frames in the player.
    pub fn total_frames(&self) -> u64 {
        (self.samples.len() / self.channels) as u64
    }

    /// The length of the source in seconds. Independent of the rate.
    pub fn duration(&self) -> f64 {
        self.total_frames() as f64 / self.sample_rate as f64
    }

    /// The playhead in seconds from the start.
    pub fn current_time(&self) -> f64 {
        self.position / self.sample_rate as f64
    }

    /// Moves the playhead. A time past the end is clamped to the duration; a negative or non-finite time is refused.
    pub fn set_current_time(&mut self, seconds: f64) -> Result<(), PlayerError> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(PlayerError::InvalidValue);
        }
        self.position = (seconds * self.sample_rate as f64).min(self.total_frames() as f64);
        Ok(())
    }

    /// The play state.
    pub fn state(&self) -> PlayState {
        self.state
    }

    /// True while the player is playing (not paused or stopped).
    pub fn is_playing(&self) -> bool {
        self.state == PlayState::Playing
    }

    /// Starts or resumes playback. A play from the stopped state starts the loop count afresh; a resume from pause
    /// keeps the passes already left.
    pub fn play(&mut self) {
        if self.state == PlayState::Stopped {
            self.passes_left = passes_for(self.loops);
        }
        self.state = PlayState::Playing;
    }

    /// Pauses at the current time.
    pub fn pause(&mut self) {
        if self.state == PlayState::Playing {
            self.state = PlayState::Paused;
        }
    }

    /// Stops playback. The current time is kept, so a later play starts from it.
    pub fn stop(&mut self) {
        self.state = PlayState::Stopped;
    }

    /// The volume now, from 0 to 1. During a fade this is the ramp's current value.
    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Sets the volume at once and cancels any fade. The value must be in 0 to 1.
    pub fn set_volume(&mut self, volume: f32) -> Result<(), PlayerError> {
        self.volume = finite_in(volume, 0.0, 1.0)?;
        self.fade = None;
        Ok(())
    }

    /// Ramps the volume linearly from its current value to `volume` over `seconds`. A zero time sets it at once.
    pub fn fade_to(&mut self, volume: f32, seconds: f64) -> Result<(), PlayerError> {
        let to = finite_in(volume, 0.0, 1.0)?;
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(PlayerError::InvalidValue);
        }
        let total = (seconds * self.sample_rate as f64).round() as u64;
        if total == 0 {
            self.volume = to;
            self.fade = None;
        } else {
            self.fade = Some(Fade { from: self.volume, to, elapsed: 0, total });
        }
        Ok(())
    }

    /// The stereo pan, from -1 (left) to 1 (right), 0 centre.
    pub fn pan(&self) -> f32 {
        self.pan
    }

    /// Sets the stereo pan. The value must be in -1 to 1.
    pub fn set_pan(&mut self, pan: f32) -> Result<(), PlayerError> {
        self.pan = finite_in(pan, -1.0, 1.0)?;
        Ok(())
    }

    /// The playback rate. 1 is the source speed.
    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// Sets the playback rate. The value must be in `MIN_RATE` to `MAX_RATE`.
    pub fn set_rate(&mut self, rate: f32) -> Result<(), PlayerError> {
        self.rate = finite_in(rate, MIN_RATE, MAX_RATE)?;
        Ok(())
    }

    /// The loop count: 0 plays once, N plays N more times, -1 loops until stopped.
    pub fn loops(&self) -> i32 {
        self.loops
    }

    /// Sets the loop count. Values below -1 are refused.
    pub fn set_loops(&mut self, loops: i32) -> Result<(), PlayerError> {
        if loops < -1 {
            return Err(PlayerError::InvalidValue);
        }
        self.loops = loops;
        Ok(())
    }

    /// Renders `frames` frames of interleaved stereo. Frames after the end of a finished play are silence.
    pub fn render(&mut self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * OUTPUT_CHANNELS];
        for f in 0..frames {
            if self.state != PlayState::Playing {
                break;
            }
            if self.position >= self.total_frames() as f64 && !self.wrap_or_finish() {
                break;
            }
            let (l, r) = self.sample_pair();
            let gain = self.step_volume();
            let (gl, gr) = pan_gains(self.pan);
            out[2 * f] = l * gl * gain;
            out[2 * f + 1] = r * gr * gain;
            self.position += self.rate as f64;
        }
        out
    }

    /// Handles the playhead passing the end: starts another pass when the loop count allows one, otherwise stops and
    /// rewinds. Returns false when the play is over.
    fn wrap_or_finish(&mut self) -> bool {
        let again = match self.passes_left {
            None => true,
            Some(n) if n > 0 => {
                self.passes_left = Some(n - 1);
                true
            }
            Some(_) => false,
        };
        if again {
            self.position -= self.total_frames() as f64;
        } else {
            self.state = PlayState::Stopped;
            self.position = 0.0;
        }
        again
    }

    /// The source at the playhead, interpolated linearly, as a left and right sample before pan and volume.
    fn sample_pair(&self) -> (f32, f32) {
        let last = self.total_frames() as usize - 1;
        let idx = (self.position.floor() as usize).min(last);
        let next = (idx + 1).min(last);
        let frac = (self.position - idx as f64) as f32;
        let frame = |i: usize, c: usize| self.samples[i * self.channels + c];
        if self.channels == 1 {
            let a = frame(idx, 0);
            let b = frame(next, 0);
            let m = a + (b - a) * frac;
            (m, m)
        } else {
            let (al, ar) = (frame(idx, 0), frame(idx, 1));
            let (bl, br) = (frame(next, 0), frame(next, 1));
            (al + (bl - al) * frac, ar + (br - ar) * frac)
        }
    }

    /// Advances the volume by one frame and returns the gain for that frame.
    fn step_volume(&mut self) -> f32 {
        if let Some(fade) = self.fade.as_mut() {
            fade.elapsed += 1;
            if fade.elapsed >= fade.total {
                self.volume = fade.to;
                self.fade = None;
            } else {
                let t = fade.elapsed as f32 / fade.total as f32;
                self.volume = fade.from + (fade.to - fade.from) * t;
            }
        }
        self.volume
    }
}

/// The passes left for a loop count: None for an endless loop, otherwise the plays after the first.
fn passes_for(loops: i32) -> Option<u32> {
    if loops < 0 {
        None
    } else {
        Some(loops as u32)
    }
}

/// Equal-power stereo gains for a pan in -1 to 1 (the same law as `panner.rs`).
fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = (pan + 1.0) * 0.5 * std::f32::consts::FRAC_PI_2;
    (angle.cos(), angle.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 8_000;

    fn mono(samples: &[f32]) -> AudioPlayer {
        AudioPlayer::new(samples.to_vec(), 1, RATE).unwrap()
    }

    fn left_only(mut p: AudioPlayer) -> AudioPlayer {
        p.set_pan(-1.0).unwrap();
        p
    }

    #[test]
    fn new_refuses_bad_formats_and_values() {
        assert_eq!(AudioPlayer::new(vec![], 1, RATE), Err(PlayerError::InvalidFormat));
        assert_eq!(AudioPlayer::new(vec![0.0; 3], 2, RATE), Err(PlayerError::InvalidFormat));
        assert_eq!(AudioPlayer::new(vec![0.0; 3], 3, RATE), Err(PlayerError::InvalidFormat));
        assert_eq!(AudioPlayer::new(vec![0.0; 3], 1, 4_000), Err(PlayerError::InvalidFormat));
        assert_eq!(AudioPlayer::new(vec![f32::NAN], 1, RATE), Err(PlayerError::InvalidValue));
    }

    #[test]
    fn duration_and_current_time_use_source_time() {
        let mut p = mono(&vec![0.0; RATE as usize]);
        assert_eq!(p.duration(), 1.0);
        p.set_current_time(0.5).unwrap();
        assert_eq!(p.current_time(), 0.5);
        p.set_current_time(10.0).unwrap();
        assert_eq!(p.current_time(), 1.0);
        assert_eq!(p.set_current_time(-1.0), Err(PlayerError::InvalidValue));
        assert_eq!(p.set_current_time(f64::NAN), Err(PlayerError::InvalidValue));
    }

    #[test]
    fn play_once_then_stop_and_rewind() {
        let mut p = left_only(mono(&[0.5, 0.5, 0.5, 0.5]));
        p.play();
        assert!(p.is_playing());
        let out = p.render(6);
        assert_eq!(out, vec![0.5, 0.0, 0.5, 0.0, 0.5, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(p.state(), PlayState::Stopped);
        assert_eq!(p.current_time(), 0.0);
    }

    #[test]
    fn loop_count_adds_passes() {
        let mut p = left_only(mono(&[0.5, 0.5, 0.5, 0.5]));
        p.set_loops(1).unwrap();
        p.play();
        let out = p.render(10);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        assert_eq!(left, vec![0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.0, 0.0]);
        assert_eq!(p.state(), PlayState::Stopped);
    }

    #[test]
    fn endless_loop_keeps_playing_until_stopped() {
        let mut p = left_only(mono(&[0.25, 0.25]));
        p.set_loops(-1).unwrap();
        p.play();
        let _ = p.render(100);
        assert!(p.is_playing());
        p.stop();
        assert_eq!(p.render(4), vec![0.0; 8]);
    }

    #[test]
    fn pause_keeps_time_and_play_resumes() {
        let mut p = left_only(mono(&[0.1, 0.2, 0.3, 0.4]));
        p.play();
        let _ = p.render(2);
        p.pause();
        assert_eq!(p.state(), PlayState::Paused);
        assert_eq!(p.render(2), vec![0.0; 4]);
        p.play();
        let out = p.render(2);
        assert_eq!(out[0], 0.3);
        assert_eq!(out[2], 0.4);
    }

    #[test]
    fn rate_two_skips_every_other_frame() {
        let mut p = left_only(mono(&[0.0, 0.25, 0.5, 0.75]));
        p.set_rate(2.0).unwrap();
        p.play();
        let out = p.render(2);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[2], 0.5);
        assert_eq!(p.current_time(), 4.0 / RATE as f64);
    }

    #[test]
    fn half_rate_interpolates_linearly() {
        let mut p = left_only(mono(&[0.0, 0.25, 0.5, 0.75]));
        p.set_rate(0.5).unwrap();
        p.play();
        let out = p.render(2);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[2], 0.125);
    }

    #[test]
    fn rate_outside_range_is_refused() {
        let mut p = mono(&[0.0]);
        assert_eq!(p.set_rate(0.4), Err(PlayerError::InvalidValue));
        assert_eq!(p.set_rate(2.5), Err(PlayerError::InvalidValue));
        assert_eq!(p.rate(), 1.0);
    }

    #[test]
    fn pan_uses_equal_power_gains() {
        let mut p = mono(&[1.0, 1.0]);
        p.play();
        let out = p.render(1);
        let half = std::f32::consts::FRAC_1_SQRT_2;
        assert!((out[0] - half).abs() < 1e-6);
        assert!((out[1] - half).abs() < 1e-6);
        assert_eq!(p.set_pan(1.5), Err(PlayerError::InvalidValue));
    }

    #[test]
    fn volume_fade_ramps_over_time() {
        let mut p = AudioPlayer::new(vec![0.5; 10_000], 1, RATE).unwrap();
        p.set_loops(-1).unwrap();
        p.play();
        p.fade_to(0.0, 0.5).unwrap();
        let _ = p.render(2_000);
        assert!((p.volume() - 0.5).abs() < 1e-3);
        let _ = p.render(2_000);
        assert_eq!(p.volume(), 0.0);
    }

    #[test]
    fn set_volume_cancels_fade_and_clamps_range() {
        let mut p = mono(&[0.5; 100]);
        p.fade_to(0.0, 1.0).unwrap();
        p.set_volume(0.25).unwrap();
        p.play();
        let _ = p.render(10);
        assert_eq!(p.volume(), 0.25);
        assert_eq!(p.set_volume(1.5), Err(PlayerError::InvalidValue));
        assert_eq!(p.fade_to(0.5, -1.0), Err(PlayerError::InvalidValue));
    }

    #[test]
    fn zero_length_fade_sets_volume_at_once() {
        let mut p = mono(&[0.5; 4]);
        p.fade_to(0.0, 0.0).unwrap();
        assert_eq!(p.volume(), 0.0);
    }
}
