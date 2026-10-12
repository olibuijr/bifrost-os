//! `AlsaDriver` and `AlsaIo`: the ALSA implementation of `bifrost_audio_driver::{Driver, DeviceIo}`.
//!
//! Enumeration walks `/dev/snd/controlC*`, one `DeviceDesc` per PCM device (both directions in one description).
//! `open` configures the streams (`pcm::negotiate`), links capture to playback for full duplex and returns the
//! `AlsaIo` the IO thread uses: `position` reads the mapped status page and feeds the DLL, `write_mix` and
//! `read_input` convert between the server's f32 and the ring and recover from under- and overruns.
//! Events come from the control device: `* Jack` booleans become `JackChanged`, other mixer elements
//! `ControlChanged`, and a rescan every second reports hot-plug.

use crate::convert::SampleFormat;
use crate::ctl::{jack_pcm, Control, Ctl, ElemId, Kind, IFACE_MIXER};
use crate::dll::Dll;
use crate::pcm::{converter_delay, Direction, Pcm, PcmCaps, PcmRequest};
use crate::ring::{after_resume, plan_capture, plan_playback, recovery_for, CapturePlan, PlaybackPlan, Recovery};
use crate::sys::now_raw_ns;
use crate::uapi::STATE_RUNNING;
use bifrost_audio_driver::{
    ControlDesc, ControlId, ControlValue, DeviceDesc, DeviceIo, Driver, DriverError, DriverEvent, HwPosition, IoConfig, Latency, Xrun, ZeroTimeStamp,
};
use bifrost_audio_types::{BufferList, FormatFlags, FormatId, StreamFormat};
use std::collections::HashMap;
use std::io;

/// Unified Logging subsystem `is.olibuijr.audio`, category `driver` (this crate cannot reach the shell logger).
macro_rules! dlog {
    ($($arg:tt)*) => { eprintln!("[is.olibuijr.audio/driver] {}", format_args!($($arg)*)) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlsaOptions {
    /// Ask for `NO_PERIOD_WAKEUP` where the device has it. The status page then only moves on a `HWSYNC`
    /// ioctl, which `position` issues: the one syscall besides the futex. Off by default: with period wakeups
    /// at the quantum the status page is current without any syscall.
    pub no_period_wakeup: bool,
    /// Most channels a device is opened with when the caller does not ask for a count.
    pub max_channels: u32,
    /// The smallest ring (2 periods, else 3) instead of the largest. On by default: lowest latency.
    pub low_latency: bool,
}

impl Default for AlsaOptions {
    fn default() -> Self {
        AlsaOptions { no_period_wakeup: false, max_channels: 2, low_latency: true }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    card: u32,
    device: u32,
    hdmi: bool,
    playback: bool,
    capture: bool,
    desc: DeviceDesc,
}

pub struct AlsaDriver {
    entries: Vec<Entry>,
    ctls: HashMap<u32, Ctl>,
    last_scan_ns: u64,
    opts: AlsaOptions,
}

/// The cards present: the numbers of `/dev/snd/controlC{n}`.
fn list_cards() -> Vec<u32> {
    let mut cards: Vec<u32> = std::fs::read_dir("/dev/snd")
        .map(|d| d.filter_map(|e| e.ok()).filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_prefix("controlC")).and_then(|n| n.parse().ok())).collect())
        .unwrap_or_default();
    cards.sort_unstable();
    cards
}

fn uid_of(card_id: &str, device: u32) -> String {
    format!("alsa:{card_id}:{device}")
}

/// The transport class of a device from its card driver and names.
pub fn transport_of(driver: &str, pcm_name: &str) -> &'static str {
    let d = driver.to_ascii_lowercase();
    if pcm_name.to_ascii_lowercase().contains("hdmi") || pcm_name.to_ascii_lowercase().contains("displayport") {
        "hdmi"
    } else if d.contains("usb") {
        "usb"
    } else {
        "pci"
    }
}

/// Channel counts, rates and buffer range of a duplex device from its two directions' capabilities.
/// The graph quantum range from the buffer size range: with a two-period ring (low latency) the smallest quantum
/// is half the smallest buffer, so a device with a 64 frame minimum buffer runs a 32 frame quantum.
pub fn quantum_range(buffer: (u32, u32), low_latency: bool) -> (u32, u32) {
    if low_latency {
        ((buffer.0 / 2).max(1), buffer.1)
    } else {
        buffer
    }
}

