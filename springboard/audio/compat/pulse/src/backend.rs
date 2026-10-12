//! What the protocol session needs from the audio server: the devices, and a way to open a playback or capture
//! stream on one. [`HalBackend`] makes each stream a HAL client (an IO procedure with its own buffer size on the
//! stream's device); the deterministic test double (`MockBackend`) exists in test builds only.

use crate::proto::err::{INTERNAL as ERR_INTERNAL, NOENTITY as ERR_NOENTITY};
use crate::ring::{Resampler, Ring, MAX_CHANNELS};
use bifrost_audio_hal::{Device, IoProcConfig, IoProcId, Role, System};
use bifrost_audio_types::props;
use bifrost_audio_types::{AudioError, PropertyAddress, PropertyValue, Scope};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// A sink or a source as the protocol sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceInfo {
    pub name: String,
    pub description: String,
    pub rate: u32,
    pub channels: u8,
    /// The device's own latency in microseconds.
    pub latency_us: u64,
}

/// State shared between a stream's protocol side and its real-time IO procedure. All fields are atomics.
pub struct StreamCtl {
    /// The linear gain (f32 bits): the stream volume times the mute.
    gain: AtomicU32,
    /// Whether the IO procedure plays (playback only).
    pub run: AtomicBool,
    /// Whether an underrun stops playback until the buffer refills (a stream with a prebuffer).
    pub rebuffer: AtomicBool,
    /// Underruns (playback) or overruns (capture) so far.
    pub xruns: AtomicU32,
    /// The stream's current sample rate (variable-rate streams change it).
    pub rate: AtomicU32,
}

impl StreamCtl {
    pub fn new(rate: u32) -> StreamCtl {
        StreamCtl { gain: AtomicU32::new(1.0f32.to_bits()), run: AtomicBool::new(false), rebuffer: AtomicBool::new(false), xruns: AtomicU32::new(0), rate: AtomicU32::new(rate) }
    }
    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }
    pub fn set_gain(&self, g: f32) {
        self.gain.store(g.to_bits(), Ordering::Relaxed);
    }
}

/// A running stream; dropping it stops and removes it.
pub trait StreamHandle {}

/// How a stream is opened.
pub struct StreamParams {
    /// Channels the stream carries in its ring.
    pub channels: usize,
    /// The stream's sample rate.
    pub rate: u32,
    /// Frames per IO cycle the stream asks for (0 = the device's).
    pub cycle_frames: u32,
}

pub trait Backend {
    fn sinks(&self) -> Vec<DeviceInfo>;
    fn sources(&self) -> Vec<DeviceInfo>;
    /// Starts a playback stream on sink `sink`: the IO procedure drains `ring` while `ctl.run` is set.
    fn open_playback(&mut self, sink: &str, p: &StreamParams, ring: Arc<Ring>, ctl: Arc<StreamCtl>) -> Result<Box<dyn StreamHandle>, u32>;
    /// Starts a capture stream on `source`: the IO procedure fills `ring` at the device rate with `p.channels`.
    fn open_capture(&mut self, source: &str, p: &StreamParams, ring: Arc<Ring>, ctl: Arc<StreamCtl>) -> Result<Box<dyn StreamHandle>, u32>;
    /// Sets a device's volume (linear scalar) and mute.
    fn set_device_volume(&mut self, sink: bool, name: &str, volume: f32, mute: bool);
}

const MAX_CYCLE_FRAMES: usize = 8192;

/// The IO cycle a stream asks of its device: its period (frames at its own rate) converted to the device rate,
/// rounded up to a power of two and kept within 64 to 4096 frames.
pub fn cycle_for(p: &StreamParams, dev: &DeviceInfo) -> u32 {
    if p.cycle_frames == 0 {
        return 0;
    }
    let at_dev = (p.cycle_frames as u64 * dev.rate as u64 / p.rate.max(1) as u64).max(1) as u32;
    at_dev.next_power_of_two().clamp(64, 4096)
}

/// Maps a stream channel to a device channel: a mono stream feeds all, otherwise channel to channel.
fn source_channel(stream_ch: usize, dev_ch: usize) -> Option<usize> {
    if stream_ch == 1 {
        Some(0)
    } else if dev_ch < stream_ch {
        Some(dev_ch)
    } else {
        None
    }
}

