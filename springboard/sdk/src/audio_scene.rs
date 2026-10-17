//! A positional audio scene in the shape of OpenAL: buffers of samples, sources placed in space that play them, a
//! listener, and a mixer that renders the scene to interleaved stereo.
//!
//! The geometry is `positional_audio` (inverse-distance attenuation clamped to the source's reference and maximum
//! distances, and an equal-power pan against the listener's right vector). This module adds what turns that model into
//! sound: sample buffers, the source life cycle (initial, playing, paused, stopped), looping, per-source gain and pitch,
//! sources that move with the listener, sample-rate conversion by linear interpolation, and the mix itself. A stereo
//! buffer is played as recorded, without positioning, as OpenAL does.
//!
//! `Scene::mix` returns the next block of frames as `f32` pairs in the range -1 to 1. The app writes them to the audio
//! service through its usual output (`audio_session`); mixing here keeps the scene's timing in the caller's hands. The
//! Doppler shift of moving sources is not modelled.

use crate::positional_audio::{self, Listener, PositionError, Source, Vec3};
use std::fmt;

/// A buffer of samples. One channel is positioned in space; two channels play as recorded.
#[derive(Clone, Debug, PartialEq)]
pub struct Buffer {
    pub sample_rate: u32,
    pub channels: u8,
    /// Interleaved samples, -1 to 1.
    pub samples: Vec<f32>,
}

impl Buffer {
    /// The number of frames (a frame has one sample per channel).
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }
}

/// The handle of a buffer in a scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferId(usize);

/// The handle of a source in a scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceId(usize);

/// Where a source is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceState {
    Initial,
    Playing,
    Paused,
    Stopped,
}

