//! `--live`: `audio-roundtrip`, `xrun-sweep` and `midi-latency` against the running `bifrost-audiod` through the HAL
//! client (`bifrost-audio-hal`) instead of the in-process model of `rig`. The bench adds an IO procedure on the chosen
//! device, and everything it measures comes from the HAL's `TimeStamp`s. The IO procedures only store into
//! preallocated atomics (`StampLog`), so they keep the real-time rules (no allocation, no locks).
//!
//! Time lines (`bifrost_audio_hal::IoProc`): the input of a call was captured one buffer before the call and the
//! output plays one buffer after it. A sample written at output sample time `S_out` and seen at input sample time
//! `S_in` crossed the device path in `S_in - S_out` frames, and the client saw it `S_in - S_out + 2 * buffer` frames
//! after writing it. That client view is what the HAL latency properties predict (`LATENCY + SAFETY_OFFSET` of both
//! scopes, `LATENCY` including the server's quantum), the same quantity as the in-process `audio-roundtrip`.
//!
//! `midi-latency`: `bifrost-midid` has no client socket yet, so the notes go through the same `bifrost_midi::hub`
//! in this process (`extra.midi_path = "hub-in-process"`); the Bench Instrument renders inside the bench's own IO
//! procedure on the real server, and the impulse is timed from the HAL's output timestamp.

use crate::args::DriverSpec;
use crate::instrument::{BenchInstrument, BenchMidi};
use crate::json::{hostname, Json, Report};
use crate::midi_latency::{self, note_samples};
use crate::rig::{RealTime, RigOptions, TimeSource};
use crate::roundtrip;
use crate::schedule::{EventScheduler, Timeline};
use crate::stats::{summarize, Summary};
use crate::sweep;
use bifrost_audio_hal::{Device, IoContext, IoProc, IoProcConfig, Role, System};
use bifrost_audio_types::{props, BufferList, PropertyAddress, PropertyValue, Scope, TimeStamp};
use bifrost_audio_units::{AudioUnit, Block, MidiEvent, RenderConfig};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The client name the bench says hello with.
pub const CLIENT: &str = "bifrost-audio-bench";
/// Calls of the IO procedure before the first impulse (the device and the thread settle).
pub const WARMUP_CYCLES: u64 = 8;
/// Output planes the bench renders (stereo; the planes live on the stack in `render_into`).
const MAX_PLANES: usize = 8;
/// How long to keep listening after the last impulse or note.
const TAIL: Duration = Duration::from_millis(200);

/// The largest latency a live measurement may report (ms). Anything outside `0..=MAX_SANE_MS` is a clock or unit
/// mix-up in the bench, not a latency, and fails the run.
pub const MAX_SANE_MS: f64 = 1000.0;
/// The error text (and prefix) of an `audio-roundtrip --live` run that heard none of its first impulses; `main`
/// exits with `EXIT_NO_LOOPBACK` for it so `tools/audio-bench.py` can show the bench as skipped.
pub const NO_LOOPBACK: &str = "no loopback: connect line out to line in, or run with --aloop";
pub const EXIT_NO_LOOPBACK: i32 = 4;
/// Impulses that must be heard among the first ones sent, else there is no loopback.
pub const LOOPBACK_PROBE: usize = 8;

/// Fails when any latency is not a finite value in `0..=MAX_SANE_MS`.
pub fn check_latencies_ms(ms: &[f64]) -> Result<(), String> {
    match ms.iter().enumerate().find(|(_, v)| !v.is_finite() || **v < 0.0 || **v > MAX_SANE_MS) {
        Some((i, v)) => Err(format!("measurement error: sample {i} is {v} ms, outside 0..{MAX_SANE_MS} ms (the send and receive stamps are not on one clock)")),
        None => Ok(()),
    }
}

/// True when none of the first `LOOPBACK_PROBE` (or fewer, if fewer were sent) impulses was heard.
pub fn no_loopback(sent: &[f64], found: &[f64]) -> bool {
    let probe = sent.len().min(LOOPBACK_PROBE);
    probe > 0 && pair_times(sent, found)[..probe].iter().all(|p| p.is_none())
}