pub fn merge_caps(play: Option<&PcmCaps>, cap: Option<&PcmCaps>) -> (u32, u32, Vec<(f64, f64)>, (u32, u32)) {
    let mut lo = u32::MAX;
    let mut hi = 0;
    let (mut blo, mut bhi) = (u32::MAX, 0);
    for c in [play, cap].into_iter().flatten() {
        lo = lo.min(c.rates.0);
        hi = hi.max(c.rates.1);
        blo = blo.min(c.buffer_frames.0);
        bhi = bhi.max(c.buffer_frames.1);
    }
    let rates = if hi == 0 { Vec::new() } else { vec![(lo as f64, hi as f64)] };
    (cap.map_or(0, |c| c.channels.1), play.map_or(0, |c| c.channels.1), rates, if bhi == 0 { (0, 0) } else { (blo, bhi) })
}

impl AlsaDriver {
    pub fn new() -> AlsaDriver {
        AlsaDriver::with_options(AlsaOptions::default())
    }

    pub fn with_options(opts: AlsaOptions) -> AlsaDriver {
        AlsaDriver { entries: Vec::new(), ctls: HashMap::new(), last_scan_ns: 0, opts }
    }

    /// Opens (once) and subscribes the card's control device; false when it cannot be opened.
    fn ensure_ctl(&mut self, card: u32) -> bool {
        if !self.ctls.contains_key(&card) {
            let Ok(c) = Ctl::open(card) else { return false };
            let _ = c.subscribe(true);
            self.ctls.insert(card, c);
        }
        true
    }

    /// Scans the cards; known devices keep their description (probing would race our own open streams).
    /// Returns the devices that appeared and the uids that went away.
    fn rescan(&mut self) -> (Vec<DeviceDesc>, Vec<String>) {
        let mut found: Vec<Entry> = Vec::new();
        let mut added = Vec::new();
        for card in list_cards() {
            if !self.ensure_ctl(card) {
                continue;
            }
            let ctl = &self.ctls[&card];
            let (Ok(info), Ok(devs)) = (ctl.card_info(), ctl.pcm_devices()) else { continue };
            for device in devs {
                let uid = uid_of(&info.id, device);
                if let Some(known) = self.entries.iter().find(|e| e.desc.uid == uid) {
                    found.push(known.clone());
                    continue;
                }
                let Ok(pcm) = ctl.pcm_info(device) else { continue };
                let probe = |dir| Pcm::open(card, device, dir).and_then(|p| p.caps()).ok();
                let play = pcm.playback.then(|| probe(Direction::Playback)).flatten();
                let cap = pcm.capture.then(|| probe(Direction::Capture)).flatten();
                let (inch, outch, rates, buf) = merge_caps(play.as_ref(), cap.as_ref());
                let transport = transport_of(&info.driver, &pcm.name);
                let name = if transport == "hdmi" { ctl.monitor_name(device).unwrap_or_else(|| pcm.name.clone()) } else { pcm.name.clone() };
                let desc = DeviceDesc {
                    uid,
                    name,
                    manufacturer: info.name.clone(),
                    transport: transport.to_string(),
                    input_channels: inch,
                    output_channels: outch,
                    rates,
                    buffer_frames_range: quantum_range(buf, self.opts.low_latency),
                    clock_domain: card + 1,
                };
                let e = Entry { card, device, hdmi: transport == "hdmi", playback: pcm.playback, capture: pcm.capture, desc: desc.clone() };
                added.push(desc);
                found.push(e);
            }
        }
        let gone = self.entries.iter().filter(|e| !found.iter().any(|f| f.desc.uid == e.desc.uid)).map(|e| e.desc.uid.clone()).collect();
        self.entries = found;
        self.last_scan_ns = now_raw_ns();
        (added, gone)
    }

    fn entry(&self, uid: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.desc.uid == uid)
    }
}

impl Default for AlsaDriver {
    fn default() -> Self {
        AlsaDriver::new()
    }
}

fn map_io(e: io::Error) -> DriverError {
    match e.raw_os_error() {
        Some(libc::EBUSY) => DriverError::Busy,
        Some(libc::ENOENT) | Some(libc::ENODEV) | Some(libc::ENXIO) => DriverError::NotFound,
        _ if e.kind() == io::ErrorKind::Unsupported => DriverError::Unsupported(e.to_string()),
        _ => DriverError::Io(e.to_string()),
    }
}

