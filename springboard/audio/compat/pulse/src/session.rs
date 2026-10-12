//! One client connection's protocol state machine: command in, reply frames out. It owns the client's streams
//! (a ring and a [`StreamCtl`] each, shared with the stream's IO procedure) and talks to the audio server through
//! [`Backend`]. It never blocks and does no socket IO, so every behaviour is testable with the mock backend.

use crate::backend::{Backend, DeviceInfo, StreamCtl, StreamHandle, StreamParams};
use crate::proto::{cmd, command_frame, data_frame, default_channel_map, err, sub, volume_to_gain, Descriptor, SampleFormat, COMMAND_CHANNEL, INVALID, PROTOCOL_VERSION, VERSION_MASK, VOLUME_NORM};
use crate::ring::{Resampler, Ring, MAX_CHANNELS};
use crate::tagstruct::{Malformed, Reader, Writer};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Sink and source flags: hardware volume and mute control, latency reporting, a hardware device.
const DEVICE_FLAGS: u32 = 0x0001 | 0x0002 | 0x0004 | 0x0010;
/// The sample format sinks and sources report (the server's internal one).
const DEVICE_FORMAT: u8 = 5;
const DRIVER: &str = "bifrost-audio";
const SERVER_NAME: &str = "bifrost-audio-pulse";
const SERVER_VERSION: &str = "16.1";
/// The default playback buffer: 100 ms.
const DEFAULT_BUFFER_DIV: u32 = 10;

enum Fail {
    Malformed,
    Code(u32),
}

impl From<Malformed> for Fail {
    fn from(_: Malformed) -> Fail {
        Fail::Malformed
    }
}

type Res = Result<(), Fail>;

struct Playback {
    index: u32,
    name: String,
    fmt: SampleFormat,
    channels: usize,
    rate: u32,
    frame_bytes: usize,
    sink: String,
    sink_index: u32,
    ring: Arc<Ring>,
    ctl: Arc<StreamCtl>,
    _handle: Box<dyn StreamHandle>,
    maxlength: usize,
    tlength: usize,
    prebuf: usize,
    minreq: usize,
    /// Bytes requested from the client and not yet received.
    outstanding: usize,
    written: u64,
    corked: bool,
    prebuffered: bool,
    volume: Vec<u32>,
    mute: bool,
    drain_tag: Option<u32>,
    last_xruns: u32,
    had_data: bool,
    underflow_sent: bool,
    props: Vec<(String, Vec<u8>)>,
    scratch: Vec<f32>,
}

impl Playback {
    fn avail_frames(&self) -> usize {
        self.ring.available() / self.channels
    }
    fn apply_gain(&self) {
        let g = if self.mute { 0.0 } else { avg_gain(&self.volume) };
        self.ctl.set_gain(g);
    }
}

struct Record {
    index: u32,
    name: String,
    fmt: SampleFormat,
    channels: usize,
    rate: u32,
    frame_bytes: usize,
    source: String,
    source_index: u32,
    ring: Arc<Ring>,
    ctl: Arc<StreamCtl>,
    _handle: Box<dyn StreamHandle>,
    resampler: Resampler,
    maxlength: usize,
    fragsize: usize,
    corked: bool,
    delivered: u64,
    last_xruns: u32,
    volume: Vec<u32>,
    mute: bool,
    props: Vec<(String, Vec<u8>)>,
    scratch: Vec<f32>,
}

fn avg_gain(vols: &[u32]) -> f32 {
    if vols.is_empty() {
        return 1.0;
    }
    let sum: u64 = vols.iter().map(|&v| v as u64).sum();
    volume_to_gain((sum / vols.len() as u64) as u32)
}

fn now_timeval() -> (u32, u32) {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (d.as_secs() as u32, d.subsec_micros())
}

/// Resolves the buffer attributes the way the reference server defaults them, in bytes, whole frames.
/// Returns (maxlength, tlength, prebuf, minreq).
pub fn playback_attrs(rate: u32, frame_bytes: usize, maxlength: u32, tlength: u32, prebuf: u32, minreq: u32) -> (usize, usize, usize, usize) {
    let fb = frame_bytes;
    let whole = |v: usize| (v / fb).max(1) * fb;
    let t = whole(if tlength == INVALID { (rate / DEFAULT_BUFFER_DIV) as usize * fb } else { tlength as usize });
    let m = whole(if minreq == INVALID { t / 4 } else { minreq as usize }).min(t);
    let p = if prebuf == INVALID { t } else { (prebuf as usize / fb) * fb }.min(t);
    let max = whole(if maxlength == INVALID { t * 4 } else { maxlength as usize }).max(t);
    (max, t, p, m)
}

pub struct Session {
    client_index: u32,
    version: u32,
    authed: bool,
    name: String,
    props: Vec<(String, Vec<u8>)>,
    subscribe: u32,
    playback: BTreeMap<u32, Playback>,
    record: BTreeMap<u32, Record>,
    next_channel: u32,
    next_stream: u32,
    default_sink: Option<String>,
    default_source: Option<String>,
    dev_state: BTreeMap<(bool, String), (Vec<u32>, bool)>,
    out: Vec<Vec<u8>>,
    pub closed: bool,
}

impl Session {
    pub fn new(client_index: u32) -> Session {
        Session {
            client_index,
            version: 0,
            authed: false,
            name: String::new(),
            props: Vec::new(),
            subscribe: 0,
            playback: BTreeMap::new(),
            record: BTreeMap::new(),
            next_channel: 0,
            next_stream: 0,
            default_sink: None,
            default_source: None,
            dev_state: BTreeMap::new(),
            out: Vec::new(),
            closed: false,
        }
    }

    pub fn has_streams(&self) -> bool {
        !self.playback.is_empty() || !self.record.is_empty()
    }

    // ---- output helpers ----

    fn reply(&mut self, tag: u32, f: impl FnOnce(&mut Writer)) {
        let mut w = Writer::new();
        w.u32(cmd::REPLY).u32(tag);
        f(&mut w);
        self.out.push(command_frame(&w.buf));
    }

    fn ack(&mut self, tag: u32) {
        self.reply(tag, |_| {});
    }

    fn error(&mut self, tag: u32, code: u32) {
        let mut w = Writer::new();
        w.u32(cmd::ERROR).u32(tag).u32(code);
        self.out.push(command_frame(&w.buf));
    }

    /// A server-initiated command (tag -1).
    fn notify(&mut self, command: u32, f: impl FnOnce(&mut Writer)) {
        let mut w = Writer::new();
        w.u32(command).u32(COMMAND_CHANNEL);
        f(&mut w);
        self.out.push(command_frame(&w.buf));
    }

    fn event(&mut self, facility: u32, kind: u32, index: u32) {
        if self.subscribe & sub::mask_bit(facility) != 0 {
            self.notify(cmd::SUBSCRIBE_EVENT, |w| {
                w.u32(facility | kind).u32(index);
            });
        }
    }

