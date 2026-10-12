//! The test rig: one device on a driver plus a model of the server's IO cycle.
//!
//! The model follows docs/audio-architecture.md, "IO cycle": a cycle wakes at the host time of its first frame; the
//! mix a client renders in cycle `k` is written to the device in cycle `k + 1` (the graph quantum is the server's own
//! buffer), and input read in cycle `k` is delivered to the client in cycle `k + 1`. That is exactly the "+ quantum"
//! of the HAL latency properties (`bifrost_audiod::latency::latency_frames`), so a measurement through the rig can be
//! compared with what the HAL reports.
//!
//! Time comes from a `TimeSource`: the `ManualTime` of the null driver (and of a loopback on request), which only moves
//! when the benchmark waits for the next cycle, or `RealTime`, `CLOCK_MONOTONIC_RAW`.

use crate::args::DriverSpec;
use crate::json::{hostname, Json, Report};
use crate::schedule::Timeline;
use crate::stats::Summary;
use bifrost_audio_alsa::sys::now_raw_ns;
use bifrost_audio_alsa::AlsaDriver;
use bifrost_audio_driver::loopback::{LoopbackDriver, IN_UID, OUT_UID};
use bifrost_audio_driver::null::{ClockFn, ManualClock, NullDriver, NullHandle};
use bifrost_audio_driver::{DeviceDesc, DeviceIo, Driver, IoConfig, Latency};
use bifrost_audio_types::{BufferList, Scope};
use bifrost_audiod::latency::{latency_frames, safety_offset};
use std::sync::Arc;

/// A host clock in nanoseconds that the benchmark can wait on.
pub trait TimeSource: Send + Sync {
    fn now_ns(&self) -> u64;
    /// Returns when the clock has reached `ns` (immediately if it already has).
    fn wait_until(&self, ns: u64);
    fn is_manual(&self) -> bool;
}

/// A clock that moves only when waited on: waiting for `ns` sets it to `ns`.
pub struct ManualTime(pub ManualClock);

impl TimeSource for ManualTime {
    fn now_ns(&self) -> u64 {
        self.0.now()
    }
    fn wait_until(&self, ns: u64) {
        let now = self.0.now();
        if ns > now {
            self.0.advance(ns - now);
        }
    }
    fn is_manual(&self) -> bool {
        true
    }
}

/// `CLOCK_MONOTONIC_RAW`: sleeps until shortly before the deadline, then spins.
pub struct RealTime;

impl TimeSource for RealTime {
    fn now_ns(&self) -> u64 {
        now_raw_ns()
    }
    fn wait_until(&self, ns: u64) {
        loop {
            let now = now_raw_ns();
            if now >= ns {
                return;
            }
            let left = ns - now;
            if left > 1_500_000 {
                std::thread::sleep(std::time::Duration::from_nanos(left - 1_000_000));
            } else {
                std::hint::spin_loop();
            }
        }
    }
    fn is_manual(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct RigOptions {
    pub driver: DriverSpec,
    pub rate: u32,
    pub quantum: u32,
    pub device: Option<String>,
    /// Open the capture side too.
    pub input: bool,
    /// Run a loopback on a manual clock (the null driver is always manual, alsa never).
    pub manual: bool,
}

pub struct Rig {
    pub time: Arc<dyn TimeSource>,
    pub rate: u32,
    pub quantum: u32,
    pub tl: Timeline,
    pub driver_label: String,
    pub out_lat: Latency,
    pub in_lat: Latency,
    /// Device xruns seen by `submit` and `capture`.
    pub xruns: u64,
    out_ch: u32,
    dev: Box<dyn DeviceIo>,
    cap: Option<Box<dyn DeviceIo>>,
    input: bool,
    cable: Option<(NullHandle, String)>,
    stage: BufferList,
    pending: BufferList,
    scratch_in: BufferList,
    held_in: BufferList,
    written: u64,
    _driver: Box<dyn Driver>,
}

fn find<'a>(descs: &'a [DeviceDesc], uid: &str) -> Result<&'a DeviceDesc, String> {
    descs.iter().find(|d| d.uid == uid).ok_or_else(|| format!("device '{uid}' not found"))
}