/// The card's elements a jack event concerns: an HDMI/DP jack (`...,pcm=N Jack`) names its device, any other jack
/// belongs to the card's non-HDMI devices. `devices` is `(device number, is hdmi)`.
pub fn jack_targets(jack: &str, devices: &[(u32, bool)]) -> Vec<u32> {
    match jack_pcm(jack) {
        Some(n) => devices.iter().filter(|(d, _)| *d == n).map(|(d, _)| *d).collect(),
        None => devices.iter().filter(|(_, hdmi)| !hdmi).map(|(d, _)| *d).collect(),
    }
}

/// How a mixer element appears to the server: its kind label, or `None` when it is not offered.
pub fn classify(c: &Control) -> Option<&'static str> {
    if c.iface != IFACE_MIXER {
        return None;
    }
    match c.kind {
        Kind::Int if c.name.ends_with("Volume") && c.max > c.min => Some("volume"),
        Kind::Bool if c.name.ends_with("Playback Switch") || c.name.ends_with("Capture Switch") => Some("mute"),
        Kind::Enum if !c.items.is_empty() && (c.name.contains("Source") || c.name.contains("Input")) => Some("data-source"),
        Kind::Enum if !c.items.is_empty() && !c.name.contains("Jack") => Some("selector"),
        _ => None,
    }
}

/// Whether an element acts on the output side.
pub fn is_output(name: &str) -> bool {
    !["Capture", "Mic", "Input", "Source"].iter().any(|w| name.contains(w))
}

pub fn describe_control(c: &Control) -> Option<ControlDesc> {
    let kind = classify(c)?;
    let db_range = c.db.map_or((0.0, 0.0), |d| d.range_db(c.min, c.max));
    Some(ControlDesc { id: ControlId(c.numid), name: c.name.clone(), kind: kind.to_string(), output: is_output(&c.name), db_range })
}

/// The raw element values for a `ControlValue`. A volume scalar spreads linearly over the raw steps on every
/// channel; a mute is the inverse of a "Switch" element (1 means sound on).
pub fn control_values(c: &Control, v: &ControlValue) -> Result<Vec<i64>, DriverError> {
    let n = c.count.max(1) as usize;
    match (classify(c), v) {
        (Some("volume"), ControlValue::Scalar(x)) => {
            let x = x.clamp(0.0, 1.0) as f64;
            let raw = c.min + (x * (c.max - c.min) as f64).round() as i64;
            Ok(vec![raw; n])
        }
        (Some("mute"), ControlValue::Bool(muted)) => Ok(vec![if *muted { 0 } else { 1 }; n]),
        (Some("data-source" | "selector"), ControlValue::Index(i)) if (*i as usize) < c.items.len() => Ok(vec![*i as i64; n]),
        _ => Err(DriverError::Unsupported(format!("{} takes no such value", c.name))),
    }
}

impl Driver for AlsaDriver {
    fn name(&self) -> &str {
        "alsa"
    }

    fn enumerate(&mut self, out: &mut Vec<DeviceDesc>) {
        self.rescan();
        out.extend(self.entries.iter().map(|e| e.desc.clone()));
    }

    fn open(&mut self, uid: &str, cfg: &IoConfig) -> Result<Box<dyn DeviceIo>, DriverError> {
        let e = self.entry(uid).ok_or(DriverError::NotFound)?.clone();
        Ok(Box::new(AlsaIo::open(&e, cfg, &self.opts)?))
    }

    fn poll_events(&mut self, out: &mut Vec<DriverEvent>) {
        if now_raw_ns().saturating_sub(self.last_scan_ns) > 1_000_000_000 {
            let (added, gone) = self.rescan();
            out.extend(added.into_iter().map(DriverEvent::DeviceAdded));
            out.extend(gone.into_iter().map(|uid| DriverEvent::DeviceRemoved { uid }));
        }
        let cards: Vec<u32> = self.ctls.keys().copied().collect();
        for card in cards {
            let devices: Vec<(u32, bool)> = self.entries.iter().filter(|e| e.card == card).map(|e| (e.device, e.hdmi)).collect();
            let Some(ctl) = self.ctls.get(&card) else { continue };
            for _ in 0..64 {
                let Ok(Some(ev)) = ctl.read_event() else { break };
                if !ev.is_value_change() {
                    continue;
                }
                let Ok(c) = ctl.get(ElemId::by_numid(ev.numid)) else { continue };
                if c.kind == Kind::Bool && c.name.ends_with(" Jack") {
                    let connected = c.values.first().copied().unwrap_or(0) != 0;
                    for dev in jack_targets(&c.name, &devices) {
                        if let Some(e) = self.entries.iter().find(|e| e.card == card && e.device == dev) {
                            dlog!("jack {} {} on {}", c.name, if connected { "plugged" } else { "unplugged" }, e.desc.uid);
                            out.push(DriverEvent::JackChanged { uid: e.desc.uid.clone(), jack: c.name.clone(), connected });
                        }
                    }
                } else if classify(&c).is_some() {
                    for dev in jack_targets("", &devices) {
                        if let Some(e) = self.entries.iter().find(|e| e.card == card && e.device == dev) {
                            out.push(DriverEvent::ControlChanged { uid: e.desc.uid.clone(), control: ControlId(c.numid) });
                        }
                    }
                }
            }
        }
    }

