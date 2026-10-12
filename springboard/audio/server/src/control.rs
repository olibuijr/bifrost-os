//! The control side of the server: `ServerCore` (clients, IO procedures, devices, the start and stop policy) and the
//! `/run/bifrost/audio/control` socket loop (`SOCK_SEQPACKET`, `SCM_RIGHTS`) around it. Owner: task 05.
//!
//! The object tree (task 02) plugs in through `ObjectApi`: `Get`, `Set`, `Listen`, `Unlisten` and `Owned` go to it.
//! Everything about IO procedures is handled here.
//!
//! Device lifecycle: a device starts when its first IO procedure starts running and stops `idle_stop` (2 s) after
//! the last one stopped. With no running device the loop blocks in `poll` with no timeout: no wakeups when idle.

use crate::client::{recv_msg, send_msg, Conn, HELLO_TIMEOUT, MAX_STALLED};
use crate::io::{
    log, set_realtime, ClientSlot, DeviceRunner, RunnerConfig, Shared, WaitPolicy, CLIENT_PRIORITY, DEFAULT_QUANTUM,
    MAX_CH, MAX_QUANTUM, MIN_QUANTUM,
};
use bifrost_audio_driver::{DeviceDesc, Driver, DriverEvent, IoConfig};
use bifrost_audio_types::control::{
    decode_request, encode_reply, IoProcRequest, Reply, Request, CONTROL_SOCKET, MAX_LEN, PROTOCOL_VERSION,
};
use bifrost_audio_types::{AudioError, ObjectId, PropertyAddress, PropertyValue, SYSTEM_OBJECT};
use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Device `i` is object `DEVICE_ID_BASE + i` until the object tree (task 02) assigns ids and passes them in.
pub const DEVICE_ID_BASE: u32 = 1000;
/// How often the loop wakes while a device runs (overload notices, driver events).
pub const TICK: Duration = Duration::from_millis(250);

/// What the server needs from the object tree.
pub trait ObjectApi: Send {
    fn get(&mut self, object: ObjectId, address: PropertyAddress) -> Result<PropertyValue, AudioError>;
    fn set(
        &mut self,
        client: u32,
        policy: bool,
        object: ObjectId,
        address: PropertyAddress,
        value: PropertyValue,
    ) -> Result<(), AudioError>;
    fn listen(&mut self, client: u32, object: ObjectId, address: PropertyAddress) -> Result<(), AudioError>;
    fn unlisten(&mut self, client: u32, object: ObjectId, address: PropertyAddress) -> Result<(), AudioError>;
    fn owned(&mut self, object: ObjectId) -> Result<Vec<ObjectId>, AudioError>;
    /// Coalesced change notices to deliver, per client.
    fn drain_changes(&mut self) -> Vec<(u32, Vec<(ObjectId, PropertyAddress)>)> {
        Vec::new()
    }
    fn client_gone(&mut self, _client: u32) {}
}

/// Stand-in until the object tree is wired in: every object is unknown.
pub struct NoObjects;

