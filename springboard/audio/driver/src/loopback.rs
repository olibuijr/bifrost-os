//! `LoopbackDriver`: an output device whose mix comes back as a paired input device, in memory (owner: task 05).
//! Two devices: `loopback:out` (stereo playback) and `loopback:in` (stereo capture). What the server writes to the
//! first is read from the second. Opening the playback side pre-rolls one quantum of silence, which is the loopback's
//! latency (reported), so a capture read in the same cycle as the first write finds data and never underruns.

use crate::null::{frames_in, ClockFn};
use crate::{
    ControlDesc, ControlId, ControlValue, DeviceDesc, DeviceIo, Driver, DriverError, DriverEvent, HwPosition, IoConfig,
    Latency, Xrun, ZeroTimeStamp,
};
use bifrost_audio_types::{BufferList, StreamFormat};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub const OUT_UID: &str = "loopback:out";
pub const IN_UID: &str = "loopback:in";
const CHANNELS: usize = 2;

#[derive(Default)]
struct Pipe {
    planes: Vec<VecDeque<f32>>,
    /// Frames the reader has taken (or that overflow dropped) per plane: the absolute index of `planes[c][0]`.
    consumed: Vec<u64>,
    /// Open playback sides (several may be open; their mixes are summed).
    outs: u32,
    in_open: bool,
}

/// The most quanta the pipe holds; a writer that gets further ahead of the reader loses the oldest frames.
const MAX_QUEUED_QUANTA: usize = 16;

pub struct LoopbackDriver {
    clock: ClockFn,
    pipe: Arc<Mutex<Pipe>>,
    out_uid: String,
    in_uid: String,
    label: String,
    channels: usize,
}

impl LoopbackDriver {
    /// A loopback on an explicit host clock (tests pass a `ManualClock`).
    pub fn with_clock(clock: ClockFn) -> LoopbackDriver {
        LoopbackDriver::named(clock, OUT_UID, IN_UID, "Loopback", CHANNELS)
    }

    /// A loopback pair with its own uids, display name and channel count (a virtual device a plug-in publishes).
    /// The playback device is `out_uid`, the capture device `in_uid`; names read "<label> (playback)" and
    /// "<label> (capture)".
    pub fn named(clock: ClockFn, out_uid: &str, in_uid: &str, label: &str, channels: usize) -> LoopbackDriver {
        let channels = channels.max(1);
        let pipe = Pipe { planes: vec![VecDeque::new(); channels], consumed: vec![0; channels], ..Default::default() };
        LoopbackDriver { clock, pipe: Arc::new(Mutex::new(pipe)), out_uid: out_uid.to_string(), in_uid: in_uid.to_string(), label: label.to_string(), channels }
    }

    /// A loopback on the process's monotonic clock (nanoseconds since the first call).
    pub fn realtime() -> LoopbackDriver {
        static BASE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let base = *BASE.get_or_init(std::time::Instant::now);
        LoopbackDriver::with_clock(Arc::new(move || base.elapsed().as_nanos() as u64))
    }

    fn desc(&self, uid: &str, input: bool) -> DeviceDesc {
        DeviceDesc {
            uid: uid.into(),
            name: format!("{} ({})", self.label, if input { "capture" } else { "playback" }),
            manufacturer: "BifrOSt".into(),
            transport: "virtual".into(),
            input_channels: if input { self.channels as u32 } else { 0 },
            output_channels: if input { 0 } else { self.channels as u32 },
            rates: vec![(44_100.0, 96_000.0)],
            buffer_frames_range: (16, 4096),
            clock_domain: 1,
        }
    }
}

impl Driver for LoopbackDriver {
    fn name(&self) -> &str {
        "loopback"
    }

    fn enumerate(&mut self, out: &mut Vec<DeviceDesc>) {
        out.push(self.desc(&self.out_uid, false));
        out.push(self.desc(&self.in_uid, true));
    }

    fn open(&mut self, uid: &str, cfg: &IoConfig) -> Result<Box<dyn DeviceIo>, DriverError> {
        let input = if uid == self.out_uid {
            false
        } else if uid == self.in_uid {
            true
        } else {
            return Err(DriverError::NotFound);
        };
        let mut p = self.pipe.lock().unwrap();
        if input {
            if p.in_open {
                return Err(DriverError::Busy);
            }
            p.in_open = true;
        }
        let mut written = 0;
        if !input {
            if p.outs == 0 {
                // First playback side: pre-roll one quantum of silence.
                p.consumed.iter_mut().for_each(|c| *c = 0);
                for pl in &mut p.planes {
                    pl.clear();
                    pl.extend(std::iter::repeat(0.0).take(cfg.quantum as usize));
                }
            }
            p.outs += 1;
            // This writer's first frame lands one quantum after what the reader has taken.
            written = p.consumed[0] + cfg.quantum as u64;
        }
        Ok(Box::new(LoopIo {
            pipe: self.pipe.clone(),
            clock: self.clock.clone(),
            input,
            rate: cfg.rate.round() as u32,
            quantum: cfg.quantum,
            channels: self.channels,
            start_ns: 0,
            running: false,
            open: true,
            written,
        }))
    }