    fn controls(&mut self, uid: &str) -> Vec<ControlDesc> {
        let Some(card) = self.entry(uid).map(|e| e.card) else { return Vec::new() };
        if !self.ensure_ctl(card) {
            return Vec::new();
        }
        self.ctls[&card].list().map(|l| l.iter().filter_map(describe_control).collect()).unwrap_or_default()
    }

    fn set_control(&mut self, uid: &str, id: ControlId, v: ControlValue) -> Result<(), DriverError> {
        let card = self.entry(uid).ok_or(DriverError::NotFound)?.card;
        if !self.ensure_ctl(card) {
            return Err(DriverError::NotFound);
        }
        let ctl = &self.ctls[&card];
        let c = ctl.get(ElemId::by_numid(id.0)).map_err(map_io)?;
        let values = control_values(&c, &v)?;
        ctl.set(&c, &values).map_err(map_io)
    }
}

/// The physical format of an open stream as the server describes it.
pub fn physical_format(rate: u32, channels: u32, fmt: SampleFormat, interleaved: bool) -> StreamFormat {
    let mut flags = FormatFlags::SIGNED_INTEGER;
    if fmt != SampleFormat::S24 {
        flags = flags | FormatFlags::PACKED;
    }
    if !interleaved {
        flags = flags | FormatFlags::NON_INTERLEAVED;
    }
    let per_sample = fmt.bytes() as u32;
    let bytes_per_frame = if interleaved { per_sample * channels } else { per_sample };
    StreamFormat {
        sample_rate: rate as f64,
        format: FormatId::LinearPcm,
        flags,
        bytes_per_packet: bytes_per_frame,
        frames_per_packet: 1,
        bytes_per_frame,
        channels,
        bits_per_channel: fmt.bits(),
    }
}

/// An open device: the IO thread's handle.
pub struct AlsaIo {
    out: Option<Pcm>,
    inp: Option<Pcm>,
    dll: Dll,
    quantum: u32,
    prefill: u32,
    last_ts: u64,
    xruns: u64,
    /// Playback converter latency in frames: hardware FIFO plus the codec and link delay the kernel reports.
    conv_out: u32,
    /// Capture converter latency in frames (the FIFO).
    conv_in: u32,
    /// Between `start` and `stop`: only then does a state other than RUNNING mean trouble.
    started: bool,
    /// The kernel reported the device disconnected (unplugged).
    gone: bool,
    /// The zero timestamp of a restart `recover` did, until the server takes it (`take_restart`).
    restarted: Option<ZeroTimeStamp>,
}

impl AlsaIo {
    fn open(e: &Entry, cfg: &IoConfig, opts: &AlsaOptions) -> Result<AlsaIo, DriverError> {
        let req = PcmRequest { rate: cfg.rate.round() as u32, quantum: cfg.quantum, channels: None, max_channels: opts.max_channels, no_period_wakeup: opts.no_period_wakeup, low_latency: opts.low_latency };
        if (cfg.output && !e.playback) || (cfg.input && !e.capture) || !(cfg.input || cfg.output) {
            return Err(DriverError::Unsupported("the device lacks that direction".into()));
        }
        let open_one = |dir| -> Result<Pcm, DriverError> {
            let mut p = Pcm::open(e.card, e.device, dir).map_err(map_io)?;
            p.configure(&req).map_err(map_io)?;
            Ok(p)
        };
        let out = if cfg.output { Some(open_one(Direction::Playback)?) } else { None };
        let inp = if cfg.input { Some(open_one(Direction::Capture)?) } else { None };
        let rate = out.as_ref().or(inp.as_ref()).map_or(0, |p| p.rate);
        if let (Some(o), Some(i)) = (&out, &inp) {
            if o.rate != i.rate {
                return Err(DriverError::Unsupported(format!("capture runs at {} Hz, playback at {} Hz", i.rate, o.rate)));
            }
            o.link(i).map_err(map_io)?;
        }
        let buffer = out.as_ref().or(inp.as_ref()).map_or(0, |p| p.buffer_frames);
        let prefill = (cfg.quantum * 2).min(buffer / 2).max(cfg.quantum.min(buffer));
        dlog!("opened {} {}Hz quantum {} prefill {} buffer {}", e.desc.uid, rate, cfg.quantum, prefill, buffer);
        Ok(AlsaIo { out, inp, dll: Dll::new(rate as f64, cfg.quantum), quantum: cfg.quantum, prefill, last_ts: 0, xruns: 0, conv_out: 0, conv_in: 0, started: false, gone: false, restarted: None })
    }