/// Host time (the server's `CLOCK_MONOTONIC_RAW`, the clock of `TimeStamp::host_ns`) at which the frame
/// `offset + device_frames` after the start of an output buffer plays. Uses the nominal rate: the offset is a few
/// hundred frames, and a (possibly noisy) `rate_scalar` must not move it.
pub fn landing_ns(out_host_ns: u64, offset: u32, device_frames: f64, rate: f64) -> u64 {
    out_host_ns + ((offset as f64 + device_frames) * 1e9 / rate).round() as u64
}

/// A fixed-capacity log that one real-time thread appends to and other threads read: no allocation, no lock.
pub struct StampLog {
    vals: Box<[AtomicU64]>,
    len: AtomicUsize,
}

impl StampLog {
    pub fn new(capacity: usize) -> StampLog {
        StampLog { vals: (0..capacity).map(|_| AtomicU64::new(0)).collect(), len: AtomicUsize::new(0) }
    }

    /// Appends `v` (one writer only); false when the log is full.
    pub fn push(&self, v: u64) -> bool {
        let n = self.len.load(Ordering::Relaxed);
        if n >= self.vals.len() {
            return false;
        }
        self.vals[n].store(v, Ordering::Relaxed);
        self.len.store(n + 1, Ordering::Release);
        true
    }

    pub fn push_f64(&self, v: f64) -> bool {
        self.push(v.to_bits())
    }

    pub fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn values(&self) -> Vec<u64> {
        let n = self.len();
        self.vals[..n].iter().map(|v| v.load(Ordering::Relaxed)).collect()
    }

    pub fn values_f64(&self) -> Vec<f64> {
        self.values().into_iter().map(f64::from_bits).collect()
    }
}

/// The `driver` field of a live report.
pub fn label(uid: &str) -> String {
    format!("live:{uid}")
}

/// The device to measure: `want` must match a uid exactly; without it the default output, else the first device.
pub fn pick(uids: &[String], want: Option<&str>, default: Option<usize>) -> Result<usize, String> {
    if uids.is_empty() {
        return Err("bifrost-audiod has no devices".to_string());
    }
    match want {
        Some(w) => uids.iter().position(|u| u == w).ok_or_else(|| format!("no device '{w}' on bifrost-audiod (have: {})", uids.join(", "))),
        None => Ok(default.filter(|i| *i < uids.len()).unwrap_or(0)),
    }
}

fn uid_of(d: &Device) -> String {
    match d.get(PropertyAddress::global(props::DEVICE_UID)) {
        Ok(PropertyValue::String(s)) if !s.is_empty() => s,
        _ => format!("object:{}", d.id.0),
    }
}

/// The first sample of `plane` at or above `threshold` (absolute value) as a sample time on the input timeline
/// (`start` is the sample time of `plane[0]`), unless it lies within `holdoff` frames of the previous detection.
pub fn detect(plane: &[f32], start: f64, threshold: f32, last: &mut Option<f64>, holdoff: f64) -> Option<f64> {
    let i = plane.iter().position(|s| s.abs() >= threshold)?;
    let at = start + i as f64;
    if last.is_some_and(|l| at - l < holdoff) {
        return None;
    }
    *last = Some(at);
    Some(at)
}

/// For each sent sample time, the first found one in `[sent[i], sent[i + 1])` (the last window is open-ended).
pub fn pair_times(sent: &[f64], found: &[f64]) -> Vec<Option<f64>> {
    sent.iter()
        .enumerate()
        .map(|(i, &s)| {
            let end = sent.get(i + 1).copied().unwrap_or(f64::INFINITY);
            found.iter().copied().find(|&f| f >= s && f < end)
        })
        .collect()
}

/// The round trips as the client sees them, in frames: `found - sent + 2 * buffer` for each paired impulse.
pub fn round_trips(sent: &[f64], found: &[f64], buffer: u32) -> Vec<f64> {
    pair_times(sent, found).iter().zip(sent).filter_map(|(f, s)| f.map(|f| f - s + 2.0 * buffer as f64)).collect()
}

/// Calls of `quantum` frames between two impulses: `interval_ms` rounded up, at least 4 (as in-process).
pub fn interval_cycles(interval_ms: f64, rate: f64, quantum: u32) -> u64 {
    (((interval_ms * rate / 1000.0) / quantum as f64).ceil() as u64).max(4)
}

