//! Bluetooth File Exchange's disk and bus work. Requests run serially on the utility dispatch queue, so slow transfers
//! never stall the window; replies wake it through a descriptor. One file is sent at a time, and cancel/close asks obexd
//! to stop the active transfer.

use crate::model::{self, FileEntry, Status};
use crate::transport::Obexd;
use bifrost_app::dispatch::{DispatchQueue, Qos};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// How often the transfer's progress is read from obexd (milliseconds).
const POLL_MS: u64 = 250;
/// A transfer that sends nothing for this long is given up on.
const STALL: Duration = Duration::from_secs(120);

/// A request for the worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// List the files to send (in `downloads`) and the received files (in `received`).
    List { downloads: PathBuf, received: PathBuf },
    /// Send the file at `path` to the device at `address` ("AA:BB:CC:DD:EE:FF"). Progress is reported under `id`.
    Send { id: u64, address: String, path: PathBuf },
    /// Stop the transfer `id` if it is the one running.
    Cancel { id: u64 },
}

/// What the worker sends back. Every transfer reply carries the transfer's id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Listed { outbox: Vec<FileEntry>, inbox: Vec<FileEntry> },
    Progress { id: u64, sent: u64 },
    Done { id: u64 },
    Failed { id: u64, text: String },
    Cancelled { id: u64 },
}

/// The window's worker state. Requests run serially on dispatch; dropping it cancels pending and active sends.
pub struct Link {
    replies: Sender<Reply>,
    rx: Receiver<Reply>,
    cancels: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>>,
    closed: Arc<AtomicBool>,
    wake_rx: Option<UnixStream>,
    wake_tx: Option<Arc<Mutex<UnixStream>>>,
    obex: Arc<Mutex<Option<Obexd>>>,
    queue: DispatchQueue,
}

impl Link {
    /// Creates the worker state. Slow work never blocks the window loop.
    pub fn start() -> Link {
        let (replies, rx) = mpsc::channel::<Reply>();
        let (wake_rx, wake_tx) = UnixStream::pair().ok().and_then(|(rx, tx)| {
            rx.set_nonblocking(true).ok()?;
            tx.set_nonblocking(true).ok()?;
            Some((rx, Arc::new(Mutex::new(tx))))
        }).map_or((None, None), |(rx, tx)| (Some(rx), Some(tx)));
        Link {
            replies, rx,
            cancels: Arc::new(Mutex::new(HashMap::new())),
            closed: Arc::new(AtomicBool::new(false)),
            wake_rx, wake_tx,
            obex: Arc::new(Mutex::new(None)),
            queue: DispatchQueue::serial_qos("bluetooth-exchange", Qos::Utility),
        }
    }

    pub fn send(&self, request: Request) {
        if self.closed.load(Ordering::Relaxed) { return; }
        match request {
            Request::Cancel { id } => {
                if let Ok(cancels) = self.cancels.lock() {
                    if let Some(cancel) = cancels.get(&id) { cancel.store(true, Ordering::Relaxed); }
                }
            }
            Request::List { downloads, received } => {
                let (replies, wake, closed) = (self.replies.clone(), self.wake_tx.clone(), self.closed.clone());
                self.queue.async_(move || {
                    if closed.load(Ordering::Relaxed) { return; }
                    report(&replies, &wake, &closed, Reply::Listed {
                        outbox: model::listing(files_in(&downloads)), inbox: model::listing(files_in(&received)),
                    });
                });
            }
            Request::Send { id, address, path } => {
                let cancel = Arc::new(AtomicBool::new(false));
                if let Ok(mut cancels) = self.cancels.lock() { cancels.insert(id, cancel.clone()); }
                let (replies, wake, closed, cancels, obex) = (self.replies.clone(), self.wake_tx.clone(), self.closed.clone(), self.cancels.clone(), self.obex.clone());
                self.queue.async_(move || {
                    if closed.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed) {
                        if let Ok(mut cancels) = cancels.lock() { cancels.remove(&id); }
                        if !closed.load(Ordering::Relaxed) { report(&replies, &wake, &closed, Reply::Cancelled { id }); }
                        return;
                    }
                    let reply = match obex.lock() {
                        Ok(mut slot) => match bus(&mut slot) {
                            Ok(obexd) => transfer(obexd, id, &address, &path, &cancel, &closed, &replies, &wake),
                            Err(text) => Reply::Failed { id, text },
                        },
                        Err(_) => Reply::Failed { id, text: "The transfer worker is unavailable".into() },
                    };
                    if let Ok(mut cancels) = cancels.lock() { cancels.remove(&id); }
                    report(&replies, &wake, &closed, reply);
                });
            }
        }
    }

    /// The replies that arrived since the last call.
    pub fn drain(&self) -> Vec<Reply> {
        if let Some(mut wake) = self.wake_rx.as_ref() {
            let mut buf = [0u8; 64];
            while wake.read(&mut buf).is_ok_and(|n| n > 0) {}
        }
        self.rx.try_iter().collect()
    }

    pub fn wake_fd(&self) -> Option<i32> { self.wake_rx.as_ref().map(AsRawFd::as_raw_fd) }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
        if let Ok(cancels) = self.cancels.lock() {
            for cancel in cancels.values() { cancel.store(true, Ordering::Relaxed); }
        }
    }
}