    /// Cycles lost to under- and overruns since open.
    pub fn xruns(&self) -> u64 {
        self.xruns
    }

    /// The DLL, for monitoring (`rate_hz`, `bandwidth`).
    pub fn dll(&self) -> &Dll {
        &self.dll
    }

    /// The granted rate in Hz.
    pub fn rate(&self) -> u32 {
        self.out.as_ref().or(self.inp.as_ref()).map_or(0, |p| p.rate)
    }

    /// Frames queued ahead of the hardware on the playback side.
    pub fn output_fill(&mut self) -> i64 {
        match self.out.as_mut() {
            Some(o) => {
                let hw = o.status().hw_raw;
                crate::ring::ptr_diff(o.appl(), hw, o.boundary)
            }
            None => 0,
        }
    }

    /// Checks a started stream and recovers it: an `XRUN` or other stop restarts it (`prepare`, refill, `start`),
    /// `SUSPENDED` tries `RESUME`, `DISCONNECTED` ends it. Returns true when this cycle can use the ring; false
    /// when the cycle is lost (the caller reports an `Xrun`).
    fn recover(&mut self) -> bool {
        if !self.started {
            return true;
        }
        if self.gone {
            return false;
        }
        let Some(state) = self.primary().map(|p| p.status().state) else { return true };
        let mut action = recovery_for(state);
        if action == Recovery::Resume {
            let mut err = None;
            for p in [self.out.as_ref(), self.inp.as_ref()].into_iter().flatten() {
                if let Err(e) = p.resume() {
                    err = err.or(Some(e.raw_os_error().unwrap_or(libc::EIO)));
                }
            }
            action = match after_resume(err) {
                // Resumed: the stream runs again but this cycle's timing is gone.
                Recovery::None => Recovery::Restart,
                other => other,
            };
        }
        match action {
            Recovery::None => true,
            Recovery::Resume => false,
            Recovery::Gone => {
                self.gone = true;
                dlog!("device disconnected");
                false
            }
            Recovery::Restart => {
                dlog!("stream left RUNNING (state {state}), restarting");
                match self.start() {
                    // The counter starts over and the DLL was reset: the server re-anchors on this stamp.
                    Ok(z) => self.restarted = Some(z),
                    Err(DriverError::NotFound) | Err(DriverError::Busy) => self.gone = true,
                    Err(e) => dlog!("restart failed: {e:?}"),
                }
                false
            }
        }
    }

    fn primary(&mut self) -> Option<&mut Pcm> {
        self.out.as_mut().or(self.inp.as_mut())
    }
}

impl DeviceIo for AlsaIo {
    fn start(&mut self) -> Result<ZeroTimeStamp, DriverError> {
        for p in [self.out.as_mut(), self.inp.as_mut()].into_iter().flatten() {
            p.prepare().map_err(map_io)?;
        }
        if let Some(o) = self.out.as_mut() {
            let layout = o.layout;
            layout.silence(o.ring_mut(), 0, self.prefill as usize, None);
            o.commit_appl(self.prefill as u64).map_err(map_io)?;
        }
        self.primary().ok_or(DriverError::NotFound)?.start().map_err(map_io)?;
        self.started = true;
        let now = now_raw_ns();
        // converter latency, read once per start (an ioctl, never in the cycle)
        // capture: the FIFO plus the delay beyond the frames already captured (the converter's own latency)
        self.conv_in = self.inp.as_mut().map_or(0, |p| p.converter_latency());
        self.conv_out = self.out.as_ref().map_or(0, |o| {
            let queued = self.prefill as i64;
            o.fifo_frames() + o.delay_frames().map_or(0, |d| converter_delay(d, queued))
        });
        dlog!("converter latency: playback {} frames, capture {} frames", self.conv_out, self.conv_in);
        self.dll.reset(0, now);
        self.last_ts = 0;
        Ok(ZeroTimeStamp { sample_time: 0, host_ns: now })
    }

