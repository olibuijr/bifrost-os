//! Time-coded audio input for a speech analyzer. An input is a block of interleaved samples, the time its first frame
//! starts on the analysis timeline, and the format the samples are in. The converter turns any input into the mono,
//! fixed-rate format the analyzers read: channels are averaged, then the rate is changed by linear interpolation. The
//! start time is kept across conversion, so results can be placed on the original timeline. These are pure models; no
//! capture device or file is opened here.

use std::collections::VecDeque;

/// Sample layout of a block of audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioFormat {
    /// A zero sample rate or channel count is raised to 1.
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        AudioFormat {
            sample_rate: sample_rate.max(1),
            channels: channels.max(1),
        }
    }
}

/// The format the analyzers read: 16 kHz mono.
pub const ANALYSIS_FORMAT: AudioFormat = AudioFormat {
    sample_rate: 16_000,
    channels: 1,
};

/// A block of time-coded audio. Samples are interleaved f32 values in the range -1 to 1.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalyzerInput {
    start: f64,
    format: AudioFormat,
    samples: Vec<f32>,
}

impl AnalyzerInput {
    /// `start` is in seconds on the analysis timeline; negative and non-finite values become zero. Samples that do not
    /// fill a whole frame are dropped.
    pub fn new(start: f64, format: AudioFormat, samples: &[f32]) -> Self {
        let width = format.channels as usize;
        let whole = samples.len() / width * width;
        AnalyzerInput {
            start: if start.is_finite() { start.max(0.0) } else { 0.0 },
            format,
            samples: samples[..whole].to_vec(),
        }
    }

    /// Time of the first frame, in seconds.
    pub fn start(&self) -> f64 {
        self.start
    }

    pub fn format(&self) -> AudioFormat {
        self.format
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// Number of frames, one frame holding one sample per channel.
    pub fn frames(&self) -> usize {
        self.samples.len() / self.format.channels as usize
    }

    /// Length of the block in seconds.
    pub fn duration(&self) -> f64 {
        self.frames() as f64 / self.format.sample_rate as f64
    }

    /// Time of the end of the block, in seconds.
    pub fn end(&self) -> f64 {
        self.start + self.duration()
    }
}

/// Converts inputs to the analysis format, mono at one rate. The start time of each input is carried over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnalyzerInputConverter {
    target: AudioFormat,
}

impl AnalyzerInputConverter {
    /// Converts to `ANALYSIS_FORMAT`.
    pub fn new() -> Self {
        AnalyzerInputConverter {
            target: ANALYSIS_FORMAT,
        }
    }

    /// Converts to mono at the given rate. The channel count of `target` is ignored because analyzers read mono.
    pub fn with_target(target: AudioFormat) -> Self {
        AnalyzerInputConverter {
            target: AudioFormat::new(target.sample_rate, 1),
        }
    }

    pub fn target(&self) -> AudioFormat {
        self.target
    }

    pub fn convert(&self, input: &AnalyzerInput) -> AnalyzerInput {
        let mono = downmix(input.samples(), input.format().channels as usize);
        let samples = resample_linear(&mono, input.format().sample_rate, self.target.sample_rate);
        AnalyzerInput {
            start: input.start(),
            format: self.target,
            samples,
        }
    }
}

impl Default for AnalyzerInputConverter {
    fn default() -> Self {
        Self::new()
    }
}

/// A source of time-coded blocks for an analyzer, taken one at a time in timeline order.
pub trait InputSequence {
    /// The next block, or `None` when no block is ready (the end of a file, or an empty microphone queue).
    fn next_input(&mut self) -> Option<AnalyzerInput>;
}

/// The blocks of a block of samples that is already in memory (a decoded file), cut into blocks of a fixed number of
/// frames. The first block starts at `start`; each later block starts where the previous one ends. The last block may be
/// shorter.
#[derive(Clone, Debug, PartialEq)]
pub struct FileSequence {
    start: f64,
    format: AudioFormat,
    samples: Vec<f32>,
    block_frames: usize,
    next_frame: usize,
}

impl FileSequence {
    /// `None` when `block_frames` is zero. Samples that do not fill a whole frame are dropped.
    pub fn new(start: f64, format: AudioFormat, samples: &[f32], block_frames: usize) -> Option<Self> {
        if block_frames == 0 {
            return None;
        }
        let format = AudioFormat::new(format.sample_rate, format.channels);
        let width = format.channels as usize;
        let whole = samples.len() / width * width;
        Some(FileSequence {
            start: if start.is_finite() { start.max(0.0) } else { 0.0 },
            format,
            samples: samples[..whole].to_vec(),
            block_frames,
            next_frame: 0,
        })
    }

    /// The number of whole frames in the sequence.
    pub fn frame_count(&self) -> usize {
        self.samples.len() / self.format.channels as usize
    }
}

impl InputSequence for FileSequence {
    fn next_input(&mut self) -> Option<AnalyzerInput> {
        let total = self.frame_count();
        if self.next_frame >= total {
            return None;
        }
        let frames = self.block_frames.min(total - self.next_frame);
        let width = self.format.channels as usize;
        let from = self.next_frame * width;
        let start = self.start + self.next_frame as f64 / self.format.sample_rate as f64;
        self.next_frame += frames;
        Some(AnalyzerInput::new(start, self.format, &self.samples[from..from + frames * width]))
    }
}

