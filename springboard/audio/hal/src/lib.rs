//! `bifrost-audio-hal`: the client library (docs/audio-architecture.md, "Client IO", "Real-time threads"): the
//! object model's get, set and listen over the control socket, IO procedures on a real-time thread that the library
//! creates and the server promotes, and the device workgroup. The SDK re-exports it as `bifrost_app::audio_hardware`.
//!
//! Owner: task 06. The public API below is the lead's contract for the clients (tasks 07, 08, 09, 13 to 18).
//!
//! Real-time rules for an [`IoProc`]: no allocation, no locks, no syscalls other than the futex. In debug builds an
//! app (or test binary) can enforce them by installing [`RtGuardAllocator`] as its global allocator: an allocation
//! inside the IOProc is counted ([`RtScope::violations`]) and reported on stderr, and aborts the process only with
//! `BIFROST_RT_STRICT=1`.

mod conn;
mod iothread;

/// Aggregate devices: the pure sub-device list, master and clock resolution, and channel mapping.
pub mod aggregate;
/// Audio processes: the pure list of processes with audio IO and their running flags.
pub mod process;

use bifrost_audio_shm::Region;
use bifrost_audio_types::control::{IoProcRequest, Reply, Request, ShmLayout, CONTROL_SOCKET, PROTOCOL_VERSION};
use bifrost_audio_types::{props, AudioError, BufferList, ObjectId, PropertyAddress, PropertyValue, TimeStamp, SYSTEM_OBJECT};
use conn::Conn;
use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub use iothread::{now_ns, RtGuardAllocator, RtScope, MAX_CHANNELS};

/// A connection to `bifrost-audiod` (one per process is enough; clone handles share it).
#[derive(Clone)]
pub struct System {
    inner: Arc<Inner>,
}

struct Inner {
    conn: Arc<Conn>,
    procs: Mutex<HashMap<u32, ProcEntry>>,
    next_listener: AtomicU64,
}

/// Why [`System::connect_detailed`] failed: the step, the socket path, the audio error and the OS error if any.
#[derive(Debug)]
pub struct ConnectError {
    pub path: String,
    /// "socket", "path", "connect" or "hello".
    pub stage: &'static str,
    pub error: AudioError,
    pub os: Option<std::io::Error>,
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}: ", self.stage, self.path)?;
        match (&self.os, self.stage, self.error) {
            (Some(e), _, _) => write!(f, "{e}"),
            (None, "hello", AudioError::Io) => write!(f, "connected, but the server sent no Welcome within {} s", conn::REPLY_TIMEOUT.as_secs()),
            (None, "hello", AudioError::NoServer) => write!(f, "connected, but the server closed the connection during the handshake"),
            (None, "hello", AudioError::Protocol) => write!(f, "handshake refused (this client speaks protocol {PROTOCOL_VERSION}; the server another)"),
            (None, _, e) => write!(f, "{e:?}"),
        }
    }
}

impl std::error::Error for ConnectError {}

/// One IO procedure's client side.
struct ProcEntry {
    region: Arc<Region>,
    layout: ShmLayout,
    input_channels: u32,
    output_channels: u32,
    frames: u32,
    /// The wake eventfd the server sent (kept open for clients that epoll on it).
    _event: Option<OwnedFd>,
    /// The IOProc while no thread owns it.
    f: Option<IoProc>,
    thread: Option<(JoinHandle<IoProc>, Arc<AtomicBool>)>,
}

/// The default-device roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Output,
    Input,
    SoundEffects,
}

impl Role {
    fn selector(self) -> bifrost_audio_types::FourCC {
        match self {
            Role::Output => props::DEFAULT_OUTPUT,
            Role::Input => props::DEFAULT_INPUT,
            Role::SoundEffects => props::DEFAULT_SOUND_EFFECTS,
        }
    }
}

type ListenerKey = (ObjectId, PropertyAddress);

/// A listener registration; dropping it unlistens.
pub struct Listener {
    conn: Arc<Conn>,
    key: ListenerKey,
    id: u64,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let last = {
            let mut map = self.conn.shared.listeners.lock().unwrap();
            let Some(v) = map.get_mut(&self.key) else { return };
            v.retain(|(id, _)| *id != self.id);
            let empty = v.is_empty();
            if empty {
                map.remove(&self.key);
            }
            empty
        };
        if last {
            let _ = self.conn.request(&Request::Unlisten { object: self.key.0, address: self.key.1 }, &[]);
        }
    }
}

