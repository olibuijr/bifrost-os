//! Audio recorder model (`AVAudioRecorder` parity): record, pause, scheduled start, timed recording, format settings
//! and per-channel metering, producing a 16-bit linear PCM WAV file.
//!
//! The recorder is fed capture blocks with [`AudioRecorder::push`]. Its device clock advances with the input frames, so
//! `record(atTime:)` starts at a point in that clock and `record(forDuration:)` stops after the given recorded time.
//! Input is converted to the configured channel count and sample rate with `pcm_convert`. No device is opened here.

use crate::pcm_convert::{f32_to_s16, remap_channels, ConvertError, Resampler};

/// Metering floor in decibels, reported for silence.
pub const SILENCE_DB: f32 = -160.0;

/// `AVFormatIDKey` value for linear PCM (the four characters `lpcm`).
pub const FORMAT_LINEAR_PCM: u32 = 0x6C70_636D;

/// Why a call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecorderError {
    /// The settings name a format other than linear PCM.
    UnsupportedFormat,
    /// The sample rate or channel count is zero.
    InvalidSettings,
    /// The capture block does not hold whole frames.
    Ragged,
    /// The channel index is past the recorded channel count.
    NoSuchChannel,
}

/// The recorder settings dictionary: `AVFormatIDKey`, `AVSampleRateKey` and `AVNumberOfChannelsKey`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub format_id: u32,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Settings {
    /// Linear PCM at the given rate and channel count.
    pub fn linear_pcm(sample_rate: u32, channels: u16) -> Self {
        Settings { format_id: FORMAT_LINEAR_PCM, sample_rate, channels }
    }

    fn validate(&self) -> Result<(), RecorderError> {
        if self.format_id != FORMAT_LINEAR_PCM {
            return Err(RecorderError::UnsupportedFormat);
        }
        if self.sample_rate == 0 || self.channels == 0 {
            return Err(RecorderError::InvalidSettings);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Stopped,
    Recording,
    Paused,
}

/// A recorder over interleaved float capture blocks.
pub struct AudioRecorder {
    settings: Settings,
    input_rate: u32,
    input_channels: u16,
    resampler: Option<Resampler>,
    state: State,
    /// Device clock in seconds: the total input time pushed so far.
    device_time: f64,
    /// Start of a scheduled recording, in device seconds.
    start_at: Option<f64>,
    /// Recorded time at which recording stops by itself.
    stop_after: Option<f64>,
    samples: Vec<i16>,
    metering: bool,
    average: Vec<f32>,
    peak: Vec<f32>,
}

impl AudioRecorder {
    /// A recorder for capture blocks of `input_channels` at `input_rate`, writing `settings`.
    pub fn new(settings: Settings, input_rate: u32, input_channels: u16) -> Result<Self, RecorderError> {
        settings.validate()?;
        if input_rate == 0 || input_channels == 0 {
            return Err(RecorderError::InvalidSettings);
        }
        let resampler = if input_rate == settings.sample_rate {
            None
        } else {
            Some(Resampler::new(input_rate, settings.sample_rate, settings.channels).map_err(convert_error)?)
        };
        let n = settings.channels as usize;
        Ok(AudioRecorder {
            settings,
            input_rate,
            input_channels,
            resampler,
            state: State::Stopped,
            device_time: 0.0,
            start_at: None,
            stop_after: None,
            samples: Vec::new(),
            metering: false,
            average: vec![SILENCE_DB; n],
            peak: vec![SILENCE_DB; n],
        })
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// `record()`: starts, or resumes after a pause. Clears any scheduled start and any duration limit.
    pub fn record(&mut self) {
        self.start_at = None;
        self.stop_after = None;
        self.state = State::Recording;
    }

    /// `record(atTime:)`: starts when the device clock reaches `time` seconds. A time already passed starts now.
    pub fn record_at(&mut self, time: f64) {
        self.record();
        if time > self.device_time {
            self.start_at = Some(time);
        }
    }

    /// `record(forDuration:)`: records, and stops once `seconds` of audio have been recorded in total.
    pub fn record_for(&mut self, seconds: f64) {
        self.record();
        self.stop_after = Some(seconds.max(0.0));
        self.stop_if_done();
    }

    /// `pause()`. Captured blocks are dropped until `record()` is called again; the file keeps what it has.
    pub fn pause(&mut self) {
        if self.state == State::Recording {
            self.state = State::Paused;
            self.start_at = None;
        }
    }

    /// `stop()`.
    pub fn stop(&mut self) {
        self.state = State::Stopped;
        self.start_at = None;
    }

    /// `isRecording`: true while recording, including while waiting for a scheduled start.
    pub fn is_recording(&self) -> bool {
        self.state == State::Recording
    }

    /// `currentTime`: seconds recorded so far.
    pub fn current_time(&self) -> f64 {
        self.recorded_frames() as f64 / self.settings.sample_rate as f64
    }

    /// The device clock in seconds (`deviceCurrentTime`).
    pub fn device_current_time(&self) -> f64 {
        self.device_time
    }

    /// `isMeteringEnabled`.
    pub fn is_metering_enabled(&self) -> bool {
        self.metering
    }

    pub fn set_metering_enabled(&mut self, on: bool) {
        self.metering = on;
        if !on {
            self.average.iter_mut().for_each(|v| *v = SILENCE_DB);
            self.peak.iter_mut().for_each(|v| *v = SILENCE_DB);
        }
    }

    /// `averagePower(forChannel:)`: RMS level of the latest block in decibels, 0 at full scale.
    pub fn average_power(&self, channel: usize) -> Result<f32, RecorderError> {
        self.average.get(channel).copied().ok_or(RecorderError::NoSuchChannel)
    }

    /// `peakPower(forChannel:)`: peak level of the latest block in decibels, 0 at full scale.
    pub fn peak_power(&self, channel: usize) -> Result<f32, RecorderError> {
        self.peak.get(channel).copied().ok_or(RecorderError::NoSuchChannel)
    }

    /// Feeds one interleaved capture block. Time passes on the device clock whatever the state; the block is kept
    /// only while recording and after the scheduled start.
    pub fn push(&mut self, block: &[f32]) -> Result<(), RecorderError> {
        let ch_in = self.input_channels as usize;
        if block.len() % ch_in != 0 {
            return Err(RecorderError::Ragged);
        }
        let frames = block.len() / ch_in;
        let block_start = self.device_time;
        self.device_time += frames as f64 / self.input_rate as f64;
        if self.state != State::Recording {
            return Ok(());
        }
        let skip = match self.start_at {
            Some(at) if at >= self.device_time => return Ok(()),
            Some(at) => (((at - block_start).max(0.0)) * self.input_rate as f64).round() as usize,
            None => 0,
        };
        if self.start_at.is_some() {
            self.start_at = None;
        }
        let usable = &block[skip.min(frames) * ch_in..];
        let mut converted = remap_channels(usable, self.input_channels, self.settings.channels).map_err(convert_error)?;
        if let Some(r) = self.resampler.as_mut() {
            converted = r.process(&converted).map_err(convert_error)?;
        }
        self.truncate_to_limit(&mut converted);
        if self.metering {
            self.update_meters(&converted);
        }
        self.samples.extend(converted.iter().map(|&s| f32_to_s16(s)));
        self.stop_if_done();
        Ok(())
    }

    fn recorded_frames(&self) -> usize {
        self.samples.len() / self.settings.channels as usize
    }

    fn truncate_to_limit(&self, converted: &mut Vec<f32>) {
        if let Some(limit) = self.stop_after {
            let ch = self.settings.channels as usize;
            let max_frames = (limit * self.settings.sample_rate as f64).round() as usize;
            let room = max_frames.saturating_sub(self.recorded_frames());
            converted.truncate(room * ch);
        }
    }

    fn stop_if_done(&mut self) {
        if let Some(limit) = self.stop_after {
            let max_frames = (limit * self.settings.sample_rate as f64).round() as usize;
            if self.state == State::Recording && self.recorded_frames() >= max_frames {
                self.state = State::Stopped;
            }
        }
    }

    fn update_meters(&mut self, converted: &[f32]) {
        let ch = self.settings.channels as usize;
        let frames = converted.len() / ch;
        if frames == 0 {
            return;
        }
        for c in 0..ch {
            let (mut peak, mut sum) = (0.0_f32, 0.0_f64);
            for f in 0..frames {
                let v = converted[f * ch + c];
                peak = peak.max(v.abs());
                sum += (v as f64) * (v as f64);
            }
            self.peak[c] = to_db(peak);
            self.average[c] = to_db((sum / frames as f64).sqrt() as f32);
        }
    }

    /// The recorded audio as a complete WAV file (16-bit PCM). Valid at any time, including while paused.
    pub fn wav_bytes(&self) -> Vec<u8> {
        let data_len = (self.samples.len() * 2) as u32;
        let channels = self.settings.channels;
        let rate = self.settings.sample_rate;
        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
        out.extend_from_slice(&(channels * 2).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for s in &self.samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }
}

fn to_db(level: f32) -> f32 {
    if level <= 0.0 {
        SILENCE_DB
    } else {
        (20.0 * level.log10()).max(SILENCE_DB)
    }
}

fn convert_error(e: ConvertError) -> RecorderError {
    match e {
        ConvertError::Ragged => RecorderError::Ragged,
        ConvertError::ZeroRate | ConvertError::ZeroChannels => RecorderError::InvalidSettings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(rate: u32, channels: u16) -> AudioRecorder {
        AudioRecorder::new(Settings::linear_pcm(rate, channels), rate, channels).unwrap()
    }

    fn u32_at(b: &[u8], i: usize) -> u32 {
        u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
    }

    #[test]
    fn only_linear_pcm_with_real_rate_and_channels_is_accepted() {
        let bad_format = Settings { format_id: 1, sample_rate: 8000, channels: 1 };
        assert_eq!(AudioRecorder::new(bad_format, 8000, 1).err(), Some(RecorderError::UnsupportedFormat));
        assert_eq!(AudioRecorder::new(Settings::linear_pcm(0, 1), 8000, 1).err(), Some(RecorderError::InvalidSettings));
        assert_eq!(AudioRecorder::new(Settings::linear_pcm(8000, 0), 8000, 1).err(), Some(RecorderError::InvalidSettings));
    }

    #[test]
    fn blocks_are_dropped_until_record_is_called() {
        let mut r = rec(1000, 1);
        r.push(&[0.5; 100]).unwrap();
        assert_eq!(r.current_time(), 0.0);
        r.record();
        r.push(&[0.5; 100]).unwrap();
        assert!((r.current_time() - 0.1).abs() < 1e-9);
        assert!((r.device_current_time() - 0.2).abs() < 1e-9);
    }

    #[test]
    fn pause_and_resume_make_one_continuous_wav_with_a_correct_header() {
        let mut r = rec(1000, 2);
        r.record();
        r.push(&[0.25; 200]).unwrap();
        r.pause();
        assert!(!r.is_recording());
        r.push(&[0.9; 200]).unwrap();
        r.record();
        r.push(&[-0.25; 200]).unwrap();
        r.stop();
        let wav = r.wav_bytes();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 2);
        assert_eq!(u32_at(&wav, 24), 1000);
        assert_eq!(u32_at(&wav, 28), 4000);
        assert_eq!(u32_at(&wav, 40), 800, "two recorded blocks of 100 stereo frames, 4 bytes each");
        assert_eq!(u32_at(&wav, 4), 36 + 800);
        assert_eq!(wav.len(), 44 + 800);
        let first = i16::from_le_bytes([wav[44], wav[45]]);
        let last = i16::from_le_bytes([wav[wav.len() - 2], wav[wav.len() - 1]]);
        assert!(first > 0 && last < 0, "the paused block is not in the file");
        assert!((r.current_time() - 0.2).abs() < 1e-9);
    }

    #[test]
    fn record_for_stops_on_time_and_keeps_exactly_that_much() {
        let mut r = rec(1000, 1);
        r.record_for(0.25);
        r.push(&[0.1; 200]).unwrap();
        assert!(r.is_recording());
        r.push(&[0.1; 200]).unwrap();
        assert!(!r.is_recording());
        assert!((r.current_time() - 0.25).abs() < 1e-9);
        r.push(&[0.1; 200]).unwrap();
        assert!((r.current_time() - 0.25).abs() < 1e-9, "nothing is kept after the stop");
    }

    #[test]
    fn record_for_zero_stops_at_once() {
        let mut r = rec(1000, 1);
        r.record_for(0.0);
        assert!(!r.is_recording());
    }

    #[test]
    fn record_at_starts_inside_the_block_that_holds_the_start_time() {
        let mut r = rec(1000, 1);
        r.record_at(0.15);
        r.push(&[0.5; 100]).unwrap();
        assert_eq!(r.current_time(), 0.0, "before the start time");
        assert!(r.is_recording(), "armed counts as recording");
        r.push(&[0.5; 100]).unwrap();
        assert!((r.current_time() - 0.05).abs() < 1e-9, "the second block starts at 0.1 and is cut at 0.15");
        r.push(&[0.5; 100]).unwrap();
        assert!((r.current_time() - 0.15).abs() < 1e-9);
    }

    #[test]
    fn a_past_start_time_records_immediately() {
        let mut r = rec(1000, 1);
        r.push(&[0.0; 500]).unwrap();
        r.record_at(0.1);
        r.push(&[0.5; 100]).unwrap();
        assert!((r.current_time() - 0.1).abs() < 1e-9);
    }

    #[test]
    fn mono_input_is_written_as_the_configured_stereo() {
        let mut r = AudioRecorder::new(Settings::linear_pcm(1000, 2), 1000, 1).unwrap();
        r.record();
        r.push(&[0.5; 10]).unwrap();
        let wav = r.wav_bytes();
        assert_eq!(u32_at(&wav, 40), 40);
        assert_eq!(wav[44..46], wav[46..48], "both channels carry the sample");
    }

    #[test]
    fn a_lower_output_rate_shortens_the_recording_in_proportion() {
        let mut r = AudioRecorder::new(Settings::linear_pcm(1000, 1), 4000, 1).unwrap();
        r.record();
        r.push(&[0.3; 4000]).unwrap();
        let t = r.current_time();
        assert!((t - 1.0).abs() < 0.01, "one second of input is about one second recorded, got {t}");
        assert_eq!(u32_at(&r.wav_bytes(), 24), 1000);
    }

    #[test]
    fn a_ragged_block_is_refused() {
        let mut r = rec(1000, 2);
        r.record();
        assert_eq!(r.push(&[0.0; 3]), Err(RecorderError::Ragged));
    }

    #[test]
    fn metering_reports_peak_and_average_per_channel() {
        let mut r = rec(1000, 2);
        r.set_metering_enabled(true);
        r.record();
        // Left is a constant 0.5, right is silent.
        let block: Vec<f32> = (0..100).flat_map(|_| [0.5, 0.0]).collect();
        r.push(&block).unwrap();
        let db_half = 20.0 * 0.5_f32.log10();
        assert!((r.peak_power(0).unwrap() - db_half).abs() < 1e-3);
        assert!((r.average_power(0).unwrap() - db_half).abs() < 1e-3);
        assert_eq!(r.peak_power(1).unwrap(), SILENCE_DB);
        assert_eq!(r.average_power(2), Err(RecorderError::NoSuchChannel));
    }

    #[test]
    fn disabling_metering_resets_the_levels() {
        let mut r = rec(1000, 1);
        r.set_metering_enabled(true);
        r.record();
        r.push(&[1.0; 10]).unwrap();
        assert!(r.peak_power(0).unwrap().abs() < 1e-6, "full scale is 0 dB");
        r.set_metering_enabled(false);
        assert_eq!(r.peak_power(0).unwrap(), SILENCE_DB);
    }
}
