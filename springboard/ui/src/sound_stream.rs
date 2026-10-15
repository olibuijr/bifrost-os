//! Streaming sound analysis: audio arrives in buffers, and the analyzer classifies each full window as soon as its
//! samples have all arrived. This is the live counterpart of [`crate::sound_classify::SoundFileAnalyzer`], which
//! runs over a whole buffer at once.
//!
//! The caller passes buffers in order and says where each one starts in frames, counted from the first frame of the
//! stream. Interleaved channels are averaged to mono before classification, as the file analyzer expects. Windows and
//! overlaps come from the same [`crate::sound_classify::ClassifyRequest`] as a file analysis, so both paths produce
//! the same windows for the same audio. A partial window at the end of the stream is not classified.

use crate::sound_classify::{top_candidates, ClassificationResult, ClassifyRequest, SoundError};

/// The layout of the interleaved samples a stream delivers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamFormat {
    /// Frames per second.
    pub sample_rate: u32,
    /// Interleaved channels per frame.
    pub channels: u16,
}

/// Classifies a live stream window by window. `C` is the classifier: one mono window in, labelled confidences out.
pub struct StreamAnalyzer<C>
where
    C: Fn(&[f32]) -> Vec<(String, f64)>,
{
    format: StreamFormat,
    window: usize,
    hop: usize,
    classifier: C,
    /// Mono samples not yet dropped, starting at frame `pending_start`.
    pending: Vec<f32>,
    pending_start: u64,
    /// The frame position the next buffer must start at.
    next_frame: u64,
    results: Vec<ClassificationResult>,
}

impl<C> StreamAnalyzer<C>
where
    C: Fn(&[f32]) -> Vec<(String, f64)>,
{
    /// A stream analysis for `format`, using the windows of `request` and the labels `classifier` returns. A zero
    /// sample rate or channel count is refused.
    pub fn new(format: StreamFormat, request: &ClassifyRequest, classifier: C) -> Result<Self, SoundError> {
        if format.sample_rate == 0 || format.channels == 0 {
            return Err(SoundError::InvalidFormat);
        }
        Ok(Self {
            format,
            window: request.window_frames(format.sample_rate),
            hop: request.hop_frames(format.sample_rate),
            classifier,
            pending: Vec::new(),
            pending_start: 0,
            next_frame: 0,
            results: Vec::new(),
        })
    }

    /// Adds the next buffer of interleaved samples, which starts at `frame_position` frames into the stream, and
    /// returns the results for the windows that this buffer completed. A buffer whose length is not a whole number of
    /// frames is refused, and so is one whose position does not follow on from the frames already received.
    pub fn analyze(&mut self, samples: &[f32], frame_position: u64) -> Result<Vec<ClassificationResult>, SoundError> {
        let channels = self.format.channels as usize;
        if samples.len() % channels != 0 {
            return Err(SoundError::InvalidFormat);
        }
        if frame_position != self.next_frame {
            return Err(SoundError::OperationFailed);
        }
        let scale = 1.0 / channels as f32;
        self.pending.extend(samples.chunks_exact(channels).map(|frame| frame.iter().sum::<f32>() * scale));
        self.next_frame += (samples.len() / channels) as u64;

        let sample_rate = self.format.sample_rate as f64;
        let first_new = self.results.len();
        while self.pending.len() >= self.window {
            let raw = (self.classifier)(&self.pending[..self.window]);
            let start = self.pending_start;
            let result = ClassificationResult {
                time_range: start as f64 / sample_rate..(start + self.window as u64) as f64 / sample_rate,
                classifications: top_candidates(raw)?,
            };
            self.results.push(result);
            self.pending.drain(..self.hop);
            self.pending_start += self.hop as u64;
        }
        Ok(self.results[first_new..].to_vec())
    }

    /// Ends the stream and returns every result it produced, in time order. Consuming the analyzer means no buffer
    /// can follow the end.
    pub fn complete_analysis(self) -> Vec<ClassificationResult> {
        self.results
    }
}