impl workgroup::Promote for Conn {
    fn promote_thread(&self, tid: i32) -> Result<(), AudioError> {
        // SAFETY: pidfd_open takes our own pid and flags 0; a negative result is an error, a positive one a new fd.
        let r = unsafe { libc::syscall(libc::SYS_pidfd_open, libc::getpid(), 0) };
        if r < 0 {
            return Err(AudioError::Io);
        }
        // SAFETY: pidfd_open returned a fresh fd that nothing else owns.
        let pidfd = unsafe { OwnedFd::from_raw_fd(r as i32) };
        match self.request(&Request::PromoteThread { tid }, &[pidfd.as_raw_fd()])? {
            (Reply::Done, _) => Ok(()),
            _ => Err(AudioError::Protocol),
        }
    }
}

fn unexpected<T>() -> Result<T, AudioError> {
    Err(AudioError::Protocol)
}

impl System {
    /// Connects to the control socket and says hello as `client` (the app id).
    pub fn connect(client: &str) -> Result<System, AudioError> {
        System::connect_path(CONTROL_SOCKET, client)
    }

    /// Like [`System::connect`] with another socket path (tests, a private server).
    pub fn connect_path(path: &str, client: &str) -> Result<System, AudioError> {
        System::connect_detailed(path, client).map_err(|e| e.error)
    }

    /// Like [`System::connect_path`], but a failure says which step failed, the path and the OS error.
    pub fn connect_detailed(path: &str, client: &str) -> Result<System, ConnectError> {
        System::connect_detailed_within(path, client, None)
    }

    /// Like [`System::connect`], but gives up after `limit`: the connect does not block (a full accept queue fails at
    /// once) and the Welcome must arrive within `limit`. A server that accepts and never answers costs at most `limit`.
    /// Only workers use this (100 ms); the shell's UI thread never connects.
    pub fn connect_within(client: &str, limit: Duration) -> Result<System, AudioError> {
        System::connect_detailed_within(CONTROL_SOCKET, client, Some(limit)).map_err(|e| e.error)
    }

    fn connect_detailed_within(path: &str, client: &str, limit: Option<Duration>) -> Result<System, ConnectError> {
        let fail = |stage: &'static str, error: AudioError, os: Option<std::io::Error>| ConnectError { path: path.to_string(), stage, error, os };
        // SAFETY: plain socket(2); a negative result is an error.
        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
        if fd < 0 {
            return Err(fail("socket", AudioError::Io, Some(std::io::Error::last_os_error())));
        }
        // SAFETY: socket returned a fresh fd.
        let sock = unsafe { OwnedFd::from_raw_fd(fd) };
        // SAFETY: sockaddr_un is plain data; all-zero is valid.
        let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        addr.sun_family = libc::AF_UNIX as _;
        let bytes = path.as_bytes();
        if bytes.len() >= addr.sun_path.len() {
            return Err(fail("path", AudioError::BadValue, None));
        }
        for (d, s) in addr.sun_path.iter_mut().zip(bytes) {
            *d = *s as _;
        }
        // SAFETY: `addr` is a valid sockaddr_un and the length is its size.
        if limit.is_some() {
            // SAFETY: fcntl on a valid fd; non-blocking for the connect only, so a full accept queue fails with EAGAIN.
            unsafe { libc::fcntl(sock.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) };
        }
        let r = unsafe { libc::connect(sock.as_raw_fd(), &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_un>() as u32) };
        if r < 0 {
            return Err(fail("connect", AudioError::NoServer, Some(std::io::Error::last_os_error())));
        }
        if limit.is_some() {
            // SAFETY: fcntl on a valid fd; blocking again for the reader thread.
            unsafe { libc::fcntl(sock.as_raw_fd(), libc::F_SETFL, 0) };
        }
        System::from_socket_within(sock, client, limit.unwrap_or(conn::REPLY_TIMEOUT)).map_err(|e| fail("hello", e, None))
    }

    /// Takes an already connected SEQPACKET socket and says hello on it.
    pub fn from_socket(sock: OwnedFd, client: &str) -> Result<System, AudioError> {
        System::from_socket_within(sock, client, conn::REPLY_TIMEOUT)
    }

