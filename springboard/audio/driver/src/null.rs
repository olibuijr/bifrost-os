//! `NullDriver`: devices on a manual clock for deterministic host tests (owner: task 05). `advance(ns)` moves every
//! device's hardware pointer by the frames of that time at its rate times (1 + drift_ppm / 1e6). The state sits
//! behind a mutex: this is a test driver, not a real-time one.

use crate::{
    ControlDesc, ControlId, ControlValue, DeviceDesc, DeviceIo, Driver, DriverError, DriverEvent, HwPosition, IoConfig,
    Latency, Xrun, ZeroTimeStamp,
};
use bifrost_audio_types::{BufferList, StreamFormat};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A host clock in nanoseconds.
pub type ClockFn = Arc<dyn Fn() -> u64 + Send + Sync>;

/// A clock that only moves when told to.
#[derive(Debug, Clone, Default)]
pub struct ManualClock(Arc<AtomicU64>);

impl ManualClock {
    pub fn new() -> ManualClock {
        ManualClock::default()
    }
    pub fn now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
    pub fn advance(&self, ns: u64) {
        self.0.fetch_add(ns, Ordering::SeqCst);
    }
    pub fn as_fn(&self) -> ClockFn {
        let c = self.clone();
        Arc::new(move || c.now())
    }
}

/// Frames a device at `rate` with `drift_ppm` has run in `dt_ns`, rounded down (exact integer arithmetic).
pub fn frames_in(dt_ns: u64, rate: u32, drift_ppm: i32) -> u64 {
    let ppm = (1_000_000i64 + drift_ppm as i64) as u128;
    (dt_ns as u128 * rate as u128 * ppm / 1_000_000_000_000u128) as u64
}

#[derive(Default)]
struct NullDev {
    desc: Option<DeviceDesc>,
    rate: u32,
    opened: bool,
    running: bool,
    start_ns: u64,
    drift_ppm: i32,
    starts: u32,
    stops: u32,
    written: Vec<Vec<f32>>,
    input: Vec<VecDeque<f32>>,
    xrun_next_write: Option<u32>,
}

#[derive(Default)]
struct State {
    devs: HashMap<String, NullDev>,
    order: Vec<String>,
    events: Vec<DriverEvent>,
}

/// The test's handle on a `NullDriver` (clone freely).
#[derive(Clone)]
pub struct NullHandle {
    clock: ManualClock,
    st: Arc<Mutex<State>>,
}

impl NullHandle {
    pub fn clock(&self) -> &ManualClock {
        &self.clock
    }
    /// Moves the manual clock (and so every running device's hardware pointer).
    pub fn advance(&self, ns: u64) {
        self.clock.advance(ns);
    }
    pub fn set_drift_ppm(&self, uid: &str, ppm: i32) {
        self.st.lock().unwrap().devs.get_mut(uid).unwrap().drift_ppm = ppm;
    }
    /// Everything `write_mix` has received, per channel.
    pub fn written(&self, uid: &str) -> Vec<Vec<f32>> {
        self.st.lock().unwrap().devs[uid].written.clone()
    }
    /// Queues capture data (per channel) for `read_input`.
    pub fn push_input(&self, uid: &str, planes: &[Vec<f32>]) {
        let mut s = self.st.lock().unwrap();
        let d = s.devs.get_mut(uid).unwrap();
        d.input.resize_with(planes.len(), Default::default);
        for (q, p) in d.input.iter_mut().zip(planes) {
            q.extend(p.iter().copied());
        }
    }
    /// The next `write_mix` fails with an xrun of `frames` frames.
    pub fn inject_xrun(&self, uid: &str, frames: u32) {
        self.st.lock().unwrap().devs.get_mut(uid).unwrap().xrun_next_write = Some(frames);
    }
    pub fn push_event(&self, e: DriverEvent) {
        self.st.lock().unwrap().events.push(e);
    }
    pub fn is_open(&self, uid: &str) -> bool {
        self.st.lock().unwrap().devs[uid].opened
    }
    pub fn is_running(&self, uid: &str) -> bool {
        self.st.lock().unwrap().devs[uid].running
    }
    /// (start calls, stop calls).
    pub fn start_stop_counts(&self, uid: &str) -> (u32, u32) {
        let s = self.st.lock().unwrap();
        (s.devs[uid].starts, s.devs[uid].stops)
    }
}

pub struct NullDriver {
    h: NullHandle,
}