/// How late each call of the IO procedure came, in frames: its phase against the first call wrapped into
/// `[-q/2, q/2)`, minus the earliest phase (the best call counts as on time). `now_samples` are the sample times of
/// the HAL's `now` stamps.
pub fn wake_late_frames(now_samples: &[f64], quantum: u32) -> Vec<f64> {
    let Some(&first) = now_samples.first() else { return Vec::new() };
    let q = quantum as f64;
    let d: Vec<f64> = now_samples.iter().map(|s| (s - first + q / 2.0).rem_euclid(q) - q / 2.0).collect();
    let min = d.iter().copied().fold(f64::INFINITY, f64::min);
    d.iter().map(|x| x - min).collect()
}

/// Host times of the notes: `note_samples` (stride of 7 frames inside the cycle) from `start_ns`.
pub fn plan_notes(start_ns: u64, count: usize, quantum: u32, rate: f64, interval_ms: f64) -> Vec<u64> {
    note_samples(count, 0, quantum, rate as u32, interval_ms).iter().map(|s| start_ns + (*s as f64 * 1e9 / rate).round() as u64).collect()
}

/// Renders `unit` into the HAL's output planes without allocating (the plane slices live on the stack).
pub fn render_into(unit: &mut dyn AudioUnit, frames: usize, sample_time: f64, midi: &[MidiEvent], out: &mut BufferList) {
    let mut slots: [&mut [f32]; MAX_PLANES] = Default::default();
    let mut n = 0;
    for (slot, b) in slots.iter_mut().zip(out.buffers.iter_mut()) {
        let len = frames.min(b.data.len());
        *slot = &mut b.data[..len];
        n += 1;
    }
    let frames = slots[..n].iter().map(|s| s.len()).min().unwrap_or(0);
    let mut block = Block { frames, sample_time, inputs: &[], outputs: &mut slots[..n], midi };
    unit.render(&mut block);
}

/// The device on the running server.
pub struct LiveDevice {
    pub system: System,
    pub device: Device,
    pub uid: String,
    /// The nominal rate (the `--rate` flag when the server does not report one).
    pub rate: f64,
}

/// What one run of an IO procedure left: the body's result and the server's counts.
struct ProcRun<R> {
    out: R,
    /// Cycles the server counted as missed for the procedure (the activation record).
    xruns: u64,
    /// Overload notifications the server sent for it.
    overloads: u64,
}

impl LiveDevice {
    /// Connects to `bifrost-audiod` and picks the device (`want`: a uid; default: the default output).
    pub fn connect(want: Option<&str>, fallback_rate: u32) -> Result<LiveDevice, String> {
        let system = System::connect_detailed(bifrost_audio_types::control::CONTROL_SOCKET, CLIENT).map_err(|e| format!("cannot reach bifrost-audiod: {e}"))?;
        let devices = system.devices().map_err(|e| format!("bifrost-audiod device list: {e:?}"))?;
        let uids: Vec<String> = devices.iter().map(uid_of).collect();
        let default = system.default_device(Role::Output).ok().and_then(|d| devices.iter().position(|x| x.id == d.id));
        let i = pick(&uids, want, default)?;
        let device = devices[i].clone();
        let rate = match device.get(PropertyAddress::global(props::NOMINAL_RATE)) {
            Ok(PropertyValue::F64(r)) if r > 0.0 => r,
            _ => fallback_rate as f64,
        };
        Ok(LiveDevice { system, device, uid: uids[i].clone(), rate })
    }

    pub fn label(&self) -> String {
        label(&self.uid)
    }

    fn prop_u32(&self, selector: bifrost_audio_types::FourCC, scope: Scope) -> Option<u32> {
        match self.device.get(PropertyAddress::scoped(selector, scope)) {
            Ok(PropertyValue::U32(v)) => Some(v),
            _ => None,
        }
    }

    /// `LATENCY + SAFETY_OFFSET` of one scope as the HAL reports them (None when the server does not).
    pub fn scope_total(&self, scope: Scope) -> Option<u32> {
        Some(self.prop_u32(props::LATENCY, scope)? + self.prop_u32(props::SAFETY_OFFSET, scope)?)
    }