    fn from_socket_within(sock: OwnedFd, client: &str, limit: Duration) -> Result<System, AudioError> {
        let conn = Conn::new(sock);
        match conn.request_within(&Request::Hello { version: PROTOCOL_VERSION, client: client.to_string(), policy: false }, &[], limit)? {
            (Reply::Welcome { version, .. }, _) if version == PROTOCOL_VERSION => {}
            (Reply::Welcome { .. }, _) => return Err(AudioError::Protocol),
            _ => return unexpected(),
        }
        Ok(System { inner: Arc::new(Inner { conn, procs: Mutex::new(HashMap::new()), next_listener: AtomicU64::new(1) }) })
    }

    pub fn default_device(&self, role: Role) -> Result<Device, AudioError> {
        match self.get(SYSTEM_OBJECT, PropertyAddress::global(role.selector()))? {
            PropertyValue::Object(id) if id.0 != 0 => Ok(Device { id, system: self.clone() }),
            PropertyValue::Object(_) => Err(AudioError::UnknownObject),
            _ => Err(AudioError::BadValueType),
        }
    }

    pub fn devices(&self) -> Result<Vec<Device>, AudioError> {
        match self.inner.conn.request(&Request::Owned { object: SYSTEM_OBJECT }, &[])? {
            (Reply::Objects(ids), _) => Ok(ids.into_iter().map(|id| Device { id, system: self.clone() }).collect()),
            _ => unexpected(),
        }
    }

    pub fn get(&self, object: ObjectId, address: PropertyAddress) -> Result<PropertyValue, AudioError> {
        match self.inner.conn.request(&Request::Get { object, address }, &[])? {
            (Reply::Value(v), _) => Ok(v),
            _ => unexpected(),
        }
    }

    pub fn set(&self, object: ObjectId, address: PropertyAddress, value: PropertyValue) -> Result<(), AudioError> {
        match self.inner.conn.request(&Request::Set { object, address, value }, &[])? {
            (Reply::Done, _) => Ok(()),
            _ => unexpected(),
        }
    }

    /// Calls `f` (on the library's notification thread) when the property changes; re-read the value in `f`.
    pub fn listen(&self, object: ObjectId, address: PropertyAddress, f: Box<dyn Fn(ObjectId, PropertyAddress) + Send + Sync>) -> Result<Listener, AudioError> {
        let key = (object, address);
        let id = self.inner.next_listener.fetch_add(1, Ordering::Relaxed);
        let first = {
            let mut map = self.inner.conn.shared.listeners.lock().unwrap();
            let v = map.entry(key).or_default();
            v.push((id, Arc::from(f)));
            v.len() == 1
        };
        let l = Listener { conn: self.inner.conn.clone(), key, id };
        if first {
            // on failure `l` drops and removes the registration (its Unlisten is harmless)
            match self.inner.conn.request(&Request::Listen { object, address }, &[])? {
                (Reply::Done, _) => {}
                _ => return unexpected(),
            }
        }
        Ok(l)
    }

    /// How many deadline misses the server reported for IO procedure `id` so far.
    pub fn overloads(&self, id: IoProcId) -> u32 {
        self.inner.conn.shared.overloads.lock().unwrap().get(&id.0).copied().unwrap_or(0)
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        let procs = self.procs.get_mut().unwrap();
        for (_, e) in procs.iter_mut() {
            stop_thread(e);
        }
    }
}

/// Stops the entry's IO thread (if any) and takes the IOProc back.
fn stop_thread(e: &mut ProcEntry) {
    if let Some((handle, stop)) = e.thread.take() {
        stop.store(true, Ordering::Release);
        let act: &bifrost_audio_shm::Activation = e.region.get(e.layout.activation);
        // Bump the word too, so a thread that is between its stop check and its futex wait does not miss the wake.
        act.wake.fetch_add(1, Ordering::AcqRel);
        bifrost_audio_shm::futex::wake(&act.wake, 1);
        if let Ok(f) = handle.join() {
            e.f = Some(f);
        }
    }
}

/// A device handle.
#[derive(Clone)]
pub struct Device {
    pub id: ObjectId,
    pub system: System,
}

/// What an IO procedure gets each cycle (the reference IOProc, made safe). It runs on a real-time thread: no
/// allocation, no locks, no syscalls other than the futex (enforced in debug builds).
pub struct IoContext {
    pub cycle: u64,
    /// Frames this call covers (the client's buffer size).
    pub frames: u32,
}

