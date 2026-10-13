//! The Print Center's link to the scheduler. A thread owns the CUPS client, so a slow or stopped scheduler never stalls the
//! window: requests go in on one channel and replies come back on another, which the window drains on each tick.

use crate::model::{Action, Reply};
use bifrost_app::controls::cups::{Client, Transport};
use std::sync::mpsc::{self, Receiver, Sender};

/// A request for the worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// List the printers, and the queue of `printer` when there is one.
    List { printer: Option<String> },
    /// Perform `action` on `printer` (or on one of its jobs).
    Act { printer: String, action: Action },
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
            Reply::Listed { printers, printer, jobs }
        }
        Request::Act { printer, action } => {
            let result = match &action {
                Action::Pause => client.pause_printer(&printer),
                Action::Resume => client.resume_printer(&printer),
                Action::SetDefault => client.set_default(&printer),
                Action::CancelJob(id) => client.cancel_job(&printer, *id),
                Action::HoldJob(id) => client.hold_job(&printer, *id),
                Action::ReleaseJob(id) => client.release_job(&printer, *id),
            };
            match result {
                Ok(()) => Reply::Done(action.done()),
                Err(e) => Reply::Failed(e.to_string()),
            }
        }
    }
}