/// The real-time playback work for one cycle: pulls from the ring, applies the gain, writes the device buffers.
/// No allocation: `scratch` is sized up front.
pub struct PlaybackProc {
    ring: Arc<Ring>,
    ctl: Arc<StreamCtl>,
    resampler: Resampler,
    scratch: Vec<f32>,
    channels: usize,
    dev_rate: u32,
    last_rate: u32,
}

impl PlaybackProc {
    pub fn new(ring: Arc<Ring>, ctl: Arc<StreamCtl>, channels: usize, rate: u32, dev_rate: u32) -> PlaybackProc {
        let channels = channels.clamp(1, MAX_CHANNELS);
        PlaybackProc { ring, ctl, resampler: Resampler::new(channels, rate, dev_rate), scratch: vec![0.0; MAX_CYCLE_FRAMES * channels], channels, dev_rate, last_rate: rate }
    }

    /// `out` is one slice per device channel (non-interleaved), all the same length.
    pub fn process(&mut self, out: &mut [&mut [f32]]) {
        let frames = out.first().map(|b| b.len()).unwrap_or(0).min(MAX_CYCLE_FRAMES);
        for b in out.iter_mut() {
            b.iter_mut().for_each(|s| *s = 0.0);
        }
        if frames == 0 || !self.ctl.run.load(Ordering::Relaxed) {
            return;
        }
        let rate = self.ctl.rate.load(Ordering::Relaxed);
        if rate != self.last_rate {
            self.last_rate = rate;
            self.resampler.set_rates(rate, self.dev_rate);
        }
        let ch = self.channels;
        let got = self.resampler.pull(&self.ring, &mut self.scratch[..frames * ch]);
        if got < frames {
            self.ctl.xruns.fetch_add(1, Ordering::Relaxed);
            if self.ctl.rebuffer.load(Ordering::Relaxed) {
                self.ctl.run.store(false, Ordering::Relaxed);
            }
        }
        let gain = self.ctl.gain();
        for (dc, buf) in out.iter_mut().enumerate() {
            if let Some(sc) = source_channel(ch, dc) {
                for f in 0..got.min(buf.len()) {
                    buf[f] = self.scratch[f * ch + sc] * gain;
                }
            }
        }
    }
}

/// The real-time capture work: interleaves the device input into the ring.
pub struct CaptureProc {
    ring: Arc<Ring>,
    ctl: Arc<StreamCtl>,
    scratch: Vec<f32>,
    channels: usize,
}

impl CaptureProc {
    pub fn new(ring: Arc<Ring>, ctl: Arc<StreamCtl>, channels: usize) -> CaptureProc {
        let channels = channels.clamp(1, MAX_CHANNELS);
        CaptureProc { ring, ctl, scratch: vec![0.0; MAX_CYCLE_FRAMES * channels], channels }
    }

    /// `input` is one slice per device channel (non-interleaved).
    pub fn process(&mut self, input: &[&[f32]]) {
        let frames = input.first().map(|b| b.len()).unwrap_or(0).min(MAX_CYCLE_FRAMES);
        if frames == 0 {
            return;
        }
        let ch = self.channels;
        let gain = self.ctl.gain();
        for f in 0..frames {
            for c in 0..ch {
                let src = if c < input.len() { c } else { 0 };
                self.scratch[f * ch + c] = input[src][f] * gain;
            }
        }
        if self.ring.write(&self.scratch[..frames * ch]) < frames * ch {
            self.ctl.xruns.fetch_add(1, Ordering::Relaxed);
        }
    }
}

struct HalStream {
    device: Device,
    id: IoProcId,
}

impl StreamHandle for HalStream {}

impl Drop for HalStream {
    fn drop(&mut self) {
        let _ = self.device.stop(self.id);
        let _ = self.device.remove_io_proc(self.id);
    }
}

/// The HAL-backed implementation. Sinks and sources are the devices that have output or input streams.
pub struct HalBackend {
    system: System,
}

impl HalBackend {
    pub fn connect() -> Result<HalBackend, AudioError> {
        Ok(HalBackend { system: System::connect("is.olibuijr.pulse")? })
    }

