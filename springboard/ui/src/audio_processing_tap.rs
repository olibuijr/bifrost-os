//! Audio processing taps (`MTAudioProcessingTap`): a processor that sits in an audio mix, sees the source audio block by
//! block, and may change it in place. The life cycle follows the tap API: create (optionally with a preferred format),
//! prepare with the source format and the largest block, process blocks, unprepare. A processor keeps its own state,
//! and the tap owns it, so the caller reads it back through `storage` and `storage_mut`. Samples are interleaved `f32`.
//!
//! This is a pure model. The caller supplies each block (there is no pull from a source), and nothing here touches a
//! device. The audio service would call `process` in its render path. `process_tap` is a different thing: it captures
//! what a process plays into a ring for recording, and does not change the mix.
//!
//! Gaps: no hook in the audio service yet, so an app cannot attach a tap to a playing mix; the placement is recorded
//! but the service does not act on it; the tap does not convert the source into the preferred format, so the caller
//! must supply blocks in `format()`; and no timing or host-time information reaches the processor.

/// Where the tap sits in the chain: before the effects of the mix, or after them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    PreEffects,
    PostEffects,
}

/// A sample rate in hertz and a channel count.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioFormat {
    pub sample_rate: f64,
    pub channels: u32,
}

impl AudioFormat {
    fn validate(self) -> Result<(), TapError> {
        if self.channels == 0 {
            Err(TapError::ZeroChannels)
        } else if !(self.sample_rate.is_finite() && self.sample_rate > 0.0) {
            Err(TapError::BadRate)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TapError {
    /// The format has no channels.
    ZeroChannels,
    /// The sample rate is zero, negative or not a number.
    BadRate,
    /// The largest block size is zero.
    ZeroBlock,
    /// The tap is not prepared, so it takes no blocks.
    NotPrepared,
    /// A block has more frames than the tap was prepared for.
    BlockTooLarge { frames: usize, max: usize },
    /// The sample buffer does not hold exactly `frames * channels` samples.
    BadBuffer { expected: usize, got: usize },
}

/// The work a processor does. `prepare` runs before the first block and again after each `unprepare` and re-prepare;
/// `unprepare` runs once when the tap stops.
pub trait TapProcessor {
    fn prepare(&mut self, format: AudioFormat, max_frames: usize);
    /// Changes `samples` in place. The buffer holds `frames` interleaved frames.
    fn process(&mut self, samples: &mut [f32], frames: usize);
    fn unprepare(&mut self) {}
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Prepared {
    format: AudioFormat,
    max_frames: usize,
}

/// A tap around a processor. Dropping a prepared tap unprepares the processor.
pub struct AudioTap<P: TapProcessor> {
    placement: Placement,
    preferred: Option<AudioFormat>,
    processor: P,
    prepared: Option<Prepared>,
}

impl<P: TapProcessor> AudioTap<P> {
    /// A tap with no preferred format: it runs in the format of the source.
    pub fn create(placement: Placement, processor: P) -> Self {
        Self { placement, preferred: None, processor, prepared: None }
    }

    /// A tap that asks for `preferred` as its format. The format is checked here, so a bad one is refused before the
    /// tap is ever prepared.
    pub fn create_with_preferred_format(placement: Placement, preferred: AudioFormat, processor: P) -> Result<Self, TapError> {
        preferred.validate()?;
        Ok(Self { placement, preferred: Some(preferred), processor, prepared: None })
    }

    pub fn placement(&self) -> Placement {
        self.placement
    }

    pub fn preferred_format(&self) -> Option<AudioFormat> {
        self.preferred
    }

    /// The format blocks are processed in while the tap is prepared: the preferred one if there is one, else the source's.
    pub fn format(&self) -> Option<AudioFormat> {
        self.prepared.map(|p| p.format)
    }

    pub fn is_prepared(&self) -> bool {
        self.prepared.is_some()
    }

    /// The processor's own state, such as a meter's reading.
    pub fn storage(&self) -> &P {
        &self.processor
    }

    pub fn storage_mut(&mut self) -> &mut P {
        &mut self.processor
    }

    /// Prepares the tap for a source of `source` format with blocks of up to `max_frames` frames. Returns the format the
    /// blocks are processed in. Preparing an already prepared tap unprepares it first.
    pub fn prepare(&mut self, source: AudioFormat, max_frames: usize) -> Result<AudioFormat, TapError> {
        source.validate()?;
        if max_frames == 0 {
            return Err(TapError::ZeroBlock);
        }
        self.unprepare();
        let format = self.preferred.unwrap_or(source);
        self.processor.prepare(format, max_frames);
        self.prepared = Some(Prepared { format, max_frames });
        Ok(format)
    }

    /// Runs the processor over one block of `frames` interleaved frames, changing `samples` in place.
    pub fn process(&mut self, samples: &mut [f32], frames: usize) -> Result<(), TapError> {
        let prepared = self.prepared.ok_or(TapError::NotPrepared)?;
        if frames > prepared.max_frames {
            return Err(TapError::BlockTooLarge { frames, max: prepared.max_frames });
        }
        let expected = frames * prepared.format.channels as usize;
        if samples.len() != expected {
            return Err(TapError::BadBuffer { expected, got: samples.len() });
        }
        self.processor.process(samples, frames);
        Ok(())
    }

    /// Stops the tap. The processor is told once, and a tap that is not prepared does nothing.
    pub fn unprepare(&mut self) {
        if self.prepared.take().is_some() {
            self.processor.unprepare();
        }
    }
}

impl<P: TapProcessor> Drop for AudioTap<P> {
    fn drop(&mut self) {
        self.unprepare();
    }
}

/// A processor that leaves the audio alone and reads it: the largest absolute sample since the last prepare, and how
/// many blocks it has seen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PeakMeter {
    pub peak: f32,
    pub blocks: u64,
    pub format: Option<AudioFormat>,
    pub max_frames: usize,
    pub unprepared: u32,
}

impl TapProcessor for PeakMeter {
    fn prepare(&mut self, format: AudioFormat, max_frames: usize) {
        self.peak = 0.0;
        self.format = Some(format);
        self.max_frames = max_frames;
    }

    fn process(&mut self, samples: &mut [f32], _frames: usize) {
        for &s in samples.iter() {
            let a = s.abs();
            if a > self.peak {
                self.peak = a;
            }
        }
        self.blocks += 1;
    }

    fn unprepare(&mut self) {
        self.unprepared += 1;
        self.format = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo(rate: f64) -> AudioFormat {
        AudioFormat { sample_rate: rate, channels: 2 }
    }

    #[test]
    fn a_preferred_format_is_checked_when_the_tap_is_created() {
        let zero_channels = AudioTap::create_with_preferred_format(Placement::PreEffects, AudioFormat { sample_rate: 48_000.0, channels: 0 }, PeakMeter::default());
        assert!(matches!(zero_channels, Err(TapError::ZeroChannels)));
        let no_rate = AudioTap::create_with_preferred_format(Placement::PreEffects, stereo(0.0), PeakMeter::default());
        assert!(matches!(no_rate, Err(TapError::BadRate)));
        let nan_rate = AudioTap::create_with_preferred_format(Placement::PostEffects, stereo(f64::NAN), PeakMeter::default());
        assert!(matches!(nan_rate, Err(TapError::BadRate)));
        let ok = AudioTap::create_with_preferred_format(Placement::PostEffects, stereo(44_100.0), PeakMeter::default()).unwrap();
        assert_eq!(ok.preferred_format(), Some(stereo(44_100.0)));
        assert_eq!(ok.placement(), Placement::PostEffects);
    }

    #[test]
    fn prepare_runs_in_the_preferred_format_or_else_the_source_format() {
        let mut plain = AudioTap::create(Placement::PreEffects, PeakMeter::default());
        let got = plain.prepare(stereo(48_000.0), 512).unwrap();
        assert_eq!(got, stereo(48_000.0));
        assert_eq!(plain.storage().format, Some(stereo(48_000.0)));
        assert_eq!(plain.storage().max_frames, 512);

        let mono_16k = AudioFormat { sample_rate: 16_000.0, channels: 1 };
        let mut asked = AudioTap::create_with_preferred_format(Placement::PreEffects, mono_16k, PeakMeter::default()).unwrap();
        assert_eq!(asked.prepare(stereo(48_000.0), 256), Ok(mono_16k));
        assert!(asked.is_prepared());
        assert_eq!(asked.format(), Some(mono_16k));
        assert_eq!(asked.prepare(stereo(48_000.0), 0), Err(TapError::ZeroBlock));
    }

    #[test]
    fn a_block_is_refused_until_the_tap_is_prepared() {
        let mut tap = AudioTap::create(Placement::PreEffects, PeakMeter::default());
        let mut block = [0.1_f32, 0.2, 0.3, 0.4];
        assert_eq!(tap.process(&mut block, 2), Err(TapError::NotPrepared));
        assert_eq!(tap.storage().blocks, 0);
    }

    #[test]
    fn the_processor_sees_the_block_and_keeps_the_peak() {
        let mut tap = AudioTap::create(Placement::PostEffects, PeakMeter::default());
        tap.prepare(stereo(48_000.0), 4).unwrap();
        // two stereo frames: the largest magnitude is -0.5
        let mut block = [0.1_f32, -0.5, 0.25, 0.2];
        assert_eq!(tap.process(&mut block, 2), Ok(()));
        assert_eq!(tap.storage().peak, 0.5);
        assert_eq!(tap.storage().blocks, 1);
        // a quieter block leaves the peak where it was
        let mut quiet = [0.05_f32, 0.0];
        tap.process(&mut quiet, 1).unwrap();
        assert_eq!(tap.storage().peak, 0.5);
        assert_eq!(tap.storage().blocks, 2);
        // the processor does not change the samples
        assert_eq!(block, [0.1, -0.5, 0.25, 0.2]);
    }

    #[test]
    fn a_block_larger_than_prepared_or_with_the_wrong_length_is_refused() {
        let mut tap = AudioTap::create(Placement::PreEffects, PeakMeter::default());
        tap.prepare(stereo(48_000.0), 2).unwrap();
        let mut three = [0.0_f32; 6];
        assert_eq!(tap.process(&mut three, 3), Err(TapError::BlockTooLarge { frames: 3, max: 2 }));
        // two stereo frames need four samples, not three
        let mut short = [0.0_f32; 3];
        assert_eq!(tap.process(&mut short, 2), Err(TapError::BadBuffer { expected: 4, got: 3 }));
        assert_eq!(tap.storage().blocks, 0);
    }

    #[test]
    fn unprepare_is_told_once_and_stops_the_blocks() {
        let mut tap = AudioTap::create(Placement::PreEffects, PeakMeter::default());
        tap.prepare(stereo(48_000.0), 4).unwrap();
        tap.unprepare();
        assert_eq!(tap.storage().unprepared, 1);
        assert!(!tap.is_prepared());
        let mut block = [0.0_f32; 2];
        assert_eq!(tap.process(&mut block, 1), Err(TapError::NotPrepared));
        // a second stop does nothing, so the processor is not told twice
        tap.unprepare();
        assert_eq!(tap.storage().unprepared, 1);
    }

    #[test]
    fn preparing_again_resets_the_processor_state() {
        let mut tap = AudioTap::create(Placement::PreEffects, PeakMeter::default());
        tap.prepare(stereo(48_000.0), 4).unwrap();
        let mut block = [0.5_f32, 0.5];
        tap.process(&mut block, 1).unwrap();
        assert_eq!(tap.storage().peak, 0.5);
        // a new prepare unprepares the old run first, then starts over at the new format
        let mono = AudioFormat { sample_rate: 16_000.0, channels: 1 };
        tap.prepare(mono, 8).unwrap();
        assert_eq!(tap.storage().peak, 0.0);
        assert_eq!(tap.storage().unprepared, 1);
        assert_eq!(tap.storage().format, Some(mono));
    }
}