    fn position(&mut self) -> HwPosition {
        let Some(p) = self.out.as_mut().or(self.inp.as_mut()) else { return HwPosition::default() };
        if p.no_period_wakeup {
            let _ = p.hwsync();
        }
        let s = p.status();
        let running = s.state == STATE_RUNNING;
        if running && s.tstamp_ns != 0 && s.tstamp_ns != self.last_ts {
            self.last_ts = s.tstamp_ns;
            self.dll.update(s.hw_frames, s.tstamp_ns);
        }
        HwPosition { hw_frames: s.hw_frames, host_ns: s.tstamp_ns, running }
    }

    fn read_input(&mut self, frames: u32, dst: &mut BufferList) -> Result<(), Xrun> {
        if self.inp.is_none() {
            return Ok(());
        }
        if !self.recover() {
            self.xruns += 1;
            dst.clear();
            return Err(Xrun { frames_lost: frames });
        }
        let Some(p) = self.inp.as_mut() else { return Ok(()) };
        let hw = p.status().hw_raw;
        let plan = plan_capture(p.appl(), hw, p.buffer_frames, p.boundary, frames);
        let (read_at, lost) = match plan {
            CapturePlan::Read { read_at } => (read_at, 0),
            CapturePlan::Overrun { lost, read_at } => (read_at, lost),
            CapturePlan::NotReady { avail } => {
                dst.clear();
                self.xruns += 1;
                return Err(Xrun { frames_lost: frames - avail.min(frames) });
            }
        };
        p.layout.unpack(p.ring(), read_at, &mut dst.buffers, frames as usize);
        if p.advance_to(read_at, frames as u64).is_err() {
            return Err(Xrun { frames_lost: frames });
        }
        if lost > 0 {
            self.xruns += 1;
            return Err(Xrun { frames_lost: lost });
        }
        Ok(())
    }

    fn write_mix(&mut self, frames: u32, src: &BufferList) -> Result<(), Xrun> {
        if self.out.is_none() {
            return Ok(());
        }
        if !self.recover() {
            self.xruns += 1;
            return Err(Xrun { frames_lost: frames });
        }
        let Some(p) = self.out.as_mut() else { return Ok(()) };
        let hw = p.status().hw_raw;
        let layout = p.layout;
        let plan = plan_playback(p.appl(), hw, p.buffer_frames, p.boundary, frames, self.prefill);
        let (write_at, lost) = match plan {
            PlaybackPlan::Write { write_at } => (write_at, 0),
            PlaybackPlan::Underrun { lost, silence_from, silence_frames, write_at } => {
                layout.silence(p.ring_mut(), silence_from, silence_frames as usize, None);
                (write_at, lost)
            }
            PlaybackPlan::Overflow => {
                self.xruns += 1;
                return Err(Xrun { frames_lost: frames });
            }
        };
        layout.pack(p.ring_mut(), write_at, &src.buffers, frames as usize);
        if p.advance_to(write_at, frames as u64).is_err() {
            return Err(Xrun { frames_lost: frames });
        }
        if lost > 0 {
            self.xruns += 1;
            return Err(Xrun { frames_lost: lost });
        }
        Ok(())
    }

    fn abort_hook(&self) -> Option<Box<dyn Fn() + Send + Sync>> {
        let hooks: Vec<_> = [self.out.as_ref(), self.inp.as_ref()].into_iter().flatten().filter_map(|p| p.abort_fn()).collect();
        if hooks.is_empty() {
            return None;
        }
        Some(Box::new(move || hooks.iter().for_each(|h| h())))
    }

    fn stop(&mut self) {
        self.started = false;
        self.restarted = None;
        if let Some(p) = self.primary() {
            let _ = p.drop_stream();
        }
    }

    fn latency(&self) -> Latency {
        // Frames queued ahead of the hardware (output) and one capture period (input), plus the converter latency
        // measured at start: the hardware FIFO and the delay `PCM_DELAY` reports beyond the queued frames.
        Latency {
            input: if self.inp.is_some() { self.quantum + self.conv_in } else { 0 },
            output: if self.out.is_some() { self.prefill + self.conv_out } else { 0 },
            input_safety: 0,
            output_safety: 0,
        }
    }

    fn physical_formats(&self) -> (Option<StreamFormat>, Option<StreamFormat>) {
        let f = |p: &Pcm| physical_format(p.rate, p.channels, p.layout.format, p.layout.interleaved);
        (self.inp.as_ref().map(f), self.out.as_ref().map(f))
    }