    fn frames_to_ms(&self, frames: f64) -> f64 {
        frames * 1000.0 / self.rate
    }

    /// Adds and starts an IO procedure, runs `body` while it runs, then stops and removes it.
    fn run_proc<R>(&self, cfg: IoProcConfig, f: IoProc, body: impl FnOnce() -> R) -> Result<ProcRun<R>, String> {
        let d = &self.device;
        let id = d.add_io_proc(cfg, f).map_err(|e| format!("add IO proc ({} frames): {e:?}", cfg.buffer_frames))?;
        if let Err(e) = d.start(id) {
            let _ = d.remove_io_proc(id);
            return Err(format!("start IO proc: {e:?}"));
        }
        let out = body();
        let xruns = d.io_proc_xruns(id).unwrap_or(0) as u64;
        let overloads = self.system.overloads(id) as u64;
        let _ = d.stop(id);
        let _ = d.remove_io_proc(id);
        Ok(ProcRun { out, xruns, overloads })
    }

    #[allow(clippy::too_many_arguments)]
    fn report(&self, bench: &str, quantum: u32, stats: Summary, reported_ms: Option<f64>, xruns: u64, dropped: u64, extra: Vec<(String, Json)>) -> Report {
        Report { bench: bench.to_string(), host: hostname(), driver: self.label(), rate: self.rate.round() as u32, quantum, stats, reported_ms, xruns, dropped, extra }
    }
}