impl Rig {
    pub fn open(o: &RigOptions) -> Result<Rig, String> {
        let rate = o.rate as f64;
        let cfg = IoConfig { rate, quantum: o.quantum, input: o.input, output: true };
        let mut cable = None;
        let time: Arc<dyn TimeSource>;
        let mut driver: Box<dyn Driver>;
        let out_uid: String;
        let in_uid: Option<String>;
        match &o.driver {
            DriverSpec::Null => {
                let d = NullDriver::new(vec![NullDriver::stereo("null0", rate)]);
                let h = d.handle();
                h.clock().advance(1_000_000_000);
                time = Arc::new(ManualTime(h.clock().clone()));
                if o.input {
                    // A cable from the output to the input: what is written comes straight back.
                    cable = Some((h, "null0".to_string()));
                }
                driver = Box::new(d);
                out_uid = "null0".to_string();
                in_uid = None;
            }
            DriverSpec::Loopback => {
                let clock_fn: ClockFn;
                if o.manual {
                    let c = ManualClock::new();
                    c.advance(1_000_000_000);
                    clock_fn = c.as_fn();
                    time = Arc::new(ManualTime(c));
                } else {
                    clock_fn = Arc::new(now_raw_ns);
                    time = Arc::new(RealTime);
                }
                driver = Box::new(LoopbackDriver::with_clock(clock_fn));
                out_uid = OUT_UID.to_string();
                in_uid = o.input.then(|| IN_UID.to_string());
            }
            DriverSpec::Alsa(card) => {
                time = Arc::new(RealTime);
                let mut d = AlsaDriver::new();
                let mut descs = Vec::new();
                d.enumerate(&mut descs);
                let prefix = format!("alsa:{card}:");
                let usable = |x: &&DeviceDesc| x.output_channels > 0 && (!o.input || x.input_channels > 0);
                let pick = match &o.device {
                    Some(uid) => descs.iter().find(|x| &x.uid == uid),
                    None => descs.iter().filter(usable).find(|x| x.uid.starts_with(&prefix)),
                };
                let uid = pick.map(|x| x.uid.clone()).ok_or_else(|| format!("no usable device on card '{card}'"))?;
                driver = Box::new(d);
                out_uid = uid;
                in_uid = None;
            }
        }
        let mut descs = Vec::new();
        driver.enumerate(&mut descs);
        let out_desc = find(&descs, &out_uid)?.clone();
        let in_desc = match &in_uid {
            Some(u) => Some(find(&descs, u)?.clone()),
            None => None,
        };
        let mut dev = driver.open(&out_uid, &cfg).map_err(|e| format!("open {out_uid}: {e:?}"))?;
        let mut cap = match &in_uid {
            Some(u) => Some(driver.open(u, &cfg).map_err(|e| format!("open {u}: {e:?}"))?),
            None => None,
        };
        let zero = dev.start().map_err(|e| format!("start: {e:?}"))?;
        if let Some(c) = cap.as_mut() {
            c.start().map_err(|e| format!("start capture: {e:?}"))?;
        }
        let out_lat = dev.latency();
        let in_lat = cap.as_ref().map_or(out_lat, |c| c.latency());
        let out_ch = out_desc.output_channels.clamp(1, 2);
        let in_ch = in_desc.as_ref().map_or(out_desc.input_channels, |d| d.input_channels).clamp(1, 2);
        let q = o.quantum;
        let tl = Timeline::new(zero.sample_time as f64, zero.host_ns, rate, dev.rate_scalar());
        Ok(Rig {
            time,
            rate: o.rate,
            quantum: q,
            tl,
            driver_label: o.driver.label(),
            out_lat,
            in_lat,
            xruns: 0,
            out_ch,
            dev,
            cap,
            input: o.input,
            cable,
            stage: BufferList::silent(out_ch, q),
            pending: BufferList::silent(out_ch, q),
            scratch_in: BufferList::silent(in_ch, q),
            held_in: BufferList::silent(in_ch, q),
            written: 0,
            _driver: driver,
        })
    }

    /// A rig on the null driver (manual clock, a cable from output to input when `input`).
    pub fn null(rate: u32, quantum: u32, input: bool) -> Rig {
        Rig::open(&RigOptions { driver: DriverSpec::Null, rate, quantum, device: None, input, manual: true })
            .expect("the null driver always opens")
    }

    pub fn out_channels(&self) -> usize {
        self.out_ch as usize
    }

    /// Host time at which cycle `cycle` wakes: the host time of its first frame.
    pub fn wake_ns(&self, cycle: u64) -> u64 {
        self.tl.host_of((cycle * self.quantum as u64) as f64)
    }

    /// Waits for the wake of `cycle`; returns how late (ns, signed) the thread got there.
    pub fn wait_cycle(&self, cycle: u64) -> i64 {
        let w = self.wake_ns(cycle);
        self.time.wait_until(w);
        self.time.now_ns() as i64 - w as i64
    }

    /// Output frames the HAL reports between a client's write and the converter: device latency plus the graph
    /// quantum (`latency_frames`) plus the safety offset.
    pub fn out_total_frames(&self) -> u32 {
        latency_frames(&self.out_lat, Scope::Output, self.quantum) + safety_offset(&self.out_lat, Scope::Output)
    }

    /// The same for the input scope.
    pub fn in_total_frames(&self) -> u32 {
        latency_frames(&self.in_lat, Scope::Input, self.quantum) + safety_offset(&self.in_lat, Scope::Input)
    }