impl ObjectApi for NoObjects {
    fn get(&mut self, _: ObjectId, _: PropertyAddress) -> Result<PropertyValue, AudioError> {
        Err(AudioError::UnknownObject)
    }
    fn set(&mut self, _: u32, _: bool, _: ObjectId, _: PropertyAddress, _: PropertyValue) -> Result<(), AudioError> {
        Err(AudioError::UnknownObject)
    }
    fn listen(&mut self, _: u32, _: ObjectId, _: PropertyAddress) -> Result<(), AudioError> {
        Err(AudioError::UnknownObject)
    }
    fn unlisten(&mut self, _: u32, _: ObjectId, _: PropertyAddress) -> Result<(), AudioError> {
        Err(AudioError::UnknownObject)
    }
    fn owned(&mut self, _: ObjectId) -> Result<Vec<ObjectId>, AudioError> {
        Err(AudioError::UnknownObject)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CoreConfig {
    /// Run each device on its own thread (false: tests step the runner by hand).
    pub spawn_threads: bool,
    pub wait: WaitPolicy,
    /// How long a device keeps running after its last stream stopped.
    pub idle_stop: Duration,
}

impl Default for CoreConfig {
    fn default() -> CoreConfig {
        CoreConfig {
            spawn_threads: true,
            // Wake half a quantum after the trigger at most; the budget is per cycle, not per client.
            wait: WaitPolicy::Futex { budget_ns: 2_500_000 },
            idle_stop: Duration::from_secs(2),
        }
    }
}

struct DeviceHost {
    driver: usize,
    desc: DeviceDesc,
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
    manual: Option<DeviceRunner>,
    /// Drops the PCM from the control thread (see `DeviceIo::abort_hook`); held while the IO thread lives.
    abort: Option<Box<dyn Fn() + Send + Sync>>,
    /// An IO thread that outlived its stop request. The device stays busy (no reopen) until it is reaped.
    stopping: Option<std::thread::JoinHandle<()>>,
    running: bool,
    idle_since: Option<Instant>,
}

struct ClientRec {
    name: Option<String>,
    policy: bool,
    uid: u32,
    pid: i32,
}

struct SlotRec {
    device: usize,
    slot: Arc<ClientSlot>,
}

/// A reply and the descriptors that go with it.
pub struct Handled {
    pub reply: Reply,
    pub fds: Vec<OwnedFd>,
}

fn plain(reply: Reply) -> Handled {
    Handled { reply, fds: Vec::new() }
}

fn err(e: AudioError) -> Handled {
    plain(Reply::Error(e))
}

pub struct ServerCore {
    drivers: Vec<Box<dyn Driver>>,
    devices: Vec<DeviceHost>,
    clients: HashMap<u32, ClientRec>,
    slots: HashMap<u32, SlotRec>,
    next_slot: u32,
    objects: Box<dyn ObjectApi>,
    cfg: CoreConfig,
    server_uid: u32,
}

impl ServerCore {
    pub fn new(mut drivers: Vec<Box<dyn Driver>>, objects: Box<dyn ObjectApi>, cfg: CoreConfig) -> ServerCore {
        let mut devices = Vec::new();
        for (i, d) in drivers.iter_mut().enumerate() {
            let mut descs = Vec::new();
            d.enumerate(&mut descs);
            for desc in descs {
                log("driver", &format!("{}: device {} ({})", d.name(), desc.uid, desc.name));
                devices.push(Self::host(i, desc));
            }
        }
        // SAFETY: getuid cannot fail.
        let server_uid = unsafe { libc::getuid() };
        ServerCore { drivers, devices, clients: HashMap::new(), slots: HashMap::new(), next_slot: 1, objects, cfg, server_uid }
    }

    fn host(driver: usize, desc: DeviceDesc) -> DeviceHost {
        let q = DEFAULT_QUANTUM.clamp(desc.buffer_frames_range.0.max(MIN_QUANTUM), desc.buffer_frames_range.1.min(MAX_QUANTUM));
        DeviceHost { driver, desc, shared: Shared::new(q), thread: None, manual: None, abort: None, stopping: None, running: false, idle_since: None }
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    pub fn device_id(&self, index: usize) -> ObjectId {
        ObjectId(DEVICE_ID_BASE + index as u32)
    }

    pub fn device_running(&self, index: usize) -> bool {
        self.devices[index].running
    }

    pub fn device_quantum(&self, index: usize) -> u32 {
        self.devices[index].shared.quantum.load(Ordering::Acquire)
    }

    pub fn device_shared(&self, index: usize) -> Arc<Shared> {
        self.devices[index].shared.clone()
    }

    /// The runner of a device started with `spawn_threads: false`.
    pub fn manual_runner(&mut self, index: usize) -> Option<&mut DeviceRunner> {
        self.devices[index].manual.as_mut()
    }

    pub fn slot(&self, id: u32) -> Option<Arc<ClientSlot>> {
        self.slots.get(&id).map(|r| r.slot.clone())
    }

    pub fn add_client(&mut self, id: u32, pid: i32, uid: u32) {
        self.clients.insert(id, ClientRec { name: None, policy: false, uid, pid });
    }

    /// A connection closed: its IO procedures go with it.
    pub fn remove_client(&mut self, id: u32) {
        let mine: Vec<u32> = self.slots.iter().filter(|(_, r)| r.slot.owner == id).map(|(k, _)| *k).collect();
        for s in mine {
            self.remove_slot(s);
        }
        self.clients.remove(&id);
        self.objects.client_gone(id);
        log("client", &format!("client {id} gone"));
    }

    /// Whether the client has said Hello.
    pub fn greeted(&self, id: u32) -> bool {
        self.clients.get(&id).map_or(false, |c| c.name.is_some())
    }

    pub fn handle(&mut self, client: u32, req: Request) -> Handled {
        if !matches!(req, Request::Hello { .. }) && self.clients.get(&client).map_or(true, |c| c.name.is_none()) {
            return err(AudioError::Protocol);
        }
        match req {
            Request::Hello { version, client: name, policy } => {
                if version != PROTOCOL_VERSION {
                    return err(AudioError::Protocol);
                }
                let server_uid = self.server_uid;
                let Some(c) = self.clients.get_mut(&client) else { return err(AudioError::Protocol) };
                if policy && c.uid != 0 && c.uid != server_uid {
                    return err(AudioError::PermissionDenied);
                }
                log("client", &format!("client {client}: {name} (pid {}, policy {policy})", c.pid));
                c.name = Some(name);
                c.policy = policy;
                plain(Reply::Welcome { version: PROTOCOL_VERSION, client_id: client })
            }
            Request::Get { object, address } => match self.objects.get(object, address) {
                Ok(v) => plain(Reply::Value(v)),
                Err(e) => err(e),
            },
            Request::Set { object, address, value } => {
                let policy = self.clients[&client].policy;
                match self.objects.set(client, policy, object, address, value) {
                    Ok(()) => plain(Reply::Done),
                    Err(e) => err(e),
                }
            }
            Request::Listen { object, address } => match self.objects.listen(client, object, address) {
                Ok(()) => plain(Reply::Done),
                Err(e) => err(e),
            },
            Request::Unlisten { object, address } => match self.objects.unlisten(client, object, address) {
                Ok(()) => plain(Reply::Done),
                Err(e) => err(e),
            },
            Request::Owned { object } => match self.objects.owned(object) {
                Ok(v) => plain(Reply::Objects(v)),
                Err(AudioError::UnknownObject) if object == SYSTEM_OBJECT => {
                    plain(Reply::Objects((0..self.devices.len()).map(|i| self.device_id(i)).collect()))
                }
                Err(e) => err(e),
            },
            Request::AddIoProc(r) => self.add_io_proc(client, r),
            Request::RemoveIoProc { id } => match self.owned_slot(client, id) {
                Ok(_) => {
                    self.remove_slot(id);
                    plain(Reply::Done)
                }
                Err(e) => err(e),
            },
            Request::StartIoProc { id } => match self.owned_slot(client, id) {
                Ok(r) => {
                    r.slot.running.store(true, Ordering::Release);
                    let dev = r.device;
                    self.recompute_quantum(dev);
                    self.devices[dev].idle_since = None;
                    match self.start_device(dev) {
                        Ok(()) => plain(Reply::Done),
                        Err(e) => {
                            self.slots[&id].slot.running.store(false, Ordering::Release);
                            err(e)
                        }
                    }
                }
                Err(e) => err(e),
            },
            Request::StopIoProc { id } => match self.owned_slot(client, id) {
                Ok(r) => {
                    r.slot.running.store(false, Ordering::Release);
                    self.recompute_quantum(r.device);
                    plain(Reply::Done)
                }
                Err(e) => err(e),
            },
            Request::PromoteThread { tid } => {
                let pid = self.clients[&client].pid;
                if tid <= 0 || !std::path::Path::new(&format!("/proc/{pid}/task/{tid}")).exists() {
                    return err(AudioError::PermissionDenied);
                }
                match set_realtime(tid, CLIENT_PRIORITY, 1024) {
                    Ok(()) => plain(Reply::Done),
                    Err(e) if e.raw_os_error() == Some(libc::EPERM) => err(AudioError::PermissionDenied),
                    Err(e) => {
                        log("client", &format!("promote tid {tid}: {e}"));
                        err(AudioError::Io)
                    }
                }
            }
        }
    }

    fn owned_slot(&self, client: u32, id: u32) -> Result<SlotView, AudioError> {
        match self.slots.get(&id) {
            Some(r) if r.slot.owner == client => Ok(SlotView { slot: r.slot.clone(), device: r.device }),
            Some(_) => Err(AudioError::PermissionDenied),
            None => Err(AudioError::UnknownObject),
        }
    }

    fn add_io_proc(&mut self, client: u32, r: IoProcRequest) -> Handled {
        let Some(dev) = r.device.0.checked_sub(DEVICE_ID_BASE).map(|i| i as usize).filter(|i| *i < self.devices.len())
        else {
            return err(AudioError::UnknownObject);
        };
        let range = self.devices[dev].desc.buffer_frames_range;
        if r.input_channels as usize > MAX_CH
            || r.output_channels as usize > MAX_CH
            || r.input_channels + r.output_channels == 0
            || r.buffer_frames < range.0.max(MIN_QUANTUM)
            || r.buffer_frames > range.1.min(MAX_QUANTUM * 2)
        {
            return err(AudioError::BadValue);
        }
        let id = self.next_slot;
        self.next_slot += 1;
        let ring_frames = (r.buffer_frames.max(DEFAULT_QUANTUM) * 4).next_power_of_two();
        let slot = match ClientSlot::new(id, client, r.input_channels, r.output_channels, r.buffer_frames, ring_frames) {
            Ok(s) => Arc::new(s),
            Err(e) => {
                log("client", &format!("cannot create shared memory: {e}"));
                return err(AudioError::Io);
            }
        };
        let (Ok(mem), Ok(ev)) = (slot.region.fd.try_clone(), slot.wake_fd.try_clone()) else { return err(AudioError::Io) };
        let layout = slot.layout;
        self.slots.insert(id, SlotRec { device: dev, slot });
        self.publish(dev);
        log("client", &format!("client {client}: io proc {id} on device {dev} ({} frames)", r.buffer_frames));
        Handled { reply: Reply::IoProcAdded { id, layout }, fds: vec![mem, ev] }
    }

    fn remove_slot(&mut self, id: u32) {
        if let Some(r) = self.slots.remove(&id) {
            r.slot.running.store(false, Ordering::Release);
            self.publish(r.device);
            self.recompute_quantum(r.device);
        }
    }

    fn publish(&self, dev: usize) {
        let mut v: Vec<Arc<ClientSlot>> =
            self.slots.values().filter(|r| r.device == dev).map(|r| r.slot.clone()).collect();
        v.sort_by_key(|s| s.id);
        self.devices[dev].shared.publish(v);
    }

    /// The graph quantum is the smallest buffer any running client asked for (default 256), within the device's range.
    fn recompute_quantum(&self, dev: usize) {
        let d = &self.devices[dev];
        let want = self
            .slots
            .values()
            .filter(|r| r.device == dev && r.slot.running.load(Ordering::Acquire))
            .map(|r| r.slot.buffer_frames)
            .min()
            .unwrap_or(DEFAULT_QUANTUM);
        let q = want.clamp(d.desc.buffer_frames_range.0.max(MIN_QUANTUM), d.desc.buffer_frames_range.1.min(MAX_QUANTUM));
        // never below what the sustained-xrun fallback raised the device to
        let q = q.max(d.shared.fallback_floor.load(Ordering::Acquire));
        d.shared.quantum.store(q, Ordering::Release);
    }

    fn start_device(&mut self, idx: usize) -> Result<(), AudioError> {
        if self.devices[idx].running {
            return Ok(());
        }
        if !self.reap_stopping(idx) {
            log("io", &format!("device {} still stopping; not reopening", self.devices[idx].desc.uid));
            return Err(AudioError::Io);
        }
        let desc = self.devices[idx].desc.clone();
        let rate = if desc.rates.iter().any(|r| 48_000.0 >= r.0 && 48_000.0 <= r.1) {
            48_000.0
        } else {
            desc.rates.first().map(|r| r.0).unwrap_or(48_000.0)
        };
        let shared = self.devices[idx].shared.clone();
        let quantum = shared.quantum.load(Ordering::Acquire);
        let cfg = IoConfig { rate, quantum, input: desc.input_channels > 0, output: desc.output_channels > 0 };
        let io = self.drivers[self.devices[idx].driver].open(&desc.uid, &cfg).map_err(|e| {
            log("driver", &format!("open {}: {e:?}", desc.uid));
            AudioError::Io
        })?;
        let rc = RunnerConfig {
            rate: rate as u32,
            in_channels: desc.input_channels,
            out_channels: desc.output_channels,
            wait: self.cfg.wait,
        };
        let abort = io.abort_hook();
        let mut runner = DeviceRunner::new(io, shared.clone(), rc);
        shared.stop.store(false, Ordering::Release);
        if self.cfg.spawn_threads {
            let tuning = crate::rt::device_started();
            let h = crate::io::spawn_io_thread_tuned(runner, format!("audio-io-{idx}"), tuning).map_err(|_| {
                crate::rt::device_stopped();
                AudioError::Io
            })?;
            self.devices[idx].thread = Some(h);
            self.devices[idx].abort = abort;
        } else {
            runner.start().map_err(|_| AudioError::Io)?;
            self.devices[idx].manual = Some(runner);
        }
        self.devices[idx].running = true;
        log("io", &format!("device {} started at {rate} Hz, quantum {quantum}", desc.uid));
        Ok(())
    }

    fn stop_device(&mut self, idx: usize) {
        let d = &mut self.devices[idx];
        if !d.running {
            return;
        }
        d.shared.stop.store(true, Ordering::Release);
        if let Some(h) = d.thread.take() {
            // The IO loop checks the stop flag every period, so this normally takes a few ms. If it does not end,
            // drop the PCM from here (wakes a thread blocked in the driver), wait once more, and never detach: the
            // handle is kept in `stopping` and the device cannot be reopened until the thread has really ended.
            let wait = |h: &std::thread::JoinHandle<()>| {
                let until = Instant::now() + STOP_WAIT;
                while !h.is_finished() && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(2));
                }
            };
            wait(&h);
            if !h.is_finished() {
                log("io", &format!("device {} did not stop within {:?}; dropping the stream", d.desc.uid, STOP_WAIT));
                if let Some(a) = &d.abort {
                    a();
                }
                wait(&h);
            }
            if h.is_finished() {
                let _ = h.join();
                d.abort = None;
                crate::rt::device_stopped();
            } else {
                log("io", &format!("device {} IO thread still alive; the device stays busy until it ends", d.desc.uid));
                d.stopping = Some(h);
            }
        }
        d.shared.fallback_floor.store(0, Ordering::Release);
        if let Some(mut r) = d.manual.take() {
            r.stop();
        }
        d.running = false;
        d.idle_since = None;
        log("io", &format!("device {} stopped", d.desc.uid));
    }

    /// Joins a stop-requested IO thread that has ended since. False while one is still alive.
    fn reap_stopping(&mut self, idx: usize) -> bool {
        let d = &mut self.devices[idx];
        match d.stopping.take() {
            None => true,
            Some(h) if h.is_finished() => {
                let _ = h.join();
                d.abort = None;
                crate::rt::device_stopped();
                log("io", &format!("device {} IO thread ended; reopen allowed", d.desc.uid));
                true
            }
            Some(h) => {
                if let Some(a) = &d.abort {
                    a();
                }
                d.stopping = Some(h);
                false
            }
        }
    }

    fn running_slots(&self, dev: usize) -> usize {
        self.slots.values().filter(|r| r.device == dev && r.slot.running.load(Ordering::Acquire)).count()
    }

    /// Stops devices that have been idle for `idle_stop`, and picks up driver events.
    pub fn tick(&mut self, now: Instant) {
        for i in 0..self.devices.len() {
            if self.devices[i].stopping.is_some() {
                self.reap_stopping(i);
            }
        }
        for i in 0..self.devices.len() {
            if !self.devices[i].running {
                continue;
            }
            if self.running_slots(i) > 0 {
                self.devices[i].idle_since = None;
                continue;
            }
            let since = *self.devices[i].idle_since.get_or_insert(now);
            if now.duration_since(since) >= self.cfg.idle_stop {
                self.stop_device(i);
            }
        }
        let mut events = Vec::new();
        for (di, d) in self.drivers.iter_mut().enumerate() {
            let mut ev = Vec::new();
            d.poll_events(&mut ev);
            events.extend(ev.into_iter().map(|e| (di, e)));
        }
        for (driver, e) in events {
            match e {
                DriverEvent::DeviceAdded(desc) => {
                    if !self.devices.iter().any(|d| d.desc.uid == desc.uid) {
                        log("driver", &format!("device added: {}", desc.uid));
                        self.devices.push(Self::host(driver, desc));
                    }
                }
                other => log("driver", &format!("event: {other:?}")),
            }
        }
    }

    /// How long the loop may sleep: `None` (forever) when no device runs.
    pub fn next_timeout(&self, now: Instant) -> Option<Duration> {
        let mut t: Option<Duration> = None;
        for (i, d) in self.devices.iter().enumerate() {
            if !d.running {
                continue;
            }
            let mut w = TICK;
            if self.running_slots(i) == 0 {
                if let Some(s) = d.idle_since {
                    w = w.min(self.cfg.idle_stop.saturating_sub(now.duration_since(s)));
                }
            }
            t = Some(t.map_or(w, |x| x.min(w)));
        }
        t
    }

    /// Overload notices not yet delivered: (client connection, reply).
    pub fn take_overloads(&mut self) -> Vec<(u32, Reply)> {
        let mut out = Vec::new();
        for (id, r) in &self.slots {
            let n = r.slot.overloads.swap(0, Ordering::AcqRel);
            if n > 0 {
                out.push((r.slot.owner, Reply::Overload { id: *id, count: n }));
            }
        }
        out
    }

    pub fn take_changes(&mut self) -> Vec<(u32, Reply)> {
        self.objects.drain_changes().into_iter().map(|(c, v)| (c, Reply::Changed(v))).collect()
    }

    pub fn shutdown(&mut self) {
        for i in 0..self.devices.len() {
            self.stop_device(i);
        }
    }
}

struct SlotView {
    slot: Arc<ClientSlot>,
    device: usize,
}

// ---------------------------------------------------------------------------------------------------------------
// The socket

/// Binds and listens on the control socket (`SOCK_SEQPACKET`, mode 0666: any local user may connect).
pub fn listen(path: &str) -> std::io::Result<OwnedFd> {
    if let Some(dir) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(path);
    // SAFETY: socket takes no pointers; the result is checked and then owned.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: a fresh descriptor nothing else owns.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: a zeroed sockaddr_un is valid; the path is copied within sun_path's bounds below.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = path.as_bytes();
    if bytes.len() >= addr.sun_path.len() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "socket path too long"));
    }
    for (d, s) in addr.sun_path.iter_mut().zip(bytes) {
        *d = *s as libc::c_char;
    }
    // SAFETY: `addr` is a valid sockaddr_un of the size passed.
    let r = unsafe {
        libc::bind(fd, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_un>() as u32)
    };
    if r < 0 {
        return Err(std::io::Error::last_os_error());
    }
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))?;
    // SAFETY: listen on the bound socket.
    if unsafe { libc::listen(fd, 16) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(owned)
}

