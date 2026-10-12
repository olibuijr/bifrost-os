//! Process and system taps (task 17, docs/audio-architecture.md "Hog mode, aggregates and taps").
//!
//! A tap captures the output of chosen processes (or of every process except some) as an input stream. It is read
//! through a private aggregate (`Tap::read_input`), needs the `audio-capture` permission, and the menu-bar
//! indicator shows while any tap runs (`TapRegistry::indicator_visible`). A muted tap keeps its processes' audio
//! out of the output mix (`TapRegistry::is_muted`). Subsystem `is.olibuijr.audio`, category `tap`.
//!
//! Real-time rules: `accumulate`, `finish_cycle`, `read_input` and `is_muted` never allocate or lock; all buffers
//! are sized at creation. Samples are interleaved `f32`.

use bifrost_audio_types::AudioError;

/// Whether the tapped audio still reaches the output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MuteBehavior {
    #[default]
    Unmuted,
    Muted,
    /// Muted only while the tap runs.
    MutedWhenTapped,
}

/// How the tap's channels relate to the source's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mixdown {
    /// Same channel count as the source ("matching").
    #[default]
    Source,
    Mono,
    Stereo,
}

/// What a tap captures. Build with `of_processes` (listed processes) or `global` (all but the listed ones).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapDescription {
    pub name: String,
    processes: Vec<u32>,
    exclusive: bool,
    pub mixdown: Mixdown,
    pub mute: MuteBehavior,
    /// A private tap is visible only to its creator.
    pub private: bool,
}

impl TapDescription {
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

    /// Every process except `excluded` (an empty list taps all).
    pub fn global(excluded: &[u32], mixdown: Mixdown) -> Self {
        TapDescription { exclusive: true, ..Self::of_processes(excluded, mixdown) }
    }

    pub fn processes(&self) -> &[u32] {
        &self.processes
    }

    pub fn is_global(&self) -> bool {
        self.exclusive
    }

    /// Channels delivered for a source of `source_channels`.
    pub fn channels(&self, source_channels: u32) -> u32 {
        match self.mixdown {
            Mixdown::Source => source_channels,
            Mixdown::Mono => 1,
            Mixdown::Stereo => 2,
        }
    }

    /// Does the tap include audio from `pid`?
    pub fn covers(&self, pid: u32) -> bool {
        self.processes.contains(&pid) != self.exclusive
    }

    fn validate(&self) -> Result<(), AudioError> {
        if !self.exclusive && self.processes.is_empty() {
            return Err(AudioError::BadValue);
        }
        Ok(())
    }

    fn muted(&self, running: bool) -> bool {
        match self.mute {
            MuteBehavior::Unmuted => false,
            MuteBehavior::Muted => true,
            MuteBehavior::MutedWhenTapped => running,
        }
    }
}

/// Converts one interleaved frame of `src` channels into `dst`. Equal counts copy; mono is copied to every output
/// channel; fewer outputs average the source channels whose index modulo `dst.len()` matches; more outputs keep
/// the source channels and fill the rest with silence.
pub fn convert_frame(src: &[f32], dst: &mut [f32]) {
    let (ic, oc) = (src.len(), dst.len());
    for (c, d) in dst.iter_mut().enumerate() {
        *d = if ic == oc {
            src[c]
        } else if ic == 1 {
            src[0]
        } else if oc < ic {
            let (mut sum, mut n) = (0.0f32, 0u32);
            let mut j = c;
            while j < ic {
                sum += src[j];
                n += 1;
                j += oc;
            }
            sum / n as f32
        } else if c < ic {
            src[c]
        } else {
            0.0
        };
    }
}

/// One tap: a cycle accumulator and a fixed ring that the aggregate reads.
#[derive(Debug)]
pub struct Tap {
    description: TapDescription,
    owner: u32,
    src_channels: usize,
    out_channels: usize,
    sample_rate: u32,
    running: bool,
    acc: Vec<f32>,
    acc_frames: usize,
    ring: Vec<f32>,
    ring_frames: usize,
    head: usize,
    len: usize,
    overruns: u64,
}

