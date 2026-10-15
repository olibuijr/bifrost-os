//! Sample buffers: one coded sample with its timing, as a media pipeline hands it to a consumer. The model mirrors the
//! timing and sync-flag queries of a sample buffer: presentation and decode times, the duration, whether the sample
//! can start decoding on its own, and the presentation order of a batch. The data is owned bytes, not a shared buffer.
//!
//! The demuxer's [`springboard::media::demux::Sample`] converts into a [`SampleBuffer`] with [`SampleBuffer::from_demuxed`].

use springboard::media::demux::Sample;

/// The timing of one sample, in microseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleTiming {
    pub presentation_us: i64,
    pub decode_us: i64,
    pub duration_us: i64,
}

/// Why a sample was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SampleError {
    /// The sample carries no bytes.
    EmptyData,
    /// The duration is below zero.
    NegativeDuration(i64),
}

/// One coded sample of a track with its timing and sync flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleBuffer {
    track: u32,
    timing: SampleTiming,
    sync: bool,
    data: Vec<u8>,
}

impl SampleBuffer {
    /// Builds a sample. `sync` is true when the sample can be decoded without earlier samples.
    pub fn new(track: u32, timing: SampleTiming, sync: bool, data: Vec<u8>) -> Result<SampleBuffer, SampleError> {
        if data.is_empty() {
            return Err(SampleError::EmptyData);
        }
        if timing.duration_us < 0 {
            return Err(SampleError::NegativeDuration(timing.duration_us));
        }
        Ok(SampleBuffer { track, timing, sync, data })
    }

    /// Converts a demuxed sample, copying its bytes and keeping its keyframe flag as the sync flag.
    pub fn from_demuxed(sample: &Sample) -> Result<SampleBuffer, SampleError> {
        let timing = SampleTiming { presentation_us: sample.pts_us, decode_us: sample.dts_us, duration_us: sample.duration_us };
        SampleBuffer::new(sample.track, timing, sample.keyframe, sample.data.clone())
    }

    pub fn track(&self) -> u32 {
        self.track
    }

    pub fn timing(&self) -> SampleTiming {
        self.timing
    }

    /// True when the sample can start decoding on its own.
    pub fn is_sync(&self) -> bool {
        self.sync
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn data_len(&self) -> usize {
        self.data.len()
    }

    /// The presentation time at which the sample ends. Saturates at the largest time value.
    pub fn presentation_end_us(&self) -> i64 {
        self.timing.presentation_us.saturating_add(self.timing.duration_us)
    }
}

/// Sorts a batch of samples by presentation time. Samples with the same time keep their order.
pub fn sort_by_presentation(samples: &mut [SampleBuffer]) {
    samples.sort_by_key(|s| s.timing.presentation_us);
}