    fn take_out(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.out)
    }

    // ---- device helpers ----

    fn default_sink_name(&self, be: &dyn Backend) -> String {
        let sinks = be.sinks();
        if let Some(n) = &self.default_sink {
            if sinks.iter().any(|d| &d.name == n) {
                return n.clone();
            }
        }
        sinks.first().map(|d| d.name.clone()).unwrap_or_default()
    }

    fn default_source_name(&self, be: &dyn Backend) -> String {
        let sources = be.sources();
        if let Some(n) = &self.default_source {
            if sources.iter().any(|d| &d.name == n) {
                return n.clone();
            }
        }
        sources.first().map(|d| d.name.clone()).unwrap_or_default()
    }

    /// Resolves (index, name) of a sink or source from the wire's index-or-name pair.
    fn resolve(&self, be: &dyn Backend, sink: bool, index: u32, name: Option<String>) -> Result<(u32, DeviceInfo), Fail> {
        let list = if sink { be.sinks() } else { be.sources() };
        let default = if sink { self.default_sink_name(be) } else { self.default_source_name(be) };
        let want = match name.as_deref() {
            Some("@DEFAULT_SINK@") | Some("@DEFAULT_SOURCE@") | Some("") => Some(default.clone()),
            Some(n) => Some(n.to_string()),
            None => None,
        };
        let pos = match want {
            Some(n) => list.iter().position(|d| d.name == n),
            None if index != INVALID => Some(index as usize).filter(|&i| i < list.len()),
            None => list.iter().position(|d| d.name == default),
        };
        let i = pos.ok_or(Fail::Code(err::NOENTITY))?;
        Ok((i as u32, list[i].clone()))
    }

    fn state_of(&self, sink: bool, d: &DeviceInfo) -> (Vec<u32>, bool) {
        self.dev_state.get(&(sink, d.name.clone())).cloned().unwrap_or_else(|| (vec![VOLUME_NORM; d.channels as usize], false))
    }

    fn write_device(&self, w: &mut Writer, sink: bool, index: u32, d: &DeviceInfo) {
        let (vol, mute) = self.state_of(sink, d);
        w.u32(index).string(Some(&d.name)).string(Some(&d.description));
        w.sample_spec(DEVICE_FORMAT, d.channels, d.rate).channel_map(&default_channel_map(d.channels));
        w.u32(INVALID).cvolume(&vol).bool(mute);
        // the monitor source (of a sink) or the monitored sink (of a source): none
        w.u32(INVALID).string(None);
        w.usec(d.latency_us).string(Some(DRIVER)).u32(DEVICE_FLAGS);
        w.proplist(&[("device.description".to_string(), cstr(&d.description))]);
        w.usec(d.latency_us);
        w.volume(VOLUME_NORM).u32(0).u32(65537).u32(INVALID);
        // ports: none; the active port: none
        w.u32(0).string(None);
        w.u8(1).format_info(1);
    }

    // ---- entry points ----

    /// Handles one received packet and returns the frames to send.
    pub fn handle(&mut self, be: &mut dyn Backend, d: &Descriptor, payload: &[u8]) -> Vec<Vec<u8>> {
        if d.channel != COMMAND_CHANNEL {
            self.on_data(d.channel, payload);
            return self.take_out();
        }
        let mut r = Reader::new(payload);
        let (command, tag) = match (r.u32(), r.u32()) {
            (Ok(c), Ok(t)) => (c, t),
            _ => {
                self.closed = true;
                return Vec::new();
            }
        };
        if !self.authed && command != cmd::AUTH {
            self.error(tag, err::ACCESS);
            return self.take_out();
        }
        match self.dispatch(be, command, tag, &mut r) {
            Ok(()) => {}
            Err(Fail::Malformed) => self.error(tag, err::PROTOCOL),
            Err(Fail::Code(c)) => self.error(tag, c),
        }
        self.take_out()
    }

    fn dispatch(&mut self, be: &mut dyn Backend, command: u32, tag: u32, r: &mut Reader) -> Res {
        match command {
            cmd::AUTH => {
                let v = r.u32()?;
                let _cookie = r.arbitrary()?;
                let client = v & VERSION_MASK;
                if client < 13 {
                    return Err(Fail::Code(err::VERSION));
                }
                self.version = client.min(PROTOCOL_VERSION);
                self.authed = true;
                self.reply(tag, |w| {
                    w.u32(PROTOCOL_VERSION);
                });
            }
            cmd::SET_CLIENT_NAME => {
                self.props = r.proplist()?;
                self.name = self.props.iter().find(|(k, _)| k == "application.name").map(|(_, v)| String::from_utf8_lossy(v).trim_end_matches('\0').to_string()).unwrap_or_default();
                let idx = self.client_index;
                self.reply(tag, |w| {
                    w.u32(idx);
                });
                self.event(sub::CLIENT, sub::NEW, idx);
            }
            cmd::UPDATE_CLIENT_PROPLIST => {
                let _mode = r.u32()?;
                let list = r.proplist()?;
                self.props.extend(list);
                self.ack(tag);
            }
            cmd::REMOVE_CLIENT_PROPLIST | cmd::KILL_CLIENT | cmd::SUSPEND_SINK | cmd::SUSPEND_SOURCE => self.ack(tag),
            cmd::EXIT => {
                self.ack(tag);
                self.closed = true;
            }
            cmd::GET_SERVER_INFO => {
                let sink = self.default_sink_name(be);
                let source = self.default_source_name(be);
                let user = std::env::var("USER").unwrap_or_else(|_| "audio".to_string());
                let host = hostname();
                self.reply(tag, |w| {
                    w.string(Some(SERVER_NAME)).string(Some(SERVER_VERSION)).string(Some(&user)).string(Some(&host));
                    w.sample_spec(DEVICE_FORMAT, 2, 48000);
                    w.string(Some(&sink)).string(Some(&source)).u32(0xB1F0_0001);
                    w.channel_map(&default_channel_map(2));
                });
            }
            cmd::STAT => {
                let total: usize = self.playback.values().map(|p| p.ring.capacity() * 4).sum();
                self.reply(tag, |w| {
                    w.u32(0).u32(0).u32(0).u32(total as u32).u32(0);
                });
            }
            cmd::LOOKUP_SINK | cmd::LOOKUP_SOURCE => {
                let name = r.string()?;
                let (i, _) = self.resolve(be, command == cmd::LOOKUP_SINK, INVALID, Some(name.unwrap_or_default()))?;
                self.reply(tag, |w| {
                    w.u32(i);
                });
            }
            cmd::GET_SINK_INFO | cmd::GET_SOURCE_INFO => {
                let sink = command == cmd::GET_SINK_INFO;
                let index = r.u32()?;
                let name = r.string()?;
                let (i, d) = self.resolve(be, sink, index, name)?;
                let mut w = Writer::new();
                self.write_device(&mut w, sink, i, &d);
                self.reply(tag, |out| out.buf.extend_from_slice(&w.buf));
            }
            cmd::GET_SINK_INFO_LIST | cmd::GET_SOURCE_INFO_LIST => {
                let sink = command == cmd::GET_SINK_INFO_LIST;
                let list = if sink { be.sinks() } else { be.sources() };
                let mut w = Writer::new();
                for (i, d) in list.iter().enumerate() {
                    self.write_device(&mut w, sink, i as u32, d);
                }
                self.reply(tag, |out| out.buf.extend_from_slice(&w.buf));
            }
            cmd::GET_CLIENT_INFO | cmd::GET_CLIENT_INFO_LIST => {
                let mut w = Writer::new();
                w.u32(self.client_index).string(Some(&self.name)).u32(INVALID).string(Some(DRIVER)).proplist(&self.props);
                self.reply(tag, |out| out.buf.extend_from_slice(&w.buf));
            }
            cmd::GET_MODULE_INFO_LIST | cmd::GET_SAMPLE_INFO_LIST | 89 => self.ack(tag),
            cmd::GET_SINK_INPUT_INFO | cmd::GET_SINK_INPUT_INFO_LIST => {
                let wanted = if command == cmd::GET_SINK_INPUT_INFO { Some(r.u32()?) } else { None };
                if let Some(i) = wanted {
                    if !self.playback.values().any(|p| p.index == i) {
                        return Err(Fail::Code(err::NOENTITY));
                    }
                }
                let mut w = Writer::new();
                for p in self.playback.values().filter(|p| wanted.map_or(true, |i| p.index == i)) {
                    self.write_sink_input(&mut w, p);
                }
                self.reply(tag, |out| out.buf.extend_from_slice(&w.buf));
            }
            cmd::GET_SOURCE_OUTPUT_INFO | cmd::GET_SOURCE_OUTPUT_INFO_LIST => {
                let wanted = if command == cmd::GET_SOURCE_OUTPUT_INFO { Some(r.u32()?) } else { None };
                if let Some(i) = wanted {
                    if !self.record.values().any(|p| p.index == i) {
                        return Err(Fail::Code(err::NOENTITY));
                    }
                }
                let mut w = Writer::new();
                for s in self.record.values().filter(|s| wanted.map_or(true, |i| s.index == i)) {
                    self.write_source_output(&mut w, s);
                }
                self.reply(tag, |out| out.buf.extend_from_slice(&w.buf));
            }
            cmd::SUBSCRIBE => {
                self.subscribe = r.u32()?;
                self.ack(tag);
            }
            cmd::SET_SINK_VOLUME | cmd::SET_SOURCE_VOLUME => {
                let sink = command == cmd::SET_SINK_VOLUME;
                let index = r.u32()?;
                let name = r.string()?;
                let vol = r.cvolume()?;
                let (i, d) = self.resolve(be, sink, index, name)?;
                let (_, mute) = self.state_of(sink, &d);
                be.set_device_volume(sink, &d.name, avg_gain(&vol), mute);
                self.dev_state.insert((sink, d.name.clone()), (vol, mute));
                self.ack(tag);
                self.event(if sink { sub::SINK } else { sub::SOURCE }, sub::CHANGE, i);
            }
            cmd::SET_SINK_MUTE | cmd::SET_SOURCE_MUTE => {
                let sink = command == cmd::SET_SINK_MUTE;
                let index = r.u32()?;
                let name = r.string()?;
                let mute = r.bool()?;
                let (i, d) = self.resolve(be, sink, index, name)?;
                let (vol, _) = self.state_of(sink, &d);
                be.set_device_volume(sink, &d.name, avg_gain(&vol), mute);
                self.dev_state.insert((sink, d.name.clone()), (vol, mute));
                self.ack(tag);
                self.event(if sink { sub::SINK } else { sub::SOURCE }, sub::CHANGE, i);
            }
            cmd::SET_DEFAULT_SINK | cmd::SET_DEFAULT_SOURCE => {
                let sink = command == cmd::SET_DEFAULT_SINK;
                let name = r.string()?;
                let (_, d) = self.resolve(be, sink, INVALID, name)?;
                if sink {
                    self.default_sink = Some(d.name);
                } else {
                    self.default_source = Some(d.name);
                }
                self.ack(tag);
                self.event(sub::SERVER, sub::CHANGE, INVALID);
            }
            cmd::CREATE_PLAYBACK_STREAM => self.create_playback(be, tag, r)?,
            cmd::CREATE_RECORD_STREAM => self.create_record(be, tag, r)?,
            cmd::DELETE_PLAYBACK_STREAM => {
                let ch = r.u32()?;
                let p = self.playback.remove(&ch).ok_or(Fail::Code(err::NOENTITY))?;
                self.ack(tag);
                self.event(sub::SINK_INPUT, sub::REMOVE, p.index);
            }
            cmd::DELETE_RECORD_STREAM => {
                let ch = r.u32()?;
                let s = self.record.remove(&ch).ok_or(Fail::Code(err::NOENTITY))?;
                self.ack(tag);
                self.event(sub::SOURCE_OUTPUT, sub::REMOVE, s.index);
            }
            cmd::KILL_SINK_INPUT | cmd::KILL_SOURCE_OUTPUT => {
                let idx = r.u32()?;
                if command == cmd::KILL_SINK_INPUT {
                    let ch = self.playback.iter().find(|(_, p)| p.index == idx).map(|(c, _)| *c).ok_or(Fail::Code(err::NOENTITY))?;
                    self.playback.remove(&ch);
                    self.ack(tag);
                    self.notify(cmd::PLAYBACK_STREAM_KILLED, |w| {
                        w.u32(ch);
                    });
                    self.event(sub::SINK_INPUT, sub::REMOVE, idx);
                } else {
                    let ch = self.record.iter().find(|(_, s)| s.index == idx).map(|(c, _)| *c).ok_or(Fail::Code(err::NOENTITY))?;
                    self.record.remove(&ch);
                    self.ack(tag);
                    self.notify(cmd::RECORD_STREAM_KILLED, |w| {
                        w.u32(ch);
                    });
                    self.event(sub::SOURCE_OUTPUT, sub::REMOVE, idx);
                }
            }
            cmd::DRAIN_PLAYBACK_STREAM => {
                let ch = r.u32()?;
                let p = self.playback.get_mut(&ch).ok_or(Fail::Code(err::NOENTITY))?;
                p.drain_tag = Some(tag);
                p.prebuffered = true;
            }
            cmd::CORK_PLAYBACK_STREAM => {
                let ch = r.u32()?;
                let cork = r.bool()?;
                self.playback.get_mut(&ch).ok_or(Fail::Code(err::NOENTITY))?.corked = cork;
                self.ack(tag);
            }
            cmd::TRIGGER_PLAYBACK_STREAM => {
                let ch = r.u32()?;
                self.playback.get_mut(&ch).ok_or(Fail::Code(err::NOENTITY))?.prebuffered = true;
                self.ack(tag);
            }
            cmd::PREBUF_PLAYBACK_STREAM => {
                let ch = r.u32()?;
                let p = self.playback.get_mut(&ch).ok_or(Fail::Code(err::NOENTITY))?;
                if p.prebuf > 0 {
                    p.prebuffered = false;
                }
                self.ack(tag);
            }
            cmd::FLUSH_PLAYBACK_STREAM => {
                let ch = r.u32()?;
                let p = self.playback.get_mut(&ch).ok_or(Fail::Code(err::NOENTITY))?;
                p.written -= (p.ring.available() * p.fmt.bytes()) as u64;
                p.ring.request_flush();
                p.outstanding = 0;
                if p.prebuf > 0 {
                    p.prebuffered = false;
                }
                self.ack(tag);
            }
            cmd::CORK_RECORD_STREAM => {
                let ch = r.u32()?;
                let cork = r.bool()?;
                self.record.get_mut(&ch).ok_or(Fail::Code(err::NOENTITY))?.corked = cork;
                self.ack(tag);
            }
            cmd::FLUSH_RECORD_STREAM => {
                let ch = r.u32()?;
                let s = self.record.get_mut(&ch).ok_or(Fail::Code(err::NOENTITY))?;
                let mut sink = [0.0f32; 256];
                while s.ring.read(&mut sink) > 0 {}
                self.ack(tag);
            }
            cmd::GET_PLAYBACK_LATENCY => {
                let ch = r.u32()?;
                let (sec, usec) = r.timeval()?;
                let p = self.playback.get(&ch).ok_or(Fail::Code(err::NOENTITY))?;
                let latency = be.sinks().get(p.sink_index as usize).map(|d| d.latency_us).unwrap_or(0);
                let read = p.written.saturating_sub((p.ring.available() * p.fmt.bytes()) as u64);
                let (write, playing, v) = (p.written as i64, p.ctl.run.load(Ordering::Relaxed), self.version);
                let (nsec, nusec) = now_timeval();
                self.reply(tag, |w| {
                    w.usec(latency).usec(0).bool(playing).timeval(sec, usec).timeval(nsec, nusec).s64(write).s64(read as i64);
                    if v >= 13 {
                        w.u64(0).u64(0);
                    }
                });
            }
            cmd::GET_RECORD_LATENCY => {
                let ch = r.u32()?;
                let (sec, usec) = r.timeval()?;
                let s = self.record.get(&ch).ok_or(Fail::Code(err::NOENTITY))?;
                let latency = be.sources().get(s.source_index as usize).map(|d| d.latency_us).unwrap_or(0);
                let delivered = s.delivered as i64;
                let (nsec, nusec) = now_timeval();
                self.reply(tag, |w| {
                    w.usec(0).usec(latency).bool(true).timeval(sec, usec).timeval(nsec, nusec).s64(delivered).s64(delivered);
                });
            }
            cmd::SET_PLAYBACK_STREAM_NAME | cmd::SET_RECORD_STREAM_NAME => {
                let ch = r.u32()?;
                let name = r.string()?.unwrap_or_default();
                if command == cmd::SET_PLAYBACK_STREAM_NAME {
                    self.playback.get_mut(&ch).ok_or(Fail::Code(err::NOENTITY))?.name = name;
                } else {
                    self.record.get_mut(&ch).ok_or(Fail::Code(err::NOENTITY))?.name = name;
                }
                self.ack(tag);
            }
            cmd::UPDATE_PLAYBACK_STREAM_PROPLIST | cmd::UPDATE_RECORD_STREAM_PROPLIST => {
                let ch = r.u32()?;
                let _mode = r.u32()?;
                let list = r.proplist()?;
                if command == cmd::UPDATE_PLAYBACK_STREAM_PROPLIST {
                    self.playback.get_mut(&ch).ok_or(Fail::Code(err::NOENTITY))?.props.extend(list);
                } else {
                    self.record.get_mut(&ch).ok_or(Fail::Code(err::NOENTITY))?.props.extend(list);
                }
                self.ack(tag);
            }
            cmd::REMOVE_PLAYBACK_STREAM_PROPLIST | cmd::REMOVE_RECORD_STREAM_PROPLIST => self.ack(tag),
            cmd::SET_SINK_INPUT_VOLUME | cmd::SET_SINK_INPUT_MUTE => {
                let idx = r.u32()?;
                let p = self.playback.values_mut().find(|p| p.index == idx).ok_or(Fail::Code(err::NOENTITY))?;
                if command == cmd::SET_SINK_INPUT_VOLUME {
                    p.volume = r.cvolume()?;
                } else {
                    p.mute = r.bool()?;
                }
                p.apply_gain();
                self.ack(tag);
                self.event(sub::SINK_INPUT, sub::CHANGE, idx);
            }
            cmd::SET_SOURCE_OUTPUT_VOLUME | cmd::SET_SOURCE_OUTPUT_MUTE => {
                let idx = r.u32()?;
                let s = self.record.values_mut().find(|s| s.index == idx).ok_or(Fail::Code(err::NOENTITY))?;
                if command == cmd::SET_SOURCE_OUTPUT_VOLUME {
                    s.volume = r.cvolume()?;
                } else {
                    s.mute = r.bool()?;
                }
                s.ctl.set_gain(if s.mute { 0.0 } else { avg_gain(&s.volume) });
                self.ack(tag);
                self.event(sub::SOURCE_OUTPUT, sub::CHANGE, idx);
            }
            cmd::SET_PLAYBACK_STREAM_BUFFER_ATTR => {
                let ch = r.u32()?;
                let (maxlength, tlength, prebuf, minreq) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                let latency = be.sinks().first().map(|d| d.latency_us).unwrap_or(0);
                let p = self.playback.get_mut(&ch).ok_or(Fail::Code(err::NOENTITY))?;
                let cap = p.ring.capacity() * p.fmt.bytes();
                let fb = p.frame_bytes;
                let pick = |given: u32, cur: usize| if given == INVALID { cur } else { (given as usize / fb).max(1) * fb };
                p.tlength = pick(tlength, p.tlength).min(cap - cap % fb);
                p.minreq = pick(minreq, p.minreq).min(p.tlength);
                p.prebuf = if prebuf == INVALID { p.prebuf } else { (prebuf as usize / fb) * fb }.min(p.tlength);
                p.maxlength = pick(maxlength, p.maxlength).max(p.tlength);
                p.ctl.rebuffer.store(p.prebuf > 0, Ordering::Relaxed);
                let (m, t, pb, mr) = (p.maxlength as u32, p.tlength as u32, p.prebuf as u32, p.minreq as u32);
                self.reply(tag, |w| {
                    w.u32(m).u32(t).u32(pb).u32(mr).usec(latency);
                });
            }
            cmd::SET_RECORD_STREAM_BUFFER_ATTR => {
                let ch = r.u32()?;
                let (maxlength, fragsize) = (r.u32()?, r.u32()?);
                let latency = be.sources().first().map(|d| d.latency_us).unwrap_or(0);
                let s = self.record.get_mut(&ch).ok_or(Fail::Code(err::NOENTITY))?;
                let fb = s.frame_bytes;
                if fragsize != INVALID {
                    s.fragsize = (fragsize as usize / fb).max(1) * fb;
                }
                if maxlength != INVALID {
                    s.maxlength = (maxlength as usize / fb).max(1) * fb;
                }
                s.maxlength = s.maxlength.max(s.fragsize);
                let (m, f) = (s.maxlength as u32, s.fragsize as u32);
                self.reply(tag, |w| {
                    w.u32(m).u32(f).usec(latency);
                });
            }
            cmd::UPDATE_PLAYBACK_STREAM_SAMPLE_RATE | cmd::UPDATE_RECORD_STREAM_SAMPLE_RATE => {
                let ch = r.u32()?;
                let rate = r.u32()?;
                if rate == 0 || rate > 384_000 {
                    return Err(Fail::Code(err::INVALID));
                }
                if command == cmd::UPDATE_PLAYBACK_STREAM_SAMPLE_RATE {
                    let p = self.playback.get_mut(&ch).ok_or(Fail::Code(err::NOENTITY))?;
                    p.rate = rate;
                    p.ctl.rate.store(rate, Ordering::Relaxed);
                } else {
                    let s = self.record.get_mut(&ch).ok_or(Fail::Code(err::NOENTITY))?;
                    s.rate = rate;
                    let dev = be.sources().get(s.source_index as usize).map(|d| d.rate).unwrap_or(48000);
                    s.resampler.set_rates(dev, rate);
                }
                self.ack(tag);
            }
            cmd::STARTED => self.ack(tag),
            _ => return Err(Fail::Code(err::NOTIMPLEMENTED)),
        }
        Ok(())
    }

    fn write_sink_input(&self, w: &mut Writer, p: &Playback) {
        w.u32(p.index).string(Some(&p.name)).u32(INVALID).u32(self.client_index).u32(p.sink_index);
        w.sample_spec(p.fmt.to_wire(), p.channels as u8, p.rate).channel_map(&default_channel_map(p.channels as u8));
        w.cvolume(&p.volume).usec(0).usec(0).string(None).string(Some(DRIVER)).bool(p.mute);
        w.proplist(&p.props).bool(p.corked).bool(true).bool(true).format_info(1);
    }

    fn write_source_output(&self, w: &mut Writer, s: &Record) {
        w.u32(s.index).string(Some(&s.name)).u32(INVALID).u32(self.client_index).u32(s.source_index);
        w.sample_spec(s.fmt.to_wire(), s.channels as u8, s.rate).channel_map(&default_channel_map(s.channels as u8));
        w.usec(0).usec(0).string(None).string(Some(DRIVER));
        w.proplist(&s.props).bool(s.corked).cvolume(&s.volume).bool(s.mute).bool(true).bool(true).format_info(1);
    }

    fn alloc_stream_index(&mut self) -> u32 {
        self.next_stream += 1;
        self.client_index * 4096 + self.next_stream
    }

    fn create_playback(&mut self, be: &mut dyn Backend, tag: u32, r: &mut Reader) -> Res {
        let (fmt_n, chs, rate) = r.sample_spec()?;
        let _map = r.channel_map()?;
        let sink_idx = r.u32()?;
        let sink_name = r.string()?;
        let (maxlength, corked, tlength, prebuf, minreq) = (r.u32()?, r.bool()?, r.u32()?, r.u32()?, r.u32()?);
        let _sync = r.u32()?;
        let cvol = r.cvolume()?;
        let v = self.version;
        if v >= 12 {
            for _ in 0..7 {
                r.bool()?;
            }
        }
        let (mut muted, mut props, mut volume_set, mut muted_set) = (false, Vec::new(), true, false);
        if v >= 13 {
            muted = r.bool()?;
            let _adjust = r.bool()?;
            props = r.proplist()?;
        }
        if v >= 14 {
            volume_set = r.bool()?;
            let _early = r.bool()?;
        }
        if v >= 15 {
            muted_set = r.bool()?;
            r.bool()?;
            r.bool()?;
        }
        let fmt = SampleFormat::from_wire(fmt_n).ok_or(Fail::Code(err::NOTSUPPORTED))?;
        if chs == 0 || chs as usize > MAX_CHANNELS || rate == 0 || rate > 384_000 {
            return Err(Fail::Code(err::INVALID));
        }
        let (sink_index, dev) = self.resolve(be, true, sink_idx, sink_name)?;
        let channels = chs as usize;
        let frame_bytes = channels * fmt.bytes();
        let (maxlength, tlength, prebuf, minreq) = playback_attrs(rate, frame_bytes, maxlength, tlength, prebuf, minreq);
        let ring = Arc::new(Ring::new((tlength / frame_bytes + minreq / frame_bytes) * channels));
        let ctl = Arc::new(StreamCtl::new(rate));
        ctl.rebuffer.store(prebuf > 0, Ordering::Relaxed);
        let volume = if volume_set && cvol.len() == channels { cvol } else { vec![VOLUME_NORM; channels] };
        let params = StreamParams { channels, rate, cycle_frames: (minreq / frame_bytes) as u32 };
        let handle = be.open_playback(&dev.name, &params, ring.clone(), ctl.clone()).map_err(Fail::Code)?;
        let channel = self.next_channel;
        self.next_channel += 1;
        let index = self.alloc_stream_index();
        let outstanding = (tlength / frame_bytes).min(ring.space() / channels) * frame_bytes;
        let p = Playback {
            index,
            name: props.iter().find(|(k, _)| k == "media.name").map(|(_, v)| String::from_utf8_lossy(v).trim_end_matches('\0').to_string()).unwrap_or_default(),
            fmt,
            channels,
            rate,
            frame_bytes,
            sink: dev.name.clone(),
            sink_index,
            ring,
            ctl,
            _handle: handle,
            maxlength,
            tlength,
            prebuf,
            minreq,
            outstanding,
            written: 0,
            corked,
            prebuffered: prebuf == 0,
            volume,
            mute: muted_set && muted,
            drain_tag: None,
            last_xruns: 0,
            had_data: false,
            underflow_sent: false,
            props,
            scratch: Vec::new(),
        };
        p.apply_gain();
        p.ctl.run.store(!p.corked && p.prebuffered, Ordering::Relaxed);
        let sample_spec = (fmt.to_wire(), chs, rate);
        self.reply(tag, |w| {
            w.u32(channel).u32(index).u32(outstanding as u32);
            w.u32(maxlength as u32).u32(tlength as u32).u32(prebuf as u32).u32(minreq as u32);
            w.sample_spec(sample_spec.0, sample_spec.1, sample_spec.2).channel_map(&default_channel_map(chs));
            w.u32(sink_index).string(Some(&dev.name)).bool(false);
            if v >= 13 {
                w.usec(dev.latency_us);
            }
            if v >= 21 {
                w.format_info(1);
            }
        });
        self.playback.insert(channel, p);
        self.event(sub::SINK_INPUT, sub::NEW, index);
        Ok(())
    }

    fn create_record(&mut self, be: &mut dyn Backend, tag: u32, r: &mut Reader) -> Res {
        let (fmt_n, chs, rate) = r.sample_spec()?;
        let _map = r.channel_map()?;
        let src_idx = r.u32()?;
        let src_name = r.string()?;
        let (maxlength, corked, fragsize) = (r.u32()?, r.bool()?, r.u32()?);
        let v = self.version;
        if v >= 12 {
            for _ in 0..7 {
                r.bool()?;
            }
        }
        let mut props = Vec::new();
        if v >= 13 {
            let _peak = r.bool()?;
            let _adjust = r.bool()?;
            props = r.proplist()?;
            let _direct_on_input = r.u32()?;
        }
        let fmt = SampleFormat::from_wire(fmt_n).ok_or(Fail::Code(err::NOTSUPPORTED))?;
        if chs == 0 || chs as usize > MAX_CHANNELS || rate == 0 || rate > 384_000 {
            return Err(Fail::Code(err::INVALID));
        }
        let (source_index, dev) = self.resolve(be, false, src_idx, src_name)?;
        let channels = chs as usize;
        let frame_bytes = channels * fmt.bytes();
        let whole = |b: usize| (b / frame_bytes).max(1) * frame_bytes;
        let fragsize = whole(if fragsize == INVALID { (rate / 50) as usize * frame_bytes } else { fragsize as usize });
        let maxlength = whole(if maxlength == INVALID { fragsize * 4 } else { maxlength as usize }).max(fragsize);
        let ring = Arc::new(Ring::new(dev.rate as usize / 2 * channels));
        let ctl = Arc::new(StreamCtl::new(rate));
        let params = StreamParams { channels, rate: dev.rate, cycle_frames: (fragsize / frame_bytes) as u32 * dev.rate / rate };
        let handle = be.open_capture(&dev.name, &params, ring.clone(), ctl.clone()).map_err(Fail::Code)?;
        let channel = self.next_channel;
        self.next_channel += 1;
        let index = self.alloc_stream_index();
        let s = Record {
            index,
            name: props.iter().find(|(k, _)| k == "media.name").map(|(_, v)| String::from_utf8_lossy(v).trim_end_matches('\0').to_string()).unwrap_or_default(),
            fmt,
            channels,
            rate,
            frame_bytes,
            source: dev.name.clone(),
            source_index,
            ring,
            ctl,
            _handle: handle,
            resampler: Resampler::new(channels, dev.rate, rate),
            maxlength,
            fragsize,
            corked,
            delivered: 0,
            last_xruns: 0,
            volume: vec![VOLUME_NORM; channels],
            mute: false,
            props,
            scratch: Vec::new(),
        };
        self.reply(tag, |w| {
            w.u32(channel).u32(index).u32(maxlength as u32).u32(fragsize as u32);
            w.sample_spec(fmt_n, chs, rate).channel_map(&default_channel_map(chs));
            w.u32(source_index).string(Some(&dev.name)).bool(false);
            if v >= 13 {
                w.usec(dev.latency_us);
            }
            if v >= 22 {
                w.format_info(1);
            }
        });
        let _ = &s.source;
        self.record.insert(channel, s);
        self.event(sub::SOURCE_OUTPUT, sub::NEW, index);
        Ok(())
    }

    /// Stream data from the client for `channel`.
    fn on_data(&mut self, channel: u32, payload: &[u8]) {
        let Some(p) = self.playback.get_mut(&channel) else { return };
        p.scratch.clear();
        p.fmt.decode(payload, &mut p.scratch);
        let whole = p.scratch.len() / p.channels * p.channels;
        let wrote = p.ring.write(&p.scratch[..whole]);
        let bytes = whole * p.fmt.bytes();
        p.outstanding = p.outstanding.saturating_sub(bytes);
        p.written += (wrote * p.fmt.bytes()) as u64;
        p.had_data = true;
        p.underflow_sent = false;
        if wrote < whole {
            self.notify(cmd::OVERFLOW, |_| {});
        }
    }

    /// Periodic work: prebuffer and underrun bookkeeping, flow control requests, drains, capture delivery.
    pub fn tick(&mut self) -> Vec<Vec<u8>> {
        let channels: Vec<u32> = self.playback.keys().copied().collect();
        for ch in channels {
            let mut notices: Vec<(u32, u32)> = Vec::new();
            let mut drained = None;
            {
                let p = self.playback.get_mut(&ch).unwrap();
                let x = p.ctl.xruns.load(Ordering::Relaxed);
                if x != p.last_xruns {
                    p.last_xruns = x;
                    if p.prebuf > 0 && p.drain_tag.is_none() {
                        p.prebuffered = false;
                    }
                    if p.had_data && !p.underflow_sent && p.drain_tag.is_none() {
                        p.underflow_sent = true;
                        notices.push((cmd::UNDERFLOW, ch));
                    }
                }
                if !p.prebuffered && p.avail_frames() * p.frame_bytes >= p.prebuf {
                    p.prebuffered = true;
                }
                p.ctl.run.store(!p.corked && p.prebuffered, Ordering::Relaxed);
                if let Some(t) = p.drain_tag {
                    if p.ring.available() == 0 {
                        p.drain_tag = None;
                        drained = Some(t);
                    }
                }
                let have = p.avail_frames() * p.frame_bytes + p.outstanding;
                let space = p.ring.space() / p.channels * p.frame_bytes;
                let want = p.tlength.saturating_sub(have).min(space.saturating_sub(p.outstanding));
                if want >= p.minreq.max(p.frame_bytes) {
                    p.outstanding += want;
                    notices.push((cmd::REQUEST, want as u32));
                }
            }
            for (command, arg) in notices {
                if command == cmd::UNDERFLOW {
                    let offset = self.playback.get(&ch).map(|p| p.written as i64).unwrap_or(0);
                    self.notify(cmd::UNDERFLOW, |w| {
                        w.u32(ch);
                        w.s64(offset);
                    });
                } else {
                    self.notify(cmd::REQUEST, |w| {
                        w.u32(ch).u32(arg);
                    });
                }
            }
            if let Some(t) = drained {
                self.ack(t);
            }
        }
        let channels: Vec<u32> = self.record.keys().copied().collect();
        for ch in channels {
            let mut frames_out: Vec<Vec<u8>> = Vec::new();
            let mut overflow = false;
            {
                let s = self.record.get_mut(&ch).unwrap();
                let x = s.ctl.xruns.load(Ordering::Relaxed);
                if x != s.last_xruns {
                    s.last_xruns = x;
                    overflow = true;
                }
                if s.corked {
                    // keep the ring from filling while corked
                    let mut sink = [0.0f32; 256];
                    while s.ring.read(&mut sink) > 0 {}
                } else {
                    let want = s.fragsize / s.frame_bytes;
                    s.scratch.resize(want * s.channels, 0.0);
                    loop {
                        let got = s.resampler.pull(&s.ring, &mut s.scratch[..want * s.channels]);
                        if got == 0 {
                            break;
                        }
                        let mut bytes = Vec::with_capacity(got * s.frame_bytes);
                        s.fmt.encode(&s.scratch[..got * s.channels], &mut bytes);
                        s.delivered += bytes.len() as u64;
                        frames_out.push(data_frame(ch, &bytes));
                        if got < want {
                            break;
                        }
                    }
                }
            }
            self.out.extend(frames_out);
            if overflow {
                self.notify(cmd::OVERFLOW, |w| {
                    w.u32(ch);
                });
            }
        }
        self.take_out()
    }
}