impl Tap {
    fn new(
        description: TapDescription,
        owner: u32,
        src_channels: u32,
        sample_rate: u32,
        cycle_frames: usize,
        ring_frames: usize,
    ) -> Result<Tap, AudioError> {
        description.validate()?;
        if src_channels == 0 || sample_rate == 0 || cycle_frames == 0 || ring_frames == 0 {
            return Err(AudioError::BadValue);
        }
        let out = description.channels(src_channels) as usize;
        Ok(Tap {
            description,
            owner,
            src_channels: src_channels as usize,
            out_channels: out,
            sample_rate,
            running: false,
            acc: vec![0.0; cycle_frames * src_channels as usize],
            acc_frames: 0,
            ring: vec![0.0; ring_frames * out],
            ring_frames,
            head: 0,
            len: 0,
            overruns: 0,
        })
    }

    pub fn description(&self) -> &TapDescription {
        &self.description
    }
    pub fn owner(&self) -> u32 {
        self.owner
    }
    pub fn is_running(&self) -> bool {
        self.running
    }
    /// (channels, sample rate) of the frames `read_input` returns.
    pub fn format(&self) -> (u32, u32) {
        (self.out_channels as u32, self.sample_rate)
    }
    pub fn overruns(&self) -> u64 {
        self.overruns
    }
    pub fn available_frames(&self) -> usize {
        self.len
    }

    /// Adds one process's output for this cycle to the tap, if the tap covers `pid`. Real-time safe. Frames
    /// beyond the cycle size are ignored.
    pub fn accumulate(&mut self, pid: u32, frames: &[f32]) {
        if !self.running || !self.description.covers(pid) {
            return;
        }
        let n = (frames.len() / self.src_channels).min(self.acc.len() / self.src_channels);
        for (a, s) in self.acc[..n * self.src_channels].iter_mut().zip(frames) {
            *a += *s;
        }
        self.acc_frames = self.acc_frames.max(n);
    }

    /// Ends the cycle: mixes down the accumulated frames into the ring and clears the accumulator. A full ring
    /// drops the oldest frames and counts them as overruns. Real-time safe.
    pub fn finish_cycle(&mut self) {
        let (ic, oc) = (self.src_channels, self.out_channels);
        for f in 0..self.acc_frames {
            if self.len == self.ring_frames {
                self.head = (self.head + 1) % self.ring_frames;
                self.len -= 1;
                self.overruns += 1;
            }
            let slot = (self.head + self.len) % self.ring_frames;
            convert_frame(&self.acc[f * ic..(f + 1) * ic], &mut self.ring[slot * oc..(slot + 1) * oc]);
            self.len += 1;
        }
        self.acc[..self.acc_frames * ic].fill(0.0);
        self.acc_frames = 0;
    }

    /// The aggregate's read: copies up to `out.len() / channels` frames, returns the frame count.
    pub fn read_input(&mut self, out: &mut [f32]) -> usize {
        let oc = self.out_channels;
        let n = self.len.min(out.len() / oc);
        for f in 0..n {
            let slot = (self.head + f) % self.ring_frames;
            out[f * oc..(f + 1) * oc].copy_from_slice(&self.ring[slot * oc..(slot + 1) * oc]);
        }
        self.head = (self.head + n) % self.ring_frames;
        self.len -= n;
        n
    }
}

/// Handle to a tap in the registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TapId(pub u32);

/// All taps of the server. Control-thread operations may allocate; the per-cycle ones (`accumulate_all`,
/// `finish_all`, `is_muted`) do not.
#[derive(Debug, Default)]
pub struct TapRegistry {
    taps: Vec<(TapId, Tap)>,
    next: u32,
}