/// The blocks a capture callback delivers, held in order until the analyzer takes them. The first block fixes the
/// format; a block in another format, a block that starts before the previous one, or an empty block is refused. The
/// queue holds at most its capacity, and a block pushed when it is full is refused and counted. No capture device is
/// opened here.
#[derive(Clone, Debug)]
pub struct MicrophoneSequence {
    capacity: usize,
    format: Option<AudioFormat>,
    last_start: Option<f64>,
    queue: VecDeque<AnalyzerInput>,
    refused: usize,
}

impl MicrophoneSequence {
    /// A queue that holds up to `capacity` blocks (at least one).
    pub fn new(capacity: usize) -> Self {
        MicrophoneSequence { capacity: capacity.max(1), format: None, last_start: None, queue: VecDeque::new(), refused: 0 }
    }

    /// Queues a block captured at `start` seconds on the timeline. `false` when the block is refused.
    pub fn push(&mut self, start: f64, format: AudioFormat, samples: &[f32]) -> bool {
        let block = AnalyzerInput::new(start, AudioFormat::new(format.sample_rate, format.channels), samples);
        let accepted = block.frames() > 0
            && self.format.is_none_or(|f| f == block.format())
            && self.last_start.is_none_or(|last| block.start() >= last)
            && self.queue.len() < self.capacity;
        if !accepted {
            self.refused += 1;
            return false;
        }
        self.format = Some(block.format());
        self.last_start = Some(block.start());
        self.queue.push_back(block);
        true
    }

    /// The number of blocks waiting to be taken.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// The number of blocks refused so far.
    pub fn refused(&self) -> usize {
        self.refused
    }
}

impl InputSequence for MicrophoneSequence {
    fn next_input(&mut self) -> Option<AnalyzerInput> {
        self.queue.pop_front()
    }
}

/// Averages the channels of each frame.
fn downmix(samples: &[f32], channels: usize) -> Vec<f32> {
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Changes the rate of a mono signal by linear interpolation. The last sample is held past the end.
fn resample_linear(src: &[f32], from: u32, to: u32) -> Vec<f32> {
    if src.is_empty() {
        return Vec::new();
    }
    if from == to {
        return src.to_vec();
    }
    let out_len = (src.len() as f64 * to as f64 / from as f64).round() as usize;
    let last = src.len() - 1;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * from as f64 / to as f64;
            let idx = pos.floor() as usize;
            let frac = (pos - idx as f64) as f32;
            let a = src[idx.min(last)];
            let b = src[(idx + 1).min(last)];
            a + (b - a) * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_is_the_start_plus_the_duration_of_the_frames() {
        let input = AnalyzerInput::new(2.0, ANALYSIS_FORMAT, &vec![0.0; 1600]);
        assert_eq!(input.frames(), 1600);
        assert!((input.duration() - 0.1).abs() < 1e-9);
        assert!((input.end() - 2.1).abs() < 1e-9);
    }

    #[test]
    fn a_partial_frame_is_dropped_and_bad_starts_become_zero() {
        let stereo = AudioFormat::new(16_000, 2);
        let input = AnalyzerInput::new(f64::NAN, stereo, &[0.1, 0.2, 0.3, 0.4, 0.5]);
        assert_eq!(input.samples().len(), 4);
        assert_eq!(input.frames(), 2);
        assert_eq!(input.start(), 0.0);
        let early = AnalyzerInput::new(-3.0, stereo, &[]);
        assert_eq!(early.start(), 0.0);
    }

    #[test]
    fn stereo_is_averaged_to_mono_at_the_analysis_format() {
        let stereo = AnalyzerInput::new(0.0, AudioFormat::new(16_000, 2), &[1.0, 0.0, 0.5, 0.5]);
        let out = AnalyzerInputConverter::new().convert(&stereo);
        assert_eq!(out.samples(), &[0.5, 0.5]);
        assert_eq!(out.format(), ANALYSIS_FORMAT);
    }

    #[test]
    fn conversion_keeps_the_start_time() {
        let input = AnalyzerInput::new(3.25, AudioFormat::new(48_000, 2), &[0.2; 96]);
        let out = AnalyzerInputConverter::new().convert(&input);
        assert_eq!(out.start(), 3.25);
        assert_eq!(out.format().sample_rate, 16_000);
        assert_eq!(out.format().channels, 1);
        assert_eq!(out.frames(), 16);
    }

    #[test]
    fn upsampling_interpolates_between_samples() {
        let input = AnalyzerInput::new(0.0, AudioFormat::new(8_000, 1), &[0.0, 1.0]);
        let out = AnalyzerInputConverter::new().convert(&input);
        assert_eq!(out.samples(), &[0.0, 0.5, 1.0, 1.0]);
    }

    #[test]
    fn downsampling_takes_every_other_sample_and_mono_at_the_same_rate_is_unchanged() {
        let input = AnalyzerInput::new(0.0, AudioFormat::new(16_000, 1), &[0.0, 1.0, 2.0, 3.0]);
        let down = AnalyzerInputConverter::with_target(AudioFormat::new(8_000, 1)).convert(&input);
        assert_eq!(down.samples(), &[0.0, 2.0]);
        let same = AnalyzerInputConverter::new().convert(&input);
        assert_eq!(same.samples(), input.samples());
    }
}
