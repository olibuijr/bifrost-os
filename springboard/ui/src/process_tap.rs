//! Process tap model (`ProcessTap`): captures the output of chosen processes, or of every process except some, as a
//! stream of interleaved frames. A `TapDescription` says which processes are tapped, whether the tap is exclusive
//! (every process except the listed ones), how the channels are mixed down, and whether the captured audio is muted
//! at the output. `ProcessTap` checks the description against the processes the system knows, keeps the captured
//! frames in a bounded ring, and hands them out tagged with their source. Nothing here opens a device or reads a
//! process: the audio service owns the real tap and pushes its samples in. Samples are interleaved `f32` frames.

use std::collections::VecDeque;

/// Whether the captured audio still reaches the output while it is tapped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MuteBehavior {
    /// The captured audio plays as usual.
    #[default]
    Unmuted,
    /// The captured audio never reaches the output.
    Muted,
    /// The captured audio is muted only while the tap is running.
    MutedWhenTapped,
}

/// How the tap's channels relate to the source channels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mixdown {
    /// The tap keeps the channel count of the source.
    #[default]
    Source,
    /// The tap delivers one channel, the mean of all source channels.
    Mono,
    /// The tap delivers two channels, the mean of the even and the odd source channels.
    Stereo,
}

/// Where the frames of a block came from. Applications capture their own output or the microphone; the two are
/// kept apart so a recorder can tell them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureKind {
    AppAudio,
    Microphone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TapError {
    /// A non-exclusive tap names no process, so it would capture nothing.
    NoProcesses,
    /// A process id in the description is not known to the system.
    UnknownProcess(u32),
    /// The source or the buffer has no channels, or no frames.
    ZeroSize,
    /// The source sample rate is zero.
    ZeroRate,
    /// The tap is not running, so it takes no samples.
    NotRunning,
    /// The sample count is not a whole number of frames.
    Misaligned,
}

/// What a tap captures and how it delivers it. Build one with `of_processes` or `excluding`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapDescription {
    pub name: String,
    processes: Vec<u32>,
    exclusive: bool,
    pub mixdown: Mixdown,
    pub mute: MuteBehavior,
    /// A private tap is only visible to the process that made it.
    pub private: bool,
}

impl TapDescription {
    /// Taps the output of the listed processes. Mirrors the process-list initializer of the platform's tap
    /// description. The list must not be empty when the description is checked.
    pub fn of_processes(processes: &[u32], mixdown: Mixdown) -> Self {
        TapDescription {
            name: String::new(),
            processes: processes.to_vec(),
            exclusive: false,
            mixdown,
            mute: MuteBehavior::Unmuted,
            private: false,
        }
    }

    /// Taps every process except the listed ones. An empty list taps all processes. Mirrors the exclusive
    /// initializers of the platform's tap description.
    pub fn excluding(processes: &[u32], mixdown: Mixdown) -> Self {
        TapDescription { exclusive: true, ..Self::of_processes(processes, mixdown) }
    }

    /// The process ids named in the description. For an exclusive tap these are the processes left out.
    pub fn processes(&self) -> &[u32] {
        &self.processes
    }

    pub fn is_exclusive(&self) -> bool {
        self.exclusive
    }

    /// The number of channels the tap delivers for a source with `source_channels` channels.
    pub fn channels(&self, source_channels: u32) -> u32 {
        match self.mixdown {
            Mixdown::Source => source_channels,
            Mixdown::Mono => 1,
            Mixdown::Stereo => 2,
        }
    }

    /// Checks the description against the processes the system knows. A non-exclusive tap must name at least one
    /// process. Every named process must be known, and an exclusive tap with an empty list is a tap of everything.
    pub fn validate(&self, known: impl Fn(u32) -> bool) -> Result<(), TapError> {
        if !self.exclusive && self.processes.is_empty() {
            return Err(TapError::NoProcesses);
        }
        match self.processes.iter().find(|pid| !known(**pid)) {
            Some(pid) => Err(TapError::UnknownProcess(*pid)),
            None => Ok(()),
        }
    }

    /// True while the captured audio is kept from the output, given whether the tap is running.
    pub fn output_muted(&self, running: bool) -> bool {
        match self.mute {
            MuteBehavior::Unmuted => false,
            MuteBehavior::Muted => true,
            MuteBehavior::MutedWhenTapped => running,
        }
    }
}

