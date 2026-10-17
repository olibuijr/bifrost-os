//! `bifrost-audio-driver`: the interface between `bifrost-audiod` and its device drivers (the reference platform's
//! server plug-in interface), exactly as in docs/audio-architecture.md, "Driver interface", plus `NullDriver`
//! (a manual clock, for deterministic host tests of the whole server) and `LoopbackDriver`.
//!
//! Owner: task 05 (the null and loopback drivers; the traits are the lead's contract).

use bifrost_audio_types::{BufferList, StreamFormat};

/// A device as a driver describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceDesc {
    pub uid: String,
    pub name: String,
    pub manufacturer: String,
    /// "pci", "hdmi", "usb", "bluetooth", "virtual".
    pub transport: String,
    pub input_channels: u32,
    pub output_channels: u32,
    /// Supported rates as (min, max) ranges.
    pub rates: Vec<(f64, f64)>,
    pub buffer_frames_range: (u32, u32),
    /// Devices with the same clock domain share one hardware clock (no drift between them).
    pub clock_domain: u32,
}

/// How to open a device.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IoConfig {
    pub rate: f64,
    /// Frames per server cycle (the graph quantum).
    pub quantum: u32,
    pub input: bool,
    pub output: bool,
}

/// The hardware position, read on the IO thread (no syscall on the ALSA driver: the mmapped status page).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HwPosition {
    /// Frames the hardware has played (or captured) since start.
    pub hw_frames: u64,
    /// When it was measured (host ns, `CLOCK_MONOTONIC_RAW`).
    pub host_ns: u64,
    pub running: bool,
}

/// The device's zero timestamp at start.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ZeroTimeStamp {
    pub sample_time: u64,
    pub host_ns: u64,
}

/// Latency per scope, in frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Latency {
    pub input: u32,
    pub output: u32,
    pub input_safety: u32,
    pub output_safety: u32,
}

/// An over- or under-run: the cycle's data is lost; the server counts it and recovers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Xrun {
    pub frames_lost: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DriverError {
    NotFound,
    Busy,
    Unsupported(String),
    Io(String),
}

/// Something a driver noticed outside the IO cycle.
#[derive(Debug, Clone, PartialEq)]
pub enum DriverEvent {
    DeviceAdded(DeviceDesc),
    DeviceRemoved { uid: String },
    JackChanged { uid: String, jack: String, connected: bool },
    RateChanged { uid: String, rate: f64 },
    ControlChanged { uid: String, control: ControlId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ControlId(pub u32);

/// A device control (volume, mute, data source, selector).
#[derive(Debug, Clone, PartialEq)]
pub struct ControlDesc {
    pub id: ControlId,
    pub name: String,
    /// "volume", "mute", "data-source", "selector".
    pub kind: String,
    /// Output or input side.
    pub output: bool,
    /// Volume range in dB, for volume controls.
    pub db_range: (f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlValue {
    /// 0..1 volume scalar.
    Scalar(f32),
    Bool(bool),
    Index(u32),
}

pub trait Driver: Send {
    fn name(&self) -> &str;
    fn enumerate(&mut self, out: &mut Vec<DeviceDesc>);
    fn open(&mut self, uid: &str, cfg: &IoConfig) -> Result<Box<dyn DeviceIo>, DriverError>;
    fn poll_events(&mut self, out: &mut Vec<DriverEvent>);
    fn controls(&mut self, uid: &str) -> Vec<ControlDesc>;
    fn set_control(&mut self, uid: &str, id: ControlId, v: ControlValue) -> Result<(), DriverError>;
}

/// Called only on the device's IO thread.
pub trait DeviceIo: Send {
    fn start(&mut self) -> Result<ZeroTimeStamp, DriverError>;
    fn position(&mut self) -> HwPosition;
    fn read_input(&mut self, frames: u32, dst: &mut BufferList) -> Result<(), Xrun>;
    fn write_mix(&mut self, frames: u32, src: &BufferList) -> Result<(), Xrun>;
    fn stop(&mut self);
    fn latency(&self) -> Latency;
    /// The physical formats of the open streams (input, output).
    fn physical_formats(&self) -> (Option<StreamFormat>, Option<StreamFormat>);
    /// Hardware sample rate over the nominal rate, as the driver's DLL estimates it (1.0 when it has no estimate).
    /// Added by task 04 so the server can publish the rate scalar of the zero timestamp.
    fn rate_scalar(&self) -> f64 {
        1.0
    }
    /// The predicted host time (ns, `CLOCK_MONOTONIC_RAW`) at which `hw_frames` is played or captured, from the
    /// driver's DLL; `None` until it has locked on. The server arms its cycle timer from this.
    fn host_time_of(&self, hw_frames: u64) -> Option<u64> {
        let _ = hw_frames;
        None
    }
    /// The zero timestamp of a restart the driver did on its own since the last call: it recovered from an xrun or
    /// a suspend by restarting the stream, so the hardware counter starts over at `sample_time` and its DLL was
    /// reset. The server polls this after every cycle and, on `Some`, re-anchors its cycle timeline and republishes
    /// the zero timestamp (with a new seed). `None` (the default): no restart. Added by task 24.
    fn take_restart(&mut self) -> Option<ZeroTimeStamp> {
        None
    }
    /// A closure that stops the hardware stream from another thread (a PCM `DROP` on a duplicate descriptor), so
    /// the control thread can end a stop the IO thread has not finished. `None`: nothing to abort.
    fn abort_hook(&self) -> Option<Box<dyn Fn() + Send + Sync>> {
        None
    }
}

/// A driver with devices on a manual clock: tests advance time with `advance` and every device's hardware pointer
/// moves by the frames of that time at its (possibly drifting) rate. Owner: task 05.
pub mod null;

/// A driver whose output devices loop back into a paired input device (the `snd-aloop` shape, in memory).
pub mod loopback;

/// A driver that publishes virtual devices from description files, each as a loopback pair (the plug-in loader).
pub mod plugin;

/// The transport names of devices and the composite driver that serves devices from several providers.
pub mod transport;