/// The IO procedure: `now`, the input of this cycle with its timestamp, and the output to fill with its timestamp.
/// The input was captured one buffer before `now`; the output plays one buffer after it. Both lists hold one
/// single-channel buffer per channel (non-interleaved), `frames` long.
pub type IoProc = Box<dyn FnMut(&IoContext, &TimeStamp, &BufferList, &TimeStamp, &mut BufferList, &TimeStamp) + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IoProcId(pub u32);

/// How a client wants to run on a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoProcConfig {
    pub input_channels: u32,
    pub output_channels: u32,
    /// 0 means the device's current buffer size.
    pub buffer_frames: u32,
}

impl Device {
    /// Registers an IO procedure (not running yet). The server replies with the memfd of the rings and the
    /// activation record, which this maps.
    pub fn add_io_proc(&self, cfg: IoProcConfig, f: IoProc) -> Result<IoProcId, AudioError> {
        let (ic, oc) = (cfg.input_channels, cfg.output_channels);
        if (ic == 0 && oc == 0) || ic as usize > MAX_CHANNELS || oc as usize > MAX_CHANNELS {
            return Err(AudioError::BadValue);
        }
        let frames = if cfg.buffer_frames == 0 {
            match self.get(PropertyAddress::global(props::BUFFER_FRAMES))? {
                PropertyValue::U32(n) if n > 0 => n,
                PropertyValue::U32(_) => return Err(AudioError::BadValue),
                _ => return Err(AudioError::BadValueType),
            }
        } else {
            cfg.buffer_frames
        };
        let conn = &self.system.inner.conn;
        let req = IoProcRequest { device: self.id, input_channels: ic, output_channels: oc, buffer_frames: frames };
        let (reply, mut fds) = conn.request(&Request::AddIoProc(req), &[])?;
        let Reply::IoProcAdded { id, layout } = reply else { return unexpected() };
        if fds.is_empty() || frames > layout.ring_frames {
            // best effort: tell the server we do not want it
            let _ = conn.request(&Request::RemoveIoProc { id }, &[]);
            return Err(if fds.is_empty() { AudioError::Protocol } else { AudioError::BadValue });
        }
        let memfd = fds.remove(0);
        let event = if fds.is_empty() { None } else { Some(fds.remove(0)) };
        let region = match Region::map(memfd, layout.total as usize) {
            Ok(r) => Arc::new(r),
            Err(_) => {
                let _ = conn.request(&Request::RemoveIoProc { id }, &[]);
                return Err(AudioError::Io);
            }
        };
        let entry = ProcEntry { region, layout, input_channels: ic, output_channels: oc, frames, _event: event, f: Some(f), thread: None };
        self.system.inner.procs.lock().unwrap().insert(id, entry);
        Ok(IoProcId(id))
    }

    /// Starts the IO procedure: creates the IO thread, joins the device workgroup (the server grants SCHED_FIFO),
    /// then tells the server to include it in the cycle.
    pub fn start(&self, id: IoProcId) -> Result<(), AudioError> {
        let inner = &self.system.inner;
        let mut procs = inner.procs.lock().unwrap();
        let e = procs.get_mut(&id.0).ok_or(AudioError::UnknownObject)?;
        if e.thread.is_some() {
            return Ok(());
        }
        let f = e.f.take().ok_or(AudioError::Io)?;
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = channel();
        let args = iothread::IoArgs {
            region: e.region.clone(),
            layout: e.layout,
            input_channels: e.input_channels,
            output_channels: e.output_channels,
            frames: e.frames,
            stop: stop.clone(),
            promote: inner.conn.clone(),
            f,
            ready: ready_tx,
        };
        let handle = std::thread::Builder::new().name(format!("audio-io-{}", id.0)).spawn(move || iothread::run(args)).map_err(|_| AudioError::Io)?;
        e.thread = Some((handle, stop));
        if ready_rx.recv_timeout(Duration::from_secs(10)).is_err() {
            stop_thread(e);
            return Err(AudioError::Io);
        }
        match inner.conn.request(&Request::StartIoProc { id: id.0 }, &[]) {
            Ok((Reply::Done, _)) => Ok(()),
            Ok(_) => {
                stop_thread(e);
                unexpected()
            }
            Err(err) => {
                stop_thread(e);
                Err(err)
            }
        }
    }