impl NullDriver {
    pub fn new(devices: Vec<DeviceDesc>) -> NullDriver {
        let mut st = State::default();
        for d in devices {
            let rate = d.rates.first().map(|r| r.0.round() as u32).unwrap_or(48_000);
            st.order.push(d.uid.clone());
            st.devs.insert(d.uid.clone(), NullDev { desc: Some(d), rate, ..Default::default() });
        }
        NullDriver { h: NullHandle { clock: ManualClock::new(), st: Arc::new(Mutex::new(st)) } }
    }

    /// One stereo in/out device at `rate`, buffer range 16..=4096.
    pub fn stereo(uid: &str, rate: f64) -> DeviceDesc {
        DeviceDesc {
            uid: uid.into(),
            name: uid.into(),
            manufacturer: "BifrOSt".into(),
            transport: "virtual".into(),
            input_channels: 2,
            output_channels: 2,
            rates: vec![(rate, rate)],
            buffer_frames_range: (16, 4096),
            clock_domain: 0,
        }
    }

    pub fn handle(&self) -> NullHandle {
        self.h.clone()
    }
}

impl Driver for NullDriver {
    fn name(&self) -> &str {
        "null"
    }

    fn enumerate(&mut self, out: &mut Vec<DeviceDesc>) {
        let s = self.h.st.lock().unwrap();
        out.extend(s.order.iter().filter_map(|u| s.devs[u].desc.clone()));
    }

    fn open(&mut self, uid: &str, cfg: &IoConfig) -> Result<Box<dyn DeviceIo>, DriverError> {
        let mut s = self.h.st.lock().unwrap();
        let d = s.devs.get_mut(uid).ok_or(DriverError::NotFound)?;
        if d.opened {
            return Err(DriverError::Busy);
        }
        let desc = d.desc.clone().unwrap();
        if !desc.rates.iter().any(|r| cfg.rate >= r.0 && cfg.rate <= r.1) {
            return Err(DriverError::Unsupported(format!("rate {}", cfg.rate)));
        }
        d.opened = true;
        d.rate = cfg.rate.round() as u32;
        d.written = vec![Vec::new(); if cfg.output { desc.output_channels as usize } else { 0 }];
        Ok(Box::new(NullIo {
            uid: uid.into(),
            h: self.h.clone(),
            cfg: *cfg,
            in_ch: desc.input_channels,
            out_ch: desc.output_channels,
        }))
    }

    fn poll_events(&mut self, out: &mut Vec<DriverEvent>) {
        out.append(&mut self.h.st.lock().unwrap().events);
    }

    fn controls(&mut self, _uid: &str) -> Vec<ControlDesc> {
        Vec::new()
    }

    fn set_control(&mut self, _uid: &str, _id: ControlId, _v: ControlValue) -> Result<(), DriverError> {
        Err(DriverError::Unsupported("the null driver has no controls".into()))
    }
}

struct NullIo {
    uid: String,
    h: NullHandle,
    cfg: IoConfig,
    in_ch: u32,
    out_ch: u32,
}

impl DeviceIo for NullIo {
    fn start(&mut self) -> Result<ZeroTimeStamp, DriverError> {
        let now = self.h.clock.now();
        let mut s = self.h.st.lock().unwrap();
        let d = s.devs.get_mut(&self.uid).unwrap();
        d.running = true;
        d.start_ns = now;
        d.starts += 1;
        Ok(ZeroTimeStamp { sample_time: 0, host_ns: now })
    }

    fn position(&mut self) -> HwPosition {
        let now = self.h.clock.now();
        let s = self.h.st.lock().unwrap();
        let d = &s.devs[&self.uid];
        let hw = if d.running { frames_in(now - d.start_ns, d.rate, d.drift_ppm) } else { 0 };
        HwPosition { hw_frames: hw, host_ns: now, running: d.running }
    }

    fn read_input(&mut self, frames: u32, dst: &mut BufferList) -> Result<(), Xrun> {
        let mut s = self.h.st.lock().unwrap();
        let d = s.devs.get_mut(&self.uid).unwrap();
        for (c, b) in dst.buffers.iter_mut().enumerate() {
            let n = (frames as usize).min(b.data.len());
            for v in &mut b.data[..n] {
                *v = d.input.get_mut(c).and_then(|q| q.pop_front()).unwrap_or(0.0);
            }
        }
        Ok(())
    }

    fn write_mix(&mut self, frames: u32, src: &BufferList) -> Result<(), Xrun> {
        let mut s = self.h.st.lock().unwrap();
        let d = s.devs.get_mut(&self.uid).unwrap();
        if let Some(lost) = d.xrun_next_write.take() {
            return Err(Xrun { frames_lost: lost });
        }
        for (w, b) in d.written.iter_mut().zip(&src.buffers) {
            w.extend_from_slice(&b.data[..frames as usize]);
        }
        Ok(())
    }

