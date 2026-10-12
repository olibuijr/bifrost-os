//! The runtime: the one engine of the process, its driver and the notifier thread.
//!
//! The driver is a HAL client (`bifrost_audio_hal`): one IO procedure on the default output device, 2 capture and 2
//! playback channels, which runs `Engine::cycle` each period. `BIFROST_JACK_DUMMY=1` selects a wall-clock driver
//! instead (like the reference server's dummy backend) so the ABI can be exercised without a server;
//! `BIFROST_JACK_RATE` and `BIFROST_JACK_FRAMES` set its rate and period.

use crate::engine::{Call, Engine, Shared, MAX_FRAMES};
use bifrost_audio_hal::{Device, IoProcConfig, IoProcId, Role, System};
use bifrost_audio_types::{props, AudioError, Buffer, BufferList, PropertyAddress, PropertyValue, TimeStampFlags};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::*};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Capture and playback channels the HAL driver opens.
pub const PHYS_CHANNELS: usize = 2;

pub fn log(category: &str, msg: &str) {
    eprintln!("[is.olibuijr.audio:{category}] jack: {msg}");
}

/// Microseconds on the monotonic raw clock (the host time base of the HAL timestamps, in us).
pub fn now_usecs() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid out pointer; clock_gettime is a vDSO call on this clock.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &mut ts) };
    ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1000
}

pub fn run_calls(calls: Vec<Call>) {
    for c in calls {
        c();
    }
}

enum Driver {
    Hal { device: Device, id: IoProcId },
    Dummy { stop: Arc<AtomicBool>, thread: Option<JoinHandle<()>> },
}

pub struct Runtime {
    pub sh: Arc<Shared>,
    pub engine: Mutex<Engine>,
    driver: Mutex<Option<Driver>>,
    stop: Arc<AtomicBool>,
    notifier: Mutex<Option<JoinHandle<()>>>,
    /// Cycles that found the engine busy (counted as xruns by the notifier).
    missed: AtomicU32,
    pub clients: AtomicU32,
    pub handles: Mutex<std::collections::HashMap<u32, Box<crate::ffi::PortObj>>>,
}

impl Runtime {
    fn new(rate: u32, frames: u32) -> Arc<Runtime> {
        let sh = Arc::new(Shared::new(rate, frames));
        Arc::new(Runtime {
            engine: Mutex::new(Engine::new(sh.clone(), PHYS_CHANNELS, PHYS_CHANNELS)),
            sh,
            driver: Mutex::new(None),
            stop: Arc::new(AtomicBool::new(false)),
            notifier: Mutex::new(None),
            missed: AtomicU32::new(0),
            clients: AtomicU32::new(0),
            handles: Mutex::new(Default::default()),
        })
    }

    /// Runs one device period of `frames` frames (chunked to the port buffer size) at host time `usecs`. Real-time:
    /// no allocation, no blocking lock.
    pub fn run_cycle(&self, frames: usize, input: &BufferList, output: &mut BufferList, usecs: u64) {
        let started = now_usecs();
        let Ok(mut e) = self.engine.try_lock() else {
            self.missed.fetch_add(1, Relaxed);
            for b in &mut output.buffers {
                b.data.iter_mut().for_each(|s| *s = 0.0);
            }
            return;
        };
        // SAFETY: pthread_self has no preconditions.
        self.sh.rt_thread.store(unsafe { libc::pthread_self() } as u64, Relaxed);
        let rate = self.sh.sample_rate.load(Relaxed).max(1) as u64;
        let mut off = 0;
        while off < frames {
            let n = (frames - off).min(MAX_FRAMES);
            e.cycle(n, input, output, off, usecs + off as u64 * 1_000_000 / rate);
            off += n;
        }
        drop(e);
        let period = self.sh.period_usecs.load(Relaxed).max(1);
        let load = (now_usecs().saturating_sub(started) * 10_000 / period).min(u32::MAX as u64);
        self.sh.load.store(load as u32, Relaxed);
    }