    /// Stops the IO procedure: the server stops waking it, then the IO thread ends.
    pub fn stop(&self, id: IoProcId) -> Result<(), AudioError> {
        let inner = &self.system.inner;
        let mut procs = inner.procs.lock().unwrap();
        let e = procs.get_mut(&id.0).ok_or(AudioError::UnknownObject)?;
        let r = match inner.conn.request(&Request::StopIoProc { id: id.0 }, &[]) {
            Ok((Reply::Done, _)) => Ok(()),
            Ok(_) => unexpected(),
            Err(err) => Err(err),
        };
        stop_thread(e);
        r
    }

    pub fn remove_io_proc(&self, id: IoProcId) -> Result<(), AudioError> {
        let inner = &self.system.inner;
        let mut entry = inner.procs.lock().unwrap().remove(&id.0).ok_or(AudioError::UnknownObject)?;
        stop_thread(&mut entry);
        match inner.conn.request(&Request::RemoveIoProc { id: id.0 }, &[])? {
            (Reply::Done, _) => Ok(()),
            _ => unexpected(),
        }
    }

    /// Cycles the server counted as missed for this IO procedure (read from the shared activation record).
    pub fn io_proc_xruns(&self, id: IoProcId) -> Result<u32, AudioError> {
        let procs = self.system.inner.procs.lock().unwrap();
        let e = procs.get(&id.0).ok_or(AudioError::UnknownObject)?;
        let act: &bifrost_audio_shm::Activation = e.region.get(e.layout.activation);
        Ok(act.xruns.load(Ordering::Relaxed))
    }

    pub fn get(&self, address: PropertyAddress) -> Result<PropertyValue, AudioError> {
        self.system.get(self.id, address)
    }

    pub fn set(&self, address: PropertyAddress, value: PropertyValue) -> Result<(), AudioError> {
        self.system.set(self.id, address, value)
    }
}