/// Why a call was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneError {
    /// The id names nothing (never created, or deleted).
    InvalidName,
    /// A value is out of range (a non-finite number, a negative gain or pitch, a zero rate, odd channel count).
    InvalidValue,
    /// The buffer is attached to a source and cannot be deleted, or the source has no buffer to play.
    InvalidOperation,
    /// The listener or a source breaks the geometry rules.
    Position(PositionError),
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SceneError::InvalidName => f.write_str("no such buffer or source"),
            SceneError::InvalidValue => f.write_str("a value is out of range"),
            SceneError::InvalidOperation => f.write_str("the operation does not apply in this state"),
            SceneError::Position(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SceneError {}

impl From<PositionError> for SceneError {
    fn from(e: PositionError) -> SceneError {
        SceneError::Position(e)
    }
}

struct Voice {
    buffer: Option<BufferId>,
    geometry: Source,
    /// The position is relative to the listener (x right, y up, -z ahead) and moves with it.
    relative: bool,
    gain: f32,
    pitch: f32,
    looping: bool,
    state: SourceState,
    /// The read position in frames of the buffer.
    at: f64,
}

/// The scene: a listener, buffers and sources.
pub struct Scene {
    rate: u32,
    listener: Listener,
    master_gain: f32,
    buffers: Vec<Option<Buffer>>,
    voices: Vec<Option<Voice>>,
}

fn finite3(v: Vec3) -> bool {
    v.iter().all(|x| x.is_finite())
}

fn unit(v: Vec3) -> Option<Vec3> {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (n > 0.0 && n.is_finite()).then(|| [v[0] / n, v[1] / n, v[2] / n])
}

impl Scene {
    /// A scene that renders at `sample_rate` frames a second.
    pub fn new(sample_rate: u32) -> Result<Scene, SceneError> {
        if sample_rate == 0 {
            return Err(SceneError::InvalidValue);
        }
        Ok(Scene { rate: sample_rate, listener: Listener::default(), master_gain: 1.0, buffers: Vec::new(), voices: Vec::new() })
    }

    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    pub fn listener(&self) -> &Listener {
        &self.listener
    }

    /// Places and turns the listener. The orientation is checked here, so the mix cannot fail on it later.
    pub fn set_listener(&mut self, listener: Listener) -> Result<(), SceneError> {
        if !finite3(listener.position) || !finite3(listener.forward) || !finite3(listener.up) {
            return Err(SceneError::InvalidValue);
        }
        positional_audio::right_vector(&listener)?;
        self.listener = listener;
        Ok(())
    }

    /// The gain applied to the whole mix (0 is silence, 1 leaves it as is).
    pub fn set_master_gain(&mut self, gain: f32) -> Result<(), SceneError> {
        if !gain.is_finite() || gain < 0.0 {
            return Err(SceneError::InvalidValue);
        }
        self.master_gain = gain;
        Ok(())
    }

    // --- buffers -------------------------------------------------------------------------------------------------

    /// Stores samples as a buffer. One or two channels; the sample count must be a whole number of frames.
    pub fn create_buffer(&mut self, sample_rate: u32, channels: u8, samples: Vec<f32>) -> Result<BufferId, SceneError> {
        if sample_rate == 0 || !(1..=2).contains(&channels) || samples.len() % usize::from(channels) != 0 {
            return Err(SceneError::InvalidValue);
        }
        self.buffers.push(Some(Buffer { sample_rate, channels, samples }));
        Ok(BufferId(self.buffers.len() - 1))
    }

    pub fn buffer(&self, id: BufferId) -> Option<&Buffer> {
        self.buffers.get(id.0).and_then(|b| b.as_ref())
    }

    /// Deletes a buffer. A buffer a source still uses is refused.
    pub fn delete_buffer(&mut self, id: BufferId) -> Result<(), SceneError> {
        if self.buffer(id).is_none() {
            return Err(SceneError::InvalidName);
        }
        if self.voices.iter().flatten().any(|v| v.buffer == Some(id)) {
            return Err(SceneError::InvalidOperation);
        }
        self.buffers[id.0] = None;
        Ok(())
    }

    // --- sources -------------------------------------------------------------------------------------------------

    /// A new source at the origin with the default distance model and no buffer.
    pub fn create_source(&mut self) -> SourceId {
        self.voices.push(Some(Voice {
            buffer: None,
            geometry: Source::at([0.0; 3]),
            relative: false,
            gain: 1.0,
            pitch: 1.0,
            looping: false,
            state: SourceState::Initial,
            at: 0.0,
        }));
        SourceId(self.voices.len() - 1)
    }

    pub fn delete_source(&mut self, id: SourceId) -> Result<(), SceneError> {
        self.voice(id)?;
        self.voices[id.0] = None;
        Ok(())
    }

    fn voice(&self, id: SourceId) -> Result<&Voice, SceneError> {
        self.voices.get(id.0).and_then(|v| v.as_ref()).ok_or(SceneError::InvalidName)
    }

    fn voice_mut(&mut self, id: SourceId) -> Result<&mut Voice, SceneError> {
        self.voices.get_mut(id.0).and_then(|v| v.as_mut()).ok_or(SceneError::InvalidName)
    }

    /// Attaches a buffer to a source. A source that is playing or paused is stopped first, as OpenAL requires.
    pub fn set_buffer(&mut self, source: SourceId, buffer: BufferId) -> Result<(), SceneError> {
        if self.buffer(buffer).is_none() {
            return Err(SceneError::InvalidName);
        }
        let v = self.voice_mut(source)?;
        v.buffer = Some(buffer);
        v.state = SourceState::Initial;
        v.at = 0.0;
        Ok(())
    }

    pub fn set_position(&mut self, source: SourceId, position: Vec3) -> Result<(), SceneError> {
        if !finite3(position) {
            return Err(SceneError::InvalidValue);
        }
        self.voice_mut(source)?.geometry.position = position;
        Ok(())
    }

    /// The distance model: gain 1 inside `reference`, none gained beyond `max`, falling by `rolloff` between.
    pub fn set_distance_model(&mut self, source: SourceId, reference: f32, max: f32, rolloff: f32) -> Result<(), SceneError> {
        let mut geometry = self.voice(source)?.geometry;
        geometry.reference_distance = reference;
        geometry.max_distance = max;
        geometry.rolloff = rolloff;
        positional_audio::check_source(&geometry)?;
        self.voice_mut(source)?.geometry = geometry;
        Ok(())
    }

    /// Makes the source's position relative to the listener, so it moves and turns with it.
    pub fn set_relative(&mut self, source: SourceId, relative: bool) -> Result<(), SceneError> {
        self.voice_mut(source)?.relative = relative;
        Ok(())
    }

    pub fn set_gain(&mut self, source: SourceId, gain: f32) -> Result<(), SceneError> {
        if !gain.is_finite() || gain < 0.0 {
            return Err(SceneError::InvalidValue);
        }
        self.voice_mut(source)?.gain = gain;
        Ok(())
    }

    /// The playback speed: 1 as recorded, 2 an octave up and twice as fast.
    pub fn set_pitch(&mut self, source: SourceId, pitch: f32) -> Result<(), SceneError> {
        if !pitch.is_finite() || pitch <= 0.0 {
            return Err(SceneError::InvalidValue);
        }
        self.voice_mut(source)?.pitch = pitch;
        Ok(())
    }

    pub fn set_looping(&mut self, source: SourceId, looping: bool) -> Result<(), SceneError> {
        self.voice_mut(source)?.looping = looping;
        Ok(())
    }

    pub fn state(&self, source: SourceId) -> Result<SourceState, SceneError> {
        Ok(self.voice(source)?.state)
    }

    /// The read position in seconds of the buffer.
    pub fn position_seconds(&self, source: SourceId) -> Result<f64, SceneError> {
        let v = self.voice(source)?;
        let rate = v.buffer.and_then(|b| self.buffer(b)).map_or(1, |b| b.sample_rate);
        Ok(v.at / f64::from(rate))
    }

    /// Starts or resumes a source. A source without a buffer cannot play.
    pub fn play(&mut self, source: SourceId) -> Result<(), SceneError> {
        let has_buffer = self.voice(source)?.buffer.is_some();
        if !has_buffer {
            return Err(SceneError::InvalidOperation);
        }
        let v = self.voice_mut(source)?;
        if v.state == SourceState::Stopped {
            v.at = 0.0;
        }
        v.state = SourceState::Playing;
        Ok(())
    }

    pub fn pause(&mut self, source: SourceId) -> Result<(), SceneError> {
        let v = self.voice_mut(source)?;
        if v.state == SourceState::Playing {
            v.state = SourceState::Paused;
        }
        Ok(())
    }

    /// Stops a source and rewinds it.
    pub fn stop(&mut self, source: SourceId) -> Result<(), SceneError> {
        let v = self.voice_mut(source)?;
        if v.state != SourceState::Initial {
            v.state = SourceState::Stopped;
        }
        v.at = 0.0;
        Ok(())
    }

    /// Rewinds a source to its initial state.
    pub fn rewind(&mut self, source: SourceId) -> Result<(), SceneError> {
        let v = self.voice_mut(source)?;
        v.state = SourceState::Initial;
        v.at = 0.0;
        Ok(())
    }

    // --- mixing --------------------------------------------------------------------------------------------------

    /// The world position of a source (a relative one is placed in the listener's own frame).
    fn world_position(&self, v: &Voice) -> Vec3 {
        if !v.relative {
            return v.geometry.position;
        }
        let l = &self.listener;
        let (Some(forward), Some(up), Ok(right)) = (unit(l.forward), unit(l.up), positional_audio::right_vector(l)) else {
            return v.geometry.position;
        };
        let p = v.geometry.position;
        [
            l.position[0] + right[0] * p[0] + up[0] * p[1] - forward[0] * p[2],
            l.position[1] + right[1] * p[0] + up[1] * p[1] - forward[1] * p[2],
            l.position[2] + right[2] * p[0] + up[2] * p[1] - forward[2] * p[2],
        ]
    }

    /// The two channel gains of a source for this block.
    fn channel_gains(&self, v: &Voice, channels: u8) -> Result<(f32, f32), SceneError> {
        let level = v.gain * self.master_gain;
        if channels == 2 {
            return Ok((level, level));
        }
        let placed = Source { position: self.world_position(v), ..v.geometry };
        let pan = positional_audio::pan(&self.listener, &placed)?;
        Ok((pan.left * level, pan.right * level))
    }

    /// Renders the next `frames` frames: interleaved left and right samples, summed over the playing sources and
    /// clipped to -1..1. Sources that reach the end of a non-looping buffer stop.
    pub fn mix(&mut self, frames: usize) -> Result<Vec<f32>, SceneError> {
        let mut out = vec![0.0f32; frames * 2];
        for index in 0..self.voices.len() {
            let Some(v) = self.voices[index].as_ref() else { continue };
            if v.state != SourceState::Playing {
                continue;
            }
            let Some(buffer) = v.buffer.and_then(|b| self.buffer(b)) else { continue };
            let (lg, rg) = self.channel_gains(v, buffer.channels)?;
            let step = f64::from(buffer.sample_rate) / f64::from(self.rate) * f64::from(v.pitch);
            let count = buffer.frames();
            let looping = v.looping;
            let mut at = v.at;
            let mut finished = count == 0;
            if !finished {
                for frame in 0..frames {
                    let (l, r) = frame_at(buffer, at, looping);
                    out[2 * frame] += l * lg;
                    out[2 * frame + 1] += r * rg;
                    at += step;
                    if at >= count as f64 {
                        if looping {
                            at %= count as f64;
                        } else {
                            finished = true;
                            break;
                        }
                    }
                }
            }
            let voice = self.voices[index].as_mut().expect("the voice was just read");
            if finished {
                voice.state = SourceState::Stopped;
                voice.at = 0.0;
            } else {
                voice.at = at;
            }
        }
        for s in &mut out {
            *s = s.clamp(-1.0, 1.0);
        }
        Ok(out)
    }
}

/// The interpolated frame at a (fractional) frame position. A mono buffer gives the same sample on both sides.
fn frame_at(buffer: &Buffer, at: f64, looping: bool) -> (f32, f32) {
    let n = buffer.frames();
    let i = (at.floor() as usize).min(n - 1);
    let frac = (at - i as f64) as f32;
    let j = if i + 1 < n {
        i + 1
    } else if looping {
        0
    } else {
        i
    };
    let get = |k: usize| -> (f32, f32) {
        if buffer.channels == 1 {
            (buffer.samples[k], buffer.samples[k])
        } else {
            (buffer.samples[2 * k], buffer.samples[2 * k + 1])
        }
    };
    let (a0, a1) = get(i);
    let (b0, b1) = get(j);
    (a0 + (b0 - a0) * frac, a1 + (b1 - a1) * frac)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HALF_POWER: f32 = std::f32::consts::FRAC_1_SQRT_2;

    fn scene_with(samples: Vec<f32>) -> (Scene, SourceId) {
        let mut scene = Scene::new(48_000).unwrap();
        let buffer = scene.create_buffer(48_000, 1, samples).unwrap();
        let source = scene.create_source();
        scene.set_buffer(source, buffer).unwrap();
        (scene, source)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn a_source_straight_ahead_plays_equally_in_both_ears() {
        let (mut scene, source) = scene_with(vec![1.0; 16]);
        scene.set_position(source, [0.0, 0.0, -1.0]).unwrap();
        scene.play(source).unwrap();
        let out = scene.mix(4).unwrap();
        assert!(close(out[0], HALF_POWER) && close(out[1], HALF_POWER));
    }

    #[test]
    fn a_source_to_the_right_is_heard_only_in_the_right_ear() {
        let (mut scene, source) = scene_with(vec![1.0; 16]);
        scene.set_position(source, [1.0, 0.0, 0.0]).unwrap();
        scene.play(source).unwrap();
        let out = scene.mix(2).unwrap();
        assert!(close(out[0], 0.0));
        assert!(close(out[1], 1.0));
        // and to the left only in the left
        scene.set_position(source, [-1.0, 0.0, 0.0]).unwrap();
        let out = scene.mix(2).unwrap();
        assert!(close(out[0], 1.0) && close(out[1], 0.0));
    }

    #[test]
    fn distance_attenuates_the_source() {
        let (mut scene, source) = scene_with(vec![1.0; 16]);
        scene.set_position(source, [2.0, 0.0, 0.0]).unwrap();
        scene.play(source).unwrap();
        let out = scene.mix(1).unwrap();
        // reference 1 m, rolloff 1: gain 1 / (1 + 1 * (2 - 1)) = 0.5
        assert!(close(out[1], 0.5));
        scene.set_distance_model(source, 1.0, 100.0, 0.0).unwrap();
        scene.rewind(source).unwrap();
        scene.play(source).unwrap();
        assert!(close(scene.mix(1).unwrap()[1], 1.0));
    }

    #[test]
    fn a_non_looping_source_stops_at_the_end_and_goes_quiet() {
        let (mut scene, source) = scene_with(vec![1.0; 4]);
        scene.set_position(source, [1.0, 0.0, 0.0]).unwrap();
        scene.play(source).unwrap();
        let out = scene.mix(8).unwrap();
        assert!(close(out[1], 1.0) && close(out[7], 1.0));
        assert!(close(out[9], 0.0) && close(out[15], 0.0));
        assert_eq!(scene.state(source).unwrap(), SourceState::Stopped);
    }

    #[test]
    fn a_looping_source_keeps_playing() {
        let (mut scene, source) = scene_with(vec![1.0; 4]);
        scene.set_position(source, [1.0, 0.0, 0.0]).unwrap();
        scene.set_looping(source, true).unwrap();
        scene.play(source).unwrap();
        let out = scene.mix(20).unwrap();
        assert!(close(out[39], 1.0));
        assert_eq!(scene.state(source).unwrap(), SourceState::Playing);
    }

    #[test]
    fn pause_holds_the_position_and_stop_rewinds() {
        let ramp: Vec<f32> = (0..8).map(|i| i as f32 / 8.0).collect();
        let (mut scene, source) = scene_with(ramp);
        scene.set_position(source, [1.0, 0.0, 0.0]).unwrap();
        scene.play(source).unwrap();
        scene.mix(3).unwrap();
        scene.pause(source).unwrap();
        assert_eq!(scene.state(source).unwrap(), SourceState::Paused);
        let silent = scene.mix(2).unwrap();
        assert!(silent.iter().all(|&s| s == 0.0));
        assert!(close(scene.position_seconds(source).unwrap() as f32, 3.0 / 48_000.0));
        scene.play(source).unwrap();
        let resumed = scene.mix(1).unwrap();
        assert!(close(resumed[1], 3.0 / 8.0));
        scene.stop(source).unwrap();
        assert_eq!(scene.state(source).unwrap(), SourceState::Stopped);
        assert_eq!(scene.position_seconds(source).unwrap(), 0.0);
    }

    #[test]
    fn pitch_two_plays_twice_as_fast() {
        let ramp: Vec<f32> = (0..8).map(|i| i as f32).collect();
        let (mut scene, source) = scene_with(ramp);
        scene.set_position(source, [1.0, 0.0, 0.0]).unwrap();
        scene.set_distance_model(source, 1.0, 100.0, 0.0).unwrap();
        scene.set_pitch(source, 2.0).unwrap();
        scene.play(source).unwrap();
        let out = scene.mix(4).unwrap();
        // frames 0, 2, 4, 6 of the ramp, clipped to 1
        assert!(close(out[1], 0.0) && close(out[3], 1.0));
        assert_eq!(scene.state(source).unwrap(), SourceState::Stopped);
    }

    #[test]
    fn a_buffer_at_half_the_output_rate_is_stretched_with_interpolation() {
        let mut scene = Scene::new(48_000).unwrap();
        let buffer = scene.create_buffer(24_000, 1, vec![0.0, 0.5, 1.0, 0.5]).unwrap();
        let source = scene.create_source();
        scene.set_buffer(source, buffer).unwrap();
        scene.set_position(source, [1.0, 0.0, 0.0]).unwrap();
        scene.set_distance_model(source, 1.0, 100.0, 0.0).unwrap();
        scene.play(source).unwrap();
        let out = scene.mix(4).unwrap();
        let right: Vec<f32> = out.chunks(2).map(|c| c[1]).collect();
        assert!(close(right[0], 0.0) && close(right[1], 0.25) && close(right[2], 0.5) && close(right[3], 0.75));
    }

    #[test]
    fn a_stereo_buffer_is_not_positioned() {
        let mut scene = Scene::new(48_000).unwrap();
        let buffer = scene.create_buffer(48_000, 2, vec![1.0, 0.25, 1.0, 0.25]).unwrap();
        let source = scene.create_source();
        scene.set_buffer(source, buffer).unwrap();
        scene.set_position(source, [50.0, 0.0, 0.0]).unwrap();
        scene.set_gain(source, 0.5).unwrap();
        scene.play(source).unwrap();
        let out = scene.mix(2).unwrap();
        assert!(close(out[0], 0.5) && close(out[1], 0.125));
    }

    #[test]
    fn the_master_gain_and_the_sum_of_sources_are_applied_and_clipped() {
        let mut scene = Scene::new(48_000).unwrap();
        let buffer = scene.create_buffer(48_000, 1, vec![0.8; 8]).unwrap();
        let a = scene.create_source();
        let b = scene.create_source();
        for s in [a, b] {
            scene.set_buffer(s, buffer).unwrap();
            scene.set_position(s, [1.0, 0.0, 0.0]).unwrap();
            scene.play(s).unwrap();
        }
        let out = scene.mix(1).unwrap();
        assert!(close(out[1], 1.0));
        scene.set_master_gain(0.25).unwrap();
        scene.rewind(a).unwrap();
        scene.rewind(b).unwrap();
        scene.play(a).unwrap();
        scene.play(b).unwrap();
        let quiet = scene.mix(1).unwrap();
        assert!(close(quiet[1], 0.8 * 0.25 * 2.0));
    }

    #[test]
    fn a_relative_source_turns_with_the_listener() {
        let (mut scene, source) = scene_with(vec![1.0; 8]);
        // the listener faces +x, so its right side is +z
        scene.set_listener(Listener { position: [0.0; 3], forward: [1.0, 0.0, 0.0], up: [0.0, 1.0, 0.0] }).unwrap();
        scene.set_relative(source, true).unwrap();
        scene.set_position(source, [1.0, 0.0, 0.0]).unwrap();
        scene.play(source).unwrap();
        let out = scene.mix(1).unwrap();
        assert!(close(out[0], 0.0) && close(out[1], 1.0));
    }

    #[test]
    fn bad_calls_are_refused() {
        let mut scene = Scene::new(48_000).unwrap();
        assert!(Scene::new(0).is_err());
        assert!(scene.create_buffer(0, 1, vec![]).is_err());
        assert!(scene.create_buffer(48_000, 3, vec![0.0; 3]).is_err());
        assert!(scene.create_buffer(48_000, 2, vec![0.0; 3]).is_err());
        let source = scene.create_source();
        assert_eq!(scene.play(source), Err(SceneError::InvalidOperation));
        assert_eq!(scene.set_gain(source, -1.0), Err(SceneError::InvalidValue));
        assert_eq!(scene.set_pitch(source, 0.0), Err(SceneError::InvalidValue));
        assert_eq!(scene.set_position(source, [f32::NAN, 0.0, 0.0]), Err(SceneError::InvalidValue));
        assert!(scene.set_distance_model(source, 0.0, 10.0, 1.0).is_err());
        let bad = Listener { position: [0.0; 3], forward: [0.0, 1.0, 0.0], up: [0.0, 1.0, 0.0] };
        assert!(matches!(scene.set_listener(bad), Err(SceneError::Position(_))));
        assert_eq!(scene.state(SourceId(99)), Err(SceneError::InvalidName));
    }

    #[test]
    fn a_buffer_in_use_cannot_be_deleted_until_its_source_is_gone() {
        let mut scene = Scene::new(48_000).unwrap();
        let buffer = scene.create_buffer(48_000, 1, vec![0.0; 4]).unwrap();
        let source = scene.create_source();
        scene.set_buffer(source, buffer).unwrap();
        assert_eq!(scene.delete_buffer(buffer), Err(SceneError::InvalidOperation));
        scene.delete_source(source).unwrap();
        scene.delete_buffer(buffer).unwrap();
        assert!(scene.buffer(buffer).is_none());
        assert_eq!(scene.delete_buffer(buffer), Err(SceneError::InvalidName));
    }

    #[test]
    fn an_empty_buffer_stops_at_once() {
        let (mut scene, source) = scene_with(vec![]);
        scene.play(source).unwrap();
        let out = scene.mix(4).unwrap();
        assert!(out.iter().all(|&s| s == 0.0));
        assert_eq!(scene.state(source).unwrap(), SourceState::Stopped);
    }
}
