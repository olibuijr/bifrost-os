//! The process-local JACK graph: clients, ports, connections, the cycle and the transport. One `Engine` serves all
//! clients opened in a process; a single HAL IO procedure (see `driver.rs`) runs one `cycle` per device period, which
//! mixes the capture channels into `system:capture_N`, runs the active clients' process callbacks in dependency
//! order and hands `system:playback_N` to the device. (Clients in other processes join through the server graph:
//! docs/audio/18-jack.md, "What is left".)
//!
//! Real-time rules: `cycle` allocates nothing, takes no lock and makes no system call. The driver calls it under
//! `try_lock` of the engine mutex; a cycle that loses the race outputs silence.

use bifrost_audio_types::BufferList;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::*};
use std::sync::{Arc, Mutex};

use crate::midi;
use crate::pattern;

pub const MAX_FRAMES: usize = 8192;
pub const BUF_BYTES: usize = MAX_FRAMES * 4;
pub const PORT_NAME_SIZE: usize = 256;
pub const CLIENT_NAME_SIZE: usize = 64;
pub const AUDIO_TYPE: &str = "32 bit float mono audio";
pub const MIDI_TYPE: &str = "8 bit raw midi";

pub const PORT_IS_INPUT: u32 = 0x1;
pub const PORT_IS_OUTPUT: u32 = 0x2;
pub const PORT_IS_PHYSICAL: u32 = 0x4;
pub const PORT_IS_TERMINAL: u32 = 0x10;

pub const STATUS_FAILURE: u32 = 0x01;
pub const STATUS_NAME_NOT_UNIQUE: u32 = 0x04;
pub const STATUS_SERVER_FAILED: u32 = 0x10;

pub const TRANSPORT_STOPPED: u32 = 0;
pub const TRANSPORT_ROLLING: u32 = 1;
pub const TRANSPORT_STARTING: u32 = 3;

pub const EEXIST: c_int = 17;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortType {
    Audio,
    Midi,
}

impl PortType {
    pub fn name(self) -> &'static str {
        match self {
            PortType::Audio => AUDIO_TYPE,
            PortType::Midi => MIDI_TYPE,
        }
    }
}

/// `jack_position_t` (the public header's layout; size and offsets are pinned in the tests).
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Position {
    pub unique_1: u64,
    pub usecs: u64,
    pub frame_rate: u32,
    pub frame: u32,
    pub valid: u32,
    pub bar: i32,
    pub beat: i32,
    pub tick: i32,
    pub bar_start_tick: f64,
    pub beats_per_bar: f32,
    pub beat_type: f32,
    pub ticks_per_beat: f64,
    pub beats_per_minute: f64,
    pub frame_time: f64,
    pub next_time: f64,
    pub bbt_offset: u32,
    pub audio_frames_per_video_frame: f32,
    pub video_offset: u32,
    pub tick_double: f64,
    pub padding: [i32; 5],
    pub unique_2: u64,
}

pub const POSITION_BBT: u32 = 0x10;

pub type ProcessFn = extern "C" fn(u32, *mut c_void) -> c_int;
pub type ThreadInitFn = extern "C" fn(*mut c_void);
pub type GraphOrderFn = extern "C" fn(*mut c_void) -> c_int;
pub type XrunFn = extern "C" fn(*mut c_void) -> c_int;
pub type BufferSizeFn = extern "C" fn(u32, *mut c_void) -> c_int;
pub type SampleRateFn = extern "C" fn(u32, *mut c_void) -> c_int;
pub type PortRegFn = extern "C" fn(u32, c_int, *mut c_void);
pub type ClientRegFn = extern "C" fn(*const c_char, c_int, *mut c_void);
pub type PortConnectFn = extern "C" fn(u32, u32, c_int, *mut c_void);
pub type PortRenameFn = extern "C" fn(u32, *const c_char, *const c_char, *mut c_void);
pub type FreewheelFn = extern "C" fn(c_int, *mut c_void);
pub type ShutdownFn = extern "C" fn(*mut c_void);
pub type InfoShutdownFn = extern "C" fn(c_int, *const c_char, *mut c_void);
pub type LatencyFn = extern "C" fn(c_int, *mut c_void);
pub type SyncFn = extern "C" fn(u32, *mut Position, *mut c_void) -> c_int;
pub type TimebaseFn = extern "C" fn(u32, u32, *mut Position, c_int, *mut c_void);

/// A callback with its argument (the pointer is stored as an integer so the struct is `Send`).
#[derive(Clone, Copy)]
pub struct Cb<F: Copy>(pub F, pub usize);

/// The callbacks a client registered. All `Copy`, so they can be taken out of the engine and called unlocked.
#[derive(Clone, Copy, Default)]
pub struct Callbacks {
    pub process: Option<Cb<ProcessFn>>,
    pub thread_init: Option<Cb<ThreadInitFn>>,
    pub graph_order: Option<Cb<GraphOrderFn>>,
    pub xrun: Option<Cb<XrunFn>>,
    pub buffer_size: Option<Cb<BufferSizeFn>>,
    pub sample_rate: Option<Cb<SampleRateFn>>,
    pub port_reg: Option<Cb<PortRegFn>>,
    pub client_reg: Option<Cb<ClientRegFn>>,
    pub port_connect: Option<Cb<PortConnectFn>>,
    pub port_rename: Option<Cb<PortRenameFn>>,
    pub freewheel: Option<Cb<FreewheelFn>>,
    pub shutdown: Option<Cb<ShutdownFn>>,
    pub info_shutdown: Option<Cb<InfoShutdownFn>>,
    pub latency: Option<Cb<LatencyFn>>,
    pub sync: Option<Cb<SyncFn>>,
    pub timebase: Option<Cb<TimebaseFn>>,
}

/// A deferred notification: built under the engine lock, run after it is released (a callback may call back in).
pub type Call = Box<dyn FnOnce() + Send>;

/// The part of a port the client API reads from the process thread without the engine lock: immutable metadata plus
/// name and peer lists kept under their own small mutexes (never held across a callback or a cycle).
pub struct PortMeta {
    pub client: u32,
    pub flags: u32,
    pub ptype: PortType,
    full: Mutex<CString>,
    /// Names replaced by a rename stay allocated so pointers already handed to clients remain valid.
    retired: Mutex<Vec<CString>>,
    peers: Mutex<Vec<Arc<PortMeta>>>,
}

impl PortMeta {
    fn new(client: u32, flags: u32, ptype: PortType, full: CString) -> Arc<PortMeta> {
        Arc::new(PortMeta { client, flags, ptype, full: Mutex::new(full), retired: Mutex::new(vec![]), peers: Mutex::new(vec![]) })
    }

    fn set_full(&self, new: CString) {
        let old = std::mem::replace(&mut *self.full.lock().unwrap_or_else(|p| p.into_inner()), new);
        self.retired.lock().unwrap_or_else(|p| p.into_inner()).push(old);
    }

    /// Pointer to the NUL-terminated full name; valid for the life of the process.
    pub fn full_ptr(&self) -> *const c_char {
        self.full.lock().unwrap_or_else(|p| p.into_inner()).as_ptr()
    }

    /// Pointer to the short name (the part after the first colon) of the current full name.
    pub fn short_ptr(&self) -> *const c_char {
        let g = self.full.lock().unwrap_or_else(|p| p.into_inner());
        let b = g.as_bytes_with_nul();
        let colon = b.iter().position(|&c| c == b':').map_or(0, |i| i + 1);
        b[colon..].as_ptr() as *const c_char
    }