/// The shape of the frames a tap delivers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TapFormat {
    pub channels: u32,
    pub sample_rate: u32,
}

/// Converts interleaved frames from `in_ch` channels to `out_ch` channels. Equal counts copy. A mono source is
/// copied into every output channel. Fewer output channels take the mean of the source channels whose index maps
/// to them (index modulo `out_ch`). More output channels keep the source channels and fill the rest with silence.
/// `input` must hold a whole number of frames.
pub fn convert_frames(input: &[f32], in_ch: u32, out_ch: u32) -> Result<Vec<f32>, TapError> {
    if in_ch == 0 || out_ch == 0 {
        return Err(TapError::ZeroSize);
    }
    let (ic, oc) = (in_ch as usize, out_ch as usize);
    if input.len() % ic != 0 {
        return Err(TapError::Misaligned);
    }
    let mut out = Vec::with_capacity(input.len() / ic * oc);
    for frame in input.chunks(ic) {
        for c in 0..oc {
            let sample = if ic == oc {
                frame[c]
            } else if ic == 1 {
                frame[0]
            } else if oc < ic {
                let (sum, count) = frame
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| j % oc == c)
                    .fold((0.0f32, 0u32), |(s, n), (_, v)| (s + v, n + 1));
                sum / count as f32
            } else if c < ic {
                frame[c]
            } else {
                0.0
            };
            out.push(sample);
        }
    }
    Ok(out)
}

/// A bounded queue of interleaved frames. When a push does not fit, the oldest frames are dropped and counted as
/// overruns, so a reader that falls behind loses the oldest audio rather than blocking the writer.
#[derive(Clone, Debug)]
pub struct FrameRing {
    channels: usize,
    capacity_frames: usize,
    samples: VecDeque<f32>,
    overruns: u64,
}

impl FrameRing {
    /// An empty ring for `channels` channels that holds `capacity_frames` frames.
    pub fn new(channels: u32, capacity_frames: usize) -> Result<Self, TapError> {
        if channels == 0 || capacity_frames == 0 {
            return Err(TapError::ZeroSize);
        }
        Ok(FrameRing {
            channels: channels as usize,
            capacity_frames,
            samples: VecDeque::new(),
            overruns: 0,
        })
    }

    /// Frames waiting to be read.
    pub fn len_frames(&self) -> usize {
        self.samples.len() / self.channels
    }

    /// Total frames dropped to make room, since the ring was made.
    pub fn overruns(&self) -> u64 {
        self.overruns
    }

    /// Appends whole frames. Frames beyond the capacity drop the oldest waiting frames first.
    pub fn push(&mut self, frames: &[f32]) -> Result<(), TapError> {
        if frames.len() % self.channels != 0 {
            return Err(TapError::Misaligned);
        }
        let incoming = frames.len() / self.channels;
        let dropped = (self.len_frames() + incoming).saturating_sub(self.capacity_frames);
        let drop_samples = (dropped * self.channels).min(self.samples.len());
        self.samples.drain(..drop_samples);
        self.overruns += dropped as u64;
        // Only the newest capacity-worth of incoming frames can fit.
        let skip = incoming.saturating_sub(self.capacity_frames) * self.channels;
        self.samples.extend(frames[skip..].iter().copied());
        Ok(())
    }

    /// Removes and returns up to `max_frames` of the oldest frames.
    pub fn pull(&mut self, max_frames: usize) -> Vec<f32> {
        let take = (max_frames.min(self.len_frames()) * self.channels).min(self.samples.len());
        self.samples.drain(..take).collect()
    }

    /// Drops every waiting frame. The overrun count is kept.
    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

/// A run of captured frames with the source they came from.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioBlock {
    pub kind: CaptureKind,
    pub channels: u32,
    pub frames: Vec<f32>,
}

/// A process tap that is checked, started and stopped. It converts the source frames to the shape its description
/// asks for, and keeps them until a reader pulls them.
#[derive(Clone, Debug)]
pub struct ProcessTap {
    description: TapDescription,
    kind: CaptureKind,
    source: TapFormat,
    running: bool,
    ring: FrameRing,
}

