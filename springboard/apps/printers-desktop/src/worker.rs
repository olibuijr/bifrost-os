//! The Print Center's link to the scheduler. A thread owns the CUPS client, so a slow or stopped scheduler never stalls the
//! window: requests go in on one channel and replies come back on another, which the window drains on each tick.

use crate::model::{self, Action, Reply};
use bifrost_app::controls::cups::{self, Client, Printer, Transport};
use std::collections::{HashMap, HashSet};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// How long a printer gets to accept a connection before it counts as offline.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);

/// A request for the worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// List the printers, and the queue of `printer` when there is one.
    List { printer: Option<String> },
    /// Perform `action` on `printer` (or on one of its jobs).
    Act { printer: String, action: Action },
    /// Add a queue for the device at this address (driverless).
    Add { device: String },
    /// Print a document on `printer`: `name` is the job name, `mime` the document type.
    Print { printer: String, name: String, mime: String, data: Vec<u8> },
}

/// The window's end of the channels. Dropping it ends the worker thread.
pub struct Link {
    tx: Sender<Request>,
    rx: Receiver<Reply>,
}

impl Link {
    /// Starts the worker thread for the local scheduler. If the thread cannot start, requests are dropped and no replies
    /// come back, so the window keeps showing what it has.
    pub fn start() -> Link {
        let (tx, requests) = mpsc::channel::<Request>();
        let (replies, rx) = mpsc::channel::<Reply>();
        let _ = std::thread::Builder::new().name("print-center".into()).spawn(move || serve(requests, replies));
        Link { tx, rx }
    }

    /// A link with no worker behind it: requests are dropped and no reply comes. For tests.
    #[cfg(test)]
    pub fn detached() -> Link {
        let (tx, _requests) = mpsc::channel::<Request>();
        let (_replies, rx) = mpsc::channel::<Reply>();
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

fn serve(requests: Receiver<Request>, replies: Sender<Reply>) {
    let mut client = Client::local();
    for request in requests {
        let reply = handle(&mut client, request);
        if replies.send(reply).is_err() {
            break;
        }
    }
}

/// Answers one request with the scheduler behind `client`.
pub fn handle<T: Transport>(client: &mut Client<T>, request: Request) -> Reply {
    match request {
        Request::List { printer } => {
            let printers = match client.printers() {
                Ok(printers) => printers,
                Err(e) => return Reply::Failed(e.to_string()),
            };
            let jobs = match printer.as_deref().map(|name| client.jobs(name)) {
                None => Vec::new(),
                Some(Ok(jobs)) => jobs,
                Some(Err(e)) => return Reply::Failed(e.to_string()),
            };
            let offline = offline_names(&printers);
            Reply::Listed { printers, printer, jobs, offline }
        }
        Request::Act { printer, action } => {
            let result = match &action {
                Action::Pause => client.pause_printer(&printer),
                Action::Resume => client.resume_printer(&printer),
                Action::SetDefault => client.set_default(&printer),
                Action::Delete => client.delete_printer(&printer),
                Action::DeletePool => client.delete_class(&printer),
                Action::SetShared(shared) => client.set_shared(&printer, *shared),
                Action::SetDetails { info, location } => client.set_printer_details(&printer, info, location),
                Action::CancelJob(id) => client.cancel_job(&printer, *id),
                Action::HoldJob(id) => client.hold_job(&printer, *id),
                Action::ReleaseJob(id) => client.release_job(&printer, *id),
            };
            match result {
                Ok(()) => Reply::Done(action.done()),
                Err(e) => Reply::Failed(e.to_string()),
            }
        }
        Request::Add { device } => match cups::add_device(client, &device) {
            Ok(name) => Reply::Added(name),
            Err(e) => Reply::Failed(e.to_string()),
        },
        Request::Print { printer, name, mime, data } => {
            let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
            match client.print_document(&printer, &name, &user, &mime, data) {
                Ok(_) => Reply::Printed(name),
                Err(e) => Reply::Failed(e.to_string()),
            }
        }
    }
}

/// What the probes found, by device address, and the addresses being probed now.
#[derive(Default)]
struct Probes {
    reachable: HashMap<String, bool>,
    running: HashSet<String>,
}

fn probes() -> &'static Mutex<Probes> {
    static PROBES: OnceLock<Mutex<Probes>> = OnceLock::new();
    PROBES.get_or_init(|| Mutex::new(Probes::default()))
}

/// Whether something accepts a connection at `host:port` within `PROBE_TIMEOUT`.
fn reachable(host: &str, port: u16) -> bool {
    (host, port)
        .to_socket_addrs()
        .map(|mut addrs| addrs.any(|a| TcpStream::connect_timeout(&a, PROBE_TIMEOUT).is_ok()))
        .unwrap_or(false)
}

/// The printers whose device did not answer the last probe. Each listing starts a fresh probe per network device on its
/// own thread (a name lookup can be slow), so the listing never waits for the network and shows the previous answer.
/// A printer not probed yet counts as reachable.
fn offline_names(printers: &[Printer]) -> Vec<String> {
    let mut offline = Vec::new();
    for p in printers.iter().filter(|p| !p.is_class) {
        let Some((host, port)) = model::probe_target(&p.device_uri) else { continue };
        let key = p.device_uri.clone();
        let Ok(mut state) = probes().lock() else { continue };
        if state.reachable.get(&key) == Some(&false) {
            offline.push(p.name.clone());
        }
        if state.running.insert(key.clone()) {
            drop(state);
            let _ = std::thread::Builder::new().name("print-center-probe".into()).spawn(move || {
                let ok = reachable(&host, port);
                if let Ok(mut state) = probes().lock() {
                    state.running.remove(&key);
                    state.reachable.insert(key, ok);
                }
            });
        }
    }
    offline
}