/// The device workgroup: real-time threads that share the device's deadline (SCHED_FIFO through the server,
/// `sched_util_min` while the device runs, CPU affinity hints). Owner: task 06.
pub mod workgroup;

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_shm::{futex, RingHeader, ZeroStamp, ZeroStampValue, STATUS_FINISHED};
    use bifrost_audio_types::control::{decode_request, encode_reply, MAX_LEN};
    use std::os::fd::RawFd;

    type Out = Vec<(Reply, Vec<RawFd>)>;

    /// A scripted server on one end of a socket pair: `h` turns each request into replies (with fds).
    fn serve(sock: OwnedFd, mut h: impl FnMut(Request) -> Out + Send + 'static) -> JoinHandle<()> {
        std::thread::spawn(move || {
            let mut buf = vec![0u8; MAX_LEN];
            while let Ok(Some((n, _fds))) = conn::recv_packet(sock.as_raw_fd(), &mut buf) {
                let Ok(req) = decode_request(&buf[..n]) else { break };
                for (r, fds) in h(req) {
                    if conn::send_packet(sock.as_raw_fd(), &encode_reply(&r), &fds).is_err() {
                        return;
                    }
                }
            }
        })
    }

    fn hello_ok(r: &Request) -> Option<Out> {
        matches!(r, Request::Hello { .. }).then(|| vec![(Reply::Welcome { version: PROTOCOL_VERSION, client_id: 1 }, vec![])])
    }

    #[test]
    fn listener_fires_on_default_device_change() {
        let (a, b) = conn::socket_pair();
        let addr = PropertyAddress::global(props::DEFAULT_OUTPUT);
        let _srv = serve(b, move |r| {
            if let Some(o) = hello_ok(&r) {
                return o;
            }
            match r {
                // the default output changes right after the listener is registered
                Request::Listen { .. } => vec![(Reply::Done, vec![]), (Reply::Changed(vec![(SYSTEM_OBJECT, addr)]), vec![])],
                Request::Get { .. } => vec![(Reply::Value(PropertyValue::Object(ObjectId(42))), vec![])],
                Request::Unlisten { .. } => vec![(Reply::Done, vec![])],
                _ => vec![(Reply::Error(AudioError::Protocol), vec![])],
            }
        });
        let sys = System::from_socket(a, "test.app").unwrap();
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        let l = sys
            .listen(SYSTEM_OBJECT, addr, Box::new(move |o, a| {
                let _ = tx.lock().unwrap().send((o, a));
            }))
            .unwrap();
        let (o, got) = rx.recv_timeout(Duration::from_secs(2)).expect("the listener did not fire");
        assert_eq!(o, SYSTEM_OBJECT);
        assert_eq!(got, addr);
        // re-reading the value in the callback's place gives the new default
        let d = sys.default_device(Role::Output).unwrap();
        assert_eq!(d.id, ObjectId(42));
        drop(l);
        assert!(sys.inner.conn.shared.listeners.lock().unwrap().is_empty());
    }

    #[test]
    fn default_device_zero_is_unknown_object() {
        let (a, b) = conn::socket_pair();
        let _srv = serve(b, |r| hello_ok(&r).unwrap_or_else(|| vec![(Reply::Value(PropertyValue::Object(ObjectId(0))), vec![])]));
        let sys = System::from_socket(a, "t").unwrap();
        assert_eq!(sys.default_device(Role::Input).err(), Some(AudioError::UnknownObject));
    }

    #[test]
    fn an_error_reply_becomes_the_error() {
        let (a, b) = conn::socket_pair();
        let _srv = serve(b, |r| hello_ok(&r).unwrap_or_else(|| vec![(Reply::Error(AudioError::NotSettable), vec![])]));
        let sys = System::from_socket(a, "t").unwrap();
        let r = sys.set(ObjectId(5), PropertyAddress::global(props::NAME), PropertyValue::String("x".into()));
        assert_eq!(r, Err(AudioError::NotSettable));
    }

    #[test]
    fn a_wrong_protocol_version_is_refused() {
        let (a, b) = conn::socket_pair();
        let _srv = serve(b, |_| vec![(Reply::Welcome { version: PROTOCOL_VERSION + 1, client_id: 1 }, vec![])]);
        assert_eq!(System::from_socket(a, "t").err(), Some(AudioError::Protocol));
    }

    #[test]
    fn a_dead_server_is_no_server() {
        let (a, b) = conn::socket_pair();
        drop(b);
        assert_eq!(System::from_socket(a, "t").err(), Some(AudioError::NoServer));
    }

    #[test]
    fn devices_lists_the_owned_objects() {
        let (a, b) = conn::socket_pair();
        let _srv = serve(b, |r| hello_ok(&r).unwrap_or_else(|| vec![(Reply::Objects(vec![ObjectId(2), ObjectId(9)]), vec![])]));
        let sys = System::from_socket(a, "t").unwrap();
        let ids: Vec<u32> = sys.devices().unwrap().iter().map(|d| d.id.0).collect();
        assert_eq!(ids, vec![2, 9]);
    }

    #[test]
    fn add_io_proc_rejects_no_channels_and_too_many() {
        let (a, b) = conn::socket_pair();
        let _srv = serve(b, |r| hello_ok(&r).unwrap_or_default());
        let sys = System::from_socket(a, "t").unwrap();
        let d = Device { id: ObjectId(2), system: sys };
        let f = || -> IoProc { Box::new(|_, _, _, _, _, _| {}) };
        assert_eq!(d.add_io_proc(IoProcConfig { input_channels: 0, output_channels: 0, buffer_frames: 64 }, f()).err(), Some(AudioError::BadValue));
        assert_eq!(d.add_io_proc(IoProcConfig { input_channels: 33, output_channels: 0, buffer_frames: 64 }, f()).err(), Some(AudioError::BadValue));
    }

    /// The server side of the cycle: the device is two channels in and out, the ring holds 8 frames, the client
    /// asks for 4. Input channel 0 is 1 2 3 4 and channel 1 is 5 6 7 8; the IOProc doubles it.
    #[test]
    fn io_proc_receives_input_and_output_for_the_same_cycle() {
        let layout = bifrost_audio_shm::layout(2, 2, 8);
        let region = Arc::new(Region::create("hal-test", layout.total as usize).unwrap());
        // SAFETY: the test owns the fresh mapping; the headers are plain u32 fields nobody else touches yet, and
        // the offsets come from `layout` so they are inside the mapping.
        unsafe {
            for off in [layout.input_ring, layout.output_ring] {
                let h = (region.addr + off as usize) as *mut RingHeader;
                (*h).frames = 8;
                (*h).channels = 2;
            }
            let act = (region.addr + layout.activation as usize) as *mut bifrost_audio_shm::Activation;
            (*act).rate.store(48_000, std::sync::atomic::Ordering::Relaxed);
            (*act).quantum.store(4, std::sync::atomic::Ordering::Relaxed);
        }
        let (a, b) = conn::socket_pair();
        let r2 = region.clone();
        let srv_fd = r2.fd.as_raw_fd();
        let _srv = serve(b, move |r| {
            if let Some(o) = hello_ok(&r) {
                return o;
            }
            match r {
                Request::Get { .. } => vec![(Reply::Value(PropertyValue::U32(4)), vec![])],
                Request::AddIoProc(q) => {
                    assert_eq!((q.input_channels, q.output_channels, q.buffer_frames), (2, 2, 4));
                    vec![(Reply::IoProcAdded { id: 7, layout }, vec![srv_fd])]
                }
                Request::PromoteThread { tid } => {
                    assert!(tid > 0);
                    vec![(Reply::Done, vec![])]
                }
                Request::StartIoProc { id } | Request::StopIoProc { id } | Request::RemoveIoProc { id } => {
                    assert_eq!(id, 7);
                    vec![(Reply::Done, vec![])]
                }
                _ => vec![(Reply::Error(AudioError::Protocol), vec![])],
            }
        });
        let sys = System::from_socket(a, "daw").unwrap();
        let dev = Device { id: ObjectId(2), system: sys };

        let seen_cycle = Arc::new(AtomicU64::new(u64::MAX));
        let seen_frames = Arc::new(AtomicU64::new(0));
        let (sc, sf) = (seen_cycle.clone(), seen_frames.clone());
        let id = dev
            .add_io_proc(
                IoProcConfig { input_channels: 2, output_channels: 2, buffer_frames: 0 },
                Box::new(move |ctx, _now, input, _it, output, _ot| {
                    sc.store(ctx.cycle, Ordering::Relaxed);
                    sf.store(ctx.frames as u64, Ordering::Relaxed);
                    for (o, i) in output.buffers.iter_mut().zip(input.buffers.iter()) {
                        for (os, is) in o.data.iter_mut().zip(i.data.iter()) {
                            *os = *is * 2.0;
                        }
                    }
                }),
            )
            .unwrap();
        assert_eq!(id, IoProcId(7));
        dev.start(id).unwrap();

        // act as the server's cycle: input into the ring, publish the zero stamp, wake the client
        let act: &bifrost_audio_shm::Activation = region.get(layout.activation);
        let zs: &ZeroStamp = region.get(layout.zero_stamp);
        zs.publish(ZeroStampValue { sample_time: 0, host_ns: now_ns(), rate_scalar: 1.0, seed: 1 });
        let in_ring = iothread::ring_view(&region, layout.input_ring, 2).unwrap();
        assert_eq!(in_ring.write(&[&[1.0, 2.0, 3.0, 4.0], &[5.0, 6.0, 7.0, 8.0]], 4), 4);
        act.status.store(bifrost_audio_shm::STATUS_TRIGGERED, Ordering::Release);
        act.wake.fetch_add(1, Ordering::AcqRel);
        futex::wake(&act.wake, 1);

        let t0 = now_ns();
        while act.status.load(Ordering::Acquire) != STATUS_FINISHED {
            assert!(now_ns() - t0 < 3_000_000_000, "the IOProc did not finish the cycle");
            std::thread::sleep(Duration::from_millis(1));
        }
        let out_ring = iothread::ring_view(&region, layout.output_ring, 2).unwrap();
        let (mut c0, mut c1) = ([0f32; 4], [0f32; 4]);
        assert_eq!(out_ring.read(&mut [&mut c0[..], &mut c1[..]], 4), 4);
        assert_eq!(c0, [2.0, 4.0, 6.0, 8.0]);
        assert_eq!(c1, [10.0, 12.0, 14.0, 16.0]);
        assert_eq!(seen_cycle.load(Ordering::Relaxed), 0);
        assert_eq!(seen_frames.load(Ordering::Relaxed), 4);
        assert_eq!(dev.io_proc_xruns(id).unwrap(), 0);

        dev.stop(id).unwrap();
        dev.remove_io_proc(id).unwrap();
        assert_eq!(dev.start(id).err(), Some(AudioError::UnknownObject));
    }
}

/// Hog mode requests (owner: task 10).
pub mod hog;