    fn poll_events(&mut self, _out: &mut Vec<DriverEvent>) {}

    fn controls(&mut self, _uid: &str) -> Vec<ControlDesc> {
        Vec::new()
    }

    fn set_control(&mut self, _uid: &str, _id: ControlId, _v: ControlValue) -> Result<(), DriverError> {
        Err(DriverError::Unsupported("the loopback has no controls".into()))
    }
}

struct LoopIo {
    pipe: Arc<Mutex<Pipe>>,
    clock: ClockFn,
    input: bool,
    rate: u32,
    quantum: u32,
    channels: usize,
    start_ns: u64,
    running: bool,
    open: bool,
    /// Absolute index of the next frame this (playback) side writes.
    written: u64,
}

impl DeviceIo for LoopIo {
    fn start(&mut self) -> Result<ZeroTimeStamp, DriverError> {
        self.start_ns = (self.clock)();
        self.running = true;
        Ok(ZeroTimeStamp { sample_time: 0, host_ns: self.start_ns })
    }

    fn position(&mut self) -> HwPosition {
        let now = (self.clock)();
        let hw = if self.running { frames_in(now.saturating_sub(self.start_ns), self.rate, 0) } else { 0 };
        HwPosition { hw_frames: hw, host_ns: now, running: self.running }
    }

    fn read_input(&mut self, frames: u32, dst: &mut BufferList) -> Result<(), Xrun> {
        let mut guard = self.pipe.lock().unwrap();
        let p = &mut *guard;
        let mut lost = 0;
        for (c, b) in dst.buffers.iter_mut().enumerate() {
            let n = (frames as usize).min(b.data.len());
            let mut got = 0;
            for s in &mut b.data[..n] {
                match p.planes.get_mut(c).and_then(|q| q.pop_front()) {
                    Some(v) => {
                        p.consumed[c] += 1;
                        *s = v;
                        got += 1;
                    }
                    None => *s = 0.0,
                }
            }
            lost = lost.max(n - got);
        }
        if lost > 0 {
            Err(Xrun { frames_lost: lost as u32 })
        } else {
            Ok(())
        }
    }

    fn write_mix(&mut self, frames: u32, src: &BufferList) -> Result<(), Xrun> {
        let mut guard = self.pipe.lock().unwrap();
        let p = &mut *guard;
        let cap = MAX_QUEUED_QUANTA * self.quantum as usize;
        let mut lost = 0usize;
        for (c, b) in src.buffers.iter().enumerate().take(self.channels) {
            let q = &mut p.planes[c];
            let n = (frames as usize).min(b.data.len());
            for (i, v) in b.data[..n].iter().enumerate() {
                let abs = self.written + i as u64;
                if abs < p.consumed[c] {
                    continue;
                }
                let idx = (abs - p.consumed[c]) as usize;
                while q.len() <= idx {
                    q.push_back(0.0);
                }
                q[idx] += *v;
            }
            let mut dropped = 0;
            while q.len() > cap {
                q.pop_front();
                p.consumed[c] += 1;
                dropped += 1;
            }
            lost = lost.max(dropped);
        }
        self.written += frames as u64;
        if lost > 0 {
            Err(Xrun { frames_lost: lost as u32 })
        } else {
            Ok(())
        }
    }

    fn stop(&mut self) {
        self.running = false;
        if !self.open {
            return;
        }
        self.open = false;
        let mut p = self.pipe.lock().unwrap();
        if self.input {
            p.in_open = false;
        } else {
            p.outs = p.outs.saturating_sub(1);
        }
    }

    fn latency(&self) -> Latency {
        if self.input {
            Latency::default()
        } else {
            Latency { output: self.quantum, ..Default::default() }
        }
    }