impl TapRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a stopped tap. Needs the `audio-capture` permission (`has_capture`), else `PermissionDenied`.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &mut self,
        caller_pid: u32,
        has_capture: bool,
        description: TapDescription,
        src_channels: u32,
        sample_rate: u32,
        cycle_frames: usize,
        ring_frames: usize,
    ) -> Result<TapId, AudioError> {
        if !has_capture {
            eprintln!("tap: pid {caller_pid} denied (audio-capture)");
            return Err(AudioError::PermissionDenied);
        }
        let tap = Tap::new(description, caller_pid, src_channels, sample_rate, cycle_frames, ring_frames)?;
        let id = TapId(self.next);
        self.next += 1;
        self.taps.push((id, tap));
        Ok(id)
    }

    /// The tap, if it exists and `caller_pid` may see it (private taps belong to their creator).
    pub fn get_mut(&mut self, id: TapId, caller_pid: u32) -> Result<&mut Tap, AudioError> {
        match self.taps.iter_mut().find(|(i, _)| *i == id) {
            Some((_, t)) if !t.description.private || t.owner == caller_pid => Ok(t),
            Some(_) => Err(AudioError::PermissionDenied),
            None => Err(AudioError::UnknownObject),
        }
    }

    pub fn start(&mut self, id: TapId, caller_pid: u32) -> Result<(), AudioError> {
        self.get_mut(id, caller_pid)?.running = true;
        Ok(())
    }

    /// Stops the tap and drops undelivered frames.
    pub fn stop(&mut self, id: TapId, caller_pid: u32) -> Result<(), AudioError> {
        let t = self.get_mut(id, caller_pid)?;
        t.running = false;
        t.head = 0;
        t.len = 0;
        t.acc.fill(0.0);
        t.acc_frames = 0;
        Ok(())
    }

    pub fn destroy(&mut self, id: TapId, caller_pid: u32) -> Result<(), AudioError> {
        self.get_mut(id, caller_pid)?;
        self.taps.retain(|(i, _)| *i != id);
        Ok(())
    }

    /// Drops every tap of a process that exited.
    pub fn process_exited(&mut self, pid: u32) {
        self.taps.retain(|(_, t)| t.owner != pid);
    }

    /// True while any tap runs: the menu-bar indicator.
    pub fn indicator_visible(&self) -> bool {
        self.taps.iter().any(|(_, t)| t.running)
    }

    /// Owners of running taps, for the indicator's menu. Allocates (control thread).
    pub fn active_owners(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.taps.iter().filter(|(_, t)| t.running).map(|(_, t)| t.owner).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Per cycle: offers one process's output to every tap.
    pub fn accumulate_all(&mut self, pid: u32, frames: &[f32]) {
        for (_, t) in self.taps.iter_mut() {
            t.accumulate(pid, frames);
        }
    }

    /// Per cycle: finishes every tap's cycle.
    pub fn finish_all(&mut self) {
        for (_, t) in self.taps.iter_mut() {
            t.finish_cycle();
        }
    }

    /// Must the mixer keep `pid`'s output out of the output mix?
    pub fn is_muted(&self, pid: u32) -> bool {
        self.taps.iter().any(|(_, t)| t.description.covers(pid) && t.description.muted(t.running))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg_with(desc: TapDescription, src: u32) -> (TapRegistry, TapId) {
        let mut r = TapRegistry::new();
        let id = r.create(10, true, desc, src, 48_000, 4, 8).unwrap();
        (r, id)
    }

    #[test]
    fn needs_the_capture_permission() {
        let mut r = TapRegistry::new();
        let d = TapDescription::of_processes(&[1], Mixdown::Source);
        assert_eq!(r.create(10, false, d, 2, 48_000, 4, 8), Err(AudioError::PermissionDenied));
    }

    #[test]
    fn empty_process_list_is_rejected_unless_global() {
        let mut r = TapRegistry::new();
        let d = TapDescription::of_processes(&[], Mixdown::Source);
        assert_eq!(r.create(1, true, d, 2, 48_000, 4, 8), Err(AudioError::BadValue));
        assert!(r.create(1, true, TapDescription::global(&[], Mixdown::Source), 2, 48_000, 4, 8).is_ok());
    }

    #[test]
    fn covers_listed_or_all_but_listed() {
        let a = TapDescription::of_processes(&[5], Mixdown::Source);
        assert!(a.covers(5) && !a.covers(6));
        let g = TapDescription::global(&[5], Mixdown::Source);
        assert!(!g.covers(5) && g.covers(6));
    }

    #[test]
    fn mixdown_channel_counts() {
        let d = |m| TapDescription::of_processes(&[1], m);
        assert_eq!(d(Mixdown::Mono).channels(2), 1);
        assert_eq!(d(Mixdown::Stereo).channels(6), 2);
        assert_eq!(d(Mixdown::Source).channels(6), 6);
    }

    #[test]
    fn convert_frame_cases() {
        let mut m = [0.0f32; 1];
        convert_frame(&[1.0, 3.0], &mut m);
        assert_eq!(m, [2.0]);
        let mut s = [0.0f32; 2];
        convert_frame(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &mut s);
        assert_eq!(s, [3.0, 4.0]);
        convert_frame(&[0.5], &mut s);
        assert_eq!(s, [0.5, 0.5]);
        let mut q = [9.0f32; 4];
        convert_frame(&[1.0, 2.0], &mut q);
        assert_eq!(q, [1.0, 2.0, 0.0, 0.0]);
    }

    #[test]
    fn tap_records_exactly_what_the_process_plays() {
        let (mut r, id) = reg_with(TapDescription::of_processes(&[7], Mixdown::Source), 2);
        r.start(id, 10).unwrap();
        r.accumulate_all(7, &[0.25, -0.5, 1.0, 0.0]);
        r.accumulate_all(8, &[9.0, 9.0, 9.0, 9.0]); // not tapped
        r.finish_all();
        let mut out = [0.0f32; 8];
        let n = r.get_mut(id, 10).unwrap().read_input(&mut out);
        assert_eq!(n, 2);
        assert_eq!(&out[..4], &[0.25, -0.5, 1.0, 0.0]);
        // The accumulator was cleared: an empty cycle adds nothing.
        r.finish_all();
        assert_eq!(r.get_mut(id, 10).unwrap().available_frames(), 0);
    }

    #[test]
    fn two_processes_sum_and_mix_down() {
        let (mut r, id) = reg_with(TapDescription::global(&[], Mixdown::Mono), 2);
        r.start(id, 10).unwrap();
        r.accumulate_all(1, &[1.0, 2.0]);
        r.accumulate_all(2, &[0.0, 1.0]);
        r.finish_all();
        let mut out = [0.0f32; 4];
        assert_eq!(r.get_mut(id, 10).unwrap().read_input(&mut out), 1);
        assert_eq!(out[0], 2.0); // (1 + 3) / 2
    }

    #[test]
    fn stopped_tap_captures_nothing() {
        let (mut r, id) = reg_with(TapDescription::of_processes(&[7], Mixdown::Source), 1);
        r.accumulate_all(7, &[1.0]);
        r.finish_all();
        assert_eq!(r.get_mut(id, 10).unwrap().available_frames(), 0);
    }

    #[test]
    fn ring_overrun_drops_oldest() {
        let mut r = TapRegistry::new();
        let id = r.create(1, true, TapDescription::of_processes(&[7], Mixdown::Source), 1, 48_000, 4, 3).unwrap();
        r.start(id, 1).unwrap();
        for v in [1.0f32, 2.0] {
            r.accumulate_all(7, &[v, v]);
            r.finish_all();
        }
        // 4 frames into a 3-frame ring: the oldest is dropped.
        let t = r.get_mut(id, 1).unwrap();
        assert_eq!(t.overruns(), 1);
        let mut out = [0.0f32; 3];
        assert_eq!(t.read_input(&mut out), 3);
        assert_eq!(out, [1.0, 2.0, 2.0]);
    }

    #[test]
    fn mute_behaviours() {
        let mut d = TapDescription::of_processes(&[7], Mixdown::Source);
        d.mute = MuteBehavior::Muted;
        let (mut r, id) = reg_with(d, 2);
        assert!(r.is_muted(7) && !r.is_muted(8));
        r.start(id, 10).unwrap();
        assert!(r.is_muted(7));
        let mut d2 = TapDescription::of_processes(&[7], Mixdown::Source);
        d2.mute = MuteBehavior::MutedWhenTapped;
        let (mut r2, id2) = reg_with(d2, 2);
        assert!(!r2.is_muted(7));
        r2.start(id2, 10).unwrap();
        assert!(r2.is_muted(7));
        r2.stop(id2, 10).unwrap();
        assert!(!r2.is_muted(7));
    }

    #[test]
    fn indicator_follows_running_taps() {
        let (mut r, id) = reg_with(TapDescription::of_processes(&[7], Mixdown::Source), 2);
        assert!(!r.indicator_visible());
        r.start(id, 10).unwrap();
        assert!(r.indicator_visible());
        assert_eq!(r.active_owners(), vec![10]);
        r.stop(id, 10).unwrap();
        assert!(!r.indicator_visible());
    }

    #[test]
    fn private_tap_belongs_to_its_creator_and_dies_with_it() {
        let mut d = TapDescription::of_processes(&[7], Mixdown::Source);
        d.private = true;
        let (mut r, id) = reg_with(d, 2);
        assert_eq!(r.start(id, 99), Err(AudioError::PermissionDenied));
        assert!(r.start(id, 10).is_ok());
        r.process_exited(10);
        assert_eq!(r.start(id, 10), Err(AudioError::UnknownObject));
        assert!(!r.indicator_visible());
    }
}