    fn describe(&self, d: &Device, scope: Scope) -> Option<DeviceInfo> {
        let streams = d.get(PropertyAddress::scoped(props::STREAMS, scope)).ok()?;
        match streams {
            PropertyValue::Objects(v) if !v.is_empty() => {}
            _ => return None,
        }
        let string = |sel| match d.get(PropertyAddress::global(sel)) {
            Ok(PropertyValue::String(s)) => Some(s),
            _ => None,
        };
        let name = string(props::DEVICE_UID)?;
        let description = string(props::NAME).unwrap_or_else(|| name.clone());
        let rate = match d.get(PropertyAddress::global(props::NOMINAL_RATE)) {
            Ok(PropertyValue::F64(r)) => r as u32,
            _ => 48000,
        };
        let frames = match d.get(PropertyAddress::scoped(props::LATENCY, scope)) {
            Ok(PropertyValue::U32(f)) => f as u64,
            _ => 0,
        };
        Some(DeviceInfo { name, description, rate, channels: 2, latency_us: frames * 1_000_000 / rate.max(1) as u64 })
    }

    fn list(&self, scope: Scope) -> Vec<DeviceInfo> {
        let Ok(devices) = self.system.devices() else { return Vec::new() };
        devices.iter().filter_map(|d| self.describe(d, scope)).collect()
    }

    fn find(&self, name: &str, scope: Scope) -> Result<(Device, DeviceInfo), u32> {
        let devices = self.system.devices().map_err(|_| ERR_INTERNAL)?;
        if name.is_empty() {
            let role = if scope == Scope::Output { Role::Output } else { Role::Input };
            let d = self.system.default_device(role).map_err(|_| ERR_NOENTITY)?;
            let info = self.describe(&d, scope).ok_or(ERR_NOENTITY)?;
            return Ok((d, info));
        }
        for d in devices {
            if let Some(info) = self.describe(&d, scope) {
                if info.name == name {
                    return Ok((d, info));
                }
            }
        }
        Err(ERR_NOENTITY)
    }
}

impl Backend for HalBackend {
    fn sinks(&self) -> Vec<DeviceInfo> {
        self.list(Scope::Output)
    }
    fn sources(&self) -> Vec<DeviceInfo> {
        self.list(Scope::Input)
    }
    fn open_playback(&mut self, sink: &str, p: &StreamParams, ring: Arc<Ring>, ctl: Arc<StreamCtl>) -> Result<Box<dyn StreamHandle>, u32> {
        let (device, info) = self.find(sink, Scope::Output)?;
        let mut proc = PlaybackProc::new(ring, ctl, p.channels, p.rate, info.rate);
        let cfg = IoProcConfig { input_channels: 0, output_channels: info.channels as u32, buffer_frames: cycle_for(p, &info) };
        let id = device
            .add_io_proc(
                cfg,
                Box::new(move |_ctx, _in_ts, _input, _out_ts, output, _now| {
                    let mut slices: [&mut [f32]; MAX_CHANNELS] = Default::default();
                    let mut n = 0;
                    for b in output.buffers.iter_mut().take(MAX_CHANNELS) {
                        slices[n] = b.data.as_mut_slice();
                        n += 1;
                    }
                    proc.process(&mut slices[..n]);
                }),
            )
            .map_err(|_| ERR_INTERNAL)?;
        device.start(id).map_err(|_| ERR_INTERNAL)?;
        Ok(Box::new(HalStream { device, id }))
    }
    fn open_capture(&mut self, source: &str, p: &StreamParams, ring: Arc<Ring>, ctl: Arc<StreamCtl>) -> Result<Box<dyn StreamHandle>, u32> {
        let (device, info) = self.find(source, Scope::Input)?;
        let mut proc = CaptureProc::new(ring, ctl, p.channels);
        let cfg = IoProcConfig { input_channels: info.channels as u32, output_channels: 0, buffer_frames: cycle_for(p, &info) };
        let id = device
            .add_io_proc(
                cfg,
                Box::new(move |_ctx, _in_ts, input, _out_ts, _output, _now| {
                    let mut slices: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
                    let mut n = 0;
                    for b in input.buffers.iter().take(MAX_CHANNELS) {
                        slices[n] = b.data.as_slice();
                        n += 1;
                    }
                    proc.process(&slices[..n]);
                }),
            )
            .map_err(|_| ERR_INTERNAL)?;
        device.start(id).map_err(|_| ERR_INTERNAL)?;
        Ok(Box::new(HalStream { device, id }))
    }
    fn set_device_volume(&mut self, sink: bool, name: &str, volume: f32, mute: bool) {
        let scope = if sink { Scope::Output } else { Scope::Input };
        if let Ok((d, _)) = self.find(name, scope) {
            let _ = d.set(PropertyAddress::scoped(props::VOLUME_SCALAR, scope), PropertyValue::F32(volume));
            let _ = d.set(PropertyAddress::scoped(props::MUTE, scope), PropertyValue::Bool(mute));
        }
    }
}