    pub fn frames_to_ms(&self, frames: f64) -> f64 {
        frames * 1000.0 / self.rate as f64
    }

    /// Hands the mix rendered in this cycle (one plane per output channel, at least `quantum` frames) to the server
    /// model: the previous cycle's mix is written to the device now, this one next cycle. Returns the device stream
    /// frame at which this mix will start.
    pub fn submit(&mut self, planes: &[Vec<f32>]) -> u64 {
        let q = self.quantum as usize;
        if let Err(_lost) = self.dev.write_mix(self.quantum, &self.pending) {
            self.xruns += 1;
        }
        if let Some((h, uid)) = &self.cable {
            let written: Vec<Vec<f32>> = self.pending.buffers.iter().map(|b| b.data.clone()).collect();
            h.push_input(uid, &written);
        }
        self.written += self.quantum as u64;
        for (dst, src) in self.stage.buffers.iter_mut().zip(planes) {
            dst.data[..q].copy_from_slice(&src[..q]);
        }
        std::mem::swap(&mut self.pending, &mut self.stage);
        self.written
    }

    /// Reads this cycle's input and returns the previous cycle's (silence in the first cycle, and when the capture
    /// side is not open).
    pub fn capture(&mut self) -> &BufferList {
        if !self.input {
            return &self.scratch_in;
        }
        let src = self.cap.as_mut().unwrap_or(&mut self.dev);
        if let Err(_lost) = src.read_input(self.quantum, &mut self.scratch_in) {
            self.xruns += 1;
        }
        std::mem::swap(&mut self.scratch_in, &mut self.held_in);
        &self.scratch_in
    }

    /// A report in the shared shape with this rig's driver, rate and quantum.
    pub fn report(&self, bench: &str, stats: Summary, reported_ms: Option<f64>, dropped: u64, extra: Vec<(String, Json)>) -> Report {
        Report {
            bench: bench.to_string(),
            host: hostname(),
            driver: self.driver_label.clone(),
            rate: self.rate,
            quantum: self.quantum,
            stats,
            reported_ms,
            xruns: self.xruns,
            dropped,
            extra,
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.dev.stop();
        if let Some(c) = self.cap.as_mut() {
            c.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_rig_timeline_and_latency() {
        let r = Rig::null(50_000, 64, false);
        assert!(r.time.is_manual());
        // The device started at 1 s on the manual clock.
        assert_eq!(r.tl.zero_host_ns, 1_000_000_000);
        // 2 cycles = 128 samples = 2.56 ms at 50 kHz.
        assert_eq!(r.wake_ns(2), 1_002_560_000);
        // Null driver: output latency 0, safety = quantum; HAL adds the quantum.
        assert_eq!((r.out_lat.output, r.out_lat.output_safety), (0, 64));
        assert_eq!(r.out_total_frames(), 128);
        assert_eq!(r.in_total_frames(), 64);
        assert_eq!(r.frames_to_ms(50.0), 1.0);
        assert_eq!(r.out_channels(), 2);
    }

    #[test]
    fn wait_cycle_moves_the_manual_clock() {
        let r = Rig::null(50_000, 64, false);
        assert_eq!(r.wait_cycle(3), 0);
        assert_eq!(r.time.now_ns(), r.wake_ns(3));
        // Waiting for a cycle in the past does not move time back, and reports the lateness.
        assert_eq!(r.wait_cycle(1), (r.wake_ns(3) - r.wake_ns(1)) as i64);
    }

    #[test]
    fn submit_writes_one_cycle_late() {
        let mut r = Rig::null(50_000, 4, false);
        let a = vec![vec![1.0f32; 4], vec![1.0f32; 4]];
        let b = vec![vec![2.0f32; 4], vec![2.0f32; 4]];
        // Cycle 0: silence goes to the device, `a` will start at frame 4.
        assert_eq!(r.submit(&a), 4);
        assert_eq!(r.submit(&b), 8);
        assert_eq!(r.xruns, 0);
    }

    #[test]
    fn unknown_alsa_card_is_an_error() {
        let e = Rig::open(&RigOptions { driver: DriverSpec::Alsa("nonexistent-card".into()), rate: 48_000, quantum: 64, device: None, input: false, manual: false });
        assert!(e.is_err());
    }

    #[test]
    fn loopback_on_a_manual_clock() {
        let r = Rig::open(&RigOptions { driver: DriverSpec::Loopback, rate: 50_000, quantum: 64, device: None, input: true, manual: true }).unwrap();
        assert!(r.time.is_manual());
        // Playback side: driver latency = quantum, + graph quantum; capture side: graph quantum.
        assert_eq!(r.out_total_frames(), 128);
        assert_eq!(r.in_total_frames(), 64);
        assert_eq!(r.driver_label, "loopback");
    }
}