    fn physical_formats(&self) -> (Option<StreamFormat>, Option<StreamFormat>) {
        let f = StreamFormat::float32(self.rate as f64, self.channels as u32);
        if self.input {
            (Some(f), None)
        } else {
            (None, Some(f))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::null::ManualClock;
    use bifrost_audio_types::Buffer;

    fn bl(planes: &[&[f32]]) -> BufferList {
        BufferList { buffers: planes.iter().map(|p| Buffer { channels: 1, data: p.to_vec() }).collect() }
    }

    fn cfg() -> IoConfig {
        IoConfig { rate: 48_000.0, quantum: 4, input: true, output: true }
    }

    #[test]
    fn enumerates_a_playback_and_a_capture_device() {
        let mut d = LoopbackDriver::with_clock(ManualClock::new().as_fn());
        let mut v = Vec::new();
        d.enumerate(&mut v);
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].output_channels, v[0].input_channels), (2, 0));
        assert_eq!((v[1].output_channels, v[1].input_channels), (0, 2));
    }

    #[test]
    fn playback_comes_back_after_the_preroll() {
        let mut d = LoopbackDriver::with_clock(ManualClock::new().as_fn());
        let mut out = d.open(OUT_UID, &cfg()).unwrap();
        let mut inp = d.open(IN_UID, &cfg()).unwrap();
        out.start().unwrap();
        inp.start().unwrap();
        out.write_mix(4, &bl(&[&[1.0, 2.0, 3.0, 4.0], &[5.0, 6.0, 7.0, 8.0]])).unwrap();
        let mut dst = bl(&[&[9.0; 4], &[9.0; 4]]);
        inp.read_input(4, &mut dst).unwrap();
        assert_eq!(dst.buffers[0].data, vec![0.0; 4]);
        inp.read_input(4, &mut dst).unwrap();
        assert_eq!(dst.buffers[0].data, vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(dst.buffers[1].data, vec![5.0, 6.0, 7.0, 8.0]);
        // Nothing left: an underrun of 4 frames, output zero-filled.
        assert_eq!(inp.read_input(4, &mut dst), Err(Xrun { frames_lost: 4 }));
        assert_eq!(dst.buffers[0].data, vec![0.0; 4]);
    }

    #[test]
    fn position_follows_the_clock_and_open_twice_is_busy() {
        let clk = ManualClock::new();
        let mut d = LoopbackDriver::with_clock(clk.as_fn());
        let mut out = d.open(OUT_UID, &cfg()).unwrap();
        let mut inp = d.open(IN_UID, &cfg()).unwrap();
        assert_eq!(d.open(IN_UID, &cfg()).err(), Some(DriverError::Busy));
        inp.stop();
        assert!(d.open(IN_UID, &cfg()).is_ok());
        out.start().unwrap();
        clk.advance(500_000_000);
        assert_eq!(out.position().hw_frames, 24_000);
        assert_eq!(out.latency().output, 4);
        out.stop();
        assert!(d.open(OUT_UID, &cfg()).is_ok());
    }

    #[test]
    fn output_appears_after_exactly_the_device_latency() {
        let mut d = LoopbackDriver::with_clock(ManualClock::new().as_fn());
        let mut out = d.open(OUT_UID, &cfg()).unwrap();
        let mut inp = d.open(IN_UID, &cfg()).unwrap();
        let lat = out.latency().output as usize;
        assert_eq!(lat, 4);
        for q in 0..3 {
            let v: Vec<f32> = (1..=4).map(|i| (q * 4 + i) as f32).collect();
            out.write_mix(4, &bl(&[&v, &v])).unwrap();
        }
        let mut dst = bl(&[&[9.0; 4], &[9.0; 4]]);
        inp.read_input(4, &mut dst).unwrap();
        assert_eq!(dst.buffers[0].data, vec![0.0; lat]);
        inp.read_input(4, &mut dst).unwrap();
        assert_eq!(dst.buffers[0].data, vec![1.0, 2.0, 3.0, 4.0]);
        inp.read_input(4, &mut dst).unwrap();
        assert_eq!(dst.buffers[1].data, vec![5.0, 6.0, 7.0, 8.0]);
    }

    #[test]
    fn two_outputs_into_one_input_are_summed() {
        let mut d = LoopbackDriver::with_clock(ManualClock::new().as_fn());
        let mut a = d.open(OUT_UID, &cfg()).unwrap();
        let mut b = d.open(OUT_UID, &cfg()).unwrap();
        let mut inp = d.open(IN_UID, &cfg()).unwrap();
        a.write_mix(4, &bl(&[&[1.0; 4], &[2.0; 4]])).unwrap();
        b.write_mix(4, &bl(&[&[10.0; 4], &[20.0; 4]])).unwrap();
        let mut dst = bl(&[&[9.0; 4], &[9.0; 4]]);
        inp.read_input(4, &mut dst).unwrap();
        assert_eq!(dst.buffers[0].data, vec![0.0; 4]);
        inp.read_input(4, &mut dst).unwrap();
        assert_eq!(dst.buffers[0].data, vec![11.0; 4]);
        assert_eq!(dst.buffers[1].data, vec![22.0; 4]);
    }

    #[test]
    fn a_reader_that_falls_behind_causes_an_overflow_xrun() {
        let mut d = LoopbackDriver::with_clock(ManualClock::new().as_fn());
        let mut out = d.open(OUT_UID, &cfg()).unwrap();
        let _inp = d.open(IN_UID, &cfg()).unwrap();
        let src = bl(&[&[1.0; 4], &[1.0; 4]]);
        // cap is 16 quanta = 64 frames; the pre-roll holds 4 and each write adds 4, so the 16th write reaches 68
        for _ in 0..15 {
            out.write_mix(4, &src).unwrap();
        }
        assert_eq!(out.write_mix(4, &src), Err(Xrun { frames_lost: 4 }));
    }
}