/// Sends without ever waiting (the socket is non-blocking). Returns true when the client's queue was full, so the
/// caller can count stalls and drop a client that stopped reading.
fn send_reply(fd: RawFd, reply: &Reply, fds: &[RawFd]) -> bool {
    let bytes = encode_reply(reply);
    if let Err(e) = send_msg(fd, &bytes, fds) {
        if e.kind() == std::io::ErrorKind::WouldBlock {
            return true;
        }
        log("client", &format!("send failed: {e}"));
    }
    false
}

/// How long the control loop waits for a device's IO thread to end before it detaches it.
const STOP_WAIT: Duration = Duration::from_millis(500);

/// At most this many requests from one client per round of the loop, so one chatty client cannot starve the others.
const MAX_PER_ROUND: usize = 32;
/// The loop wakes at least this often to drop clients that never said Hello.
const HOUSEKEEPING_MS: i32 = 1000;

/// Runs the server until `stop` is set: accept, read requests, reply, deliver notices, start and stop devices.
///
/// `wake` is an optional descriptor (a signalfd) whose readability also ends the loop, so a termination signal
/// never races with the blocking `poll`.
pub fn serve(core: &mut ServerCore, listener: &OwnedFd, stop: &AtomicBool, wake: Option<RawFd>) {
    let mut conns: Vec<Conn> = Vec::new();
    let mut next_id = 1u32;
    let mut buf = vec![0u8; MAX_LEN];
    while !stop.load(Ordering::Acquire) {
        let mut pfds: Vec<libc::pollfd> = Vec::with_capacity(conns.len() + 1);
        pfds.push(libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        pfds.push(libc::pollfd { fd: wake.unwrap_or(-1), events: libc::POLLIN, revents: 0 });
        for c in &conns {
            pfds.push(libc::pollfd { fd: c.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        }
        let timeout_ms = core.next_timeout(Instant::now()).map_or(HOUSEKEEPING_MS, |d| (d.as_millis().max(1) as i32).min(HOUSEKEEPING_MS));
        // SAFETY: `pfds` is a valid array of `len` pollfd structs for the call.
        let n = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, timeout_ms) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            log("io", "poll failed; stopping");
            break;
        }
        if pfds[1].revents & libc::POLLIN != 0 {
            break;
        }
        if pfds[0].revents & libc::POLLIN != 0 {
            loop {
                // SAFETY: accept4 with null address pointers is allowed.
                let fd = unsafe {
                    libc::accept4(listener.as_raw_fd(), std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK)
                };
                if fd < 0 {
                    break;
                }
                // SAFETY: a fresh descriptor from accept4.
                let c = Conn::new(unsafe { OwnedFd::from_raw_fd(fd) }, next_id);
                next_id += 1;
                core.add_client(c.id, c.pid, c.uid);
                conns.push(c);
            }
        }
        let mut dead = Vec::new();
        // Only the connections that were polled have a `pfds` entry; the ones accepted above start next round.
        for (i, c) in conns.iter_mut().enumerate().take(pfds.len() - 2) {
            if pfds[i + 2].revents == 0 {
                continue;
            }
            let mut served = 0;
            loop {
                if served >= MAX_PER_ROUND {
                    // the rest is still queued: poll reports the socket readable again next round
                    break;
                }
                served += 1;
                match recv_msg(c.fd.as_raw_fd(), &mut buf) {
                    Ok((0, _)) => {
                        dead.push(c.id);
                        break;
                    }
                    Ok((len, _fds)) => {
                        // Descriptors a client attaches (a pidfd with PromoteThread) are closed on drop; the thread
                        // is checked against the peer's pid instead.
                        let h = match decode_request(&buf[..len]) {
                            Ok(req) => core.handle(c.id, req),
                            Err(_) => err(AudioError::Protocol),
                        };
                        let raw: Vec<RawFd> = h.fds.iter().map(|f| f.as_raw_fd()).collect();
                        if send_reply(c.fd.as_raw_fd(), &h.reply, &raw) {
                            c.stalled += 1;
                        } else {
                            c.stalled = 0;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        dead.push(c.id);
                        break;
                    }
                }
            }
        }
        // a client that connected and never said Hello, or stopped reading its replies, is dropped
        let now = Instant::now();
        for c in &conns {
            if (!core.greeted(c.id) && now.duration_since(c.accepted) >= HELLO_TIMEOUT) || c.stalled >= MAX_STALLED {
                log("client", &format!("client {} dropped ({})", c.id, if c.stalled >= MAX_STALLED { "not reading" } else { "no Hello" }));
                if !dead.contains(&c.id) {
                    dead.push(c.id);
                }
            }
        }
        for id in dead {
            conns.retain(|c| c.id != id);
            core.remove_client(id);
        }
        core.tick(Instant::now());
        for (client, reply) in core.take_overloads().into_iter().chain(core.take_changes()) {
            if let Some(c) = conns.iter_mut().find(|c| c.id == client) {
                if send_reply(c.fd.as_raw_fd(), &reply, &[]) {
                    c.stalled += 1;
                } else {
                    c.stalled = 0;
                }
            }
        }
    }
    core.shutdown();
}

/// The default socket path.
pub fn default_socket() -> &'static str {
    CONTROL_SOCKET
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_driver::null::{NullDriver, NullHandle};

    fn core() -> (ServerCore, NullHandle) {
        let d = NullDriver::new(vec![NullDriver::stereo("n0", 48_000.0)]);
        let h = d.handle();
        let cfg = CoreConfig { spawn_threads: false, wait: WaitPolicy::None, idle_stop: Duration::from_secs(2) };
        let mut c = ServerCore::new(vec![Box::new(d)], Box::new(NoObjects), cfg);
        // SAFETY: getuid cannot fail.
        c.add_client(1, std::process::id() as i32, unsafe { libc::getuid() });
        c.add_client(2, 1, 12345);
        (c, h)
    }

    fn hello(c: &mut ServerCore, id: u32, policy: bool) -> Reply {
        c.handle(id, Request::Hello { version: PROTOCOL_VERSION, client: "test".into(), policy }).reply
    }

    fn proc_req(frames: u32) -> Request {
        Request::AddIoProc(IoProcRequest {
            device: ObjectId(DEVICE_ID_BASE),
            input_channels: 0,
            output_channels: 2,
            buffer_frames: frames,
        })
    }

    fn add(c: &mut ServerCore, client: u32, frames: u32) -> u32 {
        match c.handle(client, proc_req(frames)).reply {
            Reply::IoProcAdded { id, .. } => id,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_threaded_device_stops_quickly_and_can_start_again_for_every_quantum() {
        let d = NullDriver::new(vec![NullDriver::stereo("n0", 48_000.0)]);
        let h = d.handle();
        let cfg = CoreConfig { spawn_threads: true, wait: WaitPolicy::Futex { budget_ns: 2_500_000 }, idle_stop: Duration::from_secs(2) };
        let mut c = ServerCore::new(vec![Box::new(d)], Box::new(NoObjects), cfg);
        // SAFETY: getuid cannot fail.
        c.add_client(1, std::process::id() as i32, unsafe { libc::getuid() });
        hello(&mut c, 1, false);
        for q in [32, 64, 128, 256] {
            let a = add(&mut c, 1, q);
            assert_eq!(c.handle(1, Request::StartIoProc { id: a }).reply, Reply::Done);
            assert!(c.device_running(0));
            // A client that never signals: the IO loop must still see the stop within a bound.
            std::thread::sleep(Duration::from_millis(30));
            let t0 = Instant::now();
            c.remove_client(1);
            c.stop_device(0);
            assert!(t0.elapsed() < Duration::from_millis(400), "stop took {:?}", t0.elapsed());
            assert!(!c.device_running(0) && !h.is_running("n0"));
            assert!(c.devices[0].stopping.is_none());
            c.add_client(1, std::process::id() as i32, unsafe { libc::getuid() });
            hello(&mut c, 1, false);
        }
    }

    #[test]
    fn requests_before_hello_and_wrong_versions_are_refused() {
        let (mut c, _) = core();
        assert_eq!(c.handle(1, proc_req(256)).reply, Reply::Error(AudioError::Protocol));
        let bad = c.handle(1, Request::Hello { version: 99, client: "x".into(), policy: false });
        assert_eq!(bad.reply, Reply::Error(AudioError::Protocol));
        assert_eq!(hello(&mut c, 1, false), Reply::Welcome { version: PROTOCOL_VERSION, client_id: 1 });
    }

    #[test]
    fn policy_rights_need_the_servers_uid_or_root() {
        let (mut c, _) = core();
        assert_eq!(hello(&mut c, 2, true), Reply::Error(AudioError::PermissionDenied));
        assert!(matches!(hello(&mut c, 1, true), Reply::Welcome { .. }));
    }

    #[test]
    fn add_io_proc_returns_a_memfd_and_an_eventfd_and_validates() {
        let (mut c, _) = core();
        hello(&mut c, 1, false);
        let h = c.handle(1, proc_req(256));
        assert_eq!(h.fds.len(), 2);
        let Reply::IoProcAdded { id: 1, layout } = h.reply else { panic!() };
        assert!(layout.total > layout.output_ring);
        let bad_dev = c.handle(
            1,
            Request::AddIoProc(IoProcRequest { device: ObjectId(1), input_channels: 0, output_channels: 2, buffer_frames: 256 }),
        );
        assert_eq!(bad_dev.reply, Reply::Error(AudioError::UnknownObject));
        assert_eq!(c.handle(1, proc_req(1)).reply, Reply::Error(AudioError::BadValue));
        let none = Request::AddIoProc(IoProcRequest { device: ObjectId(DEVICE_ID_BASE), input_channels: 0, output_channels: 0, buffer_frames: 256 });
        assert_eq!(c.handle(1, none).reply, Reply::Error(AudioError::BadValue));
    }

    #[test]
    fn the_device_starts_with_the_first_stream_and_stops_two_seconds_after_the_last() {
        let (mut c, h) = core();
        hello(&mut c, 1, false);
        let a = add(&mut c, 1, 256);
        assert!(!c.device_running(0));
        assert_eq!(c.next_timeout(Instant::now()), None);
        assert_eq!(c.handle(1, Request::StartIoProc { id: a }).reply, Reply::Done);
        assert!(c.device_running(0) && h.is_running("n0"));
        assert_eq!(c.next_timeout(Instant::now()), Some(TICK));
        c.handle(1, Request::StopIoProc { id: a });
        let t0 = Instant::now();
        c.tick(t0);
        assert!(c.device_running(0));
        assert_eq!(c.next_timeout(t0), Some(Duration::from_secs(2).min(TICK)));
        c.tick(t0 + Duration::from_millis(1999));
        assert!(c.device_running(0));
        c.tick(t0 + Duration::from_secs(2));
        assert!(!c.device_running(0) && !h.is_running("n0"));
        assert_eq!(h.start_stop_counts("n0"), (1, 1));
        // Idle: no timers at all.
        assert_eq!(c.next_timeout(t0), None);
    }

    #[test]
    fn a_new_stream_cancels_the_pending_stop() {
        let (mut c, h) = core();
        hello(&mut c, 1, false);
        let a = add(&mut c, 1, 256);
        c.handle(1, Request::StartIoProc { id: a });
        c.handle(1, Request::StopIoProc { id: a });
        let t0 = Instant::now();
        c.tick(t0);
        c.handle(1, Request::StartIoProc { id: a });
        c.tick(t0 + Duration::from_secs(5));
        assert!(c.device_running(0));
        assert_eq!(h.start_stop_counts("n0"), (1, 0));
    }

    #[test]
    fn the_quantum_is_the_smallest_running_buffer() {
        let (mut c, _) = core();
        hello(&mut c, 1, false);
        let big = add(&mut c, 1, 1024);
        let small = add(&mut c, 1, 64);
        assert_eq!(c.device_quantum(0), 256);
        c.handle(1, Request::StartIoProc { id: big });
        assert_eq!(c.device_quantum(0), 1024);
        c.handle(1, Request::StartIoProc { id: small });
        assert_eq!(c.device_quantum(0), 64);
        c.handle(1, Request::StopIoProc { id: small });
        assert_eq!(c.device_quantum(0), 1024);
        c.handle(1, Request::RemoveIoProc { id: big });
        assert_eq!(c.device_quantum(0), 256);
    }

    #[test]
    fn only_the_owner_may_change_an_io_proc_and_disconnect_removes_them() {
        let (mut c, _) = core();
        hello(&mut c, 1, false);
        hello(&mut c, 2, false);
        let a = add(&mut c, 1, 256);
        assert_eq!(c.handle(2, Request::RemoveIoProc { id: a }).reply, Reply::Error(AudioError::PermissionDenied));
        assert_eq!(c.handle(1, Request::RemoveIoProc { id: 99 }).reply, Reply::Error(AudioError::UnknownObject));
        c.handle(1, Request::StartIoProc { id: a });
        c.remove_client(1);
        assert!(c.slot(a).is_none());
    }

    #[test]
    fn two_clients_through_the_core_are_mixed_by_the_started_device() {
        let (mut c, h) = core();
        hello(&mut c, 1, false);
        hello(&mut c, 2, false);
        let a = add(&mut c, 1, 256);
        let b = add(&mut c, 2, 256);
        c.handle(1, Request::StartIoProc { id: a });
        c.handle(2, Request::StartIoProc { id: b });
        for (id, v) in [(a, 0.25f32), (b, 0.5f32)] {
            let s = c.slot(id).unwrap();
            let p = vec![v; 256];
            s.output_ring().unwrap().write(&[&p[..], &p[..]], 256);
        }
        h.advance(5_333_333);
        let t = h.clock().now();
        let rep = c.manual_runner(0).unwrap().cycle(t);
        assert_eq!((rep.triggered, rep.late), (2, 0));
        assert_eq!(h.written("n0")[0], vec![0.75; 256]);
    }

    #[test]
    fn owned_of_the_system_lists_the_devices_and_overloads_are_drained_once() {
        let (mut c, _) = core();
        hello(&mut c, 1, false);
        assert_eq!(c.handle(1, Request::Owned { object: SYSTEM_OBJECT }).reply, Reply::Objects(vec![ObjectId(1000)]));
        let a = add(&mut c, 1, 256);
        c.slot(a).unwrap().overloads.store(3, Ordering::Relaxed);
        assert_eq!(c.take_overloads(), vec![(1, Reply::Overload { id: a, count: 3 })]);
        assert!(c.take_overloads().is_empty());
    }

    #[test]
    fn the_socket_serves_hello_over_seqpacket() {
        let path = format!("/tmp/bifrost-audio-test-{}.sock", std::process::id());
        let l = listen(&path).unwrap();
        let (mut core, _) = core();
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let t = std::thread::spawn(move || serve(&mut core, &l, &s2, None));
        // SAFETY: socket and connect with a valid sockaddr_un built below.
        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
        let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (d, s) in addr.sun_path.iter_mut().zip(path.as_bytes()) {
            *d = *s as libc::c_char;
        }
        let r = unsafe { libc::connect(fd, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_un>() as u32) };
        assert_eq!(r, 0);
        // SAFETY: a fresh descriptor.
        let sock = unsafe { OwnedFd::from_raw_fd(fd) };
        let msg = bifrost_audio_types::control::encode_request(&Request::Hello { version: PROTOCOL_VERSION, client: "t".into(), policy: false });
        send_msg(sock.as_raw_fd(), &msg, &[]).unwrap();
        let mut buf = vec![0u8; MAX_LEN];
        let (n, _) = recv_msg(sock.as_raw_fd(), &mut buf).unwrap();
        let reply = bifrost_audio_types::control::decode_reply(&buf[..n]).unwrap();
        assert!(matches!(reply, Reply::Welcome { version: PROTOCOL_VERSION, .. }));
        stop.store(true, Ordering::Release);
        // Closing the connection wakes the poll, which then sees the stop flag.
        drop(sock);
        let _ = t.join();
        let _ = std::fs::remove_file(&path);
    }

    fn connect_to(path: &str) -> OwnedFd {
        // SAFETY: socket and connect with a valid sockaddr_un built below.
        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
        let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (d, s) in addr.sun_path.iter_mut().zip(path.as_bytes()) {
            *d = *s as libc::c_char;
        }
        let r = unsafe { libc::connect(fd, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_un>() as u32) };
        assert_eq!(r, 0);
        // SAFETY: a fresh descriptor.
        unsafe { OwnedFd::from_raw_fd(fd) }
    }

    #[test]
    fn a_silent_client_neither_blocks_the_others_nor_stays() {
        let path = format!("/tmp/bifrost-audio-test-silent-{}.sock", std::process::id());
        let l = listen(&path).unwrap();
        let (mut core, _) = core();
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let t = std::thread::spawn(move || serve(&mut core, &l, &s2, None));
        // connects and never sends Hello (the wedged bench)
        let silent = connect_to(&path);
        let sock = connect_to(&path);
        let msg = bifrost_audio_types::control::encode_request(&Request::Hello { version: PROTOCOL_VERSION, client: "t".into(), policy: false });
        send_msg(sock.as_raw_fd(), &msg, &[]).unwrap();
        let mut buf = vec![0u8; MAX_LEN];
        let began = Instant::now();
        let (n, _) = recv_msg(sock.as_raw_fd(), &mut buf).unwrap();
        assert!(matches!(bifrost_audio_types::control::decode_reply(&buf[..n]).unwrap(), Reply::Welcome { .. }));
        assert!(began.elapsed() < Duration::from_millis(500), "{:?}", began.elapsed());
        // after HELLO_TIMEOUT the server has closed the silent connection: end of stream, not a wait
        std::thread::sleep(HELLO_TIMEOUT + Duration::from_millis(1500));
        let (n, _) = recv_msg(silent.as_raw_fd(), &mut buf).unwrap();
        assert_eq!(n, 0);
        stop.store(true, Ordering::Release);
        drop(sock);
        let _ = t.join();
        let _ = std::fs::remove_file(&path);
    }
}
