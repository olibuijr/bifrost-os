//! MIDI file playback through the audio engine (`MidiPlayer`): a Standard MIDI File, or any timed sequence, plays
//! through an instrument unit (the built-in General MIDI synth, or any `AudioUnit`: a plug-in loaded from a library,
//! say) with the sequencing done sample-accurately inside the render callback.
//!
//! ```ignore
//! let mut player = MidiPlayer::from_bytes(&std::fs::read("song.mid")?, 48_000.0)?;
//! player.control().set_loop(Some((0.0, 8.0)));
//! player.control().play();
//! // offline: the whole file as a 16-bit WAV
//! let wav = player.render_to_wav(1.0)?;
//! // or on a device: player.engine_mut().start_on_device(&device, 0)?;
//! ```
//!
//! The transport is the `SequencerHandle` from `control()`: play, pause, stop, seek, loop region, rate and the
//! position, from any thread. `render` pulls audio by hand (a block at a time, or the whole file), and
//! `engine_mut` reaches the engine for everything else: node gains, taps, effects after the instrument, running on a
//! device. A sound bank is not needed: the built-in synth is additive (see `GmSynth`), simpler than sampled
//! instruments.

use bifrost_audio_engine::units::{AudioUnit, GmSynth, SeqEvent, Sequence, SequencerHandle, SequencerUnit};
use bifrost_audio_engine::{Engine, EngineConfig, EngineError, OUTPUT};
use bifrost_core::smf::{Message, Smf, SmfError};
use std::fmt;

/// Why a player could not be made or run.
#[derive(Debug, PartialEq)]
pub enum MidiPlayerError {
    /// The file is not a usable Standard MIDI File.
    File(SmfError),
    Engine(EngineError),
}