/// Polls every 5 ms until `done` or `limit` has passed.
fn wait_for(limit: Duration, done: impl Fn() -> bool) {
    let t0 = Instant::now();
    while !done() && t0.elapsed() < limit {
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn num_or_null(v: Option<f64>) -> Json {
    v.map_or(Json::Null, Json::Num)
}

/// `audio-roundtrip --live`: an impulse on output frame 0 every `interval` and its detection in input channel 0 (a
/// cable or a loopback device between them).
pub fn roundtrip(dev: &LiveDevice, o: &roundtrip::Opts, quantum: u32) -> Result<Report, String> {
    let every = interval_cycles(o.interval_ms, dev.rate, quantum);
    let count = o.count;
    let sent = Arc::new(StampLog::new(count));
    let found = Arc::new(StampLog::new(count * 4 + 16));
    let (s2, f2, threshold) = (sent.clone(), found.clone(), o.threshold);
    let holdoff = (every * quantum as u64) as f64 / 2.0;
    let mut last = None;
    let f: IoProc = Box::new(move |ctx: &IoContext, _now: &TimeStamp, input: &BufferList, in_ts: &TimeStamp, output: &mut BufferList, out_ts: &TimeStamp| {
        let n = ctx.frames as usize;
        if let Some(b) = input.buffers.first() {
            if let Some(at) = detect(&b.data[..n.min(b.data.len())], in_ts.sample_time, threshold, &mut last, holdoff) {
                f2.push_f64(at);
            }
        }
        if ctx.cycle >= WARMUP_CYCLES && (ctx.cycle - WARMUP_CYCLES) % every == 0 && s2.len() < count {
            for b in output.buffers.iter_mut() {
                if let Some(x) = b.data.first_mut() {
                    *x = 1.0;
                }
            }
            s2.push_f64(out_ts.sample_time);
        }
    });
    let expected = Duration::from_secs_f64((WARMUP_CYCLES + count as u64 * every) as f64 * quantum as f64 / dev.rate);
    let cfg = IoProcConfig { input_channels: 1, output_channels: 2, buffer_frames: quantum };
    let s3 = sent.clone();
    let run = dev.run_proc(cfg, f, || {
        wait_for(expected * 2 + Duration::from_secs(3), || s3.len() >= count);
        std::thread::sleep(TAIL + Duration::from_secs_f64(4.0 * quantum as f64 / dev.rate));
    })?;

    let (sent_v, found_v) = (sent.values_f64(), found.values_f64());
    if no_loopback(&sent_v, &found_v) {
        return Err(NO_LOOPBACK.to_string());
    }
    let trips = round_trips(&sent_v, &found_v, quantum);
    let ms: Vec<f64> = trips.iter().map(|f| dev.frames_to_ms(*f)).collect();
    check_latencies_ms(&ms)?;
    let reported = match (dev.scope_total(Scope::Output), dev.scope_total(Scope::Input)) {
        (Some(a), Some(b)) => Some(a + b),
        _ => None,
    };
    let p50_frames = summarize(&trips).map(|s| s.p50);
    let extra = vec![
        ("impulses".to_string(), Json::Uint(count as u64)),
        ("impulses_sent".to_string(), Json::Uint(sent_v.len() as u64)),
        ("threshold".to_string(), Json::Num(o.threshold as f64)),
        ("measured_frames_p50".to_string(), num_or_null(p50_frames)),
        ("device_frames_p50".to_string(), num_or_null(p50_frames.map(|p| p - 2.0 * quantum as f64))),
        ("reported_frames".to_string(), reported.map_or(Json::Null, |r| Json::Uint(r as u64))),
        ("overloads".to_string(), Json::Uint(run.overloads)),
    ];
    let stats = summarize(&ms).unwrap_or_else(Summary::empty);
    let dropped = (count - trips.len().min(count)) as u64;
    Ok(dev.report("audio-roundtrip", quantum, stats, reported.map(|r| dev.frames_to_ms(r as f64)), run.xruns, dropped, extra))
}

/// One quantum of `xrun-sweep --live`: the Bench Instrument renders silence in an output-only IO procedure of
/// `quantum` frames for `seconds`; xruns are the cycles the server counted as missed.
pub fn sweep_quantum(dev: &LiveDevice, quantum: u32, seconds: f64) -> Result<sweep::Run, String> {
    let cycles = ((seconds * dev.rate) / quantum as f64).ceil().max(1.0) as u64;
    let log = Arc::new(StampLog::new(cycles as usize + 64));
    let l2 = log.clone();
    let mut unit = BenchInstrument::new();
    let cfg = RenderConfig { sample_rate: dev.rate, max_frames: quantum, input_channels: 0, output_channels: 2 };
    unit.initialize(&cfg).map_err(|e| e.to_string())?;
    let f: IoProc = Box::new(move |ctx: &IoContext, now: &TimeStamp, _in: &BufferList, _in_ts: &TimeStamp, out: &mut BufferList, out_ts: &TimeStamp| {
        l2.push_f64(now.sample_time);
        render_into(&mut unit, ctx.frames as usize, out_ts.sample_time, &[], out);
    });
    let target = cycles as usize;
    let l3 = log.clone();
    let proc_cfg = IoProcConfig { input_channels: 0, output_channels: 2, buffer_frames: quantum };
    let run = dev.run_proc(proc_cfg, f, || wait_for(Duration::from_secs_f64(seconds * 2.0 + 3.0), || l3.len() >= target))?;
    let seen = log.values_f64();
    let late_ms: Vec<f64> = wake_late_frames(&seen, quantum).iter().map(|f| dev.frames_to_ms(*f)).collect();
    // the activation count and the notifications report the same misses; take the larger rather than add them
    let misses = run.xruns.max(run.overloads);
    Ok(sweep::Run {
        quantum,
        cycles: seen.len() as u64,
        xruns: misses,
        deadline_misses: misses,
        wake_late: summarize(&late_ms).unwrap_or_else(Summary::empty),
        reported_ms: dev.scope_total(Scope::Output).map_or(f64::NAN, |f| dev.frames_to_ms(f as f64)),
    })
}

/// `xrun-sweep --live`: every quantum of `o.quanta` the server accepts; the others are listed as refused.
pub fn sweep(dev: &LiveDevice, o: &sweep::Opts) -> Result<Report, String> {
    let _stress = (o.stress > 0).then(|| sweep::Stress::start(o.stress));
    let mut runs = Vec::new();
    let mut refused = Vec::new();
    for &q in &o.quanta {
        match sweep_quantum(dev, q, o.seconds) {
            Ok(r) => runs.push(r),
            Err(e) => {
                eprintln!("bifrost-audio-bench: quantum {q}: {e}");
                refused.push(Json::Uint(q as u64));
            }
        }
    }
    if runs.is_empty() {
        return Err("bifrost-audiod refused every quantum".to_string());
    }
    let base = RigOptions { driver: DriverSpec::Null, rate: dev.rate.round() as u32, quantum: runs[0].quantum, device: None, input: false, manual: false };
    let mut r = sweep::report(&base, o, &runs);
    r.driver = dev.label();
    r.extra.push(("refused_quanta".to_string(), Json::Arr(refused)));
    Ok(r)
}

/// `midi-latency --live`: note-ons through the hub to the Bench Instrument in the bench's IO procedure. Each note's
/// impulse is placed with the HAL's output timestamp (`Timeline` anchored at it) and heard
/// `LATENCY - quantum + SAFETY_OFFSET` frames after the frame the device receives (the output timestamp already
/// includes the server's buffer), so `latency = host time heard - note stamp`, as in-process.
pub fn midi_latency(dev: &LiveDevice, o: &midi_latency::Opts, quantum: u32) -> Result<Report, String> {
    let out_total = dev.scope_total(Scope::Output);
    let device_frames = out_total.map_or(0, |t| t.saturating_sub(quantum)) as f64;
    let rate = dev.rate;
    let count = o.count;
    let landing = Arc::new(StampLog::new(count + 16));
    let late = Arc::new(AtomicU64::new(0));
    let (tx, rx) = channel::<(u64, MidiEvent)>();
    let mut sched = EventScheduler::with_capacity(count + 16);
    let mut evs: Vec<MidiEvent> = Vec::with_capacity(count + 16);
    let mut unit = BenchInstrument::new();
    let cfg = RenderConfig { sample_rate: rate, max_frames: quantum, input_channels: 0, output_channels: 2 };
    unit.initialize(&cfg).map_err(|e| e.to_string())?;
    let (l2, late2) = (landing.clone(), late.clone());
    // `try_recv` on the std channel neither locks nor allocates on the receiving side.
    let f: IoProc = Box::new(move |ctx: &IoContext, _now: &TimeStamp, _in: &BufferList, _in_ts: &TimeStamp, out: &mut BufferList, out_ts: &TimeStamp| {
        while let Ok((ns, ev)) = rx.try_recv() {
            sched.push(ns, ev);
        }
        // re-anchored on every cycle's output stamp; the nominal rate is exact over one buffer (the server's DLL
        // scalar is not trusted: a wrong one pushes every note into "later" or far into the future)
        let tl = Timeline::new(0.0, out_ts.host_ns, rate, 1.0);
        let before = sched.late;
        sched.take_cycle(&tl, 0, ctx.frames, &mut evs);
        late2.fetch_add(sched.late - before, Ordering::Relaxed);
        render_into(&mut unit, ctx.frames as usize, out_ts.sample_time, &evs, out);
        for e in evs.iter().filter(|e| e.is_note_on()) {
            l2.push(landing_ns(out_ts.host_ns, e.offset, device_frames, rate));
        }
    });
    let buffer_ns = (quantum as f64 * 1e9 / rate) as u64;
    let lead_ns = o.lead as u64 * buffer_ns;
    let (interval_ms, lead) = (o.interval_ms, o.lead);
    let proc_cfg = IoProcConfig { input_channels: 0, output_channels: 2, buffer_frames: quantum };
    let run = dev.run_proc(proc_cfg, f, move || {
        let t = RealTime;
        // let the IO thread settle before the first note
        let plan = plan_notes(t.now_ns() + 100_000_000, count, quantum, rate, interval_ms);
        let mut midi = BenchMidi::new();
        let mut stamps = Vec::with_capacity(plan.len());
        for ns in plan {
            let stamp = if lead == 0 {
                t.wait_until(ns);
                t.now_ns()
            } else {
                t.wait_until(ns.saturating_sub(lead_ns));
                ns
            };
            midi.send_note_on(stamp, 60);
            for e in midi.take_events(u64::MAX) {
                let _ = tx.send(e);
            }
            stamps.push(stamp);
        }
        t.wait_until(t.now_ns() + TAIL.as_nanos() as u64 + (lead as u64 + 4) * buffer_ns);
        stamps
    })?;

    let stamps = run.out;
    let heard = landing.values();
    let n = stamps.len().min(heard.len());
    let latencies_ms: Vec<f64> = (0..n).map(|i| (heard[i] as f64 - stamps[i] as f64) / 1e6).collect();
    check_latencies_ms(&latencies_ms)?;
    let extra = vec![
        ("notes_sent".to_string(), Json::Uint(stamps.len() as u64)),
        ("impulses".to_string(), Json::Uint(heard.len() as u64)),
        ("late".to_string(), Json::Uint(late.load(Ordering::Relaxed))),
        ("lead_cycles".to_string(), Json::Uint(o.lead as u64)),
        ("interval_ms".to_string(), Json::Num(o.interval_ms)),
        ("async_source".to_string(), Json::Bool(o.lead == 0)),
        ("out_total_frames".to_string(), out_total.map_or(Json::Null, |t| Json::Uint(t as u64))),
        ("overloads".to_string(), Json::Uint(run.overloads)),
        ("midi_path".to_string(), Json::str("hub-in-process")),
    ];
    let stats = summarize(&latencies_ms).unwrap_or_else(Summary::empty);
    let reported = out_total.map(|t| dev.frames_to_ms(t as f64));
    Ok(dev.report("midi-latency", quantum, stats, reported, run.xruns, (count - n.min(count)) as u64, extra))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_log_keeps_order_and_stops_when_full() {
        let l = StampLog::new(3);
        assert!(l.is_empty());
        assert!(l.push(5) && l.push(7));
        assert!(l.push_f64(1.5));
        assert!(!l.push(9));
        assert_eq!(l.len(), 3);
        assert_eq!(&l.values()[..2], &[5, 7]);
        assert_eq!(l.values_f64()[2], 1.5);
    }

    #[test]
    fn labels_and_picking() {
        assert_eq!(label("alsa:PCH:0"), "live:alsa:PCH:0");
        let uids: Vec<String> = ["loopback-out", "alsa:PCH:0"].iter().map(|s| s.to_string()).collect();
        assert_eq!(pick(&uids, Some("alsa:PCH:0"), Some(0)), Ok(1));
        assert!(pick(&uids, Some("alsa:USB:0"), None).is_err());
        assert_eq!(pick(&uids, None, Some(1)), Ok(1));
        // no (or a stale) default: the first device
        assert_eq!(pick(&uids, None, None), Ok(0));
        assert_eq!(pick(&uids, None, Some(5)), Ok(0));
        assert!(pick(&[], None, None).is_err());
    }

    #[test]
    fn detect_finds_the_first_sample_and_holds_off() {
        let mut last = None;
        let mut p = [0.0f32; 8];
        p[3] = -0.9;
        p[5] = 1.0;
        // input timeline: plane[0] is sample 100.5, so the onset is 103.5
        assert_eq!(detect(&p, 100.5, 0.25, &mut last, 50.0), Some(103.5));
        assert_eq!(last, Some(103.5));
        // 140 - 103.5 = 36.5 < 50: ringing of the same impulse
        assert_eq!(detect(&p, 137.0, 0.25, &mut last, 50.0), None);
        // 200 - 103.5 >= 50: a new impulse
        assert_eq!(detect(&p, 197.0, 0.25, &mut last, 50.0), Some(200.0));
        assert_eq!(detect(&[0.1, 0.2], 0.0, 0.25, &mut last, 50.0), None);
    }

    #[test]
    fn round_trips_add_both_buffers() {
        // impulses written at output sample times 1000 and 2000; seen at input times 1064 and 2064.5
        let sent = [1000.0, 2000.0];
        assert_eq!(pair_times(&sent, &[990.0, 1064.0, 1070.0, 2064.5]), vec![Some(1064.0), Some(2064.5)]);
        // device path 64 and 64.5 frames, plus 2 * 64 frames of buffers
        assert_eq!(round_trips(&sent, &[1064.0, 2064.5], 64), vec![192.0, 192.5]);
        // a lost impulse is dropped from the list
        assert_eq!(round_trips(&sent, &[2100.0], 64), vec![228.0]);
        assert_eq!(pair_times(&sent, &[2100.0]), vec![None, Some(2100.0)]);
    }

    #[test]
    fn interval_cycles_round_up_with_a_floor_of_four() {
        // 20 ms at 48 kHz = 960 frames = 15 cycles of 64
        assert_eq!(interval_cycles(20.0, 48_000.0, 64), 15);
        // 20 ms at 50 kHz = 1000 frames = 15.6 -> 16 cycles
        assert_eq!(interval_cycles(20.0, 50_000.0, 64), 16);
        assert_eq!(interval_cycles(1.0, 48_000.0, 256), 4);
    }

    #[test]
    fn wake_late_is_the_phase_against_the_best_call() {
        // calls at 0, +2, -1 and +20 frames off a 64-frame grid that starts at 1000
        let seen = [1000.0, 1066.0, 1127.0, 1212.0];
        assert_eq!(wake_late_frames(&seen, 64), vec![1.0, 3.0, 0.0, 21.0]);
        assert!(wake_late_frames(&[], 64).is_empty());
        assert_eq!(wake_late_frames(&[7.0], 64), vec![0.0]);
    }

    #[test]
    fn landing_is_on_the_host_clock_of_the_output_stamp() {
        // uptime of 3 days in ns on CLOCK_MONOTONIC_RAW, 48 kHz, note at offset 5, 96 device frames: 101 frames
        let out = 259_200_000_000_000u64;
        let land = landing_ns(out, 5, 96.0, 48_000.0);
        assert_eq!(land - out, 2_104_167);
        // a note stamped 100 ms before the cycle on the same clock is heard about 102 ms later: sane
        let ms = (land as f64 - (out - 100_000_000) as f64) / 1e6;
        assert!((ms - 102.104).abs() < 0.001);
        assert!(check_latencies_ms(&[ms, 3.2, 0.0]).is_ok());
    }

    #[test]
    fn latencies_outside_zero_to_a_second_are_measurement_errors() {
        assert!(check_latencies_ms(&[]).is_ok());
        assert!(check_latencies_ms(&[1.0, 1000.0]).is_ok());
        assert!(check_latencies_ms(&[1.0, -0.001]).is_err());
        assert!(check_latencies_ms(&[1000.5]).is_err());
        assert!(check_latencies_ms(&[f64::NAN]).is_err());
        // a ns stamp against a sample time (the reported 6e10 ms) is caught
        assert!(check_latencies_ms(&[60_740_487_042.663]).is_err());
    }

    #[test]
    fn no_loopback_needs_silence_for_the_first_impulses() {
        let sent: Vec<f64> = (0..10).map(|i| 1000.0 * (i + 1) as f64).collect();
        assert!(no_loopback(&sent, &[]));
        assert!(!no_loopback(&sent, &[1064.0]));
        // the 8th impulse (8000) heard: inside the probe; the 9th (9000) heard: outside it
        assert!(!no_loopback(&sent, &[8064.0]));
        assert!(no_loopback(&sent, &[9064.0]));
        assert!(!no_loopback(&[], &[]));
    }

    #[test]
    fn notes_are_planned_on_the_host_clock() {
        // 50 kHz, quantum 64, 20 ms: samples 0, 1007, 2014 (stride 7); one sample is 20 000 ns
        assert_eq!(plan_notes(1_000_000_000, 3, 64, 50_000.0, 20.0), vec![1_000_000_000, 1_020_140_000, 1_040_280_000]);
    }

    #[test]
    fn render_into_writes_impulses_into_the_hal_planes() {
        let mut unit = BenchInstrument::new();
        let cfg = RenderConfig { sample_rate: 50_000.0, max_frames: 16, input_channels: 0, output_channels: 2 };
        unit.initialize(&cfg).unwrap();
        let mut out = BufferList::silent(2, 16);
        out.buffers[1].data[0] = 0.5;
        let evs = [MidiEvent::note_on(3, 0, 60, 100)];
        render_into(&mut unit, 16, 0.0, &evs, &mut out);
        for b in &out.buffers {
            let hot: Vec<usize> = (0..16).filter(|i| b.data[*i] != 0.0).collect();
            assert_eq!(hot, vec![3]);
            assert_eq!(b.data[3], 1.0);
        }
        assert_eq!(unit.impulses(), 1);
    }
}