    fn stop(&mut self) {
        let mut s = self.h.st.lock().unwrap();
        let d = s.devs.get_mut(&self.uid).unwrap();
        d.running = false;
        d.opened = false;
        d.stops += 1;
    }

    fn latency(&self) -> Latency {
        Latency { input: 0, output: 0, input_safety: 0, output_safety: self.cfg.quantum }
    }

    fn physical_formats(&self) -> (Option<StreamFormat>, Option<StreamFormat>) {
        (
            self.cfg.input.then(|| StreamFormat::float32(self.cfg.rate, self.in_ch)),
            self.cfg.output.then(|| StreamFormat::float32(self.cfg.rate, self.out_ch)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_types::Buffer;

    fn cfg() -> IoConfig {
        IoConfig { rate: 48_000.0, quantum: 256, input: true, output: true }
    }

    fn bl(ch: usize, data: &[f32]) -> BufferList {
        BufferList { buffers: (0..ch).map(|_| Buffer { channels: 1, data: data.to_vec() }).collect() }
    }

    #[test]
    fn frames_in_is_exact() {
        assert_eq!(frames_in(1_000_000_000, 48_000, 0), 48_000);
        // 48000 * 1.0002 = 48009.6, rounded down.
        assert_eq!(frames_in(1_000_000_000, 48_000, 200), 48_009);
        assert_eq!(frames_in(1_000_000_000, 48_000, -200), 47_990);
    }

    #[test]
    fn clock_moves_hw_position_only_when_advanced() {
        let mut d = NullDriver::new(vec![NullDriver::stereo("n0", 48_000.0)]);
        let h = d.handle();
        let mut io = d.open("n0", &cfg()).unwrap();
        h.advance(7);
        let z = io.start().unwrap();
        assert_eq!((z.sample_time, z.host_ns), (0, 7));
        assert_eq!(io.position().hw_frames, 0);
        h.advance(1_000_000_000);
        let p = io.position();
        assert_eq!((p.hw_frames, p.host_ns, p.running), (48_000, 1_000_000_007, true));
        h.set_drift_ppm("n0", 200);
        assert_eq!(io.position().hw_frames, 48_009);
    }

    #[test]
    fn write_mix_records_and_second_open_is_busy() {
        let mut d = NullDriver::new(vec![NullDriver::stereo("n0", 48_000.0)]);
        let h = d.handle();
        let mut io = d.open("n0", &cfg()).unwrap();
        assert_eq!(d.open("n0", &cfg()).err(), Some(DriverError::Busy));
        assert_eq!(d.open("x", &cfg()).err(), Some(DriverError::NotFound));
        io.start().unwrap();
        io.write_mix(2, &bl(2, &[0.5, -0.5, 9.0])).unwrap();
        assert_eq!(h.written("n0"), vec![vec![0.5, -0.5], vec![0.5, -0.5]]);
        h.inject_xrun("n0", 3);
        assert_eq!(io.write_mix(2, &bl(2, &[1.0, 1.0])), Err(Xrun { frames_lost: 3 }));
        io.write_mix(1, &bl(2, &[0.25])).unwrap();
        assert_eq!(h.written("n0")[0], vec![0.5, -0.5, 0.25]);
        io.stop();
        assert!(!h.is_open("n0"));
        assert_eq!(h.start_stop_counts("n0"), (1, 1));
    }

    #[test]
    fn read_input_returns_queued_then_silence() {
        let mut d = NullDriver::new(vec![NullDriver::stereo("n0", 48_000.0)]);
        let h = d.handle();
        let mut io = d.open("n0", &cfg()).unwrap();
        h.push_input("n0", &[vec![1.0, 2.0], vec![3.0, 4.0]]);
        let mut dst = bl(2, &[9.0, 9.0, 9.0]);
        io.read_input(3, &mut dst).unwrap();
        assert_eq!(dst.buffers[0].data, vec![1.0, 2.0, 0.0]);
        assert_eq!(dst.buffers[1].data, vec![3.0, 4.0, 0.0]);
    }

    #[test]
    fn unsupported_rate_is_refused() {
        let mut d = NullDriver::new(vec![NullDriver::stereo("n0", 48_000.0)]);
        let c = IoConfig { rate: 44_100.0, ..cfg() };
        assert!(matches!(d.open("n0", &c), Err(DriverError::Unsupported(_))));
    }
}