    fn rate_scalar(&self) -> f64 {
        self.dll.rate_scalar()
    }

    fn host_time_of(&self, hw_frames: u64) -> Option<u64> {
        (self.dll.updates() > 0).then(|| self.dll.host_time_of(hw_frames))
    }

    fn take_restart(&mut self) -> Option<ZeroTimeStamp> {
        self.restarted.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctl::DbScale;

    fn control(name: &str, kind: Kind, count: u32, min: i64, max: i64) -> Control {
        Control { numid: 5, iface: IFACE_MIXER, device: 0, name: name.into(), kind, count, min, max, step: 1, items: vec![], values: vec![], db: None }
    }

    fn caps(ch: u32, rates: (u32, u32), buf: (u32, u32)) -> PcmCaps {
        PcmCaps { channels: (1, ch), rates, buffer_frames: buf, formats: vec![SampleFormat::S32], no_period_wakeup: true }
    }

    #[test]
    fn a_two_period_ring_halves_the_smallest_quantum() {
        assert_eq!(quantum_range((64, 2048), true), (32, 2048));
        assert_eq!(quantum_range((64, 2048), false), (64, 2048));
        assert_eq!(quantum_range((1, 8), true), (1, 8));
    }

    #[test]
    fn transport_classes() {
        assert_eq!(transport_of("HDA-Intel", "ALC233 Analog"), "pci");
        assert_eq!(transport_of("HDA-Intel", "HDMI 0"), "hdmi");
        assert_eq!(transport_of("USB-Audio", "USB Audio"), "usb");
        assert_eq!(transport_of("HDA-Intel", "DisplayPort 1"), "hdmi");
    }

    #[test]
    fn duplex_caps_merge_into_one_description() {
        let play = caps(2, (44_100, 192_000), (64, 16_384));
        let cap = caps(2, (8_000, 96_000), (32, 8_192));
        let (inch, outch, rates, buf) = merge_caps(Some(&play), Some(&cap));
        assert_eq!((inch, outch), (2, 2));
        assert_eq!(rates, vec![(8_000.0, 192_000.0)]);
        assert_eq!(buf, (32, 16_384));
        // playback only (HDMI)
        let (inch, outch, _, buf) = merge_caps(Some(&caps(8, (32_000, 192_000), (128, 4096))), None);
        assert_eq!((inch, outch, buf), (0, 8, (128, 4096)));
        assert_eq!(merge_caps(None, None), (0, 0, vec![], (0, 0)));
    }

    #[test]
    fn jacks_map_to_their_devices() {
        let devs = [(0, false), (2, false), (3, true), (7, true)];
        assert_eq!(jack_targets("Headphone Jack", &devs), vec![0, 2]);
        assert_eq!(jack_targets("HDMI/DP,pcm=3 Jack", &devs), vec![3]);
        assert_eq!(jack_targets("HDMI/DP,pcm=9 Jack", &devs), Vec::<u32>::new());
    }

    #[test]
    fn mixer_elements_are_classified() {
        assert_eq!(classify(&control("Master Playback Volume", Kind::Int, 2, 0, 87)), Some("volume"));
        assert_eq!(classify(&control("Master Playback Volume", Kind::Int, 2, 0, 0)), None, "a fixed control is not offered");
        assert_eq!(classify(&control("Master Playback Switch", Kind::Bool, 2, 0, 1)), Some("mute"));
        assert_eq!(classify(&control("Headphone Jack", Kind::Bool, 1, 0, 1)), None, "jacks are events, not controls");
        let mut e = control("Capture Source", Kind::Enum, 1, 0, 1);
        e.items = vec!["Mic".into(), "Line".into()];
        assert_eq!(classify(&e), Some("data-source"));
        e.name = "Auto-Mute Mode".into();
        assert_eq!(classify(&e), Some("selector"));
        let mut other = control("Master Playback Volume", Kind::Int, 2, 0, 87);
        other.iface = 3;
        assert_eq!(classify(&other), None);
    }

    #[test]
    fn direction_of_a_control_follows_its_name() {
        assert!(is_output("Master Playback Volume"));
        assert!(is_output("Speaker Playback Switch"));
        assert!(!is_output("Capture Volume"));
        assert!(!is_output("Mic Boost Volume"));
        assert!(!is_output("Input Source"));
    }

    #[test]
    fn control_descriptions_carry_the_db_range() {
        let mut c = control("Master Playback Volume", Kind::Int, 2, 0, 87);
        c.db = Some(DbScale { min_cdb: -6525, step_cdb: 75, mute_at_min: true });
        let d = describe_control(&c).unwrap();
        assert_eq!((d.id, d.kind.as_str(), d.output), (ControlId(5), "volume", true));
        // -65.25 dB + 87 * 0.75 dB = 0 dB
        assert_eq!(d.db_range, (-65.25, 0.0));
    }

    #[test]
    fn volume_scalars_spread_over_the_raw_steps_on_every_channel() {
        let c = control("Master Playback Volume", Kind::Int, 2, 0, 127);
        assert_eq!(control_values(&c, &ControlValue::Scalar(0.0)).unwrap(), vec![0, 0]);
        assert_eq!(control_values(&c, &ControlValue::Scalar(1.0)).unwrap(), vec![127, 127]);
        // 0.5 * 127 = 63.5 rounds away from zero
        assert_eq!(control_values(&c, &ControlValue::Scalar(0.5)).unwrap(), vec![64, 64]);
        assert_eq!(control_values(&c, &ControlValue::Scalar(7.0)).unwrap(), vec![127, 127], "clamped");
        let off = control("Capture Volume", Kind::Int, 1, 10, 20);
        assert_eq!(control_values(&off, &ControlValue::Scalar(0.5)).unwrap(), vec![15]);
    }

    #[test]
    fn mute_inverts_the_switch() {
        let c = control("Master Playback Switch", Kind::Bool, 2, 0, 1);
        assert_eq!(control_values(&c, &ControlValue::Bool(true)).unwrap(), vec![0, 0]);
        assert_eq!(control_values(&c, &ControlValue::Bool(false)).unwrap(), vec![1, 1]);
    }

    #[test]
    fn selectors_take_an_index_in_range() {
        let mut e = control("Input Source", Kind::Enum, 1, 0, 1);
        e.items = vec!["Mic".into(), "Line".into()];
        assert_eq!(control_values(&e, &ControlValue::Index(1)).unwrap(), vec![1]);
        assert!(control_values(&e, &ControlValue::Index(2)).is_err());
        assert!(control_values(&e, &ControlValue::Scalar(0.5)).is_err(), "wrong value kind");
    }

    #[test]
    fn physical_formats_describe_the_ring() {
        let f = physical_format(48_000, 2, SampleFormat::S32, true);
        assert_eq!((f.sample_rate, f.channels, f.bits_per_channel, f.bytes_per_frame, f.bytes_per_packet), (48_000.0, 2, 32, 8, 8));
        assert!(f.is_interleaved() && !f.is_float());
        assert!(f.flags.contains(FormatFlags::SIGNED_INTEGER) && f.flags.contains(FormatFlags::PACKED));
        // S24_LE sits in a 32-bit container and is not packed
        let f = physical_format(44_100, 2, SampleFormat::S24, true);
        assert_eq!((f.bits_per_channel, f.bytes_per_frame), (24, 8));
        assert!(!f.flags.contains(FormatFlags::PACKED));
        // non-interleaved: one buffer per channel
        let f = physical_format(48_000, 6, SampleFormat::S16, false);
        assert_eq!((f.bytes_per_frame, f.channels), (2, 6));
        assert!(!f.is_interleaved());
    }

    #[test]
    fn io_errors_map_to_driver_errors() {
        assert_eq!(map_io(io::Error::from_raw_os_error(libc::EBUSY)), DriverError::Busy);
        assert_eq!(map_io(io::Error::from_raw_os_error(libc::ENOENT)), DriverError::NotFound);
        assert!(matches!(map_io(io::Error::from_raw_os_error(libc::EIO)), DriverError::Io(_)));
        assert!(matches!(map_io(io::Error::new(io::ErrorKind::Unsupported, "x")), DriverError::Unsupported(_)));
    }

    #[test]
    fn uids_name_the_card_and_device() {
        assert_eq!(uid_of("PCH", 0), "alsa:PCH:0");
        assert_eq!(uid_of("HDMI", 3), "alsa:HDMI:3");
    }

    #[test]
    fn the_driver_starts_empty_and_names_itself() {
        let mut d = AlsaDriver::new();
        assert_eq!(d.name(), "alsa");
        assert!(d.controls("alsa:none:0").is_empty());
        assert_eq!(d.open("alsa:none:0", &IoConfig { rate: 48_000.0, quantum: 64, input: false, output: true }).err(), Some(DriverError::NotFound));
        assert_eq!(d.set_control("alsa:none:0", ControlId(1), ControlValue::Bool(true)), Err(DriverError::NotFound));
    }
}