/// The test double: fixed devices, and the opened streams are kept for the test to drive.
#[cfg(test)]
#[derive(Default)]
pub struct MockBackend {
    pub sinks: Vec<DeviceInfo>,
    pub sources: Vec<DeviceInfo>,
    pub playback: Vec<(String, Arc<Ring>, Arc<StreamCtl>)>,
    pub capture: Vec<(String, Arc<Ring>, Arc<StreamCtl>)>,
    pub volumes: Vec<(bool, String, f32, bool)>,
}

#[cfg(test)]
struct NoHandle;
#[cfg(test)]
impl StreamHandle for NoHandle {}

#[cfg(test)]
impl MockBackend {
    pub fn standard() -> MockBackend {
        let dev = |n: &str| DeviceInfo { name: n.to_string(), description: n.to_string(), rate: 48000, channels: 2, latency_us: 10_000 };
        MockBackend { sinks: vec![dev("out0")], sources: vec![dev("in0")], ..Default::default() }
    }
}

#[cfg(test)]
impl Backend for MockBackend {
    fn sinks(&self) -> Vec<DeviceInfo> {
        self.sinks.clone()
    }
    fn sources(&self) -> Vec<DeviceInfo> {
        self.sources.clone()
    }
    fn open_playback(&mut self, sink: &str, _p: &StreamParams, ring: Arc<Ring>, ctl: Arc<StreamCtl>) -> Result<Box<dyn StreamHandle>, u32> {
        let name = if sink.is_empty() { self.sinks.first().map(|d| d.name.clone()).ok_or(ERR_NOENTITY)? } else { sink.to_string() };
        if !self.sinks.iter().any(|d| d.name == name) {
            return Err(ERR_NOENTITY);
        }
        self.playback.push((name, ring, ctl));
        Ok(Box::new(NoHandle))
    }
    fn open_capture(&mut self, source: &str, _p: &StreamParams, ring: Arc<Ring>, ctl: Arc<StreamCtl>) -> Result<Box<dyn StreamHandle>, u32> {
        let name = if source.is_empty() { self.sources.first().map(|d| d.name.clone()).ok_or(ERR_NOENTITY)? } else { source.to_string() };
        if !self.sources.iter().any(|d| d.name == name) {
            return Err(ERR_NOENTITY);
        }
        self.capture.push((name, ring, ctl));
        Ok(Box::new(NoHandle))
    }
    fn set_device_volume(&mut self, sink: bool, name: &str, volume: f32, mute: bool) {
        self.volumes.push((sink, name.to_string(), volume, mute));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bufs(n: usize, frames: usize) -> Vec<Vec<f32>> {
        vec![vec![9.0; frames]; n]
    }

    #[test]
    fn stopped_stream_outputs_silence_and_leaves_ring() {
        let ring = Arc::new(Ring::new(64));
        ring.write(&[1.0, 1.0]);
        let ctl = Arc::new(StreamCtl::new(48000));
        let mut p = PlaybackProc::new(ring.clone(), ctl, 2, 48000, 48000);
        let mut b = bufs(2, 4);
        let mut s: Vec<&mut [f32]> = b.iter_mut().map(|v| v.as_mut_slice()).collect();
        p.process(&mut s);
        assert!(b.iter().flatten().all(|&x| x == 0.0));
        assert_eq!(ring.available(), 2);
    }

    #[test]
    fn stereo_stream_is_deinterleaved_with_gain() {
        let ring = Arc::new(Ring::new(64));
        ring.write(&[0.5, -0.5, 0.25, -0.25]);
        let ctl = Arc::new(StreamCtl::new(48000));
        ctl.run.store(true, Ordering::Relaxed);
        ctl.set_gain(0.5);
        let mut p = PlaybackProc::new(ring, ctl.clone(), 2, 48000, 48000);
        let mut b = bufs(2, 3);
        let mut s: Vec<&mut [f32]> = b.iter_mut().map(|v| v.as_mut_slice()).collect();
        p.process(&mut s);
        assert_eq!(b[0], vec![0.25, 0.125, 0.0]);
        assert_eq!(b[1], vec![-0.25, -0.125, 0.0]);
        // one frame short: an underrun was counted
        assert_eq!(ctl.xruns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn mono_stream_feeds_both_channels() {
        let ring = Arc::new(Ring::new(16));
        ring.write(&[0.5, 0.25]);
        let ctl = Arc::new(StreamCtl::new(48000));
        ctl.run.store(true, Ordering::Relaxed);
        let mut p = PlaybackProc::new(ring, ctl, 1, 48000, 48000);
        let mut b = bufs(2, 2);
        let mut s: Vec<&mut [f32]> = b.iter_mut().map(|v| v.as_mut_slice()).collect();
        p.process(&mut s);
        assert_eq!(b[0], vec![0.5, 0.25]);
        assert_eq!(b[1], vec![0.5, 0.25]);
    }

    #[test]
    fn underrun_with_prebuffer_stops_the_stream() {
        let ring = Arc::new(Ring::new(16));
        let ctl = Arc::new(StreamCtl::new(48000));
        ctl.run.store(true, Ordering::Relaxed);
        ctl.rebuffer.store(true, Ordering::Relaxed);
        let mut p = PlaybackProc::new(ring, ctl.clone(), 2, 48000, 48000);
        let mut b = bufs(2, 2);
        let mut s: Vec<&mut [f32]> = b.iter_mut().map(|v| v.as_mut_slice()).collect();
        p.process(&mut s);
        assert!(!ctl.run.load(Ordering::Relaxed));
    }

    #[test]
    fn capture_interleaves_and_maps_channels() {
        let ring = Arc::new(Ring::new(32));
        let ctl = Arc::new(StreamCtl::new(48000));
        let mut c = CaptureProc::new(ring.clone(), ctl, 2);
        let left = [1.0f32, 2.0];
        let right = [3.0f32, 4.0];
        c.process(&[&left, &right]);
        let mut out = [0.0; 4];
        assert_eq!(ring.read(&mut out), 4);
        assert_eq!(out, [1.0, 3.0, 2.0, 4.0]);
        // a mono device feeds both stream channels
        c.process(&[&left]);
        assert_eq!(ring.read(&mut out), 4);
        assert_eq!(out, [1.0, 1.0, 2.0, 2.0]);
    }

    #[test]
    fn capture_overrun_is_counted() {
        let ring = Arc::new(Ring::new(2));
        let ctl = Arc::new(StreamCtl::new(48000));
        let mut c = CaptureProc::new(ring, ctl.clone(), 2);
        c.process(&[&[1.0, 2.0], &[3.0, 4.0]]);
        assert_eq!(ctl.xruns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn cycle_is_a_power_of_two_in_range() {
        let dev = DeviceInfo { name: String::new(), description: String::new(), rate: 48000, channels: 2, latency_us: 0 };
        let p = |f, r| StreamParams { channels: 2, rate: r, cycle_frames: f };
        assert_eq!(cycle_for(&p(0, 48000), &dev), 0);
        assert_eq!(cycle_for(&p(1200, 48000), &dev), 2048);
        assert_eq!(cycle_for(&p(10, 48000), &dev), 64);
        // 441 frames at 44.1 kHz is 480 at 48 kHz -> 512
        assert_eq!(cycle_for(&p(441, 44100), &dev), 512);
        assert_eq!(cycle_for(&p(100_000, 48000), &dev), 4096);
    }

    #[test]
    fn channel_mapping() {
        assert_eq!(source_channel(1, 1), Some(0));
        assert_eq!(source_channel(2, 1), Some(1));
        assert_eq!(source_channel(2, 2), None);
    }
}
