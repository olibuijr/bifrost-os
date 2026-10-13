//! Bluetooth File Exchange's disk and bus work. A thread runs the requests, so a slow transfer never stalls the window:
//! requests go in on one channel and replies come back on another, which the window drains on each tick. One file is
//! sent at a time; a cancel asks obexd to stop the running transfer.

use crate::model::{self, FileEntry, Status};
use crate::transport::Obexd;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
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

/// The window's end of the channels. Dropping it ends the worker thread.
pub struct Link {
    tx: Sender<Request>,
    rx: Receiver<Reply>,
}

impl Link {
    /// Starts the worker thread. If it cannot start, requests are dropped and no replies come back.
    pub fn start() -> Link {
        let (tx, requests) = mpsc::channel::<Request>();
        let (replies, rx) = mpsc::channel::<Reply>();
        let _ = thread::Builder::new().name("bluetooth-exchange".into()).spawn(move || serve(requests, replies));
        Link { tx, rx }
    }

    pub fn send(&self, request: Request) {
        let _ = self.tx.send(request);
    }

    /// The replies that arrived since the last call.
    pub fn drain(&self) -> Vec<Reply> {
        self.rx.try_iter().collect()
    }
}

/// Why a transfer stopped before it was sent.
enum Stop {
    Cancelled,
    Failed(String),
}

fn serve(requests: Receiver<Request>, replies: Sender<Reply>) {
    // requests that arrive during a transfer and are not its cancel wait here, in order
    let mut pending: VecDeque<Request> = VecDeque::new();
    let mut obex: Option<Obexd> = None;
    loop {
        let request = match pending.pop_front() {
            Some(request) => request,
            None => match requests.recv() {
                Ok(request) => request,
                Err(_) => return,
            },
        };
        let sent = match request {
            Request::List { downloads, received } => replies.send(Reply::Listed {
                outbox: model::listing(files_in(&downloads)),
                inbox: model::listing(files_in(&received)),
            }),
            // nothing is running, so a cancel has nothing to stop
            Request::Cancel { .. } => Ok(()),
            Request::Send { id, address, path } => {
                let reply = match bus(&mut obex) {
                    Ok(obexd) => transfer(obexd, id, &address, &path, &requests, &mut pending, &replies),
                    Err(text) => Reply::Failed { id, text },
                };
                replies.send(reply)
            }
        };
        if sent.is_err() {
            return;
        }
    }
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
    requests: &Receiver<Request>,
    pending: &mut VecDeque<Request>,
    replies: &Sender<Reply>,
) -> Reply {
    let session = match obexd.create_session(address) {
        Ok(session) => session,
        Err(text) => return Reply::Failed { id, text },
    };
    let outcome = run(obexd, &session, id, path, requests, pending, replies);
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
    requests: &Receiver<Request>,
    pending: &mut VecDeque<Request>,
    replies: &Sender<Reply>,
) -> Result<(), Stop> {
    let file = path.to_str().ok_or_else(|| Stop::Failed("The file name is not readable text".to_string()))?;
    let transfer = obexd.send_file(session, file).map_err(Stop::Failed)?;
    let mut last_sent = 0u64;
    let mut last_change = Instant::now();
    loop {
        while let Ok(request) = requests.try_recv() {
            match request {
                Request::Cancel { id: wanted } if wanted == id => {
                    let _ = obexd.cancel(&transfer);
                    return Err(Stop::Cancelled);
                }
                other => pending.push_back(other),
            }
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
            let _ = replies.send(Reply::Progress { id, sent });
        } else if last_change.elapsed() > STALL {
            let _ = obexd.cancel(&transfer);
            return Err(Stop::Failed("The transfer stopped moving and was given up".to_string()));
        }
        thread::sleep(Duration::from_millis(POLL_MS));
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
        // the worker answers a cancel without a transfer by sending nothing back
        let (tx, requests) = mpsc::channel::<Request>();
        let (replies, rx) = mpsc::channel::<Reply>();
        tx.send(Request::Cancel { id: 7 }).unwrap();
        drop(tx);
        serve(requests, replies);
        assert!(rx.try_recv().is_err());
    }
}
