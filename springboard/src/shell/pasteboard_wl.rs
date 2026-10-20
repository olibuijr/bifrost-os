//! The shell's side of the window server's selection: a `zwlr_data_control_v1` client on a thread of its own
//! (`pasteboard_sync.rs` is the logic, this is the [`Transport`]).
//!
//! The thread connects through the privileged socket (`bare::privileged`), where the window server lets the shell set the
//! selection without a keyboard-focused surface of its own (`wm/src/data_control.rs`). It
//!
//! - **offers**: a command from the shell makes a new data source with every offered MIME type and sets it as the
//!   selection; when a program asks for a type (`send`), the bytes are written to the descriptor it gave, on a short-lived
//!   thread (at most [`MAX_SENDERS`] at a time, each with a deadline) so a slow reader never stalls the connection;
//! - **imports**: when another program sets the selection, the types worth keeping (`pasteboard_sync::wanted`) are read
//!   through pipes, all at once under one deadline, and handed to the shell, which wakes through [`notify_fd`]. A
//!   selection that carries the password-manager hint is passed on with [`SECRET_MARK`]; a program clearing the selection
//!   it owned is passed on as [`CLEARED_MARK`]. Selections that follow each other faster than [`MIN_IMPORT_GAP_MS`] are
//!   coalesced: only the newest is read.
//!
//! While the shell's own source is the selection, the selection events it receives describe that source and are
//! ignored. The window server cancels the source before it announces a program's selection, so the first `selection`
//! after a `cancelled` is a program's. Offers that are replaced before they were read, or never selected, are destroyed.
//!
//! The connection is kept alive by a supervisor: any error, or a panic, ends the connection and a new one is made after a
//! delay that doubles up to [`BACKOFF_MAX_MS`] (the window server may restart, or not be up yet). The newest offer the
//! shell made is offered again on every new connection. The descriptor the shell polls has a write end held by the
//! shell's handle, so the thread ending can never leave it readable forever.
//!
//! What was copied is never logged. A missing global only keeps the retries going, quietly; the shell runs without the
//! window server clipboard meanwhile (the apps and the shell's fields still share theirs).
#![cfg_attr(test, allow(dead_code))]

use super::pasteboard_sync::{is_secret_hint, valid_mime, wanted, Offer, Read, Transport, CLEARED_MARK, MAX_IMPORT_BYTES, MAX_IMPORT_TOTAL, SECRET_MARK};
use crate::bare::sys::{fcntl, now_ms, poll_revents, wait, O_NONBLOCK, POLLERR};
use std::collections::HashMap;
use std::ffi::c_int;
use std::io::{Read as _, Write as _};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use wayland_client::backend::ObjectId;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{event_created_child, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1 as dev, zwlr_data_control_manager_v1 as mgr, zwlr_data_control_offer_v1 as off, zwlr_data_control_source_v1 as src,
};

/// How long reading the types of another program's selection may take, all of them together.
const READ_MS: f64 = 1500.0;
/// The shortest time between two imports; a selection that arrives sooner waits (a newer one replaces it).
const MIN_IMPORT_GAP_MS: f64 = 250.0;
/// How many offered types of one selection are kept (the rest is ignored).
const MAX_OFFER_MIMES: usize = 256;
/// The first and the longest delay before the thread connects again, in ms.
const BACKOFF_MIN_MS: f64 = 500.0;
const BACKOFF_MAX_MS: f64 = 30_000.0;
/// A connection that held this long (ms) starts the delays over.
const STABLE_MS: f64 = 10_000.0;
/// The most threads writing a copy to programs that asked for it, and how long each may take, in ms.
const MAX_SENDERS: usize = 8;
const SEND_MS: f64 = 5000.0;

const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;
const POLLOUT: i16 = 4;
const POLLNVAL: i16 = 0x20;

/// The descriptor the shell polls; `-1` until the thread exists.
static NOTIFY_FD: AtomicI32 = AtomicI32::new(-1);