    pub fn full_name(&self) -> CString {
        self.full.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn connection_count(&self) -> usize {
        self.peers.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    pub fn connected_to(&self, name: &str) -> bool {
        self.peers.lock().unwrap_or_else(|p| p.into_inner()).iter().any(|q| q.full.lock().unwrap_or_else(|p| p.into_inner()).to_str() == Ok(name))
    }

    pub fn peer_names(&self) -> Vec<CString> {
        self.peers.lock().unwrap_or_else(|p| p.into_inner()).iter().map(|q| q.full_name()).collect()
    }

    fn link(a: &Arc<PortMeta>, b: &Arc<PortMeta>) {
        a.peers.lock().unwrap_or_else(|p| p.into_inner()).push(b.clone());
        b.peers.lock().unwrap_or_else(|p| p.into_inner()).push(a.clone());
    }

    fn unlink(a: &Arc<PortMeta>, b: &Arc<PortMeta>) {
        a.peers.lock().unwrap_or_else(|p| p.into_inner()).retain(|q| !Arc::ptr_eq(q, b));
        b.peers.lock().unwrap_or_else(|p| p.into_inner()).retain(|q| !Arc::ptr_eq(q, a));
    }
}

pub struct Port {
    pub meta: Arc<PortMeta>,
    pub id: u32,
    pub client: u32,
    pub short: String,
    pub full: CString,
    pub ptype: PortType,
    pub flags: u32,
    buf: *mut f32,
    /// Output port: its destinations. Input port: its sources.
    pub conns: Vec<u32>,
    /// [capture, playback] (min, max) latency in frames.
    pub latency: [(u32, u32); 2],
    pub aliases: Vec<CString>,
}

// SAFETY: `buf` is an allocation owned by the port; the cycle (under the engine lock) is its only writer.
unsafe impl Send for Port {}

impl Port {
    fn new(id: u32, client: u32, cname: &str, short: &str, ptype: PortType, flags: u32) -> Port {
        let buf = Box::into_raw(vec![0f32; MAX_FRAMES].into_boxed_slice()) as *mut f32;
        let full = CString::new(format!("{cname}:{short}")).unwrap_or_default();
        let meta = PortMeta::new(client, flags, ptype, full.clone());
        let mut p = Port { meta, id, client, short: short.to_string(), full, ptype, flags, buf, conns: vec![], latency: [(0, 0); 2], aliases: vec![] };
        if ptype == PortType::Midi {
            midi::clear(p.bytes_mut());
        }
        p
    }

    /// The port buffer as returned by `jack_port_get_buffer`.
    pub fn buf_ptr(&self) -> *mut f32 {
        self.buf
    }

    fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: the allocation is MAX_FRAMES f32 = BUF_BYTES bytes, owned by this port.
        unsafe { std::slice::from_raw_parts_mut(self.buf as *mut u8, BUF_BYTES) }
    }
}

impl Drop for Port {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` as a boxed slice of MAX_FRAMES f32.
        unsafe { drop(Box::from_raw(std::slice::from_raw_parts_mut(self.buf, MAX_FRAMES))) };
    }
}

pub struct Client {
    pub name: String,
    pub active: bool,
    pub kicked: bool,
    pub cb: Callbacks,
    pub ports: Vec<u32>,
    sync_ready: bool,
}

/// State read without the engine lock: the clock, the transport and the latest position.
pub struct Shared {
    pub sample_rate: AtomicU32,
    pub buffer_size: AtomicU32,
    /// Frame counter at the start of the current cycle (wraps like `jack_nframes_t`).
    pub last_frames: AtomicU32,
    pub cycle_usecs: AtomicU64,
    pub next_usecs: AtomicU64,
    pub period_usecs: AtomicU64,
    /// Load of the last cycle in 1/10000 of the period.
    pub load: AtomicU32,
    pub rt_thread: AtomicU64,
    pub state: AtomicU32,
    pub frame: AtomicU32,
    pub freewheel: AtomicBool,
    pub pos: Mutex<Position>,
}

impl Shared {
    pub fn new(rate: u32, frames: u32) -> Shared {
        Shared {
            sample_rate: AtomicU32::new(rate),
            buffer_size: AtomicU32::new(frames),
            last_frames: AtomicU32::new(0),
            cycle_usecs: AtomicU64::new(0),
            next_usecs: AtomicU64::new(0),
            period_usecs: AtomicU64::new(frames as u64 * 1_000_000 / rate.max(1) as u64),
            load: AtomicU32::new(0),
            rt_thread: AtomicU64::new(0),
            state: AtomicU32::new(TRANSPORT_STOPPED),
            frame: AtomicU32::new(0),
            freewheel: AtomicBool::new(false),
            pos: Mutex::new(Position { frame_rate: rate, ..Position::default() }),
        }
    }
}

pub struct Transport {
    locate_pending: bool,
    timeout_usecs: u64,
    starting_since: u64,
    timebase: Option<u32>,
    new_pos: bool,
}

pub struct Engine {
    pub sh: Arc<Shared>,
    clients: Vec<Option<Client>>,
    ports: Vec<Option<Box<Port>>>,
    by_name: HashMap<String, u32>,
    order: Vec<u32>,
    sys_capture: Vec<u32>,
    sys_playback: Vec<u32>,
    t: Transport,
    pub xruns: u32,
    pub pending_buffer_size: Option<u32>,
    cycles: u64,
    frames_done: u32,
}

fn valid_short_name(s: &str) -> bool {
    !s.is_empty() && !s.contains(':') && s.len() + CLIENT_NAME_SIZE < PORT_NAME_SIZE
}

impl Engine {
    /// An engine with the `system` client and `capture`/`playback` physical ports.
    pub fn new(sh: Arc<Shared>, capture: usize, playback: usize) -> Engine {
        let mut e = Engine {
            sh,
            clients: vec![Some(Client { name: "system".into(), active: true, kicked: false, cb: Callbacks::default(), ports: vec![], sync_ready: true })],
            ports: vec![],
            by_name: HashMap::new(),
            order: vec![],
            sys_capture: vec![],
            sys_playback: vec![],
            t: Transport { locate_pending: false, timeout_usecs: 2_000_000, starting_since: 0, timebase: None, new_pos: false },
            xruns: 0,
            pending_buffer_size: None,
            cycles: 0,
            frames_done: 0,
        };
        for i in 1..=capture {
            let id = e.add_port(0, &format!("capture_{i}"), PortType::Audio, PORT_IS_OUTPUT | PORT_IS_PHYSICAL | PORT_IS_TERMINAL).unwrap();
            e.sys_capture.push(id);
        }
        for i in 1..=playback {
            let id = e.add_port(0, &format!("playback_{i}"), PortType::Audio, PORT_IS_INPUT | PORT_IS_PHYSICAL | PORT_IS_TERMINAL).unwrap();
            e.sys_playback.push(id);
        }
        e
    }

    fn add_port(&mut self, client: u32, short: &str, ptype: PortType, flags: u32) -> Result<u32, c_int> {
        let cname = self.clients.get(client as usize).and_then(|c| c.as_ref()).map(|c| c.name.clone()).ok_or(-1)?;
        let full = format!("{cname}:{short}");
        if !valid_short_name(short) || (flags & PORT_IS_INPUT != 0) == (flags & PORT_IS_OUTPUT != 0) || self.by_name.contains_key(&full) {
            return Err(-1);
        }
        let id = self.ports.len() as u32;
        self.ports.push(Some(Box::new(Port::new(id, client, &cname, short, ptype, flags))));
        self.by_name.insert(full, id);
        self.clients[client as usize].as_mut().unwrap().ports.push(id);
        Ok(id)
    }

    pub fn port(&self, id: u32) -> Option<&Port> {
        self.ports.get(id as usize).and_then(|p| p.as_deref())
    }

    fn port_mut(&mut self, id: u32) -> Option<&mut Port> {
        self.ports.get_mut(id as usize).and_then(|p| p.as_deref_mut())
    }

    pub fn client(&self, id: u32) -> Option<&Client> {
        self.clients.get(id as usize).and_then(|c| c.as_ref())
    }

    pub fn client_mut(&mut self, id: u32) -> Option<&mut Client> {
        self.clients.get_mut(id as usize).and_then(|c| c.as_mut())
    }

    pub fn port_by_name(&self, name: &str) -> Option<u32> {
        self.by_name.get(name).copied()
    }

    pub fn client_names(&self) -> Vec<String> {
        self.clients.iter().flatten().map(|c| c.name.clone()).collect()
    }

    /// Opens a client. A taken name gets `-01`, `-02`, ... unless `exact`, which fails instead. Returns the id, the
    /// final name and the status bits.
    pub fn open_client(&mut self, name: &str, exact: bool) -> Result<(u32, String, u32, Vec<Call>), u32> {
        let clean: String = name.chars().filter(|c| *c != ':').take(CLIENT_NAME_SIZE - 1).collect();
        if clean.is_empty() {
            return Err(STATUS_FAILURE);
        }
        let taken = |e: &Engine, n: &str| e.clients.iter().flatten().any(|c| c.name == n);
        let mut final_name = clean.clone();
        let mut status = 0;
        if taken(self, &final_name) {
            if exact {
                return Err(STATUS_FAILURE | STATUS_NAME_NOT_UNIQUE);
            }
            status |= STATUS_NAME_NOT_UNIQUE;
            let mut n = 1;
            loop {
                final_name = format!("{clean}-{n:02}");
                if !taken(self, &final_name) {
                    break;
                }
                n += 1;
            }
        }
        let slot = self.clients.iter().skip(1).position(|c| c.is_none()).map(|i| i + 1);
        let c = Client { name: final_name.clone(), active: false, kicked: false, cb: Callbacks::default(), ports: vec![], sync_ready: true };
        let id = match slot {
            Some(i) => {
                self.clients[i] = Some(c);
                i as u32
            }
            None => {
                self.clients.push(Some(c));
                (self.clients.len() - 1) as u32
            }
        };
        let calls = self.client_reg_calls(&final_name, true, id);
        Ok((id, final_name, status, calls))
    }

    fn client_reg_calls(&self, name: &str, reg: bool, except: u32) -> Vec<Call> {
        let mut calls: Vec<Call> = vec![];
        for (i, c) in self.clients.iter().enumerate() {
            if let Some(Client { active: true, cb: Callbacks { client_reg: Some(Cb(f, a)), .. }, .. }) = c {
                if i as u32 != except {
                    let (f, a, n) = (*f, *a, CString::new(name).unwrap_or_default());
                    calls.push(Box::new(move || f(n.as_ptr(), reg as c_int, a as *mut c_void)));
                }
            }
        }
        calls
    }

    pub fn close_client(&mut self, id: u32) -> Vec<Call> {
        let Some(c) = self.client(id) else { return vec![] };
        let (name, ports) = (c.name.clone(), c.ports.clone());
        let mut calls = vec![];
        for p in ports {
            calls.extend(self.unregister_port(id, p));
        }
        if self.t.timebase == Some(id) {
            self.t.timebase = None;
        }
        self.clients[id as usize] = None;
        self.recompute_order();
        calls.extend(self.client_reg_calls(&name, false, id));
        calls.extend(self.graph_order_calls());
        calls
    }

    pub fn activate(&mut self, id: u32) -> Vec<Call> {
        let Some(c) = self.client_mut(id) else { return vec![] };
        c.active = true;
        c.kicked = false;
        c.sync_ready = c.cb.sync.is_none();
        if c.cb.sync.is_some() {
            self.t.locate_pending = true;
        }
        self.recompute_order();
        self.graph_order_calls()
    }

    pub fn deactivate(&mut self, id: u32) -> Vec<Call> {
        let Some(c) = self.client_mut(id) else { return vec![] };
        c.active = false;
        if self.t.timebase == Some(id) {
            self.t.timebase = None;
        }
        self.recompute_order();
        self.graph_order_calls()
    }

    fn graph_order_calls(&self) -> Vec<Call> {
        let mut calls: Vec<Call> = vec![];
        for c in self.clients.iter().flatten() {
            if let (true, Some(Cb(f, a))) = (c.active, c.cb.graph_order) {
                calls.push(Box::new(move || {
                    f(a as *mut c_void);
                }));
            }
        }
        calls
    }

    fn port_reg_calls(&self, port: u32, reg: bool) -> Vec<Call> {
        let mut calls: Vec<Call> = vec![];
        for c in self.clients.iter().flatten() {
            if let (true, Some(Cb(f, a))) = (c.active, c.cb.port_reg) {
                calls.push(Box::new(move || f(port, reg as c_int, a as *mut c_void)));
            }
        }
        calls
    }

    fn connect_calls(&self, a: u32, b: u32, connected: bool) -> Vec<Call> {
        let mut calls: Vec<Call> = vec![];
        for c in self.clients.iter().flatten() {
            if let (true, Some(Cb(f, arg))) = (c.active, c.cb.port_connect) {
                calls.push(Box::new(move || f(a, b, connected as c_int, arg as *mut c_void)));
            }
        }
        calls
    }

    pub fn register_port(&mut self, client: u32, short: &str, ptype: PortType, flags: u32) -> Result<(u32, Vec<Call>), c_int> {
        let id = self.add_port(client, short, ptype, flags)?;
        Ok((id, self.port_reg_calls(id, true)))
    }

    pub fn unregister_port(&mut self, client: u32, port: u32) -> Vec<Call> {
        let Some(p) = self.port(port) else { return vec![] };
        if p.client != client {
            return vec![];
        }
        let mut calls = vec![];
        for other in p.conns.clone() {
            calls.extend(self.disconnect_ids(port, other));
        }
        calls.extend(self.port_reg_calls(port, false));
        let p = self.ports[port as usize].take().unwrap();
        self.by_name.remove(p.full.to_str().unwrap_or(""));
        if let Some(c) = self.client_mut(client) {
            c.ports.retain(|&x| x != port);
        }
        calls
    }

    pub fn rename_port(&mut self, port: u32, short: &str) -> Result<Vec<Call>, c_int> {
        let p = self.port(port).ok_or(-1)?;
        let cname = self.client(p.client).map(|c| c.name.clone()).ok_or(-1)?;
        let (old, new) = (p.full.to_str().unwrap_or("").to_string(), format!("{cname}:{short}"));
        if !valid_short_name(short) || self.by_name.contains_key(&new) {
            return Err(-1);
        }
        self.by_name.remove(&old);
        self.by_name.insert(new.clone(), port);
        let p = self.port_mut(port).unwrap();
        p.short = short.to_string();
        p.full = CString::new(new.clone()).unwrap_or_default();
        p.meta.set_full(p.full.clone());
        let mut calls: Vec<Call> = vec![];
        for c in self.clients.iter().flatten() {
            if let (true, Some(Cb(f, a))) = (c.active, c.cb.port_rename) {
                let (o, n) = (CString::new(old.clone()).unwrap_or_default(), CString::new(new.clone()).unwrap_or_default());
                calls.push(Box::new(move || f(port, o.as_ptr(), n.as_ptr(), a as *mut c_void)));
            }
        }
        Ok(calls)
    }

    /// Connects an output port to an input port of the same type. 0 on success, `EEXIST` if already connected, -1
    /// for anything else.
    pub fn connect(&mut self, src: &str, dst: &str) -> (c_int, Vec<Call>) {
        let (Some(s), Some(d)) = (self.port_by_name(src), self.port_by_name(dst)) else { return (-1, vec![]) };
        let (ps, pd) = (self.port(s).unwrap(), self.port(d).unwrap());
        if ps.flags & PORT_IS_OUTPUT == 0 || pd.flags & PORT_IS_INPUT == 0 || ps.ptype != pd.ptype {
            return (-1, vec![]);
        }
        if ps.conns.contains(&d) {
            return (EEXIST, vec![]);
        }
        self.port_mut(s).unwrap().conns.push(d);
        self.port_mut(d).unwrap().conns.push(s);
        let (ms, md) = (self.port(s).unwrap().meta.clone(), self.port(d).unwrap().meta.clone());
        PortMeta::link(&ms, &md);
        self.recompute_order();
        let mut calls = self.connect_calls(s, d, true);
        calls.extend(self.graph_order_calls());
        (0, calls)
    }

    pub fn disconnect(&mut self, src: &str, dst: &str) -> (c_int, Vec<Call>) {
        let (Some(s), Some(d)) = (self.port_by_name(src), self.port_by_name(dst)) else { return (-1, vec![]) };
        if !self.port(s).unwrap().conns.contains(&d) {
            return (-1, vec![]);
        }
        (0, self.disconnect_ids(s, d))
    }

    fn disconnect_ids(&mut self, a: u32, b: u32) -> Vec<Call> {
        if let (Some(pa), Some(pb)) = (self.port(a), self.port(b)) {
            PortMeta::unlink(&pa.meta.clone(), &pb.meta.clone());
        }
        if let Some(p) = self.port_mut(a) {
            p.conns.retain(|&x| x != b);
        }
        if let Some(p) = self.port_mut(b) {
            p.conns.retain(|&x| x != a);
        }
        self.recompute_order();
        let (out, inp) = if self.port(a).map_or(false, |p| p.flags & PORT_IS_OUTPUT != 0) { (a, b) } else { (b, a) };
        let mut calls = self.connect_calls(out, inp, false);
        calls.extend(self.graph_order_calls());
        calls
    }

    pub fn disconnect_all(&mut self, port: u32) -> Vec<Call> {
        let mut calls = vec![];
        let conns = self.port(port).map(|p| p.conns.clone()).unwrap_or_default();
        for o in conns {
            calls.extend(self.disconnect_ids(port, o));
        }
        calls
    }

    /// Full names of the ports matching the patterns and carrying all `flags`.
    pub fn get_ports(&self, name_pat: &str, type_pat: &str, flags: u32) -> Vec<CString> {
        self.ports
            .iter()
            .flatten()
            .filter(|p| p.flags & flags == flags)
            .filter(|p| pattern::matches(name_pat, p.full.to_str().unwrap_or("")))
            .filter(|p| pattern::matches(type_pat, p.ptype.name()))
            .map(|p| p.full.clone())
            .collect()
    }

    /// Client execution order: a client runs after every client that feeds one of its input ports.
    fn recompute_order(&mut self) {
        let ids: Vec<u32> = self.clients.iter().enumerate().skip(1).filter_map(|(i, c)| c.as_ref().filter(|c| c.active).map(|_| i as u32)).collect();
        let mut indeg: HashMap<u32, usize> = ids.iter().map(|&i| (i, 0)).collect();
        let mut edges: Vec<(u32, u32)> = vec![];
        for &i in &ids {
            for &p in &self.client(i).unwrap().ports {
                let port = self.port(p).unwrap();
                if port.flags & PORT_IS_OUTPUT == 0 {
                    continue;
                }
                for &d in &port.conns {
                    let dc = self.port(d).unwrap().client;
                    if dc != i && dc != 0 && indeg.contains_key(&dc) && !edges.contains(&(i, dc)) {
                        edges.push((i, dc));
                    }
                }
            }
        }
        for &(_, b) in &edges {
            *indeg.get_mut(&b).unwrap() += 1;
        }
        let mut order = vec![];
        let mut left = ids.clone();
        while !left.is_empty() {
            // The first client with nothing left to wait for; on a cycle, the first one that remains.
            let pick = left.iter().position(|i| indeg[i] == 0).unwrap_or(0);
            let id = left.remove(pick);
            order.push(id);
            for &(a, b) in &edges {
                if a == id {
                    if let Some(x) = indeg.get_mut(&b) {
                        *x = x.saturating_sub(1);
                    }
                }
            }
        }
        self.order = order;
    }

    pub fn order(&self) -> &[u32] {
        &self.order
    }

    // ----- latency ---------------------------------------------------------------------------------------------

    pub fn set_latency_range(&mut self, port: u32, playback: bool, range: (u32, u32)) {
        if let Some(p) = self.port_mut(port) {
            p.latency[playback as usize] = range;
        }
    }

    pub fn latency_range(&self, port: u32, playback: bool) -> (u32, u32) {
        self.port(port).map_or((0, 0), |p| p.latency[playback as usize])
    }

    /// The summed (max) latency from this port along its connections: outputs follow towards the device, inputs
    /// follow back to the sources. Each step moves away from where it came from: playback walks only to connected
    /// inputs, capture only to connected outputs. Depth is bounded so a feedback loop terminates.
    pub fn total_latency(&self, port: u32) -> u32 {
        fn walk(e: &Engine, id: u32, playback: bool, depth: u32) -> u32 {
            let Some(p) = e.port(id) else { return 0 };
            let own = p.latency[playback as usize].1;
            if depth == 0 {
                return own;
            }
            let want = if playback { PORT_IS_INPUT } else { PORT_IS_OUTPUT };
            own + p
                .conns
                .iter()
                .filter(|&&c| e.port(c).map_or(false, |q| q.flags & want != 0))
                .map(|&c| walk(e, c, playback, depth - 1))
                .max()
                .unwrap_or(0)
        }
        let playback = self.port(port).map_or(true, |p| p.flags & PORT_IS_OUTPUT != 0);
        walk(self, port, playback, 16)
    }

    /// Runs the latency callbacks of the active clients (capture mode 0, then playback mode 1).
    pub fn recompute_latencies(&self) -> Vec<Call> {
        let mut calls: Vec<Call> = vec![];
        for mode in [0, 1] {
            for c in self.clients.iter().flatten() {
                if let (true, Some(Cb(f, a))) = (c.active, c.cb.latency) {
                    calls.push(Box::new(move || f(mode, a as *mut c_void)));
                }
            }
        }
        calls
    }

    // ----- transport -------------------------------------------------------------------------------------------

    pub fn transport_start(&mut self) {
        if self.sh.state.load(Acquire) == TRANSPORT_ROLLING {
            return;
        }
        let any_sync = self.clients.iter().flatten().any(|c| c.active && c.cb.sync.is_some());
        for c in self.clients.iter_mut().flatten() {
            c.sync_ready = c.cb.sync.is_none();
        }
        self.sh.state.store(if any_sync { TRANSPORT_STARTING } else { TRANSPORT_ROLLING }, Release);
        self.t.starting_since = 0;
        self.t.new_pos = true;
    }

    pub fn transport_stop(&mut self) {
        self.sh.state.store(TRANSPORT_STOPPED, Release);
    }

    pub fn transport_locate(&mut self, frame: u32) {
        self.sh.frame.store(frame, Release);
        self.t.locate_pending = true;
        self.t.new_pos = true;
    }

    pub fn set_sync_timeout(&mut self, usecs: u64) {
        self.t.timeout_usecs = usecs;
    }

    /// Becomes the timebase master. With `conditional`, fails (returns EBUSY) when another client holds it.
    pub fn set_timebase(&mut self, client: u32, conditional: bool) -> c_int {
        if conditional && self.t.timebase.map_or(false, |m| m != client) {
            return libc::EBUSY;
        }
        self.t.timebase = Some(client);
        self.t.new_pos = true;
        0
    }

    pub fn release_timebase(&mut self, client: u32) -> c_int {
        if self.t.timebase != Some(client) {
            return libc::EINVAL;
        }
        self.t.timebase = None;
        if let Ok(mut p) = self.sh.pos.lock() {
            p.valid &= !POSITION_BBT;
        }
        0
    }

    pub fn timebase_master(&self) -> Option<u32> {
        self.t.timebase
    }

    pub fn set_sync_callback(&mut self, client: u32) {
        if let Some(c) = self.client_mut(client) {
            c.sync_ready = c.cb.sync.is_none();
        }
        self.t.locate_pending = true;
    }

    fn call_sync(&mut self, state: u32, pos: &mut Position) {
        for i in 1..self.clients.len() {
            let Some(c) = self.clients[i].as_mut() else { continue };
            if !c.active || c.kicked || c.sync_ready {
                continue;
            }
            if let Some(Cb(f, a)) = c.cb.sync {
                if f(state, pos, a as *mut c_void) != 0 {
                    c.sync_ready = true;
                }
            }
        }
    }

    fn transport_pre(&mut self, frames: u32, usecs: u64) {
        let sh = self.sh.clone();
        let mut state = sh.state.load(Acquire);
        let rate = sh.sample_rate.load(Relaxed);
        let mut pos = match sh.pos.lock() {
            Ok(p) => *p,
            Err(_) => return,
        };
        pos.frame = sh.frame.load(Acquire);
        pos.frame_rate = rate;
        pos.usecs = usecs;
        let new_pos = self.t.new_pos || self.t.locate_pending;
        if self.t.locate_pending {
            self.t.locate_pending = false;
            let any_sync = self.clients.iter().flatten().any(|c| c.active && c.cb.sync.is_some());
            if state == TRANSPORT_ROLLING && any_sync {
                state = TRANSPORT_STARTING;
            }
            for c in self.clients.iter_mut().flatten() {
                c.sync_ready = c.cb.sync.is_none();
            }
            let st = if state == TRANSPORT_STARTING { TRANSPORT_STARTING } else { state };
            self.call_sync(st, &mut pos);
            self.t.starting_since = usecs;
        }
        if state == TRANSPORT_STARTING {
            if self.t.starting_since == 0 {
                self.t.starting_since = usecs;
            }
            self.call_sync(TRANSPORT_STARTING, &mut pos);
            let all_ready = self.clients.iter().flatten().all(|c| !c.active || c.sync_ready);
            if all_ready || usecs.saturating_sub(self.t.starting_since) >= self.t.timeout_usecs {
                state = TRANSPORT_ROLLING;
            }
        }
        sh.state.store(state, Release);
        if let Some(m) = self.t.timebase {
            if let Some(Client { active: true, cb: Callbacks { timebase: Some(Cb(f, a)), .. }, .. }) = self.client(m) {
                if state == TRANSPORT_ROLLING || new_pos {
                    f(state, frames, &mut pos, new_pos as c_int, *a as *mut c_void);
                }
            }
        }
        self.t.new_pos = false;
        pos.unique_1 = self.cycles;
        pos.unique_2 = self.cycles;
        if let Ok(mut p) = sh.pos.lock() {
            *p = pos;
        };
    }

    // ----- the cycle ---------------------------------------------------------------------------------------------

    /// Mixes the sources of input port `pid` into its buffer.
    fn mix_input(&self, pid: u32, frames: usize) {
        let p = self.port(pid).unwrap();
        let live = |s: &&u32| self.port(**s).map_or(false, |sp| self.clients[sp.client as usize].as_ref().map_or(false, |c| c.active && !c.kicked));
        match p.ptype {
            PortType::Audio => {
                // SAFETY: every buffer is MAX_FRAMES long and `frames` <= MAX_FRAMES; a port is never its own
                // source (inputs connect to outputs), and the cycle is the only accessor while the lock is held.
                unsafe {
                    let dst = std::slice::from_raw_parts_mut(p.buf, frames);
                    dst.iter_mut().for_each(|x| *x = 0.0);
                    for s in p.conns.iter().filter(live) {
                        let src = std::slice::from_raw_parts(self.port(*s).unwrap().buf, frames);
                        for (d, s) in dst.iter_mut().zip(src) {
                            *d += *s;
                        }
                    }
                }
            }
            PortType::Midi => {
                // SAFETY: as above; the byte views cover each port's own allocation.
                unsafe {
                    let dst = std::slice::from_raw_parts_mut(p.buf as *mut u8, BUF_BYTES);
                    let mut srcs: [&[u8]; 32] = [&[]; 32];
                    let mut n = 0;
                    for s in p.conns.iter().filter(live).take(32) {
                        srcs[n] = std::slice::from_raw_parts(self.port(*s).unwrap().buf as *const u8, BUF_BYTES);
                        n += 1;
                    }
                    midi::merge(dst, &srcs[..n], frames as u32);
                }
            }
        }
    }

    fn clear_output(&self, pid: u32, frames: usize) {
        let p = self.port(pid).unwrap();
        // SAFETY: as in `mix_input`.
        unsafe {
            match p.ptype {
                PortType::Audio => std::slice::from_raw_parts_mut(p.buf, frames).iter_mut().for_each(|x| *x = 0.0),
                PortType::Midi => midi::clear(std::slice::from_raw_parts_mut(p.buf as *mut u8, BUF_BYTES)),
            }
        }
    }

    /// One device period of `frames` frames starting at `offset` in the capture and playback lists. `usecs` is the
    /// host time of the period start in microseconds.
    pub fn cycle(&mut self, frames: usize, capture: &BufferList, playback: &mut BufferList, offset: usize, usecs: u64) {
        let frames = frames.min(MAX_FRAMES);
        let sh = self.sh.clone();
        let rate = sh.sample_rate.load(Relaxed).max(1) as u64;
        let period = frames as u64 * 1_000_000 / rate;
        sh.last_frames.store(self.frames_done, Release);
        sh.cycle_usecs.store(usecs, Release);
        sh.period_usecs.store(period, Release);
        sh.next_usecs.store(usecs + period, Release);
        if sh.buffer_size.load(Relaxed) as usize != frames {
            sh.buffer_size.store(frames as u32, Release);
            self.pending_buffer_size = Some(frames as u32);
        }
        self.cycles += 1;

        for (i, &pid) in self.sys_capture.iter().enumerate() {
            let p = self.port(pid).unwrap();
            // SAFETY: as in `mix_input`.
            let dst = unsafe { std::slice::from_raw_parts_mut(p.buf, frames) };
            match capture.buffers.get(i).and_then(|b| b.data.get(offset..offset + frames)) {
                Some(src) => dst.copy_from_slice(src),
                None => dst.iter_mut().for_each(|x| *x = 0.0),
            }
        }
        self.transport_pre(frames as u32, usecs);

        for k in 0..self.order.len() {
            let cid = self.order[k] as usize;
            let nports = self.clients[cid].as_ref().map_or(0, |c| c.ports.len());
            for j in 0..nports {
                let pid = self.clients[cid].as_ref().unwrap().ports[j];
                if self.port(pid).unwrap().flags & PORT_IS_INPUT != 0 {
                    self.mix_input(pid, frames);
                } else {
                    self.clear_output(pid, frames);
                }
            }
            let c = self.clients[cid].as_mut().unwrap();
            if c.kicked {
                continue;
            }
            if let Some(Cb(f, a)) = c.cb.process {
                if f(frames as u32, a as *mut c_void) != 0 {
                    c.kicked = true;
                    c.active = false;
                }
            }
        }

        for (i, &pid) in self.sys_playback.iter().enumerate() {
            self.mix_input(pid, frames);
            let p = self.port(pid).unwrap();
            if let Some(dst) = playback.buffers.get_mut(i).and_then(|b| b.data.get_mut(offset..offset + frames)) {
                // SAFETY: as in `mix_input`.
                dst.copy_from_slice(unsafe { std::slice::from_raw_parts(p.buf, frames) });
            }
        }
        if sh.state.load(Acquire) == TRANSPORT_ROLLING {
            sh.frame.fetch_add(frames as u32, AcqRel);
        }
        self.frames_done = self.frames_done.wrapping_add(frames as u32);
    }

    /// Clients whose process callback failed (kicked) and are not yet reported; the driver's notifier turns these
    /// into shutdown callbacks.
    pub fn take_kicked(&mut self) -> Vec<Call> {
        let mut calls: Vec<Call> = vec![];
        for c in self.clients.iter_mut().flatten() {
            if c.kicked && c.cb.shutdown.is_some() || c.kicked && c.cb.info_shutdown.is_some() {
                c.kicked = false;
                c.cb.process = None;
                if let Some(Cb(f, a)) = c.cb.info_shutdown {
                    let reason = CString::new("process callback failed").unwrap();
                    calls.push(Box::new(move || f(STATUS_FAILURE as c_int | 0x1000, reason.as_ptr(), a as *mut c_void)));
                } else if let Some(Cb(f, a)) = c.cb.shutdown {
                    calls.push(Box::new(move || f(a as *mut c_void)));
                }
            }
        }
        calls
    }

    /// Notifications owed to the clients after cycles: a changed buffer size and the xrun count.
    pub fn take_events(&mut self) -> Vec<Call> {
        let mut calls = self.take_kicked();
        if let Some(n) = self.pending_buffer_size.take() {
            for c in self.clients.iter().flatten() {
                if let (true, Some(Cb(f, a))) = (c.active, c.cb.buffer_size) {
                    calls.push(Box::new(move || {
                        f(n, a as *mut c_void);
                    }));
                }
            }
        }
        for _ in 0..std::mem::take(&mut self.xruns) {
            for c in self.clients.iter().flatten() {
                if let (true, Some(Cb(f, a))) = (c.active, c.cb.xrun) {
                    calls.push(Box::new(move || {
                        f(a as *mut c_void);
                    }));
                }
            }
        }
        calls
    }

    /// All active clients get a shutdown notification (the device went away).
    pub fn shutdown_calls(&mut self) -> Vec<Call> {
        let mut calls: Vec<Call> = vec![];
        for c in self.clients.iter_mut().flatten() {
            if !c.active {
                continue;
            }
            c.active = false;
            if let Some(Cb(f, a)) = c.cb.info_shutdown {
                let reason = CString::new("audio device gone").unwrap();
                calls.push(Box::new(move || f(STATUS_SERVER_FAILED as c_int, reason.as_ptr(), a as *mut c_void)));
            } else if let Some(Cb(f, a)) = c.cb.shutdown {
                calls.push(Box::new(move || f(a as *mut c_void)));
            }
        }
        self.recompute_order();
        calls
    }

    /// Freewheel mode changed: the clients' freewheel callbacks (the flag itself lives in `Shared`).
    pub fn freewheel_calls(&self, starting: bool) -> Vec<Call> {
        let mut calls: Vec<Call> = vec![];
        for c in self.clients.iter().flatten() {
            if let (true, Some(Cb(f, a))) = (c.active, c.cb.freewheel) {
                calls.push(Box::new(move || f(starting as c_int, a as *mut c_void)));
            }
        }
        calls
    }

    pub fn port_aliases_mut(&mut self, id: u32) -> Option<&mut Vec<CString>> {
        self.port_mut(id).map(|p| &mut p.aliases)
    }

    pub fn cycles(&self) -> u64 {
        self.cycles
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_types::Buffer;
    use std::sync::atomic::AtomicUsize;

    fn lists(ch: usize, frames: usize) -> (BufferList, BufferList) {
        let mk = || BufferList { buffers: (0..ch).map(|_| Buffer { channels: 1, data: vec![0.0; frames] }).collect() };
        (mk(), mk())
    }

    fn engine() -> Engine {
        Engine::new(Arc::new(Shared::new(48000, 8)), 2, 2)
    }

    static GEN_CALLS: AtomicUsize = AtomicUsize::new(0);

    extern "C" fn gen(_: u32, _: *mut c_void) -> c_int {
        GEN_CALLS.fetch_add(1, Relaxed);
        0
    }

    extern "C" fn fail(_: u32, _: *mut c_void) -> c_int {
        1
    }

    #[test]
    fn position_layout_matches_the_header() {
        // The header declares the struct packed (PRE_PACKED_STRUCTURE); size and offsets as the C compiler reports
        // them for jack/types.h: 136 bytes, frame_rate 16, bar_start_tick 40, ticks_per_beat 56, frame_time 72,
        // bbt_offset 88, video_offset 96, tick_double 100, padding 108, unique_2 128.
        use std::mem::{offset_of, size_of};
        assert_eq!(size_of::<Position>(), 136);
        assert_eq!(std::mem::align_of::<Position>(), 1);
        assert_eq!(offset_of!(Position, usecs), 8);
        assert_eq!(offset_of!(Position, frame_rate), 16);
        assert_eq!(offset_of!(Position, frame), 20);
        assert_eq!(offset_of!(Position, valid), 24);
        assert_eq!(offset_of!(Position, bar_start_tick), 40);
        assert_eq!(offset_of!(Position, ticks_per_beat), 56);
        assert_eq!(offset_of!(Position, frame_time), 72);
        assert_eq!(offset_of!(Position, bbt_offset), 88);
        assert_eq!(offset_of!(Position, video_offset), 96);
        assert_eq!(offset_of!(Position, tick_double), 100);
        assert_eq!(offset_of!(Position, padding), 108);
        assert_eq!(offset_of!(Position, unique_2), 128);
    }

    #[test]
    fn system_ports_exist() {
        let e = engine();
        let names: Vec<String> = e.get_ports("", "", 0).iter().map(|c| c.to_str().unwrap().to_string()).collect();
        assert_eq!(names, ["system:capture_1", "system:capture_2", "system:playback_1", "system:playback_2"]);
        let phys_in = e.get_ports("", "", PORT_IS_OUTPUT | PORT_IS_PHYSICAL);
        assert_eq!(phys_in.len(), 2);
    }

    #[test]
    fn client_names_are_made_unique() {
        let mut e = engine();
        let (_, n1, s1, _) = e.open_client("synth", false).unwrap();
        let (_, n2, s2, _) = e.open_client("synth", false).unwrap();
        assert_eq!((n1.as_str(), s1), ("synth", 0));
        assert_eq!((n2.as_str(), s2), ("synth-01", STATUS_NAME_NOT_UNIQUE));
        assert!(e.open_client("synth", true).is_err());
    }

    #[test]
    fn connect_rules() {
        let mut e = engine();
        let (c, ..) = e.open_client("a", false).unwrap();
        e.register_port(c, "out", PortType::Audio, PORT_IS_OUTPUT).unwrap();
        e.register_port(c, "in", PortType::Audio, PORT_IS_INPUT).unwrap();
        e.register_port(c, "midi", PortType::Midi, PORT_IS_OUTPUT).unwrap();
        assert!(e.register_port(c, "out", PortType::Audio, PORT_IS_OUTPUT).is_err()); // duplicate
        assert!(e.register_port(c, "bad:name", PortType::Audio, PORT_IS_OUTPUT).is_err());
        assert_eq!(e.connect("a:out", "system:playback_1").0, 0);
        assert_eq!(e.connect("a:out", "system:playback_1").0, EEXIST);
        assert_eq!(e.connect("a:in", "system:playback_1").0, -1); // input as source
        assert_eq!(e.connect("a:midi", "system:playback_2").0, -1); // type mismatch
        assert_eq!(e.disconnect("a:out", "system:playback_1").0, 0);
        assert_eq!(e.disconnect("a:out", "system:playback_1").0, -1);
    }

    #[test]
    fn capture_reaches_playback_through_a_client_graph() {
        // capture_1 -> a:in; a:out -> system:playback_1; with no process callback the client's outputs stay
        // silent, so playback is silence: only connected, live sources are mixed.
        let mut e = engine();
        let (c, ..) = e.open_client("a", false).unwrap();
        e.register_port(c, "in", PortType::Audio, PORT_IS_INPUT).unwrap();
        e.register_port(c, "out", PortType::Audio, PORT_IS_OUTPUT).unwrap();
        e.connect("system:capture_1", "a:in");
        e.connect("a:out", "system:playback_1");
        e.activate(c);
        let (mut cap, mut play) = lists(2, 8);
        cap.buffers[0].data = vec![1.0; 8];
        e.cycle(8, &cap, &mut play, 0, 1000);
        assert_eq!(play.buffers[0].data, vec![0.0; 8]);
        // The input port has the capture mixed in.
        let pid = e.port_by_name("a:in").unwrap();
        let buf = unsafe { std::slice::from_raw_parts(e.port(pid).unwrap().buf_ptr(), 8) };
        assert_eq!(buf, &[1.0; 8]);
    }

    #[test]
    fn direct_connection_mixes_sources() {
        // Two capture channels both connected to playback_1: 1.0 + 0.5 = 1.5. A direct system->system link is not
        // allowed (capture is an output, playback an input, so it is): this is the "monitor" path.
        let mut e = engine();
        assert_eq!(e.connect("system:capture_1", "system:playback_1").0, 0);
        assert_eq!(e.connect("system:capture_2", "system:playback_1").0, 0);
        let (mut cap, mut play) = lists(2, 8);
        cap.buffers[0].data = vec![1.0; 8];
        cap.buffers[1].data = vec![0.5; 8];
        e.cycle(8, &cap, &mut play, 0, 1000);
        assert_eq!(play.buffers[0].data, vec![1.5; 8]);
        assert_eq!(play.buffers[1].data, vec![0.0; 8]);
    }

    #[test]
    fn offset_selects_the_slice() {
        let mut e = engine();
        e.connect("system:capture_1", "system:playback_1");
        let (mut cap, mut play) = lists(2, 16);
        cap.buffers[0].data = (0..16).map(|i| i as f32).collect();
        e.cycle(8, &cap, &mut play, 8, 0);
        assert_eq!(&play.buffers[0].data[8..], &[8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0]);
        assert_eq!(&play.buffers[0].data[..8], &[0.0; 8]);
    }

    #[test]
    fn process_runs_in_dependency_order() {
        let mut e = engine();
        let (a, ..) = e.open_client("a", false).unwrap();
        let (b, ..) = e.open_client("b", false).unwrap();
        e.register_port(a, "in", PortType::Audio, PORT_IS_INPUT).unwrap();
        e.register_port(a, "out", PortType::Audio, PORT_IS_OUTPUT).unwrap();
        e.register_port(b, "in", PortType::Audio, PORT_IS_INPUT).unwrap();
        e.register_port(b, "out", PortType::Audio, PORT_IS_OUTPUT).unwrap();
        // b feeds a although a was opened first.
        e.connect("b:out", "a:in");
        e.activate(a);
        e.activate(b);
        assert_eq!(e.order(), &[b, a]);
        // Removing the link restores registration order.
        e.disconnect("b:out", "a:in");
        assert_eq!(e.order(), &[a, b]);
        // A cycle still yields every client once.
        e.connect("b:out", "a:in");
        e.connect("a:out", "b:in");
        let mut o = e.order().to_vec();
        o.sort();
        assert_eq!(o, vec![a, b]);
    }

    #[test]
    fn failing_process_kicks_the_client() {
        let mut e = engine();
        let (a, ..) = e.open_client("a", false).unwrap();
        e.client_mut(a).unwrap().cb.process = Some(Cb(fail, 0));
        e.activate(a);
        let (cap, mut play) = lists(2, 8);
        e.cycle(8, &cap, &mut play, 0, 0);
        assert!(!e.client(a).unwrap().active);
        assert!(e.client(a).unwrap().kicked);
    }

    #[test]
    fn process_callback_is_called_once_per_cycle() {
        let mut e = engine();
        let (a, ..) = e.open_client("a", false).unwrap();
        e.client_mut(a).unwrap().cb.process = Some(Cb(gen, 0));
        e.activate(a);
        let (cap, mut play) = lists(2, 8);
        let before = GEN_CALLS.load(Relaxed);
        e.cycle(8, &cap, &mut play, 0, 0);
        e.cycle(8, &cap, &mut play, 0, 166);
        assert_eq!(GEN_CALLS.load(Relaxed) - before, 2);
        assert_eq!(e.cycles(), 2);
    }

    #[test]
    fn cycle_times_follow_the_period() {
        let mut e = engine();
        let (cap, mut play) = lists(2, 8);
        e.cycle(8, &cap, &mut play, 0, 5000);
        // 8 frames at 48000 Hz = 166 us (integer microseconds).
        assert_eq!(e.sh.period_usecs.load(Relaxed), 166);
        assert_eq!(e.sh.next_usecs.load(Relaxed), 5166);
        assert_eq!(e.sh.last_frames.load(Relaxed), 0);
        e.cycle(8, &cap, &mut play, 0, 5166);
        assert_eq!(e.sh.last_frames.load(Relaxed), 8);
    }

    #[test]
    fn transport_rolls_and_counts_frames() {
        let mut e = engine();
        let (cap, mut play) = lists(2, 8);
        e.cycle(8, &cap, &mut play, 0, 0);
        assert_eq!(e.sh.frame.load(Relaxed), 0); // stopped: no advance
        e.transport_start();
        assert_eq!(e.sh.state.load(Relaxed), TRANSPORT_ROLLING);
        e.cycle(8, &cap, &mut play, 0, 166);
        e.cycle(8, &cap, &mut play, 0, 332);
        assert_eq!(e.sh.frame.load(Relaxed), 16);
        e.transport_locate(1000);
        e.cycle(8, &cap, &mut play, 0, 498);
        assert_eq!(e.sh.frame.load(Relaxed), 1008);
        e.transport_stop();
        e.cycle(8, &cap, &mut play, 0, 664);
        assert_eq!(e.sh.frame.load(Relaxed), 1008);
        let rate = e.sh.pos.lock().unwrap().frame_rate;
        assert_eq!(rate, 48000);
    }

    static SYNC_READY_AFTER: AtomicU32 = AtomicU32::new(0);
    static SYNC_CALLS: AtomicU32 = AtomicU32::new(0);
    extern "C" fn slow_sync(_: u32, _: *mut Position, _: *mut c_void) -> c_int {
        (SYNC_CALLS.fetch_add(1, Relaxed) + 1 >= SYNC_READY_AFTER.load(Relaxed)) as c_int
    }

    #[test]
    fn slow_sync_client_holds_the_transport_in_starting() {
        let mut e = engine();
        let (a, ..) = e.open_client("a", false).unwrap();
        e.client_mut(a).unwrap().cb.sync = Some(Cb(slow_sync, 0));
        e.activate(a);
        SYNC_CALLS.store(0, Relaxed);
        SYNC_READY_AFTER.store(4, Relaxed);
        let (cap, mut play) = lists(2, 8);
        e.cycle(8, &cap, &mut play, 0, 1); // activation polls once (call 1)
        e.transport_start();
        assert_eq!(e.sh.state.load(Relaxed), TRANSPORT_STARTING);
        e.cycle(8, &cap, &mut play, 0, 2); // call 2
        assert_eq!(e.sh.state.load(Relaxed), TRANSPORT_STARTING);
        e.cycle(8, &cap, &mut play, 0, 3); // call 3
        assert_eq!(e.sh.state.load(Relaxed), TRANSPORT_STARTING);
        e.cycle(8, &cap, &mut play, 0, 4); // call 4: ready
        assert_eq!(e.sh.state.load(Relaxed), TRANSPORT_ROLLING);
    }

    #[test]
    fn sync_timeout_starts_the_transport_anyway() {
        let mut e = engine();
        let (a, ..) = e.open_client("a", false).unwrap();
        e.client_mut(a).unwrap().cb.sync = Some(Cb(slow_sync, 0));
        e.activate(a);
        SYNC_CALLS.store(0, Relaxed);
        SYNC_READY_AFTER.store(u32::MAX, Relaxed);
        e.set_sync_timeout(1000);
        let (cap, mut play) = lists(2, 8);
        e.cycle(8, &cap, &mut play, 0, 10);
        e.transport_start();
        e.cycle(8, &cap, &mut play, 0, 100); // starting_since = 100
        assert_eq!(e.sh.state.load(Relaxed), TRANSPORT_STARTING);
        e.cycle(8, &cap, &mut play, 0, 1099); // 999 us waited
        assert_eq!(e.sh.state.load(Relaxed), TRANSPORT_STARTING);
        e.cycle(8, &cap, &mut play, 0, 1100); // 1000 us: timeout
        assert_eq!(e.sh.state.load(Relaxed), TRANSPORT_ROLLING);
    }

    extern "C" fn bbt(_: u32, nframes: u32, pos: *mut Position, new_pos: c_int, _: *mut c_void) {
        // 120 bpm at 48000 Hz = 24000 frames per beat.
        let p = unsafe { &mut *pos };
        p.valid |= POSITION_BBT;
        p.beats_per_minute = 120.0;
        p.beats_per_bar = 4.0;
        p.beat_type = 4.0;
        p.ticks_per_beat = 1920.0;
        let beats = p.frame / 24000;
        p.bar = (beats / 4) as i32 + 1;
        p.beat = (beats % 4) as i32 + 1;
        p.tick = ((p.frame % 24000) as f64 * 1920.0 / 24000.0) as i32;
        let _ = (nframes, new_pos);
    }

    #[test]
    fn timebase_master_fills_bbt_and_is_exclusive() {
        let mut e = engine();
        let (a, ..) = e.open_client("a", false).unwrap();
        let (b, ..) = e.open_client("b", false).unwrap();
        e.client_mut(a).unwrap().cb.timebase = Some(Cb(bbt, 0));
        e.activate(a);
        e.activate(b);
        assert_eq!(e.set_timebase(a, true), 0);
        assert_eq!(e.set_timebase(b, true), libc::EBUSY);
        assert_eq!(e.release_timebase(b), libc::EINVAL);
        e.transport_locate(24000 * 5 + 12000); // 5 beats and a half in: bar 2, beat 2, tick 960
        let (cap, mut play) = lists(2, 8);
        e.cycle(8, &cap, &mut play, 0, 7);
        let p = *e.sh.pos.lock().unwrap();
        assert_eq!((p.bar, p.beat, p.tick), (2, 2, 960));
        assert_eq!(p.valid & POSITION_BBT, POSITION_BBT);
        let bpm = p.beats_per_minute;
        assert_eq!(bpm, 120.0);
        assert_eq!(e.release_timebase(a), 0);
        assert_eq!(e.sh.pos.lock().unwrap().valid & POSITION_BBT, 0);
    }

    #[test]
    fn midi_flows_from_a_client_output_to_another_input() {
        let mut e = engine();
        let (a, ..) = e.open_client("a", false).unwrap();
        let (b, ..) = e.open_client("b", false).unwrap();
        let (src, _) = e.register_port(a, "m", PortType::Midi, PORT_IS_OUTPUT).unwrap();
        let (dst, _) = e.register_port(b, "m", PortType::Midi, PORT_IS_INPUT).unwrap();
        e.connect("a:m", "b:m");
        e.activate(a);
        e.activate(b);
        let (cap, mut play) = lists(2, 8);
        e.cycle(8, &cap, &mut play, 0, 0);
        // A real client writes into its output buffer during process; emulate it after the clear.
        let sb = unsafe { std::slice::from_raw_parts_mut(e.port(src).unwrap().buf_ptr() as *mut u8, BUF_BYTES) };
        assert!(midi::write(sb, 2, &[0x90, 60, 100], 8));
        // Next cycle clears outputs first, so a callback-less client sends nothing: the input has 0 events.
        e.cycle(8, &cap, &mut play, 0, 166);
        let db = unsafe { std::slice::from_raw_parts(e.port(dst).unwrap().buf_ptr() as *const u8, BUF_BYTES) };
        assert_eq!(midi::count(db), 0);
    }

    #[test]
    fn close_client_removes_ports_and_connections() {
        let mut e = engine();
        let (a, ..) = e.open_client("a", false).unwrap();
        e.register_port(a, "out", PortType::Audio, PORT_IS_OUTPUT).unwrap();
        e.connect("a:out", "system:playback_1");
        e.activate(a);
        e.close_client(a);
        assert!(e.port_by_name("a:out").is_none());
        let sp = e.port_by_name("system:playback_1").unwrap();
        assert!(e.port(sp).unwrap().conns.is_empty());
        assert_eq!(e.client_names(), ["system"]);
        // The slot is reused.
        let (a2, ..) = e.open_client("z", false).unwrap();
        assert_eq!(a2, a);
    }

    #[test]
    fn rename_keeps_connections() {
        let mut e = engine();
        let (a, ..) = e.open_client("a", false).unwrap();
        let (p, _) = e.register_port(a, "out", PortType::Audio, PORT_IS_OUTPUT).unwrap();
        e.connect("a:out", "system:playback_1");
        e.rename_port(p, "left").unwrap();
        assert!(e.port_by_name("a:out").is_none());
        assert_eq!(e.port_by_name("a:left"), Some(p));
        assert_eq!(e.disconnect("a:left", "system:playback_1").0, 0);
    }

    #[test]
    fn total_latency_adds_along_the_connections() {
        let mut e = engine();
        let (a, ..) = e.open_client("a", false).unwrap();
        let (p, _) = e.register_port(a, "out", PortType::Audio, PORT_IS_OUTPUT).unwrap();
        e.connect("a:out", "system:playback_1");
        let pb = e.port_by_name("system:playback_1").unwrap();
        e.set_latency_range(p, true, (10, 10));
        e.set_latency_range(pb, true, (256, 256));
        // The output's own playback latency plus the physical port behind it.
        assert_eq!(e.total_latency(p), 10 + 256 + 0);
    }

    #[test]
    fn buffer_size_change_is_noted_for_the_notifier() {
        let mut e = engine();
        let (cap, mut play) = lists(2, 16);
        e.cycle(16, &cap, &mut play, 0, 0);
        assert_eq!(e.pending_buffer_size, Some(16));
        assert_eq!(e.sh.buffer_size.load(Relaxed), 16);
    }
}