fn cstr(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is a valid writable buffer of the length passed; gethostname NUL-terminates within it.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len() - 1) };
    if rc != 0 {
        return "bifrost".to_string();
    }
    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;

    fn frame_payload(f: &[u8]) -> &[u8] {
        &f[20..]
    }

    /// Sends a command and returns the reply frames' payload readers' bytes.
    fn send(s: &mut Session, be: &mut MockBackend, command: u32, tag: u32, build: impl FnOnce(&mut Writer)) -> Vec<Vec<u8>> {
        let mut w = Writer::new();
        w.u32(command).u32(tag);
        build(&mut w);
        let d = Descriptor { length: w.buf.len() as u32, channel: COMMAND_CHANNEL, offset: 0, flags: 0 };
        s.handle(be, &d, &w.buf).iter().map(|f| frame_payload(f).to_vec()).collect()
    }

    fn auth(s: &mut Session, be: &mut MockBackend) {
        let r = send(s, be, cmd::AUTH, 1, |w| {
            w.u32(0x8000_0000 | 35).arbitrary(&[0; 256]);
        });
        let mut rd = Reader::new(&r[0]);
        assert_eq!((rd.u32(), rd.u32(), rd.u32()), (Ok(cmd::REPLY), Ok(1), Ok(35)));
    }

    fn setup() -> (Session, MockBackend) {
        let mut s = Session::new(1);
        let mut be = MockBackend::standard();
        auth(&mut s, &mut be);
        (s, be)
    }

    fn error_code(reply: &[u8]) -> u32 {
        let mut rd = Reader::new(reply);
        assert_eq!(rd.u32(), Ok(cmd::ERROR));
        rd.u32().unwrap();
        rd.u32().unwrap()
    }

    /// Creates a stereo S16LE 48 kHz playback stream with the given byte lengths.
    fn create_pb(s: &mut Session, be: &mut MockBackend, tlength: u32, prebuf: u32, minreq: u32, corked: bool) -> Vec<u8> {
        let r = send(s, be, cmd::CREATE_PLAYBACK_STREAM, 7, |w| {
            w.sample_spec(3, 2, 48000).channel_map(&[1, 2]).u32(INVALID).string(None).u32(INVALID).bool(corked);
            w.u32(tlength).u32(prebuf).u32(minreq).u32(0).cvolume(&[VOLUME_NORM, VOLUME_NORM]);
            for _ in 0..7 {
                w.bool(false);
            }
            w.bool(false).bool(false).proplist(&[("media.name".to_string(), b"song\0".to_vec())]);
            w.bool(false).bool(false);
            w.bool(false).bool(false).bool(false);
        });
        assert_eq!(r.len(), 1, "{:?}", r);
        r.into_iter().next().unwrap()
    }

    fn data(s: &mut Session, be: &mut MockBackend, channel: u32, bytes: &[u8]) {
        let d = Descriptor { length: bytes.len() as u32, channel, offset: 0, flags: 0 };
        s.handle(be, &d, bytes);
    }

    #[test]
    fn commands_before_auth_are_refused() {
        let mut s = Session::new(1);
        let mut be = MockBackend::standard();
        let r = send(&mut s, &mut be, cmd::GET_SERVER_INFO, 3, |_| {});
        assert_eq!(error_code(&r[0]), err::ACCESS);
    }

    #[test]
    fn old_protocol_versions_are_refused() {
        let mut s = Session::new(1);
        let mut be = MockBackend::standard();
        let r = send(&mut s, &mut be, cmd::AUTH, 1, |w| {
            w.u32(12).arbitrary(&[0; 256]);
        });
        assert_eq!(error_code(&r[0]), err::VERSION);
    }

    #[test]
    fn server_info_names_the_default_devices() {
        let (mut s, mut be) = setup();
        let r = send(&mut s, &mut be, cmd::GET_SERVER_INFO, 2, |_| {});
        let mut rd = Reader::new(&r[0]);
        rd.u32().unwrap();
        rd.u32().unwrap();
        assert_eq!(rd.string().unwrap().as_deref(), Some(SERVER_NAME));
        rd.string().unwrap();
        rd.string().unwrap();
        rd.string().unwrap();
        assert_eq!(rd.sample_spec(), Ok((5, 2, 48000)));
        assert_eq!(rd.string().unwrap().as_deref(), Some("out0"));
        assert_eq!(rd.string().unwrap().as_deref(), Some("in0"));
    }

    #[test]
    fn sink_list_has_one_entry_and_lookup_works() {
        let (mut s, mut be) = setup();
        let r = send(&mut s, &mut be, cmd::GET_SINK_INFO_LIST, 2, |_| {});
        let mut rd = Reader::new(&r[0]);
        rd.u32().unwrap();
        rd.u32().unwrap();
        assert_eq!(rd.u32(), Ok(0));
        assert_eq!(rd.string().unwrap().as_deref(), Some("out0"));
        let r = send(&mut s, &mut be, cmd::LOOKUP_SINK, 3, |w| {
            w.string(Some("out0"));
        });
        let mut rd = Reader::new(&r[0]);
        rd.u32().unwrap();
        rd.u32().unwrap();
        assert_eq!(rd.u32(), Ok(0));
        let r = send(&mut s, &mut be, cmd::LOOKUP_SINK, 4, |w| {
            w.string(Some("nope"));
        });
        assert_eq!(error_code(&r[0]), err::NOENTITY);
    }

    #[test]
    fn unknown_and_malformed_commands_get_errors() {
        let (mut s, mut be) = setup();
        let r = send(&mut s, &mut be, 999, 5, |_| {});
        assert_eq!(error_code(&r[0]), err::NOTIMPLEMENTED);
        let r = send(&mut s, &mut be, cmd::SUBSCRIBE, 6, |_| {});
        assert_eq!(error_code(&r[0]), err::PROTOCOL);
    }

    #[test]
    fn buffer_attrs_default_and_round() {
        // 4-byte frames at 48 kHz: default tlength = 4800 frames, minreq a quarter, prebuf = tlength, max = 4x
        assert_eq!(playback_attrs(48000, 4, INVALID, INVALID, INVALID, INVALID), (76800, 19200, 19200, 4800));
        // 400 bytes (100 frames), prebuf 0, minreq 40; a ragged value rounds down to whole frames
        assert_eq!(playback_attrs(48000, 4, INVALID, 402, 0, 41), (1600, 400, 0, 40));
    }

    #[test]
    fn create_playback_reply_and_hal_client() {
        let (mut s, mut be) = setup();
        let r = create_pb(&mut s, &mut be, 400, 0, 40, false);
        let mut rd = Reader::new(&r);
        assert_eq!((rd.u32(), rd.u32()), (Ok(cmd::REPLY), Ok(7)));
        assert_eq!(rd.u32(), Ok(0)); // channel
        assert_eq!(rd.u32(), Ok(4096 + 1)); // stream index: client 1
        assert_eq!(rd.u32(), Ok(400)); // missing
        assert_eq!((rd.u32(), rd.u32(), rd.u32(), rd.u32()), (Ok(1600), Ok(400), Ok(0), Ok(40)));
        assert_eq!(rd.sample_spec(), Ok((3, 2, 48000)));
        assert_eq!(be.playback.len(), 1);
        assert_eq!(be.playback[0].0, "out0");
        // prebuf 0 and not corked: playing at once
        assert!(be.playback[0].2.run.load(Ordering::Relaxed));
        assert!(s.has_streams());
    }

    #[test]
    fn data_fills_the_ring_and_consuming_triggers_a_request() {
        let (mut s, mut be) = setup();
        create_pb(&mut s, &mut be, 400, 0, 40, false);
        let ring = be.playback[0].1.clone();
        // 10 frames of S16LE 0x4000 (0.5) on both channels
        let bytes: Vec<u8> = (0..20).flat_map(|_| [0x00u8, 0x40]).collect();
        data(&mut s, &mut be, 0, &bytes);
        assert_eq!(ring.available(), 20);
        let mut got = [0.0f32; 20];
        assert_eq!(ring.read(&mut got), 20);
        assert!(got.iter().all(|&x| x == 0.5));
        // 100 frames wanted, 90 still outstanding with the client: 10 frames = 40 bytes = minreq
        let frames = s.tick();
        assert_eq!(frames.len(), 1);
        let mut rd = Reader::new(frame_payload(&frames[0]));
        assert_eq!((rd.u32(), rd.u32(), rd.u32(), rd.u32()), (Ok(cmd::REQUEST), Ok(COMMAND_CHANNEL), Ok(0), Ok(40)));
        // nothing more until more is consumed
        assert!(s.tick().is_empty());
    }

    #[test]
    fn prebuffer_holds_playback_until_filled() {
        let (mut s, mut be) = setup();
        create_pb(&mut s, &mut be, 400, 80, 40, false);
        let ctl = be.playback[0].2.clone();
        assert!(!ctl.run.load(Ordering::Relaxed));
        let ten_frames: Vec<u8> = vec![0; 40];
        data(&mut s, &mut be, 0, &ten_frames);
        s.tick();
        assert!(!ctl.run.load(Ordering::Relaxed));
        data(&mut s, &mut be, 0, &ten_frames);
        s.tick();
        assert!(ctl.run.load(Ordering::Relaxed));
        // cork stops it
        send(&mut s, &mut be, cmd::CORK_PLAYBACK_STREAM, 9, |w| {
            w.u32(0).bool(true);
        });
        s.tick();
        assert!(!ctl.run.load(Ordering::Relaxed));
    }

    #[test]
    fn trigger_starts_a_short_prebuffer() {
        let (mut s, mut be) = setup();
        create_pb(&mut s, &mut be, 400, 400, 40, false);
        let ctl = be.playback[0].2.clone();
        data(&mut s, &mut be, 0, &[0; 40]);
        s.tick();
        assert!(!ctl.run.load(Ordering::Relaxed));
        send(&mut s, &mut be, cmd::TRIGGER_PLAYBACK_STREAM, 9, |w| {
            w.u32(0);
        });
        s.tick();
        assert!(ctl.run.load(Ordering::Relaxed));
    }

    #[test]
    fn underrun_sends_one_notice_and_rebuffers() {
        let (mut s, mut be) = setup();
        create_pb(&mut s, &mut be, 400, 80, 40, false);
        let ctl = be.playback[0].2.clone();
        data(&mut s, &mut be, 0, &[0; 80]);
        s.tick();
        assert!(ctl.run.load(Ordering::Relaxed));
        // the IO procedure ran dry
        ctl.xruns.fetch_add(1, Ordering::Relaxed);
        ctl.run.store(false, Ordering::Relaxed);
        be.playback[0].1.read(&mut [0.0; 64]);
        let frames = s.tick();
        let underflows = frames.iter().filter(|f| Reader::new(frame_payload(f)).u32() == Ok(cmd::UNDERFLOW)).count();
        assert_eq!(underflows, 1);
        assert!(!ctl.run.load(Ordering::Relaxed));
        ctl.xruns.fetch_add(1, Ordering::Relaxed);
        let frames = s.tick();
        assert!(frames.iter().all(|f| Reader::new(frame_payload(f)).u32() != Ok(cmd::UNDERFLOW)));
    }

    #[test]
    fn drain_replies_when_the_ring_is_empty() {
        let (mut s, mut be) = setup();
        create_pb(&mut s, &mut be, 400, 400, 40, false);
        data(&mut s, &mut be, 0, &[0; 40]);
        let r = send(&mut s, &mut be, cmd::DRAIN_PLAYBACK_STREAM, 11, |w| {
            w.u32(0);
        });
        assert!(r.is_empty());
        let frames = s.tick();
        assert!(frames.iter().all(|f| Reader::new(frame_payload(f)).u32() != Ok(cmd::REPLY)));
        // the drain forced playback; the IO procedure consumes everything
        assert!(be.playback[0].2.run.load(Ordering::Relaxed));
        be.playback[0].1.read(&mut [0.0; 64]);
        let frames = s.tick();
        let mut rd = Reader::new(frame_payload(frames.iter().find(|f| Reader::new(frame_payload(f)).u32() == Ok(cmd::REPLY)).unwrap()));
        assert_eq!((rd.u32(), rd.u32()), (Ok(cmd::REPLY), Ok(11)));
    }

    #[test]
    fn flush_drops_buffered_audio() {
        let (mut s, mut be) = setup();
        create_pb(&mut s, &mut be, 400, 0, 40, false);
        data(&mut s, &mut be, 0, &[0; 40]);
        assert_eq!(be.playback[0].1.available(), 20);
        send(&mut s, &mut be, cmd::FLUSH_PLAYBACK_STREAM, 12, |w| {
            w.u32(0);
        });
        assert_eq!(be.playback[0].1.available(), 0);
    }

    #[test]
    fn stream_volume_is_cubic_and_mute_silences() {
        let (mut s, mut be) = setup();
        create_pb(&mut s, &mut be, 400, 0, 40, false);
        let ctl = be.playback[0].2.clone();
        assert_eq!(ctl.gain(), 1.0);
        let r = send(&mut s, &mut be, cmd::SET_SINK_INPUT_VOLUME, 13, |w| {
            w.u32(4097).cvolume(&[VOLUME_NORM / 2, VOLUME_NORM / 2]);
        });
        assert_eq!(r.len(), 1);
        assert!((ctl.gain() - 0.125).abs() < 1e-6);
        send(&mut s, &mut be, cmd::SET_SINK_INPUT_MUTE, 14, |w| {
            w.u32(4097).bool(true);
        });
        assert_eq!(ctl.gain(), 0.0);
        let r = send(&mut s, &mut be, cmd::SET_SINK_INPUT_MUTE, 15, |w| {
            w.u32(9999).bool(true);
        });
        assert_eq!(error_code(&r[0]), err::NOENTITY);
    }

    #[test]
    fn sink_volume_goes_to_the_device() {
        let (mut s, mut be) = setup();
        send(&mut s, &mut be, cmd::SET_SINK_VOLUME, 2, |w| {
            w.u32(INVALID).string(Some("out0")).cvolume(&[VOLUME_NORM, VOLUME_NORM]);
        });
        assert_eq!(be.volumes, vec![(true, "out0".to_string(), 1.0, false)]);
        send(&mut s, &mut be, cmd::SET_SINK_MUTE, 3, |w| {
            w.u32(0).string(None).bool(true);
        });
        assert_eq!(be.volumes[1], (true, "out0".to_string(), 1.0, true));
    }

    #[test]
    fn subscribe_reports_new_and_removed_streams() {
        let (mut s, mut be) = setup();
        send(&mut s, &mut be, cmd::SUBSCRIBE, 2, |w| {
            w.u32(0x3FF);
        });
        let d = Descriptor { length: 0, channel: COMMAND_CHANNEL, offset: 0, flags: 0 };
        let mut w = Writer::new();
        w.u32(cmd::CREATE_PLAYBACK_STREAM).u32(8);
        w.sample_spec(3, 2, 48000).channel_map(&[1, 2]).u32(INVALID).string(None).u32(INVALID).bool(false);
        w.u32(400).u32(0).u32(40).u32(0).cvolume(&[VOLUME_NORM; 2]);
        for _ in 0..7 {
            w.bool(false);
        }
        w.bool(false).bool(false).proplist(&[]);
        w.bool(false).bool(false).bool(false).bool(false).bool(false);
        let frames = s.handle(&mut be, &d, &w.buf);
        assert_eq!(frames.len(), 2);
        let mut rd = Reader::new(frame_payload(&frames[1]));
        assert_eq!((rd.u32(), rd.u32(), rd.u32(), rd.u32()), (Ok(cmd::SUBSCRIBE_EVENT), Ok(COMMAND_CHANNEL), Ok(sub::SINK_INPUT | sub::NEW), Ok(4097)));
        let r = send(&mut s, &mut be, cmd::DELETE_PLAYBACK_STREAM, 9, |w| {
            w.u32(0);
        });
        assert_eq!(r.len(), 2);
        let mut rd = Reader::new(&r[1]);
        assert_eq!((rd.u32(), rd.u32(), rd.u32(), rd.u32()), (Ok(cmd::SUBSCRIBE_EVENT), Ok(COMMAND_CHANNEL), Ok(sub::SINK_INPUT | sub::REMOVE), Ok(4097)));
        assert!(!s.has_streams());
    }

    #[test]
    fn playback_latency_reports_write_and_read_indexes() {
        let (mut s, mut be) = setup();
        create_pb(&mut s, &mut be, 400, 400, 40, false);
        data(&mut s, &mut be, 0, &[0; 40]);
        let r = send(&mut s, &mut be, cmd::GET_PLAYBACK_LATENCY, 20, |w| {
            w.u32(0).timeval(5, 6);
        });
        let mut rd = Reader::new(&r[0]);
        rd.u32().unwrap();
        rd.u32().unwrap();
        assert_eq!(rd.usec(), Ok(10_000));
        assert_eq!(rd.usec(), Ok(0));
        assert_eq!(rd.bool(), Ok(false));
        assert_eq!(rd.timeval(), Ok((5, 6)));
        rd.timeval().unwrap();
        assert_eq!(rd.s64(), Ok(40));
        assert_eq!(rd.s64(), Ok(0));
    }

    #[test]
    fn record_stream_delivers_captured_audio_in_the_client_format() {
        let (mut s, mut be) = setup();
        let r = send(&mut s, &mut be, cmd::CREATE_RECORD_STREAM, 30, |w| {
            w.sample_spec(3, 2, 48000).channel_map(&[1, 2]).u32(INVALID).string(None).u32(INVALID).bool(false).u32(16);
            for _ in 0..7 {
                w.bool(false);
            }
            w.bool(false).bool(false).proplist(&[]).u32(INVALID);
            w.bool(false).bool(false).bool(false);
        });
        let mut rd = Reader::new(&r[0]);
        assert_eq!((rd.u32(), rd.u32(), rd.u32(), rd.u32(), rd.u32()), (Ok(cmd::REPLY), Ok(30), Ok(0), Ok(4097), Ok(64)));
        assert_eq!(rd.u32(), Ok(16)); // fragsize
        let ring = be.capture[0].1.clone();
        ring.write(&[0.5, -0.5, 0.5, -0.5, 0.5, -0.5, 0.5, -0.5]);
        let frames = s.tick();
        assert_eq!(frames.len(), 1);
        let d = Descriptor::parse(frames[0][..20].try_into().unwrap());
        assert_eq!((d.channel, d.length), (0, 16));
        assert_eq!(&frames[0][20..24], &[0x00, 0x40, 0x00, 0xC0]);
        assert!(s.tick().is_empty());
    }

    #[test]
    fn unknown_sink_fails_stream_creation() {
        let (mut s, mut be) = setup();
        let r = send(&mut s, &mut be, cmd::CREATE_PLAYBACK_STREAM, 7, |w| {
            w.sample_spec(3, 2, 48000).channel_map(&[1, 2]).u32(INVALID).string(Some("ghost")).u32(INVALID).bool(false);
            w.u32(400).u32(0).u32(40).u32(0).cvolume(&[VOLUME_NORM; 2]);
            for _ in 0..7 {
                w.bool(false);
            }
            w.bool(false).bool(false).proplist(&[]);
            w.bool(false).bool(false);
            w.bool(false).bool(false).bool(false);
        });
        assert_eq!(error_code(&r[0]), err::NOENTITY);
        assert!(!s.has_streams());
    }
}