    /// Asks the device for another period size.
    pub fn set_buffer_size(&self, frames: u32) -> i32 {
        if !(16..=MAX_FRAMES as u32).contains(&frames) || !frames.is_power_of_two() {
            return libc::EINVAL;
        }
        match self.driver.lock().ok().as_deref() {
            Some(Some(Driver::Hal { device, .. })) => match device.set(PropertyAddress::global(props::BUFFER_FRAMES), PropertyValue::U32(frames)) {
                Ok(()) => 0,
                Err(_) => libc::EIO,
            },
            Some(Some(Driver::Dummy { .. })) => {
                self.sh.buffer_size.store(frames, Release);
                0
            }
            _ => libc::EINVAL,
        }
    }

    fn start_notifier(self: &Arc<Self>) {
        let rt = self.clone();
        let t = std::thread::Builder::new()
            .name("jack-notify".into())
            .spawn(move || {
                while !rt.stop.load(Acquire) {
                    std::thread::sleep(Duration::from_millis(20));
                    let calls = match rt.engine.lock() {
                        Ok(mut e) => {
                            e.xruns += rt.missed.swap(0, Relaxed);
                            e.take_events()
                        }
                        Err(_) => vec![],
                    };
                    run_calls(calls);
                }
            })
            .ok();
        *self.notifier.lock().unwrap() = t;
    }

    /// Stops the driver and the notifier.
    pub fn shutdown(&self) {
        self.stop.store(true, Release);
        let d = self.driver.lock().ok().and_then(|mut d| d.take());
        match d {
            Some(Driver::Hal { device, id }) => {
                let _ = device.stop(id);
                let _ = device.remove_io_proc(id);
            }
            Some(Driver::Dummy { stop, thread }) => {
                stop.store(true, Release);
                if let Some(t) = thread {
                    let _ = t.join();
                }
            }
            None => {}
        }
        if let Some(t) = self.notifier.lock().ok().and_then(|mut n| n.take()) {
            if t.thread().id() != std::thread::current().id() {
                let _ = t.join();
            }
        }
    }
}