/// See [`super::pasteboard_sync::poll_fd`].
pub fn notify_fd() -> c_int {
    NOTIFY_FD.load(Ordering::Relaxed)
}

/// The shell's handle on the thread.
pub struct Wl {
    commands: Sender<Vec<Offer>>,
    wake: UnixStream,
    incoming: Receiver<Vec<Read>>,
    notified: UnixStream,
    /// A write end of the notify pair that outlives the thread: the pair never reads as closed.
    _keep_open: UnixStream,
}

impl Wl {
    /// Starts the thread. It connects on its own time: until then, and if it cannot, offers are held for the connection.
    pub fn start() -> Option<Wl> {
        let (wake_shell, wake_thread) = UnixStream::pair().ok()?;
        let (notify_shell, notify_thread) = UnixStream::pair().ok()?;
        for s in [&wake_shell, &wake_thread, &notify_shell, &notify_thread] {
            s.set_nonblocking(true).ok()?;
        }
        let keep_open = notify_thread.try_clone().ok()?;
        let (commands, command_rx) = channel();
        let (out, incoming) = channel();
        std::thread::Builder::new().name("clipboard".into()).spawn(move || supervise(command_rx, wake_thread, notify_thread, out)).ok()?;
        NOTIFY_FD.store(notify_shell.as_raw_fd(), Ordering::Relaxed);
        Some(Wl { commands, wake: wake_shell, incoming, notified: notify_shell, _keep_open: keep_open })
    }
}

impl Drop for Wl {
    fn drop(&mut self) {
        // the descriptor is about to close: the shell must not poll a number another file may get
        let _ = NOTIFY_FD.compare_exchange(self.notified.as_raw_fd(), -1, Ordering::Relaxed, Ordering::Relaxed);
    }
}

impl Transport for Wl {
    fn set(&mut self, offers: Vec<Offer>) {
        if self.commands.send(offers).is_ok() {
            let _ = self.wake.write(&[1]);
        }
    }

    fn incoming(&mut self) -> Option<Vec<Read>> {
        let mut scratch = [0u8; 64];
        while matches!(self.notified.read(&mut scratch), Ok(n) if n > 0) {}
        self.incoming.try_iter().last()
    }
}

/// An offer announced by the window server: the proxy (so it can be destroyed) and the types it lists.
type Announced = (off::ZwlrDataControlOfferV1, Vec<String>);