impl ProcessTap {
    /// A stopped tap for `source` frames, with room for `buffer_frames` output frames.
    pub fn new(
        description: TapDescription,
        kind: CaptureKind,
        source: TapFormat,
        buffer_frames: usize,
    ) -> Result<Self, TapError> {
        if source.channels == 0 || buffer_frames == 0 {
            return Err(TapError::ZeroSize);
        }
        if source.sample_rate == 0 {
            return Err(TapError::ZeroRate);
        }
        let channels = description.channels(source.channels);
        let ring = FrameRing::new(channels, buffer_frames)?;
        Ok(ProcessTap { description, kind, source, running: false, ring })
    }

    pub fn description(&self) -> &TapDescription {
        &self.description
    }

    /// The shape of the frames `pull` returns.
    pub fn format(&self) -> TapFormat {
        TapFormat { channels: self.description.channels(self.source.channels), sample_rate: self.source.sample_rate }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Checks the description and starts the tap. Starting a running tap checks again and changes nothing else.
    pub fn start(&mut self, known: impl Fn(u32) -> bool) -> Result<(), TapError> {
        self.description.validate(known)?;
        self.running = true;
        Ok(())
    }

    /// Stops the tap and drops the frames it had not delivered. Returns false when it was not running.
    pub fn stop(&mut self) -> bool {
        let was = std::mem::replace(&mut self.running, false);
        self.ring.clear();
        was
    }

    /// Whether the captured audio is kept from the output right now.
    pub fn output_muted(&self) -> bool {
        self.description.output_muted(self.running)
    }

    /// Frames dropped because the reader fell behind.
    pub fn overruns(&self) -> u64 {
        self.ring.overruns()
    }

    /// Takes interleaved source frames while running, converts them to the tap's format and keeps them.
    pub fn push(&mut self, source_frames: &[f32]) -> Result<(), TapError> {
        if !self.running {
            return Err(TapError::NotRunning);
        }
        let converted = convert_frames(source_frames, self.source.channels, self.format().channels)?;
        self.ring.push(&converted)
    }

    /// Takes up to `max_frames` frames from the buffer. Returns an empty block when nothing is waiting.
    pub fn pull(&mut self, max_frames: usize) -> AudioBlock {
        AudioBlock { kind: self.kind, channels: self.format().channels, frames: self.ring.pull(max_frames) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo_source() -> TapFormat {
        TapFormat { channels: 2, sample_rate: 48_000 }
    }

    #[test]
    fn description_needs_processes_unless_it_is_exclusive() {
        let known = |pid: u32| pid < 100;
        let none = TapDescription::of_processes(&[], Mixdown::Source);
        assert_eq!(none.validate(known), Err(TapError::NoProcesses));
        let global = TapDescription::excluding(&[], Mixdown::Source);
        assert_eq!(global.validate(known), Ok(()));
        let bad = TapDescription::of_processes(&[7, 250], Mixdown::Source);
        assert_eq!(bad.validate(known), Err(TapError::UnknownProcess(250)));
        let good = TapDescription::of_processes(&[7, 42], Mixdown::Stereo);
        assert_eq!(good.validate(known), Ok(()));
    }

    #[test]
    fn mixdown_sets_the_channel_count_of_the_tap() {
        assert_eq!(TapDescription::of_processes(&[1], Mixdown::Mono).channels(2), 1);
        assert_eq!(TapDescription::of_processes(&[1], Mixdown::Stereo).channels(6), 2);
        assert_eq!(TapDescription::of_processes(&[1], Mixdown::Source).channels(6), 6);
    }

    #[test]
    fn convert_frames_mixes_stereo_to_mono_and_six_channels_to_stereo() {
        // Stereo to mono is the mean of the two channels in each frame.
        let mono = convert_frames(&[1.0f32, 3.0, -1.0, 1.0], 2, 1).unwrap();
        assert_eq!(mono, vec![2.0f32, 0.0]);
        // Six to stereo: left takes channels 0, 2, 4 and right takes 1, 3, 5, each as a mean.
        let stereo = convert_frames(&[1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0], 6, 2).unwrap();
        assert_eq!(stereo, vec![3.0f32, 4.0]);
        // Mono into stereo copies the sample to both channels.
        assert_eq!(convert_frames(&[0.5f32], 1, 2).unwrap(), vec![0.5f32, 0.5]);
        // Stereo into four channels keeps the two and adds silence.
        assert_eq!(convert_frames(&[1.0f32, 2.0], 2, 4).unwrap(), vec![1.0f32, 2.0, 0.0, 0.0]);
        assert_eq!(convert_frames(&[1.0f32, 2.0, 3.0], 2, 1), Err(TapError::Misaligned));
    }

    #[test]
    fn frame_ring_drops_the_oldest_frames_when_full() {
        let mut ring = FrameRing::new(1, 2).unwrap();
        ring.push(&[1.0f32, 2.0]).unwrap();
        ring.push(&[3.0f32]).unwrap();
        assert_eq!(ring.len_frames(), 2);
        assert_eq!(ring.overruns(), 1);
        assert_eq!(ring.pull(10), vec![2.0f32, 3.0]);
        assert!(ring.pull(10).is_empty());
        // A push larger than the whole ring keeps only its newest frames.
        ring.push(&[4.0f32, 5.0, 6.0, 7.0]).unwrap();
        assert_eq!(ring.overruns(), 3);
        assert_eq!(ring.pull(1), vec![6.0f32]);
        assert_eq!(ring.pull(5), vec![7.0f32]);
    }

    #[test]
    fn tap_converts_source_frames_to_its_mixdown_while_running() {
        let desc = TapDescription::of_processes(&[9], Mixdown::Mono);
        let mut tap = ProcessTap::new(desc, CaptureKind::AppAudio, stereo_source(), 8).unwrap();
        assert_eq!(tap.format(), TapFormat { channels: 1, sample_rate: 48_000 });
        assert_eq!(tap.push(&[1.0f32, 3.0]), Err(TapError::NotRunning));
        tap.start(|pid| pid == 9).unwrap();
        tap.push(&[1.0f32, 3.0, 2.0, 4.0]).unwrap();
        let block = tap.pull(16);
        assert_eq!(block, AudioBlock { kind: CaptureKind::AppAudio, channels: 1, frames: vec![2.0f32, 3.0] });
        assert!(tap.pull(16).frames.is_empty());
    }

    #[test]
    fn stopping_drops_waiting_frames_and_start_checks_the_processes() {
        let desc = TapDescription::of_processes(&[3], Mixdown::Source);
        let mut tap = ProcessTap::new(desc, CaptureKind::Microphone, stereo_source(), 4).unwrap();
        assert_eq!(tap.start(|_| false), Err(TapError::UnknownProcess(3)));
        assert!(!tap.is_running());
        tap.start(|_| true).unwrap();
        tap.push(&[0.25f32, 0.5]).unwrap();
        assert!(tap.stop());
        assert!(!tap.stop(), "a stopped tap reports it was not running");
        assert!(tap.pull(4).frames.is_empty());
    }

    #[test]
    fn mute_behaviour_follows_whether_the_tap_runs() {
        let mut always = TapDescription::of_processes(&[1], Mixdown::Source);
        always.mute = MuteBehavior::Muted;
        assert!(always.output_muted(false) && always.output_muted(true));

        let mut when_tapped = TapDescription::of_processes(&[1], Mixdown::Source);
        when_tapped.mute = MuteBehavior::MutedWhenTapped;
        let mut tap = ProcessTap::new(when_tapped, CaptureKind::AppAudio, stereo_source(), 4).unwrap();
        assert!(!tap.output_muted());
        tap.start(|_| true).unwrap();
        assert!(tap.output_muted());
        tap.stop();
        assert!(!tap.output_muted());
        assert_eq!(TapDescription::of_processes(&[1], Mixdown::Source).mute, MuteBehavior::Unmuted);
    }

    #[test]
    fn new_tap_refuses_empty_sizes_and_zero_rates() {
        let desc = || TapDescription::of_processes(&[1], Mixdown::Source);
        assert_eq!(
            ProcessTap::new(desc(), CaptureKind::AppAudio, TapFormat { channels: 0, sample_rate: 48_000 }, 4)
                .map(|_| ()),
            Err(TapError::ZeroSize)
        );
        assert_eq!(
            ProcessTap::new(desc(), CaptureKind::AppAudio, TapFormat { channels: 2, sample_rate: 0 }, 4).map(|_| ()),
            Err(TapError::ZeroRate)
        );
        assert_eq!(
            ProcessTap::new(desc(), CaptureKind::AppAudio, stereo_source(), 0).map(|_| ()),
            Err(TapError::ZeroSize)
        );
    }
}