impl fmt::Display for MidiPlayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MidiPlayerError::File(e) => write!(f, "{e}"),
            MidiPlayerError::Engine(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MidiPlayerError {}

impl From<SmfError> for MidiPlayerError {
    fn from(e: SmfError) -> MidiPlayerError {
        MidiPlayerError::File(e)
    }
}

impl From<EngineError> for MidiPlayerError {
    fn from(e: EngineError) -> MidiPlayerError {
        MidiPlayerError::Engine(e)
    }
}

/// The sequence a parsed file describes: every channel message at its time in seconds.
pub fn sequence_from_smf(smf: &Smf) -> Sequence {
    let events = smf
        .events()
        .iter()
        .map(|e| {
            let (status, data1, data2) = match e.message {
                Message::NoteOff { channel, key, velocity } => (0x80 | channel, key, velocity),
                Message::NoteOn { channel, key, velocity } => (0x90 | channel, key, velocity),
                Message::PolyPressure { channel, key, pressure } => (0xA0 | channel, key, pressure),
                Message::ControlChange { channel, controller, value } => (0xB0 | channel, controller, value),
                Message::ProgramChange { channel, program } => (0xC0 | channel, program, 0),
                Message::ChannelPressure { channel, pressure } => (0xD0 | channel, pressure, 0),
                Message::PitchBend { channel, value } => (0xE0 | channel, (value & 127) as u8, ((value >> 7) & 127) as u8),
            };
            SeqEvent { seconds: e.seconds, status: status & 0xFF, data1: data1 & 127, data2: data2 & 127 }
        })
        .collect();
    Sequence::new(events)
}

/// 16-bit PCM WAV bytes for planar float channels (clipped to full scale, interleaved).
pub fn wav_bytes_from_planar(sample_rate: u32, planes: &[Vec<f32>]) -> Vec<u8> {
    let channels = planes.len().max(1) as u16;
    let frames = planes.first().map_or(0, |p| p.len());
    let data_len = (frames * channels as usize * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        for p in planes {
            let s = (p[i].clamp(-1.0, 1.0) * 32767.0).round() as i16;
            out.extend_from_slice(&s.to_le_bytes());
        }
    }
    out
}

/// A sequence playing through an instrument in an engine.
pub struct MidiPlayer {
    engine: Engine,
    ctl: SequencerHandle,
    duration: f64,
    sample_rate: f64,
    channels: usize,
}

impl MidiPlayer {
    /// Plays a Standard MIDI File through the built-in synth, in stereo at `sample_rate`.
    pub fn from_bytes(bytes: &[u8], sample_rate: f64) -> Result<MidiPlayer, MidiPlayerError> {
        let smf = Smf::parse(bytes)?;
        MidiPlayer::from_smf(&smf, sample_rate)
    }

    pub fn from_smf(smf: &Smf, sample_rate: f64) -> Result<MidiPlayer, MidiPlayerError> {
        MidiPlayer::with_instrument(sequence_from_smf(smf), Box::new(GmSynth::new()), sample_rate)
    }

    /// Plays `sequence` through any instrument unit. The unit must make stereo (`max_channels` 2 or more).
    pub fn with_instrument(sequence: Sequence, instrument: Box<dyn AudioUnit>, sample_rate: f64) -> Result<MidiPlayer, MidiPlayerError> {
        let duration = sequence.length_seconds();
        let channels = 2;
        let mut engine = Engine::new(EngineConfig { sample_rate, channels: channels as u32, max_frames: 512 })?;
        let (unit, ctl) = SequencerUnit::new(instrument, sequence);
        let (node, _params, _midi) = engine.attach_unit(Box::new(unit))?;
        engine.connect(node, OUTPUT)?;
        engine.prepare()?;
        Ok(MidiPlayer { engine, ctl, duration, sample_rate, channels })
    }

    /// The transport: play, pause, stop, seek, loop region, rate, position.
    pub fn control(&self) -> &SequencerHandle {
        &self.ctl
    }

    /// The engine, for node gains, taps, effects and running on a device.
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// The time of the last message, in seconds.
    pub fn duration_seconds(&self) -> f64 {
        self.duration
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Renders `frames` frames of whatever the transport does (silence while paused), one planar channel each.
    pub fn render(&mut self, frames: usize) -> Result<Vec<Vec<f32>>, MidiPlayerError> {
        Ok(self.engine.render_offline(frames)?)
    }

    /// Plays from the start through the end of the sequence plus `tail_seconds` (the release of the last notes) and
    /// returns the result as a 16-bit stereo WAV file.
    pub fn render_to_wav(&mut self, tail_seconds: f64) -> Result<Vec<u8>, MidiPlayerError> {
        self.ctl.stop();
        self.ctl.play();
        let seconds = self.duration + tail_seconds.max(0.0);
        let frames = (seconds * self.sample_rate).ceil() as usize + 1;
        let planes = self.render(frames)?;
        self.ctl.pause();
        debug_assert_eq!(planes.len(), self.channels);
        Ok(wav_bytes_from_planar(self.sample_rate as u32, &planes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A format 0 file, 480 ticks per quarter, default tempo (120 bpm): middle C for one beat, then E for one beat.
    fn smf_bytes() -> Vec<u8> {
        let mut track = Vec::new();
        track.extend_from_slice(&[0x00, 0x90, 60, 100]); // note on at tick 0
        track.extend_from_slice(&[0x83, 0x60, 0x80, 60, 0]); // 480 ticks later: note off (0.5 s)
        track.extend_from_slice(&[0x00, 0x90, 64, 100]); // note on E at 0.5 s
        track.extend_from_slice(&[0x83, 0x60, 0x80, 64, 0]); // note off at 1.0 s
        track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]); // end of track
        let mut file = Vec::new();
        file.extend_from_slice(b"MThd");
        file.extend_from_slice(&6u32.to_be_bytes());
        file.extend_from_slice(&0u16.to_be_bytes());
        file.extend_from_slice(&1u16.to_be_bytes());
        file.extend_from_slice(&480u16.to_be_bytes());
        file.extend_from_slice(b"MTrk");
        file.extend_from_slice(&(track.len() as u32).to_be_bytes());
        file.extend_from_slice(&track);
        file
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn a_midi_file_becomes_a_timed_sequence() {
        let smf = Smf::parse(&smf_bytes()).unwrap();
        let seq = sequence_from_smf(&smf);
        let t: Vec<(f64, u8, u8)> = seq.events().iter().map(|e| (e.seconds, e.status, e.data1)).collect();
        assert_eq!(t, vec![(0.0, 0x90, 60), (0.5, 0x80, 60), (0.5, 0x90, 64), (1.0, 0x80, 64)]);
    }

    #[test]
    fn every_message_kind_maps_to_its_wire_bytes() {
        use bifrost_core::smf::Message as M;
        // reach each mapping through the public function with a hand-built sequence of messages
        let msgs = [
            (M::PolyPressure { channel: 3, key: 60, pressure: 9 }, (0xA3, 60, 9)),
            (M::ControlChange { channel: 1, controller: 7, value: 90 }, (0xB1, 7, 90)),
            (M::ProgramChange { channel: 2, program: 40 }, (0xC2, 40, 0)),
            (M::ChannelPressure { channel: 4, pressure: 33 }, (0xD4, 33, 0)),
            (M::PitchBend { channel: 5, value: 8192 + 100 }, (0xE5, (8292 & 127) as u8, (8292 >> 7) as u8)),
        ];
        // an SMF carrying them: delta 0 each
        let mut track = Vec::new();
        for (m, _) in &msgs {
            match *m {
                M::PolyPressure { channel, key, pressure } => track.extend_from_slice(&[0, 0xA0 | channel, key, pressure]),
                M::ControlChange { channel, controller, value } => track.extend_from_slice(&[0, 0xB0 | channel, controller, value]),
                M::ProgramChange { channel, program } => track.extend_from_slice(&[0, 0xC0 | channel, program]),
                M::ChannelPressure { channel, pressure } => track.extend_from_slice(&[0, 0xD0 | channel, pressure]),
                M::PitchBend { channel, value } => track.extend_from_slice(&[0, 0xE0 | channel, (value & 127) as u8, (value >> 7) as u8]),
                _ => unreachable!(),
            }
        }
        track.extend_from_slice(&[0, 0xFF, 0x2F, 0]);
        let mut file = b"MThd".to_vec();
        file.extend_from_slice(&6u32.to_be_bytes());
        file.extend_from_slice(&[0, 0, 0, 1, 0x01, 0xE0]);
        file.extend_from_slice(b"MTrk");
        file.extend_from_slice(&(track.len() as u32).to_be_bytes());
        file.extend_from_slice(&track);
        let seq = sequence_from_smf(&Smf::parse(&file).unwrap());
        let got: Vec<(u8, u8, u8)> = seq.events().iter().map(|e| (e.status, e.data1, e.data2)).collect();
        let want: Vec<(u8, u8, u8)> = msgs.iter().map(|(_, w)| *w).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn the_file_plays_through_the_engine_with_the_notes_where_the_file_says() {
        let mut p = MidiPlayer::from_bytes(&smf_bytes(), 48_000.0).unwrap();
        assert_eq!(p.duration_seconds(), 1.0);
        // paused: silence
        let silent = p.render(4800).unwrap();
        assert!(silent.iter().all(|c| c.iter().all(|s| *s == 0.0)));
        p.control().play();
        let out = p.render(48_000 + 12_000).unwrap();
        assert_eq!(out.len(), 2);
        // the first beat sounds, the second beat sounds, and the release has died by 1.17 s
        assert!(rms(&out[0][2_400..22_000]) > 0.005);
        assert!(rms(&out[0][26_000..46_000]) > 0.005);
        assert!(out[0][56_000..].iter().all(|s| s.abs() < 1e-4));
        assert!(p.control().is_finished());
    }

    #[test]
    fn the_two_beats_have_different_pitches() {
        let mut p = MidiPlayer::from_bytes(&smf_bytes(), 48_000.0).unwrap();
        p.control().play();
        let out = p.render(46_000).unwrap();
        let cycles = |x: &[f32]| x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as f32;
        // C4 at 261.6 Hz over 0.35 s against E4 at 329.6 Hz over 0.35 s
        let c = cycles(&out[2_400..19_200]);
        let e = cycles(&out[26_400..43_200]);
        assert!((e / c - 329.63 / 261.63).abs() < 0.06, "{c} {e}");
    }

    #[test]
    fn render_to_wav_is_a_complete_stereo_file_of_the_right_length() {
        let mut p = MidiPlayer::from_bytes(&smf_bytes(), 48_000.0).unwrap();
        let wav = p.render_to_wav(0.5).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 2);
        assert_eq!(u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]), 48_000);
        let data = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as usize;
        assert_eq!(data, wav.len() - 44);
        // 1.5 s plus one frame, two channels of two bytes
        assert_eq!(data, (72_000 + 1) * 4);
        assert!(wav[44..].iter().any(|b| *b != 0));
    }

    #[test]
    fn seek_and_loop_work_through_the_player() {
        let mut p = MidiPlayer::from_bytes(&smf_bytes(), 48_000.0).unwrap();
        p.control().seek(0.5);
        p.control().play();
        let out = p.render(4_800).unwrap();
        // starting at 0.5 s the E sounds at once
        assert!(rms(&out[0][1_000..4_800]) > 0.005);
        p.control().stop();
        p.control().set_loop(Some((0.0, 0.5)));
        p.control().play();
        let _ = p.render(96_000).unwrap();
        assert!(!p.control().is_finished());
        assert!(p.control().is_playing());
    }

    #[test]
    fn a_bad_file_is_a_file_error() {
        assert!(matches!(MidiPlayer::from_bytes(b"not midi at all", 48_000.0), Err(MidiPlayerError::File(SmfError::NotMidi))));
        assert!(matches!(
            MidiPlayer::with_instrument(Sequence::new(vec![]), Box::new(GmSynth::new()), 100.0),
            Err(MidiPlayerError::Engine(EngineError::Config))
        ));
    }

    #[test]
    fn wav_clips_and_interleaves() {
        let wav = wav_bytes_from_planar(8_000, &[vec![0.0, 2.0, -2.0], vec![0.5, 0.0, 0.0]]);
        let s = |i: usize| i16::from_le_bytes([wav[44 + i * 2], wav[45 + i * 2]]);
        assert_eq!((s(0), s(1), s(2), s(3), s(4), s(5)), (0, 16384, 32767, 0, -32767, 0));
    }
}