/// What the event handlers record for the loop.
#[derive(Default)]
struct State {
    globals: Vec<(u32, String, u32)>,
    /// The offers announced and not made the selection (yet).
    mimes: HashMap<ObjectId, Announced>,
    /// The newest `selection` event: `Some(None)` is a cleared selection.
    selection: Option<Option<Announced>>,
    /// The generation of the shell's source while it is the selection.
    alive: Option<u64>,
    /// What the source of generation `.0` serves.
    served: (u64, Vec<Offer>),
    /// Sources the window server cancelled, by generation.
    cancelled: Vec<u64>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(st: &mut Self, _: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            st.globals.push((name, interface, version));
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<mgr::ZwlrDataControlManagerV1, ()> for State {
    fn event(_: &mut Self, _: &mgr::ZwlrDataControlManagerV1, _: mgr::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<dev::ZwlrDataControlDeviceV1, ()> for State {
    fn event(st: &mut Self, _: &dev::ZwlrDataControlDeviceV1, ev: dev::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            dev::Event::DataOffer { id } => {
                st.mimes.insert(id.id(), (id, Vec::new()));
            }
            dev::Event::Selection { id } => {
                // a selection that was never read is superseded by this one: its offer is destroyed, not dropped
                if let Some(Some((old, _))) = st.selection.take() {
                    old.destroy();
                }
                let chosen = id.map(|o| {
                    let mimes = st.mimes.remove(&o.id()).map(|(_, m)| m).unwrap_or_default();
                    (o, mimes)
                });
                // offers announced and never selected (a primary selection) are not kept
                for (_, (unused, _)) in st.mimes.drain() {
                    unused.destroy();
                }
                st.selection = Some(chosen);
            }
            _ => {}
        }
    }
    event_created_child!(State, dev::ZwlrDataControlDeviceV1, [
        dev::EVT_DATA_OFFER_OPCODE => (off::ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<off::ZwlrDataControlOfferV1, ()> for State {
    fn event(st: &mut Self, o: &off::ZwlrDataControlOfferV1, ev: off::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let off::Event::Offer { mime_type } = ev {
            if let Some((_, list)) = st.mimes.get_mut(&o.id()) {
                if list.len() < MAX_OFFER_MIMES {
                    list.push(mime_type);
                }
            }
        }
    }
}

impl Dispatch<src::ZwlrDataControlSourceV1, u64> for State {
    fn event(st: &mut Self, _: &src::ZwlrDataControlSourceV1, ev: src::Event, generation: &u64, _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            src::Event::Send { mime_type, fd } => {
                if st.served.0 != *generation {
                    return;
                }
                let Some((_, data)) = st.served.1.iter().find(|(m, _)| *m == mime_type) else { return };
                spawn_send(fd, data.clone());
            }
            src::Event::Cancelled => st.cancelled.push(*generation),
            _ => {}
        }
    }
}

/// The count of threads writing a copy to a program; one is held by each [`SenderSlot`].
static SENDERS: AtomicUsize = AtomicUsize::new(0);

/// One of the [`MAX_SENDERS`] places for a thread that writes to a program.
struct SenderSlot;

impl SenderSlot {
    fn take() -> Option<SenderSlot> {
        if SENDERS.fetch_add(1, Ordering::AcqRel) >= MAX_SENDERS {
            SENDERS.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(SenderSlot)
    }
}

impl Drop for SenderSlot {
    fn drop(&mut self) {
        SENDERS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Writes `data` to the descriptor a program gave, on a thread of its own so that a slow reader never stalls the
/// connection. At most [`MAX_SENDERS`] run at once (a request beyond that is dropped: the descriptor closes and the reader
/// sees an empty copy), and each gives up after [`SEND_MS`]. A thread that cannot be started drops the request too.
fn spawn_send(fd: OwnedFd, data: std::sync::Arc<Vec<u8>>) {
    let Some(slot) = SenderSlot::take() else { return };
    let _ = std::thread::Builder::new().name("clipboard-send".into()).spawn(move || {
        let _slot = slot;
        let mut file = std::fs::File::from(fd);
        write_with_deadline(&mut file, &data, SEND_MS);
    });
}

/// Writes all of `data` to `file`, or gives up `ms` from now (a reader that holds its pipe open and never reads). The
/// descriptor is made non-blocking; true when everything was written.
fn write_with_deadline(file: &mut std::fs::File, data: &[u8], ms: f64) -> bool {
    let fd = file.as_raw_fd();
    // SAFETY: `fcntl` on a descriptor `file` owns.
    unsafe {
        let flags = fcntl(fd, F_GETFL);
        if flags < 0 || fcntl(fd, F_SETFL, flags | O_NONBLOCK) < 0 {
            return false;
        }
    }
    let end = now_ms() + ms;
    let mut at = 0;
    let mut revents = Vec::new();
    while at < data.len() {
        match file.write(&data[at..]) {
            Ok(0) => return false,
            Ok(n) => at += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let left = end - now_ms();
                if left <= 0.0 {
                    return false;
                }
                poll_revents(&[fd], &[POLLOUT], left.ceil() as i32, &mut revents);
                if revents.first().copied().unwrap_or(0) & (POLLERR | POLLNVAL) != 0 {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
    true
}

/// The thread's connection and objects.
struct Client {
    conn: Connection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    st: State,
    manager: mgr::ZwlrDataControlManagerV1,
    device: dev::ZwlrDataControlDeviceV1,
    /// The shell's source while it exists, by generation.
    source: Option<(u64, src::ZwlrDataControlSourceV1)>,
    generation: u64,
    /// When the last selection was read (monotonic ms; 0 before the first).
    last_import: f64,
}

impl Client {
    fn connect() -> Result<Client, String> {
        let conn = crate::bare::privileged::connect().map_err(|e| e.to_string())?;
        let mut queue = conn.new_event_queue::<State>();
        let qh = queue.handle();
        let registry = conn.display().get_registry(&qh, ());
        let mut st = State::default();
        queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
        let (mut seat, mut manager) = (None, None);
        for (name, iface, version) in st.globals.clone() {
            match iface.as_str() {
                "wl_seat" if seat.is_none() => seat = Some(registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(1), &qh, ())),
                "zwlr_data_control_manager_v1" if manager.is_none() => manager = Some(registry.bind::<mgr::ZwlrDataControlManagerV1, _, _>(name, 1, &qh, ())),
                _ => {}
            }
        }
        let seat = seat.ok_or("the window server lists no seat")?;
        let manager = manager.ok_or("the window server offers no data control")?;
        let device = manager.get_data_device(&seat, &qh, ());
        queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
        Ok(Client { conn, queue, qh, st, manager, device, source: None, generation: 0, last_import: 0.0 })
    }

    /// Makes `offers` the selection, replacing the shell's earlier source. A type the request could not carry (a NUL or
    /// another control character would end the C string the protocol sends) is left out.
    fn set(&mut self, mut offers: Vec<Offer>) {
        offers.retain(|(m, _)| valid_mime(m));
        if let Some((_, old)) = self.source.take() {
            old.destroy();
        }
        self.st.alive = None;
        if offers.is_empty() {
            self.st.served = (self.generation, Vec::new());
            self.device.set_selection(None);
            return;
        }
        self.generation += 1;
        let source = self.manager.create_data_source(&self.qh, self.generation);
        for (mime, _) in &offers {
            source.offer(mime.clone());
        }
        self.st.served = (self.generation, offers);
        self.st.alive = Some(self.generation);
        self.device.set_selection(Some(&source));
        self.source = Some((self.generation, source));
    }

    /// How long (ms) until a selection held back by the import gap can be read, `None` when nothing waits.
    fn import_wait(&self, now: f64) -> Option<f64> {
        matches!(self.st.selection, Some(Some(_))).then(|| (self.last_import + MIN_IMPORT_GAP_MS - now).max(1.0))
    }

    /// Reacts to what the window server announced since the last pass. `abort_fd` becomes readable when the shell has a
    /// new offer: a read in progress is dropped then, the shell's copy is newer, and the selection is kept to be read again
    /// (the byte can also belong to a command that was applied already, and that import must not be lost).
    ///
    /// True when another program's selection (or its clearing) was dealt with: the shell's last offer no longer stands for
    /// the selection, so a new connection must not offer it again.
    fn settle(&mut self, out: &Sender<Vec<Read>>, notify: &mut UnixStream, abort_fd: c_int) -> bool {
        for generation in std::mem::take(&mut self.st.cancelled) {
            if self.st.alive == Some(generation) {
                self.st.alive = None;
            }
            if self.source.as_ref().is_some_and(|(g, _)| *g == generation) {
                if let Some((_, source)) = self.source.take() {
                    source.destroy();
                }
            }
        }
        let tell = |batch: Vec<Read>, notify: &mut UnixStream| {
            if !batch.is_empty() && out.send(batch).is_ok() {
                let _ = notify.write(&[1]);
            }
        };
        match self.st.selection {
            None => return false,
            Some(None) => {
                self.st.selection = None;
                // the shell's source is cancelled before a program clears the selection, so a clear while the source is
                // alive is the shell's own (the shell ignores a clear that does not follow an import in any case)
                if self.st.alive.is_none() {
                    tell(vec![(CLEARED_MARK.to_string(), Vec::new())], notify);
                    return true;
                }
                return false;
            }
            Some(Some(_)) => {}
        }
        if self.st.alive.is_some() {
            // the selection is the shell's own source
            if let Some(Some((offer, _))) = self.st.selection.take() {
                offer.destroy();
            }
            return false;
        }
        let now = now_ms();
        if self.last_import > 0.0 && now < self.last_import + MIN_IMPORT_GAP_MS {
            // selections follow each other too fast: wait, and a newer one replaces this one unread
            return false;
        }
        let Some(Some((offer, mimes))) = self.st.selection.take() else { return false };
        let previous_import = self.last_import;
        self.last_import = now;
        let secret = mimes.iter().any(|m| is_secret_hint(m));
        let Some(mut read) = read_offer(&self.conn, &offer, &wanted(&mimes), abort_fd) else {
            // stopped by the wake byte: kept for the next pass, which applies the command (a newer shell offer then makes
            // the window server send another selection, and this one is destroyed with it) or, when the command was applied
            // already, reads it
            self.last_import = previous_import;
            self.st.selection = Some(Some((offer, mimes)));
            return false;
        };
        offer.destroy();
        if secret && !read.is_empty() {
            read.push((SECRET_MARK.to_string(), Vec::new()));
        }
        tell(read, notify);
        true
    }
}

/// A pipe, close-on-exec on both ends.
fn pipe() -> Option<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as c_int; 2];
    // SAFETY: `pipe2` fills the two-element array when it succeeds.
    if unsafe { crate::bare::sys::pipe2(fds.as_mut_ptr(), crate::bare::sys::O_CLOEXEC) } < 0 {
        return None;
    }
    // SAFETY: both descriptors are new and owned by nothing else.
    Some(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// One type being read from another program.
struct Reading {
    mime: String,
    file: std::fs::File,
    data: Vec<u8>,
    /// Whether the program closed its end after a complete write.
    done: bool,
    failed: bool,
}

/// Reads `types` of another program's selection, all at once under one deadline of [`READ_MS`] and one byte budget
/// ([`MAX_IMPORT_BYTES`] per type, [`MAX_IMPORT_TOTAL`] together). A type that did not arrive in time, was too large or
/// failed is left out. `abort_fd` becoming readable ends the read: `None`, so the caller can keep the selection.
fn read_offer(conn: &Connection, offer: &off::ZwlrDataControlOfferV1, types: &[String], abort_fd: c_int) -> Option<Vec<Read>> {
    let mut reads: Vec<Reading> = Vec::new();
    let mut writers: Vec<OwnedFd> = Vec::new();
    for mime in types {
        let Some((reader, writer)) = pipe() else { continue };
        offer.receive(mime.clone(), writer.as_fd());
        reads.push(Reading { mime: mime.clone(), file: std::fs::File::from(reader), data: Vec::new(), done: false, failed: false });
        writers.push(writer);
    }
    if conn.flush().is_err() {
        return Some(Vec::new());
    }
    // the programs hold their own copies of the write ends now; ours must go for the reads to see the end
    drop(writers);
    let end = now_ms() + READ_MS;
    let mut total = 0usize;
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let open: Vec<usize> = (0..reads.len()).filter(|&i| !reads[i].done && !reads[i].failed).collect();
        let left = end - now_ms();
        if open.is_empty() || left <= 0.0 {
            break;
        }
        let mut fds: Vec<c_int> = open.iter().map(|&i| reads[i].file.as_raw_fd()).collect();
        fds.push(abort_fd);
        let ready = wait(&fds, left.ceil() as i32);
        if ready.last().copied().unwrap_or(false) {
            return None;
        }
        for (slot, &i) in open.iter().enumerate() {
            if !ready.get(slot).copied().unwrap_or(false) {
                continue;
            }
            let r = &mut reads[i];
            match r.file.read(&mut chunk) {
                Ok(0) => r.done = true,
                Ok(n) => {
                    r.data.extend_from_slice(&chunk[..n]);
                    total += n;
                    if r.data.len() > MAX_IMPORT_BYTES || total > MAX_IMPORT_TOTAL {
                        // too large: drop this type, and give its bytes back to the budget
                        total -= r.data.len();
                        r.data = Vec::new();
                        r.failed = true;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => r.failed = true,
            }
        }
    }
    Some(reads.into_iter().filter(|r| r.done && !r.failed).map(|r| (r.mime, r.data)).collect())
}

/// Reports `error` on stderr unless it is the one reported last (a window server without data control would otherwise
/// fill the log with one line per retry).
fn report(last: &mut String, error: &str) {
    if last != error {
        eprintln!("springboard: clipboard bridge: {error}");
        *last = error.to_string();
    }
}

/// Runs [`run`] again and again: any error or panic ends one connection, a new one follows after a delay that doubles from
/// [`BACKOFF_MIN_MS`] to [`BACKOFF_MAX_MS`]. Ends when the shell's handle is gone.
fn supervise(commands: Receiver<Vec<Offer>>, mut wake: UnixStream, mut notify: UnixStream, out: Sender<Vec<Read>>) {
    let mut latest: Option<Vec<Offer>> = None;
    let mut backoff = BACKOFF_MIN_MS;
    let mut last_error = String::new();
    loop {
        let started = now_ms();
        let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&commands, &mut wake, &mut notify, &out, &mut latest)));
        match ended {
            Ok(Ok(())) => return,
            Ok(Err(e)) => report(&mut last_error, &e),
            Err(_) => report(&mut last_error, "the bridge thread panicked"),
        }
        if now_ms() - started > STABLE_MS {
            backoff = BACKOFF_MIN_MS;
        }
        if !pause(&commands, &mut wake, &mut latest, backoff) {
            return;
        }
        backoff = (backoff * 2.0).min(BACKOFF_MAX_MS);
    }
}

/// Waits `ms` while there is no connection, remembering the shell's newest offer for the next one. False when the shell's
/// handle is gone.
fn pause(commands: &Receiver<Vec<Offer>>, wake: &mut UnixStream, latest: &mut Option<Vec<Offer>>, ms: f64) -> bool {
    let end = now_ms() + ms;
    let mut scratch = [0u8; 64];
    loop {
        loop {
            match commands.try_recv() {
                Ok(offers) => *latest = Some(offers),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return false,
            }
        }
        while matches!(wake.read(&mut scratch), Ok(n) if n > 0) {}
        let left = end - now_ms();
        if left <= 0.0 {
            return true;
        }
        wait(&[wake.as_raw_fd()], left.ceil().min(1000.0) as i32);
    }
}

/// One connection's life: connects, offers the shell's newest offer again, then serves commands and events until an error.
/// `Ok` when the shell's handle is gone.
fn run(commands: &Receiver<Vec<Offer>>, wake: &mut UnixStream, notify: &mut UnixStream, out: &Sender<Vec<Read>>, latest: &mut Option<Vec<Offer>>) -> Result<(), String> {
    let mut c = Client::connect()?;
    if let Some(offers) = latest.as_ref().filter(|o| !o.is_empty()) {
        c.set(offers.clone());
    }
    let mut scratch = [0u8; 64];
    loop {
        // the wake bytes go first, then the commands they announce (a command is sent before its byte is written): a byte
        // still unread while a selection is being read means a command that arrived after this point, and ends that read
        while matches!(wake.read(&mut scratch), Ok(n) if n > 0) {}
        loop {
            match commands.try_recv() {
                Ok(offers) => {
                    *latest = Some(offers.clone());
                    c.set(offers);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        c.queue.dispatch_pending(&mut c.st).map_err(|e| e.to_string())?;
        if c.settle(out, notify, wake.as_raw_fd()) {
            // another program's selection stands now: the shell's older offer must not replace it on a reconnect
            *latest = None;
        }
        c.queue.flush().map_err(|e| e.to_string())?;
        if let Some(guard) = c.queue.prepare_read() {
            let fd = guard.connection_fd().as_raw_fd();
            let wait_ms = c.import_wait(now_ms()).map_or(5000, |w| w.ceil().min(5000.0) as i32);
            let ready = wait(&[fd, wake.as_raw_fd()], wait_ms);
            if ready.first().copied().unwrap_or(false) {
                guard.read().map_err(|e| e.to_string())?;
            } else {
                drop(guard);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pipe_carries_bytes_and_ends_when_the_writer_closes() {
        let (reader, writer) = pipe().expect("a pipe");
        std::fs::File::from(writer).write_all(b"hello").unwrap();
        let mut got = Vec::new();
        std::fs::File::from(reader).read_to_end(&mut got).unwrap();
        assert_eq!(got, b"hello");
    }

    #[test]
    fn the_poll_descriptor_is_none_before_the_thread_starts() {
        if NOTIFY_FD.load(Ordering::Relaxed) == -1 {
            assert_eq!(notify_fd(), -1);
        }
    }

    #[test]
    fn a_reader_that_never_reads_cannot_hold_a_writer_past_its_deadline() {
        let (reader, writer) = pipe().expect("a pipe");
        let started = now_ms();
        let done = write_with_deadline(&mut std::fs::File::from(writer), &vec![7u8; 4 << 20], 100.0);
        assert!(!done, "four megabytes do not fit a pipe nobody reads");
        assert!(now_ms() - started < 3000.0);
        drop(reader);
    }

    #[test]
    fn a_reader_that_reads_gets_everything_even_past_the_pipe_size() {
        let (reader, writer) = pipe().expect("a pipe");
        let data: Vec<u8> = (0..(1u32 << 20)).map(|i| (i % 251) as u8).collect();
        let expect = data.clone();
        let t = std::thread::spawn(move || {
            let mut got = Vec::new();
            std::fs::File::from(reader).read_to_end(&mut got).unwrap();
            got
        });
        assert!(write_with_deadline(&mut std::fs::File::from(writer), &data, 5000.0));
        assert_eq!(t.join().unwrap(), expect);
    }

    #[test]
    fn a_reader_that_went_away_ends_the_write_at_once() {
        let (reader, writer) = pipe().expect("a pipe");
        drop(reader);
        assert!(!write_with_deadline(&mut std::fs::File::from(writer), b"x", 5000.0));
    }

    #[test]
    fn at_most_a_few_threads_write_at_a_time() {
        let held: Vec<SenderSlot> = (0..MAX_SENDERS).filter_map(|_| SenderSlot::take()).collect();
        // other tests may hold none of them; if the pool was already shared the count is simply lower
        if held.len() == MAX_SENDERS {
            assert!(SenderSlot::take().is_none(), "the ninth request is dropped");
            drop(held);
            assert!(SenderSlot::take().is_some(), "a freed place can be used again");
        }
    }

    #[test]
    fn the_notify_pair_never_reads_as_closed_while_the_handle_lives() {
        let (shell, thread) = UnixStream::pair().unwrap();
        shell.set_nonblocking(true).unwrap();
        let keep = thread.try_clone().unwrap();
        drop(thread); // the thread ends
        let mut buf = [0u8; 8];
        let mut shell_end = &shell;
        assert_eq!(shell_end.read(&mut buf).unwrap_err().kind(), std::io::ErrorKind::WouldBlock, "no EOF: the poll set does not spin");
        drop(keep);
        assert_eq!(shell_end.read(&mut buf).unwrap(), 0, "only without the keep-alive does it read as closed");
    }

    #[test]
    fn the_pause_between_connections_keeps_the_newest_offer() {
        let (tx, rx) = channel::<Vec<Offer>>();
        let (mut wake, _other) = UnixStream::pair().unwrap();
        wake.set_nonblocking(true).unwrap();
        let mut latest = None;
        tx.send(vec![("text/plain".to_string(), std::sync::Arc::new(b"one".to_vec()))]).unwrap();
        tx.send(vec![("text/plain".to_string(), std::sync::Arc::new(b"two".to_vec()))]).unwrap();
        assert!(pause(&rx, &mut wake, &mut latest, 20.0));
        assert_eq!(latest.unwrap()[0].1.as_slice(), b"two");
        drop(tx);
        assert!(!pause(&rx, &mut wake, &mut None, 20.0), "the handle is gone: the thread ends");
    }
}
