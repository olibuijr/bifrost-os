//! The shell's side of the window server's selection: a `zwlr_data_control_v1` client on a thread of its own
//! (`pasteboard_sync.rs` is the logic, this is the [`Transport`]).
//!
//! The thread connects through the privileged socket (`bare::privileged`), where the window server lets the shell set the
//! selection without a keyboard-focused surface of its own (`wm/src/data_control.rs`). It
//!
//! - **offers**: a command from the shell makes a new data source with every offered MIME type and sets it as the
//!   selection; when a program asks for a type (`send`), the bytes are written to the descriptor it gave, on a short-lived
//!   thread so a slow reader never stalls the connection;
//! - **imports**: when another program sets the selection, the types worth keeping (`pasteboard_sync::wanted`) are read
//!   through pipes, each with a deadline, and handed to the shell, which wakes through [`notify_fd`].
//!
//! While the shell's own source is the selection, the selection events it receives describe that source and are
//! ignored. The window server cancels the source before it announces a program's selection, so the first `selection`
//! after a `cancelled` is a program's.
//!
//! What was copied is never logged. A missing global, a broken connection or a refused request only ends the thread; the
//! shell then runs without the window server clipboard (the apps and the shell's fields still share theirs).
#![cfg_attr(test, allow(dead_code))]

use super::pasteboard_sync::{wanted, Offer, Read, Transport, MAX_IMPORT_BYTES};
use crate::bare::sys::{now_ms, spawn_named, wait};
use std::collections::HashMap;
use std::ffi::c_int;
use std::io::{Read as _, Write as _};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use wayland_client::backend::ObjectId;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{event_created_child, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1 as dev, zwlr_data_control_manager_v1 as mgr, zwlr_data_control_offer_v1 as off, zwlr_data_control_source_v1 as src,
};

/// How long reading one type from another program may take.
const READ_MS: f64 = 1500.0;

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
}

impl Wl {
    /// Starts the thread. It connects on its own time: until then, and if it cannot, offers are dropped.
    pub fn start() -> Option<Wl> {
        let (wake_shell, wake_thread) = UnixStream::pair().ok()?;
        let (notify_shell, notify_thread) = UnixStream::pair().ok()?;
        for s in [&wake_shell, &wake_thread, &notify_shell, &notify_thread] {
            s.set_nonblocking(true).ok()?;
        }
        let (commands, command_rx) = channel();
        let (out, incoming) = channel();
        NOTIFY_FD.store(notify_shell.as_raw_fd(), Ordering::Relaxed);
        spawn_named("clipboard", move || {
            if let Err(e) = run(command_rx, wake_thread, notify_thread, out) {
                eprintln!("springboard: clipboard bridge: {e}");
            }
        });
        Some(Wl { commands, wake: wake_shell, incoming, notified: notify_shell })
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

/// What the event handlers record for the loop.
#[derive(Default)]
struct State {
    globals: Vec<(u32, String, u32)>,
    /// The types announced for offers that have not been made the selection yet.
    mimes: HashMap<ObjectId, Vec<String>>,
    /// The newest `selection` event: `Some(None)` is a cleared selection.
    selection: Option<Option<(off::ZwlrDataControlOfferV1, Vec<String>)>>,
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
                st.mimes.insert(id.id(), Vec::new());
            }
            dev::Event::Selection { id } => {
                st.selection = Some(id.map(|o| {
                    let mimes = st.mimes.remove(&o.id()).unwrap_or_default();
                    (o, mimes)
                }));
                // offers announced and never selected (a primary selection) are not kept
                st.mimes.clear();
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
            st.mimes.entry(o.id()).or_default().push(mime_type);
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
                let data = data.clone();
                // a reader may be slow, or gone: its descriptor is written on a thread of its own and closed after
                spawn_named("clipboard-send", move || {
                    let mut file = std::fs::File::from(fd);
                    let _ = file.write_all(&data);
                });
            }
            src::Event::Cancelled => st.cancelled.push(*generation),
            _ => {}
        }
    }
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
        Ok(Client { conn, queue, qh, st, manager, device, source: None, generation: 0 })
    }

    /// Makes `offers` the selection, replacing the shell's earlier source.
    fn set(&mut self, offers: Vec<Offer>) {
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

    /// Reacts to what the window server announced since the last pass.
    fn settle(&mut self, out: &Sender<Vec<Read>>, notify: &mut UnixStream) {
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
        let Some(selection) = self.st.selection.take() else { return };
        let Some((offer, mimes)) = selection else { return };
        if self.st.alive.is_some() {
            // the selection is the shell's own source
            offer.destroy();
            return;
        }
        let read: Vec<Read> = wanted(&mimes).into_iter().filter_map(|mime| read_offer(&self.conn, &offer, &mime).map(|data| (mime, data))).collect();
        offer.destroy();
        if !read.is_empty() && out.send(read).is_ok() {
            let _ = notify.write(&[1]);
        }
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

/// Reads one type of another program's selection, or `None` when it did not arrive in time, was too large or failed.
fn read_offer(conn: &Connection, offer: &off::ZwlrDataControlOfferV1, mime: &str) -> Option<Vec<u8>> {
    let (reader, writer) = pipe()?;
    offer.receive(mime.to_string(), writer.as_fd());
    conn.flush().ok()?;
    // the program holds its own copy of the write end now; ours must go for the read to see the end
    drop(writer);
    let mut file = std::fs::File::from(reader);
    let end = now_ms() + READ_MS;
    let mut data = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let left = end - now_ms();
        if left <= 0.0 || !wait(&[file.as_raw_fd()], left.ceil() as i32).first().copied().unwrap_or(false) {
            return None;
        }
        match file.read(&mut chunk) {
            Ok(0) => return Some(data),
            Ok(n) => {
                data.extend_from_slice(&chunk[..n]);
                if data.len() > MAX_IMPORT_BYTES {
                    return None;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
}

fn run(commands: Receiver<Vec<Offer>>, mut wake: UnixStream, mut notify: UnixStream, out: Sender<Vec<Read>>) -> Result<(), String> {
    let mut c = Client::connect()?;
    let mut scratch = [0u8; 64];
    loop {
        loop {
            match commands.try_recv() {
                Ok(offers) => c.set(offers),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        c.queue.dispatch_pending(&mut c.st).map_err(|e| e.to_string())?;
        c.settle(&out, &mut notify);
        c.queue.flush().map_err(|e| e.to_string())?;
        if let Some(guard) = c.queue.prepare_read() {
            let fd = guard.connection_fd().as_raw_fd();
            let ready = wait(&[fd, wake.as_raw_fd()], 5000);
            if ready.first().copied().unwrap_or(false) {
                guard.read().map_err(|e| e.to_string())?;
            } else {
                drop(guard);
            }
            if ready.get(1).copied().unwrap_or(false) {
                while matches!(wake.read(&mut scratch), Ok(n) if n > 0) {}
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
}