fn blank(ch: usize, frames: usize) -> BufferList {
    BufferList { buffers: (0..ch).map(|_| Buffer { channels: 1, data: vec![0.0; frames] }).collect() }
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn hal_open() -> Result<(u32, u32, Device), AudioError> {
    let system = System::connect("jack")?;
    let device = system.default_device(Role::Output)?;
    let rate = match device.get(PropertyAddress::global(props::NOMINAL_RATE)) {
        Ok(PropertyValue::F64(r)) => r as u32,
        Ok(_) => return Err(AudioError::BadValueType),
        Err(AudioError::UnknownProperty) => 48_000,
        Err(e) => return Err(e),
    };
    let frames = match device.get(PropertyAddress::global(props::BUFFER_FRAMES)) {
        Ok(PropertyValue::U32(f)) => f,
        Ok(_) => return Err(AudioError::BadValueType),
        Err(AudioError::UnknownProperty) => 256,
        Err(e) => return Err(e),
    };
    Ok((rate, frames, device))
}

/// Creates the runtime with its driver.
pub fn create() -> Result<Arc<Runtime>, u32> {
    if std::env::var("BIFROST_JACK_DUMMY").map_or(false, |v| v == "1") {
        let (rate, frames) = (env_u32("BIFROST_JACK_RATE", 48_000), env_u32("BIFROST_JACK_FRAMES", 256));
        let rt = Runtime::new(rate, frames);
        let stop = Arc::new(AtomicBool::new(false));
        let (rt2, stop2) = (rt.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("jack-dummy".into())
            .spawn(move || {
                let (mut input, mut output) = (blank(PHYS_CHANNELS, MAX_FRAMES), blank(PHYS_CHANNELS, MAX_FRAMES));
                let mut next = now_usecs();
                while !stop2.load(Acquire) {
                    let frames = (rt2.sh.buffer_size.load(Acquire) as usize).clamp(16, MAX_FRAMES);
                    let rate = rt2.sh.sample_rate.load(Relaxed).max(1) as u64;
                    rt2.run_cycle(frames, &input, &mut output, next);
                    next += frames as u64 * 1_000_000 / rate;
                    let now = now_usecs();
                    if next > now {
                        std::thread::sleep(Duration::from_micros(next - now));
                    } else if now - next > 10 * frames as u64 * 1_000_000 / rate {
                        next = now; // far behind: resynchronise instead of bursting
                    }
                    input.buffers.iter_mut().for_each(|b| b.data.iter_mut().for_each(|s| *s = 0.0));
                }
            })
            .map_err(|_| crate::engine::STATUS_FAILURE)?;
        *rt.driver.lock().unwrap() = Some(Driver::Dummy { stop, thread: Some(thread) });
        rt.start_notifier();
        log("driver", &format!("dummy driver, {rate} Hz, {frames} frames"));
        return Ok(rt);
    }
    let (rate, frames, device) = hal_open().map_err(|e| {
        log("client", &format!("cannot reach the audio server: {e:?}"));
        crate::engine::STATUS_FAILURE | crate::engine::STATUS_SERVER_FAILED
    })?;
    let rt = Runtime::new(rate, frames);
    let rt2 = rt.clone();
    let cfg = IoProcConfig { input_channels: PHYS_CHANNELS as u32, output_channels: PHYS_CHANNELS as u32, buffer_frames: 0 };
    let id = device
        .add_io_proc(
            cfg,
            Box::new(move |ctx, _now, input, _in_ts, output, out_ts| {
                let usecs = if out_ts.flags.contains(TimeStampFlags::HOST_TIME) { out_ts.host_ns / 1000 } else { now_usecs() };
                rt2.run_cycle(ctx.frames as usize, input, output, usecs);
            }),
        )
        .map_err(|_| crate::engine::STATUS_FAILURE | crate::engine::STATUS_SERVER_FAILED)?;
    device.start(id).map_err(|_| crate::engine::STATUS_FAILURE | crate::engine::STATUS_SERVER_FAILED)?;
    *rt.driver.lock().unwrap() = Some(Driver::Hal { device, id });
    rt.start_notifier();
    log("driver", &format!("device io proc, {rate} Hz, {frames} frames"));
    Ok(rt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dummy_driver_runs_cycles_and_stops() {
        std::env::set_var("BIFROST_JACK_DUMMY", "1");
        std::env::set_var("BIFROST_JACK_FRAMES", "64");
        std::env::set_var("BIFROST_JACK_RATE", "48000");
        let rt = create().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        // 64 frames at 48000 Hz = 1.33 ms: far more than 5 periods in 100 ms.
        assert!(rt.engine.lock().unwrap().cycles() > 5);
        assert_eq!(rt.sh.buffer_size.load(Relaxed), 64);
        rt.shutdown();
        let n = rt.engine.lock().unwrap().cycles();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(rt.engine.lock().unwrap().cycles(), n);
    }

    #[test]
    fn set_buffer_size_validates() {
        let rt = Runtime::new(48000, 256);
        assert_eq!(rt.set_buffer_size(100), libc::EINVAL); // not a power of two
        assert_eq!(rt.set_buffer_size(8), libc::EINVAL); // below the minimum
        assert_eq!(rt.set_buffer_size(16384), libc::EINVAL); // above the port buffers
        assert_eq!(rt.set_buffer_size(128), libc::EINVAL); // no driver attached
    }

    #[test]
    fn busy_engine_outputs_silence() {
        let rt = Runtime::new(48000, 8);
        let input = blank(2, 8);
        let mut out = blank(2, 8);
        out.buffers[0].data = vec![0.7; 8];
        let _guard = rt.engine.lock().unwrap();
        // Another thread's cycle finds the lock taken.
        std::thread::scope(|s| {
            s.spawn(|| rt.run_cycle(8, &input, &mut out, 0));
        });
        assert_eq!(out.buffers[0].data, vec![0.0; 8]);
        assert_eq!(rt.missed.load(Relaxed), 1);
    }
}