fn report(replies: &Sender<Reply>, wake: &Option<Arc<Mutex<UnixStream>>>, closed: &AtomicBool, reply: Reply) {
    if !closed.load(Ordering::Relaxed) && replies.send(reply).is_ok() {
        if let Some(mut wake) = wake.as_ref().and_then(|w| w.lock().ok()) { let _ = wake.write(&[1]); }
    }
}

/// Why a transfer stopped before it was sent.
enum Stop {
    Cancelled,
    Failed(String),
}

/// The bus connection, made on first use. A failed connection is tried again on the next transfer.
fn bus(slot: &mut Option<Obexd>) -> Result<&Obexd, String> {
    if slot.is_none() {
        *slot = Some(Obexd::connect()?);
    }
    slot.as_ref().ok_or_else(|| "No session bus".to_string())
}

/// Sends one file in its own session and closes the session afterwards, whatever happened.
fn transfer(
    obexd: &Obexd,
    id: u64,
    address: &str,
    path: &Path,
    cancel: &AtomicBool,
    closed: &AtomicBool,
    replies: &Sender<Reply>,
    wake: &Option<Arc<Mutex<UnixStream>>>,
) -> Reply {
    let session = match obexd.create_session(address) {
        Ok(session) => session,
        Err(text) => return Reply::Failed { id, text },
    };
    let outcome = run(obexd, &session, id, path, cancel, closed, replies, wake);
    let _ = obexd.remove_session(&session);
    match outcome {
        Ok(()) => Reply::Done { id },
        Err(Stop::Cancelled) => Reply::Cancelled { id },
        Err(Stop::Failed(text)) => Reply::Failed { id, text },
    }
}

/// Starts the send on `session` and reads its progress until obexd reports it complete, failed or stalled.
fn run(
    obexd: &Obexd,
    session: &str,
    id: u64,
    path: &Path,
    cancel: &AtomicBool,
    closed: &AtomicBool,
    replies: &Sender<Reply>,
    wake: &Option<Arc<Mutex<UnixStream>>>,
) -> Result<(), Stop> {
    let file = path.to_str().ok_or_else(|| Stop::Failed("The file name is not readable text".to_string()))?;
    let transfer = obexd.send_file(session, file).map_err(Stop::Failed)?;
    let mut last_sent = 0u64;
    let mut last_change = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) || closed.load(Ordering::Relaxed) {
            let _ = obexd.cancel(&transfer);
            return Err(Stop::Cancelled);
        }
        let (sent, status) = obexd.progress(&transfer).map_err(Stop::Failed)?;
        match status {
            Status::Complete => return Ok(()),
            Status::Error => return Err(Stop::Failed("The device did not accept the file".to_string())),
            Status::Queued | Status::Active | Status::Other => {}
        }
        if sent != last_sent {
            last_sent = sent;
            last_change = Instant::now();
            report(replies, wake, closed, Reply::Progress { id, sent });
        } else if last_change.elapsed() > STALL {
            let _ = obexd.cancel(&transfer);
            return Err(Stop::Failed("The transfer stopped moving and was given up".to_string()));
        }
        std::thread::sleep(Duration::from_millis(POLL_MS));
    }
}

/// The regular files in `dir`, with their sizes and times. A folder that cannot be read lists nothing.
fn files_in(dir: &Path) -> Vec<FileEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
            Some(FileEntry { name: entry.file_name().to_string_lossy().into_owned(), path: entry.path(), size: meta.len(), modified })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_that_is_missing_lists_nothing() {
        assert!(files_in(Path::new("/no/such/bluetooth-exchange-folder")).is_empty());
    }

    #[test]
    fn a_cancel_with_nothing_running_is_a_no_op() {
        let link = Link::start();
        link.send(Request::Cancel { id: 7 });
        assert!(link.drain().is_empty());
    }
}
